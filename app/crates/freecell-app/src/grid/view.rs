//! [`GridView`] — the raw-GPUI spreadsheet grid entity (`components/grid.md`).
//!
//! Per frame the render path (the hot path): reads the viewport size, takes the caches
//! read lock **once** to clone the two `Arc<Axis>` handles + snapshot the visible cells'
//! resolved styles, drops the lock, does one atomic load of the `Publication`, then emits
//! absolutely-positioned divs for the visible viewport + `RENDER_OVERSCAN` only. Zero
//! engine calls; no lock held while painting; no allocation proportional to sheet size
//! (`architecture.md §4`).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use parking_lot::RwLock;

use gpui::{
    canvas, deferred, div, prelude::*, px, rgb, rgba, AnyElement, App, Bounds, Context, Entity,
    FocusHandle, Focusable, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Rgba, SharedString, Window,
};
use gpui_component::input::{Input, InputState};
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};

use freecell_core::cache::SheetCaches;
use freecell_core::color::Rgb;
use freecell_core::publication::{CellKind, Publication};
use freecell_core::refs::{column_label, SheetId};
use freecell_core::selection::{Direction, Motion};
use freecell_core::{apply_motion, Align, Axis, CellRef, RenderStyle, SelectionModel, SheetDims};

use super::input::{command_for_key, GridKeyCommand};
use super::layout::{
    self, ContentArea, GridHit, COL_HEADER_H, RENDER_OVERSCAN, SCROLLBAR_INSET, SCROLLBAR_THICKNESS,
};
use super::{
    GridEvent, GridEventSink, ACCENT, AUTOSCROLL_INTERVAL_MS, CELL_BG, CELL_FONT_PX, CELL_H_PAD,
    CELL_TEXT, EDGE_AUTOSCROLL_HOTZONE_PX, EDGE_AUTOSCROLL_STEP_PX, GRIDLINE, HEADER_BG,
    HEADER_FONT_PX, HEADER_HAIRLINE, HEADER_SELECTED_BG, HEADER_TEXT, SCROLLBAR_FADE_SECS,
    SCROLLBAR_RGBA, SELECTION_FILL_ALPHA,
};

/// The worker-written / UI-read data the grid renders from (`components/grid.md §Public
/// interface`). In Phase 6 these are built from hand fixtures ([`super::fixtures`]); the
/// worker fills them for real from the `DocumentClient`'s shared read-surfaces (Phase 11).
///
/// There is deliberately **no generation counter** here: the grid re-reads the `publication`
/// `ArcSwap` every frame, and the window schedules a repaint on `WorkerEvent::Published` via
/// `grid.notify()` — so a separate generation the grid would poll is redundant.
pub struct GridDataSources {
    /// The active sheet's overscanned viewport values snapshot, swapped by the worker.
    pub publication: Arc<ArcSwap<Publication>>,
    /// The resident geometry + resolved-style caches (worker writes, UI reads).
    pub caches: Arc<RwLock<SheetCaches>>,
}

/// An in-flight mouse drag-selection (`components/grid.md §State`: "anchor cell + last hovered
/// cell"). The anchor is the fixed corner the range extends from; the hovered cell is recomputed
/// from the pointer each move, so only the anchor is retained.
#[derive(Debug, Clone, Copy)]
struct DragState {
    anchor: CellRef,
}

/// The custom virtualized grid view.
pub struct GridView {
    sources: GridDataSources,
    events: GridEventSink,
    focus_handle: FocusHandle,
    active_sheet: SheetId,
    /// Per-sheet content scroll offset (px).
    scroll: HashMap<SheetId, (f64, f64)>,
    /// Per-sheet selection (restored on sheet switch).
    selection: HashMap<SheetId, SelectionModel>,
    /// `Some("name")` renders the file-open loading overlay.
    loading: Option<String>,
    /// Whether the overlay scrollbars are currently shown (set on scroll, faded after 2 s).
    scrollbars_visible: bool,
    /// Render-test / debug override: keep the scrollbars visible regardless of activity.
    force_scrollbars: bool,
    /// Render-test / debug override: freeze the loading spinner to a static icon (no
    /// wall-clock-driven rotation) so a capture is deterministic. The app leaves this off.
    freeze_spinner: bool,
    /// Monotonic scroll epoch; a fade task only hides if the epoch is unchanged when it fires.
    scroll_activity: u64,
    /// The last emitted visible range (for `ViewportChanged` debouncing).
    last_viewport: Option<(Range<u32>, Range<u32>)>,
    /// A pending `scroll_cell_into_view` request applied on the next render.
    pending_reveal: Option<(u32, u32)>,
    /// The in-flight mouse drag-selection, if any (`None` = not dragging).
    drag: Option<DragState>,
    /// Whether the edge auto-scroll timer loop is currently running.
    autoscrolling: bool,
    /// Monotonic epoch; a running auto-scroll loop stops as soon as this changes (drag end /
    /// pointer back inside), the same guard pattern as the scrollbar fade.
    autoscroll_epoch: u64,
    /// Reused per-frame index: visible `(row, col)` → index into the publication's cells.
    cell_index: HashMap<(u32, u32), usize>,
    /// Reused per-frame snapshot: visible `(row, col)` → resolved style (default = absent).
    visible_styles: HashMap<(u32, u32), RenderStyle>,
    /// The grid element's real laid-out bounds, captured during paint (a `canvas` probe).
    /// `None` until the first paint. Used instead of `window.viewport_size()` so the grid's
    /// virtualization + hit-testing are correct once chrome wraps it (the grid is no longer
    /// full-window). Falls back to the window size (whole window) when not yet captured — the
    /// full-window render-tests / demo are unaffected (bounds ≈ the window).
    bounds: Option<Bounds<Pixels>>,

    // ---- Pending-edit overlays (pushed by the chrome, `components/edit_controller.md`) ----
    /// The live cell mirror: raw pending text to paint in `(sheet, cell)` instead of its published
    /// value while an edit is pending (`functional_spec.md §1.2`). `None` when no edit is pending.
    mirror: Option<(SheetId, CellRef, SharedString)>,
    /// The cell the in-cell editor overlay covers, or `None` when the overlay is closed.
    incell_open: Option<CellRef>,
    /// The reused in-cell editor input (owned by the chrome; the grid renders it as the overlay).
    incell_input: Option<Entity<InputState>>,
    /// The in-cell editor's cap-error popover message, if a cap rejection is active there.
    incell_cap: Option<SharedString>,
}

/// The per-frame geometry resolved under the (briefly held) caches read lock.
struct Frame {
    row_axis: Arc<Axis>,
    col_axis: Arc<Axis>,
    total_w: f64,
    total_h: f64,
    rows: Range<u32>,
    cols: Range<u32>,
    row_header_w: f32,
    content_w: f64,
    content_h: f64,
    scroll_x: f64,
    scroll_y: f64,
}

/// Optional per-frame timing captured by [`GridView::build_grid_layers`] for the Phase-12
/// perf harness (`freecell_core::perf`). `None` on the normal render path — zero overhead
/// (no `Instant` is even read).
#[derive(Default)]
struct FrameTiming {
    /// Nanoseconds spent building the visible-cell index from the publication — the
    /// O(published-cells) per-frame scan the perf harness watches (`architecture.md §4`).
    cell_index_ns: u64,
    /// How many content-layer elements (cells + selection overlays) the frame built — the
    /// FORCE + ASSERT witness that the per-cell build actually ran.
    content_cells: u32,
}

