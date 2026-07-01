//! # styled_read — SP4 value+style viewport read core
//!
//! Extends the frozen `round-2/harness` viewport read to fetch **value AND style** per
//! visible cell, so we can gate the styled read against the 2 ms viewport budget and
//! compare it to Phase-1's value-only baseline (392 µs p99). The harness stays untouched:
//! its `IronCalcEngine::read_viewport` reads values only, so this crate adds the style
//! half by calling the adapter's `.model() -> &Model` and IronCalc's `get_style_for_cell`
//! (the same public read the real FreeCell UI would use).
//!
//! **Build vs read split.** Style *setters* (`set_cell_style`, `set_row_style`,
//! `set_column_style`) live only on `&mut Model`, and the frozen adapter exposes just
//! `&Model`. Rather than edit the frozen crate or reach for `unsafe`, SP4 owns the raw
//! [`Model`] while building the styled sheet (full mutable access), then wraps the
//! finished model with [`IronCalcEngine::from_model`] for the read half (which only needs
//! `&self`). Value writes go through the raw model too, so one builder produces the
//! benchmark fixture; the wrapped engine is read-only, exactly like the UI's read path.
//!
//! It also holds the **style-API coverage** helpers the `probe` bin asserts against:
//! per-cell styles, row/column **band** styles, and **empty-cell** styling — verified
//! against IronCalc 0.7.1's real public API, not assumed.
//!
//! All coordinates in this crate's public helpers are **0-based** (datagen / harness
//! space); IronCalc is 1-based, so every `Model` call adds `+1` (matching the adapter).

use ironcalc_base::types::{
    Alignment, BorderItem, BorderStyle, HorizontalAlignment, Style, VerticalAlignment,
};
use ironcalc_base::Model;
use round2_harness::engine::SpreadsheetEngine;
use round2_harness::{EngineValue, IronCalcEngine, Viewport};

/// The single sheet all SP4 work uses (index 0).
pub const SHEET: u32 = 0;

/// Excel's maximum row count (1,048,576) — IronCalc's `LAST_ROW`. Last valid **0-based**
/// row is [`EXCEL_MAX_ROW_0`].
pub const EXCEL_ROWS: u32 = 1_048_576;
/// Excel's maximum column count (16,384) — IronCalc's `LAST_COLUMN`. Last valid
/// **0-based** column is [`EXCEL_MAX_COL_0`].
pub const EXCEL_COLS: u32 = 16_384;
/// The last addressable **0-based** row: read here → IronCalc row `1_048_576` = `LAST_ROW`.
pub const EXCEL_MAX_ROW_0: u32 = EXCEL_ROWS - 1;
/// The last addressable **0-based** column: read here → IronCalc column `16_384`.
pub const EXCEL_MAX_COL_0: u32 = EXCEL_COLS - 1;

/// The IronCalc default font size (points). A cell whose resolved size differs from this
/// (or is bold/filled/number-formatted) carries a real, non-default style.
const DEFAULT_FONT_SZ: i32 = 13;

/// A compact projection of what FreeCell's grid actually reads per visible cell: the
/// **value** plus a few load-bearing **style** fields.
///
/// We project a handful of fields rather than cloning the whole [`Style`]: the projection
/// still forces IronCalc to resolve the effective style (the real per-cell work —
/// cell → row band → column band → default), while keeping the returned struct small so
/// the timed cost is the *read*, not a giant clone that would flatter the number.
#[derive(Debug, Clone, PartialEq)]
pub struct StyledCell {
    /// The cell's value (same neutral type the harness value read produces).
    pub value: EngineValue,
    /// Bold (`Style.font.b`).
    pub bold: bool,
    /// Italic (`Style.font.i`).
    pub italic: bool,
    /// Font size in points (`Style.font.sz`).
    pub font_size: i32,
    /// Fill foreground colour (`Style.fill.fg_color`), if any — the common "highlight".
    pub fill_argb: Option<String>,
    /// Number-format code (`Style.num_fmt`) — drives how the value is displayed.
    pub num_fmt: String,
}

