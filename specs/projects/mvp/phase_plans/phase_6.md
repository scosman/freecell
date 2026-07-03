---
status: complete
---

# Phase 6: Grid static rendering

## Overview

Build the custom raw-GPUI spreadsheet grid — the one component we design properly
(`components/grid.md`, `ui_design.md §3.3`, `architecture.md §4`). It renders headers,
gridlines, cells (fills, text attributes, alignment, clipping), variable geometry, wheel
scroll + clamping, custom macOS-style overlay scrollbars, and a loading overlay —
reading **only** from hand-built `freecell-core` fixtures (`SheetCaches` +
`Publication`), never the engine (that integration is Phase 11).

This is Track B's first grid phase. Scope is **static rendering + wheel scroll**; mouse
selection, keyboard motions, edge auto-scroll, and `ViewportChanged`-driven re-publish
wiring are Phase 8. The selection *layer* still renders here (from a `SelectionModel` in
grid state) so the render suite (Phase 7) can drive selection scenes.

The load-bearing invariant (`architecture.md §4`): the render/scroll path makes **zero
engine calls**, holds the `RwLock` only briefly to clone the two `Arc<Axis>` handles,
does **one** atomic load of the `Publication`, and allocates nothing proportional to
sheet size — only the visible viewport (+`RENDER_OVERSCAN`) is materialized as elements.

Direct port-and-extend of `experiments/04-ui-poc/raw-gpui/src/grid.rs` (absolutely
positioned divs at `Axis::offset_of` positions), adapted to the app's core read models.

## Steps

1. **Crate shape.** Add a library target to `freecell-app` (`src/lib.rs` → `pub mod
   grid;`) so `render-tests` (Phase 7) can depend on `GridView`. Keep the `freecell`
   bin. Add `arc-swap` + `parking_lot` to `freecell-app/Cargo.toml` (workspace pins).

2. **`grid/layout.rs` — pure geometry (no gpui; unit-tested).** Dimension constants
   (`COL_HEADER_H=24`, `ROW_HEADER_MIN_W=48`, `RENDER_OVERSCAN=2`, digit-width estimate,
   header label padding, scrollbar inset/thickness/min-length). Pure functions over
   `freecell_core::Axis` + `f64`:
   - `row_header_width(last_visible_row: u32) -> f32` — widens to fit the widest visible
     row label (7-digit at Excel-max) + padding, min 48.
   - `max_scroll(total, content) -> f64`; `clamp_scroll(...) -> (f64,f64)`.
   - `scrollbar_thumb(total, content, scroll, track_len) -> Option<Thumb{offset,length}>`
     — `None` when content fits; length `= max(min_len, track*viewport/total)`, offset
     `= (track-length)*scroll/max_scroll`.
   - `GridHit` + `hit_test(local_x, local_y, row_header_w, scroll, &row_axis, &col_axis)
     -> GridHit` (Corner / ColHeader / RowHeader / Cell) — px→cell incl. header zones.
   - `scroll_to_reveal(row, col, &row_axis, &col_axis, content_w, content_h, scroll)
     -> (f64,f64)` — minimal scroll so a cell is fully in view.
   - `range_overlay_rects(range, active) -> Vec<(Range<u32>,Range<u32>)>` — the range
     minus the active cell as ≤4 sub-rectangles (the Excel "white anchor" look).

3. **`grid/mod.rs` — public surface + look constants.** Colour/font constants
   (gridline `#E2E2E2`, cell text `#1F1F1F`, header bg `#F5F5F5`, hairline `#D9D9D9`,
   header text `#555555`, selected-header tint, accent `#2563EB` = gpui-component
   blue-600, selection fill 10% alpha, scrollbar grey, 13 px cell / 11.5 px header).
   `GridEvent { SelectionChanged(SelectionModel), ViewportChanged{rows,cols},
   EditCommitRequested }`. `GridEventSink` wrapping `Box<dyn Fn(&GridEvent, &mut Window,
   &mut App)>` with `new`/`noop`/`emit`. Re-export `GridView`, `GridDataSources`.

