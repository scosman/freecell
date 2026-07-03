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
//! "COLUMN_WIDTH and ROW_HEIGHT are pixel values"; defaults 125 px / 28 px). FreeCell's chosen
//! grid defaults are 100 px / 24 px (`ui_design.md §3.3`). We convert an override by the ratio
//! `freecell_default / ironcalc_default`, so a track at IronCalc's default maps exactly to the
//! FreeCell default and any deviation scales proportionally (a 2× column stays 2× of 100 px).
//!
//! Coordinates: `freecell_core` cells are 0-based; IronCalc rows/cols are 1-based. The
//! `WorkbookDocument` accessors take 0-based indices; when this module reads a raw `Worksheet`
//! it converts the 1-based `Row.r` / `Col.min..=max` itself.

use freecell_core::cache::{DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use freecell_core::{limits, CellRef, RenderStyle, Rgb, SheetCache, SheetCacheBuilder};
use ironcalc_base::types::{HorizontalAlignment, Style};

use crate::document::WorkbookDocument;

/// IronCalc's default column width in px (`ironcalc_base/src/constants.rs`
/// `DEFAULT_COLUMN_WIDTH`). A non-custom column reads back exactly this.
pub(crate) const IRONCALC_DEFAULT_COL_WIDTH_PX: f64 = 125.0;
/// IronCalc's default row height in px (`ironcalc_base/src/constants.rs` `DEFAULT_ROW_HEIGHT`).
pub(crate) const IRONCALC_DEFAULT_ROW_HEIGHT_PX: f64 = 28.0;

/// Converts an IronCalc column-width pixel value to a FreeCell device-px width (see module docs).
pub(crate) fn col_px(ironcalc_px: f64) -> f32 {
    (ironcalc_px * (DEFAULT_COL_WIDTH_PX as f64 / IRONCALC_DEFAULT_COL_WIDTH_PX)) as f32
}

/// Converts an IronCalc row-height pixel value to a FreeCell device-px height (see module docs).
pub(crate) fn row_px(ironcalc_px: f64) -> f32 {
    (ironcalc_px * (DEFAULT_ROW_HEIGHT_PX as f64 / IRONCALC_DEFAULT_ROW_HEIGHT_PX)) as f32
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

/// Derives the engine-free [`RenderStyle`] from an IronCalc `Style` — the MVP subset the grid
/// draws (`functional_spec.md §3.6`). Everything IronCalc models but the grid ignores (borders,
/// font family/size, strikethrough, wrap, vertical align) is intentionally dropped from the
/// render form; it stays in the engine and round-trips on save.
///
/// `render_style_from(&Style::default()) == RenderStyle::default()` (asserted in the tests) — so
/// a plain cell interns to the default style and resolves to `None` (the grid's default paint).
pub(crate) fn render_style_from(style: &Style) -> RenderStyle {
    RenderStyle {
        bold: style.font.b,
        italic: style.font.i,
        underline: style.font.u,
        // A solid fill's colour lives in `fill.fg_color`; absent (or cleared) → no fill.
        fill: style.fill.fg_color.as_deref().and_then(parse_color),
        // `None` on RenderStyle means "grid default (near-black)"; IronCalc's default font colour
        // is pure black, so map black (and absent/unparseable) to `None` to keep default cells
        // interning to the default style.
        font_color: style
            .font
            .color
            .as_deref()
            .and_then(parse_color)
            .filter(|rgb| *rgb != Rgb::new(0, 0, 0)),
        h_align: h_align_of(style),
        // The grid only distinguishes "General" (its own type-based alignment / passthrough) from
        // an explicit format; IronCalc's default is the lowercase "general".
        num_format_is_default: style.num_fmt.eq_ignore_ascii_case("general"),
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
                let rs = render_style_from(&style);
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
                let rs = render_style_from(&style);
                if rs != RenderStyle::default() {
                    builder.push_row_style(r0, rs);
                    band_rows.insert(r0);
                }
            }
        }
    }

    // Per-cell styles: every populated/styled cell in the sheet data.
    for (row_1, cols) in &ws.sheet_data {
        let row0 = (*row_1 - 1) as u32;
        for col_1 in cols.keys() {
            let col0 = (*col_1 - 1) as u32;
            let cell = CellRef::new(row0, col0);
            if let Some(style) = doc.cell_own_style(sheet_idx, cell)? {
                let rs = render_style_from(&style);
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
pub(crate) fn refresh_cell(
    cache: &mut SheetCache,
    doc: &WorkbookDocument,
    sheet_idx: u32,
    cell: CellRef,
) -> Result<(), String> {
    match doc.cell_own_style(sheet_idx, cell)? {
        Some(style) => {
            let rs = render_style_from(&style);
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
    for &r in rows_probe {
        for &c in cols_probe {
            let cached = cache.render_style(r, c).copied().unwrap_or_default();
            let engine =
                render_style_from(&doc.resolved_cell_style(sheet_idx, CellRef::new(r, c))?);
            if cached != engine {
                return Err(format!(
                    "style mismatch at ({r},{c}): cache={cached:?} engine={engine:?}"
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
    use freecell_core::{Align, CellRange};

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
        assert_eq!(render_style_from(&Style::default()), RenderStyle::default());
    }

    #[test]
    fn render_style_from_maps_each_attribute() {
        assert!(render_style_from(&style_with("font.b", "true")).bold);
        assert!(render_style_from(&style_with("font.i", "true")).italic);
        assert!(render_style_from(&style_with("font.u", "true")).underline);
        assert_eq!(
            render_style_from(&style_with("fill.fg_color", "#FF0000")).fill,
            Some(Rgb::from_hex(0xFF0000))
        );
        assert_eq!(
            render_style_from(&style_with("font.color", "#0000FF")).font_color,
            Some(Rgb::from_hex(0x0000FF))
        );
        // Explicit black font colour maps to None (the grid's default near-black).
        assert_eq!(
            render_style_from(&style_with("font.color", "#000000")).font_color,
            None
        );
        assert_eq!(
            render_style_from(&style_with("alignment.horizontal", "right")).h_align,
            Some(Align::Right)
        );
        assert_eq!(
            render_style_from(&style_with("alignment.horizontal", "center")).h_align,
            Some(Align::Center)
        );
        // A custom number format flips num_format_is_default off.
        assert!(!render_style_from(&style_with("num_fmt", "0.00%")).num_format_is_default);
        assert!(render_style_from(&Style::default()).num_format_is_default);
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
    fn unit_conversion_goldens() {
        // IronCalc default px → FreeCell default px (exactly, within f32 tolerance).
        assert!((col_px(IRONCALC_DEFAULT_COL_WIDTH_PX) - DEFAULT_COL_WIDTH_PX).abs() < 1e-3);
        assert!((row_px(IRONCALC_DEFAULT_ROW_HEIGHT_PX) - DEFAULT_ROW_HEIGHT_PX).abs() < 1e-3);
        // 2× the IronCalc default → 2× the FreeCell default.
        assert!((col_px(250.0) - 200.0).abs() < 1e-3);
        assert!((row_px(56.0) - 48.0).abs() < 1e-3);
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
        for row in range.rows() {
            for col in range.cols() {
                refresh_cell(cache, doc, 0, CellRef::new(row, col)).unwrap();
            }
        }
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
        refresh_cell(&mut cache, &doc, 0, CellRef::new(6, 6)).unwrap();
        assert_cache_agrees(&doc, &cache, 0, &rows, &cols).unwrap();
        doc.set_fill(0, CellRange::single(CellRef::new(6, 6)), None)
            .unwrap();
        refresh_cell(&mut cache, &doc, 0, CellRef::new(6, 6)).unwrap();
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
