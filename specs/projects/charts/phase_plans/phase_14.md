---
status: complete
---

# Phase 14: Robustness on real files (line + graceful degrade)

## Overview

P14 closes the **robustness gap** carried since P7/P8/P10: a chart the load walk reaches but
cannot parse into a typed `Chart` (surface/radar/stock/ofPie/bubble/malformed) is currently
**dropped** — never placeholder-rendered, and only best-effort byte-carried on save. It also
hardens the shared `discover` walk so a **dangling drawing/relationship never aborts the whole
load**. Finally it assembles a committed **real-file + generated corpus** and a headless
robustness test proving the whole corpus opens without breakage.

Exit (implementation_plan P14): *corpus green; workbook open never breaks.*

Three behavior changes + one test asset:

1. **Retain unparseable charts as Unsupported specs.** Model a chart with no typed render
   picture as `ChartBody::Unsupported { title }` inside `ChartSpec`, so the load walk keeps its
   source XML + anchor + ranges, `display_fidelity()` is forced to `Unsupported` (→ P8
   placeholder), P10 byte-preserves it as a bound spec, and P9 treats it as static (no live
   values to re-resolve).
2. **3-D → 2-D at parse time.** A `bar3DChart`/`line3DChart`/`pie3DChart`/`area3DChart` now
   parses into its 2-D `ChartKind` (via the existing `normalize_3d_chart_group`), so it
   classifies **Degraded** and renders as its 2-D equivalent + badge (functional_spec §5), rather
   than being dropped. This moves 3-D out of the "can't parse → Unsupported" set into
   "parses-as-2-D → Degraded".
3. **Per-chart-resilient walk.** `discover` no longer `?`-aborts the whole load on a broken
   drawing/rel: a missing drawing `_rels`, an absent chart `rId`, or a missing drawing part drops
   just that drawing/chart (logged); the rest of the workbook + its other charts still open.
4. **Corpus + robustness test.** Commit the owner's real Excel line workbook; generate fixtures
   for every other type + edge cases; a headless test asserts the whole corpus opens, line
   classifies Faithful, unsupported types are retained (placeholder-able) not dropped, 3-D
   degrades, and edge cases fall back without crashing.

## Model representation decision (deliverable 1)

`ChartSpec.chart: Chart` → `ChartSpec.body: ChartBody`:

```rust
pub enum ChartBody {
    /// A chart parsed into a typed render model (fidelity classified from the source).
    Parsed(Chart),
    /// A chart the walk reached but could not parse into a typed `Chart` — retained (source +
    /// anchor + ranges) for placeholder render + byte-preserving save, but with no render picture.
    /// Carries the salvaged chart title for the placeholder caption.
    Unsupported { title: Option<String> },
}
```

Chosen over `chart: Option<Chart>` + a sibling `title` field because it makes the invariant
"an unsupported chart has no `Chart`, is always placeholdered, and has no live values"
**unrepresentable-if-violated** (the same rationale the existing `Origin` enum uses), and it
carries the placeholder title only where it exists. `display_fidelity()` short-circuits an
`Unsupported` body to `Fidelity::Unsupported` **regardless of source classification** — we have
no picture to draw, so the placeholder is the only honest outcome (architecture §4.2: Unsupported
→ placeholder, which does not use the `Chart` content).

