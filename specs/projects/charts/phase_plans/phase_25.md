---
status: complete
---

# Phase 25: Scatter (XY)

## Overview

P25 continues the **breadth batch** (implementation_plan §"New graph types") after P22 (column &
bar), P23 (area), and P24 (pie & doughnut): the scatter (XY) type slots onto the already-hardened,
editable pipeline (anchor/cull/clip, live binding, save/source-patch, insert/move/resize/delete,
chrome editing) proven on the line chart through P21 and inherited by every breadth type. As with
P22–P24 the machinery is **reused, not rebuilt** — the net-new work is the type's own renderer
fidelity + regression baselines + round-trip.

Scatter is the **first XY type**, and its defining difference from line/bar/area/pie is **two
numeric axes** (both value axes with nice-tick scaling), not a category axis: each series carries
`c:xVal`/`c:yVal` numeric pairs (not `c:cat`/`c:val`), and a point maps `(x,y)→pixel` through two
independent nice-tick numeric scales spanning every series' data. Much of the scatter surface is
already lifted from the PoC and hardened through P21–P24:

- **Model** already types `ChartKind::Scatter` and `SeriesData::Xy { x, y }`; `ChartInsertKind::Scatter`
  is the only `is_xy()` kind.
- **Load** already parses `c:scatterChart` → `ChartKind::Scatter`, reading `c:xVal`/`c:yVal` numeric
  caches and mapping the two `c:valAx` to `(cat_axis=X, val_axis=Y)`.
- **Write** already emits `c:scatterChart` (with two `c:valAx`) in `CT_ScatterChart`/`CT_ScatterSer`
  order (`xVal`/`yVal`, not `cat`/`val`).
- **Live binding** already resolves the **XY pair**: `binding.rs` reads each series' domain ref as
  `["cat","xVal"]` and value ref as `["val","yVal"]`, and `build_series_shells(n, is_xy)` builds the
  xy shape — so a scatter binds **both** x and y ranges and re-resolves both on recompute. **The XY
  dual-range binding the brief flagged as the risk is already handled** — no design blocker.
- **Renderer** `scatter.rs` already draws standalone dots over two `ScaleLinear` axes with the shared
  X/Y nice domains, the chrome, and the multi-series color cycle.

So the **net-new** P25 work is the OOXML `c:scatterStyle` fidelity the PoC skipped, plus the type's
proof-of-production layer:

1. **`c:scatterStyle`** (`ST_ScatterStyle`: `marker` / `line` / `lineMarker` / `smooth` /
   `smoothMarker`) — governs whether a scatter series draws **connecting line segments**, **point
   markers**, or **both**. The PoC renderer always drew dots-only and the write path hard-coded
   `lineMarker`. P25 models the style, parses it, writes it **from the model**, and renders it:
   `marker` → dots only; `line` → straight connecting segments only; `lineMarker` → segments **and**
   dots (Excel's insert default). `smooth`/`smoothMarker` fall back to **straight** segments (an
   honest fidelity choice — badged **Degraded**, see below).
2. **Markers reuse the line renderer's `c:marker` support.** The PoC hand-drew a fixed circle quad;
   P25 draws each point through the **shared** `line::paint_marker` (every OOXML symbol —
   circle/square/diamond/triangle/star/plus/x/dash/dot), so a scatter marker honors `c:marker` exactly
   like a line marker. Multi-series scatter uses distinct series colors via the shared
   `resolve_series_hsla` (matched swatch-for-dot-cloud by the legend).
3. **Regression baselines** — scatter has never been exercised by a render scene (the chart baseline
   inventory through P24 is line/column/bar/area/pie only). Add standalone `chart_scatter_*` scenes
   (marker-only multi-series, lineMarker, a non-trivial numeric X axis) + one in-grid
   `grid_chart_scatter`, generate + **eyeball** each baseline, commit with the code — the first real
   validation of `ScatterPlot::paint`.
4. **Round-trip** — prove scatter round-trips through `discover_and_parse` / `parse_chart_xml` in all
   three modes: **loaded** (parse `c:scatterChart` + `scatterStyle` + `xVal`/`yVal`), **authored**
   (write → reopen as `ChartKind::Scatter` with the style preserved), **edited** (`SetChartType(Scatter)`
   → save → reopen as scatter, the XY range binding preserved).

