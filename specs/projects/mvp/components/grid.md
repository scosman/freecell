---
status: draft
---

# Component: Grid (`freecell-app::grid`)

The custom spreadsheet surface — the one component we design and build properly. A raw
GPUI view that renders headers, gridlines, cells, and selection for an Excel-max sheet
at 120 fps, reading only from the resident caches and the published viewport. Direct
port-and-extend of the proven POC (`experiments/04-ui-poc/raw-gpui/src/grid.rs`);
visual spec in `ui_design.md §3.3`.

## Purpose and scope

**Does:** virtualized 2D cell rendering; scroll (wheel/trackpad + scrollbars);
mouse selection (click/drag/shift-click); keyboard navigation dispatch; loading
overlay; per-sheet scroll/selection restore.

**Does not:** talk to IronCalc (ever); own document state; edit text (the data row
does); draw chrome outside the grid rect; number formatting (display strings arrive
pre-formatted).

## Public interface

```rust
pub struct GridView { /* gpui Entity */ }

pub struct GridDataSources {                    // injected by WorkbookWindow
    pub publication: Arc<ArcSwapish<Publication>>, // worker-swapped snapshot (see engine_worker.md)
    pub caches: Arc<RwLock<SheetCaches>>,          // geometry + styles (see style_cache.md)
    pub generation: Arc<AtomicU64>,
}

impl GridView {
    pub fn new(sources: GridDataSources, events: GridEventSink, cx: …) -> Self;
    pub fn set_active_sheet(&mut self, sheet: SheetId, cx: …);   // restores scroll+selection
    pub fn selection(&self) -> &SelectionModel;
    pub fn set_loading(&mut self, loading: Option<String>, cx: …); // "Opening name…"
    pub fn scroll_cell_into_view(&mut self, row: u32, col: u32, cx: …);
}

pub enum GridEvent {                             // → WorkbookWindow
    SelectionChanged(SelectionModel),            // drives data row + action row + ref box
    ViewportChanged { rows: Range<u32>, cols: Range<u32> }, // pre-overscan; window forwards →  worker SetViewport with 3× overscan
    EditCommitRequested,                         // click-away while data row is editing
}
```

Keyboard: the grid registers GPUI key bindings (arrows, Shift+arrows, Cmd+arrow,
Tab/Enter variants, Page, Home, Delete) that mutate `SelectionModel` via
`freecell-core::selection::apply_motion(sel, motion, sheet_dims) -> SelectionModel`
(pure function, unit-tested on Linux) and emit `SelectionChanged`. Delete emits a
`ClearCells` request via the event sink. Cmd+B/I/U are bound at window level, not here.

## Internal design

### State

```rust
struct GridState {
    scroll: HashMap<SheetId, Point<f64>>,      // px offsets per sheet
    selection: HashMap<SheetId, SelectionModel>,
    active_sheet: SheetId,
    drag: Option<DragState>,                   // anchor cell + last hovered cell
    loading: Option<String>,
}
```

Geometry questions (offsets, sizes, hit-testing, visible ranges) all answer from the
`GeometryCache`'s two `freecell-core::Axis` values (ported POC `layout.rs`: BLOCK=512
two-level prefix sums; `offset_of`, `size_of`, `index_at`, `visible_range(scroll,
extent, overscan)`, `total()`).

### Render pass (per frame — the hot path)

1. Read viewport px size; subtract header strip (col header height 24 px, row header
   width 48 px min — widens to fit the widest visible row label +8 px padding).
2. `RwLock::read()` the caches once; copy the two `Axis` handles (`Arc<Axis>` inside
   the cache so the guard drops immediately — no lock held during layout).
