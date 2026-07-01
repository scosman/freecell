//! Formualizer 0.7 smoke probes for FreeCell Phase 1 (Sub-project A gate).
//!
//! Each test is a probe of a specific API area the later phases (B–E) depend on.
//! The assertions confirm real behavior; the comments explain what the probe
//! establishes for the gate. If an assumed API were missing, the crate would fail
//! to compile — that too is a finding, recorded in `findings.md`.

use formualizer::LiteralValue;
use formualizer::workbook::{CalamineAdapter, SpreadsheetReader};
use smoke::{
    DEFAULT_SHEET, build_sum_workbook, config_with_parallel_eval, load_csv_str, load_xlsx_bytes,
    new_workbook_with_changelog, xlsx_bytes_from,
};

/// Extracts a numeric value regardless of whether the engine returned it as an
/// `Int` or a `Number` (both are valid engine representations of "3").
fn as_f64(v: &LiteralValue) -> Option<f64> {
    match v {
        LiteralValue::Int(i) => Some(*i as f64),
        LiteralValue::Number(n) => Some(*n),
        _ => None,
    }
}

/// PROBE: single-cell evaluation of a dependent cell, and recalculation after an
/// edit to a precedent. This is the most load-bearing engine behavior for FreeCell.
#[test]
fn builds_and_evaluates_dependent_cell() {
    let mut wb = build_sum_workbook().expect("build workbook");

    // A3 = A1 + A2 = 1 + 2 = 3
    let a3 = wb.evaluate_cell(DEFAULT_SHEET, 3, 1).expect("eval A3");
    assert_eq!(as_f64(&a3), Some(3.0), "A3 should evaluate to 1 + 2");

    // Mutate a precedent and confirm the dependent recomputes (recalc confirmed).
    wb.set_value(DEFAULT_SHEET, 1, 1, LiteralValue::Int(10))
        .expect("set A1 = 10");
    let a3_after = wb
        .evaluate_cell(DEFAULT_SHEET, 3, 1)
        .expect("re-eval A3 after edit");
    assert_eq!(
        as_f64(&a3_after),
        Some(12.0),
        "A3 should recompute to 10 + 2 after editing A1"
    );
}

/// PROBE: bulk / range reads. `read_range` returns a 2D grid via the columnar
/// range view; `evaluate_cells` batches evaluation. Both matter for the
/// scroll-viewport binding pattern (Sub-project C).
#[test]
fn range_bulk_read_returns_grid() {
    use formualizer::RangeAddress;

    let mut wb = build_sum_workbook().expect("build workbook");
    // Ensure the formula cell has a materialized value before the bulk read.
    wb.evaluate_all().expect("evaluate all");

    // read_range over A1:A3 (1-based, inclusive) returns 3 rows x 1 col.
    let addr = RangeAddress::new(DEFAULT_SHEET, 1, 1, 3, 1).expect("valid range");
    let grid = wb.read_range(&addr);
    assert_eq!(grid.len(), 3, "range height should be 3 rows");
    assert!(grid.iter().all(|row| row.len() == 1), "each row is 1 col");
    assert_eq!(as_f64(&grid[0][0]), Some(1.0), "A1 == 1");
    assert_eq!(as_f64(&grid[1][0]), Some(2.0), "A2 == 2");
    assert_eq!(as_f64(&grid[2][0]), Some(3.0), "A3 == 3");

    // evaluate_cells: batch evaluation, order preserved.
    let batch = wb
        .evaluate_cells(&[(DEFAULT_SHEET, 1, 1), (DEFAULT_SHEET, 3, 1)])
        .expect("batch eval");
    assert_eq!(batch.len(), 2, "batch returns one value per target");
    assert_eq!(as_f64(&batch[0]), Some(1.0));
    assert_eq!(as_f64(&batch[1]), Some(3.0));
}

/// PROBE: `.xlsx` write + read round trip. Build a workbook, serialize to `.xlsx`
/// bytes via umya, reload via calamine, confirm values survive. No committed
/// binary fixture — the file is produced by committed code (functional_spec §5.3).
#[test]
fn xlsx_roundtrip_via_umya_and_calamine() {
    let mut wb = build_sum_workbook().expect("build workbook");
    // Materialize the formula's value so it is written into the .xlsx.
    wb.evaluate_all().expect("evaluate all");

    let bytes = xlsx_bytes_from(&wb).expect("serialize to .xlsx bytes");
    assert!(!bytes.is_empty(), ".xlsx bytes should be non-empty");
    // XLSX is a ZIP; sanity-check the magic bytes.
    assert_eq!(&bytes[0..2], b"PK", ".xlsx should be a ZIP (PK header)");

    let reloaded = load_xlsx_bytes(&bytes).expect("reload .xlsx via calamine");
    // Literal values survive the round trip.
    let a1 = reloaded
        .get_value(DEFAULT_SHEET, 1, 1)
        .expect("A1 present after reload");
    assert_eq!(as_f64(&a1), Some(1.0), "A1 literal survives .xlsx round trip");
    // The evaluated formula result was written as a cached value and survives.
    let a3 = reloaded
        .get_value(DEFAULT_SHEET, 3, 1)
        .expect("A3 present after reload");
    assert_eq!(
        as_f64(&a3),
        Some(3.0),
        "A3 cached formula result survives .xlsx round trip"
    );
}

