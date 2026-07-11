---
status: complete
---

# Phase 26: Bubble (the final type + closing cross-type sweep)

## Overview

P26 closes the **breadth batch** (implementation_plan §"New graph types") after P22 (column & bar),
P23 (area), P24 (pie & doughnut), and P25 (scatter): the **bubble** type slots onto the
already-hardened, editable pipeline (anchor/cull/clip, live binding, save/source-patch,
insert/move/resize/delete, chrome editing) proven on the line chart through P21 and inherited by
every breadth type. As with P22–P25 the machinery is **reused, not rebuilt** — the net-new work is
the type's own renderer fidelity + regression baselines + round-trip, plus (because bubble is the
*last* type) the **closing cross-type validation gate**: the full pixel suite, a huge-sheet
cross-type perf re-measure (addressing GAPS **C-P25-1**), and the round-trip proof.

Bubble is **scatter + a third value per point** (`c:bubbleSize`): each point is `(x, y, size)` over
**two numeric axes** (like scatter), and the size sets the **marker area**. So most of the surface is
scatter's, extended by one range and one visual variable:

- **Model** — `SeriesData::Xy` gains `size: Option<Vec<f64>>` (`None` = scatter, `Some` = bubble);
  new `ChartKind::Bubble { size_representation }` + `SizeRepresentation` enum (`Area` default /
  `Width`, `c:sizeRepresents`); `Series::bubble(name, x, y, size)`; `ChartInsertKind::Bubble` +
  `SeriesShape` for the shell builder.
- **Three-range binding** — `SeriesBinding` gains `size: Option<CfRef>` read from `c:bubbleSize`;
  `resolve_series` re-resolves x, y **and** size and preserves the `kind`; `binding_from_refs` reads
  `SeriesRefs.sizes`; the dirty test includes the size range. **This is the "extend scatter's
  dual-range binding to also resolve bubbleSize" ask — no design blocker** (the binding layer already
  keys roles by holder tag; bubbleSize is one more holder).
- **Load** — `c:bubbleChart` → `ChartKind::Bubble`, reads `c:sizeRepresents`; each series reads
  `c:xVal`/`c:yVal`/`c:bubbleSize`; two `c:valAx` like scatter.
- **Write** — `SeriesRefs.sizes`; `group_element` bubble arm emits `CT_BubbleChart` order
  (varyColors, ser*, sizeRepresents, axId×2); `series_element` bubble emits `xVal`/`yVal`/`bubbleSize`
  (`CT_BubbleSer` order); two `c:valAx`.
- **Save reflow** — the `Xy` reflow arm also patches `bubbleSize` when present.
- **Renderer** — `bubble.rs`: scatter's two-numeric-axis frame + a filled **circle per point whose
  AREA encodes the size** — radius ∝ √(size) for `Area` (Excel's default; equal size ratios ⇒ equal
  area ratios), radius ∝ size for `Width` — with a **min/max radius clamp** so tiny/huge values stay
  legible. Translucent fills + a solid series-colored edge so overlapping bubbles read; distinct
  per-series palette colors; **draw-largest-first** so small bubbles stay visible.
- **C-P25-1 (perf)** — a **cloud-aware paint cap** (`cap_markers_for_paint`, uniform-stride
  subsample; identity below the cap so no baseline moves) bounds the per-frame marker/segment count
  for **both** scatter and bubble; measured in the perf sweep.

**Fidelity — decided honestly for bubble.** The bubble renderer honors two numeric axes, √-area (and
width) size encoding with a clamp, and translucent per-series fills — all **Faithful**.
`c:bubble3D val="1"` → **Degraded** (we draw flat 2-D circles — an honest badge, not a silent
flatten). `c:sizeRepresents` (area/width) is **honored** → Faithful either way. Axis `scaling` /
data labels stay line-scoped (a bubble carrying those keeps its honest badge), matching scatter.

## Steps

### Model — `freecell-chart-model` (`lib.rs`)

1. Add `size: Option<Vec<f64>>` to `SeriesData::Xy` (`None` = scatter, `Some` = bubble). `Series::xy`
   sets `size: None`; add `Series::bubble(name, x, y, size)` → `Xy { x, y, size: Some(size) }`.
   `Series::len` Xy arm unchanged (`x.len()`).
