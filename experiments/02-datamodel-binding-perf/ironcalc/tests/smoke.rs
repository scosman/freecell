//! # IronCalc 0.7 smoke / API-surface capture (mirrors the Phase-1 Formualizer smoke)
//!
//! These probes both **exercise** and **document** IronCalc's real 0.7.1 API for the
//! operations FreeCell's binding needs, and regression-lock the facts that most shape
//! the bake-off (no incremental recalc; no native range read; styles present on read).
//! They run against `ironcalc` / `ironcalc_base` directly (not through the adapter) so
//! this file is a faithful record of the engine, not our wrapper.
//!
//! ## Captured IronCalc 0.7.1 API surface (verified by these probes)
//!
//! **Row/column indices are 1-based `i32`; sheet index is `u32` (`0` == first sheet).**
//! Core type is `ironcalc_base::Model` (facade + xlsx I/O in the `ironcalc` crate).
//!
//! ### Build & mutate
//! - `Model::new_empty(name, locale_id, timezone, language_id) -> Result<Model, String>`
//!   (single implicit sheet at index 0).
//! - `set_user_input(sheet, row, col, String) -> Result<(), String>` — the *only*
//!   input setter; auto-detects a leading `=` as a formula, otherwise parses the
//!   string as a value. There is no typed value setter.
//! - `cell_clear_contents` / `cell_clear_all`, `set_column_width` / `set_row_height`,
//!   `set_frozen_rows` / `set_frozen_columns`.
//!
//! ### Read & evaluate
//! - `get_cell_value_by_index(sheet,row,col) -> Result<CellValue>` (`CellValue` =
//!   `None|String(String)|Number(f64)|Boolean(bool)`).
//! - `get_formatted_cell_value(..)`, `get_cell_formula(..)`, `get_cell_type(..)`.
//! - `evaluate(&mut self)` — **FULL-workbook recompute**: clears all computed cells
//!   and re-evaluates every cell. **No incremental / dirty recalc, no single-cell
//!   eval, no parallelism.** (`no_incremental_recalc` locks this.)
//! - `get_all_cells() -> Vec<CellIndex>` — the only "bulk" accessor; returns
//!   coordinates only (you still read each value individually). **No range/bulk value
//!   read API.** (`no_native_range_read` documents this.)
//!
//! ### Styles / formatting (present on read — better than Formualizer 0.7)
//! - `get_style_for_cell(sheet,row,col) -> Result<Style>` with `style.font.b` (bold),
//!   `style.font.i` (italic), and `style.fill`. (`styles_present_on_read` locks this.)
//!
//! ### Change notification / diff (collaborative-sync diff-list, on `UserModel`)
//! - `UserModel::{new_empty, set_user_input(&str), evaluate, pause_evaluation,
//!   resume_evaluation, undo, redo, flush_send_queue() -> Vec<u8>,
//!   apply_external_diffs(&[u8])}`. `flush_send_queue` drains a `bitcode`-encoded
//!   diff-list of `SetCellValue { old, new }`-style diffs — built for keeping two
//!   remote models in sync, **not** a "which computed values changed" subscription.
//!   (`usermodel_diff_list_records_edit` exercises it.)
//!
//! ### File I/O (`ironcalc` crate; not exercised in this perf phase)
//! - `ironcalc::import::load_from_xlsx_bytes(..)`, `ironcalc::export::save_to_xlsx(..)`.
//!
//! ### Storage (from source, `base/src/types.rs`)
//! `SheetData = HashMap<i32, HashMap<i32, Cell>>` — nested row→col hashmaps, sparse,
//! **not** columnar/Arrow. The architectural antithesis of "stupid-fast on huge
//! sheets"; recorded here as the key perf-lens caveat for Sub-project G.

use ironcalc_base::cell::CellValue;
use ironcalc_base::{Model, UserModel};

/// A fresh single-sheet model.
fn new_model() -> Model<'static> {
    Model::new_empty("smoke", "en", "UTC", "en").expect("new_empty")
}

#[test]
fn builds_empty_model() {
    let model = new_model();
    // Index 0 exists and starts empty.
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 1).unwrap(),
        CellValue::None
    );
}

#[test]
fn set_user_input_and_read() {
    let mut model = new_model();
    model.set_user_input(0, 1, 1, "42".to_string()).unwrap();
    model.set_user_input(0, 1, 2, "hello".to_string()).unwrap();
    model.evaluate();
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 1).unwrap(),
        CellValue::Number(42.0)
    );
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 2).unwrap(),
        CellValue::String("hello".to_string())
    );
}

