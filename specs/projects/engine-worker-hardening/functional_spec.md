---
status: complete
---

# Functional Spec: engine-worker-hardening

Scope, behaviour and contracts for the four units in
[`project_overview.md`](project_overview.md). Nothing here is user-facing *feature* work — three
of the four units are invisible when they work. The observable surface is limited to two new
error paths (F1, F2) and one changed event *ordering* (F5).

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
| **B1** | Reachable `panic!` in the pinned exporter | **CONFIRMED** | `~/.cargo/git/checkouts/ironcalc-*/cee2859/xlsx/src/export/worksheets.rs:177` — `panic!("Model needs to be evaluated before saving!")` on a `FormulaValue::Unevaluated` cell, with an upstream `// TODO: We should NOT panic here.` |
| **B1** | Silent zombie: handle discarded, `send` swallows | **CONFIRMED** | `worker/client.rs:90-94` — `Builder::spawn(...).expect(...)`, `JoinHandle` dropped. `client.rs:120-122` — `send` is `let _ = self.tx.send(cmd)`. `window.rs:350-361` — the event loop just `break`s when `recv()` yields `None`, with no arm for it. Nothing sends `Command::Shutdown` anywhere in the app. |
| **E1** | `Published` emitted between the value commit and the style commit | **CONFIRMED** | `run.rs:966-971` — `publish(); emit(Published); apply_cache_refresh(...)`, with the comment "Ordered after `Published` (unchanged event order)" making it deliberate. Same shape at `:1194-1200`, `:1236-1246`, `:1563-1572`, `:1708-1720`. |
| **E1** | `commit_chart_op` emits `Published` without publishing | **CONFIRMED** | `run.rs:2926-2934` — bumps `chart_version`, stores the snapshot, emits `Published`. No `publish()`, so `Shared::generation` does not move. Same at `:2346-2348` and `:2411-2413`. |
| **F1** | ~1,200 lines of chart machinery in `run.rs`, next to an empty `charts.rs` | **CONFIRMED** | `run.rs` = 9,288 lines, production 1–3,984. Chart items sum to ≈1,180 production lines across 27 functions + 2 types. `worker/charts.rs` = 39 lines holding only `ChartSnapshot`. |

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

**What the divergence actually costs.** Two things, both real.

*The band stops responding to structural edits until the model comes back under the cap.* Every
surface stays bounded and safe — the band, the hit-testing, the header-menu label and the
publication all read the clamped cache, and the publish loop's bound (§F1.4) holds — but "bounded"
is not "unaffected". Walked from a legal freeze at 64:

| Gesture | Model | Band the user sees |
|---|---|---|
| freeze 64 rows | 64 | 64 |
| insert 8 rows above the band | 72 | 64 — no change |
| insert 8 more | 80 | 64 — no change |
| delete 8 rows in the band | 72 | 64 — no change |
| delete 8 more | 64 | 64 |
| delete 8 more | 56 | **56** |

So four consecutive band-affecting gestures produce no visible change at all, and the fifth
moves the band. That is a genuinely confusing few seconds for a user who freezes at exactly the
cap and then reorganises rows. It is bounded, self-healing (the band tracks normally again the
moment the model drops under the cap) and never wrong about safety — but it is not invisible,
and this spec should not have said it was.

*The saved count is unbounded.* IronCalc's boundary adjustment (`base/src/actions.rs:1051`, and
the column twin at `:725`) has no upper guard, and `insert_rows` range-checks only the
**populated** dimension — which empty inserted rows do not grow. So the model's count is not
merely "over the cap": freeze 1 row and insert 1,000,000 rows three times and it is 3,000,001 on
a 1,048,576-row sheet, and it survives save → reopen. FreeCell writes a `<pane ySplit>` that is
not a row of the sheet it describes, and another application reading that file gets a
structurally invalid freeze rather than just a larger one.

That second cost is an **engine** defect, not a FreeCell one, and it is not fixed here: per
`CLAUDE.md` a fork bug gets its own `fix/<slug>` branch and one focused upstream PR, never a
compensating workaround in FreeCell and never folded into an unrelated phase. It is captured in
`PROJECTS.md` → [`projects/frozen-pane-boundary-overflow.md`](../../../projects/frozen-pane-boundary-overflow.md).
FreeCell stays bounded regardless, because the cache clamp is what every consumer reads. Once
the fork clamps the boundary to the sheet dimension, this paragraph reduces to
"bounded-but-over-cap".

