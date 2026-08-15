---
status: complete
---

# Phase 2: Fork fix — batch input emits link diffs (and rolls them back)

## Overview

`UserModel::set_user_inputs` (our fork's batched, single-undo write API) calls
`Model::set_user_input` directly and records only `Diff::SetCellValue`. Upstream's own
single-cell `set_user_input` and CSV paste both route through
`set_user_input_with_link_diffs`, which additionally records the `SetCellStyle` +
`SetCellLink` diffs that make auto-linking reversible. Our batch path missed that treatment
because it predates the upstream capability.

Two consequences, both in scope (arch §2):

1. **Undo.** Paste/fill a column of URLs through the batch API, hit undo: the values revert
   but the auto-created links and their blue-underline styling stay (breaks F7.5).
2. **Mid-batch rollback.** `set_user_inputs`' own atomicity guarantee replays its
   `SetCellValue` snapshots, so a rejected batch restores values while leaving behind links
   and link styling the successful writes auto-created.

Routing the batch's writes through `set_user_input_with_link_diffs` fixes both at once,
because the rollback replays the very same diff list the history entry would have carried.

Work happens in `/home/user/IronCalc` (the fork checkout). Nothing in `/home/user/freecell`
changes except this phase plan.

**Branch base — `fix/batch-set-inputs`, not `main`** (Phase 1 correction, arch §2): the batch
API exists only on that branch, so `main` has nothing to fix. The two go upstream as stacked
PRs (batch API first, link diffs on top). The compare link must be
`fix/batch-set-inputs...scosman:ironcalc:fix/batch-set-inputs-link-diffs` — a `main...` compare
would drag the batch API itself into the diff, which is exactly the bundling the
one-fix-one-branch-one-PR rule forbids.

## Steps

1. **Branch.** `git switch -c fix/batch-set-inputs-link-diffs fix/batch-set-inputs`.
   Confirm the base tip is `180192ff` and that `set_user_input_with_link_diffs` is present on
   it (it arrived with the Phase-1 rebase onto the new `main`).

2. **Restructure `set_user_inputs`** (`base/src/user_model/common.rs`). The up-front
   validate-and-snapshot pass is the source of the batch's atomicity and must not move; only
   the write loop changes.

   - Rename the pre-pass accumulator to `value_diffs` — it is now only the *seed* of the final
     diff list, not the whole of it.
   - In the write loop, build `diff_list` incrementally: push cell *i*'s pre-snapshotted
     `SetCellValue`, then call `set_user_input_with_link_diffs`, which performs the write and
     appends that cell's `SetCellStyle` (only for a newly created link) + `SetCellLink` diffs:

     ```rust
     let mut diff_list = Vec::with_capacity(inputs.len());
     for (value_diff, (sheet, row, column, value)) in value_diffs.into_iter().zip(inputs) {
         diff_list.push(value_diff);
         if let Err(error) = self.set_user_input_with_link_diffs(
             *sheet, *row, *column, value.to_string(), &mut diff_list,
         ) {
             let _ = self.apply_undo_diff_list(&diff_list);
             return Err(error);
         }
     }
     ```

     The failing cell's own `SetCellValue` is pushed *before* the call, so the rollback covers
     any partial mutation the failing write made. `apply_undo_diff_list` replays in reverse, so
     per cell it deletes the link, then restores the style, then restores the value — the
     ordering `set_user_input_with_link_diffs` already relies on in `paste_csv_string`.
   - The `to_vec()` clone the old rollback needed disappears: the whole of `diff_list` is
     exactly "everything this batch has done so far".

3. **Widen the doc comment.** Phase 1 narrowed `set_user_inputs`' contract to "the rollback
   covers values only" and added a "a future revision will record the link changes too" note.
   Delete that caveat and state the restored guarantee — but **enumerate it, don't generalize**:
   the rollback restores *the values, links and link styling already written*. It is not total,
   and must not claim to be: a number format or quote prefix `Model::set_user_input` applies is
   recorded by no diff and survives the rollback (see "Found while building" below), exactly as
   it does for single-cell `set_user_input`.

4. **Update the write-loop comments** so they describe the new interleaved diff list rather
   than the value-only one — including *why* replaying it undone lands on the pre-batch state,
   since the two kinds of diff get there differently. Only the `SetCellValue` old values are
   pre-batch snapshots; the link/style old values are captured per write, inside the helper,
   against what earlier writes in the same batch left. The list is still exact because a cell's
   diffs are chained in write order and the replay is in reverse, so repeats telescope back.
   The comment has to say that, or a later reader will "notice" a cell's intermediate link diffs
   look redundant and dedupe them. Warn only about what a test backs (mutation 4) — hoisting the
   snapshots into the pre-pass leaves undo's end state identical, so the comment does not claim
   it breaks; what it *does* break is the recorded diff list, and that is pinned by the
   diff-shape test rather than by the comment (mutation 8).

5. **Add the tests** below to `base/src/test/user_model/test_set_user_inputs.rs` (upstream's
   own layout for this API), then **mutation-check every one of them** — and not only against
   "revert the whole change": for each property a test's *name or comment* claims, run the
   mutation that breaks **that** property specifically and confirm that test fails. A test that
   fails only under the blanket revert is not evidence for the property it is named for; two
   rounds of review were spent on exactly that (a style assertion masked by the value snapshot,
   then a link-chain assertion masked by the style undo).

6. **Verify.** `cargo build`, `cargo test`, `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets` on the branch.

7. **Prepare the upstream PR** — compare link, title, description — for the owner to open,
   noting it is stacked on the `fix/batch-set-inputs` PR.

8. *(commit invocation)* Commit on `fix/batch-set-inputs-link-diffs`, **merge it into
   `freecell-fixes`** and re-run the full verification there. `freecell-fixes` already contains
   `fix/batch-set-inputs`, and the **`set_user_inputs` function body is unchanged between the
   two branches**, so the merge should be clean — but the *file* does diverge elsewhere and the
   conflict check is real, not a formality: `git diff fix/batch-set-inputs freecell-fixes --
   base/src/user_model/common.rs` is +71/−2 (`freecell-fixes` carries the merged-cells guard in
   `set_user_input`, `area_contains_region`, the clear-all unmerge, and the frozen-pane
   snapshots in `delete_rows`/`delete_columns`), and `undo_redo.rs` diverges by a further
   +103/−1. None of those hunks fall inside `set_user_inputs`. Push both refs.

## Tests

All in `base/src/test/user_model/test_set_user_inputs.rs`, alongside the existing batch cases.

**Undo (the F7.5 half):**

- `batch_auto_link_is_undone_with_the_batch` — a batch writing a URL auto-links and styles the
  cell; one undo removes value, link, *and* the underline + theme-hyperlink color; one redo puts
  all three back.
- `batch_auto_link_undo_restores_the_previous_link` — a cell that already carried a link is
  overwritten by a batch with a different URL; one undo restores the *old* link target, not
  merely "no link".
- `batch_emptying_a_linked_cell_undoes_to_the_link` — an empty value in a batch removes the
  cell's link (`Model::set_user_input`'s clear path); one undo restores content and link.