impl GridView {
    /// Builds the grid over `sources`, delivering [`GridEvent`]s to `events`. The active
    /// sheet defaults to the publication's sheet, at origin scroll with an A1 selection.
    pub fn new(sources: GridDataSources, events: GridEventSink, cx: &mut Context<Self>) -> Self {
        let active_sheet = sources.publication.load().sheet;
        let mut scroll = HashMap::new();
        scroll.insert(active_sheet, (0.0, 0.0));
        let mut selection = HashMap::new();
        selection.insert(active_sheet, SelectionModel::default());
        Self {
            sources,
            events,
            focus_handle: cx.focus_handle(),
            active_sheet,
            scroll,
            selection,
            loading: None,
            scrollbars_visible: false,
            force_scrollbars: false,
            freeze_spinner: false,
            scroll_activity: 0,
            last_viewport: None,
            pending_reveal: None,
            drag: None,
            autoscrolling: false,
            autoscroll_epoch: 0,
            cell_index: HashMap::new(),
            visible_styles: HashMap::new(),
            bounds: None,
            mirror: None,
            incell_open: None,
            incell_input: None,
            incell_cap: None,
        }
    }

    /// Installs the reused in-cell editor input the chrome owns, so the grid can render the overlay
    /// (`components/edit_controller.md §4.4`). Called once at window wiring time.
    pub fn set_incell_input(&mut self, input: Entity<InputState>, cx: &mut Context<Self>) {
        self.incell_input = Some(input);
        cx.notify();
    }

    /// Pushes the chrome's current edit state onto the grid (live mirror, in-cell overlay cell,
    /// in-cell cap message). `None`s clear the corresponding overlay. Repaints so the mirror tracks
    /// each keystroke (`components/edit_controller.md §4.3–4.4`).
    pub fn set_edit_state(
        &mut self,
        mirror: Option<(SheetId, CellRef, SharedString)>,
        incell_open: Option<CellRef>,
        incell_cap: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.mirror = mirror;
        self.incell_open = incell_open;
        self.incell_cap = incell_cap;
        cx.notify();
    }

    /// The mirror's raw text for `(sheet, cell)` if a pending edit is mirrored there on this sheet
    /// (`functional_spec.md §1.2`).
    fn mirror_text_for(&self, cell: CellRef) -> Option<&str> {
        match &self.mirror {
            Some((sheet, mcell, text)) if *sheet == self.active_sheet && *mcell == cell => {
                Some(text.as_ref())
            }
            _ => None,
        }
    }

    /// The grid's viewport size in px — its real laid-out bounds once captured, else the whole
    /// window (the pre-composition fallback). This is the extent virtualization + scroll-clamp
    /// math measure against, so it must be the grid's own area, not the window's.
    fn viewport_wh(&self, window: &Window) -> (f64, f64) {
        match self.bounds {
            Some(b) => (
                f32::from(b.size.width) as f64,
                f32::from(b.size.height) as f64,
            ),
            None => {
                let vp = window.viewport_size();
                (f32::from(vp.width) as f64, f32::from(vp.height) as f64)
            }
        }
    }

    /// Focuses the grid (window shell hands focus back after a data-row commit / escape).
    pub fn focus_self(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.focus_handle.clone();
        window.focus(&handle, cx);
    }

    /// Switches the active sheet, restoring its scroll + selection (origin + A1 if unseen).
    pub fn set_active_sheet(&mut self, sheet: SheetId, cx: &mut Context<Self>) {
        self.active_sheet = sheet;
        self.scroll.entry(sheet).or_insert((0.0, 0.0));
        self.selection.entry(sheet).or_default();
        // Force the next scroll/publish to re-announce the viewport for the new sheet.
        self.last_viewport = None;
        // Defensive: an edit overlay is anchored to the *previous* sheet's cell (the chrome commits
        // the pending edit before a switch); drop it so it can never leak onto the new sheet.
        self.mirror = None;
        self.incell_open = None;
        self.incell_cap = None;
        cx.notify();
    }

    /// The active sheet's selection (drives the data row / ref box).
    pub fn selection(&self) -> &SelectionModel {
        self.selection
            .get(&self.active_sheet)
            .expect("the active sheet always has a selection entry")
    }

    /// Replaces the active sheet's selection (used by the demo/tests; Phase 8 drives this
    /// from keyboard/mouse input).
    pub fn set_selection(&mut self, selection: SelectionModel, cx: &mut Context<Self>) {
        self.selection.insert(self.active_sheet, selection);
        cx.notify();
    }

    /// Shows/hides the file-open loading overlay ("Opening *name*…").
    pub fn set_loading(&mut self, loading: Option<String>, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }

    /// Forces the overlay scrollbars visible (render-test / debug hook).
    pub fn set_force_scrollbars(&mut self, force: bool, cx: &mut Context<Self>) {
        self.force_scrollbars = force;
        cx.notify();
    }

    /// Freezes the loading spinner to a static loader icon (render-test / debug hook). The
    /// animated `Spinner` rotates by wall-clock elapsed time, so a capture taken at an
    /// arbitrary moment (after `xrefresh`) would be non-deterministic; freezing makes the
    /// `grid_loading_overlay` baseline stable. The normal app leaves this off.
    pub fn set_freeze_spinner(&mut self, freeze: bool, cx: &mut Context<Self>) {
        self.freeze_spinner = freeze;
        cx.notify();
    }

    /// Requests that `(row, col)` be scrolled fully into view on the next render.
    pub fn scroll_cell_into_view(&mut self, row: u32, col: u32, cx: &mut Context<Self>) {
        self.pending_reveal = Some((row, col));
        cx.notify();
    }

    fn scroll_of(&self, sheet: SheetId) -> (f64, f64) {
        self.scroll.get(&sheet).copied().unwrap_or((0.0, 0.0))
    }

    /// Resolves this frame's geometry under one brief caches read lock: clones the two axes,
    /// applies a pending reveal, computes the visible ranges + content area, and snapshots
    /// the visible cells' resolved styles into `visible_styles`. Returns `None` (blank grid)
    /// when the active sheet has no resident cache yet.
    fn resolve_frame(&mut self, viewport_w: f64, viewport_h: f64) -> Option<Frame> {
        let (mut scroll_x, mut scroll_y) = self.scroll_of(self.active_sheet);
        let active = self.active_sheet;
        let reveal = self.pending_reveal.take();

        let caches = self.sources.caches.read();
        let cache = caches.get(active)?;
        let (row_axis, col_axis) = cache.axes();
        let total_w = cache.total_width();
        let total_h = cache.total_height();
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        // Compute visible ranges + gutter width for a given scroll (the gutter width depends
        // on the deepest visible row, which depends on scroll — hence a small closure).
        let ranges = |sx: f64, sy: f64| -> (Range<u32>, f32, f64, Range<u32>) {
            let rows = row_axis.visible_range(sy, content_h, RENDER_OVERSCAN);
            let row_header_w = layout::row_header_width(rows.end.saturating_sub(1));
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let cols = col_axis.visible_range(sx, content_w, RENDER_OVERSCAN);
            (rows, row_header_w, content_w, cols)
        };

        let (mut rows, mut row_header_w, mut content_w, mut cols) = ranges(scroll_x, scroll_y);

        if let Some((rr, rc)) = reveal {
            // Size the reveal's content area with the gutter for the *target* row too:
            // revealing a much deeper row widens the gutter across a digit-count boundary,
            // which would shrink the real content width. Using the wider of the current and
            // target-row gutters is conservative — it never over-estimates content width, so a
            // cell near the right/bottom edge is never revealed a few px short (the second-order
            // edge this guards; drivers land in Phase 8).
            let reveal_gutter = row_header_w.max(layout::row_header_width(rr));
            let area = ContentArea {
                row_header_w: reveal_gutter,
                width: (viewport_w - reveal_gutter as f64).max(0.0),
                height: content_h,
            };
            let (nx, ny) =
                layout::scroll_to_reveal(rr, rc, &row_axis, &col_axis, area, scroll_x, scroll_y);
            scroll_x = nx;
            scroll_y = ny;
            self.scroll.insert(active, (nx, ny));
            let r = ranges(scroll_x, scroll_y);
            rows = r.0;
            row_header_w = r.1;
            content_w = r.2;
            cols = r.3;
        }

        // Snapshot the visible cells' resolved styles (copied — `RenderStyle: Copy`), so the
        // lock is released before any painting. Bounded by visible cell count.
        self.visible_styles.clear();
        for r in rows.clone() {
            for c in cols.clone() {
                if let Some(style) = cache.render_style(r, c) {
                    self.visible_styles.insert((r, c), *style);
                }
            }
        }
        drop(caches);

        Some(Frame {
            row_axis,
            col_axis,
            total_w,
            total_h,
            rows,
            cols,
            row_header_w,
            content_w,
            content_h,
            scroll_x,
            scroll_y,
        })
    }

