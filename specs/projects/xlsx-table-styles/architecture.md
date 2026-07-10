---
status: draft
---

# Architecture: Excel Table-Style Import + Resolution

> Two-repo feature. Fork paths are in `scosman/ironcalc` (cloned at `/workspace/ironcalc`);
> FreeCell paths are prefixed `freecell:`. Fork line numbers below were read from the
> `freecell-fixes` branch during speccing and are **grounded in the actual code**; they may
> drift by implementation time — treat them as "here, roughly" not "exactly line N".
> The fork→upstream mechanics follow `specs/projects/ironcalc-upstreaming/` §Operating
> model verbatim (branch-per-fix, PR on sign-off).

## 1. Repos, crates, branches

- **Fork:** `scosman/ironcalc` — workspace: `base/` → crate `ironcalc_base`, `xlsx/` → crate
  `ironcalc`. FreeCell builds against branch `freecell-fixes` via `freecell:app/Cargo.toml`
  `[patch.crates-io]`.
- **Branch topology (per operating model):** `main` = clean upstream mirror; one
  `fix/<slug>` per upstream-submittable change off `main`; `freecell-fixes` merges them and
  is what FreeCell patches to. This feature is naturally **two upstream-shaped fixes**:
  - `fix/table-style-info-parse` — the `tableStyleInfo` / `dataDxfId` parse bugs (§4). Small,
    self-contained, independently valuable — a clean standalone PR.
  - `fix/table-style-resolve` — parse `<tableStyles>` + the resolver + `get_style_for_cell`
    wiring (§3, §5). The substantive fix.
  (Built-in/theme-derived styles, if built, are a third: `fix/table-style-builtin` — §7.)
- **FreeCell** changes (§6) live in this repo on the feature branch; no upstream component.

## 2. Data model

### 2.1 Fork types that already exist (read from code)

- `Table` (`base/src/types.rs:353`) — `reference: String` (e.g. `"B11:E23"`),
  `header_row_count`, `totals_row_count`, `header_row_dxf_id`/`data_dxf_id`/
  `totals_row_dxf_id: Option<u32>`, `columns: Vec<TableColumn>`, `style_info:
  TableStyleInfo`, `sheet_name`. Stored workbook-global at `Workbook.tables:
  HashMap<String, Table>` (`types.rs:132`), reachable from `Model` as
  `self.workbook.tables`.
- `TableStyleInfo` (`types.rs:396`) — `name: Option<String>`, `show_first_column`,
  `show_last_column`, `show_row_stripes`, `show_column_stripes`.
- `TableColumn` (`types.rs:371`) — per-column `header_row_dxf_id`/`data_dxf_id`/
  `totals_row_dxf_id`.
- `Dxf` (`types.rs:426`) — `font: Option<DxfFont>`, `fill: Option<Fill>`, `border:
  Option<Border>`, `num_fmt: Option<NumFmt>`, `alignment: Option<Alignment>`, plus
  `Dxf::apply_to(&self, base: &Style) -> Style` (`base/src/styles.rs:424`) which overlays
  each present field onto `base`. **We reuse `apply_to` unchanged.**
- `Styles` (`types.rs:435`) — has `dxfs: Vec<Dxf>` (populated by `load_dxfs`,
  `xlsx/src/import/styles.rs:434`) but **no** `table_styles` field. This is the gap.

### 2.2 New fork type — the named table-style catalog

Add to `Styles` a parsed `<tableStyles>` catalog. `<tableStyles>` lives in `styles.xml`:

```xml
<tableStyles count="1" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="…">
  <tableStyle name="Personal monthly budget" pivot="0" count="3">
    <tableStyleElement type="wholeTable" dxfId="68"/>
    <tableStyleElement type="headerRow"  dxfId="67"/>
    <tableStyleElement type="totalRow"   dxfId="66"/>
  </tableStyle>
</tableStyles>
```