/// PROBE: CSV import. Generate a tiny CSV from the shared, engine-neutral
/// `datagen` generator (committed code) and load it via the CSV backend.
#[test]
fn csv_load_reads_values() {
    // A deterministic 3x2 CSV: two numeric-ish columns of synthetic data.
    let sheet = datagen::SyntheticSheet::new(7, 8, 4);
    let csv = datagen::csv_string(&sheet, 3, 2);
    assert_eq!(csv.lines().count(), 3, "generated CSV should have 3 rows");

    let wb = load_csv_str(&csv).expect("load CSV");
    let names = wb.sheet_names();
    assert!(!names.is_empty(), "CSV import should yield at least one sheet");

    // The imported sheet should have 3 rows x 2 cols of data (1-based dims).
    let dims = wb
        .sheet_dimensions(&names[0])
        .expect("imported sheet has dimensions");
    assert!(
        dims.0 >= 3 && dims.1 >= 2,
        "CSV sheet should be at least 3x2, got {dims:?}"
    );

    // Every generated cell should read back as a non-empty value.
    let top_left = wb
        .get_value(&names[0], 1, 1)
        .expect("CSV cell (1,1) present");
    assert!(
        !matches!(top_left, LiteralValue::Empty),
        "CSV cell (1,1) should not be empty"
    );
}

/// PROBE: change notification / dirty tracking. With the changelog enabled, edits
/// append `ChangeEvent`s that a binding layer could poll to invalidate visible
/// cells (the substrate for Sub-project C's subscription design).
#[test]
fn changelog_tracks_edits() {
    let mut wb = new_workbook_with_changelog();

    let before = wb.changelog().len();
    wb.set_value(DEFAULT_SHEET, 1, 1, LiteralValue::Int(42))
        .expect("set A1");
    wb.set_formula(DEFAULT_SHEET, 2, 1, "=A1*2")
        .expect("set A2 formula");
    let after = wb.changelog().len();

    assert!(
        after > before,
        "changelog should grow after edits (before={before}, after={after})"
    );

    // The most recent events should describe our edits. We look for a SetValue
    // carrying the new literal we wrote, proving old/new state is captured.
    use formualizer::eval::engine::ChangeEvent;
    let saw_set_value = wb.changelog().events().iter().any(|e| {
        matches!(
            e,
            ChangeEvent::SetValue { new: LiteralValue::Int(42), .. }
        )
    });
    assert!(
        saw_set_value,
        "changelog should contain a SetValue event carrying the new value"
    );
}

/// PROBE: parallel evaluation is a real, reachable knob. Build a workbook with
/// `enable_parallel` + `max_threads` and confirm it evaluates successfully.
#[test]
fn parallel_eval_config_is_exposed() {
    use formualizer::Workbook;

    let config = config_with_parallel_eval(4);
    assert!(config.eval.enable_parallel, "parallel flag should be set");
    assert_eq!(config.eval.max_threads, Some(4));

    let mut wb = Workbook::new_with_config(config);
    wb.set_value(DEFAULT_SHEET, 1, 1, LiteralValue::Int(1))
        .expect("set A1");
    wb.set_value(DEFAULT_SHEET, 2, 1, LiteralValue::Int(2))
        .expect("set A2");
    wb.set_formula(DEFAULT_SHEET, 3, 1, "=A1+A2")
        .expect("set A3");
    wb.prepare_graph_all().expect("prepare graph");

    // Whole-workbook evaluation runs under the parallel scheduler config.
    wb.evaluate_all().expect("parallel evaluate_all");
    let a3 = wb.evaluate_cell(DEFAULT_SHEET, 3, 1).expect("eval A3");
    assert_eq!(as_f64(&a3), Some(3.0));
}

/// PROBE (negative finding, regression-locked): styles/formatting are NOT surfaced
/// through the standard `CellData` read path in 0.7.0. The calamine backend
/// reports `styles = false`, and a reloaded cell's `style` is `None`. This is the
/// key input for Sub-project D (formatting must read umya directly / use a side
/// table). Locking it as a test makes the finding unambiguous and catches any
/// future upstream change.
#[test]
fn styles_not_surfaced_through_celldata() {
    // Calamine (the .xlsx read backend) explicitly advertises no style support.
    let mut wb = build_sum_workbook().expect("build workbook");
    wb.evaluate_all().expect("evaluate all");
    let bytes = xlsx_bytes_from(&wb).expect("serialize");

    let adapter = CalamineAdapter::open_bytes(bytes).expect("open bytes");
    let caps = adapter.capabilities();
    assert!(
        !caps.styles,
        "calamine read backend advertises styles=false (documented gap for Sub-project D)"
    );

    // Reading a cell through the reader trait yields style: None even where a cell
    // exists — formatting is not carried on CellData in 0.7.0.
    let mut adapter = adapter;
    let names = adapter.sheet_names().expect("sheet names");
    let cell = adapter
        .read_cell(&names[0], 1, 1)
        .expect("read_cell ok")
        .expect("cell (1,1) exists");
    assert!(
        cell.style.is_none(),
        "CellData.style is None: formatting is not surfaced via the read path in 0.7.0"
    );
}
