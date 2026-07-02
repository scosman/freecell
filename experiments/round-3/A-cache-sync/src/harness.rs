//! IronCalc ↔ resident-cache sync driver + the agreement contract (architecture §4.4).
//!
//! `build_sheet` creates a sheet with cross-referencing formulas, row/column **band**
//! styles, and custom row heights / column widths. `hydrate_cache` pulls IronCalc's
//! authoritative sizes/band-styles into a fresh [`ResidentCache`] (the "on load" path).
//! After each structural edit (and each undo/redo), the caller mirrors the shift onto the
//! cache and calls [`assert_cache_agrees`] — the load-bearing test: a design that is fast
//! but disagrees with IronCalc is a FAIL.
//!
//! All indices are **1-based** (IronCalc convention). Sheet is always index 0.

use ironcalc_base::expressions::types::Area;
use ironcalc_base::types::Style;
use ironcalc_base::UserModel;

use crate::cache::{ResidentCache, StyleId};

pub const SHEET: u32 = 0;

/// IronCalc's Excel-max axis lengths (used to build a full-row / full-column selection so
/// `update_range_style` takes its band-style branch).
const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

/// Rows that receive a colored band style + a custom height in the reference sheet.
pub const BANDED_ROWS: [i32; 3] = [3, 10, 25];
/// Columns that receive a band style + a custom width.
pub const BANDED_COLS: [i32; 2] = [2, 5];
pub const CUSTOM_ROW_HEIGHT: f64 = 42.0;
pub const CUSTOM_COL_WIDTH: f64 = 137.0;

/// Builds a reference sheet of `data_rows` populated rows:
///   - Column A: literals `A_r = r` (so references are checkable).
///   - Column B: cross-references `B_r = A_r + A_{r-1}` (formula that must re-target).
///   - C1: `=SUM(A1:A{n})` — a range total that must expand/contract on insert/delete.
///   - Row band style + custom height on [`BANDED_ROWS`]; column band style + custom
///     width on [`BANDED_COLS`].
pub fn build_sheet(model: &mut UserModel<'static>, data_rows: i32) -> Result<(), String> {
    model.pause_evaluation();
    for r in 1..=data_rows {
        model.set_user_input(SHEET, r, 1, &r.to_string())?;
        if r > 1 {
            model.set_user_input(SHEET, r, 2, &format!("=A{r}+A{}", r - 1))?;
        }
    }
    model.set_user_input(SHEET, 1, 3, &format!("=SUM(A1:A{data_rows})"))?;

    // Row band styles + custom heights.
    for &r in BANDED_ROWS.iter() {
        if r <= data_rows {
            // A full-row style: update_range_style over the whole row band.
            set_row_band_fill(model, r, "00FF00")?;
            model.set_rows_height(SHEET, r, r, CUSTOM_ROW_HEIGHT)?;
        }
    }
    // Column band styles + custom widths.
    for &c in BANDED_COLS.iter() {
        set_col_band_fill(model, c, "0000FF")?;
        model.set_columns_width(SHEET, c, c, CUSTOM_COL_WIDTH)?;
    }

    model.resume_evaluation();
    model.evaluate();
    Ok(())
}

/// Applies a solid fill to an entire **row band** (`hex` like `"00FF00"`) via a full-row
/// `update_range_style`, which IronCalc lands in `worksheet.rows[r].s` — exactly the
/// structure insert/delete re-keys, and what `get_row_style` reads back.
pub fn set_row_band_fill(
    model: &mut UserModel<'static>,
    row: i32,
    hex: &str,
) -> Result<(), String> {
    model.set_selected_cell(row, 1)?;
    let area = Area {
        sheet: SHEET,
        row,
        column: 1,
        width: LAST_COLUMN,
        height: 1,
    };
    model.update_range_style(&area, "fill.bg_color", &format!("#{hex}"))
}

/// Applies a solid fill to an entire **column band** via a full-column
/// `update_range_style` (lands in `worksheet.cols[..].style`; read via `get_column_style`).
pub fn set_col_band_fill(
    model: &mut UserModel<'static>,
    col: i32,
    hex: &str,
) -> Result<(), String> {
    model.set_selected_cell(1, col)?;
    let area = Area {
        sheet: SHEET,
        row: 1,
        column: col,
        width: 1,
        height: LAST_ROW,
    };
    model.update_range_style(&area, "fill.bg_color", &format!("#{hex}"))
}

/// The authoritative sizes/band-styles read back from IronCalc for one axis line.
pub struct EngineLine {
    pub size: f64,
    pub band_style: Option<Style>,
}

/// Reads IronCalc's row height + row band style (authoritative).
pub fn engine_row(model: &UserModel<'static>, row: i32) -> Result<EngineLine, String> {
    Ok(EngineLine {
        size: model.get_row_height(SHEET, row)?,
        band_style: model.get_model().get_row_style(SHEET, row)?,
    })
}

