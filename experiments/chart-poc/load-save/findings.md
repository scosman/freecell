# load-save — findings

Phase 4 (Gate 4 — load/save stitching). Goal (functional_spec §5, §7, §10 #2/#4): parse chart
definitions out of a real `.xlsx` into `chart-model` and render them; and re-inject chart parts
on save so a chart **survives** IronCalc's chart-dropping writer. This answers the spec's
question and sets **display+save-preservation vs display-only** for the follow-on.

## Result: LOAD **PASS** · SAVE re-injection **PASS** → recommend **display + save-preservation**

Both halves work end-to-end in this container, gpui-free at the model/parse layer:

- **LOAD PASS.** `load_charts_from_xlsx` walks `worksheet → drawing → chart` and parses the
  authored fixture's three embedded charts (clustered column, multi-series line, pie) into
  `chart_model::Chart` from the cached `numCache`/`strCache` — **no formula eval, no IronCalc**.
  Rendering each loaded chart back through `chart-render` produces
  `results/loaded_{column,line,pie}.png`, all **agent-reviewed PASS** (`results/review.md`):
  correct kinds, cached values, series names, per-series colors (from `c:spPr`), titles, axis
  titles, and legends. The seam **parse → model → render** is proven on a real file.
- **SAVE re-injection PASS.** `save_with_charts` runs IronCalc's real writer into an in-memory
  zip, then splices the original chart machinery back in byte-for-byte. The round-trip
  (`open → IronCalc save → re-inject → reopen`) is verified three ways in `roundtrip_preserves_charts`
  and by the `fixtures` bin: (a) our own loader re-finds all three charts with **identical**
  cached values (`after == before`); (b) the output **reopens in IronCalc** without error (not
  corrupted); (c) structurally the worksheet carries a `<drawing>` ref, `[Content_Types].xml`
  declares the chart parts, and all `xl/charts/*` + `xl/drawings/*` parts are present
  (`results/roundtrip_charts_basic.xlsx`).

## Is load stitching "as quick as hoped"? — Yes, exactly as the research predicted

The extractor is the same shape as `open_fixups.rs`: `zip 0.6` + `roxmltree 0.19`, a few tag
walks, never touching IronCalc. `load.rs` + `xlsx.rs` are ~450 lines including the full
relationship-chain resolver and a cache reader that handles `strCache`/`numCache`, per-series
`spPr` color, scatter's two `valAx`/`xVal`/`yVal` shape, doughnut `holeSize`, and legend
position. The basic in-scope types (`bar`/`line`/`area`/`pie`/`doughnut`/`scatter`) all share the
`c:ser → c:cat/c:val` shape, so one parser covers them. **Load is not a swamp** — it is the
weekend-sized, tractable read the research called it.

Key implementation notes (verified, not assumed):
- **roxmltree matching by local name.** All tag/attr matching uses `tag_name().name()` and an
  attribute-by-local-name helper, so it is namespace/prefix-agnostic (`c:`, `a:`, `r:`, default
  namespaces all work) without binding namespace URIs.
- **Cached values only.** Points are read from `numCache`/`strCache`, sorted by `idx`. The `c:f`
  range formula is ignored — matching the "render purely from the cache, zero IronCalc
  dependence" path from `research/excel-chart-data-model.md §4c`.

## Is save re-injection "tractable or a swamp"? — Tractable, but with three sharp edges

Byte-preservation re-injection (§10 #2, the accepted bar) is real and works, but it is fiddlier
than "copy some zip entries." The three things that actually bite:

1. **IronCalc's naive `_rels` / `sheetData` parsers reject pretty-printed XML.** `load_sheet_rels`
   iterates the **raw children** of `<Relationships>` and reads `Type` on each; the worksheet
   parser iterates raw `<row>`/`<c>` children and reads `r` on each. A whitespace/newline text
   node between elements trips `Missing "Type"` / `Missing "r"`. **The authored fixtures and the
   re-injected `_rels` must be whitespace-free between elements** (real Excel is, for the same
   reason). This was the single biggest gotcha — worth flagging loudly for the follow-on.
2. **Three parts must be patched, not just carried.** IronCalc regenerates the worksheet without
   a `<drawing>`, emits no worksheet `_rels`, and omits the chart/drawing content-type overrides.
   Re-injection carries `xl/charts/*` + `xl/drawings/*` verbatim, but **must** (a) inject
   `<drawing r:id=…/>` into IronCalc's worksheet (and bind `xmlns:r`, which IronCalc may drop),
   (b) write a worksheet `_rels` with the drawing relationship, and (c) merge the chart/drawing
   `<Override>`s into IronCalc's `[Content_Types].xml`. All three are done by string splice + a
   little roxmltree.
3. **Worksheet ↔ part-name mapping.** Single-sheet fixtures map 1:1 (`sheet1.xml` ↔ `sheet1.xml`).
   Multi-sheet workbooks need a proper `sheet index → part` mapping via `xl/_rels/workbook.xml.rels`
   (IronCalc's output part order is not guaranteed to match the original). Out of PoC scope; the
   re-injection **fails loudly** if a targeted worksheet part is absent rather than silently
   dropping the chart.

None of these is a blocker; all are bounded and now solved for the single-sheet case. A
ship-quality version would generalize (2)/(3) and add the chart-aux parts (`colorsN`/`styleN`,
already carried by prefix) — a few days, not a rewrite.

## Stale-data caveat (unchanged from research)

Re-injection preserves the chart **as it was**, including its cached values. If the user edits the
data cells before saving, the re-injected chart's `numCache` goes stale relative to the sheet
(and the `c:f` still points at the old range). That is the accepted limitation of byte-preservation
(§8: "no reflow of cached values"); the follow-on either accepts stale-on-edit or refreshes the
cache from IronCalc's evaluated cells (a separate, larger effort — the "synthesize chart XML"
stretch goal).

## LibreOffice cross-check — inconclusive in this container (environment, not the file)

`soffice --headless --convert` fails with `source file could not be loaded` on **every** `.xlsx`
here — including the app's known-good real fixture `numbers_table.xlsx` (which IronCalc and the
app load fine) — so LibreOffice conversion is simply broken/unavailable in this container (a Java
/ headless-profile issue), not a problem with the authored or round-tripped files. Validity is
instead confirmed by our own loader + IronCalc reopen (both pass) and structural zip inspection.

## Recommendation for the follow-on

**Display + save-preservation is justified.** Load is cheap and reliable; save re-injection is
tractable with the three sharp edges above handled. Recommend the follow-on ship **display + save
byte-preservation** (not display-only), scoped to: single- and multi-sheet mapping via
workbook rels, carry-by-prefix for all chart-aux parts, and a documented stale-on-edit caveat
(with cache-refresh as a later enhancement).

## What's here

- `src/xlsx.rs` — zip read + OPC relationship helpers (resolve `Id→Target`, `../`-relative parts).
- `src/load.rs` — the `worksheet→drawing→chart` walk + chart-XML→`chart-model` parser (cached values).
- `src/save.rs` — IronCalc-writer + byte-preservation re-injection (content-types merge, worksheet
  + `_rels` patch).
- `src/authoring.rs` — programmatic authoring of the valid, IronCalc-loadable 3-chart fixture.
- `src/bin/fixtures.rs` — authors `fixtures/charts_basic.xlsx` + writes `results/roundtrip_charts_basic.xlsx`.
- `src/bin/{render_loaded,capture_loaded}.rs` (feature `render`) — render each loaded chart to
  `results/loaded_*.png` via `chart-render`.
- `fixtures/charts_basic.xlsx`, `results/loaded_*.png`, `results/roundtrip_charts_basic.xlsx`,
  `results/manifest.json`, `results/review.md` — the committed evidence.
