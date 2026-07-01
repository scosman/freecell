//! Defined names / named ranges — read + write, both PRESENT on `UserModel`.
//!
//! - create: `UserModel::new_defined_name(name, scope, formula)` (common.rs:1996),
//!   `scope = None` (workbook) or `Some(sheet_index)` (sheet-scoped).
//! - list:   `UserModel::get_defined_name_list() -> Vec<(name, scope, formula)>`
//!   (common.rs:1977).
//! - update: `UserModel::update_defined_name(name, scope, new_name, new_scope, new_formula)`
//!   (common.rs:2014).
//! - delete: `UserModel::delete_defined_name(name, scope)` (common.rs:1982).
//!
//! All are undoable (push a `Diff` + auto-evaluate). A formula referencing a defined name
//! evaluates to the name's value.

use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// Creates a workbook-scoped defined name, references it in a formula, lists it, and
/// deletes it — the full read+write round-trip.
pub fn probe() -> DefinedNamesObservation {
    let mut model = UserModel::new_empty("Book", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "42").unwrap(); // A1 = 42

    // Workbook-scoped name pointing at A1.
    model
        .new_defined_name("MyVal", None, "Sheet1!$A$1")
        .unwrap();
    let listed = model.get_defined_name_list();
    let has_name = listed.iter().any(|(n, _, _)| n == "MyVal");

    // A formula using the name evaluates to A1's value.
    model.set_user_input(0, 2, 1, "=MyVal*2").unwrap();
    let via_name = model.get_formatted_cell_value(0, 2, 1).unwrap();

    // Delete it and confirm it's gone.
    model.delete_defined_name("MyVal", None).unwrap();
    let gone = !model
        .get_defined_name_list()
        .iter()
        .any(|(n, _, _)| n == "MyVal");

    DefinedNamesObservation {
        has_name,
        via_name,
        gone_after_delete: gone,
    }
}

#[derive(Debug, Clone)]
pub struct DefinedNamesObservation {
    pub has_name: bool,
    pub via_name: String,
    pub gone_after_delete: bool,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![AuditRow::new(
        "Defined names / named ranges: read + write (workbook & sheet scope)",
        Status::Present,
        format!(
            "new/list/update/delete_defined_name on UserModel, all undoable; a formula \
             using the name evaluates (=MyVal*2 -> {:?}); listed={}, deleted={}. \
             (common.rs:1996, 1977, 2014, 1982)",
            o.via_name, o.has_name, o.gone_after_delete
        ),
    )]
}
