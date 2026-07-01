//! # formualizer_file — Formualizer 0.7 file-I/O round-trip crate (Sub-project B)
//!
//! This crate answers, for the **Formualizer** side of the engine bake-off: *can we
//! load, edit, and save modern `.xlsx` and CSV, and what survives a
//! `load → edit → save → reload` round-trip?* (functional_spec §6.B, architecture
//! §6). Every claim in `../findings.md` is backed by a passing test in
//! `tests/roundtrip.rs` that reloads and checks specific cells.
//!
//! **Scope boundary (coordinate with Sub-project D).** This crate covers file
//! *structure*: values, formulas, cached-vs-recomputed results, multiple sheets,
//! dates (as serials), CSV. Deep *style* round-trip fidelity (bold/italic/fills/
//! number-format faithfulness) is Sub-project D's job; here we only note that the
//! calamine read path does not surface styles, so this crate does not probe styles.
//!
//! ## Formualizer file-I/O surface exercised here (0.7.0)
//!
//! **Row/column indices are 1-based.** All inputs are produced by committed code —
//! the shared `datagen` generators and the small workbook builders below — so there
//! is no hand-made binary fixture (functional_spec §5.3).
//!
//! - **Write `.xlsx`:** [`Workbook::to_xlsx_bytes`] (umya backend under the hood).
//! - **Read `.xlsx`:** [`CalamineAdapter::open_bytes`] + [`Workbook::from_reader`]
//!   with [`LoadStrategy::EagerAll`].
//! - **Read CSV:** [`CsvAdapter::open_bytes`] + `from_reader` — imports a single
//!   sheet of literal values.
//! - **Write CSV:** Formualizer's file features are *read*-oriented for CSV, so we
//!   export by reading `get_value` per cell and formatting — see [`workbook_to_csv`].
//!
//! ## Observed round-trip fidelity (locked by tests; full table in `findings.md`)
//!
//! - **Literal values survive** as values (numbers, text).
//! - **Formulas survive as formula text, but their cached results are dropped.**
//!   After reload calamine reports `get_value == None`, `get_formula == Some(..)`;
//!   `prepare_graph_all()` + re-eval restores the value.
//! - **Multiple sheets survive**, including cross-sheet formula references.
//! - **Dates survive as their numeric serial** (number formats — the "looks like a
//!   date" layer — are Sub-project D's concern).

use anyhow::{Context, Result};
use datagen::{CellSource, CellValue, SyntheticSheet};
// `open_bytes` is a `SpreadsheetReader` trait method; the trait must be in scope.
use formualizer::workbook::{CalamineAdapter, CsvAdapter, SpreadsheetReader};
use formualizer::{LiteralValue, LoadStrategy, Workbook, WorkbookConfig};

/// The default sheet name a fresh [`Workbook`] exposes.
pub const SHEET1: &str = "Sheet1";
/// The second sheet name used by the multi-sheet edge-case workbook.
pub const SHEET2: &str = "Sheet2";

/// An Excel date serial for 2025-01-01 (days since the 1900 epoch, Excel's leap-bug
/// convention). Used to probe that dates round-trip as their numeric serial.
pub const DATE_SERIAL_2025_01_01: f64 = 45_658.0;

/// Builds an in-memory [`Workbook`] from a deterministic [`SyntheticSheet`], writing
/// `rows × cols` cells, and serializes it to `.xlsx` bytes.
///
/// Numbers and text become literal cells; `Empty` cells are skipped (a blank cell is
/// the absence of a value). This is the committed `.xlsx` generator for the
/// value-fidelity tests — no binary fixture is stored.
pub fn write_synthetic_xlsx(seed: u64, rows: u32, cols: u32) -> Result<Vec<u8>> {
    let sheet = SyntheticSheet::new(seed, rows, cols);
    let mut wb = Workbook::new();
    for r in 0..rows {
        for c in 0..cols {
            match sheet.cell(r, c).value {
                CellValue::Empty => {}
                CellValue::Number(n) => wb
                    .set_value(SHEET1, r + 1, c + 1, LiteralValue::Number(n))
                    .map_err(|e| anyhow::anyhow!("set number ({r},{c}): {e}"))?,
                CellValue::Text(t) => wb
                    .set_value(SHEET1, r + 1, c + 1, LiteralValue::Text(t))
                    .map_err(|e| anyhow::anyhow!("set text ({r},{c}): {e}"))?,
            }
        }
    }
    xlsx_bytes(&wb)
}

/// The `A1`-style cell coordinates the feature workbook populates, so tests read the
/// same addresses the builder wrote.
pub mod feature {
    /// `Sheet1` literal cells.
    pub const INT_CELL: (u32, u32) = (1, 1); // A1 = 1
    pub const NUM_CELL: (u32, u32) = (2, 1); // A2 = 2.5
    pub const TEXT_CELL: (u32, u32) = (3, 1); // A3 = "hello, world"
    pub const BOOL_CELL: (u32, u32) = (4, 1); // A4 = TRUE
    pub const DATE_CELL: (u32, u32) = (5, 1); // A5 = 45658 (2025-01-01 serial)
    /// `Sheet1` formula `=A1+A2` (expected 3.5).
    pub const SUM_CELL: (u32, u32) = (6, 1); // A6 = =A1+A2
    /// `Sheet2` cross-sheet formula `=Sheet1!A1*10` (expected 10).
    pub const CROSS_CELL: (u32, u32) = (1, 1); // Sheet2!A1
}

