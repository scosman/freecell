---
status: complete
---

# Phase 3: Gate 3 — scatter (two numeric axes + dots)

## Overview

Gates 1 (multi-series line) and 2 (bar family, stacked area, pie/doughnut) both PASSed on
gpui-component's raw `plot/` primitives. Gate 3 (functional_spec §4, §7) tests the one
in-scope-adjacent type the research flagged as a genuine step-change: **scatter (XY)**. Its
*only* net-new demand over line/area is that the **X axis is also a numeric `ScaleLinear`**
(not a `ScaleBand`/`ScalePoint` category axis) and the marks are **standalone dots** drawn
from `c:xVal`/`c:yVal` pairs — not a connected path. Everything else (chart title, BOTH
numeric axis titles, nice ticks on both axes, legend + multi-series color cycle) is the
Phase 1/2 scaffolding, reused verbatim.

The spec question this answers: **is scatter a modest addition once the axis/legend
scaffolding exists, or does the category-vs-value orientation baked into the primitives make
it painful?** That directly sets whether scatter is in-scope for the follow-on. Checkpoint
semantics are lower-stakes: a scatter FAIL records scatter *out-of-scope for the follow-on*,
NOT a whole-project NO-GO — report the verdict straight.

Deliverable: `chart-render/src/scatter.rs` (a `ScatterPlot`), wired through
`lib.rs::chart_element`; two new scenes (single-series + multi-series scatter) in
`scenes.rs`; captured to `results/*.png` + regenerated `manifest.json`; single-agent review
each appended to `results/review.md`; findings appended to `chart-render/findings.md`.

## Design notes / key primitive facts (verified against the pinned rev a9a7341)

- **Dots are the `Line` primitive's dot mark, hand-drawn directly.** `plot/shape/line.rs`
  paints each dot as `quad(bounds(top_left, size(d,d)), d/2, fill, 1px, stroke, default)` —
  a rounded quad whose corner radius = half its side, i.e. a filled circle. Scatter wants
  dots WITHOUT the connecting path, so rather than abuse `Line` with a transparent stroke,
  `ScatterPlot::paint` calls `window.paint_quad(...)` per point with that exact shape (the
  proven dot recipe, no path). This is the "hand-draw filled circles like `area.rs`
  hand-rolls polygons" fallback the phase brief allows; it is simpler than either.
- **Both axes are `ScaleLinear`.** X: `ScaleLinear::new([x_scale.min, x_scale.max],
  [plot_left, plot_right])` (data increases left→right). Y: `ScaleLinear::new([y_scale.min,
  y_scale.max], [plot_bottom, plot_top])` (inverted, min at the bottom) — identical to the
  line/area value axis. No `ScaleBand`/`ScalePoint` gutter gotcha applies because there is
  no band scale.
