---
status: complete
---

# Phase 7: Structure — resize, header selection, insert/delete + merge guard

## Overview

Adds the grid's structural interactions (`functional_spec.md §5`, `components/grid_structure.md`):

- **Row/column resize** — divider hotspots with resize cursors, a live drag preview (guide
  line + size tooltip), min-size clamps, Escape-cancel, and a commit that writes engine
  widths/heights (one undo step; whole selected run when the dragged header is inside a
  header selection).
- **Header selection + select-all** — clicking a column/row header selects the full
  column/row (ordinary full-extent range so the engine band fast path engages); drag +
  Shift extend; the corner / Cmd·Ctrl+A select the whole sheet; the ref box shows
  `C:C` / `3:7` / `A:XFD`; Delete on a header selection clamps to the used range.
- **Insert/delete rows/cols** — a right-click header context menu (counts pluralize from the
  selected run) that inserts/deletes via the undoable engine ops, guarded by a **merge
  guard**: a merge at/after the affected index blocks (UI disables the item; the worker
  re-checks authoritatively and dialogs).

All engine APIs were verified against ironcalc_base 0.7.1 at the pinned rev
(`insert_rows`/`insert_columns`/`delete_rows`/`delete_columns` at `user_model/common.rs:882/907/932/974`,
`set_columns_width`/`set_rows_height` at `:1055/:1081`, `merge_cells: Vec<String>` at
`types.rs:113`) — all undoable (`push_diff_list`), 1-based coords.

## Steps

### freecell-core (headless; unit-tested)

1. **`refs.rs`** — add `CellRange::from_a1(&str) -> Option<CellRange>`: parse `"A1:B2"` (split on
   `':'`) or a single `"A1"`; reuse `CellRef::from_a1`. Shared parser for merges + tests.
2. **`selection.rs`** — add `pub fn format_selection_ref(sel: &SelectionModel) -> String`:
   - full-rows (`start.row==0 && end.row==MAX_ROWS-1`) → column form `col_label(c0):col_label(c1)`
     (`C:C` / `C:E`; select-all → `A:XFD`);
   - else full-cols (`start.col==0 && end.col==MAX_COLS-1`) → row form `(r0+1):(r1+1)`
     (`3:3` / `3:7`);
   - else `range.to_a1()` (`A1` / `B2:D9`).
   Also `is_full_column_selection` / `is_full_row_selection` helpers (used by the grid's
   resize-run + header-drag logic).
3. **new `merge_guard.rs`** — `blocks_row_op(&[CellRange], row0) -> bool =
   merges.any(|m| m.end.row >= row0)`; `blocks_col_op(&[CellRange], col0)` col analog. 0-based
   affected index; merges 0-based. Export from `lib.rs`.
4. **`cache.rs`** — add `merges: Vec<CellRange>` to `SheetCache` + `SheetCacheBuilder`
   (seed empty), `push_merge` / consuming `merge` setter + `merges()` accessor; thread through
   `build()`. `SheetCache` stays `Send + Sync` (`CellRange: Copy`).
5. **`lib.rs`** — export `merge_guard::{blocks_row_op, blocks_col_op}` and
   `selection::format_selection_ref`.

### freecell-engine

6. **`cache.rs`** — add inverse px conversions `col_ironcalc_px(device)` / `row_ironcalc_px(device)`
   (inverse of `col_px`/`row_px`). In `build_sheet_cache`, parse `ws.merge_cells` via
   `CellRange::from_a1` (skip+log unparseable) into `builder.push_merge`.
7. **`document.rs`**:
   - `set_column_widths(idx, c0, c1, device_px)` / `set_row_heights_px(idx, r0, r1, device_px)`
     — convert device→IronCalc px, call `set_columns_width`/`set_rows_height`.
   - `insert_rows` / `insert_columns` / `delete_rows` / `delete_columns` (0-based → 1-based;
     `count` rows/cols).
   - `merge_ranges(idx) -> Result<Vec<CellRange>, String>` (parse `worksheet.merge_cells`).
   - **`clear_contents`** — clamp to `clamp_to_used` first (the §5.2 clamping rule: a header-
     selection Delete must not iterate a 1M-cell band).
8. **`worker/protocol.rs`** — new `Command` variants `SetColumnWidths { sheet, col_start,
   col_end, px }`, `SetRowHeights { sheet, row_start, row_end, px }`, `InsertRows { sheet, row,
   count }`, `InsertColumns { sheet, col, count }`, `DeleteRows`, `DeleteColumns`. Add
   `EditRejectedReason::MergedCells` (fixed §5.3 message).
9. **`worker/run.rs`**:
   - route the six commands into the `edits` bucket (exhaustive match).
   - `AppliedKind`: add `GeometryOnly` (no eval) + `Structure` (eval). `apply_edit_batch` arms:
     both push `op_of`; `Structure` sets `needs_eval`.
   - `apply_one`: dispatch the six → `doc` methods.
   - `AppliedOp`: add `Rebuild { sheet }`; `op_of` maps the six → `Rebuild`.
   - `Touch`: add `Rebuild { sheet }`. `mirror_applied_ops`: `Rebuild` pushes the touch + marks
     the sheet for a full `build_and_store_cache`; Undo/Redo of a `Rebuild` touch re-rebuilds.
     `touch_refresh_ranges(Rebuild)` → empty (rebuild handled separately).
   - `pre_validate`: for the four insert/delete commands, read `doc.merge_ranges(idx)` and the
     affected 0-based index, return `Err(EditRejectedReason::MergedCells)` when blocked.
