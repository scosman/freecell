//! # ironcalc_file — IronCalc 0.7 file-I/O round-trip crate (Sub-project B)
//!
//! This crate answers, for the **IronCalc** side of the engine bake-off: *can we
//! load, edit, and save modern `.xlsx` and CSV, and what survives a
//! `load → edit → save → reload` round-trip?* (functional_spec §6.B, architecture
//! §6, §1.1). Every claim in `../findings.md` is backed by a passing test in
//! `tests/roundtrip.rs` that reloads and checks specific cells.
//!
//! **Scope boundary (coordinate with Sub-project D).** This crate covers file
//! *structure*: values, formulas, cached results, multiple sheets, number-formatted
//! dates, CSV. Deep *style* round-trip fidelity is Sub-project D's job; here we make
//! only a **shallow** style-survival probe (one bold cell) to record that IronCalc's
//! native writer carries styles at all — we do not enumerate style faithfulness.
//!
//! ## IronCalc file-I/O surface exercised here (0.7.1)
//!
//! **Row/column indices are 1-based `i32`; sheet index is `u32` (`0` == first).**
//! All inputs are produced by committed code (the shared `datagen` generators and
//! the builders below) — no hand-made binary fixture (functional_spec §5.3).
//!
//! - **Read `.xlsx`:** [`ironcalc::import::load_from_xlsx_bytes`] returns a
//!   `Workbook`; wrap it with [`Model::from_workbook`] to get an evaluatable model.
//! - **Write `.xlsx`:** [`ironcalc::export::save_xlsx_to_writer`] takes any
//!   `Write + Seek`; we pass an in-memory [`std::io::Cursor`] and take its buffer.
//!   (`save_to_xlsx` is path-based and refuses to overwrite, so the writer form is
//!   the one that fits an in-memory round-trip.)
//! - **CSV:** IronCalc ships **no** CSV support, so this crate hand-rolls a minimal
//!   RFC-4180 bridge over `set_user_input` / `get_formatted_cell_value`
//!   ([`load_csv_into_model`], [`model_to_csv`]) — itself a finding.
//!
//! ## Observed round-trip fidelity (locked by tests; full table in `findings.md`)
//!
//! - **Literal values survive** as values.
//! - **Formulas survive as formula text**; because IronCalc's `evaluate()` is a full
//!   recompute, the value is available after `evaluate()`.
//! - **Multiple sheets survive**, including cross-sheet references.
//! - **Number-formatted dates survive**: the serial value *and* its number format,
//!   so `get_formatted_cell_value` still renders a date string after reload.
//! - **Styles survive the native writer** (shallow probe only; depth is D).

use std::io::Cursor;

use anyhow::{Context, Result};
use datagen::{CellSource, CellValue, SyntheticSheet};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::types::Style;
use ironcalc_base::Model;

/// Sheet index 0 — the sheet a fresh model creates.
pub const SHEET0: u32 = 0;

/// An Excel date serial for 2025-01-01 (the same serial the Formualizer crate uses,
/// so the two engines' date probes are directly comparable).
pub const DATE_SERIAL_2025_01_01: f64 = 45_658.0;

/// A locale/tz/language triple used consistently for build + reload, so nothing
/// shifts because of a config mismatch across the round trip.
const LOCALE: &str = "en";
const TIMEZONE: &str = "UTC";
const LANGUAGE: &str = "en";

/// Creates a fresh single-sheet model. Uses `'static` string literals so the model
/// owns no shorter borrow (matching the `ironcalc_bench` adapter's pattern).
pub fn new_model() -> Result<Model<'static>> {
    Model::new_empty("roundtrip", LOCALE, TIMEZONE, LANGUAGE)
        .map_err(|e| anyhow::anyhow!("new_empty: {e}"))
}

/// Builds a model from a deterministic [`SyntheticSheet`] (writing `rows × cols`
/// non-empty cells) and serializes it to `.xlsx` bytes. The committed `.xlsx`
/// generator for the value-fidelity tests — no binary fixture is stored.
pub fn write_synthetic_xlsx(seed: u64, rows: u32, cols: u32) -> Result<Vec<u8>> {
    let sheet = SyntheticSheet::new(seed, rows, cols);
    let mut model = new_model()?;
    for r in 0..rows {
        for c in 0..cols {
            let input = match sheet.cell(r, c).value {
                CellValue::Empty => continue,
                CellValue::Number(n) => format!("{n}"),
                CellValue::Text(t) => t,
            };
            model
                .set_user_input(SHEET0, (r + 1) as i32, (c + 1) as i32, input)
                .map_err(|e| anyhow::anyhow!("set ({r},{c}): {e}"))?;
        }
    }
    model.evaluate();
    xlsx_bytes(&model)
}

