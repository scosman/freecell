---
status: draft
---

# Component: EditController

## Purpose and scope

One owner for the single pending cell edit: type-to-replace, the data-row editor, the
in-cell editor, the live cell mirror, commit/cancel/move, cap validation + popover
state. Replaces the edit logic currently spread through `chrome/view.rs` (reducer
~:285-317, commit/escape :222-230, cap check :686-688). NOT responsible for:
selection movement rules (grid), the optimistic post-commit pending display
(existing worker publication path), IME.

New module: `freecell-app/src/shell/edit.rs`. Architecture refs: §4.

## Public interface

```rust
pub enum EditOrigin { DataRow, InCell }
pub enum MoveDir { Down, Up, Right, Left }          // Enter/Shift+Enter/Tab/Shift+Tab
pub enum CommitOutcome { NoEdit, CapRejected(CapErrorKind), Committed }
pub enum CapErrorKind { TooLong, TooDeep }           // reuse freecell-core::input_cap kinds

pub struct EditController {
    state: Option<PendingEdit>,
    data_row: Entity<InputState>,     // created in WorkbookWindow::build (has &mut Window)
    in_cell: Entity<InputState>,      // ditto; one instance reused for every in-cell edit
    syncing: bool,                    // re-entrancy guard, see below
}

struct PendingEdit {
    sheet: SheetId,
    cell: CellRef,                    // the ANCHOR cell being edited
    text: SharedString,
    origin: EditOrigin,               // which editor currently drives (== has focus)
    in_cell_open: bool,               // overlay visible (origin can be DataRow while open)
    cap_error: Option<CapErrorKind>,
}

impl EditController {
    // entry points
    pub fn begin_typed(&mut self, cell: CellRef, first: &str, win, cx);          // §type-to-replace
    pub fn begin_in_cell(&mut self, cell: CellRef, raw: SharedString,
                         select_all: bool, win, cx);                             // dbl-click/F2
    pub fn begin_data_row(&mut self, cell: CellRef, current: SharedString, cx);  // user typed in field

    // event plumbing
    pub fn sync_from_input(&mut self, origin: EditOrigin, text: &str, cx);       // InputEvent::Change
    pub fn commit(&mut self, mv: Option<MoveDir>, win, cx) -> CommitOutcome;
    pub fn cancel(&mut self, win, cx);
    pub fn on_selection_will_change(&mut self, win, cx);   // commit-first rule (existing behavior)

    // render queries
    pub fn mirror_for(&self, sheet: SheetId) -> Option<(CellRef, &str)>;         // grid live mirror
    pub fn in_cell_editor(&self, sheet: SheetId) -> Option<CellRef>;             // overlay position
    pub fn cap_error(&self) -> Option<(EditOrigin, CapErrorKind)>;               // popover anchor
    pub fn is_editing(&self) -> bool;
}
```

## Internal design

### State machine

```
Idle ──begin_typed(cell, ch)────────► Editing{origin: DataRow, in_cell_open: false, text: ch}
Idle ──begin_in_cell(cell, raw)─────► Editing{origin: InCell,  in_cell_open: true,  text: raw}
Idle ──begin_data_row(cell, cur)────► Editing{origin: DataRow, in_cell_open: false, text: cur}
Editing ──F2──────────────────────► in_cell_open = true, origin = InCell, focus in-cell (text kept)
Editing ──focus moves between editors─► origin follows focus; text unchanged
Editing ──commit(Ok)──────────────► Idle  (+ SetCellInput, + optional selection move, focus grid)
Editing ──commit(CapRejected)─────► Editing (cap_error set, danger border, focus stays)
Editing ──cancel──────────────────► Idle  (restore data-row text to committed raw, focus grid)
Editing ──dbl-click OTHER cell────► commit first (click-elsewhere rule), then begin_in_cell(other)
```

- `begin_*` while `Editing` on a different cell: `commit(None)` first; if `CapRejected`
  the begin is aborted (focus stays in the failing editor — matches existing behavior).
- `begin_typed` with a multi-cell selection targets the selection **anchor**
  (functional spec §1.1); grid passes the anchor.

### Two-inputs sync without feedback loops

Both `InputState`s stay bound the whole time; the overlay merely isn't rendered when
`in_cell_open == false`. Sync rule:

