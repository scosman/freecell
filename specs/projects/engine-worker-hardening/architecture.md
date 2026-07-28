---
status: complete
---

# Architecture: engine-worker-hardening

Technical design for the four units in [`functional_spec.md`](functional_spec.md). Single
document — the project is four contained changes to one crate plus one UI file, not a system
with components worth their own docs.

**File ownership.** This project owns `engine/src/worker/*`, `core/src/publication.rs`, and the
event-loop half of `app/src/shell/window.rs`. It additionally touches
`engine/src/cache.rs` (two lines — §A2.3) and `engine/src/document.rs` (two error variants —
§A3.1), neither of which is claimed by a parallel project. It does **not** touch
`app/.../chrome/view.rs`, `app/.../grid/view.rs`, manifests, workflows, or `engine/chart/*`.

---

## A1. Where things stand today

The worker's commit sequence for an edit batch, at HEAD (`run.rs:956-1012`):

```
collect_edited_ranges          → touch sets
reresolve_charts               → WRITES chart_snapshot   (+ chart_version)
publish                        → WRITES publication, BUMPS generation
emit(Published)                                          ← UI may read all four HERE
apply_cache_refresh            → WRITES caches           (+ emits StyleCacheUpdated)
refresh_cf_caches_after_recompute → WRITES caches        (+ emits StyleCacheUpdated)
reconcile_published_cond_fmt   → WRITES cond_fmt         (+ emits CondFmtUpdated)
```

Two of the four surfaces are written *after* the generation bump and *after* the event that
tells the UI to read them. That is the whole of E1: the window between `emit(Published)` and
`apply_cache_refresh` is a real window in which the grid paints generation-N values against
generation-(N−1) styles.

Five call sites share this shape (`:966`, `:1194`, `:1236`, `:1563`, `:1708`), each open-coded.
Three more (`:2346`, `:2411`, `:2932`) emit `Published` while writing only the chart snapshot,
leaving `generation` untouched.

---

## A2. Unit 1 — Bounded frozen-pane band (B2)

### A2.1 Constants

New, in `worker/run.rs`, beside the existing `MAX_PUBLISH_ROWS` / `MAX_PUBLISH_COLS`:

```rust
/// The most leading rows / columns FreeCell will pin (`functional_spec.md F1`).
///
/// The frozen band is published on EVERY publish, on top of the body window, so an unbounded
/// band is an unbounded publish — `(0..M) × (0..K)` from a value the UI derives from a header
/// SELECTION, which Select-All makes `1_048_576`. It is also rendered by the grid as
/// `for r in 0..frozen_rows`, so an unbounded band wedges the render thread as well.
///
/// The caps are sized past any usable freeze — a band taller than the window makes the sheet
/// unusable, and a 4K display shows ~100 rows — and keep the worst-case publish at
/// `(64 + MAX_PUBLISH_ROWS) × (32 + MAX_PUBLISH_COLS)` = 165,888 probe cells, a constant.
const MAX_FROZEN_ROWS: u32 = 64;
const MAX_FROZEN_COLS: u32 = 32;
```

They are exported `pub(crate)` so `engine/src/cache.rs` can apply the same numbers.

### A2.2 Rejection in `pre_validate`

`protocol.rs` gains:

```rust
/// Which axis a frozen-pane rejection refers to (`EditRejectedReason::FrozenPaneTooLarge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenAxis { Rows, Columns }

impl FrozenAxis {
    /// The lowercase plural noun for the dialog copy ("rows" / "columns").
    pub fn noun(self) -> &'static str { … }
}
```

and one `EditRejectedReason` variant:

```rust
/// A `SetFrozen` asked to pin more tracks than FreeCell supports (`functional_spec.md F1.2`).
/// Carries the axis, the requested count and the cap so the dialog can name all three.
FrozenPaneTooLarge { axis: FrozenAxis, requested: u32, max: u32 },
```

`Worker::pre_validate` gains an arm. Both axes are checked (the UI sends one, the command
permits both); rows are checked first so a hypothetical both-axes command reports the row
failure:

```rust
Command::SetFrozen { rows, cols, .. } => {
    if let Some(n) = rows.filter(|n| *n > MAX_FROZEN_ROWS) {
        return Err(EditRejectedReason::FrozenPaneTooLarge {
            axis: FrozenAxis::Rows, requested: n, max: MAX_FROZEN_ROWS });
    }
    if let Some(n) = cols.filter(|n| *n > MAX_FROZEN_COLS) {
        return Err(EditRejectedReason::FrozenPaneTooLarge {
            axis: FrozenAxis::Columns, requested: n, max: MAX_FROZEN_COLS });
    }
    Ok(())
}
```

`pre_validate` already runs outside the panic guard and its `Err` already routes to
`EditRejected`; no plumbing is needed.

### A2.3 Clamp at cache build

`engine/src/cache.rs:412-413`, the one place a worksheet's frozen counts enter the read model:

```rust
builder.set_frozen_rows((ws.frozen_rows.max(0) as u32).min(worker::MAX_FROZEN_ROWS));
builder.set_frozen_cols((ws.frozen_columns.max(0) as u32).min(worker::MAX_FROZEN_COLS));
```

This is the load-bearing half of the fix and the correction to the review: because *both* the
publish loop and the grid's frozen-band renderer read the cache (never the model), clamping
here bounds both. Clamping only at the publish site — what the review proposed — would leave
`grid/view.rs:4529` looping to 500,000 on the render thread.

### A2.4 Clamp at the publish site

`build_publication` clamps its own inputs regardless:

```rust
let (m, k) = self.shared.caches.read().get(sheet)
    .map(|c| (c.frozen_rows().min(MAX_FROZEN_ROWS), c.frozen_cols().min(MAX_FROZEN_COLS)))
    .unwrap_or((0, 0));
