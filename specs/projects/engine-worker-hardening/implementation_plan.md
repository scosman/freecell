---
status: complete
---

# Implementation Plan: engine-worker-hardening

Phase order is fixed by the overview: the two small safety fixes land before the structural
change, so the unification is built on stable ground.

## Phases

- [x] **Phase 1 — B2: bound the frozen-pane band.** `MAX_FROZEN_ROWS` / `MAX_FROZEN_COLS`;
      reject an over-cap `SetFrozen` in `pre_validate`; clamp at cache build (closes the
      crafted-file path for the render thread too) and again at the publish loop. Rewrite the
      doc comment that asserted the bound it did not enforce.
      → `architecture.md §A2`

- [x] **Phase 2 — B1: guard load/save; surface worker death.** `catch_unwind` around
      `from_source` and `save_workbook` with new `LoadError::EnginePanic` /
      `SaveError::EnginePanic`; split `handle_caught_panic` so the save path reuses the
      poisoning policy without the edit-path event; retain the `JoinHandle` and track a
      requested shutdown; make the window treat a stream close it did not ask for as fatal.
      → `architecture.md §A3`
      - **Deviation from §A3.6, recorded per its own instruction:** the load/save panic hooks are
        `#[cfg(test)]` sentinel **file names** (`document::PANIC_SENTINEL`), not the planned
        `test-support` `DocumentSource::TestPanic` + `Command::TestPanicOnSave`. Better trade —
        no test-only shape reaches the public `DocumentSource`/`Command` — but `#[cfg(test)]`
        items are invisible to an integration-test build, so the guard tests live in the `run.rs`
        / `charts.rs` / `client.rs` unit modules instead of `worker_seam.rs`. They still exercise
        the guards at their real call sites.
      - **Code-review follow-ups (B1 round 2):** a distinct `worker_lost` window state (no Save As
        in the bar, every save/export *entry* refuses, the unsaved-changes prompt drops its Save —
        round 3 below is what made that actually cover the in-flight cases); the save-time chart
        sweep made re-entrant across a caught panic (and the `AssertUnwindSafe` justification
        corrected in `architecture.md §A3.3`); `WorkerExit::Running` joined off the UI thread
        instead of being logged as a non-answer; the loading overlay cleared on a death before
        `Loaded`; the fatal report no longer swallowed by a non-terminal modal; the CSV export
        guarded like the save; `has_worker` split out of `shutdown_requested`.
      - **Code-review follow-ups (B1 round 3):** guarding save *entry* left both in-flight
        orderings hanging — a worker dying under a quit prompt (the replaced modal skips
        `dismiss_modal`'s quit-abort branch) and a save armed before the death (only
        `Saved`/`SaveFailed` clear it). `on_worker_lost` now tears down the pending save/export and
        stands the quit down unconditionally; both orderings are pinned by tests that were
        confirmed failing first. Also: the refusal notice no longer stomps a terminal load dialog;
        the sweep marks a sheet after parse **and** bind, in `SheetId` order, so the surviving
        prefix is deterministic (test now panics on the second of two sheets); the lazy path's
        opposite ordering is documented where a reader meets it; the `WorkerExit::Panicked` test
        uses the shared `quiet_panics` from the parent thread; the loading-overlay tests assert the
        grid half too.
      - **Code-review follow-ups (B1 round 4):** round 3's teardown stood down *any* quit, including
        one the dying window was never part of (probed: a clean window's death switched off the
        quit prompting a different window). Scoped it via `QuitPlan::is_pending` —
        `FreeCellApp::note_quit_prompt_unanswerable(key, cx)` — matching the check
        `on_window_closed` already makes, with §F2.3 / §A3.5 / the doc comments reconciled to it.
        Also: a bind-side `#[cfg(test)]` injection now gates "marked after parse **and** bind" (the
        parse-side test alone passed under an ordering that marks between them); §F2.3 quotes the
        export refusal's real wording.
      - **Code-review follow-ups (B1 round 5):** corrected a false parenthetical claiming the
        `SaveFailed` / backup-failure sites are "always in-plan" windows — `save` has no dirty
        guard, so a ⌘S arms a save on a clean window, which is never in the plan. The underlying
        defect (three save-failure sites aborting a quit that prompts a *different* window) is
        pre-existing §5.2/§7.3 behaviour, so per `CLAUDE.md` it is **captured, not fixed here**:
        `PROJECTS.md` → `projects/quit-stand-down-scope.md`. Also scoped §F2.3/§A3.5's motivation
        for the `is_pending` gate to what it actually removes (a queued-but-unprompted window's
        death still stands the quit down — deliberately, since the narrower rule is worse).

- [x] **Phase 3 — extract chart handling out of `run.rs`.** Planned as ≈1,180 production lines
      plus the chart tests into `worker/charts.rs`; **actually moved 1,048** (see the closing
      note for the prediction-vs-outcome reconciliation). Behaviour-preserving; every existing
      test passes with its assertions unchanged.
      → `architecture.md §A4`

