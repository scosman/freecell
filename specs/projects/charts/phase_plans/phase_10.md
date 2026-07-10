---
status: complete
---

# Phase 10: Save / restore (source-first)

## Overview

Production chart save (charts/architecture §4.1, §5 challenge 3, §6; functional_spec §3). The PoC
already **byte-preserves** an unedited chart on save (`chart::save::reinject` /
`save_with_charts`). This phase adds the two remaining source-first write modes the plan calls
for **at the engine layer**, proven headless:

1. **Unedited (loaded, not edited)** → BYTE-PRESERVE the retained source XML + related parts
   exactly (already PoC-proven; kept bit-stable).
2. **Edited-loaded** (a source cell feeding the chart changed) → **PATCH the retained source**:
   reflow the `numCache`/`strCache` cached values to the chart's current values, **keeping `c:f`
   and ALL unmodeled styling** — the fidelity win over a lossy `parse → model → regenerate`. Same
   targeted-XML second-pass pattern as `open_fixups.rs` / the existing `reinject`.

Plus the two multi-sheet items P9 explicitly deferred to P10, both **enabled by the
`xl/_rels/workbook.xml.rels` part map** this phase builds:
- **Multi-sheet save part map** — map each original chart-bearing worksheet to IronCalc's
  regenerated worksheet **by sheet name** (not by identical part name), so a multi-sheet workbook
  re-injects each `<drawing>` into the right output worksheet; **fail loudly** (error, no silent
  chart drop) on a missing target part (architecture §6).
- **Multi-sheet chart→SheetId anchor placement** — group discovered charts by their **owning
  worksheet** so the worker anchors each chart to the correct `SheetId` (P8/P9 anchored all charts
  to the first sheet for lack of this map).

The authored / synthesize-from-template write mode is the **authoring phase (P16; renumbered from P22)**, explicitly
**out of scope** here. Rendering is untouched (engine-only phase; **no pixel suite** — the P10
change moves no grid/cell/sheet/titlebar/chart-render baseline).

**External round-trip is a human checkpoint.** The exit says "keeps the line chart in Excel +
LibreOffice"; we cannot run either in the container. We prove the round-trip **headlessly** via
our own `discover_and_parse`: edited values survive, untouched charts are bit-identical, the file
is a valid OPC zip IronCalc reopens, and content-types / drawing-rels are correct. The actual
Excel/LibreOffice visual round-trip stays a deferred human-checkpoint item.

**App-save wiring (§E) — the worker's `Command::Save` / Save-As preserves charts.** So the running
app actually meets the exit criterion (the post-P11 human checkpoint reviews save/restore in Excel +
LibreOffice), the worker routes save through the engine machinery: it retains the chart-source file
path, serializes the current model to a chart-less body, and re-injects the file's charts (patching
edited ones) via a new engine convenience `reinject_live_charts`. Each bound chart **self-describes**
its `xl/charts/chartN.xml` part (threaded through discovery into `BoundChart`) and its host worksheet,
so the save never guesses an association from XML bytes or list position:

- **Patch keying is by the chart's own part**, not by matching source XML across charts — two
  byte-identical parts bound to different sheets (unqualified `c:f` resolving against different
  anchor sheets) carry different live values, so XML-matching would mis-patch the second with the
  first's values. Keying by part writes each part's own values.
- **The `<drawing>` re-inject target is resolved through each chart's stable anchor `SheetId` →
  CURRENT worksheet name** (rename-safe), NOT original-vs-model name equality. A **rename** still
  saves, the chart following onto the renamed worksheet; a **deleted** host sheet drops that chart's
  re-injection gracefully (logged, save succeeds); fail-loud is reserved for the genuine "a sheet
  that still exists in the model has no output part" corruption case.
- **No silent chart drop (architecture §6).** A drawing whose charts were ALL unparseable at load
  (surface/radar/`cx:` — never bound) is **best-effort byte-preserved** onto its host worksheet when
  that sheet **still exists** (matched by the drawing's original sheet name → the model's current
  worksheet). Only a drawing whose host sheet is genuinely gone (deleted, or renamed with no bound
  chart to follow) drops — a logged best-effort drop — and when a drawing IS dropped its whole part
  chain (drawing + charts + `_rels`/aux) is excluded from the carry **and** the `[Content_Types]`
  overrides, so no orphaned parts leak into the output.

Reinject `anyhow` errors map to `SaveError`; temp-file+atomic-rename and the Save-As path are
preserved; a workbook with **no charts** saves through the unchanged plain writer. The actual
Excel/LibreOffice visual round-trip remains the human-checkpoint item.