```

Redundant if A2.3 holds, and deliberately so: this is the loop whose *comment* asserted the
bound. The remediation plan's H1 sweep exists because comments do not hold; this is its worked
example, so the loop enforces its own precondition rather than inheriting it.

The doc comment above the loop is rewritten to state the enforced bound instead of asserting an
unenforced one.

### A2.5 Tests

| Test | Location | Asserts |
|---|---|---|
| `set_frozen_beyond_the_cap_is_rejected` | `run.rs` unit | ⌘A-scale `SetFrozen { rows: Some(1_048_576) }` → `EditRejected { FrozenPaneTooLarge { .. } }`, `frozen(&worker, sheet) == (0, 0)`, no `Published` |
| `set_frozen_at_the_cap_is_accepted` | `run.rs` unit | `rows: Some(64)` and `cols: Some(32)` apply; the boundary is inclusive |
| `set_frozen_cols_beyond_the_cap_is_rejected` | `run.rs` unit | the column axis reports `FrozenAxis::Columns` and its own cap |
| `publish_clamps_an_oversized_frozen_band` | `run.rs` unit | seed the cache with a sheet-size band directly (bypassing both guards), publish, assert `publication.frozen_rows == 64` and that the publish returns at all |
| `crafted_pane_element_opens_clamped` | `run.rs` unit | extends the existing `<pane>` fixture (the `pane_fixture` / `pane_fixture_with` helpers in `run.rs`'s test module) with `ySplit="500000"`; the sheet opens and `frozen(&worker, sheet).0 == 64` |
| `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` | `run.rs` unit | freeze at the cap, insert rows above the band: the model's count grows past 64 while the cache, the band and the publication stay at 64 (`functional_spec.md F1.3` path 2); Unfreeze clears both |
| `oversized_freeze_is_rejected_and_the_worker_keeps_serving` | `worker_seam.rs` | the same rejection over the real spawned-worker seam |
| `edit_rejected_frozen_pane_too_large_names_the_axis_request_and_cap` (+ `…_uses_the_column_noun`) | `shell/app.rs` gpui test | the dialog itself: exact title, and a detail carrying the cap, the **grouped** request and the axis noun; dismissing does not close the window |

The publish-clamp test is the one that would **hang** on a full regression, and `cargo test` has
no per-test timeout — so it runs the publish on a spawned thread and bounds it with
`recv_timeout` (30 s), which turns a never-returning loop into a test failure. `Timeout` and
`Disconnected` are handled separately: a timeout is reported as the hang it is, while a
disconnect joins the thread and re-raises its panic, so a crash inside `commit` is never
mislabelled as a hang. On a real timeout the wedged thread is left detached; the harness exits
the process without joining it. A separate, tighter wall-clock assertion (5 s, measured across
the whole spawn→result round trip) catches a *partial* regression — a clamp that still bounds the
loop but costs far too much.

---

## A3. Unit 2 — Load/save panic guards and worker-death surfacing (B1)

### A3.1 New error variants

`engine/src/document.rs`:

```rust
// LoadError
/// The calculation engine panicked while opening the file (caught; `functional_spec.md F2.1`).
#[error("The calculation engine crashed while opening this file. The file may use a feature \
         FreeCell can't read yet.")]
EnginePanic,

// SaveError
/// The calculation engine panicked while writing the file (caught; `functional_spec.md F2.2`).
/// The pinned exporter still contains a reachable `panic!` on an unevaluated formula cell.
#[error("The calculation engine crashed while writing the file. Your work is still open and \
         unchanged — try Save As to a new file.")]
EnginePanic,
```

Both enums are `thiserror`-derived and already render through `error.to_string()` into the
existing dialogs (`window.rs:476-487` and `:609-627`), so no UI plumbing is needed for these two.

### A3.2 Guarding the load

`Worker::load_and_run`:

```rust
let doc = match catch_unwind(AssertUnwindSafe(|| WorkbookDocument::from_source(&source))) {
    Ok(Ok(doc)) => doc,
    Ok(Err(error)) => { let _ = event_tx.try_send(WorkerEvent::LoadFailed { error }); return }
    Err(_) => {
        tracing::error!("worker: caught panic in from_source; reporting LoadFailed");
        let _ = event_tx.try_send(WorkerEvent::LoadFailed { error: LoadError::EnginePanic });
        return
    }
};
```

`AssertUnwindSafe` is correct here for the same reason it is in the six existing guards: the
only state the closure touches is `source` (immutable) and the not-yet-constructed document,
which is dropped on unwind.

### A3.3 Guarding the save

The save loop borrows `&mut self` inside the closure, matching the existing guarded regions:

```rust
for (path, req_id) in saves {
    let outcome = catch_unwind(AssertUnwindSafe(|| self.save_workbook(&path)));
    match outcome {
        Ok(Ok(())) => self.emit(WorkerEvent::Saved { req_id, ops_seen: self.ops_seen }),
        Ok(Err(error)) => self.emit(WorkerEvent::SaveFailed { req_id, error }),
        Err(_) => {
            tracing::error!("worker: caught panic in save_workbook; reporting SaveFailed");
            self.emit(WorkerEvent::SaveFailed { req_id, error: SaveError::EnginePanic });
            self.note_caught_panic();
        }
    }
}
```

`handle_caught_panic` is split so the save path can reuse the poisoning policy without the
edit-path event:

```rust
/// The locked poisoning policy WITHOUT the `EditRejected` announcement: count the panic,
/// probe the model, and degrade on a second panic or an unresponsive probe. Returns whether
/// the worker is now degraded. The edit paths wrap this; the save path uses it directly,
/// because `SaveFailed` has already told the user (`functional_spec.md F2.2`).
fn note_caught_panic(&mut self) -> bool { … }