- `a_peer_replaying_the_batch_and_its_undo_ends_up_link_free` — a peer fed the batch's diffs and
  then the batch's *undo* diffs via `apply_external_diffs` has no link left. The peer sees only
  the recorded diffs, and clearing a cell's contents does not by itself drop its link, so this
  pins that the link change is genuinely in the diff list.

  (A redo-only and a replay-the-batch-only test were written first and **dropped**: both pass
  without the fix, because `apply_diff_list`'s `SetCellValue` arm calls `Model::set_user_input`,
  which re-auto-links on the redo/peer side independently. Replaying the *undo* is the direction
  that discriminates.)

**Mid-batch rollback (the second half):**

- `mid_batch_failure_rolls_back_an_auto_created_link` — batch = [A1 = a URL (succeeds,
  auto-links + styles), B1 = a cell inside a CSE array formula (fails)]. After the error A1 is
  back to its pre-batch value with **no** link and its exact pre-batch style. No history entry
  was recorded either: the next undo walks past the rejected batch to the array formula and
  leaves A1 where the rollback put it.
- `mid_batch_failure_restores_a_replaced_link` — A1 already carries link X; the batch overwrites
  it with URL Y and then fails on the array-formula cell; A1's link is X again and the cell looks
  exactly as it did. Both style assertions in these two cases are **plain end-state guards, not
  checks on the style diff** — in both, A1 pre-exists, so its `SetCellValue` snapshot carries the
  whole `Cell` (style index included) and undo's `update_cell` rebuilds the style from the value
  diff no matter what the `SetCellStyle` diffs say. Cell state can never see that difference; the
  diff-list test below is what pins it.

