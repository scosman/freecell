//! # formualizer_bench — Formualizer 0.7 adapter for the engine bake-off
//!
//! Implements [`binding_common::SpreadsheetEngine`] on top of a Formualizer
//! [`Workbook`], so the shared D1/D2/D3 binding designs and the five benchmark
//! scenarios (in `binding_common`) run against Formualizer with directly-comparable
//! numbers (Sub-project C, functional_spec §6.C, architecture §5).
//!
//! ## Mapping notes (verified against the 0.7.0 source)
//!
//! - **Indexing.** Trait coordinates are 0-based (datagen space); Formualizer is
//!   1-based, so the adapter adds `+1` on every call. The single sheet is `Sheet1`.
//! - **Reads.** `get_value` → `Workbook::get_value`; `read_viewport` uses the native
//!   columnar `Workbook::read_range` over a `RangeAddress` — Formualizer's headline
//!   advantage over IronCalc, which has no bulk read.
//! - **Recompute.** Formualizer has *incremental* recalc: `evaluate_cell` /
//!   `evaluate_cells` pull only what a target needs, and `evaluate_all` recomputes the
//!   dirty set. The adapter's `recompute` calls `evaluate_all` (recompute-after-edit).
//! - **Batching.** `set_batch` uses `write_range`, whose deferred-dirty scope does a
//!   single propagation for the whole batch instead of a BFS per cell.
//! - **Change tracking.** `set_changelog_enabled(true)` turns on an append-only
//!   `ChangeLog`; the adapter tracks a read cursor and reports `SetValue`/`SetFormula`
//!   addresses since the last drain (the substrate for the D3 cached binding).

use std::collections::BTreeMap;

use binding_common::{CellInput, EngineCaps, EngineValue, SpreadsheetEngine, Viewport};
use formualizer::workbook::traits::CellData;
use formualizer::{LiteralValue, RangeAddress, Workbook};

/// The single sheet all benchmarks use.
const SHEET: &str = "Sheet1";

/// A Formualizer-backed engine implementing the shared binding surface.
pub struct FormualizerEngine {
    wb: Workbook,
    /// Read cursor into the changelog: events at/after this index are "new" since the
    /// last [`SpreadsheetEngine::drain_dirty`].
    change_cursor: usize,
    tracking: bool,
}

impl FormualizerEngine {
    /// Wraps an existing workbook (used by tests that pre-build a shape).
    pub fn from_workbook(wb: Workbook) -> Self {
        Self {
            wb,
            change_cursor: 0,
            tracking: false,
        }
    }

    /// Borrows the underlying workbook (for adapter tests / advanced probes).
    pub fn workbook(&self) -> &Workbook {
        &self.wb
    }
}

/// Converts a neutral [`EngineValue`] to a Formualizer [`LiteralValue`].
fn to_literal(v: &EngineValue) -> LiteralValue {
    match v {
        EngineValue::Empty => LiteralValue::Empty,
        EngineValue::Number(n) => LiteralValue::Number(*n),
        EngineValue::Text(t) => LiteralValue::Text(t.clone()),
        EngineValue::Bool(b) => LiteralValue::Boolean(*b),
        EngineValue::Error(_) => LiteralValue::Error(formualizer::ExcelError::new(
            formualizer::ExcelErrorKind::Value,
        )),
    }
}

/// Converts a Formualizer [`LiteralValue`] to a neutral [`EngineValue`].
fn from_literal(v: LiteralValue) -> EngineValue {
    match v {
        LiteralValue::Empty | LiteralValue::Pending => EngineValue::Empty,
        LiteralValue::Int(i) => EngineValue::Number(i as f64),
        LiteralValue::Number(n) => EngineValue::Number(n),
        LiteralValue::Text(t) => EngineValue::Text(t),
        LiteralValue::Boolean(b) => EngineValue::Bool(b),
        LiteralValue::Error(e) => EngineValue::Error(format!("{e:?}")),
        // Dates/times/arrays are not produced by our scenarios; surface a label.
        other => EngineValue::Text(format!("{other:?}")),
    }
}