fn handle_caught_panic(&mut self) {
    if !self.note_caught_panic() {
        self.emit(WorkerEvent::EditRejected { reason: EditRejectedReason::EnginePanic });
    }
}
```

Behaviour for the six existing call sites is bit-identical — the same count, the same probe, the
same threshold, the same events.

**What `AssertUnwindSafe` over `&mut self` actually costs here.** The first version of this
section claimed `save_workbook`'s only `self` mutations are its last statements, so a panic could
leave nothing half-applied. **That was wrong**, and the code has been changed rather than the
comment: `save_workbook`'s *first* statement is `ensure_all_charts_discovered()`, which binds
charts, bumps `chart_version`, and (since Phase 4) commits — and inside its loop it called
`discover_and_parse_for_part`, a zip/XML parse of user-supplied bytes and the most panic-prone
call on the save path.

So a caught save panic *can* leave part of that sweep applied. The sweep is therefore made
**re-entrant** instead of assumed atomic: it walks sheets in `SheetId` order (so the survivors are
deterministic rather than hash order) and records a sheet in `discovered_chart_sheets` only after
**both** its parse and its bind have returned — the `insert` is the last statement of the loop
body, below the whole `match`, so a panic anywhere in either step leaves that sheet unmarked. What
survives a panic is therefore a prefix of *fully bound* sheets plus `charts_fully_discovered ==
false` — a partially-swept worker that the next save, or a lazy per-sheet discovery, completes.
Marking *before* the parse (the original order) left sheets flagged "walked" whose charts were
never bound: `ensure_sheet_charts_discovered` then returned early for them forever while
`charts_fully_discovered` stayed `false`, so their charts silently went missing until some later
save re-ran the whole sweep.

The **lazy** sibling `ensure_sheet_charts_discovered` keeps the opposite order (mark, then parse),
and its doc comment now says why rather than leaving the asymmetry to look like an oversight: it
does not run inside any guard, so a panic there takes the thread down outright and there is no
surviving worker to leave torn — and marking up front is what stops a cleanly-failing parse being
re-attempted on every paint. If it ever moves inside a guard it must adopt the sweep's order.

The late mutations (`chart_source_path`, `loaded_anchor_edits`, `loaded_deletes`) *are* the last
statements before the `Ok`, after every fallible step, so those genuinely cannot be half-applied.
The call-site comment states both halves.

**The CSV export is guarded the same way** (`Command::ExportCsv`, two statements below the save
in the same batch): the same engine over every cell of a sheet, reported as `CsvExportFailed`
with `SaveError::EnginePanic` and counted through `note_caught_panic`. Its `AssertUnwindSafe` is
unconditionally sound — the closure takes only shared borrows and mutates no worker state — which
is cheaper to establish than an argument for leaving it out.

### A3.4 Retaining the handle and surfacing death

`worker/client.rs`:

```rust
/// How the worker thread ended, for the window's fatal-death report (`functional_spec.md F2.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExit {
    /// The thread returned normally.
    Clean,
    /// The thread unwound out of `load_and_run` — a panic no guard caught.
    Panicked,
    /// The thread has not finished yet (nothing was joined).
    Running,
}
```

`DocumentClient` gains two fields:

```rust
/// The worker thread's handle, retained so a death can be reported as panicked vs. clean
/// instead of being inferred from a closed channel. Taken by the first `worker_exit()` call.
join: Mutex<Option<JoinHandle<()>>>,
/// Set when the window itself asked the worker to stop, so an expected stream close is not
/// reported as a crash. Nothing sends `Shutdown` today; the flag keeps the check honest for
/// when something does.
shutdown_requested: AtomicBool,
/// Whether this client was built over a real worker thread at all — `false` only for the
/// worker-less `detached()` test client, whose stream is closed from birth. Deliberately NOT
/// folded into `shutdown_requested`: that flag answers "did we ask the worker to stop", and
/// storing `true` into it from a constructor that never had a worker is a lie the moment
/// anything else reads it.
has_worker: bool,
```

with:

```rust
pub fn send(&self, cmd: Command) {
    if matches!(cmd, Command::Shutdown) {
        self.shutdown_requested.store(true, Ordering::Release);
    }
    let _ = self.tx.send(cmd);
}

pub fn shutdown_requested(&self) -> bool { … }
pub fn has_worker(&self) -> bool { … }

/// How the worker thread ended, WITHOUT ever blocking — this is called from the UI thread. A
/// thread that has not finished reports `Running` and keeps its handle.
pub fn worker_exit(&self) -> WorkerExit { … }

/// Takes the retained handle so the caller can join it OFF the UI thread.
pub fn take_worker_handle(&self) -> Option<JoinHandle<()>> { … }

