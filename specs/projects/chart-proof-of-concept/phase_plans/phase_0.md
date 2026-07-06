---
status: complete
---

# Phase 0: Enablement (M0)

## Overview

Stand up the whole chart-poc harness and prove it end-to-end on the cheapest possible chart,
so that later phases (starting with the make-or-break Gate 1) build on a known-good foundation
instead of discovering plumbing problems mid-experiment.

"Prove end-to-end" means all of: the pinned gpui / gpui-component pair builds under the
experiment's toolchain; a FreeCell-owned chart widget built over gpui-component's `plot/`
primitives paints; the widget renders **headless** to a non-blank PNG in this Linux container;
and a reviewer agent, given the PNG + an expectation, confirms it is "a bar chart." The chart
itself is deliberately trivial — one single-series column chart. The real deliverable is the
harness plus the early finding of whether headless capture can be made to work here at all.

## Steps

1. **Scaffold `experiments/chart-poc/`** as a small workspace (so `chart-model` can be one
   shared crate). Add `Cargo.toml` (workspace), `rust-toolchain.toml` pinning `1.95.0` (the
   app's pin doesn't reach `experiments/`), `.gitignore` (`/target`), and `README.md`.
   Mirror the known-good pins from `app/Cargo.toml`: gpui/gpui_platform @ zed
   `1d217ee3…` (gpui_platform features font-kit/x11/wayland/runtime_shaders),
   gpui-component/-assets @ `a9a7341c…`, `image` 0.25, `png` 0.17; set
   `profile.dev.package.gpui* opt-level = 3`.

2. **`chart-model` crate** (the §2 OOXML-shaped data model) — gpui-free, ironcalc-free:
   `Color` (hex round-trip), `BarDir`, `Grouping`, `ChartKind` (Bar/Line/Area/Pie/Scatter),
   `Category` (Text|Number), `SeriesData` (CategoryValue|Xy), `Series`, `Axis`, `Legend`,
   `Chart`. Path-dependency of `chart-render` (and, later, `load-save`).

3. **`chart-render` crate**, wired to the pinned gpui/gpui-component pair:
   - `ticks.rs` — a "nice numbers" value-axis generator (`NiceScale`) the library's
     `ScaleLinear` doesn't provide; pure + unit-tested.
   - `palette.rs` — the `chart_1..chart_5` color cycle, extended past 5 by hue rotation.
   - `scenes.rs` — example scene table + capture metadata (`description`, `expectation`);
     Phase 0 defines exactly one `bar_single` scene.
   - `bar.rs` — `BarPlot`, a `#[derive(IntoPlot)] + impl Plot` over the raw `Bar` +
     `ScaleBand` + `ScaleLinear` + `PlotAxis` + `Grid` primitives, feeding it OUR `NiceScale`
     domain so bars and ticks share one scale; plus `chart_element()`, the owned wrapper
     (title, axis titles, legend) as gpui `div` layout.
   - `render.rs` — `run_render_scene(name, exit_after_ms)`: open one viewport-sized gpui
     window hosting the chart in a gpui-component `Root`, self-quit on a timer (adapted from
     `app/render-tests/src/render.rs`).
   - `capture.rs` — the headless capture harness adapted from `app/render-tests/src/capture.rs`
     (per-scene Xvfb + lavapipe, `xrefresh`, screenshot the window by id with `import`), plus a
     `manifest.json` writer and a blank-guard.
   - Bins `render_scene` and `capture`.

4. **Install the container prerequisites** the capture path needs and record them:
   `mesa-vulkan-drivers` (lavapipe ICD — the Vulkan loader ships but there is no driver),
   `x11-xserver-utils` (`xrefresh`), `x11-utils` (`xwininfo`), `imagemagick` (`import`), and
   `libxkbcommon-dev` + wayland/xcb/x11 dev libs (gpui link step needs `-lxkbcommon`).

5. **Capture + review.** Run `capture` → `results/bar_single.png` + `results/manifest.json`.
   Spawn a fresh reviewer sub-agent to judge the PNG against the §6 rubric; record the verdict
   in `results/review.md`.

6. **Findings.** Write `chart-render/findings.md` (what worked / what was hard, container
   setup, the `ScaleBand` range-start gotcha).

## Tests

- `chart-model`: `color_hex_round_trips`; `category_labels_render_text_and_numbers`;
  `series_len_reflects_underlying_data`; `chart_round_trips_through_accessors` (the seam's
  in-memory shape reads back what it was built with).
- `ticks`: `covers_data_with_round_step` (0..97 → step 20, ticks 0..100);
  `ticks_span_the_domain_and_are_evenly_spaced`; `value_axis_includes_zero_baseline`;
  `all_negative_values_still_include_zero`; `degenerate_inputs_do_not_panic_or_loop`;
  `fraction_maps_endpoints`; `tick_formatting_trims_zeros`.
- `palette`: `first_five_are_the_base_palette`; `beyond_five_stays_distinct_from_first_lap`;
  `hsl_round_trip_is_close`.
- `scenes`: `every_scene_is_lookupable_and_has_metadata`;
  `phase0_bar_scene_is_a_single_series_column`.
- End-to-end evidence (not a unit test): `capture` produces a non-blank
  `results/bar_single.png` of the expected size, and the reviewer agent returns PASS.
