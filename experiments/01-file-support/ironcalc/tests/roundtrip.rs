//! IronCalc `.xlsx`/CSV `load → edit → save → reload` round-trip probes.
//!
//! Each test reloads a file produced by committed code and asserts specific cells,
//! so the fidelity table in `../findings.md` is backed by passing tests (Sub-project
//! B guardrail: verify, don't assume). Small on purpose — fidelity, not perf.

use datagen::CellSource;
use ironcalc_base::cell::CellValue;
use ironcalc_file::{
    build_feature_model, feature, is_bold, load_csv_into_model, load_xlsx, model_to_csv,
    sheet_names, write_synthetic_xlsx, xlsx_bytes, DATE_SERIAL_2025_01_01, SHEET0,
};

/// Literal numbers and text survive an `.xlsx` round trip as values.
#[test]
fn xlsx_literals_survive() {
    let bytes = write_synthetic_xlsx(7, 12, 6).expect("write synthetic .xlsx");
    assert_eq!(&bytes[0..2], b"PK", ".xlsx is a ZIP (PK header)");

    let reloaded = load_xlsx(&bytes).expect("reload .xlsx");
    let source = datagen::SyntheticSheet::new(7, 12, 6);

    let mut checked_number = false;
    let mut checked_text = false;
    for r in 0..12 {
        for c in 0..6 {
            let got = reloaded
                .get_cell_value_by_index(SHEET0, (r + 1) as i32, (c + 1) as i32)
                .expect("read cell");
            match source.cell(r, c).value {
                datagen::CellValue::Empty => {}
                datagen::CellValue::Number(n) => {
                    assert_eq!(got, CellValue::Number(n), "number at ({r},{c}) survives");
                    checked_number = true;
                }
                datagen::CellValue::Text(t) => {
                    assert_eq!(got, CellValue::String(t), "text at ({r},{c}) survives");
                    checked_text = true;
                }
            }
        }
    }
    assert!(checked_number && checked_text, "sample covered both kinds");
}

/// A formula survives as its formula text, and — unlike Formualizer's write path —
/// IronCalc's native writer **persists the cached result**: the reloaded formula
/// value is already correct *before* any `evaluate()`. This is the symmetric
/// counterpart to the Formualizer test, which asserts a `None` value pre-eval, so
/// the "cached formula result" row of the findings table is test-backed on both
/// engines. A subsequent `evaluate()` (IronCalc's only recalc) keeps it correct.
#[test]
fn xlsx_formula_survives_and_recomputes() {
    let model = build_feature_model().expect("feature model");
    let bytes = xlsx_bytes(&model).expect("serialize");
    let mut reloaded = load_xlsx(&bytes).expect("reload");

    let (r, c) = feature::SUM_CELL;
    let f = reloaded
        .get_cell_formula(SHEET0, r, c)
        .expect("get formula")
        .expect("formula present");
    assert!(
        f.replace(' ', "").eq_ignore_ascii_case("=A1+A2"),
        "formula text survives, got {f:?}"
    );

    // Symmetric to the Formualizer probe: read the cached result BEFORE evaluate().
    // IronCalc's writer stores it, so it is already A1+A2 = 3.5 (Formualizer: None).
    assert_eq!(
        reloaded
            .get_cell_value_by_index(SHEET0, r, c)
            .expect("read formula value pre-eval"),
        CellValue::Number(3.5),
        "IronCalc persists the cached formula result across the round trip (pre-eval)"
    );

    // And a full evaluate() keeps it correct.
    reloaded.evaluate();
    assert_eq!(
        reloaded
            .get_cell_value_by_index(SHEET0, r, c)
            .expect("read formula value"),
        CellValue::Number(3.5),
        "formula still correct after IronCalc's full evaluate()"
    );
}

/// Multiple sheets survive, including a cross-sheet formula reference.
#[test]
fn xlsx_multiple_sheets_survive() {
    let model = build_feature_model().expect("feature model");
    let bytes = xlsx_bytes(&model).expect("serialize");
    let mut reloaded = load_xlsx(&bytes).expect("reload");

    let names = sheet_names(&reloaded);
    assert!(
        names.iter().any(|n| n == "Sheet1"),
        "Sheet1 present: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "Sheet2"),
        "Sheet2 present: {names:?}"
    );

    reloaded.evaluate();
    let (r, c) = feature::CROSS_CELL;
    // Sheet2 is index 1; cross-sheet formula = Sheet1!A1 * 10 = 10.
    assert_eq!(
        reloaded
            .get_cell_value_by_index(1, r, c)
            .expect("read cross-sheet value"),
        CellValue::Number(10.0),
        "cross-sheet formula recomputes"
    );
}

