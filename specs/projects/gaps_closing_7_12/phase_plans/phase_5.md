---
status: complete
---

# Phase 5: Cell-area right-click context menu

## Overview

Right-clicking the grid **cell body** currently only dismisses open popovers
(`handle_right_mouse_down`'s cell/corner arm). This phase adds a real context menu over the
cell area, cloned from the existing custom-`div` popover pattern (`chart_menu_elements` /
`header_menu_elements`): a `.absolute().occlude()` card of items that emit **existing**
`GridEvent`s, plus a deferred full-grid dismiss backdrop.

Behavior (functional_spec §2, architecture §2, decision D2.1 default):

- **Selection interaction (Excel):** a right-click on a cell **outside** the current
  selection first moves the selection to that single cell; a right-click **inside** the
  current selection keeps it (so the menu acts on the whole multi-cell selection).
- **Items (top→bottom):** Cut, Copy, Paste, Paste values, Clear contents, — separator —,
  Insert row(s) above, Insert row(s) below, Delete row(s), Insert column(s) left, Insert
  column(s) right, Delete column(s). All reuse existing `GridEvent`s (nothing is
  reimplemented). **Clear Formatting is omitted** — no style-clear `GridEvent`/`Command`
  exists (D2.1: include only if one already exists).
- **Enable/disable:** Cut/Copy/Clear always enabled (the architecture's `CellMenu` carries no
  copy/cut/clear gate — a selection is always ≥1 cell). Paste + Paste values gate on the
  system clipboard having text (read once at open). Insert/Delete rows/cols gate on the
  existing header-menu **merge-displacement guard** (`merge_block_flags`), computed for both
  axes over the selection's row/col span.
- **One menu at a time / dismissal:** opening the cell menu clears the header menu (the chart
  menu is already cleared earlier in `handle_right_mouse_down`); opening the header menu
  clears the cell menu; Escape and the click-away backdrop close it (same pattern as the
  header/chart menus).

**Paste-values enable note (deviation from the literal architecture wording):** the
architecture lists `paste_enabled` **and** `paste_values_enabled`. The grid cannot know
whether the clipboard payload is *internal* vs *foreign* (that state lives in the window's
`ClipboardCoordinator.last_copy_text`, not the grid), and per functional_spec §5 Paste Values
falls back to a TSV paste for a foreign clipboard anyway — so both paste items share one
`paste_enabled` gate (any clipboard text). This is the correct runtime behavior without new
cross-view plumbing.

Non-pixel (popover chrome, per the render-scope table) → gpui view tests + a
`VisualTestContext` paint test that opens the menu and asserts the card paints. No pixel
baseline; the pixel suite is not run this phase.

## Steps

1. **`grid/view.rs` — `CellMenu` struct** (near `HeaderMenu`, ~L109): a `Copy` struct
   `{ x: f32, y: f32, range: CellRange, paste_enabled: bool,
   insert_row_above_blocked: bool, insert_row_below_blocked: bool, delete_rows_blocked: bool,
   insert_col_left_blocked: bool, insert_col_right_blocked: bool, delete_cols_blocked: bool }`.
   `range` is the selection rectangle snapshot at open (menu is modal, can't drift); its
   rows/cols give the insert/delete span and it is itself the Clear-contents target.

2. **`GridView` field** (~L303, after `chart_menu`): `cell_menu: Option<CellMenu>` + init
   `cell_menu: None` in `new` (~L553).

3. **Open logic — `handle_right_mouse_down`** (~L1739): change the `hit` match so
   `GridHit::Cell { row, col }` calls a new `open_cell_menu(row, col, local_x, local_y,
   &merges, window, cx)` and returns; `GridHit::Corner` keeps the dismiss-only behavior (now
   also clearing `cell_menu`). The header-open path (~L1772) additionally sets
   `self.cell_menu = None`. The chart-open path (~L1727) additionally sets
   `self.cell_menu = None`.

4. **`open_cell_menu`**: if `!self.selection().range().contains(CellRef::new(row, col))`,
   `set_selection_and_emit(SelectionModel::single(cell))` (move-if-outside; the guard means an
   inside click emits nothing). Then snapshot `range = self.selection().range()`; compute
   `merge_block_flags(Row, (range.start.row, range.end.row), merges)` and the `Col` analogue;
   read the clipboard once (`cx.read_from_clipboard().and_then(|i| i.text())`) →
   `paste_enabled`; clear `header_menu`; set `cell_menu = Some(...)`; `cx.notify()`.

5. **`close_cell_menu`** (mirror `close_chart_menu`): `if self.cell_menu.take().is_some() {
   cx.notify() }`.

6. **`cell_menu_elements(&self, menu, cx) -> Vec<AnyElement>`** (clone `chart_menu_elements`,
   ~L3189): a `.absolute().left(menu.x).top(menu.y).occlude()` card with
   `debug_selector("cell-menu-card")`. Build an ordered list of `Option<(label, enabled,
   GridEvent)>` (`None` = separator). Enabled items get `cursor_pointer` + hover +
   `on_mouse_down(Left, …)` that emits the event, `close_cell_menu`, `stop_propagation`;
   disabled items render dimmed (`text_color(HEADER_TEXT).opacity(0.4)`, no listener) — exactly
   the header-menu item styling. Insert/Delete events use `range` rows/cols
   (`InsertRows { at: start.row, count: height }`, below at `end.row + 1`, `DeleteRows { at:
   start.row, count: height }`; column analogues). Clear = `ClearCells(range)`. Append the same
   deferred full-grid backdrop (Left+Right mouse-down → `close_cell_menu` + `stop_propagation`).
   Return `vec![deferred(backdrop), deferred(card)]`.

7. **Escape** (~L2093): add `|| self.cell_menu.is_some()` to the condition and
   `self.cell_menu = None;` to the body.

8. **Sheet switch** (~L806): add `self.cell_menu = None;` (structural interaction anchored to
   the previous sheet's geometry).

9. **Root render extend** (~L4213, after the chart-menu block): `if let Some(menu) =
   self.cell_menu { root_children.extend(self.cell_menu_elements(menu, cx)); }`.

No `grid/mod.rs` change — `GridEvent::PasteValues` already exists (Phase 4).

## Tests

All in `grid/view.rs` `#[cfg(test)]`, using the existing `grid_recording` / `mouse_ev` /
`key_ev` helpers over `demo_sources`.

- `right_click_cell_outside_selection_moves_and_opens_menu`: right-click a cell point →
  `cell_menu` is `Some` (coords match), a `SelectionChanged` was emitted, and the selection
  collapsed to the clicked single cell; `header_menu`/`chart_menu` are `None`.
- `right_click_cell_inside_selection_keeps_it`: right-click a point once (moves + selects),
  clear the recorder, right-click the **same** point again → no new `SelectionChanged` (the
  selection is kept) and `cell_menu` is `Some`.
- `cell_menu_paste_disabled_when_clipboard_empty` / enabled when seeded: with an empty
  clipboard `menu.paste_enabled == false`; after `cx.write_to_clipboard(new_string("x"))`,
  a fresh right-click yields `menu.paste_enabled == true`.
- `cell_menu_item_emits_event`: open the menu, invoke the Clear-contents / a Delete item via a
  simulated `VisualTestContext` click on the item's `debug_bounds`, assert the matching
  `GridEvent` (`ClearCells` / `DeleteRows`) was recorded and the menu closed. (If item-level
  `debug_selector`s are awkward, assert via the emitted event after closing the menu through
  the builder path.)
- `cell_menu_escape_closes`: open, `handle_key_down(escape)` → `cell_menu` is `None`.
- `cell_menu_card_paints` (`VisualTestContext`): open the menu, `run_until_parked`,
  `debug_bounds("cell-menu-card")` is `Some` (mirrors `header_menu_padding_click_keeps_menu_open`).
</content>
</invoke>
