//! Edit diff-list (`UserModel`) — shape + how FreeCell consumes it.
//!
//! Confirms and extends Phase A's finding (`../A-cache-sync/findings.md` §5, §carry-forward)
//! with a fresh probe:
//! - `UserModel::flush_send_queue() -> Vec<u8>` (common.rs:376) returns
//!   `bitcode::encode(&self.send_queue)` — **opaque bytes**, not a field-by-field
//!   structure. The `Diff` enum (`user_model/history.rs:20`) is `pub(crate)`, so an
//!   external crate cannot match on diff variants.
//! - `UserModel::apply_external_diffs(&[u8])` (common.rs:389) decodes those bytes into a
//!   *replica* model (`bitcode::decode::<Vec<QueueDiffs>>`), replaying redo/undo diff
//!   lists. This is the collaborative / multi-replica sync channel.
//!
//! Consequence for FreeCell (matches A's locked design): the diff-list is a
//! **replica-sync transport**, NOT a surgical-UI-update channel. FreeCell drives surgical
//! updates by **mirroring the op it issued** (it originates every edit, so it knows
//! `(kind, at, count)` / the edited cell), and can additionally use flush/apply for a
//! future collaborative path. Edit-sites only (no downstream-dirty set), per SP1.

use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// Probes the diff-list: makes edits, flushes the (opaque) send queue, and replays it
/// into a fresh replica — proving the bytes carry the edits even though they can't be
/// inspected field-by-field. Returns `(diff_byte_len, replica_matches_original)`.
pub fn probe() -> DiffListObservation {
    let mut origin = UserModel::new_empty("origin", "en", "UTC", "en").unwrap();
    origin.set_user_input(0, 1, 1, "=1+2").unwrap();
    origin.insert_rows(0, 1, 1).unwrap(); // a structural edit -> a diff too

    // flush_send_queue returns opaque bitcode bytes (not inspectable field-by-field).
    let diff_bytes = origin.flush_send_queue();
    let diff_byte_len = diff_bytes.len();

    // The ONLY external use of those bytes is to sync a replica via apply_external_diffs.
    let mut replica = UserModel::new_empty("origin", "en", "UTC", "en").unwrap();
    replica.apply_external_diffs(&diff_bytes).unwrap();

    // After replay, the replica reflects the same edits (=1+2 shifted down to row 2 by the
    // inserted row).
    let origin_val = origin.get_formatted_cell_value(0, 2, 1).unwrap();
    let replica_val = replica.get_formatted_cell_value(0, 2, 1).unwrap();

    DiffListObservation {
        diff_byte_len,
        replica_matches_origin: origin_val == replica_val && origin_val == "3",
    }
}

#[derive(Debug, Clone)]
pub struct DiffListObservation {
    pub diff_byte_len: usize,
    pub replica_matches_origin: bool,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "Edit diff-list: extract as structured, field-by-field data",
            Status::Absent,
            "flush_send_queue() returns bitcode-encoded opaque bytes; the Diff enum is \
             pub(crate) (history.rs:20) so it cannot be matched externally. \
             (common.rs:376)",
        ),
        AuditRow::new(
            "Edit diff-list: replica / collaborative sync transport",
            Status::Present,
            format!(
                "flush_send_queue() -> {} opaque bytes; apply_external_diffs() replays \
                 them into a replica (matches origin = {}). Use this for collab, NOT for \
                 surgical UI updates. (common.rs:376, 389)",
                o.diff_byte_len, o.replica_matches_origin
            ),
        ),
        AuditRow::new(
            "Surgical UI updates from the diff-list",
            Status::Workaround,
            "FreeCell mirrors the op it issued (it knows kind/at/count/cell) instead of \
             consuming structured diffs — exactly A's locked cache-sync design. \
             Edit-sites only, no downstream-dirty (per SP1).",
        ),
    ]
}
