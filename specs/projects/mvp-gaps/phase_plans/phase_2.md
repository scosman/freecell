---
status: complete
---

# Phase 2: Editing feel

## Overview

Phase 2 delivers the "editing feel" cluster from `functional_spec.md §1` and
`components/edit_controller.md`: one owner for the single pending cell edit, type-to-replace,
the live cell mirror, the in-cell editor overlay (double-click / F2), Tab-commit from both
editors, and the cap-error popover on the in-cell editor.

### Architecture decision (deviation from the literal component doc — see DECISIONS §Phase 2)

`components/edit_controller.md` sketches an `EditController` **owned by `WorkbookWindow`** that
owns *both* `InputState`s and the whole pending-edit state machine, with the existing data-row
logic torn out of chrome. Moving the data-row `InputState` + the proven `DataRow` reducer
(fetch / spinner / disabled / stale-reply / cap / commit / escape — all table-tested) out of
`ChromeView` and coordinating two editors that live in **different entities** (data row in
chrome, in-cell overlay in the grid) would require brittle cross-entity `InputState` text sync
— exactly the feedback-loop the doc's `syncing` guard fights, but across entity boundaries.

Instead the pending edit is owned by a **single entity, `ChromeView`**, which already owns the
data-row `InputState` + `DataRow` reducer. A new `chrome/edit.rs` `EditController` holds the
**second** (in-cell) `InputState`, the `in_cell_open` cell, the current `origin`, and the
`syncing` guard; the two editors sync **inside one entity** (no cross-entity loop). The grid
renders the mirror + the in-cell overlay from state the window pushes to it via the existing
`ChromeGridSink`; the grid emits type-to-replace / open-in-cell / in-cell Tab+Escape as new
`GridEvent`s. This keeps every existing data-row test green and reuses all the commit / cap /
escape logic. The pending edit's canonical **text + commit/cap** stays in the `DataRow`
reducer; `EditController` layers the in-cell editor + cross-editor sync + origin tracking on top.

## Steps

1. **`chrome/edit.rs` (new)** — `pub enum EditOrigin { DataRow, InCell }` and
   `pub struct EditController { in_cell: Entity<InputState>, open: Option<CellRef>, origin:
   EditOrigin, syncing: bool }` with accessors (`in_cell_input`, `open_cell`, `origin`,
   `is_open`) and small mutators. Register `mod edit;` + `pub use` in `chrome/mod.rs`.

2. **`chrome/view.rs` — construct + own the controller.** In `ChromeView::new` build a second
   `InputState` (in-cell), subscribe to it (`on_incell_event`), and store
   `edit: EditController`. Add `incell_input()` accessor (window hands the handle to the grid).

3. **`chrome/view.rs` — type-to-replace.** `pub fn begin_typed(&mut self, text: &str, window,
   cx)`: force the field to Editing on the active cell (`reduce(Edited{text})`), set the
   content input to `text` (caret at end via `set_value`), focus it, close any in-cell overlay,
   `origin = DataRow`, push mirror state to the grid.

4. **`chrome/view.rs` — in-cell editor.** `pub fn begin_in_cell(&mut self, cell, window, cx)`:
   commit any pending edit on another cell first (abort on cap-reject), seed both editors with
   the cell's current raw content (the value already fetched into the content input), set the
   reducer to Editing, open the overlay (`edit.open = Some(cell)`, `origin = InCell`), focus the
   in-cell input, push in-cell + mirror state to the grid. `pub fn commit_incell_move(dir)` /
   `cancel_incell()` for the grid-routed Tab / Escape. Close the overlay on any commit/cancel.

5. **`chrome/view.rs` — two-editor sync.** `on_incell_event`: on `Change`, guard `syncing`,
   push text into the content input (`set_value`, events suppressed) + `reduce(Edited)`, push
   mirror to grid; on `PressEnter { shift }` commit (down/up); on `Focus` set `origin = InCell`.
   Extend `on_content_event`: on `Change`, mirror text into the in-cell input when the overlay
   is open; on `Focus` set `origin = DataRow`. All commit paths (Enter, Tab, click-away, format)
   close the overlay and clear the grid mirror.

