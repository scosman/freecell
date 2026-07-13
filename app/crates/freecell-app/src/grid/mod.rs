//! The custom virtualized spreadsheet grid (`components/grid.md`, `ui_design.md §3.3`).
//!
//! A raw-GPUI view that renders headers, gridlines, cells, and selection for an Excel-max
//! sheet, reading **only** from the resident caches and the published viewport — the render
//! path makes zero engine calls and materializes only the visible viewport
//! (`architecture.md §4`). Port-and-extend of the proven POC
//! (`experiments/04-ui-poc/raw-gpui/src/grid.rs`).
//!
//! Phase 6 scope: static rendering + wheel scroll + clamping + overlay scrollbars +
//! loading overlay, driven by hand-built `freecell-core` fixtures. Phase 8 adds the input
//! plumbing: mouse selection (click / drag / shift-click + edge auto-scroll), keyboard
//! motions dispatched through `freecell_core::apply_motion`, and the `SelectionChanged` /
//! `ClearCells` events (`components/grid.md §Input`, `ui_design.md §5–6`).

pub mod chart_layer;
pub mod fixtures;
pub mod input;
pub mod layout;
mod view;

use std::ops::Range;

use gpui::{App, Window};

use freecell_chart_model::{Anchor, ChartId};
use freecell_core::selection::Direction;
use freecell_core::{CellRange, CellRef, SelectionModel};

pub(crate) use view::caret_intent_modifiers;
pub use view::{GridDataSources, GridView};

// --- Look constants (`ui_design.md §3.3`) -------------------------------------------
// Colours are `0xRRGGBB`, mapped onto `gpui::rgb(...)` at draw time.

/// Gridline colour — 1 px light grey lines under cell fills (`#E2E2E2`).
pub const GRIDLINE: u32 = 0xE2E2E2;
/// Default near-black cell text colour when no format colour overrides it (`#1F1F1F`).
pub const CELL_TEXT: u32 = 0x1F1F1F;
/// Default white cell background.
pub const CELL_BG: u32 = 0xFFFFFF;
/// Header strip / gutter fill (`#F5F5F5`).
pub const HEADER_BG: u32 = 0xF5F5F5;
/// Header hairline border colour (`#D9D9D9`).
pub const HEADER_HAIRLINE: u32 = 0xD9D9D9;
/// Header label text (muted dark).
pub const HEADER_TEXT: u32 = 0x555555;
/// Darker header tint for the selected rows/columns ("you are here").
pub const HEADER_SELECTED_BG: u32 = 0xE4E4E4;
/// Selection accent — gpui-component's blue-600 (`#2563EB`). The default gpui-component
/// theme's `primary` token is *neutral*, not blue (see DECISIONS_TO_REVIEW), so the
/// spreadsheet selection blue is pinned to the theme's blue ramp instead.
pub const ACCENT: u32 = 0x2563EB;
/// Range-fill overlay opacity (accent at ~10%).
pub const SELECTION_FILL_ALPHA: f32 = 0.10;
/// Overlay-scrollbar thumb colour (semi-transparent grey).
pub const SCROLLBAR_RGBA: u32 = 0x8A8A8A99;

/// Cell text size (px) — `ui_design.md §3.3` ("13 px bundled Inter"). Defined in
/// `freecell-core` so the engine's font-size row auto-grow can keep the default
/// row-height : font-size ratio as fonts scale (see the constant's doc there).
pub const CELL_FONT_PX: f32 = freecell_core::cache::DEFAULT_CELL_FONT_PX;
/// Header text size (px) — small medium-weight labels.
pub const HEADER_FONT_PX: f32 = 11.5;
/// Horizontal text padding inside a cell (px).
pub const CELL_H_PAD: f32 = 4.0;
/// Seconds the overlay scrollbars stay visible after the last scroll before fading.
pub const SCROLLBAR_FADE_SECS: u64 = 2;

