//! FreeCell Round-3 Investigation B (breadth) — needed-API audit.
//!
//! A present / absent / workaround audit of IronCalc 0.7.1's public API against the
//! checklist in `functional_spec §6-B` / `architecture §5`. Every **present** claim is
//! backed by a runtime probe here (called by `tests/audit.rs` and by `main.rs`); every
//! **absent** claim by a documented source search recorded in `findings.md` (cited to
//! `~/.cargo/registry/.../ironcalc*-0.7.1/`).
//!
//! Headline: **who owns display formatting?** — see [`display_format`]. IronCalc owns it
//! (`get_formatted_cell_value` runs the cell's number format and returns the display
//! string), so FreeCell does NOT implement Excel number-format rendering. That is the
//! load-bearing answer for the renderer.
//!
//! This crate builds ON Phase A (`../A-cache-sync/findings.md`), which already
//! established (cited, not redone): merges have no public API; the clipboard isn't
//! externally chainable; the `UserModel` diff-list is opaque bitcode; `UserModel` is
//! `Send`. [`diff_list`] re-confirms the diff-list shape with a fresh probe.

pub mod cell_extras;
pub mod defined_names;
pub mod diff_list;
pub mod display_format;
pub mod formula_helpers;
pub mod known_gaps;
pub mod sheet_ops;
pub mod view_state;

/// One row of the audit matrix, as observed by a probe (or a documented source search).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A public API exists and a runtime probe exercised it successfully.
    Present,
    /// No public API; a documented source search found none. FreeCell must own it or
    /// work around it.
    Absent,
    /// Partially reachable: some capability exists but with a caveat (e.g. read but not
    /// write, or reachable only via a lower-level path). The note explains the caveat.
    Workaround,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Present => "PRESENT",
            Status::Absent => "ABSENT",
            Status::Workaround => "WORKAROUND",
        }
    }
}

/// A single audited capability + its status + a one-line note, for the printed report.
#[derive(Debug, Clone)]
pub struct AuditRow {
    pub capability: &'static str,
    pub status: Status,
    pub note: String,
}

impl AuditRow {
    pub fn new(capability: &'static str, status: Status, note: impl Into<String>) -> Self {
        AuditRow {
            capability,
            status,
            note: note.into(),
        }
    }
}

/// Runs every probe and returns the assembled matrix. Panics (via the probes' own
/// asserts) if a claimed-present API stops behaving — so `cargo run` doubles as a
/// liveness check of the "present" claims.
pub fn run_full_audit() -> Vec<AuditRow> {
    let mut rows = Vec::new();
    rows.extend(display_format::audit());
    rows.extend(diff_list::audit());
    rows.extend(sheet_ops::audit());
    rows.extend(defined_names::audit());
    rows.extend(view_state::audit());
    rows.extend(cell_extras::audit());
    rows.extend(formula_helpers::audit());
    rows.extend(known_gaps::audit());
    rows
}
