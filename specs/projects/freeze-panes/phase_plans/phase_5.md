---
status: complete
---

# Phase 5: Structural-edit boundary tracking (Q4 checkpoint)

## Overview

The one CONTINGENT phase (`architecture.md §5`, `implementation_plan.md` Phase 5, Q4). It
resolves whether inserting/deleting rows or columns adjusts the frozen boundary the Excel way
(insert above/within the band grows it; delete within shrinks it; edits entirely past it leave
it unchanged), and it must stay **one undo step**.

**Checkpoint (the crux):** probe whether IronCalc's structural ops already adjust
`frozen_rows`/`frozen_columns`. Result determines the branch:

- **Native →** no fork code, just a FreeCell regression test.
- **Not native →** ONE focused fork branch `fix/structural-edits-adjust-frozen-pane` (upstream
  fix, no FreeCell-side compensating call), re-pin `freecell-fixes`, bump FreeCell's pin.

## Probe result

**NOT native.** Confirmed by (a) reading the fork's `base/src/actions.rs`
`insert_rows`/`delete_rows`/`insert_columns`/`delete_columns` — zero references to the frozen
fields; and (b) an empirical worker test: with `frozen_rows = 3`, inserting a row above the band
left `M = 3` (Excel expects 4) and deleting a row within left `M = 3` (Excel expects 2). So the
NEGATIVE branch: a fork fix is required. Per CLAUDE.md fix-upstream policy, no FreeCell-side
compensating code.

## Steps

1. **Fork fix (`fix/structural-edits-adjust-frozen-pane`, off `main`).**
   - `base/src/actions.rs`: forward Excel adjustment at the tail of all four ops — insert with
     position `p <= frozen_count` grows the band by the inserted count; delete shrinks it by the
     number of frozen tracks removed (`min(last_deleted, frozen) - p + 1`), clamped to the band.
   - Undo correctness: insert-undo is the symmetric `delete_rows`/`delete_columns` (exact in all
     cases, since inserted tracks are always inside the grown band). Delete-undo re-inserts via
     `insert_*`, which can only regrow the band partially — so snapshot the pre-delete count in the
     `DeleteRows`/`DeleteColumns` diffs (`old_frozen_rows`/`old_frozen_columns`, mirroring
     `DeleteSheet`) and restore it directly in the undo arm (`history.rs`, `common.rs`,
     `undo_redo.rs`). Redo replays the forward op.
   - Tests: `base/src/test/test_frozen_structural_edits.rs` — grow/shrink/unchanged both axes +
     undo/redo round-trips incl. full-band collapse. Update the one existing test whose expectation
     encoded the old no-tracking behavior (`test_diff_queue::queue_undo_redo_multiple`: 6 → 8).
2. **Integrate:** merge the fix branch into `freecell-fixes` (resolves cleanly against the
   already-present merged-cells fix — both add fields to the same two delete diffs; keep both).
   Push both branches.
3. **Bump FreeCell's pin:** `cargo update -p ironcalc -p ironcalc_base` → `freecell-fixes`
   `cee2859d`.
4. **FreeCell regression test** (`worker/run.rs`): assert the boundary tracks an insert (grows)
   AND a delete (shrinks), each reverted by a SINGLE undo — proving it is one undo step (no
   FreeCell-side compensating call). No other FreeCell code changes.

## Tests

- Fork `test_frozen_structural_edits` (13 cases): insert above/within grows; insert below leaves;
  delete within shrinks; delete entire band → 0 and undo restores; delete spanning the boundary
  removes only the frozen part; column analogs; undo/redo round-trips.
- Fork `test_diff_queue::queue_undo_redo_multiple`: updated expectation (freeze 6 + two in-band
  inserts → 8) with an explanatory comment.
- FreeCell `structural_edits_track_frozen_boundary_in_one_undo_step`: insert-above grows M 3→4,
  one undo → 3, redo → 4; delete-within shrinks 3→2, one undo → 3.

## Handoff (owner action)

The agent cannot open the upstream PR. Prepare compare link + title + description for the owner to
open `scosman:fix/structural-edits-adjust-frozen-pane` → `ironcalc/IronCalc:main`.
