//! # formualizer_formatting — Formualizer 0.7 formatting-exposure probe (Sub-project D)
//!
//! This crate answers, for **Formualizer**, the Sub-project D question
//! (functional_spec §6.D, architecture §6): *what formatting/metadata does the engine
//! expose on read, on write, and across a load → edit → save round-trip, and what
//! FreeCell formatting model does that imply?*
//!
//! ## The headline finding (verified by [`tests`](../tests/probe.rs))
//!
//! Formualizer 0.7.0's calc `Workbook` is a **values + formulas pipe with no style
//! path in either direction**:
//!
//! - **Read.** Every read path emits `CellData { style: None, .. }`. The calamine
//!   backend advertises `capabilities().styles == false` and the umya backend's
//!   `read_cell` hard-codes `style: None` (Sub-project A locked this; we re-confirm
//!   it here structurally — the engine never surfaces a non-`None` style).
//! - **Write.** [`Workbook::to_xlsx_bytes`](formualizer::Workbook::to_xlsx_bytes)
//!   builds a **fresh** `umya` file from the engine's values/formulas — it does **not**
//!   carry any style from a loaded workbook. So even styles that existed in the source
//!   `.xlsx` are dropped on save through the engine.
//! - **No bridge.** Formualizer's own `UmyaAdapter` wraps a private
//!   `umya_spreadsheet::Spreadsheet` with **no public accessor** (`into_inner` /
//!   `workbook()` do not exist), and `Workbook` does not retain the adapter after
//!   `from_reader`. So there is no way to reach a styled umya workbook *through*
//!   Formualizer.
//!
//! ## Consequence — the umya-direct path
//!
//! To read or preserve formatting alongside a Formualizer workbook, FreeCell must own a
//! **`umya_spreadsheet::Spreadsheet` directly** (a direct crate dependency, not via
//! Formualizer). umya *does* fully expose styles on read and preserves them across a
//! round-trip. This crate exercises that path: it builds a styled `.xlsx` from committed
//! umya code, reads bold/italic/size/fill/number-format/row-height/col-width/merges back,
//! edits a style, saves, reloads, and confirms survival. That is the evidence behind the
//! recommended "engine for calc + umya-owned side workbook for styles" model.

use serde::Serialize;

pub use umya_spreadsheet::Spreadsheet;

/// A small, engine-neutral snapshot of the formatting attributes FreeCell cares about
/// (mirrors `datagen::CellFormat` plus a couple of Excel attributes it omits). Every
/// field is optional/absent-aware so "the engine did not surface this" is representable
/// distinctly from "the engine surfaced a default".
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct NeutralFormat {
    pub bold: bool,
    pub italic: bool,
    /// Font size in points, if the engine surfaces one.
    pub font_size: Option<f64>,
    /// Fill / highlight colour as an ARGB hex string (e.g. `"FFFFFF00"`), if any.
    pub fill_argb: Option<String>,
    /// Number-format code (e.g. `"0.00"`), if any.
    pub number_format: Option<String>,
}

/// The three axes each formatting attribute is scored on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Support {
    /// Exposed by the engine's own native API.
    Native,
    /// Reachable only by owning a `umya_spreadsheet` workbook alongside the engine.
    ViaUmya,
    /// Not reachable at all through this engine's ecosystem in 0.7.0.
    None,
}

/// One row of the capability matrix: an attribute and its read / write / round-trip
/// support, plus a short note.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRow {
    pub attribute: &'static str,
    pub read: Support,
    pub write: Support,
    pub roundtrip: Support,
    pub note: &'static str,
}

/// The full, env-stamped capability matrix serialized to `results/formualizer/`.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityMatrix {
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub umya_version: &'static str,
    pub rustc: &'static str,
    pub date: &'static str,
    pub rows: Vec<CapabilityRow>,
}

/// The single sheet used throughout.
pub const SHEET: &str = "Sheet1";

