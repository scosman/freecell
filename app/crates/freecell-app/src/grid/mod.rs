//! The custom virtualized spreadsheet grid (`components/grid.md`, `ui_design.md §3.3`).
//!
//! A raw-GPUI view that renders headers, gridlines, cells, and selection for an Excel-max
//! sheet, reading **only** from the resident caches and the published viewport — the render
//! path makes zero engine calls and materializes only the visible viewport
//! (`architecture.md §4`). Port-and-extend of the proven POC
//! (`experiments/04-ui-poc/raw-gpui/src/grid.rs`).
//!
//! Phase 6 scope: static rendering + wheel scroll + clamping + overlay scrollbars +
//! loading overlay, driven by hand-built `freecell-core` fixtures. Mouse selection,
//! keyboard motions, and edge auto-scroll are Phase 8.

pub mod fixtures;
pub mod layout;
mod view;

use std::ops::Range;

use gpui::{App, Window};

use freecell_core::SelectionModel;

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

/// Cell text size (px) — `ui_design.md §3.3` ("13 px bundled Inter").
pub const CELL_FONT_PX: f32 = 13.0;
/// Header text size (px) — small medium-weight labels.
pub const HEADER_FONT_PX: f32 = 11.5;
/// Horizontal text padding inside a cell (px).
pub const CELL_H_PAD: f32 = 4.0;
/// Seconds the overlay scrollbars stay visible after the last scroll before fading.
pub const SCROLLBAR_FADE_SECS: u64 = 2;

/// The grid/cell font family (`ui_design.md §3.3`: bundled Inter). Registered at app
/// startup (Phase 10); until then gpui falls back to the default UI font. Reserved here so
/// the render path names the intended family in one place.
pub const GRID_FONT_FAMILY: &str = "Inter";

/// Events the grid raises to its owner (`WorkbookWindow`, Phase 11). Phase 6 emits only
/// [`GridEvent::ViewportChanged`] (from the scroll path); selection/commit events arrive
/// with input wiring in Phase 8.
#[derive(Debug, Clone)]
pub enum GridEvent {
    /// The selection changed — drives the data row, action row, and ref box.
    SelectionChanged(SelectionModel),
    /// The visible index range changed (pre-overscan). The window forwards this to the
    /// worker as `SetViewport` with its own ~3× overscan.
    ViewportChanged { rows: Range<u32>, cols: Range<u32> },
    /// A click-away happened while the data row was editing (commit the pending edit).
    EditCommitRequested,
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
