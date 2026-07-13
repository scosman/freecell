---
status: draft
---

# Functional Spec: Excel Table-Style Import + Resolution

> **Scope shape.** This spec covers what the *engine + FreeCell* must do to resolve and
> render Excel table styles. It intentionally separates **custom (dxf-based) table styles**
> — which unblock the fixture and are the v1 target — from **built-in (theme-derived) table
> styles**, which are larger and may be a separate phase or an explicit v1 cut (see Open
> Questions). Design detail lives in `architecture.md`; the fork→upstream mechanics follow
> `specs/projects/ironcalc-upstreaming/` §Operating model.

## 1. Goal & shape

When FreeCell opens a workbook that contains an Excel **table** (`<table>` +
`<tableStyleInfo>`), the cells inside that table must resolve the formatting the table style
prescribes — fills, font (bold / italic / colour), borders, number format, alignment — for
every table region (header row, total row, first/last column, and the striped data bands),
exactly as Excel paints them. Two capability tiers:

1. **Custom table styles (dxf-based)** — the style is defined in the file's
   `<tableStyles>` as `<tableStyleElement>`s that point at entries in the file's `<dxfs>`
   list. This is the fixture's case (dxf 66/67/68). **v1 target.**
2. **Built-in table styles (theme-derived)** — the style name is one of Excel's ~60
   built-in styles (e.g. `TableStyleMedium2`) and the file carries **no** `<tableStyles>`
   entry and often **no** dxfs; the visual is derived from the theme (accent colours,
   tints, banding). This is the common "Format as Table" default. **Larger sub-problem —
   see §5 and the Open Questions.**

The deliverable is: the engine resolves table styling into each cell's resolved `Style`,
and FreeCell's publish/cache/render paths consume that resolved style so the grid paints
it.

## 2. Behaviors — custom (dxf-based) table styles

### 2.1 Region membership

For a cell inside a table's `reference` rectangle, the engine determines which table
**regions** the cell belongs to. A cell can belong to several at once (e.g. the top-left
data cell is in `firstColumn` *and* a row-stripe band); the styling of all matching regions
is layered in a defined precedence order (§2.3). Regions:

