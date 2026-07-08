//! `freecell_engine::cache` — the IronCalc-facing builder/mutator for the style & geometry
//! cache (`components/style_cache.md`, `architecture.md §6`).
//!
//! This is the single place that reads IronCalc geometry + `Style` and converts them into the
//! engine-free read model the grid consumes (`freecell_core::SheetCache`, holding resolved
//! [`RenderStyle`](freecell_core::RenderStyle)s + px geometry). It exists so the render path
//! makes **zero engine calls** and the grid renders fully styled during a multi-second eval.
//!
//! Two operations:
//! - [`build_sheet_cache`] — **build on activation**: scan a worksheet's populated cells +
//!   row/col band collections + custom sizes into a fresh `SheetCache`.
//! - [`refresh_cell`] — **mirror the issued op**: after FreeCell applies an edit, re-read the
//!   touched cell's style and update just that entry, keeping the cache provably in agreement
//!   with a fresh engine re-read (the load-bearing contract; see the tests).
//!
//! ## Unit conversion (one place, `architecture.md §10`)
//!
//! IronCalc's geometry getters already return **pixels** (`ironcalc_base/src/constants.rs`:
//! "COLUMN_WIDTH and ROW_HEIGHT are pixel values"; defaults 90 px / 25 px on our fork, 125/28 on
//! 0.7.1 — see `IRONCALC_DEFAULT_*_PX`). FreeCell's chosen
//! grid defaults are 100 px / 24 px (`ui_design.md §3.3`). We convert an override by the ratio
//! `freecell_default / ironcalc_default`, so a track at IronCalc's default maps exactly to the
//! FreeCell default and any deviation scales proportionally (a 2× column stays 2× of 100 px).
//!
//! Coordinates: `freecell_core` cells are 0-based; IronCalc rows/cols are 1-based. The
//! `WorkbookDocument` accessors take 0-based indices; when this module reads a raw `Worksheet`
//! it converts the 1-based `Row.r` / `Col.min..=max` itself.

