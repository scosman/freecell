---
status: complete
---

# Functional Spec: engine-worker-hardening

Scope, behaviour and contracts for the four units in
[`project_overview.md`](project_overview.md). Nothing here is user-facing *feature* work — three
of the four units are invisible when they work. The observable surface is limited to two new
error paths (F1, F2) and F4's one moved counter — a chart-only op now bumps the generation it
announces. (An earlier draft of this line promised a changed event *ordering*, citing a
non-existent "F5". No event's order or payload changes; see §F4.2.)

---

## 0. Confirmation status (done before writing this spec)

The overview requires every finding to be re-derived at HEAD before it is treated as real. All
four were, by reading the source at `6e05e76`:

| Unit | Claim | Verdict at HEAD | Evidence |
|---|---|---|---|
| **B2** | Unclamped frozen-pane band in `build_publication` | **CONFIRMED** | `worker/run.rs:3245-3278` — `for row in (0..m).chain(body_rows)` over `m = cache.frozen_rows()`, unvalidated. Doc comment at `:3241-3244` asserts "never a sheet-size loop". |
| **B2** | Reachable in two clicks | **CONFIRMED, and worse than described** | `grid/view.rs:4696` `boundary_count = menu.run.1 + 1`, where `run` is the *selected* header run. Select-All (⌘A) → right-click a row header → "Freeze rows" ⇒ `SetFrozen { rows: Some(1_048_576) }`. `pre_validate` (`run.rs:2171-2204`) has no `SetFrozen` arm. |
| **B2** | Also reachable from a crafted `.xlsx` | **CONFIRMED, and it hangs a *second* loop** | `engine/src/cache.rs:412-413` copies `ws.frozen_rows` into the cache verbatim; `grid/view.rs:4529` renders `for r in 0..frame.frozen_rows`. So a crafted `<pane ySplit="500000" state="frozen"/>` hangs the **render thread** too, not only the worker. The review missed this half. |
| **B1** | `from_source` / `save_workbook` unguarded | **CONFIRMED** | `run.rs:369` (`from_source`, outside any `catch_unwind`) and `run.rs:757-765` (`save_workbook`). Six other mutation regions are guarded (`:862, :1101, :1166, :1264, :1521, :1634`). |
| **B1** | Reachable `panic!` in the pinned exporter | **CONFIRMED** | `~/.cargo/git/checkouts/ironcalc-*/c1acacb/xlsx/src/export/worksheets.rs:177` — `panic!("Model needs to be evaluated before saving!")` on a `FormulaValue::Unevaluated` cell, with an upstream `// TODO: We should NOT panic here.` |
| **B1** | Silent zombie: handle discarded, `send` swallows | **CONFIRMED** | `worker/client.rs:90-94` — `Builder::spawn(...).expect(...)`, `JoinHandle` dropped. `client.rs:120-122` — `send` is `let _ = self.tx.send(cmd)`. `window.rs:350-361` — the event loop just `break`s when `recv()` yields `None`, with no arm for it. Nothing sends `Command::Shutdown` anywhere in the app. |
| **E1** | `Published` emitted between the value commit and the style commit | **CONFIRMED** | `run.rs:966-971` — `publish(); emit(Published); apply_cache_refresh(...)`, with the comment "Ordered after `Published` (unchanged event order)" making it deliberate. Same shape at `:1194-1200`, `:1236-1246`, `:1563-1572`, `:1708-1720`. |
| **E1** | `commit_chart_op` emits `Published` without publishing | **CONFIRMED** | `run.rs:2926-2934` — bumps `chart_version`, stores the snapshot, emits `Published`. No `publish()`, so `Shared::generation` does not move. Same at `:2346-2348` and `:2411-2413`. |
| **F1** | ~1,200 lines of chart machinery in `run.rs`, next to an empty `charts.rs` | **CONFIRMED** | `run.rs` = 9,288 lines, production 1–3,984. Chart items sum to ≈1,180 production lines across 28 functions + 2 types (30 items; §A4.1 enumerates them — the actual cut was 1,048 lines, see the `implementation_plan.md` closing note). `worker/charts.rs` = 39 lines holding only `ChartSnapshot`. |

Nothing was disproved. `projects/architecture-review-remediation.md` needs one **correction**, not a
retraction: B2's blast radius includes the render thread, so the fix must clamp where *both*
consumers read (see A2 below), not only at the publish site.

---

## 1. F1 — Bounded frozen-pane band (B2)

### F1.1 Behaviour

FreeCell pins at most **64 leading rows** and **32 leading columns**. Past that a freeze is not
a meaningful UI state (the band would fill or exceed the window) and it is a denial-of-service
against both the worker and the render thread.

The cap is enforced at three independent points, because there are three independent ways in:

| Path | Enforcement | User-visible result |
|---|---|---|
| A `SetFrozen` command (the header-menu Freeze item) | **Rejected** in `pre_validate` | An OK-only error dialog. The workbook is unchanged; no undo step is created. |
| A workbook file whose `<pane>` asks for more | **Clamped** when the sheet cache is built | The sheet opens, showing a band clamped to the cap. The file's own bytes are not modified. |
| A **structural edit** that grows the frozen boundary past the cap (insert rows/columns above the band) | **Clamped** when the sheet cache is rebuilt | The band stops growing at the cap. The model keeps the grown count (§F1.3). |
| Any residual path into the publish loop | **Clamped** at the loop | None — a backstop that cannot be observed if the three above hold. |

### F1.2 The rejection

`Command::SetFrozen { rows: Some(n), .. }` with `n > 64`, or `{ cols: Some(n), .. }` with
`n > 32`, is rejected before it reaches the engine.

- Dialog title: **"Can't freeze that many rows"** / **"Can't freeze that many columns"**
- Dialog detail: *"FreeCell can pin at most 64 rows (you asked for 1,048,576). Select fewer
  rows and try again."* — the requested count is echoed so the ⌘A-then-Freeze case explains
  itself.
- OK-only. Nothing changes; no undo entry; the existing freeze (if any) stays.

`n = 0` (Unfreeze) is always valid on both axes.

### F1.3 The clamp, and the model/cache divergence it accepts

A cache built from a worksheet whose `frozen_rows`/`frozen_columns` exceed the cap stores the
capped value. Every consumer — the grid's frozen-band layout and hit-testing, the header menu's
Freeze/Unfreeze label, and the publication's band — therefore agrees, because they all read the
same cache.

Consequence, accepted and documented: when the model's count is over the cap, the model and the
cache disagree. The model keeps its count (so a save preserves it); the cache, and everything
the user sees, uses the capped one.

**Two paths put the model over the cap, and the second is not exotic:**

1. **A crafted or foreign file.** Its `<pane>` asks for more than the cap; the count enters the
   model at load and is capped on the way into the cache.
2. **An ordinary structural edit.** IronCalc adjusts the frozen boundary *inside* an
   insert/delete's own undo diff (the `fix/structural-edits-adjust-frozen-pane` fork fix, relied
   on by `structural_edits_track_frozen_boundary_in_one_undo_step`), and `InsertRows` /
   `InsertColumns` are **not** range-checked against the frozen cap. So freezing at the cap and then
   inserting rows above the band — two ordinary gestures, no hostile input — leaves the model at,
   say, 72 while the cache stays at 64.