/// Joins a taken handle and reports the outcome. BLOCKS — background executor only.
pub fn join_worker(handle: JoinHandle<()>) -> WorkerExit { … }
```

`Running` is the **common** answer at the moment the stream closes, not a rare one — the
first draft of this section ("by then the join returns immediately") was wrong. The stream
closes when the worker's frame drops `event_tx`, and the thread then still has to unwind the
whole `Worker` (the `UserModel`, the caches, the chart bindings) before it is finished — a
structural ordering, not a race that happens to be lost sometimes. (The code review that found
this measured 159/200 runs answering `Running` over ~8 MB of worker state; a real workbook is
heavier.) So
`Running` must not be a dead end: the window takes the handle and `join_worker`s it on the
background executor, logging the real outcome when it lands.

`DocumentClient::detached()` (the `test-support` constructor) sets `join: Mutex::new(None)`,
which `worker_exit` reports as `Clean`, and `has_worker: false`, which is what stops its
closed-from-birth stream being read as a death.

Using `parking_lot::Mutex` (already a dependency, already used for `Shared::caches`) keeps this
lock-poison-free.

### A3.5 The window half

`window.rs::spawn_event_loop` gains a tail, and one new method:

```rust
cx.spawn_in(window, async move |this, cx| {
    while let Some(event) = receiver.recv().await { … }
    // The stream ended. If the window is gone this is a no-op; if it is alive, the worker died.
    let _ = this.update_in(cx, |this, window, cx| this.on_worker_lost(window, cx));
})
```

```rust
/// The worker→UI event stream closed. Unless the window asked for it, the worker thread is
/// gone and this document can no longer be edited or saved (`functional_spec.md F2.3`) — log
/// it, enter the degraded state, and say so. Called once, when the event loop's `recv()`
/// yields `None`.
fn on_worker_lost(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    if !self.client.has_worker() || self.client.shutdown_requested() { return; }
    self.report_worker_exit(cx);          // `Running` → take the handle, join in the background
    self.worker_lost = true;              // the state that takes every save path out of service
    self.degraded = Some("the calculation engine stopped".to_string());
    self.chrome.update(cx, |c, cx| c.set_degraded(true, cx));
    self.loading = None;                  // a death before `Loaded` must not leave the overlay up
    self.grid.update(cx, |g, cx| g.set_loading(None, cx));
    if !current_modal_is_a_terminal_report { self.modal = Some(ActiveModal::Error { … }); }
    cx.notify();
}
```

**Which modal survives.** Only one that is *itself* a terminal report — the load-failure error
with `close_window_on_dismiss: true`. That case is the reason the guard exists: a failed load
emits `LoadFailed` and *then* returns, closing the stream, and its accurate "Couldn't open the
workbook" must not be replaced by the generic crash one. A blanket `modal.is_none()` guard (the
first draft) went further than that reason supports: it also suppressed the report behind an
`UnsavedChanges` prompt or a merge `Confirm`, and since `on_worker_lost` runs **once** with no
re-arm, dismissing that prompt left the user with the bar and no notice at all — the F2.3 report
lost entirely. Those modals are replaced.

**`worker_lost` is a separate field, not a `degraded` string.** The two states differ in exactly
what the degraded bar offers. A degraded worker is alive and answering, so its "Save As to keep
your work" button works. A *lost* worker cannot write anything: `DocumentClient::send` drops the
command, so no `Saved`/`SaveFailed` ever returns. Hence, gated on `worker_lost`:

- `save()` / `send_save()` (and `export_csv`) refuse with an OK-only notice. `send_save` is
  checked as well as `save` because the native panel is async — the worker can die while it is
  open. The notice does **not** replace a terminal report: a window whose load failed is showing
  the close-on-dismiss dialog *and* is worker-lost, and ⌘S routes to `save` regardless of the
  modal, so without that guard the refusal would swap a terminal dialog for a non-terminal one and
  the window would stop closing on dismiss.
- the bar renders without the `Save As…` button, with copy matching the dialog.
- the unsaved-changes prompt drops its **Save** button (Cancel / Close Without Saving only).

**Guarding the entry to a save is not sufficient**, and this was the round-1 gap. Those refusals
only cover requests *starting* after the loss; they do nothing for a request already in flight —
and the unguarded panic that kills the worker can happen mid-save, which is B1's own premise.
`on_worker_lost` therefore performs the teardown **on every worker loss**, not only when a save
was attempted afterwards (`abandon_work_waiting_on_the_worker`): `close_after_save`,
`pending_save_req`, `pending_save_path` and `pending_export_req` are cleared, and the quit
stand-down below runs. Two orderings make it necessary:

| Ordering (both reproduced as failing tests before the fix) | Without the teardown |
|---|---|
| Worker dies while the **quit prompt** is up | `on_worker_lost` swaps `UnsavedChanges` for the fatal `Error`, so `dismiss_modal`'s quit-abort branch — which only fires for `UnsavedChanges` — never runs. The plan stays pending on a window that can never answer: ⌘Q looks like a no-op, and closing some *other* dirty window later re-prompts this one out of nowhere. |
| A save **already armed** when the worker dies | `close_after_save` + `pending_save_req` are only ever cleared by `Saved`/`SaveFailed`, neither of which can arrive. The window never closes and `advance_quit` waits on it forever — the round-1 failure mode, untouched by the entry guards. |

**The stand-down is scoped to windows the plan is pending on.** It goes through
`FreeCellApp::note_quit_prompt_unanswerable(self.key, cx)`, not the unscoped
`note_prompt_cancelled` the user-driven paths call, because a worker death is not a user gesture:
it can land in a window the quit never included — a clean one, or one already resolved — where
nothing about that window's document is in question and aborting would switch the quit off on
account of a window it never involved. Probed: two windows, only window 0 dirty, ⌘Q prompts it,
then the clean window 1's worker dies and the plan is gone, so answering window 0's still-visible
prompt closes that window and nothing further happens. The gate is `QuitPlan::is_pending`, the same
check (for the same reason) `on_window_closed` already applies so an unrelated window *closing*
mid-quit doesn't disturb the prompt in flight.

`is_pending` is true for a window **queued behind** the one being prompted, so a death there does
still stand the quit down while another window's prompt is on screen — that prompt is left
standing and answering it drives nothing. That is deliberate, not a residue of the gate: the
narrower rule (`plan.next() == Prompt(key)`) would let the quit reach the dead window's turn and
replace its fatal report with an unsaved-changes prompt, which is worse. F2.3 bullet 5 states the
consequence rather than leaving it implied.

When the plan **is** waiting on the dying window the whole quit stands down, not just that
window's step — matching `abort_save_with_backup_error` and the `SaveFailed` arm. `advance_quit`
then takes `QuitStep::Aborted` and drops the plan entirely, so there is nothing left for a later
window close to revive. The window stays dirty, so a re-issued quit prompts it again in the
Cancel / Close Without Saving form, both of which resolve.

`on_edit_rejected` gains an arm for `FrozenPaneTooLarge` (exhaustive match), producing the F1.2
dialog.

### A3.6 Tests

| Test | Location | Asserts |
|---|---|---|
| `load_panic_is_caught_and_reported_as_load_failed` | `run.rs` unit | a source that panics in `from_source` yields `LoadFailed { EnginePanic }` and the thread does not take the process down |
| `save_panic_is_caught_and_reported_as_save_failed` | `run.rs` unit | a save that panics yields `SaveFailed { EnginePanic }`, the worker keeps answering a subsequent edit + a real save, and no `EditRejected` rides along |
| `export_panic_is_caught_and_reported_as_export_failed` | `run.rs` unit | the same for `ExportCsv` → `CsvExportFailed { EnginePanic }`, worker still serving |
| `a_save_panic_mid_sweep_keeps_the_prefix_and_leaves_the_rest_re_discoverable` | `charts.rs` unit | a save on a two-sheet workbook that really has charts to discover, panicking on the **second** sheet's parse: the first stays bound + marked, the second is not marked, and lazy discovery still binds it afterwards |
| `a_save_panic_counts_toward_the_degrade_threshold` | `run.rs` unit | the split preserves the 2-panic / failed-probe threshold exactly |
| `shutdown_requested_tracks_only_the_shutdown_command` | `client.rs` unit | `send(Shutdown)` sets the flag; other commands do not; a spawned client reports `has_worker` |
| `worker_exit_reports_clean_after_a_requested_shutdown` | `client.rs` unit | a real worker sent `Shutdown` joins `Clean` |
| `worker_exit_reports_a_thread_that_unwound` | `client.rs` unit | the `Err(_) => Panicked` arm — a thread that unwound out of its entry point |
| `a_running_worker_reports_running_and_stays_joinable` | `client.rs` unit | `Running` does **not** consume the handle; `take_worker_handle` + `join_worker` then report the real outcome |
| `the_worker_less_client_reports_no_worker_rather_than_a_shutdown` | `client.rs` unit (`test-support`) | `detached()` is `has_worker: false`, *not* a fake shutdown request |
| `worker_death_degrades_the_window_and_says_so` | `window.rs` gpui test | drop the sender behind a detached-live client's event channel → the window is degraded and shows the fatal modal |
| `worker_death_after_a_load_failure_keeps_the_load_dialog` | `window.rs` gpui test | `LoadFailed` then stream close → the "Couldn't open the workbook" dialog survives |
| `worker_death_replaces_a_non_terminal_modal` | `window.rs` gpui test | an `UnsavedChanges` prompt showing at death is replaced by the fatal report, not swallowed by it |
| `worker_death_clears_the_loading_overlay` | `window.rs` gpui test | a death before `Loaded` clears "Opening …" instead of spinning behind the dialog |
| `a_lost_worker_refuses_to_save` | `window.rs` gpui test | `save` on a lost worker writes nothing, arms no `pending_save_req`, disarms `close_after_save`, and explains itself |
| `a_refused_save_keeps_a_terminal_load_dialog` | `window.rs` gpui test | ⌘S on a load-failed, worker-lost window does not swap the close-on-dismiss dialog for the refusal notice |
| `worker_death_during_a_quit_prompt_stands_the_quit_down` | `window.rs` gpui test | ordering (a): the plan is abandoned, dismissing the report does not revive it, and a re-issued quit still prompts + resolves |
| `worker_death_disarms_a_save_that_was_already_in_flight` | `window.rs` gpui test | ordering (b): a save armed before the death is cleared and the waiting quit stood down |
| `a_clean_windows_worker_death_leaves_someone_elses_quit_alone` | `window.rs` gpui test | a death in a window the plan never included leaves the quit running and its prompt answerable |
| `a_bind_panic_mid_sweep_also_leaves_the_sheet_re_discoverable` | `charts.rs` unit | the sweep marks a sheet only after the **bind** too — the ordering that marks between parse and bind fails this while passing the parse-side test |

**Deviation from this section, recorded rather than left silent.** The plan above called for
`worker_seam.rs` integration tests over a `#[cfg(feature = "test-support")]`
`DocumentSource::TestPanic` variant plus a `Command::TestPanicOnSave` flag. The implementation
instead keys the injections off a `#[cfg(test)]` sentinel **file name**
(`document::PANIC_SENTINEL`), which adds no public shape at all — a better trade, and the one
kept. The consequence is that the panic hooks are invisible outside the crate's own unit tests
(`#[cfg(test)]` items do not exist for an integration-test build), so these tests live in the
`run.rs` / `charts.rs` / `client.rs` unit modules rather than in `worker_seam.rs`. The guards are
exercised at their real call sites (`process_batch`, `load_and_run`) either way.

