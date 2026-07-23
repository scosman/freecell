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
    canvas, deferred, div, font, prelude::*, px, rgb, rgba, AnyElement, App, Bounds, Context,
    Entity, FocusHandle, Focusable, FontWeight, Hsla, KeyDownEvent, LineFragment, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, SharedString, TextRun,
    Window,
};
use gpui_component::input::{Input, InputState};
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};

use freecell_chart_model::{ChartId, ChartSpec};

use freecell_core::cache::{SheetCaches, DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX};
use freecell_core::color::Rgb;
use freecell_core::publication::{CellKind, Publication};
use freecell_core::refs::{column_label, SheetId};
use freecell_core::selection::{Direction, Motion};
use freecell_core::{
    apply_motion, blocks_col_op, blocks_row_op, effective_edge, is_full_column_selection,
    is_full_row_selection, spans_all_cols, spans_all_rows, Align, Axis, BorderSpec, CellRange,
    CellRef, Edge, FillAxis, LinePattern, RenderStyle, SelectionModel, SheetDims, VAlign,
};

use crate::chrome::AutocompleteDisplay;

use super::chart_layer::{self, ChartPlacement, ChartRect, Handle, HANDLE_HIT_HALF, HANDLE_PX};
use super::input::{command_for_key, GridKeyCommand};
use super::layout::{
    self, ContentArea, GridHit, COL_HEADER_H, RENDER_OVERSCAN, SCROLLBAR_INSET, SCROLLBAR_THICKNESS,
};
use super::{
    GridEvent, GridEventSink, RowOrCol, ACCENT, AUTOSCROLL_INTERVAL_MS, CELL_BG, CELL_FONT_PX,
    CELL_H_PAD, CELL_TEXT, EDGE_AUTOSCROLL_HOTZONE_PX, EDGE_AUTOSCROLL_STEP_PX, GRIDLINE,
    GRID_FONT_FAMILY, HEADER_BG, HEADER_FONT_PX, HEADER_HAIRLINE, HEADER_SELECTED_BG, HEADER_TEXT,
    SCROLLBAR_FADE_SECS, SCROLLBAR_RGBA, SELECTION_FILL_ALPHA,
};

/// Half-width (px) of a divider resize hotspot (`ui_design.md §3`: a 6 px zone centered on the
/// divider). Also the ±3 px within which the resize cursor shows (`functional_spec.md §5.1`).
const RESIZE_HOTSPOT_HALF: f32 = 3.0;
/// Minimum column width / row height a resize drag clamps to (`functional_spec.md §5.1`).
const MIN_COL_WIDTH_PX: f32 = 8.0;
const MIN_ROW_HEIGHT_PX: f32 = 12.0;

/// Extra width (device px) added around the widest measured cell text when autofitting a column
/// (double-click the column divider, `functional_spec.md §7`): the cell's left+right text padding
/// (`2 × CELL_H_PAD`) plus a small buffer so the content clears the gridline. Keeps an autofit
/// column just wide enough that its widest published cell does not overflow/spill
/// (`text_overflows_column` fits when `col_w >= text + 2 · CELL_H_PAD`).
const AUTOFIT_PADDING_PX: f32 = 2.0 * CELL_H_PAD + 3.0;
/// Floor (device px) for an autofit column — the configured minimum (D7.3), wide enough to keep the
/// column-letter header label readable; an empty column shrinks to this.
const AUTOFIT_MIN_WIDTH_PX: f32 = 24.0;
/// Cap (device px) for an autofit column so a single very long value can't produce a runaway width.
const AUTOFIT_MAX_WIDTH_PX: f32 = 800.0;

/// gpui's default text line-height multiple — the golden ratio `phi` (`gpui::geometry::phi` =
/// `relative(1.618034)`, applied as `Style::line_height`). The wrap auto-grow measurement uses the
/// SAME factor so a grown row fits exactly the number of lines gpui renders (a smaller factor would
/// under-grow and clip the top line).
const GRID_LINE_HEIGHT_FACTOR: f32 = 1.618_034;

/// Whether a keystroke's modifiers signal **caret / selection intent** for quick-edit
/// (`functional_spec.md §5.3`): Shift, Cmd/Ctrl (`platform`/`control`), or Alt/Option. Deliberately
/// **excludes** `function` — macOS sets `Modifiers::function` on the arrow / Home / End cluster
/// itself, so `Modifiers::modified()` would report a *plain* arrow as modified and defeat §5.2's
/// commit-and-move on macOS. This mirrors [`command_for_key`](super::input::command_for_key), which
/// decomposes arrows into `(secondary, shift)` and never consults `function`. Shared by the data-row
/// and in-cell arrow arms so both apply the identical rule.
pub(crate) fn caret_intent_modifiers(modifiers: &Modifiers) -> bool {
    modifiers.shift || modifiers.control || modifiers.alt || modifiers.platform
}

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
    /// Hide is disabled when hiding the run would leave **zero** visible tracks on the axis
    /// (reachable only via Select-All → Hide, since the axis is Excel-max; `gaps_closing_7_15 §4`).
    hide_blocked: bool,
    /// The minimal `[first_hidden, last_hidden]` span of hidden tracks **within** the run, or
    /// `None` when the run contains no hidden track (Unhide disabled). Unhide targets this span (not
    /// the whole run) so Select-All → Unhide reveals the hidden cluster in one undo step without
    /// touching the whole 1M-row axis.
    unhide_run: Option<(u32, u32)>,
    /// How many tracks in the run are already hidden — drives the accurate menu-item counts
    /// (Unhide N; Hide counts the newly-hidden `run_len − hidden_in_run`), independent of the
    /// span width (`unhide_run` may be wider than the count when the hidden tracks are sparse).
    hidden_in_run: u32,
}

/// The open cell-area right-click context menu (`functional_spec.md §2`, cloned from
/// [`HeaderMenu`]). `x`/`y` are grid-local; `range` is the selection rectangle snapshotted at
/// open (the menu is modal via its backdrop, so the selection can't drift while it's up) — its
/// rows/cols set the insert/delete span and it is itself the Clear-contents target. `paste_enabled`
/// gates both Paste and Paste-values (the grid can't distinguish an internal vs foreign clipboard —
/// that state lives in the window's `ClipboardCoordinator` — and Paste-values falls back to a TSV
/// paste for a foreign clipboard anyway, `functional_spec.md §5`, so they share one gate). The
/// `*_blocked` flags disable an insert/delete item when the op would displace a file-loaded merge,
/// reusing the header menu's merge guard for each axis.
#[derive(Debug, Clone, Copy)]
struct CellMenu {
    x: f32,
    y: f32,
    range: CellRange,
    paste_enabled: bool,
    insert_row_above_blocked: bool,
    insert_row_below_blocked: bool,
    delete_rows_blocked: bool,
    insert_col_left_blocked: bool,
    insert_col_right_blocked: bool,
    delete_cols_blocked: bool,
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

/// What an in-flight chart drag on the ChartLayer is doing (P18, `ui_design §3.2`): moving the whole
/// chart body, or resizing it from one of its eight [`Handle`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChartDragMode {
    Move,
    Resize(Handle),
}

/// An in-flight chart move/resize drag on the ChartLayer (P18). `grab` is the content-local pointer
/// position at mouse-down; `start_rect` the chart's rect then; `current_rect` the live previewed rect
/// (recomputed each move). On release [`rect_to_anchor`](chart_layer::rect_to_anchor) maps the final
/// rect back to an [`Anchor`] the worker persists.
#[derive(Debug, Clone, Copy)]
struct ChartDrag {
    id: ChartId,
    mode: ChartDragMode,
    grab: (f32, f32),
    start_rect: ChartRect,
    current_rect: ChartRect,
}

/// An in-flight drag of the selection's fill handle (`gaps_closing_7_15 §3`), mirroring
/// [`ChartDrag`]. `seed` is the selection at mouse-down; `target` (⊇ seed) is the live previewed
/// fill region, recomputed each move; `axis` is the dominant fill direction — `None` until the
/// pointer first leaves the seed, then **sticky** (kept until the pointer returns inside the seed),
/// so a drag doesn't flip axis mid-gesture (Excel behavior, D3.1).
#[derive(Debug, Clone, Copy)]
struct FillDrag {
    seed: CellRange,
    target: CellRange,
    axis: Option<FillAxis>,
}

/// An in-flight **point-mode** drag (`formula-point-mode/functional_spec.md §2`): sweeping a range
/// into the active formula edit. `origin` is the merge-anchor-resolved cell the drag started on;
/// `last_range` is the last (merge-expanded) range emitted as a reference, used both to dedupe
/// per-cell re-emits and as the single-cell case when the drag releases on its origin.
#[derive(Debug, Clone, Copy)]
struct PointDrag {
    origin: CellRef,
    last_range: CellRange,
}

/// A right-click "Delete chart" context menu over a chart (P18, `ui_design §3.2` — the alternate
/// delete affordance to Delete/Backspace). `x`/`y` are grid-local.
#[derive(Debug, Clone, Copy)]
struct ChartMenu {
    id: ChartId,
    x: f32,
    y: f32,
}

/// What a content-local point grabbed on the ChartLayer ([`GridView::chart_hit_test`]).
#[derive(Debug, Clone, Copy)]
enum ChartHit {
    /// A resize handle of the selected chart.
    Handle {
        id: ChartId,
        handle: Handle,
        rect: ChartRect,
    },
    /// A chart body (the whole rect) — selects + begins a move.
    Body { id: ChartId, rect: ChartRect },
}

/// A [`chart_layer::GridGeometry`] over a frame's committed axes — the seam
/// [`GridView::chart_hit_test`] resolves chart rects against on the mouse path (which reads the
/// axes directly under the caches lock rather than building a full [`Frame`]).
struct AxisGeometry<'a> {
    col_axis: &'a Axis,
    row_axis: &'a Axis,
}

impl chart_layer::GridGeometry for AxisGeometry<'_> {
    fn col_start(&self, col: u32) -> f64 {
        self.col_axis.offset_of(col)
    }
    fn row_start(&self, row: u32) -> f64 {
        self.row_axis.offset_of(row)
    }
    fn col_at(&self, x: f64) -> u32 {
        self.col_axis.index_at(x)
    }
    fn row_at(&self, y: f64) -> u32 {
        self.row_axis.index_at(y)
    }
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
    /// The function-completion list to render under the in-cell overlay, or `None`
    /// (`gaps_closing_7_15 §1`). Pushed by the chrome via `set_edit_state`; also read in
    /// `capture_key_down` to intercept nav/accept/dismiss keys while it is open.
    incell_autocomplete: Option<AutocompleteDisplay>,
    /// The passive signature-hint template to render under the in-cell overlay, or `None`.
    incell_sig_hint: Option<SharedString>,
    /// The in-cell editor overlay's measured, viewport-clamped `(width, height)` in device px for the
    /// current frame, or `None` when the editor is closed (or on a non-`render` build path that skips
    /// measurement — the overlay then falls back to its base cell-rect size). Computed in
    /// [`Render::render`] (which holds the `Window` text system) so the editor **grows** to fit the
    /// live text: rightward over neighbours for a wrap-off cell, downward for a wrap-on one
    /// (`DECISIONS_TO_REVIEW.md`). Only the single editing cell is measured, so it stays O(1)/frame.
    incell_geom: Option<(f32, f32)>,
    /// Whether the current pending edit is in **quick-edit** mode (pushed by the chrome via
    /// [`set_edit_state`](Self::set_edit_state), `functional_spec.md §5`). Consumed by the grid-root
    /// `capture_key_down` so an unmodified arrow in the in-cell overlay commits + moves the active
    /// cell instead of the caret. In the current flow type-to-replace edits in the data row and
    /// `begin_in_cell` clears quick-edit, so this is a defensive symmetric mirror of the data-row
    /// path (only ever `true` while an edit is live).
    quick_edit: bool,

    // ---- Formula reference highlighting + point-mode (`formula-point-mode`) --------------------
    /// The same-sheet reference highlights to paint while a formula edit is open
    /// (`formula-point-mode/architecture.md §4.1`): each target range + its palette slot, pushed by
    /// the chrome via [`set_edit_state`](Self::set_edit_state). Painted as a rich fill + border in
    /// the overlay pass; empty while not editing a formula. Cleared on sheet switch.
    ref_highlights: Vec<(CellRange, u8)>,
    /// Whether the driving formula editor's caret is reference-ready (pushed by the chrome,
    /// `formula-point-mode/architecture.md §3.1`). Consumed by [`mouse_down_cell`](Self::mouse_down_cell)
    /// to branch point-insert vs commit.
    reference_ready: bool,
    /// Whether a just-pointed reference is pending (pushed by the chrome). Consumed by
    /// [`mouse_down_cell`](Self::mouse_down_cell): a grid click while set replaces the pending ref
    /// even when the caret is not reference-ready (the pending-ref override).
    pending_ref: bool,

    /// The charts painted on each sheet's **ChartLayer** (P8/P11, `charts/architecture.md §4.2`,
    /// §5 challenge 5), keyed by sheet. See [`SheetChartLayer`]: the per-sheet spec list is **shared**
    /// with the engine's published snapshot (no app-side copy), and the per-frame scan reads only the
    /// tiny [`ChartPlacement`]s, materializing the heavy render `Chart` for the on-screen few.
    charts: HashMap<SheetId, SheetChartLayer>,
    /// The currently **selected** chart (P18) — drawn with a selection outline + resize handles.
    /// `None` when no chart is selected. Keyed by the stable [`ChartId`] the worker stamps, so a
    /// live re-resolve republish keeps the selection; a selection whose chart is gone is dropped on
    /// the next [`set_sheet_charts`](Self::set_sheet_charts).
    selected_chart: Option<ChartId>,
    /// The in-flight chart move/resize drag, if any (`None` = not dragging a chart).
    chart_drag: Option<ChartDrag>,
    /// The in-flight drag of the selection's fill handle, if any (`gaps_closing_7_15 §3`;
    /// `None` = not fill-dragging). Drives the live preview rect + the committed fill on release.
    fill_drag: Option<FillDrag>,
    /// The in-flight point-mode drag, if any (`formula-point-mode/functional_spec.md §2`;
    /// `None` = not point-dragging). Armed by the [`mouse_down_cell`](Self::mouse_down_cell) point
    /// branch when a reference-ready / pending grid click points; drives the live preview rect + the
    /// per-cell `InsertReference` re-emits as the range grows.
    point_drag: Option<PointDrag>,
    /// The open right-click "Delete chart" context menu, if any.
    chart_menu: Option<ChartMenu>,
    /// The open cell-area right-click context menu, if any (`functional_spec.md §2`).
    cell_menu: Option<CellMenu>,

    // ---- Wrap-driven row auto-grow (`functional_spec.md §3`) ----------------------------------
    /// Reused per-frame buffer of the visible **wrap-on, non-empty** cells (populated in
    /// `build_grid_layers` on the render path only — the perf harness leaves it empty). The
    /// post-layout auto-grow pass measures these to grow their rows to fit the wrapped text.
    wrap_cells: Vec<WrapCell>,
    /// Per active-sheet 0-based row → the last-measured **signature** of its wrap inputs
    /// (content / font / column width — NOT the row height). A row is re-measured only when its
    /// signature changes, so a height-only republish never re-triggers auto-grow: the feedback loop
    /// converges in one frame (`architecture.md §3.2`). Cleared on sheet switch.
    wrap_sig: HashMap<u32, u64>,
}

/// A visible wrap-on, non-empty cell captured during the frame build, carrying exactly what the
/// post-layout auto-grow measurement needs (`architecture.md §3.2`): the committed text, the
/// resolved font (size / weight / style / family), and the cell's column width. Collected instead
/// of measured inline because measurement needs the render thread's `Window` (a text system), which
/// `build_grid_layers` doesn't hold.
struct WrapCell {
    row: u32,
    text: SharedString,
    font_px: f32,
    bold: bool,
    italic: bool,
    font_family: Option<SharedString>,
    col_w: f32,
}

/// A populated cell snapshotted for a **row-autofit** measurement (`functional_spec.md §5`): the
/// committed text, its resolved font (size / weight / style / family), its own column width, and
/// whether it wraps — everything [`GridView::measure_row_height`] needs to compute the cell's line
/// box. Captured under the caches lock (like [`WrapCell`]) so measuring can drop the lock first. A
/// wrap-on cell soft-wraps at `col_w`; a wrap-off cell counts only its explicit `\n` segments.
struct AutofitRowCell {
    text: SharedString,
    col_w: f32,
    font_px: f32,
    bold: bool,
    italic: bool,
    font_family: Option<SharedString>,
    wrap: bool,
}

/// One sheet's installed charts (P11, `charts/architecture.md §5` challenge 5, "off-screen free").
///
/// `specs` is the **shared** `Arc<[ChartSpec]>` the engine published — the grid holds a refcount, not
/// a copy, so a sheet with K charts adds no independent resident duplicate of their render pictures
/// or retained source. `placements` is one tiny [`ChartPlacement`] per spec (anchor + derived
/// fidelity), classified once at install so the per-frame cull scan never re-parses source XML. The
/// heavy `specs[i].chart()` is borrowed **only** for a chart that is currently on-screen; an off-screen
/// chart contributes nothing but its placement to the scan (and re-materializes when it scrolls in).
struct SheetChartLayer {
    specs: Arc<[ChartSpec]>,
    placements: Vec<ChartPlacement>,
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

/// The frame's committed (pre-scroll) column/row start offsets are exactly what a chart anchor
/// maps against (`charts/architecture.md §5` challenge 1) — so the ChartLayer reads the same
/// geometry (incl. a live resize preview) the cells do, and scroll/zoom come free.
impl chart_layer::GridGeometry for Frame {
    fn col_start(&self, col: u32) -> f64 {
        self.col_offset(col)
    }
    fn row_start(&self, row: u32) -> f64 {
        self.row_offset(row)
    }
    // The inverse (content x/y → track) resolves against the committed axes. Chart manipulation and
    // a live/frozen resize are separate interaction modes (a chart drag never runs while resizing),
    // so the previewed offset the render path uses and this committed inverse agree in practice.
    fn col_at(&self, x: f64) -> u32 {
        self.col_axis.index_at(x)
    }
    fn row_at(&self, y: f64) -> u32 {
        self.row_axis.index_at(y)
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

/// A deferred text-spill paint (`functional_spec.md §2`): the origin row + the inclusive
/// [`layout::SpillSpan`] of empty neighbour columns the text is painted across, plus the origin
/// cell's resolved text attributes. Collected during the cell loop (whose origin cells suppress
/// their own clipped text) and painted as separate positioned elements after the cell + border
/// layers, so the spilled text sits over the neighbours' fills/gridlines/borders.
struct SpillPlan {
    row: u32,
    span: layout::SpillSpan,
    text: String,
    text_color: Rgba,
    /// The origin cell's effective horizontal alignment — anchors the text within the spill rect
    /// (left → start, right → end, center → centred).
    align: Align,
    /// The origin cell's resolved style (bold/italic/underline/strike/size/vertical-align), or
    /// `None` for the grid default.
    style: Option<RenderStyle>,
    font_family: Option<SharedString>,
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
            incell_autocomplete: None,
            incell_sig_hint: None,
            ref_highlights: Vec::new(),
            reference_ready: false,
            pending_ref: false,
            incell_geom: None,
            quick_edit: false,
            charts: HashMap::new(),
            selected_chart: None,
            chart_drag: None,
            fill_drag: None,
            point_drag: None,
            chart_menu: None,
            cell_menu: None,
            wrap_cells: Vec::new(),
            wrap_sig: HashMap::new(),
        }
    }

    /// Installs the charts to paint on `sheet`'s **ChartLayer** (P8/P11, `charts/architecture.md
    /// §4.2`, §5 challenge 5). `specs` is the **shared** `Arc<[ChartSpec]>` the engine published: the
    /// grid keeps the `Arc` (a refcount bump — no per-chart copy of its render picture / retained
    /// source) and derives one tiny [`ChartPlacement`] per spec (anchor + fidelity) for the per-frame
    /// cull scan. An empty install clears the sheet's charts. Live re-resolves arrive as fresh
    /// snapshots on later `Published` events (P9); each is installed here, replacing the shared slice.
    pub fn set_sheet_charts(
        &mut self,
        sheet: SheetId,
        specs: Arc<[ChartSpec]>,
        cx: &mut Context<Self>,
    ) {
        if specs.is_empty() {
            self.charts.remove(&sheet);
        } else {
            let placements = specs.iter().map(ChartPlacement::from_spec).collect();
            self.charts
                .insert(sheet, SheetChartLayer { specs, placements });
        }
        // Drop a selection / drag / menu whose chart no longer exists on the active sheet (e.g. it
        // was deleted, or its sheet was cleared). A live re-resolve republish keeps the same ids, so
        // it does NOT clear the selection.
        if sheet == self.active_sheet && !self.active_chart_id_present(self.selected_chart) {
            self.selected_chart = None;
            self.chart_drag = None;
            self.chart_menu = None;
        }
        cx.notify();
    }

    /// Whether `id` (if any) names a chart currently installed on the **active** sheet's ChartLayer.
    fn active_chart_id_present(&self, id: Option<ChartId>) -> bool {
        let Some(id) = id else {
            return false;
        };
        self.charts
            .get(&self.active_sheet)
            .is_some_and(|layer| layer.specs.iter().any(|s| s.id == id))
    }

    /// Selects `id`'s chart on the active sheet (P18) — drives the selection outline + resize
    /// handles. `None` clears the selection. The window sets this on chart-manipulation events; the
    /// render harness sets it to baseline the selection chrome.
    pub fn set_selected_chart(&mut self, id: Option<ChartId>, cx: &mut Context<Self>) {
        self.selected_chart = id;
        cx.notify();
    }

    /// The charts of `layer` that fall within the content viewport at the given scroll, as
    /// `(spec index, content-local rect)` pairs — the on-screen set the ChartLayer materializes (P11
    /// "off-screen free"). Scans only the tiny [`ChartPlacement`]s (never the heavy `Chart`), mapping
    /// each anchor to a rect and dropping the ones [`is_offscreen`](chart_layer::ChartRect::is_offscreen)
    /// culls. The single source of truth for both the paint loop and its test helper.
    fn visible_charts(
        layer: &SheetChartLayer,
        geom: &impl chart_layer::GridGeometry,
        scroll_x: f64,
        scroll_y: f64,
        content_w: f64,
        content_h: f64,
    ) -> Vec<(usize, chart_layer::ChartRect)> {
        layer
            .placements
            .iter()
            .enumerate()
            .filter_map(|(i, placement)| {
                let rect = chart_layer::anchor_rect(&placement.anchor, geom, scroll_x, scroll_y);
                (!rect.is_offscreen(content_w, content_h)).then_some((i, rect))
            })
            .collect()
    }

    /// What a content-local point grabs on the active sheet's ChartLayer (P18): a resize
    /// [`Handle`] of the currently-selected chart (checked first — handles straddle the border and
    /// win over the body), or a chart **body** (topmost-first). `None` = the point missed every
    /// chart (the caller then falls through to cell hit-testing / deselects). Scans only the tiny
    /// [`ChartPlacement`]s via [`visible_charts`](Self::visible_charts).
    fn chart_hit_test(
        &self,
        geom: &impl chart_layer::GridGeometry,
        scroll: (f64, f64),
        content: (f64, f64),
        point: (f32, f32),
    ) -> Option<ChartHit> {
        let layer = self.charts.get(&self.active_sheet)?;
        let (cx, cy) = point;
        let visible = Self::visible_charts(layer, geom, scroll.0, scroll.1, content.0, content.1);
        // A handle of the SELECTED chart wins first (handles sit on/just outside the border).
        if let Some(sel) = self.selected_chart {
            if let Some((_, rect)) = visible.iter().find(|(i, _)| layer.specs[*i].id == sel) {
                if let Some(handle) = chart_layer::handle_at(*rect, cx, cy) {
                    return Some(ChartHit::Handle {
                        id: sel,
                        handle,
                        rect: *rect,
                    });
                }
            }
        }
        // Otherwise the topmost chart body under the point (later charts paint on top).
        visible.iter().rev().find_map(|(i, rect)| {
            let inside =
                cx >= rect.x && cx <= rect.x + rect.w && cy >= rect.y && cy <= rect.y + rect.h;
            inside.then(|| ChartHit::Body {
                id: layer.specs[*i].id,
                rect: *rect,
            })
        })
    }

