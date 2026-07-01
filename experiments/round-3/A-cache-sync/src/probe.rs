//! `UserModel` API + `Send`-ness probe (functional_spec §6-A, architecture §4.3).
//!
//! Two questions this module answers with compile-time + runtime evidence:
//!   1. Is `UserModel<'static>` `Send`? (drives whether the SP1 worker seam — a worker
//!      thread owning the model — still holds).
//!   2. Does apply/evaluate block the same way `Model::evaluate` does? (structural edits
//!      on `UserModel` are `&mut self` + a full `evaluate()`, so the seam's coalescing
//!      logic still applies).

use std::thread;

use ironcalc_base::{Model, UserModel};

/// **Compile-time proof** that `UserModel<'static>` is `Send`, mirroring the SP1 seam's
/// `assert_send::<Model<'static>>()` (round-2 `01-async-interop/src/seam.rs:52`). If a
/// future IronCalc adds a non-`Send` field to `UserModel`/`Model`, this stops compiling —
/// which would itself be the finding.
pub fn assert_usermodel_send() {
    fn assert_send<T: Send>() {}
    assert_send::<UserModel<'static>>();
    // Control: the low-level Model was already proven Send in SP1.
    assert_send::<Model<'static>>();
}

/// A captured summary of the observed `UserModel` API surface, for `findings.md`.
#[derive(Debug, Clone)]
pub struct ApiProbe {
    pub has_undo_redo: bool,
    pub has_insert_delete_rows: bool,
    pub has_insert_delete_columns: bool,
    pub has_get_model: bool,
    pub has_flush_send_queue_diff_bytes: bool,
    pub diff_list_publicly_inspectable: bool,
    pub copy_paste_roundtrip_externally_usable: bool,
    pub merge_cells_public_api: bool,
    pub notes: Vec<String>,
}

/// Runs a runtime smoke over the interactive API and returns what it observed.
/// Uses `'static` string literals so the model is `UserModel<'static>`.
pub fn probe_api() -> Result<ApiProbe, String> {
    let mut model = UserModel::new_empty("probe", "en", "UTC", "en")?;

    // Value edit + evaluate + read back (auto-evaluate on set_user_input).
    model.set_user_input(0, 1, 1, "=1+1")?;
    assert_eq!(model.get_formatted_cell_value(0, 1, 1)?, "2");

    // Undo / redo present and functional.
    let can_undo_before = model.can_undo();
    model.undo()?;
    let reverted = model.get_formatted_cell_value(0, 1, 1)? == *"";
    model.redo()?;
    let reapplied = model.get_formatted_cell_value(0, 1, 1)? == *"2";

    // Structural edits present (probe with a 1-row insert then delete on an empty area).
    model.insert_rows(0, 1, 1)?;
    model.delete_rows(0, 1, 1)?;
    model.insert_columns(0, 1, 1)?;
    model.delete_columns(0, 1, 1)?;

    // get_model() exposes the underlying &Model (the styled/size getters + xlsx export
    // path live there).
    let _m: &Model = model.get_model();

    // The diff-list is only reachable as opaque bitcode bytes via flush_send_queue();
    // the `Diff` enum is pub(crate) and cannot be matched field-by-field externally.
    // After the edits above, the send queue holds their diffs, so the bytes are non-empty.
    let mut edited = UserModel::new_empty("diffprobe", "en", "UTC", "en")?;
    edited.set_user_input(0, 1, 1, "=1+2")?;
    edited.insert_rows(0, 1, 1)?;
    let diff_bytes = edited.flush_send_queue();
    let has_diff_bytes = !diff_bytes.is_empty();

    let notes = vec![
        "UserModel<'a> wraps Model<'a>; UserModel<'static> is Send (compile-time asserted)."
            .to_string(),
        "Structural edits are &mut self and auto-run a full evaluate() \
         (evaluate_if_not_paused), so they block reads exactly like Model::evaluate — the \
         SP1 coalescing seam carries over unchanged."
            .to_string(),
        "flush_send_queue() returns bitcode-encoded QueueDiffs; the Diff enum is \
         pub(crate) so the diff-list is NOT publicly inspectable field-by-field. \
         Cache-sync must mirror the primitive it issued, not consume structured diffs."
            .to_string(),
        "copy_to_clipboard() returns a Clipboard whose `data` field is pub(crate); \
         ClipboardCell is not externally constructible, so copy->paste cannot be chained \
         through the public API from an external crate. Relative-reference translation is \
         still reachable via the public Model::extend_copied_value()."
            .to_string(),
        "merge_cells: Vec<String> exists on Worksheet but has NO public Model/UserModel \
         setter or getter in 0.7.1 — merges are unreachable through the public API."
            .to_string(),
    ];

    Ok(ApiProbe {
        has_undo_redo: can_undo_before && reverted && reapplied,
        has_insert_delete_rows: true,
        has_insert_delete_columns: true,
        has_get_model: true,
        has_flush_send_queue_diff_bytes: has_diff_bytes,
        diff_list_publicly_inspectable: false,
        copy_paste_roundtrip_externally_usable: false,
        merge_cells_public_api: false,
        notes,
    })
}

/// The SP1 worker-seam probe applied to `UserModel`: move a `UserModel<'static>` onto a
/// spawned worker thread (runtime proof of `Send`), run a structural edit + evaluate on
/// the worker, hand it back via the join handle, and confirm reads still work. Returns
/// the value read back from the model after the round-trip.
pub fn probe_worker_seam() -> Result<String, String> {
    let mut model = UserModel::new_empty("seam", "en", "UTC", "en")?;
    model.set_user_input(0, 1, 1, "10")?;
    model.set_user_input(0, 2, 1, "=A1*2")?; // A2 = A1*2 = 20

    // Move the model onto a worker thread — this only compiles/links if it is `Send`,
    // and running it proves it at runtime too (mirrors SP1 `EvalWorker::spawn`).
    let handle = thread::spawn(move || -> Result<UserModel<'static>, String> {
        // Structural edit on the worker: insert a row at the top shifts A1->A2, A2->A3.
        model.insert_rows(0, 1, 1)?;
        model.evaluate();
        Ok(model)
    });

    let model = handle
        .join()
        .map_err(|_| "worker thread panicked".to_string())??;

    // After inserting a row at top: original A1(10) is now A2, its formula (was A2=A1*2)
    // is now A3 and still references the shifted A2 -> still 20.
    let a3 = model.get_formatted_cell_value(0, 3, 1)?;
    Ok(a3)
}
