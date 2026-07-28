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

## Closing note — `run.rs` against the F2 ceiling

Recorded at Phase 5, not attempted here: what the chart extraction leaves behind, and what a
future phase should move to reach 2,000 production lines.
