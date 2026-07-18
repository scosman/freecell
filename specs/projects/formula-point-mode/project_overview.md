---
status: complete
---

# Formula Point-Mode + Range Highlighting

The second half of formula-entry UX (the first half — function autocomplete + signature
hints — shipped in `gaps_closing_7_15`). While a user is editing a formula:

1. **Range highlighting** — every cell reference in the in-progress formula is
   color-highlighted on the grid, with the reference's token in the editor drawn in the
   same color (Excel's colored-refs behavior). Makes "what does this formula point at"
   legible at a glance.
2. **Point-mode** — clicking (or click-dragging) a cell/range on the grid inserts its
   reference into the formula at the cursor, instead of having to type `A1:B5` by hand.

Without point-mode, every formula reference must be typed manually — the single biggest
remaining rough edge in formula entry.

## What to build

- Tokenize the in-progress formula using the engine's **public `Lexer`/`Parser`** to find
  reference tokens and their positions.
- Paint colored range overlays on the grid for each distinct reference; color the matching
  editor token to match.
- Route grid clicks/drags into the editor as reference insertions when the cursor is in a
  "reference-ready" position (right after `=`, an operator, a comma, or an open paren);
  otherwise a grid click commits/closes the editor as it does today.
- Works in **both** the in-cell editor and the data-row editor.

## Scope boundaries (v0.5)

- In: colored range highlighting (same-sheet and already-typed cross-sheet refs);
  click/drag point-mode insertion for **same-sheet** references.
- Out (defer to v1): arrow-key point-mode (arrow around the grid to build a reference —
  the fiddlier interaction); cross-sheet point-mode insertion (click another sheet tab
  mid-formula).

## Why it's non-trivial

It threads live tokenization into the editor render path and, harder, routes grid mouse
input into an active editor without breaking the existing "click a cell to commit + move"
behavior — delicate input/selection code that also interacts with the in-progress
merged-cells work.

## Source

GAPS.md v0.5 tier row "Formula range highlighting + point-mode"; pairs with the shipped
autocomplete (`specs/projects/gaps_closing_7_15`, Phase 1).