/// The cell coordinates the feature model populates (1-based), so tests read the
/// same addresses the builder wrote.
pub mod feature {
    /// Sheet 0 literal cells.
    pub const INT_CELL: (i32, i32) = (1, 1); // A1 = 1
    pub const NUM_CELL: (i32, i32) = (2, 1); // A2 = 2.5
    pub const TEXT_CELL: (i32, i32) = (3, 1); // A3 = "hello, world"
    pub const BOOL_CELL: (i32, i32) = (4, 1); // A4 = TRUE
    pub const DATE_CELL: (i32, i32) = (5, 1); // A5 = 45658, formatted as a date
    /// Sheet 0 formula `=A1+A2` (expected 3.5).
    pub const SUM_CELL: (i32, i32) = (6, 1); // A6 = =A1+A2
    /// Sheet 1 cross-sheet formula `=Sheet1!A1*10` (expected 10).
    pub const CROSS_CELL: (i32, i32) = (1, 1); // Sheet2!A1
    /// A cell we make bold, for the shallow style-survival probe.
    pub const BOLD_CELL: (i32, i32) = (7, 1); // A7
}

/// An Excel number-format code for an ISO-ish date, applied to the date cell so
/// IronCalc renders the serial as a date string.
pub const DATE_NUM_FMT: &str = "yyyy-mm-dd";

/// Builds a small, hand-composed model covering the round-trip edge cases: mixed
/// literals (int/number/text/bool), a number-formatted date, an in-sheet formula, a
/// second sheet, a cross-sheet formula, and one bold cell (shallow style probe). The
/// model is evaluated before it is returned.
pub fn build_feature_model() -> Result<Model<'static>> {
    let mut model = new_model()?;

    let put = |model: &mut Model, (r, c): (i32, i32), s: &str| -> Result<()> {
        model
            .set_user_input(SHEET0, r, c, s.to_string())
            .map_err(|e| anyhow::anyhow!("set ({r},{c}): {e}"))
    };

    put(&mut model, feature::INT_CELL, "1")?;
    put(&mut model, feature::NUM_CELL, "2.5")?;
    put(&mut model, feature::TEXT_CELL, "hello, world")?;
    put(&mut model, feature::BOOL_CELL, "TRUE")?;
    put(
        &mut model,
        feature::DATE_CELL,
        &format!("{DATE_SERIAL_2025_01_01}"),
    )?;
    put(&mut model, feature::SUM_CELL, "=A1+A2")?;

    // Apply a date number format to the date cell so it renders as a date string.
    let (dr, dc) = feature::DATE_CELL;
    let mut date_style = model
        .get_style_for_cell(SHEET0, dr, dc)
        .map_err(|e| anyhow::anyhow!("get date style: {e}"))?;
    date_style.num_fmt = DATE_NUM_FMT.to_string();
    model
        .set_cell_style(SHEET0, dr, dc, &date_style)
        .map_err(|e| anyhow::anyhow!("set date num_fmt: {e}"))?;

    // Make one cell bold for the shallow style-survival probe.
    let (br, bc) = feature::BOLD_CELL;
    put(&mut model, feature::BOLD_CELL, "bold")?;
    let mut bold_style = model
        .get_style_for_cell(SHEET0, br, bc)
        .map_err(|e| anyhow::anyhow!("get bold style: {e}"))?;
    bold_style.font.b = true;
    model
        .set_cell_style(SHEET0, br, bc, &bold_style)
        .map_err(|e| anyhow::anyhow!("set bold: {e}"))?;

    // Second sheet + a cross-sheet formula referencing Sheet1 (index 0).
    model
        .add_sheet("Sheet2")
        .map_err(|e| anyhow::anyhow!("add Sheet2: {e}"))?;
    model
        .set_user_input(1, 1, 1, "=Sheet1!A1*10".to_string())
        .map_err(|e| anyhow::anyhow!("Sheet2!A1 formula: {e}"))?;

    model.evaluate();
    Ok(model)
}

