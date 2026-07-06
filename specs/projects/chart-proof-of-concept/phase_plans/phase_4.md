---
status: complete
---

# Phase 4: Gate 4 — load/save stitching

## Overview

Gates 1–3 proved the render side (multi-series line, bar family, stacked area, pie/doughnut,
scatter) on gpui-component's primitives, rendering *from* the `chart-model`. Gate 4
(functional_spec §5, §7, §10 #2/#4) closes the seam on the **file** side: a new **`load-save`
crate** that parses chart definitions *out of* a real `.xlsx` **into** `chart-model` (proving
it by rendering the loaded charts through `chart-render`), and re-injects those chart parts on
**save** so a chart survives IronCalc's chart-dropping writer.

Two questions this phase answers (for the follow-on scope call):
1. **Load:** is following the `worksheet → drawing → chart` relationship chain and reading the
   cached `numCache`/`strCache` "as quick as hoped"? (Load FAIL is serious — no chart data
   without it.)
2. **Save:** is **byte-preservation re-injection** (§10 #2 — the accepted bar) tractable or a
   swamp? → sets **display+save-preservation vs display-only** for the follow-on. (Save FAIL is
   an acceptable PoC outcome: display-only recommendation.)

The `load-save` **model/parse layer is gpui-free and ironcalc-free** (mirrors `open_fixups.rs`:
`zip 0.6` + `roxmltree 0.19`, into `chart-model`). The **save** path uses `ironcalc =0.7.1` /
`ironcalc_base =0.7.1` (the app's pins) to run IronCalc's real writer and post-process its zip.
A **render-proof** binary depends on `chart-render` (and thus gpui) behind an optional `render`
feature, so the default test loop stays gpui-free and fast.

## Design notes

### Crate layout (`experiments/chart-poc/load-save/`)
- `src/xlsx.rs` — zip read helper (mirrors `open_fixups::read_zip_entry`) + OPC relationship
  helpers (resolve a `_rels` file's `Id → Target`, resolve a `../`-relative part path).
- `src/load.rs` — `discover(path) -> Vec<SheetDrawing>` (walk `worksheets/sheetN.xml →
  <drawing r:id> → _rels → drawings/drawingM.xml → graphicFrame <c:chart r:id> → _rels →
  charts/chartK.xml`) and `parse_chart_xml(&str) -> Chart` (chart-group element → `ChartKind`;
  `c:ser` → `Series` from `numCache`/`strCache`; title/axis-titles/legend). `load_charts_from_xlsx`
  = discover + read + parse. roxmltree matching is by **local name** (`tag_name().name()`) so it
  is namespace/prefix-agnostic; attributes (`val`, `r:id`, `idx`) read by local name too.
- `src/save.rs` — `reinject(original, ironcalc_zip_bytes) -> Vec<u8>`: copy IronCalc's output
  verbatim except the parts we patch; carry every original `xl/charts/*` + `xl/drawings/*`
  entry byte-for-byte; merge the chart/drawing Overrides into `[Content_Types].xml`; inject
  `<drawing r:id=…/>` into each affected worksheet + a matching `_rels`. `save_with_charts(path,
  out)` = `ironcalc::import::load_from_xlsx` → `ironcalc::export::save_xlsx_to_writer` into a
  `Cursor<Vec<u8>>` → `reinject` → write `out`.
- `src/authoring.rs` — programmatic OOXML authoring (the `open_fixups.rs` `write_crafted_xlsx`
  pattern, scaled): builds a valid, minimal, single-sheet `.xlsx` carrying THREE embedded charts
  (clustered column, multi-series line, pie) with hand-set `numCache`/`strCache` so the loaded
  values are known/asserted. Used by the `fixtures` bin and by tests.
- Bins: `fixtures` (default feature — author fixtures + run the save round-trip, write artifacts
  + a report), `render_loaded` (feature `render` — one gpui window for a loaded chart, mirrors
  `render_scene`), `capture_loaded` (feature `render` — xvfb/lavapipe/xrefresh capture of each
  loaded chart to `results/loaded_*.png`, reusing `chart_render::capture`).

### `chart-render` additive changes (no behavior change to existing scenes)
- `render.rs`: extract `run_render_chart(chart, viewport, exit_after_ms)`; `run_render_scene`
  delegates to it.
- `capture.rs`: extract `capture_window(launch_cmd, viewport, icd, out)` (the xvfb + xrefresh +
  grab-by-size + non-blank assert) from `render_one`, which now calls it; add `sibling_bin(name)`
  (generalizes `sibling_render_scene_bin`). `load-save`'s `capture_loaded` reuses both.

### Save re-injection specifics (byte-preservation, §10 #2)
- Single-sheet fixtures ⇒ IronCalc's `xl/worksheets/sheet1.xml` maps 1:1 to the original's; the
  worksheet patched is matched by identical part name (multi-sheet index→part mapping via
  `workbook.xml.rels` is out of PoC scope, documented).
- The injected drawing relationship uses a distinctive Id (`rIdChartPoc{n}`) chosen to not
  collide with any Id IronCalc already emitted in that sheet's `_rels`.
- Verify the round-trip three ways: (a) our own `discover`/`load_charts_from_xlsx` re-finds the
  charts + same cached values; (b) `ironcalc::import::load_from_xlsx` re-opens the output without
  error (not corrupt); (c) best-effort `soffice --headless` convert as a bonus (not required).

## Steps

1. **Workspace wiring.** Add `load-save` to `Cargo.toml` `members`; add `zip = "0.6"`,
   `roxmltree = "0.19"`, `ironcalc = "=0.7.1"`, `ironcalc_base = "=0.7.1"` to
   `[workspace.dependencies]`.
2. **`load-save/Cargo.toml`.** Default deps: `chart-model`, `zip`, `roxmltree`, `ironcalc`,
   `ironcalc_base`, `anyhow`. Optional `render` feature → `chart-render` + `serde`/`serde_json`;
   `render_loaded`/`capture_loaded` bins `required-features = ["render"]`.
3. **`src/xlsx.rs`.** `read_entry(path,name)`, `read_entry_from<R>(zip,name)`, `Rels` parser
   (`Id → (Type, Target)`), `resolve_part(base_dir, target)`.
4. **`src/load.rs`.** `SheetDrawing`, `discover`, `parse_chart_xml`, `load_charts_from_xlsx`, plus
   the cache readers. Local-name node/attr helpers.
5. **`src/authoring.rs`.** `write_fixture(path)` → the 3-chart workbook; XML builder helpers
   (`str_ref`, `num_ref`, `catval_series`, `axis`, `title`).
6. **`src/save.rs`.** `reinject`, `save_with_charts`, content-types merge, worksheet patch.
7. **`chart-render`** additive refactor (`run_render_chart`, `capture_window`, `sibling_bin`).
8. **Bins.** `fixtures`, `render_loaded`, `capture_loaded`.
9. **Prove it.** Run `fixtures` (author + round-trip artifact + report); run `capture_loaded`
   (loaded charts → `results/loaded_*.png` + `manifest.json`); agent-review the PNGs into
   `results/review.md`; write `findings.md`.

## Tests

- `authoring::fixture_loads_in_ironcalc` — the authored `.xlsx` opens via
  `ironcalc::import::load_from_xlsx` without error (guards the save path's premise).
- `load::discover_finds_all_three_charts` — `discover` returns the sheet→drawing→chart chain for
  all three fixture charts (correct chart part paths).
- `load::parses_column_line_pie_kinds_and_values` — the three parsed `Chart`s have the expected
  `ChartKind` (clustered `Bar`, `Line`, `Pie`), series names, categories, and **cached values**
  matching what was authored.
- `load::parse_chart_xml_reads_axes_title_legend` — title, both axis titles, and legend presence
  parse from a small inline chart XML.
- `save::roundtrip_preserves_charts` — `save_with_charts` output, reopened with `discover` +
  `load_charts_from_xlsx`, still contains the charts with the same cached values; the worksheet
  XML has a `<drawing>` ref; `[Content_Types].xml` declares the chart parts.
- `save::roundtrip_reopens_in_ironcalc` — the re-injected output loads again via IronCalc without
  error (not corrupted).
- Light per relaxed rigor. The **loaded-chart PNGs**, the **round-tripped `.xlsx`**, and
  `findings.md` are the real Gate-4 evidence.