    /// Test introspection for P11 "off-screen free": the spec indices of `sheet`'s charts that are
    /// on-screen at the given scroll/viewport (against a supplied geometry) — proving off-screen
    /// charts are culled out of the materialized set and re-enter it when scrolled back into view.
    #[cfg(test)]
    pub(crate) fn on_screen_chart_indices(
        &self,
        sheet: SheetId,
        geom: &impl chart_layer::GridGeometry,
        scroll_x: f64,
        scroll_y: f64,
        content_w: f64,
        content_h: f64,
    ) -> Vec<usize> {
        self.charts
            .get(&sheet)
            .map(|layer| {
                Self::visible_charts(layer, geom, scroll_x, scroll_y, content_w, content_h)
                    .into_iter()
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Installs the reused in-cell editor input the chrome owns, so the grid can render the overlay
    /// (`components/edit_controller.md §4.4`). Called once at window wiring time.
    pub fn set_incell_input(&mut self, input: Entity<InputState>, cx: &mut Context<Self>) {
        self.incell_input = Some(input);
        cx.notify();
    }

    /// Pushes the chrome's current edit state onto the grid (live mirror, in-cell overlay cell,
    /// in-cell cap message, autocomplete/sig-hint) plus the formula reference state
    /// (`formula-point-mode/architecture.md §3.1`): `reference_ready` / `pending_ref` (stored for
    /// the Phase-3 point-vs-commit branch) and `ref_highlights` (the same-sheet ranges painted in
    /// the overlay pass). `None`s / empties clear the corresponding overlay. Repaints so the mirror
    /// + highlights track each keystroke (`components/edit_controller.md §4.3–4.4`).
    #[allow(clippy::too_many_arguments)]
    pub fn set_edit_state(
        &mut self,
        mirror: Option<(SheetId, CellRef, SharedString)>,
        incell_open: Option<CellRef>,
        incell_cap: Option<SharedString>,
        quick_edit: bool,
        autocomplete: Option<AutocompleteDisplay>,
        sig_hint: Option<SharedString>,
        reference_ready: bool,
        pending_ref: bool,
        ref_highlights: Vec<(CellRange, u8)>,
        cx: &mut Context<Self>,
    ) {
        self.mirror = mirror;
        self.quick_edit = quick_edit;
        self.incell_autocomplete = autocomplete;
        self.incell_sig_hint = sig_hint;
        self.reference_ready = reference_ready;
        self.pending_ref = pending_ref;
        self.ref_highlights = ref_highlights;
        // Opening the in-cell editor ends any grid selection drag at its root (BUG #2): a drag armed
        // before the editor opened must not survive into (or past) the editor's lifetime. The
        // overlay `.occlude()`s the follow-up mouse-up, so the grid would never clear such a drag,
        // and after the editor closes a later hover would extend a phantom selection. The
        // `handle_mouse_move` gate remains as belt-and-braces while the editor is open.
        if incell_open.is_some() {
            self.drag = None;
        }
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

    /// Classifies a candidate spill-neighbour `(row, col)` for the text-spill scan
    /// (`functional_spec.md §2`): [`layout::Occupancy::Empty`] only when the cell is content-free
    /// **and** its coverage is known — a pending edit (mirror), an off-coverage cell (never treat
    /// "beyond covered" as empty, §2.5), or a published (non-empty) cell all read `Blocked`. A
    /// fill/border-only empty cell is `Empty` (content-only stop): it carries a resolved style but
    /// no entry in the publication-derived `cell_index`. Called only for in-frame columns, where
    /// `cell_index` is authoritative over the published cells.
    fn neighbor_occupancy(
        &self,
        row: u32,
        col: u32,
        publication: &Publication,
    ) -> layout::Occupancy {
        if self.mirror_text_for(CellRef::new(row, col)).is_some()
            || !publication.covers(row, col)
            || self.cell_index.contains_key(&(row, col))
        {
            layout::Occupancy::Blocked
        } else {
            layout::Occupancy::Empty
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
        self.incell_autocomplete = None;
        self.incell_sig_hint = None;
        // Reference highlights + point-mode signals are anchored to the previous sheet's formula
        // edit — drop them so they can never leak onto the new sheet.
        self.ref_highlights.clear();
        self.reference_ready = false;
        self.pending_ref = false;
        self.point_drag = None;
        // Structural interactions are anchored to the previous sheet's geometry — drop them.
        self.resize_drag = None;
        self.resize_preview = None;
        self.header_menu = None;
        self.cell_menu = None;
        // The wrap-measurement signatures are per-sheet; drop them so the new sheet's wrap rows are
        // measured fresh (heights themselves live in the worker's cache, so this only re-measures).
        self.wrap_sig.clear();
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

    /// Select a single cell and scroll it into view — the find bar's current-match reveal
    /// (`functional_spec.md §4.3`). Sets the selection **without** emitting a grid
    /// `SelectionChanged` (the chrome-grid sink mirrors it into the chrome itself, avoiding a
    /// double fold), then reveals it (which announces the possibly-widened viewport so an
    /// off-screen match is published). The caller keeps the find field focused.
    pub fn select_and_reveal(
        &mut self,
        cell: CellRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_selection(SelectionModel::single(cell), cx);
        self.reveal_and_announce(cell.row, cell.col, window, cx);
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

    /// Arms a fill-drag preview state directly (render-test / debug hook), so a static capture can
    /// show the drag preview rectangle without synthesizing a live mouse gesture. `seed` is the
    /// origin selection, `target` (⊇ seed) the previewed fill region, `axis` the decided dominant
    /// direction — exactly the state the live [`handle_mouse_move`](Self::handle_mouse_move) drag
    /// builds (`gaps_closing_7_15 §3`). The normal app never calls this (the drag machine owns the
    /// state); it exists so the pixel suite can baseline the preview overlay.
    pub fn set_fill_drag_preview(
        &mut self,
        seed: CellRange,
        target: CellRange,
        axis: FillAxis,
        cx: &mut Context<Self>,
    ) {
        self.fill_drag = Some(FillDrag {
            seed,
            target,
            axis: Some(axis),
        });
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
        // this as a selection click. Likewise a live fill drag or point-mode drag owns the pointer.
        if self.resize_drag.is_some() || self.fill_drag.is_some() || self.point_drag.is_some() {
            return;
        }
        // Any new mouse-down ends a frozen resize preview (e.g. after a degraded-mode no-op) and
        // closes an open chart context menu.
        self.resize_preview = None;
        self.chart_menu = None;

        // Focus the grid so arrow keys route here (the window arranges focus after commits).
        let handle = self.focus_handle.clone();
        window.focus(&handle, cx);

        let (local_x, local_y) = self.event_local(event.position);
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);

        let (hit, chart_hit, fill_hit) = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            let content_w = (viewport_w - row_header_w as f64).max(0.0);
            // A chart floats above the cells, so hit-test the ChartLayer first — but only in the
            // content area (a point over a header can't be over a clipped chart).
            let content_x = local_x - row_header_w;
            let content_y = local_y - COL_HEADER_H;
            let chart_hit = if content_x >= 0.0 && content_y >= 0.0 {
                let geom = AxisGeometry {
                    col_axis: &col_axis,
                    row_axis: &row_axis,
                };
                self.chart_hit_test(
                    &geom,
                    (scroll_x, scroll_y),
                    (content_w, content_h),
                    (content_x, content_y),
                )
            } else {
                None
            };
            // The fill handle sits at the selection's bottom-right corner, shown only when not
            // editing and no other drag is active (`gaps_closing_7_15 §3`) — mirror the overlay's
            // suppression guard for symmetry/defensiveness. Hit-test the same clamped square the
            // overlay draws, within ± `HANDLE_HIT_HALF` of its center.
            let fill_hit = if self.incell_open.is_none()
                && self.drag.is_none()
                && self.resize_drag.is_none()
                && self.chart_drag.is_none()
                && content_x >= 0.0
                && content_y >= 0.0
            {
                let sel_range = self.selection().range();
                let right_x = col_axis.offset_of(sel_range.end.col + 1);
                let bottom_y = row_axis.offset_of(sel_range.end.row + 1);
                let (hx, hy, _, _) =
                    fill_handle_square(right_x, bottom_y, scroll_x, scroll_y, content_w, content_h);
                let hcx = hx + HANDLE_PX / 2.0;
                let hcy = hy + HANDLE_PX / 2.0;
                (content_x - hcx).abs() <= HANDLE_HIT_HALF
                    && (content_y - hcy).abs() <= HANDLE_HIT_HALF
            } else {
                false
            };
            let hit = layout::hit_test(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
            );
            (hit, chart_hit, fill_hit)
        };
        // A chart under the pointer wins over the cell beneath it (this is the left-button handler:
        // a chart click = select + begin a move/resize drag).
        if let Some(chart_hit) = chart_hit {
            let id = match chart_hit {
                ChartHit::Handle { id, .. } | ChartHit::Body { id, .. } => id,
            };
            self.begin_chart_interaction(chart_hit, (local_x, local_y), cx);
            // Tell the owner a chart was selected (P19) so it opens the edit panel. A programmatic
            // `set_selected_chart` stays silent; only a user click emits this.
            self.events.emit(&GridEvent::ChartSelected(id), window, cx);
            return;
        }
        // A click that missed every chart deselects the current chart.
        if self.selected_chart.take().is_some() {
            cx.notify();
        }
        // A grab on the fill handle begins a fill drag (before the cell/header arms) — the seed is
        // the current selection, the axis undecided until the first move outside the seed.
        if fill_hit {
            let seed = self.selection().range();
            self.fill_drag = Some(FillDrag {
                seed,
                target: seed,
                axis: None,
            });
            cx.notify();
            return;
        }
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

    /// Begin a chart interaction from a [`ChartHit`] (P18): select the hit chart and arm a
    /// move (body hit) or resize (handle hit) drag from `grab` (grid-local mouse-down px). Cancels
    /// any cell drag (a chart interaction is never also a selection drag).
    fn begin_chart_interaction(&mut self, hit: ChartHit, grab: (f32, f32), cx: &mut Context<Self>) {
        let (id, mode, rect) = match hit {
            ChartHit::Handle { id, handle, rect } => (id, ChartDragMode::Resize(handle), rect),
            ChartHit::Body { id, rect } => (id, ChartDragMode::Move, rect),
        };
        self.selected_chart = Some(id);
        self.drag = None;
        self.chart_drag = Some(ChartDrag {
            id,
            mode,
            grab,
            start_rect: rect,
            current_rect: rect,
        });
        cx.notify();
    }

    /// Commit a chart move/resize on release (P18): map the final content-local rect back to an
    /// [`Anchor`] against the committed geometry and emit [`GridEvent::ChartAnchorChanged`] so the
    /// worker persists it. A press that never moved (`current_rect == start_rect`) is a pure select
    /// — nothing is persisted.
    fn commit_chart_drag(&mut self, drag: ChartDrag, window: &mut Window, cx: &mut Context<Self>) {
        if drag.current_rect == drag.start_rect {
            cx.notify();
            return;
        }
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let anchor = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                cx.notify();
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let geom = AxisGeometry {
                col_axis: &col_axis,
                row_axis: &row_axis,
            };
            chart_layer::rect_to_anchor(drag.current_rect, &geom, scroll_x, scroll_y)
        };
        self.events.emit(
            &GridEvent::ChartAnchorChanged {
                id: drag.id,
                anchor,
            },
            window,
            cx,
        );
        cx.notify();
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
        // Point-mode branch (`formula-point-mode/architecture.md §3.2`): while a formula edit is
        // reference-ready (or a ref is pending), a plain click INSERTS the clicked reference into
        // the formula instead of committing + moving the selection. Shift-click is excluded (its
        // range-extend selection semantics stay). Emits `InsertReference` — never `SelectionChanged`
        // — so the grid selection is untouched, and arms a `point_drag` so a drag sweeps a range.
        let point_ready = self.reference_ready || self.pending_ref;
        if point_ready && !event.modifiers.shift {
            let anchor = self.resolve_merge_anchor(row, col); // DPM.6: covered cell → merge anchor
            let a1 = CellRange::single(anchor).to_a1();
            self.events.emit(
                &GridEvent::InsertReference {
                    a1,
                    replace_pending: self.pending_ref,
                },
                window,
                cx,
            );
            self.point_drag = Some(PointDrag {
                origin: anchor,
                last_range: CellRange::single(anchor),
            });
            // Suppress gpui's built-in end-of-dispatch focus transfer to the grid root (it is gated
            // on `!default_prevented()`). NOTE: this does NOT undo the explicit grid focus already
            // taken at the top of `handle_mouse_down`; the chrome's `insert_reference` re-focuses the
            // driving editor after the synchronous splice, and that focus survives because this
            // `prevent_default` runs afterward.
            window.prevent_default();
            cx.notify();
            return;
        }
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
        // Begin a cell drag from the (kept or new) anchor so subsequent moves extend the range —
        // but NEVER on the double-click that opens the in-cell editor (BUG #2). That press belongs
        // to the editor (text selection); the editor overlay `.occlude()`s the follow-up mouse-up,
        // so a drag armed here would never be cleared and the editor's own press+drag would then
        // extend a phantom grid selection (which also emits `SelectionChanged` → the chrome closes
        // the editor, stealing focus). `handle_mouse_move` additionally refuses to extend any drag
        // while the editor is open, belt-and-braces.
        if !is_double {
            self.drag = Some(DragState {
                anchor: selection.anchor,
                mode: DragMode::Cell,
            });
        }
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
        // A live chart move/resize takes precedence — its delta is measured from the grab point in
        // grid-local px (== content px; the constant header offset cancels), so it needs no lock.
        if let Some(drag) = self.chart_drag.as_mut() {
            let dx = local_x - drag.grab.0;
            let dy = local_y - drag.grab.1;
            drag.current_rect = match drag.mode {
                ChartDragMode::Move => chart_layer::move_rect(drag.start_rect, dx, dy),
                ChartDragMode::Resize(handle) => {
                    chart_layer::resize_rect(drag.start_rect, handle, dx, dy)
                }
            };
            cx.notify();
            return;
        }
        if self.resize_drag.is_some() {
            self.update_resize(local_x, local_y, cx);
            return;
        }
        // A live fill drag updates its previewed target region (and kicks auto-scroll near an edge).
        if self.fill_drag.is_some() {
            self.update_fill_drag(local_x, local_y, window, cx);
            return;
        }
        // A live point-mode drag grows the swept reference range (checked BEFORE the `incell_open`
        // guard: a point-drag armed during an in-cell formula edit must still extend — the in-cell
        // overlay occludes only its own cell, so clicks on other cells reach the grid).
        if self.point_drag.is_some() {
            self.update_point_drag(local_x, local_y, window, cx);
            return;
        }
        // While the in-cell editor owns the pointer, the grid must not extend a selection drag: a
        // press+drag inside the editor is text selection, not a cell-range drag (BUG #2). This
        // guards any drag still live from before the editor opened; `mouse_down_cell` also refuses
        // to arm a drag on the editor-opening double-click.
        if self.incell_open.is_some() {
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
        if let Some(drag) = self.chart_drag.take() {
            self.commit_chart_drag(drag, window, cx);
            return;
        }
        if let Some(rd) = self.resize_drag.take() {
            self.commit_resize(rd, window, cx);
            return;
        }
        if let Some(fd) = self.fill_drag.take() {
            self.commit_fill_drag(fd, window, cx);
            return;
        }
        // A point-mode drag needs no commit — every move already emitted the current range as an
        // `InsertReference`, so the editor text already reflects the released rectangle. Just clear
        // the drag and stop any auto-scroll loop (epoch bump, like the selection/fill drags).
        if self.point_drag.take().is_some() {
            self.autoscroll_epoch = self.autoscroll_epoch.wrapping_add(1);
            cx.notify();
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
        // A click on the divider with no drag (or a drag that returns to the grab point) leaves the
        // width unchanged: freezing a preview and emitting a redundant `SetColumnWidths` would add a
        // no-op undo step — and, on a double-click-to-autofit (`functional_spec.md §7`), a spurious
        // step just before the autofit. Skip an unchanged resize entirely; the repaint clears the
        // transient drag visuals.
        if rd.current_px == rd.start_px {
            cx.notify();
            return;
        }
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

    /// Autofit the column(s) at a double-clicked column divider (`functional_spec.md §7`): size each
    /// to fit its content. Reuses [`resize_run_for`](Self::resize_run_for) so a divider inside a
    /// bounded multi-column header selection autofits the whole run — each column to **its own**
    /// content (D7.1) — while a lone divider autofits just that column. Each column rides the existing
    /// [`GridEvent::ResizeCommitted`] → `Command::SetColumnWidths` (undoable, xlsx round-trip, same
    /// path as drag-resize; no new worker command). A single-column autofit is one undo step;
    /// multi-column is one per column (the consequence of the per-width command carrying one width).
    ///
    /// **Whole-sheet guard.** `resize_run_for` classifies a select-all (or any run that spans every
    /// column) as a full-column run `(0, MAX_COLS-1)`. Drag-resize collapses that to ONE ranged
    /// `SetColumnWidths`, but autofit computes a **distinct** width per column, so fanning out here
    /// would emit 16,384 commands + undo steps and mass-shrink the sheet to the empty-column floor.
    /// A whole-sheet run therefore autofits only the divider's own column; bounded multi-column
    /// selections (a handful of columns) still fan out.
    fn autofit_column(&mut self, index: u32, window: &mut Window, cx: &mut Context<Self>) {
        let (start, end) = self.resize_run_for(RowOrCol::Col, index);
        let spans_all_columns = start == 0 && end >= freecell_core::limits::MAX_COLS - 1;
        let (start, end) = if spans_all_columns {
            (index, index)
        } else {
            (start, end)
        };
        for col in start..=end {
            let px = self.autofit_width_for_column(col, window);
            self.events.emit(
                &GridEvent::ResizeCommitted {
                    axis: RowOrCol::Col,
                    start: col,
                    end: col,
                    px,
                },
                window,
                cx,
            );
        }
    }

    /// The autofit width (device px) for `col` (D7.3): the widest shaped text among the column's
    /// currently **published/overscanned** cells, plus [`AUTOFIT_PADDING_PX`], clamped to
    /// `[AUTOFIT_MIN_WIDTH_PX, AUTOFIT_MAX_WIDTH_PX]`. Render-thread only — measures the values
    /// already materialized in the publication with the render thread's text system
    /// ([`measure_incell_text_width`]), each at its own resolved font (family/size/bold/italic from
    /// the resident cache), so a bold/larger cell widens the fit. A wide value scrolled beyond the
    /// overscan is not measured — a documented limitation. An empty column resolves to the floor.
    fn autofit_width_for_column(&self, col: u32, window: &mut Window) -> f32 {
        let publication = self.sources.publication.load_full();
        if publication.sheet != self.active_sheet {
            return autofit_width(0.0);
        }
        // Snapshot each published cell's text + resolved font while the caches lock is held, then
        // release it before shaping (mirrors `resolve_frame`'s "drop the lock before painting"). This
        // scans the whole publication once, keeping only this column's non-empty cells — O(published
        // cells) per call, the snapshot Vec holding at most one column's worth. Cheap overall now the
        // whole-sheet fan-out is guarded above: it runs O(published) × a small selected-column count.
        let snapshots: Vec<(String, f32, Option<SharedString>, bool, bool)> = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(self.active_sheet) else {
                return autofit_width(0.0);
            };
            publication
                .cells
                .iter()
                .filter(|pc| pc.col == col && !pc.display_text.is_empty())
                .map(|pc| {
                    let style = cache.render_style(pc.row, pc.col).copied();
                    let font_px = style.map(font_px_of).unwrap_or(CELL_FONT_PX);
                    let family = style.and_then(|s| {
                        cache
                            .font_families()
                            .get(s.font_family as usize)
                            .filter(|name| !name.is_empty())
                            .map(|name| SharedString::from(name.to_string()))
                    });
                    (
                        pc.display_text.clone(),
                        font_px,
                        family,
                        style.map(|s| s.bold).unwrap_or(false),
                        style.map(|s| s.italic).unwrap_or(false),
                    )
                })
                .collect()
        };
        let max_text_px = snapshots
            .iter()
            .fold(0.0_f32, |acc, (text, px, fam, b, i)| {
                acc.max(measure_incell_text_width(
                    text,
                    *px,
                    fam.clone(),
                    *b,
                    *i,
                    window,
                ))
            });
        autofit_width(max_text_px)
    }

    /// Autofit the row(s) at a double-clicked row divider (`functional_spec.md §5`): size each to fit
    /// its tallest populated cell — the row-height twin of [`autofit_column`](Self::autofit_column).
    /// Reuses [`resize_run_for`](Self::resize_run_for) so a divider inside a bounded multi-row header
    /// selection autofits the whole run (each row to **its own** content) while a lone divider
    /// autofits just that row. Each row rides the existing [`GridEvent::ResizeCommitted`] →
    /// `Command::SetRowHeights` (undoable, xlsx round-trip, same path as drag-resize; no new worker
    /// command), which **marks the row manual** (D5.1) so it is thereafter exempt from live wrap
    /// auto-grow — consistent with the column autofit and the manual-resize model. One undo step per
    /// row (the per-height command carries one height).
    ///
    /// **Whole-sheet guard.** A select-all is not classified as a full-**row** selection (see
    /// `resize_run_for`), so it already resolves to `(index, index)`; the explicit guard here mirrors
    /// [`autofit_column`](Self::autofit_column) and defends against any run that spans every row, so
    /// autofit never fans out to 1,048,576 per-row `SetRowHeights`. Bounded multi-row selections (a
    /// handful of rows) still fan out.
    fn autofit_row(&mut self, index: u32, window: &mut Window, cx: &mut Context<Self>) {
        let (start, end) = self.resize_run_for(RowOrCol::Row, index);
        let spans_all_rows = start == 0 && end >= freecell_core::limits::MAX_ROWS - 1;
        let (start, end) = if spans_all_rows {
            (index, index)
        } else {
            (start, end)
        };
        for row in start..=end {
            let px = self.autofit_height_for_row(row, window);
            self.events.emit(
                &GridEvent::ResizeCommitted {
                    axis: RowOrCol::Row,
                    start: row,
                    end: row,
                    px,
                },
                window,
                cx,
            );
        }
    }

    /// The autofit height (device px) for `row` (D5.2): the tallest line box among the row's currently
    /// **published/overscanned** populated cells, clamped to `[DEFAULT_ROW_HEIGHT_PX,
    /// MAX_AUTO_ROW_HEIGHT_PX]`. Render-thread only — measures the values already materialized in the
    /// publication with the render thread's text system, each at its own resolved font and **own
    /// column width** (a wrap-on cell soft-wraps; a wrap-off cell counts explicit `\n` segments). A
    /// value scrolled beyond the overscan is not measured — a documented limitation mirroring
    /// [`autofit_width_for_column`](Self::autofit_width_for_column). An empty row resolves to the
    /// default height.
    fn autofit_height_for_row(&self, row: u32, window: &mut Window) -> f32 {
        let publication = self.sources.publication.load_full();
        if publication.sheet != self.active_sheet {
            return DEFAULT_ROW_HEIGHT_PX;
        }
        // Snapshot each published cell's text + resolved font + column width while the caches lock is
        // held, then release it before shaping (mirrors `autofit_width_for_column`). Keeps only this
        // row's non-empty cells — O(published cells) per call, the snapshot Vec holding at most one
        // row's worth.
        let snapshots: Vec<AutofitRowCell> = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(self.active_sheet) else {
                return DEFAULT_ROW_HEIGHT_PX;
            };
            publication
                .cells
                .iter()
                .filter(|pc| pc.row == row && !pc.display_text.is_empty())
                .map(|pc| {
                    let style = cache.render_style(pc.row, pc.col).copied();
                    let font_px = style.map(font_px_of).unwrap_or(CELL_FONT_PX);
                    let font_family = style.and_then(|s| {
                        cache
                            .font_families()
                            .get(s.font_family as usize)
                            .filter(|name| !name.is_empty())
                            .map(|name| SharedString::from(name.to_string()))
                    });
                    AutofitRowCell {
                        text: SharedString::from(pc.display_text.clone()),
                        col_w: cache.col_width(pc.col),
                        font_px,
                        bold: style.map(|s| s.bold).unwrap_or(false),
                        italic: style.map(|s| s.italic).unwrap_or(false),
                        font_family,
                        wrap: style.map(|s| s.wrap).unwrap_or(false),
                    }
                })
                .collect()
        };
        Self::measure_row_height(&snapshots, window)
    }

    /// The autofit height a set of a row's populated cells needs (`functional_spec.md §5.2`): the
    /// **max** over the cells of each cell's own line box ([`cell_line_box_height`]), clamped to
    /// `[DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX]`. A **wrap-on** cell soft-wraps at its column
    /// width (gpui's real `LineWrapper`, summed over explicit-`\n` segments — same as
    /// [`measure_wrap_height`](Self::measure_wrap_height)); a **wrap-off** cell counts only its
    /// explicit `\n` segments (one visual line each, no soft-wrap). An empty row (no cells) resolves
    /// to the default.
    fn measure_row_height(cells: &[AutofitRowCell], window: &mut Window) -> f32 {
        let mut needed = DEFAULT_ROW_HEIGHT_PX;
        for c in cells {
            let lines: u32 = if c.wrap {
                let avail = (c.col_w - 2.0 * CELL_H_PAD).max(1.0);
                let family = c
                    .font_family
                    .clone()
                    .unwrap_or_else(|| SharedString::from(GRID_FONT_FAMILY));
                let mut cell_font = font(family);
                if c.bold {
                    cell_font = cell_font.bold();
                }
                if c.italic {
                    cell_font = cell_font.italic();
                }
                let mut wrapper = window.text_system().line_wrapper(cell_font, px(c.font_px));
                c.text
                    .split('\n')
                    .map(|segment| {
                        1 + wrapper
                            .wrap_line(&[LineFragment::text(segment)], px(avail))
                            .count() as u32
                    })
                    .sum()
            } else {
                c.text.split('\n').count() as u32
            };
            needed = needed.max(cell_line_box_height(lines, c.font_px));
        }
        needed.clamp(DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX)
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
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        // Hit-test the ChartLayer + headers + read the merge list + hidden sets under one lock.
        let (hit, chart_hit, merges, hidden_rows, hidden_cols, dims) = {
            let caches = self.sources.caches.read();
            let Some(cache) = caches.get(active) else {
                return;
            };
            let (row_axis, col_axis) = cache.axes();
            let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
            let content_x = local_x - row_header_w;
            let content_y = local_y - COL_HEADER_H;
            let chart_hit = if content_x >= 0.0 && content_y >= 0.0 {
                let content_w = (viewport_w - row_header_w as f64).max(0.0);
                let geom = AxisGeometry {
                    col_axis: &col_axis,
                    row_axis: &row_axis,
                };
                self.chart_hit_test(
                    &geom,
                    (scroll_x, scroll_y),
                    (content_w, content_h),
                    (content_x, content_y),
                )
            } else {
                None
            };
            let hit = layout::hit_test(
                local_x,
                local_y,
                row_header_w,
                scroll_x,
                scroll_y,
                &row_axis,
                &col_axis,
            );
            (
                hit,
                chart_hit,
                cache.merges().to_vec(),
                cache.hidden_rows().clone(),
                cache.hidden_cols().clone(),
                cache.dims(),
            )
        };
        // A right-click on a chart selects it and opens the "Delete chart" context menu (P18) — the
        // alternate delete affordance. Any chart hit (body or a handle of the already-selected
        // chart) targets that chart.
        if let Some(chart_hit) = chart_hit {
            let id = match chart_hit {
                ChartHit::Handle { id, .. } | ChartHit::Body { id, .. } => id,
            };
            self.selected_chart = Some(id);
            self.header_menu = None;
            self.cell_menu = None;
            self.chart_menu = Some(ChartMenu {
                id,
                x: local_x,
                y: local_y,
            });
            cx.notify();
            return;
        }
        // A right-click off any chart dismisses an open chart menu (but continues to the header menu).
        if self.chart_menu.take().is_some() {
            cx.notify();
        }
        let (axis, index) = match hit {
            GridHit::ColHeader { col } => (RowOrCol::Col, col),
            GridHit::RowHeader { row } => (RowOrCol::Row, row),
            GridHit::Cell { row, col } => {
                // A right-click on the cell body opens the cell-area context menu
                // (`functional_spec.md §2`), adjusting the selection first (move-if-outside).
                self.open_cell_menu(
                    CellRef::new(row, col),
                    local_x,
                    local_y,
                    &merges,
                    window,
                    cx,
                );
                return;
            }
            GridHit::Corner => {
                // A right-click on the select-all corner has no menu; dismiss any open one. Take
                // both unconditionally — a `||` would short-circuit the second `take` (fragile even
                // though the two menus are mutually exclusive today).
                let had_header = self.header_menu.take().is_some();
                let had_cell = self.cell_menu.take().is_some();
                if had_header || had_cell {
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
        let (hidden_set, total) = match axis {
            RowOrCol::Row => (&hidden_rows, dims.0),
            RowOrCol::Col => (&hidden_cols, dims.1),
        };
        let (hide_blocked, unhide_run, hidden_in_run) = hide_unhide_flags(run, hidden_set, total);
        self.cell_menu = None;
        self.header_menu = Some(HeaderMenu {
            axis,
            run,
            x: local_x,
            y: local_y,
            insert_before_blocked: before,
            insert_after_blocked: after,
            delete_blocked: delete,
            hide_blocked,
            unhide_run,
            hidden_in_run,
        });
        cx.notify();
    }

    /// Closes the header context menu (click-away / Escape / after an item runs).
    fn close_header_menu(&mut self, cx: &mut Context<Self>) {
        if self.header_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Opens the cell-area right-click context menu over cell `(row, col)` at grid-local `(x, y)`
    /// (`functional_spec.md §2`). Excel selection semantics: a right-click **outside** the current
    /// selection first collapses it to the clicked cell; a click **inside** keeps the (possibly
    /// multi-cell) selection so the menu's ops span it. The insert/delete items reuse the header
    /// menu's per-axis merge guard over the selection's row/column span; `paste_enabled` reflects
    /// whether the system clipboard currently holds text (gating both Paste and Paste-values).
    fn open_cell_menu(
        &mut self,
        cell: CellRef,
        x: f32,
        y: f32,
        merges: &[CellRange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection().range().contains(cell) {
            self.set_selection_and_emit(SelectionModel::single(cell), window, cx);
        }
        let range = self.selection().range();
        let (insert_row_above_blocked, insert_row_below_blocked, delete_rows_blocked) =
            merge_block_flags(RowOrCol::Row, (range.start.row, range.end.row), merges);
        let (insert_col_left_blocked, insert_col_right_blocked, delete_cols_blocked) =
            merge_block_flags(RowOrCol::Col, (range.start.col, range.end.col), merges);
        let paste_enabled = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| !text.is_empty());
        self.header_menu = None;
        self.cell_menu = Some(CellMenu {
            x,
            y,
            range,
            paste_enabled,
            insert_row_above_blocked,
            insert_row_below_blocked,
            delete_rows_blocked,
            insert_col_left_blocked,
            insert_col_right_blocked,
            delete_cols_blocked,
        });
        cx.notify();
    }

    /// Closes the cell-area context menu (click-away / Escape / after an item runs).
    fn close_cell_menu(&mut self, cx: &mut Context<Self>) {
        if self.cell_menu.take().is_some() {
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

    /// Advance a live fill drag (`gaps_closing_7_15 §3`): map the pointer to a cell, recompute the
    /// previewed target region, and kick edge auto-scroll so a fill can run past the visible area.
    fn update_fill_drag(
        &mut self,
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
        self.set_fill_target_from_cell(cell);
        self.maybe_start_autoscroll(window, cx);
        cx.notify();
    }

    /// Recompute the fill drag's `target` + dominant `axis` for a pointer over `cell` (pure; no
    /// window/cx — shared by the mouse-move and the auto-scroll tick). The axis is the direction the
    /// pointer has left the seed **farther** along, and is **sticky** once set (kept until the
    /// pointer returns inside the seed) so a gesture never flips axis mid-drag (D3.1). Inside the
    /// seed → axis cleared, target collapses back to the seed.
    fn set_fill_target_from_cell(&mut self, cell: CellRef) {
        let Some(drag) = self.fill_drag.as_mut() else {
            return;
        };
        let seed = drag.seed;
        // How far `cell` lies outside the seed along each axis (0 = within the seed's span).
        let vext = if cell.row > seed.end.row {
            cell.row - seed.end.row
        } else {
            seed.start.row.saturating_sub(cell.row)
        };
        let hext = if cell.col > seed.end.col {
            cell.col - seed.end.col
        } else {
            seed.start.col.saturating_sub(cell.col)
        };
        if vext == 0 && hext == 0 {
            drag.axis = None;
            drag.target = seed;
            return;
        }
        let axis = drag.axis.unwrap_or(if vext >= hext {
            FillAxis::Vertical
        } else {
            FillAxis::Horizontal
        });
        drag.axis = Some(axis);
        drag.target = match axis {
            // Vertical: columns pinned to the seed, rows extended to include `cell` (down or up).
            FillAxis::Vertical => {
                let top = seed.start.row.min(cell.row);
                let bottom = seed.end.row.max(cell.row);
                CellRange::new(
                    CellRef::new(top, seed.start.col),
                    CellRef::new(bottom, seed.end.col),
                )
            }
            // Horizontal: rows pinned to the seed, columns extended to include `cell` (right/left).
            FillAxis::Horizontal => {
                let left = seed.start.col.min(cell.col);
                let right = seed.end.col.max(cell.col);
                CellRange::new(
                    CellRef::new(seed.start.row, left),
                    CellRef::new(seed.end.row, right),
                )
            }
        };
    }

    /// Update a live point-mode drag from a grid-local pointer (`formula-point-mode §2`): resolve the
    /// hovered cell, grow the swept range, and (past a viewport edge) kick the auto-scroll loop.
    fn update_point_drag(
        &mut self,
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
        self.set_point_target_from_cell(cell, window, cx);
        self.maybe_start_autoscroll(window, cx);
        cx.notify();
    }

    /// Recompute the point-drag's swept range for a pointer over `cell` and, when it changed, re-emit
    /// it as a reference (`formula-point-mode/architecture.md §3.3`). The range is `origin..cell`
    /// normalized then **expanded for merges** (DPM.6 — a swept rect touching a merge grows to the
    /// whole span). Shared by the mouse-move and the auto-scroll tick. Every emit is
    /// `replace_pending: true` — the grid's own prior insert is the pending ref, so the drag re-aims
    /// locally without waiting on the deferred `EditState` round-trip (`architecture.md §10`). A
    /// release on the origin cell keeps `single(origin)` → a single-cell ref (no degenerate range).
    fn set_point_target_from_cell(
        &mut self,
        cell: CellRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.point_drag.as_ref().map(|pd| pd.origin) else {
            return;
        };
        let expanded = self.expand_range_for_merges(CellRange::new(origin, cell));
        let changed = match self.point_drag.as_mut() {
            Some(pd) if pd.last_range != expanded => {
                pd.last_range = expanded;
                true
            }
            _ => false,
        };
        if changed {
            self.events.emit(
                &GridEvent::InsertReference {
                    a1: expanded.to_a1(),
                    replace_pending: true,
                },
                window,
                cx,
            );
        }
    }

    /// Resolve a clicked `(row, col)` to the reference to insert (DPM.6, Q6): the top-left **anchor**
    /// of the merge covering it, or the cell itself when uncovered. Reads the same `cache.merges()`
    /// the selection/render path uses — one source of truth for merge geometry.
    fn resolve_merge_anchor(&self, row: u32, col: u32) -> CellRef {
        let caches = self.sources.caches.read();
        match caches.get(self.active_sheet) {
            Some(cache) => resolve_merge_anchor_in(row, col, cache.merges()),
            None => CellRef::new(row, col),
        }
    }

    /// Expand a swept point-drag `range` so it never bisects a merge (DPM.6, Q6): union it with every
    /// merge it touches, over the same `cache.merges()` list. Identity when no cache is resident.
    fn expand_range_for_merges(&self, range: CellRange) -> CellRange {
        let caches = self.sources.caches.read();
        match caches.get(self.active_sheet) {
            Some(cache) => expand_range_for_merges_in(range, cache.merges()),
            None => range,
        }
    }

    /// Commit a fill drag on release (`gaps_closing_7_15 §3`): stop any auto-scroll loop, then — if
    /// the target actually extended past the seed along a decided axis — emit
    /// [`GridEvent::FillDrag`] and expand the selection to the filled region (Excel behavior). A
    /// release onto the seed, or with no axis, or an inward target (D3.3) is a no-op (no event, no
    /// selection change).
    fn commit_fill_drag(&mut self, drag: FillDrag, window: &mut Window, cx: &mut Context<Self>) {
        // Stop the auto-scroll loop the same way the selection drag does (epoch bump).
        self.autoscroll_epoch = self.autoscroll_epoch.wrapping_add(1);
        let FillDrag { seed, target, axis } = drag;
        // A superset target that differs from the seed is a real outward fill; anything else (no
        // axis, unchanged, or — defensively — a non-superset inward target) does nothing.
        let is_outward = target != seed
            && target.start.row <= seed.start.row
            && target.start.col <= seed.start.col
            && target.end.row >= seed.end.row
            && target.end.col >= seed.end.col;
        let Some(axis) = axis.filter(|_| is_outward) else {
            cx.notify();
            return;
        };
        self.events
            .emit(&GridEvent::FillDrag { seed, target, axis }, window, cx);
        // Expand the selection to seed∪target (= target, since target ⊇ seed).
        self.set_selection(
            SelectionModel {
                anchor: target.start,
                active: target.end,
            },
            cx,
        );
        cx.notify();
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
        // Fires for a selection drag, a fill drag (`gaps_closing_7_15 §3`), or a point-mode drag
        // (`formula-point-mode §2`) past a viewport edge.
        if self.autoscrolling
            || (self.drag.is_none() && self.fill_drag.is_none() && self.point_drag.is_none())
        {
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
                    if this.autoscroll_epoch != epoch
                        || (this.drag.is_none()
                            && this.fill_drag.is_none()
                            && this.point_drag.is_none())
                    {
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

    /// One auto-scroll frame: apply the fixed edge step (clamped), re-extend the selection (or the
    /// fill-drag target) to the hovered cell, and announce a debounced `ViewportChanged`. Returns
    /// whether to keep looping (`false` once the pointer returns inside the content).
    fn autoscroll_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.drag.is_none() && self.fill_drag.is_none() && self.point_drag.is_none() {
            self.autoscrolling = false;
            return false;
        }
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
        if let Some(drag) = self.drag {
            let selection = SelectionModel {
                anchor: drag.anchor,
                active: cell,
            };
            if *self.selection() != selection {
                self.set_selection_and_emit(selection, window, cx);
                changed = true;
            }
        } else if self.fill_drag.is_some() {
            // A fill drag auto-scrolls too: re-extend its previewed target to the hovered cell.
            self.set_fill_target_from_cell(cell);
            changed = true;
        } else if self.point_drag.is_some() {
            // A point-mode drag auto-scrolls too: re-emit the swept reference for the hovered cell.
            self.set_point_target_from_cell(cell, window, cx);
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

        // Chart selection intercepts (P18, `ui_design §3.2`): a selected chart owns Escape (cancel a
        // drag / clear the selection + menu) and Delete/Backspace (delete the chart) — handled here,
        // BEFORE the `ClearCells` mapping below, so deleting a selected chart never also clears the
        // cell selection's contents.
        if self.chart_drag.is_some() || self.selected_chart.is_some() || self.chart_menu.is_some() {
            match key {
                "escape" => {
                    self.chart_drag = None;
                    self.selected_chart = None;
                    self.chart_menu = None;
                    cx.notify();
                    return;
                }
                "delete" | "backspace" if self.chart_drag.is_none() && !modifiers.modified() => {
                    if let Some(id) = self.selected_chart.take() {
                        self.chart_menu = None;
                        self.events
                            .emit(&GridEvent::ChartDeleted { id }, window, cx);
                        cx.notify();
                    }
                    return;
                }
                _ => {}
            }
        }

        // Escape cancels a live resize (no command sent) — the preview clears, geometry reverts
        // (`functional_spec.md §5.1`) — clears a lingering frozen preview (e.g. a degraded-mode
        // post-commit preview), or closes the header context menu.
        if key == "escape"
            && (self.resize_drag.is_some()
                || self.resize_preview.is_some()
                || self.header_menu.is_some()
                || self.cell_menu.is_some())
        {
            self.resize_drag = None;
            self.resize_preview = None;
            self.header_menu = None;
            self.cell_menu = None;
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
            GridKeyCommand::PasteValues => self.events.emit(&GridEvent::PasteValues, window, cx),
            GridKeyCommand::SelectAll => self.select_all(window, cx),
            // Fill Down / Right carry the current selection range; the window forwards them to the
            // worker (`functional_spec.md §3`).
            GridKeyCommand::FillDown => {
                let range = self.selection().range();
                self.events.emit(&GridEvent::FillDown(range), window, cx);
            }
            GridKeyCommand::FillRight => {
                let range = self.selection().range();
                self.events.emit(&GridEvent::FillRight(range), window, cx);
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
        // ⌘+arrow / ⌘⇧+arrow use edge-of-**data** targets, whose occupancy lives in the engine past
        // the published viewport — route only these two to the async worker query (`functional_spec.md
        // §4`, D4.1 Option A). The window applies the `EdgeResolved` reply. Every other motion stays
        // synchronous through `apply_motion` below.
        let sel = *self.selection();
        if let Some((dir, extend)) = match motion {
            Motion::JumpEdge(dir) => Some((dir, false)),
            Motion::ExtendEdge(dir) => Some((dir, true)),
            _ => None,
        } {
            self.events.emit(
                &GridEvent::ResolveEdge {
                    from: sel.active,
                    anchor: sel.anchor,
                    dir,
                    extend,
                },
                window,
                cx,
            );
            return;
        }
        let selection = apply_motion(sel, motion, dims);
        if *self.selection() != selection {
            self.set_selection_and_emit(selection, window, cx);
            self.reveal_and_announce(selection.active.row, selection.active.col, window, cx);
        }
    }

    /// Applies a worker-resolved selection (the async ⌘+arrow edge jump, `functional_spec.md §4`) and
    /// reveals its active cell. Like [`select_and_reveal`](Self::select_and_reveal) but keeps the range
    /// (so ⌘⇧+arrow's extended selection survives) and does **not** emit `SelectionChanged` — the
    /// window folds the chrome + shared state directly on the `EdgeResolved` reply, mirroring the paste
    /// path (avoids a double fold).
    pub fn set_selection_and_reveal(
        &mut self,
        selection: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_selection(selection, cx);
        self.reveal_and_announce(selection.active.row, selection.active.col, window, cx);
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
        // Deferred text-spill paints (`functional_spec.md §2`); typically empty. Painted after
        // the cell + border layers so spilled text sits over the neighbours' fills/gridlines.
        let mut spill_plans: Vec<SpillPlan> = Vec::new();
        // Reset the wrap-on-cell buffer the post-layout auto-grow pass reads (`functional_spec.md
        // §3`). Only the real render path fills it; the perf harness (`timing` set) leaves it empty
        // so its measurement is untouched.
        self.wrap_cells.clear();

        for r in frame.rows.clone() {
            for c in frame.cols.clone() {
                let (x, y, w, h) = cell_rect(r, c, frame);
                let style = self.visible_styles.get(&(r, c)).copied();
                let fill_color = style.and_then(|s| s.fill);
                let fill = fill_color.map(to_rgba).unwrap_or_else(|| rgb(CELL_BG));
                // 8a: within a contiguous same-fill block the interior gridlines are hidden so the
                // block reads as one solid rectangle (the Excel look). A *filled* cell drops the
                // gridline it shares with a right / bottom neighbour that resolves to the SAME fill;
                // unfilled cells keep every gridline, and the block's outer boundary (a different
                // fill, an unfilled cell, or an off-viewport neighbour — which reads as absent here)
                // still draws. Explicit cell borders are a separate later pass and are unaffected.
                let same_fill = |nr: u32, nc: u32| {
                    fill_color.is_some()
                        && self.visible_styles.get(&(nr, nc)).and_then(|s| s.fill) == fill_color
                };
                let skip_right_gridline = same_fill(r, c + 1);
                let skip_bottom_gridline = same_fill(r + 1, c);
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

                // Auto-grow (`functional_spec.md §3`): capture each committed **wrap-on**, non-empty
                // cell so the post-layout pass (which has the render thread's text system) can
                // measure its wrapped height at the column width `w`. Mirrored (being-edited) cells
                // carry `attr_style: None`, so the `.wrap` gate skips them; only the real render path
                // collects (`timing.is_none()`) — the perf harness stays allocation-light.
                if timing.is_none() {
                    if let Some(s) = attr_style {
                        if s.wrap && !text.is_empty() {
                            self.wrap_cells.push(WrapCell {
                                row: r,
                                text: text.clone().into(),
                                font_px: font_px_of(s),
                                bold: s.bold,
                                italic: s.italic,
                                font_family: font_family.clone(),
                                col_w: w,
                            });
                        }
                    }
                }

                // Text spill (`functional_spec.md §2`): a committed, wrap-off TEXT cell whose text
                // overflows its column spills over empty neighbours in its alignment direction.
                // Numbers/dates/bools/errors and mirrored (being-edited) cells never spill; a
                // fitting cell falls through to the unchanged render path (pixels untouched).
                let spill = if covers_active
                    && kind == CellKind::Text
                    && !text.is_empty()
                    && attr_style.is_none_or(|s| !s.wrap)
                    && self.mirror_text_for(CellRef::new(r, c)).is_none()
                {
                    let align = attr_style
                        .and_then(|s| s.h_align)
                        .unwrap_or_else(|| kind.default_align());
                    let font_px = attr_style.map(font_px_of).unwrap_or(CELL_FONT_PX);
                    if layout::text_overflows_column(&text, font_px, w, CELL_H_PAD) {
                        let span = layout::spill_span(
                            c,
                            layout::spill_direction(align),
                            frame.cols.start,
                            frame.cols.end.saturating_sub(1),
                            |nc| self.neighbor_occupancy(r, nc, &publication),
                        );
                        span.spills(c).then_some((span, align))
                    } else {
                        None
                    }
                } else {
                    None
                };

                match spill {
                    // A spilling cell suppresses its own clipped text (the spill element repaints
                    // the full run — no double-paint); the origin div still draws fill + gridlines.
                    Some((span, align)) => {
                        content_children.push(cell_element(
                            x,
                            y,
                            w,
                            h,
                            fill,
                            String::new(),
                            text_color,
                            kind,
                            attr_style,
                            font_family.clone(),
                            skip_right_gridline,
                            skip_bottom_gridline,
                        ));
                        spill_plans.push(SpillPlan {
                            row: r,
                            span,
                            text,
                            text_color,
                            align,
                            style: attr_style,
                            font_family,
                        });
                    }
                    None => content_children.push(cell_element(
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
                        skip_right_gridline,
                        skip_bottom_gridline,
                    )),
                }
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
                    push_vertical_edge(&mut content_children, x + w, y, h, edge);
                }
                // Bottom edge (shared with r+1) — always drawn by this (upper) cell.
                if let Some(edge) = effective_edge(spec.bottom, self.border_spec_at(r + 1, c).top) {
                    push_horizontal_edge(&mut content_children, x, y + h, w, edge);
                }
                // Left edge: only when the left neighbour won't draw it as its right edge.
                if self.no_left_owner(r, c, frame) {
                    let left_nbr = if c == 0 {
                        BorderSpec::NONE
                    } else {
                        self.border_spec_at(r, c - 1)
                    };
                    if let Some(edge) = effective_edge(spec.left, left_nbr.right) {
                        push_vertical_edge(&mut content_children, x, y, h, edge);
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
                        push_horizontal_edge(&mut content_children, x, y, w, edge);
                    }
                }
            }
        }

        // ---- Text spill (`functional_spec.md §2`): paint each overflowing text cell's content as
        // a separate positioned element over its empty-neighbour run, ABOVE the cell fills /
        // gridlines / borders (so the text reads continuously across gridlines) but still inside
        // the content clip wrapper — it never escapes into the headers. The origin cell already
        // suppressed its own clipped text, so there is no double-paint.
        for plan in spill_plans {
            let (sx, sy, sw, sh) = span_rect(
                plan.row..plan.row + 1,
                plan.span.left..plan.span.right + 1,
                frame,
            );
            content_children.push(spill_element(
                sx,
                sy,
                sw,
                sh,
                plan.text,
                plan.text_color,
                plan.align,
                plan.style,
                plan.font_family,
            ));
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

        // Formula reference highlights (`formula-point-mode/architecture.md §4.1`): while a formula
        // edit is open, each distinct same-sheet reference already typed is drawn as a rich colored
        // fill + border in its assigned palette slot. Painted ABOVE the selection overlay and BELOW
        // the fill handle / in-cell overlay, clipped to the visible frame exactly like the selection
        // overlay — an off-screen ref clips to nothing (no visible highlight, by construction), and
        // cross-sheet refs are excluded upstream (only the same-sheet subset reaches the grid). The
        // grid renders on the white `CELL_BG` regardless of OS appearance, so the light palette
        // variant is used (`ref_slot_border(_, false)`); the `is_dark` seam stays for the future
        // theme-aware / in-editor styling control.
        for (target, slot) in &self.ref_highlights {
            let rows =
                target.start.row.max(frame.rows.start)..(target.end.row + 1).min(frame.rows.end);
            let cols =
                target.start.col.max(frame.cols.start)..(target.end.col + 1).min(frame.cols.end);
            if rows.start >= rows.end || cols.start >= cols.end {
                continue;
            }
            let (x, y, w, h) = span_rect(rows, cols, frame);
            let color = ref_slot_border(*slot, false);
            content_children.push(
                rect_div(x, y, w, h)
                    .bg(rgb(color).opacity(REF_HIGHLIGHT_FILL_ALPHA))
                    .border_2()
                    .border_color(rgb(color))
                    .into_any_element(),
            );
        }

        // Point-mode drag preview (`formula-point-mode/functional_spec.md §2`): while a point drag is
        // live, draw its swept range as a 2px **dashed** indigo marquee — visually distinct from the
        // solid-blue selection rectangle and the colored reference highlights (three overlays can be
        // on screen at once). Clipped to the visible frame like the selection overlay; no fill, no
        // handles (DPM.7). The editor text already tracks the range (each move emitted an insert).
        if let Some(pd) = self.point_drag {
            let t = pd.last_range;
            let rows = t.start.row.max(frame.rows.start)..(t.end.row + 1).min(frame.rows.end);
            let cols = t.start.col.max(frame.cols.start)..(t.end.col + 1).min(frame.cols.end);
            if rows.start < rows.end && cols.start < cols.end {
                let (x, y, w, h) = span_rect(rows, cols, frame);
                content_children.push(
                    rect_div(x, y, w, h)
                        .border_2()
                        .border_dashed()
                        .border_color(rgb(POINT_PREVIEW_BORDER))
                        .into_any_element(),
                );
            }
        }

        // Fill handle + drag preview (`gaps_closing_7_15 §3`). While a fill drag is live, draw its
        // previewed target region (a 2px accent border, like the range border); otherwise — when not
        // editing and no other drag is active — draw the grabbable handle square at the selection's
        // bottom-right corner, clamped into the viewport (D3.4).
        if let Some(fd) = self.fill_drag {
            if fd.axis.is_some() {
                let t = fd.target;
                let (x, y, w, h) = span_rect(
                    t.start.row..t.end.row + 1,
                    t.start.col..t.end.col + 1,
                    frame,
                );
                content_children.push(
                    rect_div(x, y, w, h)
                        .border_2()
                        .border_color(rgb(ACCENT))
                        .into_any_element(),
                );
            }
        } else if self.incell_open.is_none()
            && self.drag.is_none()
            && self.resize_drag.is_none()
            && self.chart_drag.is_none()
        {
            let right_x = frame.col_offset(range.end.col + 1);
            let bottom_y = frame.row_offset(range.end.row + 1);
            let (hx, hy, hw, hh) = fill_handle_square(
                right_x,
                bottom_y,
                frame.scroll_x,
                frame.scroll_y,
                frame.content_w,
                frame.content_h,
            );
            content_children.push(
                rect_div(hx, hy, hw, hh)
                    .bg(rgb(CELL_BG))
                    .border_1()
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

        // ---- ChartLayer: charts painted OVER the cells, BELOW the header/chrome layers, clipped
        // to the content area (P8, `charts/architecture.md §4.2`, `ui_design.md §1`). Each chart's
        // `twoCellAnchor` maps to a content-local pixel rect through the same frame geometry the
        // cells use (so scroll/zoom are free); off-screen charts are culled; the dispatch draws the
        // plot (+ a corner badge when Degraded) or the placeholder (Unsupported). A chart being
        // dragged (P18) paints at its live drag rect; the selected chart gets a selection outline +
        // resize handles overlaid. When the active sheet has no charts, nothing is pushed.
        if let Some(layer) = self.charts.get(&self.active_sheet) {
            // The per-frame scan touches only the tiny `placements` (P11 "off-screen free", via
            // [`on_screen_charts`]): off-screen charts are culled without ever borrowing their heavy
            // render `Chart`; the shared `specs[i].chart()` is materialized only for the on-screen few
            // (re-materialized when a chart scrolls back into view).
            let visible = Self::visible_charts(
                layer,
                frame,
                frame.scroll_x,
                frame.scroll_y,
                frame.content_w,
                frame.content_h,
            );
            let mut chart_children: Vec<AnyElement> = Vec::with_capacity(visible.len());
            // The paint rect of the selected chart (for the outline + handles), if it is on-screen.
            let mut selected_rect: Option<ChartRect> = None;
            for (i, anchored) in visible {
                let id = layer.specs[i].id;
                // A chart being dragged paints at its live preview rect; every other at its anchor.
                let rect = match self.chart_drag {
                    Some(drag) if drag.id == id => drag.current_rect,
                    _ => anchored,
                };
                if self.selected_chart == Some(id) {
                    selected_rect = Some(rect);
                }
                chart_children.push(
                    div()
                        .absolute()
                        .left(px(rect.x))
                        .top(px(rect.y))
                        .w(px(rect.w))
                        .h(px(rect.h))
                        .child(crate::chart::in_grid_chart_element(
                            layer.specs[i].chart(),
                            layer.specs[i].title(),
                            layer.placements[i].fidelity,
                        ))
                        .into_any_element(),
                );
            }
            // Selection chrome (P18): an accent outline + eight resize-handle squares over the
            // selected chart's rect. New ChartLayer chrome → the `grid_chart_selected` baseline.
            if let Some(rect) = selected_rect {
                chart_children.push(
                    rect_div(rect.x, rect.y, rect.w, rect.h)
                        .border_2()
                        .border_color(rgb(ACCENT))
                        .into_any_element(),
                );
                for handle in Handle::ALL {
                    let sq = handle.square(rect);
                    chart_children.push(
                        rect_div(sq.x, sq.y, sq.w, sq.h)
                            .bg(rgb(CELL_BG))
                            .border_1()
                            .border_color(rgb(ACCENT))
                            .into_any_element(),
                    );
                }
            }
            if !chart_children.is_empty() {
                root_children.push(
                    div()
                        .absolute()
                        .left(px(frame.row_header_w))
                        .top(px(COL_HEADER_H))
                        .w(px(frame.content_w as f32))
                        .h(px(frame.content_h as f32))
                        .overflow_hidden()
                        .children(chart_children)
                        .into_any_element(),
                );
            }
        }

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

    /// The wrap-driven row-height a set of same-row wrap-on cells needs (`functional_spec.md §3.2`):
    /// the **max** over the cells of each cell's own wrapped height. A cell's height is
    /// `lines * line_height + vpad`, where `lines` is its soft-wrap line count at the column width
    /// (from gpui's real `LineWrapper`, so the grown row fits the text the grid actually paints,
    /// summed over its explicit-`\n` segments) and `line_height` is gpui's default **`phi`** line box
    /// (`round(1.618 * font_px)`, matching `Style::line_height` — NOT a made-up factor, so the row
    /// fits the rendered lines exactly). `vpad` is the slack a single default line leaves in the
    /// default row height, so a one-line default cell measures to the default. Clamped to
    /// `[default, MAX_AUTO_ROW_HEIGHT_PX]` (content beyond the cap clips within the cell).
    fn measure_wrap_height(cells: &[&WrapCell], window: &mut Window) -> f32 {
        let mut needed = DEFAULT_ROW_HEIGHT_PX;
        for wc in cells {
            let avail = (wc.col_w - 2.0 * CELL_H_PAD).max(1.0);
            let family = wc
                .font_family
                .clone()
                .unwrap_or_else(|| SharedString::from(GRID_FONT_FAMILY));
            let mut cell_font = font(family);
            if wc.bold {
                cell_font = cell_font.bold();
            }
            if wc.italic {
                cell_font = cell_font.italic();
            }
            let mut wrapper = window.text_system().line_wrapper(cell_font, px(wc.font_px));
            // Each explicit-newline segment wraps independently; visual lines = boundaries + 1.
            let lines: u32 = wc
                .text
                .split('\n')
                .map(|segment| {
                    1 + wrapper
                        .wrap_line(&[LineFragment::text(segment)], px(avail))
                        .count() as u32
                })
                .sum();
            needed = needed.max(cell_line_box_height(lines, wc.font_px));
        }
        needed.clamp(DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX)
    }

    /// A per-row signature of the wrap inputs (content / font / column width) that drive its wrapped
    /// height — deliberately **excluding the row height** so a height-only republish leaves it
    /// unchanged (the convergence guard, `architecture.md §3.2`).
    fn wrap_row_signature(cells: &[&WrapCell]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for wc in cells {
            wc.text.hash(&mut hasher);
            wc.font_px.to_bits().hash(&mut hasher);
            wc.bold.hash(&mut hasher);
            wc.italic.hash(&mut hasher);
            wc.font_family.hash(&mut hasher);
            wc.col_w.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The post-layout wrap auto-grow pass (`functional_spec.md §3`, run from [`Render::render`]):
    /// group this frame's visible wrap-on cells by row, measure the rows whose wrap **inputs**
    /// changed since last frame (plus rows that stopped wrapping → shrink to default), and emit one
    /// [`GridEvent::AutoGrowRows`] with the measured heights. Clearing the dirty rows by storing the
    /// fresh signature is what makes the loop converge: after the worker applies the height and
    /// republishes, the re-render measures the same inputs → same signature → no re-emit.
    fn run_autogrow(&mut self, frame: &Frame, window: &mut Window, cx: &mut Context<Self>) {
        // Zero-cost early-out for the overwhelmingly common case (no wrap-on cells anywhere in the
        // viewport, and none previously grown): the whole pass is skipped, so a normal sheet pays
        // nothing per frame. When wrap-on cells ARE visible the pass is O(visible wrap cells): it
        // re-hashes each wrapped cell's text every frame to detect input changes (measurement itself
        // is gated on that signature, so it only runs on a real change). Re-hashing short, rare wrap
        // text on idle repaints is cheap; a coarser gate was avoided because a style-only edit
        // (font/wrap-toggle) doesn't bump the publication generation, so it would risk missing a
        // genuine change. Accepted tradeoff.
        if self.wrap_cells.is_empty() && self.wrap_sig.is_empty() {
            return;
        }
        // Group the frame's wrap-on cells by row (indices into `self.wrap_cells`).
        let mut by_row: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, wc) in self.wrap_cells.iter().enumerate() {
            by_row.entry(wc.row).or_default().push(i);
        }

        let mut heights: Vec<(u32, f32)> = Vec::new();
        let mut fresh_sigs: HashMap<u32, u64> = HashMap::with_capacity(by_row.len());
        for (row, idxs) in &by_row {
            let cells: Vec<&WrapCell> = idxs.iter().map(|&i| &self.wrap_cells[i]).collect();
            let sig = Self::wrap_row_signature(&cells);
            fresh_sigs.insert(*row, sig);
            if self.wrap_sig.get(row) != Some(&sig) {
                let px = Self::measure_wrap_height(&cells, window);
                heights.push((*row, px));
            }
        }

        // Shrink: a row that used to wrap but has no wrap-on cell this frame (wrap toggled off,
        // content cleared) — and is still in the visible range — is told to drop its wrap
        // contribution (the worker floors it at the base / default). Bounded by the small
        // `wrap_sig` set.
        let visible = frame.rows.clone();
        let shrunk: Vec<u32> = self
            .wrap_sig
            .keys()
            .copied()
            .filter(|row| !by_row.contains_key(row) && visible.contains(row))
            .collect();
        for row in shrunk {
            heights.push((row, DEFAULT_ROW_HEIGHT_PX));
            self.wrap_sig.remove(&row);
        }

        // Commit the fresh signatures (measured rows are no longer dirty → convergence).
        for (row, sig) in fresh_sigs {
            self.wrap_sig.insert(row, sig);
        }

        if !heights.is_empty() {
            self.events
                .emit(&GridEvent::AutoGrowRows { heights }, window, cx);
        }
    }

    /// Render-test hook (`render.rs`): the pixel harness renders a single static frame over a
    /// shut-down worker, so the live measure→worker→republish loop can't round-trip in-capture.
    /// This runs the **real** wrap measurement once, up front, and applies each grown height
    /// **directly** to the shared cache the grid reads — for rows **without** an existing non-default
    /// override, emulating the worker's manual-skip (a file/injected custom height = manual). It
    /// mutates only the cache (never the worker), and is never called by the app.
    pub fn autogrow_measure_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (vw, vh) = self.viewport_wh(window);
        let Some(frame) = self.resolve_frame(vw, vh) else {
            return;
        };
        // Populate `self.wrap_cells` for the frame (discard the built layers).
        let _ = self.build_grid_layers(&frame, None);

        let mut by_row: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, wc) in self.wrap_cells.iter().enumerate() {
            by_row.entry(wc.row).or_default().push(i);
        }
        let mut grown: Vec<(u32, f32)> = Vec::new();
        for (row, idxs) in &by_row {
            let cells: Vec<&WrapCell> = idxs.iter().map(|&i| &self.wrap_cells[i]).collect();
            grown.push((*row, Self::measure_wrap_height(&cells, window)));
        }

        {
            let mut caches = self.sources.caches.write();
            if let Some(cache) = caches.get_mut(self.active_sheet) {
                for (row, px) in grown {
                    // A row already carrying a non-default height is treated as manual (file/injected
                    // custom height) — auto-grow leaves it. An auto (default) row grows to fit.
                    let has_override = (cache.row_height(row) - DEFAULT_ROW_HEIGHT_PX).abs() > 0.5;
                    if !has_override && px > DEFAULT_ROW_HEIGHT_PX + 0.5 {
                        cache.set_row_height(row, px);
                    }
                }
            }
        }
        cx.notify();
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
                        // A single press begins the drag-resize; the 2nd click of a double-click
                        // autofits the column to its content (`functional_spec.md §7`) without
                        // beginning a resize. The 3rd+ click of a rapid multi-click burst is ignored
                        // so a triple-click neither re-autofits (a redundant same-width undo step) nor
                        // starts a stray resize.
                        match event.click_count {
                            1 => this.begin_resize(RowOrCol::Col, c, event, window, cx),
                            2 => this.autofit_column(c, window, cx),
                            _ => {}
                        }
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
                        // A single press begins the drag-resize; the 2nd click of a double-click
                        // autofits the row to its content (`functional_spec.md §5`) without beginning a
                        // resize. The 3rd+ click of a rapid multi-click burst is ignored so a
                        // triple-click neither re-autofits (a redundant same-height undo step) nor
                        // starts a stray resize. Mirrors the column hotspot above.
                        match event.click_count {
                            1 => this.begin_resize(RowOrCol::Row, r, event, window, cx),
                            2 => this.autofit_row(r, window, cx),
                            _ => {}
                        }
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
    /// The header context-menu item list — `(label, enabled, event)` — for the axis + run in `menu`
    /// (`functional_spec.md §2`, `gaps_closing_7_15 §4`): Insert before / Insert after / Delete, then
    /// **Hide** / **Unhide**. Pure (no gpui) so the mapping is unit-testable, mirroring
    /// [`cell_menu_items`](Self::cell_menu_items). Note `enabled` (not `blocked`) — the render loop
    /// draws a disabled item when `!enabled`.
    fn header_menu_items(menu: &HeaderMenu) -> Vec<(String, bool, GridEvent)> {
        let count = menu.run.1 - menu.run.0 + 1;
        let (unit, before_word, after_word) = match menu.axis {
            RowOrCol::Row => ("row", "above", "below"),
            RowOrCol::Col => ("column", "left", "right"),
        };
        let plural = |n: u32| if n == 1 { "" } else { "s" };
        let p = plural(count);
        let (start, end) = (menu.run.0, menu.run.1);
        let after_at = end.saturating_add(1);
        // Axis-specific event constructors keep the labels/flags below axis-agnostic.
        // Alias the constructor type so the 4-tuple annotation stays under clippy's
        // type-complexity threshold.
        type HeaderMenuEvent = fn(u32, u32) -> GridEvent;
        let (insert_ev, delete_ev, hide_ev, unhide_ev): (
            HeaderMenuEvent,
            HeaderMenuEvent,
            HeaderMenuEvent,
            HeaderMenuEvent,
        ) = match menu.axis {
            RowOrCol::Row => (
                |at, count| GridEvent::InsertRows { at, count },
                |at, count| GridEvent::DeleteRows { at, count },
                |at, count| GridEvent::HideRows { at, count },
                |at, count| GridEvent::UnhideRows { at, count },
            ),
            RowOrCol::Col => (
                |at, count| GridEvent::InsertColumns { at, count },
                |at, count| GridEvent::DeleteColumns { at, count },
                |at, count| GridEvent::HideColumns { at, count },
                |at, count| GridEvent::UnhideColumns { at, count },
            ),
        };
        let mut items = vec![
            (
                format!("Insert {count} {unit}{p} {before_word}"),
                !menu.insert_before_blocked,
                insert_ev(start, count),
            ),
            (
                format!("Insert {count} {unit}{p} {after_word}"),
                !menu.insert_after_blocked,
                insert_ev(after_at, count),
            ),
            (
                format!("Delete {count} {unit}{p}"),
                !menu.delete_blocked,
                delete_ev(start, count),
            ),
            {
                // The label counts the tracks Hide will actually collapse (visible ones in the run),
                // not the run width — already-hidden tracks in the run stay hidden (a no-op). The
                // EVENT still targets the whole run (one undo step; re-hiding a hidden track is inert).
                let newly = count - menu.hidden_in_run;
                (
                    format!("Hide {newly} {unit}{}", plural(newly)),
                    !menu.hide_blocked,
                    hide_ev(start, count),
                )
            },
        ];
        // Unhide targets the minimal hidden SPAN in the run (bounded, one undo step), but the label
        // counts the ACTUAL hidden tracks in it (`hidden_in_run`) — which is < the span width when the
        // hidden tracks are sparse. Disabled (with the axis label) when the run holds no hidden track.
        match menu.unhide_run {
            Some((first, last)) => {
                let span = last - first + 1;
                let uc = menu.hidden_in_run;
                items.push((
                    format!("Unhide {uc} {unit}{}", plural(uc)),
                    true,
                    unhide_ev(first, span),
                ));
            }
            None => items.push((format!("Unhide {unit}s"), false, unhide_ev(start, count))),
        }
        items
    }

    fn header_menu_elements(&self, menu: HeaderMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let items = Self::header_menu_items(&menu);
        // The "merged cells" footnote is about the insert/delete merge guard only — Hide/Unhide are
        // never merge-blocked, so gate the note on the merge-guard flags, not every disabled item.
        let any_blocked =
            menu.insert_before_blocked || menu.insert_after_blocked || menu.delete_blocked;

        let mut card = div()
            .debug_selector(|| "header-menu-card".into())
            .absolute()
            .left(px(menu.x))
            .top(px(menu.y))
            // Occlude the card so a mouse-down anywhere on it — the p(4) padding ring or the
            // "Sheet has merged cells…" footnote row, neither of which carries a listener — can't
            // fall through to the deferred dismiss backdrop and close the menu without acting (same
            // backdrop-on-down bug as the action-bar popovers, BUG A/B). The item rows already
            // `stop_propagation`; this covers the dead zones around them.
            .occlude()
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
        for (label, enabled, event) in items {
            let mut item = div()
                .px(px(10.0))
                .py(px(4.0))
                .rounded_sm()
                .whitespace_nowrap()
                .child(label);
            if !enabled {
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

    /// Closes the chart context menu (click-away / Escape / after the item runs).
    fn close_chart_menu(&mut self, cx: &mut Context<Self>) {
        if self.chart_menu.take().is_some() {
            cx.notify();
        }
    }

    /// The right-click chart context menu overlay (P18, `ui_design §3.2`): a click-away backdrop +
    /// a one-item "Delete chart" card. Mirrors [`header_menu_elements`](Self::header_menu_elements).
    fn chart_menu_elements(&self, menu: ChartMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let id = menu.id;
        let card = div()
            .debug_selector(|| "chart-menu-card".into())
            .absolute()
            .left(px(menu.x))
            .top(px(menu.y))
            .occlude()
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(rgb(CELL_BG))
            .border_1()
            .border_color(rgb(HEADER_HAIRLINE))
            .rounded_md()
            .shadow_md()
            .text_size(px(CELL_FONT_PX))
            .min_w(px(140.0))
            .child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded_sm()
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .text_color(rgb(CELL_TEXT))
                    .hover(|s| s.bg(rgb(HEADER_SELECTED_BG)))
                    .child("Delete chart")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            this.selected_chart = None;
                            this.events
                                .emit(&GridEvent::ChartDeleted { id }, window, cx);
                            this.close_chart_menu(cx);
                            cx.stop_propagation();
                        }),
                    ),
            );
        let backdrop = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_chart_menu(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_chart_menu(cx);
                    cx.stop_propagation();
                }),
            );
        vec![
            deferred(backdrop).into_any_element(),
            deferred(card).into_any_element(),
        ]
    }

    /// The cell-area context menu's rows, top-to-bottom, as `(label, enabled, event)` — a `None`
    /// entry is a separator (`functional_spec.md §2` / D2.1 ordering). Split out from
    /// [`cell_menu_elements`](Self::cell_menu_elements) so the label/enable/event mapping is unit-
    /// testable without painting. Cut/Copy/Clear are always enabled (a selection is always ≥1 cell);
    /// Paste + Paste-values share `paste_enabled`; Insert/Delete are gated by the per-axis merge
    /// guard and reuse the header menu's row/column commands scoped to the selection's span. Clear
    /// Formatting is omitted (no style-clear op exists this batch).
    ///
    /// **Full-line suppression (data safety):** a full-**column** selection is stored literally as
    /// `rows 0..=MAX_ROWS-1`, so its row-structural items would read "Delete 1048576 rows" and wipe
    /// the sheet — the row items are dropped (its column items, `width == 1`, are the meaningful
    /// ones, matching Excel). A full-**row** selection symmetrically drops the column items;
    /// whole-sheet (spans all rows AND cols) drops both structural sets, leaving only the clipboard
    /// group.
    fn cell_menu_items(menu: &CellMenu) -> Vec<Option<(String, bool, GridEvent)>> {
        let range = menu.range;
        let (start, end) = (range.start, range.end);
        let rows = range.height();
        let cols = range.width();
        let row_after = end.row.saturating_add(1);
        let col_after = end.col.saturating_add(1);
        let rp = if rows == 1 { "" } else { "s" };
        let cp = if cols == 1 { "" } else { "s" };
        // Row-structural ops are meaningless / destructive on a full-column selection (and vice
        // versa); suppress the cross-axis set.
        let show_rows = !spans_all_rows(&range);
        let show_cols = !spans_all_cols(&range);

        let mut items = vec![
            Some(("Cut".to_string(), true, GridEvent::Copy { cut: true })),
            Some(("Copy".to_string(), true, GridEvent::Copy { cut: false })),
            Some(("Paste".to_string(), menu.paste_enabled, GridEvent::Paste)),
            Some((
                "Paste values".to_string(),
                menu.paste_enabled,
                GridEvent::PasteValues,
            )),
            Some((
                "Clear contents".to_string(),
                true,
                GridEvent::ClearCells(range),
            )),
        ];
        if show_rows || show_cols {
            items.push(None); // separator before the structural group
        }
        if show_rows {
            items.extend([
                Some((
                    format!("Insert {rows} row{rp} above"),
                    !menu.insert_row_above_blocked,
                    GridEvent::InsertRows {
                        at: start.row,
                        count: rows,
                    },
                )),
                Some((
                    format!("Insert {rows} row{rp} below"),
                    !menu.insert_row_below_blocked,
                    GridEvent::InsertRows {
                        at: row_after,
                        count: rows,
                    },
                )),
                Some((
                    format!("Delete {rows} row{rp}"),
                    !menu.delete_rows_blocked,
                    GridEvent::DeleteRows {
                        at: start.row,
                        count: rows,
                    },
                )),
            ]);
        }
        if show_cols {
            items.extend([
                Some((
                    format!("Insert {cols} column{cp} left"),
                    !menu.insert_col_left_blocked,
                    GridEvent::InsertColumns {
                        at: start.col,
                        count: cols,
                    },
                )),
                Some((
                    format!("Insert {cols} column{cp} right"),
                    !menu.insert_col_right_blocked,
                    GridEvent::InsertColumns {
                        at: col_after,
                        count: cols,
                    },
                )),
                Some((
                    format!("Delete {cols} column{cp}"),
                    !menu.delete_cols_blocked,
                    GridEvent::DeleteColumns {
                        at: start.col,
                        count: cols,
                    },
                )),
            ]);
        }
        items
    }

    /// The cell-area right-click context menu overlay (`functional_spec.md §2`): a click-away
    /// backdrop + a card of items that each emit an **existing** [`GridEvent`]. Cloned from
    /// [`chart_menu_elements`](Self::chart_menu_elements) / the header menu; item styling +
    /// enable/disable + the `.occlude()`d modal card + the deferred dismiss backdrop match those.
    /// Item order follows `functional_spec.md §2` / decision D2.1; Clear Formatting is omitted (no
    /// style-clear op exists this batch). Insert/Delete reuse the header menu's row/column
    /// commands, scoped to the snapshotted selection's span.
    fn cell_menu_elements(&self, menu: CellMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let items = Self::cell_menu_items(&menu);

        let mut card = div()
            .debug_selector(|| "cell-menu-card".into())
            .absolute()
            .left(px(menu.x))
            .top(px(menu.y))
            // Occlude the card so a mouse-down on its padding ring / a separator (no listener there)
            // can't fall through to the dismiss backdrop and close the menu without acting — the same
            // guard the header menu applies (BUG A/B).
            .occlude()
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
        for (i, entry) in items.into_iter().enumerate() {
            let Some((label, enabled, event)) = entry else {
                card = card.child(
                    div()
                        .my(px(4.0))
                        .mx(px(6.0))
                        .h(px(1.0))
                        .bg(rgb(HEADER_HAIRLINE)),
                );
                continue;
            };
            // The row index matches `cell_menu_items` so a test can target a specific row.
            let mut item = div()
                .debug_selector(move || format!("cell-menu-item-{i}"))
                .px(px(10.0))
                .py(px(4.0))
                .rounded_sm()
                .whitespace_nowrap()
                .child(label);
            if enabled {
                item = item
                    .cursor_pointer()
                    .text_color(rgb(CELL_TEXT))
                    .hover(|s| s.bg(rgb(HEADER_SELECTED_BG)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            this.events.emit(&event, window, cx);
                            this.close_cell_menu(cx);
                            cx.stop_propagation();
                        }),
                    );
            } else {
                item = item.text_color(rgb(HEADER_TEXT)).opacity(0.4);
            }
            card = card.child(item);
        }

        let backdrop = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_cell_menu(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.close_cell_menu(cx);
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

/// The `(hide_blocked, unhide_run, hidden_in_run)` header-menu flags for a run over an axis of
/// `total` tracks with the given `hidden` set (`gaps_closing_7_15 §4`).
///
/// - **Hide** is blocked when hiding the run would leave zero visible tracks:
///   `total − |hidden ∪ run| == 0` (where `|hidden ∪ run| = |hidden| + run_len − hidden_in_run`).
///   Usually this means Select-All → Hide, but it also (correctly) blocks a *smaller* run that
///   happens to cover every remaining visible track — e.g. when most of the axis is already hidden.
/// - **Unhide** targets the minimal `[first_hidden, last_hidden]` span **within** the run (so
///   Select-All → Unhide reveals the hidden cluster without spanning the whole axis); `None` when
///   the run contains no hidden track.
/// - **`hidden_in_run`** = how many tracks in the run are already hidden — the accurate count for
///   the menu labels (Unhide N; Hide counts the *newly*-hidden `run_len − hidden_in_run`).
fn hide_unhide_flags(
    run: (u32, u32),
    hidden: &std::collections::BTreeSet<u32>,
    total: u32,
) -> (bool, Option<(u32, u32)>, u32) {
    let (start, end) = run;
    // One pass over the hidden tracks within the run: first, last, and count.
    let mut first_hidden = None;
    let mut last_hidden = 0u32;
    let mut hidden_in_run = 0u32;
    for &i in hidden.range(start..=end) {
        if first_hidden.is_none() {
            first_hidden = Some(i);
        }
        last_hidden = i;
        hidden_in_run += 1;
    }
    let unhide_run = first_hidden.map(|f| (f, last_hidden));

    let run_len = (end - start + 1) as u64;
    let hidden_count = hidden.len() as u64;
    // Tracks hidden after the op = |hidden ∪ run| = run_len + (already-hidden outside the run).
    // Computed as a union size (not a running subtraction) so it can't underflow when run_len == total.
    // `hidden_in_run <= hidden_count`, so the inner subtraction is safe.
    let hidden_after = run_len + (hidden_count - hidden_in_run as u64);
    let visible_after = (total as u64).saturating_sub(hidden_after);
    (visible_after == 0, unhide_run, hidden_in_run)
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

/// The content-local px rect of the fill handle (`gaps_closing_7_15 §3`): a [`HANDLE_PX`] square
/// centered on the selection's bottom-right corner `(right_x, bottom_y)` (pre-scroll content
/// offsets). The center is clamped into the visible `[0, content_w] × [0, content_h]` box so a
/// whole-row / whole-column selection whose true corner is off-viewport still shows a grabbable
/// handle at the visible edge (D3.4). The same rect drives the render overlay and the mouse-down
/// hit-test, so they can never disagree.
fn fill_handle_square(
    right_x: f64,
    bottom_y: f64,
    scroll_x: f64,
    scroll_y: f64,
    content_w: f64,
    content_h: f64,
) -> (f32, f32, f32, f32) {
    let cx = (right_x - scroll_x).clamp(0.0, content_w) as f32;
    let cy = (bottom_y - scroll_y).clamp(0.0, content_h) as f32;
    (
        cx - HANDLE_PX / 2.0,
        cy - HANDLE_PX / 2.0,
        HANDLE_PX,
        HANDLE_PX,
    )
}

/// One dash's length (px) and the gap after it, for [`LinePattern::Dashed`] edges. Chosen so a
/// dash reads clearly at typical column widths/row heights without looking dotted.
const DASH_LEN: f32 = 4.0;
const DASH_GAP: f32 = 3.0;

/// Invokes `emit(offset, length)` once per dash of a dashed line spanning `[start, start + span)`:
/// dashes of [`DASH_LEN`] separated by [`DASH_GAP`], the final dash clamped so it never overruns the
/// span. The phase restarts at each edge's `start` — i.e. dashes are per-cell-edge, not continuous
/// across cell boundaries; this is intentional for MVP (consistent with how solid edges are drawn
/// per-cell, `architecture.md §7`).
fn for_each_dash(start: f32, span: f32, mut emit: impl FnMut(f32, f32)) {
    let mut pos = 0.0;
    while pos < span {
        emit(start + pos, DASH_LEN.min(span - pos));
        pos += DASH_LEN + DASH_GAP;
    }
}

/// Pushes the quad(s) for a vertical border edge centred on `boundary_x` (the shared column
/// boundary), spanning the cell's row height `h`, into `out`. Painted over the gridline/fills. The
/// quad(s) depend on `edge.pattern` (`architecture.md §7`): `Solid` → one `edge.weight`-px strip
/// (zero extra allocation — the common case); `Dashed` → a run of short strips; `Double` → two 1px
/// strips separated by a gap, spanning the weight.
fn push_vertical_edge(out: &mut Vec<AnyElement>, boundary_x: f32, y: f32, h: f32, edge: Edge) {
    let w = edge.weight as f32;
    let color = to_rgba(edge.color);
    let left = boundary_x - w / 2.0;
    match edge.pattern {
        LinePattern::Solid => out.push(rect_div(left, y, w, h).bg(color).into_any_element()),
        LinePattern::Dashed => for_each_dash(y, h, |oy, len| {
            out.push(rect_div(left, oy, w, len).bg(color).into_any_element());
        }),
        LinePattern::Double => {
            out.push(rect_div(left, y, 1.0, h).bg(color).into_any_element());
            out.push(
                rect_div(left + w - 1.0, y, 1.0, h)
                    .bg(color)
                    .into_any_element(),
            );
        }
    }
}

/// Pushes the quad(s) for a horizontal border edge centred on `boundary_y` (the shared row
/// boundary), spanning the cell's column width `w`, into `out`. Pattern handling mirrors
/// [`push_vertical_edge`].
fn push_horizontal_edge(out: &mut Vec<AnyElement>, x: f32, boundary_y: f32, w: f32, edge: Edge) {
    let h = edge.weight as f32;
    let color = to_rgba(edge.color);
    let top = boundary_y - h / 2.0;
    match edge.pattern {
        LinePattern::Solid => out.push(rect_div(x, top, w, h).bg(color).into_any_element()),
        LinePattern::Dashed => for_each_dash(x, w, |ox, len| {
            out.push(rect_div(ox, top, len, h).bg(color).into_any_element());
        }),
        LinePattern::Double => {
            out.push(rect_div(x, top, w, 1.0).bg(color).into_any_element());
            out.push(
                rect_div(x, top + h - 1.0, w, 1.0)
                    .bg(color)
                    .into_any_element(),
            );
        }
    }
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
    skip_right_gridline: bool,
    skip_bottom_gridline: bool,
) -> AnyElement {
    let mut el = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .bg(fill)
        .border_color(rgb(GRIDLINE))
        .flex()
        // Default vertical placement is BOTTOM — Excel-faithful (decision C): every cell
        // bottom-aligns its text unless it carries an explicit `v_align` (handled below). This is
        // also what mirror/pending cells (`style: None`) get, so the default is uniform.
        .items_end()
        .overflow_hidden()
        .whitespace_nowrap()
        .px(px(CELL_H_PAD))
        .text_size(px(CELL_FONT_PX))
        .text_color(text_color)
        // Render the grid in the bundled Inter family (an explicit per-cell family below still
        // wins, e.g. the serif case).
        .font_family(GRID_FONT_FAMILY);

    // Right + bottom gridlines — a fill paints over them (Excel look). A filled cell inside a
    // same-fill block suppresses the shared interior gridline (8a): `skip_*_gridline` is set when
    // the right / bottom neighbour resolves to the same fill, so the block reads as one solid
    // rectangle. Outer-boundary gridlines still draw; explicit borders are a separate later pass.
    if !skip_right_gridline {
        el = el.border_r_1();
    }
    if !skip_bottom_gridline {
        el = el.border_b_1();
    }

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
        // Strikethrough: a line through the text (mirrors the underline seam; combines with it —
        // `functional_spec.md §1.1`).
        if s.strikethrough {
            el = el.line_through();
        }
        // Explicit vertical alignment positions the text block within the row height. `None` keeps
        // the base default — BOTTOM, Excel-faithful (decision C) — so unset and explicit-`Bottom`
        // render identically (`functional_spec.md §1.3`, `architecture.md §7`).
        el = match s.v_align {
            Some(VAlign::Top) => el.items_start(),
            Some(VAlign::Center) => el.items_center(),
            Some(VAlign::Bottom) => el.items_end(),
            None => el,
        };
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
    } else if style.map(|s| s.wrap).unwrap_or(false) {
        // Wrapped text needs a width-bounded box to flow into: a flex row's direct text child is
        // sized to its (unwrapped) content, so `whitespace_normal` only breaks lines once the text
        // has a definite width. Wrap it in a full-width content box (which sets `whitespace_normal`,
        // overriding the cell's base `whitespace_nowrap`) so gpui wraps at the column width.
        // Horizontal placement moves from the flex's justify-content to the box's text-align; the
        // outer flex's `items_*` still positions the whole block vertically. The row now auto-grows
        // to fit the wrapped lines (`functional_spec.md §3`, the render-thread measurement pass), so
        // `overflow_hidden` only clips content beyond the auto-grow cap `MAX_AUTO_ROW_HEIGHT_PX` (or
        // a manually-shrunk row).
        let h_align = style
            .and_then(|s| s.h_align)
            .unwrap_or_else(|| kind.default_align());
        let content = div().w_full().whitespace_normal();
        let content = match h_align {
            Align::Left => content.text_left(),
            Align::Center => content.text_center(),
            Align::Right => content.text_right(),
        };
        el.child(content.child(text)).into_any_element()
    } else {
        el.child(text).into_any_element()
    }
}

/// The rendered font size (px) for a resolved style, mirroring [`cell_element`]: the grid default
/// [`CELL_FONT_PX`] unless the style pins a non-default quarter-point size (`q/4` pt → px). Shared
/// by the spill width gate so it measures against the same size the cell paints at.
fn font_px_of(style: RenderStyle) -> f32 {
    if style.font_size_q != 0 {
        style.font_size_q as f32 / 4.0 * 96.0 / 72.0
    } else {
        CELL_FONT_PX
    }
}

/// Builds a text-spill overlay element (`functional_spec.md §2.4`): the origin cell's text painted
/// across the spill rect `(x, y, w, h)` — the origin cell through its last empty neighbour — with
/// **no** fill, border, or gridline (those belong to the underlying cells). It mirrors
/// [`cell_element`]'s text styling exactly (padding, size, colour, family, bold/italic/underline/
/// strike, vertical alignment) so the origin portion paints identically to a non-spilling cell;
/// only the horizontal anchor differs, per the spill direction:
///
/// - **Left** (rightward spill): the box's left edge is the origin cell start → `justify_start`
///   pins the text at the origin, flowing right.
/// - **Right** (leftward spill): the box's right edge is the origin cell end → `justify_end` pins
///   the text at the origin's right, flowing left.
/// - **Center** (both): `justify_center` centres the text over the empty run.
///
/// `overflow_hidden` clips to the spill rect; `whitespace_nowrap` keeps it one line (wrap-on cells
/// never reach here).
#[allow(clippy::too_many_arguments)]
fn spill_element(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
    text_color: Rgba,
    align: Align,
    style: Option<RenderStyle>,
    font_family: Option<SharedString>,
) -> AnyElement {
    let mut el = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .flex()
        // Default vertical placement is BOTTOM (decision C), matching `cell_element`; an explicit
        // `v_align` below overrides it so spilled text sits at the origin cell's vertical position.
        .items_end()
        .overflow_hidden()
        .whitespace_nowrap()
        .px(px(CELL_H_PAD))
        .text_size(px(CELL_FONT_PX))
        .text_color(text_color)
        .font_family(GRID_FONT_FAMILY);

    el = match align {
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
        if s.strikethrough {
            el = el.line_through();
        }
        el = match s.v_align {
            Some(VAlign::Top) => el.items_start(),
            Some(VAlign::Center) => el.items_center(),
            Some(VAlign::Bottom) => el.items_end(),
            None => el,
        };
        if s.font_size_q != 0 {
            el = el.text_size(px(font_px_of(s)));
        }
    }
    if let Some(name) = font_family {
        el = el.font_family(name);
    }
    el.child(text).into_any_element()
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
        .font_family(GRID_FONT_FAMILY)
        .child(label)
        .into_any_element()
}

/// A filled accent rectangle (selection borders/edges are transparent-bg bordered divs; the
/// range fill + header edges are solid).
fn rect_div(x: f32, y: f32, w: f32, h: f32) -> gpui::Div {
    div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h))
}

/// Translucency of a reference-highlight **fill** (`formula-point-mode/architecture.md §4.1`): a
/// touch richer than the selection fill so a colored ref reads distinctly, yet still translucent so
/// the cell content stays legible underneath.
const REF_HIGHLIGHT_FILL_ALPHA: f32 = 0.16;

/// The point-mode drag **preview** border color (`formula-point-mode/functional_spec.md §2`): an
/// indigo distinct from the solid-blue selection rectangle (`ACCENT`) and from the reference-highlight
/// palette, drawn dashed (marching-ants marquee) so the three overlays that can coexist — selection,
/// ref highlights, point preview — stay visually distinct.
const POINT_PREVIEW_BORDER: u32 = 0x4F46E5;

/// Resolves a clicked `(row, col)` to the reference cell to insert (DPM.6, Q6): the top-left anchor
/// of the first merge in `merges` that covers it, or the cell itself when uncovered. Pure over the
/// merge list so it is unit-testable headless.
fn resolve_merge_anchor_in(row: u32, col: u32, merges: &[CellRange]) -> CellRef {
    let cell = CellRef::new(row, col);
    merges
        .iter()
        .find(|m| m.contains(cell))
        .map(|m| m.start)
        .unwrap_or(cell)
}

/// Expands a swept point-drag `range` so it never bisects a merge (DPM.6, Q6): union it with every
/// merge it intersects, iterating to a fixed point (an expansion can newly touch another merge).
/// Pure over the merge list so it is unit-testable headless.
fn expand_range_for_merges_in(range: CellRange, merges: &[CellRange]) -> CellRange {
    let mut result = range;
    loop {
        let mut grew = false;
        for m in merges {
            if m.intersects(&result) {
                let unioned = CellRange::new(
                    CellRef::new(
                        result.start.row.min(m.start.row),
                        result.start.col.min(m.start.col),
                    ),
                    CellRef::new(result.end.row.max(m.end.row), result.end.col.max(m.end.col)),
                );
                if unioned != result {
                    result = unioned;
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    result
}

/// Resolves a reference-highlight palette slot to a concrete `0xRRGGBB` color for the current theme
/// (`formula-point-mode/architecture.md §4.2`) — the highlight border color, and (via `.opacity`)
/// its translucent fill. `is_dark` selects the palette's dark variant; the grid currently renders on
/// the white `CELL_BG` regardless of OS appearance, so callers pass `false`, but the seam stays for
/// the future theme-aware grid + the in-editor styling control that will reuse this one palette.
fn ref_slot_border(slot: u8, is_dark: bool) -> u32 {
    let color = freecell_core::palette::ref_color(slot as usize);
    if is_dark {
        color.dark.to_hex()
    } else {
        color.light.to_hex()
    }
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
/// Horizontal slack (px) added to the measured glyph width when the wrap-off in-cell editor grows
/// rightward: `2 × CELL_H_PAD` so the text is not flush against the accent border, plus a caret's
/// worth of room so the last glyph + blinking caret stay visible as the user types
/// (`DECISIONS_TO_REVIEW.md`).
const IN_CELL_GROW_SLACK_PX: f32 = 2.0 * CELL_H_PAD + 4.0;
/// The in-cell editor's cap-reject danger border/tooltip colour (theme danger, matching chrome).
const IN_CELL_DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the in-cell cap-error popover (`ui_design.md §4`, matching chrome).
const IN_CELL_TOOLTIP_BG: u32 = 0x2B2B2B;
const IN_CELL_TOOLTIP_TEXT: u32 = 0xF5F5F5;
/// The in-cell editor wrapper's total vertical border: `border_2` = 2 px top + 2 px bottom. The
/// hosted input's height is the cell height minus this, floored at the line box (see
/// [`incell_input_geometry`]).
const IN_CELL_BORDER_TOTAL_PX: f32 = 4.0;
/// Line-box factor for the in-cell editor's hosted input, applied to the font px so the line box
/// scales with the font instead of gpui-component's fixed `Rems(1.25)` (= 20 px at the 16 px rem).
/// This is the editor overlay's OWN line box, independent of the engine's font-size row auto-grow
/// (which is proportional — `cache::autofit_row_ironcalc_px`); `1.25` is a tight single-line box
/// that always fits inside a proportionally grown row, and the floor below covers rows that were
/// *not* grown.
const IN_CELL_LINE_HEIGHT_FACTOR: f32 = 1.25;

/// Pure sizing math for the in-cell editor overlay's grown box (`DECISIONS_TO_REVIEW.md`): given the
/// anchored cell's geometry and the pre-measured content, returns the editor's `(width, height)` in
/// **content-local** device px. Extracted from the render path so the growth + viewport clamp is
/// unit-testable without a `Window`.
///
/// - **Wrap-off** (`wrap == false`): the WIDTH grows to `max(cell_w, min_w, measured_text_w + slack)`
///   so a long string in a narrow column is readable, then is capped at the content viewport's right
///   edge — the editor never draws past `content_w` (content-local), i.e. never into the row header
///   or outside the grid. When the anchored cell is at/over the right edge (`avail < base`) or
///   scrolled off, the box keeps its base size and the content layer's `overflow_hidden` does the
///   clipping. Height stays the cell's own height.
/// - **Wrap-on** (`wrap == true`): the WIDTH is left at the cell's own width (floored at `min_w`, as
///   today) and the HEIGHT grows to the pre-measured wrapped height, floored at the cell height and
///   capped at `max_h` (Phase 7's `MAX_AUTO_ROW_HEIGHT_PX`) — the box previews the committed wrapped
///   footprint.
///
/// `cell_x` is the cell's content-local left edge (may be negative when scrolled under the gutter —
/// the left side is then clipped by `overflow_hidden`, not by this math).
#[allow(clippy::too_many_arguments)]
fn incell_editor_size(
    cell_x: f32,
    cell_w: f32,
    cell_h: f32,
    content_w: f32,
    min_w: f32,
    wrap: bool,
    measured_text_w: f32,
    slack: f32,
    wrapped_h: f32,
    max_h: f32,
) -> (f32, f32) {
    let base_w = cell_w.max(min_w);
    if wrap {
        let h = wrapped_h.clamp(cell_h, max_h.max(cell_h));
        (base_w, h)
    } else {
        let desired = (measured_text_w + slack).max(base_w);
        // Distance from the cell's left edge to the content viewport's right edge. `avail >= base_w`
        // exactly when the whole base box fits before the edge (always true for a fully-visible cell);
        // there we grow up to `min(desired, avail)` so the right border lands at most at the edge.
        // Otherwise the cell straddles / is past the edge, so keep the base box and let the content
        // layer's `overflow_hidden` clip it.
        let avail = content_w - cell_x;
        let w = if avail >= base_w {
            desired.min(avail)
        } else {
            base_w
        };
        (w, cell_h)
    }
}

/// The shaped width (device px) of `text` at the given resolved cell font, using the render thread's
/// text system — the exact advance the in-cell editor's hosted input paints, so the grown box fits it
/// (`DECISIONS_TO_REVIEW.md`). Empty text is zero-width (no shaping). One shaped line per frame for
/// the single active editor — O(text), not O(grid).
fn measure_incell_text_width(
    text: &str,
    font_px: f32,
    family: Option<SharedString>,
    bold: bool,
    italic: bool,
    window: &mut Window,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let family = family.unwrap_or_else(|| SharedString::from(GRID_FONT_FAMILY));
    let mut cell_font = font(family);
    if bold {
        cell_font = cell_font.bold();
    }
    if italic {
        cell_font = cell_font.italic();
    }
    let run = TextRun {
        len: text.len(),
        font: cell_font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_string()),
            px(font_px),
            &[run],
            None,
        )
        .width()
        .as_f32()
}

/// Pure autofit-width math (`functional_spec.md §7`, D7.3): the widest measured cell-text width
/// (device px) plus [`AUTOFIT_PADDING_PX`], clamped to `[AUTOFIT_MIN_WIDTH_PX, AUTOFIT_MAX_WIDTH_PX]`.
/// An empty column (`max_text_px == 0.0`) resolves to the floor. Extracted from
/// [`GridView::autofit_width_for_column`] so the padding + clamp is unit-testable without a `Window`.
fn autofit_width(max_text_px: f32) -> f32 {
    (max_text_px + AUTOFIT_PADDING_PX).clamp(AUTOFIT_MIN_WIDTH_PX, AUTOFIT_MAX_WIDTH_PX)
}

/// The device-px height a cell of `lines` visual lines at `font_px` occupies (`functional_spec.md
/// §5`): `lines * line_height + vpad`, where `line_height` is gpui's default **`phi`** line box
/// (`round(1.618 * font_px)`, matching `Style::line_height`) and `vpad` is the vertical slack a
/// single default line leaves in [`DEFAULT_ROW_HEIGHT_PX`] (so `lines == 1` at the default size ⇒
/// the default row height). Shared by wrap auto-grow ([`GridView::measure_wrap_height`]) and row
/// autofit ([`GridView::measure_row_height`]) so the two never diverge. Pure — unit-testable
/// without a `Window`.
fn cell_line_box_height(lines: u32, font_px: f32) -> f32 {
    let line_px = |px: f32| (GRID_LINE_HEIGHT_FACTOR * px).round();
    let vpad = DEFAULT_ROW_HEIGHT_PX - line_px(CELL_FONT_PX);
    lines as f32 * line_px(font_px) + vpad
}

/// Geometry the in-cell editor feeds its hosted single-line [`Input`] so a large font is not
/// clipped vertically (BUG A): `(control_height, line_height)` in **device** px.
///
/// - `line_height` = `font_px * 1.25` — font-relative, so the line box scales with the glyph.
/// - `control_height` = `(h - 4).max(line_height)` — the wrapper's inner box (cell height `h` minus
///   the 2 px top+bottom accent border), **floored at the line box**.
///
/// gpui-component's single-line `Input` otherwise pins a FIXED control height (`Size::Medium` →
/// `h_8` = 32 px, `input.rs`) and a FIXED line height (`const LINE_HEIGHT: Rems = Rems(1.25)` =
/// 20 px, `input.rs`), both independent of the applied `text_size`. A 24 pt (= 32 px) glyph then
/// overflows the 20 px line box inside the 32 px control and is cut off — while the committed cell
/// (`cell_element`, a plain `div().h(px(h)).text_size(..)` whose line height scales with the font)
/// renders it fine.
///
/// Why the floor is needed: `h` arrives in device px, and a cell may hold a large font in a row
/// that was NOT grown to fit it — a user-shrunk row, or a large font applied where auto-grow did
/// not run — so `h - 4` can land below the font's line box. (A row grown by the proportional
/// font-size auto-grow, `cache::autofit_row_ironcalc_px`, always clears the line box, so the floor
/// is a no-op there; it exists for the un-grown / shrunk case.) The `.max(line_height)` floor keeps
/// the control at least as tall as its own line box, so the glyph is never clipped; when
/// `h - 4 < line_height` the `Input` simply overflows the cell wrapper by a few px (single-line
/// `Input` has only `overflow_x_hidden` — no vertical mask — so the overflow stays visible and
/// vertically centered, not cut off). Pure so it is unit-testable; the on-screen result (that the
/// `Input` honours these) is the owner's Mac check.
fn incell_input_geometry(h: f32, font_px: f32) -> (f32, f32) {
    let line_h = font_px * IN_CELL_LINE_HEIGHT_FACTOR;
    // Floor at the line box so `line_h <= control_h` in ALL cases (auto-grown large fonts, default
    // cells, and short/min-height rows). `.max(0.0)` keeps a torn/negative `h` from underflowing;
    // `.max(line_h)` (line_h >= 0) subsumes it but the guard is kept for robustness.
    let control_h = (h - IN_CELL_BORDER_TOTAL_PX).max(0.0).max(line_h);
    (control_h, line_h)
}

/// The in-cell editor's resolved text attributes for a cell — the WYSIWYG font the overlay renders
/// (BUG #4). Mirrors [`cell_element`]'s resolution so editing looks like the committed cell.
struct IncellFont {
    /// Text size in px (the cell's `q/4` pt → px, or [`CELL_FONT_PX`] for a default-font cell).
    size_px: f32,
    /// The cell's explicit font family, or `None` for the workbook default.
    family: Option<SharedString>,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// Resolves the edited cell's [`IncellFont`] from its [`RenderStyle`] (if any) and the sheet's
/// `font_families` side table — pure so the resolution is unit-testable. A `None` style (default
/// cell, or an anchor scrolled out of the visible-styles snapshot) falls back to the default font
/// with no character styling. Mirrors [`cell_element`]: size from `font_size_q` (`0` = default),
/// family from the side table (index `0`/empty = default), and bold/italic/underline as-is.
fn resolve_incell_font(style: Option<RenderStyle>, families: &[SharedString]) -> IncellFont {
    let size_px = style
        .filter(|s| s.font_size_q != 0)
        .map(|s| s.font_size_q as f32 / 4.0 * 96.0 / 72.0)
        .unwrap_or(CELL_FONT_PX);
    let family = style.and_then(|s| {
        families
            .get(s.font_family as usize)
            .filter(|name| !name.is_empty())
            .cloned()
    });
    IncellFont {
        size_px,
        family,
        bold: style.map(|s| s.bold).unwrap_or(false),
        italic: style.map(|s| s.italic).unwrap_or(false),
        underline: style.map(|s| s.underline).unwrap_or(false),
    }
}

impl GridView {
    /// Measures the in-cell editor overlay's grown, viewport-clamped `(width, height)` for `cell`
    /// this frame (`DECISIONS_TO_REVIEW.md`). Reads the live edit text straight from the hosted
    /// [`InputState`] — the exact glyphs the input paints, so the grown box fits them, and it tracks
    /// each keystroke (the grid re-renders on the chrome's mirror push). For a **wrap-off** cell the
    /// text is shaped once to grow the WIDTH; for a **wrap-on** cell the wrapped HEIGHT is measured
    /// via the shared [`measure_wrap_height`](Self::measure_wrap_height) (same `LineWrapper` +
    /// `MAX_AUTO_ROW_HEIGHT_PX` cap as Phase 7 auto-grow). Pure clamping is delegated to
    /// [`incell_editor_size`]. Only the one editing cell is measured, so this is O(1)/frame.
    fn measure_incell_geom(
        &self,
        cell: CellRef,
        frame: &Frame,
        window: &mut Window,
        cx: &App,
    ) -> Option<(f32, f32)> {
        let (x, _y, cell_w, cell_h) = cell_rect(cell.row, cell.col, frame);
        let style = self.visible_styles.get(&(cell.row, cell.col)).copied();
        let wrap = style.map(|s| s.wrap).unwrap_or(false);
        let content_w = frame.content_w as f32;
        // The editor's live text is the hosted input's own value (kept in sync with the mirror by the
        // chrome). Empty (or a missing input) simply keeps the base box.
        let text = self
            .incell_input
            .as_ref()
            .map(|input| input.read(cx).value())
            .unwrap_or_default();
        let IncellFont {
            size_px,
            family,
            bold,
            italic,
            underline: _,
        } = resolve_incell_font(style, &self.visible_font_families);
        if wrap {
            // Measure the wrapped height of the live text at the cell's own width, reusing the Phase 7
            // measurement (clamped to `[default, MAX_AUTO_ROW_HEIGHT_PX]`); `incell_editor_size` then
            // floors it at the cell height and re-applies the cap.
            let wc = WrapCell {
                row: cell.row,
                text,
                font_px: size_px,
                bold,
                italic,
                font_family: family,
                col_w: cell_w,
            };
            let wrapped_h = Self::measure_wrap_height(&[&wc], window);
            Some(incell_editor_size(
                x,
                cell_w,
                cell_h,
                content_w,
                IN_CELL_MIN_W,
                true,
                0.0,
                0.0,
                wrapped_h,
                MAX_AUTO_ROW_HEIGHT_PX,
            ))
        } else {
            let measured = measure_incell_text_width(&text, size_px, family, bold, italic, window);
            Some(incell_editor_size(
                x,
                cell_w,
                cell_h,
                content_w,
                IN_CELL_MIN_W,
                false,
                measured,
                IN_CELL_GROW_SLACK_PX,
                0.0,
                MAX_AUTO_ROW_HEIGHT_PX,
            ))
        }
    }

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
        let (x, y, cell_w, cell_h) = cell_rect(cell.row, cell.col, frame);
        let wrap = self
            .visible_styles
            .get(&(cell.row, cell.col))
            .map(|s| s.wrap)
            .unwrap_or(false);
        // The grown box measured this frame (`measure_incell_geom`): rightward over neighbours for a
        // wrap-off cell, downward for a wrap-on one. `None` on a non-`render` build path (perf/test
        // harness) → the base cell-rect size, matching the pre-grow behaviour there.
        let (w, h) = self
            .incell_geom
            .unwrap_or((cell_w.max(IN_CELL_MIN_W), cell_h));
        let danger = self.incell_cap.is_some();
        let border = if danger {
            rgb(IN_CELL_DANGER)
        } else {
            rgb(ACCENT)
        };
        // Resolve the edited cell's own font so the overlay is WYSIWYG (BUG #4): a large-font title
        // cell must edit in place at that size + style, not the grid's default 13 px. The snapshot
        // only holds visible cells, so a scrolled-out anchor (the overlay renders even off-viewport)
        // simply falls back to the default font. Mirrors `cell_element`'s resolution.
        let IncellFont {
            size_px: font_px,
            family,
            bold,
            italic,
            underline,
        } = resolve_incell_font(
            self.visible_styles.get(&(cell.row, cell.col)).copied(),
            &self.visible_font_families,
        );
        // Size the hosted input to (at least) its own font-scaled line box so a large font is not
        // clipped vertically (BUG A). gpui-component's single-line `Input` pins a fixed 32 px
        // control height (`Size::Medium` → `h_8`) and a fixed 20 px line height (`Rems(1.25)`)
        // regardless of `text_size`; `Input::h()`/`h_full()` only affect multi-line mode, so pin
        // the single-line control height via `min_h`/`max_h` (both applied after gpui-component's
        // `input_h` via `refine_style`) and override the line box. `incell_input_geometry` fills the
        // cell inner height where it can and floors at the line box otherwise (the control may then
        // overflow the cell wrapper a few px — visible, not clipped; see its doc). Sized from the
        // CELL height (not the grown box `h`): a wrap-on editor grows its box downward but its hosted
        // single-line input stays a first-line control at the top, so the caret is not left floating
        // in the middle of a tall box.
        let (control_h, line_h) = incell_input_geometry(cell_h, font_px);
        let mut input_el = Input::new(input)
            .appearance(false)
            .text_size(px(font_px))
            .px_0()
            .w_full()
            .min_h(px(control_h))
            .max_h(px(control_h))
            .line_height(px(line_h));
        if let Some(name) = family {
            input_el = input_el.font_family(name);
        }
        if bold {
            input_el = input_el.font_weight(FontWeight::BOLD);
        }
        if italic {
            input_el = input_el.italic();
        }
        if underline {
            input_el = input_el.underline();
        }
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
            // Wrap-off: the box is the cell height, so centre the single line. Wrap-on: the box grows
            // downward, so pin the input to the top (first line) — see `incell_input_geometry` above.
            .when(wrap, |d| d.items_start())
            .when(!wrap, |d| d.items_center())
            .bg(rgb(CELL_BG))
            .border_2()
            .border_color(border)
            .px(px(1.0))
            .text_size(px(font_px))
            .text_color(rgb(CELL_TEXT))
            // Strip the hosted input's own chrome (border / rounded / background / shadow) via
            // `appearance(false)` so it reads as editing the cell in place, not a control-in-a-box
            // (BUG D). The 2 px accent border on this wrapper is the intended in-place edit cue
            // (`ui_design.md §3`). The input's text is pinned to the EDITED CELL's resolved font —
            // size + family + bold/italic (BUG #4) — so a big-font title edits WYSIWYG; a default
            // cell falls back to the 13 px cell font (the input's own default is `text_sm` = 14 px,
            // one off). Its horizontal padding is dropped so glyphs line up with the cell's text.
            .child(input_el);

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

    /// The in-cell overlay's function-completion list + signature hint (`gaps_closing_7_15 §1`),
    /// anchored below the measured editor box (like the in-cell cap popover). Interactive (row
    /// clicks accept), so it is built at the root render level where `cx` is available. The cap
    /// error takes precedence at the shared anchor; the list takes precedence over the hint.
    fn incell_autocomplete_elements(
        &self,
        frame: &Frame,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if self.incell_cap.is_some() {
            return Vec::new();
        }
        let Some(cell) = self.incell_open else {
            return Vec::new();
        };
        let (x, y, cell_w, cell_h) = cell_rect(cell.row, cell.col, frame);
        let (_, h) = self
            .incell_geom
            .unwrap_or((cell_w.max(IN_CELL_MIN_W), cell_h));
        let top = y + h + 2.0;

        if let Some(list) = &self.incell_autocomplete {
            let card = div()
                .id("incell-autocomplete")
                .absolute()
                .left(px(x))
                .top(px(top))
                .occlude()
                .flex()
                .flex_col()
                .min_w(px(300.0))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .bg(rgb(CELL_BG))
                .border_1()
                .border_color(rgb(HEADER_HAIRLINE))
                .rounded_md()
                .shadow_md()
                .children(list.rows.iter().enumerate().map(|(i, row)| {
                    let highlighted = i == list.highlight;
                    div()
                        .id(gpui::ElementId::Name(
                            format!("incell-autocomplete-row-{i}").into(),
                        ))
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .px_2()
                        .py(px(2.0))
                        .when(highlighted, |d| d.bg(rgb(HEADER_SELECTED_BG)))
                        // Hover highlights a row too (`functional_spec.md §1`, Mouse).
                        .hover(|s| s.bg(rgb(HEADER_SELECTED_BG)))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(CELL_TEXT))
                                .font_weight(FontWeight::MEDIUM)
                                .child(row.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(HEADER_TEXT))
                                .whitespace_nowrap()
                                .child(row.template.clone()),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                                this.events
                                    .emit(&GridEvent::AutocompleteAcceptAt(i), window, cx);
                            }),
                        )
                }));
            return vec![deferred(card).into_any_element()];
        }

        if let Some(template) = &self.incell_sig_hint {
            let hint = div()
                .absolute()
                .left(px(x))
                .top(px(top))
                .px_2()
                .py_1()
                .bg(rgb(CELL_BG))
                .text_color(rgb(HEADER_TEXT))
                .text_size(px(11.0))
                .border_1()
                .border_color(rgb(HEADER_HAIRLINE))
                .rounded_md()
                .shadow_md()
                .whitespace_nowrap()
                .child(template.clone());
            return vec![deferred(hint).into_any_element()];
        }

        Vec::new()
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

            // Measure the in-cell editor's grown size (it grows to fit the live text — rightward for
            // a wrap-off cell, downward for a wrap-on one) BEFORE the layer build reads it, because
            // measuring needs the render thread's text system (`window`). Only the single editing
            // cell is measured, so this is O(1)/frame. `None` closes/clears it (the base cell-rect
            // size then applies). `visible_styles` was just populated by `resolve_frame`.
            self.incell_geom = self
                .incell_open
                .and_then(|cell| self.measure_incell_geom(cell, &frame, window, cx));
            root_children.extend(self.build_grid_layers(&frame, None));
            // Wrap-driven row auto-grow (`functional_spec.md §3`): measure the just-laid-out wrap-on
            // cells and, for rows whose wrap inputs changed, ask the worker to grow/shrink them.
            // Runs after the layer build (which filled `self.wrap_cells`) so it has the real column
            // widths; the emit routes to `Command::AutoGrowRowHeights`.
            self.run_autogrow(&frame, window, cx);
            // Divider resize hotspots paint last (over the header strips) so they win the hit-test.
            root_children.extend(self.resize_hotspots(&frame, cx));
            // The in-cell completion list / signature hint (`gaps_closing_7_15 §1`) — rendered at
            // the root (not in `build_grid_layers`) so its rows can carry `cx.listener` click
            // handlers, like the header/cell context menus.
            if self.incell_open.is_some() {
                root_children.extend(self.incell_autocomplete_elements(&frame, cx));
            }
        }

        // Header insert/delete context menu (deferred → above everything but the loading overlay).
        if let Some(menu) = self.header_menu {
            root_children.extend(self.header_menu_elements(menu, cx));
        }
        // Chart "Delete chart" context menu (P18) — same deferred overlay pattern.
        if let Some(menu) = self.chart_menu {
            root_children.extend(self.chart_menu_elements(menu, cx));
        }
        // Cell-area right-click context menu (`functional_spec.md §2`) — same deferred overlay.
        if let Some(menu) = self.cell_menu {
            root_children.extend(self.cell_menu_elements(menu, cx));
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
                // The in-cell completion list preempts navigation/accept/dismiss keys before the
                // Tab/Esc/quick-edit arms below (`gaps_closing_7_15 §1`), routed to the chrome
                // (the list-state owner) via the window like the other in-cell events.
                if this.incell_autocomplete.is_some() {
                    match event.keystroke.key.as_str() {
                        "down" => {
                            cx.stop_propagation();
                            this.events.emit(
                                &GridEvent::AutocompleteNav { down: true },
                                window,
                                cx,
                            );
                            return;
                        }
                        "up" => {
                            cx.stop_propagation();
                            this.events.emit(
                                &GridEvent::AutocompleteNav { down: false },
                                window,
                                cx,
                            );
                            return;
                        }
                        "enter" | "tab" => {
                            cx.stop_propagation();
                            this.events.emit(&GridEvent::AutocompleteAccept, window, cx);
                            return;
                        }
                        "escape" => {
                            cx.stop_propagation();
                            this.events
                                .emit(&GridEvent::AutocompleteDismiss, window, cx);
                            return;
                        }
                        _ => {}
                    }
                }
                let modifiers = event.keystroke.modifiers;
                match event.keystroke.key.as_str() {
                    "tab" => {
                        cx.stop_propagation();
                        let dir = if modifiers.shift {
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
                    // Quick-edit: an unmodified arrow commits + moves the active cell
                    // (`functional_spec.md §5.2`), reusing the Tab commit-move plumbing. Uses the
                    // shared caret-intent predicate (excludes `function`, which macOS sets on the
                    // arrow cluster) so a plain arrow still moves. Defensive symmetric mirror of the
                    // data-row path — type-to-replace edits in the data row, and `begin_in_cell`
                    // clears quick-edit, so the in-cell overlay is not open in quick-edit today; kept
                    // in sync so a future overlay-hosted quick-edit works.
                    key @ ("left" | "right" | "up" | "down")
                        if this.quick_edit && !caret_intent_modifiers(&modifiers) =>
                    {
                        cx.stop_propagation();
                        let dir = match key {
                            "left" => Direction::Left,
                            "right" => Direction::Right,
                            "up" => Direction::Up,
                            _ => Direction::Down,
                        };
                        this.events
                            .emit(&GridEvent::InCellCommitMove(dir), window, cx);
                    }
                    // A caret-only key falls through to the input (no `stop_propagation`); ask the
                    // chrome to recompute the list/hint once the caret has moved, since the input
                    // fires no event on a pure caret move (`gaps_closing_7_15 §1`).
                    "left" | "right" | "home" | "end" => {
                        this.events
                            .emit(&GridEvent::AutocompleteCaretMoved, window, cx);
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

    /// Whether a selection drag is currently armed (test introspection for BUG #2 — opening the
    /// in-cell editor must not leave a live grid drag).
    #[cfg(test)]
    pub(crate) fn has_active_drag(&self) -> bool {
        self.drag.is_some()
    }

    /// The derived fidelity of each installed chart on `sheet`, in install order (P8 test
    /// introspection — proves `set_sheet_charts` resolves + classifies the specs).
    #[cfg(test)]
    pub(crate) fn sheet_chart_fidelities(
        &self,
        sheet: SheetId,
    ) -> Vec<freecell_chart_model::Fidelity> {
        self.charts
            .get(&sheet)
            .map(|layer| layer.placements.iter().map(|p| p.fidelity).collect())
            .unwrap_or_default()
    }

    /// The cell the in-cell overlay currently covers, if open (test introspection for BUG #5 — the
    /// commit/cancel handlers close it, which proves an in-cell key command routed through the grid).
    #[cfg(test)]
    pub(crate) fn incell_open_for_test(&self) -> Option<CellRef> {
        self.incell_open
    }

    /// Test seam: whether the grid's copy of the chrome's quick-edit flag is set (proves
    /// [`set_edit_state`](Self::set_edit_state) threads it, `functional_spec.md §5`).
    #[cfg(test)]
    pub(crate) fn quick_edit_for_test(&self) -> bool {
        self.quick_edit
    }

    /// Test seam: the grid's stored reference highlights (proves
    /// [`set_edit_state`](Self::set_edit_state) threads `ref_highlights`, which the overlay pass
    /// paints — `formula-point-mode/architecture.md §4.1`).
    #[cfg(test)]
    pub(crate) fn ref_highlights_for_test(&self) -> &[(CellRange, u8)] {
        &self.ref_highlights
    }

    /// The grid-local center of the current selection's fill handle for `window` — computed exactly
    /// as the render overlay + `handle_mouse_down` hit-test do (`gaps_closing_7_15 §3`), so a test
    /// can synthesize a mouse-down on the handle without hard-coding pixel geometry.
    #[cfg(test)]
    pub(crate) fn fill_handle_center_for_test(&self, window: &Window) -> Option<(f32, f32)> {
        let active = self.active_sheet;
        let (scroll_x, scroll_y) = self.scroll_of(active);
        let (viewport_w, viewport_h) = self.viewport_wh(window);
        let content_h = (viewport_h - COL_HEADER_H as f64).max(0.0);
        let caches = self.sources.caches.read();
        let cache = caches.get(active)?;
        let (row_axis, col_axis) = cache.axes();
        let row_header_w = Self::gutter_width(&row_axis, scroll_y, content_h);
        let content_w = (viewport_w - row_header_w as f64).max(0.0);
        let sel = self.selection().range();
        let right_x = col_axis.offset_of(sel.end.col + 1);
        let bottom_y = row_axis.offset_of(sel.end.row + 1);
        let (hx, hy, _, _) =
            fill_handle_square(right_x, bottom_y, scroll_x, scroll_y, content_w, content_h);
        // Content-local center → grid-local by adding the gutter/header offsets.
        Some((
            hx + HANDLE_PX / 2.0 + row_header_w,
            hy + HANDLE_PX / 2.0 + COL_HEADER_H,
        ))
    }

    /// Whether a fill drag is currently armed, and its current `(seed, target, axis)` if so — test
    /// introspection for the fill-handle drag state machine (`gaps_closing_7_15 §3`).
    #[cfg(test)]
    pub(crate) fn fill_drag_for_test(&self) -> Option<(CellRange, CellRange, Option<FillAxis>)> {
        self.fill_drag.map(|d| (d.seed, d.target, d.axis))
    }

    /// Emits [`GridEvent::InCellCommitMove`] exactly as the `capture_key_down` Tab handler does — for
    /// a BUG #5 test that must reproduce the emit happening **while the grid entity is leased**
    /// (`cx.listener` == `grid.update`). The headless key-dispatch path cannot route a keystroke
    /// through the nested + `deferred()` overlay input to this grid ancestor (on macOS the key
    /// arrives via `do_command_by_selector`), so the test calls this from inside `grid.update`.
    #[cfg(test)]
    pub(crate) fn emit_incell_commit_move_for_test(
        &self,
        dir: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.events
            .emit(&GridEvent::InCellCommitMove(dir), window, cx);
    }

    /// Emits [`GridEvent::InCellCancel`] exactly as the `capture_key_down` Escape handler does — the
    /// Escape twin of [`emit_incell_commit_move_for_test`](Self::emit_incell_commit_move_for_test).
    #[cfg(test)]
    pub(crate) fn emit_incell_cancel_for_test(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.events.emit(&GridEvent::InCellCancel, window, cx);
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

    // ---- ChartLayer install (P8, `charts/architecture.md §4.2`) ----------------------------

    /// `set_sheet_charts` resolves each spec into a `RenderedChart`, deriving its display fidelity
    /// from the retained source — a Faithful line, a Degraded (3-D→2-D) chart, and an Unsupported
    /// group, in order; an empty list then clears them.
    #[gpui::test]
    fn set_sheet_charts_stores_and_derives_fidelity(cx: &mut TestAppContext) {
        use freecell_chart_model::{
            Anchor, AnchorCell, Axis, Category, Chart, ChartKind, Fidelity, Grouping, Legend,
            Series, SourceXml,
        };

        let line = || Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("A"),
                vec![Category::Text("Q1".into()), Category::Text("Q2".into())],
                vec![1.0, 2.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        let anchor = Anchor::new(AnchorCell::new(1, 1), AnchorCell::new(6, 14));
        let spec = |xml: &str| ChartSpec::loaded(line(), SourceXml::new(xml), Vec::new(), anchor);

        let grid = grid(cx);
        let fidelities = grid.update(cx, |g, cx| {
            let sheet = g.active_sheet();
            g.set_sheet_charts(
                sheet,
                Arc::from(vec![
                    spec("<c:lineChart/>"),
                    spec("<c:bar3DChart/>"),
                    spec("<c:surfaceChart/>"),
                ]),
                cx,
            );
            g.sheet_chart_fidelities(sheet)
        });
        assert_eq!(
            fidelities,
            vec![
                Fidelity::Faithful,
                Fidelity::Degraded,
                Fidelity::Unsupported
            ]
        );

        // An empty install clears the sheet's charts.
        let cleared = grid.update(cx, |g, cx| {
            let sheet = g.active_sheet();
            g.set_sheet_charts(sheet, Arc::from(Vec::new()), cx);
            g.sheet_chart_fidelities(sheet)
        });
        assert!(cleared.is_empty(), "an empty install clears the ChartLayer");
    }

    /// P11 "off-screen free": the ChartLayer materializes only the on-screen charts and culls the
    /// off-screen ones from the build; a chart that scrolls out is freed from the materialized set
    /// and re-materializes when it scrolls back in. Driven through the grid's placement-based cull
    /// against a mock geometry (the paint loop uses the same [`GridView::visible_charts`]).
    #[gpui::test]
    fn offscreen_charts_are_freed_and_rematerialize_on_scrollback(cx: &mut TestAppContext) {
        use freecell_chart_model::{
            Anchor, AnchorCell, Axis, Category, Chart, ChartKind, Grouping, Legend, Series,
            SourceXml,
        };

        // A uniform grid geometry (100 px columns, 24 px rows) so the anchor→pixel mapping is exact.
        struct UniformGeom;
        impl chart_layer::GridGeometry for UniformGeom {
            fn col_start(&self, col: u32) -> f64 {
                col as f64 * 100.0
            }
            fn row_start(&self, row: u32) -> f64 {
                row as f64 * 24.0
            }
            fn col_at(&self, x: f64) -> u32 {
                (x.max(0.0) / 100.0).floor() as u32
            }
            fn row_at(&self, y: f64) -> u32 {
                (y.max(0.0) / 24.0).floor() as u32
            }
        }

        let line = || Chart {
            title: Some("S".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("A"),
                vec![Category::Text("Q1".into())],
                vec![1.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        // Three charts spread far apart horizontally: cols 0–5, 20–25, 40–45 (x ≈ 0, 2000, 4000 px).
        let spec_at = |from_col: u32, to_col: u32| {
            ChartSpec::loaded(
                line(),
                SourceXml::new("<c:lineChart/>"),
                Vec::new(),
                Anchor::new(AnchorCell::new(from_col, 0), AnchorCell::new(to_col, 5)),
            )
        };
        let specs = Arc::from(vec![spec_at(0, 5), spec_at(20, 25), spec_at(40, 45)]);

        let grid = grid(cx);
        let sheet = grid.update(cx, |g, cx| {
            let s = g.active_sheet();
            g.set_sheet_charts(s, specs, cx);
            s
        });

        let geom = UniformGeom;
        let (cw, ch) = (600.0_f64, 300.0_f64);

        let on_screen = |cx: &mut TestAppContext, scroll_x: f64| {
            grid.update(cx, |g, _cx| {
                g.on_screen_chart_indices(sheet, &geom, scroll_x, 0.0, cw, ch)
            })
        };

        // At the origin only chart 0 (x 0..500) is on-screen; the far ones are freed (culled).
        assert_eq!(
            on_screen(cx, 0.0),
            vec![0],
            "only the on-screen chart is materialized",
        );

        // Scroll right so chart 1 (cols 20–25) maps into the viewport; chart 0 is now freed.
        assert_eq!(
            on_screen(cx, 2000.0),
            vec![1],
            "a previously off-screen chart re-materializes; the scrolled-away one is freed",
        );

        // Scroll back to the origin → chart 0 re-materializes (correct on scroll-back).
        assert_eq!(
            on_screen(cx, 0.0),
            vec![0],
            "scrolling back re-materializes the origin chart",
        );
    }

    // ---- P18: chart manipulation (select / move / resize / delete) --------------------------

    /// A loaded line-chart spec at `anchor` stamped with `id`, for the manipulation tests.
    fn chart_spec_at(anchor: freecell_chart_model::Anchor, id: ChartId) -> ChartSpec {
        use freecell_chart_model::{
            Axis as CAxis, Category, Chart, ChartKind, Grouping, Legend, Series, SourceXml,
        };
        let chart = Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("A"),
                vec![Category::Text("Q1".into()), Category::Text("Q2".into())],
                vec![1.0, 2.0],
            )],
            cat_axis: CAxis::untitled(),
            val_axis: CAxis::untitled(),
            legend: Some(Legend::default()),
        };
        ChartSpec::loaded(chart, SourceXml::new("<c:lineChart/>"), Vec::new(), anchor).with_id(id)
    }

    /// A chart anchored over cols 1..6, rows 1..15 — content rect ≈ x[100,680] y[24,380] against the
    /// demo geometry (100 px cols / 24 px rows). A grid-local click at (400, 200) lands inside it.
    fn big_chart_anchor() -> freecell_chart_model::Anchor {
        use freecell_chart_model::{Anchor, AnchorCell};
        Anchor::new(AnchorCell::new(1, 1), AnchorCell::new(6, 15))
    }

    #[gpui::test]
    fn chart_body_click_selects_and_move_persists_new_anchor(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    events.borrow_mut().clear();
                    // Mouse-down on the chart body selects it + arms a move drag (no cell selection).
                    let before_sel = *grid.selection();
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, 400.0, 200.0), window, cx);
                    assert_eq!(grid.selected_chart, Some(ChartId(7)), "chart selected");
                    assert!(
                        events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::ChartSelected(ChartId(7)))),
                        "a chart click emits ChartSelected so the window opens the edit panel (P19)"
                    );
                    assert!(
                        matches!(
                            grid.chart_drag,
                            Some(ChartDrag {
                                mode: ChartDragMode::Move,
                                ..
                            })
                        ),
                        "a body click arms a move drag"
                    );
                    assert_eq!(
                        *grid.selection(),
                        before_sel,
                        "chart click didn't move the cell selection"
                    );
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "no SelectionChanged from a chart click"
                    );
                    // Drag right+down by 60,48 px, then release → anchor persists.
                    grid.handle_mouse_move(&move_ev(460.0, 248.0), window, cx);
                    grid.handle_mouse_up(&up_ev(), window, cx);
                    assert!(grid.chart_drag.is_none(), "drag cleared on release");
                    let moved = events
                        .borrow()
                        .iter()
                        .find_map(|e| match e {
                            GridEvent::ChartAnchorChanged { id, anchor } if *id == ChartId(7) => {
                                Some(*anchor)
                            }
                            _ => None,
                        })
                        .expect("a move emits ChartAnchorChanged");
                    // The chart translated right (+60 px = past col 1's 180 px? no → same col, +offset)
                    // and down, so the from-corner shifted from the original A-B2 anchor.
                    assert_ne!(
                        moved,
                        big_chart_anchor(),
                        "the persisted anchor reflects the move"
                    );
                    assert!(
                        moved.from.col > 1 || moved.from.col_off_emu > 0,
                        "the from corner moved right: {moved:?}"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn no_movement_click_is_a_pure_select(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    events.borrow_mut().clear();
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, 400.0, 200.0), window, cx);
                    grid.handle_mouse_up(&up_ev(), window, cx); // released without moving
                    assert_eq!(grid.selected_chart, Some(ChartId(7)));
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::ChartAnchorChanged { .. })),
                        "a click that never moved persists no anchor"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn chart_handle_resize_persists_larger_anchor(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    events.borrow_mut().clear();
                    // Arm a bottom-right resize directly (avoids pixel-precise handle hit math).
                    let rect = ChartRect {
                        x: 100.0,
                        y: 24.0,
                        w: 580.0,
                        h: 356.0,
                    };
                    grid.begin_chart_interaction(
                        ChartHit::Handle {
                            id: ChartId(7),
                            handle: Handle::BottomRight,
                            rect,
                        },
                        (680.0, 380.0),
                        cx,
                    );
                    // Drag the bottom-right corner out by +100,+100 → the chart grows.
                    grid.handle_mouse_move(&move_ev(780.0, 480.0), window, cx);
                    grid.handle_mouse_up(&up_ev(), window, cx);
                    let resized = events
                        .borrow()
                        .iter()
                        .find_map(|e| match e {
                            GridEvent::ChartAnchorChanged { id, anchor } if *id == ChartId(7) => {
                                Some(*anchor)
                            }
                            _ => None,
                        })
                        .expect("a resize emits ChartAnchorChanged");
                    // The from-corner is unchanged (bottom-right resize pins the top-left); the
                    // to-corner grew past the original col 6 / row 15.
                    assert_eq!(resized.from, big_chart_anchor().from, "top-left pinned");
                    assert!(
                        resized.to.col > 6 || (resized.to.col == 6 && resized.to.col_off_emu > 0),
                        "the chart grew to the right: {resized:?}"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn delete_key_emits_chart_deleted_and_clears_selection(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    grid.set_selected_chart(Some(ChartId(7)), cx);
                    events.borrow_mut().clear();
                    grid.handle_key_down(&key_ev("delete", None, false), window, cx);
                    assert!(grid.selected_chart.is_none(), "selection cleared on delete");
                    assert!(
                        events.borrow().iter().any(
                            |e| matches!(e, GridEvent::ChartDeleted { id } if *id == ChartId(7))
                        ),
                        "Delete emits ChartDeleted"
                    );
                    // The chart is NOT also treated as a cell clear.
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::ClearCells(_))),
                        "deleting a selected chart does not clear cell contents"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn miss_click_clears_chart_selection(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    grid.set_selected_chart(Some(ChartId(7)), cx);
                    // A click in the column-header strip (content_y < 0) misses every chart → deselect.
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, 60.0, 10.0), window, cx);
                    assert!(
                        grid.selected_chart.is_none(),
                        "a miss-click clears the chart selection"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn escape_cancels_chart_drag_and_selection(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let rect = ChartRect {
                        x: 100.0,
                        y: 24.0,
                        w: 200.0,
                        h: 120.0,
                    };
                    grid.begin_chart_interaction(
                        ChartHit::Body {
                            id: ChartId(7),
                            rect,
                        },
                        (200.0, 80.0),
                        cx,
                    );
                    assert!(grid.chart_drag.is_some());
                    grid.handle_key_down(&key_ev("escape", None, false), window, cx);
                    assert!(grid.chart_drag.is_none() && grid.selected_chart.is_none());
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn right_click_chart_opens_delete_menu(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let sheet = grid.active_sheet;
                    grid.set_sheet_charts(
                        sheet,
                        Arc::from(vec![chart_spec_at(big_chart_anchor(), ChartId(7))]),
                        cx,
                    );
                    events.borrow_mut().clear();
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 200.0),
                        window,
                        cx,
                    );
                    let menu = grid
                        .chart_menu
                        .expect("a right-click on a chart opens its menu");
                    assert_eq!(menu.id, ChartId(7));
                    assert_eq!(grid.selected_chart, Some(ChartId(7)));
                    // A header right-click was NOT opened (chart took priority).
                    assert!(grid.header_menu.is_none());
                });
            })
            .unwrap();
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

    /// A long single line that wraps to several visual lines at a narrow column — the auto-grow
    /// measurement fixture.
    const WRAP_LONG: &str = "this note wraps across several visual lines at a narrow column width";

    /// Sources with a wrap-on long-text cell at B2, optionally a second wrap-on cell at B3 carrying
    /// a manual row-height override (for the harness-hook skip test).
    fn wrap_sources(col_w: f32, b3_override: Option<f32>) -> (GridDataSources, SheetId) {
        use freecell_core::cache::{SheetCacheBuilder, SheetCaches};
        use freecell_core::publication::{CellKind, Publication, PublishedCell};
        let sheet = SheetId(0);
        let wrap = RenderStyle {
            wrap: true,
            ..RenderStyle::default()
        };
        let cell = |row, col, text: &str| PublishedCell {
            row,
            col,
            display_text: text.to_string(),
            kind: CellKind::Text,
            text_color: None,
        };
        let mut builder = SheetCacheBuilder::new(
            freecell_core::limits::MAX_ROWS,
            freecell_core::limits::MAX_COLS,
        )
        .col_width(1, col_w)
        .cell_style(1, 1, wrap);
        let mut cells = vec![cell(1, 1, WRAP_LONG)];
        if let Some(h) = b3_override {
            builder = builder.cell_style(2, 1, wrap).row_height(2, h);
            cells.push(cell(2, 1, WRAP_LONG));
        }
        let mut caches = SheetCaches::new();
        caches.insert(sheet, builder.build());
        let publication = Publication {
            sheet,
            rows: 0..40,
            cols: 0..20,
            generation: 1,
            cells,
        };
        (
            GridDataSources {
                publication: Arc::new(ArcSwap::from_pointee(publication)),
                caches: Arc::new(RwLock::new(caches)),
            },
            sheet,
        )
    }

    /// A recording `GridView` over the given sources (mirrors [`grid_recording`] but lets the caller
    /// pick the sources — the auto-grow fixtures need a wrap-on cell).
    #[allow(clippy::type_complexity)]
    fn recording_over(
        cx: &mut TestAppContext,
        sources: GridDataSources,
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
            let g = cx.new(|cx| GridView::new(sources, sink, cx));
            *slot = Some(g.clone());
            Root::new(g, window, cx)
        });
        (out.expect("grid built"), window, events)
    }

    /// The heights carried by the single `AutoGrowRows` the recording captured, or `None`.
    fn captured_autogrow(events: &Rc<RefCell<Vec<GridEvent>>>) -> Option<Vec<(u32, f32)>> {
        events.borrow().iter().find_map(|e| match e {
            GridEvent::AutoGrowRows { heights } => Some(heights.clone()),
            _ => None,
        })
    }

    /// Runs the real post-layout auto-grow pass once (resolve a frame, fill the wrap buffer, measure
    /// + emit) — the render-path measurement without a full paint.
    fn drive_autogrow(
        cx: &mut TestAppContext,
        g: &gpui::Entity<GridView>,
        window: &gpui::WindowHandle<Root>,
    ) {
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let (vw, vh) = grid.viewport_wh(window);
                    let frame = grid.resolve_frame(vw, vh).expect("frame resolves");
                    let _ = grid.build_grid_layers(&frame, None);
                    grid.run_autogrow(&frame, window, cx);
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn autogrow_measures_wrapped_height_and_emits_once_then_converges(cx: &mut TestAppContext) {
        // A wrap-on long-text cell at a narrow column is measured to a height > default and emitted
        // ONCE; a second identical pass emits nothing — the wrap-input signature is unchanged, so a
        // height-only republish never re-triggers auto-grow (convergence, no oscillation).
        let (sources, _sheet) = wrap_sources(80.0, None);
        let (g, window, events) = recording_over(cx, sources);

        drive_autogrow(cx, &g, &window);
        let heights = captured_autogrow(&events).expect("first pass emits AutoGrowRows");
        assert_eq!(heights.len(), 1, "one dirty wrap row");
        let (row, px) = heights[0];
        assert_eq!(row, 1);
        assert!(
            px > DEFAULT_ROW_HEIGHT_PX + 1.0,
            "the narrow wrap row grew beyond default (got {px})"
        );

        // Second pass over the same inputs → the dirty set is empty → NO further command.
        events.borrow_mut().clear();
        drive_autogrow(cx, &g, &window);
        assert!(
            captured_autogrow(&events).is_none(),
            "a settled wrap row must not re-emit (converges in one step)"
        );
    }

    #[gpui::test]
    fn autogrow_narrower_column_grows_taller(cx: &mut TestAppContext) {
        // Narrowing the column produces more wrapped lines → a taller measured height (§3.2).
        let measure = |cx: &mut TestAppContext, col_w: f32| -> f32 {
            let (sources, _s) = wrap_sources(col_w, None);
            let (g, window, events) = recording_over(cx, sources);
            drive_autogrow(cx, &g, &window);
            captured_autogrow(&events).expect("emits")[0].1
        };
        let wide = measure(cx, 200.0);
        let narrow = measure(cx, 60.0);
        assert!(
            narrow > wide + 1.0,
            "narrower column grows the row taller (narrow {narrow} vs wide {wide})"
        );
    }

    #[gpui::test]
    fn autogrow_measure_now_grows_default_but_skips_overridden(cx: &mut TestAppContext) {
        // The render-harness hook grows a default-height wrap row and leaves an already-overridden
        // (manual) wrap row untouched.
        let (sources, sheet) = wrap_sources(80.0, Some(30.0));
        let (g, window, _events) = recording_over(cx, sources);
        let (grown, manual) = window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.autogrow_measure_now(window, cx);
                    let guard = grid.sources.caches.read();
                    let cache = guard.get(sheet).unwrap();
                    (cache.row_height(1), cache.row_height(2))
                })
            })
            .unwrap();
        assert!(
            grown > DEFAULT_ROW_HEIGHT_PX + 1.0,
            "the default wrap row grew (got {grown})"
        );
        assert!(
            (manual - 30.0).abs() < 0.6,
            "the manually-sized wrap row is unchanged (got {manual})"
        );
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
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(0, 0)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
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

    #[gpui::test]
    fn set_edit_state_threads_quick_edit(cx: &mut TestAppContext) {
        // The chrome pushes quick-edit through the edit-state; the grid stores it for its in-cell
        // arrow branch (`functional_spec.md §5`).
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, _window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(0, 0)),
                        None,
                        true,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    assert!(
                        grid.quick_edit_for_test(),
                        "quick_edit must thread through set_edit_state"
                    );
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(0, 0)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    assert!(
                        !grid.quick_edit_for_test(),
                        "quick_edit clears when pushed false"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn set_edit_state_threads_ref_highlights(cx: &mut TestAppContext) {
        // The chrome pushes the same-sheet reference highlights through the edit-state; the grid
        // stores them for its overlay paint pass (`formula-point-mode/architecture.md §4.1`), and a
        // later push (edit committed / cancelled) with an empty vec clears them.
        let (g, window, _events) = grid_recording(cx);
        let highlights = vec![
            (CellRange::single(CellRef::new(0, 0)), 0u8),
            (CellRange::new(CellRef::new(2, 2), CellRef::new(6, 4)), 1u8),
        ];
        window
            .update(cx, |_root, _window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(0, 0)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        highlights.clone(),
                        cx,
                    );
                    assert_eq!(
                        grid.ref_highlights_for_test(),
                        highlights.as_slice(),
                        "ref_highlights must thread through set_edit_state"
                    );
                    // A commit/cancel pushes an empty vec → highlights clear.
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    assert!(
                        grid.ref_highlights_for_test().is_empty(),
                        "ref_highlights clear when an empty vec is pushed"
                    );
                });
            })
            .unwrap();
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

    /// A left-button mouse-move to grid-local `(x, y)` (P18 chart-drag tests).
    fn move_ev(x: f32, y: f32) -> MouseMoveEvent {
        MouseMoveEvent {
            position: gpui::point(px(x), px(y)),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        }
    }

    /// A left mouse-up (position is irrelevant to the chart-drag commit).
    fn up_ev() -> MouseUpEvent {
        MouseUpEvent {
            button: MouseButton::Left,
            position: gpui::point(px(0.0), px(0.0)),
            modifiers: Modifiers::default(),
            click_count: 1,
        }
    }

    // ---- Drag fill handle (`gaps_closing_7_15 §3`) ------------------------------------------

    /// A mouse-down on the selection's fill handle arms a fill drag (seed = the selection); the same
    /// grab while the in-cell editor is open does NOT (the handle is suppressed during editing).
    #[gpui::test]
    fn fill_handle_grab_arms_fill_drag_and_hides_while_editing(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let seed = CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)); // B2:C3
                    grid.set_selection(
                        SelectionModel {
                            anchor: seed.start,
                            active: seed.end,
                        },
                        cx,
                    );
                    let (hx, hy) = grid
                        .fill_handle_center_for_test(window)
                        .expect("handle center resolves");
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, hx, hy), window, cx);
                    let (armed_seed, armed_target, axis) = grid
                        .fill_drag_for_test()
                        .expect("grabbing the handle arms a fill drag");
                    assert_eq!(armed_seed, seed, "seed = the selection at grab");
                    assert_eq!(armed_target, seed, "target starts equal to the seed");
                    assert!(
                        axis.is_none(),
                        "axis undecided until the pointer leaves the seed"
                    );

                    // Release, then re-grab while editing → no fill drag (handle suppressed).
                    grid.handle_mouse_up(&up_ev(), window, cx);
                    grid.incell_open = Some(seed.start);
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, hx, hy), window, cx);
                    assert!(
                        grid.fill_drag_for_test().is_none(),
                        "the fill handle is not grabbable while the in-cell editor is open"
                    );
                });
            })
            .unwrap();
    }

