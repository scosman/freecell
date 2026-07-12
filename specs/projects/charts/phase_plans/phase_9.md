---
status: complete
---

# Phase 9: Live binding

## Overview

Charts currently render **static** cached values: P8 has the app call `discover_and_parse` on a
background executor and install a resident `Vec<RenderedChart>` on the grid, whose values never
change. P9 makes charts **live**: a chart's series values track the *current* worksheet cells its
`c:f` ranges resolve to, so editing a source cell re-renders the chart — and **only** charts whose
source ranges intersect the edit recompute (architecture §4.1 / §5 challenge 2, functional_spec §2).

The engine (the worker) is the only place with **all** cell values (the `Publication` covers just
the active sheet's viewport, so the app can't resolve arbitrary source ranges). So live binding is
worker-owned:

1. The worker **owns the charts** (discovers + parses them on load from the opened file's path).
2. Each `c:f` string is parsed **once** into a structured range (sheet-by-name + rect), captured
   **per series per role** (name / category-or-x / value-or-y) — the range→chart index.
3. On each edit batch, the worker intersects the **edited-cell set** (the same `(sheet, range)`
   set it already computes for the style-cache mirror, including undo/redo re-reads) against the
   index to get the **dirty chart set** — no full rescan.
4. Only dirty charts are **re-resolved** from IronCalc's current values (`get_cell_value_by_index`)
   and their `Chart` rebuilt; the file's `numCache` stays the **first-paint** value and the
   **fallback** when a range can't be resolved.
5. Resolved charts ride the **existing worker publication seam**: a lock-free `ArcSwap<ChartSnapshot>`
   in `Shared`, stored **before** the existing `WorkerEvent::Published` is emitted (so the edit's
   one `Published` carries both fresh cells and fresh charts — no bespoke channel, no extra event).
   A `version` on the snapshot is bumped only when charts actually change, so a scroll-only publish —
   or a disjoint edit — leaves the app's installed charts untouched.

The app becomes a pure consumer: on `Loaded` / `Published` it reads the chart snapshot and (when the
version changed) installs it via the existing `GridView::set_sheet_charts`. P8's app-side
`discover_and_parse` + static install is removed.

**Scope boundaries (per the plan + task):**
- **SAVE is P10** — not touched here.
- **Multi-sheet chart→SheetId placement stays deferred (P10).** `discover_and_parse` returns a flat
  `Vec<ChartSpec>` with no worksheet association; correlating each chart's *anchor* worksheet to its
  `SheetId` needs the `workbook.xml.rels` part map that P10 builds. So all discovered charts are
  anchored to the **first sheet** (matching P8's active-sheet install). Note: a chart's *data* sheet
  is independent of its anchor sheet and IS resolved correctly — by **name** (`Data!…`) against the
  model — so cross-sheet data references bind live even with single-sheet anchoring.
- **Changed-cell set = the edited-cell set** (direct edits + undo/redo re-reads), which is what the
  worker already tracks. IronCalc 0.7.1 exposes current cell values (`get_cell_value_by_index`) — so
  re-resolving ranges is fully supported (no roadblock) — but it does **not** surface a recompute's
  transitive dependent-cell delta. So the one uncovered case is: editing a cell **outside** a chart's
  ranges that a formula **inside** a range depends on (the inside cell's value changes but wasn't
  directly edited). This is outside P9's exit criteria (direct source-cell edits) and is noted as a
  future hardening item, not a workaround.
- **Structural edits (insert/delete rows/cols):** the retained `c:f` isn't reflowed until save (P10),
  so re-resolving after a structural edit is best-effort (may read shifted cells); it never crashes.
  A dirty chart on a structurally-rebuilt data sheet is re-resolved conservatively.

**Render scope:** P9 re-resolves values and re-uses the **same** line/chart widgets — it does not
change any chart render code, and no render scene installs charts over the grid (the `chart_*`
scenes are standalone from fixtures). So **no pixel baseline moves**; the pixel suite is not run
(CLAUDE.md render scope). Validation is headless engine + worker-seam + gpui view tests.

**Deferred efficiency (P11):** `ChartBindings::specs_by_sheet` deep-clones every chart's full
`ChartSpec` (including its retained source XML) into the snapshot on each intersecting edit — fine at
the line-slice's chart counts / XML sizes, but if either grows, wrap the specs in `Arc` (or split the
render `Chart` from the heavy source) so a re-resolve clones cheap. Flagged with a `// P11:` comment
at the call site; off the P9 critical path.

## Steps

### Engine — pure c:f binding (`crates/freecell-engine/src/chart/binding.rs`, NEW)

1. `CellData` — engine-free resolved cell value enum: `Number(f64) | Text(String) | Bool(bool) |
   Empty`. The bridge type `WorkbookDocument::cell_value` returns and the resolver consumes (no
   IronCalc type escapes).
2. `CfArea { sheet: Option<String>, range: CellRange }` and `CfRef { areas: Vec<CfArea> }`.
   `parse_cf(&str) -> Option<CfRef>`: trims, strips one layer of surrounding parens, splits into
   areas on **top-level** commas (quote-aware), and for each area parses an optional
   `SheetName!` / `'Quoted Name'!` prefix then `CellRange::from_a1(rest)` (which already ignores
   `$` absolute markers). Returns `None` if no area parses; skips areas that don't.
3. `SeriesBinding { name: Option<CfRef>, cat: Option<CfRef>, val: Option<CfRef> }` and
   `ChartBinding { series: Vec<SeriesBinding> }`. `parse_chart_binding(chart_xml) -> ChartBinding`:
   parallels `parse_chart_xml`'s series walk (same doc order, so `binding.series[i]` ↔
   `chart.series[i]`), reading each series' `c:tx`→name, `c:cat`|`c:xVal`→cat, `c:val`|`c:yVal`→val
   `<c:f>` text → `parse_cf`.