3. `rows = row_axis.visible_range(scroll_y, content_h, RENDER_OVERSCAN)`, same for
   cols. `RENDER_OVERSCAN = 2` (render overscan is small; the *publication* overscan
   of ~3× viewport is the worker's concern).
4. Look up the current `Publication` (one atomic load). Build an index
   `HashMap<(u32,u32), &PublishedCell>` per frame — capped by visible cells (~2k),
   reused allocation (kept in state, cleared per frame).
5. Emit absolutely-positioned elements, in draw order:
   - **Cell layer**: for each visible (r, c): background div at
     `(col_axis.offset_of(c) - scroll_x + row_hdr_w, row_axis.offset_of(r) -
     scroll_y + col_hdr_h)`, size `(size_of(c), size_of(r))`; `bg =` style fill or
     white; 1 px right+bottom borders in gridline grey `#E2E2E2` (fill paints over —
     borders belong to the filled cell so a fill covers its own gridlines, Excel-look);
     text from `PublishedCell.display_text` with `RenderStyle` attrs (13 px bundled
     Inter, bold / italic / underline, text color = format color override or
     near-black `#1F1F1F`,
     `overflow_hidden` + `whitespace_nowrap`, align per style/type, 4 px h-padding,
     v-centered). Cells with no publication entry and no style = plain white cell (one
     shared cheap element). Cells outside publication coverage (beyond-overscan
     mid-eval) render style-only with blank text.
   - **Selection layer**: range overlay div(s) (accent @ 10% alpha) clipped to the
     range rect minus the active cell; 2 px accent border div for the range rect; 2 px
     accent border for the active cell. Computed from the same Axis math.
   - **Header layer** (fixed): row-header gutter, col-header strip, corner cap;
     selected rows/cols headers get darker tint + 2 px accent edge line. Header cells
     are divs like the POC (`header_cell` styling: `#F5F5F5` bg, `#D9D9D9` hairlines,
     11.5 px medium text, centered; col labels via ported `column_label`).
   - **Scrollbars**: two overlay thumbs sized `viewport/total`, positioned
     `scroll/total`; MVP custom-drawn (two rounded rects + drag handling) — simpler
     than adapting gpui-component's scrollbar to external virtual extent. Always
     visible.
   - **Loading overlay** when `loading.is_some()`: translucent white sheet + centered
     gpui-component spinner + label.
6. No allocation proportional to sheet size anywhere; per-frame allocations bounded by
   visible cell count and reused where easy.

### Input

- `on_scroll_wheel`: convert delta (line-based deltas × `window.line_height()` like
  the POC), clamp each axis to `[0, axis.total() - content_extent].max(0)`, store,
  `cx.notify()`. Emit `ViewportChanged` (debounced: only when the visible index range
  actually changed).
- Mouse: hit-test via `index_at(scroll + local_px)` per axis (subtracting header
  offsets). Down on a cell → begin drag (anchor = cell, selection = single); move
  while dragging → extend range to hovered cell, auto-scroll when dragging past edges
  (fixed 20 px/frame step); up → end drag. Shift+click → extend from existing anchor.
  Click on headers/corner: no-op in MVP (P2: row/col selection). Double-click cell:
  no-op (no in-cell edit in MVP).
- Focus: the grid is focusable (GPUI `FocusHandle`); it claims focus on click and
  after commits/cancels (window shell arranges this).

### Scroll/selection restore per sheet

`set_active_sheet` swaps the maps' entries; missing entries default to origin scroll +
A1 selection. Emits `ViewportChanged` so the worker re-publishes the new sheet.

## Dependencies

Depends on: `freecell-core` only for data (Axis, SelectionModel, RenderStyle, refs,
`Publication`/`PublishedCell`, `SheetCaches` read model — all engine-free core types)
plus gpui. No `freecell-engine` dependency — the grid is buildable and render-testable
against hand-built core fixtures before the engine track lands. Depended on by:
`WorkbookWindow` (app shell), `render-tests`, perf harness.

## Test plan

Pure logic (Linux CI, in `freecell-core`): `axis_*` ported POC tests (offsets, ranges,
binary-search edges, 1M-row totals), `apply_motion_*` (each key incl. clamping at
edges, range extension, Cmd+jumps), `hit_test_*` (px→cell incl. header zones, scrolled,
variable sizes), `newly_visible_*`.

Render snapshots (macOS CI, via render-tests): `grid_empty_origin`,
`grid_headers_scrolled_deep` (row 1,000,000 labels + widened gutter),
`grid_selection_single`, `grid_selection_range`, `grid_selection_range_spans_edge`,
`grid_variable_geometry` (mixed widths/heights), `grid_loading_overlay`, plus every
`cell_*` case (render_test_harness.md).

Perf (macOS CI): POC run-test script vs the real grid + a 1M×100 styled fixture
workbook; gates per `architecture.md §4`.
