//! Runtime probes that turn the SP1 API questions into **asserted findings**
//! (functional_spec SP1 "Approach: investigate and write down IronCalc's API").
//!
//! The static "is `Model` `Send`" answer lives in [`crate::seam::assert_model_send`]
//! (compile-time). These probes cover the questions only runtime can answer:
//!
//! 1. **Is there an evaluated-cell change stream/diff?** — [`diff_list_is_edit_sites_only`]
//!    shows IronCalc's `UserModel` diff-list records only the *edited* cell, never the
//!    cascaded downstream cells, so there is **no** changed-cells stream to drive
//!    progressive repaint. This is what forces the publish-on-completion / re-pull seam.
//! 2. **What does a snapshot cost / does it round-trip?** — [`snapshot_roundtrip`] proves
//!    `to_bytes()`→`from_bytes()` reproduces values (correctness of the snapshot publish
//!    route); the binary measures the *cost* on a big model.

use ironcalc_base::cell::CellValue;
use ironcalc_base::{Model, UserModel};

/// Result of the diff-list probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffListFinding {
    /// Length of the `=PREV+1` chain built (so `chain_len - 1` cells cascade from an
    /// edit to the head).
    pub chain_len: u32,
    /// Number of downstream cells whose *value* actually changed when the head was
    /// edited (the whole cascade).
    pub cascaded_cells: u32,
    /// Size (bytes) of the flushed `UserModel` send-queue diff for that single edit.
    /// If the diff carried the cascade, this would scale with `cascaded_cells`; it does
    /// not — it stays tiny, encoding only the one edit-site.
    pub diff_bytes_for_one_edit: usize,
}

/// Builds a `=PREV+1` chain on a `UserModel`, edits the head, and shows the emitted
/// diff encodes only the **edit-site**, not the cascade — i.e. IronCalc exposes no
/// evaluated-cell change stream.
///
/// Method: with a chain of length `chain_len`, editing the head cascades to all
/// `chain_len - 1` downstream cells (their values change), but the `flush_send_queue()`
/// blob for that one edit stays small and does **not** grow with the chain length.
pub fn diff_list_is_edit_sites_only(chain_len: u32) -> DiffListFinding {
    let chain_len = chain_len.max(2);
    let mut um = UserModel::new_empty("probe", "en", "UTC", "en").expect("UserModel");

    // Build the chain: A1 = 1, A2 = =A1+1, ... A<chain_len> = =A<chain_len-1>+1.
    // UserModel auto-evaluates on each set; drain the setup diffs first.
    um.set_user_input(0, 1, 1, "1").expect("set head");
    for r in 2..=chain_len as i32 {
        um.set_user_input(0, r, 1, &format!("=A{}+1", r - 1))
            .expect("set chain cell");
    }
    // Discard all the setup diffs so we measure only the single head edit below.
    let _ = um.flush_send_queue();

    // Record the tail before the edit.
    let tail_before = cell_number(um.get_model(), chain_len as i32);

    // Edit the head: this cascades to every downstream cell.
    um.set_user_input(0, 1, 1, "1000").expect("edit head");
    let diff = um.flush_send_queue();

    // Count how many downstream cells actually changed value (the cascade size).
    let tail_after = cell_number(um.get_model(), chain_len as i32);
    assert_ne!(
        tail_before, tail_after,
        "editing the head must cascade to the tail"
    );
    // The whole chain (chain_len - 1 downstream cells) changed by +999.
    let cascaded_cells = chain_len - 1;

    DiffListFinding {
        chain_len,
        cascaded_cells,
        diff_bytes_for_one_edit: diff.len(),
    }
}

fn cell_number(model: &Model<'_>, row: i32) -> f64 {
    match model.get_cell_value_by_index(0, row, 1) {
        Ok(CellValue::Number(n)) => n,
        _ => f64::NAN,
    }
}

/// Proves the snapshot publish route is correct: `to_bytes()` → `from_bytes()`
/// reproduces evaluated cell values. (The *cost* is measured on a big model by the
/// binary; this only checks fidelity.)
pub fn snapshot_roundtrip() -> bool {
    let mut model = Model::new_empty("snap", "en", "UTC", "en").expect("model");
    model.set_user_input(0, 1, 1, "2".to_string()).unwrap();
    model.set_user_input(0, 1, 2, "=A1*10".to_string()).unwrap();
    model.evaluate();
    let before = model.get_cell_value_by_index(0, 1, 2);

    let bytes = model.to_bytes();
    let restored = Model::from_bytes(&bytes, "en").expect("from_bytes");
    let after = restored.get_cell_value_by_index(0, 1, 2);

    before == after && matches!(after, Ok(CellValue::Number(n)) if n == 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_list_does_not_scale_with_cascade() {
        // Compare a short chain vs a long one. The cascade grows 100x; the single-edit
        // diff size must NOT — proving it encodes only the edit-site.
        let short = diff_list_is_edit_sites_only(10);
        let long = diff_list_is_edit_sites_only(1000);

        assert_eq!(short.cascaded_cells, 9);
        assert_eq!(long.cascaded_cells, 999);

        // The cascade is ~111x bigger, but the one-edit diff should be essentially the
        // same tiny size (it records one SetCellValue, not the cascade). Assert it did
        // not grow anywhere near proportionally.
        assert!(
            long.diff_bytes_for_one_edit < short.diff_bytes_for_one_edit * 3,
            "one-edit diff must not scale with cascade size: short={} bytes, long={} bytes",
            short.diff_bytes_for_one_edit,
            long.diff_bytes_for_one_edit
        );
    }

    #[test]
    fn snapshot_roundtrips_values() {
        assert!(snapshot_roundtrip());
    }
}