4. Resolution helpers taking closures (so they unit-test with fakes, no worker/model):
   - `resolve_sheet: impl Fn(&str) -> Option<SheetId>` (name→id, unqualified → a supplied
     `default_sheet`).
   - `read_cell: impl Fn(SheetId, CellRef) -> CellData`.
   - `resolve_numbers(&CfRef, …) -> Vec<f64>` (non-numeric/empty → `f64::NAN` so positions align and
     the P5 renderer blanks them; unresolvable sheet → the area contributes nothing);
     `resolve_categories(&CfRef, …) -> Vec<Category>`; `resolve_name(&CfRef, …) -> Option<String>`.
   - `resolve_series(template: &Series, &SeriesBinding, …) -> Series`: rebuild values (+ categories/x,
     name) from live cells, **preserving** the template's `color`/`marker` and its `SeriesData`
     variant (CategoryValue vs Xy — Xy reads cat-ref + val-ref both as numbers). Roles with no ref, or
     whose ranges don't resolve, keep the template's data (cache fallback).
   - `resolve_chart(template: &Chart, &ChartBinding, …) -> Chart`: rebuild each series; keep
     kind/axes/legend/title.
5. `ranges_intersect(a: &CellRange, b: &CellRange) -> bool` (rectangle overlap).

### Engine — the bound-chart set (`crates/freecell-engine/src/chart/binding.rs`, same file)

6. `ChartBindings` — the worker-held set: `Vec<BoundChart>` where `BoundChart { anchor_sheet: SheetId,
   spec: ChartSpec, binding: ChartBinding }`.
   - `from_specs(Vec<ChartSpec>, anchor_sheet: SheetId) -> Self`: build each chart's `ChartBinding`
     from `spec.source().chart_xml` (authored/sourceless charts → empty binding). Keeps the specs'
     **cached** `chart` values as first-paint (no model read on load).
   - `dirty_indices(edited: &[(SheetId, CellRange)], rebuilt_sheets: &[SheetId], resolve_sheet) ->
     Vec<usize>`: a chart is dirty iff any series' any ref-area (resolved sheet) intersects an edited
     range on that sheet, or resolves to a sheet in `rebuilt_sheets`.
   - `reresolve(indices, resolve_sheet, read_cell) -> bool`: rebuild `spec.chart` for each dirty
     chart; return whether any chart's `chart` actually changed.
   - `specs_by_sheet(&self) -> Vec<(SheetId, Vec<ChartSpec>)>` (grouped by `anchor_sheet`).
   - `is_empty()`.

### Engine — document cell read (`crates/freecell-engine/src/document.rs`)

7. `pub(crate) fn cell_value(&self, sheet_idx: u32, cell: CellRef) -> CellData` — maps
   `self.model.get_model().get_cell_value_by_index(sheet_idx, row, col)`'s `CellValue`
   (None/String/Number/Boolean) to `CellData`, via the existing `to_engine_coords`. `record_engine_call()`.

### Engine — the publication seam (`crates/freecell-engine/src/worker/`)