impl SpreadsheetEngine for FormualizerEngine {
    fn name(&self) -> &'static str {
        "formualizer"
    }

    fn new_blank() -> Self {
        Self::from_workbook(Workbook::new())
    }

    fn set_value(&mut self, row: u32, col: u32, v: EngineValue) {
        self.wb
            .set_value(SHEET, row + 1, col + 1, to_literal(&v))
            .expect("formualizer set_value");
    }

    fn set_formula(&mut self, row: u32, col: u32, formula: &str) {
        self.wb
            .set_formula(SHEET, row + 1, col + 1, formula)
            .expect("formualizer set_formula");
    }

    fn set_batch(&mut self, cells: &[(u32, u32, CellInput)]) {
        // One deferred-dirty write_range for the whole batch: a single propagation
        // instead of a BFS per cell.
        let mut map: BTreeMap<(u32, u32), CellData> = BTreeMap::new();
        let mut has_formula = false;
        for (r, c, input) in cells {
            let data = match input {
                CellInput::Value(v) => CellData {
                    value: Some(to_literal(v)),
                    formula: None,
                    style: None,
                },
                CellInput::Formula(f) => {
                    has_formula = true;
                    CellData {
                        value: None,
                        formula: Some(f.clone()),
                        style: None,
                    }
                }
            };
            map.insert((r + 1, c + 1), data);
        }
        self.wb
            .write_range(SHEET, (1, 1), map)
            .expect("formualizer write_range");
        // Only evaluate if the batch introduced formulas whose results readers need.
        // A pure-literal batch (e.g. a bulk load) has nothing to compute, so we skip
        // the graph walk — important when a big load is written in many chunks.
        if has_formula {
            let _ = self.wb.evaluate_all();
        }
    }

    fn get_value(&self, row: u32, col: u32) -> EngineValue {
        match self.wb.get_value(SHEET, row + 1, col + 1) {
            Some(v) => from_literal(v),
            None => EngineValue::Empty,
        }
    }

    fn evaluate_cell(&mut self, row: u32, col: u32) -> EngineValue {
        match self.wb.evaluate_cell(SHEET, row + 1, col + 1) {
            Ok(v) => from_literal(v),
            Err(e) => EngineValue::Error(format!("{e:?}")),
        }
    }

    fn read_viewport(&self, vp: Viewport) -> Vec<EngineValue> {
        // Native columnar range read (RangeAddress is 1-based, inclusive).
        let addr = RangeAddress::new(
            SHEET,
            vp.row0 + 1,
            vp.col0 + 1,
            vp.row0 + vp.rows,
            vp.col0 + vp.cols,
        )
        .expect("valid range");
        let grid = self.wb.read_range(&addr);
        let mut out = Vec::with_capacity(vp.cell_count());
        for row in grid {
            for v in row {
                out.push(from_literal(v));
            }
        }
        out
    }

    fn recompute(&mut self) {
        // Incremental: evaluate_all recomputes the dirty set produced by recent edits.
        let _ = self.wb.evaluate_all();
    }

    fn enable_change_tracking(&mut self) {
        self.wb.set_changelog_enabled(true);
        self.tracking = true;
        self.change_cursor = self.wb.changelog().len();
    }

    fn drain_dirty(&mut self) -> Vec<(u32, u32)> {
        if !self.tracking {
            return Vec::new();
        }
        let log = self.wb.changelog();
        let events = log.events();
        let mut dirty = Vec::new();
        for event in &events[self.change_cursor.min(events.len())..] {
            use formualizer::eval::engine::ChangeEvent;
            let addr = match event {
                ChangeEvent::SetValue { addr, .. } => Some(addr),
                ChangeEvent::SetFormula { addr, .. } => Some(addr),
                _ => None,
            };
            if let Some(addr) = addr {
                // `Coord` stores 0-based internally (built via `from_excel`, which
                // subtracts 1), so `row()`/`col()` are already in datagen's 0-based
                // space — no further adjustment.
                dirty.push((addr.coord.row(), addr.coord.col()));
            }
        }
        self.change_cursor = events.len();
        dirty
    }

    fn caps(&self) -> EngineCaps {
        EngineCaps {
            native_range_read: true,
            incremental_recalc: true,
            parallel_eval: true,
            change_log: true,
            styles_on_read: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_from_literal_roundtrip() {
        assert_eq!(
            from_literal(to_literal(&EngineValue::Number(3.5))),
            EngineValue::Number(3.5)
        );
        assert_eq!(
            from_literal(to_literal(&EngineValue::Text("hi".into()))),
            EngineValue::Text("hi".into())
        );
        assert_eq!(from_literal(LiteralValue::Int(7)), EngineValue::Number(7.0));
    }
}