#[test]
fn formula_evaluates_after_full_evaluate() {
    // A1=1, A2=2, A3==A1+A2 -> 3; edit A1:=10, re-evaluate -> A3 == 12.
    let mut model = new_model();
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "2".to_string()).unwrap();
    model.set_user_input(0, 3, 1, "=A1+A2".to_string()).unwrap();
    model.evaluate();
    assert_eq!(
        model.get_cell_value_by_index(0, 3, 1).unwrap(),
        CellValue::Number(3.0)
    );

    model.set_user_input(0, 1, 1, "10".to_string()).unwrap();
    model.evaluate();
    assert_eq!(
        model.get_cell_value_by_index(0, 3, 1).unwrap(),
        CellValue::Number(12.0)
    );
}

#[test]
fn styles_present_on_read() {
    // IronCalc surfaces styles on the read path (unlike Formualizer 0.7, which
    // hard-codes style:None). Set a cell bold and read the style back.
    let mut model = new_model();
    model.set_user_input(0, 1, 1, "x".to_string()).unwrap();
    let mut style = model.get_style_for_cell(0, 1, 1).unwrap();
    style.font.b = true;
    model.set_cell_style(0, 1, 1, &style).unwrap();

    let read_back = model.get_style_for_cell(0, 1, 1).unwrap();
    assert!(
        read_back.font.b,
        "bold should be readable from the style path"
    );
}

#[test]
fn usermodel_diff_list_records_edit() {
    // The change/diff surface is on UserModel, not Model. An edit pushes a diff onto
    // the send queue; flush_send_queue drains it (bitcode-encoded, non-empty).
    // Baseline: an empty queue still bitcode-encodes to a few framing bytes, so we
    // compare *lengths* rather than emptiness to detect that a diff was enqueued.
    let mut um = UserModel::new_empty("smoke", "en", "UTC", "en").expect("usermodel");
    let empty_len = um.flush_send_queue().len();

    um.set_user_input(0, 1, 1, "5").unwrap();
    let with_edit = um.flush_send_queue();
    assert!(
        with_edit.len() > empty_len,
        "an edit should enqueue a diff (encoded {} > empty {})",
        with_edit.len(),
        empty_len
    );

    // Draining resets the queue: the next flush is back to the empty-queue length.
    let after = um.flush_send_queue();
    assert_eq!(
        after.len(),
        empty_len,
        "queue should be drained back to the empty-queue encoding after a flush"
    );
}

#[test]
fn usermodel_undo_redo() {
    let mut um = UserModel::new_empty("smoke", "en", "UTC", "en").unwrap();
    um.set_user_input(0, 1, 1, "1").unwrap();
    assert!(um.can_undo());
    um.undo().unwrap();
    assert_eq!(
        um.get_model().get_cell_value_by_index(0, 1, 1).unwrap(),
        CellValue::None
    );
    um.redo().unwrap();
    assert_eq!(
        um.get_model().get_cell_value_by_index(0, 1, 1).unwrap(),
        CellValue::Number(1.0)
    );
}

#[test]
fn no_native_range_read() {
    // Documents the binding-relevant gap: there is NO range/bulk value read. The only
    // bulk accessor, get_all_cells(), returns coordinates only — values are still read
    // one at a time. We assert the shape of what IS available.
    let mut model = new_model();
    for r in 1..=3 {
        for c in 1..=3 {
            model
                .set_user_input(0, r, c, format!("{}", r * 10 + c))
                .unwrap();
        }
    }
    model.evaluate();
    let all = model.get_all_cells();
    assert_eq!(all.len(), 9, "get_all_cells yields coordinates for 9 cells");
    // To read a "range" we must loop scalar reads (the IronCalc viewport cost).
    let mut sum = 0.0;
    for ci in &all {
        if let CellValue::Number(n) = model
            .get_cell_value_by_index(ci.index, ci.row, ci.column)
            .unwrap()
        {
            sum += n;
        }
    }
    assert!(sum > 0.0);
}

#[test]
fn no_incremental_recalc() {
    // Regression-lock the decisive perf-lens fact: evaluate() is a full recompute with
    // no incremental/dirty path. We can't assert timing here, but we CAN assert the
    // behavioural contract the full-recompute model implies: reading a formula WITHOUT
    // calling evaluate() after editing a precedent returns the STALE value — there is
    // no automatic incremental propagation on Model.
    let mut model = new_model();
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "=A1+1".to_string()).unwrap();
    model.evaluate();
    assert_eq!(
        model.get_cell_value_by_index(0, 2, 1).unwrap(),
        CellValue::Number(2.0)
    );

    // Edit the precedent but do NOT evaluate: the dependent is stale until a full
    // evaluate() (no dirty-driven incremental recompute).
    model.set_user_input(0, 1, 1, "100".to_string()).unwrap();
    let stale = model.get_cell_value_by_index(0, 2, 1).unwrap();
    model.evaluate();
    let fresh = model.get_cell_value_by_index(0, 2, 1).unwrap();
    assert_eq!(fresh, CellValue::Number(101.0));
    assert_ne!(
        stale, fresh,
        "value only updates after a full evaluate() — confirms no incremental recalc"
    );
}