/// A date survives as its serial value AND keeps its number format, so IronCalc
/// still renders it as a date string after reload. This is a concrete IronCalc
/// advantage over the calamine read path (deep style fidelity stays with D).
#[test]
fn xlsx_date_number_format_survives() {
    let model = build_feature_model().expect("feature model");
    let bytes = xlsx_bytes(&model).expect("serialize");
    let reloaded = load_xlsx(&bytes).expect("reload");

    let (r, c) = feature::DATE_CELL;
    // Underlying value is the serial.
    assert_eq!(
        reloaded
            .get_cell_value_by_index(SHEET0, r, c)
            .expect("read date value"),
        CellValue::Number(DATE_SERIAL_2025_01_01),
        "date serial survives as a number"
    );
    // Displayed value is a date string (the number format survived the round trip).
    let shown = reloaded
        .get_formatted_cell_value(SHEET0, r, c)
        .expect("formatted date");
    assert!(
        shown.contains("2025") && shown.contains("01"),
        "date renders via its surviving number format, got {shown:?}"
    );
}

/// Shallow style-survival probe: a bold cell is still bold after the native writer's
/// round trip. Depth (all style attributes, fidelity) is Sub-project D — here we
/// only confirm IronCalc's writer carries styles at all.
#[test]
fn xlsx_styles_survive_shallow() {
    let model = build_feature_model().expect("feature model");
    let bytes = xlsx_bytes(&model).expect("serialize");
    let reloaded = load_xlsx(&bytes).expect("reload");

    let (r, c) = feature::BOLD_CELL;
    assert!(
        is_bold(&reloaded, SHEET0, r, c).expect("read bold"),
        "bold survives the native .xlsx round trip"
    );
}

/// The DIY CSV bridge round-trips values: generated CSV → model → CSV → values match.
/// Documents that CSV works on IronCalc, but only because we built the bridge.
#[test]
fn csv_bridge_roundtrips_values() {
    let sheet = datagen::SyntheticSheet::new(3, 8, 4);
    let csv_in = datagen::csv_string(&sheet, 4, 3);

    let model = load_csv_into_model(&csv_in).expect("load CSV into model");
    let csv_out = model_to_csv(&model, SHEET0, 4, 3);

    // Compare field-by-field against the source generator (numbers may reformat, so
    // compare parsed numeric values where the source is numeric, else the string).
    for r in 0..4u32 {
        for c in 0..3u32 {
            let got = model
                .get_cell_value_by_index(SHEET0, (r + 1) as i32, (c + 1) as i32)
                .expect("read cell");
            match sheet.cell(r, c).value {
                datagen::CellValue::Empty => {
                    assert_eq!(got, CellValue::None, "empty stays empty at ({r},{c})");
                }
                datagen::CellValue::Number(n) => {
                    assert_eq!(got, CellValue::Number(n), "number survives at ({r},{c})");
                }
                datagen::CellValue::Text(t) => {
                    assert_eq!(got, CellValue::String(t), "text survives at ({r},{c})");
                }
            }
        }
    }
    // And the exported CSV is non-empty / has the right number of rows.
    assert_eq!(csv_out.lines().count(), 4, "exported CSV has 4 rows");
}

/// Records the produced `.xlsx` byte sizes so the figures quoted in `../findings.md`
/// are regenerable from committed code (functional_spec §5.3). Run with
/// `cargo test --test roundtrip records_xlsx_byte_sizes -- --nocapture` to print
/// them. Asserts only that the writer produced a non-trivial, valid OOXML file
/// (exact sizes are engine-version-dependent, so we don't hard-code them here).
#[test]
fn records_xlsx_byte_sizes() {
    let feature_bytes =
        xlsx_bytes(&build_feature_model().expect("feature model")).expect("serialize feature");
    let synthetic_bytes = write_synthetic_xlsx(7, 100, 20).expect("synthetic 100x20 .xlsx");

    println!("ironcalc feature .xlsx bytes = {}", feature_bytes.len());
    println!(
        "ironcalc synthetic 100x20 .xlsx bytes = {}",
        synthetic_bytes.len()
    );

    assert_eq!(&feature_bytes[0..2], b"PK", "feature file is a ZIP");
    assert_eq!(&synthetic_bytes[0..2], b"PK", "synthetic file is a ZIP");
    assert!(feature_bytes.len() > 1_000, "feature file is non-trivial");
    assert!(synthetic_bytes.len() > 1_000, "synthetic file is non-trivial");
}