    /// Wheel/trackpad scroll: convert the delta to px, clamp per axis, store, keep the
    /// scrollbars alive, and announce a `ViewportChanged` when the visible index range moved.
    fn handle_scroll(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The grid's own laid-out bounds (`viewport_wh`), not the whole window — so the
        // visible-range / scroll-clamp math is correct now that chrome wraps the grid.
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let line_height = window.line_height();
        let delta = event.delta.pixel_delta(line_height);
        let dx = f32::from(delta.x) as f64;
        let dy = f32::from(delta.y) as f64;
        let (sx0, sy0) = self.scroll_of(self.active_sheet);
        let active = self.active_sheet;

        let resolved = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let total_w = cache.total_width();
            let total_h = cache.total_height();
            let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

            // Tentative new scroll, then clamp using the gutter width for the tentative rows.
            let tentative_y = (sy0 - dy).max(0.0);
            let tentative_rows = row_axis.visible_range(tentative_y, content_h, RENDER_OVERSCAN);
            let row_header_w = layout::row_header_width(tentative_rows.end.saturating_sub(1));
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let area = ContentArea {
                row_header_w,
                width: content_w,
                height: content_h,
            };
            let (nx, ny) = layout::clamp_scroll(sx0 - dx, sy0 - dy, total_w, total_h, area);
            let rows = row_axis.visible_range(ny, content_h, RENDER_OVERSCAN);
            let cols = col_axis.visible_range(nx, content_w, RENDER_OVERSCAN);
            (nx, ny, rows, cols)
        };
        let (nx, ny, rows, cols) = resolved;

        self.scroll.insert(active, (nx, ny));
        self.mark_scrollbars_active(cx);

