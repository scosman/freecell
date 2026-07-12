# Decisions to review — Feature Gaps 7_11

Judgment calls made during coding that a human may want to sanity-check. One bullet per call,
tagged by phase.

## Phase 2 — Quick-edit mode (§5)

- **"Modified arrow" excludes the `function` modifier (cross-platform fix).** The caret-intent
  predicate is `shift || control || alt || platform` (shared helper `grid::caret_intent_modifiers`),
  NOT gpui's `Modifiers::modified()`. macOS sets `Modifiers::function` on the arrow / Home / End
  cluster itself, so `modified()` would flag a *plain* arrow as modified and defeat §5.2's
  commit-and-move on macOS (the in-container Linux gpui backend never sets `function`, so it would
  pass CI silently). This mirrors `command_for_key`, which never consults `function`. Applied to
  both the data-row arm and the (dead) grid in-cell arm; covered by
  `quick_edit_plain_arrow_with_function_flag_still_moves` and
  `quick_edit_caret_intent_modifier_arrow_leaves_without_moving`.

- **Caret-intent "leave quick-edit" via `on_mouse_down`, NOT the `on_content_event` Focus
  handler.** `architecture.md §5.1/§5.3` suggested clearing `quick_edit` "on formula-bar focus
  (`on_content_event` Focus)". Reading the code showed that would break type-to-replace:
  `InputEvent::Focus` is emitted from a *deferred* `on_focus` observer, so `begin_typed`'s
  programmatic `input.focus()` fires Focus **after** `begin_typed` returns — clearing the flag in
  the Focus handler would immediately undo the quick-edit it just set. Instead the "clicked into
  the formula bar" case is handled precisely by an `on_mouse_down` on the data-row field (which is
  exactly the §5.3 mouse case), plus Home/End and modified-arrow in the key handler. The flag
  starts `false`, so a user who clicks straight into the formula bar (no type-to-replace) never has
  it set anyway — nothing to clear. Net behavior matches the spec; only the mechanism differs from
  the pointer. Low risk.

- **Grid-root in-cell arrow arm is a defensive symmetric mirror (currently unreachable).** The
  task asked to intercept arrows in both the data-row and the grid-root in-cell `capture_key_down`
  and to thread `quick_edit` into the grid via `ChromeGridRequest::EditState`. Both are done. But
  in the current flow type-to-replace edits live in the **data row** and never open the in-cell
  overlay, and `begin_in_cell` clears `quick_edit`, so `incell_open.is_some() && quick_edit` cannot
  co-occur — the grid arm is dead in practice. It is implemented for symmetry / future-proofing (a
  future overlay-hosted quick-edit) and commented as such. No `leave_quick_edit` routing was added
  to the grid arm (it would need a new `GridEvent` + window route for a path that can't execute).
  Flag if a future feature makes the overlay host a quick-edit — the leave-cases would then need
  wiring back to the chrome.
