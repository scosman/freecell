---
status: draft
---

# Excel Table-Style Import + Resolution

## What

Make FreeCell render the cell formatting that Excel derives from a workbook's **table
styles** — the teal section-header fills, the thin data-cell borders, and the bold
Subtotal rows that a real user's "Personal Monthly Budget" template shows in Excel but
that FreeCell currently drops. The formatting is carried by the workbook's custom Excel
**table style** (`<tableStyles>` / `<tableStyleElement>` → dxfs), not by any per-cell
style, and our engine parses the table geometry but never resolves the named style into
per-cell formatting.

This is an **engine + render fidelity** feature spanning **two repos** — our IronCalc fork
(`scosman/ironcalc`, the parsing + resolution) and FreeCell (consuming the newly-resolved
styles in its publish/cache/render paths). That two-repo scope is why it's a project, not a
task, and why it follows the `ironcalc-upstreaming` operating model (fix the engine, don't
hack FreeCell; contribute the fix upstream).

## Why

FreeCell just landed a small fork fix (commit `2b01b85`) for lost font names in this same
file (the Century Gothic title). But the *dominant* fidelity loss — the whole visual
identity of the category tables — was deliberately deferred to this project. The diagnosis
(authoritative, from the investigation):

- The template's teal section headers (HOUSING / ENTERTAINMENT / TRANSPORTATION /
  INSURANCE / LOANS / TAXES — dark-teal fill + white bold text), the thin borders on every
  data cell, and the bold Subtotal rows all come from the workbook's **custom table style**
  named "Personal monthly budget". In the xlsx, `<tableStyles>` defines it via
  `<tableStyleElement>`s pointing at dxfs: dxf 67 = dark-teal fill + white-bold on
  `headerRow`; dxf 68 = thin borders on `wholeTable`; dxf 66 = bold on `totalRow`. The
  affected cells carry **no direct style** for these (the HOUSING header cell has no `s`;
  data cells are `s=3` = a `$` number-format only).
- **IronCalc parses the table geometry but never resolves the named table style → dxfs**,
  and never overlays them in `get_style_for_cell`, so those cells resolve unstyled. By
  contrast the gray summary-box fills *do* render, because they are *direct* `fillId` cell
  fills — the "gray renders, teal doesn't" clue that localized the defect to table-style
  resolution, not rendering.
- FreeCell's renderer is **not** the problem: `render_style_from` / `border_spec_from`
  already faithfully apply engine-reported bold / fill / borders / font-color / font. Once
  the engine resolves table styles into per-cell styles, FreeCell renders them.

Table styles are not an exotic corner of xlsx — they are how Excel's built-in "Format as
Table" and most polished templates carry their look. A spreadsheet that silently strips
them looks broken on exactly the files users are most proud of. This is a common real-file
case, not a one-off.

## Who

The user opening a real Excel workbook (a template, a "Format as Table" range, a shared
budget) in FreeCell and expecting it to look like it does in Excel. No new user-facing
control or workflow — the win is that files that already have table styling render
correctly on open.

## No new UI

This is an engine/render fidelity feature. It introduces **no new user-facing UI** — no new
controls, menus, or windows. There is therefore **no `ui_design.md`** in this project (the
`new_project` UI-design step is deliberately skipped). The only visible change is that
table-styled cells render their fills / borders / bold on open. The rendering surface it
touches (grid cell/border/fill painting) is in-scope for the pixel render suite; that
validation is planned as a dedicated late phase (see `architecture.md` §Render-test plan).

## Acceptance signal already in the repo

`app/crates/freecell-engine/tests/personal_monthly_budget_fixture.rs` contains three
`#[ignore]`d tests that are the forward spec for this feature:

- `table_style_header_is_teal_white_bold` — B12 "HOUSING" → fill + bold + non-default font
  colour.
- `table_style_data_cells_have_borders` — C13 → top & bottom borders.
- `table_style_subtotal_row_is_bold` — B23 "Subtotal" → bold.

They currently fail and are `#[ignore]`d. The feature is "done for this fixture" when they
flip green (un-ignored), plus render-level verification (see the functional spec's
acceptance criteria).