        // Debounced ViewportChanged: only when the visible index range actually moved.
        let ranges = (rows, cols);
        if self.last_viewport.as_ref() != Some(&ranges) {
            self.last_viewport = Some(ranges.clone());
            let (rows, cols) = ranges;
            self.events
                .emit(&GridEvent::ViewportChanged { rows, cols }, window, cx);
        }
        cx.notify();
    }

    /// Marks the scrollbars visible and schedules a fade after [`SCROLLBAR_FADE_SECS`] that
    /// only fires if no newer scroll happened (epoch guard) and no force override is set.
    fn mark_scrollbars_active(&mut self, cx: &mut Context<Self>) {
        self.scrollbars_visible = true;
        self.scroll_activity = self.scroll_activity.wrapping_add(1);
        let epoch = self.scroll_activity;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(SCROLLBAR_FADE_SECS))
                .await;
            this.update(cx, |this, cx| {
                if this.scroll_activity == epoch && !this.force_scrollbars {
                    this.scrollbars_visible = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- Input plumbing (Phase 8) ---------------------------------------------------------

    /// Replaces the active sheet's selection and announces it (`SelectionChanged`). Unlike the
    /// demo-facing [`GridView::set_selection`], this is the input path, so it emits the event
    /// that drives the data row / ref box (`components/grid.md §Public interface`).
    fn set_selection_and_emit(
        &mut self,
        selection: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection.insert(self.active_sheet, selection);
        self.events
            .emit(&GridEvent::SelectionChanged(selection), window, cx);
    }

    /// The active sheet's dimensions (axis track counts), for clamping keyboard motions. `None`
    /// when the sheet has no resident cache yet (no motion possible).
    fn sheet_dims(&self) -> Option<SheetDims> {
        let caches = self.sources.caches.read();
        let cache = caches.get(self.active_sheet)?;
        let (row_axis, col_axis) = cache.axes();
        Some(SheetDims::new(row_axis.count(), col_axis.count()))
    }

    /// The number of rows in the current viewport — the Page Up/Down step
    /// (`ui_design.md §6`). At least 1 so a page always advances.
    fn page_rows(&self, window: &Window) -> u32 {
        let (_, scroll_y) = self.scroll_of(self.active_sheet);
        let (_, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        let caches = self.sources.caches.read();
        let Some(cache) = caches.get(self.active_sheet) else {
            return 1;
        };
        let (row_axis, _) = cache.axes();
        let visible = row_axis.visible_range(scroll_y, content_h, 0);
        (visible.end - visible.start).max(1)
    }

    /// The grid-local pixel of a mouse event (window-absolute position minus the grid element's
    /// captured origin), so hit-testing is correct once chrome wraps the grid. Falls back to raw
    /// window coordinates before the first paint captures bounds (grid ≈ full window then).
    fn event_local(&self, position: gpui::Point<gpui::Pixels>) -> (f32, f32) {
        let (ox, oy) = match self.bounds {
            Some(b) => (f32::from(b.origin.x), f32::from(b.origin.y)),
            None => (0.0, 0.0),
        };
        (f32::from(position.x) - ox, f32::from(position.y) - oy)
    }

    /// The row-header gutter width for the current scroll (matches the render path's sizing).
    fn gutter_width(row_axis: &Axis, scroll_y: f64, content_h: f64) -> f32 {
        let rows = row_axis.visible_range(scroll_y, content_h, RENDER_OVERSCAN);
        layout::row_header_width(rows.end.saturating_sub(1))
    }

    /// Mouse down: claim keyboard focus, hit-test, and (on a data cell) set the selection —
    /// shift-click extends from the current anchor, a plain click selects the single cell — then
    /// begin a drag from the resulting anchor. Header / corner clicks are a no-op in the MVP
    /// (`components/grid.md §Input`).
    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Focus the grid so arrow keys route here (the window arranges focus after commits).
        let handle = self.focus_handle.clone();
        window.focus(&handle, cx);

        let (local_x, local_y) = self.event_local(event.position);
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (_, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        let hit = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            layout::hit_test(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
            )
        };
        let GridHit::Cell { row, col } = hit else {
            return; // header / corner: no-op in MVP (row/col selection is a P2 feature)
        };

        let cell = CellRef::new(row, col);
        let selection = if event.modifiers.shift {
            // Shift-click extends the range from the existing anchor.
            SelectionModel {
                anchor: self.selection().anchor,
                active: cell,
            }
        } else {
            SelectionModel::single(cell)
        };
        let is_double = event.click_count >= 2;
        // The second mousedown of a double-click lands on the already-selected cell. Re-emitting
        // its `SelectionChanged` would restart the content fetch and blank the in-cell editor about
        // to open (data loss; `functional_spec.md §1.3`, review #1), so skip the redundant select
        // and only open the editor.
        let already_active_single = self.selection().is_single() && *self.selection() == selection;
        if !(is_double && already_active_single) {
            self.set_selection_and_emit(selection, window, cx);
        }
        if is_double {
            self.events
                .emit(&GridEvent::OpenInCellEditor(cell), window, cx);
        }
        // Begin a drag from the (kept or new) anchor; subsequent moves extend to the hovered cell.
        self.drag = Some(DragState {
            anchor: selection.anchor,
        });
        cx.notify();
    }

    /// Mouse move: while dragging, extend the selection to the hovered cell and — when the
    /// pointer is past a viewport edge — kick off the edge auto-scroll loop.
    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag else {
            return; // not dragging — nothing to do
        };
        let (local_x, local_y) = self.event_local(event.position);
        self.extend_drag_to_point(drag.anchor, local_x, local_y, window, cx);
        self.maybe_start_autoscroll(window, cx);
    }

    /// Mouse up: end the drag (stopping any auto-scroll loop via the epoch) and let the
    /// scrollbars fade if a drag-scroll had shown them.
    fn handle_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag.take().is_some() {
            // Bump the epoch to stop the loop, but deliberately DON'T clear `autoscrolling` here:
            // the running loop still clears it itself on its next tick (≤ AUTOSCROLL_INTERVAL_MS).
            // That leaves a narrow window where a brand-new drag past an edge won't relaunch
            // auto-scroll until its next move event — an acceptable, self-healing trade-off for
            // the stronger guarantee that only ONE loop is ever live (clearing the flag here could
            // let a new loop start while the old one is still awaiting its timer → double speed).
            self.autoscroll_epoch = self.autoscroll_epoch.wrapping_add(1);
            if self.scrollbars_visible {
                self.mark_scrollbars_active(cx); // schedule the fade-out
            }
        }
    }

    /// Extend the drag selection: map the pointer to a data cell and move `active` there,
    /// keeping the drag's anchor. Emits `SelectionChanged` only when the cell actually changed.
    fn extend_drag_to_point(
        &mut self,
        anchor: CellRef,
        local_x: f32,
        local_y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        let cell = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            layout::cell_at_point(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
                content_w,
                content_h,
            )
        };
        let selection = SelectionModel {
            anchor,
            active: cell,
        };
        if *self.selection() != selection {
            self.set_selection_and_emit(selection, window, cx);
            cx.notify();
        }
    }

    /// The current per-axis edge auto-scroll delta for the live pointer (`0` inside the content).
    fn current_edge_delta(&self, window: &Window) -> (f64, f64) {
        let pos = window.mouse_position();
        let (local_x, local_y) = self.event_local(pos);
        let active = self.active_sheet;
        let (_, scroll_y) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        let caches = self.sources.caches.read();
        let Some(cache) = caches.get(active) else {
            return (0.0, 0.0);
        };
        let (row_axis, _) = cache.axes();
        let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
        let content_w = (viewport_w - row_header_w as f64).max(0.0);
        layout::edge_autoscroll_delta(
            local_x,
            local_y,
            row_header_w,
            content_w,
            content_h,
            EDGE_AUTOSCROLL_STEP_PX,
            EDGE_AUTOSCROLL_HOTZONE_PX,
        )
    }

    /// If a drag is active and the pointer is past a viewport edge, start the auto-scroll loop
    /// (a `spawn_in` timer, so it advances even while the pointer is held still with no
    /// mouse-move events — the "drag to the edge and wait" case).
    fn maybe_start_autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autoscrolling || self.drag.is_none() {
            return;
        }
        let (dx, dy) = self.current_edge_delta(window);
        if dx == 0.0 && dy == 0.0 {
            return; // pointer inside — no auto-scroll needed yet
        }
        self.autoscrolling = true;
        self.autoscroll_epoch = self.autoscroll_epoch.wrapping_add(1);
        self.scrollbars_visible = true; // scrolling shows the overlay bars
        let epoch = self.autoscroll_epoch;
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(AUTOSCROLL_INTERVAL_MS))
                .await;
            let keep = this
                .update_in(cx, |this, window, cx| {
                    if this.autoscroll_epoch != epoch || this.drag.is_none() {
                        this.autoscrolling = false;
                        return false;
                    }
                    this.autoscroll_tick(window, cx)
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        })
        .detach();
    }

    /// One auto-scroll frame: apply the fixed edge step (clamped), re-extend the selection to the
    /// hovered cell, and announce a debounced `ViewportChanged`. Returns whether to keep looping
    /// (`false` once the pointer returns inside the content, stopping the loop).
    fn autoscroll_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.drag else {
            self.autoscrolling = false;
            return false;
        };
        let pos = window.mouse_position();
        let (local_x, local_y) = self.event_local(pos);
        let active = self.active_sheet;
        let (scroll_x0, scroll_y0) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        let step = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                self.autoscrolling = false;
                return false;
            };
            let (row_axis, col_axis) = cache.axes();
            let total_w = cache.total_width();
            let total_h = cache.total_height();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y0, content_h);
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let (dx, dy) = layout::edge_autoscroll_delta(
                local_x,
                local_y,
                row_header_w,
                content_w,
                content_h,
                EDGE_AUTOSCROLL_STEP_PX,
                EDGE_AUTOSCROLL_HOTZONE_PX,
            );
            if dx == 0.0 && dy == 0.0 {
                None // pointer back inside — stop
            } else {
                let area = ContentArea {
                    row_header_w,
                    width: content_w,
                    height: content_h,
                };
                let (nx, ny) =
                    layout::clamp_scroll(scroll_x0 + dx, scroll_y0 + dy, total_w, total_h, area);
                let cell = layout::cell_at_point(
                    local_x,
                    local_y,
                    row_header_w,
                    nx,
                    ny,
                    &row_axis,
                    &col_axis,
                    content_w,
                    content_h,
                );
                let rows = row_axis.visible_range(ny, content_h, RENDER_OVERSCAN);
                let cols = col_axis.visible_range(nx, content_w, RENDER_OVERSCAN);
                Some((nx, ny, cell, rows, cols))
            }
        };

        let Some((nx, ny, cell, rows, cols)) = step else {
            self.autoscrolling = false;
            return false;
        };

        let mut changed = false;
        if (nx, ny) != (scroll_x0, scroll_y0) {
            self.scroll.insert(active, (nx, ny));
            self.scrollbars_visible = true;
            changed = true;
        }
        let selection = SelectionModel {
            anchor: drag.anchor,
            active: cell,
        };
        if *self.selection() != selection {
            self.set_selection_and_emit(selection, window, cx);
            changed = true;
        }
        let ranges = (rows, cols);
        if self.last_viewport.as_ref() != Some(&ranges) {
            self.last_viewport = Some(ranges.clone());
            let (rows, cols) = ranges;
            self.events
                .emit(&GridEvent::ViewportChanged { rows, cols }, window, cx);
        }
        if changed {
            cx.notify();
        }
        true // still past an edge — keep auto-scrolling
    }

    /// Reveals `(row, col)` immediately (not via the render-time `pending_reveal`) and announces
    /// a debounced `ViewportChanged`, so a keyboard motion that scrolls the active cell into view
    /// re-publishes the newly visible window. Mirrors `handle_scroll`'s viewport-announce.
    fn reveal_and_announce(
        &mut self,
        row: u32,
        col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_sheet;
        let (scroll_x0, scroll_y0) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        let (nx, ny, rows, cols) = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            // Gutter wide enough for both the current view and the (possibly deeper) target row,
            // matching `resolve_frame`'s conservative reveal sizing.
            let cur_rows = row_axis.visible_range(scroll_y0, content_h, RENDER_OVERSCAN);
            let reveal_gutter = layout::row_header_width(cur_rows.end.saturating_sub(1))
                .max(layout::row_header_width(row));
            let content_w = (viewport_w - reveal_gutter as f64).max(0.0);
            let area = ContentArea {
                row_header_w: reveal_gutter,
                width: content_w,
                height: content_h,
            };
            let (nx, ny) = layout::scroll_to_reveal(
                row, col, &row_axis, &col_axis, area, scroll_x0, scroll_y0,
            );
            // Recompute the visible ranges (and gutter/content) at the new scroll.
            let rows = row_axis.visible_range(ny, content_h, RENDER_OVERSCAN);
            let content_w2 =
                (viewport_w - layout::row_header_width(rows.end.saturating_sub(1)) as f64).max(0.0);
            let cols = col_axis.visible_range(nx, content_w2, RENDER_OVERSCAN);
            (nx, ny, rows, cols)
        };

        self.scroll.insert(active, (nx, ny));
        let ranges = (rows, cols);
        if self.last_viewport.as_ref() != Some(&ranges) {
            self.last_viewport = Some(ranges.clone());
            let (rows, cols) = ranges;
            self.events
                .emit(&GridEvent::ViewportChanged { rows, cols }, window, cx);
        }
        cx.notify();
    }

    /// Key down: resolve the MVP keyboard map (`ui_design.md §6`) to a grid command and dispatch
    /// it — a motion updates the selection via `apply_motion` (then reveals the active cell), a
    /// clear emits `ClearCells`. Unmapped keys are ignored (propagate).
    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While the in-cell editor owns the keyboard, the grid's own motions / type-to-replace stay
        // out of the way; its Tab/Escape are handled at the grid root's capture handler
        // (`components/edit_controller.md §Grid integration`).
        if self.incell_open.is_some() {
            return;
        }

        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        // F2 opens the in-cell editor on a single-cell selection (`functional_spec.md §1.3`).
        if key == "f2" && !modifiers.modified() && self.selection().is_single() {
            let active = self.selection().active;
            self.events
                .emit(&GridEvent::OpenInCellEditor(active), window, cx);
            return;
        }

        let shift = modifiers.shift;
        // `secondary()` = Cmd on macOS, Ctrl on Linux — resolved here so the mapper stays
        // platform-agnostic (`ui_design.md §6` Linux note).
        let secondary = modifiers.secondary();
        // Only Page Up/Down need the viewport height in rows; computing it takes a caches read
        // lock, so resolve it lazily to keep every other keystroke lock-free.
        let page_rows = if matches!(key, "pageup" | "pagedown") {
            self.page_rows(window)
        } else {
            0
        };

        let Some(command) = command_for_key(key, shift, secondary, page_rows) else {
            // An unmapped key may be a printable one → start a type-to-replace edit.
            self.maybe_type_to_edit(event, window, cx);
            return;
        };
        match command {
            GridKeyCommand::Motion(motion) => self.move_active(motion, window, cx),
            GridKeyCommand::ClearCells => {
                let range = self.selection().range();
                self.events.emit(&GridEvent::ClearCells(range), window, cx);
            }
        }
    }

    /// Type-to-replace (`functional_spec.md §1.1`): a printable, modifier-free (Shift allowed)
    /// keystroke starts an edit whose content is the typed character. A multi-cell selection first
    /// collapses to the active cell (Excel behaviour). Cmd/Ctrl/Alt combinations and control keys
    /// (Enter/Tab/arrows/…) never qualify (`key_char` is `None`/control for those).
    fn maybe_type_to_edit(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let m = &event.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        let Some(ch) = event.keystroke.key_char.as_deref() else {
            return;
        };
        if ch.is_empty() || ch.chars().any(char::is_control) {
            return;
        }
        if !self.selection().is_single() {
            let active = self.selection().active;
            self.set_selection_and_emit(SelectionModel::single(active), window, cx);
        }
        self.events
            .emit(&GridEvent::TypeToEdit(ch.to_string()), window, cx);
    }

    /// Applies a keyboard `motion` to the active sheet's selection (clamped to sheet bounds),
    /// emits `SelectionChanged`, and reveals the active cell — the shared path for both the
    /// grid's own key handler and the chrome's data-row commit (`MoveActive`, Phase 11). A
    /// motion that changes nothing (at a sheet edge) is a no-op.
    pub fn move_active(&mut self, motion: Motion, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dims) = self.sheet_dims() else {
            return;
        };
        let selection = apply_motion(*self.selection(), motion, dims);
        if *self.selection() != selection {
            self.set_selection_and_emit(selection, window, cx);
            self.reveal_and_announce(selection.active.row, selection.active.col, window, cx);
        }
    }

    /// Build the frame-dependent element layers (cells + selection, headers, scrollbars) from a
    /// resolved [`Frame`] — the shared hot path used by both [`Render::render`] and the Phase-12
    /// perf harness's [`GridView::measure_frame`], so the measured build never drifts from the
    /// real render. When `timing` is `Some`, records the publication-scan duration + content-cell
    /// count (the perf harness's cell-load witness); `None` on the render path reads no clock.
    fn build_grid_layers(
        &mut self,
        frame: &Frame,
        mut timing: Option<&mut FrameTiming>,
    ) -> Vec<AnyElement> {
        let mut root_children: Vec<AnyElement> = Vec::new();

        let selection = *self.selection();
        let publication = self.sources.publication.load_full();
        let covers_active = publication.sheet == self.active_sheet;

        // Rebuild the reused visible-cell index from the publication. The scan is over the
        // published (non-empty) cells, which the worker caps at `MAX_PUBLISH_ROWS ×
        // MAX_PUBLISH_COLS` (512×256) and are typically far fewer than that — not O(sheet).
        // The publication has no spatial index, so a per-visible-cell lookup would need this
        // map first; building it once per frame is the right structure given the flat `Vec`.
        // PHASE 12: the perf harness times this scan and gates the frame p99 (`freecell_core::perf`).
        let index_start = timing.as_ref().map(|_| std::time::Instant::now());
        self.cell_index.clear();
        if covers_active {
            for (i, cell) in publication.cells.iter().enumerate() {
                if frame.rows.contains(&cell.row) && frame.cols.contains(&cell.col) {
                    self.cell_index.insert((cell.row, cell.col), i);
                }
            }
        }
        if let (Some(t), Some(start)) = (timing.as_mut(), index_start) {
            t.cell_index_ns = start.elapsed().as_nanos() as u64;
        }

        // ---- Content layer: cells + selection, clipped to the content area ----------
        let mut content_children: Vec<AnyElement> = Vec::with_capacity(
            ((frame.rows.end - frame.rows.start) * (frame.cols.end - frame.cols.start)) as usize
                + 16,
        );

        for r in frame.rows.clone() {
            for c in frame.cols.clone() {
                let (x, y, w, h) = cell_rect(r, c, frame);
                let style = self.visible_styles.get(&(r, c)).copied();
                let fill = style
                    .and_then(|s| s.fill)
                    .map(to_rgba)
                    .unwrap_or_else(|| rgb(CELL_BG));
                // A pending edit mirrors its raw text here in the grid's default style, left-
                // aligned, over the committed value (`functional_spec.md §1.2`). The cell's fill is
                // kept (no flash), but text attributes / alignment fall back to default (`None`).
                let (text, text_color, kind, attr_style) =
                    match self.mirror_text_for(CellRef::new(r, c)) {
                        Some(raw) => (raw.to_string(), rgb(CELL_TEXT), CellKind::Text, None),
                        None => match self.cell_index.get(&(r, c)) {
                            Some(&idx) => {
                                let pc = &publication.cells[idx];
                                // `pc.text_color` is already fully resolved (explicit non-black font
                                // colour → number-format colour), so the `.or(font_color)` fallback is
                                // redundant here — kept as a harmless minimal-diff belt-and-braces
                                // (both use the same black-filter, so they agree). See DECISIONS §4.
                                let color = pc
                                    .text_color
                                    .or(style.and_then(|s| s.font_color))
                                    .map(to_rgba)
                                    .unwrap_or_else(|| rgb(CELL_TEXT));
                                (pc.display_text.clone(), color, pc.kind, style)
                            }
                            // Empty cells carry no text, so their kind never drives alignment.
                            None => (String::new(), rgb(CELL_TEXT), CellKind::Text, style),
                        },
                    };
                content_children.push(cell_element(
                    x, y, w, h, fill, text, text_color, kind, attr_style,
                ));
            }
        }

        // Selection: translucent overlay (range − active), range border, active border.
        let range = selection.range();
        for (rows, cols) in layout::range_overlay_rects(range, selection.active) {
            // Clip to the visible ranges so the overlay divs stay viewport-sized.
            let rows = rows.start.max(frame.rows.start)..rows.end.min(frame.rows.end);
            let cols = cols.start.max(frame.cols.start)..cols.end.min(frame.cols.end);
            if rows.start >= rows.end || cols.start >= cols.end {
                continue;
            }
            let (x, y, w, h) = span_rect(rows, cols, frame);
            content_children.push(
                rect_div(x, y, w, h)
                    .bg(rgb(ACCENT).opacity(SELECTION_FILL_ALPHA))
                    .into_any_element(),
            );
        }
        if !range.is_single() {
            let (x, y, w, h) = span_rect(
                range.start.row..range.end.row + 1,
                range.start.col..range.end.col + 1,
                frame,
            );
            content_children.push(
                rect_div(x, y, w, h)
                    .border_2()
                    .border_color(rgb(ACCENT))
                    .into_any_element(),
            );
        }
        {
            let (x, y, w, h) = cell_rect(selection.active.row, selection.active.col, frame);
            content_children.push(
                rect_div(x, y, w, h)
                    .border_2()
                    .border_color(rgb(ACCENT))
                    .into_any_element(),
            );
        }

        // In-cell editor overlay (deferred → painted above the cells; `functional_spec.md §1.3`).
        // Rendered even when the anchored cell is scrolled out of view — the content layer's
        // `overflow_hidden` clips it, and keeping it in the tree preserves the input's focus.
        if let (Some(cell), Some(input)) = (self.incell_open, self.incell_input.clone()) {
            content_children.extend(self.in_cell_overlay_elements(cell, &input, frame));
        }

        if let Some(t) = timing.as_mut() {
            t.content_cells = content_children.len() as u32;
        }

        root_children.push(
            div()
                .absolute()
                .left(px(frame.row_header_w))
                .top(px(COL_HEADER_H))
                .w(px(frame.content_w as f32))
                .h(px(frame.content_h as f32))
                .overflow_hidden()
                .children(content_children)
                .into_any_element(),
        );

        // ---- Header layer (fixed, opaque, clipped to its strip) ---------------------
        let (sel_r0, sel_r1) = (range.start.row, range.end.row);
        let (sel_c0, sel_c1) = (range.start.col, range.end.col);

        // Column-header strip.
        let mut col_children: Vec<AnyElement> = Vec::new();
        for c in frame.cols.clone() {
            let x = (frame.col_axis.offset_of(c) - frame.scroll_x) as f32;
            let w = frame.col_axis.size_of(c);
            let selected = c >= sel_c0 && c <= sel_c1;
            col_children.push(header_element(
                x,
                0.0,
                w,
                COL_HEADER_H,
                column_label(c),
                selected,
            ));
        }
        // Accent edge under the selected columns.
        {
            let (x, _y, w, _h) = span_rect(0..1, sel_c0..sel_c1 + 1, frame);
            col_children.push(
                rect_div(x, COL_HEADER_H - 2.0, w, 2.0)
                    .bg(rgb(ACCENT))
                    .into_any_element(),
            );
        }
        root_children.push(
            div()
                .absolute()
                .left(px(frame.row_header_w))
                .top(px(0.0))
                .w(px(frame.content_w as f32))
                .h(px(COL_HEADER_H))
                .bg(rgb(HEADER_BG))
                .overflow_hidden()
                .children(col_children)
                .into_any_element(),
        );

        // Row-header gutter.
        let mut row_children: Vec<AnyElement> = Vec::new();
        for r in frame.rows.clone() {
            let y = (frame.row_axis.offset_of(r) - frame.scroll_y) as f32;
            let h = frame.row_axis.size_of(r);
            let selected = r >= sel_r0 && r <= sel_r1;
            row_children.push(header_element(
                0.0,
                y,
                frame.row_header_w,
                h,
                (r + 1).to_string(),
                selected,
            ));
        }
        // Accent edge beside the selected rows.
        {
            let (_x, y, _w, h) = span_rect(sel_r0..sel_r1 + 1, 0..1, frame);
            row_children.push(
                rect_div(frame.row_header_w - 2.0, y, 2.0, h)
                    .bg(rgb(ACCENT))
                    .into_any_element(),
            );
        }
        root_children.push(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(COL_HEADER_H))
                .w(px(frame.row_header_w))
                .h(px(frame.content_h as f32))
                .bg(rgb(HEADER_BG))
                .overflow_hidden()
                .children(row_children)
                .into_any_element(),
        );

        // Top-left corner cap.
        root_children.push(
            rect_div(0.0, 0.0, frame.row_header_w, COL_HEADER_H)
                .bg(rgb(HEADER_BG))
                .border_r_1()
                .border_b_1()
                .border_color(rgb(HEADER_HAIRLINE))
                .into_any_element(),
        );

        // ---- Overlay scrollbars -----------------------------------------------------
        if self.scrollbars_visible || self.force_scrollbars {
            if let Some(thumb) = layout::scrollbar_thumb(
                frame.total_h,
                frame.content_h,
                frame.scroll_y,
                frame.content_h as f32,
            ) {
                let x = frame.row_header_w + frame.content_w as f32
                    - SCROLLBAR_THICKNESS
                    - SCROLLBAR_INSET;
                root_children.push(
                    rect_div(
                        x,
                        COL_HEADER_H + thumb.offset,
                        SCROLLBAR_THICKNESS,
                        thumb.length,
                    )
                    .bg(rgba(SCROLLBAR_RGBA))
                    .rounded_full()
                    .into_any_element(),
                );
            }
            if let Some(thumb) = layout::scrollbar_thumb(
                frame.total_w,
                frame.content_w,
                frame.scroll_x,
                frame.content_w as f32,
            ) {
                let y =
                    COL_HEADER_H + frame.content_h as f32 - SCROLLBAR_THICKNESS - SCROLLBAR_INSET;
                root_children.push(
                    rect_div(
                        frame.row_header_w + thumb.offset,
                        y,
                        thumb.length,
                        SCROLLBAR_THICKNESS,
                    )
                    .bg(rgba(SCROLLBAR_RGBA))
                    .rounded_full()
                    .into_any_element(),
                );
            }
        }

        root_children
    }

    /// Phase-12 perf hook (`freecell_core::perf`): apply a scripted scroll to the active sheet,
    /// run the **real** render build path (`resolve_frame` + `build_grid_layers`), and return a
    /// timed [`FrameSample`](freecell_core::perf::FrameSample) plus the resulting visible ranges.
    ///
    /// Timing follows the POC's methodology: `cell_load_ns` covers the data resolution (visible
    /// style snapshot in `resolve_frame` + the publication scan), and `frame_render_ns` covers the
    /// whole build (data + element construction). gpui layout, text shaping, and GPU present run
    /// **after** this returns, inside gpui's paint — under lavapipe those are unrepresentative and
    /// are deliberately NOT timed here (a macos-verify concern; see DECISIONS_TO_REVIEW.md).
    ///
    /// FORCE + ASSERT (`CLAUDE.md`): the scroll is clamped to the sheet extents and applied (so a
    /// deep jump lands in-bounds), and the build must produce a non-empty, `black_box`ed content
    /// layer — a measurement that touched no cells panics rather than silently reporting a no-op.
    pub fn measure_frame(
        &mut self,
        scroll_x: f64,
        scroll_y: f64,
        viewport_w: f64,
        viewport_h: f64,
        prev: Option<(Range<u32>, Range<u32>)>,
    ) -> (freecell_core::perf::FrameSample, (Range<u32>, Range<u32>)) {
        use std::time::Instant;

        // Clamp the scripted scroll to the active sheet's real extents, then apply it.
        let active = self.active_sheet;
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        let (nx, ny) = {
            let caches = self.sources.caches.read();
            let cache = caches
                .get(active)
                .expect("perf fixture must build the active-sheet cache");
            let (row_axis, _) = cache.axes();
            let total_w = cache.total_width();
            let total_h = cache.total_height();
            let rows = row_axis.visible_range(scroll_y.max(0.0), content_h, RENDER_OVERSCAN);
            let row_header_w = layout::row_header_width(rows.end.saturating_sub(1));
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let area = ContentArea {
                row_header_w,
                width: content_w,
                height: content_h,
            };
            layout::clamp_scroll(scroll_x, scroll_y, total_w, total_h, area)
        };
        self.scroll.insert(active, (nx, ny));

        // Time the data resolution (axis math + visible-style snapshot under the caches lock).
        let t0 = Instant::now();
        let frame = self
            .resolve_frame(viewport_w, viewport_h)
            .expect("perf fixture must build the active-sheet cache");
        let resolve_ns = t0.elapsed().as_nanos() as u64;

        let ranges = (frame.rows.clone(), frame.cols.clone());
        let mut timing = FrameTiming::default();
        let layers = self.build_grid_layers(&frame, Some(&mut timing));
        let frame_render_ns = t0.elapsed().as_nanos() as u64;
        let cell_load_ns = resolve_ns + timing.cell_index_ns;

        // FORCE + ASSERT: the per-cell build actually ran; keep the built layers observable so the
        // optimiser can't elide the construction we just timed.
        assert!(
            timing.content_cells > 0 && !layers.is_empty(),
            "measure_frame built no content — refusing to report a no-op measurement"
        );
        std::hint::black_box(&layers);
        drop(layers);

        let newly_visible = match &prev {
            Some((pr, pc)) => freecell_core::perf::newly_visible_2d(pr, pc, &ranges.0, &ranges.1),
            None => (ranges.0.end - ranges.0.start) * (ranges.1.end - ranges.1.start),
        };

        (
            freecell_core::perf::FrameSample {
                frame_render_ns,
                cell_load_ns,
                newly_visible,
                elements: timing.content_cells,
            },
            ranges,
        )
    }
}

