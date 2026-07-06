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
use freecell_core::{
    apply_motion, blocks_col_op, blocks_row_op, effective_edge, is_full_column_selection,
    is_full_row_selection, Align, Axis, BorderSpec, CellRange, CellRef, Edge, RenderStyle,
    SelectionModel, SheetDims,
};

use super::input::{command_for_key, GridKeyCommand};
use super::layout::{
    self, ContentArea, GridHit, COL_HEADER_H, RENDER_OVERSCAN, SCROLLBAR_INSET, SCROLLBAR_THICKNESS,
};
use super::{
    GridEvent, GridEventSink, RowOrCol, ACCENT, AUTOSCROLL_INTERVAL_MS, CELL_BG, CELL_FONT_PX,
    CELL_H_PAD, CELL_TEXT, EDGE_AUTOSCROLL_HOTZONE_PX, EDGE_AUTOSCROLL_STEP_PX, GRIDLINE,
    HEADER_BG, HEADER_FONT_PX, HEADER_HAIRLINE, HEADER_SELECTED_BG, HEADER_TEXT,
    SCROLLBAR_FADE_SECS, SCROLLBAR_RGBA, SELECTION_FILL_ALPHA,
};

/// Half-width (px) of a divider resize hotspot (`ui_design.md §3`: a 6 px zone centered on the
/// divider). Also the ±3 px within which the resize cursor shows (`functional_spec.md §5.1`).
const RESIZE_HOTSPOT_HALF: f32 = 3.0;
/// Minimum column width / row height a resize drag clamps to (`functional_spec.md §5.1`).
const MIN_COL_WIDTH_PX: f32 = 8.0;
const MIN_ROW_HEIGHT_PX: f32 = 12.0;

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

/// What a mouse drag extends: an ordinary cell range, or a header selection (full column / full
/// row), which extends only the active track (`components/grid_structure.md §5.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Cell,
    ColHeader,
    RowHeader,
}

/// An in-flight mouse drag-selection (`components/grid.md §State`: "anchor cell + last hovered
/// cell"). The anchor is the fixed corner the range extends from; the hovered cell is recomputed
/// from the pointer each move, so only the anchor is retained.
#[derive(Debug, Clone, Copy)]
struct DragState {
    anchor: CellRef,
    mode: DragMode,
}

/// The open insert/delete header context menu (`functional_spec.md §5.3`). Grid-owned (it holds
/// the cache's merge list for the guard + renders overlays already). `x`/`y` are grid-local; the
/// `*_blocked` flags disable the corresponding item when the op would displace a file-loaded merge.
#[derive(Debug, Clone, Copy)]
struct HeaderMenu {
    axis: RowOrCol,
    /// The inclusive selected header run the ops apply to (sets the count `N`).
    run: (u32, u32),
    x: f32,
    y: f32,
    insert_before_blocked: bool,
    insert_after_blocked: bool,
    delete_blocked: bool,
}

/// A live row/column resize (`components/grid_structure.md §5.1`). `start_px` is the dragged
/// track's size at mouse-down and `current_px` its clamped live size; `run` is the inclusive
/// 0-based track run the release applies to (the dragged index alone, or the selected header run);
/// `origin_coord` is the grid-local pointer coordinate (x for a column, y for a row) at mouse-down,
/// so the delta is measured from the grab point.
#[derive(Debug, Clone, Copy)]
struct ResizeDrag {
    axis: RowOrCol,
    index: u32,
    start_px: f32,
    current_px: f32,
    run: (u32, u32),
    origin_coord: f32,
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
    /// The in-flight row/column resize, if any (updates on every mouse-move; `None` = not
    /// resizing). Drives the live preview geometry + guide line + tooltip.
    resize_drag: Option<ResizeDrag>,
    /// A committed resize kept as a **frozen** preview after release, so the grid keeps showing the
    /// new geometry until the worker's cache rebuild republishes it (no flicker back to the old
    /// size). Cleared on the next `StyleCacheUpdated` (window), a new mouse-down, or Escape.
    resize_preview: Option<ResizeDrag>,
    /// The open header insert/delete context menu, if any (`functional_spec.md §5.3`).
    header_menu: Option<HeaderMenu>,
    /// Whether the edge auto-scroll timer loop is currently running.
    autoscrolling: bool,
    /// Monotonic epoch; a running auto-scroll loop stops as soon as this changes (drag end /
    /// pointer back inside), the same guard pattern as the scrollbar fade.
    autoscroll_epoch: u64,
    /// Reused per-frame index: visible `(row, col)` → index into the publication's cells.
    cell_index: HashMap<(u32, u32), usize>,
    /// Reused per-frame snapshot: visible `(row, col)` → resolved style (default = absent).
    visible_styles: HashMap<(u32, u32), RenderStyle>,
    /// Reused per-frame snapshot of the active cache's font-family side table, so a cell's
    /// `RenderStyle::font_family` index resolves to a name after the cache lock is released
    /// (`components/style_render.md`). Index `0` = `""` = the workbook default (grid default family).
    visible_font_families: Vec<SharedString>,
    /// Reused per-frame snapshot of the active cache's border side table, so a cell's
    /// `RenderStyle::border` index resolves to a [`BorderSpec`] after the cache lock is released
    /// (`components/style_render.md §Border painting`). Index `0` = [`BorderSpec::NONE`].
    visible_border_specs: Vec<BorderSpec>,
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

/// A live-resize preview applied to **one** axis as a cheap **O(1) per-track delta**, NOT a
/// rebuilt axis (`components/grid_structure.md §5.1`): the committed prefix sums are reused, the
/// dragged track reports `new_px`, and every track after it shifts by `delta = new_px - base_px`
/// (`base_px` is the track's committed size, captured at grab). All reads are O(log + block) on the
/// shared axis; nothing loops over the sheet, so a drag frame stays O(visible tracks) even at
/// Excel-max — the §4 "zero work proportional to sheet size" gate.
#[derive(Debug, Clone, Copy)]
struct AxisPreview {
    index: u32,
    new_px: f32,
    base_px: f32,
}

impl AxisPreview {
    /// The signed size change of the dragged track (negative when shrinking).
    fn delta(&self) -> f64 {
        (self.new_px - self.base_px) as f64
    }

    /// The track's previewed size: `new_px` for the dragged index, else the committed size.
    fn size(&self, base: &Axis, i: u32) -> f32 {
        if i == self.index {
            self.new_px
        } else {
            base.size_of(i)
        }
    }

    /// The track's previewed start offset: committed offset, shifted by `delta` for tracks after
    /// the dragged index (which move as it resizes). O(1) over the committed offset — no rebuild.
    fn offset(&self, base: &Axis, i: u32) -> f64 {
        let raw = base.offset_of(i);
        if i > self.index {
            raw + self.delta()
        } else {
            raw
        }
    }

    /// The previewed total extent = committed total + `delta` (O(1)).
    fn total(&self, base: &Axis) -> f64 {
        base.total() + self.delta()
    }

    /// Extra viewport extent (px) at the **far** (bottom/right) edge: a **shrink** pulls tracks after
    /// the dragged index toward the origin, so `|delta|` more px of the axis enter the viewport at
    /// the far edge (a grow reveals nothing there). Widens the queried extent so those tracks draw.
    fn shrink_extent(&self) -> f64 {
        (-self.delta()).max(0.0)
    }