**Why path 2 is not "fixed" by re-clamping the model.** It could be: after a structural edit,
issue a `set_frozen_rows(min(count, cap))`. We deliberately do not, because that write is a
*second* undoable diff. It would break `SetFrozen`'s "one action = one undo step" contract and
Insert's alike — one Undo would no longer revert one gesture. Trading a documented,
user-visible undo contract for the tidiness of two numbers agreeing is the worse deal. The
divergence is the cheaper defect, so it is chosen rather than discovered.

**What the divergence actually costs.** Three things, all real.

*The band **boundary** stops moving, while the band's contents do not.* Every surface stays
bounded and safe — the band, the hit-testing, the header-menu label and the publication all read
the clamped cache, and the publish loop's bound (§F1.4) holds. What is pinned is the **number**,
not the view. Walked from a legal freeze at 64:

| Gesture | Model | Band boundary the user sees |
|---|---|---|
| freeze 64 rows | 64 | 64 |
| insert 8 rows above the band | 72 | 64 — unmoved |
| insert 8 more | 80 | 64 — unmoved |
| delete 8 rows in the band | 72 | 64 — unmoved |
| delete 8 more | 64 | 64 |
| delete 8 more | 56 | **56** |

Four consecutive band-affecting gestures leave the boundary at 64, and the fifth moves it. Read
the middle column and nothing appears to happen; that is the confusing part for a user who
freezes at exactly the cap and then reorganises rows.