impl StyledCell {
    /// `true` if this cell carries any **non-default** styling — used by the credibility
    /// guard so the benchmark can prove it read real styles, not a blank grid.
    pub fn is_styled(&self) -> bool {
        self.bold
            || self.italic
            || self.font_size != DEFAULT_FONT_SZ
            || self.fill_argb.is_some()
            || (self.num_fmt != "general" && !self.num_fmt.is_empty())
    }
}

/// Reads **value + effective style** for every cell of `vp`, in row-major order — the
/// SP4 measured operation.
///
/// Per cell it does exactly what the FreeCell binding would: one value read
/// (`get_value`) and one style resolution (`get_style_for_cell`). IronCalc has no native
/// bulk style read (mirrors the value side — no native range read either), so this is a
/// per-cell loop: the honest shape of the styled read.
pub fn read_styled_viewport(engine: &IronCalcEngine, vp: Viewport) -> Vec<StyledCell> {
    let model = engine.model();
    vp.addresses()
        .map(|(r, c)| {
            let value = engine.get_value(r, c);
            let style = model
                .get_style_for_cell(SHEET, (r + 1) as i32, (c + 1) as i32)
                .unwrap_or_default();
            project(value, &style)
        })
        .collect()
}

/// Projects a value + resolved [`Style`] into a compact [`StyledCell`].
fn project(value: EngineValue, style: &Style) -> StyledCell {
    StyledCell {
        value,
        bold: style.font.b,
        italic: style.font.i,
        font_size: style.font.sz,
        fill_argb: style.fill.fg_color.clone(),
        num_fmt: style.num_fmt.clone(),
    }
}

/// Credibility guard: counts `(non_empty_values, styled_cells)` in a styled read so the
/// benchmark can assert **both > 0** and refuse to record a number for an empty/unstyled
/// grid (we must not be "fast" by reading nothing).
pub fn count_real(cells: &[StyledCell]) -> (usize, usize) {
    let non_empty = cells
        .iter()
        .filter(|c| !matches!(c.value, EngineValue::Empty))
        .count();
    let styled = cells.iter().filter(|c| c.is_styled()).count();
    (non_empty, styled)
}

/// Builds a **non-default** [`Style`]. Distinct `tag`s produce visibly different fills so
/// a resolved read can be attributed to the right source (cell vs row band vs column
/// band).
///
/// Non-default matters twice: it exercises the full [`Style`] shape FreeCell needs, and
/// IronCalc's row-band setter only marks a row resolvable when its style index is
/// non-default (`custom_format`), so a default style would silently not apply as a band.
pub fn styled_variant(tag: u8) -> Style {
    let mut s = Style::default();
    s.font.b = true;
    s.font.sz = 14 + tag as i32;
    s.fill.pattern_type = "solid".to_string();
    s.fill.fg_color = Some(format!("#{:02X}FF00", tag.wrapping_mul(16)));
    s.num_fmt = "0.00".to_string();
    s.alignment = Some(Alignment {
        horizontal: HorizontalAlignment::Right,
        vertical: VerticalAlignment::default(),
        wrap_text: false,
    });
    s.border.left = Some(BorderItem {
        style: BorderStyle::Thin,
        color: Some("#000000".to_string()),
    });
    s
}

/// A fresh, empty single-sheet IronCalc [`Model`] the SP4 builder mutates directly.
pub fn new_model() -> Model<'static> {
    Model::new_empty("styled_read", "en", "UTC", "en").expect("ironcalc new_empty")
}