/// Maps a core `Rgb` onto a gpui colour.
fn to_rgba(c: Rgb) -> Rgba {
    rgb(c.to_hex())
}

/// A cell's px rectangle in **content-local** coordinates (origin at the content area's
/// top-left, before the scroll offset is applied via the axis offsets).
fn cell_rect(row: u32, col: u32, frame: &Frame) -> (f32, f32, f32, f32) {
    let x = (frame.col_axis.offset_of(col) - frame.scroll_x) as f32;
    let y = (frame.row_axis.offset_of(row) - frame.scroll_y) as f32;
    let w = frame.col_axis.size_of(col);
    let h = frame.row_axis.size_of(row);
    (x, y, w, h)
}

/// The px rectangle spanning an index range `[c0, c1) × [r0, r1)` in content-local coords.
fn span_rect(rows: Range<u32>, cols: Range<u32>, frame: &Frame) -> (f32, f32, f32, f32) {
    let x0 = frame.col_axis.offset_of(cols.start) - frame.scroll_x;
    let x1 = frame.col_axis.offset_of(cols.end) - frame.scroll_x;
    let y0 = frame.row_axis.offset_of(rows.start) - frame.scroll_y;
    let y1 = frame.row_axis.offset_of(rows.end) - frame.scroll_y;
    (x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32)
}