*Content silently falls out of the pinned region.* This is the visible consequence the row above
does **not** cover, and the one a support report would actually describe. The band renders sheet
rows `0..64` off the clamped publication, and the publication is rebuilt from the *shifted*
sheet. So inserting 8 rows at the top of a 64-row frozen header leaves the band showing 8 blank
rows followed by the first 56 header rows, while the last 8 header rows — pinned a moment
earlier — become body rows and scroll away. Excel would have grown the band to 72 and kept them
pinned; that is exactly what the model did and the clamp discarded. The change is immediate and
large, and it is the price of the cap: past 64 rows there is no band to grow into.

*The saved count is unbounded — on a near-empty sheet.* IronCalc's boundary adjustment
(`base/src/actions.rs:1051`, and the column twin at `:725`) has no upper guard, and `insert_rows`
range-checks only the **populated** dimension — which empty inserted rows do not grow. So on a
sheet with nothing at or below the frozen band, the model's count is not merely "over the cap":
freeze 1 row on a fresh sheet and insert 1,000,000 rows three times and it is 3,000,001 on a
1,048,576-row sheet, and it survives save → reopen. FreeCell then writes a `<pane ySplit>` that is
not a row of the sheet it describes, and another application reading that file gets a structurally
invalid freeze rather than just a larger one.

The near-empty precondition is load-bearing and applies to *every* variant of this: on a sheet
holding data at or below the band, an insert shifts that data down, `max_row` grows, and the
range check closes the window. An ordinary populated workbook stays merely over-cap, which is the
first two costs above and not this one. Details and measurements:
[`projects/frozen-pane-boundary-overflow.md`](../../../projects/frozen-pane-boundary-overflow.md).

**How the divergence ends.** While the model's count stays *within the sheet* it is genuinely
self-healing: deletes inside the band bring it down one deleted row at a time, and the boundary
starts tracking again the moment it drops under the cap — the table's last row is exactly that
moment.

Once the count is over the sheet's own height the word no longer fits, though the reason is not
that recovery is impossible. A single delete is bounded by the sheet (`delete_rows` rejects
`row + count - 1 > LAST_ROW`), so an over-height count comes down at most 1,048,576 per gesture:
measured, 3,000,001 → 1,951,425 → 902,849 → 0 over three whole-axis deletes, each of which also
destroys a sheet's worth of rows. Reachable, but not a recovery path anyone would take. The
one-step escape hatch is **Unfreeze** — offered on the boundary row's context menu, sending
`SetFrozen { rows: Some(0) }` — which clears both sides at once from any count. Reopening the
file in FreeCell also takes path 1 and clamps the view again, though it does not repair the
model.

That third cost is an **engine** defect, not a FreeCell one, and it is not fixed here: per
`CLAUDE.md` a fork bug gets its own `fix/<slug>` branch and one focused upstream PR, never a
compensating workaround in FreeCell and never folded into an unrelated phase. It is captured in
`PROJECTS.md` → [`projects/frozen-pane-boundary-overflow.md`](../../../projects/frozen-pane-boundary-overflow.md).
FreeCell stays bounded regardless, because the cache clamp is what every consumer reads. Once
the fork clamps the boundary to the sheet dimension, the count becomes bounded-but-over-cap and
the self-healing case above becomes the only case.

