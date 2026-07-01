//! Formualizer 0.7 formatting-exposure probes (Sub-project D).
//!
//! These are the *verification* behind the findings: each capability claim is backed by
//! a small passing assertion (read a known-styled cell; assert the value). They exercise
//! the real crates (`formualizer` + `umya_spreadsheet`), not our matrix strings.

use formualizer::workbook::{CalamineAdapter, SpreadsheetReader};
use formualizer_formatting::{
    build_styled_xlsx_bytes, load_into_formualizer, load_into_umya, read_col_width_via_umya,
    read_format_via_umya, read_merges_via_umya, read_row_height_via_umya, save_umya, SHEET,
};

/// The decisive gap, locked structurally: Formualizer's `.xlsx` **read** path surfaces
/// no styles. The calamine backend both advertises `capabilities().styles == false`
/// AND returns `CellData { style: None, .. }` for a cell that is bold in the source file.
#[test]
fn celldata_style_is_none_on_read() {
    let bytes = build_styled_xlsx_bytes();
    // A1 is bold + filled + number-formatted in the fixture; verify that first via umya
    // so the test can't silently pass on an unstyled fixture.
    let umya = load_into_umya(&bytes);
    assert!(
        read_format_via_umya(&umya, "A1").bold,
        "fixture A1 must actually be bold for this probe to be meaningful"
    );

    // Now read the SAME cell through Formualizer's calamine backend read path.
    let mut adapter = CalamineAdapter::open_bytes(bytes.clone()).expect("open via calamine");
    assert!(
        !adapter.capabilities().styles,
        "calamine backend advertises styles == false"
    );
    let cell = adapter
        .read_cell(SHEET, 1, 1)
        .expect("read A1")
        .expect("A1 has a value");
    assert!(
        cell.value.is_some(),
        "the VALUE is surfaced through the read path"
    );
    assert!(
        cell.style.is_none(),
        "but the STYLE is not — CellData.style is None even though A1 is bold+filled"
    );
}

/// Values survive `Workbook::to_xlsx_bytes`, but **styles are dropped**: the engine's
/// write path builds a fresh umya file from values/formulas only, and there is no bridge
/// to carry a loaded file's styles through it. We prove the drop end-to-end: load a
/// styled file into Formualizer, save via the engine, reload, and observe the value is
/// present but the bold/fill/number-format are gone.
#[test]
fn values_survive_but_styles_dropped_through_to_xlsx_bytes() {
    let bytes = build_styled_xlsx_bytes();

    // Sanity: the source file really is styled.
    let src = load_into_umya(&bytes);
    assert!(read_format_via_umya(&src, "A1").bold);

    // Round-trip the VALUES through the Formualizer engine.
    let wb = load_into_formualizer(&bytes);
    let saved = wb.to_xlsx_bytes().expect("engine to_xlsx_bytes");

    // Reload the engine's output via umya and inspect the same cell.
    let reloaded = load_into_umya(&saved);
    let fmt = read_format_via_umya(&reloaded, "A1");

    // The value made it through the engine...
    let sheet = reloaded.get_sheet_by_name(SHEET).expect("sheet");
    assert_eq!(
        sheet.get_value("A1"),
        "12.5",
        "the numeric value survives the engine round-trip"
    );
    // ...but every style attribute was dropped.
    assert!(
        !fmt.bold && fmt.fill_argb.is_none() && fmt.number_format.is_none(),
        "styles are dropped by to_xlsx_bytes (got {fmt:?})"
    );
}

/// The umya-direct read path DOES surface every attribute FreeCell cares about. This is
/// the style source of truth FreeCell would keep alongside a Formualizer workbook.
#[test]
fn styles_readable_via_umya_directly() {
    let bytes = build_styled_xlsx_bytes();
    let book = load_into_umya(&bytes);

    let a1 = read_format_via_umya(&book, "A1");
    assert!(a1.bold, "A1 bold");
    assert_eq!(a1.font_size, Some(16.0), "A1 16pt");
    assert_eq!(a1.fill_argb.as_deref(), Some("FFFFFF00"), "A1 yellow fill");
    assert_eq!(
        a1.number_format.as_deref(),
        Some("0.00"),
        "A1 number format"
    );

    let b2 = read_format_via_umya(&book, "B2");
    assert!(b2.italic, "B2 italic");
    assert!(!b2.bold, "B2 not bold");

    // Metadata: row height, column width, merges.
    assert_eq!(
        read_row_height_via_umya(&book, 1),
        Some(40.0),
        "row 1 height"
    );
    assert_eq!(
        read_col_width_via_umya(&book, "A"),
        Some(30.0),
        "col A width"
    );
    assert_eq!(
        read_merges_via_umya(&book),
        vec!["C3:D3".to_string()],
        "merge range preserved"
    );
}

/// The load → edit → save → reload verdict for the umya-direct path: format edits
/// survive AND the pre-existing attributes are re-read *after* the round-trip (not just
/// off the original fixture), so the matrix's "round-trip = ViaUmya" cells for
/// bold/italic/fill/font_size/number_format/row_height/col_width/merges are all
/// genuinely probe-backed, not merely inferred.
#[test]
fn umya_style_edit_survives_roundtrip() {
    let bytes = build_styled_xlsx_bytes();
    let mut book = load_into_umya(&bytes);

    // Edit: make B2 bold and cyan.
    {
        let sheet = book.get_sheet_by_name_mut(SHEET).expect("sheet");
        let style = sheet.get_cell_mut("B2").get_style_mut();
        style.get_font_mut().set_bold(true);
        style.set_background_color_solid("FF00FFFF");
    }

    let saved = save_umya(&book);
    let reloaded = load_into_umya(&saved);

    // The edited cell.
    let b2 = read_format_via_umya(&reloaded, "B2");
    assert!(b2.bold, "edited bold survived the round-trip");
    assert!(b2.italic, "pre-existing italic still present");
    assert_eq!(
        b2.fill_argb.as_deref(),
        Some("FF00FFFF"),
        "edited fill survived the round-trip"
    );

    // Re-read the full A1 attribute set AFTER save+reload — this is what makes
    // font_size / number_format (and, with the metadata below, row/col size + merges)
    // true round-trip evidence rather than a read off the original in-memory fixture.
    let a1 = read_format_via_umya(&reloaded, "A1");
    assert!(a1.bold, "A1 bold survived round-trip");
    assert_eq!(a1.font_size, Some(16.0), "A1 font size survived round-trip");
    assert_eq!(
        a1.fill_argb.as_deref(),
        Some("FFFFFF00"),
        "A1 fill survived round-trip"
    );
    assert_eq!(
        a1.number_format.as_deref(),
        Some("0.00"),
        "A1 number format survived round-trip"
    );

    // Metadata survives the round-trip too.
    assert_eq!(
        read_row_height_via_umya(&reloaded, 1),
        Some(40.0),
        "row 1 height survived round-trip"
    );
    assert_eq!(
        read_col_width_via_umya(&reloaded, "A"),
        Some(30.0),
        "col A width survived round-trip"
    );
    assert_eq!(
        read_merges_via_umya(&reloaded),
        vec!["C3:D3".to_string()],
        "merge survived round-trip"
    );
}
