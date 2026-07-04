---
status: complete
---

# Phase 8: Grid interaction

## Overview

Wire input to the Phase-6 `GridView`: mouse selection (click / drag / shift-click +
edge auto-scroll), keyboard motions dispatched through the Phase-2 `apply_motion` pure
function with scroll-into-view, and the `ViewportChanged` / `SelectionChanged` /
`ClearCells` events the Phase-11 window will consume (`components/grid.md §Input`,
`ui_design.md §5–6`).

The Phase-6 render/scroll invariants stay intact: **zero engine calls** on the
input/selection/scroll path (input handlers only read the resident caches briefly for
axes + dims, exactly like `handle_scroll`), viewport-only virtualization, brief-lock
discipline. Selection/motion logic that can be pure is pushed into `freecell-core`
(`apply_motion`) and pure `grid` helpers (`grid/layout.rs`, new `grid/input.rs`) so it is
unit-tested headless on Linux; GPUI is reserved for the event plumbing.

## Steps

1. **`freecell-core/src/selection.rs` — add the "go to A1" motions.** The keyboard map
   (`ui_design.md §6`) has `Cmd/Ctrl+Home → cell A1`, which no existing `Motion` can
   express (`JumpEdge(Up)` keeps the column). Add `Motion::DocumentStart` (collapse to
   A1) and `Motion::ExtendDocumentStart` (extend to A1, keep anchor), handle them in
   `apply_motion` (`SelectionModel::single(CellRef::new(0,0))` / keep-anchor form), and
   unit-test both.

2. **`grid/input.rs` (NEW, pure — no gpui).** The key→command mapper, unit-tested
   headless:
   ```rust
   pub enum GridKeyCommand { Motion(Motion), ClearCells }
   /// `secondary` = Cmd on macOS / Ctrl on Linux (resolved by the caller from
   /// `Modifiers::secondary()`); `page_rows` = the current viewport height in rows.
   pub fn command_for_key(key: &str, shift: bool, secondary: bool, page_rows: u32)
       -> Option<GridKeyCommand>;
   ```
   Mapping (per `ui_design.md §6`): arrows → `Move`/`Extend`/`JumpEdge`/`ExtendEdge`
   (by shift × secondary); `tab`/`enter` (+shift) → `Move(Right/Left/Down/Up)`;
   `pageup`/`pagedown` (+shift) → `Page`/`ExtendPage`; `home` → `RowStart`/`ExtendRowStart`,
   `secondary+home` → `DocumentStart`/`ExtendDocumentStart`; `delete`/`backspace` →
   `ClearCells`. Unknown keys → `None` (propagate).

3. **`grid/layout.rs` — two pure input-geometry helpers + tests.**
   - `cell_at_point(local_x, local_y, row_header_w, scroll_x, scroll_y, &row_axis,
     &col_axis, content_w, content_h) -> CellRef` — clamps the grid-local point into the
     content rect (so a drag into the headers / past an edge maps to the nearest data
     cell), then `index_at` per axis, clamped to `[0, count)`. Used for drag-extend.
   - `edge_autoscroll_delta(local_x, local_y, row_header_w, content_w, content_h, step)
     -> (f64, f64)` — the per-axis scroll delta (0 inside; `±step` past a content edge)
     for drag-past-edge auto-scroll. Pure; the timer loop applies + clamps it.

4. **`grid/mod.rs` — `ClearCells` event + input constants.** Add
   `GridEvent::ClearCells(CellRange)` (the window adds the active `SheetId` →
   `Command::ClearCells` in Phase 11). Add `EDGE_AUTOSCROLL_STEP_PX = 20.0` (spec: fixed
   20 px/frame) and `AUTOSCROLL_INTERVAL_MS = 16`. `pub mod input;`.