8. `ChartSnapshot { version: u64, sheets: Vec<(SheetId, Vec<ChartSpec>)> }` + `empty()` (version 0) —
   new module `worker/charts.rs`, re-exported from `worker/mod.rs` and `lib.rs`. The app-facing seam
   type (chart-model + core types only; lives in engine because core doesn't depend on chart-model).
9. `client.rs` `Shared`: add `chart_snapshot: Arc<ArcSwap<ChartSnapshot>>` (init `empty()`);
   `DocumentClient::chart_snapshot(&self) -> Arc<ChartSnapshot>` (wait-free `load_full`).
10. `run.rs` `Worker`: add `charts: ChartBindings` (init empty) + `chart_version: u64` (init 0).
    - `load_and_run`: after `Loaded`/`StyleCacheUpdated`, if `source` is `OpenFile(path)`,
      `discover_and_parse(path)` (per-chart non-fatal — already), `ChartBindings::from_specs(specs,
      first_sheet)`, and if non-empty bump `chart_version` to 1 + store the snapshot. Off the first
      cell paint's blocking path is a P11 concern (noted); correctness-first here.
    - Split `mirror_applied_ops` into `collect_edited_ranges(&applied_ops) -> (Vec<(SheetId,
      CellRange)>, Vec<SheetId>)` (the undo/redo touch-stack pops + refresh/rebuild build — the state
      mutation) and `apply_cache_refresh(refresh, rebuild, sheets_before)` (retain + refresh_cache_cells
      + rebuild + `StyleCacheUpdated` emits). Behaviour unchanged; just reordered so the edited set is
      available before publish.
    - In `apply_edit_batch`, new order: apply+eval → `ops_seen` → `collect_edited_ranges` →
      `reresolve_charts(&refresh, &rebuild)` (dirty-set + reresolve; on change bump `chart_version` +
      store snapshot) → `publish()` (cells) → `emit(Published)` → `apply_cache_refresh(...)` →
      sheets-changed check. Event order (Published before StyleCacheUpdated) is preserved.
    - `reresolve_charts`: build the two closures over `self.doc` (`resolve_sheet` via
      `sheet_properties()`, `read_cell` via `resolve()` + `cell_value`), call
      `charts.dirty_indices` then `charts.reresolve`; if changed, `self.chart_version += 1` and store
      `ChartSnapshot { version, sheets: charts.specs_by_sheet() }`.
    - `store_chart_snapshot()` helper.

### App — consume the seam (`crates/freecell-app/src/shell/window.rs`)

11. Remove `load_charts` (background `discover_and_parse` + `set_sheet_charts`) and its call in the
    `Loaded` arm.
12. Add window fields `installed_chart_version: u64` (init 0) + `installed_chart_sheets: Vec<SheetId>`.
    Add `fn sync_charts(&mut self, cx)`: read `self.client.chart_snapshot()`; if `version ==
    installed_chart_version` return; else clear sheets no longer present (`set_sheet_charts(sheet,
    vec![])`), install each `(sheet, specs)`, update `installed_chart_sheets` + `installed_chart_version`.
    Call it from the `Loaded` and `Published` arms.

## Tests

### `crates/freecell-engine/src/chart/binding.rs` (unit)
- `parse_cf_single_absolute_ref` — `Data!$B$2:$B$5` → sheet `Data`, range B2:B5.
- `parse_cf_multi_area_union` — `(Data!$B$2:$B$5,Data!$D$2:$D$5)` and the un-parenthesized
  comma form → two areas with the right rects.
- `parse_cf_unqualified_and_quoted_sheet` — `$A$1:$A$3` → `sheet: None`; `'My Data'!$A$1:$A$3` →
  sheet `My Data`; single-cell `Data!$B$1` → 1×1 range.
- `parse_cf_rejects_junk` — empty / `!A1` / `Data!` → `None` (or empty), never panics.
- `parse_chart_binding_maps_roles_in_series_order` — the line fixture's chart XML → 2 series, each
  with name(`$B$1`/`$C$1`), cat(`$A$2:$A$5`), val(`$B$2:$B$5`/`$C$2:$C$5`).
- `resolve_chart_reflects_live_values` — a fake `read_cell` returning edited numbers rebuilds the
  `Chart`'s series `values` (and a non-numeric cell → blanked `NaN`), preserving color/kind.
- `resolve_falls_back_to_cache_when_sheet_unresolvable` — `resolve_sheet` → `None` leaves the
  template values unchanged.
- `dirty_indices_selects_only_intersecting_charts` — two charts on disjoint ranges; an edit hitting
  one returns only its index; a disjoint edit returns `[]`.

### `crates/freecell-engine/tests/worker_seam.rs` (integration — via the public seam)
- `opened_line_chart_is_published_on_the_seam` — spawn over `write_line_fixture`; after the first
  viewport `Published`, `client.chart_snapshot()` carries the line chart with its cached values on
  the first sheet, version ≥ 1.
- `editing_a_source_cell_reresolves_the_chart` — edit a `Data!$B$…` value cell; poll the chart
  snapshot until the corresponding series value updates; version bumped.
- `disjoint_edit_does_not_recompute_charts` — edit a cell outside every chart range; the chart
  snapshot `version` is unchanged (only intersecting charts recompute).

### `crates/freecell-app/src/shell/app.rs` (gpui view test)
- `charts_install_from_seam_and_are_version_gated` — over a detached loaded window, publish a v1
  `ChartSnapshot` (via `DocumentClient::set_chart_snapshot`) + inject `Published` → the chart installs;
  a **same-version** snapshot with no charts injected next is a **no-op** (charts stay — the version
  gate); a **v2** snapshot dropping the sheet clears its charts (the dropped-sheet branch). Directly
  guards the version-gated install / "only intersecting charts recompute" behavior.

## Automated checks
`cargo fmt`, `cargo clippy --workspace -D warnings`, `cargo build`, `cargo test --workspace`, and
`cargo doc -p freecell-engine --no-deps` / `-p freecell-app --no-deps` with `-D warnings` (chart
modules doc-clean; only the 4 known `shell/*` app warnings acceptable). No pixel suite (out of scope).
