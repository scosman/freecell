//! IronCalc 0.7 formatting-exposure probes (Sub-project D).
//!
//! Each capability claim is backed by a small passing assertion against the real
//! `ironcalc` / `ironcalc_base` crates. Where IronCalc has no public API for an
//! attribute (merges, conditional formatting), the probe documents the absence rather
//! than asserting a false capability.

use ironcalc_base::Model;
use ironcalc_formatting::{
    new_model, read_format, roundtrip_via_xlsx, styled_model, NeutralFormat, SHEET,
};

/// Styles are readable AND writable natively: set bold+italic+size+fill+num_fmt via
/// `set_cell_style`, read them straight back with `get_style_for_cell`.
#[test]
fn styles_read_and_write_natively() {
    let model = styled_model();

    let a1 = read_format(&model, SHEET, 1, 1);
    assert_eq!(
        a1,
        NeutralFormat {
            bold: true,
            italic: false,
            font_size: Some(16.0),
            fill_argb: Some("#FFFF00".to_string()),
            number_format: Some("0.00".to_string()),
        },
        "A1 style read back exactly as written"
    );

    let b2 = read_format(&model, SHEET, 2, 2);
    assert!(b2.italic, "B2 italic");
    assert!(!b2.bold, "B2 not bold");
}

/// Row heights and column widths are first-class on `Model` and read back after being
/// set (no external metadata store required for sizing).
#[test]
fn row_col_sizes_settable() {
    let model = styled_model();
    assert_eq!(
        model.get_row_height(SHEET, 1).expect("row 1 height"),
        40.0,
        "row height read back"
    );
    assert_eq!(
        model.get_column_width(SHEET, 1).expect("col A width"),
        30.0,
        "column width read back"
    );
}

/// The load → edit → save → reload verdict: styles survive a real `.xlsx` round-trip
/// through IronCalc's own import/export. Build a styled model, round-trip through xlsx
/// bytes, and confirm bold/italic/fill/number-format all survive.
#[test]
fn styles_survive_xlsx_roundtrip() {
    let model = styled_model();
    let reloaded = roundtrip_via_xlsx(&model);

    let a1 = read_format(&reloaded, SHEET, 1, 1);
    assert!(a1.bold, "A1 bold survived xlsx round-trip");
    assert_eq!(a1.font_size, Some(16.0), "A1 16pt survived");
    assert_eq!(a1.fill_argb.as_deref(), Some("#FFFF00"), "A1 fill survived");
    assert_eq!(
        a1.number_format.as_deref(),
        Some("0.00"),
        "A1 number format survived"
    );

    let b2 = read_format(&reloaded, SHEET, 2, 2);
    assert!(b2.italic, "B2 italic survived xlsx round-trip");

    // Row/column sizing also survives the round-trip.
    assert_eq!(
        reloaded.get_row_height(SHEET, 1).expect("row height"),
        40.0,
        "row height survived"
    );
}

/// Editing a style after load also survives a further round-trip: read A1's style, flip
/// bold off + change the fill, save, reload, confirm the edit stuck.
#[test]
fn style_edit_survives_second_roundtrip() {
    let mut model = styled_model();
    {
        let mut s = model.get_style_for_cell(SHEET, 1, 1).expect("A1 style");
        s.font.b = false;
        s.fill.pattern_type = "solid".to_string();
        s.fill.fg_color = Some("#00FFFF".to_string());
        model.set_cell_style(SHEET, 1, 1, &s).expect("edit A1");
    }
    let reloaded = roundtrip_via_xlsx(&model);
    let a1 = read_format(&reloaded, SHEET, 1, 1);
    assert!(!a1.bold, "bold edit (off) survived");
    assert_eq!(
        a1.fill_argb.as_deref(),
        Some("#00FFFF"),
        "fill edit survived"
    );
}

/// Documents the IronCalc gaps for the matrix: there is no public merged-cells API and
/// no conditional-formatting API on `Model` in 0.7. We assert only what the public
/// surface guarantees (a blank model has no styling), so the matrix's `None` entries are
/// grounded in the absence of methods, not a false negative.
#[test]
fn merges_and_conditional_formatting_absent_from_public_api() {
    // A fresh model has a default (unstyled) A1 — the baseline the matrix rests on.
    let model = new_model();
    let a1 = read_format(&model, SHEET, 1, 1);
    assert!(!a1.bold && a1.fill_argb.is_none());

    // There is intentionally no `model.add_merge_cells(..)` / conditional-formatting
    // call here: those methods do not exist in ironcalc 0.7's public API. This test's
    // existence + the matrix note (attribute = "merges"/"conditional_formatting",
    // support = None) is the documented record of that gap. A compile-time attempt to
    // call them would fail to build, which is the strongest possible proof of absence.
    let _ = Model::new_empty("probe", "en", "UTC", "en").expect("model builds");
}
