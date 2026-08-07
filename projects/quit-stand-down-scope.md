# Quit stand-down scope: save-failure paths abort a quit prompting a *different* window

**Status: Future** (found in `engine-worker-hardening` Phase 2 review, 2026-07-28).

## The defect

`FreeCellApp::note_prompt_cancelled(cx)` aborts the **global** `QuitPlan` with no reference to the
window it was called from:

```rust
pub fn note_prompt_cancelled(cx: &mut App) {
    cx.update_global::<FreeCellApp, _>(|app, cx| {
        if let Some(plan) = app.quit_plan.as_mut() { plan.cancel(); app.advance_quit(cx); }
    });
}
```

That is correct for a **cancelled prompt** — the gesture is the user answering the quit's own
question. It is wrong for the **save-failure** paths, which can fire in a window the quit was never
waiting on. The result is the quit silently switching off while another window's prompt is still on
screen: the user answers that prompt, its window closes, and nothing further happens.

**Why a clean window can reach these paths.** `WorkbookWindow::save` has no dirty guard (the `Save`
/ `Save As` actions call it unconditionally), so ⌘S on a clean window — or a Save-As panel that
resolves *after* a quit starts — arms `pending_save_req` on a window that `registry.dirty_among`
never put in the plan.

Probed (review round 5): window 0 clean with a save in flight, window 1 dirty; ⌘Q → `plan =
Some((1, false))` with window 1 prompted; a `SaveFailed` folds into window 0 → `plan = None`, and
window 1's prompt is still up and no longer drives anything.

## The three sites

All in `app/crates/freecell-app/src/shell/window.rs`:

| Site | Trigger |
|---|---|
| the `WorkerEvent::SaveFailed` arm | the worker reports a failed write for *this* window's save |
| `abort_save_with_backup_error` | the pre-write `.back` backup could not be created (§7.3) |
| `prompt_then_save`'s cancelled-panel arm | the user cancels window A's native save panel while a quit prompt sits on window B |

**`dismiss_modal`'s cancel path is already safe** and must stay as it is: any window showing an
`UnsavedChanges` modal is dirty, therefore in the plan, so an `is_pending` gate would already be
`true` there. It is the one site where "a prompt was cancelled" really does mean "the quit's own
question was answered".

`on_worker_lost`'s teardown is **not** affected: B1 gave it the scoped
`note_quit_prompt_unanswerable(key, cx)`, which gates on `QuitPlan::is_pending` (see
`specs/projects/engine-worker-hardening/architecture.md §A3.5`).

## Suggested shape

The machinery already exists — `QuitPlan::is_pending(key)`, which `on_window_closed` uses for
exactly this "don't disturb an unrelated window's prompt" reason. Either:

1. **Rename to something behaviour-neutral** — `note_quit_step_unresolvable(key, cx)` — and route
   all four sites (the three above plus `dismiss_modal`) through it. The dismiss path's behaviour
   is unchanged because its window is always pending; the rename stops the name implying "the user
   cancelled" at sites where nobody did. This is the tidier end state.
2. **Give `note_prompt_cancelled` a key** and gate every call site on `is_pending`. Smaller diff,
   keeps the (now slightly inaccurate) name.

Either way each of the three sites wants its own regression test in the shape B1 used: two windows,
one dirty and prompted, the failure raised on the other, asserting the plan survives and the prompt
still resolves it.

## Why it was not fixed in `engine-worker-hardening`

Pre-existing and unrelated to B1: these are `functional_spec.md` §5.2 / §7.3 save-failure paths,
not reachable because of anything Phase 2 changed. Folding them in would have widened a
worker-death diff into unrelated save-flow behaviour with no spec text and no tests of its own. The
B1 review's standing rule was to flag rather than silently change, and this note is that flag.