**Known limitation (tracked in `PROJECTS.md`).** Source-first save keeps each `c:f` byte-for-byte, so
after an in-session data-sheet rename a preserved/patched chart's internal `c:f` still names the old
sheet — cached values are correct (chart shows right data on reopen) but the live data link dangles
in Excel/LibreOffice. Filed as `projects/chart-cf-rewrite-on-rename.md` (status: Future).

## Steps

### A. Edited-loaded patcher — `patch_chart_source` (`chart/save.rs`)

1. **`pub fn patch_chart_source(chart_xml: &str, chart: &Chart) -> Result<String>`** — reflow a
   retained chart part's cached values to `chart`'s current series values, splicing **only** the
   `numCache`/`strCache` elements and leaving everything else (the `c:f`, `c:spPr`/styling, axes,
   legend, layout, prefixes, XML declaration) byte-for-byte:
   - Parse with `roxmltree`; walk to the first chart-group (`load::CHART_GROUP_TAGS`, the same
     first-group rule `parse_chart_xml` / `parse_chart_binding` use), then its `<c:ser>` children
     in document order, aligned 1:1 with `chart.series[i]`.
   - Per series, per **role holder present**, locate the role's cache element (first `numCache` /
     `strCache` descendant of the holder) and compute a replacement:
     - value role — `c:val` (`CategoryValue`) / `c:yVal` (`Xy`) → **numCache** ← `values` / `y`.
     - category role — `c:cat` (`CategoryValue`) / `c:xVal` (`Xy`) → preserve the existing cache
       tag: **strCache** ← `Category::label()` per point; **numCache** ← numeric categories / `x`.
     - name role — `c:tx` → single-point cache ← `series.name` (only when the holder cache is a
       `strCache` and `name` is `Some`; else left verbatim).
   - Replacement preserves the element's **namespace prefix** (read from the source at
     `node.range()`), preserves the `<c:formatCode>` child text if present, and rebuilds
     `<c:ptCount val="N"/>` + `<c:pt idx="i"><c:v>V</c:v></c:pt>` points. **`NaN` values are
     omitted** (Excel's sparse-blank shape); `ptCount` stays the full length. Numbers format
     whole→integer else default; strings are XML-escaped.
   - Collect `(byte_range, replacement)` per cache; apply by `String::replace_range` in
     **descending start order** (disjoint cache ranges, so earlier offsets stay valid); return the
     patched string. A chart with fewer `<c:ser>` than `chart.series` patches the overlap and
     leaves extra series verbatim.
   - Helpers: `element_prefix(src, node) -> &str`, `rebuild_num_cache`, `rebuild_str_cache`,
     `fmt_cache_num`, `escape_xml`, `role_cache_range(...)`.

### B. Multi-sheet part map + patch injection — `reinject` (`chart/save.rs`, `chart/xlsx.rs`)

2. **`xlsx::workbook_sheet_parts(archive) -> Result<Vec<(String /*name*/, String /*part*/)>>`** —
   read `xl/workbook.xml` (`<sheet name r:id>` in order) + `xl/_rels/workbook.xml.rels`
   (`rId → target`), resolving each target against `xl/workbook.xml` into an absolute part name.
   The sheet↔part correspondence both the save mapping and the grouped discovery need.

3. **`reinject` signature** gains `patches: &BTreeMap<String /*chart part*/, String /*patched xml*/>`:
   - Map each original `SheetDrawing.sheet_part` → **sheet name** (via the original's
     `workbook_sheet_parts`), then → the **IronCalc output worksheet part** for that name (via the
     output's `workbook_sheet_parts`). A name with no output part → **`Err`** (fail loudly, no
     silent drop). `SheetPatch.sheet_part` now holds the **output** part; `drawing_target` is
     `relative_part(output_part, drawing_part)`; the report's `patched_sheets` are output parts.
   - In the carry loop, a carry part whose name is a key in `patches` is written **patched**
     instead of verbatim (byte-preserved otherwise). Extend `SaveReport` with
     `patched_charts: Vec<String>`.
   - `save_with_charts(original, out)` keeps its byte-preserve behavior by calling
     `reinject(..., &BTreeMap::new())` (unedited path unchanged; the PoC round-trip test still
     holds — single sheet "Data" → output `sheet1.xml`).

### C. Multi-sheet anchor→SheetId (deferred from P8/P9)

4. **`load::discover_and_parse_by_sheet(path) -> Result<Vec<(String /*sheet name*/, Vec<ChartSpec>)>>`**
   — like `discover_and_parse` but groups the parsed specs by their **owning worksheet's name**
   (resolved via `xlsx::workbook_sheet_parts`), skipping unparseable charts per-chart (unchanged
   resilience). Keeps `discover_and_parse` for the flat callers.

5. **`binding::ChartBindings::from_specs_by_sheet(groups: Vec<(SheetId, Vec<ChartSpec>)>) -> Self`**
   — anchor each group's specs to its own `SheetId` (the single-sheet `from_specs` is kept, now a
   one-group case).

