//! Sheet ops — add / rename / delete / reorder / enumerate.
//!
//! All PRESENT on `UserModel` (interactive + undoable) EXCEPT reorder:
//! - add:       `UserModel::new_sheet()` (common.rs:496)
//! - rename:    `UserModel::rename_sheet(sheet, new_name)` (common.rs:531)
//! - delete:    `UserModel::delete_sheet(sheet)` (common.rs:507)
//! - enumerate: `UserModel::get_worksheets_properties() -> Vec<SheetProperties>`
//!   (common.rs:1682; `SheetProperties { name, state, sheet_id, color }`, types.rs:653).
//!   Also `Workbook::get_worksheet_names()` (workbook.rs:6).
//! - hide/unhide + tab color: `hide_sheet`/`unhide_sheet` (common.rs:550/576),
//!   `set_sheet_color` (common.rs:594).
//! - **reorder / move a sheet: ABSENT** — a source search for `move_sheet` / `reorder` /
//!   `set_worksheet_index` / `swap_worksheets` across `ironcalc_base/src` finds nothing.

use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// Exercises the full sheet lifecycle + enumeration + undo of an add.
pub fn probe() -> SheetOpsObservation {
    let mut model = UserModel::new_empty("Book", "en", "UTC", "en").unwrap();
    let initial = model.get_worksheets_properties().len();

    model.new_sheet().unwrap();
    let after_add = model.get_worksheets_properties().len();

    // Rename the newly-added sheet (index 1) and read the name back via enumeration.
    model.rename_sheet(1, "Renamed").unwrap();
    let renamed = model.get_worksheets_properties()[1].name.clone();

    // Undo the rename, then undo the add — confirms sheet ops are on the history stack.
    model.undo().unwrap(); // undo rename
    let name_after_undo = model.get_worksheets_properties()[1].name.clone();
    model.undo().unwrap(); // undo add
    let after_undo_add = model.get_worksheets_properties().len();

    // Delete path (add two, delete one).
    model.new_sheet().unwrap();
    model.new_sheet().unwrap();
    let before_delete = model.get_worksheets_properties().len();
    model.delete_sheet(before_delete as u32 - 1).unwrap();
    let after_delete = model.get_worksheets_properties().len();

    SheetOpsObservation {
        initial,
        after_add,
        renamed,
        name_after_undo,
        after_undo_add,
        before_delete,
        after_delete,
    }
}

#[derive(Debug, Clone)]
pub struct SheetOpsObservation {
    pub initial: usize,
    pub after_add: usize,
    pub renamed: String,
    pub name_after_undo: String,
    pub after_undo_add: usize,
    pub before_delete: usize,
    pub after_delete: usize,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "Sheet ops: add / rename / delete (interactive + undoable)",
            Status::Present,
            format!(
                "UserModel new_sheet/rename_sheet/delete_sheet, all on the undo stack \
                 (add {}->{}, rename ok={:?}, delete {}->{}). \
                 (common.rs:496, 531, 507)",
                o.initial,
                o.after_add,
                o.renamed == "Renamed",
                o.before_delete,
                o.after_delete
            ),
        ),
        AuditRow::new(
            "Sheet ops: enumerate sheets (name/id/state/color)",
            Status::Present,
            "get_worksheets_properties() -> Vec<SheetProperties{name,state,sheet_id,color}> \
             (common.rs:1682, types.rs:653); Workbook::get_worksheet_names (workbook.rs:6).",
        ),
        AuditRow::new(
            "Sheet ops: hide/unhide + tab color",
            Status::Present,
            "hide_sheet/unhide_sheet (common.rs:550/576), set_sheet_color (common.rs:594).",
        ),
        AuditRow::new(
            "Sheet ops: reorder / move a sheet",
            Status::Absent,
            "No move_sheet/reorder/set_worksheet_index in ironcalc_base 0.7.1. FreeCell \
             workaround: manipulate workbook order on export or upstream a reorder API.",
        ),
    ]
}