/// Stamps a `rows × cols` band starting at 0-based `(row0, col0)` on a **raw model**
/// (full mutable access) with a MIX of styles: a **column band** on every column, a
/// **row band** on every row, and a **per-cell** style + value on a scattered subset — so
/// a styled read at that position exercises IronCalc's real cell → row → column → default
/// resolution fallthrough, not one trivial lookup.
///
/// Because IronCalc storage is a sparse `HashMap`, the caller places this band **at
/// Excel-max positions** and the read benchmark scrolls within it — the maximal
/// coordinate is what SP4 must measure.
pub fn stamp_styled_band(model: &mut Model<'static>, row0: u32, col0: u32, rows: u32, cols: u32) {
    // Column band across the visible columns.
    for c in col0..col0 + cols {
        model
            .set_column_style(SHEET, (c + 1) as i32, &styled_variant(1))
            .expect("set_column_style");
    }
    // Row band across the visible rows (non-default → resolvable).
    for r in row0..row0 + rows {
        model
            .set_row_style(SHEET, (r + 1) as i32, &styled_variant(2))
            .expect("set_row_style");
    }
    // Scattered per-cell styles + values on ~1/4 of the cells, so the read sees a mix of
    // per-cell, row-band, and column-band resolution AND real non-empty values.
    let mut n = 0u64;
    for r in row0..row0 + rows {
        for c in col0..col0 + cols {
            if (n & 3) == 0 {
                model
                    .set_cell_style(SHEET, (r + 1) as i32, (c + 1) as i32, &styled_variant(3))
                    .expect("set_cell_style");
                model
                    .set_user_input(SHEET, (r + 1) as i32, (c + 1) as i32, format!("{n}"))
                    .expect("set_user_input");
            }
            n += 1;
        }
    }
}

/// Reads the effective [`Style`] for a 0-based cell straight from the engine's model
/// (probe/test helper).
pub fn effective_style(engine: &IronCalcEngine, row: u32, col: u32) -> Style {
    engine
        .model()
        .get_style_for_cell(SHEET, (row + 1) as i32, (col + 1) as i32)
        .unwrap_or_default()
}

/// Reads the **row-band** style (if any) for a 0-based row via IronCalc's public
/// `get_row_style` (probe helper).
pub fn get_row_style(engine: &IronCalcEngine, row: u32) -> Option<Style> {
    engine
        .model()
        .get_row_style(SHEET, (row + 1) as i32)
        .unwrap_or(None)
}

/// Reads the **column-band** style (if any) for a 0-based column via IronCalc's public
/// `get_column_style` (probe helper).
pub fn get_column_style(engine: &IronCalcEngine, col: u32) -> Option<Style> {
    engine
        .model()
        .get_column_style(SHEET, (col + 1) as i32)
        .unwrap_or(None)
}

/// Whether a 0-based cell currently has **no value** (empty) — used to prove empty-cell
/// styling: a valueless cell that still resolves a band/cell style.
pub fn is_value_empty(engine: &IronCalcEngine, row: u32, col: u32) -> bool {
    matches!(engine.get_value(row, col), EngineValue::Empty)
}