Reopening such a file in FreeCell takes path 1 and clamps again. Right-clicking the boundary row
offers "Unfreeze", which sends `SetFrozen { rows: Some(0) }` and clears both sides.

Pinned by `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache`
(`worker/run.rs`) — which walks the table above assertion by assertion — so the behaviour cannot
drift back into being an accident.

### F1.4 Bound

With the caps in place the publish loop visits at most
`(64 + 512) × (32 + 256) = 165,888` cells — 1.27× the pre-existing
`MAX_PUBLISH_ROWS × MAX_PUBLISH_COLS` budget, and a constant. This replaces the doc comment
that claimed the property without enforcing it.

### F1.5 Out of scope

Telling the user that a freeze was clamped. The clamp stays silent, but **not** on the grounds
first written here ("a case only a crafted file reaches") — §F1.3 path 2 shows an ordinary
insert reaches it too, so that justification is retired. The reason it stays silent is that
there is nothing actionable to say: the band the user is looking at is the correct, usable one,
the freeze they asked for was never possible to render, and the escape hatch (Unfreeze) is
already one right-click away in the same header menu. A banner would report a state the user
cannot act on differently.

What that *does* buy are the two surprises §F1.3 spells out: gestures that do not move the band
while the model is over the cap, and a workbook saved with a `<pane>` count FreeCell itself will
not display (today, an unbounded one). If either bites in practice — a support report of "my
freeze stopped following my inserts", or "my freeze changed when I opened it in Excel" — the
answers are respectively a **band-level** hint and a **save-time** notice, not a load-time
banner, and both belong in `PROJECTS.md` rather than here. The unbounded half of the second is
already tracked there as an engine fix.

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

1. logs at `error` (with the thread's join result: panicked vs. exited cleanly),
2. enters the existing **degraded** state — degraded bar visible, mutating controls disabled,
3. shows an OK-only dialog:
   - Title: **"The calculation engine stopped"**
   - Detail: *"This window can't be edited or saved any more. Its unsaved changes are lost.
     Open the file again to keep working."*
4. stays open, so the last rendered data is still readable.

The window is honest rather than tidy here: it does not close itself and it does not pretend a
Save might work.

Today nothing sends `Command::Shutdown`, so in practice *every* stream close is fatal. The
window still checks the flag rather than assuming, so an orderly shutdown added later doesn't
pop a false alarm.

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

### F4.2 What changes observably

Event **ordering**, and only ordering:

- `StyleCacheUpdated` and `CondFmtUpdated` for a batch now arrive **before** that batch's
  `Published`, not after. (The surfaces are all already written by then; the events are pure
  announcements.)
- A chart-only op (insert / delete / move / resize / retype / re-range a chart) now bumps the
  generation and republishes, instead of emitting `Published` with the counter standing still.
  The UI already re-reads everything on `Published`, so the visible result is unchanged — but
  the event stops lying.

No event is added or removed. No event's payload changes. Nothing the UI *reads* changes shape
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
| Worker thread dies | No, for that window | `error!` log + degraded bar + dialog (F2.3) |

---

## 6. Constraints

- **Performance.** No path may get slower. The publish bound grows from 131,072 to a
  worst-case 165,888 probe cells (F1.4) and is otherwise unchanged. The commit unification
  moves work, it does not add any: the same surfaces are written the same number of times per
  batch. Routing chart ops through the full commit adds one `build_publication` per chart
  gesture — the same cost class as one scroll republish, which is already per-frame.
- **Compatibility.** No file-format change. A workbook saved by FreeCell before and after this
  project is byte-identical for the same edits.
- **Threading.** The seam's existing invariants are preserved without exception: one thread
  owns the `UserModel`; the UI does one wait-free `ArcSwap` load per frame; no `block_on`, no
  blocking `recv`, zero engine calls on the render path (the process-global counter with its
  negative control still guards that and must stay green).
- **Behaviour freeze.** Phases 1–3 change no observable behaviour beyond F1's and F2's new
  error paths. Phase 4 changes event ordering, and only ordering.
