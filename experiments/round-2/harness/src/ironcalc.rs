//! # ironcalc_bench — IronCalc 0.7 adapter for the engine bake-off
//!
//! Implements [`crate::engine::SpreadsheetEngine`] (was `binding_common::SpreadsheetEngine`
//! in the Phase-1 source this was copied from) on top of an IronCalc
//! [`ironcalc_base::Model`], so the shared D1/D2/D3 binding designs and the five
//! benchmark scenarios run against IronCalc with directly-comparable numbers
//! (Sub-project C, functional_spec §6.C, architecture §5). See `tests/smoke.rs` for
//! the API-surface capture that mirrors the Phase-1 Formualizer smoke.
//!
//! ## Mapping notes (verified against the 0.7.1 source)
//!
//! - **Indexing.** Trait coordinates are 0-based (datagen space); IronCalc is 1-based
//!   `i32` with a `u32` sheet index. The adapter adds `+1` on every call; sheet `0`.
//! - **Writes.** Both values and formulas go through `Model::set_user_input(sheet,
//!   row, col, String)`, which auto-detects a leading `=`. There is no typed value
//!   setter, so values are stringified.
//! - **Recompute — FULL, not incremental.** `Model::evaluate()` **clears all computed
//!   cells and re-evaluates the whole workbook** — IronCalc has no dirty/incremental
//!   recalc. To avoid an O(N) recompute on *every* seeded cell, the adapter defers:
//!   `set_value`/`set_formula`/`set_batch` write inputs without evaluating, and
//!   `recompute` does the single full `evaluate()`. Readers call `recompute` (or the
//!   scenario does) before reading cascaded results. This is the honest cost and the
//!   central architectural contrast with Formualizer.
//! - **Reads.** `get_value` → `Model::get_cell_value_by_index`. There is **no native
//!   range/bulk read**, so `read_viewport` loops per-cell (the difference from
//!   Formualizer's columnar `read_range` is a headline finding).
//! - **Change tracking.** IronCalc's change/diff surface lives on `UserModel`
//!   (`flush_send_queue`/`apply_external_diffs`, a collaborative-sync diff-list), not
//!   on `Model`. Rather than switch engine types mid-benchmark, the adapter tracks
//!   edited addresses itself (the same information the diff-list would carry) so D3 can
//!   run; `tests/smoke.rs` separately exercises the real `UserModel` diff-list.

// NOTE: adapter copied VERBATIM from
// `experiments/02-datamodel-binding-perf/ironcalc/src/lib.rs`. The ONE mechanical
// change is this import path: the `SpreadsheetEngine` trait + neutral types now live
// in this same crate (module `crate::engine`) rather than the separate
// `binding_common` crate they were in under Phase 1. Behavior is unchanged.
use crate::engine::{CellInput, EngineCaps, EngineValue, SpreadsheetEngine, Viewport};
use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;

/// The single sheet all benchmarks use (index 0, created by `new_empty`).
const SHEET: u32 = 0;

/// An IronCalc-backed engine implementing the shared binding surface.
///
/// Holds a `Model<'static>` — `new_empty` borrows its locale/language string
/// arguments, and we pass `'static` string literals, so the model owns no shorter
/// borrow. This lets it satisfy the lifetime-free [`SpreadsheetEngine`] trait.
pub struct IronCalcEngine {
    model: Model<'static>,
    /// Addresses edited since the last drain. IronCalc's own diff-list lives on
    /// `UserModel`; we mirror the same edited-cell information here so the D3 cached
    /// binding can run against `Model` (see module docs).
    dirty: Vec<(u32, u32)>,
    tracking: bool,
}

impl IronCalcEngine {
    /// Wraps an existing model (used by tests / advanced probes).
    pub fn from_model(model: Model<'static>) -> Self {
        Self {
            model,
            dirty: Vec::new(),
            tracking: false,
        }
    }

    /// Borrows the underlying model (for adapter tests / style probes).
    pub fn model(&self) -> &Model<'static> {
        &self.model
    }

    fn note_dirty(&mut self, row: u32, col: u32) {
        if self.tracking {
            self.dirty.push((row, col));
        }
    }

    /// Writes an input at 0-based `(row, col)` **without** evaluating (deferred so a
    /// batch pays a single full recompute, not one per cell).
    fn put(&mut self, row: u32, col: u32, input: String) {
        self.model
            .set_user_input(SHEET, (row + 1) as i32, (col + 1) as i32, input)
            .expect("ironcalc set_user_input");
        self.note_dirty(row, col);
    }
}