**Fidelity — decided honestly for scatter.** The scatter renderer honors two numeric axes, the
`c:scatterStyle` marker/line combination, and the full `c:marker` symbol set — all **Faithful**.
Because P25's renderer now **draws markers on scatter**, a `c:marker` symbol on a `scatterChart` is
now **Faithful** (it left the line-only marker scope). Because P25 draws **straight** segments for a
smoothed scatter, a `c:scatterStyle val="smooth"`/`"smoothMarker"` is **Degraded** (honest badge, not
a silent curve-to-straight). Axis `scaling` (min/max/reversed) and `c:dLbls` on scatter remain
**line-scoped** (their scatter phases are later), so a scatter carrying those keeps its honest badge.

**Out of P25 (kept honest, not silently dropped):**
- **Smoothed scatter** renders as straight segments (Degraded badge) — no spline yet.
- **Excel's per-series `a:ln`/`a:noFill` line-visibility quirk** (Excel writes `scatterStyle="lineMarker"`
  even for a *dots-only* scatter and hides the line via a per-series `<a:ln><a:noFill/></a:ln>`) is
  **not** modeled — P25 keys line visibility off `c:scatterStyle` (the element the brief scopes), so an
  Excel "Scatter (markers only)" file whose group says `lineMarker` renders with connecting lines. A
  `noFill`-line flag on the series stroke is a later-phase fidelity item.
- Axis `scaling` / data labels stay line-scoped; per-point `c:dPt` on scatter stays Degraded (P24 scoped
  `dPt` to pie). Bubble is P26.

## Steps

### Model — `freecell-chart-model` (`lib.rs`)

1. Add a `ScatterStyle` enum (`Clone, Copy, Debug, PartialEq, Eq`) mirroring `ST_ScatterStyle`:
   `Marker`, `Line`, `LineMarker`, `Smooth`, `SmoothMarker`. Methods: `draws_line()` (Line/LineMarker/
   Smooth/SmoothMarker), `draws_markers()` (Marker/LineMarker/SmoothMarker), `is_smooth()`
   (Smooth/SmoothMarker), and `as_ooxml()` (the `c:scatterStyle@val` string). Export from `lib.rs`.
2. Widen `ChartKind::Scatter` (unit) → `ChartKind::Scatter { style: ScatterStyle }` (`Copy`, so scatter
   pattern-matches stay copy-only). Update the doc comment (two numeric axes; `scatterStyle` governs
   line/marker).

### Model authoring — `authoring.rs`

3. `chart_kind()` Scatter arm → `ChartKind::Scatter { style: ScatterStyle::LineMarker }` (matches the
   write/insert default). `from_chart_kind` → `ChartKind::Scatter { .. } => ChartInsertKind::Scatter`.
   Update the `chart_kind_maps_each_menu_type` assertion.

### Engine load — `load.rs`

4. `parse_kind` scatter arm → `ChartKind::Scatter { style: scatter_style(group) }`. Add a
   `scatter_style(group)` helper reading `c:scatterStyle@val` → `ScatterStyle` (marker/line/lineMarker/
   smooth/smoothMarker; **absent/unknown → `LineMarker`**, the value Excel ubiquitously writes and our
   writer/insert use). Update `matches!(kind, ChartKind::Scatter)` → `ChartKind::Scatter { .. }`.
5. Update `scatter_maps_two_value_axes_and_xy_series` to assert `ChartKind::Scatter { style:
   ScatterStyle::LineMarker }`; add `parses_scatter_style` (marker/line/smooth parse into the model;
   absent → LineMarker).

### Engine write — `write.rs`

6. `group_element` scatter arm → emit `<c:scatterStyle val="{style.as_ooxml()}"/>` **from the model**
   instead of the hard-coded `lineMarker`. `axes_xml` scatter arm → `ChartKind::Scatter { .. }`.
7. Update `serialize_roundtrips_scatter` kind to `Scatter { style: LineMarker }`; add
   `serialize_roundtrips_scatter_styles` round-tripping marker / line / smooth through serialize→parse
   (the style survives). Add `write_authored_scatter_reopens_as_scatter_with_style` — an authored
   marker-style scatter written via `write_authored_charts`, reopened through `discover_and_parse` as
   `ChartKind::Scatter { style: Marker }` (the scatter twin of the authored bar/area reopen).

### Engine save — `save.rs`

