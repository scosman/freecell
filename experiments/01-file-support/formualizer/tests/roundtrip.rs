//! Formualizer `.xlsx`/CSV `load → edit → save → reload` round-trip probes.
//!
//! Each test reloads a file produced by committed code and asserts specific cells,
//! so the fidelity table in `../findings.md` is backed by passing tests (Sub-project
//! B guardrail: verify, don't assume). Row/col counts are small on purpose — this is
//! a *fidelity* study, not a perf study (perf is Sub-project C).

use datagen::CellSource;
use formualizer::LiteralValue;
use formualizer_file::{
    as_f64, build_feature_workbook, feature, load_csv, load_xlsx, workbook_to_csv,
    write_synthetic_xlsx, xlsx_bytes, DATE_SERIAL_2025_01_01, SHEET1, SHEET2,
};

/// Literal numbers and text survive an `.xlsx` round trip as values.
#[test]
fn xlsx_literals_survive() {
    // A small synthetic block; pick a seed/size with known non-empty cells to check.
    let bytes = write_synthetic_xlsx(7, 12, 6).expect("write synthetic .xlsx");
    assert_eq!(&bytes[0..2], b"PK", ".xlsx is a ZIP (PK header)");

    let reloaded = load_xlsx(&bytes).expect("reload .xlsx");
    let source = datagen::SyntheticSheet::new(7, 12, 6);

    // Compare every non-empty generated cell against the reloaded value.
    let mut checked_number = false;
    let mut checked_text = false;
    for r in 0..12 {
        for c in 0..6 {
            match source.cell(r, c).value {
                datagen::CellValue::Empty => {}
                datagen::CellValue::Number(n) => {
                    let got = reloaded.get_value(SHEET1, r + 1, c + 1);
                    assert_eq!(
                        got.as_ref().and_then(as_f64),
                        Some(n),
                        "number at ({r},{c}) should survive"
                    );
                    checked_number = true;
                }
                datagen::CellValue::Text(t) => {
                    let got = reloaded.get_value(SHEET1, r + 1, c + 1);
                    assert_eq!(
                        got,
                        Some(LiteralValue::Text(t.clone())),
                        "text at ({r},{c}) should survive"
                    );
                    checked_text = true;
                }
            }
        }
    }
    assert!(checked_number && checked_text, "sample covered both kinds");
}

/// A formula survives as its **formula text**, and its cached result is dropped by
/// the umya write path; re-evaluation after reload restores the value. This
/// regression-locks the headline Formualizer file finding.
#[test]
fn xlsx_formula_survives_as_formula_not_cached() {
    let wb = build_feature_workbook().expect("feature workbook");
    let bytes = xlsx_bytes(&wb).expect("serialize");
    let mut reloaded = load_xlsx(&bytes).expect("reload");

    let (r, c) = feature::SUM_CELL;
    // No cached value after reload.
    assert!(
        reloaded.get_value(SHEET1, r, c).is_none(),
        "cached formula result is dropped by the write path"
    );
    // Formula text survives.
    let f = reloaded
        .get_formula(SHEET1, r, c)
        .expect("formula text survives");
    assert!(
        f.replace(' ', "").eq_ignore_ascii_case("=A1+A2"),
        "formula text survives, got {f:?}"
    );
    // Re-evaluates correctly (A1=1 + A2=2.5 = 3.5).
    reloaded.prepare_graph_all().expect("prepare graph");
    let v = reloaded.evaluate_cell(SHEET1, r, c).expect("re-eval");
    assert_eq!(as_f64(&v), Some(3.5), "formula recomputes after reload");
}

/// Multiple sheets survive, including a cross-sheet formula reference.
#[test]
fn xlsx_multiple_sheets_survive() {
    let wb = build_feature_workbook().expect("feature workbook");
    let bytes = xlsx_bytes(&wb).expect("serialize");
    let mut reloaded = load_xlsx(&bytes).expect("reload");

    let names = reloaded.sheet_names();
    assert!(
        names.iter().any(|n| n == SHEET1),
        "Sheet1 present: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == SHEET2),
        "Sheet2 present: {names:?}"
    );

    // Cross-sheet formula survives as text and recomputes to Sheet1!A1 * 10 = 10.
    let (r, c) = feature::CROSS_CELL;
    let f = reloaded
        .get_formula(SHEET2, r, c)
        .expect("cross-sheet formula survives");
    assert!(
        f.replace(' ', "").eq_ignore_ascii_case("=Sheet1!A1*10"),
        "cross-sheet formula text survives, got {f:?}"
    );
    reloaded.prepare_graph_all().expect("prepare graph");
    let v = reloaded.evaluate_cell(SHEET2, r, c).expect("re-eval cross");
    assert_eq!(as_f64(&v), Some(10.0), "cross-sheet formula recomputes");
}

