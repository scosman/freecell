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
        sweep made
        re-entrant across a caught panic (and the `AssertUnwindSafe` justification corrected in
        `architecture.md §A3.3`); `WorkerExit::Running` joined off the UI thread instead of being
        logged as a non-answer; the loading overlay cleared on a death before `Loaded`; the fatal
        report no longer swallowed by a non-terminal modal; the CSV export guarded like the save;
        `has_worker` split out of `shutdown_requested`.
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

- [x] **Phase 3 — extract chart handling out of `run.rs`.** Move ≈1,180 production lines and
      the chart tests into `worker/charts.rs`. Behaviour-preserving; every existing test passes
      with its assertions unchanged.
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

`worker/run.rs` production went 3,984 → **3,148** (the chart extraction removed 1,010; Phase 4's
commit point added ~100 back plus the new error paths). That is **still over the 2,000-line
ceiling F2 will enforce**, and the chart extraction was the only move F1 scoped for this file.

Three further mechanical extractions get it under, and are recorded in the remediation plan's
F2 entry rather than attempted here: `worker/clipboard.rs` (~420 lines — `components/clipboard.md`
is already its design doc), `worker/cache_mirror.rs` (~450), and `worker/apply.rs` (~400 — the
`Command` → engine mapping, whose `apply_one` is 255 lines on its own). That leaves ~1,900: the
loop, coalescing, the commit point, the publication, the save, and the panic policy. The
`pub(super)` widening Phase 3 established makes each move mechanical.