- [x] **Phase 4 — E1: one commit point.** `StagedCommit` + `Worker::commit`; every shared-surface
      write before one `Release` store of the generation, every announcing event after it;
      `ChartSnapshot` gains a `generation` stamp; chart ops stop emitting `Published` without
      publishing. Ordering tests with non-zero-sample assertions.
      → `architecture.md §A5`

- [x] **Phase 5 — final validation + remediation-doc correction.** Whole-workspace build,
      test and fmt; correct `projects/architecture-review-remediation.md` (B2's blast radius
      includes the render thread; B1/E1/F1 confirmed as written); record the post-extraction
      `run.rs` line count against the 2,000-line ceiling and name what should move next.

## Outcome

All four findings were re-derived at HEAD before being fixed. **None was disproved.** One
needed correcting: B2's blast radius is larger than the review described — the unclamped
frozen count also drives a render-thread loop (`grid/view.rs:4529`), so clamping only at the
publish site (what the review proposed) would have left half the hang live. The clamp lands at
the sheet-cache build instead, where both consumers read.
`projects/architecture-review-remediation.md` records that correction, plus DONE notes on
B1/E1/F1.

Two things checked and deliberately *not* expanded into:

- **B3 is not made free by Phase 4.** The long cache write-lock hold is a property of
  `refresh_cache_cells`' loop, not of where that loop sits in the commit. Unchanged, still v3+.
- **Phase 4 did not need to change what the UI reads** — the overview's named escalation risk.
  Every change is to *when* a write lands and *when* an event fires; the one shape change is an
  additive `ChartSnapshot::generation` the UI never reads.

Render tests were correctly out of scope: no grid-render code was touched, and the pixel
suite's frozen-band fixtures use counts of 1–3, far under the new 64/32 caps, so no baseline
can move.

## Closing note — `run.rs` against the F2 ceiling

All counts here are **production lines** = lines before the file's first top-level `#[cfg(test)]`.
That is the method that reproduces the specs' own 3,984 baseline, and every figure below was
re-measured with it rather than carried forward from an earlier draft.

`worker/run.rs` production, phase by phase:

| Point | Commit | Production | Δ |
|---|---|---|---|
| Spec baseline | `b378edd` | 3,984 | — |
| After Phase 1 (B2) | `b61f052` | 4,036 | +52 |
| After Phase 2 (B1) | `0f4ddd1` | 4,096 | +60 |
| After Phase 3 — chart extraction | `bccd033` | **3,048** | **−1,048** |
| After Phase 4 (E1) | `167d144` | 3,148 | +100 |
| **As of this commit** (Phase 1/2 CR rounds) | — | **3,192** | +44 |

So `3,984 + 52 + 60 − 1,048 + 100 + 44 = 3,192`. The chart extraction removed **1,048** lines
(not the ~1,000/1,010 earlier drafts of this note claimed), and `worker/charts.rs` production
landed at **1,113** at `bccd033`.

**The reported figure is 3,192, measured as of this commit.** It will move again as later work
lands — Phase 4's CR round in particular — so a future re-measure that differs is an update, not
a contradiction of this note.

**Prediction vs outcome.** `architecture.md` §A4.4 and `functional_spec.md` §F3 predicted the
extraction would remove ≈1,180 and land `run.rs` at ≈2,800. It removed **1,048** and landed at
**3,048** — ~130 fewer lines moved than predicted, and ~250 above the predicted landing point.

The shortfall is **estimate error, not an incomplete extraction**: all 28 items §A4.1 listed (24
`impl Worker` methods, 4 free functions, `AuthoredEntry`, `ChartUndo`) are gone from `run.rs` and
present in `charts.rs`, and `charts.rs` production went 39 → 1,113, i.e. **+1,074** against
`run.rs`'s −1,048 — the moved code did not shrink, the destination is slightly larger for its own
module doc, imports and `impl` header. F1's ≈1,180 was an explicitly approximate sum of the chart
items' spans; the real total was ~130 lines smaller. The ≈2,800 landing point additionally
assumed the 3,984 baseline, but Phases 1–2 had already added 112 lines before the cut, which
accounts for the rest of the ~250 gap (4,096 − 1,048 = 3,048, vs 3,984 − 1,180 = 2,804).

**F2 next round should size off 3,192 and −1,048, not off 2,800 and −1,180.**

That is **still over the 2,000-line ceiling F2 will enforce**, and the chart extraction was the
only move F1 scoped for this file.

Three further mechanical extractions get it under, and are recorded in the remediation plan's
F2 entry rather than attempted here: `worker/clipboard.rs` (~420 lines — `components/clipboard.md`
is already its design doc), `worker/cache_mirror.rs` (~450), and `worker/apply.rs` (~400 — the
`Command` → engine mapping, whose `apply_one` is 255 lines on its own). That leaves ~1,900: the
loop, coalescing, the commit point, the publication, the save, and the panic policy. The
`pub(super)` widening Phase 3 established makes each move mechanical.
