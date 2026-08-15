---
status: draft
---

# Phase 1: Fork sync — `main` → upstream `91d343c3`, merged into `freecell-fixes`

## Overview

The hyperlinks engine already exists upstream (`ironcalc/IronCalc` main `91d343c3`). This phase
gets our fork `scosman/ironcalc` current so every later FreeCell phase has links available
through the `[patch.crates-io]` pin.

Work happens in `/home/user/IronCalc` (the fork checkout). Nothing in `/home/user/freecell`
changes except this phase plan.

Per `architecture.md` §1 the sequence is: fast-forward `main`; delete the five `fix/*` branches
that are already upstream; merge the new `main` into `freecell-fixes` resolving 6 files / 11
hunks (8 trivial, the real work being `user_model/common.rs`'s `delete_rows` / `delete_columns`
restructure vs our frozen-pane + merged-cells `Diff` fields); rebase the three surviving `fix/*`
branches. On top of that, add the two integration tests for the semantic gaps the merge cannot
flag (`research/ironcalc-links-api.md` §8d.2 and §8d.3), and answer the Phase-1 verification gate
about theme index 10.

The batch-input link-diff fix (§8d.1) is **explicitly not** in this phase — it is Phase 2 on its
own `fix/batch-set-inputs-link-diffs` branch, per the one-fix-one-branch-one-PR rule.

## Steps

1. **Verify preconditions.** `git merge-base --is-ancestor 60d9db19 upstream/main` (fast-forward
   is legal); `git cherry upstream/main origin/fix/<b>` for all eight `fix/*` branches to confirm
   five are patch-equivalent (`-`) and three are not (`+`).

2. **Fast-forward `main`** to `upstream/main` (`91d343c3b3a4c579621fda559f60d81784a5b266`).
   Assert the resulting `main` sha equals `91d343c3…` and that no merge commit was created.

3. **Delete the five already-upstream `fix/*` branches** on `origin`:
   `fix/trim-internal-runs`, `fix/address-empty-sheet`, `fix/xmatch-array-constant`,
   `fix/xlsx-bool-import`, `fix/e2-numfmt`. Leave the stale
   `claude/merged-cells-implementation-yv1pr7` alone (superseded, but deleting it is not in
   scope).

4. **Merge new `main` into `freecell-fixes`**, resolving the six conflicting files:
   - `base/src/lib.rs` — keep both `pub mod links;` and `mod merge_cells;`.
   - `base/src/user_model/mod.rs` — keep both `mod links;` and `mod merge_cells;`.
   - `base/src/test/user_model/mod.rs` — keep both `mod test_links;` and `mod test_merge_cells;`.
   - `base/src/actions.rs` — union the `use crate::types::{…}` import line; keep upstream's
     `displace_links` and our merged-cell displacement.
   - `base/src/user_model/undo_redo.rs` (×2) — keep both sides' appended `match` arms in the undo
     and redo matches.
   - `base/src/user_model/common.rs` (5 hunks) — 1 positional (keep upstream's
     `set_user_input_with_link_diffs` *and* our batch `set_user_inputs`); 4 real, in
     `delete_rows` / `delete_columns`: take upstream's structure
     (`let mut diff_list = self.range_link_diffs(&Area{..})?; … diff_list.push(Diff::DeleteRows{..})`)
     while keeping our `old_merge_cells` / `old_frozen_rows` / `old_frozen_columns` fields on the
     `Diff` variant.

5. **Verify the merge compiles and the whole fork suite is green** — `cargo build` and
   `cargo test` across the workspace (`base`, `xlsx`), plus `cargo fmt --check` and
   `cargo clippy`.

6. **Add the two semantic-gap integration tests** on `freecell-fixes`, in the fork's own test
   layout (`base/src/test/user_model/`), upstream style:
   - **Row/column move carrying both a merged region and a link** (§8d.2). Upstream lifts links
     out before the cell rebuild and re-attaches after; our merged-cells displacement runs in the
     same `actions.rs` functions and git interleaved them without conflict. The test moves a row
     (and a column) whose range holds both a merged region and a worksheet link and asserts both
     land at the destination, the source is clean, and no link is fabricated at the destination
     by the rebuild's `set_user_input` auto-linking.
   - **Clear-range vs merged cells** (§8d.3). Our "clear-all unmerges" logic and upstream's
     "clearing a range drops links" logic both hook `range_clear_all` / `range_clear_contents`.
     The test asserts a range clear over a merged region carrying a link drops the link, drops
     the merge for `range_clear_all`, keeps the merge for `range_clear_contents`, and that undo
     restores both.
   - **`delete_rows` / `delete_columns` over a merged region carrying a link.** These are the
     only hunks in the whole merge that were hand-resolved (step 4), so they need a test of their
     own: deleting exactly the region drops link and merge and one undo restores content + link +
     merge; deleting above/left of it shifts both.