5. **`grid/view.rs` — the GPUI event plumbing.**
   - State: `drag: Option<DragState>` (`DragState { anchor: CellRef }`), `autoscrolling:
     bool`, `autoscroll_epoch: u64`.
   - `handle_mouse_down` (Left): claim focus (`window.focus`); `hit_test`; on
     `GridHit::Cell` set selection — shift → extend from the current anchor, else single;
     begin a drag from the resulting anchor; emit `SelectionChanged`. Headers/corner =
     no-op (MVP).
   - `handle_mouse_move`: if dragging, `cell_at_point` → extend `active` (keep anchor),
     emit `SelectionChanged` when it changed; then `maybe_start_autoscroll`.
   - `handle_mouse_up` (Left): end the drag (bump `autoscroll_epoch`); schedule the
     scrollbar fade if visible.
   - Edge auto-scroll: `maybe_start_autoscroll` spawns a `cx.spawn_in(window, …)` loop
     (`background_executor().timer(AUTOSCROLL_INTERVAL_MS)`), guarded by an epoch, that
     each tick reads `window.mouse_position()`, computes `edge_autoscroll_delta`, applies
     + clamps the scroll (`clamp_scroll`), re-extends the selection to the hovered cell,
     emits debounced `ViewportChanged` + `SelectionChanged`, keeps scrollbars visible,
     and stops when the drag ends or the pointer returns inside (delta 0). This is the
     "held past the edge, no move events" case; `mouse_position()` gives the live pointer.
   - `handle_key_down`: resolve `command_for_key` from `keystroke.key` +
     `modifiers.shift`/`.secondary()` + `page_rows`. `Motion` → `apply_motion` over
     `sheet_dims()` (axis counts), store + emit `SelectionChanged`, then
     `reveal_and_announce(active)` (immediate reveal-scroll + debounced `ViewportChanged`,
     so a keyboard scroll re-publishes). `ClearCells` → emit
     `GridEvent::ClearCells(selection.range())`.
   - Helpers: `set_selection_and_emit`, `sheet_dims`, `page_rows`, `reveal_and_announce`
     (the immediate analogue of the render-time `pending_reveal`, mirroring
     `handle_scroll`'s viewport announce).
   - Wire `.on_mouse_down(Left)`, `.on_mouse_move`, `.on_mouse_up(Left)`, `.on_key_down`
     on the root div next to the existing `.on_scroll_wheel`. The render path (cells /
     selection / headers drawing) is unchanged, so existing baselines stay valid.

6. **`render-tests` — selection snapshot cases + baselines.** Add three `RenderCase`s
   capturing representative post-interaction selection states (rendered by the real
   `GridView` selection layer via the existing `selection`/`reveal` hooks — the state a
   drag/shift/scroll leaves, with the pure interaction logic unit-tested in steps 2–3):
   - `grid_selection_shift_extended` — active cell at the **top-left** of the range
     (extension up-left), exercising the "white anchor" at a corner the existing
     bottom-right cases don't cover.
   - `grid_selection_drag_extended` — a larger drag-out block (anchor top-left, active
     bottom-right).
   - `grid_selection_scrolled` — a selection scrolled so its top-left is clipped above /
     left of the viewport (top/left overlay clip + anchor off-screen, active visible),
     the complement of `grid_selection_range_spans_edge`.
   Register the three test rows in `render_suite.rs::render_cases!`; generate + eyeball +
   commit their baselines on the pinned image; run the full suite green foreground.

## Tests

Pure-logic unit tests (headless, `cargo test -p freecell-core` / `-p freecell-app`):

- `selection.rs`: `document_start_goes_to_a1`, `extend_document_start_keeps_anchor`.
- `grid/input.rs`: `arrows_map_by_shift_and_secondary` (Move/Extend/JumpEdge/ExtendEdge),
  `tab_enter_map_to_moves`, `page_keys_map`, `home_and_cmd_home`, `delete_backspace_clear`,
  `unknown_key_is_none`.
- `grid/layout.rs`: `cell_at_point_inside_and_clamped` (content, header zones, past
  edges), `cell_at_point_scrolled_variable_geometry`, `edge_autoscroll_delta_zero_inside`,
  `edge_autoscroll_delta_past_each_edge`.

Render snapshots (Linux CI, via render-tests): the three new `grid_selection_*` cases
with committed baselines, plus the whole existing suite staying green.

The GPUI event plumbing in `view.rs` (down/move/up/key wiring, the spawn_in auto-scroll
loop) is exercised end-to-end by the manual/real app and is the thin layer over the
pure, unit-tested helpers; it has no headless test seam (documented).