| Region | Membership rule (within the table's `ref` rectangle) |
|---|---|
| `wholeTable` | every cell in the table |
| `headerRow` | the top `headerRowCount` rows (typically 1; 0 if the table is header-less) |
| `totalRow` | the bottom `totalsRowCount` rows (0 or 1) |
| `firstColumn` | the leftmost column (only when `showFirstColumn`) |
| `lastColumn` | the rightmost column (only when `showLastColumn`) |
| `firstRowStripe` / `secondRowStripe` | alternating bands over the **data** rows (only when `showRowStripes`) — see §2.2 |
| `firstColumnStripe` / `secondColumnStripe` | alternating bands over the **data** columns (only when `showColumnStripes`) |
| `firstHeaderCell` / `lastHeaderCell` / `firstTotalCell` / `lastTotalCell` | the four table corners |

"Data region" = the table rectangle minus the header row(s) and total row(s). Banding is
computed over the data region only.

### 2.2 Stripe banding

Row/column stripes alternate `first*Stripe` / `second*Stripe` across the data region.
Each `<tableStyleElement>` may carry a `size` (band thickness in rows/columns, default 1);
Excel repeats `first` (size N) then `second` (size N) down/across the data region. v1 must
honour the default `size=1`; wider bands are handled by reading the element's `size`
(see architecture; treat >1 as a correctness detail to verify).

> **Open question — banding fidelity.** The fixture does **not** exercise stripes (its
> category tables use `headerRow` + `wholeTable` + `totalRow` only). Do we implement full
> stripe banding in v1 (correct but untested by the fixture), or ship header/total/column
> regions first and add stripes as a fast-follow? Recommendation: implement stripes in v1
> (they're cheap once membership + precedence exist) but gate the *render-test* coverage of
> stripes behind a synthetic fixture, since the budget file can't prove them.

### 2.3 Region precedence

When a cell matches multiple regions, the corresponding dxfs are applied **in Excel's
table-style precedence order** — least-specific first, most-specific last, so the most
specific region wins field-by-field. The order (per ECMA-376 §18.8.40 / `ST_TableStyleType`;
**verify exact ordering at implementation time**), lowest → highest precedence:

1. `wholeTable`
2. `firstColumnStripe`, `secondColumnStripe`
3. `firstRowStripe`, `secondRowStripe`
4. `lastColumn`
5. `firstColumn`
6. `headerRow`
7. `totalRow`
8. `firstHeaderCell`, `lastHeaderCell`, `firstTotalCell`, `lastTotalCell`

Consequences that match the fixture: a HOUSING header cell is `wholeTable` + `headerRow`;
`headerRow` (teal fill + white bold) is applied after `wholeTable` (borders), so the header
shows teal-on-white-bold *and* the whole-table borders where the header dxf doesn't override
them. A Subtotal cell is `wholeTable` + `totalRow`; `totalRow` (bold) wins its bold field.
A data cell is `wholeTable` only → thin borders.

### 2.4 Per-region dxf source: named style vs. per-table override

A region's dxf can come from two places; both must be honoured, with the per-table override
winning:

- **Named-style element** — `<tableStyleInfo name="Personal monthly budget">` → the
  matching `<tableStyle name="…">` in `<tableStyles>` → its `<tableStyleElement type="…"
  dxfId="…">`. (The fixture's mechanism.)
- **Per-table / per-column direct override** — `headerRowDxfId`, `dataDxfId`,
  `totalsRowDxfId` on the `<table>` element itself (and per-`<tableColumn>`), which override
  the named style for that region. IronCalc already models these fields on `Table` /
  `TableColumn` (though buggily parsed — see §4).

### 2.5 Interaction with direct cell styles — **direct wins**

A cell's own **direct** formatting (an explicitly-set fill/bold/border on the cell's `xf`)
overrides the table style, per Excel. The table style only supplies formatting the cell does
not itself specify. In the fixture the affected cells carry no competing direct style, so
this is not exercised there — but it is a required semantic and a real design challenge
because IronCalc's resolved `Style` does not record which fields were explicitly set.

> **Open question — direct-vs-table precedence (the hard one).** IronCalc's
> `get_style_for_cell` returns a **fully-resolved** `Style` with no per-field "was this set
> explicitly?" provenance, yet Excel requires direct cell formatting to win over the table
> style. Options (detailed in `architecture.md`):
> - **(a) v1 pragmatic** — overlay the table dxfs on top of the cell's resolved own style
>   (table wins where both set a field). Correct for the fixture and the common case (table-
>   body cells rarely carry competing direct formatting), but technically inverts precedence
>   when a cell *does* carry a direct fill/bold inside a table.
> - **(b) provenance heuristic** — treat any field where the cell's own `xf` differs from
>   the workbook's normal/default `xf` as "direct" and let it win over the table dxf.
>   Reasonably faithful, more code.
> - **(c) full layering** — reconstruct Excel's format stack (table < named cell style <
>   direct). Most faithful, most invasive.
> Recommendation: **(b)** as the target, **(a)** acceptable as an explicit v1 cut if (b)
> risks the schedule — but the owner should pick, because it's a real fidelity trade-off.

### 2.6 No-table cells are unaffected

A cell not inside any table's `reference` resolves exactly as it does today —
`get_style_for_cell` returns the same `Style`. The resolver is a strict overlay that only
engages inside table rectangles; it must not perturb any cell outside a table, any
band-styled cell, or any directly-styled cell (the `direct_gray_fill_resolves` guard for
E6/J4 must stay green). This is a hard non-regression requirement.

## 3. Behaviors — the `tableStyleInfo` parse fixes (prerequisite)

Independent of resolution, the fork's existing table parsing has bugs that must be fixed
first, or the resolver has nothing correct to read (verified in fork code — see
`architecture.md` §4):

- **Style name dropped.** The parser searches for the wrong element tag, so
  `TableStyleInfo.name` is always `None` — the link from a table to its named style is lost.
- **Stripe flags forced to defaults.** Because of the same wrong-tag bug, `show_row_stripes`
  / `show_first_column` / `show_last_column` / `show_column_stripes` never read their real
  attributes (`show_row_stripes` is pinned `true`). The task framed this as "inverts
  `showRowStripes`"; verify the exact defect against the fixed parse.
- **`dataDxfId` = `headerRowDxfId`.** The per-table and per-column `data_dxf_id` are read
  from the `headerRowDxfId` attribute (copy-paste), so data-region overrides equal the
  header's.

## 4. Acceptance criteria

**Engine (fork):**

1. The three `#[ignore]`d tests in
   `app/crates/freecell-engine/tests/personal_monthly_budget_fixture.rs` are **un-ignored
   and pass** against the fork:
   - `table_style_header_is_teal_white_bold` — B12: `fill.color.is_some()`, `font.b`,
     `font.color.is_some()`.
   - `table_style_data_cells_have_borders` — C13: `border.top.is_some() &&
     border.bottom.is_some()`.
   - `table_style_subtotal_row_is_bold` — B23: `font.b`.
2. `opens_through_freecell_document`, `title_resolves_century_gothic`,
   `every_font_name_survives_import`, and `direct_gray_fill_resolves` **stay green** (no
   regression to values, font-name import, or direct fills).
3. Fork unit tests: `<tableStyles>` parsing (name→region→dxfId map), the `tableStyleInfo`
   fixes (name populated, stripe flags read, `dataDxfId` distinct), region membership, and
   precedence — all with synthetic inline-XML / crafted-zip fixtures (no copyrighted xlsx in
   the fork). Fork `cargo test` + `make lint` (fmt + strict clippy) clean.

**FreeCell (consumption):**

4. FreeCell's published/cached style for B12/C13/B23 of the budget sheet reflects the table
   styling (the publish path and the resident-cache render path both show it), and the
   resident-cache **agreement contract** stays green (the cache and a fresh
   `get_style_for_cell` re-read agree, both including the table overlay).
5. `cargo test` across the FreeCell workspace green; fmt + strict clippy clean.

**Render fidelity:**

6. The grid **visibly** paints the teal HOUSING/… headers (fill + white bold), the thin
   data-cell borders, and the bold Subtotals when the budget file is open — verified via the
   pixel render suite (a table-style render case + full-suite baseline eyeball) and the CI
   `render` gate. See `architecture.md` §Render-test plan.

**Built-in styles (if in v1 scope — see Open Questions):**

7. A workbook whose table uses a built-in style (e.g. `TableStyleMedium2`) with no dxfs
   renders header/banding derived from the theme. If built-in is deferred, this is explicitly
   out of v1 scope and such a table renders unstyled (documented, tracked as a follow-up gap)
   — **not** a regression, since it renders unstyled today.

## 5. Edge cases

- **Header-less / total-less tables** (`headerRowCount=0` / `totalsRowCount=0`) — no header
  / total region; membership degrades cleanly.
- **`dxfId` out of range** — a `<tableStyleElement>`/override pointing past the `<dxfs>`
  list is skipped (no panic), matching IronCalc's tolerant style handling.
- **Missing named style** — `tableStyleInfo.name` set but no matching `<tableStyle>` (or a
  built-in name with no dxfs) → no custom dxfs to apply; falls to the built-in path (§4.7) or
  renders unstyled if built-in is out of scope. Never errors the open.
- **Overlapping / adjacent tables** — a cell in two tables' rectangles is a malformed file;
  resolve deterministically (e.g. first table by iteration/name order) and document. Real
  files don't overlap tables.
- **Table on a different sheet** — `Table.sheet_name` scopes membership to the owning sheet;
  a cell at the same row/col on another sheet is unaffected.
- **Direct fill inside a table** (e.g. a user-highlighted cell) — see §2.5 Open Question;
  v1 behaviour per the chosen option.
- **Empty but boxed data cells** — a data cell with no value and no own style still needs
  its `wholeTable` borders. This is the FreeCell **enumeration** challenge: the cache builder
  scans populated/styled cells + bands, so a truly empty in-table cell is not enumerated
  today. See `architecture.md` §6 (the resolver must drive enumeration over table
  rectangles, or the empty cell's borders are lost). The fixture's data cells (C13 etc.)
  carry a `$` number-format `xf`, so they *are* enumerated — but the general case is not, and
  the spec requires empty-but-boxed cells to paint.

## 6. Out of scope

- **Table *editing*** (creating/resizing/restyling tables, "Format as Table" UI, totals-row
  formulas). This is import/resolution/render fidelity only.
- **Table *save* fidelity beyond what already round-trips** — we do not add table-style
  export work here; existing table geometry round-trips as IronCalc already handles it.
  (Flag if resolution changes anything on save.)
- **Autofilter dropdowns, sort/filter UI** — unrelated table features.
- **Conditional formatting** — also dxf-driven, but a separate feature with its own
  region/priority model; not in scope (though the `Dxf::apply_to` reuse is shared plumbing).

## 7. Constraints

- **Fix the engine, don't hack FreeCell** — all parsing/resolution lands in the fork
  (`freecell-fixes`), contributed upstream on owner sign-off per the `ironcalc-upstreaming`
  operating model. FreeCell changes are limited to *consuming* the resolved style.
- **No new prod deps** in FreeCell; the fork already carries `roxmltree`/`zip` for import.
- **Non-regression** — no-table cells, direct fills, bands, values, number formats, and
  font-name import all stay exactly as they are (§2.6, §4.2).
- The fork already fixes theme-colour (E1), tint, and indexed-colour (E5) resolution that
  `Dxf::apply_to` depends on — **build on that, don't redo it.**

## 8. Open questions (for the owner)

1. **Built-in (theme-derived) styles in v1 or v2?** The fixture uses a *custom* dxf-based
   style, so all three acceptance tests pass with only the custom path. Built-in styles are
   the larger, subtler sub-problem (encode Excel's built-in catalog + theme derivation).
   **Recommendation:** ship custom-dxf styles first (v1, unblocks the fixture + real
   templates), built-in styles second (v2) — a clean cut because the fixture doesn't need
   them. Confirm.
2. **Direct-vs-table precedence** — see §2.5. Which option (a/b/c)?
3. **Stripe banding in v1?** — see §2.2. Implement now (recommended) or fast-follow?
4. **Fork-first vs. fork+FreeCell together?** Land + validate the engine resolver against the
   fixture's engine-level tests first (fork PR-ready), *then* wire FreeCell consumption +
   render; or do both in one project pass? **Recommendation:** one project, phased
   (fork parsing → fork resolver → FreeCell consumption → render), since the FreeCell
   enumeration design (§5, empty boxed cells) can feed back into the resolver's shape and is
   best co-designed.
