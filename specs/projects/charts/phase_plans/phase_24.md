---
status: complete
---

# Phase 24: Pie & doughnut

## Overview

P24 continues the **breadth batch** (implementation_plan §"New graph types") started by P22
(column & bar) and P23 (area): the pie & doughnut type slots onto the already-hardened, editable
pipeline (anchor/cull/clip, live binding, save/source-patch, insert/move/resize/delete, chrome
editing) proven on the line chart through P21 and inherited by every breadth type. As with P22/P23,
most of the machinery is **reused, not rebuilt** — the net-new work is the type's own renderer
fidelity + regression baselines + round-trip.

Pie/doughnut is the **most visually distinct** type so far: radial slices, **no cartesian axes**
(chrome = title + legend-by-category + optional on-slice labels), and per-**slice** (not per-series)
coloring. A PoC-lifted `pie.rs` already renders a monochrome-avoiding slice pie + doughnut hole + a
(always-on) on-slice % label; P24 promotes it to **production** by wiring the OOXML pie features the
PoC skipped:

1. **`c:varyColors` + `c:dPt` per-slice color.** `varyColors` (usually true for pie) colors each
   **slice** from the palette by slice index; a `c:dPt` entry overrides an individual slice's fill.
   Per-slice color = **dPt override if present, else the varied palette color for that index** (or,
   when `varyColors` is off, the single series fill). Resolved through the shared
   `parse_solid_fill` / theme-palette / `resolve_series_color` path the other types use, so the
   legend swatch and the slice match **by construction** (both call one resolver).
2. **`c:holeSize`** — doughnut inner radius as a percent of the outer (already modeled as
   `doughnut_hole`; render the doughnut as an **annulus**, 0 = solid pie).
