---
status: draft
---

# Formatting Expansion

We want to add more formatting options to FreeCell. We already have very similar
formatting options, so this should follow existing patterns.

The work splits into two parts.

## Part 1 — More text formatting (before borders)

- **Strikethrough** — toggle, placed after underline.
- **Wrap text** — a single wrap button placed after strikethrough; toggles like
  bold/italic/etc.
- **Vertical alignment** (top, centre, bottom) — three icons like align
  left/right/center. A new section after left/right/center.

## Part 2 — Border formatting

Add border formatting:

- **Formatting controls**
  - line weight
  - line color
  - line style (solid, dashed, etc.)
- **UI** — propose a design during speccing.
  - All under the border menu.
  - Replace the "which lines" textual buttons ("All", "Inner", "Outer", etc.)
    with icons showing 4 cells, and solid or grey lines to indicate the border.
  - Add a row under those with controls for line weight, color and style.
  - The hard part is keeping the UX simple despite the complex modality. I can
    select a range of cells that has mixed borders. The way we do it is that the
    weight/style/color controls only apply to the current "which lines"
    selection.
    - **Example:** I select a big box of cells, with all sorts of borders
      contained within. I select the "outer" option and our default solid black
      border appears on outer without changing/overriding any inner borders
      (outer icon shows as selected when I tap it, and dropdown does **not**
      instantly close unlike today). I tweak weight/color/style and it only
      applies to the selected outer border, not impacting all the inner styles.
      I then click "top" which becomes selected and change style: it only impacts
      the top cells of the selection.
    - I click away to close, then open the menu again: no "which cells" is
      selected (even though many cells have borders).
    - **(P2)** Adjusting weight/style/color without a "which cells" selected
      changes the style of all borders in selection.