/// Fixed auto-scroll step (px) applied each frame while a drag is held past a viewport edge
/// (`components/grid.md §Input`: "fixed 20 px/frame step").
pub const EDGE_AUTOSCROLL_STEP_PX: f64 = 20.0;
/// Inward hot-zone (px, ~a cell height) within which a drag near a viewport edge starts
/// auto-scrolling. Load-bearing: gpui delivers `on_mouse_move` only while the pointer is inside
/// the grid element, so the loop must be able to START from a move event fired while the pointer
/// is still `EDGE_AUTOSCROLL_HOTZONE_PX` inside the edge (the content right/bottom edges coincide
/// with the window edge). Excel-like — scrolling begins as the pointer nears an edge.
pub const EDGE_AUTOSCROLL_HOTZONE_PX: f64 = 24.0;
/// The auto-scroll timer interval (ms) — roughly one 60 fps frame. While a drag is held past
/// an edge, a `spawn_in` loop applies [`EDGE_AUTOSCROLL_STEP_PX`] every tick (the "held at the
/// edge with no mouse-move events" case; a live `window.mouse_position()` drives the extend).
pub const AUTOSCROLL_INTERVAL_MS: u64 = 16;

/// The grid/cell font family (`ui_design.md §3.3`: bundled Inter). The app registers the
/// bundled Inter faces and sets this as the UI font at startup (`shell/fonts.rs`), and the grid
/// also names it explicitly at its `grid/view.rs` text sites (cell + header) for robustness.
/// Named in one place so the registration and the text sites stay in sync.
pub const GRID_FONT_FAMILY: &str = "Inter";

/// Which axis a structural interaction targets — a resize / insert / delete of rows or columns
/// (`components/grid_structure.md §5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrCol {
    Row,
    Col,
}

