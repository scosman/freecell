//! # smoke — Formualizer 0.7 hands-on smoke test (FreeCell Phase 1, Sub-project A gate)
//!
//! This crate exists to answer one gating question with running code, not vibes:
//! **is `formualizer` a real, usable spreadsheet engine we can build FreeCell on,
//! and what does its API actually look like?** It is deliberately
//! *correctness-focused* (not a benchmark — that is Sub-project C) and small.
//!
//! The integration tests in `tests/smoke.rs` are the probes; this module holds a
//! few thin, documented helpers so the probes stay readable and so this file
//! doubles as living documentation of the **real API surface** that the later
//! phase plans (B–E) are written against.
//!
//! ## Captured Formualizer 0.7.0 API surface (verified by the tests here)
//!
//! Everything below is re-exported from the `formualizer` meta-crate. **Row and
//! column indices are 1-based** throughout this API.
//!
//! ### Build & mutate a workbook
//! - [`formualizer::Workbook::new`] — empty workbook (single implicit `Sheet1`).
//! - `Workbook::add_sheet(&str)`, `has_sheet(&str)`, `sheet_names()`,
//!   `sheet_dimensions(&str) -> Option<(u32,u32)>`.
//! - `set_value(sheet, row, col, LiteralValue)` — set a literal.
//! - `set_formula(sheet, row, col, &str)` — set a formula (`"=A1+A2"`).
//! - `set_values(..)`, `set_formulas(..)`, and
//!   `write_range(sheet, start, BTreeMap<(u32,u32), CellData>)` for bulk writes.
//!
//! ### Read & evaluate
//! - `get_value(sheet,row,col) -> Option<LiteralValue>` — the stored/cached value.
//! - `get_formula(sheet,row,col) -> Option<String>` — canonical formula text.
//! - `evaluate_cell(sheet,row,col) -> Result<LiteralValue>` — evaluate one cell,
//!   pulling its precedents; recomputes after an edit to a precedent.
//! - `read_range(&RangeAddress) -> Vec<Vec<LiteralValue>>` — bulk 2D range read
//!   backed by a columnar range view.
//! - `evaluate_cells(&[(&str,u32,u32)]) -> Result<Vec<LiteralValue>>` — batch eval;
//!   `evaluate_cells_cancellable(..)` takes an `Arc<AtomicBool>` cancel flag.
//! - `evaluate_all()`, `build_recalc_plan()` + `evaluate_with_plan(..)`,
//!   `get_eval_plan(..)` for whole-sheet / planned recompute.
//!
//! ### [`formualizer::LiteralValue`]
//! `Int, Number, Text, Boolean, Array, Date, DateTime, Time, Duration, Empty,
//! Pending, Error(ExcelError)`.
//!
//! ### Parallel evaluation
//! First-class: `formualizer::EvalConfig { enable_parallel: bool,
//! max_threads: Option<usize>, .. }`, reachable via `WorkbookConfig` (see
//! [`config_with_parallel_eval`]). The scheduler evaluates independent vertices in
//! layers.
//!
//! ### Update subscription / dirty tracking / change notification
//! `Workbook::set_changelog_enabled(true)` turns on an append-only
//! `ChangeLog` (`Workbook::changelog() -> &ChangeLog`) of `ChangeEvent`s
//! (`SetValue { old, new }`, `SetFormula`, spill/edge/named-range events…), with
//! compound grouping, `events()`, `len()`, and `take_from(index)`. This is the
//! substrate a binding layer can poll to invalidate visible cells. `undo()` /
//! `redo()` and `action(desc, ..)` transactions build on the same log.
//!
//! ### Apache Arrow
//! `formualizer-eval` depends on the real Apache `arrow` crates (`arrow`,
//! `arrow-array`, `arrow-buffer`, `arrow-cast`, `arrow-schema`, `arrow-select`).
//! Cell truth is an Arrow-backed **columnar** store; the engine journal records
//! `ArrowOp`/`ArrowUndoBatch`, and `read_range` reads a columnar range view.
//! `AccessGranularity` (`Cell/Range/Sheet/Workbook`) and `LoadStrategy`
//! (incl. `LazyRange { row_chunk, col_chunk }`) describe the columnar access and
//! lazy-load model. Arrow arrays themselves are not surfaced as a public
//! read-your-own-`RecordBatch` API in 0.7.0 — access goes through `read_range` /
//! the range view.
//!
//! ### File I/O
//! - Read `.xlsx`: `formualizer::workbook::CalamineAdapter::open_bytes(Vec<u8>)`
//!   (or `open_path`), then `Workbook::from_reader(adapter, LoadStrategy,
//!   WorkbookConfig)`.
//! - Read/write CSV: `CsvAdapter`.
//! - Write `.xlsx`: `Workbook::to_xlsx_bytes()` (uses the umya backend).
//!
//! ### Styles / formatting (KEY GAP — input to Sub-project D)
//! `CellData` carries only `style: Option<StyleId>` (opaque `u32`).
//! `BackendCaps.styles` is `true` for umya and `false` for calamine, **but** both
//! backends' read paths hard-code `style: None` in 0.7.0 — so bold/italic/fills/
//! number-formats are **not** surfaced through the standard `CellData` read path.
//! Formatting must be read from the underlying `umya_spreadsheet` workbook directly
//! (umya *does* preserve styles for round-trip). The
//! `styles_not_surfaced_through_celldata` test regression-locks this finding.