**Diff-list shape — the only place the style-diff guard is observable:**

- `the_link_style_diff_is_recorded_only_when_the_link_is_new` — decodes `flush_send_queue()` and
  asserts the recorded diff kinds. Four cases: creating a link on an empty cell records
  `[SetCellValue, SetCellStyle, SetCellLink]`; retargeting an existing link records
  `[SetCellValue, SetCellLink]` (no style diff — `old_link.is_none()` is false); a plain value
  records `[SetCellValue]` alone; and **one batch writing the same empty cell twice** records
  `[SetCellValue, SetCellStyle, SetCellLink, SetCellValue, SetCellLink]` — new on the first
  write, already linked on the second, so exactly one style diff. This is what pins
  `set_user_input_with_link_diffs`' suppression in *both* directions; a spurious style diff is
  invisible in cell state but still travels to every peer and sits in the history. The fourth
  case is also what makes the per-write `old_link` snapshot a backed invariant rather than an
  incidental detail (mutation 8).

**Repeated-cell chaining:**

- `a_cell_listed_twice_undoes_through_its_whole_link_chain` — A1 starts out **linked**, and the
  batch retargets it twice (`URL → OTHER_URL → THIRD_URL`). The recorded diffs are asserted to be
  `[SetCellValue, SetCellLink, SetCellValue, SetCellLink]` — one link diff per write, no style
  diff — and one undo walks the chain back to the *original* target.

  The pre-existing link is what makes it discriminating, and the from-empty version of this test
  is **not**: from an empty cell the style diff's undo takes `range_clear_all`'s branch for a
  now-empty cell, which drops the link whatever the link diffs said, so no arrangement of
  `SetCellLink` diffs is observable. A link-target assertion on a *pre-existing* link is the one
  shape that sees the chain. (This is not the round-2 vacuity problem repeated: that was about
  *style* assertions on a pre-existing cell.)

**Mutation check.** Every test's named property has a mutation that breaks specifically it. All
runs leave the test file untouched.

| # | Mutation | Fails |
|---|---|---|
| 1 | whole `set_user_inputs` change reverted | all 8 new tests (the 10 pre-existing pass) |
| 2 | `old_link.is_none()` → `true` (spurious style diff on retarget) | `the_link_style_diff_is_recorded_only_when_the_link_is_new`, `a_cell_listed_twice_undoes_through_its_whole_link_chain` |
| 3 | `old_link.is_none()` → `false` (never record a style diff) | 6, incl. 3 of upstream's own `test_auto_link` cases |
| 4 | post-loop dedupe keeping only the **last** `SetCellLink` per cell | `a_cell_listed_twice_undoes_through_its_whole_link_chain` (alone) |
| 5 | link diff records `old_value: None` (forgets the previous target) | `batch_auto_link_undo_restores_the_previous_link`, `batch_emptying_a_linked_cell_undoes_to_the_link`, `mid_batch_failure_restores_a_replaced_link`, the chain test, + 2 upstream |
| 6 | never record the `SetCellLink` diff at all | 14, incl. 6 upstream |
| 7 | a rejected batch still calls `push_diff_list` | `mid_batch_failure_rolls_back_an_auto_created_link`, `mid_batch_write_failure_rolls_back_the_writes_already_made` |
| 8 | link/style snapshots hoisted into the up-front pass | `the_link_style_diff_is_recorded_only_when_the_link_is_new` (alone) |

