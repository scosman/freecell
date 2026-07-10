---
status: complete
---

# Phase 7: Chart load — `discover_and_parse` → `ChartSpec`

## Overview

Turn the engine's file layer from producing bare render pictures
([`freecell_chart_model::Chart`]) into producing the full **production envelope**
([`freecell_chart_model::ChartSpec`]) the rest of the pipeline needs. The PoC-seeded
`freecell-engine::chart::load` already walks the OOXML `worksheet → drawing → chart`
relationship chain and maps a `chartN.xml` into `Chart` (title, kind, series, axes,
legend). P7 adds the three envelope fields the P2 model carries but the loader has not yet
populated (architecture §3.2 / §4.1):

- **retained `source`** — the `chartN.xml` text **verbatim** plus the chart's own related
  parts (its `_rels`, `colorsN.xml`/`styleN.xml`, embeddings) as raw bytes, the substrate
  for byte-preserving save (P10) and the derived fidelity accessor (P3);
- **`source_ranges`** — every `<c:f>` data reference, retained as-written (`CfRange`), for
  live binding (P9);
- **`anchor`** — the chart's `xdr:twoCellAnchor` from/to cells (+ EMU offsets), for in-grid
  placement (P8).

The output is `discover_and_parse(path) -> Result<Vec<ChartSpec>>`, each spec
`Origin::Loaded { source }`. **Exit:** headless unit tests parse a *real* line-chart
`.xlsx` (a real OPC zip with the full drawing/chart relationship chain) end-to-end. No UI,
no rendering — this is an engine phase, so the pixel suite is not involved.

### Scope decisions (flagged for the reviewer)

1. **Line-focused; the walk is type-agnostic.** The phase's proof is a line chart, but the
   walk / envelope construction is shared across all types — `discover_and_parse` returns a
   `ChartSpec` for every discovered chart regardless of kind (the existing `parse_chart_xml`
   maps bar/pie/scatter too). Line is what the end-to-end fixture and assertions center on.
2. **Envelope only — no new model-field parsing.** The `Chart` mapping (kind/series/values/
   axes/legend/title/smooth) is the P1-seeded parser, already line-complete. Wiring the
   *P6 fidelity fields* (`c:marker` symbols, axis `c:numFmt`, `schemeClr` theme colors) from
   XML **into** the model is **not** a P7 deliverable — the plan scopes P7 to the walk +
   envelope (source / ranges / anchor / origin), and those fields ride the **retained
   source** until the render-wiring phase (P8) needs them. Keeping P7 tight avoids
   gold-plating a fidelity parse the plan places later.
3. **Fixture via the in-repo builder, not a committed binary.** The brief prefers generating
   the fixture through a builder when the repo has one. `chart::authoring` already writes
   valid OPC `.xlsx` packages IronCalc accepts; P7 adds a dedicated **single line-chart**
   fixture (`write_line_fixture`) that additionally carries a chart `_rels` → `colorsN`/
   `styleN` chain, so related-part retention is exercised. The existing 3-chart fixture is
   reused (untouched) for the multi-chart / document-order walk test.
4. **A bad chart is per-chart non-fatal, not fatal to the load.** Per architecture §6 /
   functional_spec §1 ("workbook open never breaks; all chart errors are per-chart,
   non-fatal"), `discover_and_parse` **skips + logs** a chart it can't read/parse (an
   unsupported group — surface/radar/stock/3-D/bubble — or a malformed part) and continues,
   returning the charts that parsed. **Deferred (documented, not built now):** this phase
   *drops* an unparseable chart entirely; **P8** (placeholder render) + **P14** (cross-type
   graceful-degrade / real-file corpus) upgrade skip → **retain-source + `Unsupported`
   placeholder** so it byte-preserves on save and shows a placeholder. Skip-and-log is
   sufficient for the line-only end-to-end slice while honoring the never-breaks invariant.
   (A code comment at the skip site records the same P8/P14 boundary.)

   **Known remaining whole-load abort path (tracked for P14).** The resilience above covers a
   chart the walk *reaches* but can't parse. It does **not** yet cover a corrupt **drawing
   relationship** in the shared `discover` / `drawing_charts` walk: a missing drawing `_rels`
   part, or a `<c:chart r:id>` whose `rId` is absent from that `_rels`, still `?`-aborts the
   **entire** load (and, since `discover` is shared, the save path too). This is a
   package-corruption case, not an unsupported-chart case; making it per-chart-resilient
   (skip the dangling frame, keep the rest) belongs with the **P14** cross-type robustness /
   real-file-corpus hardening. Recorded here per review; out of scope for the line slice.