use anyhow::{Context, Result};
// `open_bytes` is a method on the `SpreadsheetReader` trait, so the trait must be
// in scope to call it on the concrete adapters.
use formualizer::workbook::{CalamineAdapter, CsvAdapter, SpreadsheetReader};
use formualizer::{LiteralValue, LoadStrategy, Workbook, WorkbookConfig};

/// The default sheet name a fresh [`Workbook`] exposes.
pub const DEFAULT_SHEET: &str = "Sheet1";

/// Builds a tiny in-memory workbook exercising a real dependency:
/// `A1 = 1`, `A2 = 2`, `A3 = =A1+A2`.
///
/// Uses [`Workbook::set_value`] and [`Workbook::set_formula`] (1-based
/// coordinates). The returned workbook has its dependency graph prepared, so
/// `evaluate_cell` works immediately.
pub fn build_sum_workbook() -> Result<Workbook> {
    let mut wb = Workbook::new();
    wb.set_value(DEFAULT_SHEET, 1, 1, LiteralValue::Int(1))
        .map_err(|e| anyhow::anyhow!("set A1: {e}"))?;
    wb.set_value(DEFAULT_SHEET, 2, 1, LiteralValue::Int(2))
        .map_err(|e| anyhow::anyhow!("set A2: {e}"))?;
    wb.set_formula(DEFAULT_SHEET, 3, 1, "=A1+A2")
        .map_err(|e| anyhow::anyhow!("set A3: {e}"))?;
    wb.prepare_graph_all()
        .map_err(|e| anyhow::anyhow!("prepare graph: {e}"))?;
    Ok(wb)
}

/// Builds a fresh workbook with change-notification (dirty tracking) enabled.
///
/// Wraps [`Workbook::set_changelog_enabled`]; edits made afterwards are recorded
/// as `ChangeEvent`s on [`Workbook::changelog`].
pub fn new_workbook_with_changelog() -> Workbook {
    let mut wb = Workbook::new();
    wb.set_changelog_enabled(true);
    wb
}

/// A [`WorkbookConfig`] with parallel evaluation turned on.
///
/// Sets [`formualizer::EvalConfig::enable_parallel`] and `max_threads`, proving the
/// parallel-eval knob is real and reachable from the public config path.
pub fn config_with_parallel_eval(max_threads: usize) -> WorkbookConfig {
    // `WorkbookConfig` has no `Default`; `ephemeral()` is the neutral base config.
    let mut config = WorkbookConfig::ephemeral();
    config.eval.enable_parallel = true;
    config.eval.max_threads = Some(max_threads);
    config
}

/// Serializes a workbook to `.xlsx` bytes via the umya backend.
///
/// Wraps [`Workbook::to_xlsx_bytes`]. No committed binary fixture is needed: the
/// `.xlsx` is produced by committed code at test time (functional_spec §5.3).
pub fn xlsx_bytes_from(wb: &Workbook) -> Result<Vec<u8>> {
    wb.to_xlsx_bytes()
        .map_err(|e| anyhow::anyhow!("to_xlsx_bytes: {e}"))
        .context("serialize workbook to .xlsx bytes")
}

/// Loads a workbook from in-memory `.xlsx` bytes via the calamine read backend.
///
/// Demonstrates the file-load path: [`CalamineAdapter::open_bytes`] +
/// [`Workbook::from_reader`].
pub fn load_xlsx_bytes(bytes: &[u8]) -> Result<Workbook> {
    let adapter =
        CalamineAdapter::open_bytes(bytes.to_vec()).context("open .xlsx bytes with calamine")?;
    Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
        .map_err(|e| anyhow::anyhow!("from_reader (calamine): {e}"))
        .context("load workbook from calamine adapter")
}

/// Loads a workbook from CSV text via the CSV read backend.
///
/// Demonstrates the CSV-load path: [`CsvAdapter::open_bytes`] +
/// [`Workbook::from_reader`]. CSV imports as a single sheet of literal values.
pub fn load_csv_str(csv: &str) -> Result<Workbook> {
    let adapter =
        CsvAdapter::open_bytes(csv.as_bytes().to_vec()).context("open CSV bytes with csv adapter")?;
    Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
        .map_err(|e| anyhow::anyhow!("from_reader (csv): {e}"))
        .context("load workbook from csv adapter")
}