6. **Worker open path (`worker/run.rs`)** — replace `discover_and_parse` +
   `from_specs(specs, active_sheet)` with `discover_and_parse_by_sheet` + a `name → SheetId` map
   (from `doc.sheet_properties()`), building `from_specs_by_sheet`; an unresolved name falls back
   to `active_sheet`. Single-sheet fixtures are unchanged (their one "Data" group maps to the
   first sheet = the active sheet).

### D. Fixtures (`chart/authoring.rs`)

7. **`write_two_sheet_fixture(path)`** — a valid, IronCalc-openable **two-worksheet** workbook
   ("Data" + "Summary"), each sheet carrying its own grid, `<drawing>`, and one embedded chart
   (a column chart on "Data", a line chart on "Summary"), reusing the existing chart/axis/series
   helpers. Exercises multi-sheet discovery, the save part map, and anchor grouping. Exposes the
   sheet names + which chart part is anchored on which sheet as consts.

### E. App-save wiring — the worker's `Command::Save` preserves charts

8. **Thread each chart's part into the binding** — `discover_and_parse_by_sheet` now yields
   `(sheet_name, [(chart_part, spec)…])` (`load::ChartsBySheet`); `BoundChart` gains `chart_part`;
   `from_specs_by_sheet` takes the paired form. `ChartBindings::live_charts(resolve_name)` produces
   one `save::LiveChart { sheet_name: resolve_name(anchor_sheet), chart_part, chart }` per bound
   chart — the host name resolved live (`None` = deleted).