7. **Rebase the three surviving `fix/*` branches** onto the new `main`:
   - `fix/paste-fill-relative-refs` — expected clean.
   - `fix/batch-set-inputs` — 1 positional conflict (same shape as step 4's `common.rs` hunk 1).
     Rebase only; the §8d.1 behaviour change is Phase 2's separate branch.
   - `fix/structural-edits-adjust-frozen-pane` — 4 `common.rs` conflicts, same shape as step 4.
   Build + test each rebased branch before force-pushing it.

8. **Push** `main`, `freecell-fixes`, and the three rebased `fix/*` branches to `origin`; delete
   the five obsolete remote branches.

9. **Verification gate (arch §3.5)** — determine whether the hyperlink theme slot resolves to a
   blue in the default theme, and report the answer. Back it with an assertion in the fork test
   suite, bound to `links::THEME_COLOR_HYPERLINK` rather than a literal index, so the Phase-3
   decision rests on executed code that cannot pass vacuously if the slot moves.

10. **Prove the merged behaviour, don't assume it.** For every hand-resolved hunk and every
    feature pair the merge brought into the same function, reintroduce the defect and confirm a
    test fails. A clean auto-merge is not evidence — see the fill-paste bug in Outcome.

## Tests

Every test below was mutation-checked: the named defect was reintroduced and the test observed to
fail, so none of them pass vacuously.

**`base/src/test/user_model/test_links_merge_cells.rs`** (new, `freecell-fixes` only — merged
cells are fork-only, so this interaction has no upstream home):

- `move_rows_carries_merged_region_and_link` — moving rows holding both a merged region and a
  worksheet link relocates both; the source keeps neither; the rebuild's `set_user_input` does
  **not** auto-link a URL-shaped value the user had unlinked; undo/redo round-trips (§8d.2).
- `move_columns_carries_merged_region_and_link` — the column mirror.
- `range_clear_all_drops_link_and_merge_in_one_undo_step` — `range_clear_all` over a merged region
  carrying a link drops link, merge and content; one undo restores all three (§8d.3).
- `range_clear_contents_drops_link_but_keeps_merge` — same range, contents-only: link and content
  go, the merge stays; undo restores the link.
- `range_clear_leaves_links_outside_the_range_alone` — the clear is bounded.
- `delete_rows_over_a_merged_region_drops_link_and_merge` /
  `delete_columns_over_a_merged_region_drops_link_and_merge` — pins the merge's only hand-resolved
  hunks: one undo restores content, link and merge together.
- `delete_rows_above_shifts_the_merged_region_with_its_link` /
  `delete_columns_before_shifts_the_merged_region_with_its_link` — displacement carries both.

**`base/src/types.rs`** (verification gate):

- `test_hyperlink_theme_slot_is_blue` — `Color::Theme(THEME_COLOR_HYPERLINK, 0.0)` resolves
  through the default theme's `hlink` to `#0563C1`, blue-dominant. Bound to the engine's own
  constant, so relocating the slot fails the gate rather than passing it vacuously. Pins the arch
  §3.5 conclusion that FreeCell needs no `LINK` substitution in `render_style_from`.

**`base/src/test/user_model/test_paste_fill.rs`** (on `fix/paste-fill-relative-refs`, merged into
`freecell-fixes`) — added while fixing the fill/link collision found in review:

- `paste_fill_repeats_the_copied_link_in_every_repetition`
- `paste_fill_repeats_the_absence_of_a_link_in_every_repetition`
- `paste_fill_repeats_links_of_a_block_in_both_axes`

**`base/src/test/user_model/test_set_user_inputs.rs`** (on `fix/batch-set-inputs`, merged into
`freecell-fixes`):

- `mid_batch_write_failure_rolls_back_the_writes_already_made` — the rollback branch that gives
  `6f086bb9` its name had no test. A CSE array formula cell makes `Model::set_user_input` fail
  *mid-write* (past the up-front coordinate validation), and the values already written are
  restored with no history entry recorded.

Regression coverage relied on (not newly written): the full `base` + `xlsx` suites, which include
upstream's link tests (`base/src/test/user_model/test_links.rs`, `test_fn_hyperlink.rs`,
`xlsx/tests/test.rs` hyperlink import/export round-trips) and our merged-cells and frozen-pane
suites — the merge resolution in step 4 is only trustworthy if all of them pass together.

## Outcome