- The **focused** editor is authoritative. `sync_from_input(origin, text)` is accepted
  only when `origin == state.origin`; it updates `state.text`, clears `cap_error`,
  then pushes text to the *other* InputState with `syncing = true` set around the
  `set_value` call. `sync_from_input` returns immediately when `syncing` is true
  (the push echoes back as an InputEvent::Change — this guard breaks the cycle).
- Focus change between the two editors updates `state.origin` (subscribe to both
  focus handles); no text is moved on focus change (already identical).

### Commit

1. Run the existing `freecell-core::input_cap` validation on `state.text`.
2. Reject → `cap_error = Some(kind)`; chrome/overlay render danger border + popover
   (ui_design §4); return `CapRejected`. State otherwise untouched.
3. Accept → send existing `Command::SetCellInput { sheet, cell, text }`; clear state;
   close overlay; return focus to the grid focus handle; grid applies `mv` via its
   existing move logic. The optimistic pending display takes over rendering the cell
   (unchanged MVP §4 path) — the mirror and the pending display never overlap because
   the mirror exists only while `state` is `Some`.

### Tab interception

gpui-component's bare `Input` doesn't emit a commit on Tab (known MVP gap). Both
editors are wrapped in a div with `.on_key_down` registered **before** the input's
handlers; on `tab`/`shift-tab` when `is_editing()`: mark the event handled, call
`commit(Some(Right|Left))`. Enter/Escape keep flowing through the existing
InputEvent::PressEnter/chrome escape paths, now routed to `commit`/`cancel`.

### Grid integration

- `grid/input.rs` printable-key case (~:75-78) emits `GridEvent::TypeToEdit(String)`
  (single selection or anchor-of-multi; modifier-free printables incl. space; `=`
  included). Window shell calls `begin_typed`, sets data-row text, focuses it.
- Double-click: `handle_mouse_down` (grid/view.rs:491) checks `event.click_count == 2`
  on a cell hit → `GridEvent::OpenInCellEditor(cell)`. F2 in the keymap does the same
  for the active cell. Raw content comes from the chrome's existing per-selection
  content fetch (chrome/view.rs:358-368) — the shell passes the already-fetched raw;
  if that fetch is still pending (>250 ms case) the overlay opens empty-with-spinner
  exactly like the data row does.
- Mirror: grid render asks `mirror_for(active_sheet)`; matching visible cell renders
  the raw text (left-aligned, default font — grid skips its RenderStyle for that cell).
- Overlay: rendered by the grid per architecture §4.4 (absolute div at `cell_rect`,
  `deferred()`, min-width 80 px), only when `in_cell_editor(sheet)` returns the cell.

### Migration plan (do first, keep green)

1. Introduce `EditController` owning the *existing* data-row flow only; move commit/
   escape/cap logic out of `chrome/view.rs`; chrome renders from controller state.
   All existing chrome tests (`cap_reject_keeps_editing_and_flags_error`,
   `multiselect_disables_field`, commit-move tests) must pass unmodified in behavior
   (relocate as needed).
2. Then add type-to-replace, mirror, in-cell editor, Tab — each with its own tests.

## Dependencies

Depends on: `freecell-core::input_cap`, gpui-component `InputState` (`new` requires
`&mut Window` — construct both in `WorkbookWindow::build`, shell/window.rs:199-217),
existing `Command::SetCellInput`, grid focus handle, chrome content-fetch state.
Depended on by: chrome view (render-only), grid view (mirror/overlay/type-to-replace),
cap popover (§7.2), clipboard (paste commits pending edit first via
`on_selection_will_change`-equivalent call).

## Test plan

Unit (controller, headless — InputStates faked behind a small trait or driven via cx):
- `typed_replaces_content_and_focuses_data_row` — begin_typed("x") → text=="x", origin DataRow.
- `typed_on_multi_selection_targets_anchor`.
- `f2_promotes_to_in_cell_keeping_text`.
- `dbl_click_other_cell_commits_then_opens` — first cell committed, second editing.
- `sync_is_one_way_no_loop` — push→echo suppressed by `syncing` (assert single update).
- `origin_follows_focus`.
- `commit_cap_reject_keeps_state_sets_error` (both origins).
- `cancel_restores_committed_text_and_grid_focus`.
- `tab_commits_and_moves_right`, `shift_tab_left` (via key-down path).
- `mirror_only_on_active_sheet` — sheet switch hides mirror (edit committed first).
- `begin_aborted_when_pending_commit_cap_rejects`.
Existing chrome tests ported unchanged in assertion (migration step 1 gate).
Render suite: `incell_editor_open`, `incell_editor_danger` (cap state).