/// Reads IronCalc's column width + column band style (authoritative).
pub fn engine_col(model: &UserModel<'static>, col: i32) -> Result<EngineLine, String> {
    Ok(EngineLine {
        size: model.get_model().get_column_width(SHEET, col)?,
        band_style: model.get_model().get_column_style(SHEET, col)?,
    })
}

/// Hydrates a fresh cache from IronCalc's current state (the "on load" path). Populates
/// row/col sizes + band styles + per-cell styles over `1..=rows` x `1..=cols`.
pub fn hydrate_cache(
    model: &UserModel<'static>,
    rows: i32,
    cols: i32,
) -> Result<ResidentCache, String> {
    let default_row = model.get_row_height(SHEET, 1_000_000).unwrap_or(21.0);
    let default_col = model.get_column_width(SHEET, 16_000).unwrap_or(100.0);
    let mut cache = ResidentCache::new(rows as usize, cols as usize, default_row, default_col);

    for r in 1..=rows {
        let line = engine_row(model, r)?;
        if (line.size - default_row).abs() > f64::EPSILON {
            cache.rows.set_size(r as i64, line.size);
        }
        if let Some(style) = line.band_style {
            let id = cache.interner.intern(&style);
            cache.rows.set_band_style(r as i64, id);
        }
    }
    for c in 1..=cols {
        let line = engine_col(model, c)?;
        if (line.size - default_col).abs() > f64::EPSILON {
            cache.cols.set_size(c as i64, line.size);
        }
        if let Some(style) = line.band_style {
            let id = cache.interner.intern(&style);
            cache.cols.set_band_style(c as i64, id);
        }
    }
    // Per-cell styles (only where the cell's resolved style differs from default).
    for r in 1..=rows {
        for c in 1..=cols {
            let style = model.get_model().get_style_for_cell(SHEET, r, c)?;
            if style != Style::default() {
                let id = cache.interner.intern(&style);
                cache.set_cell_style(r as i64, c as i64, id);
            }
        }
    }
    Ok(cache)
}

/// The **agreement contract** (architecture §4.4). For each row index in `row_samples`
/// and column index in `col_samples`, assert the cache's shifted size + band style equal
/// IronCalc's re-read authoritative values, and that the cumulative offset is consistent
/// with the summed sizes. Returns `Err` describing the first disagreement.
pub fn assert_cache_agrees(
    model: &UserModel<'static>,
    cache: &mut ResidentCache,
    row_samples: &[i32],
    col_samples: &[i32],
) -> Result<(), String> {
    for &r in row_samples {
        let engine = engine_row(model, r)?;
        let cached_size = cache.rows.size(r as i64);
        if (cached_size - engine.size).abs() > 1e-6 {
            return Err(format!(
                "row {r} size mismatch: cache={cached_size} engine={}",
                engine.size
            ));
        }
        let cached_style = cache.interner.resolve(cache.rows.band_style(r as i64)).clone();
        let engine_style = engine.band_style.unwrap_or_default();
        if cached_style != engine_style {
            return Err(format!(
                "row {r} band-style mismatch: cache={cached_style:?} engine={engine_style:?}"
            ));
        }
    }
    for &c in col_samples {
        let engine = engine_col(model, c)?;
        let cached_size = cache.cols.size(c as i64);
        if (cached_size - engine.size).abs() > 1e-6 {
            return Err(format!(
                "col {c} size mismatch: cache={cached_size} engine={}",
                engine.size
            ));
        }
        let cached_style = cache.interner.resolve(cache.cols.band_style(c as i64)).clone();
        let engine_style = engine.band_style.unwrap_or_default();
        if cached_style != engine_style {
            return Err(format!(
                "col {c} band-style mismatch: cache={cached_style:?} engine={engine_style:?}"
            ));
        }
    }

    // Cumulative-offset consistency: offset(k+1) - offset(k) == size(k). Only meaningful
    // for lines strictly inside the dense extent (offset past the last line clamps).
    let extent = cache.rows.extent() as i64;
    for &r in row_samples {
        if (r as i64) >= extent {
            continue;
        }
        let lo = cache.rows.offset(r as i64);
        let hi = cache.rows.offset(r as i64 + 1);
        let span = hi - lo;
        let size = cache.rows.size(r as i64);
        if (span - size).abs() > 1e-6 {
            return Err(format!(
                "row {r} cumulative-offset inconsistent: offset span={span} size={size}"
            ));
        }
    }
    Ok(())
}

/// Reads a cell's stored formula (for reference-shift assertions), or its literal content.
pub fn cell_content(model: &UserModel<'static>, row: i32, col: i32) -> String {
    model.get_cell_content(SHEET, row, col).unwrap_or_default()
}

/// Reads a cell's evaluated/formatted display value.
pub fn cell_display(model: &UserModel<'static>, row: i32, col: i32) -> String {
    model
        .get_formatted_cell_value(SHEET, row, col)
        .unwrap_or_default()
}

/// Interns a style into the cache's interner, returning its id (for cell-style checks).
pub fn intern(cache: &mut ResidentCache, style: &Style) -> StyleId {
    cache.interner.intern(style)
}

