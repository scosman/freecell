---
status: complete
---

# Formula Point-Mode + Range Highlighting

The second half of formula-entry UX (the first half — function autocomplete + signature
hints — shipped in `gaps_closing_7_15`). While a user is editing a formula:

1. **Range highlighting (grid)** — every cell reference in the in-progress formula is
   highlighted on the grid — a rich colored fill + border around each referenced range,
   each reference a distinct color — so "what does this formula point at" is legible at a
   glance.
2. **Point-mode** — clicking (or click-dragging) a cell/range on the grid inserts its
   reference into the formula at the cursor, instead of having to type `A1:B5` by hand.

Without point-mode, every formula reference must be typed manually — the single biggest
remaining rough edge in formula entry.

**Scope split (owner, 2026-07-18).** v0.5 delivers the two behaviors above — both
**grid-side** and FreeCell-owned. Coloring the reference *tokens inside the formula text*
(and the richer in-editor formatting — backgrounds, Excel/Numbers/Sheets-like) is
**deferred to a separate future project**, because gpui-component's `InputState` exposes no
external per-range styling and we will **not** fork it; that in-editor styling needs a
FreeCell-owned text-input control, tracked as a **v1.0 GAP**
([`projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)).
The token→color map this project computes is exactly what that future control will consume.

## What to build

- Tokenize the in-progress formula using the engine's **public `Lexer`/`Parser`** to find
  reference tokens and their positions.
- Paint rich colored range highlights on the grid (fill + border) for each distinct
  reference. (In-editor token coloring is out for v0.5 — see the scope split above.)
- Route grid clicks/drags into the editor as reference insertions when the cursor is in a
  "reference-ready" position (right after `=`, an operator, a comma, or an open paren);
  otherwise a grid click commits/closes the editor as it does today.
- Point-mode works in **both** the in-cell editor and the data-row editor (they share one
  pending edit through the consolidated `EditController`).

## Scope boundaries (v0.5)

- In: rich **grid** range highlighting for **same-sheet** references; click/drag point-mode
  insertion for **same-sheet** references; the shared tokenization seam and the consolidated
  `EditController`.
- Out (defer to v1): **in-editor token coloring / rich in-editor text formatting** (needs a
  FreeCell-owned styled text-input control — v1.0 GAP,
  [`projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md));
  arrow-key point-mode; cross-sheet point-mode insertion (click another sheet tab
  mid-formula).

## Why it's non-trivial

It threads live tokenization into the grid render path and, harder, routes grid mouse
input into an active editor without breaking the existing "click a cell to commit + move"
behavior — delicate input/selection code that also interacts with the in-progress
merged-cells work.

## Source

GAPS.md v0.5 tier row "Formula range highlighting + point-mode"; pairs with the shipped
autocomplete (`specs/projects/gaps_closing_7_15`, Phase 1).