**Verification gate (step 9) — answer: yes, the hyperlink theme slot is a blue.** Index 10 is the
`hlink` slot; the default theme is Office, whose `hlink` is `#0563C1`, and `hex_with_tint_to_rgb`
returns the hex verbatim at `tint == 0.0`. So `Color::Theme(10, 0.0)` — the colour `set_cell_link`
applies to a new link — resolves through FreeCell's existing `resolve_rgb`
(`freecell-engine/src/cache.rs`) to a blue with no help from FreeCell. **Phase 3 does not need the
`LINK` substitution in `render_style_from`** (arch §3.5). Pinned by
`test_hyperlink_theme_slot_is_blue`.

### One real bug the merge introduced — fill-paste vs links

`fix/paste-fill-relative-refs` reworked `paste_from_clipboard`'s value/style pass to iterate
`fill_offsets`; upstream's link pass, added on `main` in the meantime, still walked the clipboard
once from the target anchor. The two auto-merged with no conflict and no failing test, so a fill
linked **only its first repetition**: a copied link was dropped in tiles 2..N, and a URL-shaped
value whose link the user had deleted got auto-linked again there — with no `Diff::SetCellLink`,
so undo could not remove it either.

Fixed on `fix/paste-fill-relative-refs` (where the fill loop originates, so the upstream PR does
not regress upstream's own behaviour) and merged into `freecell-fixes`: each target cell's link is
now collected inside the same `fill_offsets` iteration that computes the value changes, and
applied from that list after the values are written. The two passes can no longer cover different
cells. Three `test_paste_fill.rs` cases cover both directions.

This is the shape `research/ironcalc-links-api.md` §8d exists to catch, and §8e's "**clean**
despite upstream's heavy `clipboard.rs` edits" verdict is what let it through — a clean auto-merge
was read as safe. **Lesson for future syncs: a clean merge of two branches that both rewrite the
same loop is a reason to test, not a reason to relax.**

### Deviations from the plan

**Deviation 1 — `freecell-fixes` was behind `fix/batch-set-inputs`, and was reconciled.**
`freecell-fixes` merged that branch at `7d4b215e`, *before* `6f086bb9` ("make `set_user_inputs`
atomic on mid-batch write failure") landed on it, so the integration branch had been carrying the
older, non-atomic `set_user_inputs` — snapshotting each `old_value` mid-batch instead of before
the first write — and was missing its two regression tests. Since `freecell-fixes` is by
definition the sum of our fork fixes, the rebased branch tip was merged in, taking the branch side
for every `set_user_inputs` hunk.

The other two surviving `fix/*` branches are **content-contained** in `freecell-fixes` but, being
rebased, are not ancestors of it, and `git cherry` reports both as `+`. The checks that actually
establish containment: `fix/paste-fill-relative-refs` — `git merge-tree` against `freecell-fixes`
produces an identical tree for every file the branch touches; `fix/structural-edits-adjust-frozen-
pane` — each of its hunks located in `freecell-fixes` by hand (`git diff <branch> freecell-fixes
-- <file>` shows only `freecell-fixes`' own additions).

**Deviation 2 — the five obsolete remote branches could not be deleted by the agent.** Both
`git push --delete` and the GitHub REST `DELETE …/git/refs/heads/*` are refused by the container's
proxy (`403 "Write access to this GitHub API path is not permitted through this proxy"`). Their
content is patch-equivalent to upstream, so nothing is at risk; **the owner needs to delete
`fix/trim-internal-runs`, `fix/address-empty-sheet`, `fix/xmatch-array-constant`,
`fix/xlsx-bool-import`, `fix/e2-numfmt` on `scosman/IronCalc`.**

### Handed to Phase 2

`set_user_inputs`' doc comment claimed a rollback that the sync silently invalidated: the rollback
replays `Diff::SetCellValue` snapshots, so post-sync it restores values while leaving behind links
and link styling that `Model::set_user_input` auto-created. The comment now states the narrowed
guarantee, and `architecture.md` §2 / Phase 2 were amended so the fix covers the **rollback** path
as well as diff emission — and to correct the branch base: `fix/batch-set-inputs-link-diffs` goes
off `fix/batch-set-inputs`, not `main` (which has no batch API), as stacked upstream PRs.

### Final fork state (pushed)

| Ref | Commit |
|---|---|
| `main` | `91d343c3` (fast-forwarded to upstream) |
| `freecell-fixes` | `a6b635ef` |
| `fix/paste-fill-relative-refs` | `a040dbbb` (rebased + fill/link fix) |
| `fix/batch-set-inputs` | `588f6461` (rebased + rollback doc/test) |
| `fix/structural-edits-adjust-frozen-pane` | `274d173c` (rebased) |

`cargo build`, `cargo test`, `cargo fmt --all --check` and `cargo clippy --workspace
--all-targets` are clean on `freecell-fixes` and on each rebased `fix/*` branch.
