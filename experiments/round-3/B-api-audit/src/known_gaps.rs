//! Re-confirm the known OPEN gaps (record, do NOT design) — merges, conditional
//! formatting, dynamic arrays. These are pre-known product-scope items (overview §2), not
//! new findings; documented here for completeness of the matrix.
//!
//! - Merges — ABSENT (public API): `Worksheet.merge_cells: Vec<String>` exists
//!   (types.rs:113) but there is NO public `Model`/`UserModel` setter or getter in 0.7.1
//!   (a search for `merge`/`set_merge`/`get_merge` in `user_model/common.rs` finds none).
//!   Confirms Phase A §2(d). Merges force owning `.xlsx` writing.
//! - Conditional formatting — ABSENT: no conditional-formatting type/field/method in
//!   `ironcalc_base/src` (the only "conditional" hits are number-format `[cond]` parsing
//!   in the formatter tests, unrelated). Confirms overview §2.
//! - Dynamic arrays / spilling — ABSENT (0/17): SP3 measured 0/17 dynamic-array cases;
//!   a pending PRODUCT decision (accept v1 / build spill / upstream), not a technical
//!   unknown. Recorded, not designed.

use crate::{AuditRow, Status};

pub fn audit() -> Vec<AuditRow> {
    vec![
        AuditRow::new(
            "Known gap: merged cells (merges)",
            Status::Absent,
            "Worksheet.merge_cells field exists (types.rs:113) but no public setter/getter \
             on Model/UserModel. Confirms Phase A §2(d). Product-scope; forces owning \
             xlsx writing.",
        ),
        AuditRow::new(
            "Known gap: conditional formatting",
            Status::Absent,
            "No conditional-formatting type/field/method in ironcalc_base 0.7.1. Confirms \
             overview §2. Product-scope; forces owning xlsx writing.",
        ),
        AuditRow::new(
            "Known gap: dynamic arrays / spilling",
            Status::Absent,
            "0/17 (SP3). Pending PRODUCT decision (accept v1 / build spill / upstream). \
             Recorded, not designed (overview §2).",
        ),
    ]
}