6. **`chrome/view.rs` — Tab-commit + mirror/in-cell push.** Add Tab / Shift+Tab to the data-row
   row's `on_key_down` → `reduce(Commit)` with the motion swapped to Right/Left. Add a
   `push_edit_state_to_grid` helper that emits `ChromeGridRequest::EditState { .. }` (mirror
   cell+text, in-cell open cell, in-cell cap message) after every edit transition, and
   `ChromeGridRequest::MoveActive(Right/Left)` for Tab. Gate the existing data-row cap popover
   to `origin == DataRow`.

7. **`chrome/mod.rs` — `ChromeGridRequest` additions.** `EditState { mirror: Option<(SheetId,
   CellRef, SharedString)>, in_cell: Option<CellRef>, cap: Option<SharedString> }` and
   `SetInCellInput(Entity<InputState>)`.

8. **`grid/mod.rs` — `GridEvent` additions.** `TypeToEdit(String)`, `OpenInCellEditor(CellRef)`,
   `InCellCommitMove(Direction)`, `InCellCancel`.

9. **`grid/view.rs` — new state + render.** Store `mirror: Option<(SheetId, CellRef,
   SharedString)>`, `incell_open: Option<CellRef>`, `incell_input: Option<Entity<InputState>>`,
   `incell_cap: Option<SharedString>` with setters (`set_mirror`, `set_incell_open`,
   `set_incell_input`, `set_incell_cap`). In `build_grid_layers`: render the mirror text (raw,
   left, default style) in the active cell instead of its published value; render the in-cell
   overlay (deferred absolute div at `cell_rect`, min-width 80 px, 2 px accent border, the
   `Input`, on_key_down for tab/shift-tab/escape → events) + the cap popover below it.

10. **`grid/view.rs` — input triggers.** `handle_mouse_down`: `event.click_count == 2` on a cell
    → emit `OpenInCellEditor`. `handle_key_down`: early-return when `incell_open.is_some()` (the
    overlay input owns keys); F2 with a single selection → `OpenInCellEditor(active)`; a
    printable modifier-free `key_char` (no ctrl/alt/platform/function) → collapse a multi
    selection to the active cell, then emit `TypeToEdit(char)`.

11. **`shell/window.rs` — wiring.** Hand the in-cell input handle to the grid at build. Route the
    new `GridEvent`s (`TypeToEdit`/`OpenInCellEditor`/`InCellCommitMove`/`InCellCancel`) to the
    chrome. Handle the new `ChromeGridRequest::EditState`/`SetInCellInput` in
    `make_chrome_grid_sink` by calling the grid setters (direct — they don't re-enter chrome).

12. **`render-tests`** — add `mirror` + `incell_input` fields to `RenderCase`, apply them in
    `render.rs`, and add `incell_editor_open` + `cell_mirror_typing` cases (+ macro rows). Record
    the baseline regeneration need in DECISIONS (cannot regenerate in this container).

## Tests

Chrome (headless, real InputStates in a test window — extends `chrome/view.rs` tests):
- `type_to_replace_starts_edit_with_char` — `begin_typed("x")` → Editing, content == "x".
- `type_to_replace_on_multiselect_targets_active` — begin from a multi selection edits the
  active cell and commits there.
- `f2_opens_in_cell_keeping_content` — begin_in_cell keeps the fetched raw text, origin InCell.
- `in_cell_and_data_row_stay_in_sync` — typing in one editor updates the other (no echo loop).
- `in_cell_enter_commits_and_moves_down` / `in_cell_tab_commits_and_moves_right`.
- `in_cell_escape_cancels_and_reverts` — overlay closes, content reverts to committed.
- `in_cell_cap_reject_keeps_editing_and_flags` — a too-long formula from the in-cell keeps the
  edit + sets the cap message (pushed for the in-cell popover).
- `begin_in_cell_on_other_cell_commits_first` — a pending edit is committed before opening.
- `data_row_tab_commits_and_moves_right` / `shift_tab_left`.
- `mirror_pushed_while_editing_cleared_on_commit` — the grid receives mirror state on edit and
  a clear on commit/cancel.

grid/input (headless keymap already covered) + grid selection:
- `printable_key_emits_type_to_edit` (via a `GridView` test asserting the emitted event).
- `double_click_emits_open_in_cell`.
- `keys_ignored_while_in_cell_open`.

freecell-core: no reducer change required (the `DataRow` reducer is reused unchanged).

Render suite (baselines regenerated on the pinned runner — recorded in DECISIONS):
- `incell_editor_open`, `cell_mirror_typing`.
</content>
</invoke>