## Steps

1. **`freecell-engine::chart::xlsx` — byte reader.** Add
   `read_entry_bytes_from(archive, name) -> Result<Vec<u8>>` (the `Vec<u8>` twin of
   `read_entry_from`), so retained related parts are captured **byte-for-byte** (not
   re-encoded through `String`).

2. **`freecell-engine::chart::load` — anchors on the walk.**
   - New `pub struct DiscoveredChart { pub part: String, pub anchor:
     freecell_chart_model::Anchor }`.
   - Replace `SheetDrawing.chart_parts: Vec<String>` with `charts: Vec<DiscoveredChart>`
     (per-chart part **and** anchor, in document order).
   - Replace `drawing_chart_parts` with `drawing_charts(...) -> Result<Vec<DiscoveredChart>>`:
     for each `<c:chart r:id>` in the drawing (document order), resolve its part via the
     drawing `_rels` **and** parse its enclosing `xdr:*Anchor` into an `Anchor`.
   - Anchor helpers: `enclosing_anchor(chart_node)` (walk ancestors to the
     `twoCellAnchor`/`oneCellAnchor`/`absoluteAnchor`), `parse_anchor(anchor_el)` →
     `Anchor::new(from, to)`, `anchor_cell(cell_el)` reads `<xdr:col>/<xdr:colOff>/<xdr:row>/
     <xdr:rowOff>` (missing `to` defaults to `from`; missing anchor defaults to a zero
     `AnchorCell` — best-effort, never fails the load).
   - Update `load_charts_from_xlsx` and `save::reinject`'s `charts_preserved` count to the
     new `charts` field (mechanical).

3. **`freecell-engine::chart::load` — `discover_and_parse`.**
   ```rust
   pub fn discover_and_parse(path: &Path) -> Result<Vec<ChartSpec>>
   ```
   The per-chart work is a fallible helper `parse_discovered_chart(&mut archive, &dc) ->
   Result<ChartSpec>`: read `chartN.xml` (verbatim), `parse_chart_xml` → `Chart`,
   `parse_cf_ranges(&xml)` → `Vec<CfRange>`, `read_related_parts(&mut archive, part)` →
   `Vec<SourcePart>`, then `ChartSpec::loaded(chart, SourceXml::new(xml)
   .with_related_parts(parts), ranges, anchor)`. The walk calls it per chart and is
   **per-chart non-fatal** (scope decision 4): `Ok` → push; `Err` → `tracing::warn!`
   skip-and-continue (never aborts the load).
   - `parse_cf_ranges(xml)` — every `<c:f>` whose parent is a `*Ref`
     (`numRef`/`strRef`/`multiLvlStrRef`), text trimmed, in document order.
   - `read_related_parts(archive, chart_part)` — if `xl/charts/_rels/chartN.xml.rels`
     exists, retain it (bytes) + every non-external target it references that exists in the
     package (bytes); else empty. Never fails on a missing target (skip); a malformed rels is
     an error surfaced through the per-chart skip (that chart is dropped, the load continues).

4. **`freecell-engine::chart::mod`** — re-export `discover_and_parse` and `DiscoveredChart`.

