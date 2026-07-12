---
status: complete
---

# Phase 2: Quick-edit mode (§5)

## Overview

When a user starts an edit by **type-to-replace** (grid focused, single cell selected, printable
key → `begin_typed`), that edit enters **quick-edit mode**: an unmodified arrow key commits the
edit and moves the active cell one step (reusing `commit_and_move`), so rapid data entry never
needs Tab/Enter. Any caret-intent signal — a mouse-down in the field, Home/End, or a modified
(Shift/Cmd/Ctrl/Alt) arrow — leaves quick-edit for the rest of that edit (arrows then move the
caret). Double-click/F2 (in-cell) and formula-bar edits are never quick-edit. Tab/Enter/
Shift+Enter/Escape are unchanged.

State + key interception only — no engine change, no pixel impact (chrome/behavior only, not a
baselined surface). Validated with gpui view/unit tests + the checks; no render suite run.

## Key design decisions

- **Home the flag on `ChromeView`** (`quick_edit: bool`), where the single pending edit already
  lives (`content_input` + `DataRow` reducer). Set `true` in `begin_typed`; `false` in
  `begin_in_cell`; cleared on caret intent; reset on commit/cancel.
- **Do NOT clear `quick_edit` in the `on_content_event` Focus handler** (the architecture pointer
  suggested formula-bar focus). Reason discovered while reading the code: `InputEvent::Focus` is
  emitted from a *deferred* `on_focus` observer, so `begin_typed`'s programmatic `input.focus()`
  fires Focus **after** `begin_typed` returns — clearing the flag there would immediately undo the
  quick-edit it just set. The "clicked into the formula bar" case §5.3 targets is instead handled
  precisely by an `on_mouse_down` on the data-row field (the flag starts `false`, so a user who
  clicks straight into the formula bar without type-to-replace never has it set anyway). Recorded
  in `DECISIONS_TO_REVIEW.md`.
- **Thread `quick_edit` through the existing `ChromeGridRequest::EditState` push** (add a 4th
  field) → `GridView::set_edit_state` → a new `GridView.quick_edit` field, consumed by the grid
  root `capture_key_down` arrow branch. In `refresh_edit_grid_state` the pushed value is
  `editing && self.quick_edit`, so the grid's copy auto-clears the instant the edit ends.
- The grid-root in-cell arm is a **symmetric mirror**: type-to-replace never opens the in-cell
  overlay and `begin_in_cell` clears `quick_edit`, so `incell_open.is_some() && quick_edit` cannot
  co-occur in the current flow. The arm is implemented (as the task requires) for symmetry/future-
  proofing; a comment notes it is defensive.

## Steps

1. **`chrome/mod.rs`** — add `quick_edit: bool` to `ChromeGridRequest::EditState` with a doc
   comment (`functional_spec.md §5`).

2. **`chrome/view.rs` — state:**
   - Add `quick_edit: bool` field to `struct ChromeView` (near `edit_state_shown`), doc-commented.
   - Initialise `quick_edit: false` in `ChromeView::new`.

3. **`chrome/view.rs` — enter/leave/reset:**
   - `begin_typed`: set `self.quick_edit = true;` (type-to-replace = quick-edit entry).
   - `begin_in_cell`: set `self.quick_edit = false;` after the divergent-cell early-return guard.
   - `escape_edit`, `cancel_incell`: set `self.quick_edit = false;`.
   - `commit_and_move`: set `self.quick_edit = false;` in the `mode != Editing` (committed) arm,
     alongside `self.edit.close()`.
   - `on_edit_commit_requested`: set `self.quick_edit = false;` in the `committed` arm.
   - New `fn leave_quick_edit(&mut self, window, cx)`: if `quick_edit`, clear it and
     `refresh_edit_grid_state` (so the grid copy updates). Idempotent.