/// A date is stored/round-tripped as its numeric serial (number-format "date-ness"
/// is Sub-project D's concern; here only the value must survive).
#[test]
fn xlsx_date_serial_survives_as_number() {
    let wb = build_feature_workbook().expect("feature workbook");
    let bytes = xlsx_bytes(&wb).expect("serialize");
    let reloaded = load_xlsx(&bytes).expect("reload");

    let (r, c) = feature::DATE_CELL;
    let v = reloaded
        .get_value(SHEET1, r, c)
        .expect("date cell survives");
    assert_eq!(
        as_f64(&v),
        Some(DATE_SERIAL_2025_01_01),
        "date serial survives as a number"
    );
}

/// CSV import via `CsvAdapter` reads generated values into a single sheet.
#[test]
fn csv_import_reads_values() {
    let sheet = datagen::SyntheticSheet::new(3, 8, 4);
    let csv = datagen::csv_string(&sheet, 4, 3);
    let wb = load_csv(&csv).expect("load CSV");

    let names = wb.sheet_names();
    assert!(!names.is_empty(), "CSV import yields a sheet");
    let dims = wb
        .sheet_dimensions(&names[0])
        .expect("imported sheet has dimensions");
    assert!(
        dims.0 >= 4 && dims.1 >= 3,
        "CSV sheet at least 4x3, got {dims:?}"
    );
}

/// Values survive an export → re-parse cycle: build a workbook, export to CSV, load
/// that CSV back, and compare the numeric cells.
#[test]
fn csv_export_roundtrips_values() {
    // Build a workbook of pure numbers so the comparison is exact (text escaping is
    // covered by the datagen crate's own tests).
    let mut wb = formualizer::Workbook::new();
    let expected = [[1.0, 2.0, 3.0], [10.5, 20.25, 30.125]];
    for (r, row) in expected.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            wb.set_value(SHEET1, r as u32 + 1, c as u32 + 1, LiteralValue::Number(v))
                .expect("set");
        }
    }

    let csv = workbook_to_csv(&wb, SHEET1, 2, 3);
    let reloaded = load_csv(&csv).expect("reload exported CSV");
    for (r, row) in expected.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            let got = reloaded.get_value(SHEET1, r as u32 + 1, c as u32 + 1);
            assert_eq!(
                got.as_ref().and_then(as_f64),
                Some(v),
                "cell ({r},{c}) survives CSV export+import"
            );
        }
    }
}

/// Records the produced `.xlsx` byte sizes so the figures quoted in `../findings.md`
/// are regenerable from committed code (functional_spec §5.3). Run with
/// `cargo test --test roundtrip records_xlsx_byte_sizes -- --nocapture` to print
/// them. Asserts only that the writer produced a non-trivial, valid OOXML file
/// (exact sizes are engine-version-dependent, so we don't hard-code them here).
#[test]
fn records_xlsx_byte_sizes() {
    let feature_bytes = xlsx_bytes(&build_feature_workbook().expect("feature workbook"))
        .expect("serialize feature workbook");
    let synthetic_bytes = write_synthetic_xlsx(7, 100, 20).expect("synthetic 100x20 .xlsx");

    println!("formualizer feature .xlsx bytes = {}", feature_bytes.len());
    println!(
        "formualizer synthetic 100x20 .xlsx bytes = {}",
        synthetic_bytes.len()
    );

    assert_eq!(&feature_bytes[0..2], b"PK", "feature file is a ZIP");
    assert_eq!(&synthetic_bytes[0..2], b"PK", "synthetic file is a ZIP");
    assert!(feature_bytes.len() > 1_000, "feature file is non-trivial");
    assert!(synthetic_bytes.len() > 1_000, "synthetic file is non-trivial");
}
