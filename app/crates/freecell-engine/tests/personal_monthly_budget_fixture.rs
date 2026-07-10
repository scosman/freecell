//! Style-fidelity regression tests for the "Personal Monthly Budget" Excel template
//! (`tests/fixtures/personal_monthly_budget.xlsx`, an unmodified copy of a real user file).
//!
//! The template exercised a fidelity gap when opened in FreeCell: its Century Gothic title
//! rendered as the default face, and its category tables (HOUSING, ENTERTAINMENT, …) lost
//! their teal section-header fills, white-bold header text, thin data-cell borders, and bold
//! Subtotal rows. Diagnosis split the losses across two engine layers:
//!
//! 1. **Font name** — the IronCalc styles importer discarded every `<font><name val="…"/>`,
//!    hardcoding the default. Fixed in our fork (`fix/font-name-import`): the title font now
//!    resolves to "Century Gothic". Locked in by [`title_resolves_century_gothic`].
//!
//! 2. **Excel table styles** — the teal fills / data borders / bold totals come from the
//!    workbook's custom table style ("Personal monthly budget") via `<tableStyles>`/`dxfs`,
//!    NOT from the cells' own styles. IronCalc parses the table geometry but does not resolve
//!    or apply table-style dxfs in `get_style_for_cell`, so these cells resolve unstyled. That
//!    is a larger engine feature (see the task roadblock); the target behaviour is captured by
//!    the `#[ignore]`d [`table_styles`] tests so they flip green once the engine gains support.
//!
//! Cells that carry *direct* fills/borders (the summary "Total monthly income" / balance boxes)
//! already render correctly — [`direct_gray_fill_resolves`] guards that they stay working.
//!
//! The style assertions read IronCalc's authoritative `get_style_for_cell` — the exact resolver
//! FreeCell's publish + cache paths consume (`document.rs::published_style`, `cache.rs`).

use std::path::PathBuf;

use freecell_core::CellRef;
use freecell_engine::WorkbookDocument;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::Model;

/// 0-based index of the "PERSONAL MONTHLY BUDGET" worksheet (the workbook's second sheet,
/// after "START").
const BUDGET_SHEET: u32 = 1;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/personal_monthly_budget.xlsx")
}

/// Loads the fixture through the raw engine so tests can read fully-resolved cell styles.
fn engine_model() -> Model<'static> {
    load_from_xlsx(fixture().to_str().expect("utf-8 path"), "en", "UTC", "en")
        .expect("the budget template must load")
}

/// `A1`-style reference → IronCalc's 1-based `(row, column)`.
fn rc(cell: &str) -> (i32, i32) {
    let col_str: String = cell
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let row_str: String = cell
        .chars()
        .skip_while(|c| c.is_ascii_alphabetic())
        .collect();
    let col = col_str
        .chars()
        .fold(0i32, |acc, c| acc * 26 + (c as i32 - 'A' as i32 + 1));
    (row_str.parse().expect("row number"), col)
}

#[test]
fn opens_through_freecell_document() {
    let doc = WorkbookDocument::open(&fixture()).expect("budget template opens via FreeCell");
    assert_eq!(
        doc.sheet_names(),
        vec!["START".to_string(), "PERSONAL MONTHLY BUDGET".to_string()]
    );
    // The Century Gothic title text (sharedString) lands in B2 of the budget sheet.
    assert_eq!(
        doc.formatted_value(BUDGET_SHEET, CellRef::new(1, 1))
            .expect("B2 in range"),
        "PERSONAL MONTHLY BUDGET"
    );
}

#[test]
fn title_resolves_century_gothic() {
    // B2 is the 22pt Century Gothic title. Before the fork's font-name import fix this resolved
    // to the default face; it must now carry the real typeface end-to-end.
    let model = engine_model();
    let (row, col) = rc("B2");
    let style = model
        .get_style_for_cell(BUDGET_SHEET, row, col)
        .expect("B2 style");
    assert_eq!(style.font.name, "Century Gothic", "title font family");
    assert_eq!(style.font.sz, 22, "title font size");
}

#[test]
fn every_font_name_survives_import() {
    // Guard the general property behind the title fix: the workbook uses Century Gothic and
    // Calibri; neither must collapse to the importer's default "Inter".
    let model = engine_model();
    let names: Vec<&str> = model
        .workbook
        .styles
        .fonts
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&"Century Gothic"), "got {names:?}");
    assert!(names.contains(&"Calibri"), "got {names:?}");
    assert!(
        !names.contains(&"Inter"),
        "no font in this workbook is Inter; a stray Inter means the name was dropped: {names:?}"
    );
}

#[test]
fn direct_gray_fill_resolves() {
    // The "Total monthly income" box (E6) and the "PROJECTED BALANCE" box (J4) carry a *direct*
    // solid fill (styles.xml fill index 2, theme background + negative tint) and a bold face.
    // These already rendered correctly (the gray-yes / teal-no diagnostic); guard them.
    let model = engine_model();
    for cell in ["E6", "J4"] {
        let (row, col) = rc(cell);
        let style = model
            .get_style_for_cell(BUDGET_SHEET, row, col)
            .unwrap_or_else(|_| panic!("{cell} style"));
        assert!(
            style.fill.color.is_some(),
            "{cell} must keep its direct gray fill"
        );
        assert!(style.font.b, "{cell} total is bold");
    }
}

// ---------------------------------------------------------------------------------------------
// Excel table-style fidelity — currently unsupported by the engine (see the task roadblock).
// These assert the *target* behaviour and are ignored until IronCalc resolves & applies table
// styles; remove the `#[ignore]` when that lands.
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "blocked: IronCalc does not apply Excel table styles (roadblock)"]
fn table_style_header_is_teal_white_bold() {
    // B12 = the HOUSING section header (a table header row). The custom table style's headerRow
    // dxf paints a dark-teal fill + white bold text. The cell carries no direct style, so today
    // it resolves unstyled.
    let model = engine_model();
    let (row, col) = rc("B12");
    let style = model.get_style_for_cell(BUDGET_SHEET, row, col).unwrap();
    assert!(style.fill.color.is_some(), "header teal fill");
    assert!(style.font.b, "header text is bold");
    assert!(
        style.font.color.is_some(),
        "header text is white (not default)"
    );
}

#[test]
#[ignore = "blocked: IronCalc does not apply Excel table styles (roadblock)"]
fn table_style_data_cells_have_borders() {
    // C13 = a Housing data cell. The table's wholeTable dxf draws thin borders on every side.
    let model = engine_model();
    let (row, col) = rc("C13");
    let style = model.get_style_for_cell(BUDGET_SHEET, row, col).unwrap();
    assert!(
        style.border.top.is_some() && style.border.bottom.is_some(),
        "data cells are boxed by the table style"
    );
}

#[test]
#[ignore = "blocked: IronCalc does not apply Excel table styles (roadblock)"]
fn table_style_subtotal_row_is_bold() {
    // B23 = the Housing "Subtotal" (a table totals row). The totalRow dxf makes it bold.
    let model = engine_model();
    let (row, col) = rc("B23");
    let style = model.get_style_for_cell(BUDGET_SHEET, row, col).unwrap();
    assert!(style.font.b, "subtotal row is bold");
}
