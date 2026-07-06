---
status: complete
---

# Phase 2: Gate 2 — harder layouts (bar family, stacked area, pie/doughnut)

## Overview

Gate 1 (Phase 1) answered the make-or-break render-quality question YES on the raw
primitives. Gate 2 (functional_spec §3 table, §7) stresses the layouts that carry the
research-flagged traps and decides whether the follow-on project can offer the full
in-scope type set (GO) or a bounded subset (PARTIAL-GO). We must render, from `chart-model`
over gpui-component's `plot/` primitives, reusing the Phase 0/1 shared chrome / palette /
ticks:

1. **Single-series column** (have it) **and horizontal bar** (`BarDir::Bar` — axes swapped:
   category on Y, value on X).
2. **Grouped (clustered) column**, multi-series — no grouped helper in the primitives; DIY
   by sub-dividing each category slot across series.
3. **Stacked column** and **100%-stacked column** — cumulative baselines (hand-rolled, the
   `Stack` primitive only computes `y0/y1` math and does not paint); percent needs a
   per-category normalize pass; value axis reflects stacked totals (or 0–100%).
4. **Stacked area** (the nastiest) and **100%-stacked area** — the `Area` primitive has a
   **scalar** `y0` baseline, so true stacked area needs hand-rolled filled polygons (trace
   the band's upper boundary forward, the lower boundary back, close). Percent = same
   normalize pass.
5. **Pie** and **doughnut** (`doughnut_hole`) — no auto-palette, so synthesize per-slice
   colors from the categorical palette; legend maps slice→color; doughnut is pie with an
   inner radius.

Each new scene is captured to `results/<name>.png` + `manifest.json`, then **single-agent**
reviewed (Gate 2 uses single verdicts, not the Gate-1 3-panel) against the §6 seven-point
rubric, appended to `results/review.md`. Checkpoint semantics: some MARGINAL is acceptable;
wholesale grouped/stacked FAIL → PARTIAL-GO signal (report straight, do not fudge).

## Design notes / key primitive facts (verified against the pinned rev)

- **`Bar<T>`** paints from `.base(px)` to `.value(px)` at cross-position `.cross(px)` with
  `.band_width(w)`, and `BarAlignment` selects orientation (`Bottom` vertical / `Left`
  horizontal). Because cross/base/band_width are all caller-supplied, both grouping (offset
  sub-bars) and stacking (per-segment base/value) are expressible. `ScaleBand::band_width()`
  is **capped at 30px**, so grouped/stacked geometry is computed **manually** (category slot
  = span/n) rather than via `ScaleBand`, for full control.
- **`Stack<T>`** is a d3-shape port that returns cumulative `(y0,y1)` — a data transform, not
  a painter. Its cumulative math is trivial to inline; we hand-roll the running totals so the
  same code serves bars and areas and the percent normalize pass.
- **`Area<T>`** closes its fill with a flat bottom edge at a scalar `y0` — cannot draw a wavy
  stacked baseline. So stacked area is **hand-rolled polygons** via `gpui::PathBuilder::fill`
  (upper boundary forward, lower boundary reversed, close), exactly the research-recommended
  approach.
- **`Pie<T>`/`Arc`** — `Pie::arcs(&data)` computes slice angles (drops values ≤ 0, sweeps
  from `start_angle`); `Arc::paint(arc, color, inner_radius, outer_radius, bounds, window)`
  draws one slice centered on the passed bounds. No auto-palette → we pass `series_color`
  per slice. Doughnut = pie with `inner_radius = doughnut_hole × outer_radius`.
- **Axis/grid orientation** (`PlotAxis`/`Grid`): vertical bars → value axis on Y (left,
  `Grid.y`), category axis on X (bottom); horizontal bars → value axis on X (bottom,
  `Grid.x`), category axis on Y (left). The axis line spans full bounds; category slot math
  offsets from the plot rect.

## Steps

1. **`chart-render/src/bar.rs` — generalize to the whole bar family.** Replace the
   single-series-column-only `BarPlot` with a `BarPlot` that carries `dir: BarDir`,
   `grouping: Grouping`, `Vec<BarSeries { values, color }>`, categories, a `NiceScale` value
   domain, and a `percent: bool` flag.
   - `BarPlot::from_chart(chart) -> Option<Self>`: accept any `ChartKind::Bar`. Collect
     every category/value series (categories from the first). Resolve colors via
     `series[i].color.unwrap_or(series_color(i))`.
   - Value domain: `Clustered` → `NiceScale::for_values` over ALL series values (each bar
     independent, forced-zero). `Stacked` → `for_values` over per-category **sums** (covers
     the tallest stack). `PercentStacked` → fixed `NiceScale::new(0.0, 100.0, 5)`.
   - `paint`: compute plot rect (orientation-aware gutters — wider gutter on the category
     side for horizontal so labels fit). Map value→pixel with `ScaleLinear` along the value
     axis (Y for `Col`, X for `Bar`). Category slot geometry computed manually:
     `slot = span / n`, `center_i = span_start + slot*(i+0.5)`, `group_width = slot *
     GROUP_FILL`. Draw value-tick grid + axis + labels (orientation-aware) and category-axis
     labels. Then:
     - **Clustered:** `sub_w = group_width / n_series`; series `j` bar at
       `group_start + j*sub_w` (small inner gap), `band_width = sub_w`, base = value-0 pixel,
       value = `value_scale(v)`; one `Bar` per series.
     - **Stacked / Percent:** one column of `group_width` per category; walk series
       accumulating running totals (normalized to 100 for percent); each series segment is a
       `Bar` from `base = value_scale(cum_lo)` to `value = value_scale(cum_hi)`.
   - `bar_element(chart)` builds any bar variant, wrapped in `chart_frame`.
2. **`chart-render/src/area.rs` (new).** `AreaPlot` for `ChartKind::Area` (Standard /
   Stacked / PercentStacked). Reuse the `ScalePoint` category axis + shared `ScaleLinear`
   value axis pattern from `line.rs`. Compute per-category cumulative `(lo,hi)` per series
   (Standard = each from 0; Stacked = running totals; Percent = normalized to 100). Value
   domain: Standard → span of individual values (forced zero via `for_values`); Stacked →
   `for_values` over per-category sums; Percent → 0–100. Paint each band as a hand-rolled
   filled polygon (semi-transparent fill + solid top stroke) via `PathBuilder::fill` /
   `paint_path`, plus grid + axis + category labels. `area_element(chart)`.
3. **`chart-render/src/pie.rs` (new).** `PiePlot` for `ChartKind::Pie { doughnut_hole }`.
   One slice per category of the first series (pie is single-series). Synthesize a per-slice
   color from `series_color(i)` (the whole point — no auto-palette). Compute
   `outer_radius = 0.4 × min(w,h)`, `inner_radius = doughnut_hole × outer_radius` (0 for
   solid pie). Use `Pie::arcs` for angles + `Arc::paint` per slice. Legend maps
   slice→color: the chrome legend keys off series, so for pie build the legend rows here (or
   pass a per-slice legend) — pie's "series" are the categories, so add a pie-aware legend in
   `chrome.rs` keyed on the first series' categories.
4. **`chart-render/src/chrome.rs`** — add a pie/doughnut legend that lists **categories**
   (one row per slice) colored by `series_color(i)`, since a pie's slices (not series) are
   what the legend maps. Keep the existing series legend for bar/line/area. Dispatch on
   `ChartKind::Pie`.
5. **`chart-render/src/palette.rs`** — expose a `slice_color(i)` (alias to `series_color`)
   used by pie so the intent reads clearly; keep the single cycle so legend and slices match.
6. **`chart-render/src/lib.rs::chart_element`** — wire `ChartKind::Area` → `area::area_element`
   and `ChartKind::Pie` → `pie::pie_element` (Bar/Line already dispatched).
7. **`chart-render/src/scenes.rs`** — add scenes: `bar_horizontal` (single-series horizontal
   bar), `bar_grouped` (3-series clustered column), `bar_stacked` (3-series stacked column),
   `bar_percent_stacked` (3-series 100%-stacked column), `area_stacked` (3-series stacked
   area), `area_percent_stacked` (3-series 100%-stacked area), `pie` (single-series pie),
   `doughnut` (single-series doughnut). Each with `description` + `expectation` metadata.
8. **Capture + review.** Build; run `capture` for all scenes → `results/<name>.png` +
   `manifest.json`. Single-agent review each new image against the §6 rubric →
   `results/review.md`. Update `chart-render/findings.md` with the Phase 2 per-type
   "what worked / was hard," especially the Area-polygon and pie-palette approaches.

## Tests (light, per relaxed rigor)

- `bar::grouped_offsets_partition_the_band` — for n series the sub-bar offsets are disjoint
  and lie within the group width (no overlap, no spill).
- `bar::stacked_baselines_are_cumulative` — successive segments' `lo/hi` chain (seg i hi ==
  seg i+1 lo) and the top equals the category total.
- `bar::percent_stacks_sum_to_100` — each category's normalized segments sum to 100.
- `bar::value_domain_matches_grouping` — clustered covers max single value; stacked covers
  max category sum; percent is 0–100.
- `bar::rejects_non_bar` — `from_chart` returns `None` for a line/pie chart.
- `area::stacked_baselines_are_cumulative` + `area::percent_sums_to_100` — same properties
  for the area cumulative math.
- `area::rejects_non_area`.
- `pie::slice_angles_sum_to_tau` — the computed slice sweep sums to 2π (within epsilon) for
  all-positive data; doughnut inner radius is `hole × outer`.
- `pie::rejects_non_pie`.
- `scenes` — each new scene is name-lookupable with non-empty metadata and the expected
  `ChartKind`.
- The **PNGs + single-agent review + `findings.md`** are the real Gate-2 evidence.
</content>
</invoke>