/// Renders a neutral [`EngineValue`] as the string IronCalc's `set_user_input` wants.
fn value_to_input(v: &EngineValue) -> String {
    match v {
        EngineValue::Empty => String::new(),
        EngineValue::Number(n) => format!("{n}"),
        EngineValue::Text(t) => t.clone(),
        EngineValue::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        EngineValue::Error(_) => "#VALUE!".to_string(),
    }
}

/// Converts an IronCalc [`CellValue`] to a neutral [`EngineValue`].
fn from_cell_value(v: CellValue) -> EngineValue {
    match v {
        CellValue::None => EngineValue::Empty,
        CellValue::String(s) => EngineValue::Text(s),
        CellValue::Number(n) => EngineValue::Number(n),
        CellValue::Boolean(b) => EngineValue::Bool(b),
    }
}

impl SpreadsheetEngine for IronCalcEngine {
    fn name(&self) -> &'static str {
        "ironcalc"
    }

    fn new_blank() -> Self {
        let model = Model::new_empty("bench", "en", "UTC", "en").expect("ironcalc new_empty");
        Self::from_model(model)
    }

    fn set_value(&mut self, row: u32, col: u32, v: EngineValue) {
        // Deferred: caller triggers recompute() before reading cascaded results.
        self.put(row, col, value_to_input(&v));
    }

    fn set_formula(&mut self, row: u32, col: u32, formula: &str) {
        self.put(row, col, formula.to_string());
    }

    fn set_batch(&mut self, cells: &[(u32, u32, CellInput)]) {
        let mut has_formula = false;
        for (r, c, input) in cells {
            let s = match input {
                CellInput::Value(v) => value_to_input(v),
                CellInput::Formula(f) => {
                    has_formula = true;
                    f.clone()
                }
            };
            self.put(*r, *c, s);
        }
        // One full recompute for the whole batch (IronCalc has no incremental path).
        // Skip it for a pure-literal batch — there is nothing to compute, and IronCalc's
        // evaluate() is O(all cells), so skipping matters most for a big chunked load.
        if has_formula {
            self.model.evaluate();
        }
    }

    fn bulk_load_block(&mut self, rows: u32, cols: u32, cell: &dyn Fn(u32, u32) -> EngineValue) {
        // IronCalc's fastest base-value load: direct `set_user_input` into the row→col
        // `HashMap`, with NO `evaluate()` — literals need no compute (its `evaluate()` is
        // O(all cells), so skipping it is the honest fast path). This is the fair
        // counterpart to Formualizer's Arrow ingest: each engine loads via its optimal
        // native path, so the recorded build/memory numbers compare like for like.
        for r in 0..rows {
            for c in 0..cols {
                let v = cell(r, c);
                if v != EngineValue::Empty {
                    self.put(r, c, value_to_input(&v));
                }
            }
        }
    }

    fn get_value(&self, row: u32, col: u32) -> EngineValue {
        match self
            .model
            .get_cell_value_by_index(SHEET, (row + 1) as i32, (col + 1) as i32)
        {
            Ok(v) => from_cell_value(v),
            Err(_) => EngineValue::Empty,
        }
    }

    fn evaluate_cell(&mut self, row: u32, col: u32) -> EngineValue {
        // No single-cell eval: recompute the whole workbook, then read.
        self.model.evaluate();
        self.get_value(row, col)
    }

    fn read_viewport(&self, vp: Viewport) -> Vec<EngineValue> {
        // No native range read: per-cell loop (the IronCalc cost vs Formualizer).
        vp.addresses().map(|(r, c)| self.get_value(r, c)).collect()
    }

    fn recompute(&mut self) {
        // Full-workbook recompute — the only recalc IronCalc offers.
        self.model.evaluate();
    }

    fn enable_change_tracking(&mut self) {
        self.tracking = true;
        self.dirty.clear();
    }

    fn drain_dirty(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.dirty)
    }

    fn caps(&self) -> EngineCaps {
        EngineCaps {
            native_range_read: false,
            incremental_recalc: false,
            parallel_eval: false,
            change_log: true,     // via UserModel diff-list (collaborative sync)
            styles_on_read: true, // get_style_for_cell exposes bold/italic/fill
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_input_rendering() {
        assert_eq!(value_to_input(&EngineValue::Number(3.5)), "3.5");
        assert_eq!(value_to_input(&EngineValue::Text("hi".into())), "hi");
        assert_eq!(value_to_input(&EngineValue::Bool(true)), "TRUE");
    }

    #[test]
    fn cell_value_conversion() {
        assert_eq!(
            from_cell_value(CellValue::Number(2.0)),
            EngineValue::Number(2.0)
        );
        assert_eq!(from_cell_value(CellValue::None), EngineValue::Empty);
    }
}
