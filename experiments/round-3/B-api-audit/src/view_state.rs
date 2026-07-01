//! View / UI state in `.xlsx` — freeze panes, hidden rows/cols, gridlines, zoom,
//! selection. Mixed: freeze panes / gridlines / selection are PRESENT and round-trip;
//! hidden rows are read-only-workaround; hidden columns / zoom are ABSENT.
//!
//! Source-cited (verified by probe where a public setter/getter exists):
//! - Frozen rows/cols — PRESENT + xlsx round-trip:
//!   `set_frozen_rows_count`/`set_frozen_columns_count` + getters (common.rs:1143/1157,
//!   1126/1135), undoable. xlsx `<pane>` import (import/worksheets.rs) + export
//!   (export/worksheets.rs). `Worksheet.frozen_rows/columns` (types.rs:115/116).
//! - Gridlines — PRESENT + xlsx export:
//!   `set_show_grid_lines`/`get_show_grid_lines` (common.rs:1687/1700); exported as
//!   `showGridLines`.
//! - Selection / active cell — PRESENT + xlsx round-trip: `set_selected_cell`,
//!   `set_selected_range`, `get_selected_cell` (ui.rs:92/118/35).
//! - Hidden ROWS — WORKAROUND: `Row.hidden` is a public field (types.rs:135), read on
//!   import and written on export, but there is **no public UserModel setter** to hide a
//!   row (only `set_rows_height`). FreeCell would set visibility via the Model field / an
//!   upstreamed setter.
//! - Hidden COLUMNS — ABSENT: `Col` (types.rs:140) has NO `hidden` field at all — columns
//!   cannot be hidden through IronCalc.
//! - Zoom — ABSENT: no zoom field/method/xlsx handling anywhere.
//! - Window size / scroll pos — present as UI state but NOT persisted to `.xlsx`
//!   (window_* / top_row / left_column read but not exported).

use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// Probes the PRESENT view-state APIs (freeze panes, gridlines, selection) with a
/// set -> read-back round-trip.
pub fn probe() -> ViewStateObservation {
    let mut model = UserModel::new_empty("Book", "en", "UTC", "en").unwrap();

    model.set_frozen_rows_count(0, 2).unwrap();
    model.set_frozen_columns_count(0, 1).unwrap();
    let frozen_rows = model.get_frozen_rows_count(0).unwrap();
    let frozen_cols = model.get_frozen_columns_count(0).unwrap();

    // Gridlines default true; set to false and read back.
    model.set_show_grid_lines(0, false).unwrap();
    let grid_off = !model.get_show_grid_lines(0).unwrap();

    // Selection round-trip.
    model.set_selected_cell(5, 3).unwrap();
    let (_sheet, sel_row, sel_col) = model.get_selected_cell();

    ViewStateObservation {
        frozen_rows,
        frozen_cols,
        grid_off,
        sel_row,
        sel_col,
    }
}

#[derive(Debug, Clone)]
pub struct ViewStateObservation {
    pub frozen_rows: i32,
    pub frozen_cols: i32,
    pub grid_off: bool,
    pub sel_row: i32,
    pub sel_col: i32,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "View state: freeze panes (frozen rows/cols) + xlsx round-trip",
            Status::Present,
            format!(
                "set/get_frozen_rows_count + columns, undoable, exported as <pane>. \
                 (rows={}, cols={}). (common.rs:1143/1157/1126/1135)",
                o.frozen_rows, o.frozen_cols
            ),
        ),
        AuditRow::new(
            "View state: gridlines show/hide (+ xlsx export)",
            Status::Present,
            format!(
                "set/get_show_grid_lines; off={}; exported as showGridLines. \
                 (common.rs:1687/1700)",
                o.grid_off
            ),
        ),
        AuditRow::new(
            "View state: selection / active cell (+ xlsx round-trip)",
            Status::Present,
            format!(
                "set_selected_cell/range + get_selected_cell -> ({},{}). (ui.rs:92/118/35)",
                o.sel_row, o.sel_col
            ),
        ),
        AuditRow::new(
            "View state: hide a row",
            Status::Workaround,
            "Row.hidden is a public field that round-trips through xlsx, but there is NO \
             public UserModel setter to hide a row. FreeCell sets it via the Model or an \
             upstreamed API. (types.rs:135)",
        ),
        AuditRow::new(
            "View state: hide a column",
            Status::Absent,
            "Col (types.rs:140) has no `hidden` field at all — columns cannot be hidden \
             through IronCalc. FreeCell owns column visibility, and it will not xlsx \
             round-trip via IronCalc.",
        ),
        AuditRow::new(
            "View state: zoom level",
            Status::Absent,
            "No zoom field/method/xlsx handling in 0.7.1. FreeCell owns zoom (view-only, \
             not persisted through IronCalc).",
        ),
    ]
}