8. `matches!(chart.kind, ChartKind::Scatter)` → `ChartKind::Scatter { .. }` (the reflow already maps
   xVal/yVal; no other change).

### Renderer — `scatter.rs`

9. Carry per-series `marker: Option<Marker>` + resolved line `width_px` on `ScatterSeries`, and
   `style: ScatterStyle` on `ScatterPlot`; build them in `from_chart` from the widened
   `ChartKind::Scatter { style }` + each series' `marker`/`stroke`. Color resolves preferring the
   `a:ln` stroke color, then the series fill/theme color, then the palette (with `a:ln` alpha), exactly
   like the line renderer.
10. `paint`: when `style.draws_line()`, draw one gpui-component `Line` per series connecting its points
    **in data order** with straight (`Linear`) segments (smooth falls back to straight); when
    `style.draws_markers()`, paint each point through the **shared** `super::line::paint_marker` (reusing
    the full `c:marker` symbol set + the white-edged default dot). Line first, markers on top.
11. Expose `pub(super) fn paint_marker` + `pub(super) fn line_width_px` from `line.rs` so scatter reuses
    them (no logic change to line — its pixels are byte-identical).

### Legend — `chrome.rs`

12. Extend the legend's mark-color helper so a **scatter** series' swatch (like a line's) follows the
    `a:ln` stroke color when present, else the fill/theme/palette color — so the dot cloud and its
    legend swatch match by construction even when a scatter is styled via `a:ln`. (`line_mark_color`
    → `mark_color(series, follows_stroke)`, `follows_stroke = Line | Scatter`.)

### Fidelity — `fidelity.rs`

13. **Scope `c:marker` to scatter too.** `unsupported_marker` returns false for a `scatterChart` (P25
    draws every marker symbol on scatter), so a non-circle marker on scatter is now **Faithful**. Add
    `is_scatter_chart(xml)`.
14. **Degrade smoothed scatter.** Add `unsupported_scatter_smooth(xml)` = a `scatterChart` whose
    `c:scatterStyle@val` is `smooth`/`smoothMarker` (we draw straight → Degraded). Wire into
    `has_render_affecting_unsupported_feature`. Update the module docs (marker now line-**and-scatter**
    scoped; smoothed scatter degrades).

### Render scenes — `chart_scene.rs` + `render_suite.rs`

15. Add three standalone scatter scenes (+ `all()` entries + a `p25_scenes_*` unit test):
    - `chart_scatter_markers` — a **marker-only** two-series scatter (`ScatterStyle::Marker`) over two
      numeric axes, distinct series colors + distinct marker symbols, title / both axis titles / right
      legend (the bread-and-butter scatter; proves two numeric nice-tick axes + dots).
    - `chart_scatter_line_markers` — the same shape as `ScatterStyle::LineMarker` (dots **and**
      connecting straight segments), proving the connecting-line path.
    - `chart_scatter_wide_x` — a single-series scatter whose **X is not 1..n** (e.g. 100..900), proving
      the numeric X nice-tick scale spans a non-trivial domain (the XY-defining property).
16. `render_suite.rs`: add the three to `chart_render_cases!` (the drift guard keeps them in lockstep
    with `chart_scene::all()`).

### Grid case — `cases.rs` + `render_suite.rs`

17. `cases.rs`: add `grid_chart_scatter` — a **loaded** marker-scatter `ChartSpec` at the shared
    `chart_anchor` over the backing table (an `in_grid_scatter_chart` fixture + `in_grid_scatter_spec`
    with a `<c:scatterChart>` source so it classifies Faithful), proving the ChartLayer →
    `scatter_element` in-grid path. Register in `render_cases!` + `CASE_NAMES`.

### Baselines (own phase item — generate + eyeball, then commit with the code)

18. `render_tests.sh generate --only chart_scatter_` / `--only grid_chart_scatter`; **eyeball** each PNG
    (two numeric axes + a dot cloud; the connecting straight segments through the dots; the wide-X nice
    ticks; the in-grid scatter over the table). Commit with the code.

### Round-trip (local `discover_and_parse` / `parse_chart_xml` reopen)

19. `worker_seam.rs`: add `retyped_to_scatter_chart_roundtrips` — `SetChartType(Scatter)` on a ranged
    chart → save → `discover_and_parse` reopens as `ChartKind::Scatter`, the XY `c:f` binding preserved
    (the edited path, mirroring the Column/Area/Pie retype tests; the reopened series is `SeriesData::Xy`).

## Tests

- **Model (`lib.rs`)** — `ScatterStyle` predicate methods (`draws_line`/`draws_markers`/`is_smooth`);
  `as_ooxml` round-trips each variant; `ChartKind::Scatter` carries a style.
- **Model authoring (`authoring.rs`)** — `chart_kind`/`from_chart_kind` still invert for Scatter with
  the style; `near_empty` scatter still xy.
- **Load (`load.rs`)** — `parses_scatter_style` (marker/line/smooth parse; absent → LineMarker);
  `scatter_maps_two_value_axes_and_xy_series` updated for the style.
- **Write (`write.rs`)** — `serialize_roundtrips_scatter` (style); `serialize_roundtrips_scatter_styles`
  (marker/line/smooth survive serialize→parse); `write_authored_scatter_reopens_as_scatter_with_style`;
  `near_empty_insert_templates_round_trip` stays green (Scatter template).
- **Edited round-trip (`worker_seam.rs`)** — `retyped_to_scatter_chart_roundtrips`.
- **Renderer (`scatter.rs`)** — shared domains cover all points; point count == data; distinct series
  colors; markers carry into the plot; `style.draws_line()`/`draws_markers()` gate the line/markers;
  rejects non-scatter/empty.
- **Legend (`chrome.rs`)** — a scatter legend swatch follows the series color (and `a:ln` stroke when
  present), matching the dot color.
- **Fidelity (`fidelity.rs`)** — `p25_scatter_markers_faithful_but_smooth_degrades`: a marker/lineMarker
  scatter (with any `c:marker`) is Faithful; a `smooth`/`smoothMarker` scatter Degrades; existing
  line-marker scoping stays green (line marker still Faithful, a non-line/non-scatter marker still
  Degrades). Update `markers_are_scoped_to_the_line_renderer` (scatter + diamond now Faithful).
- **Scenes (`chart_scene.rs`)** — `p25_scenes_carry_their_scatter_style`: each new scene is a
  `ChartKind::Scatter` of the intended style; the wide-X scene's x-values exceed the 1..n range.
- **Pixel (new baselines)** — `chart_scatter_markers`, `chart_scatter_line_markers`,
  `chart_scatter_wide_x`, `grid_chart_scatter` each render == baseline.

## Render validation

Scatter rendering **is** in-scope for the pixel suite (grid/cell/sheet **and** chart scenes, CLAUDE.md
render-tests §Scope). Per the brief, P25 does **not** run the full suite (deferred to P26, the final
type's cross-type sweep). During coding, iterate with the **subset only**, foreground under a `timeout`
(never background a render job):
- `render_tests.sh test chart_scatter_` — the three new standalone scatter scenes.
- `render_tests.sh test grid_chart_` — the new in-grid scatter (+ confirms the other `grid_chart_*` are
  untouched).
- `render_tests.sh test chart_line_` — confirm no existing line baseline moved (scatter reuses
  `paint_marker`/`line_width_px` by exposing them `pub(super)`; the line render path is unchanged, so
  its pixels are byte-identical).

New baselines are generated + **eyeballed** and committed with the code. No existing baseline is intended
to move (every prior chart case is line/column/bar/area/pie; no scatter baseline existed before). The CI
`render` gate is dispatched by the manager after commit.

## Results

All green (bar the documented `soffice` env caveat). Scatter (XY) is a production type on the
hardened pipeline: it loads, renders (two numeric nice-tick axes + dots + `c:scatterStyle`
marker/line combination via the shared marker painter), authors, edits, live-binds (both x and y
ranges), and round-trips — reusing the P1–P24 machinery, with the type's `scatterStyle` fidelity +
regression baselines + round-trip proofs added. The XY dual-range binding the brief flagged as the
risk was already handled by `binding.rs` (`["cat","xVal"]`/`["val","yVal"]`) — no design blocker.

### What shipped
- **Model** — `ChartKind::Scatter` widened to `Scatter { style: ScatterStyle }`; new `ScatterStyle`
  enum (marker / line / lineMarker / smooth / smoothMarker) with `draws_line`/`draws_markers`/
  `is_smooth`/`as_ooxml`. Transparent to live-binding (`resolve_chart` preserves `kind`; the retype
  path already builds the xy shape via `ChartInsertKind::is_xy`).
- **Load** — `parse_kind` scatter arm reads `c:scatterStyle` via a new `scatter_style` helper
  (marker/line/smooth/smoothMarker; **absent/unknown → LineMarker**, Excel's ubiquitous default).
- **Write** — `group_element` emits `<c:scatterStyle val="…"/>` **from the model** (not hard-coded)
  in `CT_ScatterChart` order. Round-trips serialize→parse (every style) and write→discover.
- **Renderer (`scatter.rs`)** — promoted the PoC dots-only widget to production: two numeric
  `ScaleLinear` axes over the shared X/Y nice domains; `c:scatterStyle` gates the connecting `Line`
  (straight, in data order; smooth falls back to straight) and the markers; markers reuse the line
  renderer's shared `paint_marker` (full `c:marker` symbol set) + `line_width_px` (exposed
  `pub(super)`, so line's pixels are byte-identical); color/width resolve like line (`a:ln` pref).
- **Legend (`chrome.rs`)** — a scatter series' swatch follows the `a:ln` stroke color (else fill/
  theme/palette), like line, so the dot cloud and its swatch match by construction.
- **Fidelity** — `c:marker` is now **line-and-scatter** scoped (`unsupported_marker` returns false for
  a `scatterChart`), so a marker on scatter is Faithful; a **smoothed** scatter
  (`scatterStyle=smooth`/`smoothMarker`) is Degraded (`unsupported_scatter_smooth`) — the honest badge
  for the straight-segment fallback. Axis scaling / data labels stay line-scoped on scatter.

### New render scenes + baselines (4, generated + eyeballed, committed with the code)
- `chart_scatter_markers` — a marker-only two-series scatter over two numeric axes (X 0–10, Y 10–70),
  blue circles + orange diamonds, matching per-series legend.
- `chart_scatter_line_markers` — the same data as `lineMarker` (straight connecting segments thread the
  dots in data order).
- `chart_scatter_wide_x` — a single-series scatter with a non-trivial numeric X axis (0–1000 nice
  ticks, X not 1..n).
- `grid_chart_scatter` — a loaded marker scatter painted in-grid over the backing table (ChartLayer →
  `scatter_element`).

### Render subset — PASS (foreground, under a `timeout` watchdog)
- `render_tests.sh test chart_scatter_` → **3 passed** (158s): the three new standalone scatter scenes
  render == their eyeballed baselines.
- `render_tests.sh test grid_chart_` → **10 passed**: the new `grid_chart_scatter` + all 9 existing
  `grid_chart_*` cases — existing grid baselines unchanged.
- `render_tests.sh test chart_line_` → **15 passed**: no existing line baseline moved (scatter reuses
  `paint_marker`/`line_width_px` by exposing them `pub(super)`; the line render path is unchanged).

The `chart_scene_names_match_table` + `case_names_match_table` drift guards are green. (Full cross-type
suite deferred to P26 per the brief; the CI `render` gate is manager-dispatched after commit.)

### Round-trip — PASS (local `discover_and_parse` / `parse_chart_xml` reopen)
- Engine chart unit — incl. `serialize_roundtrips_scatter` (style), `serialize_roundtrips_scatter_styles`
  (marker/line/lineMarker/smooth/smoothMarker survive), `write_authored_scatter_reopens_as_scatter_with_style`
  (authored marker scatter → write → `discover_and_parse` reopens as `Scatter{Marker}` + xy series),
  `parses_scatter_style` (marker/line/smooth parse; absent → LineMarker).
- Engine integration (`worker_seam`): `retyped_to_scatter_chart_roundtrips` (`SetChartType(Scatter)` →
  save → reopen as scatter, xy series, y-range binding preserved). (The two `charts_roundtrip_libreoffice`
  tests fail at the `soffice` step — the documented env-broken LibreOffice caveat, out of scope; external
  round-trip rides CI.)
- Model (incl. the new `ScatterStyle` + fidelity tests), app chart (incl. the new scatter renderer +
  scatter-legend tests), render-tests lib chart-scene (incl. `p25_scenes_carry_their_scatter_style`) green.

### Checks
`cargo fmt --all --check` clean; `cargo clippy` clean across freecell-chart-model / -engine / -app /
render-tests (`--all-targets`).