4. **`grid/view.rs` — the `GridView` entity.**
   - `GridDataSources { publication: Arc<ArcSwap<Publication>>, caches:
     Arc<RwLock<SheetCaches>>, generation: Arc<AtomicU64> }`.
   - State: per-sheet `scroll`/`selection` maps, `active_sheet`, `loading`,
     `scrollbars_visible` + `force_scrollbars` + `scroll_activity` epoch, reused
     `cell_index: HashMap<(u32,u32),usize>`, `last_viewport`, `pending_reveal`,
     `focus_handle`.
   - `new(sources, events, cx)` (active sheet defaults to the publication's sheet, A1
     selection, origin scroll); `set_active_sheet`; `selection()`; `set_loading`;
     `scroll_cell_into_view`; `set_force_scrollbars` (render-test hook).
   - `compute_layout(viewport, scroll)` — the shared read-caches-once → axes → content
     dims → visible ranges helper (used by render + scroll).
   - `handle_scroll` — line→px delta (`ScrollDelta::pixel_delta(line_height)`), clamp per
     axis, store, mark scrollbars visible + schedule a 2 s fade (`cx.spawn` +
     `background_executor().timer`, epoch-guarded), emit `ViewportChanged` when the
     visible index range changed, `notify`.
   - `impl Focusable` (grid is focusable); `impl Render` — the hot path (step 5).

5. **Render pass (`components/grid.md §Render pass`), document order = paint order:**
   1. viewport px; subtract header strip; visible ranges via `Axis::visible_range(...,
      RENDER_OVERSCAN)`; `row_header_w` from the deepest visible row.
   2. one `Publication` atomic load; rebuild the reused `cell_index` for visible cells.
   3. **Cell layer**: per visible (r,c) an absolute div — bg = style fill or white, 1 px
      right+bottom gridline borders (fill paints over them), text from `PublishedCell`
      with `RenderStyle` (bold/italic/underline, colour, alignment by style/type, 4 px
      h-pad, v-centred, `overflow_hidden`+`whitespace_nowrap`).
   4. **Selection layer**: translucent overlay rects (range − active), 2 px accent range
      border, 2 px accent active-cell border.
   5. **Header layer** (fixed, opaque, drawn last so content scrolls under it): col
      strip, row gutter, corner cap, per-track labels (`column_label` / 1-based row
      numbers); selected rows/cols get a darker tint + 2 px accent grid-facing edge.
   6. **Scrollbars**: macOS overlay thumbs (rounded grey, inset ~3 px), shown only when
      `scrollbars_visible || force_scrollbars` and content overflows.
   7. **Loading overlay**: translucent white sheet + centred gpui-component `Spinner` +
      "Opening *name*…" when `loading.is_some()`.
   Root: relative, `size_full`, white, `overflow_hidden`, `track_focus`,
   `on_scroll_wheel`.

6. **`grid/fixtures.rs` — hand-built core fixtures.** `demo_sources()` (styled small
   sheet over an Excel-max grid: a wide col, a tall row, bold / fill / italic / aligned
   cells, a currency-ish string, a clipped long string) + a demo range selection, for
   the `main.rs` demo and as a reference for Phase 7 scenes.

7. **`main.rs` demo.** Replace the Phase-1 hello-world view: mount a `GridView` over
   `demo_sources()` inside gpui-component `Root`, keep `--exit-after-ms` (the Linux
   render spike valve) + tracing init. This gives a real, capturable grid frame.

## Tests

Pure-logic unit tests (`grid/layout.rs`, run headless via `cargo test -p freecell-app`
— no window/GPU):

- `row_header_width_widens_for_deep_rows` — 48 min at shallow rows; grows for 7-digit
  Excel-max row labels.
- `clamp_scroll_bounds` — clamps to `[0, total-content]`, floors at 0 when content ≥
  total (empty/short sheet).
- `max_scroll_never_negative`.
- `scrollbar_thumb_none_when_fits` / `scrollbar_thumb_proportional_and_positioned` /
  `scrollbar_thumb_min_length` / `..._at_extremes` (offset 0 at top, `track-len` at
  bottom).
- `hit_test_zones` — corner / col-header / row-header / cell zones at origin.
- `hit_test_scrolled_variable_geometry` — px→cell correct under scroll + mixed sizes.
- `scroll_to_reveal_up/down/left/right/already_visible` — minimal scroll, clamped.
- `range_overlay_rects_single_is_empty` / `..._row` / `..._block_excludes_active` —
  sub-rectangles tile the range exactly once and never cover the active cell.
- `demo_sources_builds` — the fixture builder yields a cache + publication for its sheet
  (guards the render fixtures compile & are self-consistent).

Rendering itself is verified by the Phase-7 render suite (snapshot cases named in
`components/grid.md`), which drives this component; Phase 6 additionally does a manual
Linux capture (Xvfb + lavapipe + xrefresh, per the Phase-1 spike) to eyeball the grid.