/// Builds one data cell element (fill, gridlines, text with resolved style attributes).
#[allow(clippy::too_many_arguments)]
fn cell_element(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill: Rgba,
    text: String,
    text_color: Rgba,
    kind: CellKind,
    style: Option<RenderStyle>,
) -> AnyElement {
    let mut el = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .bg(fill)
        // Right + bottom gridlines only — a fill paints over them (Excel look).
        .border_r_1()
        .border_b_1()
        .border_color(rgb(GRIDLINE))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .px(px(CELL_H_PAD))
        .text_size(px(CELL_FONT_PX))
        .text_color(text_color);

    // Explicit alignment wins; otherwise fall back to the cell's type-aware default
    // (numbers/dates right, booleans/errors center, text left — `architecture.md §1.3`).
    el = match style
        .and_then(|s| s.h_align)
        .unwrap_or_else(|| kind.default_align())
    {
        Align::Left => el.justify_start(),
        Align::Center => el.justify_center(),
        Align::Right => el.justify_end(),
    };
    if let Some(s) = style {
        if s.bold {
            el = el.font_weight(FontWeight::BOLD);
        }
        if s.italic {
            el = el.italic();
        }
        if s.underline {
            el = el.underline();
        }
    }

    if text.is_empty() {
        el.into_any_element()
    } else {
        el.child(text).into_any_element()
    }
}

