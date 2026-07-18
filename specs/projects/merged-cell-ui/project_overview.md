---
status: draft
---

# Merged Cell UI

Add full **merged-cell UI support** to FreeCell, built on top of the merged-cell
engine work we just landed in our IronCalc fork (`scosman/ironcalc`,
`freecell-fixes` branch — the `claude/merged-cells-implementation-yv1pr7` work,
now merged in).

This project:

- **Re-pin** FreeCell to the newer `freecell-fixes` branch version that carries the
  merged-cell engine API.
- **Implement the UI side of merged cells** using the new engine work — rendering,
  selection/editing, and creating/removing merges from the UI.

## Background

The engine now exposes a real merged-cell API on `UserModel`:
`merge_cells(sheet, row, column, width, height)`, `unmerge_cells(sheet, row, column)`,
`get_merge_cells(sheet)`, and `get_merge_cell(sheet, row, column)`. Merging keeps
only the anchor (top-left) value and clears covered cells (Excel semantics); every
operation is a single undo/redo step. Writing to a covered cell is rejected by the
engine (the UI must redirect to the anchor). The engine also now **displaces merges
correctly across structural edits** (insert/delete rows & columns grow/shrink/drop
the region; a *move* that would split a region is blocked), and carries merges
through the xlsx open→save round-trip.

FreeCell today only has an **interim guard** from `mvp-gaps`: it *blocks*
insert/delete rows/cols and fill when they would touch a file-loaded merge, and it
does **not** render merges as a single box or snap selection/editing to them. This
project replaces that stopgap with real support.

This corresponds to the long-standing backlog item
[`projects/merged-cells.md`](../../../projects/merged-cells.md) (tiers a + b + c),
which was deferred out of `mvp-gaps` pending exactly this engine API.