---

## A4. Unit 3 — Chart extraction

### A4.1 What moves

Into `worker/charts.rs`, which already holds `ChartSnapshot` and is the natural home:

- **Types:** `AuthoredEntry` (`run.rs:145-164`), `ChartUndo` (`:209-257`).
- **`impl Worker` methods** (≈1,000 lines): `reresolve_charts`, `build_chart_sheet_part_map`,
  `ensure_sheet_charts_discovered`, `bind_discovered`, `ensure_all_charts_discovered`,
  `sheet_name_of`, `authored_write_list`, `insert_authored_chart`, `set_chart_anchor`,
  `delete_chart`, `set_chart_range`, `bind_authored_range_at`, `set_chart_type`,
  `set_chart_chrome`, `resolve_authored_chart`, `reresolve_authored`, `commit_chart_op`,
  `push_chart_undo`, `undo_chart_op`, `redo_chart_op`, `undo_chart_entry`, `redo_chart_entry`,
  `store_chart_snapshot` (since split into `mark_charts_changed` + `stamp_chart_snapshot`, §A5.2),
  `charts_by_sheet_with_authored`.
- **Free functions:** `apply_chrome_edit`, `existing_chart_parts`, `source_ranges_from_refs`,
  `next_chart_part`.
- **Tests:** the chart-only `#[test]`s from `run.rs`'s test module, into a test module in
  `charts.rs`.

Staying in `run.rs`: `save_workbook` (a save function that happens to call chart code),
`build_publication`, `publish`, the command dispatch, and everything else.

### A4.2 Making it compile

`Worker` lives in `run` (private module); `charts` is its sibling under `worker`, so
`charts.rs` can name `super::run::Worker` but cannot reach its private fields. Two mechanical
adjustments:

1. **Only the `Worker` fields the chart code actually reads** become `pub(super)` — visible
   within `mod worker` only, which is where both modules live. `Worker` itself stays
   `pub(super)`, so nothing outside the worker module can see the type at all, let alone its
   fields.

   > **Amended after Phase 3 CR.** This item originally read "`Worker`'s fields become
   > `pub(super)`" — a blanket, written for convenience at spec time rather than as a considered
   > decision, and in tension with this section's own "smallest widening that compiles"
   > principle. The blanket widened all 24 fields; only **16** are reachable from outside
   > `run.rs`. The other **8** — `event_tx`, `active_sheet`, `viewport`, `eval_count`,
   > `panic_count`, `clipboard`, `manual_rows`, `wrap_heights` — are private again, verified by
   > narrowing all 24 and letting `cargo check -p freecell-engine --all-targets` name the ones
   > the compiler demands back. Unused `pub` never warns, so this could not have self-corrected.
   > Widen per field, as needed, and re-derive the set the same way if the chart code's reads
   > change.
2. `Touch`, `UndoEntry`, `AppliedOp` and the small shared helpers the chart code calls
   (`resolve`, `emit`, and after Phase 4, `commit`) become `pub(super)` for the same reason.

An alternative — `run/charts.rs` as a child module — would need no visibility change, but the
overview names `worker/charts.rs` as the destination and that file already owns the chart half
of the seam. The `pub(super)` widening is contained to one module and is the better home.

Test helpers (`test_worker`, `sheet0`, `drain_events`, `set_input`, `quiet_panics`) move to a
`#[cfg(test)] pub(super) mod testutil` inside `run.rs` so both test modules share them
verbatim, with no behavioural edit to any test body.

### A4.3 Verification

The extraction is behaviour-preserving, so its verification is that **every existing test
passes with its assertions unchanged**. Test bodies may gain an import; nothing else about them
may change. Any test that needed a substantive edit to keep passing is a signal the move was not
mechanical, and is called out rather than accommodated.

