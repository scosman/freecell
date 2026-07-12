//! The chrome around the grid — action row, data row (formula bar), and sheet tab bar
//! (`components/app_shell.md`, `ui_design.md §3.1–3.4`).
//!
//! Built from stock gpui-component controls and driven by the Phase-2 pure logic
//! ([`freecell_core::data_row`] state machine, [`freecell_core::eval_indicator`] spinner,
//! [`freecell_core::palette`] fill swatches, [`freecell_core::sheet_name`] rename
//! validation). The GPUI layer ([`view::ChromeView`]) is thin plumbing: it turns widget
//! events into reducer events, performs the returned effects as client commands / grid
//! requests, and renders from state. Every user action is also a plain method on
//! `ChromeView`, so behaviour is unit-testable without simulating pixel clicks.
//!
//! The engine is reached through the [`client::ChromeClient`] seam (a trait the real
//! `DocumentClient` implements, and the [`client::RecordingClient`] double stands in for in
//! tests / the demo). Chrome ↔ grid coupling (move the active cell, focus the grid, switch
//! sheet) flows through the [`ChromeGridSink`] closure. Phase 11 wires both to the real
//! `DocumentClient` + `GridView`.

pub mod client;
mod edit;
mod view;

use freecell_core::selection::Motion;
use freecell_core::{CellRef, SheetId};
use gpui::{App, SharedString, Window};

pub use client::{ChromeClient, RecordingClient};
pub use edit::{EditController, EditOrigin};
pub use view::{ChartPanel, ChartPanelSeries, ChromeView};

/// One sheet as the tab bar mirrors it. The chrome's own view-model of the worker's
/// `SheetMeta`, extended with `has_content` (which the worker's `SheetMeta` does not carry
/// yet) so the delete-confirm rule (`components/app_shell.md §Sheet tab bar`: confirm only a
/// non-empty sheet) is decidable UI-side. Phase 11 maps `SheetMeta` → `SheetTab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetTab {
    /// The stable sheet id (survives renames), as on the worker seam.
    pub id: SheetId,
    /// The current display name.
    pub name: String,
    /// Whether the sheet has any cell content — gates the delete-confirm modal.
    pub has_content: bool,
}

impl SheetTab {
    /// A tab with the given id/name; `has_content` defaults to `false`.
    pub fn new(id: SheetId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            has_content: false,
        }
    }

    /// Sets `has_content` (builder form, for fixtures/tests).
    pub fn with_content(mut self, has_content: bool) -> Self {
        self.has_content = has_content;
        self
    }
}

/// A request the chrome makes of the grid it lives above (`components/app_shell.md`): the
/// grid owns the selection + focus, so a data-row commit that moves the active cell, an
/// Escape that returns focus, and a tab switch are all delegated to it. Analogous to the
/// grid's own [`crate::grid::GridEvent`] sink.
#[derive(Debug, Clone)]
pub enum ChromeGridRequest {
    /// Move the active cell (Enter after a commit → down; Tab → right; etc.).
    MoveActive(Motion),
    /// Return keyboard focus to the grid (after a commit / Escape).
    FocusGrid,
    /// Switch the grid to `sheet` (tab click).
    SetActiveSheet(SheetId),
    /// Push the current edit's grid-facing state after every edit transition
    /// (`components/edit_controller.md §4.3–4.4`): the live cell **mirror** (raw text under the
    /// pending cell), the open **in-cell** overlay cell, and the in-cell **cap** popover message.
    /// `None`s clear the corresponding grid overlay (edit committed / cancelled).
    EditState {
        /// The mirror: raw pending text to paint in `(sheet, cell)` instead of its published value.
        mirror: Option<(SheetId, CellRef, SharedString)>,
        /// The in-cell overlay's open cell (the overlay is rendered only while `Some`).
        in_cell: Option<CellRef>,
        /// The in-cell editor's cap-error popover message, if a cap rejection is active there.
        cap: Option<SharedString>,
        /// Whether the current edit is in **quick-edit** mode (type-to-replace entry): while set, an
        /// unmodified arrow key commits + moves the active cell instead of the caret
        /// (`functional_spec.md §5`). Only ever `true` while an edit is live; the grid consumes it in
        /// its in-cell `capture_key_down`.
        quick_edit: bool,
    },
}

/// The owner's [`ChromeGridRequest`] handler (boxed like the grid's `GridEventHandler`).
type ChromeGridHandler = dyn Fn(&ChromeGridRequest, &mut Window, &mut App);

/// The chrome's [`ChromeGridRequest`] handler — invoked with full `Window`/`App` access so
/// the window can drive the sibling `GridView`. [`ChromeGridSink::noop`] drops every request
/// (standalone chrome / tests that don't assert grid coupling).
pub struct ChromeGridSink {
    handler: Box<ChromeGridHandler>,
}

impl ChromeGridSink {
    /// Builds a sink from a handler.
    pub fn new(handler: impl Fn(&ChromeGridRequest, &mut Window, &mut App) + 'static) -> Self {
        Self {
            handler: Box::new(handler),
        }
    }

    /// A sink that drops every request.
    pub fn noop() -> Self {
        Self::new(|_, _, _| {})
    }

    /// Delivers a request to the owner.
    pub(crate) fn emit(&self, request: ChromeGridRequest, window: &mut Window, cx: &mut App) {
        (self.handler)(&request, window, cx);
    }
}