/// A minimal fill-only [`Style`] (a single distinguishing attribute) so precedence probes
/// compare one field — the fill colour — rather than the whole style.
pub fn fill_only(hex: &str) -> Style {
    let mut s = Style::default();
    s.fill.pattern_type = "solid".to_string();
    s.fill.fg_color = Some(hex.to_string());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps a raw model (built with full mutable access) into the read-only adapter.
    fn wrap(model: Model<'static>) -> IronCalcEngine {
        IronCalcEngine::from_model(model)
    }

    #[test]
    fn read_styled_viewport_reads_value_and_style() {
        let mut m = new_model();
        m.set_user_input(SHEET, 6, 6, "42".to_string()).unwrap();
        m.set_cell_style(SHEET, 6, 6, &styled_variant(3)).unwrap();
        let e = wrap(m);

        let cells = read_styled_viewport(&e, Viewport::new(5, 5, 1, 1));
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].value, EngineValue::Number(42.0));
        assert!(cells[0].bold, "style must be read alongside the value");
        assert_eq!(cells[0].num_fmt, "0.00");
        assert!(cells[0].fill_argb.is_some());
        assert!(cells[0].is_styled());
    }

    #[test]
    fn count_real_counts_values_and_styles() {
        let styled = StyledCell {
            value: EngineValue::Number(1.0),
            bold: true,
            italic: false,
            font_size: 14,
            fill_argb: Some("#FF0000".into()),
            num_fmt: "0.00".into(),
        };
        let blank = StyledCell {
            value: EngineValue::Empty,
            bold: false,
            italic: false,
            font_size: DEFAULT_FONT_SZ,
            fill_argb: None,
            num_fmt: "general".into(),
        };
        let (nonempty, styledn) = count_real(&[styled, blank.clone()]);
        assert_eq!(nonempty, 1);
        assert_eq!(styledn, 1);
        assert_eq!(count_real(&[blank]), (0, 0));
    }

    #[test]
    fn per_cell_style_roundtrips() {
        let mut m = new_model();
        m.set_cell_style(SHEET, 3, 4, &styled_variant(3)).unwrap();
        let e = wrap(m);
        let s = effective_style(&e, 2, 3);
        assert!(s.font.b);
        assert_eq!(s.num_fmt, "0.00");
    }

    #[test]
    fn row_band_applies_to_untouched_cell() {
        let mut m = new_model();
        m.set_row_style(SHEET, 8, &styled_variant(2)).unwrap(); // 0-based row 7
        let e = wrap(m);
        let s = effective_style(&e, 7, 99);
        assert!(
            s.font.b,
            "an untouched cell in the band resolves the row style"
        );
        assert!(
            get_row_style(&e, 7).is_some(),
            "get_row_style returns the band"
        );
        assert!(!effective_style(&e, 8, 99).font.b);
    }

    #[test]
    fn column_band_applies_to_untouched_cell() {
        let mut m = new_model();
        m.set_column_style(SHEET, 5, &styled_variant(1)).unwrap(); // 0-based col 4
        let e = wrap(m);
        let s = effective_style(&e, 500, 4);
        assert!(
            s.font.b,
            "an untouched cell in the column band resolves the style"
        );
        assert!(get_column_style(&e, 4).is_some());
        assert!(!effective_style(&e, 500, 5).font.b);
    }

    #[test]
    fn empty_cell_styling_resolves() {
        let mut m = new_model();
        m.set_row_style(SHEET, 11, &styled_variant(2)).unwrap(); // 0-based row 10
        let e = wrap(m);
        assert!(is_value_empty(&e, 10, 42), "cell has no value");
        let s = effective_style(&e, 10, 42);
        assert!(
            s.font.b,
            "an EMPTY cell under a band still resolves the band style (Excel empty styling)"
        );
    }

    #[test]
    fn style_precedence_cell_over_row_over_column() {
        let mut m = new_model();
        m.set_column_style(SHEET, 7, &fill_only("#0000FF")).unwrap(); // 0-based col 6
        m.set_row_style(SHEET, 21, &fill_only("#00FF00")).unwrap(); // 0-based row 20
        m.set_cell_style(SHEET, 21, 7, &fill_only("#FF0000"))
            .unwrap(); // (20,6)
        let e = wrap(m);

        // (20,6): cell wins.
        assert_eq!(
            effective_style(&e, 20, 6).fill.fg_color.as_deref(),
            Some("#FF0000")
        );
        // (20,99): only the row band (row over column) — col 99 has no column band.
        assert_eq!(
            effective_style(&e, 20, 99).fill.fg_color.as_deref(),
            Some("#00FF00")
        );
        // (21,6): only the column band (row 21 has no row band).
        assert_eq!(
            effective_style(&e, 21, 6).fill.fg_color.as_deref(),
            Some("#0000FF")
        );
        // (21,99): neither → default (no fill).
        assert_eq!(effective_style(&e, 21, 99).fill.fg_color, None);
    }

    #[test]
    fn excel_max_read_is_addressable() {
        let mut m = new_model();
        // 0-based Excel-max → IronCalc 1-based LAST_ROW / LAST_COLUMN.
        m.set_user_input(
            SHEET,
            (EXCEL_MAX_ROW_0 + 1) as i32,
            (EXCEL_MAX_COL_0 + 1) as i32,
            "7".to_string(),
        )
        .unwrap();
        m.set_cell_style(
            SHEET,
            (EXCEL_MAX_ROW_0 + 1) as i32,
            (EXCEL_MAX_COL_0 + 1) as i32,
            &styled_variant(3),
        )
        .unwrap();
        let e = wrap(m);
        let cells = read_styled_viewport(&e, Viewport::new(EXCEL_MAX_ROW_0, EXCEL_MAX_COL_0, 1, 1));
        assert_eq!(cells[0].value, EngineValue::Number(7.0));
        assert!(
            cells[0].is_styled(),
            "styled read works at Excel-max coordinates"
        );
    }
}