### A4.4 The ceiling

`run.rs` production is 3,984 lines. The extraction removes ≈1,180, landing at ≈2,800 — still
above the 2,000-line ceiling F2 will enforce. The implementation plan closes with a named
proposal for what should move next; it does not attempt it.

> **Outcome (measured, as of the Phase 3 CR commit).** The prediction above was optimistic on
> both counts. The extraction removed **1,048** production lines, not ≈1,180, and `run.rs`
> landed at **3,048**, not ≈2,800 — ~250 above the predicted point. Two independent causes,
> both verified: (a) F1's ≈1,180 was an approximate sum of the chart items' spans and ran ~130
> lines high — all **30** items §A4.1 lists did move (28 functions + 2 types), and `charts.rs`
> production went 39 → 1,113
> (**+1,074**), so nothing was left behind; (b) the ≈2,800 assumed the 3,984 baseline, but
> Phases 1–2 added 112 lines *before* the cut (4,096 − 1,048 = 3,048). With Phase 4 and the
> Phase 1/2 CR rounds on top, `run.rs` production is **3,192 as of this commit** — it will move
> again as later work lands, so re-measure rather than treating this figure as fixed. The
> conclusion the prediction drew is unchanged: still well over the 2,000 ceiling.
> **F2 should size off 3,192 / −1,048.** Full table: `implementation_plan.md` closing note.

---

## A5. Unit 4 — One commit point (E1)

### A5.1 The commit primitive

A single method replaces the eight open-coded sequences:

```rust
/// The ONE commit point for the four worker→UI shared surfaces (`functional_spec.md F4`).
///
/// Ordering contract, and the reason this is a single function:
///   1. every surface write for generation N happens HERE, before the bump;
///   2. `generation` is stored (Release) exactly once, and that store is the commit;
///   3. every event announcing N is emitted AFTER the store.
///
/// A UI reader that observes `generation == N` therefore sees all four surfaces at N or later.
/// The invariant is directional: no surface may ever be BEHIND the committed generation; one
/// running ahead is legal. Two forward-only writers stay outside this function on purpose —
/// the load path and wrap auto-grow (`functional_spec.md F4.1`).
fn commit(&mut self, staged: StagedCommit) { … }
```

with the per-batch inputs bundled so the sites read the same way:

```rust
/// What a batch has to commit. Empty vectors are the norm (a scroll republish stages nothing
/// but the publication), and every field is cheap to leave empty.
#[derive(Default)]
struct StagedCommit {
    /// Cells whose style cache entries must be re-read.
    refresh: Vec<(SheetId, CellRange)>,
    /// Sheets whose style caches must be rebuilt wholesale.
    rebuild: Vec<SheetId>,
    /// The sheet list as it was before the batch — drives the caches/CF map reconcile and the
    /// `SheetsChanged` event. `None` when the batch cannot change the sheet set.
    sheets_before: Option<Vec<SheetMeta>>,
    /// Re-run the value-dependent CF cache refresh (a recompute happened).
    cf_after_recompute: bool,
    /// Sheets whose published CF rule list must be reconciled.
    cf_sheets: Vec<SheetId>,
    /// Build the active sheet's cache if it isn't resident yet (a sheet activation).
    ensure_active_cache: bool,
    /// The batch changed no cell value — a chart-only commit. `stage_publication` re-stamps the
    /// resident `Publication` with the new generation instead of rebuilding it (A5.3).
    values_unchanged: bool,
}
```

A batch that coalesces a `SetViewport` **and** an edit passes `ensure_active_cache: true` into the
edit batch's own `StagedCommit` — the activation's cache build is staged into that one commit, not
performed after it. (Phase 4 first fixed only the pure-viewport branch and left the mixed one
committing with `ensure_active_cache: false` and building the cache after the `Release` store. That
also made the committed publication wrong, not just late: `build_publication` reads the frozen-band
counts off the very cache that had not been built yet, so the publication claimed `frozen_rows = 0`
and omitted the band's values. Regression test:
`a_batched_sheet_switch_and_edit_commit_the_new_sheets_cache_together`.)

`commit`'s body, in order:

```
// ---- stage: every shared-surface write for this generation ----
let generation = self.shared.generation.load(Acquire) + 1;
let style_sheets = self.stage_cache_refresh(staged.refresh, staged.rebuild, …);
let cf_updated   = self.stage_cond_fmt(staged.cf_sheets, …);
self.stage_publication(generation);          // charts were staged by the caller, see A5.2
self.stamp_chart_snapshot(generation);
// ---- commit: one Release store ----
self.shared.generation.store(generation, Release);
// ---- announce: nothing above this line may be re-ordered below it ----
self.emit(WorkerEvent::Published);
for sheet in style_sheets { self.emit(StyleCacheUpdated { sheet }) }
for sheet in cf_updated   { self.emit(CondFmtUpdated { sheet }) }
if sheets_changed { self.emit(SheetsChanged { sheets }) }
```

The `stage_*` helpers are the existing `apply_cache_refresh`, `refresh_cf_caches_after_recompute`,
`reconcile_published_cond_fmt` and `publish` with their `emit` calls lifted out and returned as
lists. That is the entire refactor: the work each does is unchanged, the emissions move.

### A5.2 Chart staging

`reresolve_charts` already runs before the publication is staged at every site, and it must stay
there — it reads the post-edit model to recompute chart values, so it belongs with the other
staging. It keeps its job of re-resolving and bumping `version` iff something changed, but it does
**not** store the snapshot: it calls `mark_charts_changed()`, and
`commit` then calls `stamp_chart_snapshot(generation)`, which is the worker's **only** writer of
`shared.chart_snapshot`. When the flag is set it builds the payload fresh from the live bindings and
stamps it with `generation`; otherwise it re-stores the resident payload under the new generation —
one `ArcSwap` store of a structurally identical value (a `Vec` of `Arc<[ChartSpec]>`, so refcount
bumps, not chart copies). Either way the stamp is truthful for every generation rather than only the
ones charts moved on.

The flag exists to keep it to **one** store: the chart paths used to store the snapshot on the spot,
stamped with the generation *currently* committed, and then `commit` immediately re-stored a
structurally identical value one generation later. `stamp_chart_snapshot` has no "already at this
generation" early-return, because it would be dead code — nothing else stamps, and `generation` is
always `shared.generation + 1`.