The **boundary** table above is pinned row by row by
`structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`), so that
half cannot drift back into being an accident. The content-falls-out effect is not pinned by a
test — it is a rendering consequence of the clamped count, and the pixel suite has no
over-cap-freeze fixture — so it is documented here rather than asserted anywhere.

### F1.4 Bound

With the caps in place the publish loop visits at most
`(64 + 512) × (32 + 256) = 165,888` cells — 1.27× the pre-existing
`MAX_PUBLISH_ROWS × MAX_PUBLISH_COLS` budget, and a constant. This replaces the doc comment
that claimed the property without enforcing it.

### F1.5 Out of scope

Telling the user that a freeze was clamped. The clamp stays silent, but **not** on the grounds
first written here ("a case only a crafted file reaches") — §F1.3 path 2 shows an ordinary
insert reaches it too, so that justification is retired. The reason it stays silent is weaker
than "nothing happened" and should be stated as such: the band on screen is the largest one
FreeCell can render, the count it was clamped from could not have been rendered at all, and the
two controls that change the situation — Freeze at a smaller boundary, and Unfreeze — are
already in the header menu the user right-clicks to reach a freeze. A banner would add a
sentence, not an option.

What that *does* buy are the surprises §F1.3 spells out: while the model is over the cap the band
**boundary** does not move and rows drop out of the pinned region instead, and a workbook can be
saved with a `<pane>` count FreeCell itself will not display (today, an unbounded one). If either
bites in practice — a support report of "my frozen header lost its bottom rows after an insert",
or "my freeze changed when I opened it in Excel" — the answers are respectively a **band-level**
hint and a **save-time** notice, not a load-time banner, and both belong in `PROJECTS.md` rather
than here. The unbounded half of the second is already tracked there as an engine fix.

---

## 2. F2 — Load and save survive an engine panic (B1)

### F2.1 Load

A panic inside `WorkbookDocument::from_source` is caught and reported as a typed load failure,
exactly like a corrupt-file error:

- Dialog title: **"Couldn't open the workbook"** (the existing `LoadFailed` dialog)
- Detail: *"The calculation engine crashed while opening this file. The file may use a feature
  FreeCell can't read yet."*
- The window closes on dismiss (existing `LoadFailed` behaviour — the window has no document).

The worker thread then exits, as it already does for any load failure.

### F2.2 Save

A panic inside `save_workbook` is caught and reported as a typed save failure:

- Dialog title: **"Couldn't save the workbook"** (the existing `SaveFailed` dialog)
- Detail: *"The calculation engine crashed while writing the file. Your work is still open and
  unchanged — try Save As to a new file."*
- The window stays open. Saves are atomic (temp + rename), so the destination file is
  untouched — the existing `SaveError` contract still holds.

**CSV export is guarded on the same terms.** `Command::ExportCsv` walks the same engine over
every cell of a sheet, two statements below the save in the same batch, so a panic there is the
same class of failure and gets the same treatment: caught, reported as `CsvExportFailed` with the
typed `SaveError::EnginePanic` (dialog title: **"Couldn't export the CSV"**), counted toward the
poisoning budget, with no `EditRejected` riding along. The export is atomic, so no partial `.csv`
is left behind, and the document is untouched either way (an export never changes dirty/path).

After a caught save panic the worker probes the model. If the model is now unresponsive the
worker degrades (existing `WorkerDegraded` path: the degraded bar appears and mutating controls
are disabled). If the model still answers, the worker carries on — one failed save is not a
reason to condemn the document. Unlike the edit path, a caught save panic does **not** also emit
`EditRejected` — the `SaveFailed` dialog has already told the user.

### F2.3 Worker death is never silent

If the worker thread dies for any reason not covered above — a panic outside every guard, an
abort-adjacent unwind, a future unguarded path — the window must say so. The rule:

> The worker→UI event stream ending **without** a shutdown the window itself requested is fatal
> to that window.

On that condition the window:

