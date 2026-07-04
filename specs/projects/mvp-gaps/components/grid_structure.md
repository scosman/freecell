---
status: complete
---

# Component: Grid Structure (resize, header selection, insert/delete)

## Purpose and scope

The grid's structural interactions: row/col resize with live preview + cursors,
header/select-all selection, header context menu with insert/delete rows/cols and the
merge guard. NOT responsible for: engine mechanics (architecture §5), band-style
routing (action_bar.md emits, worker executes), merged-cell rendering (deferred
project).

Touches: `grid/view.rs`, `grid/layout.rs`, `grid/input.rs`, `freecell-core/src/selection.rs`
consumers, worker commands. Architecture refs: §5.

## Public interface (new/changed)

```rust
// grid/view.rs state
struct ResizeDrag { axis: RowOrCol, index: u32, start_px: f32, current_px: f32,
                    apply_to_selection: bool }
// GridView gains: resize_drag: Option<ResizeDrag>

// grid/layout.rs — GridLayout gains a preview and all consumers read through it
pub struct SizePreview { pub axis: RowOrCol, pub index: u32, pub new_px: f32 }
impl GridLayout {
    pub fn with_preview(preview: Option<SizePreview>, ...) -> Self;
    // internal accessors adjusted: track_origin(axis, i), track_size(axis, i)
}

// grid events (grid → shell)
GridEvent::ResizeCommitted { axis, range: (u32, u32), px: f32 }
GridEvent::HeaderSelected { axis, index, extend: bool }   // + drag continuation
GridEvent::SelectAll
GridEvent::HeaderContextMenu { axis, index, screen_pos }

// freecell-core: reference-box formatting
pub fn format_selection_ref(sel: &SelectionModel) -> String;
// A1 | B2:D9 | C:C | C:E | 3:3 | 3:7 | A:XFD (select-all)
```

Worker commands `SetColumnWidths` / `SetRowHeights` / `InsertRows` / `InsertColumns` /
`DeleteRows` / `DeleteColumns` per architecture §2; insert/delete reply
`Result<(), WorkerError>` with `WorkerError::MergesInWay` / bounds errors → dialog.

## Internal design

### Resize

- **Hotspots**: absolute divs in the header strips (headers are plain divs,
  `header_element` view.rs:1285-1309): for each *visible* divider `i`, a 6 px-wide
  (cols) / 6 px-tall (rows) div centered on the divider line, `.cursor_col_resize()` /
  `.cursor_row_resize()`, mouse-down handler. Hotspots render **after** header divs
  (hit priority) and mark events handled so header-selection never fires from them.
- **Drag**: on mouse-down record `ResizeDrag { start_px: axis.size_of(i), … ,
  apply_to_selection: header-range-selected && i within it }`. Mouse-move updates
  `current_px = clamp(start + dx, MIN)` (col ≥ 8 px, row ≥ 12 px) and `cx.notify()`.
  Reuse the existing drag plumbing pattern (range-drag/autoscroll state, view.rs
  :516-560; scrollbar drag :1072-1115). Escape cancels (clear drag, no command).
- **Preview math** (the one subtle bit): all geometry flows through `GridLayout`,
  which now carries `Option<SizePreview>`. Accessor semantics:
  `track_size(axis, i)` = `new_px` when `i == index`, else the raw `Axis` value;
  `track_origin(axis, i)` = raw origin + `delta` when `i > index` (where
  `delta = new_px - axis.size_of(index)`), else raw. Consumers — `cell_rect`
  (view.rs:1212-1218), header layout, selection overlays, scrollbar extents, and
  `hit_test`/`cell_at_point` (layout.rs:126-168, 220-249) — already call through
  GridLayout, so they pick the preview up for free; **visible-range computation**
  binary-searches the raw prefix sums: run it on the raw axis, then extend the range
  by +1 track when the preview shrinks sizes (cheap over-render, exact after release).
- **Guide + tooltip**: 1 px accent line spanning the viewport at
  `track_origin + track_size` of the dragged index; tooltip div near the header shows
  `Width: {px}` / `Height: {px}` (rounded).
- **Release**: emit `ResizeCommitted` with `range = selection-run` when
  `apply_to_selection` (contiguous selected header run containing `index`) else
  `(index, index)`. Shell sends the worker command; worker calls the (verified,
  undoable, range-native) engine setters and rebuilds cache geometry via the existing
  batch path (`set_row_heights`, freecell-core/src/cache.rs:218-229; column analog).
  The grid keeps the preview until the next cache **generation** arrives (it already
  watches generation), then clears it — no flicker window where old geometry shows.