`ChartSnapshot`:

```rust
pub struct ChartSnapshot {
    /// Bumped ONLY when the bound charts change. The UI installs iff this differs from what it
    /// last installed, so a scroll-only publish never re-installs (the "off-screen free"
    /// property). Deliberately NOT the generation.
    pub version: u64,
    /// The generation this snapshot was committed at (`functional_spec.md F4.3`) — the chart
    /// surface's answer to "what does the UI see at generation N". Unlike the two `RwLock`
    /// surfaces, an `ArcSwap` payload carries no lock edge to reason from, so it carries the
    /// stamp instead. Read by the ordering tests, not by the UI.
    pub generation: u64,
    pub sheets: Vec<(SheetId, Arc<[ChartSpec]>)>,
}
```

Additive; `ChartSnapshot::empty()` yields `generation: 0`.

### A5.3 `commit_chart_op` stops lying

```rust
fn commit_chart_op(&mut self) {
    self.ops_seen += 1;
    self.shared.committed_ops.store(self.ops_seen, Ordering::Release);
    self.chart_version += 1;
    self.mark_charts_changed();
    self.commit(StagedCommit::chart_only());   // was: self.emit(Published)
}
```

Same for the two lazy-discovery sites (`ensure_sheet_charts_discovered`,
`ensure_all_charts_discovered`).

**Cost, corrected.** The first cut had every chart op pay a full `build_publication` over the
current viewport, justified as "the same cost class as one scroll republish, and a chart op is a
discrete user gesture". The second half of that is **false for one of the six chart commands**:
`SetChartAnchor` is discrete (`ChartAnchorChanged` fires on mouse-up, not per drag frame — verified),
but **`SetChartChrome` is sent live per keystroke** while a chart or axis title is typed
(`chrome/view.rs::on_chart_title_event`), and chart ops are processed one-by-one — so five
characters would have been five full-viewport rebuilds (≈20k engine probes each, 165,888 worst
case) where they used to be five `emit`s.

So a chart op commits with `StagedCommit::chart_only()` (`values_unchanged: true`): the generation
still moves — the whole point — but `stage_publication` **re-stamps** the resident `Publication`
under the new generation instead of rebuilding it. A chart op provably changes no cell value, and
the viewport cannot have moved since the last commit (`process_batch` handles `SetViewport` and
commits the activation before any chart op runs), so the re-stamp is exact. It costs one clone of
an already-built cell vector and no engine call at all. The restamp path is skipped, and a full
build taken, if the resident publication is for a different sheet — belt-and-braces.

`ensure_all_charts_discovered` runs from inside `save_workbook`. Committing there is correct
(the snapshot really did change) and harmless (the save has not written anything yet). One
consequence to name rather than discover: that call sits inside the **B1 save `catch_unwind`**, so
whatever the commit does now runs under that guard.

Measured rather than assumed: on this path the commit makes **no engine call at all**. It is a
`chart_only()` commit, so nothing is refreshed or rebuilt, `sheets_before` is `None` (so not even
`sheet_metas()` runs), and `stage_publication` takes the restamp branch — unconditionally here,
because `publication.sheet == active_sheet` is an invariant (`active_sheet` is only ever assigned in
`load_and_run` and in the `SetViewport` routing arm, and every `SetViewport` commits). So the guard's
blast radius is unchanged in practice; only the *fallback* `build_publication` would put
`formatted_value` under the save guard, and that fallback is unreachable given the invariant. The
conservative framing is kept because the fallback is real code: if it ever ran, an IronCalc panic in
it would report as a `SaveError::EnginePanic` (and count toward degrading the worker) rather than as
a publish-path panic.

### A5.4 The call sites

| Site | Today | After |
|---|---|---|
| `apply_edit_batch` (`:966`) | publish, emit, cache, CF, sheets | one `commit` with the full `StagedCommit` |
| `process_batch` scroll republish (`:627`) | publish, emit | `commit(StagedCommit::default())` |
| `apply_replace_all` (`:1194`) | publish, emit, refresh, CF | one `commit` |
| `commit_replacements` (`:1236`) | ditto | one `commit` |
| `commit_paste` (`:1563`) | ditto (+ `Pasted` after) | one `commit`, then `Pasted` |
| `apply_set_font` (`:1708`) | publish, emit, refresh | one `commit` |
| `ensure_sheet_charts_discovered` (`:2346`) | store snapshot, emit | `mark_charts_changed`, then `commit(StagedCommit::chart_only())` — the snapshot store belongs to `commit` (§A5.2) |
| `ensure_all_charts_discovered` (`:2411`) | ditto | ditto |
| `commit_chart_op` (`:2932`) | ditto | ditto |
| `ensure_active_cache_built` on sheet switch (`:631-636`) | build cache, emit | folded into the batch's `commit` — including a batch that coalesces the activation with an edit (`ensure_active_cache` rides that batch's `StagedCommit`) |
| `load_and_run` first publish / cache build (`:432-437`) | build cache, publish, emit | **unchanged — deliberately not converted** |

The load path is the one site that stays open-coded, and the reason belongs in the record rather
than in the omission: routing it through `commit` would emit a `Published` **ahead of** `Loaded`,
which F4.2 forbids (no event added, removed or reordered). It is sound on a different argument —
the `Loaded` channel send is the happens-before edge for those writes — and every surface it
touches is at generation 0, the committed generation, so nothing is left behind. `functional_spec.md`
F4.1 states the exception; so does `commit`'s doc comment, alongside the second sanctioned
forward-only writer (`apply_auto_grow`).

`Pasted`, `ReplacedCount`, `EvalStarted` / `EvalFinished`, `EditRejected` and the read replies
are **not** commit events — they are replies to a specific command and keep their current
positions.

### A5.5 Ordering tests

Two new tests in `worker_seam.rs`, both following `publish_before_bump_never_shows_a_stale_generation`'s
shape — a spinning reader thread, a violation counter, and a **non-zero sample assertion** so the
test cannot pass by never observing the interleaving.

**`all_surfaces_agree_at_a_generation`.** The reader loop, while the writer drives `ROUNDS` = 24
serialized edits (one per commit — the loop awaits each round's `Published` so a coalesced batch
cannot collapse several rounds into one generation):

