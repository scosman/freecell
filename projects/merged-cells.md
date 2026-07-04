# Merge/Unmerge UI ("tier c" of merged-cell support)

**Status:** Future

## Context

Merged-cell support was scoped for the `mvp-gaps` project (investigation 2026-07-04,
against pinned IronCalc 0.7.1 source). Three tiers were identified:

- **(a) Render file-loaded merges** — anchor spans the region, covered cells + interior
  gridlines suppressed; save round-trip. **In `mvp-gaps`.**
- **(b) Selection/editing correctness** — clicking a covered cell selects the merge,
  ranges expand to whole merges, active-cell border spans the merge; editing routes to
  the anchor via selection snapping. **In `mvp-gaps`.**
- **(c) Create/remove merges from the UI** — **this project.**

Tiers (a)+(b) need **zero IronCalc changes**: `Worksheet.merge_cells` is a public field
(`ironcalc_base src/types.rs:113`), the xlsx importer parses `<mergeCells>`
(`ironcalc src/import/worksheets.rs:176-192`) and the exporter re-emits them
(`src/export/worksheets.rs:250-298`), so merges already survive FreeCell's open→save
pipeline today.

## Why tier (c) is its own project

- **No mutation API.** `UserModel` (what FreeCell holds) exposes no merge methods and
  its inner `Model` is `pub(crate)` — there is no path to write `merge_cells` through
  the wrapper. Options, in preference order:
  1. **Upstream PR** adding `UserModel::{merge_cells(), merge(), unmerge()}` + undo/redo
     history diffs (repo is active; the merge model on `main` is unchanged from 0.7.1,
     so the PR is additive). File the issue early so this may become a version bump.
  2. **Minimal `[patch.crates-io]` fork** carrying just those methods until upstream
     lands. Maintenance cost against the deliberate `=0.7.1` pin.
  - Serialization round-trip hacks (save → mutate bytes → reload) lose undo history —
    not viable.
- **Excel merge semantics** need product/design work: merging a range with multiple
  non-empty cells keeps only the top-left value (Excel warns first); unmerge restores
  independent cells; both must be single undo steps.
- **Structural-edit landmine.** IronCalc's `Model::insert_rows/delete_rows` (and column
  equivalents, `src/actions.rs:331,397`) do **not** adjust `merge_cells`, so
  inserting/deleting through a merge leaves stale A1 refs that save incorrectly. The
  `mvp-gaps` project ships insert/delete rows/cols UI, so tier (c) — or a defensive
  open-time/edit-time normalization — must fix or guard this interaction (FreeCell-side
  adjustment of `merge_cells` after structural ops, or the upstream fix).

## Sketch (when picked up)

1. File/track the upstream IronCalc issue (merge API on `UserModel` + structural-edit
   adjustment). Decide fork-vs-wait on its response.
2. Toolbar Merge/Unmerge control (merge variants can wait; single "Merge cells" toggle
   covers most usage).
3. Warn-on-data-loss dialog for multi-value merges; single undo step per op.
4. Extend the `mvp-gaps` merge rendering/selection machinery (resident-cache merge map)
   — no new render work expected.
5. Tests: round-trip with created/removed merges, undo/redo, structural-edit
   interaction, render suite cases.

**Size:** M–L. **Risk:** fork maintenance vs. upstream latency; undo-history diffs.