    /// Extra viewport extent (px) at the **near** (top/left) edge. A **grow** shifts tracks after the
    /// dragged index *away* from the origin by `delta`, so when the dragged index is scrolled off the
    /// near edge (e.g. a frozen preview scrolled past) the previewed content at a given scroll maps
    /// to *earlier* raw indices — the query must start `delta` px earlier so those grown tracks are
    /// fetched (else a blank strip up to `delta` px). A shrink needs no near widening.
    fn grow_extent(&self) -> f64 {
        self.delta().max(0.0)
    }
}

/// The per-frame geometry resolved under the (briefly held) caches read lock. `row_axis`/`col_axis`
/// are the **committed** prefix-sum axes (never rebuilt per frame); an active resize is applied as
/// a cheap [`AxisPreview`] delta through the `*_offset` / `*_size` accessors.
struct Frame {
    row_axis: Arc<Axis>,
    col_axis: Arc<Axis>,
    /// A live/frozen row resize preview (at most one of `row_preview`/`col_preview` is `Some`).
    row_preview: Option<AxisPreview>,
    col_preview: Option<AxisPreview>,
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

impl Frame {
    /// Previewed column start offset (content-local, pre-scroll).
    fn col_offset(&self, c: u32) -> f64 {
        match self.col_preview {
            Some(p) => p.offset(&self.col_axis, c),
            None => self.col_axis.offset_of(c),
        }
    }
    /// Previewed column width.
    fn col_size(&self, c: u32) -> f32 {
        match self.col_preview {
            Some(p) => p.size(&self.col_axis, c),
            None => self.col_axis.size_of(c),
        }
    }
    /// Previewed row start offset.
    fn row_offset(&self, r: u32) -> f64 {
        match self.row_preview {
            Some(p) => p.offset(&self.row_axis, r),
            None => self.row_axis.offset_of(r),
        }
    }
    /// Previewed row height.
    fn row_size(&self, r: u32) -> f32 {
        match self.row_preview {
            Some(p) => p.size(&self.row_axis, r),
            None => self.row_axis.size_of(r),
        }
    }
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
            resize_drag: None,
            resize_preview: None,
            header_menu: None,
            autoscrolling: false,
            autoscroll_epoch: 0,
            cell_index: HashMap::new(),
            visible_styles: HashMap::new(),
            visible_font_families: Vec::new(),
            visible_border_specs: Vec::new(),
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

    /// Clears the frozen resize preview once the worker's cache rebuild has landed (the window
    /// calls this on `StyleCacheUpdated`) — the committed geometry now comes from the resident
    /// cache, so the preview is no longer needed (`components/grid_structure.md §5.1`).
    pub fn clear_resize_preview(&mut self, cx: &mut Context<Self>) {
        if self.resize_preview.take().is_some() {
            cx.notify();
        }
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
        // Structural interactions are anchored to the previous sheet's geometry — drop them.
        self.resize_drag = None;
        self.resize_preview = None;
        self.header_menu = None;
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
        // Keep the COMMITTED axes (never rebuilt per frame). A live/just-committed resize is applied
        // as a cheap O(1)-per-track `AxisPreview` delta, so a drag stays O(visible) even at
        // Excel-max (`components/grid_structure.md §5.1`; the §4 "no work proportional to sheet
        // size" gate). `visible_range` runs on the raw prefix sums; a shrinking track pulls later
        // tracks into view, so its `shrink_extent` widens the queried extent to draw them.
        let (row_axis, col_axis) = cache.axes();
        let (row_preview, col_preview) = match self.resize_drag.or(self.resize_preview) {
            Some(rd) => {
                let p = AxisPreview {
                    index: rd.index,
                    new_px: rd.current_px,
                    base_px: rd.start_px,
                };
                match rd.axis {
                    RowOrCol::Row => (Some(p), None),
                    RowOrCol::Col => (None, Some(p)),
                }
            }
            None => (None, None),
        };
        let total_w = col_preview.map_or_else(|| col_axis.total(), |p| p.total(&col_axis));
        let total_h = row_preview.map_or_else(|| row_axis.total(), |p| p.total(&row_axis));
        // Over-render both ends of the queried window under a preview: `grow_extent` widens the
        // NEAR (top/left) end so a grow whose dragged index is scrolled off the near edge still
        // fetches the shifted-in tracks (else a blank strip); `shrink_extent` widens the FAR
        // (bottom/right) end so a shrink's revealed tracks draw. Only one is ever non-zero (delta is
        // one sign); both are bounded by |delta|, so this stays O(visible).
        let (row_near, row_far) =
            row_preview.map_or((0.0, 0.0), |p| (p.grow_extent(), p.shrink_extent()));
        let (col_near, col_far) =
            col_preview.map_or((0.0, 0.0), |p| (p.grow_extent(), p.shrink_extent()));
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        // Compute visible ranges + gutter width for a given scroll (the gutter width depends
        // on the deepest visible row, which depends on scroll — hence a small closure). The preview
        // over-render (near + far) is folded into the queried start + extent so every previewed-
        // visible track is fetched (`index_at` clamps a negative start to 0).
        let ranges = |sx: f64, sy: f64| -> (Range<u32>, f32, f64, Range<u32>) {
            let rows = row_axis.visible_range(
                sy - row_near,
                content_h + row_near + row_far,
                RENDER_OVERSCAN,
            );
            let row_header_w = layout::row_header_width(rows.end.saturating_sub(1));
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let cols = col_axis.visible_range(
                sx - col_near,
                content_w + col_near + col_far,
                RENDER_OVERSCAN,
            );
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
        // Snapshot the font-family side table too (cheap — a handful of `Arc<str>` → `SharedString`),
        // so a cell's `font_family` index resolves to a name after the lock is dropped.
        self.visible_font_families = cache
            .font_families()
            .iter()
            .map(|name| SharedString::from(name.to_string()))
            .collect();
        // Snapshot the border side table (cheap — `BorderSpec` is `Copy`), so a cell's `border`
        // index (and its neighbours') resolves to a spec after the lock is dropped.
        self.visible_border_specs = cache.border_specs().to_vec();
        drop(caches);

        Some(Frame {
            row_axis,
            col_axis,
            row_preview,
            col_preview,
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

    /// Mouse down: claim keyboard focus, hit-test, and act by region — a data cell sets/extends the
    /// selection and begins a cell drag; a column/row header selects the full column/row and begins
    /// a header drag; the corner selects the whole sheet (`components/grid_structure.md §5.2`).
    /// Divider resize hotspots handle their own mouse-down (and stop propagation) before this.
    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A hotspot already started a resize (and stopped propagation); defensively, never treat
        // this as a selection click.
        if self.resize_drag.is_some() {
            return;
        }
        // Any new mouse-down ends a frozen resize preview (e.g. after a degraded-mode no-op).
        self.resize_preview = None;

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
        match hit {
            GridHit::Cell { row, col } => self.mouse_down_cell(row, col, event, window, cx),
            GridHit::ColHeader { col } => {
                self.select_column(col, event.modifiers.shift, window, cx);
                self.drag = Some(DragState {
                    anchor: self.selection().anchor,
                    mode: DragMode::ColHeader,
                });
                cx.notify();
            }
            GridHit::RowHeader { row } => {
                self.select_row(row, event.modifiers.shift, window, cx);
                self.drag = Some(DragState {
                    anchor: self.selection().anchor,
                    mode: DragMode::RowHeader,
                });
                cx.notify();
            }
            GridHit::Corner => {
                self.select_all(window, cx);
                cx.notify();
            }
        }
    }

    /// The data-cell branch of [`handle_mouse_down`]: set/extend the selection (shift extends the
    /// range from the existing anchor), open the in-cell editor on a double-click, and begin a
    /// cell drag.
    fn mouse_down_cell(
        &mut self,
        row: u32,
        col: u32,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            // Opening the in-cell editor focuses its input (synchronously, inside this emit). But
            // gpui registers an automatic focus-transfer for every `track_focus` element on
            // mouse-down, and the grid root's transfer runs *later* in this same bubble dispatch —
            // it would steal focus straight back to the grid, leaving the just-opened editor with
            // no caret ("can't type", BUG D). `prevent_default` makes that built-in transfer skip
            // (`Interactivity::paint_mouse_listeners` gates it on `!window.default_prevented()`),
            // so the editor keeps focus. The explicit grid focus above already ran and is harmless.
            self.events
                .emit(&GridEvent::OpenInCellEditor(cell), window, cx);
            window.prevent_default();
        }
        // Begin a drag from the (kept or new) anchor; subsequent moves extend to the hovered cell.
        self.drag = Some(DragState {
            anchor: selection.anchor,
            mode: DragMode::Cell,
        });
        cx.notify();
    }

    /// The full-column range for column `col` (anchor row 0 → active last row). Shift keeps the
    /// column anchor of an existing full-column selection (extend the run); a plain click anchors
    /// on `col`.
    fn select_column(
        &mut self,
        col: u32,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dims) = self.sheet_dims() else {
            return;
        };
        let anchor_col = if shift && is_full_column_selection(self.selection()) {
            self.selection().anchor.col
        } else {
            col
        };
        let sel = SelectionModel {
            anchor: CellRef::new(0, anchor_col),
            active: CellRef::new(dims.rows.saturating_sub(1), col),
        };
        self.set_selection_and_emit(sel, window, cx);
    }

    /// The full-row range for row `row` (row analog of [`select_column`](Self::select_column)).
    fn select_row(&mut self, row: u32, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dims) = self.sheet_dims() else {
            return;
        };
        let anchor_row = if shift && is_full_row_selection(self.selection()) {
            self.selection().anchor.row
        } else {
            row
        };
        let sel = SelectionModel {
            anchor: CellRef::new(anchor_row, 0),
            active: CellRef::new(row, dims.cols.saturating_sub(1)),
        };
        self.set_selection_and_emit(sel, window, cx);
    }