2. Add `SizeRepresentation` enum (`Copy`): `Area` / `Width`, with `as_ooxml()` (`"area"`/`"w"`) and a
   `from_ooxml`-style parse (absent/unknown → `Area`, Excel's default).
3. Add `ChartKind::Bubble { size_representation: SizeRepresentation }` (Copy). Doc: two numeric axes;
   bubbleSize→area.
4. Add `SeriesShape` enum (`CategoryValue` / `Xy` / `Bubble`) — the data-shape the shell builder
   constructs (replaces `build_series_shells`'s `xy: bool`).

### Model authoring — `authoring.rs`

5. `ChartInsertKind::Bubble`. `chart_kind()` → `Bubble { size_representation: Area }`;
   `from_chart_kind` → `Bubble { .. } => Bubble`; `is_xy()` true for Scatter **and** Bubble; add
   `is_bubble()` + `series_shape()` (Bubble→`SeriesShape::Bubble`, Scatter→`Xy`, else `CategoryValue`);
   `placeholder_series()` bubble arm → `Series::bubble` with x/y/size placeholder vectors. Bump the
   test `ALL` array to 8; add Bubble assertions.

### Model fidelity — `fidelity.rs`

6. `bubbleChart` is a supported group (already Faithful in `source_fidelity` once the loader parses
   it — bubble is not in the unsupported set). Add `is_bubble_chart(xml)` + `unsupported_bubble_3d`
   (`is_bubble && any <c:bubble3D val="1"/true>`), wired into
   `has_render_affecting_unsupported_feature`. `c:sizeRepresents` is honored (not degrading). Update
   the module docs + the `supported_group_sources_are_faithful` test (add `<c:bubbleChart/>`).

### Engine load — `load.rs`

7. Add `"bubbleChart"` to `CHART_GROUP_TAGS`. `parse_kind` bubble arm →
   `ChartKind::Bubble { size_representation: size_represents(group) }` (new helper reading
   `c:sizeRepresents@val`). Compute a `SeriesShape` from the kind; `is_xy = shape != CategoryValue`
   drives `parse_axes` (two valAx for scatter **and** bubble). `parse_series` takes the shape: an
   `Xy`/`Bubble` series reads `xVal`/`yVal`; a `Bubble` series additionally reads `c:bubbleSize` into
   `Series::bubble`.

### Engine binding — `binding.rs`

8. `SeriesBinding` gains `size: Option<CfRef>`; `parse_chart_binding` reads `size: ser_ref(&ser,
   &["bubbleSize"])` (None for non-bubble — no bubbleSize element). `resolve_series` Xy arm resolves
   `size` from `binding.size` when present (sets `*size = Some(resolved)`), keeping the variant.
   `binding_from_refs` reads `size: r.sizes…`. `build_series_shells(count, shape: SeriesShape)`
   builds the right shell. `binding_is_dirty` includes `&sb.size`.

### Engine write — `write.rs`

9. `SeriesRefs.sizes: Option<String>`. `group_element` bubble arm → `CT_BubbleChart` order
   (`varyColors`, ser*, `sizeRepresents`, axId×2). `series_element` Xy arm: append
   `num_role("bubbleSize", sizes_f, size)` when the series carries a size. `axes_xml` → the two-valAx
   arm covers `Scatter | Bubble`.

### Engine save — `save.rs`

10. `patch_chart_source` Xy arm also `push_num_cache("bubbleSize", size, …)` when the series carries
    a size (reflow the third range).

### Engine worker — `worker/run.rs`

11. `set_chart_range` / `set_chart_type` compute a `SeriesShape` for `build_series_shells`.
    `source_ranges_from_refs` includes `&r.sizes`. (`AuthoredChart`/`refs` flow is unchanged — the new
    `sizes` field threads through automatically.)

### Renderer — `bubble.rs` (new) + `mod.rs`

12. `bubble.rs`: `BubblePlot` mirroring `ScatterPlot` — two numeric `ScaleLinear` axes over the union
    of every series' x/y; per-point radius from size (√-area for `Area`, linear for `Width`) clamped
    to `[MIN_BUBBLE_RADIUS, MAX_BUBBLE_RADIUS]` and normalized by the max size across all series;
    translucent fill (`with_alpha`) + a solid series-colored edge; **draw-largest-first**; the
    `cap_markers_for_paint` cloud cap (C-P25-1). `bubble_element` wraps in `chart_frame`. Register in
    `mod.rs` dispatch (`ChartKind::Bubble { .. } => bubble::bubble_element`).

### C-P25-1 cloud cap — `downsample.rs`

13. Add `MAX_PAINT_MARKERS` + `cap_markers_for_paint(n, max)` (uniform stride; identity ≤ cap so no
    baseline moves). Apply in `bubble.rs` **and** `scatter.rs` (both the marker loop and, in scatter,
    the connecting `Line` — feed both the capped index set).

### Legend — `chrome.rs`

14. Bubble legend keys per-series **fill/theme/palette** color (not stroke) — the general
    `legend_entries` path already produces one swatch per series for a non-pie kind, so a bubble's dot
    cloud and its swatch match by construction. No change beyond confirming the new `Xy` field
    compiles (chrome only matches `CategoryValue`).

### Insert menu — `chrome/view.rs` + `assets.rs` + a `chart-bubble.svg` icon

15. Add `ChartInsertKind::Bubble` to `CHART_MENU` (array → 8) with a new `icons/chart-bubble.svg`
    (registered in `assets.rs`), so bubble is authorable from the action bar. Update the stale
    "Bubble omitted" comments (menu + `authoring.rs`). (Action-row chrome is **out of** the pixel
    suite scope — validate via the crate's gpui view tests + the Xvfb smoke launch, per CLAUDE.md.)

### Render scenes — `chart_scene.rs` + grid case `cases.rs` + `render_suite.rs`

16. Add standalone scenes (+ `all()`/`get()` entries + a `p26_scenes_*` unit test):
    - `chart_bubble_multi` — a **multi-series** bubble (area-encoded sizes) over two numeric axes,
      distinct series colors, title / both axis titles / right legend.
    - `chart_bubble_size_clamp` — a single-series bubble with a **very small + very large** size to
      prove the min/max radius clamp.
    Add `grid_chart_bubble` — a **loaded** bubble `ChartSpec` at the shared `chart_anchor` over the
    backing table (a `<c:bubbleChart>` source so it classifies Faithful). Register both macros
    (`chart_render_cases!` + `render_cases!`/`CASE_NAMES`); the drift guards keep them in lockstep.

### Baselines (generate + eyeball, commit with the code)

17. `render_tests.sh generate --only chart_bubble_` / `--only grid_chart_bubble`; **eyeball** each PNG
    (two numeric axes + sized bubbles; the clamp; the in-grid bubble over the table). Commit with the
    code.

### Round-trip

18. `write.rs`: `serialize_roundtrips_bubble` + `serialize_roundtrips_bubble_size_representation`
    (area/width survive) + `write_authored_bubble_reopens_as_bubble_with_size`. `load.rs`:
    `parses_bubble_chart_size_and_representation`. `worker_seam.rs`: `retyped_to_bubble_chart_roundtrips`
    (`SetChartType(Bubble)` → save → reopen as `ChartKind::Bubble`, x/y binding preserved). Add Bubble
    to `near_empty_insert_templates_round_trip`.

### Closing validation gate (this is the LAST type)

19. **Perf re-measure (C-P25-1):** extend `chart_perf.rs` with a bubble/scatter cloud paint-prep
    measurement (map N points to pixel + radius) FULL vs CAPPED + the cap cost; force+assert; report
    p50/p99, env-stamped; update `results/chart-perf.json`; record here.
20. **Full pixel suite:** run the ENTIRE suite once, FOREGROUND under a generous `timeout`
    (line/column/bar/area/pie/doughnut/scatter/bubble + grid + titlebar) — every case == baseline.
    Report the pass/fail count. (Manager also dispatches the CI `render` gate.)

## Tests

- **Model (`lib.rs`)** — `SizeRepresentation` `as_ooxml`/parse round-trip; `Series::bubble` carries a
  size; `ChartKind::Bubble` carries a representation; `SeriesData::Xy` size default `None` for xy.
- **Model authoring (`authoring.rs`)** — `chart_kind`/`from_chart_kind` invert for Bubble;
  `is_xy`/`is_bubble`/`series_shape`; bubble placeholder is a `Bubble` shape with a size.
- **Load (`load.rs`)** — `parses_bubble_chart_size_and_representation` (bubbleChart → Bubble;
  sizeRepresents area/w/absent; xVal/yVal/bubbleSize into a bubble series; two valAx).
- **Binding (`binding.rs`)** — a bubble binding reads a size ref; `resolve_series` re-resolves x/y/size
  and keeps the bubble shape; a size-range edit marks the chart dirty; shells build the bubble shape.
- **Write (`write.rs`)** — `serialize_roundtrips_bubble`; `serialize_roundtrips_bubble_size_representation`;
  `write_authored_bubble_reopens_as_bubble_with_size`; `near_empty_insert_templates_round_trip` (Bubble).
- **Save (`save.rs`)** — the bubble reflow patches bubbleSize.
- **Edited round-trip (`worker_seam.rs`)** — `retyped_to_bubble_chart_roundtrips`.
- **Renderer (`bubble.rs`)** — shared domains cover all points; radius clamps to `[min,max]`; a bigger
  size ⇒ a ≥ radius; area vs width mapping; point count == data; rejects non-bubble/empty; the cloud
  cap is identity below the cap and bounded above it.
- **Fidelity (`fidelity.rs`)** — `p26_bubble_faithful_but_3d_degrades`: a plain bubble (any
  sizeRepresents) is Faithful; `c:bubble3D val="1"` Degrades; existing scatter scoping stays green.
- **Scenes (`chart_scene.rs`)** — `p26_scenes_carry_their_bubble_kind`: each new scene is a
  `ChartKind::Bubble` whose series carry sizes; the clamp scene spans a wide size range.
- **Pixel (new baselines)** — `chart_bubble_multi`, `chart_bubble_size_clamp`, `grid_chart_bubble`
  render == baseline; **full cross-type suite** all == baseline.

## Render validation

Bubble rendering **is** in-scope for the pixel suite (chart scenes + the in-grid case, CLAUDE.md
render-tests §Scope). During coding, iterate with the **subset** foreground under a `timeout` (never
background a render job): `test chart_bubble_`, `test grid_chart_`, and a no-regression `test
chart_scatter_`/`test chart_line_`. As the LAST type, the closing step runs the **full** suite once
(item 20). New baselines are generated + **eyeballed** and committed with the code. The insert-menu
glyph is action-row chrome (out of pixel-suite scope) — validated by gpui view tests + the Xvfb smoke
launch. The CI `render` gate is manager-dispatched after commit.

## Results

All green (bar the documented `soffice` env caveat — the two `charts_roundtrip_libreoffice` tests;
external round-trip rides CI). Bubble is a production type on the hardened pipeline: it loads, renders
(two numeric nice-tick axes + √-area sized translucent circles + the min/max clamp + `sizeRepresents`),
authors, edits, live-binds (x **and** y **and** size), and round-trips — reusing the P1–P25 machinery,
with the type's renderer + three-range binding + regression baselines + round-trip added. Bubble being
the last type, P26 also ran the **closing cross-type sweep**.

### What shipped
- **Model** — `SeriesData::Xy` gained `size: Option<Vec<f64>>` (`None` = scatter, `Some` = bubble);
  `Series::bubble`; `ChartKind::Bubble { size_representation }` + `SizeRepresentation` (Area default /
  Width, `c:sizeRepresents`); `ChartInsertKind::Bubble` (+ `is_bubble`/`series_shape`); `SeriesShape`
  for the shell builder; `cap_markers_for_paint` + `MAX_PAINT_MARKERS` cloud cap.
- **Three-range binding** — `SeriesBinding.size` read from `c:bubbleSize`; `resolve_series`
  re-resolves x/y/size preserving `kind`; `binding_from_refs` reads `SeriesRefs.sizes`; the dirty test
  includes the size range. No design blocker — the binding layer keys roles by holder tag, and bubbleSize
  is one more holder.
- **Load** — `bubbleChart` → `ChartKind::Bubble`, reads `c:sizeRepresents`; series read
  `xVal`/`yVal`/`bubbleSize`; two `c:valAx`.
- **Write** — `SeriesRefs.sizes`; `group_element` emits `CT_BubbleChart` order (varyColors, ser*,
  sizeRepresents, axId×2); `series_element` emits `bubbleSize` (`CT_BubbleSer` order); two valAx.
- **Save reflow** — the Xy reflow arm patches `bubbleSize` when present.
- **Renderer (`bubble.rs`)** — two numeric `ScaleLinear` axes over the shared X/Y nice domains; a
  filled circle per point whose **area** encodes the size (`radius ∝ √(size/maxSize)·MAX`, clamped to
  `[4, 26]` px; width → `radius ∝ size`); translucent fill + solid series edge; **draw-largest-first**;
  the cloud cap. Registered in `mod.rs` dispatch.
- **C-P25-1 (perf)** — `cap_markers_for_paint` (uniform linspace, identity ≤ cap) bounds **both**
  scatter (marker loop **and** connecting `Line`) and bubble; scatter pixels stay byte-identical below
  the cap. GAPS C-P25-1 marked **Resolved**.
- **Fidelity** — `bubbleChart` is a supported (Faithful) group; `c:bubble3D val="1"` Degrades
  (`unsupported_bubble_3d`, bubble-scoped); `c:sizeRepresents` honored either way.
- **Insert menu** — `ChartInsertKind::Bubble` added to `CHART_MENU` with a new `chart-bubble.svg`
  (registered in `assets.rs`); the stale "Bubble omitted" comments dropped.

### New render scenes + baselines (3, generated + eyeballed, committed with the code)
- `chart_bubble_multi` — a multi-series area-encoded bubble over two numeric axes (blue/orange
  translucent discs, matching legend).
- `chart_bubble_size_clamp` — a single-series bubble spanning size 2 → 900: the tiny size stays a
  legible min-radius dot, the huge one is capped at max-radius.
- `grid_chart_bubble` — a loaded bubble painted in-grid over the backing table (ChartLayer →
  `bubble_element`).

### Render — subset then FULL suite (foreground, under a `timeout` watchdog)
- Subset (generate + eyeball): `chart_bubble_` (2) + `grid_chart_bubble` (1) generated, eyeballed, and
  committed.
- **FULL cross-type suite** — `render_tests.sh test` → **128 passed, 0 failed** (599 s): every case
  (line/column/bar/area/pie/doughnut/scatter/**bubble** + all `grid_chart_*` incl. `grid_chart_bubble` +
  titlebar + cell/border/etc + the two drift guards) == baseline. No existing baseline moved (the scatter
  cloud cap is identity below the cap → scatter/line pixels byte-identical). CI `render` gate is
  manager-dispatched.

### Perf re-measure (huge-sheet, cross-type — env-stamped, `results/chart-perf.json`)
FORCE+ASSERTED, headless CPU path, `x86_64-linux`. p50/p99:
- first-paint 384.70 µs / 613.22 µs; edit-rerender 1.96 µs / 3.17 µs; scroll-with-K=1000 6.33 µs /
  9.89 µs (~2 on-screen); many-line-charts open (K=200) 11.15 ms / 13.80 ms; large-series
  down-sample 348.87 µs, paint-prep FULL 288.75 µs vs DOWN-SAMPLED 7.22 µs.
- **Bubble/scatter cloud (C-P25-1):** paint-prep FULL N=100 k **p50 1.05 ms / p99 1.10 ms** per frame
  vs **CAPPED to 2048 p50 21.75 µs / p99 37.12 µs** (~49× fewer marks). The uncapped map alone is a large
  fraction of the 8.33 ms frame budget (before the far costlier per-mark tessellation), so the cap is a
  real, justified win. Sanity-checked: capped ≈ the line down-sampled prep (both ~2048 marks), as expected.

### Round-trip — PASS (local `discover_and_parse` / `parse_chart_xml` reopen)
- Engine chart unit — incl. `serialize_roundtrips_bubble`, `serialize_roundtrips_bubble_size_representation`
  (area/width survive + schema order), `write_authored_bubble_reopens_as_bubble_with_size` (authored
  bubble → write → `discover_and_parse` reopens as `Bubble{Area}` with the bubbleSize preserved),
  `parses_bubble_chart_size_and_representation`, `near_empty_insert_templates_round_trip` (Bubble).
- Binding — `resolve_bubble_reflects_all_three_ranges_and_size_range_is_dirty`,
  `parse_chart_binding_reads_bubble_size_ref`.
- Engine integration (`worker_seam`): `retyped_to_bubble_chart_roundtrips` (`SetChartType(Bubble)` →
  save → reopen as bubble, xy series, y-range binding preserved). `charts_corpus` 8 (bubble now
  Faithful). (The two `charts_roundtrip_libreoffice` tests are the documented `soffice` env caveat.)
- Model 93, app chart 76 (incl. the new bubble renderer + radius/clamp tests), render-tests lib 16
  (incl. `p26_scenes_carry_their_bubble_kind`) green.

### Checks
`cargo fmt --all --check` clean; `cargo clippy` clean across freecell-chart-model / -engine / -app /
render-tests (`--all-targets`).