```
let gen  = client.generation();
let pubn = client.publication();
let snap = client.chart_snapshot();
let styled = client.caches().read().get(sheet).map(|c| c.<style of the edited cell>);
// every surface must be at `gen` or later — never behind it
if pubn.generation < gen { violations += 1 }
if snap.generation  < gen { violations += 1 }
if styled_is_from_a_generation_before(gen) { violations += 1 }
samples += 1;
advanced += (gen != last_gen) as u64;
```

Assertions: `violations == 0`, `samples > 0`, **and `advanced > 10`** — the last is the
non-vacuity guard the overview asks for. A reader that never saw the counter move proves
nothing, so the test fails rather than passing quietly.

The style surface carries no generation of its own, so it needs a generation-comparable value. Each
round is a **number-format** edit: `SetStylePath { path: NumFmt, value: decimals(i) }` on A1, where
`decimals(i)` has exactly `i` decimal places. One command moves the published display text of the
seeded `1` (to `"1."` + `i` zeros) *and* the cached `num_fmt` code together, in one commit, and both
encode `i` directly — so "the style cache is behind the publication" is directly observable as a
decimal count that disagrees with the published text. A count that only ever grows makes "behind"
checkable. That is the concrete form of the split-brain E1 describes, and it is what the test
catches. (An earlier draft of this paragraph described a *fill colour* and "200 edits" — neither
was ever the fixture, and the fill story contradicted this section's own description of the
number-format assertion.) The workbook carries no chart and no CF rules; the chart snapshot is still
checked, as an empty payload restamped by every commit.

**`a_chart_op_bumps_the_generation_it_announces`.** Send a chart op, await `Published`, assert
`client.generation()` strictly increased and `chart_snapshot().generation == client.generation()`.
This is the regression test for `commit_chart_op` — against the pre-project code it fails, because
the counter does not move.

A third, cheap unit test in `run.rs` — `commit_emits_nothing_before_the_bump` — is the direct
statement of the contract in A5.1, asserted **at the store itself**. `commit` fires a `#[cfg(test)]`
thread-local hook (`COMMIT_STORE_PROBE`, the same house style as `Command::TestPanic` /
`document::PANIC_SENTINEL`) immediately before its `Release` store; the test installs a probe that
drains the event queue there and reads all four surfaces.

At that instant it asserts (a) no commit announcement — `Published` / `StyleCacheUpdated` /
`CondFmtUpdated` / `SheetsChanged` — is queued, and (b) every surface already carries the generation
about to be stored: the publication holds this batch's value, the chart snapshot is stamped, and the
style cache holds this batch's number format.

**(b) is the half that discriminates, and the reason the hook exists.** The first version of this
test compared positions in the drained event vector only, and that is vacuous: hoist
`stage_publication` + the store + `emit(Published)` back to the top of `commit` — the pre-Phase-4
shape, where the style cache and CF map are written after the bump — and the drained order is
unchanged, so the test stayed green against the very regression it names. Draining at the store does
not discriminate either (the pre-Phase-4 shape also stores before it emits). Only reading the
surfaces at the store does; verified by reverting the ordering and watching the number-format
assertion fail.

### A5.6 What this does not do

- The cache write lock's hold time is unchanged (B3, still deferred). `stage_cache_refresh` holds
  it for exactly as long as `apply_cache_refresh` did.
- `StyleCacheUpdated` / `CondFmtUpdated` survive as separate events. Folding them into `Published`
  would change what the UI reads; it is F4 protocol work, v2.0.
- No surface's *shape* changes except `ChartSnapshot`'s additive field. The overview's named
  escalation risk — "if unifying requires changing what the UI reads" — was checked against all
  four surfaces and does not apply.

---

## A6. Testing strategy

Per-phase, crate-scoped (the project convention):

```
cargo fmt --all --check                            # always, whole workspace
cargo build -p freecell-engine
cargo test  -p freecell-engine --lib
cargo test  -p freecell-engine --test worker_seam  # phases 1, 2, 4
cargo build -p freecell-app && cargo test -p freecell-app --lib   # phase 2 (window.rs)
```

A single `--workspace` build + test runs once, at the end, as the pre-merge validation.

**Render tests are out of scope.** The project touches no grid-render code, no fonts, no
layout, no borders, no fills, no titlebar and no chart render widgets. Phase 4 changes *when*
published content is committed, never *what* it contains — every publication this project
produces is byte-identical to the one HEAD produces for the same state, apart from a frozen band
that HEAD could only produce by hanging. Per the project convention, no pixel-suite run is
planned; if a phase turns out to alter published content, the relevant `render_tests.sh test
<prefix>` subset runs instead of the full suite.

---

## A7. Risks

| Risk | Mitigation |
|---|---|
| The `pub(super)` field widening in A4.2 invites future coupling | It is confined to `mod worker` (three files). `Worker` itself stays `pub(super)`, so the type is invisible outside the module. |
| Reordering `StyleCacheUpdated` before `Published` breaks a test asserting the old order | **Did not arise — no event moved.** The premise was wrong: every pre-project site already emitted `Published` first and the surface deltas after it, and `commit`'s announce phase reproduces that order exactly (§F4.2). The survey behind this row — every affected test uses `.any(..)` over a drained event vector, not positional matching, checked across `run.rs` and `worker_seam.rs` at the time — still holds and is now belt-and-braces; `commit_emits_nothing_before_the_bump` pins the order positively. |
| Chart ops now republish, perturbing a chart-op perf path | `set_chart_anchor` was verified during Phase 4 to fire on drop, not per drag frame — but `SetChartChrome` is sent **live per keystroke** (`chrome/view.rs::on_chart_title_event`), so "a chart op is a discrete gesture" does not hold and a `build_publication` per op was the wrong trade. Resolved rather than absorbed: a chart op commits with `values_unchanged`, re-stamping the resident publication (A5.3) — no engine work per op. |
| The load/save panic injection needs `test-support` surface that leaks | Both hooks are `#[cfg(feature = "test-support")]` / `#[cfg(test)]`, matching the existing `Command::TestPanic` precedent. If the surface grows beyond the bug, the seam test is dropped and the unit test of the guard stands alone — stated, not silently skipped. |
| The frozen cap rejects a freeze a real user wanted | 64 rows is ~2/3 of a 4K display's visible rows and ~20× a normal header freeze. The rejection names the cap and the request, so it is self-explaining rather than mysterious. |