/// Events the grid raises to its owner (`WorkbookWindow`, Phase 11). Phase 8 drives
/// [`GridEvent::SelectionChanged`] (mouse + keyboard), [`GridEvent::ViewportChanged`]
/// (scroll / keyboard reveal / edge auto-scroll), and [`GridEvent::ClearCells`]
/// (Delete/Backspace); the window forwards them to the worker + sibling chrome in Phase 11.
#[derive(Debug, Clone)]
pub enum GridEvent {
    /// The selection changed — drives the data row, action row, and ref box.
    SelectionChanged(SelectionModel),
    /// The visible index range changed (pre-overscan). The window forwards this to the
    /// worker as `SetViewport` with its own ~3× overscan.
    ViewportChanged { rows: Range<u32>, cols: Range<u32> },
    /// Delete/Backspace with grid focus: clear the selection's cell contents (keep styles).
    /// The window supplies the active `SheetId` → `Command::ClearCells` (`components/grid.md
    /// §Input`, `ui_design.md §6`).
    ClearCells(CellRange),
    /// A click-away happened while the data row was editing (commit the pending edit).
    EditCommitRequested,
    /// A printable, modifier-free keystroke on the focused grid with a single (or collapsed-to-
    /// active) selection — start a **type-to-replace** edit whose content is `text`
    /// (`functional_spec.md §1.1`). The window routes this to `ChromeView::begin_typed`.
    TypeToEdit(String),
    /// Double-click on a cell, or F2 with a single selection — open the **in-cell editor** over
    /// `cell` (`functional_spec.md §1.3`). Routed to `ChromeView::begin_in_cell`.
    OpenInCellEditor(CellRef),
    /// Tab / Shift+Tab captured in the in-cell overlay — commit + move (`functional_spec.md §1.4`).
    InCellCommitMove(Direction),
    /// Escape captured in the in-cell overlay — cancel the pending edit.
    InCellCancel,
    /// Cmd/Ctrl+C (`cut: false`) / Cmd/Ctrl+X (`cut: true`) on the focused grid — copy/cut the
    /// selection to the range clipboard (`functional_spec.md §2.1`). The window routes this to
    /// the `ClipboardCoordinator`.
    Copy { cut: bool },
    /// Cmd/Ctrl+V on the focused grid — paste at the selection anchor (`functional_spec.md §2.2`).
    Paste,
    /// Wrap-driven row auto-grow (`functional_spec.md §3`): the render thread measured each
    /// `(row, px)`'s wrapped height (device px) at its column width — geometry the worker can't
    /// compute (no gpui text system). The window forwards it as `Command::AutoGrowRowHeights` for
    /// the active sheet; the worker applies it cache-only (auto rows only, no undo step). Emitted
    /// only when a row's wrap **inputs** changed, so a height-only republish never re-emits.
    AutoGrowRows { heights: Vec<(u32, f32)> },
    /// A row/column resize was committed on release (`functional_spec.md §5.1`). `start..=end` is
    /// the inclusive 0-based track run — the dragged index alone, or the whole selected header run
    /// when the dragged header sits inside a header selection; `px` is the released device-px size.
    /// The window forwards it as `SetColumnWidths` / `SetRowHeights`.
    ResizeCommitted {
        axis: RowOrCol,
        start: u32,
        end: u32,
        px: f32,
    },
    /// Insert `count` rows so new rows appear at 0-based `at` (`functional_spec.md §5.3`, chosen
    /// from the header context menu). The window forwards it as `Command::InsertRows`.
    InsertRows { at: u32, count: u32 },
    /// Insert `count` columns at 0-based `at`.
    InsertColumns { at: u32, count: u32 },
    /// Delete `count` rows starting at 0-based `at`.
    DeleteRows { at: u32, count: u32 },
    /// Delete `count` columns starting at 0-based `at`.
    DeleteColumns { at: u32, count: u32 },
    /// A chart was **moved or resized** on the ChartLayer (P18, `ui_design §3.2`) — both produce a
    /// new [`Anchor`]. The window forwards it as `Command::SetChartAnchor` for the active sheet.
    ChartAnchorChanged { id: ChartId, anchor: Anchor },
    /// A selected chart was **deleted** (Delete/Backspace or the chart context menu, P18). The
    /// window forwards it as `Command::DeleteChart` for the active sheet.
    ChartDeleted { id: ChartId },
    /// A chart became **selected** by a user interaction (a click on its body/handle, P19). The
    /// window opens the right-docked **edit panel** for it (an authored chart is shapeable — set its
    /// range + type); a programmatic `set_selected_chart` does NOT emit this. Fires on every chart
    /// mouse-down (the window dedupes by id), so re-clicking the editing chart is a harmless no-op.
    ChartSelected(ChartId),
}

/// The owner's [`GridEvent`] handler — invoked with full `Window`/`App` access so it can
/// forward to the worker and drive sibling chrome.
type GridEventHandler = dyn Fn(&GridEvent, &mut Window, &mut App);

/// A sink the grid calls to deliver [`GridEvent`]s to its owner. Wrapping a closure (rather
/// than gpui's `EventEmitter`) lets the window route events with full `Window`/`App` access
/// — e.g. forward `ViewportChanged` to the worker and drive the data row on
/// `SelectionChanged`. Use [`GridEventSink::noop`] for the standalone demo / render tests.
pub struct GridEventSink {
    handler: Box<GridEventHandler>,
}

impl GridEventSink {
    /// Builds a sink from an event handler.
    pub fn new(handler: impl Fn(&GridEvent, &mut Window, &mut App) + 'static) -> Self {
        Self {
            handler: Box::new(handler),
        }
    }

    /// A sink that drops every event (demo / render-test scenes with no owner).
    pub fn noop() -> Self {
        Self::new(|_, _, _| {})
    }

    /// Delivers an event to the owner.
    pub(crate) fn emit(&self, event: &GridEvent, window: &mut Window, cx: &mut App) {
        (self.handler)(event, window, cx);
    }
}