4. **`chrome/view.rs` — the data-row edit-key handler.** Extract the data-row `capture_key_down`
   body into `fn handle_data_row_edit_key(&mut self, key: &str, shift: bool, modified: bool,
   window, cx) -> bool` (returns "consumed"):
   - not `Editing` → `false`.
   - `"tab"` → `commit_and_move(Left if shift else Right)`; `true` (unchanged Tab behavior).
   - not `quick_edit` → `false`.
   - `"left"|"right"|"up"|"down"`: if `modified` → `leave_quick_edit`, `false` (fall through to
     caret, no active-cell move); else `commit_and_move(dir)`, `true`.
   - `"home"|"end"` → `leave_quick_edit`, `false` (caret positioning).
   - else `false`.
   The `capture_key_down` listener calls it and `cx.stop_propagation()` iff it returned `true`.

5. **`chrome/view.rs` — mouse caret intent.** Add `.on_mouse_down(MouseButton::Left, listener →
   leave_quick_edit)` on the data-row content-field wrapper div (the `data-content-field`
   element). The gpui-component `Input` does not `stop_propagation` on mouse-down, so this
   bubble-phase listener fires on a click into the field.

6. **`chrome/view.rs` — `refresh_edit_grid_state`:** compute `let quick_edit = editing &&
   self.quick_edit;` and include it in the `ChromeGridRequest::EditState { .. }` it emits.

7. **`shell/window.rs`** — in the `ChromeGridRequest::EditState` arm, destructure `quick_edit` and
   pass it to `grid.set_edit_state(mirror, in_cell, cap, quick_edit, cx)`.

8. **`grid/view.rs`:**
   - Add `quick_edit: bool` field to `struct GridView` (near `incell_cap`); init `false` in the
     constructor.
   - `set_edit_state`: add a `quick_edit: bool` param; store `self.quick_edit = quick_edit;`.
     Update the 6 in-crate test call sites to pass `false`.
   - Grid-root `capture_key_down`: bind `key`/`modifiers`; add an arm
     `"left"|"right"|"up"|"down" if this.quick_edit && !modifiers.modified()` → `stop_propagation`
     + emit `GridEvent::InCellCommitMove(dir)` (reuses the existing Tab plumbing →
     `commit_incell_move` → `commit_and_move`). Comment that it is a defensive symmetric mirror.
   - Add a `#[cfg(test)] fn quick_edit_for_test(&self) -> bool` accessor.

## Tests

`chrome/view.rs` gpui view tests (add a test seam `test_data_row_key(key, shift, modified) ->
bool` that calls `handle_data_row_edit_key`, plus a `last_edit_state_quick` helper reading the
pushed `quick_edit`):

- `quick_edit_arrow_right_commits_and_moves`: type-to-replace `"abcd"`, arrow Right → one
  `SetCellInput("abcd")` + `MoveActive(Move(Right))`; seam returns `true` (consumed).
- `quick_edit_arrow_each_direction_moves`: Left/Up/Down each commit + `MoveActive` in that
  direction.
- `quick_edit_not_entered_by_in_cell`: `begin_in_cell` then arrow → seam returns `false`, no
  `SetCellInput`/`MoveActive`, still `Editing` (in-cell edit = caret arrows).
- `quick_edit_modified_arrow_leaves_and_does_not_move`: type-to-replace, Shift+arrow (modified)
  → seam `false`, no `MoveActive`, still `Editing`, `quick_edit` now false (a subsequent
  unmodified arrow also does not move).
- `quick_edit_home_leaves`: type-to-replace, Home → seam `false`, no move, `quick_edit` false.
- `quick_edit_mouse_down_leaves`: type-to-replace, `leave_quick_edit` (mouse-down path) → a
  following arrow no longer commits/moves (seam `false`).
- `quick_edit_pushed_to_grid_while_typing`: after `begin_typed`, the last `EditState` push has
  `quick_edit == true`; after `begin_in_cell`, `quick_edit == false`.
- `quick_edit_cleared_on_grid_push_after_commit`: after commit the last push has
  `quick_edit == false`.
- `tab_and_enter_unchanged`: type-to-replace, Tab still commits + moves Right; Enter still commits
  + moves Down (regression via `handle_data_row_edit_key("tab", ..)` and existing Enter path).

`grid/view.rs` unit test:

- `set_edit_state_threads_quick_edit`: `set_edit_state(None, Some(cell), None, true, cx)` →
  `quick_edit_for_test() == true`; `.. false ..` → `false`.