Proposed model (in `base/src/types.rs`, matching the crate's `Encode, Decode, Serialize,
Deserialize` derive conventions):

```rust
pub struct TableStyles {
    pub default_table_style: Option<String>,
    pub styles: Vec<TableStyle>,          // by name
}
pub struct TableStyle {
    pub name: String,
    pub elements: Vec<TableStyleElement>,
}
pub struct TableStyleElement {
    pub kind: TableStyleType,             // enum, ST_TableStyleType
    pub dxf_id: u32,                      // index into Styles.dxfs
    pub size: u32,                        // stripe band thickness; default 1
}
pub enum TableStyleType {
    WholeTable, HeaderRow, TotalRow, FirstColumn, LastColumn,
    FirstRowStripe, SecondRowStripe, FirstColumnStripe, SecondColumnStripe,
    FirstHeaderCell, LastHeaderCell, FirstTotalCell, LastTotalCell,
    // (blankRow, firstSubtotalColumn, … exist in the spec; add lazily.)
}
```

Add `pub table_styles: TableStyles` to `Styles` (default empty). This is a **positional
index into the existing `dxfs`** — no dxf duplication. Verify the `Encode/Decode` +
`serde` derives compile against the crate's `bitcode`/serde setup (the sibling `Dxf` type
is the template).

> **Serialization caveat (verify).** `Styles` is `Encode, Decode` (bitcode) — adding a
> field changes the on-wire `to_bytes`/`from_bytes` layout. Confirm nothing pins a fixed
> `Styles` encoding across a version boundary (IronCalc's `to_bytes` is an internal cache
> format, so this should be fine, but check).

## 3. Fork parsing — `<tableStyles>` (`xlsx/src/import/styles.rs`)

`load_dxfs` (`styles.rs:434`) already walks `style_sheet` children filtering
`has_tag_name("dxfs")`. Add a sibling `load_table_styles(style_sheet) -> TableStyles`:

1. Find the `<tableStyles>` child (`has_tag_name("tableStyles")`); absent → empty
   `TableStyles`.
2. Read `defaultTableStyle` attribute.
3. For each `<tableStyle>` child: read `name`; for each `<tableStyleElement>` child, map
   `type` → `TableStyleType` (skip unknown types), read `dxfId` (→ `u32`), read `size`
   (default 1).
4. Return the catalog; wire it into the `Styles { … }` construction next to `dxfs`
   (`styles.rs:420-434`).

No colour/border work here — the dxfs those elements point at are already parsed by
`load_dxfs`, and the fork already resolves theme/tint/indexed colours inside dxfs (E1/E5).

## 4. Fork parse fixes — `tableStyleInfo` (`xlsx/src/import/tables.rs`)

Three concrete bugs, **confirmed in code** on `freecell-fixes`:

- **Wrong element tag → name + stripe flags lost.** `load_table` builds `style_info` by
  searching `table.descendants().filter(|n| n.has_tag_name("tableInfo"))`
  (`tables.rs:172-175`). The OOXML element is **`<tableStyleInfo>`**, not `<tableInfo>`, so
  the filter never matches and `style_info` always takes the `None` arm (`tables.rs:187`):
  `name: None`, `show_row_stripes: true`, all other flags `false`. **Fix:** filter
  `has_tag_name("tableStyleInfo")`. Then the `Some(node)` arm (`tables.rs:177-186`) reads
  the real `name` + flags. This alone restores the style-name link and the stripe flags.
  - The task described this as "drops the name" + "inverts `showRowStripes`". The dropped
    name is exactly the wrong-tag bug. The "inverts `showRowStripes`": with the tag fixed,
    the `Some` arm uses `get_bool(node, "showRowStripes")` (`util.rs:96`, default **true**),
    while the other flags use `get_bool_false` (default false) — which is OOXML-correct
    (`showRowStripes` defaults true, the rest false). **Verify** whether any residual
    inversion remains once the tag is fixed; if `get_bool`/`get_bool_false` are swapped for
    a flag, correct it. (As read, the primary defect is the wrong tag.)
- **`dataDxfId` copy-pasted from `headerRowDxfId`.** `data_dxf_id` is read from attribute
  `"headerRowDxfId"` at both the table level (`tables.rs:84`) and per column
  (`tables.rs:143`). **Fix:** read from `"dataDxfId"`.

Tests (fork, synthetic XML): a `<table>` with `<tableStyleInfo name="X" showRowStripes="0"
showFirstColumn="1"/>` parses `name = Some("X")`, `show_row_stripes = false`,
`show_first_column = true`; a `<table headerRowDxfId="7" dataDxfId="9">` parses
`data_dxf_id = Some(9)` distinct from `header_row_dxf_id = Some(7)`.

## 5. Fork resolver — the core

### 5.1 Where it hooks

`get_style_for_cell` (`base/src/model.rs:3334`) is today:

```rust
let style_index = self.get_cell_style_index(sheet, row, column)?;
let style = self.workbook.styles.get_style(style_index)?;
Ok(style)
```

Add a table overlay after the base resolve:

```rust
let base = self.workbook.styles.get_style(style_index)?;
let style = self.apply_table_styles(sheet, row, column, base)?;
Ok(style)
```

`apply_table_styles` is a new `Model` method (it needs `self.workbook.tables`,
`self.workbook.styles.{dxfs, table_styles}`, and the theme, all reachable). It must be a
**no-op** when the cell is in no table (returns `base` unchanged) — §2.6 non-regression.

### 5.2 The resolver algorithm

```
apply_table_styles(sheet, row, column, base) -> Style:
  table = first table whose sheet_name == sheet-name and whose `reference` rect
          contains (row, column); if none -> return base
  regions = compute_regions(table, row, column)          // §5.3
  ordered = regions sorted by precedence (§5.4), lowest first
  style = base
  for region in ordered:
      dxf_id = dxf_for_region(table, region)             // §5.5: named style OR per-table override
      if let Some(id) = dxf_id, styles.dxfs.get(id) = Some(dxf):
          style = dxf.apply_to(&style)                   // reuse existing apply_to
  return direct_wins_reconcile(base, style)              // §5.6
```

### 5.3 Region membership (`compute_regions`)

Parse `table.reference` (`"B11:E23"`) into `(r0,c0)-(r1,c1)` (a `Range`/A1 parser already
exists in `base` for table refs — reuse it). Then:

- `wholeTable`: always.
- `headerRow`: `row < r0 + header_row_count`.
- `totalRow`: `row >= r1 - totals_row_count + 1` (when `totals_row_count > 0`).
- data region = rows `[r0+header, r1-totals]`, cols `[c0, c1]`.
- `firstColumn` (if `show_first_column`): `column == c0`.
- `lastColumn` (if `show_last_column`): `column == c1`.
- row stripes (if `show_row_stripes`): over data rows, band index `= (row - dataR0) / size`;
  even band → `firstRowStripe`, odd → `secondRowStripe`.
- column stripes (if `show_column_stripes`): analogous over data cols.
- corners: `firstHeaderCell = headerRow ∧ column==c0`, etc.

### 5.4 Precedence order

Encode the §2.3 order as a rank on `TableStyleType`. **Verify the exact ordering against
ECMA-376 §18.8.40 at implementation time** — getting header-over-banding and total-over-
banding right is what makes the fixture correct. A single `fn precedence(kind) -> u8` table
keeps it auditable.

### 5.5 `dxf_for_region` — named style vs per-table override

For a region, the dxf id is (per §2.4, override wins):

- **Per-table/per-column override first:** `headerRow` → `table.header_row_dxf_id`;
  `totalRow` → `table.totals_row_dxf_id`; data regions → `table.data_dxf_id`; column-scoped
  regions may consult `TableColumn`'s per-column ids. If `Some`, use it.
- **Else the named style:** `table.style_info.name` → find `TableStyle` in
  `styles.table_styles` by name → its `TableStyleElement` whose `kind == region` → `dxf_id`.

Both index into `styles.dxfs`. Out-of-range → skip (§Edge cases). The fixture resolves via
the named-style path (its per-table dxf attributes are absent; the style name "Personal
monthly budget" links to the elements).

### 5.6 Direct-cell-style reconciliation — **the key design decision**

`Dxf::apply_to(base)` makes the **dxf win** over `base` field-by-field. For **conditional
formatting** that's correct (CF overrides the cell). For **table styles**, Excel requires the
**cell's direct formatting to win** over the table style (§2.5). Since `base` here is the
cell's fully-resolved own style with no per-field provenance, we must choose how to reconcile:

- **(a) v1 pragmatic — table wins (no reconcile).** Return the `apply_to`-stacked `style`
  directly. Correct for the fixture (its styled regions carry no competing direct format) and
  the common case. **Inverts precedence** only when a cell inside a table also carries a
  direct fill/bold — rare. Cheapest; documented fidelity gap.
- **(b) provenance heuristic (recommended target).** Compute the cell's own `xf` style
  (`get_cell_style_or_none`, i.e. the cell's *direct* layer) and the workbook normal/default
  style; any field where own-xf ≠ normal is "direct" and is re-applied on top of the
  table-stacked style so it wins. Reasonably faithful; ~1 helper.
- **(c) full layering.** Reconstruct table < named-cell-style < direct. Most faithful, most
  invasive (touches xf resolution). Out of scope for v1.

**This is Open Question 2 in the functional spec — the owner picks (a) vs (b).** The
architecture supports either behind the single `direct_wins_reconcile` seam; default to (b)
unless the owner cuts to (a).

## 6. FreeCell consumption (`freecell:app/crates/freecell-engine/`)

### 6.1 The two style-read paths (and why it's not a one-liner)

- **Publish path** — `document.rs::published_style` (`document.rs:312`) already calls
  `model.get_style_for_cell` (`:325`). Once the resolver is inside `get_style_for_cell`,
  the published `kind`/`text_color` **automatically** pick up table styling. The
  font-override render path (`document.rs:778`) and `set_font` also read
  `get_style_for_cell` — likewise automatic. ✅ mostly free.
- **Resident-cache path — the catch.** The production cache is built from each cell's
  **own** style: `cache.rs:364` reads `doc.cell_own_style` (= `get_cell_style_or_none`,
  `document.rs:365`), **not** the resolved `get_style_for_cell`. Table styling is a
  *derived, region-spanning* property that is **not** on the cell's own style, so the cache
  as built today will **not** contain it, and the agreement contract (`cache.rs:449` re-reads
  resolved `resolved_cell_style` = `get_style_for_cell`) will **break** the moment the
  resolved side gains table styling that the own-style side lacks.

So the FreeCell change is: make the cache-build path consume the **table-aware resolved**
style, and keep the agreement contract's two sides in sync.

### 6.2 Design — feed the cache the resolved style over table regions

Two coupled changes:

1. **Read resolved style for in-table cells.** In the cache cell path (`cache.rs:~362-388`),
   for a cell inside a table use the resolved `get_style_for_cell` (table overlay included)
   rather than `cell_own_style`. Simplest correct form: add a `doc` accessor that returns the
   **resolved** style (already exists as `resolved_cell_style`, currently `#[cfg(test)]` —
   promote it to a prod accessor, or add a sibling `render_style_resolved`) and use it for
   table cells; keep `cell_own_style` for the rest to preserve the existing
   own-style-shadows-band semantics. The agreement contract already compares against
   `get_style_for_cell`, so switching in-table cells to the resolved read makes both sides
   agree.