Accessors: `chart() -> Option<&Chart>`, `chart_mut() -> Option<&mut Chart>`, `title() ->
Option<&str>` (Parsed → the chart's title; Unsupported → the salvaged title). Constructors:
`loaded(chart, …)` / `authored(chart, …)` unchanged (both → `Parsed`); new
`loaded_unsupported(title, source, ranges, anchor)` → `Unsupported`.

## Steps

1. **`freecell-chart-model/src/spec.rs`** — add `ChartBody`; change the `chart: Chart` field to
   `body: ChartBody`; keep `loaded`/`authored` (→ `Parsed`), add `loaded_unsupported`; add
   `chart()`/`chart_mut()`/`title()`; make `display_fidelity()` short-circuit `Unsupported` body →
   `Fidelity::Unsupported`, else the existing source/authored classification. Update the tests
   that read `spec.chart` → `spec.chart().unwrap()`; add tests for the new representation.
2. **`freecell-chart-model/src/lib.rs`** — re-export `ChartBody`.
3. **`freecell-engine/src/chart/load.rs`**
   - `parse_discovered_chart`: read the chart XML + related parts + `c:f` ranges **always**; on a
     `parse_chart_xml` **error**, retain the chart as `loaded_unsupported(title, source, ranges,
     anchor)` (title salvaged from the source) instead of returning `Err`. Only a genuinely
     unreadable part (missing chart part / malformed `_rels`) stays a skip+log at the caller.
   - 3-D normalization: add `pub(crate) fn is_chart_group(name)` (in `CHART_GROUP_TAGS` **or**
     `normalize_3d_chart_group(name).is_some()`); use it where the group is found in
     `parse_chart_xml`; normalize the group name (`normalize_3d_chart_group(name).unwrap_or(name)`)
     before `parse_kind`, and give `parse_kind` an explicit `name: &str` param.
   - Per-chart-resilient `discover`: process each worksheet's drawing in a fallible helper; on a
     per-sheet error (missing drawing `_rels` / missing drawing part) log + skip that sheet, keep
     the rest. `drawing_charts` skips an **individual** chart whose `rId` is absent from the
     drawing `_rels` (keeping its siblings), instead of `?`-aborting.
   - Rewrite the P8/P14-deferral doc comments to describe the retention behavior.
   - Update the `.chart`-field test reads → `.chart().unwrap()`; rewrite
     `discover_and_parse_skips_unparseable_charts_without_failing_the_load` to assert the surface
     chart is now **retained** as an Unsupported spec (2 specs, one Faithful line + one Unsupported
     surface with source retained).
4. **`freecell-engine/src/chart/binding.rs`** — `parse_chart_binding` uses `is_chart_group`;
   `live_charts` sets `chart: bc.spec.chart().cloned()`; `reresolve` skips a spec with no
   `chart()` (static) and writes through `chart_mut()`; update the `.chart`-field test reads.
5. **`freecell-engine/src/chart/save.rs`** — `LiveChart.chart: Option<Chart>`;
   `build_live_patches` skips a `None` chart (byte-preserve only, never parse/patch);
   `patch_chart_source` finds the group via `load::is_chart_group`; update `live_on` +
   `.chart`-field test reads.
6. **`freecell-engine/src/chart/authoring.rs`** — generic `write_charts_fixture(path, &[chart
   xml])` (one sheet, one drawing anchoring N charts); per-type chart-body builders (column, bar,
   area, pie, doughnut, scatter, bubble, surface/radar/stock/ofPie, bar3D/line3D/pie3D/area3D);
   edge-case bodies (unresolved `c:f`, empty range, non-numeric cell, groupless/garbage chart);
   `write_dangling_chart_rel_fixture` + `write_missing_drawing_rels_fixture` for the broken-walk
   cases.
7. **`freecell-app/src/chart/in_grid.rs`** — `in_grid_chart_element(chart: Option<&Chart>, title:
   Option<&str>, fidelity)`: Placeholder → `placeholder_element(title)`; Chart/ChartWithBadge →
   `chart.and_then(chart_element)` (else placeholder). Behavior-identical for a Parsed spec.
8. **`freecell-app/src/grid/view.rs`** — call site → `in_grid_chart_element(spec.chart(),
   spec.title(), fidelity)`; keep the surrounding doc comments accurate.
9. **`freecell-engine/tests/worker_seam.rs`**, **`render-tests/src/bin/chart_perf.rs`** — update
   `spec.chart` reads → `spec.chart().unwrap()`.
10. **`freecell-engine/tests/fixtures/charts/excel_line_chart_workbook.xlsx`** — the owner's real
    Excel workbook (4 line charts, 2 sheets), committed.
11. **NEW `freecell-engine/tests/charts_corpus.rs`** — the corpus robustness test (below).

## Tests

- **spec.rs unit** — `loaded_unsupported` carries source/ranges/anchor + `chart().is_none()` +
  `title()`; `display_fidelity()` of an Unsupported body is `Unsupported` **even when the source
  would classify Faithful/Degraded**; a Parsed body still classifies from the source; `chart_mut`
  round-trips.
- **load.rs unit** — an unparseable group (`surfaceChart`) parses into an Unsupported spec that
  retains its source + ranges + anchor and reports `Unsupported`; a `bar3DChart` parses into a
  2-D `Bar` kind and classifies `Degraded`; `drawing_charts` skips an absent `rId` but keeps its
  siblings; `discover` skips a sheet with a missing drawing `_rels` and still returns the other
  sheets' drawings.
- **binding.rs unit** — an Unsupported spec yields a `LiveChart` with `chart: None`;
  `reresolve` leaves an Unsupported spec untouched (never panics on the missing `Chart`).
- **save.rs unit** — `reinject_live_charts` with a `None`-chart `LiveChart` byte-preserves that
  chart part (never patched) while still following its host sheet.
- **NEW `tests/charts_corpus.rs`** (the exit criterion):
  - the **real** Excel workbook opens in IronCalc + parses 4 Faithful line charts;
  - the **all-types** generated workbook opens; each supported group (column/bar/area/pie/
    doughnut/scatter) parses into a typed chart (Faithful, `chart().is_some()`); each 3-D group
    classifies **Degraded** and still parses to a 2-D chart; each unsupported group
    (surface/radar/stock/ofPie/bubble) classifies **Unsupported**, is **retained** (source present,
    `chart().is_none()`), and is **not dropped**;
  - **edge cases** don't crash: unresolved `c:f` (missing sheet) parses Faithful and keeps its
    cached values (live binding falls back to cache — the `resolve_falls_back_to_cache…` guard);
    an empty range / non-numeric cell parses without panic; a groupless/garbage chart part is
    retained as Unsupported (never aborts the load);
  - the **dangling-rel** + **missing-drawing-`_rels`** workbooks open (IronCalc + our walk) and
    the load returns the surviving charts, never an error/panic.

## Render tests

**No pixel baseline moves.** The `grid_chart_*` in-grid baselines are built from `ChartSpec::
loaded(chart, SourceXml::new(…))` (a **Parsed** body with a mismatched source for fidelity), which
still renders through `in_grid_chart_element(spec.chart(), spec.title(), fidelity)` **byte-
identically** — the placeholder shows the same title, the plot the same element. The Unsupported-
**body** path renders the *same* `placeholder_element(title)` as the existing
`grid_chart_unsupported_placeholder` baseline, so no new scene is required (covered headlessly by
the fidelity/gpui tests). Because the change touches the in-grid dispatch (grid-render code), run
the **`grid_chart_` subset only**, FOREGROUND under a `timeout`, to confirm zero drift; do **not**
run the full suite. All other P14 work is engine/robustness (no pixel surface).