/// Builds a small **styled** `.xlsx` in memory, entirely from committed umya code (no
/// hand-made binary fixture — satisfies functional_spec §5.3). The fixture carries one
/// of each attribute FreeCell probes:
///
/// - `A1`: text, **bold**, 16pt, yellow fill, number format `"0.00"`.
/// - `B2`: text, *italic*.
/// - a `C3:D3` merge; row 1 height 40.0; column A width 30.0.
pub fn build_styled_xlsx_bytes() -> Vec<u8> {
    let mut book = umya_spreadsheet::new_file();
    {
        let sheet = book
            .get_sheet_by_name_mut(SHEET)
            .expect("default sheet exists");

        // A1: bold, 16pt, yellow fill, 2-decimal number format.
        {
            let cell = sheet.get_cell_mut("A1");
            cell.set_value_number(12.5);
            let style = cell.get_style_mut();
            style.get_font_mut().set_bold(true).set_size(16.0);
            style.set_background_color_solid("FFFFFF00"); // ARGB yellow
            style.get_number_format_mut().set_format_code("0.00");
        }

        // B2: italic text.
        {
            let cell = sheet.get_cell_mut("B2");
            cell.set_value("hello");
            cell.get_style_mut().get_font_mut().set_italic(true);
        }

        // A merge + explicit row height / column width.
        sheet.add_merge_cells("C3:D3");
        sheet.get_row_dimension_mut(&1).set_height(40.0);
        sheet.get_column_dimension_mut("A").set_width(30.0);
    }

    let mut buf = Vec::new();
    umya_spreadsheet::writer::xlsx::write_writer(&book, &mut buf).expect("umya write to bytes");
    buf
}

/// Loads `.xlsx` bytes into a Formualizer calc [`Workbook`](formualizer::Workbook) via
/// the calamine backend. This is the calc path — values + formulas only; **styles are
/// discarded** (see module docs).
pub fn load_into_formualizer(bytes: &[u8]) -> formualizer::Workbook {
    use formualizer::workbook::{CalamineAdapter, SpreadsheetReader};
    use formualizer::{LoadStrategy, Workbook, WorkbookConfig};
    let adapter = CalamineAdapter::open_bytes(bytes.to_vec()).expect("open .xlsx via calamine");
    Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
        .expect("formualizer from_reader")
}

/// Loads `.xlsx` bytes into a directly-owned umya [`Spreadsheet`] — the style source of
/// truth FreeCell would keep alongside a Formualizer workbook.
pub fn load_into_umya(bytes: &[u8]) -> Spreadsheet {
    let cursor = std::io::Cursor::new(bytes.to_vec());
    umya_spreadsheet::reader::xlsx::read_reader(cursor, true).expect("umya read from bytes")
}

/// Serializes a umya [`Spreadsheet`] back to `.xlsx` bytes (the save half of the
/// umya-direct round-trip).
pub fn save_umya(book: &Spreadsheet) -> Vec<u8> {
    let mut buf = Vec::new();
    umya_spreadsheet::writer::xlsx::write_writer(book, &mut buf).expect("umya write to bytes");
    buf
}

/// Reads the [`NeutralFormat`] for a cell (e.g. `"A1"`) straight from a umya
/// [`Spreadsheet`]. This is the formatting read path FreeCell would use with Formualizer
/// (Formualizer itself surfaces nothing here).
pub fn read_format_via_umya(book: &Spreadsheet, coordinate: &str) -> NeutralFormat {
    let sheet = book.get_sheet_by_name(SHEET).expect("sheet exists");
    let cell = match sheet.get_cell(coordinate) {
        Some(c) => c,
        None => return NeutralFormat::default(),
    };
    let style = cell.get_style();
    let (bold, italic, font_size) = match style.get_font() {
        Some(font) => (*font.get_bold(), *font.get_italic(), Some(*font.get_size())),
        None => (false, false, None),
    };
    // A "no fill" cell reports the default (often "FFFFFFFF"/empty); only surface a fill
    // when a background colour is actually present.
    let fill_argb = style
        .get_background_color()
        .map(|c| c.get_argb().to_string())
        .filter(|s| !s.is_empty());
    let number_format = style
        .get_number_format()
        .map(|n| n.get_format_code().to_string())
        .filter(|s| !s.is_empty() && s != "General");
    NeutralFormat {
        bold,
        italic,
        font_size,
        fill_argb,
        number_format,
    }
}