/// Builds one header label cell (`selected` gives the darker tint).
fn header_element(x: f32, y: f32, w: f32, h: f32, label: String, selected: bool) -> AnyElement {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .bg(rgb(if selected {
            HEADER_SELECTED_BG
        } else {
            HEADER_BG
        }))
        .border_r_1()
        .border_b_1()
        .border_color(rgb(HEADER_HAIRLINE))
        .text_size(px(HEADER_FONT_PX))
        .text_color(rgb(HEADER_TEXT))
        .child(label)
        .into_any_element()
}

/// A filled accent rectangle (selection borders/edges are transparent-bg bordered divs; the
/// range fill + header edges are solid).
fn rect_div(x: f32, y: f32, w: f32, h: f32) -> gpui::Div {
    div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h))
}

/// The in-cell editor overlay's minimum width (px) — grows rightward over neighbours for narrow
/// columns (`functional_spec.md §1.3`, `ui_design.md §3`).
const IN_CELL_MIN_W: f32 = 80.0;
/// The in-cell editor's cap-reject danger border/tooltip colour (theme danger, matching chrome).
const IN_CELL_DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the in-cell cap-error popover (`ui_design.md §4`, matching chrome).
const IN_CELL_TOOLTIP_BG: u32 = 0x2B2B2B;
const IN_CELL_TOOLTIP_TEXT: u32 = 0xF5F5F5;

impl GridView {
    /// The in-cell editor overlay elements at `cell`: the bordered white editor box holding the
    /// reused `Input`, plus the cap-error popover below it when a cap rejection is active
    /// (`components/edit_controller.md §4.4`, `ui_design.md §3–4`). Both are `deferred()` so they
    /// paint above the selection borders; keys (Tab/Escape) are captured at the grid root.
    fn in_cell_overlay_elements(
        &self,
        cell: CellRef,
        input: &Entity<InputState>,
        frame: &Frame,
    ) -> Vec<AnyElement> {
        let (x, y, w, h) = cell_rect(cell.row, cell.col, frame);
        let w = w.max(IN_CELL_MIN_W);
        let danger = self.incell_cap.is_some();
        let border = if danger {
            rgb(IN_CELL_DANGER)
        } else {
            rgb(ACCENT)
        };
        let editor = div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            .flex()
            .items_center()
            .bg(rgb(CELL_BG))
            .border_2()
            .border_color(border)
            .px(px(1.0))
            .text_size(px(CELL_FONT_PX))
            .text_color(rgb(CELL_TEXT))
            .child(Input::new(input).w_full());

        let mut elements = vec![deferred(editor).into_any_element()];

        if let Some(message) = &self.incell_cap {
            let popover = div()
                .absolute()
                .left(px(x))
                .top(px(y + h + 2.0))
                .px_2()
                .py_1()
                .bg(rgb(IN_CELL_TOOLTIP_BG))
                .text_color(rgb(IN_CELL_TOOLTIP_TEXT))
                .text_size(px(11.0))
                .rounded_md()
                .shadow_md()
                .whitespace_nowrap()
                .child(message.clone());
            elements.push(deferred(popover).into_any_element());
        }

        elements
    }
}

