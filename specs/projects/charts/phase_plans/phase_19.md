---
status: complete
---

# Phase 19: Edit panel + range/type

## Overview

P19 lets a **near-empty inserted chart** (P17's placeholder-literal authored chart) be **shaped into a
real one**: set its **data range** so it binds to actual cells (and live-re-renders from them), and
switch its **chart type**. It adds a right-docked **edit-panel skeleton** (chrome overlay, `ui_design §4`)
that drives two new worker commands, and the engine machinery to make range→binding + type-switch
mutate the authored `ChartSpec`, re-resolve live, and round-trip through the write path.

The heart of the phase is the **engine**: setting a range gives an authored chart a real `c:f` binding so
it transitions from P17's *snapshot-but-not-live* placeholder to a **LIVE** chart that re-resolves through
the existing dirty-set / worker-snapshot path, and that binding round-trips through
`write_authored_charts` (now emitting `c:f` refs + caches for a bound authored chart, not just literals).
Type-switch rebuilds the model to the new `ChartInsertKind`, preserving the range binding + title. Both
are mutating ops: degraded-guarded, provenance kept (`Authored`), combined save honest.

Scope: **authored** charts (the insert→shape flow). Loaded-chart chrome editing is P20 (source-patch).
The panel opens for a selected/inserted **authored** chart only.

## Steps

### Model (`freecell-chart-model`)
1. **`authoring.rs`**: add `ChartInsertKind::from_chart_kind(&ChartKind) -> Option<ChartInsertKind>` (the
   inverse of `chart_kind()`: `Bar{Col}`→Column, `Bar{Bar}`→Bar, `Line`→Line, `Area`→Area,
   `Pie{None}`→Pie, `Pie{Some}`→Doughnut, `Scatter`→Scatter). Lets the panel show a chart's current type
   and the worker map a spec back to an insert kind. Unit-tested as a round-trip of `chart_kind()`.

### Engine — range → refs (`freecell-engine::chart::range`, new)
2. New `chart/range.rs`: pure, gpui-free.
   - `series_refs_from_block(sheet_name: &str, block: CellRange) -> Vec<SeriesRefs>` — interpret a
     rectangular data block (0-based inclusive) as chart data: **first row = series-name headers, first
     column = category/x labels, each remaining column a series** (`name`=header cell, `categories`=first
     column data rows, `values`=that column's data rows). A single-row/col/cell block degrades to one
     value series over the whole block (no header/categories). Emits **absolute, sheet-qualified** `c:f`
     (`Name!$A$2:$A$5`), quoting the sheet name when it isn't a bare identifier.
   - Small A1 helpers (`abs_cell`, `abs_col_range`, `qualify`) built on `freecell_core::column_label`.
   - Unit tests: the fixture layout (`Data!A1:D5` → cats `$A$2:$A$5` + 3 series), single-column degrade,
     a quoted sheet name.

### Engine — binding from refs (`freecell-engine::chart::binding`)
3. `binding.rs`: add `binding_from_refs(&[SeriesRefs]) -> ChartBinding` (parse each `SeriesRefs`'
   formula strings via `parse_cf` into a `SeriesBinding`), and extract `BoundChart::is_dirty`'s body into
   a pure `binding_is_dirty(binding, anchor_sheet, edited, rebuilt, resolve_sheet) -> bool` that both the
   loaded path and the authored re-resolve call. `build_series_shells(refs, xy) -> Vec<Series>` (empty
   data shells, one per ref, `CategoryValue` or `Xy`), so `resolve_chart` fills them from live cells.

### Engine — protocol (`freecell-engine::worker::protocol`)
4. Add `Command::SetChartRange { sheet: SheetId, id: ChartId, data: CellRange }` and
   `Command::SetChartType { sheet: SheetId, id: ChartId, kind: ChartInsertKind }` (both engine-free seam
   types). `data` is the block on the chart's host sheet.

### Engine — worker (`freecell-engine::worker::run`)
5. `AuthoredEntry` gains `refs: Vec<SeriesRefs>` (empty on insert; the source of truth for a bound
   authored chart's `c:f` — its `ChartBinding` is derived via `binding_from_refs`, and the write path
   consumes it directly).
6. Route `SetChartRange` / `SetChartType` in the `chart_ops` bucket (exhaustive match) + one-by-one after
   the edit batch, like `SetChartAnchor`/`DeleteChart`.
7. `set_chart_range(sheet, id, data)` (degraded-guarded): find the authored entry by id; build
   `refs = series_refs_from_block(host_name, data)`; build the new `Chart` (keep kind/title/axes/legend,
   replace series with shells in the kind's data shape); resolve values from live cells; store
   `spec.body`, `spec.source_ranges` (= the ref formulas), and `entry.refs`. Bump ops/version, publish.
   Ignored (logged) for a loaded/unknown id (loaded re-range is P20).
8. `set_chart_type(sheet, id, kind)` (degraded-guarded): find the authored entry; if it has a binding
   (`refs` non-empty), rebuild series shells in the new kind's data shape keeping `refs`+title, resolve
   live; else swap to `kind.near_empty_chart()` preserving the old title. Bump ops/version, publish.
9. `reresolve_charts`: also re-resolve **bound authored** charts. Restructure so it no longer early-returns
   on `self.charts.is_empty()` when authored charts have bindings; after the loaded pass, for each authored
   entry with `refs`, build its binding, `binding_is_dirty` → `resolve_chart` from live cells; a change
   bumps the version + re-stores the snapshot.
10. `authored_write_list`: pass `entry.refs.clone()` as the `AuthoredChart.refs` (was `Vec::new()`), so a
    bound authored chart saves with `c:f` + caches (write-from-model mode 3, now cell-bound).

### App — grid selection signal (`freecell-app::grid`)
11. `grid/mod.rs`: add `GridEvent::ChartSelected(ChartId)` — a chart became selected by user interaction.
12. `grid/view.rs`: in `handle_mouse_down`, after `begin_chart_interaction`, emit
    `GridEvent::ChartSelected(id)` (window has focus + `window` in scope there). `set_selected_chart`
    stays non-emitting (programmatic window-driven selection).

### App — edit panel (`freecell-app::chrome::view`)
13. `ChromeView` gains `chart_panel: Option<ChartPanel>` (`{ sheet, id, kind, ranges: Option<String> }`).
    - `open_chart_panel(sheet, id, kind, ranges, cx)` / `close_chart_panel(cx)` / `chart_panel_target()`.
    - `set_chart_type_from_panel(kind, window, cx)` and `apply_chart_range_from_selection(window, cx)` —
      both commit any pending edit + degrade-guard (like `insert_chart`), then send
      `Command::SetChartType` / `Command::SetChartRange { data: self.selection.range() }` for the panel's
      `(sheet,id)`; the type setter optimistically updates `chart_panel.kind`.
    - `render_chart_panel`: a right-docked card (absolute, right, below the data row, above the tab bar):
      a header + close ×, a **Type** row of the `CHART_MENU` glyph buttons (current kind highlighted), a
      **Data range** section showing the current bound ranges + a "Use selection {A1}" apply button.
    - `set_degraded(true)` closes the panel; add it to `render_overlays`.

### App — window wiring (`freecell-app::shell::window`)
14. `WorkbookWindow` gains `known_authored_charts: HashSet<ChartId>`.
    - `make_grid_sink`: `GridEvent::ChartSelected(id)` → resolve the authored spec from the snapshot and
      open the chrome panel + `grid.set_selected_chart(Some(id))` (via a shared
      `resolve_authored_chart_panel(client, id) -> Option<ChartPanelInfo>` helper).
    - `make_grid_sink`: `GridEvent::ChartDeleted { id }` also closes the panel if it targets `id`.
    - `sync_charts`: after installing, drive the panel — auto-open a **newly-appeared authored** chart
      (diff `known_authored_charts`), else refresh the existing panel's kind/range (or close it if its
      chart vanished / became loaded).

## Tests

- **range (pure, engine)**: `series_refs_from_block` — the `Data!A1:D5` fixture layout (cats + 3 series
  with abs-qualified `c:f`); a single-column degrade; a space-containing sheet name is quoted.
- **binding (engine)**: `binding_from_refs` round-trips a `SeriesRefs` set into resolvable `CfRef`s;
  `binding_is_dirty` selects only intersecting edits.
- **model**: `from_chart_kind` inverts `chart_kind()` for every `ChartInsertKind`.
- **worker unit (`run.rs`)**:
  - `set_chart_range_binds_authored_chart` — insert line, put values in cells, `SetChartRange` → the
    published authored spec now carries `source_ranges` + live values from the cells.
  - `edit_reresolves_ranged_authored_chart` — after ranging, editing a source cell bumps the version +
    updates the authored chart's value.
  - `set_chart_type_switches_kind` — insert line, `SetChartType(Column)` → published kind is `Bar{Col}`,
    title preserved.
  - `set_chart_range_and_type_rejected_when_degraded`.
- **worker seam round-trip (`worker_seam.rs`, via `discover_and_parse` — LibreOffice is env-broken)**:
  - `ranged_authored_chart_saves_cf_and_roundtrips` — new workbook, write a data grid, insert line,
    `SetChartRange`, Save → reopen: a Loaded line chart whose `source_ranges` contain the `c:f` and whose
    values match the cells (proves LIVE binding round-trips, not literals).
  - `retyped_authored_chart_roundtrips` — insert line, range it, `SetChartType(Column)`, Save → reopen is
    a `barChart` (col) with the same `c:f` ranges.
- **chrome view (gpui)**: opening the panel; a type-glyph click sends `SetChartType`; the "use selection"
  button sends `SetChartRange` with the current selection; close hides it; degrade closes it.

## Render validation

The edit panel is **chrome with no pixel baseline** (like the action/data rows + tab bar), so the pixel
suite is **out of scope** for it — validated by the chrome gpui view tests + an Xvfb smoke launch. Setting
a range/type re-renders the chart through the **existing** `ChartLayer` (runtime state over unchanged
render code), so no `grid_chart_*` baseline moves; the full suite stays deferred to P21.
