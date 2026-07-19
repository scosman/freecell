---
status: complete
---

# Phase 5: Render validation + in-app-merge xlsx round-trip (dedicated late phase)

## Overview

The closing phase of the merged-cell UI. Phases 1–4 built the engine plumbing, resident
`MergeMap`, region rendering (with 6 committed `merge_*` pixel baselines), merge-aware
selection/editing, and the merge/unmerge chrome. This phase does the deferred **render
validation** and adds the one **persistence** test the earlier phases left for the end:

1. A worker-seam integration test proving an **in-app-created** merge survives an xlsx
   save → reopen (arch §10 — the last row of the worker test matrix; earlier merge worker
   tests cover apply / confirm / undo-redo / file-loaded merges, but not create→save→reopen).
2. Confirm the 6 `merge_*` pixel baselines have **no drift** (the region rendering landed in
   P2 with committed baselines; this phase re-verifies them scoped, not the full suite).
3. The CI `render` gate on the branch (dispatched by the manager after push) is the
   authoritative pixel truth — the full local suite is known to exceed the Bash 10-min cap
   under lavapipe, so it is intentionally **not** run locally here.

No production code changes — this is a test + validation phase.

## Steps

1. **In-app-merge round-trip test** (`freecell-engine/tests/worker_seam.rs`). Add
   `in_app_created_merge_survives_xlsx_roundtrip`, modeled on the adjacent
   `save_through_worker_roundtrips` (same `spawn_new` / `full_viewport` / `Command::Save` /
   reopen-through-`spawn(OpenFile)` helpers). It drives the **real UI command seam**:
   - `spawn_new`, seed A1 = `"anchor"` and B2 = `"covered"` (`set_input`), poll until both
     publish.
   - Merge A1:B2 via the real `Command::MergeCells { sheet, area, confirmed: true }` — B2 has
     content, so it is a data-losing merge; `confirmed: true` is the UI's post-dialog re-send
     (unconfirmed would reply `MergeNeedsConfirm` and apply nothing).
   - `area = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1))` — A1:B2 in **0-based
     FreeCell** coords (the base the resident cache's `merges()` returns, matching the merge
     worker unit tests in `run.rs`).
   - Poll the **first** session's public cache (`client.caches().read().get(sheet).merges()`)
     until it equals `[area]` and B2 has cleared — confirms the merge took in-session before
     saving.
   - `Command::Save` to a `tempdir()` path; await `WorkerEvent::Saved`.
   - Reopen through a **genuinely separate** worker/model: `spawn(DocumentSource::OpenFile(path))`
     (not the live model); send a viewport; await `Published`.
   - Assertions (with teeth — each fails if merge xlsx persistence regresses):
     - reopened `cache.merges() == &[area]` (region survives with **exact** A1:B2 geometry),
     - `published_text(A1) == "anchor"` (anchor value retained),
     - `published_text(B2) == ""` (covered cell cleared).
   - The `tempdir` `TempDir` is held to end-of-test, cleaned up on drop (mirrors the existing
     round-trip test's tempfile handling).

2. **Verify the test + fmt.** `cargo test -p freecell-engine --test worker_seam` (the new test
   passes; the whole seam suite stays green); `cargo fmt --all --check`.

3. **Scoped `merge_` baseline drift check.** Install the capture stack once if absent
   (`render-tests/scripts/setup_render_env.sh`), then run
   `render_tests.sh generate --only merge_` (from `app/`, so the pinned 1.95 toolchain
   activates). Confirm it reports the 6 cases **unchanged** (0 changed), `git status` on
   `render-tests/baselines/` is clean, and eyeball 2–3 `merge_*` PNGs. Do **not** run the full
   pixel suite locally (10-min cap).

4. **CI `render` gate.** The manager dispatches the `render` workflow on the branch after
   commit + push and confirms it is green (the authoritative pixel validation — the local
   scoped check above only proves no *drift*, not the full gate).

## Tests

- `in_app_created_merge_survives_xlsx_roundtrip` (`worker_seam.rs`) — an in-app merge created
  via `Command::MergeCells { confirmed: true }`, saved and reopened through a separate worker,
  restores the region with exact A1:B2 geometry in the resident merge cache, retains the anchor
  value, and leaves the covered cell cleared. Fails if merge xlsx write or re-read regresses.
- Baseline drift: `render_tests.sh generate --only merge_` reports **0 changed** across the 6
  `merge_*` cases (byte-identical to committed baselines); baselines dir git-clean.
- CI `render` gate green on the branch (manager-dispatched).