10. **`lib.rs`** — nothing new to export (variants ride `Command` / `EditRejectedReason`).

### freecell-app — grid

11. **`grid/mod.rs`** — `pub enum RowOrCol { Row, Col }`; new `GridEvent`s
    `ResizeCommitted { axis, start, end, px }` and `HeaderContextMenu(HeaderMenuRequest)`
    (`{ axis, index, run: (u32,u32), position: Point<Pixels> }`). Resize/min-size constants.
12. **`grid/input.rs`** — `GridKeyCommand::SelectAll`; map `secondary && key=="a"` (no shift).
13. **`grid/view.rs`**:
    - state: `resize_drag: Option<ResizeDrag { axis, index, start_px, current_px,
      run: (u32,u32) }>`; `DragState.mode: DragMode { Cell, ColHeader, RowHeader }`.
    - `resolve_frame`: when a resize is active, replace the dragged axis with a **preview axis**
      built from the cache's overrides + default with the dragged index set to `current_px`
      (all consumers pick it up for free).
    - header hotspots: absolute divs on each visible divider in the col/row header strips
      (6 px, `.cursor_col_resize()`/`.cursor_row_resize()`), rendered **after** the labels;
      `on_mouse_down` → start `ResizeDrag`, `cx.stop_propagation()`.
    - root `handle_mouse_move`/`handle_mouse_up`: if `resize_drag` active, update
      `current_px = clamp(start_px + d, MIN)` / commit (`ResizeCommitted`, clear preview on next
      cache generation via the existing generation watch — the drag clears on mouse-up and the
      rebuild republishes) — else the existing selection drag.
    - Escape while resizing → clear `resize_drag`.
    - `handle_mouse_down` header/corner arms: ColHeader → full-column selection; RowHeader →
      full-row; Corner → select-all; Shift extends the active track; begin a header drag.
    - `SelectAll` key command → full-sheet selection.
    - right mouse-down on a header → compute `run` (selected run if the header is inside a
      header selection of that axis, else select the single header) → emit `HeaderContextMenu`.
    - build_grid_layers: resize guide line (1 px accent, viewport span at the drag edge) +
      size tooltip (`Width: N` / `Height: N`).
14. **`chrome/view.rs`** — `ref_box_text` → `format_selection_ref(&self.selection)`.

### freecell-app — window

15. **`shell/window.rs`**:
    - route `GridEvent::ResizeCommitted` → `SetColumnWidths`/`SetRowHeights`.
    - route `GridEvent::HeaderContextMenu` → store `header_menu: Option<HeaderMenu>`, reading the
      active sheet cache's `merges()` to compute per-item block flags via `blocks_row_op`/
      `blocks_col_op`.
    - render the header menu overlay (backdrop + Insert-before/after/Delete items; disabled +
      tooltip when blocked; labels pluralize from the run size). Items send
      `Command::InsertRows`/etc.
    - `on_edit_rejected`: `EditRejectedReason::MergedCells` → the §5.3 dialog.

### render cases

16. **`render-tests`** — add `col_resized_narrow_clips_text` (col_width 20 px, number clips) +
    `row_resized_tall` (row_height 48 px) + `header_full_column_selected` /
    `header_full_row_selected` (full-extent selection tint) to `cases::all()` **and** the
    `render_cases!` list. Additive only.

## Tests

Unit (freecell-core):
- `range_from_a1_valid_and_hostile` — `A1`, `K7:L10`, `A1:XFD1048576`; junk → None.
- `format_selection_ref_all_shapes` — `A1`, `B2:D9`, `C:C`, `C:E`, `3:3`, `3:7`, `A:XFD`.
- `merge_guard_predicate` — `K7:L10` (0-based): row op at 6/9/10 blocks/blocks/allows; col op
  at 9/11/12 analog; empty merges never block.
- cache `merges` round-trips through the builder.

Unit (freecell-app):
- `select_all_key_command` — `secondary + a` → `SelectAll`.
- grid: `col_header_click_selects_full_column`, `row_header_click_selects_full_row`,
  `corner_selects_all`, `shift_click_extends_header`, `resize_clamps_to_min`,
  `resize_escape_cancels`, `resize_run_uses_selection`, `right_click_header_emits_menu`.
- preview-axis math: `preview_axis_shifts_later_tracks_and_replaces_index`.

Engine integration (freecell-engine, real UserModel):
- `set_columns_width_range_and_undo`; `set_rows_height_and_undo`.
- `insert_rows_shifts_and_undo` (a formula below shifts + undo restores);
  `delete_columns_and_undo`.
- `clear_contents_clamps_full_column` (a full-column clear touches only used cells; fast).
- `merge_guard_blocks_and_allows_on_fixture` — insert above a merge blocked; below-all merges
  succeeds; `EditRejectedReason::MergedCells` emitted.
- `cache_carries_merges_from_file` (a merged fixture → `SheetCache::merges()` populated).

Render (additive, regenerate on the pinned runner): `col_resized_narrow_clips_text`,
`row_resized_tall`, `header_full_column_selected`, `header_full_row_selected`.