Mutations 2, 4 and 8 are the ones worth noting: before this phase's tests, **all three** left the
entire base suite green (2284 passing / 8 ignored pre-change; 2292 / 8 after). Mutation 4 is
exactly the "dedupe the redundant diffs" refactor the write-loop comment warns against, so the
comment is backed by a test rather than by assertion.

**On hoisting** (mutation 8): it is equivalent *for undo* — end state is identical, including for
a cell with an inherited column style — which is why the write-loop comment no longer warns about
it. It is **not** equivalent for the recorded diff list: a hoisted `old_link` stays `None` across
both writes of a repeated previously-unlinked cell, so the second write records a second, spurious
`SetCellStyle` — the same defect class mutation 2 models. The fourth case of the diff-shape test
catches it, so the per-write snapshot is now an invariant the suite enforces.

**Regression coverage relied on (not new):** the existing `test_set_user_inputs.rs` cases —
single-undo, all-or-nothing coordinate validation, spill-anchor snapshots, repeated cells,
cross-sheet, send-queue — plus upstream's `test_links.rs` and the full `base` + `xlsx` suites.

## Upstream PR (for the owner to open — the agent cannot)

**Stacked on the `fix/batch-set-inputs` PR.** `set_user_inputs` exists only on that branch, so
this one must land after it. Two ways to surface it, in preference order:

1. **Now, as a fork-internal review of just this fix** — base `fix/batch-set-inputs`, head
   `fix/batch-set-inputs-link-diffs`, both on `scosman/IronCalc`:
   <https://github.com/scosman/IronCalc/compare/fix/batch-set-inputs...fix/batch-set-inputs-link-diffs>
2. **After the batch-API PR merges upstream**, re-target it at `ironcalc/IronCalc` `main`:
   <https://github.com/ironcalc/IronCalc/compare/main...scosman:IronCalc:fix/batch-set-inputs-link-diffs>
   (valid only once `main` carries `set_user_inputs`; before then it drags the batch API into
   the diff, which is exactly the bundling the one-fix-one-PR rule forbids).

**Title:** `fix(base): set_user_inputs must record the link diffs of the writes it makes`

**Description:**