3. **Rotation & explosion.** `c:firstSliceAng` rotates the whole pie — Excel measures it in **degrees
   clockwise from 12 o'clock**, which maps **directly** onto gpui-component's `Pie::start_angle`
   (angle 0 = 12 o'clock, increasing = clockwise, verified in `plot/shape/arc.rs`), so
   `start_angle = deg·π/180`, `end_angle = start_angle + 2π`. `c:dPt/c:explosion` pulls an individual
   slice outward along its bisector by a percent of the radius (offset the arc's center for that
   slice; shrink the base radius so exploded slices still fit).
4. **On-slice % labels.** `c:dLbls`/`showPercent` → each slice labeled with its percent of the total
   at the slice mid-angle (skipping tiny slivers). Pie is where labels matter, so this is now
   **gated on `showPercent`** (the PoC drew it unconditionally).

**Fidelity — decided honestly for pie (the brief's ask).** The pie renderer honors slice colors
(vary/dPt), holeSize, rotation, explosion, and **percent** labels — all **Faithful**. It does **not**
draw the *other* data-label kinds (value / category-name / series-name / legend-key) on a pie, so a
pie showing any of those keeps its honest **Degraded** badge. And `c:dPt` becomes **pie-scoped** in
the accessor: a `dPt` on a pie/doughnut is now Faithful (we render it), while a `dPt` on any other
group (bar/line/area/scatter — their per-point styling is a later phase) still Degrades. This mirrors
the P6/P12/P13 pattern of scoping a feature to the renderer that honors it.

**Out of P24 (kept honest, not silently dropped):** per-point label *overrides* (`c:dLbl`) still
Degrade on any group (we draw uniform labels, not per-point text). Scatter/bubble are P25/P26.

## Steps

### Model — `freecell-chart-model` (`lib.rs`)

1. Add a `DataPoint` struct (`Clone, Debug, PartialEq`) mirroring `c:dPt`: `index: u32` (`c:idx`,
   the 0-based slice), `color: Option<ChartColor>` (`c:spPr` solid fill override, `None` = use the
   varied/series color), `explosion: Option<u16>` (`c:explosion@val`, radial offset percent). Export
   it from `lib.rs`.
2. Add `data_points: Vec<DataPoint>` to `Series` (the OOXML-correct home — `c:dPt` is a child of
   `c:ser`, like the existing `data_labels`/`stroke`). Initialize it empty in both `category_value`
   and `xy` constructors; add a `with_data_points(Vec<DataPoint>)` builder. (No other Series
   construction site exists — all go through the two builders.)
3. Widen `ChartKind::Pie` with two group-level pie attrs (both `Copy`, so pie pattern-matches stay
   copy-only): `first_slice_ang: u16` (`c:firstSliceAng`, degrees clockwise from 12 o'clock, 0..=360)
   and `vary_colors: bool` (`c:varyColors`, true = per-slice palette, the pie default). Keep
   `doughnut_hole: Option<f32>`.
4. Update the two model construction sites in `authoring.rs` (`chart_kind()` for Pie/Doughnut →
   `first_slice_ang: 0, vary_colors: true`) and match sites (`from_chart_kind` adds `..`).

### Engine load — `load.rs`

5. `parse_kind` pie/doughnut arms: read `first_slice_ang(group)` (`c:firstSliceAng@val`,
   `rem_euclid(360)`, default 0) and `vary_colors(group)` (`c:varyColors@val`; `0`/`false` → false,
   **absent → true** — pie's default; Excel emits `val="1"`) into the widened `ChartKind::Pie`
   (doughnut also reads `holeSize`, unchanged).
6. `parse_series`: after building the series, parse its `c:dPt` children into `data_points` via a
   `parse_data_points(ser)` helper — each `c:dPt` reads `c:idx` (required; skip if absent), an
   optional `c:spPr` solid-fill color (via the shared `parse_solid_fill`, so a `schemeClr` dPt
   resolves to `ChartColor::Theme`), and an optional `c:explosion@val`. Attach with
   `with_data_points` when non-empty.

### Engine write — `write.rs`

7. `group_element` pie/doughnut arms: emit `c:varyColors` + `c:firstSliceAng` **from the model**
   (`bool_val(vary_colors)` / `first_slice_ang`) instead of the hard-coded `1`/`0`, in `CT_PieChart`
   / `CT_DoughnutChart` child order (varyColors, ser*, firstSliceAng[, holeSize]).
8. `series_element`: emit each series' `c:dPt` overrides (a `dpt_elements` helper) **between `spPr`
   and `dLbls`** — the `CT_*Ser` slot valid across all types (empty for non-pie, so only pie ever
   emits them). Each `c:dPt` = `c:idx`, then (`c:explosion`)?, then (`c:spPr`/`a:srgbClr`)? — the
   `CT_DPt` order (idx before explosion before spPr). A theme dPt color resolves to its office sRGB
   (authored charts use concrete sRGB, like series fills).

### Renderer — `pie.rs`

9. Carry per-slice `color` + `explosion` (fraction) + a `show_percent` flag + `first_slice_ang` on
   `PiePlot`; build them in `from_chart` from the widened `ChartKind::Pie` + the series `data_points`
   + `data_labels`. Per-slice color resolves through a new shared `resolve_slice_color(series, i,
   vary_colors)` (in `style.rs`): dPt override (theme-resolved) → varied palette color → single
   series fill. `show_percent` = the series `data_labels`' `show_percent`.
10. `paint`: rotate via `Pie::start_angle(deg·π/180).end_angle(start+2π)`; shrink the base outer
    radius by the max explosion so exploded slices fit; offset an exploded slice's arc center (and
    its label) along the slice bisector; label a slice only when `show_percent` **and** its share ≥
    the min-sliver fraction. Doughnut inner radius = `doughnut_hole·outer` (annulus).

### Legend — `chrome.rs`

11. `legend_entries` pie branch: color each slice via the same `resolve_slice_color(series, i,
    vary_colors)` the renderer uses (so swatch↔slice match by construction, including dPt overrides
    + `varyColors` off), pulling `vary_colors` from `ChartKind::Pie`.

### Fidelity — `fidelity.rs`

12. **Scope `c:dPt` to pie.** Remove `dPt` from `RENDER_AFFECTING_PRESENCE_MARKERS`; add
    `unsupported_data_point(xml) = contains dPt && !is_pie_chart(xml)` (so a dPt on a pie/doughnut is
    Faithful, on any other group Degraded). Add `is_pie_chart(xml)` (pieChart|doughnutChart).
13. **Scope pie labels.** In `unsupported_data_labels`, add a pie branch: a pie showing **only**
    percent is Faithful; a pie showing any *non-percent* label kind (showVal/showCatName/showSerName/
    showLegendKey/showBubbleSize) still Degrades. Update the module docs (firstSliceAng/varyColors now
    *honored*, dPt now pie-scoped).

### Render scenes — `chart_scene.rs` + `render_suite.rs`

14. Add four standalone pie/doughnut scenes (+ `all()` entries + a `p24_scenes_*` unit test):
    - `chart_pie_vary_colors` — a 4-slice varyColors pie with a right legend (the bread-and-butter
      pie; proves varied per-slice palette + per-slice legend).
    - `chart_doughnut_hole` — the same data as a **doughnut** (`doughnut_hole: Some(0.5)`), proving
      the annulus.
    - `chart_pie_percent_labels` — a pie with `showPercent` on-slice % labels.
    - `chart_pie_exploded` — a pie with **rotation** (`first_slice_ang`), one **exploded** slice, and
      a `c:dPt` **custom slice color** (proves all three at once).
15. `render_suite.rs`: add the four to `chart_render_cases!` (the drift guard keeps them in lockstep
    with `chart_scene::all()`).

### Grid case — `cases.rs` + `render_suite.rs`

16. `cases.rs`: add `grid_chart_pie` — a **loaded** pie `ChartSpec` at the shared `chart_anchor` over
    the backing table (an `in_grid_pie_chart` fixture + `in_grid_pie_spec` with a `<c:pieChart>`
    source so it classifies Faithful), proving the ChartLayer → `pie_element` in-grid path. Register
    in `render_cases!` + `CASE_NAMES`.

### Baselines (own phase item — generate + eyeball, then commit with the code)

17. `render_tests.sh generate --only chart_pie_` / `--only chart_doughnut_` / `--only grid_chart_pie`;
    **eyeball** each PNG (varied slice colors + matching legend; the doughnut annulus; the % labels on
    slices; the rotated + exploded + custom-color slice; the in-grid pie over the table). Commit with
    the code.

### Round-trip (local `discover_and_parse` / `parse_chart_xml` reopen)

18. `write.rs`: keep `serialize_roundtrips_pie_and_doughnut` (updated for the new fields) and add
    `serialize_roundtrips_pie_with_dpt_rotation_and_hole` — a doughnut carrying `first_slice_ang`,
    `vary_colors: false`, a `holeSize`, and a `c:dPt` (sRGB color + explosion) round-trips
    serialize→parse. Add `write_authored_pie_reopens_as_pie_with_hole` — an authored doughnut reopens
    through `discover_and_parse` as `ChartKind::Pie { doughnut_hole: Some(..) }` (the pie twin of the
    authored bar/area reopen).
19. `load.rs`: add `parses_pie_rotation_vary_colors_and_dpt` — a `c:pieChart` with `c:firstSliceAng`,
    `c:varyColors="0"`, and a `c:dPt` (idx + spPr color + explosion) parses into the model; an absent
    `varyColors` defaults to true; update `doughnut_reads_hole_size_and_pie_has_none` +
    `discover_and_parse_walks_multiple_charts_in_document_order` for the widened kind.
20. `worker_seam.rs`: add `retyped_to_pie_chart_roundtrips` — `SetChartType(Pie)` on a ranged chart →
    save → `discover_and_parse` reopens as `ChartKind::Pie`, `c:f` binding preserved (the edited path,
    mirroring the Column/Area retype tests).

## Tests

- **Model (`lib.rs`)** — `DataPoint` builder + `with_data_points`; `ChartKind::Pie` carries
  `first_slice_ang`/`vary_colors`; the round-trip accessor test still green.
- **Model authoring (`authoring.rs`)** — `chart_kind`/`from_chart_kind` still invert for Pie/Doughnut
  with the new fields; placeholder pie/doughnut values still positive.
- **Load (`load.rs`)** — `parses_pie_rotation_vary_colors_and_dpt`: firstSliceAng/varyColors/dPt
  (idx+color+explosion) parse; absent varyColors → true; a `schemeClr` dPt → `ChartColor::Theme`.
- **Write (`write.rs`)** — `serialize_roundtrips_pie_and_doughnut` (new fields); the dPt/rotation/hole
  round-trip; `write_authored_pie_reopens_as_pie_with_hole`; `near_empty_insert_templates_round_trip`
  stays green (Pie/Doughnut templates).
- **Edited round-trip (`worker_seam.rs`)** — `retyped_to_pie_chart_roundtrips`.
- **Renderer (`pie.rs`)** — slice sweeps sum to 2π; doughnut inner radius = hole·outer; slices have
  distinct colors (varyColors); a dPt color overrides its slice; an explosion offsets the slice
  center; `show_percent` gates labels; rejects non-pie.
- **Legend (`chrome.rs`)** — pie legend keys slice colors via `resolve_slice_color` (a dPt override
  wins; varyColors-off uses the series fill); existing per-slice test stays green.
- **Fidelity (`fidelity.rs`)** — `p24_pie_dpt_and_percent_labels_faithful_but_other_labels_degrade`:
  a pie with dPt / only-percent labels is Faithful; a pie with value/category labels Degrades; a dPt
  on a non-pie group still Degrades; existing dPt/label tests stay green.
- **Scenes (`chart_scene.rs`)** — `p24_scenes_carry_their_pie_kind`: each new scene is a
  `ChartKind::Pie`, the doughnut has a hole, the exploded scene carries a dPt + rotation, the
  percent-labels scene shows percent.
- **Pixel (new baselines)** — `chart_pie_vary_colors`, `chart_doughnut_hole`,
  `chart_pie_percent_labels`, `chart_pie_exploded`, `grid_chart_pie` each render == baseline.

## Render validation

Pie/doughnut rendering **is** in-scope for the pixel suite (grid/cell/sheet **and** chart scenes,
CLAUDE.md render-tests §Scope). Per the brief, P24 does **not** run the full suite (deferred to P26).
During coding, iterate with the **subset only**, foreground under a `timeout` (never background a
render job):
- `render_tests.sh test chart_pie_` / `test chart_doughnut_` — the four new standalone scenes.
- `render_tests.sh test grid_chart_` — the new in-grid pie (+ confirms the other `grid_chart_*` are
  untouched).
- `render_tests.sh test chart_line_` — confirm no existing line baseline moved (the change is new
  scenes + a scoped fidelity/legend change; no shared render-chrome code that affects line pixels).

New baselines are generated + **eyeballed** and committed with the code. No existing baseline is
intended to move (every prior chart case is line/column/bar/area; no pie baseline existed before). The
CI `render` gate is dispatched by the manager after commit.

## Results

All green (bar the documented `soffice` env caveat). Pie & doughnut is a production type on the
hardened pipeline: it loads, renders (varied/`dPt` slice colors, the doughnut annulus, rotation,
explosion, on-slice percent labels), authors, edits, live-binds, and round-trips — reusing the
P1–P21 machinery, with the type's renderer fidelity + regression baselines + round-trip proofs
added.

### What shipped
- **Model** — `ChartKind::Pie` widened with `first_slice_ang: u16` (`c:firstSliceAng`) + `vary_colors:
  bool` (`c:varyColors`); a new `DataPoint { index, color, explosion }` (`c:dPt`) + `Series.data_points`
  (empty for non-pie) with a `with_data_points` builder. Transparent to live-binding (`resolve_chart`
  preserves `kind`).
- **Load** — pie/doughnut arms read `first_slice_ang` (`rem_euclid(360)`, default 0) + `vary_colors`
  (absent → true, pie's default; `0`/`false` → off); `parse_data_points` reads each `c:dPt` (idx +
  optional `c:spPr` fill via the shared `parse_solid_fill` + optional `c:explosion`), so a `schemeClr`
  dPt resolves to `ChartColor::Theme`.
- **Write** — `group_element` emits `c:varyColors`/`c:firstSliceAng` from the model in
  `CT_PieChart`/`CT_DoughnutChart` order; `series_element` emits each `c:dPt` (idx → explosion → spPr,
  `CT_DPt` order) between `spPr` and `dLbls`. Round-trips serialize→parse and write→discover.
- **Renderer (`pie.rs`)** — promoted the PoC widget to production: per-slice color via the shared
  `resolve_slice_color` (dPt override → varied palette → single series fill); doughnut annulus (`inner
  = hole·outer`); rotation via `Pie::start_angle(deg·π/180).end_angle(+2π)` (Excel's clockwise-from-12
  convention maps straight onto the primitive); explosion offsets an exploded slice's arc center + its
  label along the bisector, shrinking the base radius so it fits; on-slice **percent** labels gated on
  `showPercent`.
- **Legend (`chrome.rs`)** — the pie legend keys each slice via the **same** `resolve_slice_color`, so
  slice↔swatch match by construction (dPt overrides + `varyColors`-off included).
- **Fidelity** — `c:dPt` is now **pie-scoped** (`unsupported_data_point`): Faithful on a pie/doughnut
  (we render it), Degraded on any other group. Pie labels are **scoped honestly**: a pie showing only
  percent is Faithful, a pie showing value/category/series-name/legend-key still Degrades. `firstSliceAng`/
  `varyColors`/`holeSize`/`explosion` now noted as **honored** (out of the degrade set).

### New render scenes + baselines (5, generated + eyeballed, committed with the code)
- `chart_pie_vary_colors` — a 4-slice varyColors pie starting at 12 o'clock (clockwise) with a matching
  per-slice legend.
- `chart_doughnut_hole` — the same data as a `holeSize: 50%` annulus.
- `chart_pie_percent_labels` — on-slice `NN%` labels (45/20/25/10) via `showPercent`.
- `chart_pie_exploded` — `first_slice_ang: 90` (slice A starts at 3 o'clock), slice A **exploded** +
  a `c:dPt` **custom purple** fill (distinct from the palette), others varied.
- `grid_chart_pie` — a loaded pie painted in-grid over the backing table (ChartLayer → `pie_element`).

### Render subset — PASS (foreground, under a `timeout` watchdog)
`render_tests.sh test chart_` → **39 passed, 0 failed** (563s): the `chart_` filter matched all 29 chart
scenes (15 `chart_line_*` + 6 column/bar + 4 area **all unchanged**, + the 4 new pie/doughnut) **and** all
9 `grid_chart_*` cases (incl. the new `grid_chart_pie`), plus the `chart_scene_names_match_table` drift
guard. No existing baseline moved. (Full cross-type suite deferred to P26; the CI `render` gate is
manager-dispatched after commit.)

### Round-trip — PASS (local `discover_and_parse` / `parse_chart_xml` reopen)
- Engine chart unit: **113 passed** — incl. `serialize_roundtrips_pie_and_doughnut`,
  `serialize_roundtrips_pie_with_dpt_rotation_and_hole` (rotation + varyColors-off + holeSize + dPt),
  `write_authored_pie_reopens_as_pie_with_hole` (authored doughnut → write → `discover_and_parse` reopens
  as `Pie{hole,firstSliceAng}`), `parses_pie_rotation_vary_colors_and_dpt`.
- Engine integration: `worker_seam` **52 passed** — incl. `retyped_to_pie_chart_roundtrips`
  (`SetChartType(Pie)` → save → reopen as pie, `c:f` binding preserved). Engine lib **226**, charts_corpus
  **8** green. (The two `charts_roundtrip_libreoffice` tests fail at the `soffice --convert-to` step — the
  documented env-broken LibreOffice caveat, out of scope; external round-trip rides CI.)
- Model **87** (incl. the new fidelity test), app chart **67** (incl. the new pie renderer tests),
  render-tests lib chart-scene **9** (incl. `p24_scenes_carry_their_pie_kind`) green.

### Checks
`cargo fmt --all --check` clean; `cargo clippy` clean across freecell-chart-model / -engine / -app /
render-tests (`--all-targets`).