2. **Enumerate table rectangles.** The cache builder scans `sheet_data` (populated/styled
   cells) + row/col bands (`document.rs:351-354` comment). An **empty, unstyled** data cell
   inside a table (no value, no own `xf`) is not scanned — so its `wholeTable` borders would
   be lost (§5 functional edge case). Add a pass that, for each table on the sheet, iterates
   its `reference` rectangle and ensures every cell resolves+interns its (table-aware) render
   style. The fixture's data cells carry a `$` number-format `xf` so they're already scanned;
   this pass is for the general empty-boxed-cell case and for correctness of the whole region.

> **Open question — enumeration strategy.** Two shapes: **(i)** an explicit "for each table,
> walk its rect" pass in the cache builder (bounded by table size — tables are small vs. the
> 1M-row sheet, so cheap); or **(ii)** treat tables as a new band-like *region-style layer*
> the cache overlays at render time (like row/col bands), avoiding per-cell materialization.
> **Recommendation: (i)** — tables are small and bounded, it reuses the existing
> per-cell-style machinery, and it keeps the agreement contract's per-cell model intact.
> (ii) is more scalable but a bigger change to the cache's layering model; revisit only if a
> pathologically huge table appears. Owner/implementer confirms at build time.

### 6.3 What does NOT change

`render_style_from` / `border_spec_from` (`cache.rs:162/179`) are untouched — they already
map `Style`'s bold/fill/border/font-color/font faithfully (the renderer was never the
problem). The engine now hands them a `Style` that *includes* the table overlay; they render
it as-is. `workbook_theme()` (`document.rs:747`) still supplies the theme for colour
resolution.