/// Serializes a model to `.xlsx` bytes via IronCalc's native writer.
pub fn xlsx_bytes(model: &Model) -> Result<Vec<u8>> {
    let cursor = save_xlsx_to_writer(model, Cursor::new(Vec::new()))
        .map_err(|e| anyhow::anyhow!("save_xlsx_to_writer: {e:?}"))
        .context("serialize model to .xlsx bytes")?;
    Ok(cursor.into_inner())
}

/// Loads a model from in-memory `.xlsx` bytes via IronCalc's native importer.
pub fn load_xlsx(bytes: &[u8]) -> Result<Model<'static>> {
    let workbook = load_from_xlsx_bytes(bytes, "roundtrip", LOCALE, TIMEZONE)
        .map_err(|e| anyhow::anyhow!("load_from_xlsx_bytes: {e:?}"))
        .context("read .xlsx bytes")?;
    Model::from_workbook(workbook, LANGUAGE)
        .map_err(|e| anyhow::anyhow!("from_workbook: {e}"))
        .context("build model from imported workbook")
}

/// Returns the sheet names in order (via `get_worksheets_properties`).
pub fn sheet_names(model: &Model) -> Vec<String> {
    model
        .get_worksheets_properties()
        .into_iter()
        .map(|p| p.name)
        .collect()
}

/// Loads generated CSV text into a fresh model, one field per cell via
/// `set_user_input`. IronCalc has no CSV import; this is the DIY bridge.
///
/// Uses a minimal RFC-4180 parser: fields split on unquoted commas; a field wrapped
/// in double quotes may contain commas/newlines and doubled `""` quotes. Every field
/// is fed verbatim to `set_user_input` (so a leading `=` would be a formula — our
/// value inputs never start with `=`).
pub fn load_csv_into_model(csv: &str) -> Result<Model<'static>> {
    let mut model = new_model()?;
    for (r, record) in parse_csv(csv).into_iter().enumerate() {
        for (c, field) in record.into_iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            model
                .set_user_input(SHEET0, (r + 1) as i32, (c + 1) as i32, field)
                .map_err(|e| anyhow::anyhow!("csv set ({r},{c}): {e}"))?;
        }
    }
    model.evaluate();
    Ok(model)
}

/// Exports the top-left `rows × cols` block of a sheet as RFC-4180-ish CSV, reading
/// each cell's *displayed* value via `get_formatted_cell_value` (so number formats
/// like the date are honored). Mirrors `datagen::csv`'s escaping.
pub fn model_to_csv(model: &Model, sheet: u32, rows: u32, cols: u32) -> String {
    let mut out = String::new();
    for r in 0..rows {
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            let raw = model
                .get_formatted_cell_value(sheet, (r + 1) as i32, (c + 1) as i32)
                .unwrap_or_default();
            out.push_str(&escape_field(&raw));
        }
        out.push('\n');
    }
    out
}

/// Quotes a CSV field only when it contains a comma/quote/newline; doubles quotes.
fn escape_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// A tiny RFC-4180 CSV parser: `Vec` of records, each a `Vec` of unescaped fields.
/// Handles quoted fields with embedded commas/newlines and doubled quotes. Adequate
/// for our generated inputs; not a general-purpose CSV library.
fn parse_csv(input: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next(); // escaped quote
                    field.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                record.push(std::mem::take(&mut field));
            }
            '\r' if !in_quotes => { /* swallow CR; LF ends the record */ }
            '\n' if !in_quotes => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(ch),
        }
    }
    // A trailing field/record with no final newline.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Reads a bold flag off a cell's style (shallow style probe helper).
pub fn is_bold(model: &Model, sheet: u32, row: i32, col: i32) -> Result<bool> {
    let style: Style = model
        .get_style_for_cell(sheet, row, col)
        .map_err(|e| anyhow::anyhow!("get_style_for_cell: {e}"))?;
    Ok(style.font.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_handles_quotes_and_commas() {
        let parsed = parse_csv("plain,\"has,comma\",\"has\"\"quote\"\n3.5,,x\n");
        assert_eq!(
            parsed,
            vec![
                vec![
                    "plain".to_string(),
                    "has,comma".to_string(),
                    "has\"quote".to_string()
                ],
                vec!["3.5".to_string(), String::new(), "x".to_string()],
            ]
        );
    }

    #[test]
    fn escape_field_quotes_when_needed() {
        assert_eq!(escape_field("plain"), "plain");
        assert_eq!(escape_field("a,b"), "\"a,b\"");
        assert_eq!(escape_field("a\"b"), "\"a\"\"b\"");
    }
}
