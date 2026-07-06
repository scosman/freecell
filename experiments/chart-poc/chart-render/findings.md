# chart-render — findings

Phase 0 (M0 — Enablement). Goal: prove the whole harness end-to-end — the pinned
gpui / gpui-component stack builds and paints, a chart renders headless to a non-blank PNG,
and an agent reviewer confirms it. Scope kept trivial: **one single-series column chart**.

## Result: ENABLEMENT PASS

The harness works end-to-end in this Linux container:
pinned gpui/gpui-component build → chart widget over the `plot/` primitives paints →
headless capture to `results/bar_single.png` (640×440, 1447 unique colors, non-blank) →
reviewer agent verdict **PASS** ("a recognizable, non-blank bar chart"; see `results/review.md`).
The make-or-break render-quality question (multi-series line + legend a user would accept) is
Gate 1 (Phase 1), not this phase.

## What worked

- **Pinned dependency pair builds unchanged.** The exact `app/Cargo.toml` pins
  (gpui/gpui_platform @ zed `1d217ee3…`, gpui-component/-assets @ `a9a7341c…`, `image` 0.25,
  `png` 0.17) build here with the 1.95.0 toolchain and `profile.dev.package.gpui* opt-level = 3`.
  First build is slow (heavy git deps), as expected. A `rust-toolchain.toml` pinning `1.95.0`
  under `chart-poc/` was required — the app's pin does not reach `experiments/`.
- **Building over the `plot/` primitives is viable.** `bar.rs::BarPlot` is a
  `#[derive(IntoPlot)] + impl Plot` over the raw `Bar` + `ScaleBand` + `ScaleLinear` +
  `PlotAxis` + `Grid` primitives — the pattern the library's own hand-rolled `stacked_bar_chart`
  story uses ("You can draw any chart you want by using the `Plot`"). So Phase 0 already
  exercises the Gate 1 approach rather than leaning on the stock `BarChart` struct.
- **We own the value axis.** `ticks.rs::NiceScale` (Heckbert "nice numbers") generates the
  rounded domain + tick step that the library's `ScaleLinear` does NOT provide. Feeding that
  domain into `ScaleLinear::new(vec![nice.min, nice.max], vec![bottom, top])` makes the bars
  and the tick labels share one scale, so they line up — the fix for the research-flagged
  "each stock chart normalizes its own domain, no nice ticks" trap. The captured chart shows
  0/50/100/150/200 at even intervals with gridlines, bars landing correctly against them.
- **We own title / axis titles / legend.** These live in a plain gpui `div` layout around the
  plot element (the stock structs have none). Legend swatch colors come from the same
  `palette.rs` cycle the bars use, so series→color mapping is correct by construction.
- **Theme-independent colors.** The widget passes explicit colors to the plot primitives
  instead of reading `cx.theme()`, so the capture is deterministic and high-contrast regardless
  of the ambient (possibly dark) gpui-component theme.

## Headless capture — the key risk, retired

gpui has **no windowless GPU capture on Linux** at the pinned rev (that path is macOS/Metal
only — see `round-3/C-ci-rendering/findings.md`). So we reuse the repo's proven **on-screen**
path (`app/render-tests/src/capture.rs`), copied into `capture.rs`: a real gpui window under
`xvfb-run` + Mesa **lavapipe** (software Vulkan), `xrefresh` to force presentation, screenshot
the window by id with ImageMagick `import`.

**Container setup needed (was missing; installed via apt):**

    apt-get install -y mesa-vulkan-drivers x11-xserver-utils x11-utils imagemagick
    # and, for the gpui link step (below):
    apt-get install -y libxkbcommon-dev libwayland-dev libxcb1-dev libx11-dev

- The Vulkan **loader** (`libvulkan.so.1`) ships in the base image, but there was **no ICD /
  driver** — `/usr/share/vulkan/icd.d/` did not exist. `mesa-vulkan-drivers` installs the
  lavapipe ICD (`lvp_icd.json`); `vulkaninfo` under Xvfb then reports an `llvmpipe` device.
- `xrefresh` (x11-xserver-utils), `xwininfo` (x11-utils), and `import` (imagemagick) were all
  missing from the base image and are all load-bearing for the capture path.
