# `set_user_inputs` Skips the Merged-Cell Edit Guard (IronCalc fork)

**Status: Future** (found 2026-08-06 during the upstream fork sync; **pre-existing**, not
introduced by it). **Reachable in the shipped UI today** — see "FreeCell-side impact" below;
this is not a latent-only gap.

## The gap

Merged regions keep their value only in the **anchor** (top-left) cell; the covered cells hold
nothing and are painted over by the anchor. So the engine's interactive write path refuses a write
to a covered cell:

```
UserModel::set_user_input(sheet, row, column, value)
  → Err("cannot edit a cell inside a merged region; edit the anchor at B2")
```

(`base/src/user_model/common.rs`, the merged-cells project's Phase-2 guard.)

The **batched** write path added later, `UserModel::set_user_inputs` — one history entry for many
non-contiguous cells, used for things like find-and-replace — validates only sheet / row / column
bounds. It has no equivalent merge check, so a batch can land a value in a covered cell that the
single-cell path rejects. The value is then stored but never displayed, and it survives until the
region is unmerged.

## Why it happened

The two capabilities were built on **separate** fork branches — `merged-cells` and
`fix/batch-set-inputs` — per the one-fix-one-branch rule. Neither review saw the other, and the
merge into `freecell-fixes` was textually clean because they touch different functions in the same
file. Nothing flagged it.

## Why it wasn't fixed during the sync

The 2026-08-06 sync was scoped as a pure merge (fast-forward `main`, merge into `freecell-fixes`,
reconcile drift in FreeCell). Folding a behaviour fix into it would have muddied a merge that
reviewers need to be able to read as "no new behaviour".

## What the fix looks like

Hoist the covered-cell check out of `set_user_input` into a small shared helper and call it from
`set_user_inputs`' existing **up-front** validation loop — that loop already exists precisely so
the batch is all-or-nothing, so the guard belongs there rather than mid-write. Add a test that a
batch containing one covered-cell target is rejected whole, leaving the model and the history
untouched.

Both APIs are headed upstream as independent PRs, so **whichever lands second should carry the
guard**, keeping each PR single-purpose.

## FreeCell-side impact today

**Moderate, and reachable now** — not latent. FreeCell has two callers of the batch path, and they
sit on opposite sides of this gap:

- **Replace All (`WorkbookDocument::replace_all_matches`) is safe**, structurally rather than by
  luck. It builds its batch only from cells that *matched* the search, and `merge_cells` clears
  the covered cells' content when the region is created — so a covered cell holds nothing, can
  never match, and never enters the batch.
- **Paste Values (`WorkbookDocument::paste_values`, ⌘⇧V) is the live path.** It builds an entry
  for **every** cell of the `paste_w × paste_h` destination rectangle, unfiltered, and hands the
  whole batch to `set_user_inputs`. Nothing upstream of it filters merges: FreeCell's only merge
  guard is `fill_merge_guard` in `worker/run.rs`, wired to `FillDown`/`FillRight`/`FillDrag` alone
  (`EditRejectedReason::MergedCells` is raised nowhere else in the product code), and
  `run_guarded_paste` is a `catch_unwind` + paused-evaluation wrapper, not a merge check.

So today, pasting values over a range that contains a merged region writes values into covered
cells. They are stored but never painted (the anchor covers them), they survive until the region
is unmerged, and they are written out on xlsx save. FreeCell ships merge/unmerge
(`Command::MergeCells`), so a user can reach this with ordinary actions.

It stops short of severe: nothing *visible* is corrupted, the anchor write itself is legitimate,
and the damage is confined to sheets that actually contain merges. But "silent, persistent,
invisible state produced by a standard keyboard shortcut" is well above the "Low" this note
originally claimed.

## Pointers

- Fork bookkeeping: `specs/projects/merged-cells/implementation_plan.md` → "Known gap".
- The guard being duplicated: `base/src/user_model/common.rs`, in `set_user_input`.
- The live FreeCell path: `app/crates/freecell-engine/src/document.rs`, `paste_values`.
- The guard that does *not* cover it: `app/crates/freecell-engine/src/worker/run.rs`,
  `fill_merge_guard`.