### Header selection

- `hit_test` already classifies header/corner regions (currently no-op at
  view.rs:501-503). New behavior: col-header click → `SelectionModel { anchor: (1, c),
  active: (1_048_576, c) }`; row analog; corner or Cmd/Ctrl+A →
  `A1:XFD1048576`. Shift+click on a header extends `active`'s track only. Dragging
  across headers updates the active track per mouse-move (existing drag loop), rows
  stay full-extent. **No new selection representation** — full extents are ordinary
  ranges, which is exactly what makes the band fast path trigger (`area_of` emits
  `row==1 && height==1_048_576`; unit-tested).
- Rendering: existing range overlay + header tint work unchanged (overlay rect is
  viewport-clamped already); corner gets a hover tint (ui_design §3).
- Reference box: `format_selection_ref` (table above); data row disabled per existing
  multi-select rule.
- Delete on a header selection: existing ClearCells path, worker clamps to
  `dimension()` first (architecture §5.2 clamping rule — `clamp_to_used`).

### Insert/delete + merge guard

- Right-click on a header → `HeaderContextMenu`; shell opens a gpui-component context
  menu (pattern: sheet-tab menu, chrome/view.rs:1074-1178) with, for rows:
  `Insert {n} row(s) above` / `Insert {n} row(s) below` / `Delete {n} row(s)`
  (n = selected header-run size when the click is inside the header selection, else
  1; column variant: left/right).
- **Merge guard, two layers** (reconciling functional spec §5.3 dialog with ui_design
  §3 disabled-items — both, coherently):
  - *UI layer:* `SheetCache` carries the sheet's parsed merge ranges
    (`Vec<CellRange>`, parsed at cache build from `worksheet().merge_cells` — see
    style_render.md). Menu items whose operation would displace a merge render
    **disabled with tooltip** ("Sheet has merged cells — not yet supported here").
  - *Worker layer (authoritative):* re-check before dispatch; blocked →
    `WorkerError::MergesInWay` → the §5.3 dialog. Covers staleness races.
  - Predicate (`freecell-core`, shared by both layers):
    `blocks_row_op(merges, row) = merges.iter().any(|m| m.max_row >= row)`; column
    analog. (Insert at r displaces everything at/after r; delete of rows r..r+n
    likewise — one predicate serves both, conservative and simple.)
- Post-op: worker rebuilds cache (geometry + styles shift) and publishes; selection
  clamps to sheet bounds if it pointed past a deletion (shell-side fix-up).

## Dependencies

Depends on: `Axis`/`GridLayout`, SelectionModel, worker command plumbing, SheetCache
merge list (style_render.md), gpui cursor styles + context menu (verified available).
Depended on by: action_bar.md (band-exact `area_of` from header selections),
clipboard (full-column copy), future merged-cells project (replaces the guard).

## Test plan

Unit:
- `preview_track_math_origin_and_size` — origins after index shift by delta; index
  size replaced; other tracks untouched.
- `preview_hit_test_consistency` — `cell_at_point(cell_rect(c).center()) == c` under
  active preview (property-style over a few indices/deltas).
- `resize_clamps_to_min_sizes`; `resize_escape_cancels`.
- `selection_run_resize_range` — drag inside a 3-col header selection → range (c1,c3).
- `format_selection_ref_all_shapes` — A1, B2:D9, C:C, C:E, 3:3, 3:7, A:XFD.
- `area_of_full_column_exact` — header selection → `row==1, height==1_048_576`
  (band-path gate; likewise full row).
- `merge_guard_predicate` — merge K7:L10: row op at 7/10/11 blocks?/blocks/allows
  (11 > max_row 10 ⇒ allows is FALSE — insert at 11 doesn't displace K7:L10 ⇒ allows);
  col ops analog; empty merge list never blocks; A1-parse of `"K7:L10"`.
Engine integration:
- `insert_rows_shifts_formulas_heights_and_undo` (verified engine behavior pinned by
  test); `delete_columns_restores_width_style_on_undo`.
- `merge_guard_blocks_on_fixture` — workbook fixture with merges: op above → blocked;
  op below-all-merges → succeeds; file saved after allowed op keeps valid merges.
- `set_columns_width_range_and_undo`.
Render suite: `col_resized_narrow_clips_text`, `row_resized_tall` (geometry honored
end-to-end through cache rebuild).
Manual/smoke: cursor flips on hotspots; drag feel; context menu counts pluralize.
