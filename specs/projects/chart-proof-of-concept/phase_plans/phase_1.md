---
status: complete
---

# Phase 1: Gate 1 — MAKE-OR-BREAK (multi-series line)

## Overview

Gate 1 is the whole bet (functional_spec §3, §7): can we render, from `chart-model` over
gpui-component's raw `plot/` primitives, a **multi-series line chart (2–4 series)** that a
user would accept as a real chart of their data — with a **chart title + both axis titles**,
a **numeric value-axis with readable "nice" tick labels**, a **category axis**, a **legend**
(swatch + series name, correct series→color mapping), and a **multi-series color cycle**?

The known trap (`research/compare-line.md`): each stock `LineChart` normalizes its own
y-domain, so overlaying them does not share a value scale. Multi-line therefore needs the raw
`Line` primitive over **one SHARED `ScaleLinear`** whose domain covers the union of every
series' values. Excel's default line is **straight**, so segments must use `.linear()`
(`StrokeStyle::Linear`), not the primitive's `Natural` default. We own the wrapper
(title / axis-titles / legend / layout) the same way Phase 0's `bar.rs` does.

Deliverable: a Gate 1 multi-series line scene captured to a PNG + manifest, then a **3-agent
majority panel** verdict recorded in `results/review.md` (functional_spec §6, §10 decision #3).

## Steps

1. **`chart-render/src/style.rs` (new).** Extract the theme-independent color constants
   (`BACKGROUND`, `TITLE_TEXT`, `AXIS_TITLE_TEXT`, `MUTED_TEXT`, `AXIS_STROKE`, `GRID_STROKE`)
   and the `hsla(u32) -> Hsla` / `model_hsla(ModelColor) -> Hsla` helpers out of `bar.rs` into
   one shared module, so `bar.rs`, `line.rs`, and `chrome.rs` all use the same palette. No
   behavior change for bar.

2. **`chart-render/src/chrome.rs` (new).** Extract the chart *frame* (title + value-axis-title
   caption + `[plot | legend]` row + category-axis title) and the legend builder out of
   `bar.rs::chart_element` into `chrome::chart_frame(chart: &Chart, plot: AnyElement) ->
   AnyElement`. The legend lists **every** series with `series[i].color.unwrap_or(series_color(i))`
   — the exact color each mark uses, so legend↔mark mapping is correct by construction. Now
   that line draws all series, this legend is finally correct for the multi-series case.

3. **`chart-render/src/ticks.rs`.** Add `NiceScale::spanning(values, target_ticks)`: a nice
   scale over the data's actual min..max (NOT forced to zero) — Excel's auto-ranging value axis
   for line charts (`research/compare-line.md`: the value axis auto-ranges and does not force
   zero). This is the SHARED value domain for all line series. Reuses `NiceScale::new`.

4. **`chart-render/src/line.rs` (new).** The multi-series line widget:
   - `LineSeries { values: Vec<f64>, color: Hsla }` and
     `#[derive(IntoPlot)] LinePlot { categories: Vec<SharedString>, series: Vec<LineSeries>,
     scale: NiceScale }` — `scale` is the ONE shared value domain.
   - `LinePlot::multi_series(chart) -> Option<Self>`: accept `ChartKind::Line`; take categories
     from the first `CategoryValue` series; collect every series' values; assign colors
     `series[i].color.unwrap_or(series_color(i))` (matches the legend); compute the shared
     `NiceScale::spanning` over **all** series' values (target 5 ticks).
   - `impl Plot::paint`: category axis via `ScalePoint` (endpoints inset from the frame so end
     dots/labels breathe — `ScalePoint` respects its range start, unlike `ScaleBand`); shared
     value axis via `ScaleLinear::new([scale.min, scale.max], [plot_bottom, plot_top])`;
     dashed gridlines + value labels (right-aligned) + category labels (centered) exactly like
     `bar.rs`; then one `Line` per series with `.stroke(color).stroke_width(px(2))
     .stroke_style(StrokeStyle::Linear).dot()` (small dots filled with the series color, thin
     white stroke). Precompute per-category x pixels once and share them across series' closures.
   - `pub fn line_element(chart) -> Option<AnyElement>` = `chart_frame(chart, LinePlot::…)`.

5. **`chart-render/src/bar.rs`.** Trim to the plot + `pub fn bar_element(chart) -> Option<..>`
   that calls `chrome::chart_frame`; drop the now-shared color consts/legend (moved to
   `style.rs`/`chrome.rs`). Behavior identical.

6. **`chart-render/src/lib.rs`.** Add `mod style; mod chrome; mod line;` and a top-level
   `pub fn chart_element(chart: &Chart) -> Option<AnyElement>` dispatching on `chart.kind`
   (`Line` → `line::line_element`, `Bar` → `bar::bar_element`).

7. **`chart-render/src/render.rs`.** `ChartView::render` calls `crate::chart_element` (was
   `bar::chart_element`) so both bar and line scenes render through one entry point.

8. **`chart-render/src/scenes.rs`.** Add the Gate 1 scene `line_multi` (3 series — North/South/
   West — over Jan–Jun, values that cross, so multi-series is unmistakable) with a description +
   an `expectation` covering all seven rubric points, and a supporting single-series
   `line_single` sanity scene. Both `ChartKind::Line { grouping: Standard, smooth: false }`.

## Tests

- `ticks::spanning_covers_data_without_forcing_zero` — `spanning([30,95],5)` spans the data
  (min ≤ 30, max ≥ 95) and does NOT snap min to 0 when data is far from zero; ticks sane.
- `line::shared_scale_covers_all_series` — build `LinePlot::multi_series` from the Gate 1
  chart; assert the shared `scale` domain `[min,max]` covers **every** value of **every**
  series (the core "one shared domain across all series" property).
- `line::multi_series_reads_all_series_and_categories` — the plot keeps all 3 series and the
  6 categories (not just series[0]).
- `line::rejects_non_line_and_empty` — `multi_series` returns `None` for a bar chart / no series.
- `scenes::gate1_line_scene_is_multi_series_line` — `line_multi` is `ChartKind::Line`, ≥2
  series, shared category count.
- Existing `scenes` / `ticks` / `palette` / `chart-model` tests still pass (structural refactor
  must not regress Phase 0's bar path).
- **Real evidence (relaxed rigor):** the captured `results/line_multi.png` + the 3-agent panel
  in `results/review.md` + `findings.md`, not test coverage.
</content>
</invoke>