## 7. Built-in (theme-derived) table styles — scope explicitly

The common "Format as Table" case: `tableStyleInfo.name = "TableStyleMedium2"`, **no**
`<tableStyles>` entry, often **no** dxfs — the look is *generated* from the theme (header
fill = an accent colour; banded rows = a tinted accent; borders from the theme). Excel ships
~60 built-in styles (Light 1–21, Medium 1–28, Dark 1–11) whose header/stripe/border recipes
are defined by the built-in style *index* against the workbook theme.

Reproducing this faithfully means encoding the built-in catalog (per-style: which theme slot
+ tint for header/stripe1/stripe2/borders) and deriving dxf-equivalents at resolve time. This
is the **largest, subtlest** sub-problem and the fixture does **not** need it (its style is
custom/dxf-based).

**Recommendation (functional-spec Open Question 1): make built-in styles a separate phase /
explicit v1 cut.** v1 ships custom-dxf resolution (unblocks the fixture + real templates that
carry dxfs). v2 (`fix/table-style-builtin`) adds a `builtin_table_style(name, theme) ->
synthetic per-region Dxf set` that plugs into the same resolver seam (§5.5 falls back to it
when `name` is a built-in and no `<tableStyle>`/dxfs exist). Deferring built-in is **not** a
regression — such tables render unstyled today; v1 leaves them unstyled and tracks the gap.