- **Linking gpui failed until `libxkbcommon-dev` was installed** (`rust-lld: unable to find
  library -lxkbcommon`); gpui_platform's x11/wayland backends link against it. Installed the
  wayland/xcb/x11 dev libs alongside it.

**Load-bearing details inherited from the proven path:** each scene renders under its **own**
Xvfb display sized to the viewport (+8px) — lavapipe only *presents* when the window nearly
fills the screen; `xrefresh` forces the Expose gpui needs to present (Xvfb has no compositor);
a blank-guard rejects any capture with ≤1 unique color.

## What was hard / notable

- **Discovering the container gaps.** The base image had the Vulkan loader but no driver, none
  of `xrefresh` / `xwininfo` / `import`, and no `libxkbcommon-dev` for the gpui link. These are
  the things a fresh container needs before capture works; now documented in `README.md`.
- **`ScaleBand::tick` ignores its range start (a real gotcha, found and fixed).** It positions
  bands from `0`, not from `min(range)`. Passing `[plot_left, plot_right]` therefore slid the
  bars left into the value-axis gutter, and the bars (painted last) covered the 50/100 tick
  labels. Fix: give the band scale the available *width* (`[0, plot_right-plot_left]`) and add
  `plot_left` to every band position (bar cross + category label x). After the fix all five
  value labels render and the bars sit correctly right of the axis. (This is exactly how the
  library's own `BarChart` uses a 0-based band range — the value gutter is our addition.)
- **Primitive API is closure/`'static`-heavy.** `Bar`'s accessor closures are `'static`, so the
  widget must move owned clones (`ScaleBand`/`ScaleLinear` are `Clone`) into them rather than
  borrow `self` — mirrors how the library's own `BarChart::paint` does it.
- **Value type locked to `f64`.** `ScaleLinear`'s value type is a sealed trait (`f64` or
  `Decimal` only). Our data model already stores values as `f64`, so a non-issue for us.
- **Paint order matters.** Grid → axis+labels → bars. Bars paint last, so anything a bar
  overlaps is covered (this is what surfaced the band-range bug above).

## Agentic review

`capture` writes `results/manifest.json` (`{name, png, description, expectation}` per image).
A fresh reviewer sub-agent is given each PNG + its `expectation`, judges it against the §6
rubric, and the verdict is recorded in `results/review.md`. Phase 0 = single verdict ("a bar
chart, non-blank"); Gate 1 upgrades this to a 3-agent majority panel on the make-or-break image.

## Tests (light, per relaxed rigor)

- `chart-model`: color hex round-trip, category labels, series length, whole-chart round-trip
  through the public API (the seam's in-memory shape). 4 tests.
- `ticks`: nice-scale covers the data with a 1/2/5×10^k step; ticks span the domain evenly;
  value axis includes the zero baseline; degenerate inputs don't panic/loop; fraction mapping;
  tick formatting.
- `palette`: first five = the base cycle; >5 stays distinct; HSL round-trip.
- `scenes`: every scene is name-lookupable with non-empty metadata; the Phase-0 scene is a
  single-series column.
- The **PNG + agent review** in `results/` is the real evidence, not test coverage.

---

# Phase 1 (Gate 1 — MAKE-OR-BREAK): multi-series line

Goal: the whole bet (functional_spec §3, §7) — a **multi-series line chart (2–4 series)** from
`chart-model` that a user would accept, with a chart title + both axis titles, a numeric value
axis with nice ticks, a category axis, a legend (correct series→color mapping), and a
multi-series color cycle. Built on the raw `Line` primitive over **one shared `ScaleLinear`**,
straight segments (`.linear()`), owning the wrapper the way `bar.rs` does.

## Result: GATE 1 PASS

`results/line_multi.png` (720×460) — a 3-series line (North/South/West over Jan–Jun) whose
lines cross, on one shared 20–100 value axis. The **3-agent majority panel** (three independent
fresh reviewers, §10 decision #3) returned **PASS / PASS / PASS → majority PASS**; all three
agreed YES on every one of the §6 seven rubric points (see `results/review.md`). Supporting
`line_single.png` (single line) also reads cleanly. **The make-or-break question is answered
yes: acceptable multi-series line charts are buildable on the primitives. The PoC proceeds to
Gate 2 — it is not a NO-GO.**

## What worked

- **The raw `Line` primitive shares a scale trivially.** The research trap ("each stock
  `LineChart` normalizes its own y-domain, so overlays don't share a scale") only bites the
  `LineChart` *struct*. The `plot::shape::Line` primitive takes plain `x`/`y` accessor
  closures (`Fn(&T) -> Option<f32>`) and draws whatever pixels you hand it — it has **no
  domain of its own**. So multi-series-on-one-scale is just: compute one shared value domain,
  build one `ScaleLinear` from it, and give every series' `Line` the *same* `value_scale`
  closure. `LinePlot::paint` loops the series and paints N `Line`s against the shared scale.
  No fork, no patch — the primitive composes as hoped.
- **The shared value domain is `NiceScale::spanning` over the union of ALL series' values.**
  New in `ticks.rs`: unlike the bars' `for_values` (which forces zero, correct for bars),
  `spanning` snaps to the data's actual min..max — Excel's auto-ranging line value axis
  (`research/compare-line.md`), and it zooms the axis to where the data is (our data 32–91 →
  a clean 20–100 with 20/40/60/80/100 ticks). A unit test asserts the shared domain covers
  every value of every series — the core Gate-1 property.
- **Straight segments = `StrokeStyle::Linear`.** The primitive defaults to `Natural` (a
  Catmull-Rom spline — the *curved* look). Excel's default line is straight, so we pass
  `.stroke_style(StrokeStyle::Linear)`. The panel confirmed straight, non-smoothed segments.
- **`ScalePoint` for the category axis — no `ScaleBand` gutter bug here.** The Phase-0 gotcha
  (`ScaleBand::tick` ignores its range start) does **not** apply: `ScalePoint` *does* honor its
  range start (`range_start + i*range_tick`), so we hand it the true `[plot_left+inset,
  plot_right-inset]` pixel range directly and the points land correctly. A small `POINT_INSET`
  keeps the first/last dots + their centered labels off the axis line / frame edge.
- **Legend↔mark mapping correct by construction.** Both the legend (in `chrome.rs`) and each
  line resolve color the same way: `series[i].color.unwrap_or(series_color(i))`. Same index,
  same source → the swatch is always the line's color. All three reviewers confirmed the
  mapping. `series_color` is the Tableau-style categorical cycle from Phase 0 (NOT the
  monochrome-blue `chart_1..5`), so the three lines are genuinely distinct hues.
- **Shared chrome, one dispatch point.** Extracted the title/axis-title/legend frame out of
  `bar.rs` into `chrome::chart_frame(chart, plot_element)` and the colors into `style.rs`, so
  bar and line render identical chrome. `lib.rs::chart_element` dispatches on `ChartKind`
  (Line → line, Bar → bar); `render.rs` calls that one entry point. Phase 0's bar path is
  unchanged (its scene still captures + passes).

## What was hard / notable

- **`Line::paint` vs `Bar::paint` signature mismatch.** `Bar::paint(&bounds, window, cx)` takes
  the `App`; `Line::paint(&bounds, window)` does **not** (it only paints a path + dot quads).
  Minor, but you can't copy the bar call shape verbatim.
- **`'static` accessor closures, as in Phase 0.** `Line`'s `x`/`y` are `'static`, so each
  series' closures must own their data. We precompute the per-category x pixels **once**
  (`xs: Vec<f32>`, shared across series) and move a clone of `xs` + the series `values` + the
  cloned `value_scale` into each `Line`'s closures. Clean and avoids a per-point category
  lookup.
- **Value-axis title orientation (cosmetic, non-defect).** Two reviewers noted the value-axis
  title sits horizontally *above* the axis rather than rotated vertically alongside it. gpui
  text has no cheap rotation here, and the horizontal caption is legible and correctly
  associated (no rubric point docked). A follow-on ship-quality project could rotate it; not
  worth it for the PoC.
- **Axis not forced to zero — deliberate.** A line value axis starting at 20 (not 0) is Excel's
  behavior and reads fine; reviewers accepted it. (Bars still force zero, which *is* correct
  for bars — the two kinds legitimately want different value domains, hence `spanning` vs
  `for_values`.)

## Tests added (light, per relaxed rigor)

- `ticks::spanning_covers_data_without_forcing_zero` — `spanning` covers both data ends and does
  NOT snap to zero when data is far from it; empty input is a safe unit scale.
- `line::shared_scale_covers_all_series` — the one shared domain contains every value of every
  series (and is zoomed, not zero-forced).
- `line::multi_series_reads_all_series_and_categories` — all 3 series + all categories kept;
  series colors distinct.
- `line::rejects_non_line_and_empty` — `multi_series` returns `None` for a bar chart / no series.
- `scenes::gate1_line_scene_is_multi_series_line` — `line_multi` is a Line, ≥2 series, shared
  category count. (21 unit tests total across the crate + `chart-model`, all passing.)
- The **`line_multi.png` + 3-agent panel** in `results/` is the real Gate-1 evidence.

---

# Phase 2 (Gate 2 — harder layouts): bar family, stacked area, pie/doughnut

Goal (functional_spec §3 table, §7): the layouts that carry the research-flagged traps —
single-series **column** + **horizontal bar**, **grouped (clustered)** column, **stacked** +
**100%-stacked** column, **stacked** + **100%-stacked area**, and **pie** + **doughnut** with a
synthesized palette — all from `chart-model`, reusing the Phase 0/1 shared chrome / palette /
ticks. Single-agent review each (Gate 2 is not the Gate-1 3-panel).

## Result: GATE 2 PASS (9/9 scenes PASS)

Every new scene rendered as a chart a user would accept; a fresh single-agent reviewer returned
**PASS** on all nine (`results/review.md`): `bar_horizontal`, `bar_grouped`, `bar_stacked`,
`bar_percent_stacked`, `area_stacked`, `area_percent_stacked`, `pie`, `doughnut`, plus a
re-review of `bar_single` after `bar.rs` was generalized. **No wholesale grouped/stacked FAIL →
this is a GO signal for the harder layouts, not a PARTIAL-GO.** The two research-named hard
problems — stacked *area* (scalar-baseline `Area` primitive) and the pie *no-auto-palette* crux
— both came out clean.

## What worked

- **One `BarPlot` covers the whole bar family.** `bar.rs` was generalized from Phase 0's
  single-series-column-only widget into a `BarPlot { dir, grouping, series, scale, percent }`
  that renders `Col`/`Bar` × `Clustered`/`Stacked`/`PercentStacked`. The raw `Bar` primitive
  takes caller-supplied `cross` (category-axis pixel), `base`/`value` (value-axis pixels), and
  `band_width`, and a `BarAlignment` selects orientation — so the SAME `cross`/`value` closures
  serve both orientations (only the alignment differs), because the shared `Geometry` already
  maps the value axis to Y (columns) or X (bars) and the category axis to the other.
- **Grouped/stacked geometry is DIY and computed manually — NOT via `ScaleBand`.** `ScaleBand`
  is a trap here: its `band_width()` is hard-capped at 30px (`plot/scale/band.rs:37`), far too
  narrow for a multi-series cluster. So the slot math is our own: `slot = plot_span /
  n_categories`, `center_i = span_start + slot*(i+0.5)`, cluster occupies `slot*GROUP_FILL`. For
  **clustered**, that group is sub-divided across series (`sub_w = group_width/n_series`, each
  bar `SUB_BAR_FILL` of its sub-slot) — a unit test asserts the sub-bars are disjoint and inside
  the group. For **stacked**, one column of `group_width` holds the cumulative segments.
- **Stacking math is one shared, gpui-free module (`stacking.rs`).** `stacked_segments` +
  `percent_segments` + `category_totals` produce the per-(series, category) cumulative `(lo,hi)`
  that both the stacked bars and the stacked area consume. (gpui-component's `Stack` primitive
  computes the same numbers but paints nothing and has **no percent mode**, so inlining the ~10
  lines is simpler than adapting it and lets bars + areas share one implementation + the percent
  normalize pass.) The value axis reflects the grouping: `for_values` over single values
  (clustered), over per-category **sums** (stacked, so the axis reaches the tallest stack), or a
  fixed 0–100 (percent, with `%`-suffixed tick labels).
- **Stacked area = hand-rolled filled polygons (the `Area`-fork the research called for).** The
  `Area` primitive closes its fill with a **flat** bottom edge at a scalar `y0` (`area.rs`), so
  it *cannot* draw a stacked band's wavy per-x baseline. `area.rs` instead builds each band as a
  `gpui::PathBuilder::fill()` polygon: trace the upper boundary forward (`value_scale(seg.hi)`
  at each category x), then the lower boundary **backward** (`value_scale(seg.lo)` reversed),
  `close()`, `window.paint_path`; a solid `PathBuilder::stroke` traces the top edge. Painting
  bottom→top means each higher band sits over the one below. This is the exact
  research-recommended approach and it produced clean, correct stacks + 100%-stacks on the first
  capture — the reviewer confirmed the bands stack cumulatively rather than all rising from zero.
- **Pie/doughnut: the no-auto-palette crux is solved by synthesizing per-slice colors.** The
  stock behavior (and an unset `.color()`) paints every slice `chart_2` → a useless monochrome
  disc. A pie is single-series, so its *slices are the categories*; `pie.rs` colors slice `i`
  with `slice_color(i)` (an alias of the categorical `series_color` cycle), and the legend in
  `chrome.rs` keys off the same function over the same categories — so slice↔swatch match by
  construction. Angles come from `Pie::arcs`, each slice painted with `Arc::paint` (centered on
  the plot bounds); **doughnut = the same with `inner_radius = doughnut_hole × outer_radius`**.
  On-slice percentage labels (via the public `plot::label::Text` + `PlotLabel`, placed at each
  slice's mid-angle) give the part-to-whole read a pie has instead of a numeric value axis.

## What was hard / notable

- **Orientation-aware chrome caption.** The shared `chart_frame` puts the value-axis title above
  the plot and the category-axis title below — correct for columns, but a **horizontal** bar has
  its value axis at the *bottom* and categories on the *left*. So `chrome.rs` now swaps the two
  captions for `Bar { dir: Bar }` (value title below, category caption above). Reviewers still
  noted the value-axis title is a horizontal caption, not a rotated vertical title — the same
  cosmetic non-defect flagged at Gate 1; legible and correctly associated, not docked.
- **`Bar` primitive `cross` is the bar's *near edge*, not its center.** Centering a clustered
  sub-bar in its sub-slot means computing the center, then subtracting `bar_w/2` for `cross`. Off
  by that half-width and the bars drift within the group. A unit test pins the partition.
- **Percent axis label formatting.** The percent variants reuse the same `NiceScale::new(0,100)`
  ticks but the value labels get a `%` suffix (a `percent` flag on the widget), so `0/20/…/100`
  reads as `0%/20%/…/100%`. Small, but needed for the axis to read as a share.
- **`Arc::paint` centers on the passed `bounds`, and its angle 0 is 12 o'clock (clockwise).**
  Convenient — the pie centers itself in the plot slot with no extra math — but on-slice label
  placement must use the same `angle - π/2` convention to land labels on the right wedge.
- **Regenerated `bar_single`.** Generalizing `bar.rs` changed the Phase-0 column render (bars are
  now `slot*GROUP_FILL*SUB_BAR_FILL` wide instead of the 30px `ScaleBand` cap). It was
  re-captured and re-reviewed → still PASS, no regression; the wider Excel-like columns read at
  least as well.

## Tests added (light, per relaxed rigor)

- `stacking`: cumulative baselines chain (seg top == next seg bottom, top == category total);
  percent segments sum to 100 per category; zero-total category collapses; negatives clamped.
- `bar`: clustered sub-bar offsets partition the band (disjoint, inside the group); stacked
  baselines cumulative + axis covers the stack total; percent sums to 100 and axis is 0–100;
  clustered axis covers the max single value (not inflated to the stack); horizontal dir carried
  through; rejects non-bar.
- `area`: stacked baselines cumulative; percent sums to 100 and axis is 0–100; standard bands all
  start at zero; rejects non-area.
- `pie`: slice sweep angles sum to 2π; doughnut inner radius = hole × outer; slices distinct
  colors; rejects non-pie.
- `scenes`: the eight new scenes are name-lookupable with the expected `ChartKind`. (36 unit
  tests total across the workspace, all passing.)
- The **9 PNGs + single-agent review** in `results/` are the real Gate-2 evidence.