    /// A downward handle drag sets a Vertical preview extending past the seed, and on release emits
    /// `GridEvent::FillDrag` and expands the selection to the filled region.
    #[gpui::test]
    fn fill_drag_down_previews_emits_and_expands_selection(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Default selection is A1 (single-cell seed).
                    let (hx, hy) = grid
                        .fill_handle_center_for_test(window)
                        .expect("handle center resolves");
                    grid.handle_mouse_down(&mouse_ev(MouseButton::Left, hx, hy), window, cx);
                    events.borrow_mut().clear();
                    // Drag straight down ~100 px (staying in column A) → a multi-row target.
                    grid.handle_mouse_move(&move_ev(hx - 5.0, hy + 100.0), window, cx);
                    let (_, target, axis) = grid.fill_drag_for_test().expect("still fill-dragging");
                    assert_eq!(axis, Some(FillAxis::Vertical), "dominant axis is vertical");
                    assert_eq!(
                        target.start,
                        CellRef::new(0, 0),
                        "target still anchored at A1"
                    );
                    assert!(target.end.row > 0, "target extended below the seed");
                    assert_eq!(target.start.col, target.end.col, "single column preserved");

                    grid.handle_mouse_up(&up_ev(), window, cx);
                    assert!(
                        grid.fill_drag_for_test().is_none(),
                        "drag cleared on release"
                    );
                    let (ev_seed, ev_target, ev_axis) = events
                        .borrow()
                        .iter()
                        .find_map(|e| match e {
                            GridEvent::FillDrag { seed, target, axis } => {
                                Some((*seed, *target, *axis))
                            }
                            _ => None,
                        })
                        .expect("release emits FillDrag");
                    assert_eq!(ev_seed, CellRange::single(CellRef::new(0, 0)));
                    assert_eq!(ev_axis, FillAxis::Vertical);
                    assert_eq!(
                        ev_target, target,
                        "the emitted target = the previewed target"
                    );
                    // The selection now covers the whole filled region.
                    assert_eq!(
                        grid.selection().range(),
                        target,
                        "selection expands to the fill"
                    );
                });
            })
            .unwrap();
    }

    /// The sticky axis: once a drag has committed to Vertical, a subsequent larger horizontal
    /// excursion keeps it Vertical (Excel D3.1), and a return inside the seed resets it.
    #[gpui::test]
    fn fill_drag_axis_is_sticky_then_resets_inside_seed(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, _window, cx| {
                g.update(cx, |grid, cx| {
                    let seed = CellRange::new(CellRef::new(2, 2), CellRef::new(2, 2)); // C3
                    grid.set_selection(SelectionModel::single(seed.start), cx);
                    grid.fill_drag = Some(FillDrag {
                        seed,
                        target: seed,
                        axis: None,
                    });
                    // First move: 3 rows down, 0 cols → Vertical.
                    grid.set_fill_target_from_cell(CellRef::new(5, 2));
                    assert_eq!(
                        grid.fill_drag_for_test().unwrap().2,
                        Some(FillAxis::Vertical)
                    );
                    // Now a bigger horizontal excursion — axis stays Vertical (sticky), so the
                    // target keeps the seed's single column.
                    grid.set_fill_target_from_cell(CellRef::new(3, 9));
                    let (_, target, axis) = grid.fill_drag_for_test().unwrap();
                    assert_eq!(
                        axis,
                        Some(FillAxis::Vertical),
                        "axis stays vertical mid-drag"
                    );
                    assert_eq!(target.start.col, 2, "still pinned to the seed column");
                    assert_eq!(target.end.col, 2);
                    // Returning inside the seed clears the axis.
                    grid.set_fill_target_from_cell(seed.start);
                    let (_, target, axis) = grid.fill_drag_for_test().unwrap();
                    assert!(axis.is_none(), "axis resets inside the seed");
                    assert_eq!(target, seed, "target collapses to the seed");
                });
            })
            .unwrap();
    }

    /// A drag released without leaving the seed (target == seed) is a no-op: no `FillDrag`, and the
    /// selection is unchanged (D3.3 — inward is not a clear).
    #[gpui::test]
    fn fill_drag_onto_seed_is_a_noop(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let seed = CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)); // B2:C3
                    grid.set_selection(
                        SelectionModel {
                            anchor: seed.start,
                            active: seed.end,
                        },
                        cx,
                    );
                    grid.fill_drag = Some(FillDrag {
                        seed,
                        target: seed,
                        axis: None,
                    });
                    events.borrow_mut().clear();
                    grid.handle_mouse_up(&up_ev(), window, cx);
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::FillDrag { .. })),
                        "a release onto the seed emits no fill"
                    );
                    assert_eq!(grid.selection().range(), seed, "selection unchanged");
                });
            })
            .unwrap();
    }

    // ---- Point-mode routing (formula-point-mode Phase 3) ------------------------------------

    /// The `InsertReference` targets emitted by a recording grid, in order.
    fn inserted_refs(events: &Rc<RefCell<Vec<GridEvent>>>) -> Vec<(String, bool)> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                GridEvent::InsertReference {
                    a1,
                    replace_pending,
                } => Some((a1.clone(), *replace_pending)),
                _ => None,
            })
            .collect()
    }

    /// A reference-ready formula edit turns a grid click into a point INSERT — it emits
    /// `InsertReference` (never `SelectionChanged`) and leaves the grid selection untouched; a
    /// pending-ref push makes the emit carry `replace_pending: true`.
    #[gpui::test]
    fn point_ready_click_inserts_not_selects(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let before = grid.selection().range();
                    // Reference-ready formula edit → a click points instead of selecting.
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        true,
                        false,
                        Vec::new(),
                        cx,
                    );
                    events.borrow_mut().clear();
                    grid.mouse_down_cell(2, 2, &mouse_ev(MouseButton::Left, 0.0, 0.0), window, cx);
                    assert_eq!(
                        inserted_refs(&events),
                        vec![("C3".to_string(), false)],
                        "the click appends C3"
                    );
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "no SelectionChanged is emitted in point-mode"
                    );
                    assert_eq!(grid.selection().range(), before, "selection is untouched");

                    // A pending ref makes the next click a REPLACE even at a not-ready caret.
                    grid.point_drag = None;
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        false,
                        true,
                        Vec::new(),
                        cx,
                    );
                    events.borrow_mut().clear();
                    grid.mouse_down_cell(0, 0, &mouse_ev(MouseButton::Left, 0.0, 0.0), window, cx);
                    assert_eq!(
                        inserted_refs(&events),
                        vec![("A1".to_string(), true)],
                        "a pending ref makes the click replace"
                    );
                });
            })
            .unwrap();
    }

    /// With no reference-ready / pending signal, a grid click behaves exactly as today: it emits
    /// `SelectionChanged` (the commit path) and no `InsertReference`.
    #[gpui::test]
    fn not_ready_click_selects_as_today(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    events.borrow_mut().clear();
                    grid.mouse_down_cell(2, 2, &mouse_ev(MouseButton::Left, 0.0, 0.0), window, cx);
                    assert!(
                        events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "a not-ready click commits via SelectionChanged"
                    );
                    assert!(
                        inserted_refs(&events).is_empty(),
                        "no reference is inserted"
                    );
                });
            })
            .unwrap();
    }

    /// A point drag re-emits the swept range as it grows (dedupe per cell), and a release on the
    /// origin cell yields a single-cell ref (`functional_spec.md §2` Drag details).
    #[gpui::test]
    fn point_drag_emits_expanded_range_and_dedupes(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        true,
                        false,
                        Vec::new(),
                        cx,
                    );
                    events.borrow_mut().clear();
                    // Mouse-down at C3 → origin, emits "C3".
                    grid.mouse_down_cell(2, 2, &mouse_ev(MouseButton::Left, 0.0, 0.0), window, cx);
                    // Grow to E7 → "C3:E7"; a repeat is deduped; back to origin → "C3".
                    grid.set_point_target_from_cell(CellRef::new(6, 4), window, cx);
                    grid.set_point_target_from_cell(CellRef::new(6, 4), window, cx);
                    grid.set_point_target_from_cell(CellRef::new(2, 2), window, cx);
                    assert_eq!(
                        inserted_refs(&events),
                        vec![
                            ("C3".to_string(), false),
                            ("C3:E7".to_string(), true),
                            ("C3".to_string(), true),
                        ],
                        "grow, dedupe the repeat, collapse back to a single cell"
                    );
                    // Release clears the drag.
                    grid.handle_mouse_up(&up_ev(), window, cx);
                    assert!(grid.point_drag.is_none(), "release clears the point drag");
                });
            })
            .unwrap();
    }

    /// A merged cache: a reference-ready click on a covered cell inserts the merge's anchor (DPM.6).
    #[gpui::test]
    fn point_click_on_merge_inserts_anchor(cx: &mut TestAppContext) {
        let (g, window, events) = recording_over(cx, merged_sources());
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        true,
                        false,
                        Vec::new(),
                        cx,
                    );
                    events.borrow_mut().clear();
                    // C3 (2,2) is covered by the merge B2:C3 → the anchor B2 is inserted.
                    grid.mouse_down_cell(2, 2, &mouse_ev(MouseButton::Left, 0.0, 0.0), window, cx);
                    assert_eq!(
                        inserted_refs(&events),
                        vec![("B2".to_string(), false)],
                        "a click on a covered cell inserts the merge anchor"
                    );
                });
            })
            .unwrap();
    }

    /// The pure merge helpers: anchor resolution + fixed-point range expansion.
    #[test]
    fn merge_anchor_and_range_expansion() {
        let merges = vec![CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2))]; // B2:C3
        assert_eq!(
            resolve_merge_anchor_in(2, 2, &merges),
            CellRef::new(1, 1),
            "a covered cell resolves to the anchor"
        );
        assert_eq!(
            resolve_merge_anchor_in(5, 5, &merges),
            CellRef::new(5, 5),
            "an uncovered cell resolves to itself"
        );
        // A swept rect touching the merge grows to include the whole span (A1:B2 → A1:C3).
        let swept = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1));
        assert_eq!(
            expand_range_for_merges_in(swept, &merges),
            CellRange::new(CellRef::new(0, 0), CellRef::new(2, 2))
        );
        // Chained fixed-point: expanding via the first merge newly touches the second.
        let chained = vec![
            CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)), // B2:C3
            CellRange::new(CellRef::new(2, 2), CellRef::new(4, 4)), // C3:E5
        ];
        assert_eq!(
            expand_range_for_merges_in(CellRange::single(CellRef::new(1, 1)), &chained),
            CellRange::new(CellRef::new(1, 1), CellRef::new(4, 4)),
            "B2 → B2:C3 → B2:E5 (both merges folded in)"
        );
    }

    /// A sheet switch drops any armed point drag so it can never leak onto the new sheet.
    #[gpui::test]
    fn set_active_sheet_clears_point_drag(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, _window, cx| {
                g.update(cx, |grid, cx| {
                    grid.point_drag = Some(PointDrag {
                        origin: CellRef::new(2, 2),
                        last_range: CellRange::single(CellRef::new(2, 2)),
                    });
                    grid.set_active_sheet(SheetId(1), cx);
                    assert!(
                        grid.point_drag.is_none(),
                        "point drag cleared on sheet switch"
                    );
                });
            })
            .unwrap();
    }

    /// Sources with a single merge B2:C3 (no published cells) for the point-on-merge test.
    fn merged_sources() -> GridDataSources {
        use freecell_core::cache::{SheetCacheBuilder, SheetCaches};
        use freecell_core::publication::Publication;
        let sheet = SheetId(0);
        let cache = SheetCacheBuilder::new(
            freecell_core::limits::MAX_ROWS,
            freecell_core::limits::MAX_COLS,
        )
        .merge(CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)))
        .build();
        let mut caches = SheetCaches::new();
        caches.insert(sheet, cache);
        let publication = Publication {
            sheet,
            rows: 0..40,
            cols: 0..20,
            generation: 1,
            cells: Vec::new(),
        };
        GridDataSources {
            publication: Arc::new(ArcSwap::from_pointee(publication)),
            caches: Arc::new(RwLock::new(caches)),
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

    // ---- Autofit column width (`functional_spec.md §7`) ------------------------------------

    /// Sources with the given `(row, col, text)` published plain-text cells over Excel-max dims —
    /// the fixtures for the autofit width tests.
    fn autofit_sources(cells: &[(u32, u32, &str)]) -> (GridDataSources, SheetId) {
        use freecell_core::cache::{SheetCacheBuilder, SheetCaches};
        use freecell_core::publication::{CellKind, Publication, PublishedCell};
        let sheet = SheetId(0);
        let published: Vec<PublishedCell> = cells
            .iter()
            .map(|(r, c, t)| PublishedCell {
                row: *r,
                col: *c,
                display_text: t.to_string(),
                kind: CellKind::Text,
                text_color: None,
            })
            .collect();
        let mut caches = SheetCaches::new();
        caches.insert(
            sheet,
            SheetCacheBuilder::new(
                freecell_core::limits::MAX_ROWS,
                freecell_core::limits::MAX_COLS,
            )
            .build(),
        );
        let publication = Publication {
            sheet,
            rows: 0..40,
            cols: 0..20,
            generation: 1,
            cells: published,
        };
        (
            GridDataSources {
                publication: Arc::new(ArcSwap::from_pointee(publication)),
                caches: Arc::new(RwLock::new(caches)),
            },
            sheet,
        )
    }

    /// The `(start, end, px)` of each column `ResizeCommitted` the recording captured, in order.
    fn captured_resizes(events: &Rc<RefCell<Vec<GridEvent>>>) -> Vec<(u32, u32, f32)> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                GridEvent::ResizeCommitted {
                    axis: RowOrCol::Col,
                    start,
                    end,
                    px,
                } => Some((*start, *end, *px)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn autofit_width_pads_and_clamps() {
        // Empty column (no text) → the floor.
        assert_eq!(autofit_width(0.0), AUTOFIT_MIN_WIDTH_PX);
        // A normal measured width → text + padding.
        assert_eq!(autofit_width(100.0), 100.0 + AUTOFIT_PADDING_PX);
        // A tiny width still clamps up to the floor.
        assert_eq!(autofit_width(1.0), AUTOFIT_MIN_WIDTH_PX);
        // A very long value clamps down to the cap.
        assert_eq!(autofit_width(100_000.0), AUTOFIT_MAX_WIDTH_PX);
    }

    #[gpui::test]
    fn autofit_column_fits_published_cell(cx: &mut TestAppContext) {
        // Double-click autofit sizes a column to its widest published cell + padding, over the floor.
        let text = "a reasonably wide value";
        let (sources, _s) = autofit_sources(&[(1, 2, text)]);
        let (g, window, events) = recording_over(cx, sources);
        let measured = window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    let m =
                        measure_incell_text_width(text, CELL_FONT_PX, None, false, false, window);
                    grid.autofit_column(2, window, cx);
                    m
                })
            })
            .unwrap();
        let resizes = captured_resizes(&events);
        assert_eq!(resizes.len(), 1, "single-column autofit emits one resize");
        let (start, end, px) = resizes[0];
        assert_eq!((start, end), (2, 2), "resizes just the clicked column");
        assert!(
            (px - autofit_width(measured)).abs() < 0.5,
            "px {px} vs expected {}",
            autofit_width(measured)
        );
        assert!(px > AUTOFIT_MIN_WIDTH_PX, "wide content exceeds the floor");
    }

    #[gpui::test]
    fn autofit_empty_column_shrinks_to_floor(cx: &mut TestAppContext) {
        // A column with no published cells autofits to the configured floor (D7.3).
        let (sources, _s) = autofit_sources(&[]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.autofit_column(5, window, cx);
                });
            })
            .unwrap();
        let resizes = captured_resizes(&events);
        assert_eq!(resizes.len(), 1);
        assert_eq!(resizes[0], (5, 5, AUTOFIT_MIN_WIDTH_PX));
    }

    #[gpui::test]
    fn autofit_multi_column_selection_fits_each(cx: &mut TestAppContext) {
        // A divider double-clicked inside a full-column multi-column selection autofits every column
        // in the run — each to its own content — one `ResizeCommitted` per column (D7.1).
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.select_column(1, false, window, cx);
                    grid.select_column(3, true, window, cx);
                    assert_eq!(grid.resize_run_for(RowOrCol::Col, 2), (1, 3));
                    grid.autofit_column(2, window, cx);
                });
            })
            .unwrap();
        let cols: Vec<u32> = captured_resizes(&events)
            .iter()
            .map(|(s, e, _)| {
                assert_eq!(s, e, "each autofit resize targets a single column");
                *s
            })
            .collect();
        assert_eq!(cols, vec![1, 2, 3], "each selected column autofit once");
    }

    #[gpui::test]
    fn autofit_under_select_all_fits_only_the_divider_column(cx: &mut TestAppContext) {
        // Select-all classifies as a full-column run spanning every column; autofit must NOT fan out
        // to 16,384 per-column `SetColumnWidths` (that would mass-shrink the sheet). The whole-sheet
        // run collapses to just the divider's own column — exactly one resize.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.select_all(window, cx);
                    assert_eq!(
                        grid.resize_run_for(RowOrCol::Col, 3),
                        (0, freecell_core::limits::MAX_COLS - 1),
                        "select-all resolves to the whole-sheet column run"
                    );
                    grid.autofit_column(3, window, cx);
                });
            })
            .unwrap();
        let resizes = captured_resizes(&events);
        assert_eq!(
            resizes.len(),
            1,
            "whole-sheet autofit emits exactly one resize, not one per column"
        );
        assert_eq!(resizes[0].0, 3);
        assert_eq!(resizes[0].1, 3, "only the divider column is autofit");
    }

    // ---- Autofit row height (`functional_spec.md §5`) ------------------------------------

    #[test]
    fn cell_line_box_height_matches_default_and_scales() {
        // One line at the default font is exactly the default row height (the vpad slack absorbs it).
        assert!((cell_line_box_height(1, CELL_FONT_PX) - DEFAULT_ROW_HEIGHT_PX).abs() < 0.001);
        // Each extra visual line adds one phi line box.
        let one = cell_line_box_height(1, CELL_FONT_PX);
        let three = cell_line_box_height(3, CELL_FONT_PX);
        let step = (GRID_LINE_HEIGHT_FACTOR * CELL_FONT_PX).round();
        assert!(
            (three - one - 2.0 * step).abs() < 0.001,
            "lines grow by phi·font"
        );
        assert!(three > one);
        // A larger font makes a single line taller than the default.
        assert!(
            cell_line_box_height(1, CELL_FONT_PX * 3.0) > cell_line_box_height(1, CELL_FONT_PX)
        );
    }

    /// Sources for the row-autofit tests: `(row, col, text, wrap)` published plain-text cells (a
    /// `wrap` cell gets a wrap-on cell style), plus optional per-column width overrides so a wrap-on
    /// cell can be forced to soft-wrap in a narrow column.
    fn autofit_row_sources(
        cells: &[(u32, u32, &str, bool)],
        col_widths: &[(u32, f32)],
    ) -> (GridDataSources, SheetId) {
        use freecell_core::cache::{SheetCacheBuilder, SheetCaches};
        use freecell_core::publication::{CellKind, Publication, PublishedCell};
        use freecell_core::style::RenderStyle;
        let sheet = SheetId(0);
        let published: Vec<PublishedCell> = cells
            .iter()
            .map(|(r, c, t, _)| PublishedCell {
                row: *r,
                col: *c,
                display_text: t.to_string(),
                kind: CellKind::Text,
                text_color: None,
            })
            .collect();
        let mut builder = SheetCacheBuilder::new(
            freecell_core::limits::MAX_ROWS,
            freecell_core::limits::MAX_COLS,
        );
        for (col, px) in col_widths {
            builder.push_col_width(*col, *px);
        }
        for (r, c, _, wrap) in cells {
            if *wrap {
                builder.push_cell_style(
                    *r,
                    *c,
                    RenderStyle {
                        wrap: true,
                        ..Default::default()
                    },
                );
            }
        }
        let mut caches = SheetCaches::new();
        caches.insert(sheet, builder.build());
        let publication = Publication {
            sheet,
            rows: 0..40,
            cols: 0..20,
            generation: 1,
            cells: published,
        };
        (
            GridDataSources {
                publication: Arc::new(ArcSwap::from_pointee(publication)),
                caches: Arc::new(RwLock::new(caches)),
            },
            sheet,
        )
    }

    /// The `(start, end, px)` of each **row** `ResizeCommitted` the recording captured, in order.
    fn captured_row_resizes(events: &Rc<RefCell<Vec<GridEvent>>>) -> Vec<(u32, u32, f32)> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                GridEvent::ResizeCommitted {
                    axis: RowOrCol::Row,
                    start,
                    end,
                    px,
                } => Some((*start, *end, *px)),
                _ => None,
            })
            .collect()
    }

    #[gpui::test]
    fn autofit_row_single_line_is_default(cx: &mut TestAppContext) {
        // A row of one-line populated cells autofits to the default height — one resize for that row.
        let (sources, _s) = autofit_row_sources(&[(2, 1, "hello", false)], &[]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| grid.autofit_row(2, window, cx));
            })
            .unwrap();
        let resizes = captured_row_resizes(&events);
        assert_eq!(resizes.len(), 1);
        assert_eq!(resizes[0].0, 2);
        assert_eq!(resizes[0].1, 2, "only the divider row is autofit");
        assert!(
            (resizes[0].2 - DEFAULT_ROW_HEIGHT_PX).abs() < 0.5,
            "single-line row fits the default, got {}",
            resizes[0].2
        );
    }

    #[gpui::test]
    fn autofit_row_explicit_newlines_grow(cx: &mut TestAppContext) {
        // A wrap-off cell with two explicit newlines is three visual lines → three line boxes.
        let (sources, _s) = autofit_row_sources(&[(3, 1, "a\nb\nc", false)], &[]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| grid.autofit_row(3, window, cx));
            })
            .unwrap();
        let px = captured_row_resizes(&events)[0].2;
        let expected = cell_line_box_height(3, CELL_FONT_PX);
        assert!(
            (px - expected).abs() < 0.5,
            "px {px} vs expected {expected}"
        );
        assert!(px > DEFAULT_ROW_HEIGHT_PX, "three lines exceed the default");
        assert!(px < MAX_AUTO_ROW_HEIGHT_PX, "well below the cap");
    }

    #[gpui::test]
    fn autofit_row_wrap_on_counts_wrapped_lines(cx: &mut TestAppContext) {
        // A wrap-on cell whose text far exceeds a narrow column soft-wraps to multiple lines, so the
        // fitted height exceeds a single line (the wrap-off single-line default).
        let long = "the quick brown fox jumps over the lazy dog again and again and again";
        let (sources, _s) = autofit_row_sources(&[(4, 1, long, true)], &[(1, 40.0)]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| grid.autofit_row(4, window, cx));
            })
            .unwrap();
        let px = captured_row_resizes(&events)[0].2;
        assert!(
            px > cell_line_box_height(1, CELL_FONT_PX) + 0.5,
            "wrapped text grows past a single line, got {px}"
        );
    }

    #[gpui::test]
    fn autofit_row_clamps_at_max(cx: &mut TestAppContext) {
        // A pathologically tall cell (many explicit lines) is clamped at `MAX_AUTO_ROW_HEIGHT_PX`.
        let many = "x\n".repeat(60);
        let (sources, _s) = autofit_row_sources(&[(5, 1, &many, false)], &[]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| grid.autofit_row(5, window, cx));
            })
            .unwrap();
        assert_eq!(captured_row_resizes(&events)[0].2, MAX_AUTO_ROW_HEIGHT_PX);
    }

    #[gpui::test]
    fn autofit_empty_row_is_default(cx: &mut TestAppContext) {
        // A row with no published cells autofits to the default height (no shrink below default).
        let (sources, _s) = autofit_row_sources(&[], &[]);
        let (g, window, events) = recording_over(cx, sources);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| grid.autofit_row(7, window, cx));
            })
            .unwrap();
        let resizes = captured_row_resizes(&events);
        assert_eq!(resizes.len(), 1);
        assert_eq!(resizes[0], (7, 7, DEFAULT_ROW_HEIGHT_PX));
    }

    #[gpui::test]
    fn autofit_multi_row_selection_fits_each(cx: &mut TestAppContext) {
        // A divider double-clicked inside a full-row multi-row selection autofits every row in the run
        // — one `ResizeCommitted{Row}` per row.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.select_row(1, false, window, cx);
                    grid.select_row(3, true, window, cx);
                    assert_eq!(grid.resize_run_for(RowOrCol::Row, 2), (1, 3));
                    grid.autofit_row(2, window, cx);
                });
            })
            .unwrap();
        let rows: Vec<u32> = captured_row_resizes(&events)
            .iter()
            .map(|(s, e, _)| {
                assert_eq!(s, e, "each autofit resize targets a single row");
                *s
            })
            .collect();
        assert_eq!(rows, vec![1, 2, 3], "each selected row autofit once");
    }

    #[gpui::test]
    fn autofit_row_under_select_all_fits_only_the_divider_row(cx: &mut TestAppContext) {
        // Select-all is not a full-row selection, so a row divider resolves to just its own row; the
        // whole-sheet guard mirrors the column autofit. Exactly one resize, for the divider's row.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.select_all(window, cx);
                    grid.autofit_row(4, window, cx);
                });
            })
            .unwrap();
        let resizes = captured_row_resizes(&events);
        assert_eq!(
            resizes.len(),
            1,
            "whole-sheet autofit emits exactly one row resize"
        );
        assert_eq!(resizes[0].0, 4);
        assert_eq!(resizes[0].1, 4, "only the divider row is autofit");
    }

    #[gpui::test]
    fn commit_resize_noop_is_skipped(cx: &mut TestAppContext) {
        // A divider click with no drag (`current_px == start_px`) must not freeze a preview or emit a
        // redundant `SetColumnWidths` — otherwise a double-click-to-autofit gains a spurious pre-step.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.commit_resize(
                        ResizeDrag {
                            axis: RowOrCol::Col,
                            index: 2,
                            start_px: 100.0,
                            current_px: 100.0,
                            run: (2, 2),
                            origin_coord: 0.0,
                        },
                        window,
                        cx,
                    );
                    assert!(
                        grid.resize_preview.is_none(),
                        "an unchanged resize freezes no preview"
                    );
                });
            })
            .unwrap();
        assert!(
            captured_resizes(&events).is_empty(),
            "an unchanged resize emits no ResizeCommitted"
        );
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

    // ---- Cell-area right-click context menu (`functional_spec.md §2`, Phase 5) -------------

    /// A `CellMenu` over the B2:D4 (rows 1..=3, cols 1..=3) selection with nothing blocked, for the
    /// pure item-mapping test.
    fn cell_menu_b2_d4(paste_enabled: bool) -> CellMenu {
        CellMenu {
            x: 0.0,
            y: 0.0,
            range: CellRange::new(CellRef::new(1, 1), CellRef::new(3, 3)),
            paste_enabled,
            insert_row_above_blocked: false,
            insert_row_below_blocked: false,
            delete_rows_blocked: false,
            insert_col_left_blocked: false,
            insert_col_right_blocked: false,
            delete_cols_blocked: false,
        }
    }

    #[test]
    fn cell_menu_items_map_to_the_right_events() {
        let items = GridView::cell_menu_items(&cell_menu_b2_d4(false));
        // Clipboard group (Cut/Copy always enabled; Paste + Paste-values follow `paste_enabled`).
        assert!(matches!(
            &items[0],
            Some((l, true, GridEvent::Copy { cut: true })) if l == "Cut"
        ));
        assert!(matches!(
            &items[1],
            Some((l, true, GridEvent::Copy { cut: false })) if l == "Copy"
        ));
        assert!(matches!(&items[2], Some((_, false, GridEvent::Paste))));
        assert!(matches!(
            &items[3],
            Some((_, false, GridEvent::PasteValues))
        ));
        assert!(matches!(
            &items[4],
            Some((l, true, GridEvent::ClearCells(r)))
                if l == "Clear contents" && *r == CellRange::new(CellRef::new(1, 1), CellRef::new(3, 3))
        ));
        // Separator between the clipboard and structural groups.
        assert!(items[5].is_none());
        // Structural group: rows above/below/delete, then columns left/right/delete — scoped to the
        // selection's span (3 rows starting at row 1; below inserts past the run at row 4).
        assert!(matches!(
            &items[6],
            Some((l, true, GridEvent::InsertRows { at: 1, count: 3 })) if l == "Insert 3 rows above"
        ));
        assert!(matches!(
            &items[7],
            Some((l, true, GridEvent::InsertRows { at: 4, count: 3 })) if l == "Insert 3 rows below"
        ));
        assert!(matches!(
            &items[8],
            Some((l, true, GridEvent::DeleteRows { at: 1, count: 3 })) if l == "Delete 3 rows"
        ));
        assert!(matches!(
            &items[9],
            Some((l, true, GridEvent::InsertColumns { at: 1, count: 3 })) if l == "Insert 3 columns left"
        ));
        assert!(matches!(
            &items[10],
            Some((l, true, GridEvent::InsertColumns { at: 4, count: 3 })) if l == "Insert 3 columns right"
        ));
        assert!(matches!(
            &items[11],
            Some((l, true, GridEvent::DeleteColumns { at: 1, count: 3 })) if l == "Delete 3 columns"
        ));
    }

    #[test]
    fn cell_menu_items_single_cell_labels_and_paste_and_block_flags() {
        // A single-cell A1 selection singularizes the row/column labels.
        let single = CellMenu {
            range: CellRange::single(CellRef::new(0, 0)),
            paste_enabled: true,
            // Block delete-rows + insert-column-left to prove the guard disables the right items.
            delete_rows_blocked: true,
            insert_col_left_blocked: true,
            ..cell_menu_b2_d4(true)
        };
        let items = GridView::cell_menu_items(&single);
        // Paste + Paste-values enabled when the clipboard has text.
        assert!(matches!(&items[2], Some((_, true, GridEvent::Paste))));
        assert!(matches!(&items[3], Some((_, true, GridEvent::PasteValues))));
        assert!(matches!(
            &items[6],
            Some((l, _, GridEvent::InsertRows { at: 0, count: 1 })) if l == "Insert 1 row above"
        ));
        // Blocked ops render disabled (enabled == false).
        assert!(matches!(
            &items[8],
            Some((l, false, GridEvent::DeleteRows { .. })) if l == "Delete 1 row"
        ));
        assert!(matches!(
            &items[9],
            Some((l, false, GridEvent::InsertColumns { .. })) if l == "Insert 1 column left"
        ));
    }

    fn header_menu_fixture(
        axis: RowOrCol,
        run: (u32, u32),
        hide_blocked: bool,
        unhide_run: Option<(u32, u32)>,
        hidden_in_run: u32,
    ) -> HeaderMenu {
        HeaderMenu {
            axis,
            run,
            x: 0.0,
            y: 0.0,
            insert_before_blocked: false,
            insert_after_blocked: false,
            delete_blocked: false,
            hide_blocked,
            unhide_run,
            hidden_in_run,
        }
    }

    #[test]
    fn header_menu_items_include_hide_and_unhide() {
        use freecell_core::limits;
        // A 3-column run C:E with nothing hidden → Hide (all 3 newly-hidden) enabled, Unhide disabled.
        let items = GridView::header_menu_items(&header_menu_fixture(
            RowOrCol::Col,
            (2, 4),
            false,
            None,
            0,
        ));
        assert!(matches!(
            &items[3],
            (l, true, GridEvent::HideColumns { at: 2, count: 3 }) if l == "Hide 3 columns"
        ));
        assert!(matches!(
            &items[4],
            (l, false, GridEvent::UnhideColumns { .. }) if l == "Unhide columns"
        ));

        // Same run but D (3) hidden → Unhide enabled (1 hidden), scoped to the hidden span; Hide now
        // counts only the 2 newly-hidden (C, E), while its event still targets the whole run.
        let items = GridView::header_menu_items(&header_menu_fixture(
            RowOrCol::Col,
            (2, 4),
            false,
            Some((3, 3)),
            1,
        ));
        assert!(matches!(
            &items[3],
            (l, true, GridEvent::HideColumns { at: 2, count: 3 }) if l == "Hide 2 columns"
        ));
        assert!(matches!(
            &items[4],
            (l, true, GridEvent::UnhideColumns { at: 3, count: 1 }) if l == "Unhide 1 column"
        ));

        // SPARSE case (the reviewer's fix): a run C:G where only D and F are hidden → the span is
        // [3, 5] (width 3) but the label counts the ACTUAL 2 hidden ("Unhide 2 columns"); the event
        // still clears the whole span.
        let items = GridView::header_menu_items(&header_menu_fixture(
            RowOrCol::Col,
            (2, 6),
            false,
            Some((3, 5)),
            2,
        ));
        assert!(matches!(
            &items[4],
            (l, true, GridEvent::UnhideColumns { at: 3, count: 3 }) if l == "Unhide 2 columns"
        ));

        // Select-All → Hide is blocked (would hide every visible row) → the Hide item is disabled.
        let items = GridView::header_menu_items(&header_menu_fixture(
            RowOrCol::Row,
            (0, limits::MAX_ROWS - 1),
            true,
            None,
            0,
        ));
        assert!(matches!(
            &items[3],
            (l, false, GridEvent::HideRows { at: 0, .. }) if l == "Hide 1048576 rows"
        ));
    }

    #[test]
    fn hide_unhide_flags_compute_span_count_and_hide_all_guard() {
        use freecell_core::limits;
        use std::collections::BTreeSet;
        let total = limits::MAX_ROWS;
        // Nothing hidden in the run → Hide allowed, Unhide None, count 0.
        let (hb, ur, n) = hide_unhide_flags((2, 4), &BTreeSet::new(), total);
        assert!(!hb && ur.is_none() && n == 0);
        // Hidden 3 and 7 within run 2..=9 → the unhide span is the minimal [3, 7] but the count is 2
        // (the sparse case: span width 5, only 2 actually hidden).
        let hidden: BTreeSet<u32> = [3u32, 7].into_iter().collect();
        let (_, ur, n) = hide_unhide_flags((2, 9), &hidden, total);
        assert_eq!(ur, Some((3, 7)));
        assert_eq!(n, 2);
        // Select-All over the whole axis with nothing hidden → hiding all → blocked.
        let (hb, _, _) = hide_unhide_flags((0, total - 1), &BTreeSet::new(), total);
        assert!(hb);
        // Select-All with some already hidden → still hides every remaining visible → blocked.
        let (hb, _, _) = hide_unhide_flags((0, total - 1), &hidden, total);
        assert!(hb);
        // A partial run that leaves visible tracks is never hide-blocked.
        let (hb, _, _) = hide_unhide_flags((0, 4), &BTreeSet::new(), total);
        assert!(!hb);
        // A smaller-than-select-all run that still covers every remaining visible track is blocked
        // too (most of the axis already hidden) — the softened-doc case.
        let mut almost_all: BTreeSet<u32> = (0..total).collect();
        almost_all.remove(&5);
        almost_all.remove(&6);
        let (hb, _, _) = hide_unhide_flags((5, 6), &almost_all, total);
        assert!(hb, "hiding the last 2 visible tracks is blocked");
    }

    /// Whether any item emits a row-structural (Insert/Delete rows) command.
    fn has_row_ops(items: &[Option<(String, bool, GridEvent)>]) -> bool {
        items.iter().any(|e| {
            matches!(
                e,
                Some((
                    _,
                    _,
                    GridEvent::InsertRows { .. } | GridEvent::DeleteRows { .. }
                ))
            )
        })
    }

    /// Whether any item emits a column-structural (Insert/Delete columns) command.
    fn has_col_ops(items: &[Option<(String, bool, GridEvent)>]) -> bool {
        items.iter().any(|e| {
            matches!(
                e,
                Some((
                    _,
                    _,
                    GridEvent::InsertColumns { .. } | GridEvent::DeleteColumns { .. }
                ))
            )
        })
    }

    /// Whether the always-present clipboard group (Cut/Copy/Paste/Paste-values/Clear) is intact.
    fn has_clipboard_group(items: &[Option<(String, bool, GridEvent)>]) -> bool {
        items
            .iter()
            .any(|e| matches!(e, Some((_, _, GridEvent::Copy { cut: true }))))
            && items
                .iter()
                .any(|e| matches!(e, Some((_, _, GridEvent::Paste))))
            && items
                .iter()
                .any(|e| matches!(e, Some((_, _, GridEvent::PasteValues))))
            && items
                .iter()
                .any(|e| matches!(e, Some((_, _, GridEvent::ClearCells(_)))))
    }

    #[test]
    fn cell_menu_full_column_selection_suppresses_row_ops() {
        use freecell_core::limits;
        // A full-column selection is stored as rows 0..=MAX_ROWS-1 (col 2). Its row-structural items
        // would be a sheet-wiping "Delete 1048576 rows" — they must be absent; the column items
        // (width 1) survive with count 1 (Excel: a column selection's ops are column-oriented).
        let menu = CellMenu {
            range: CellRange::new(CellRef::new(0, 2), CellRef::new(limits::MAX_ROWS - 1, 2)),
            ..cell_menu_b2_d4(false)
        };
        let items = GridView::cell_menu_items(&menu);
        assert!(
            !has_row_ops(&items),
            "row-structural items dropped for a full column"
        );
        assert!(has_col_ops(&items), "column-structural items remain");
        assert!(has_clipboard_group(&items));
        // The surviving column ops are scoped to the single selected column.
        assert!(items.iter().any(|e| matches!(
            e,
            Some((l, _, GridEvent::DeleteColumns { at: 2, count: 1 })) if l == "Delete 1 column"
        )));
    }

    #[test]
    fn cell_menu_full_row_selection_suppresses_column_ops() {
        use freecell_core::limits;
        // Symmetric: a full-row selection (cols 0..=MAX_COLS-1, row 3) drops the column items and
        // keeps the row items (height 1).
        let menu = CellMenu {
            range: CellRange::new(CellRef::new(3, 0), CellRef::new(3, limits::MAX_COLS - 1)),
            ..cell_menu_b2_d4(false)
        };
        let items = GridView::cell_menu_items(&menu);
        assert!(
            !has_col_ops(&items),
            "column-structural items dropped for a full row"
        );
        assert!(has_row_ops(&items), "row-structural items remain");
        assert!(has_clipboard_group(&items));
        assert!(items.iter().any(|e| matches!(
            e,
            Some((l, _, GridEvent::DeleteRows { at: 3, count: 1 })) if l == "Delete 1 row"
        )));
    }

    #[test]
    fn cell_menu_whole_sheet_selection_suppresses_both_structural_sets() {
        use freecell_core::limits;
        // Select-all spans all rows AND all columns → both structural sets are destructive; only the
        // clipboard group remains (no separator either, since there is nothing after it).
        let menu = CellMenu {
            range: CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(limits::MAX_ROWS - 1, limits::MAX_COLS - 1),
            ),
            ..cell_menu_b2_d4(true)
        };
        let items = GridView::cell_menu_items(&menu);
        assert!(
            !has_row_ops(&items) && !has_col_ops(&items),
            "no structural ops on the whole sheet"
        );
        assert!(has_clipboard_group(&items));
        assert!(
            items.iter().all(|e| e.is_some()),
            "no dangling separator when both structural sets are suppressed"
        );
    }

    #[gpui::test]
    fn right_click_cell_outside_selection_moves_and_opens_menu(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // The default selection is A1. A right-click deep in the cell body (y > 24, x >
                    // gutter) is outside it → the selection collapses to the clicked cell first.
                    events.borrow_mut().clear();
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    let menu = grid
                        .cell_menu
                        .expect("a cell right-click opens the cell menu");
                    assert!(
                        menu.range.is_single(),
                        "an outside click collapses to one cell"
                    );
                    assert_ne!(
                        menu.range.start,
                        CellRef::new(0, 0),
                        "the selection moved off A1"
                    );
                    assert!(
                        events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "moving the selection emits SelectionChanged"
                    );
                    // Only one menu is ever open.
                    assert!(grid.header_menu.is_none() && grid.chart_menu.is_none());
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn right_click_cell_inside_selection_keeps_it(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Select the whole sheet so any cell click lands inside the (multi-cell)
                    // selection; Excel then keeps it so the menu acts on the whole selection.
                    grid.select_all(window, cx);
                    let before = *grid.selection();
                    events.borrow_mut().clear();
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    let menu = grid.cell_menu.expect("the cell menu opened");
                    assert_eq!(
                        *grid.selection(),
                        before,
                        "an inside click keeps the selection"
                    );
                    assert!(
                        !menu.range.is_single(),
                        "the whole-sheet selection is preserved"
                    );
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "keeping the selection emits no SelectionChanged"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn right_click_bounded_cell_inside_selection_keeps_it(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // A representative (non-degenerate) inside-click: select B2:D4, then right-click
                    // C3 (inside) → the multi-cell selection is kept, not collapsed to C3.
                    let b2_d4 = SelectionModel {
                        anchor: CellRef::new(1, 1),
                        active: CellRef::new(3, 3),
                    };
                    grid.set_selection(b2_d4, cx);
                    events.borrow_mut().clear();
                    grid.open_cell_menu(CellRef::new(2, 2), 0.0, 0.0, &[], window, cx);
                    let menu = grid.cell_menu.expect("the cell menu opened");
                    assert_eq!(*grid.selection(), b2_d4, "an inside click keeps B2:D4");
                    assert_eq!(
                        menu.range,
                        CellRange::new(CellRef::new(1, 1), CellRef::new(3, 3)),
                        "the menu spans the kept selection, not the clicked cell"
                    );
                    assert!(
                        !events
                            .borrow()
                            .iter()
                            .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
                        "keeping the selection emits no SelectionChanged"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn cell_menu_paste_gates_on_clipboard(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // An empty clipboard disables Paste + Paste-values.
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    assert!(
                        !grid.cell_menu.expect("menu opened").paste_enabled,
                        "empty clipboard disables paste"
                    );
                    grid.close_cell_menu(cx);
                    // Seeding the system clipboard enables both paste items on the next open.
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string("1\t2".to_string()));
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    let menu = grid.cell_menu.expect("menu reopened");
                    assert!(menu.paste_enabled, "a non-empty clipboard enables paste");
                    let items = GridView::cell_menu_items(&menu);
                    assert!(matches!(&items[2], Some((_, true, GridEvent::Paste))));
                    assert!(matches!(&items[3], Some((_, true, GridEvent::PasteValues))));
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn cell_menu_escape_closes(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    assert!(grid.cell_menu.is_some());
                    grid.handle_key_down(&key_ev("escape", None, false), window, cx);
                    assert!(grid.cell_menu.is_none(), "Escape closes the cell menu");
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn cell_menu_item_click_emits_event_and_closes(cx: &mut TestAppContext) {
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    assert!(grid.cell_menu.is_some(), "the cell menu opened");
                });
            })
            .unwrap();

        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        // "Clear contents" is row index 4 in `cell_menu_items`.
        let row = vcx
            .debug_bounds("cell-menu-item-4")
            .expect("the Clear-contents row was painted");
        events.borrow_mut().clear();
        vcx.simulate_mouse_down(row.center(), MouseButton::Left, Modifiers::default());
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::ClearCells(_))),
            "clicking Clear contents emits ClearCells: {:?}",
            events.borrow()
        );
        assert!(
            vcx.update(|_w, cx| g.read(cx).cell_menu.is_none()),
            "choosing an item closes the menu"
        );
    }

    #[gpui::test]
    fn cell_menu_card_paints(cx: &mut TestAppContext) {
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 400.0, 300.0),
                        window,
                        cx,
                    );
                    assert!(grid.cell_menu.is_some(), "the cell menu opened");
                });
            })
            .unwrap();

        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cell-menu-card").is_some(),
            "the cell menu card was painted"
        );
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
    fn escape_in_focused_in_cell_editor_routes_through_capture_key_down(cx: &mut TestAppContext) {
        // Locks the routing BUG #5 depends on: with the in-cell overlay open and its input focused,
        // an Escape keystroke is intercepted by the grid root's `capture_key_down` and emitted as
        // `InCellCancel` (Tab/Shift+Tab are, in the headless harness, swallowed by gpui's focus
        // traversal before reaching the capture handler — on macOS they arrive via
        // `do_command_by_selector`; see the window-level re-entrancy test's note).
        let (g, window, events) = grid_recording(cx);
        let input = window
            .update(cx, |_root, window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx));
                g.update(cx, |grid, cx| {
                    grid.set_incell_input(input.clone(), cx);
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(3, 3)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                });
                input
            })
            .unwrap();
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, cx| input.update(cx, |i, cx| i.focus(window, cx)));
        vcx.run_until_parked();
        events.borrow_mut().clear();
        vcx.simulate_keystrokes("escape");
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::InCellCancel)),
            "Escape in the focused in-cell editor must route through capture_key_down: {:?}",
            events.borrow()
        );
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
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(3, 3)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
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

    fn mouse_down_at(pos: gpui::Point<gpui::Pixels>, click_count: usize) -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position: pos,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        }
    }

    fn mouse_move_at(pos: gpui::Point<gpui::Pixels>) -> MouseMoveEvent {
        MouseMoveEvent {
            position: pos,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        }
    }

    #[gpui::test]
    fn double_click_to_open_editor_arms_no_grid_drag(cx: &mut TestAppContext) {
        // BUG #2 root cause: the double-click that opens the in-cell editor used to arm a cell
        // selection drag. The editor overlay then `.occlude()`s the follow-up mouse-up, so that drag
        // is never cleared — and a press+drag *inside* the editor (text selection) extends the stale
        // grid drag, selecting cells and closing the editor. Opening the editor must arm no drag.
        let (g, window, _events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // A1 is the default active single selection, so this is a double-click on the
                    // already-active cell — the exact open-in-place gesture.
                    grid.mouse_down_cell(0, 0, &mouse_down_at(gpui::point(px(1.0), px(1.0)), 2), window, cx);
                    assert!(
                        !grid.has_active_drag(),
                        "opening the in-cell editor via double-click must not arm a grid selection drag"
                    );
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn move_is_ignored_while_in_cell_editor_open(cx: &mut TestAppContext) {
        // BUG #2 belt-and-braces: even if a drag becomes live while the in-cell editor is open, a
        // pointer move must not extend the grid's selection — the press+drag belongs to the editor's
        // text selection. Without the `incell_open` gate in `handle_mouse_move` the move would emit a
        // `SelectionChanged` and close the editor. (The editor is opened FIRST here, then a drag is
        // armed directly — `set_edit_state` now clears a pre-open drag, so arming after open is what
        // isolates the gate; in the real app such a press is occluded.)
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(2, 2)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    grid.mouse_down_cell(
                        2,
                        2,
                        &mouse_down_at(gpui::point(px(120.0), px(80.0)), 1),
                        window,
                        cx,
                    );
                    assert!(grid.has_active_drag(), "a single click arms a cell drag");
                    events.borrow_mut().clear();
                    // A drag move to a far cell would normally extend the selection.
                    grid.handle_mouse_move(
                        &mouse_move_at(gpui::point(px(700.0), px(500.0))),
                        window,
                        cx,
                    );
                });
            })
            .unwrap();
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
            "with the in-cell editor open, a pointer move must not extend a grid selection: {:?}",
            events.borrow()
        );
    }

    #[gpui::test]
    fn drag_armed_before_editor_open_leaves_no_phantom_after_close(cx: &mut TestAppContext) {
        // BUG #2 root fix (NIT): a drag armed *before* the editor opened must be cleared when the
        // editor opens — its mouse-up is occluded by the overlay, so the grid never clears it, and
        // after the editor closes a later hover would extend a phantom selection. `set_edit_state`
        // clears `self.drag` at the point `incell_open` becomes `Some`.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Arm a real cell drag (single click on C3).
                    grid.mouse_down_cell(
                        2,
                        2,
                        &mouse_down_at(gpui::point(px(120.0), px(80.0)), 1),
                        window,
                        cx,
                    );
                    assert!(grid.has_active_drag(), "a single click arms a cell drag");
                    // Open the editor → the drag must be cleared at the root.
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(2, 2)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    assert!(
                        !grid.has_active_drag(),
                        "opening the in-cell editor clears the pre-armed drag"
                    );
                    // Close the editor; the drag must stay cleared (no move-gate applies now).
                    grid.set_edit_state(
                        None,
                        None,
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                    assert!(
                        !grid.has_active_drag(),
                        "the drag stays cleared after the editor closes"
                    );
                    events.borrow_mut().clear();
                    grid.handle_mouse_move(
                        &mouse_move_at(gpui::point(px(700.0), px(500.0))),
                        window,
                        cx,
                    );
                });
            })
            .unwrap();
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::SelectionChanged(_))),
            "a drag armed before the editor opened must not extend a selection after it closes: {:?}",
            events.borrow()
        );
    }

    #[test]
    fn incell_font_resolves_cell_style_including_underline() {
        // BUG #4 (NIT): the overlay's hosted Input renders at the edited cell's resolved font —
        // size + family + bold/italic/underline — mirroring `cell_element` so editing is WYSIWYG.
        let families = [SharedString::from(""), SharedString::from("Georgia")];

        // A default cell (no style) → default size, no family, no character styling.
        let d = resolve_incell_font(None, &families);
        assert_eq!(d.size_px, CELL_FONT_PX);
        assert!(d.family.is_none());
        assert!(!d.bold && !d.italic && !d.underline);

        // A styled cell: 24pt, Georgia (index 1), bold + italic + underline.
        let style = RenderStyle {
            bold: true,
            italic: true,
            underline: true,
            font_size_q: 24 * 4,
            font_family: 1,
            ..RenderStyle::default()
        };
        let r = resolve_incell_font(Some(style), &families);
        assert_eq!(r.size_px, 24.0 * 96.0 / 72.0);
        assert_eq!(r.family.as_deref(), Some("Georgia"));
        assert!(r.bold, "bold resolved");
        assert!(r.italic, "italic resolved");
        assert!(r.underline, "underline resolved (was previously dropped)");
    }

    #[test]
    fn incell_input_floors_control_at_line_box() {
        // BUG A: the hosted single-line `Input` must never be shorter than its own font-scaled line
        // box, else a large glyph is clipped. The floor guards the case where a large font sits in a
        // row that was NOT grown to fit it — a user-shrunk row, or a large font applied where the
        // proportional font-size auto-grow (`cache::autofit_row_ironcalc_px`) did not run. A
        // proportionally grown row always clears the line box, so the floor is a no-op there; here
        // we feed a SHORT row so `h - 4` lands below the line box and the floor must engage.
        let font_px: f32 = 24.0 * 96.0 / 72.0; // 24 pt -> 32 px, as in resolve_incell_font.
        let line_expect = font_px * 1.25; // 40 px

        // A 24 px row (the default) holding a 24 pt font: inner height h-4 = 20 px, well below the
        // 40 px line box — the un-grown / shrunk case the floor exists for.
        let h = 24.0_f32;
        assert!(
            h - 4.0 < line_expect,
            "precondition: without the floor control_h would be h-4 = {} px, below the line box {line_expect} px",
            h - 4.0
        );

        let (control_h, line_h) = incell_input_geometry(h, font_px);
        assert!((line_h - line_expect).abs() < 1e-4, "line_h = {line_h}");
        // The floor engages: the control is lifted to the line box (40), NOT left at h-4 (20).
        // Dropping `.max(line_h)` from `incell_input_geometry` makes THIS assertion fail
        // (control_h would be 20 < 40) — it is the floor-discriminating check.
        assert!(
            (control_h - line_h).abs() < 1e-4,
            "floor must lift control_h to the line box, got {control_h} (line box {line_h})"
        );
        assert!(line_h <= control_h + 1e-4);

        // A tall/plain cell where h-4 exceeds the line box keeps the full inner height (no floor).
        let (tall_c, tall_l) = incell_input_geometry(80.0, font_px);
        assert!((tall_c - 76.0).abs() < 1e-6, "tall control_h = {tall_c}");
        assert!(
            tall_l <= tall_c,
            "line box ({tall_l}) must fit the tall control ({tall_c})"
        );

        // A default-font cell (13 px in a 24 px row): inner height 20 > line box 16.25 → no floor.
        let (dc, dl) = incell_input_geometry(24.0, 13.0);
        assert!((dc - 20.0).abs() < 1e-6, "default control_h = {dc}");
        assert!(
            dl <= dc,
            "default line box ({dl}) must fit the default control ({dc})"
        );
    }

    #[test]
    fn incell_editor_size_grows_and_clamps() {
        // The pure sizing math behind the grow-right / grow-down in-cell editor
        // (`DECISIONS_TO_REVIEW.md`). Cell at content-local x=100, 64×24 px, min box 80, viewport
        // right edge at 800, wrap-on cap 240.
        let (cx, cw, ch, cont_w, min_w, max_h) = (100.0, 64.0, 24.0, 800.0, 80.0, 240.0);

        // Wrap-off, text that fits the base box: stays at the base (min) width, cell height.
        let (w, h) = incell_editor_size(cx, cw, ch, cont_w, min_w, false, 20.0, 12.0, 0.0, max_h);
        assert!(
            (w - min_w).abs() < 1e-3,
            "short text keeps the base width, got {w}"
        );
        assert!(
            (h - ch).abs() < 1e-6,
            "wrap-off height stays the cell height, got {h}"
        );

        // Wrap-off, long text: width grows to measured + slack, wider than the cell.
        let (w2, _) = incell_editor_size(cx, cw, ch, cont_w, min_w, false, 300.0, 12.0, 0.0, max_h);
        assert!(
            (w2 - 312.0).abs() < 1e-3,
            "grows to measured+slack, got {w2}"
        );
        assert!(w2 > cw, "editor wider than the cell");

        // Wrap-off, text far wider than the viewport: clamped at the content viewport's right edge —
        // never drawn past `content_w` (into the row header / outside the grid).
        let (w3, _) =
            incell_editor_size(cx, cw, ch, cont_w, min_w, false, 5000.0, 12.0, 0.0, max_h);
        assert!(
            (w3 - (cont_w - cx)).abs() < 1e-3,
            "clamped to the viewport edge, got {w3}"
        );
        assert!(
            cx + w3 <= cont_w + 1e-3,
            "never past the content right edge"
        );

        // Anchored cell straddling / past the right edge (avail < base): keep the base box; the
        // content layer's `overflow_hidden` does the clipping.
        let (w4, _) = incell_editor_size(
            cont_w - 10.0,
            cw,
            ch,
            cont_w,
            min_w,
            false,
            5000.0,
            12.0,
            0.0,
            max_h,
        );
        assert!(
            (w4 - min_w).abs() < 1e-3,
            "edge cell keeps the base width, got {w4}"
        );

        // Wrap-on: width stays the cell width (floored at min); height grows to the wrapped height.
        let (ww, wh) =
            incell_editor_size(cx, 120.0, ch, cont_w, min_w, true, 0.0, 0.0, 96.0, max_h);
        assert!(
            (ww - 120.0).abs() < 1e-6,
            "wrap-on keeps the cell width, got {ww}"
        );
        assert!(
            (wh - 96.0).abs() < 1e-6,
            "wrap-on grows to the wrapped height, got {wh}"
        );
        assert!(wh > ch, "editor taller than the cell");

        // Wrap-on cap: a pathologically tall wrapped height is capped at max_h (Phase 7's cap).
        let (_, wh2) = incell_editor_size(
            cx, 120.0, ch, cont_w, min_w, true, 0.0, 0.0, 10_000.0, max_h,
        );
        assert!(
            (wh2 - max_h).abs() < 1e-6,
            "wrap-on height capped at max_h, got {wh2}"
        );
    }

    #[gpui::test]
    fn incell_editor_grows_right_for_long_text(cx: &mut TestAppContext) {
        // Editing a long string in a wrap-off cell grows the editor box WIDER than the cell (it grows
        // with the text); a short string keeps it at the base size. On close the overlay is gone.
        let (g, window, _events) = grid_recording(cx);
        let input = window
            .update(cx, |_root, window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx));
                g.update(cx, |grid, cx| {
                    grid.set_incell_input(input.clone(), cx);
                    grid.set_edit_state(
                        None,
                        Some(CellRef::new(3, 3)),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                });
                input
            })
            .unwrap();
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);

        // Short live text → base box.
        vcx.update(|window, cx| {
            input.update(cx, |i, cx| i.set_value("x", window, cx));
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    Some(CellRef::new(3, 3)),
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            });
        });
        vcx.run_until_parked();
        let base = vcx
            .debug_bounds("in-cell-editor")
            .expect("the in-cell editor overlay was painted");

        // Long live text → grows wider than the base.
        vcx.update(|window, cx| {
            input.update(cx, |i, cx| {
                i.set_value(
                    "a really long label that overflows a narrow column and grows the editor",
                    window,
                    cx,
                )
            });
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    Some(CellRef::new(3, 3)),
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            });
        });
        vcx.run_until_parked();
        let grown = vcx
            .debug_bounds("in-cell-editor")
            .expect("the in-cell editor overlay was painted");
        assert!(
            grown.size.width > base.size.width,
            "a long string must grow the wrap-off editor wider than a short one: {:?} vs {:?}",
            grown.size.width,
            base.size.width
        );
        // Height unchanged (wrap-off grows only rightward).
        assert!(
            (grown.size.height.as_f32() - base.size.height.as_f32()).abs() < 0.5,
            "wrap-off editor height must not change when it grows right"
        );

        // Cancel closes the overlay — normal rendering resumes (no persistent overlay).
        vcx.update(|_window, cx| {
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    None,
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            })
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("in-cell-editor").is_none(),
            "closing the in-cell editor must remove the overlay"
        );
    }

    #[gpui::test]
    fn incell_editor_grows_down_for_wrapped_text(cx: &mut TestAppContext) {
        // Editing a wrap-ON cell grows the editor box TALLER for wrapped text (not wider) — the box
        // previews the committed multi-line footprint. Commit closes it.
        let (sources, _sheet) = wrap_sources(80.0, None);
        let (g, window, _events) = recording_over(cx, sources);
        let cell = CellRef::new(1, 1); // B2 — the wrap-on cell in `wrap_sources`.
        let input = window
            .update(cx, |_root, window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx));
                g.update(cx, |grid, cx| {
                    grid.set_incell_input(input.clone(), cx);
                    grid.set_edit_state(
                        None,
                        Some(cell),
                        None,
                        false,
                        None,
                        None,
                        false,
                        false,
                        Vec::new(),
                        cx,
                    );
                });
                input
            })
            .unwrap();
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);

        vcx.update(|window, cx| {
            input.update(cx, |i, cx| i.set_value("x", window, cx));
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    Some(cell),
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            });
        });
        vcx.run_until_parked();
        let base = vcx
            .debug_bounds("in-cell-editor")
            .expect("the in-cell editor overlay was painted");

        vcx.update(|window, cx| {
            input.update(cx, |i, cx| i.set_value(WRAP_LONG, window, cx));
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    Some(cell),
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            });
        });
        vcx.run_until_parked();
        let grown = vcx
            .debug_bounds("in-cell-editor")
            .expect("the in-cell editor overlay was painted");
        assert!(
            grown.size.height > base.size.height,
            "a wrap-on cell must grow the editor taller for wrapped text: {:?} vs {:?}",
            grown.size.height,
            base.size.height
        );
        // Grows DOWN, not right: width unchanged (stays the cell width).
        assert!(
            (grown.size.width.as_f32() - base.size.width.as_f32()).abs() < 0.5,
            "wrap-on editor keeps its width; only height grows"
        );

        // Commit closes the overlay.
        vcx.update(|_window, cx| {
            g.update(cx, |grid, cx| {
                grid.set_edit_state(
                    None,
                    None,
                    None,
                    false,
                    None,
                    None,
                    false,
                    false,
                    Vec::new(),
                    cx,
                )
            })
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("in-cell-editor").is_none(),
            "committing the in-cell editor must remove the overlay"
        );
    }

    #[gpui::test]
    fn header_menu_padding_click_keeps_menu_open(cx: &mut TestAppContext) {
        // BUG A/B (header insert/delete menu): a mouse-DOWN on the menu card's padding ring — not on
        // an item row (those `stop_propagation`) — must not fall through to the dismiss backdrop and
        // close the menu without acting. Verified to fail without the card `.occlude()`.
        let (g, window, events) = grid_recording(cx);
        window
            .update(cx, |_root, window, cx| {
                g.update(cx, |grid, cx| {
                    // Open the column-header menu (col-header strip: y < 24, x past the gutter).
                    grid.handle_right_mouse_down(
                        &mouse_ev(MouseButton::Right, 60.0, 10.0),
                        window,
                        cx,
                    );
                    assert!(grid.header_menu.is_some(), "the header menu opened");
                });
            })
            .unwrap();

        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("header-menu-card")
            .expect("the header menu card was painted");
        events.borrow_mut().clear();
        // The card's top-left padding corner (inside the p(4) ring, above the first item row).
        let pad = gpui::point(card.origin.x + px(1.0), card.origin.y + px(1.0));
        vcx.simulate_mouse_down(pad, MouseButton::Left, Modifiers::default());

        assert!(
            vcx.update(|_w, cx| g.read(cx).header_menu.is_some()),
            "a press on the menu's padding must not dismiss it"
        );
        assert!(
            !events.borrow().iter().any(|e| matches!(
                e,
                GridEvent::InsertColumns { .. } | GridEvent::DeleteColumns { .. }
            )),
            "a press on the menu padding dispatches no structure command: {:?}",
            events.borrow()
        );
    }
}