use freecell_core::cache::{DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use freecell_core::{
    limits, BorderSpec, CellRef, Edge, LinePattern, RenderStyle, Rgb, SheetCache, SheetCacheBuilder,
};
use ironcalc_base::types::{
    Border, BorderItem, BorderStyle, Color, HorizontalAlignment, Style, Theme, VerticalAlignment,
};

use crate::document::WorkbookDocument;

// These mirror the pinned engine's default row/col size (`ironcalc_base/src/constants.rs`) — the
// reference our px conversion maps onto FreeCell's own defaults, and the sentinel that marks a
// non-custom row/col. They MUST track the pinned engine; update them when the engine's defaults
// change (our fork set `DEFAULT_ROW_HEIGHT = 25`, `DEFAULT_COLUMN_WIDTH = 90`, vs 0.7.1's 28/125).
/// IronCalc's default column width in px (`ironcalc_base/src/constants.rs`
/// `DEFAULT_COLUMN_WIDTH`). A non-custom column reads back exactly this.
pub(crate) const IRONCALC_DEFAULT_COL_WIDTH_PX: f64 = 90.0;
/// IronCalc's default row height in px (`ironcalc_base/src/constants.rs` `DEFAULT_ROW_HEIGHT`).
pub(crate) const IRONCALC_DEFAULT_ROW_HEIGHT_PX: f64 = 25.0;

/// Converts an IronCalc column-width pixel value to a FreeCell device-px width (see module docs).
pub(crate) fn col_px(ironcalc_px: f64) -> f32 {
    (ironcalc_px * (DEFAULT_COL_WIDTH_PX as f64 / IRONCALC_DEFAULT_COL_WIDTH_PX)) as f32
}

/// Converts an IronCalc row-height pixel value to a FreeCell device-px height (see module docs).
pub(crate) fn row_px(ironcalc_px: f64) -> f32 {
    (ironcalc_px * (DEFAULT_ROW_HEIGHT_PX as f64 / IRONCALC_DEFAULT_ROW_HEIGHT_PX)) as f32
}

/// Converts a FreeCell device-px column width back to IronCalc pixels — the inverse of [`col_px`],
/// used to write a user resize (the grid speaks device px; the engine stores IronCalc px).
pub(crate) fn col_ironcalc_px(device_px: f64) -> f64 {
    device_px * (IRONCALC_DEFAULT_COL_WIDTH_PX / DEFAULT_COL_WIDTH_PX as f64)
}

/// Converts a FreeCell device-px row height back to IronCalc pixels — the inverse of [`row_px`].
pub(crate) fn row_ironcalc_px(device_px: f64) -> f64 {
    device_px * (IRONCALC_DEFAULT_ROW_HEIGHT_PX / DEFAULT_ROW_HEIGHT_PX as f64)
}

/// The FreeCell-px row-height **override** for `row` (0-based): `Some(px)` when the engine reports
/// a non-default height (a custom or auto-fit row), `None` when it is at the IronCalc default (so
/// the cache uses its own default). The worker's mirror path uses this to reflect IronCalc's
/// row-height auto-fit — a `set_user_input` grows a row when its content is taller than the
/// current height (`ironcalc_base/src/user_model/common.rs`, `set_user_input`) — after a value
/// edit, keeping the cache geometry in agreement across the edit and its undo.
pub(crate) fn row_override_px(doc: &WorkbookDocument, sheet_idx: u32, row: u32) -> Option<f32> {
    let ic = doc
        .row_height_px(sheet_idx, row)
        .unwrap_or(IRONCALC_DEFAULT_ROW_HEIGHT_PX);
    if (ic - IRONCALC_DEFAULT_ROW_HEIGHT_PX).abs() < 1e-6 {
        None
    } else {
        Some(row_px(ic))
    }
}

/// Parses an IronCalc colour string into an [`Rgb`]. Accepts `#RRGGBB` (the form IronCalc
/// stores and validates for styles set through it) and tolerates `#AARRGGBB` (Excel ARGB,
/// dropping the alpha byte). Any other shape → `None` (so an unexpected colour never panics; the
/// cell falls back to the render default).
pub(crate) fn parse_color(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?;
    // All-ASCII-hex check first: it rejects junk AND guarantees the `[2..]` slice below lands on
    // a char boundary (so a multibyte-garbage colour string can never panic).
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = match hex.len() {
        6 => hex,
        8 => &hex[2..], // #AARRGGBB → keep RRGGBB
        _ => return None,
    };
    let value = u32::from_str_radix(rgb, 16).ok()?;
    Some(Rgb::from_hex(value))
}

/// Maps an IronCalc [`BorderStyle`] to the grid's px weight class (`architecture.md §1.1`, corrected
/// to the actual nine 0.7.1 variants — the spec's `Hair`/`Dashed` don't exist at this rev; see
/// DECISIONS_TO_REVIEW): Thin/Dotted → `1`; the Medium family + SlantDashDot → `2`; Thick/Double →
/// `3`. The *pattern* (solid/dashed/double) is a separate axis — see [`border_pattern`].
pub(crate) fn border_weight(style: &BorderStyle) -> u8 {
    match style {
        BorderStyle::Thin | BorderStyle::Dotted => 1,
        BorderStyle::Medium
        | BorderStyle::MediumDashed
        | BorderStyle::MediumDashDot
        | BorderStyle::MediumDashDotDot
        | BorderStyle::SlantDashDot => 2,
        BorderStyle::Thick | BorderStyle::Double => 3,
    }
}

/// Maps an IronCalc [`BorderStyle`] to the grid's line [`LinePattern`] (`architecture.md §5`):
/// `MediumDashed → Dashed`, `Double → Double`; every other variant — the thin/medium/thick solids
/// and the **deferred** Dotted / dash-dot / SlantDashDot families — falls back to `Solid`. The
/// fallback keeps files that already carry a deferred style rendering exactly as they did before
/// (GAPS F3).
pub(crate) fn border_pattern(style: &BorderStyle) -> LinePattern {
    match style {
        BorderStyle::MediumDashed => LinePattern::Dashed,
        BorderStyle::Double => LinePattern::Double,
        BorderStyle::Thin
        | BorderStyle::Medium
        | BorderStyle::Thick
        | BorderStyle::Dotted
        | BorderStyle::MediumDashDot
        | BorderStyle::MediumDashDotDot
        | BorderStyle::SlantDashDot => LinePattern::Solid,
    }
}

/// Resolves an IronCalc [`Color`] to an `#RRGGBB` [`Rgb`], consulting the workbook `theme` for
/// theme-indexed colours (`Color::to_rgb`). `Color::None` — and any unparseable resolved string —
/// yields `None`, so an unexpected colour never panics; the cell falls back to the render default.
pub(crate) fn resolve_rgb(color: &Color, theme: &Theme) -> Option<Rgb> {
    parse_color(&color.to_rgb(theme))
}

/// Resolves one IronCalc [`BorderItem`] to a render [`Edge`]: its weight class + colour (defaulting
/// to black when the item carries none or an unparseable colour — the render never panics).
fn edge_from(item: &BorderItem, theme: &Theme) -> Edge {
    let color = resolve_rgb(&item.color, theme).unwrap_or(Rgb::new(0, 0, 0));
    Edge::with_pattern(
        border_weight(&item.style),
        color,
        border_pattern(&item.style),
    )
}

/// Resolves an IronCalc [`Border`] into the engine-free [`BorderSpec`] the grid paints. Only the
/// four side edges are drawn (diagonals are out of scope — `functional_spec.md §3.6`).
pub(crate) fn border_spec_from(border: &Border, theme: &Theme) -> BorderSpec {
    BorderSpec {
        top: border.top.as_ref().map(|e| edge_from(e, theme)),
        right: border.right.as_ref().map(|e| edge_from(e, theme)),
        bottom: border.bottom.as_ref().map(|e| edge_from(e, theme)),
        left: border.left.as_ref().map(|e| edge_from(e, theme)),
    }
}

/// Derives the engine-free [`RenderStyle`] from an IronCalc `Style` — the subset the grid draws
/// (`functional_spec.md §3.6`). Font family/size, borders, and number format are **side-table
/// indices** resolved by the caller (which holds the interning tables); everything IronCalc models
/// but the grid ignores (strikethrough, wrap, vertical/diagonal) stays in the engine and
/// round-trips on save.
///
/// `rsf(&Style::default()) == RenderStyle::default()` (asserted in the tests) — so
/// a plain cell interns to the default style and resolves to `None` (the grid's default paint).
pub(crate) fn render_style_from(style: &Style, theme: &Theme) -> RenderStyle {
    RenderStyle {
        bold: style.font.b,
        italic: style.font.i,
        underline: style.font.u,
        strikethrough: style.font.strike,
        // Wrap is `alignment.wrap_text` (a cell with no alignment record reads `false`).
        wrap: style
            .alignment
            .as_ref()
            .map(|a| a.wrap_text)
            .unwrap_or(false),
        // A solid fill's colour is `fill.color` (resolved against the theme); `Color::None` (or an
        // unparseable colour) → no fill.
        fill: resolve_rgb(&style.fill.color, theme),
        // `None` on RenderStyle means "grid default (near-black)"; IronCalc's default font colour
        // is pure black, so map black (and none/unparseable) to `None` to keep default cells
        // interning to the default style.
        font_color: resolve_rgb(&style.font.color, theme).filter(|rgb| *rgb != Rgb::new(0, 0, 0)),
        h_align: h_align_of(style),
        v_align: v_align_of(style),
        // `num_fmt` / `font_family` / `border` are side-table indices resolved by the caller
        // (build/refresh), which holds the interning tables; a bare conversion carries `0`
        // (= "general" / the workbook default family / BorderSpec::NONE). `font_size_q` is likewise
        // resolved by the caller against the workbook default (`font_size_q_of`). The caller
        // overrides all four before the default-check so a font/format/border-only cell is still stored.
        num_fmt: 0,
        font_size_q: 0,
        font_family: 0,
        border: 0,
    }
}

/// Converts an IronCalc font size (whole points, `i32`) to [`RenderStyle::font_size_q`]
/// quarter-points, mapping the **workbook default** size (`default_sz`) — and any non-positive
/// garbage — to `0` ("the workbook default"). Resolving the default relative to `default_sz`
/// (rather than a hardcoded 11/13) keeps every default-font cell interning to the default style,
/// so opened files render unchanged (`architecture.md §1.1`; the spec's literal "11pt" is a
/// documented misstatement — IronCalc's real default is 13pt). Saturates for hostile huge sizes.
pub(crate) fn font_size_q_of(sz: i32, default_sz: i32) -> u16 {
    if sz <= 0 || sz == default_sz {
        0
    } else {
        (sz.clamp(0, u16::MAX as i32) as u16).saturating_mul(4)
    }
}

/// Resolves a cell/band font `name` to a [`RenderStyle::font_family`] id against the workbook
/// default: the default name (and `""`) → `0`; any other name interns into the builder's side
/// table. Keeps a default-font cell mapping to the default style.
fn resolve_family(builder: &mut SheetCacheBuilder, name: &str, default_name: &str) -> u16 {
    if name == default_name {
        0
    } else {
        builder.intern_font_family(name)
    }
}

/// Maps IronCalc's horizontal alignment to the MVP grid's `Align`. Only Left/Center/Right are
/// drawn; `General` (and the Fill/Justify/Distributed/CenterContinuous variants the grid does
/// not implement) resolve to `None` = "engine default by cell type".
fn h_align_of(style: &Style) -> Option<freecell_core::Align> {
    use freecell_core::Align;
    match style.alignment.as_ref().map(|a| &a.horizontal) {
        Some(HorizontalAlignment::Left) => Some(Align::Left),
        Some(HorizontalAlignment::Center) => Some(Align::Center),
        Some(HorizontalAlignment::Right) => Some(Align::Right),
        _ => None,
    }
}

/// Maps IronCalc's vertical alignment to the grid's [`VAlign`](freecell_core::VAlign) (parallel to
/// [`h_align_of`]). Only Top/Center/Bottom are drawn; Justify/Distributed — and a cell with no
/// alignment record — resolve to `None`, which the grid renders as its default placement:
/// **bottom**, Excel-faithful (decision C — `functional_spec.md §1.3`, `architecture.md §5`).
///
/// IronCalc's `VerticalAlignment` default is `Bottom`, so a cell with *any* alignment record (e.g.
/// only `horizontal` set, or one loaded from `.xlsx`) carries `vertical = Bottom` and resolves to
/// `Some(Bottom)`. Under decision C this is coherent: `None` (no alignment record) and
/// `Some(Bottom)` (a record whose vertical is explicit-or-defaulted bottom) both render bottom, so
/// there is no visible split between "unset" and "defaulted-bottom". The accepted consequence is
/// that such a cell lights the **Align bottom** toolbar button — matching Excel's model where every
/// cell is bottom-aligned by default. The engine cannot distinguish an explicit `bottom` from a
/// defaulted one, and under C it does not need to.
fn v_align_of(style: &Style) -> Option<freecell_core::VAlign> {
    use freecell_core::VAlign;
    match style.alignment.as_ref().map(|a| &a.vertical) {
        Some(VerticalAlignment::Top) => Some(VAlign::Top),
        Some(VerticalAlignment::Center) => Some(VAlign::Center),
        Some(VerticalAlignment::Bottom) => Some(VAlign::Bottom),
        _ => None,
    }
}

/// Builds a fresh [`SheetCache`] for `sheet_idx` from the engine's current state — the
/// build-on-activation path (open builds the active sheet; other sheets build on first switch).
/// Cost is bounded by the sheet's populated/styled cells + band/size records, not the Excel-max
/// grid (`components/style_cache.md §Lifecycle`).
pub(crate) fn build_sheet_cache(
    doc: &WorkbookDocument,
    sheet_idx: u32,
) -> Result<SheetCache, String> {
    let mut builder = SheetCacheBuilder::new(limits::MAX_ROWS, limits::MAX_COLS);
    let ws = doc.worksheet(sheet_idx)?;
    // The workbook default font — each cell's font_size_q/font_family is resolved relative to it,
    // so a default-font cell interns to the default style (`font_size_q_of` + the family check).
    let (def_sz, def_name) = doc.default_font();
    // Record the workbook default size (quarter-points) so the action bar can label a default cell
    // with the real workbook default rather than a hardcoded value.
    builder.set_default_font_size_q((def_sz.clamp(0, u16::MAX as i32) as u16).saturating_mul(4));

    // Track the rows/cols that carry a *non-default* band, so a populated cell reverting to the
    // default style still gets an explicit entry to shadow the band (IronCalc's rule that a cell
    // present in the sheet data uses its own style over any band).
    let mut band_rows: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut band_cols: std::collections::HashSet<u32> = std::collections::HashSet::new();

    // Column custom widths + band styles. A `Col` record covers the inclusive 1-based range
    // [min, max]; the width/style is uniform across it.
    for col in &ws.cols {
        if col.custom_width {
            let px = col_px(doc.col_width_px(sheet_idx, (col.min - 1) as u32)?);
            for c in col.min..=col.max {
                builder.push_col_width((c - 1) as u32, px);
            }
        }
        if col.style.is_some() {
            if let Some(style) = doc.col_band_style(sheet_idx, (col.min - 1) as u32)? {
                let mut rs = render_style_from(&style, doc.workbook_theme());
                rs.num_fmt = builder.intern_num_fmt(&style.num_fmt);
                rs.font_size_q = font_size_q_of(style.font.sz, def_sz);
                rs.font_family = resolve_family(&mut builder, &style.font.name, &def_name);
                rs.border = builder
                    .intern_border_spec(border_spec_from(&style.border, doc.workbook_theme()));
                if rs != RenderStyle::default() {
                    for c in col.min..=col.max {
                        let c0 = (c - 1) as u32;
                        builder.push_col_style(c0, rs);
                        band_cols.insert(c0);
                    }
                }
            }
        }
    }

    // Row custom heights + band styles. IronCalc only applies a row band when `custom_format`
    // (see `Model::get_cell_style_index`), so mirror that gate here.
    for r in &ws.rows {
        let r0 = (r.r - 1) as u32;
        if r.custom_height {
            let px = row_px(doc.row_height_px(sheet_idx, r0)?);
            builder.push_row_height(r0, px);
        }
        if r.custom_format && r.s != 0 {
            if let Some(style) = doc.row_band_style(sheet_idx, r0)? {
                let mut rs = render_style_from(&style, doc.workbook_theme());
                rs.num_fmt = builder.intern_num_fmt(&style.num_fmt);
                rs.font_size_q = font_size_q_of(style.font.sz, def_sz);
                rs.font_family = resolve_family(&mut builder, &style.font.name, &def_name);
                rs.border = builder
                    .intern_border_spec(border_spec_from(&style.border, doc.workbook_theme()));
                if rs != RenderStyle::default() {
                    builder.push_row_style(r0, rs);
                    band_rows.insert(r0);
                }
            }
        }
    }

    // Merged ranges: parse the sheet's file-loaded A1 merge strings once (0-based) for the
    // insert/delete merge guard (`components/grid_structure.md §5.3`). A hostile/unparseable
    // entry is skipped + logged (never panics — defensive against malformed files).
    for m in &ws.merge_cells {
        match freecell_core::CellRange::from_a1(m) {
            Some(range) => builder.push_merge(range),
            None => tracing::debug!(merge = %m, "cache: skipping unparseable merge range"),
        }
    }

    // Per-cell styles: every populated/styled cell in the sheet data.
    for (row_1, cols) in &ws.sheet_data {
        let row0 = (*row_1 - 1) as u32;
        for col_1 in cols.keys() {
            let col0 = (*col_1 - 1) as u32;
            let cell = CellRef::new(row0, col0);
            if let Some(style) = doc.cell_own_style(sheet_idx, cell)? {
                let mut rs = render_style_from(&style, doc.workbook_theme());
                rs.num_fmt = builder.intern_num_fmt(&style.num_fmt);
                rs.font_size_q = font_size_q_of(style.font.sz, def_sz);
                rs.font_family = resolve_family(&mut builder, &style.font.name, &def_name);
                rs.border = builder
                    .intern_border_spec(border_spec_from(&style.border, doc.workbook_theme()));
                // `band_rows`/`band_cols` hold only *non-default* bands. A populated cell with a
                // default own style on such a band gets an explicit default entry to shadow it.
                // MICRO-EDGE (DECISIONS_TO_REVIEW, Phase 5): a `custom_format` row whose style
                // maps to the *default* RenderStyle (e.g. a future border-only band the grid
                // ignores) is not in `band_rows`, so it can't shadow a non-default col band here.
                // Not reachable via the edit APIs today; revisit when border/number-format
                // styling lands (it interacts with the full-row-band mirror rebuild then).
                let on_band = band_rows.contains(&row0) || band_cols.contains(&col0);
                if rs != RenderStyle::default() || on_band {
                    builder.push_cell_style(row0, col0, rs);
                }
            }
        }
    }

    Ok(builder.build())
}

/// Re-reads `cell`'s own style from the engine and updates its entry in `cache` — the
/// mirror-on-edit / undo-redo primitive. Reproduces IronCalc's `get_cell_style_index`: a cell
/// present in the sheet data uses its own style (even the default, which shadows a band); an
/// absent cell falls through to the band/default. Guarantees `cache.render_style(cell)` keeps
/// matching `get_style_for_cell(cell)`.
///
/// `def_sz` / `def_name` are the workbook default font (`doc.default_font()`), passed in so a
/// multi-cell refresh resolves it **once** rather than re-reading + re-cloning it per cell.
pub(crate) fn refresh_cell(
    cache: &mut SheetCache,
    doc: &WorkbookDocument,
    sheet_idx: u32,
    cell: CellRef,
    def_sz: i32,
    def_name: &str,
) -> Result<(), String> {
    match doc.cell_own_style(sheet_idx, cell)? {
        Some(style) => {
            let mut rs = render_style_from(&style, doc.workbook_theme());
            rs.num_fmt = cache.intern_num_fmt(&style.num_fmt);
            rs.font_size_q = font_size_q_of(style.font.sz, def_sz);
            rs.font_family = if style.font.name == def_name {
                0
            } else {
                cache.intern_font_family(&style.font.name)
            };
            rs.border =
                cache.intern_border_spec(border_spec_from(&style.border, doc.workbook_theme()));
            if rs != RenderStyle::default() {
                cache.set_cell_style(cell.row, cell.col, rs);
            } else if cache.is_on_band(cell.row, cell.col) {
                // Default own style shadowing a band → store an explicit default entry.
                cache.set_cell_style(cell.row, cell.col, RenderStyle::default());
            } else {
                cache.clear_cell_style(cell.row, cell.col);
            }
        }
        // Absent from the sheet data → the band/default resolution applies.
        None => cache.clear_cell_style(cell.row, cell.col),
    }
    Ok(())
}

/// The agreement contract's re-read helper (`components/style_cache.md §The agreement
/// contract`): asserts `cache` equals a **fresh engine re-read** over the probe grid
/// `rows_probe × cols_probe` — resolved `RenderStyle` (vs `get_style_for_cell`) and geometry (vs
/// the converted size getters). Returns `Err` describing the first divergence, so the negative
/// control can prove the check discriminates. Test-only (it re-reads IronCalc per probe cell).
#[cfg(test)]
pub(crate) fn assert_cache_agrees(
    doc: &WorkbookDocument,
    cache: &SheetCache,
    sheet_idx: u32,
    rows_probe: &[u32],
    cols_probe: &[u32],
) -> Result<(), String> {
    let (def_sz, def_name) = doc.default_font();
    for &r in rows_probe {
        for &c in cols_probe {
            let cached = cache.render_style(r, c).copied().unwrap_or_default();
            let engine_style = doc.resolved_cell_style(sheet_idx, CellRef::new(r, c))?;
            // Structural compare ignores the cache-local indices (`num_fmt`, `font_family`), but
            // resolves the *value* fields the engine side also carries (`font_size_q` is an absolute
            // quarter-point value, comparable directly once resolved against the workbook default).
            let mut engine = render_style_from(&engine_style, doc.workbook_theme());
            engine.font_size_q = font_size_q_of(engine_style.font.sz, def_sz);
            let cached_structural = RenderStyle {
                num_fmt: 0,
                font_family: 0,
                border: 0,
                ..cached
            };
            let engine_structural = RenderStyle {
                num_fmt: 0,
                font_family: 0,
                border: 0,
                ..engine
            };
            if cached_structural != engine_structural {
                return Err(format!(
                    "style mismatch at ({r},{c}): cache={cached:?} engine={engine:?}"
                ));
            }
            // The border agreement is over the resolved *spec* (the `border` field is a cache-local
            // index; the engine side resolves the spec directly).
            let cached_spec = cache.border_spec(cached.border);
            let engine_spec = border_spec_from(&engine_style.border, doc.workbook_theme());
            if cached_spec != engine_spec {
                return Err(format!(
                    "border mismatch at ({r},{c}): cache={cached_spec:?} engine={engine_spec:?}"
                ));
            }
            // The num-fmt agreement is over the resolved *string* (general normalized).
            let cached_code = cache.num_fmt_code(cached.num_fmt);
            let engine_general = engine_style.num_fmt.eq_ignore_ascii_case("general");
            let cached_general = cached_code.eq_ignore_ascii_case("general");
            let code_ok = if engine_general {
                cached_general
            } else {
                cached_code == engine_style.num_fmt
            };
            if !code_ok {
                return Err(format!(
                    "num_fmt mismatch at ({r},{c}): cache={cached_code:?} engine={:?}",
                    engine_style.num_fmt
                ));
            }
            // The font-family agreement is over the resolved *name* (default → "").
            let cached_family = cache.font_family_name(cached.font_family);
            let engine_family = if engine_style.font.name == def_name {
                ""
            } else {
                engine_style.font.name.as_str()
            };
            if cached_family != engine_family {
                return Err(format!(
                    "font_family mismatch at ({r},{c}): cache={cached_family:?} engine={engine_family:?}"
                ));
            }
        }
    }
    for &r in rows_probe {
        let cached = cache.row_height(r) as f64;
        let engine = row_px(doc.row_height_px(sheet_idx, r)?) as f64;
        if (cached - engine).abs() > 1e-3 {
            return Err(format!(
                "row {r} height mismatch: cache={cached} engine={engine}"
            ));
        }
    }
    for &c in cols_probe {
        let cached = cache.col_width(c) as f64;
        let engine = col_px(doc.col_width_px(sheet_idx, c)?) as f64;
        if (cached - engine).abs() > 1e-3 {
            return Err(format!(
                "col {c} width mismatch: cache={cached} engine={engine}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_core::{Align, CellRange, VAlign};

    // Colour resolution needs a workbook theme; these tests exercise explicit `#rgb` colours
    // (theme-independent), so the default theme suffices.
    fn rsf(style: &Style) -> RenderStyle {
        render_style_from(style, &Theme::default())
    }
    fn bsf(border: &Border) -> BorderSpec {
        border_spec_from(border, &Theme::default())
    }

    /// A tiny seeded LCG so the "random probe" set is deterministic without a `rand` dep.
    struct Lcg(u64);
    impl Lcg {
        fn next_in(&mut self, bound: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as u32) % bound
        }
    }

    fn style_with(path: &str, value: &str) -> Style {
        // Build an IronCalc Style by driving a real model edit, then reading it back — the exact
        // shape the app produces (so the conversion is tested against real engine output).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "x").unwrap();
        let area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        };
        doc.user_model_mut()
            .update_range_style(&area, path, value)
            .unwrap();
        doc.resolved_cell_style(0, CellRef::new(0, 0)).unwrap()
    }

    #[test]
    fn render_style_from_default_is_plain() {
        assert_eq!(rsf(&Style::default()), RenderStyle::default());
    }

    #[test]
    fn render_style_from_maps_each_attribute() {
        assert!(rsf(&style_with("font.b", "true")).bold);
        assert!(rsf(&style_with("font.i", "true")).italic);
        assert!(rsf(&style_with("font.u", "true")).underline);
        assert!(rsf(&style_with("font.strike", "true")).strikethrough);
        assert!(rsf(&style_with("alignment.wrap_text", "true")).wrap);
        assert_eq!(
            rsf(&style_with("alignment.vertical", "top")).v_align,
            Some(VAlign::Top)
        );
        assert_eq!(
            rsf(&style_with("alignment.vertical", "center")).v_align,
            Some(VAlign::Center)
        );
        assert_eq!(
            rsf(&style_with("alignment.vertical", "bottom")).v_align,
            Some(VAlign::Bottom)
        );
        // Justify is out of scope → treated as unset.
        assert_eq!(
            rsf(&style_with("alignment.vertical", "justify")).v_align,
            None
        );
        // Decision C: a cell that has an alignment record but only a *defaulted* vertical (here,
        // only `horizontal` was set → IronCalc fills `vertical = Bottom` by default) resolves to
        // `Some(Bottom)`. This guards the real engine mapping — the grid renders it bottom, exactly
        // like the `None` (no-record) default, so the two are coherent.
        assert_eq!(
            rsf(&style_with("alignment.horizontal", "center")).v_align,
            Some(VAlign::Bottom)
        );
        // A cell with no alignment record has no vertical alignment and no wrap.
        assert_eq!(rsf(&Style::default()).v_align, None);
        assert!(!rsf(&Style::default()).wrap);
        assert!(!rsf(&Style::default()).strikethrough);
        assert_eq!(
            rsf(&style_with("fill.fg_color", "#FF0000")).fill,
            Some(Rgb::from_hex(0xFF0000))
        );
        assert_eq!(
            rsf(&style_with("font.color", "#0000FF")).font_color,
            Some(Rgb::from_hex(0x0000FF))
        );
        // Explicit black font colour maps to None (the grid's default near-black).
        assert_eq!(rsf(&style_with("font.color", "#000000")).font_color, None);
        assert_eq!(
            rsf(&style_with("alignment.horizontal", "right")).h_align,
            Some(Align::Right)
        );
        assert_eq!(
            rsf(&style_with("alignment.horizontal", "center")).h_align,
            Some(Align::Center)
        );
        // `render_style_from` never resolves the num-fmt index itself (the caller does, via the
        // interning table); it always carries the default 0.
        assert_eq!(rsf(&style_with("num_fmt", "0.00%")).num_fmt, 0);
        assert_eq!(rsf(&Style::default()).num_fmt, 0);
    }

    #[test]
    fn build_carries_num_fmt_from_file() {
        // A cell whose ONLY styling is a custom number format is still stored (num_fmt != 0), and
        // its resolved string round-trips through the side table; a plain cell stays default.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        let area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        };
        doc.user_model_mut()
            .update_range_style(&area, "num_fmt", "$#,##0.00")
            .unwrap();

        let cache = build_sheet_cache(&doc, 0).unwrap();
        let formatted = cache
            .render_style(0, 0)
            .expect("format-only cell is stored");
        assert_ne!(formatted.num_fmt, 0);
        assert_eq!(cache.num_fmt_code(formatted.num_fmt), "$#,##0.00");
        // The plain number cell resolves to the default (general) — no stored style, index 0.
        assert!(cache.render_style(1, 0).is_none());
    }

    #[test]
    fn default_font_detects_workbook_default() {
        // A fresh workbook's default font (13pt Calibri) resolves cells to font_size_q/font_family
        // 0 — so a default-font cell interns to the default style and renders at the grid default.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "plain").unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        assert!(
            cache.render_style(0, 0).is_none(),
            "a default-font cell is not stored (interns to default)"
        );
    }

    #[test]
    fn opened_file_nondefault_default_font_interns_to_sentinel() {
        // Simulate an OPENED file whose default (style 0) font is Arial 10 — NOT new_empty's
        // Calibri 13. This substantiates "no opened-file regression": unstyled cells still intern
        // to the sentinel (render unchanged) and the cache reports the file's default size.
        let mut model = ironcalc_base::Model::new_empty("t", "en", "UTC", "en").unwrap();
        model.workbook.styles.fonts[0].name = "Arial".to_string();
        model.workbook.styles.fonts[0].sz = 10;
        let mut doc = WorkbookDocument::from_test_model(model);
        assert_eq!(doc.default_font(), (10, "Arial".to_string()));

        doc.set_cell_input(0, CellRef::new(0, 0), "x").unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        assert!(
            cache.render_style(0, 0).is_none(),
            "a cell inheriting the file default font interns to the sentinel (renders unchanged)"
        );
        assert_eq!(
            cache.default_font_size_q(),
            40,
            "10pt → 40 quarter-points (the action-bar default label)"
        );

        // A cell explicitly set to a DIFFERENT font (Calibri 13) is stored non-default.
        let (_, def_name) = doc.default_font();
        doc.set_cell_input(0, CellRef::new(1, 0), "y").unwrap();
        doc.set_font(
            0,
            CellRange::single(CellRef::new(1, 0)),
            Some("Calibri"),
            Some(13.0),
            &def_name,
        )
        .unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let rs = cache
            .render_style(1, 0)
            .expect("a font differing from the file default is stored");
        assert_eq!(cache.font_family_name(rs.font_family), "Calibri");
        assert_eq!(rs.font_size_q, 13 * 4);
    }

    #[test]
    fn build_carries_font_from_file() {
        // A cell with a non-default family + size resolves to a non-zero font_family/font_size_q
        // whose name round-trips through the side table; a plain cell stays 0/0 (unstored).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = CellRef::new(0, 0);
        doc.set_cell_input(0, a1, "styled").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "plain").unwrap();
        let (_, def_name) = doc.default_font();
        doc.set_font(
            0,
            CellRange::single(a1),
            Some("Times New Roman"),
            Some(24.0),
            &def_name,
        )
        .unwrap();

        let cache = build_sheet_cache(&doc, 0).unwrap();
        let rs = cache.render_style(0, 0).expect("font-only cell is stored");
        assert_eq!(rs.font_size_q, 24 * 4, "24pt → 96 quarter-points");
        assert_ne!(rs.font_family, 0);
        assert_eq!(cache.font_family_name(rs.font_family), "Times New Roman");
        // The plain cell resolves to the default (no stored style).
        assert!(cache.render_style(1, 0).is_none());
    }

    #[test]
    fn band_font_resolves_into_cells() {
        // A whole-column band font-size (an in-engine col band via the full-height fast path)
        // resolves into the RenderStyle of every cell in that column — the band path shares the
        // build loop's `font_size_q_of` resolution.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // `font.size_delta` on a full column creates a col band whose font size is default + delta.
        let col_area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 4, // 0-based column 3
            width: 1,
            height: limits::MAX_ROWS as i32,
        };
        doc.user_model_mut()
            .update_range_style(&col_area, "font.size_delta", "10")
            .unwrap();
        let (def_sz, _) = doc.default_font();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        // A cell in the band (no own style) resolves to the band's font size.
        let rs = cache
            .render_style(20, 3)
            .expect("a col-band cell resolves to the band style");
        assert_eq!(rs.font_size_q, font_size_q_of(def_sz + 10, def_sz));
        assert_ne!(rs.font_size_q, 0, "the band's larger size is non-default");
    }

    #[test]
    fn parse_color_goldens() {
        assert_eq!(parse_color("#FF0000"), Some(Rgb::new(0xFF, 0, 0)));
        assert_eq!(parse_color("#00FF00"), Some(Rgb::new(0, 0xFF, 0)));
        // 8-digit ARGB: alpha byte dropped.
        assert_eq!(parse_color("#FF123456"), Some(Rgb::from_hex(0x123456)));
        // Junk → None (never panics), including non-ASCII bytes that could otherwise trip the
        // `[2..]` slice on a char boundary.
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#GGGGGG"), None);
        assert_eq!(parse_color("#12é45678"), None);
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn border_weight_mapping_all_nine_styles() {
        use ironcalc_base::types::BorderStyle::*;
        // The actual nine 0.7.1 variants → px weight class (`architecture.md §1.1`, corrected).
        assert_eq!(border_weight(&Thin), 1);
        assert_eq!(border_weight(&Dotted), 1);
        assert_eq!(border_weight(&Medium), 2);
        assert_eq!(border_weight(&MediumDashed), 2);
        assert_eq!(border_weight(&MediumDashDot), 2);
        assert_eq!(border_weight(&MediumDashDotDot), 2);
        assert_eq!(border_weight(&SlantDashDot), 2);
        assert_eq!(border_weight(&Thick), 3);
        assert_eq!(border_weight(&Double), 3);
    }

    #[test]
    fn border_pattern_mapping_all_nine_styles() {
        use ironcalc_base::types::BorderStyle::*;
        // Only MediumDashed → Dashed and Double → Double; every other style (incl. the deferred
        // Dotted / dash-dot / SlantDashDot families) falls back to Solid — unchanged from before.
        assert_eq!(border_pattern(&MediumDashed), LinePattern::Dashed);
        assert_eq!(border_pattern(&Double), LinePattern::Double);
        assert_eq!(border_pattern(&Thin), LinePattern::Solid);
        assert_eq!(border_pattern(&Medium), LinePattern::Solid);
        assert_eq!(border_pattern(&Thick), LinePattern::Solid);
        assert_eq!(border_pattern(&Dotted), LinePattern::Solid);
        assert_eq!(border_pattern(&MediumDashDot), LinePattern::Solid);
        assert_eq!(border_pattern(&MediumDashDotDot), LinePattern::Solid);
        assert_eq!(border_pattern(&SlantDashDot), LinePattern::Solid);
    }

    #[test]
    fn border_spec_from_reads_all_four_edges_and_colour() {
        use freecell_core::Edge;
        use ironcalc_base::types::{Border, BorderItem, BorderStyle};
        let item = |style, color: &str| {
            Some(BorderItem {
                style,
                color: Color::Rgb(color.to_string()),
            })
        };
        let border = Border {
            top: item(BorderStyle::Thin, "#000000"),
            right: item(BorderStyle::Thick, "#FF0000"),
            bottom: item(BorderStyle::MediumDashed, "#00FF00"),
            left: item(BorderStyle::Double, "#0000FF"),
            ..Border::default()
        };
        let spec = bsf(&border);
        // Solid edges (thin/thick) keep the default Solid pattern …
        assert_eq!(spec.top, Some(Edge::new(1, Rgb::new(0, 0, 0))));
        assert_eq!(spec.right, Some(Edge::new(3, Rgb::new(0xFF, 0, 0))));
        // … while MediumDashed → Dashed (weight 2) and Double → Double (weight 3) carry their pattern.
        assert_eq!(
            spec.bottom,
            Some(Edge::with_pattern(
                2,
                Rgb::new(0, 0xFF, 0),
                LinePattern::Dashed
            ))
        );
        assert_eq!(
            spec.left,
            Some(Edge::with_pattern(
                3,
                Rgb::new(0, 0, 0xFF),
                LinePattern::Double
            ))
        );
        // A colourless item defaults to black (never panics).
        let b2 = Border {
            top: Some(BorderItem {
                style: BorderStyle::Thin,
                color: Color::None,
            }),
            ..Border::default()
        };
        assert_eq!(bsf(&b2).top, Some(Edge::new(1, Rgb::new(0, 0, 0))));
    }

    #[test]
    fn cache_carries_border_from_file() {
        use freecell_core::{BorderSpec, Edge};
        // A cell whose ONLY styling is an All-thin border is stored (border != 0), and its resolved
        // BorderSpec round-trips through the side table; a plain neighbour stays default.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "x").unwrap();
        doc.set_cell_input(0, CellRef::new(5, 5), "y").unwrap();
        doc.set_borders(
            0,
            CellRange::single(CellRef::new(0, 0)),
            "All",
            "thin",
            "#000000",
        )
        .unwrap();

        let cache = build_sheet_cache(&doc, 0).unwrap();
        let rs = cache
            .render_style(0, 0)
            .expect("a border-only cell is stored");
        assert_ne!(rs.border, 0);
        let thin = Some(Edge::new(1, Rgb::new(0, 0, 0)));
        assert_eq!(
            cache.border_spec(rs.border),
            BorderSpec {
                top: thin,
                right: thin,
                bottom: thin,
                left: thin,
            }
        );
        // The plain far cell has no border (unstored).
        assert!(cache.render_style(5, 5).is_none());
    }

    #[test]
    fn unit_conversion_goldens() {
        // IronCalc default px → FreeCell default px (exactly, within f32 tolerance).
        assert!((col_px(IRONCALC_DEFAULT_COL_WIDTH_PX) - DEFAULT_COL_WIDTH_PX).abs() < 1e-3);
        assert!((row_px(IRONCALC_DEFAULT_ROW_HEIGHT_PX) - DEFAULT_ROW_HEIGHT_PX).abs() < 1e-3);
        // 2× the IronCalc default → 2× the FreeCell default (expressed via the constants so this
        // stays correct if the pinned engine's defaults change again).
        assert!(
            (col_px(2.0 * IRONCALC_DEFAULT_COL_WIDTH_PX) - 2.0 * DEFAULT_COL_WIDTH_PX).abs() < 1e-3
        );
        assert!(
            (row_px(2.0 * IRONCALC_DEFAULT_ROW_HEIGHT_PX) - 2.0 * DEFAULT_ROW_HEIGHT_PX).abs()
                < 1e-3
        );
    }

    /// A styled reference workbook (mirrors round-3 A's harness sheet): per-cell attributes, a
    /// row band, a column band, a custom row height + column width, and cross cells.
    fn reference_doc() -> WorkbookDocument {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Per-cell styles (A1 bold, B1 italic+red fill, C3 blue font).
        for (cell, path, val) in [
            (CellRef::new(0, 0), "font.b", "true"),
            (CellRef::new(0, 1), "font.i", "true"),
            (CellRef::new(0, 1), "fill.fg_color", "#FFFF00"),
            (CellRef::new(2, 2), "font.color", "#0000FF"),
        ] {
            doc.set_cell_input(0, cell, "v").unwrap();
            let area = ironcalc_base::expressions::types::Area {
                sheet: 0,
                row: cell.row as i32 + 1,
                column: cell.col as i32 + 1,
                width: 1,
                height: 1,
            };
            doc.user_model_mut()
                .update_range_style(&area, path, val)
                .unwrap();
        }
        // Row band on row 5 (0-based) + a value on a cell of that row (shadow case).
        doc.user_model_mut().set_rows_height(0, 6, 6, 40.0).unwrap();
        let row_area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 6,
            column: 1,
            width: limits::MAX_COLS as i32,
            height: 1,
        };
        doc.user_model_mut()
            .update_range_style(&row_area, "fill.fg_color", "#DDDDDD")
            .unwrap();
        doc.set_cell_input(0, CellRef::new(5, 0), "on-band")
            .unwrap();
        // Column band on col 7 (0-based) + a custom width there.
        doc.user_model_mut()
            .set_columns_width(0, 8, 8, 220.0)
            .unwrap();
        let col_area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 8,
            width: 1,
            height: limits::MAX_ROWS as i32,
        };
        doc.user_model_mut()
            .update_range_style(&col_area, "font.b", "true")
            .unwrap();
        doc
    }

    /// A probe set spanning the styled region + empties + seeded-random cells.
    fn probes() -> (Vec<u32>, Vec<u32>) {
        let mut rng = Lcg(0x5EED);
        let mut rows: Vec<u32> = (0..12).collect();
        let mut cols: Vec<u32> = (0..12).collect();
        for _ in 0..12 {
            rows.push(rng.next_in(50));
            cols.push(rng.next_in(50));
        }
        (rows, cols)
    }

    #[test]
    fn build_matches_engine_styled_fixture() {
        let doc = reference_doc();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let (rows, cols) = probes();
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();
    }

    #[test]
    fn build_matches_engine_empty() {
        let doc = WorkbookDocument::new_empty().unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let rows: Vec<u32> = (0..8).collect();
        assert_cache_agrees(&doc, &cache, 0, &rows, &rows).unwrap();
    }

    #[test]
    fn build_matches_engine_band_only() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A row band with no per-cell styles anywhere.
        let row_area = ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 3,
            column: 1,
            width: limits::MAX_COLS as i32,
            height: 1,
        };
        doc.user_model_mut()
            .update_range_style(&row_area, "fill.fg_color", "#EEEEEE")
            .unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let rows: Vec<u32> = (0..8).collect();
        let cols: Vec<u32> = (0..8).collect();
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();
    }

    #[test]
    fn excel_max_geometry_totals_match_engine() {
        // A 1M-row default sheet: the axis total equals rows × the converted engine default.
        let doc = WorkbookDocument::new_empty().unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let expected = limits::MAX_ROWS as f64 * row_px(IRONCALC_DEFAULT_ROW_HEIGHT_PX) as f64;
        assert!((cache.total_height() - expected).abs() < 1.0);
    }

    #[test]
    fn interner_dedups() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Three cells bold (share one StyleId), one italic (distinct).
        for cell in [CellRef::new(0, 0), CellRef::new(1, 1), CellRef::new(2, 2)] {
            doc.set_cell_input(0, cell, "b").unwrap();
            doc.set_font_flag(
                0,
                CellRange::single(cell),
                crate::document::FontFlag::Bold,
                true,
            )
            .unwrap();
        }
        doc.set_cell_input(0, CellRef::new(3, 3), "i").unwrap();
        doc.set_font_flag(
            0,
            CellRange::single(CellRef::new(3, 3)),
            crate::document::FontFlag::Italic,
            true,
        )
        .unwrap();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        assert_eq!(cache.render_style(0, 0), cache.render_style(1, 1));
        assert_ne!(cache.render_style(0, 0), cache.render_style(3, 3));
    }

    /// Apply one style edit to the model AND mirror it into the cache (mimicking the worker).
    fn set_bold(doc: &mut WorkbookDocument, cache: &mut SheetCache, range: CellRange) {
        doc.set_font_flag(0, range, crate::document::FontFlag::Bold, true)
            .unwrap();
        let (def_sz, def_name) = doc.default_font();
        for row in range.rows() {
            for col in range.cols() {
                refresh_cell(cache, doc, 0, CellRef::new(row, col), def_sz, &def_name).unwrap();
            }
        }
    }

    /// Refresh a single cell's cache entry, resolving the workbook default font inline (test-only).
    fn refresh_one(cache: &mut SheetCache, doc: &WorkbookDocument, cell: CellRef) {
        let (def_sz, def_name) = doc.default_font();
        refresh_cell(cache, doc, 0, cell, def_sz, &def_name).unwrap();
    }

    #[test]
    fn mirror_set_style_each_attr_agrees() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let mut cache = build_sheet_cache(&doc, 0).unwrap();
        let (rows, cols) = probes();

        // Single cell, then a multi-cell range, then a range overlapping a col band.
        set_bold(&mut doc, &mut cache, CellRange::single(CellRef::new(1, 1)));
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();

        set_bold(
            &mut doc,
            &mut cache,
            CellRange::new(CellRef::new(2, 0), CellRef::new(4, 3)),
        );
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();

        // Fill then no-fill on a single cell.
        doc.set_fill(
            0,
            CellRange::single(CellRef::new(6, 6)),
            Some(Rgb::from_hex(0x00FF00)),
        )
        .unwrap();
        refresh_one(&mut cache, &doc, CellRef::new(6, 6));
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();
        doc.set_fill(0, CellRange::single(CellRef::new(6, 6)), None)
            .unwrap();
        refresh_one(&mut cache, &doc, CellRef::new(6, 6));
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();
    }

    #[test]
    fn negative_control_skipping_a_mirror_diverges() {
        // The contract must have discriminating power: apply an edit to the engine but DON'T
        // mirror it into the cache, and the agreement helper must FAIL.
        let mut doc = reference_doc();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let (rows, cols) = probes();
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap(); // agrees before

        // Engine changes, cache does not.
        doc.set_font_flag(
            0,
            CellRange::single(CellRef::new(1, 1)),
            crate::document::FontFlag::Bold,
            true,
        )
        .unwrap();
        assert!(
            assert_cache_agrees(&doc, &cache, 0, &rows, &cols).is_err(),
            "a skipped mirror must be caught by the agreement contract"
        );
    }

    #[test]
    fn perf_smoke_viewport_lookup_is_not_o_sheet_size() {
        // Guards against an accidental O(sheet-size) style/offset lookup — a real regression
        // (e.g. scanning all 1M rows) would take *seconds*, blowing past this generous bound;
        // the exact per-frame budget is the macOS perf harness (Phase 12). The axes span the
        // full Excel-max sheet, so a passing sweep proves the lookups are sub-linear in it.
        let doc = reference_doc();
        let cache = build_sheet_cache(&doc, 0).unwrap();
        let (row_axis, col_axis) = cache.axes();
        // Precompute per-track offsets once (as the real grid does), then resolve the 2k cells.
        let start = std::time::Instant::now();
        let row_offsets: Vec<f64> = (0..40u32).map(|r| row_axis.offset_of(r)).collect();
        let col_offsets: Vec<f64> = (0..50u32).map(|c| col_axis.offset_of(c)).collect();
        let mut acc = 0.0f64;
        for r in 0..40u32 {
            for c in 0..50u32 {
                let _ = cache.render_style(r, c);
                acc += row_offsets[r as usize] + col_offsets[c as usize];
            }
        }
        let elapsed = start.elapsed();
        assert!(acc >= 0.0); // force the work
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "2k-cell viewport lookup took {elapsed:?} — suspiciously slow (possible O(n) lookup)"
        );
    }
}
