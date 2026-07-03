//! Fixture workbooks — small, deterministic `WorkbookDocument`s built from code.
//!
//! Used by this crate's round-trip tests and by downstream crates that need a real
//! engine-backed document to exercise (e.g. `render-tests` scene fixtures,
//! `components/engine_worker.md §Dependencies`). Every fixture is populated through the
//! same edit APIs the app uses (`set_user_input` / `update_range_style`), so a saved
//! fixture is a faithful IronCalc `.xlsx`.
//!
//! Coordinates are **0-based** here (matching `freecell_core::CellRef`), converted to
//! IronCalc's 1-based `(row, column)` inside the helpers.

use freecell_core::CellRef;
use ironcalc_base::expressions::types::Area;

use crate::document::WorkbookDocument;

/// Sets a cell's raw input (a literal or an `=formula`). Panics on engine rejection — a
/// fixture is committed code, so a rejected input is a bug in the fixture, not a runtime
/// condition.
fn set(doc: &mut WorkbookDocument, sheet: u32, cell: CellRef, input: &str) {
    let (row, col) = (cell.row as i32 + 1, cell.col as i32 + 1);
    doc.user_model_mut()
        .set_user_input(sheet, row, col, input)
        .expect("fixture cell input should be valid");
}

/// Applies a single-cell style attribute (`font.b`, `fill.fg_color`, `num_fmt`, …). Panics
/// on rejection for the same reason as [`set`].
fn style(doc: &mut WorkbookDocument, sheet: u32, cell: CellRef, path: &str, value: &str) {
    let area = Area {
        sheet,
        row: cell.row as i32 + 1,
        column: cell.col as i32 + 1,
        width: 1,
        height: 1,
    };
    doc.user_model_mut()
        .update_range_style(&area, path, value)
        .expect("fixture style should be valid");
}

/// A single sheet of plain values: a number, text, a decimal, a negative, and a boolean.
pub fn values() -> WorkbookDocument {
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    set(&mut doc, 0, CellRef::new(0, 0), "42"); // A1
    set(&mut doc, 0, CellRef::new(0, 1), "hello"); // B1
    set(&mut doc, 0, CellRef::new(1, 0), "3.14"); // A2
    set(&mut doc, 0, CellRef::new(1, 1), "-7"); // B2
    set(&mut doc, 0, CellRef::new(0, 2), "TRUE"); // C1
    doc
}

/// Formula cells: a `SUM` range, a scalar formula, and a `=1/0` that resolves to `#DIV/0!`.
pub fn formulas() -> WorkbookDocument {
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    set(&mut doc, 0, CellRef::new(0, 0), "10"); // A1
    set(&mut doc, 0, CellRef::new(1, 0), "20"); // A2
    set(&mut doc, 0, CellRef::new(2, 0), "30"); // A3
    set(&mut doc, 0, CellRef::new(3, 0), "=SUM(A1:A3)"); // A4 → 60
    set(&mut doc, 0, CellRef::new(0, 1), "=A1*2"); // B1 → 20
    set(&mut doc, 0, CellRef::new(0, 2), "=1/0"); // C1 → #DIV/0!
    doc
}

/// Character-format styles: bold, italic, underline, a solid red fill, and a blue font
/// colour (one attribute per cell for interpretable round-trip assertions).
pub fn styles() -> WorkbookDocument {
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    set(&mut doc, 0, CellRef::new(0, 0), "1"); // A1
    style(&mut doc, 0, CellRef::new(0, 0), "font.b", "true");
    set(&mut doc, 0, CellRef::new(0, 1), "2"); // B1
    style(&mut doc, 0, CellRef::new(0, 1), "font.i", "true");
    set(&mut doc, 0, CellRef::new(0, 2), "3"); // C1
    style(&mut doc, 0, CellRef::new(0, 2), "font.u", "true");
    set(&mut doc, 0, CellRef::new(1, 0), "4"); // A2
    style(&mut doc, 0, CellRef::new(1, 0), "fill.fg_color", "#FF0000");
    set(&mut doc, 0, CellRef::new(1, 1), "5"); // B2
    style(&mut doc, 0, CellRef::new(1, 1), "font.color", "#0000FF");
    doc
}

/// Number-format families: currency, percent, and a date serial — the engine renders each
/// to its display string (round-3 B: display formatting is engine-owned).
pub fn number_formats() -> WorkbookDocument {
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    set(&mut doc, 0, CellRef::new(0, 0), "1234.5"); // A1
    style(&mut doc, 0, CellRef::new(0, 0), "num_fmt", "$#,##0.00"); // → "$1,234.50"
    set(&mut doc, 0, CellRef::new(0, 1), "1"); // B1
    style(&mut doc, 0, CellRef::new(0, 1), "num_fmt", "0.00%"); // → "100.00%"
    set(&mut doc, 0, CellRef::new(0, 2), "44197"); // C1 (Excel serial for 2021-01-01)
    style(&mut doc, 0, CellRef::new(0, 2), "num_fmt", "yyyy-mm-dd"); // → "2021-01-01"
    doc
}

/// Three sheets with a cross-sheet formula (`Sheet2!A1 = Sheet1!A1 * 2`).
pub fn multi_sheet() -> WorkbookDocument {
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    doc.user_model_mut().new_sheet().expect("add Sheet2");
    doc.user_model_mut().new_sheet().expect("add Sheet3");
    set(&mut doc, 0, CellRef::new(0, 0), "10"); // Sheet1!A1
    set(&mut doc, 1, CellRef::new(0, 0), "=Sheet1!A1*2"); // Sheet2!A1 → 20
    set(&mut doc, 2, CellRef::new(0, 0), "world"); // Sheet3!A1
    doc
}

/// [`multi_sheet`] with the second sheet renamed to "Data" — exercises sheet-rename
/// persistence through a save→reopen round-trip. The rename runs through the engine's undoable
/// `rename_sheet`; the cross-sheet formula (which references *Sheet1*) is unaffected.
pub fn multi_sheet_renamed() -> WorkbookDocument {
    let mut doc = multi_sheet();
    doc.user_model_mut()
        .rename_sheet(1, "Data")
        .expect("rename the second sheet");
    doc
}

/// A ring of `ring` cells where each references the next and the last wraps to the first,
/// so every cell resolves to `#CIRC!` (the round-3 D circular-reference reproducer;
/// validated to resolve in ms, never a hang). Built with evaluation paused so the ring is
/// assembled once and evaluated a single time.
pub fn circular_ref(ring: u32) -> WorkbookDocument {
    assert!(ring >= 2, "a circular ring needs at least two cells");
    let mut doc = WorkbookDocument::new_empty().expect("new empty workbook");
    doc.user_model_mut().pause_evaluation();
    for i in 0..ring {
        let next = (i + 1) % ring; // 0-based row of the referenced cell
        let formula = format!("=A{}", next + 1); // A1-notation is 1-based
        set(&mut doc, 0, CellRef::new(i, 0), &formula);
    }
    doc.user_model_mut().resume_evaluation();
    doc.user_model_mut().evaluate();
    doc
}
