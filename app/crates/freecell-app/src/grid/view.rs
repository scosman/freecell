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
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use parking_lot::RwLock;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, Context, FocusHandle, Focusable, FontWeight,
    Rgba, Window,
};
use gpui_component::spinner::Spinner;

use freecell_core::cache::SheetCaches;
use freecell_core::color::Rgb;
use freecell_core::publication::Publication;
use freecell_core::refs::{column_label, SheetId};
use freecell_core::{Align, Axis, RenderStyle, SelectionModel};

use super::layout::{
    self, ContentArea, COL_HEADER_H, RENDER_OVERSCAN, SCROLLBAR_INSET, SCROLLBAR_THICKNESS,
};
use super::{
    GridEvent, GridEventSink, ACCENT, CELL_BG, CELL_FONT_PX, CELL_H_PAD, CELL_TEXT, GRIDLINE,
    HEADER_BG, HEADER_FONT_PX, HEADER_HAIRLINE, HEADER_SELECTED_BG, HEADER_TEXT,
    SCROLLBAR_FADE_SECS, SCROLLBAR_RGBA, SELECTION_FILL_ALPHA,
};

/// The worker-written / UI-read data the grid renders from (`components/grid.md §Public
/// interface`). In Phase 6 these are built from hand fixtures ([`super::fixtures`]); the
/// worker fills them for real in Phase 11.
pub struct GridDataSources {
    /// The active sheet's overscanned viewport values snapshot, swapped by the worker.
    pub publication: Arc<ArcSwap<Publication>>,
    /// The resident geometry + resolved-style caches (worker writes, UI reads).
    pub caches: Arc<RwLock<SheetCaches>>,
    /// The generation counter — bumped after each publish (read by Phase-11 wiring).
    pub generation: Arc<AtomicU64>,
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
    /// Monotonic scroll epoch; a fade task only hides if the epoch is unchanged when it fires.
    scroll_activity: u64,
    /// The last emitted visible range (for `ViewportChanged` debouncing).
    last_viewport: Option<(Range<u32>, Range<u32>)>,
    /// A pending `scroll_cell_into_view` request applied on the next render.
    pending_reveal: Option<(u32, u32)>,
    /// Reused per-frame index: visible `(row, col)` → index into the publication's cells.
    cell_index: HashMap<(u32, u32), usize>,
    /// Reused per-frame snapshot: visible `(row, col)` → resolved style (default = absent).
    visible_styles: HashMap<(u32, u32), RenderStyle>,
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
            scroll_activity: 0,
            last_viewport: None,
            pending_reveal: None,
            cell_index: HashMap::new(),
            visible_styles: HashMap::new(),
        }
    }

    /// Switches the active sheet, restoring its scroll + selection (origin + A1 if unseen).
    pub fn set_active_sheet(&mut self, sheet: SheetId, cx: &mut Context<Self>) {
        self.active_sheet = sheet;
        self.scroll.entry(sheet).or_insert((0.0, 0.0));
        self.selection.entry(sheet).or_default();
        // Force the next scroll/publish to re-announce the viewport for the new sheet.
        self.last_viewport = None;
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
        // PHASE 11: `viewport_size()` is the whole window — correct while the grid is
        // full-window (Phase 6 demo + Phase 7 render harness). Once chrome (toolbar / data row
        // / tab bar) wraps the grid, this must switch to the grid element's own laid-out bounds,
        // or the visible-range / scroll-clamp math over-computes and allows slight over-scroll.
        let viewport = window.viewport_size();
        let viewport_w = f32::from(viewport.width) as f64;
        let viewport_h = f32::from(viewport.height) as f64;
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

    el = match style.and_then(|s| s.h_align).unwrap_or(Align::Left) {
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

impl Focusable for GridView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // PHASE 11: `viewport_size()` is the whole window — correct while the grid is
        // full-window (Phase 6 demo + Phase 7 render harness). Once chrome wraps the grid, this
        // must switch to the grid element's own laid-out bounds, or virtualization over-computes
        // the visible range (see the matching note in `handle_scroll`).
        let viewport = window.viewport_size();
        let viewport_w = f32::from(viewport.width) as f64;
        let viewport_h = f32::from(viewport.height) as f64;

        let mut root_children: Vec<AnyElement> = Vec::new();

        if let Some(frame) = self.resolve_frame(viewport_w, viewport_h) {
            let selection = *self.selection();
            let publication = self.sources.publication.load_full();
            let covers_active = publication.sheet == self.active_sheet;

            // Rebuild the reused visible-cell index from the publication. The scan is over the
            // published (non-empty) cells, which the worker caps at `MAX_PUBLISH_ROWS ×
            // MAX_PUBLISH_COLS` (512×256) and are typically far fewer than that — not O(sheet).
            // The publication has no spatial index, so a per-visible-cell lookup would need this
            // map first; building it once per frame is the right structure given the flat `Vec`.
            // PHASE 12: the perf harness should confirm this scan stays within the frame
            // p99 ≤ 8.33 ms gate on the 1M×100 styled fixture.
            self.cell_index.clear();
            if covers_active {
                for (i, cell) in publication.cells.iter().enumerate() {
                    if frame.rows.contains(&cell.row) && frame.cols.contains(&cell.col) {
                        self.cell_index.insert((cell.row, cell.col), i);
                    }
                }
            }

            // ---- Content layer: cells + selection, clipped to the content area ----------
            let mut content_children: Vec<AnyElement> = Vec::with_capacity(
                ((frame.rows.end - frame.rows.start) * (frame.cols.end - frame.cols.start))
                    as usize
                    + 16,
            );

            for r in frame.rows.clone() {
                for c in frame.cols.clone() {
                    let (x, y, w, h) = cell_rect(r, c, &frame);
                    let style = self.visible_styles.get(&(r, c)).copied();
                    let fill = style
                        .and_then(|s| s.fill)
                        .map(to_rgba)
                        .unwrap_or_else(|| rgb(CELL_BG));
                    let (text, text_color) = match self.cell_index.get(&(r, c)) {
                        Some(&idx) => {
                            let pc = &publication.cells[idx];
                            let color = pc
                                .text_color
                                .or(style.and_then(|s| s.font_color))
                                .map(to_rgba)
                                .unwrap_or_else(|| rgb(CELL_TEXT));
                            (pc.display_text.clone(), color)
                        }
                        None => (String::new(), rgb(CELL_TEXT)),
                    };
                    content_children.push(cell_element(x, y, w, h, fill, text, text_color, style));
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
                let (x, y, w, h) = span_rect(rows, cols, &frame);
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
                    &frame,
                );
                content_children.push(
                    rect_div(x, y, w, h)
                        .border_2()
                        .border_color(rgb(ACCENT))
                        .into_any_element(),
                );
            }
            {
                let (x, y, w, h) = cell_rect(selection.active.row, selection.active.col, &frame);
                content_children.push(
                    rect_div(x, y, w, h)
                        .border_2()
                        .border_color(rgb(ACCENT))
                        .into_any_element(),
                );
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
                let (x, _y, w, _h) = span_rect(0..1, sel_c0..sel_c1 + 1, &frame);
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
                let (_x, y, _w, h) = span_rect(sel_r0..sel_r1 + 1, 0..1, &frame);
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
                    let y = COL_HEADER_H + frame.content_h as f32
                        - SCROLLBAR_THICKNESS
                        - SCROLLBAR_INSET;
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
        }

        // ---- Loading overlay (over everything) ------------------------------------------
        if let Some(name) = self.loading.clone() {
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
                    .child(Spinner::new())
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
            .children(root_children)
    }
}

impl GridView {
    /// The active sheet (for tests / Phase-11 wiring).
    pub fn active_sheet(&self) -> SheetId {
        self.active_sheet
    }
}