/// Builds a small, hand-composed workbook covering the round-trip edge cases:
/// mixed literals (int/number/text/bool/date-serial), an in-sheet formula, a second
/// sheet, and a cross-sheet formula. The graph is prepared and fully evaluated, so
/// the (soon-to-be-dropped) cached results exist *before* writing.
pub fn build_feature_workbook() -> Result<Workbook> {
    let mut wb = Workbook::new();

    wb.set_value(SHEET1, 1, 1, LiteralValue::Int(1))
        .map_err(|e| anyhow::anyhow!("A1: {e}"))?;
    wb.set_value(SHEET1, 2, 1, LiteralValue::Number(2.5))
        .map_err(|e| anyhow::anyhow!("A2: {e}"))?;
    wb.set_value(SHEET1, 3, 1, LiteralValue::Text("hello, world".into()))
        .map_err(|e| anyhow::anyhow!("A3: {e}"))?;
    wb.set_value(SHEET1, 4, 1, LiteralValue::Boolean(true))
        .map_err(|e| anyhow::anyhow!("A4: {e}"))?;
    // A date is stored as its numeric serial; the "date-ness" is a number format
    // (Sub-project D). Here we only assert the serial survives.
    wb.set_value(SHEET1, 5, 1, LiteralValue::Number(DATE_SERIAL_2025_01_01))
        .map_err(|e| anyhow::anyhow!("A5: {e}"))?;
    wb.set_formula(SHEET1, 6, 1, "=A1+A2")
        .map_err(|e| anyhow::anyhow!("A6 formula: {e}"))?;

    wb.add_sheet(SHEET2)
        .map_err(|e| anyhow::anyhow!("add Sheet2: {e}"))?;
    wb.set_formula(SHEET2, 1, 1, "=Sheet1!A1*10")
        .map_err(|e| anyhow::anyhow!("Sheet2!A1 formula: {e}"))?;

    wb.prepare_graph_all()
        .map_err(|e| anyhow::anyhow!("prepare graph: {e}"))?;
    wb.evaluate_all()
        .map_err(|e| anyhow::anyhow!("evaluate all: {e}"))?;
    Ok(wb)
}

/// Serializes a workbook to `.xlsx` bytes via the umya backend.
pub fn xlsx_bytes(wb: &Workbook) -> Result<Vec<u8>> {
    wb.to_xlsx_bytes()
        .map_err(|e| anyhow::anyhow!("to_xlsx_bytes: {e}"))
        .context("serialize workbook to .xlsx bytes")
}

/// Loads a workbook from in-memory `.xlsx` bytes via the calamine read backend.
pub fn load_xlsx(bytes: &[u8]) -> Result<Workbook> {
    let adapter =
        CalamineAdapter::open_bytes(bytes.to_vec()).context("open .xlsx bytes with calamine")?;
    Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
        .map_err(|e| anyhow::anyhow!("from_reader (calamine): {e}"))
        .context("load workbook from calamine adapter")
}

/// Loads a workbook from CSV text via the CSV read backend (single sheet of literals).
pub fn load_csv(csv: &str) -> Result<Workbook> {
    let adapter = CsvAdapter::open_bytes(csv.as_bytes().to_vec())
        .context("open CSV bytes with csv adapter")?;
    Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
        .map_err(|e| anyhow::anyhow!("from_reader (csv): {e}"))
        .context("load workbook from csv adapter")
}

/// Exports the top-left `rows × cols` block of a sheet as RFC-4180-ish CSV text.
///
/// Formualizer's CSV feature is import-oriented, so export is done by reading
/// `get_value` per cell and formatting. Numbers use `{}` formatting; text is quoted
/// only when it contains a comma/quote/newline (embedded quotes doubled); empty and
/// unset cells are blank fields. Mirrors `datagen::csv`'s escaping so round-trips
/// line up.
pub fn workbook_to_csv(wb: &Workbook, sheet: &str, rows: u32, cols: u32) -> String {
    let mut out = String::new();
    for r in 0..rows {
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            out.push_str(&format_field(wb.get_value(sheet, r + 1, c + 1)));
        }
        out.push('\n');
    }
    out
}

/// Formats a single (optional) literal as a CSV field.
fn format_field(value: Option<LiteralValue>) -> String {
    match value {
        None | Some(LiteralValue::Empty) => String::new(),
        Some(LiteralValue::Int(i)) => i.to_string(),
        Some(LiteralValue::Number(n)) => n.to_string(),
        Some(LiteralValue::Boolean(b)) => {
            if b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Some(LiteralValue::Text(t)) => {
            if t.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", t.replace('"', "\"\""))
            } else {
                t
            }
        }
        // No other variants are produced by our CSV inputs.
        Some(other) => format!("{other:?}"),
    }
}

/// Extracts a numeric value regardless of `Int` vs `Number` representation.
pub fn as_f64(v: &LiteralValue) -> Option<f64> {
    match v {
        LiteralValue::Int(i) => Some(*i as f64),
        LiteralValue::Number(n) => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_field_escapes_text() {
        assert_eq!(format_field(Some(LiteralValue::Number(3.5))), "3.5");
        assert_eq!(
            format_field(Some(LiteralValue::Text("has,comma".into()))),
            "\"has,comma\""
        );
        assert_eq!(format_field(None), "");
    }
}