/// Reads a row's height (points) from a umya workbook, if an explicit dimension exists.
pub fn read_row_height_via_umya(book: &Spreadsheet, row_1_based: u32) -> Option<f64> {
    let sheet = book.get_sheet_by_name(SHEET)?;
    sheet
        .get_row_dimension(&row_1_based)
        .map(|r| *r.get_height())
}

/// Reads a column's width from a umya workbook, if an explicit dimension exists.
pub fn read_col_width_via_umya(book: &Spreadsheet, column_letter: &str) -> Option<f64> {
    let sheet = book.get_sheet_by_name(SHEET)?;
    sheet
        .get_column_dimension(column_letter)
        .map(|c| *c.get_width())
}

/// Returns the merge ranges (as `"A1:B2"` strings) declared on the sheet.
pub fn read_merges_via_umya(book: &Spreadsheet) -> Vec<String> {
    let sheet = match book.get_sheet_by_name(SHEET) {
        Some(s) => s,
        None => return Vec::new(),
    };
    sheet
        .get_merge_cells()
        .iter()
        .map(|r| r.get_range())
        .collect()
}

/// Builds the Formualizer capability matrix (the numbers/statuses below are all backed
/// by the passing probes in `tests/probe.rs`).
pub fn capability_matrix() -> CapabilityMatrix {
    use Support::*;
    let rows = vec![
        CapabilityRow {
            attribute: "bold",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "Formualizer CellData.style == None; readable/writable only via a directly-owned umya workbook.",
        },
        CapabilityRow {
            attribute: "italic",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "As bold.",
        },
        CapabilityRow {
            attribute: "font_size",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Font::get_size / set_size.",
        },
        CapabilityRow {
            attribute: "fill_color",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Style::set_background_color_solid / get_background_color -> ARGB.",
        },
        CapabilityRow {
            attribute: "number_format",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya NumberingFormat::get/set_format_code.",
        },
        CapabilityRow {
            attribute: "borders",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Style::get_borders per-side; not probed exhaustively (Round 2).",
        },
        CapabilityRow {
            attribute: "row_height",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Row::get/set_height.",
        },
        CapabilityRow {
            attribute: "col_width",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Column::get/set_width.",
        },
        CapabilityRow {
            attribute: "merges",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: ViaUmya,
            note: "umya Worksheet::get_merge_cells / add_merge_cells.",
        },
        CapabilityRow {
            attribute: "conditional_formatting",
            read: ViaUmya,
            write: ViaUmya,
            roundtrip: None,
            note: "umya exposes get/add_conditional_formatting_collection, but write-back fidelity is Unverified (deferred to Round 2).",
        },
        CapabilityRow {
            attribute: "to_xlsx_bytes(engine) preserves styles",
            read: None,
            write: None,
            roundtrip: None,
            note: "Workbook::to_xlsx_bytes builds a fresh umya file from values/formulas only; ALL styles are dropped.",
        },
    ];
    CapabilityMatrix {
        engine: "formualizer",
        engine_version: "0.7.0",
        umya_version: "2.3.2",
        rustc: "1.94.1",
        date: "2026-07-01",
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_format_defaults_are_absent() {
        let f = NeutralFormat::default();
        assert!(!f.bold);
        assert!(f.fill_argb.is_none());
        assert!(f.number_format.is_none());
    }

    #[test]
    fn matrix_serializes() {
        let json = serde_json::to_string(&capability_matrix()).expect("serialize matrix");
        assert!(json.contains("formualizer"));
        assert!(json.contains("to_xlsx_bytes"));
    }
}