    /// Selects the whole sheet (`A1:XFD1048576`) — the corner button and Cmd/Ctrl+A
    /// (`functional_spec.md §5.2`).
    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dims) = self.sheet_dims() else {
            return;
        };
        let sel = SelectionModel {
            anchor: CellRef::new(0, 0),
            active: CellRef::new(dims.rows.saturating_sub(1), dims.cols.saturating_sub(1)),
        };
        self.set_selection_and_emit(sel, window, cx);
    }

    /// Mouse move: update a live resize, or extend the drag selection (cell or header) and — for a
    /// cell drag past a viewport edge — kick off the edge auto-scroll loop.
    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (local_x, local_y) = self.event_local(event.position);
        if self.resize_drag.is_some() {
            self.update_resize(local_x, local_y, cx);
            return;
        }
        let Some(drag) = self.drag else {
            return; // not dragging — nothing to do
        };
        match drag.mode {
            DragMode::Cell => {
                self.extend_drag_to_point(drag.anchor, local_x, local_y, window, cx);
                self.maybe_start_autoscroll(window, cx);
            }
            DragMode::ColHeader => {
                self.extend_header_drag(drag.anchor, RowOrCol::Col, local_x, local_y, window, cx)
            }
            DragMode::RowHeader => {
                self.extend_header_drag(drag.anchor, RowOrCol::Row, local_x, local_y, window, cx)
            }
        }
    }

    /// Mouse up: commit a live resize, or end the selection drag (stopping any auto-scroll loop via
    /// the epoch) and let the scrollbars fade if a drag-scroll had shown them.
    fn handle_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(rd) = self.resize_drag.take() {
            self.commit_resize(rd, window, cx);
            return;
        }
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

    /// Begin a row/column resize from a divider hotspot mouse-down (`components/grid_structure.md
    /// §5.1`). Records the dragged track's start size + the grid-local grab coordinate; the run is
    /// the whole selected header run when the dragged index sits inside a header selection of that
    /// axis, else the dragged index alone (so a drag inside a 3-column header selection resizes all
    /// three).
    fn begin_resize(
        &mut self,
        axis: RowOrCol,
        index: u32,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Focus the grid so Escape (cancel) routes here.
        let handle = self.focus_handle.clone();
        window.focus(&handle, cx);
        let (local_x, local_y) = self.event_local(event.position);

        let active = self.active_sheet;
        let start_px = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            match axis {
                RowOrCol::Row => cache.row_height(index),
                RowOrCol::Col => cache.col_width(index),
            }
        };
        let run = self.resize_run_for(axis, index);
        let origin_coord = match axis {
            RowOrCol::Col => local_x,
            RowOrCol::Row => local_y,
        };
        self.resize_preview = None;
        self.drag = None; // a resize is never also a selection drag
        self.resize_drag = Some(ResizeDrag {
            axis,
            index,
            start_px,
            current_px: start_px,
            run,
            origin_coord,
        });
        cx.notify();
    }

    /// The inclusive track run a resize applies to: the whole contiguous selected header run when
    /// the dragged `index` is inside a header selection of `axis`, else `(index, index)`
    /// (`functional_spec.md §5.1`).
    ///
    /// Select-all resize is intentionally bounded-but-wide: it is classified as a **full-column**
    /// selection (`is_full_column_selection` is true for the whole sheet), so a **column** divider
    /// drag under select-all resizes all 16,384 columns in one `SetColumnWidths` op — bounded,
    /// one-time at commit, and Excel-parity. The dangerous **row** analog (a 1,048,576-row
    /// `SetRowHeights`) is deliberately avoided: select-all is NOT a full-row selection, so a **row**
    /// divider drag under it stays a single track `(index, index)` (the `RowOrCol::Row` arm's
    /// `is_full_row_selection` guard is false for the whole sheet).
    fn resize_run_for(&self, axis: RowOrCol, index: u32) -> (u32, u32) {
        let range = self.selection().range();
        match axis {
            RowOrCol::Col
                if is_full_column_selection(self.selection())
                    && index >= range.start.col
                    && index <= range.end.col =>
            {
                (range.start.col, range.end.col)
            }
            RowOrCol::Row
                if is_full_row_selection(self.selection())
                    && index >= range.start.row
                    && index <= range.end.row =>
            {
                (range.start.row, range.end.row)
            }
            _ => (index, index),
        }
    }

    /// Update a live resize from the current pointer: `current_px = clamp(start_px + delta, MIN)`,
    /// where `delta` is the pointer's movement along the drag axis from the grab point.
    fn update_resize(&mut self, local_x: f32, local_y: f32, cx: &mut Context<Self>) {
        let Some(rd) = self.resize_drag.as_mut() else {
            return;
        };
        let coord = match rd.axis {
            RowOrCol::Col => local_x,
            RowOrCol::Row => local_y,
        };
        let min = match rd.axis {
            RowOrCol::Col => MIN_COL_WIDTH_PX,
            RowOrCol::Row => MIN_ROW_HEIGHT_PX,
        };
        rd.current_px = (rd.start_px + (coord - rd.origin_coord)).max(min);
        cx.notify();
    }

    /// Commit a live resize on release: freeze the preview (so the grid keeps the new geometry
    /// until the worker's rebuild republishes it) and emit `ResizeCommitted` over the run
    /// (`components/grid_structure.md §5.1`).
    fn commit_resize(&mut self, rd: ResizeDrag, window: &mut Window, cx: &mut Context<Self>) {
        self.resize_preview = Some(rd);
        self.events.emit(
            &GridEvent::ResizeCommitted {
                axis: rd.axis,
                start: rd.run.0,
                end: rd.run.1,
                px: rd.current_px,
            },
            window,
            cx,
        );
        cx.notify();
    }

    /// Extend a header drag: map the pointer to a track on `axis` and move the selection's active
    /// track there, keeping the full extent (`components/grid_structure.md §5.2`).
    fn extend_header_drag(
        &mut self,
        anchor: CellRef,
        axis: RowOrCol,
        local_x: f32,
        local_y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        let (Some(dims), point_cell) = ({
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            let cell = layout::cell_at_point(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
                content_w,
                content_h,
            );
            (
                Some(SheetDims::new(row_axis.count(), col_axis.count())),
                cell,
            )
        }) else {
            return;
        };
        // Full extent on the fixed axis; the active track follows the pointer.
        let active_cell = match axis {
            RowOrCol::Col => CellRef::new(dims.rows.saturating_sub(1), point_cell.col),
            RowOrCol::Row => CellRef::new(point_cell.row, dims.cols.saturating_sub(1)),
        };
        let selection = SelectionModel {
            anchor,
            active: active_cell,
        };
        if *self.selection() != selection {
            self.set_selection_and_emit(selection, window, cx);
            cx.notify();
        }
    }

    /// Right mouse-down on a header opens the insert/delete context menu (`functional_spec.md
    /// §5.3`). A right-click outside the current header selection first selects that single header
    /// (Excel behaviour); the menu's item-disable flags come from the sheet's file-loaded merges.
    fn handle_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (local_x, local_y) = self.event_local(event.position);
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (_, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        // Hit-test + read the merge list under one lock.
        let (hit, merges) = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            let hit = layout::hit_test(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
            );
            (hit, cache.merges().to_vec())
        };
        let (axis, index) = match hit {
            GridHit::ColHeader { col } => (RowOrCol::Col, col),
            GridHit::RowHeader { row } => (RowOrCol::Row, row),
            _ => {
                // A right-click off the headers dismisses any open menu.
                if self.header_menu.take().is_some() {
                    cx.notify();
                }
                return;
            }
        };
        // If the clicked header isn't inside the current header selection of that axis, select it
        // (so the op targets what the user clicked); the run then reflects the selection.
        let inside = match axis {
            RowOrCol::Col => {
                is_full_column_selection(self.selection())
                    && index >= self.selection().range().start.col
                    && index <= self.selection().range().end.col
            }
            RowOrCol::Row => {
                is_full_row_selection(self.selection())
                    && index >= self.selection().range().start.row
                    && index <= self.selection().range().end.row
            }
        };
        if !inside {
            match axis {
                RowOrCol::Col => self.select_column(index, false, window, cx),
                RowOrCol::Row => self.select_row(index, false, window, cx),
            }
        }
        let run = self.resize_run_for(axis, index);
        let (before, after, delete) = merge_block_flags(axis, run, &merges);
        self.header_menu = Some(HeaderMenu {
            axis,
            run,
            x: local_x,
            y: local_y,
            insert_before_blocked: before,
            insert_after_blocked: after,
            delete_blocked: delete,
        });
        cx.notify();
    }

    /// Closes the header context menu (click-away / Escape / after an item runs).
    fn close_header_menu(&mut self, cx: &mut Context<Self>) {
        if self.header_menu.take().is_some() {
            cx.notify();
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
        // Escape cancels a live resize (no command sent) — the preview clears, geometry reverts
        // (`functional_spec.md §5.1`) — clears a lingering frozen preview (e.g. a degraded-mode
        // post-commit preview), or closes the header context menu.
        if key == "escape"
            && (self.resize_drag.is_some()
                || self.resize_preview.is_some()
                || self.header_menu.is_some())
        {
            self.resize_drag = None;
            self.resize_preview = None;
            self.header_menu = None;
            cx.notify();
            return;
        }
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
            // Copy/cut/paste route to the window's `ClipboardCoordinator`. Reaching here means the
            // grid is focused and no in-cell edit is open (the early return above), so the data-row
            // / in-cell inputs keep their native text clipboard.
            GridKeyCommand::Copy => self
                .events
                .emit(&GridEvent::Copy { cut: false }, window, cx),
            GridKeyCommand::Cut => self.events.emit(&GridEvent::Copy { cut: true }, window, cx),
            GridKeyCommand::Paste => self.events.emit(&GridEvent::Paste, window, cx),
            GridKeyCommand::SelectAll => self.select_all(window, cx),
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
                // Resolve the cell's font family name from the snapshot (index 0 / mirror = default).
                let font_family = attr_style.and_then(|s| {
                    let idx = s.font_family as usize;
                    self.visible_font_families
                        .get(idx)
                        .filter(|name| !name.is_empty())
                        .cloned()
                });
                content_children.push(cell_element(
                    x,
                    y,
                    w,
                    h,
                    fill,
                    text,
                    text_color,
                    kind,
                    attr_style,
                    font_family,
                ));
            }
        }

        // ---- Border edges: painted after every cell fill so they cover the gridline + any
        // neighbouring cell's fill (Excel look, `components/style_render.md §Border painting`).
        // Each shared edge is drawn exactly ONCE: a bordered cell always draws its right + bottom
        // effective edges; it draws its left/top only when no neighbour to the left/above will draw
        // that shared edge (the first visible track, or an unbordered neighbour that is skipped).
        // Effective edge = the heavier of the cell's own edge and the neighbour's opposing one.
        for r in frame.rows.clone() {
            for c in frame.cols.clone() {
                let spec = self.border_spec_at(r, c);
                if spec.is_none() {
                    continue;
                }
                let (x, y, w, h) = cell_rect(r, c, frame);
                // Right edge (shared with the cell at c+1) — always drawn by this (left) cell.
                if let Some(edge) = effective_edge(spec.right, self.border_spec_at(r, c + 1).left) {
                    content_children.push(vertical_edge_quad(x + w, y, h, edge));
                }
                // Bottom edge (shared with r+1) — always drawn by this (upper) cell.
                if let Some(edge) = effective_edge(spec.bottom, self.border_spec_at(r + 1, c).top) {
                    content_children.push(horizontal_edge_quad(x, y + h, w, edge));
                }
                // Left edge: only when the left neighbour won't draw it as its right edge.
                if self.no_left_owner(r, c, frame) {
                    let left_nbr = if c == 0 {
                        BorderSpec::NONE
                    } else {
                        self.border_spec_at(r, c - 1)
                    };
                    if let Some(edge) = effective_edge(spec.left, left_nbr.right) {
                        content_children.push(vertical_edge_quad(x, y, h, edge));
                    }
                }
                // Top edge: only when the top neighbour won't draw it as its bottom edge.
                if self.no_top_owner(r, c, frame) {
                    let top_nbr = if r == 0 {
                        BorderSpec::NONE
                    } else {
                        self.border_spec_at(r - 1, c)
                    };
                    if let Some(edge) = effective_edge(spec.top, top_nbr.bottom) {
                        content_children.push(horizontal_edge_quad(x, y, w, edge));
                    }
                }
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
            let x = (frame.col_offset(c) - frame.scroll_x) as f32;
            let w = frame.col_size(c);
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
            let y = (frame.row_offset(r) - frame.scroll_y) as f32;
            let h = frame.row_size(r);
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

        // ---- Resize guide line + size tooltip (only during a live drag, `ui_design.md §3`) ----
        if let Some(rd) = self.resize_drag {
            let grid_h = COL_HEADER_H + frame.content_h as f32;
            let grid_w = frame.row_header_w + frame.content_w as f32;
            match rd.axis {
                RowOrCol::Col => {
                    // The drag edge = the dragged column's previewed right boundary.
                    let edge = frame.row_header_w
                        + (frame.col_offset(rd.index) + frame.col_size(rd.index) as f64
                            - frame.scroll_x) as f32;
                    root_children.push(
                        rect_div(edge - 0.5, 0.0, 1.0, grid_h)
                            .bg(rgb(ACCENT))
                            .into_any_element(),
                    );
                    root_children.push(resize_tooltip(
                        edge + 4.0,
                        2.0,
                        format!("Width: {}", rd.current_px.round() as i32),
                    ));
                }
                RowOrCol::Row => {
                    let edge = COL_HEADER_H
                        + (frame.row_offset(rd.index) + frame.row_size(rd.index) as f64
                            - frame.scroll_y) as f32;
                    root_children.push(
                        rect_div(0.0, edge - 0.5, grid_w, 1.0)
                            .bg(rgb(ACCENT))
                            .into_any_element(),
                    );
                    root_children.push(resize_tooltip(
                        2.0,
                        edge + 4.0,
                        format!("Height: {}", rd.current_px.round() as i32),
                    ));
                }
            }
        }

        root_children
    }

    /// The divider resize hotspots — a 6 px `col-resize` / `row-resize` zone centered on each
    /// visible divider, painted over the header strips (hit priority) with a mouse-down that begins
    /// a resize and stops propagation so header-selection never fires (`components/grid_structure.md
    /// §5.1`). Built in `render` (not `build_grid_layers`) because the listeners need `cx`.
    fn resize_hotspots(&self, frame: &Frame, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let content_right = frame.row_header_w + frame.content_w as f32;
        let content_bottom = COL_HEADER_H + frame.content_h as f32;
        // Column dividers (drag a column's RIGHT edge → resize that column).
        for c in frame.cols.clone() {
            let edge = frame.row_header_w
                + (frame.col_offset(c) + frame.col_size(c) as f64 - frame.scroll_x) as f32;
            if edge <= frame.row_header_w || edge > content_right {
                continue; // divider off the visible header strip
            }
            out.push(
                rect_div(
                    edge - RESIZE_HOTSPOT_HALF,
                    0.0,
                    RESIZE_HOTSPOT_HALF * 2.0,
                    COL_HEADER_H,
                )
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.begin_resize(RowOrCol::Col, c, event, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
            );
        }
        // Row dividers (drag a row's BOTTOM edge → resize that row).
        for r in frame.rows.clone() {
            let edge = COL_HEADER_H
                + (frame.row_offset(r) + frame.row_size(r) as f64 - frame.scroll_y) as f32;
            if edge <= COL_HEADER_H || edge > content_bottom {
                continue;
            }
            out.push(
                rect_div(
                    0.0,
                    edge - RESIZE_HOTSPOT_HALF,
                    frame.row_header_w,
                    RESIZE_HOTSPOT_HALF * 2.0,
                )
                .cursor_row_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.begin_resize(RowOrCol::Row, r, event, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
            );
        }
        out
    }

    /// The header insert/delete context menu overlay: a click-away backdrop + a small card of items
    /// (`functional_spec.md §5.3`). Items whose op would displace a merge are disabled + a footnote
    /// explains why. Built in `render` (needs `cx` for the item listeners).
    fn header_menu_elements(&self, menu: HeaderMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let count = menu.run.1 - menu.run.0 + 1;
        let (unit, before_word, after_word) = match menu.axis {
            RowOrCol::Row => ("row", "above", "below"),
            RowOrCol::Col => ("column", "left", "right"),
        };
        let plural = if count == 1 { "" } else { "s" };
        let n = |verb: &str, side: &str| format!("{verb} {count} {unit}{plural} {side}");

        // The three items: (label, disabled, event).
        let (start, end) = (menu.run.0, menu.run.1);
        let after_at = end.saturating_add(1);
        let items: [(String, bool, GridEvent); 3] = match menu.axis {
            RowOrCol::Row => [
                (
                    n("Insert", before_word),
                    menu.insert_before_blocked,
                    GridEvent::InsertRows { at: start, count },
                ),
                (
                    n("Insert", after_word),
                    menu.insert_after_blocked,
                    GridEvent::InsertRows {
                        at: after_at,
                        count,
                    },
                ),
                (
                    format!("Delete {count} {unit}{plural}"),
                    menu.delete_blocked,
                    GridEvent::DeleteRows { at: start, count },
                ),
            ],
            RowOrCol::Col => [
                (
                    n("Insert", before_word),
                    menu.insert_before_blocked,
                    GridEvent::InsertColumns { at: start, count },
                ),
                (
                    n("Insert", after_word),
                    menu.insert_after_blocked,
                    GridEvent::InsertColumns {
                        at: after_at,
                        count,
                    },
                ),
                (
                    format!("Delete {count} {unit}{plural}"),
                    menu.delete_blocked,
                    GridEvent::DeleteColumns { at: start, count },
                ),
            ],
        };
        let any_blocked = items.iter().any(|(_, blocked, _)| *blocked);

        let mut card = div()
            .absolute()
            .left(px(menu.x))
            .top(px(menu.y))
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(rgb(CELL_BG))
            .border_1()
            .border_color(rgb(HEADER_HAIRLINE))
            .rounded_md()
            .shadow_md()
            .text_size(px(CELL_FONT_PX))
            .min_w(px(180.0));
        for (label, blocked, event) in items {
            let mut item = div()
                .px(px(10.0))
                .py(px(4.0))
                .rounded_sm()
                .whitespace_nowrap()
                .child(label);
            if blocked {
                item = item.text_color(rgb(HEADER_TEXT)).opacity(0.4);
            } else {
                item = item
                    .cursor_pointer()
                    .text_color(rgb(CELL_TEXT))
                    .hover(|s| s.bg(rgb(HEADER_SELECTED_BG)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            this.events.emit(&event, window, cx);
                            this.close_header_menu(cx);
                            cx.stop_propagation();
                        }),
                    );
            }
            card = card.child(item);
        }
        if any_blocked {
            card = card.child(
                div()
                    .px(px(10.0))
                    .py(px(3.0))
                    .text_size(px(11.0))
                    .text_color(rgb(HEADER_TEXT))
                    .max_w(px(220.0))
                    .child("Sheet has merged cells — not yet supported here."),
            );
        }

        // A transparent full-grid backdrop closes the menu on any click outside it (and swallows
        // the click so it doesn't also start a selection — the menu is modal while open).
        let backdrop = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_header_menu(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_header_menu(cx);
                    cx.stop_propagation();
                }),
            );
        vec![
            deferred(backdrop).into_any_element(),
            deferred(card).into_any_element(),
        ]
    }

    /// The resolved [`BorderSpec`] for a visible cell, from the per-frame snapshot: its
    /// `RenderStyle::border` index into `visible_border_specs`. A cell absent from the style
    /// snapshot (the common, borderless case) → [`BorderSpec::NONE`], so neighbour lookups
    /// short-circuit. Off-frame neighbours are also absent → treated as unbordered (the accepted
    /// viewport-boundary approximation, `architecture.md §3.4`).
    fn border_spec_at(&self, row: u32, col: u32) -> BorderSpec {
        self.visible_styles
            .get(&(row, col))
            .and_then(|rs| self.visible_border_specs.get(rs.border as usize).copied())
            .unwrap_or(BorderSpec::NONE)
    }

    /// Whether no cell to the *left* of `(row, col)` will draw the shared left edge as its own
    /// right edge — i.e. this cell owns that edge. True at the first visible column (no in-frame
    /// left neighbour) or when the left neighbour is unbordered (skipped by the paint loop).
    ///
    /// The "is the neighbour bordered?" test goes through [`border_spec_at`](Self::border_spec_at) —
    /// the SAME predicate the paint loop uses to decide whether that neighbour draws its right edge —
    /// so ownership and draw can never disagree (a `border != 0` that resolved to `NONE` under some
    /// future partial snapshot would be skipped by BOTH, not dropped between them).
    fn no_left_owner(&self, row: u32, col: u32, frame: &Frame) -> bool {
        col == frame.cols.start || self.border_spec_at(row, col - 1).is_none()
    }

    /// Whether no cell *above* `(row, col)` will draw the shared top edge as its own bottom edge
    /// (the horizontal analogue of [`no_left_owner`](Self::no_left_owner); same shared predicate).
    fn no_top_owner(&self, row: u32, col: u32, frame: &Frame) -> bool {
        row == frame.rows.start || self.border_spec_at(row - 1, col).is_none()
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

/// The `(insert_before, insert_after, delete)` merge-guard block flags for a header run
/// (`components/grid_structure.md §5.3`). Insert-before / delete affect the run's start index;
/// insert-after affects one past the run's end. `true` = the op would displace a merge → disabled.
fn merge_block_flags(axis: RowOrCol, run: (u32, u32), merges: &[CellRange]) -> (bool, bool, bool) {
    let (start, end) = run;
    let after = end.saturating_add(1);
    match axis {
        RowOrCol::Row => (
            blocks_row_op(merges, start),
            blocks_row_op(merges, after),
            blocks_row_op(merges, start),
        ),
        RowOrCol::Col => (
            blocks_col_op(merges, start),
            blocks_col_op(merges, after),
            blocks_col_op(merges, start),
        ),
    }
}

/// A cell's px rectangle in **content-local** coordinates (origin at the content area's
/// top-left, before the scroll offset is applied via the axis offsets). Reads through the frame's
/// preview accessors, so a live resize reflows it with no axis rebuild.
fn cell_rect(row: u32, col: u32, frame: &Frame) -> (f32, f32, f32, f32) {
    let x = (frame.col_offset(col) - frame.scroll_x) as f32;
    let y = (frame.row_offset(row) - frame.scroll_y) as f32;
    let w = frame.col_size(col);
    let h = frame.row_size(row);
    (x, y, w, h)
}

/// The px rectangle spanning an index range `[c0, c1) × [r0, r1)` in content-local coords.
fn span_rect(rows: Range<u32>, cols: Range<u32>, frame: &Frame) -> (f32, f32, f32, f32) {
    let x0 = frame.col_offset(cols.start) - frame.scroll_x;
    let x1 = frame.col_offset(cols.end) - frame.scroll_x;
    let y0 = frame.row_offset(rows.start) - frame.scroll_y;
    let y1 = frame.row_offset(rows.end) - frame.scroll_y;
    (x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32)
}

/// A vertical border edge: a solid `edge.weight`-px strip centred on `boundary_x` (the shared
/// column boundary), spanning the cell's row height. Painted over the gridline/fills.
fn vertical_edge_quad(boundary_x: f32, y: f32, h: f32, edge: Edge) -> AnyElement {
    let w = edge.weight as f32;
    rect_div(boundary_x - w / 2.0, y, w, h)
        .bg(to_rgba(edge.color))
        .into_any_element()
}

/// A horizontal border edge: a solid `edge.weight`-px strip centred on `boundary_y` (the shared
/// row boundary), spanning the cell's column width.
fn horizontal_edge_quad(x: f32, boundary_y: f32, w: f32, edge: Edge) -> AnyElement {
    let h = edge.weight as f32;
    rect_div(x, boundary_y - h / 2.0, w, h)
        .bg(to_rgba(edge.color))
        .into_any_element()
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
    font_family: Option<SharedString>,
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
        // A non-default font size renders at `q/4` pt → px (`components/style_render.md`); the
        // default (`0`) keeps the grid's `CELL_FONT_PX`. Mirror/pending cells pass `style: None`,
        // so they always render in the default font (`functional_spec.md §1.2`).
        if s.font_size_q != 0 {
            el = el.text_size(px(s.font_size_q as f32 / 4.0 * 96.0 / 72.0));
        }
    }
    // A non-default family renders per-cell (missing families fall back via gpui's fallback stack).
    if let Some(name) = font_family {
        el = el.font_family(name);
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

/// The live-resize size tooltip (`Width: N` / `Height: N`) anchored at grid-local `(x, y)`
/// (`ui_design.md §3`). A small dark chip matching the app tooltip style.
fn resize_tooltip(x: f32, y: f32, label: String) -> AnyElement {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .px_2()
        .py_1()
        .bg(rgb(IN_CELL_TOOLTIP_BG))
        .text_color(rgb(IN_CELL_TOOLTIP_TEXT))
        .text_size(px(11.0))
        .rounded_md()
        .shadow_md()
        .whitespace_nowrap()
        .child(label)
        .into_any_element()
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
            .debug_selector(|| "in-cell-editor".into())
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            // Capture clicks inside the editor so a click within its bounds moves the caret instead
            // of falling through to the grid's mouse-down — which would treat it as a click-away and
            // commit + close the edit (BUG D). The hosted input paints above this hitbox, so it
            // still receives its own clicks; a click OUTSIDE the editor still reaches the grid and
            // commits (outside-commit preserved).
            .occlude()
            .flex()
            .items_center()
            .bg(rgb(CELL_BG))
            .border_2()
            .border_color(border)
            .px(px(1.0))
            .text_size(px(CELL_FONT_PX))
            .text_color(rgb(CELL_TEXT))
            // Strip the hosted input's own chrome (border / rounded / background / shadow) via
            // `appearance(false)` so it reads as editing the cell in place, not a control-in-a-box
            // (BUG D). The 2 px accent border on this wrapper is the intended in-place edit cue
            // (`ui_design.md §3`). Pin the input's text to the cell font (its default is `text_sm`
            // = 14 px, one off the 13 px cell) and drop its horizontal padding so glyphs line up
            // with the cell's own text rather than sitting inset like a control.
            .child(
                Input::new(input)
                    .appearance(false)
                    .text_size(px(CELL_FONT_PX))
                    .px_0()
                    .w_full(),
            );

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
            // Divider resize hotspots paint last (over the header strips) so they win the hit-test.
            root_children.extend(self.resize_hotspots(&frame, cx));
        }

        // Header insert/delete context menu (deferred → above everything but the loading overlay).
        if let Some(menu) = self.header_menu {
            root_children.extend(self.header_menu_elements(menu, cx));
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
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.handle_right_mouse_down(event, window, cx);
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

    // ---- Phase 7: structure (resize, header selection, merge-menu) ------------------------

    /// A left mouse-down at grid-local `(x, y)` with no modifiers.
    fn mouse_ev(button: MouseButton, x: f32, y: f32) -> MouseDownEvent {
        MouseDownEvent {
            button,
            position: gpui::point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }
    }

    #[test]
    fn axis_preview_is_o_visible_over_a_huge_axis() {
        // A live-resize preview must NOT rebuild the axis (that would be O(sheet) per drag frame,
        // blowing the §4 budget). `AxisPreview` reads through the COMMITTED prefix sums with an
        // O(1)-per-track delta — verify over an Excel-max axis (built once here, never per read).
        let axis = Axis::new(freecell_core::limits::MAX_ROWS, |i| {
            if i == 100 {
                40.0
            } else {
                24.0
            }
        });
        // Grow track 100 from 40 → 60 px (delta +20).
        let grow = AxisPreview {
            index: 100,
            new_px: 60.0,
            base_px: 40.0,
        };
        assert_eq!(grow.size(&axis, 100), 60.0); // dragged track reports the new size
        assert_eq!(grow.size(&axis, 101), 24.0); // neighbour unchanged
                                                 // Offsets: up to/at the index unchanged; after the index shifted by +20.
        assert!((grow.offset(&axis, 100) - axis.offset_of(100)).abs() < 1e-6);
        assert!((grow.offset(&axis, 101) - (axis.offset_of(101) + 20.0)).abs() < 1e-6);
        // A deep track (near the end) also shifts by exactly the delta — O(1), no rebuild.
        let deep = freecell_core::limits::MAX_ROWS - 5;
        assert!((grow.offset(&axis, deep) - (axis.offset_of(deep) + 20.0)).abs() < 1e-6);
        // Total is the committed total + delta (O(1)).
        assert!((grow.total(&axis) - (axis.total() + 20.0)).abs() < 1e-6);
        // A grow widens the NEAR end (not the far end): when the dragged index is scrolled off the
        // top, the grown tracks map to earlier raw indices, so the query starts `delta` earlier.
        assert_eq!(grow.grow_extent(), 20.0);
        assert_eq!(grow.shrink_extent(), 0.0);
        // A shrink pulls later tracks into view → its FAR extent widens by |delta| (no near widen).
        let shrink = AxisPreview {
            index: 100,
            new_px: 10.0,
            base_px: 40.0,
        };
        assert_eq!(shrink.shrink_extent(), 30.0);
        assert_eq!(shrink.grow_extent(), 0.0);
        assert!((shrink.offset(&axis, 101) - (axis.offset_of(101) - 30.0)).abs() < 1e-6);
    }

    #[gpui::test]
    fn grow_preview_scrolled_past_index_widens_near_end(cx: &mut TestAppContext) {
        // Regression: a GROW frozen preview whose dragged track scrolls off the top must not blank
        // the near edge. A grow shifts later tracks away from the origin, so at a given scroll the
        // top of the viewport shows EARLIER raw indices than the un-previewed range — the query must
        // start earlier (the near-widening). Verified by comparing the fetched start with/without the
        // preview at the same deep scroll (robust to the demo's row sizes).
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, _window, cx| {
                g.update(cx, |grid, _cx| {
                    let sheet = grid.active_sheet();
                    // Scroll well past a shallow row, no preview → the baseline range.
                    grid.scroll.insert(sheet, (0.0, 300.0));
                    let baseline = grid.resolve_frame(1200.0, 800.0).expect("resolves").rows;
                    // Same scroll, but a large grow frozen at row 2 → the near-widening must pull the
                    // fetched start EARLIER so the grown tracks shifted into view at the top are drawn.
                    grid.resize_preview = Some(ResizeDrag {
                        axis: RowOrCol::Row,
                        index: 2,
                        start_px: 24.0,
                        current_px: 200.0,
                        run: (2, 2),
                        origin_coord: 0.0,
                    });
                    let with_grow = grid.resolve_frame(1200.0, 800.0).expect("resolves").rows;
                    assert!(
                        with_grow.start < baseline.start,
                        "grow near-widening must fetch earlier tracks: {with_grow:?} vs baseline {baseline:?}"
                    );
                });
            })
            .unwrap();
    }

    #[test]
    fn merge_block_flags_match_predicate() {
        use freecell_core::{CellRange, CellRef};
        // A merge over columns 2..=4 (0-based): a column op at/before 4 blocks, past 4 allows.
        let merges = [CellRange::new(CellRef::new(0, 2), CellRef::new(0, 4))];
        // Run (0,1): insert-before at 0 → blocked (merge extends to 4 >= 0); insert-after at 2 →
        // blocked; delete at 0 → blocked.
        assert_eq!(
            merge_block_flags(RowOrCol::Col, (0, 1), &merges),
            (true, true, true)
        );
        // Run (5,6): insert-before at 5 (past the merge) → allowed; after at 7 → allowed.
        assert_eq!(
            merge_block_flags(RowOrCol::Col, (5, 6), &merges),
            (false, false, false)
        );
        // No merges → nothing blocked.
        assert_eq!(
            merge_block_flags(RowOrCol::Row, (0, 0), &[]),
            (false, false, false)
        );
    }

    #[gpui::test]
    fn header_clicks_and_select_all(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // A column header click selects the full column (band form).
                    grid.select_column(3, false, window, cx);
                    assert!(is_full_column_selection(grid.selection()));
                    assert_eq!(grid.selection().range().start.col, 3);
                    assert_eq!(grid.selection().range().end.col, 3);

                    // Shift extends the column run (anchor stays at col 3).
                    grid.select_column(5, true, window, cx);
                    assert!(is_full_column_selection(grid.selection()));
                    assert_eq!(grid.selection().range().start.col, 3);
                    assert_eq!(grid.selection().range().end.col, 5);

                    // A row header click selects the full row.
                    grid.select_row(2, false, window, cx);
                    assert!(is_full_row_selection(grid.selection()));
                    assert_eq!(grid.selection().range().start.row, 2);

                    // Select-all covers the whole sheet.
                    grid.select_all(window, cx);
                    assert!(is_full_column_selection(grid.selection()));
                    assert_eq!(grid.selection().range().start, CellRef::new(0, 0));
                    assert_eq!(
                        grid.selection().range().end,
                        CellRef::new(
                            freecell_core::limits::MAX_ROWS - 1,
                            freecell_core::limits::MAX_COLS - 1
                        )
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn resize_run_uses_selection_and_clamps(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Select full columns 1..=3, then a resize inside that run applies to all three.
                    grid.select_column(1, false, window, cx);
                    grid.select_column(3, true, window, cx);
                    assert_eq!(grid.resize_run_for(RowOrCol::Col, 2), (1, 3));
                    // Outside the run → just that column.
                    assert_eq!(grid.resize_run_for(RowOrCol::Col, 7), (7, 7));

                    // A live resize clamps to the minimum column width.
                    grid.resize_drag = Some(ResizeDrag {
                        axis: RowOrCol::Col,
                        index: 2,
                        start_px: 100.0,
                        current_px: 100.0,
                        run: (1, 3),
                        origin_coord: 100.0,
                    });
                    grid.update_resize(-100.0, 0.0, cx); // dragged 200 px left → below the 8 px min
                    assert_eq!(grid.resize_drag.unwrap().current_px, MIN_COL_WIDTH_PX);

                    // Committing emits ResizeCommitted over the whole run and freezes the preview.
                    let rd = grid.resize_drag.take().unwrap();
                    events.borrow_mut().clear();
                    grid.commit_resize(rd, window, cx);
                    assert!(grid.resize_preview.is_some());
                    assert!(events.borrow().iter().any(|e| matches!(
                        e,
                        GridEvent::ResizeCommitted {
                            axis: RowOrCol::Col,
                            start: 1,
                            end: 3,
                            ..
                        }
                    )));
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn resize_escape_cancels(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.resize_drag = Some(ResizeDrag {
                        axis: RowOrCol::Row,
                        index: 4,
                        start_px: 24.0,
                        current_px: 40.0,
                        run: (4, 4),
                        origin_coord: 0.0,
                    });
                    grid.handle_key_down(&key_ev("escape", None, false), window, cx);
                    assert!(grid.resize_drag.is_none(), "Escape cancels a live resize");
                    assert!(grid.resize_preview.is_none());
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn right_click_column_header_opens_menu(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Bounds are uncaptured in a headless test, so grid-local == window coords: a
                    // point in the column-header strip (y < 24) past the gutter (x > 48).
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 60.0, 10.0),
                        window,
                        cx,
                    );
                    let menu = grid
                        .header_menu
                        .expect("a header right-click opens the menu");
                    assert_eq!(menu.axis, RowOrCol::Col);
                    // The demo sheet has no merges → nothing blocked.
                    assert!(!menu.insert_before_blocked && !menu.delete_blocked);
                    // Escape closes it.
                    grid.handle_key_down(&key_ev("escape", None, false), window, cx);
                    assert!(grid.header_menu.is_none());
                });
            })
            .unwrap();
    }

    // ---- BUG D: in-cell editor focus + click capture (`functional_spec.md §1.3`) ----------

    /// A real `GridView` over the demo sources plus a real in-cell `InputState`, wired so the event
    /// sink focuses that input on `OpenInCellEditor` — exactly what the window's `begin_in_cell`
    /// does. Returns the grid, the input, and the window so a **real** mouse event can be dispatched
    /// through gpui (driving the grid root's built-in focus-transfer, which a direct
    /// `handle_mouse_down` call would bypass).
    fn grid_with_incell_focus_sink(
        cx: &mut TestAppContext,
    ) -> (
        gpui::Entity<GridView>,
        gpui::Entity<InputState>,
        gpui::WindowHandle<Root>,
    ) {
        cx.update(gpui_component::init);
        let mut g_out = None;
        let mut in_out = None;
        let g_slot = &mut g_out;
        let in_slot = &mut in_out;
        let window = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx));
            *in_slot = Some(input.clone());
            let sink_input = input.clone();
            let sink = GridEventSink::new(move |e, window, cx| {
                if let GridEvent::OpenInCellEditor(_) = e {
                    sink_input.update(cx, |i, cx| i.focus(window, cx));
                }
            });
            let g = cx.new(|cx| GridView::new(demo_sources(), sink, cx));
            *g_slot = Some(g.clone());
            Root::new(g, window, cx)
        });
        (g_out.unwrap(), in_out.unwrap(), window)
    }

    #[gpui::test]
    fn double_click_keeps_focus_on_in_cell_input(cx: &mut TestAppContext) {
        // Reproduces the exact interactive bug a direct-call test cannot: opening the editor focuses
        // its input, but the grid root's built-in mouse-down focus-transfer runs later in the same
        // bubble dispatch and used to steal focus straight back, leaving no caret. The
        // `prevent_default` in `mouse_down_cell` defeats that; here we drive a *real* double-click.
        let (grid, input, window) = grid_with_incell_focus_sink(cx);
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();

        // Deep in the data area (past the header gutter). The first click selects the cell; the
        // click_count==2 down opens + focuses the editor.
        let pos = gpui::point(px(400.0), px(300.0));
        let mods = Modifiers::default();
        vcx.simulate_click(pos, mods);
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: pos,
            modifiers: mods,
            click_count: 2,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: pos,
            modifiers: mods,
            click_count: 2,
        });

        let (input_focused, grid_focused) = vcx.update(|window, cx| {
            (
                input.read(cx).focus_handle(cx).is_focused(window),
                grid.read(cx).focus_handle(cx).is_focused(window),
            )
        });
        assert!(
            input_focused,
            "after a double-click the in-cell input must hold focus (a blinking caret)"
        );
        assert!(
            !grid_focused,
            "the grid must not re-steal focus after opening the in-cell editor"
        );
    }

    #[gpui::test]
    fn click_inside_open_in_cell_editor_does_not_reach_grid(cx: &mut TestAppContext) {
        // With the editor open, a click *inside* its bounds must be captured by the editor overlay
        // (`.occlude()`), not fall through to the grid's mouse-down — which would treat it as a
        // click-away and commit + close the edit (BUG D). We assert no click-away `SelectionChanged`
        // reaches the grid's event sink.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx));
                g.update(cx, |grid, cx| {
                    grid.set_incell_input(input, cx);
                    grid.set_edit_state(None, Some(CellRef::new(3, 3)), None, cx);
                });
            })
            .unwrap();

        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let bounds = vcx
            .debug_bounds("in-cell-editor")
            .expect("the in-cell editor overlay was painted");
        events.borrow_mut().clear();

        vcx.simulate_click(bounds.center(), Modifiers::default());
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
            "a click inside the in-cell editor must not reach the grid: {:?}",
            events.borrow()
        );
    }
}
