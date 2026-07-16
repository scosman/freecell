---
status: complete
---

# Phase 4: Drag fill handle + series autofill

## Overview

Add the signature spreadsheet affordance: a small square **fill handle** at the
bottom-right corner of the selection, whose drag extends the selection's content into the
swept cells — **copying** a single-cell seed, or **extrapolating a series** (1,2,3…;
Jan,Feb…; Mon,Tue…) from a multi-cell seed. Direction is one dominant axis per drag
(Excel), supporting down/right AND up/left. The fill rides IronCalc's
`auto_fill_rows`/`auto_fill_columns` — which the fork already implements with progression
detection and native up/left support — seeded with the **full multi-cell block** (the key
difference from the existing ⌘D/⌘R wrappers, which force a 1×N seed = copy).

**Fork binding (read-only, NO fork change).** The fork's `auto_fill_rows(source_area,
to_row)` / `auto_fill_columns(source_area, to_column)`
(`base/src/user_model/autofill.rs:87,227`) natively support **both** directions: `to_row >
last_row` fills downward, `to_row < row1` fills upward (it flips `anchor_row`/`sign`,
reverses the seed values, and re-runs `detect_progression` so an up-fill counts the series
down). So the document method does **not** need to reverse anything — it passes the far
target edge as `to_row`/`to_column` and the fork handles up/left. One `auto_fill_*` call ⇒
one undo step (single `push_diff_list`).

## Steps

1. **`freecell-core/src/refs.rs` + `lib.rs`** — add a shared `FillAxis { Vertical,
   Horizontal }` enum (the fill's dominant axis) and export it. It is shared by the app's
   `GridEvent`, the window mapping, and the engine `Command`/`document` (the engine crate
   can't see app types), so it lives in `freecell-core` beside `CellRange`.

2. **`freecell-engine/src/document.rs`** — new `pub(crate) fn fill_drag(&mut self,
   sheet_idx, seed: CellRange, target: CellRange, axis: FillAxis) -> Result<bool, String>`:
   - Build the IronCalc source `Area` = the **full seed** (`width = seed.width()`, `height
     = seed.height()`, NOT clamped to 1) — this is what lets `detect_progression`
     extrapolate a multi-cell seed. (Single-cell seed → width=height=1 → falls through to
     `extend_to` = copy, exactly like ⌘D/⌘R.)
   - `Vertical`: `to_row` = the target's far row edge along the fill direction —
     `target.end.row` when extending **down** (target.end.row > seed.end.row), else
     `target.start.row` when extending **up**. Call `auto_fill_rows(&source, to_row + 1)`.
   - `Horizontal`: symmetric with `to_col` = `target.end.col` (right) / `target.start.col`
     (left) → `auto_fill_columns(&source, to_col + 1)`.
   - If `target` doesn't extend past `seed` along the axis (no far edge) → `Ok(false)`
     (no-op; the drag committed onto the seed). Otherwise `Ok(true)`.

3. **`freecell-engine/src/worker/protocol.rs`** — add `Command::FillDrag { sheet: SheetId,
   seed: CellRange, target: CellRange, axis: FillAxis }`.

4. **`freecell-engine/src/worker/run.rs`** — wire `FillDrag` exactly like `FillDown`:
   - classify into the `edits` bucket (the `edit @ (…)` arm ~L558);
   - merge guard (~L2027): `blocks_fill(merges, *target)`;
   - **overflow guard** in `apply_one` (~L3183): reject with `Err("…too large…")` when
     `range_area(target) > MAX_REFRESH_CELLS` (the same 100k cap paste/fill use), then
     `doc.fill_drag(idx, *seed, *target, *axis)?` → `Cell`/`NoOp`;
   - `op_of` (~L3432): `AppliedOp::Cells { sheet, range: *target }` (the written rectangle
     = seed∪target = target) → needs-eval + refresh classified like FillDown.

5. **`freecell-app/src/grid/mod.rs`** — add `GridEvent::FillDrag { seed, target, axis }`.

6. **`freecell-app/src/shell/window.rs`** — map `GridEvent::FillDrag{seed,target,axis}` →
   `Command::FillDrag { sheet: active, seed, target, axis }` (beside FillDown/FillRight).

7. **`freecell-app/src/grid/view.rs`** — the render + drag state machine:
   - **State:** `struct FillDrag { seed: CellRange, target: CellRange, axis:
     Option<FillAxis> }` + field `fill_drag: Option<FillDrag>` (init `None`), mirroring
     `chart_drag`.
   - **Handle render** (selection-overlay pass, after the active-cell border ~L2919):
     when NOT editing (`incell_open.is_none()`) and no other drag is active
     (`drag`/`resize_drag`/`chart_drag`/`fill_drag` all `None`), draw a `HANDLE_PX` square
     (reuse `chart_layer::HANDLE_PX`) centered on the selection's bottom-right corner,
     `rect_div(...).bg(CELL_BG).border_1().border_color(ACCENT)` (the chart-handle look).
     The corner center is clamped into `[0, content_w] × [0, content_h]` so a whole-row/col
     selection's off-viewport corner clamps to the visible edge (D3.4). Shared free fn
     `fill_handle_square(right_x, bottom_y, scroll_x, scroll_y, content_w, content_h)`.
   - **Preview render:** when `fill_drag` is set with a decided axis, draw a 2px accent
     border `rect_div` over `span_rect(target)` (reuse the range-border pattern).
   - **Hit-test** (`handle_mouse_down`): add `if self.fill_drag.is_some() { return; }` at
     the top (mirroring the resize guard). In the caches block, after the chart arm and
     before the `match hit`, test the pointer (content-local) against the handle square
     (± `HANDLE_HIT_HALF`); a hit sets `fill_drag = Some(FillDrag{ seed: selection.range(),
     target: same, axis: None })`, `notify`, `return`.
   - **Move** (`handle_mouse_move`): after the `resize_drag` check, before the in-cell
     guard, `if self.fill_drag.is_some() { self.update_fill_drag(local_x, local_y, window,
     cx); return; }`. `update_fill_drag` maps the point → cell (`layout::cell_at_point`),
     calls the pure `set_fill_target_from_cell(cell)` (dominant-axis decision + target
     compute), then `maybe_start_autoscroll` + `notify`.
   - **`set_fill_target_from_cell(cell)`** (pure, no window/cx): compute vertical vs
     horizontal extension magnitude of `cell` beyond the seed; inside the seed (both 0) →
     `axis = None`, `target = seed`; else lock `axis` (sticky: keep the existing
     `Some(axis)`, else pick `Vertical` when `vext >= hext` else `Horizontal`) and set
     `target` = seed extended along the axis to include `cell` (down/right OR up/left).
   - **Up** (`handle_mouse_up`): before the `self.drag.take()` arm,
     `if let Some(d) = self.fill_drag.take() { self.commit_fill_drag(d, window, cx); return; }`.
     `commit_fill_drag`: bump `autoscroll_epoch` (stop the loop); if `axis` is `None` or
     `target == seed` (or target not a strict superset — inward, D3.3) → no-op
     (`notify`, return); else emit `GridEvent::FillDrag{seed,target,axis}`, expand the
     selection to `target` (= seed∪target), `notify`.
   - **Auto-scroll reuse:** extend `maybe_start_autoscroll`'s gate to fire for
     `fill_drag` too (`self.drag.is_none() && self.fill_drag.is_none()`), the spawn-loop
     guard likewise, and `autoscroll_tick` to update the fill target (via
     `set_fill_target_from_cell`) instead of the selection when `fill_drag` is active.

## Tests

**Engine (`freecell-engine`, in `document.rs` tests):**
- `fill_drag_two_cell_numeric_seed_extrapolates_series_down` — A1=1,A2=2, seed A1:A2,
  target A1:A5, Vertical → A3=3,A4=4,A5=5.
- `fill_drag_single_cell_seed_copies_down` — A1=7, seed A1, target A1:A4 → all 7 (copy,
  not series).
- `fill_drag_single_cell_copies_relative_formula` — A1="=B1", B1:B4 set → filled cells get
  =B2…=B4 (relative adjust).
- `fill_drag_month_seed_extrapolates` — A1="Jan",A2="Feb", target A1:A4 → Mar, Apr.
- `fill_drag_up_reverses_series` — A4=3,A5=4 seed A4:A5, target A1:A5, Vertical (up) →
  A3=2,A2=1,A1=0 (native fork up-fill reverses the progression).
- `fill_drag_horizontal_series_right` — A1=1,B1=2, target A1:E1, Horizontal → C1=3…E1=5.
- `fill_drag_is_one_undo_step` — after a series fill, a single `undo()` clears every filled
  cell (seed intact).
- `fill_drag_no_extension_is_noop` — target == seed → `Ok(false)`.

**gpui view tests (`freecell-app`, in `grid/view.rs` tests):**
- `fill_handle_shows_for_selection_and_hides_while_editing` — the handle square is present
  in the overlay for a range selection, absent when `incell_open` is set (assert via a new
  test accessor `fill_handle_visible()` / rect).
- `fill_drag_down_sets_preview_and_emits_event` — arm `fill_drag` on the handle (direct
  hit or helper), `handle_mouse_move` down a few rows → `target` extends vertically, axis
  Vertical; `handle_mouse_up` emits `GridEvent::FillDrag` with seed/target/Vertical.
- `fill_drag_expands_selection_after_commit` — post-commit the selection == target.
- `fill_drag_onto_seed_is_noop` — a drag that never leaves the seed emits no `FillDrag`
  and leaves the selection unchanged.

**Render subset (defer full regen to Phase 6):** `render_tests.sh test selection` /
`… test grid_` while iterating to confirm the handle draws on selected ranges. The handle
WILL change existing selection baselines — NOTED for Phase 6, not regenerated here.

## Render-baseline impact — FOR PHASE 6 (regenerate + eyeball)

The fill handle draws on **every visible, non-editing selection**, and the GridView's
**default selection is A1**, which renders in nearly every grid scene. So the affected set
is **essentially all non-chart grid baselines**. Phase 6 must regenerate + eyeball them and
add a dedicated fill-handle + drag-preview baseline case.

**AFFECTED (handle now draws at the selection's bottom-right corner):**
- All `cell_*` cases (default A1 selection → handle at A1's corner) — `cell_plain`,
  `cell_bold`, `cell_align_*`, `cell_number_*`, `cell_valign_*`, `cell_fill_*`,
  `cell_wrap_*`, `cell_date_default`, `cell_boolean`, `cell_error_*`, `cell_tall_row`,
  `cell_wide_column`, `cell_text_*`, `cell_narrow_column_clipped_number`,
  `cell_mirror_typing` (the mirror does **not** set `incell_open`, so the handle is NOT
  suppressed), etc.
- All `border_*` cases.
- All `font_*` cases (`font_family_serif`, `font_missing_family_fallback`,
  `font_size_24_row_grown`).
- All `autogrow_*` cases.
- `col_resized_narrow_clips_text`, `grid_empty_origin`.
- All `grid_selection_*` (`grid_selection_single`, `grid_selection_range`,
  `grid_selection_range_spans_edge`, `grid_selection_scrolled`,
  `grid_selection_shift_extended`, `grid_selection_drag_extended`).
- `header_full_column_selected` / `header_full_row_selected` — the off-viewport corner
  **clamps to the visible edge** (D3.4), so a handle still shows.
- The `grid_chart_*` grid scenes (`grid_chart_line`, `grid_chart_column`, `grid_chart_area`,
  `grid_chart_pie`, `grid_chart_bubble`, `grid_chart_scatter`, `grid_chart_degraded_badge`,
  `grid_chart_unsupported_placeholder`, `grid_chart_authored_inserted`,
  `grid_chart_scrolled_clipped`, `grid_chart_selected`) — the A1 selection handle draws in
  addition to any chart chrome.

**NOT AFFECTED (do NOT regenerate):**
- The standalone `chart_*` chart-render scenes (no grid, no selection).
- Any in-cell-editor case (uses `.in_cell(...)` → sets `incell_open` → handle suppressed).
- `grid_loading_overlay` (the loading overlay occludes the content-layer handle).

Not run this phase: the pixel subset — the lavapipe software-Vulkan capture stack is not
provisioned in this container, and the full regen/eyeball/CI-`render`-gate is deferred to
Phase 6 per the plan. The handle **rectangle** is validated by the gpui test
(`fill_handle_grab_arms_fill_drag_and_hides_while_editing`) through the identical
`fill_handle_square` helper the renderer uses, plus the drag state machine tests.