1. logs at `error` how the thread ended. The stream closes when the worker's frame drops its
   event sender, which happens *before* the thread has finished unwinding the rest of the
   `Worker`, so a UI-thread probe at that instant usually answers "still running" and says
   nothing about the cause. The window therefore takes the join handle and joins it on the
   **background** executor, logging the real outcome (panicked vs. clean) when it lands. The UI
   thread never parks.
2. enters the existing **degraded** state — degraded bar visible, mutating controls disabled,
3. clears any "Opening <name>…" loading state (window flag + grid overlay). A worker can die
   *before* `Loaded` — everything after the guarded `from_source` still runs unguarded on
   freshly parsed input — and an overlay left up would spin forever behind the dialog,
4. shows an OK-only dialog:
   - Title: **"The calculation engine stopped"**
   - Detail: *"This window can't be edited or saved any more. Its unsaved changes are lost.
     Open the file again to keep working."*

   This report fires **once**, with no re-arm, so it replaces whatever modal is showing —
   *except* a dialog that is itself a terminal report (a load failure, which closes the window
   on dismiss). A load failure is emitted and *then* the thread returns, so the accurate
   "Couldn't open the workbook" dialog survives; an unsaved-changes prompt or a merge confirm
   does not, because it is offering choices that can no longer be carried out.
5. stays open, so the last rendered data is still readable.

The window is honest rather than tidy here: it does not close itself and it does not pretend a
Save might work. Concretely, in the worker-lost state:

