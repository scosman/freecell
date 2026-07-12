//! Engine-call instrumentation — the counter behind Phase 12's "zero engine calls on the
//! scroll/render path" gate (`architecture.md §4, §9`).
//!
//! Every method on [`WorkbookDocument`](crate::document::WorkbookDocument) that reads or
//! mutates the constructed IronCalc `UserModel` bumps this process-global counter via
//! `record_engine_call` at its entry — the per-cell/geometry reads (`formatted_value`,
//! `cell_content`, `cell_own_style`, `row_band_style`, `col_band_style`, `row_height_px`,
//! `col_width_px`, `worksheet`), the sheet-metadata reads (`sheet_names`, `sheet_count`,
//! `sheet_properties`, `sheet_properties_with_content`), the edits (`set_cell_input`,
//! `clear_contents`, `set_font_flag`, `set_fill`, `add_sheet`, `rename_sheet`,
//! `delete_sheet`, `undo`, `redo`, `font_flag`), the batch controls (`pause_evaluation`,
//! `resume_evaluation`, `evaluate`), `save`, and the `user_model_mut` escape hatch. The only
//! methods that do **not** bump it are the constructors (`new_empty`/`open`/`from_source` —
//! they *build* a fresh model rather than access an existing one) and two `#[cfg(test)]`
//! style-read helpers (`resolved_cell_style`/`cell_style`, deliberately excluded so they
//! can't pollute unrelated tests' counts).
//!
//! `WorkbookDocument` is the *single* IronCalc boundary and lives entirely worker-side — the
//! grid's render path never holds one — so a scroll/render sweep must leave the counter
//! unchanged. The perf harness snapshots it before/after the sweep and asserts a zero delta;
//! a **negative control** (a real read/edit) proves the counter can climb, so the gate is
//! discriminating rather than vacuous.
//!
//! The counter is `Relaxed`: it is a coarse "did the engine do work" signal read after the
//! worker has gone idle (a happens-before is established by the drain), not a
//! synchronization primitive.

use std::sync::atomic::{AtomicU64, Ordering};

/// Total IronCalc model operations performed since process start (or the last reset).
static ENGINE_CALLS: AtomicU64 = AtomicU64::new(0);

/// Record one IronCalc model operation. Called at the entry of every model-touching
/// `WorkbookDocument` method.
#[inline]
pub(crate) fn record_engine_call() {
    ENGINE_CALLS.fetch_add(1, Ordering::Relaxed);
}

/// The number of IronCalc model operations performed so far. The perf harness reads this
/// before/after a scroll sweep (the delta must be zero) and after a negative-control edit
/// (the delta must be positive).
pub fn engine_call_count() -> u64 {
    ENGINE_CALLS.load(Ordering::Relaxed)
}

/// Reset the counter to zero (test / harness convenience; the harness prefers before/after
/// deltas so it never has to reset a shared global).
pub fn reset_engine_call_count() {
    ENGINE_CALLS.store(0, Ordering::Relaxed);
}