5. **`freecell-engine::chart::authoring` — line fixture.** Add
   `write_line_fixture(path) -> Result<()>`: a single-sheet workbook with **one straight
   (non-smooth) two-series line chart**, a drawing with one `twoCellAnchor` at a distinctive
   from/to (with non-zero EMU offsets), and a `xl/charts/_rels/chart1.xml.rels` →
   `xl/charts/colors1.xml` + `xl/charts/style1.xml` aux chain. Reuse the existing package
   boilerplate + chart helpers; add `line_content_types` (declares the chart/colors/style
   overrides), `line_drawing`, `line_drawing_rels`, `line_fixture_chart`, `line_chart_rels`,
   `chart_colors`, `chart_style`. Expose `pub const LINE_ANCHOR: Anchor` and
   `pub const LINE_CHART_PART: &str` for the assertions.

6. **`freecell-engine::chart::authoring` — mixed (line + unsupported) fixture.** Add
   `write_line_plus_unsupported_fixture(path) -> Result<()>`: a single-sheet workbook with
   **two** charts — a parseable line (`chart1`) and a `c:surfaceChart` our `parse_chart_xml`
   does not recognize (`chart2`) — so the per-chart non-fatal skip is testable end-to-end.

7. **Green the workspace.** `cargo fmt --all`; iterate `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace`, and
   `cargo doc -p freecell-engine --no-deps` (with `-D warnings`) until clean.

## Tests

- **`load::discover_and_parse_reads_line_fixture_end_to_end`** (the exit criterion): build
  the real line `.xlsx` via `write_line_fixture`; `discover_and_parse` yields exactly one
  `ChartSpec`; assert kind `Line`, title, both series' cached values; `anchor == LINE_ANCHOR`
  (incl. EMU offsets); `source_ranges` equals the 6 `<c:f>` refs in document order;
  `origin` is `Loaded` with `source.chart_xml` **byte-identical** to the chart part; related
  parts retained (`chart1.xml.rels`, `colors1.xml`, `style1.xml`, colors bytes carry
  `colorStyle`); `display_fidelity() == Faithful`.
- **`load::discover_and_parse_walks_multiple_charts_in_document_order`**: on the existing
  3-chart fixture, assert three specs with kinds `Bar`, `Line`, `Pie` in order; the **line**
  spec (index 1) carries the drawing's second anchor (per-chart association, not just the
  first) and non-empty `source_ranges`; charts with no `_rels` retain **empty**
  `related_parts`.
- **`load::discover_and_parse_skips_unparseable_charts_without_failing_the_load`**: on the
  mixed (line + `c:surfaceChart`) fixture, `discover_and_parse` **succeeds** (no `Err`),
  returns only the line spec (`Line`, `Faithful`), and the surface chart is skipped — the
  never-breaks invariant (architecture §6).
- **`load::parse_cf_ranges_collects_ref_formulas_in_document_order`** (synthetic XML): a
  line-chart string with `tx`/`cat`/`val` refs → the expected ordered `CfRange` list; a bare
  `<c:f>` not under a `*Ref` is ignored; a whitespace ref is skipped; a `multiLvlStrRef` ref
  is collected (locks the `*Ref` handling the plan names).
- **`load::anchor_parsing_reads_two_cell_anchor`** (synthetic drawing XML): `parse_anchor`
  over a `twoCellAnchor` reads from/to col/row + EMU offsets; a `oneCellAnchor` (no `to`)
  falls back `to = from`.
- **`authoring::line_fixture_loads_in_ironcalc`**: IronCalc opens `write_line_fixture`'s
  output (proves it is a valid workbook, not just loader-parseable).
- Existing engine chart tests (load/xlsx/save/authoring) stay green after the `SheetDrawing`
  field rename.

## Render validation

**Out of scope (no pixel suite run).** This is an engine/no-UI phase — nothing is rendered
and no grid/cell/sheet/titlebar baseline can move (CLAUDE.md render-test scope). Validation
is the headless unit tests + the standard `fmt`/`clippy`/`build`/`test`/`doc` gate. In-grid
chart rendering and its render-test coverage begin at P8.