## 8. Error handling

- Malformed `reference`, out-of-range `dxfId`, unknown `type` → **skip that element/table,
  never error the open**, matching IronCalc's tolerant style handling and the fork's E1–E5
  posture. The workbook must always open; worst case a region renders unstyled.
- Overlapping tables → deterministic pick (first by iteration), documented; not an error.
- The resolver must be **allocation-lean on the hot path**: `get_style_for_cell` is called
  per rendered cell and per cache-build cell. The no-table fast path (§5.1) must short-circuit
  before any table scan (e.g. a cheap "any tables on this sheet?" guard, then a rect test).
  On a 1M-row sheet with no tables this must add ~zero cost.

## 9. Testing strategy

**Fork (`fix/table-style-info-parse`, `fix/table-style-resolve`):** unit tests with
synthetic inline XML / crafted zips only (no copyrighted xlsx in the fork, per operating
model): the `tableStyleInfo` parse fixes (§4 tests); `<tableStyles>` catalog parse;
`compute_regions` truth table (header/total/first-col/stripe/corner membership on a small
table); precedence (a cell in `wholeTable`+`headerRow` resolves header fields over
whole-table); `dxf_for_region` override-beats-named; the no-table no-op; out-of-range dxf
skip. Fork `cargo test` + `make lint` clean.

**FreeCell engine:** the three `#[ignore]`d tests in
`freecell:app/crates/freecell-engine/tests/personal_monthly_budget_fixture.rs` un-ignored and
green (B12 teal+bold+colour, C13 borders, B23 bold); the four existing guards stay green; the
resident-cache **agreement contract** green with the table overlay on both sides; a test that
an empty-but-boxed in-table cell interns its `wholeTable` borders (enumeration, §6.2).

**Render tests — see §10.**

## 10. Render-test plan (in-scope grid/cell change — dedicated late phase)

Per `CLAUDE.md`, resolving table styles changes what the grid paints (fills, borders, bold,
font colour on cell/row/sheet) → **in scope** for the pixel render suite. Plan it as its own
**late phase, after all engine + FreeCell coding + commits**, not intermingled:

- **While iterating (each coding phase):** run only the relevant subset via the wrapper's
  test-name filter — `app/render-tests/scripts/render_tests.sh test cell_`, `… test
  border_`, `… test fill_` — fast, in-flow. No full runs mid-coding.
- **New render case backed by the real file.** Add a render case that opens
  `personal_monthly_budget.xlsx` and renders a crop of the budget sheet showing a teal header
  + bordered data rows + a bold Subtotal. **Design point / open question:** the existing
  harness builds scenes over a `NewWorkbook` with injected geometry (per the
  `ironcalc-upstreaming` Phase-6 note), not by loading a real `.xlsx`. So either (i) add a
  harness capability to load an `.xlsx` fixture and render a region, or (ii) construct a
  synthetic table via engine APIs in the scene. **Recommendation:** prefer (i) if the harness
  can be extended cheaply (it exercises the true import→resolve→render path end-to-end, which
  is the whole point); fall back to (ii) if loading real files into the harness is heavy.
  Confirm at build time.
- **Late render-validation phase (once):** run the **full** pixel suite under a ~10-min
  watchdog (foreground `timeout` + Monitor check-in — never background-and-forget). Because
  this change **intentionally** alters rendering, **regenerate + eyeball** baselines
  (`render_tests.sh generate`) — verify the new table-style case looks right *and* that no
  unrelated baseline moved — and commit the refreshed baselines with the change. Then
  **dispatch the CI `render` gate** on the branch (`gh workflow run render.yml --ref
  <branch>` / Actions MCP), poll to green, confirm. This is the required truth before merge.
- Welcome/About/other-chrome are untouched, so they're out of the pixel suite's scope (no
  action needed there).

## 11. Single-file vs component docs

Single `architecture.md` (this doc). The feature is medium-sized and the two repos' pieces
are cohesive; no component needs its own doc. If built-in/theme-derived styles (§7) grow
into their own build, give *that* its own component/architecture note at the time.
