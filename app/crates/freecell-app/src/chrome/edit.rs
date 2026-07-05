//! [`EditController`] — the in-cell editor + cross-editor sync layered over the data row
//! (`components/edit_controller.md`).
//!
//! **Ownership note (deviation from the component doc — see `DECISIONS_TO_REVIEW.md` Phase 2).**
//! The component doc sketches a `WorkbookWindow`-owned controller owning *both* editor
//! `InputState`s and the whole pending-edit state machine. FreeCell instead keeps the single
//! pending edit inside **one entity** — [`ChromeView`](super::ChromeView), which already owns the
//! data-row `InputState` and the proven [`freecell_core::data_row::DataRow`] reducer
//! (fetch / spinner / disabled / cap / commit / escape, all table-tested). This
//! [`EditController`] holds the **second** editor — the in-cell overlay `InputState` — plus the
//! overlay's open cell, the current [`EditOrigin`], and the `syncing` re-entrancy guard. The two
//! editors therefore sync *within one entity* (no cross-entity `InputState` feedback loop). The
//! canonical pending **text + commit/cap** live in the `DataRow` reducer; this controller adds the
//! in-cell editor, the two-way text sync, and origin tracking on top.

use gpui::Entity;
use gpui_component::input::InputState;

use freecell_core::CellRef;

/// Which editor currently drives the shared pending edit (== has focus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOrigin {
    /// The data-row (formula bar) editor.
    DataRow,
    /// The in-cell overlay editor.
    InCell,
}

/// The in-cell editor half of the single pending edit (`components/edit_controller.md`). Owned by
/// [`ChromeView`](super::ChromeView); the data-row half is the chrome's existing `content_input` +
/// `DataRow` reducer.
pub struct EditController {
    /// The reused in-cell overlay input (one instance for every in-cell edit). Created in
    /// `ChromeView::new` (where `&mut Window` is available) and rendered by the **grid** as an
    /// absolute overlay; the chrome subscribes to it for its edit events.
    in_cell: Entity<InputState>,
    /// The cell the in-cell overlay currently covers, or `None` when the overlay is closed. The
    /// data-row editor can be the active `origin` while the overlay is open (focus moved back).
    open: Option<CellRef>,
    /// Which editor currently drives the edit (follows focus).
    origin: EditOrigin,
    /// Re-entrancy guard for the two-way text sync: set around a programmatic `set_value` push so
    /// the resulting (suppressed) echo is ignored belt-and-braces.
    syncing: bool,
}

impl EditController {
    /// Builds the controller over the reused in-cell input, overlay closed, driving from the data
    /// row.
    pub fn new(in_cell: Entity<InputState>) -> Self {
        Self {
            in_cell,
            open: None,
            origin: EditOrigin::DataRow,
            syncing: false,
        }
    }

    /// The reused in-cell input handle (the window hands a clone to the grid to render the
    /// overlay).
    pub fn in_cell_input(&self) -> Entity<InputState> {
        self.in_cell.clone()
    }

    /// The in-cell input handle by reference (for `cx` updates without cloning).
    pub fn in_cell(&self) -> &Entity<InputState> {
        &self.in_cell
    }

    /// The cell the in-cell overlay covers, if it is open.
    pub fn open_cell(&self) -> Option<CellRef> {
        self.open
    }

    /// Whether the in-cell overlay is currently open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Which editor currently drives the edit.
    pub fn origin(&self) -> EditOrigin {
        self.origin
    }

    /// Opens the overlay on `cell`, driving from the in-cell editor.
    pub fn open_on(&mut self, cell: CellRef) {
        self.open = Some(cell);
        self.origin = EditOrigin::InCell;
    }

    /// Closes the overlay and returns the driver to the data row.
    pub fn close(&mut self) {
        self.open = None;
        self.origin = EditOrigin::DataRow;
    }

    /// Sets which editor is driving (focus moved between the two editors — no text moves).
    pub fn set_origin(&mut self, origin: EditOrigin) {
        self.origin = origin;
    }

    /// Raises/lowers the sync guard around a programmatic `set_value` push into the *other*
    /// editor, so the resulting (already event-suppressed) echo is ignored belt-and-braces.
    pub fn set_syncing(&mut self, syncing: bool) {
        self.syncing = syncing;
    }

    /// Whether a sync push is currently in progress (an incoming editor event should be ignored).
    pub fn is_syncing(&self) -> bool {
        self.syncing
    }
}
