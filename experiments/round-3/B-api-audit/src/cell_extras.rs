//! Cell extras — comments/notes, data validation, hyperlinks. All are gaps FreeCell
//! must plan around; documented source searches (no public setters).
//!
//! - Comments/notes — WORKAROUND (read-only, lossy on save):
//!   `Worksheet.comments: Vec<Comment>` is a public field (types.rs:114;
//!   `Comment { text, author_name, author_id, cell_ref }`, types.rs:229). xlsx IMPORT
//!   loads them (import/worksheets.rs), but EXPORT does NOT write them (a source search
//!   for "comment" in ironcalc/src/export/worksheets.rs finds nothing) — comments are
//!   **dropped on save**. There is no public UserModel getter/setter to create/edit a
//!   comment. FreeCell that needs comments must own comment storage + xlsx writing.
//! - Data validation — ABSENT: no `validation`/`DataValidation` field or method anywhere
//!   in `ironcalc_base/src`, and xlsx import ignores `<dataValidation>`.
//! - Hyperlinks — ABSENT: no `hyperlink`/`Hyperlink` field on `Cell`/`Worksheet`, and
//!   xlsx import/export do not handle hyperlink relationships.

use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// The only reachable probe: read the (public) comments field via `get_model()`. A fresh
/// sheet has none, proving the field is reachable and starts empty — there is no public
/// path to add one interactively.
pub fn probe() -> CellExtrasObservation {
    let model = UserModel::new_empty("Book", "en", "UTC", "en").unwrap();
    let comment_count = model
        .get_model()
        .workbook
        .worksheet(0)
        .unwrap()
        .comments
        .len();
    CellExtrasObservation { comment_count }
}

#[derive(Debug, Clone)]
pub struct CellExtrasObservation {
    pub comment_count: usize,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "Cell extras: comments / notes",
            Status::Workaround,
            format!(
                "Worksheet.comments is a public field (read-only, {} on a fresh sheet), \
                 loaded on xlsx import but DROPPED on export (no public setter, no export \
                 code). FreeCell owns comments if needed. (types.rs:114/229)",
                o.comment_count
            ),
        ),
        AuditRow::new(
            "Cell extras: data validation",
            Status::Absent,
            "No validation field/type/method in ironcalc_base 0.7.1; xlsx import ignores \
             <dataValidation>. FreeCell owns it (would need xlsx writing).",
        ),
        AuditRow::new(
            "Cell extras: hyperlinks",
            Status::Absent,
            "No hyperlink field on Cell/Worksheet; xlsx import/export do not handle \
             hyperlinks. FreeCell owns it (would need xlsx writing).",
        ),
    ]
}