- **Shared X and Y domains via `NiceScale::spanning`.** X domain = `spanning` over the union
  of EVERY series' x-values; Y domain = `spanning` over the union of every series' y-values.
  `spanning` (not `for_values`) matches Excel's scatter auto-ranging (axes zoom to the data,
  not forced to zero) — the same reuse Gate 1 made for the line value axis. The nice
  outward-rounding naturally pads the data inside the domain so dots sit inset from the
  frame; no manual pixel inset needed (keeps ticks/grid exactly at the frame edges, as
  `line.rs`'s value axis does).
- **Ticks + grid on BOTH axes.** `Grid::new().x(vertical_pixels).y(horizontal_pixels)`
  (`grid.rs`: `.x` = vertical lines, `.y` = horizontal lines). `PlotAxis` draws the two axis
  lines + numeric tick labels: Y labels right-aligned left of the axis (`AxisLabelSide::Start`,
  as line/area), X labels centered below the bottom axis at each x-tick pixel (numeric via
  `format_tick`, where line/area put category text).
- **Chrome reused unchanged.** `chrome::chart_frame` puts `val_axis.title` above the plot and
  `cat_axis.title` below (for non-horizontal-bar kinds). Scatter stores the Y-axis title in
  `val_axis` and the X-axis title in `cat_axis`, so both numeric axis titles render with zero
  chrome changes. The legend enumerates series with `series_color(i)`; scatter dots resolve
  color the identical way (`s.color.unwrap_or(series_color(i))` at the same index), so
  swatch↔dot-cloud mapping is correct by construction.

## Steps

1. **`chart-render/src/scatter.rs` (new).**
   - `struct ScatterSeries { xs: Vec<f64>, ys: Vec<f64>, color: Hsla }`.
   - `#[derive(IntoPlot)] pub struct ScatterPlot { series: Vec<ScatterSeries>, x_scale:
     NiceScale, y_scale: NiceScale }`.
   - `pub fn ScatterPlot::from_chart(chart: &Chart) -> Option<Self>`: return `None` unless
     `matches!(chart.kind, ChartKind::Scatter)`; iterate `chart.series.iter().enumerate()`,
     keep only `SeriesData::Xy { x, y }`, resolve color `model_hsla(s.color.unwrap_or_else(||
     series_color(i)))` (index `i` over ALL series, matching the legend); collect into
     `ScatterSeries`. Return `None` if no xy series. Compute `x_scale =
     NiceScale::spanning(all_xs, TARGET_TICKS)` and `y_scale = NiceScale::spanning(all_ys,
     TARGET_TICKS)`.
   - `#[cfg(test)] fn x_domain(&self)/y_domain(&self) -> NiceScale` and `fn point_count(&self)
     -> usize` (sum of series lengths) accessors for tests.
   - `impl Plot for ScatterPlot::paint`: compute plot rect (`VALUE_AXIS_GUTTER` left,
     `PLOT_RIGHT_GAP` right, `PLOT_TOP_GAP` top, `AXIS_GAP` bottom, same constants as
     line/area). Build X + Y `ScaleLinear`. Paint `Grid` (x = x-tick pixels, y = y-tick
     pixels) with the shared `GRID_STROKE` dashed style. Paint `PlotAxis` with numeric X
     labels (centered) + numeric Y labels (right-aligned). Then for each series, for each
     point, draw a dot quad (fill = series color, thin white stroke) at `(x_scale.tick(x),
     y_scale.tick(y))` offset by `bounds.origin`.
   - `pub fn scatter_element(chart: &Chart) -> Option<gpui::AnyElement>` = `chart_frame(chart,
     ScatterPlot::from_chart(chart)?.into_any_element())`.
2. **`chart-render/src/lib.rs`.** Add `pub mod scatter;` and dispatch
   `ChartKind::Scatter => scatter::scatter_element(chart)` in `chart_element`.
3. **`chart-render/src/scenes.rs`.** Add two scene builders + register in `all()`:
   - `scatter_single()` — one xy series (e.g. Ad spend vs Revenue, ~10 points, positive
     trend), title + both numeric axis titles + legend. `DEFAULT_VP`.
   - `scatter_multi()` — three xy series with distinct clusters (iris-style: Setosa /
     Versicolor / Virginica, petal length vs petal width), distinct colors + legend.
     `WIDE_VP`. Each with `description` + `expectation` review metadata (both numeric axes
     labeled, dots in the right places, legend mapping, no clipping).
4. **Capture + manifest.** Rebuild bins; run `capture --scene scatter` to render both PNGs
   into `results/` and regenerate `manifest.json` (auto-generated from scenes).
5. **Review.** Spawn a fresh reviewer sub-agent per PNG (nested spawn) — or Read-tool
   fallback — PASS/MARGINAL/FAIL + §6 rubric notes; append a Phase-3 section to
   `results/review.md`.
6. **findings.md.** Append a Phase 3 section answering the "modest addition or painful?"
   question with the per-scene verdicts.

## Tests

- `scatter::rejects_non_scatter_and_empty` — `from_chart` returns `None` for a non-scatter
  chart and for a scatter with no xy series.
- `scatter::shared_domains_cover_all_points` — the X domain covers every series' every
  x-value and the Y domain covers every series' every y-value (the core "shared numeric
  domains over the union" property).
- `scatter::point_count_matches_data` — `point_count()` equals the total number of xy pairs
  across all series (dot-count == data).
- `scatter::multi_series_has_distinct_colors` — the three series resolve to distinct colors.
- `scenes::gate3_scatter_scenes` — `scatter_single` is a `Scatter` with one xy series;
  `scatter_multi` is a `Scatter` with ≥2 xy series; both are name-lookupable.
- Light per relaxed rigor; the **2 PNGs + single-agent review** are the real Gate-3 evidence.