> `UserModel::set_user_input` and `paste_csv_string` both route their writes through
> `set_user_input_with_link_diffs`, so the link `Model::set_user_input` auto-creates from
> URL-shaped input — and the style it applies with it — is recorded in the diff list and is
> therefore undoable. `set_user_inputs` (the batched, single-undo write API added in the
> preceding PR) called `Model::set_user_input` directly and pushed only `Diff::SetCellValue`,
> so it missed that treatment.
>
> Two symptoms:
>
> - Batch-write a column of URLs and undo: the values revert, but the links and their
>   underline + theme-hyperlink styling stay behind. Same for a peer replaying the undo over
>   the send queue — clearing a cell's contents does not by itself drop its link.
> - `set_user_inputs`' mid-batch rollback replays the same snapshots, so a rejected batch
>   restored values while leaving behind links the successful writes had created.
>
> The fix routes the batch's writes through `set_user_input_with_link_diffs`. The up-front
> validate-and-snapshot pass that gives the batch its all-or-nothing guarantee is unchanged —
> every *value* `old_value` is still captured before the first write. What changes is that the
> diff list is now grown during the write loop (value diff, then that cell's link/style diffs)
> instead of being complete before it, which makes it simultaneously the history entry and the
> rollback. Both symptoms are fixed by the one change.
>
> The link/style old values are captured per write, inside the shared helper, so a cell listed
> twice in one batch contributes a chain of diffs; `apply_undo_diff_list` replays the list in
> reverse, so the chain telescopes back to the pre-batch state
> (`a_cell_listed_twice_undoes_through_its_whole_link_chain`). Capturing them per write also
> keeps the style diff to the one write that actually creates the link — asserted on the
> recorded diff list, since a redundant style diff is a no-op in cell state.
>
> Eight tests in `base/src/test/user_model/test_set_user_inputs.rs`: undo of an auto-link
> (value + link + style, and redo back), undo restoring a *previous* link target, undo of an
> empty input that dropped a link, a peer replaying batch-then-undo, the repeated-cell chain,
> the two mid-batch rollback cases (link created, link replaced), and one asserting the shape
> of the recorded diff list — which also closes a gap that predates this PR: nothing in the
> suite pinned `set_user_input_with_link_diffs`' `old_link.is_none()` suppression, so emitting
> a style diff on the retarget path passed the whole base suite. Each test was
> mutation-checked.

## Found while building — recorded, deliberately not fixed here

All of these are pre-existing upstream behaviour that this diff neither introduces nor changes.
Under one-fix-one-branch-one-PR they stay out of `fix/batch-set-inputs-link-diffs`.

- **Undo of an auto-link destroys an inherited column/row style.** Set a column-1 style with
  `font.b`, batch-write (or single-write) a URL into A1, undo: A1 loses the inherited bold.
  `apply_undo_diff_list`'s `SetCellStyle` arm, given `old_value: None` on a cell that still
  holds content, calls `set_cell_style(&Style::default())` — materializing an explicit default
  over the inherited style instead of clearing the cell's own style. The batch path is
  byte-for-byte identical to single-cell `set_user_input` here. **FreeCell will hit this from
  Phase 4 on**, so it wants its own `fix/undo-set-cell-style-clears-inherited` branch + upstream
  PR and a `GAPS.md` row — neither is in this project's scope, so capture it in `PROJECTS.md` /
  `projects/` when Phase 6 touches the docs.

- **A rejected batch can still leave a number format or quote prefix behind.**
  `[(A5,"100$"), (a CSE array cell → fails)]` on an empty sheet rolls A5's *value* back but
  leaves `num_fmt` as `"#,##0$"`; `'`-prefixed input leaves `quote_prefix` set. Nothing records
  those style changes: the `SetCellValue` snapshot restores the style index only when the cell
  pre-existed (the whole `Cell` is snapshotted); for a previously-nonexistent cell the undo path
  calls `cell_clear_contents`, which keeps the style. Single-cell `set_user_input`'s own undo
  leaks it identically. This is why `set_user_inputs`' doc comment enumerates what the rollback
  restores (values, links, link styling) rather than claiming totality.

  Even that enumeration has one hole, from the first bullet rather than this one: for a cell
  carrying **no explicit style but an inherited row/column style**, "link styling is restored"
  means `Style::default()` is materialized over it, not that the inherited look comes back.
  Fixing that is `fix/undo-set-cell-style-clears-inherited`'s job, not this PR's; the wording is
  left as-is rather than hedged into unreadability.

- **Per-cell `Style` clone on the bulk path.** `set_user_input_with_link_diffs` calls
  `get_cell_style_or_none` unconditionally, so every already-existing cell in a batch now pays a
  `Style` clone plus two link-map lookups. Upstream-consistent (`paste_csv_string` already pays
  it) and not a blocker, but worth measuring before FreeCell drives `set_user_inputs` at
  100k-cell scale.

- **`freecell-fixes`' merged-region guard is bypassed by the batch.** On that branch
  `UserModel::set_user_input` rejects a write into a covered (non-anchor) merged cell; the batch
  reaches `Model::set_user_input` through the helper and so does not. Unchanged by this diff —
  a note for the step-8 merge and for FreeCell's own command layer.

## Outcome

The fix landed as planned: `set_user_inputs`' write loop routes through
`set_user_input_with_link_diffs`, which fixes undo (F7.5) and the mid-batch rollback with one
change, because the diff list it grows *is* the rollback. No roadblocks, no design deviations,
no changes to the up-front validate-and-snapshot pass.

### What review changed (three rounds, all about evidence rather than behaviour)

The behaviour of the shipped diff was correct from round 1 and never changed. Every finding was
a claim outrunning its test — worth recording because the same trap is waiting in Phases 3–5,
where FreeCell's own cache-agreement paths have exactly this shape.

1. **Round 1** — the doc comment claimed a rejected batch "leaves nothing behind"; a number
   format survives the rollback. Enumerate what is restored, never generalize.
2. **Round 2** — a style assertion on a *pre-existing* cell cannot see a style-diff defect: the
   `SetCellValue` snapshot carries the whole `Cell`, so undo rebuilds the style from the value
   diff. Fixed by asserting on the recorded diff list instead.
3. **Round 3** — the same masking, moved to the link path: a chain test on a *previously-empty*
   cell cannot see a link-diff defect either, because the style undo's `range_clear_all` branch
   drops the link regardless. Fixed by seeding a pre-existing link.

Each of those defects passed the entire base suite. **The standing lesson: mutation-check the
specific property a test is named for, not just "revert the whole change."** Phase 3 onward
should budget for that explicitly — it is written into step 5 of this plan.

### One pre-existing gap closed as a side effect

Nothing in `ironcalc_base` pinned `set_user_input_with_link_diffs`' `old_link.is_none()`
suppression: emitting a style diff on the retarget path passed all 2284 pre-change tests. The
diff-shape test now pins it in both directions, and (fourth case) pins the per-write `old_link`
snapshot too. That is a small upstream win riding along with the fix.

### Carried forward

- **`fix/undo-set-cell-style-clears-inherited`** — undo of an auto-link materializes
  `Style::default()` over an inherited row/column style. Pre-existing, shared with single-cell
  `set_user_input`, and **FreeCell will hit it from Phase 4 on**. Wants its own fork branch +
  upstream PR and a `GAPS.md` row; capture it in `PROJECTS.md` / `projects/` at Phase 6.
- The two remaining "Found while building" notes (per-cell `Style` clone on the bulk path; the
  `freecell-fixes` merged-region guard the batch bypasses) are informational for Phase 3+.
- The upstream PR is **prepared, not opened** — the agent cannot open PRs on
  `ironcalc/IronCalc`. Compare link, title and description are in the section above; it is
  stacked on the `fix/batch-set-inputs` PR and must land after it.

### Final fork state (pushed)

| Ref | Commit | Note |
|---|---|---|
| `main` | `91d343c3` | unchanged this phase |
| `freecell-fixes` | `195f11e6` | merge of the fix branch |
| `fix/batch-set-inputs-link-diffs` | `1022da12` | **new**, off `fix/batch-set-inputs` |
| `fix/batch-set-inputs` | `180192ff` | unchanged (the stacked base) |
| `fix/paste-fill-relative-refs` | `a040dbbb` | unchanged |
| `fix/structural-edits-adjust-frozen-pane` | `274d173c` | unchanged |

The merge into `freecell-fixes` was clean, as predicted: `common.rs` diverges by +71/−2 and
`undo_redo.rs` by +103/−1 between the branches, but none of it inside `set_user_inputs`.
`cargo build`, `cargo test --workspace` (2376 base + 220 xlsx), `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets` are clean on both `fix/batch-set-inputs-link-diffs` and
`freecell-fixes`.

**Phase 3 picks up from `freecell-fixes` `195f11e6`** — that is the sha to pin.

### Still outstanding for the owner (carried from Phase 1)

Five obsolete remote branches still need deleting on `scosman/IronCalc` — the container's proxy
refuses both `git push --delete` and the REST call: `fix/trim-internal-runs`,
`fix/address-empty-sheet`, `fix/xmatch-array-constant`, `fix/xlsx-bool-import`, `fix/e2-numfmt`.