- **No save path runs.** `Save`, `Save As` and the unsaved-changes prompt's Save all refuse with
  an OK-only notice ("This workbook can't be saved from this window any more. Open the file again
  to keep working."); **Export as CSV** refuses the same way, naming what it refused ("This
  workbook can't be exported from this window any more…"). Nothing is sent and nothing is armed.
  This is not defensive tidiness: `DocumentClient::send` drops a command when the worker is gone,
  so a save that *looked* accepted would never receive `Saved` or `SaveFailed` — the native panel
  would pick a file that is never written. The notice does not replace a *terminal* dialog: on a
  window whose load failed, the close-on-dismiss report stays up (⌘S still routes to Save whatever
  modal is showing, and swapping that dialog would stop the window closing on dismiss).
- **Anything already in flight is abandoned, not left waiting.** Refusing new saves is not enough
  — the worker can die *during* one, which is this whole section's premise. So on worker loss the
  window also clears the armed save (`close_after_save`, the pending save request and path, the
  pending export request) and, when a quit is waiting on **this** window, stands it down. Without
  that, two orderings hung: a worker dying while the quit prompt was up left the plan pending on a
  window that could never answer (⌘Q looked like a no-op, and closing another dirty window later
  re-prompted this one), and a save armed before the death stayed armed forever, so the window
  never closed and the quit never finished.
- **The bar carries no Save As button.** The degraded bar's "Save As to keep your work" is an
  offer only a *live* worker can honour, so the lost-worker state renders its own bar: "The
  calculation engine stopped. This window is read-only and its unsaved changes can't be saved —
  open the file again to keep working." Same look, no button, and no contradiction with the
  dialog on top of it.
- **The close prompt offers only choices that can be honoured.** Closing a dirty window whose
  worker is gone prompts with **Cancel** and **Close Without Saving** only (title: "Unsaved
  changes can't be saved"). Offering Save would arm a close-after-save that could never fire,
  leaving the window open forever.
- **Losing the worker of a window the quit is waiting on stands the whole quit down**, rather than
  skipping that one window. The user asked to quit with that document's changes *handled*, and that
  is no longer possible, so the honest answer is to stop and let them decide — the same choice a
  failed save and a failed `.back` backup already make. The document stays **dirty** (losing the
  worker changes no op accounting), so a re-issued ⌘Q prompts that window again, now with the
  Cancel / Close Without Saving form; discarding closes it and the quit runs on. Nothing is wedged
  — the quit just has to be asked for again.

  "Waiting on" includes a window **queued behind** the one being prompted, and the consequence is
  worth stating: the quit stands down while the prompt already on screen stays up, so answering
  that prompt no longer drives anything — the question was withdrawn, and re-issuing ⌘Q is what
  restarts it. The narrower rule (stand down only when the dying window is the one *currently*
  prompted) is worse: the dead window's fatal report would then be replaced by an unsaved-changes
  prompt when its turn came.
- **A death in a window the quit was never waiting on leaves the quit alone.** Unlike a cancelled
  prompt, a worker death is not a user gesture and can land in any window — a clean one, or one
  whose prompt is already resolved. Standing the quit down from *there* is the case with no
  redeeming reading at all: nothing about that window's document is in question, so the quit would
  be switched off on account of a window it never involved. Hence the gate — the quit must be
  pending on the dying window — the same scope rule that already stops an unrelated window
  *closing* mid-quit from disturbing the prompt in flight.

This is a *distinct* state from the degraded worker of F2.2, not a second reason for it: a
degraded worker is alive and answering, so its bar's Save As really writes the file.

Today nothing sends `Command::Shutdown`, so in practice *every* stream close is fatal. The
window still checks the flag rather than assuming, so an orderly shutdown added later doesn't
pop a false alarm. It separately checks whether it *has* a worker at all, so the worker-less
test client's closed-from-birth stream is not reported as a death.

### F2.4 Out of scope

Restarting the worker, recovering the document into a fresh worker, or auto-saving a rescue
copy. All are real ideas and all are larger than this project; if the owner wants them they
belong in `PROJECTS.md`.

---

## 3. F3 — Chart machinery leaves `run.rs`

Pure code motion into `worker/charts.rs`. **No observable behaviour changes at all** — same
commands, same events, same ordering, same public API. The only externally visible artefact is
that `freecell-engine`'s module layout changed.

Success criterion: `run.rs` production lines drop by ≈1,180, and every existing test still
passes unmodified in substance.

The project reports `run.rs`'s post-extraction production line count against the 2,000-line
ceiling CI will enforce next round. If it is still over — it will be, at ≈2,800 — the project
does not silently leave it over: it names what should move next, in the implementation plan's
closing note, as input to F2 of the remediation plan.

> **Outcome (measured).** The success criterion was met in substance but not in magnitude: the
> extraction removed **1,048** production lines, not ≈1,180, and every existing test passed
> unmodified in substance. `run.rs` landed at **3,048**, not ≈2,800, and is **3,192 as of this
> commit** once Phase 4 and the Phase 1/2 CR rounds are counted — measured as of this commit and
> expected to move again as later work lands. Still over the ceiling, exactly as this section
> anticipated, so the closing-note obligation stands and is discharged there.
> **F2 should be sized off 3,192 / −1,048, not off the ≈2,800 / ≈1,180 predicted here.** See
> `architecture.md` §A4.4 and the `implementation_plan.md` closing note for the reconciliation.

---

## 4. F4 — One commit point for the four shared surfaces (E1)

### F4.1 The question that must have an answer

Four surfaces cross the worker→UI boundary:

| Surface | Primitive | Read by the UI on |
|---|---|---|
| `Publication` (cell values) | `ArcSwap` + `generation` | every frame; repaint on `Published` |
| Style/geometry cache | `RwLock<SheetCaches>` | every frame; refresh on `StyleCacheUpdated` |
| `ChartSnapshot` | `ArcSwap` + its own `version` | `Loaded` / `Published`, version-gated |
| CF rule map | `RwLock<HashMap<..>>` | `CondFmtUpdated` |

After this project, the following is true and testable:

> **When a reader observes `generation == N`, all four surfaces are at generation N or later.
> No surface write for generation N happens after the bump to N, and no event announcing
> generation N is emitted before the bump.**

Note the direction: a surface may legitimately run **ahead** of the committed generation; what is
forbidden is a surface running **behind** it. That is what a reader can act on, and it is what the
ordering tests assert.

**Every writer of these four surfaces, and why each leaves the invariant standing.** Two sit
outside the commit point inside the worker, on purpose, and both are forward-only:

- **The load path.** Before the worker's loop starts, `load_and_run` seeds the publication, the
  active sheet's style cache and the CF map, and announces them with `Loaded` +
  `StyleCacheUpdated`, with `generation` never leaving 0. It is **not** routed through the commit,
  because a commit there would emit `Published` *ahead of* `Loaded` — and F4.2 fixes the event set
  (nothing added, nothing removed, nothing reordered against `Loaded`). Its soundness rests on a
  different argument from the one above: the **`Loaded` channel send** is the happens-before edge,
  so a UI that has seen `Loaded` has seen those writes. Every surface is at generation 0, the
  committed generation, so none is behind.
- **Wrap-driven row auto-grow** (`AutoGrowRowHeights`, §3.4) writes row heights straight into the
  resident cache and emits a bare `StyleCacheUpdated` with no commit and no bump. It is a
  cache-only geometry update that rides no undo stack, applying a fresh UI measurement **on top of**
  the committed cache — it never rewinds the cache to a pre-commit state. So the cache runs at or
  ahead of the committed generation, which is legal, and never behind it.

Two more live **outside the engine crate**, and are named here so this is a completeness claim
rather than a scoped one:

- **`DocumentClient::set_chart_snapshot`** stores a snapshot straight into the shared swap. It is
  `#[cfg(feature = "test-support")]` and exists only so a headless window/view test can drive the
  seam-fed chart install with no worker running — so there is no committed generation to be behind.
- **`GridView::autogrow_measure_now`** (`freecell-app`) takes a write lock on the very
  `Arc<RwLock<SheetCaches>>` the worker owns and writes measured wrap row heights into it. It is a
  `pub fn` with **no** `cfg` gate — a real gap in the fence rather than a compile-time-impossible
  one — but its only caller anywhere is the pixel render harness (`render-tests/src/render.rs`),
  which renders a single static frame over a shut-down worker. It writes the same forward-only
  geometry as wrap auto-grow above (and skips already-overridden rows), so the invariant survives.
  Listed because a completeness claim a reviewer is asked to check has to name it.

### F4.2 What changes observably

**Not the order of the events.** An earlier draft of this section promised that
`StyleCacheUpdated` and `CondFmtUpdated` would arrive *before* their batch's `Published`. They do
not, and they never did: every pre-project publishing site emitted `Published` first and the
surface deltas after it — `run.rs` carried the comment *"Ordered after `Published` (unchanged event
order)"* on that very line — and `commit`'s announce phase emits them in exactly the same order
(`Published`, then `StyleCacheUpdated`, then `CondFmtUpdated`, then `SheetsChanged`; §A5.1's
pseudocode always showed this, and `commit_emits_nothing_before_the_bump` pins it). **No event
moved.** What moved is the *writes*.

- **The surface writes now precede the bump, and therefore precede every announcement.** All four
  surfaces are written before the single `Release` store of `generation`. Previously the value
  commit stored the publication, bumped, emitted `Published`, and only *then* wrote the style cache
  and the CF map — so `Published` announced a generation whose other surfaces were still being
  assembled, and a reader sampling in that window saw generation-N values against
  generation-(N−1) styles. Each event still follows its own surface's write, exactly as before;
  what is new is that it also follows *all* of them.
- **A chart-only op moves the counter.** Insert / delete / move / resize / retype / re-range a
  chart used to emit `Published` with `Shared::generation` standing still. It now goes through the
  commit, so the counter it announces actually advances. This is the one change a caller can read
  off an API (`DocumentClient::generation()`); the UI already re-reads everything on `Published`,
  so nothing on screen differs — the event just stops lying.
- **A sheet activation coalesced with an edit publishes the right cells.** Such a batch used to
  build the newly activated sheet's style cache *after* publishing, and the publication reads that
  sheet's frozen-band counts off exactly that cache — so a frozen band came up empty until
  something else republished. Both now ride the batch's one commit (§A5.1).

No event is added, removed, reordered or re-payloaded. Nothing the UI *reads* changes shape
— `Publication`, `SheetCaches` and the CF map are untouched; `ChartSnapshot` gains one
additive field (§F4.3) and keeps `version` with its exact current meaning.

### F4.3 `ChartSnapshot` gains a commit stamp

`ChartSnapshot::version` keeps its current job unchanged: it is bumped **only when the charts
actually change**, and the UI installs only on a change. Making it the generation would
re-install charts on every scroll and destroy the "off-screen free" property.

A new `ChartSnapshot::generation` field records the commit the snapshot was published at. It
is the chart surface's answer to "what does the UI see at generation N" — needed because,
unlike the two `RwLock` surfaces, an `ArcSwap` payload can be read with no lock edge to reason
from. The UI does not read it today; the ordering tests do.

### F4.4 What is explicitly *not* in scope

- **Changing what the UI reads.** If unification had required reshaping a surface the UI
  consumes, that would exceed this project and would have been raised. It does not: every
  change is to *when* a write lands and *when* an event fires. This was the overview's named
  risk, and it did not materialise.
- **B3, cache lock hold-time.** Unifying the commit point does not make it free — the long
  hold in `refresh_cache_cells` is a property of that function's loop, not of where it sits in
  the commit. Untouched, still deferred to v3+.
- **Collapsing `StyleCacheUpdated` / `CondFmtUpdated` into `Published`.** Tempting once
  everything commits together, but it changes what the UI reads (it would have to diff to find
  what changed), and it is protocol work — F4 in the remediation plan, v2.0.

---

## 5. Error handling summary

| Condition | Recoverable? | Surface |
|---|---|---|
| `SetFrozen` over the cap | Yes — nothing applied | `EditRejected` → OK-only dialog (F1.2) |
| File `<pane>` over the cap | Yes — clamped silently | none (F1.3) |
| Structural edit grows the boundary over the cap | Yes — clamped silently; model diverges by design | none (F1.3) |
| Panic in `from_source` | No, for that window | `LoadFailed` → dialog, window closes on dismiss (F2.1) |
| Panic in `save_workbook`, model still healthy | Yes | `SaveFailed` → dialog, window continues (F2.2) |
| Panic in `save_workbook`, model poisoned | No | `SaveFailed` dialog **and** `WorkerDegraded` bar (F2.2) |
| Panic in the CSV export | Yes | `CsvExportFailed` → "Couldn't export the CSV" dialog (F2.2) |
| Worker thread dies | No, for that window | `error!` log (join off the UI thread) + worker-lost bar (no Save As) + dialog; save/export refused; the close prompt drops its Save (F2.3) |

---

## 6. Constraints

- **Performance.** No path may get slower. The publish bound grows from 131,072 to a
  worst-case 165,888 probe cells (F1.4) and is otherwise unchanged. The commit unification
  moves work, it does not add any: the same surfaces are written the same number of times per
  batch — one `ArcSwap` store of the chart snapshot per commit, not the two the first cut made.
  Routing chart ops through the full commit adds **no** `build_publication`: a chart op changes no
  cell value, so its commit re-stamps the resident publication under the new generation instead of
  rebuilding it (`architecture.md §A5.3`). That matters because `SetChartChrome` is **not** a
  discrete gesture — it is sent live per keystroke while a chart or axis title is typed — so a
  rebuild per op would have put a full-viewport republish behind every character.
- **Compatibility.** No file-format change. A workbook saved by FreeCell before and after this
  project is byte-identical for the same edits.
- **Threading.** The seam's existing invariants are preserved without exception: one thread
  owns the `UserModel`; the UI does one wait-free `ArcSwap` load per frame; no `block_on`, no
  blocking `recv`, zero engine calls on the render path (the process-global counter with its
  negative control still guards that and must stay green).
- **Behaviour freeze.** Phases 1–3 change no observable behaviour beyond F1's and F2's new
  error paths. Phase 4 changes **no event's order or payload**: it moves the surface *writes* ahead
  of the generation bump, and makes a chart-only op bump the counter it announces (§F4.2).
