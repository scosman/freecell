---
status: complete
---

# Phase 5: Line renderer (production)

## Overview

P5 hardens the **line** renderer in `freecell-app::chart` from the PoC seed (lifted verbatim
in P1) to production quality, and grows the chart render-test suite from the single seed scene
(`chart_line_multi`) to a set that covers the production line features the plan calls out:
**multi-series on one shared value scale, nice-tick numeric axes, legend, title, axis titles**
— each exercised both present *and* absent so the chrome is honestly driven by the model, not
always-on.

Scope discipline: the plan reserves the P1-fidelity line features — **theme colors, rotated
vertical value-axis title, `numFmt` ticks, `c:marker` styling, `c:smooth`** — for **P6**, and
axis scaling / gridline toggles / `a:ln` styling / legend *positions* for **P13**. P5 therefore
does **not** implement those. It closes the gaps that make the seed *not* production quality:

1. The seed's `chart_frame` (in `chart/chrome.rs`) draws a legend **unconditionally** and emits
   empty title / axis-title rows even when the model has none — so a `legend: None` or untitled
   chart renders a stray legend column and blank gaps. Production must honor `chart.legend`
   presence and collapse absent title/axis-title text.
2. The seed's line `.y()` closure feeds raw values (incl. non-finite) straight to the scale, so
   a NaN/Inf value (functional_spec §7: "source range edited to non-numeric → render what's
   valid, blank the rest, no crash") would emit a bad point instead of a clean gap.

Exit (per implementation_plan): unit tests + committed render-test baselines; runs in the test
harness, not the app.

## Steps

1. **`chart/chrome.rs` — honor legend presence, collapse empty chrome, extract a testable
   legend mapping.**
   - Extract the inline series/slice→(color,name) logic from `legend_rows` into a pure
     `pub(crate) fn legend_entries(chart: &Chart) -> Vec<LegendEntry>` where
     `pub(crate) struct LegendEntry { color: u32, name: String }`. `legend_rows` becomes a thin
     `legend_entries(chart).into_iter().map(|e| legend_row(e.color, e.name))`. This is the
     load-bearing swatch↔series mapping (chrome doc) — now unit-testable without a GPU.
   - In `chart_frame`, render the legend column only `when(chart.legend.is_some(), …)`; when
     absent the plot row is just the plot (full width). Render the chart-title row only when the
     title is non-empty, and each axis-title caption only when its text is non-empty (use
     gpui `.when(!s.is_empty(), …)`), so an untitled chart has no blank rows. `captions` stays a
     pure helper (its horizontal-bar swap is unchanged).
   - No signature change to `chart_frame` — every other kind (`bar`/`area`/`pie`/`scatter`) keeps
     calling it; a chart that has a title + both axis titles + a legend (e.g. `chart_line_multi`)
     renders the identical element tree, so its baseline does not move.

2. **`chart/line.rs` — blank non-finite points.**
   - In the per-series `Line` builder, change the `.y()` closure from
     `value_scale.tick(&values[*i])` to skip non-finite values:
     `let v = values[*i]; v.is_finite().then(|| value_scale.tick(&v)).flatten()`. The
     gpui-component `Line` primitive already skips points whose `x`/`y` is `None` (draws a gap,
     no panic — `plot/shape/line.rs`), so a NaN/Inf value blanks cleanly. The shared
     `NiceScale::spanning` already ignores non-finite when computing the domain.

3. **`render-tests/src/chart_scene.rs` — add production line scenes.** Add four scene builders
   and append them to `all()` (keeping `chart_line_multi`):
   - `chart_line_single` — a single-series line (title, both axis titles, one-entry legend);
     proves the single-series path + one-row legend.
   - `chart_line_negative` — a two-series line whose shared domain **crosses zero** (negative and
     positive values); proves the nice-tick numeric value axis over a zero-crossing shared scale
     (a `0` tick + negative tick labels), all chrome present.
   - `chart_line_no_legend` — a two-series line with `legend: None`; proves the legend is honored
     (no legend column; plot uses the full width).
   - `chart_line_no_titles` — a three-series line with `title: None` and untitled axes, legend
     present; proves the title/axis-title rows collapse while the legend still renders.

4. **`render-tests/tests/render_suite.rs` — register the new scenes.** Add the four new names to
   the `chart_render_cases!` macro list (each becomes one `#[test]`); the existing
   `chart_scene_names_match_table` drift guard keeps the macro list and `chart_scene::all()` in
   lockstep.

5. **Generate + eyeball baselines.** `render_tests.sh generate --only chart_`, then Read each
   new/changed PNG and confirm it is a correct line chart (title where expected, nice-tick axes,
   multi-series on one shared scale, legend where expected, axis titles where expected, the
   collapse cases genuinely collapsed). Commit the baselines with the code.

## Tests

Unit (GPU-free):
- `chrome::tests::legend_entries_map_multi_series_line` — three entries, names North/South/West,
  colors are the first three distinct palette entries (swatch↔series correct by construction).
- `chrome::tests::legend_entry_honors_explicit_series_color` — a series with an explicit color
  uses it, not the palette.
- `chrome::tests::legend_entry_names_unnamed_series` — an unnamed series → "Series N".
- `chrome::tests::legend_entries_are_per_slice_for_pie` — a pie's entries are its categories with
  slice colors (one per slice), not one per series.
- `chrome::tests::captions_swap_for_horizontal_bar` — value/category caption order swaps only for
  `BarDir::Bar`; a normal chart keeps value-above/category-below; untitled axes → empty strings.
- `line::tests::non_finite_values_do_not_break_the_scale` — a line series containing NaN/Inf still
  builds a plot, and the shared scale stays finite and covers the finite values (blank-the-rest,
  no panic).
- `line::tests::single_series_line_builds` — a one-series line builds a plot with one series.
- Existing `line::tests` (`shared_scale_covers_all_series`, `multi_series_reads_all_series_and_categories`,
  `rejects_non_line_and_empty`) stay green.

Scene-table (GPU-free, in `chart_scene.rs`):
- Extend `every_scene_is_lookupable_and_nonempty` coverage implicitly (all new scenes flow through
  it) and add `production_line_scenes_cover_their_features` — asserts `chart_line_no_legend` has
  `legend: None`, `chart_line_negative` carries a negative value, `chart_line_no_titles` has
  `title: None` + untitled axes, and every `chart_line_*` scene is a `Line` kind.

Render (pixel, `chart_` subset — full suite deferred to the manager's late validation phase):
- `chart_line_multi` (unchanged baseline), `chart_line_single`, `chart_line_negative`,
  `chart_line_no_legend`, `chart_line_no_titles` — each diffs green against its committed,
  eyeballed baseline.
</content>
</invoke>