9. **`save::reinject_live_charts(original, model_bytes, live: &[LiveChart])`** — `discover(original)`
   for the drawing structure; `build_live_patches` keys patches by each live chart's **own part**
   (patch when its values differ from that part's file cache); `live_sheet_targets` maps each
   drawing to `Some(output part)` via its live chart's **current** host name (rename-safe), `None`
   to drop (deleted host / unparseable-only), and hard-errors a name that exists live but has no
   output part. `reinject(original, model_bytes, sheets, targets, patches)` takes the resolved
   `targets: &[Option<String>]` (planning moved out of `reinject` so the byte-preserve path
   [`save_with_charts`] can map by name and the live path by SheetId→name).
10. **`document.rs`** — `WorkbookDocument::to_xlsx_bytes()` (serialize the current model to an
    in-memory chart-less `.xlsx`) + a shared atomic-write helper (`write_xlsx_bytes_atomic`, and
    `new_temp_beside`/`persist_atomically` factored out of the existing `save`, so both save paths
    keep the identical temp-file+fsync+rename contract — `save` behavior unchanged).
11. **`worker/run.rs`** — retain `chart_source_path` (opened path, then the last saved path); route
    `Command::Save` through `save_workbook`: when opened from a file AND charts present, serialize
    the model, build `live_charts` (host names resolved through `sheet_name_of(anchor_sheet)`),
    `reinject_live_charts`, then `write_xlsx_bytes_atomic`; else the plain `doc.save` (non-chart path
    unchanged). Map reinject `anyhow` errors to `SaveError::Serialize`.

## Tests

**Patcher (`patch_chart_source`)**
- `patch_reflows_value_cache_keeping_cf_and_styling` — patch a line chart part with a changed
  value; assert the target `<c:v>` updated, the sibling `<c:f>` unchanged, the `<c:spPr>` fill
  unchanged, the result still parses (`parse_chart_xml`) to the new values, and it is still
  well-formed XML.
- `patch_preserves_bytes_outside_caches` — every byte outside the spliced cache elements is
  identical to the source (diff only within `numCache`/`strCache`).
- `patch_omits_nan_points_as_sparse` — a `NaN` value drops its `<c:pt>` but keeps `ptCount`.
- `patch_reflows_category_str_cache_and_name` — a changed category (text) and series name reflow
  into their `strCache`s; `formatCode` preserved on the value `numCache`.
- `element_prefix_reads_namespace_prefix` — `c:` vs unprefixed.

**Save round-trip (headless, via `discover_and_parse` + IronCalc reopen)**
- `edited_line_chart_roundtrips_reflected_values` — write line fixture → open via
  `WorkbookDocument`, set a source cell (e.g. `B2=999`), reflow the chart via
  `binding::resolve_chart` against the edited model, save the edited model bytes with a patch for
  the line chart → reopen with `discover_and_parse`: the reopened chart's value == 999; the
  reopened **model** cell `B2` == 999; the file reopens in IronCalc; `[Content_Types].xml` +
  worksheet `<drawing>` present.
- `untouched_chart_is_byte_identical_after_edited_save` — in the 3-chart fixture, patch only the
  line chart; assert `chart1.xml` / `chart3.xml` decompressed bytes in the output are **identical**
  to the original part bytes (bit-stable), and the patched chart's are not.
- `patched_chart_xml_is_wellformed_with_cf_and_style_intact` — the reopened patched part still
  contains its `c:f` formulas and `c:spPr` fills, only the cache `<c:v>`s changed.

**Multi-sheet**
- `two_sheet_fixture_loads_in_ironcalc_and_discovers_both_charts` — IronCalc opens it;
  `discover` finds both drawings; `discover_and_parse_by_sheet` groups chart→"Data" and
  chart→"Summary".
- `multi_sheet_save_maps_by_name_and_preserves_both_charts` — `save_with_charts` on the two-sheet
  fixture → reopen: both charts preserved, each `<drawing>` re-injected into the correctly-named
  output worksheet; `charts_preserved == 2`; both output worksheets in `patched_sheets`.
- `from_specs_by_sheet_anchors_each_group_to_its_sheet` — grouped specs anchor to distinct
  `SheetId`s; `specs_by_sheet()` reflects the grouping; `live_charts` carries each chart's part +
  the resolved (current / `None`) host name.

**App-save wiring + rename/delete/wrong-patch (CR fixes)**
- `reinject_live_charts_patches_each_part_with_its_own_values` (engine, **Moderate fix**) — two
  byte-identical chart parts (twin fixture) bound to different live values → each part patched with
  its OWN values (111 / 222), not the first XML match.
- `reinject_live_charts_fails_loudly_on_a_model_sheet_with_no_output_part` (engine, **fail-loud**) —
  a live chart on a sheet name absent from the serialized output → `Err` (genuine corruption, not a
  user rename).
- `reinject_live_charts_drops_a_deleted_host_sheet_without_failing` (engine, **delete**) — a live
  chart with `sheet_name = None` → save succeeds, that chart dropped, the surviving chart kept, and
  the dropped drawing leaves **no orphaned parts / content-type overrides**.
- `reinject_live_charts_carries_an_unbound_drawing_when_its_sheet_survives` (engine, **arch §6**) —
  an unsupported chart alone on a surviving sheet is byte-preserved (best-effort carry), not dropped.
- `save_preserves_an_unsupported_chart_on_a_surviving_sheet` (integration, **arch §6**) — drives the
  real `Command::Save`: edit the supported chart on sheet A, save → the unsupported chart on sheet B
  is byte-identical in the output (no silent drop).
- `save_after_renaming_the_chart_host_sheet` (integration, **Critical**) — open→edit `B2`→rename
  Data→Data2→save→reopen: chart present on the renamed sheet with the edited value (no SaveFailed).
- `save_after_deleting_the_chart_host_sheet_succeeds` (integration, **Critical**) — open two-sheet
  →delete Summary→save→`Saved` (no fail); reopen has only the surviving chart.
- `reinject_live_charts_patches_edited_and_byte_preserves_untouched` (engine) — edit one live
  chart's value → only that chart is patched; the others byte-identical; reopen shows the edit.
- `save_through_worker_preserves_and_patches_charts` (integration, `tests/worker_seam.rs`) — drives
  the real `Command::Save` over `DocumentClient`: open the 3-chart fixture, edit `B2` (feeds the
  column + line charts, not the pie), wait for the live reflow, `Command::Save`, await `Saved`, then
  reopen the saved file: both edited charts read 999, the untouched pie part is byte-identical to
  the original, IronCalc reopens. **This is the closest headless seam to the UI's Save action.**
- `chartless_workbook_save_matches_plain_save` (integration) — a chartless workbook saved through
  the worker has zip-entry contents identical to a direct `WorkbookDocument::save`, and carries no
  `xl/charts/` or `xl/drawings/` parts (non-chart path unregressed).
- `chart_save_atomic_on_failure_leaves_destination_untouched` (integration, **Mild**) — a chart-path
  save whose destination is an existing directory fails atomically: the destination is byte-stable
  and no temp file is littered.

**Regression**
- The existing `roundtrip_preserves_charts` (byte-preserve, 3 charts, `patched_sheets ==
  ["xl/worksheets/sheet1.xml"]`) still passes under the new name-based mapping.
- `save_through_worker_roundtrips` + `save_atomic_on_failure_leaves_destination_untouched` (existing
  integration) still pass — the plain save path + its atomicity are unchanged.