impl Focusable for GridView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The grid's own laid-out bounds (captured by the `canvas` probe below), not the whole
        // window — so virtualization measures the grid area now that chrome wraps it.
        let (viewport_w, viewport_h) = self.viewport_wh(window);

        let mut root_children: Vec<AnyElement> = Vec::new();

        // A zero-cost `canvas` probe filling the grid: its prepaint captures the grid element's
        // real bounds into the entity so `viewport_wh` / `event_local` use the grid's own area +
        // origin (correct once chrome wraps the grid). It notifies on an actual change so a resize
        // repaints crisply; a stable layout captures once and never render-loops. The notify is
        // suppressed under a render-test capture (`freeze_spinner`) — the grid is full-window there
        // so bounds equal the window (no correction needed) and the capture stays a single frame.
        let probe = cx.entity().downgrade();
        root_children.push(
            canvas(
                move |bounds, _window, app| {
                    probe
                        .update(app, |this, cx| {
                            if this.bounds != Some(bounds) {
                                this.bounds = Some(bounds);
                                if !this.freeze_spinner {
                                    cx.notify();
                                }
                            }
                        })
                        .ok();
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
            .into_any_element(),
        );

        if let Some(frame) = self.resolve_frame(viewport_w, viewport_h) {
            // Announce the visible range once it settles (debounced) — the single
            // viewport-announce that covers first paint, sheet switch, and resize. Scroll /
            // keyboard paths still emit eagerly; all share `last_viewport` so there is no
            // double-emit and a values-only republish (same range) never re-announces.
            let ranges = (frame.rows.clone(), frame.cols.clone());
            if self.last_viewport.as_ref() != Some(&ranges) {
                self.last_viewport = Some(ranges.clone());
                self.events.emit(
                    &GridEvent::ViewportChanged {
                        rows: ranges.0,
                        cols: ranges.1,
                    },
                    window,
                    cx,
                );
            }

            root_children.extend(self.build_grid_layers(&frame, None));
        }

        // ---- Loading overlay (over everything) ------------------------------------------
        if let Some(name) = self.loading.clone() {
            // In the app the spinner animates; under a render-test capture it is FROZEN to a
            // static loader icon (no `with_animation`), because the animation's rotation angle
            // depends on wall-clock time between first paint and the frame grabbed at xrefresh —
            // which would make the capture non-deterministic (`set_freeze_spinner`).
            let spinner: AnyElement = if self.freeze_spinner {
                Icon::new(IconName::Loader)
                    .with_size(gpui_component::Size::Medium)
                    .into_any_element()
            } else {
                Spinner::new().into_any_element()
            };
            root_children.push(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .bg(rgb(CELL_BG).opacity(0.7))
                    .child(spinner)
                    .child(
                        div()
                            .text_color(rgb(HEADER_TEXT))
                            .text_size(px(CELL_FONT_PX))
                            .child(format!("Opening {name}…")),
                    )
                    .into_any_element(),
            );
        }

        div()
            .id("freecell-grid")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(CELL_BG))
            .on_scroll_wheel(
                cx.listener(|this, event: &gpui::ScrollWheelEvent, window, cx| {
                    this.handle_scroll(event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.handle_mouse_down(event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.handle_mouse_move(event, window, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.handle_mouse_up(event, window, cx);
                }),
            )
            // Tab / Escape in the in-cell overlay, captured **before** the input consumes them
            // (`components/edit_controller.md §Tab interception`); routed to the chrome's commit /
            // cancel via the window. Everything else (typing, arrows, Enter) reaches the input.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.incell_open.is_none() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "tab" => {
                        cx.stop_propagation();
                        let dir = if event.keystroke.modifiers.shift {
                            Direction::Left
                        } else {
                            Direction::Right
                        };
                        this.events
                            .emit(&GridEvent::InCellCommitMove(dir), window, cx);
                    }
                    "escape" => {
                        cx.stop_propagation();
                        this.events.emit(&GridEvent::InCellCancel, window, cx);
                    }
                    _ => {}
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .children(root_children)
    }
}

impl GridView {
    /// The active sheet (for tests / Phase-11 wiring).
    pub fn active_sheet(&self) -> SheetId {
        self.active_sheet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::fixtures::demo_sources;
    use crate::grid::GridEventSink;
    use gpui::{px, size, Keystroke, Modifiers, TestAppContext};
    use gpui_component::Root;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Builds a real `GridView` over the demo (Excel-max, styled) sources inside a test window.
    fn grid(cx: &mut TestAppContext) -> gpui::Entity<GridView> {
        cx.update(gpui_component::init);
        let mut out: Option<gpui::Entity<GridView>> = None;
        let slot = &mut out;
        cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let g = cx.new(|cx| GridView::new(demo_sources(), GridEventSink::noop(), cx));
            *slot = Some(g.clone());
            Root::new(g, window, cx)
        });
        out.expect("grid built")
    }

    /// The Phase-12 perf hook measures REAL work: a non-empty content build with recorded
    /// timings — the FORCE + ASSERT witness that the harness isn't measuring a no-op.
    #[gpui::test]
    fn measure_frame_builds_nonempty_layers_and_times_them(cx: &mut TestAppContext) {
        let grid = grid(cx);
        let (sample, ranges) =
            grid.update(cx, |g, _cx| g.measure_frame(0.0, 0.0, 1200.0, 800.0, None));
        assert!(
            sample.elements > 0,
            "the per-cell build must have produced cells"
        );
        assert!(
            sample.newly_visible > 0,
            "the first frame reports its whole visible region as newly-visible"
        );
        // The visible region is a real, non-empty rectangle.
        assert!(ranges.0.end > ranges.0.start && ranges.1.end > ranges.1.start);
        // frame_render must be at least the cell-load segment it contains.
        assert!(sample.frame_render_ns >= sample.cell_load_ns);
    }

    /// A deep scripted scroll actually MOVES the viewport (so the harness measures scrolling,
    /// not the same frame 348 times) — and the clamp keeps it in-bounds.
    #[gpui::test]
    fn measure_frame_scroll_moves_viewport(cx: &mut TestAppContext) {
        let grid = grid(cx);
        let (_s0, origin) =
            grid.update(cx, |g, _cx| g.measure_frame(0.0, 0.0, 1200.0, 800.0, None));
        // Scroll ~4000 px down (far past the origin viewport).
        let (_s1, deep) = grid.update(cx, |g, _cx| {
            g.measure_frame(0.0, 4000.0, 1200.0, 800.0, Some(origin.clone()))
        });
        assert_ne!(
            origin.0, deep.0,
            "a deep scroll must change the visible row range"
        );
        assert!(deep.0.start > origin.0.start, "scrolled downward");
    }

    // ---- Editing-feel input triggers (`components/edit_controller.md §Grid integration`) ----

    /// A `GridView` over the demo sources with a **recording** event sink + the window handle, so a
    /// synthesized keystroke's emitted [`GridEvent`]s can be asserted.
    #[allow(clippy::type_complexity)]
    fn grid_recording(
        cx: &mut TestAppContext,
    ) -> (
        gpui::Entity<GridView>,
        gpui::WindowHandle<Root>,
        Rc<RefCell<Vec<GridEvent>>>,
    ) {
        cx.update(gpui_component::init);
        let events: Rc<RefCell<Vec<GridEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let ev = events.clone();
        let mut out: Option<gpui::Entity<GridView>> = None;
        let slot = &mut out;
        let window = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let sink = GridEventSink::new(move |e, _w, _cx| ev.borrow_mut().push(e.clone()));
            let g = cx.new(|cx| GridView::new(demo_sources(), sink, cx));
            *slot = Some(g.clone());
            Root::new(g, window, cx)
        });
        (out.expect("grid built"), window, events)
    }

    fn key_ev(key: &str, key_char: Option<&str>, shift: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers {
                    shift,
                    ..Default::default()
                },
                key: key.into(),
                key_char: key_char.map(|s| s.to_string()),
            },
            is_held: false,
            prefer_character_input: false,
        }
    }

    #[gpui::test]
    fn printable_key_emits_type_to_edit(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.handle_key_down(&key_ev("x", Some("x"), false), window, cx);
                });
            })
            .unwrap();
        assert!(events
            .borrow()
            .iter()
            .any(|e| matches!(e, GridEvent::TypeToEdit(t) if t == "x")));
    }

    #[gpui::test]
    fn f2_emits_open_in_cell(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.handle_key_down(&key_ev("f2", None, false), window, cx);
                });
            })
            .unwrap();
        assert!(events
            .borrow()
            .iter()
            .any(|e| matches!(e, GridEvent::OpenInCellEditor(c) if *c == CellRef::new(0, 0))));
    }

    #[gpui::test]
    fn keys_ignored_while_in_cell_open(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(None, Some(CellRef::new(0, 0)), None, cx);
                    events.borrow_mut().clear();
                    // A printable key and an arrow both no-op while the overlay owns the keyboard.
                    grid.handle_key_down(&key_ev("x", Some("x"), false), window, cx);
                    grid.handle_key_down(&key_ev("down", None, false), window, cx);
                });
            })
            .unwrap();
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::TypeToEdit(_) | GridEvent::SelectionChanged(_))),
            "the in-cell overlay must own the keyboard: {:?}",
            events.borrow()
        );
    }
}
