# Paste Over a Spill Cell Loses Its Content on Undo (engine `paste_from_clipboard`)

**Status: Future** (spotted during code review of `fix/paste-fill-relative-refs`, 2026-08-06;
deliberately **not** fixed on that branch — it is a pre-existing bug in a different concern, and
CLAUDE.md's fork policy is one fix = one branch = one focused upstream PR).

## The bug

`UserModel::paste_from_clipboard` (`base/src/user_model/clipboard.rs`) writes a paste in two
passes: it computes every change, **clears the whole destination rectangle** with
`range_clear_contents(target_area)`, then applies the changes and records a diff per change.

A copied cell that was a **spill cell** (part of a dynamic-array result, `ClipboardCell.is_spill`)
carries no formula or value of its own — only a style. For those the change is pushed with
`new_value: None`, so the apply loop emits **only** a `Diff::SetCellStyle`. But the destination
cell was already wiped by `range_clear_contents`, and *that* clear is not in the diff list.

Net effect: pasting a copied spill cell over a cell that had content destroys the content, and
**undo cannot bring it back** — the undo entry only restores the old style.

Repro sketch (upstream-style, `base/src/test/user_model/`):

1. `A1 = =SEQUENCE(3)` → spills A1:A3, so A2/A3 are spill cells.
2. `C1 = "keep me"`.
3. Copy **A2** (a spill cell), select C1, paste.
4. C1 is now empty (correct-ish — Excel pastes the spilled *value*, we paste nothing).
5. **Undo.** C1 stays empty; `"keep me"` is gone for good.

## Why it matters more now

The paste **fill** added by `fix/paste-fill-relative-refs` repeats the copied rectangle across a
whole-multiple selection, so a copied spill cell now clears one destination cell **per
repetition**. The hole is not new and the fill does not create it, but it multiplies its blast
radius from one cell to the whole filled block.

## Fix sketch (own `fix/` branch, upstream PR)

Capture the destination cell **before** `range_clear_contents` for spill-sourced changes too, and
push a `Diff::SetCellValue`/`RangeClearContents` for them, so the clear is undoable — i.e. treat
"the paste cleared this cell and wrote nothing" as a recorded change rather than a silent side
effect. (The neighbouring `paste_csv_string` already captures `old_values` before clearing for
exactly this reason — that is the shape to copy.)

Worth deciding at the same time: Excel pastes a copied spill cell's **value**, not nothing. Doing
that too would make the paste correct as well as undoable, but it is a behavior change and should
be its own call — the undo hole can be closed without it.

## Where it is recorded

- Mentioned in the upstream PR body prepared for `fix/paste-fill-relative-refs`, so an IronCalc
  reviewer meets it as a known pre-existing issue rather than discovering it as "the fill broke
  undo".
- FreeCell side: reachable today (⌘C/⌘V over any dynamic-array spill region), but no FreeCell-side
  workaround should be added — per the engine policy this is fixed in the fork.
