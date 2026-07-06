# Chart Comparison — Line

Per-type Excel-vs-`gpui-component` comparison for the **Line** chart family, scoped to
"at most what `gpui-component` already renders." Assesses **two** implementation levels:

- **Level A — the `LineChart` struct** (`chart/line_chart.rs`), used as-is.
- **Level B — DIY on the `plot/` primitives** (`Line` shape + `ScaleLinear`/`ScalePoint`
  + `axis`/`grid`/`tooltip`) inside a custom `Plot`, which is where multi-line, a shared
  y-scale, and a legend become achievable.

Primary inputs: `excel-chart-data-model.md` (Excel/OOXML data model) and
`gpui-component-charts.md` (Wave-1 capability doc, pinned rev
`a9a7341…4710c`; `path:line` citations below are from that doc). See also
`ironcalc-chart-exposure.md` (we must extract chart XML ourselves; series values are
available either from the `c:numCache` snapshot or live via `c:f` against IronCalc).

---

## 0. TL;DR verdict

- **Single line — near-win.** The `LineChart` struct renders a single line cleanly
  (curve/dots/grid/x-axis/hover, per-series stroke color). Two shared caveats hold it
  back from "flawless": **no numeric y-axis labels** (every gpui chart lacks them) and a
  **forced-zero y-domain** that can distort a zoomed data range. The struct's default is
  **curved** (`.natural()`), which is the *opposite* of Excel's default straight line, so
  faithfulness requires reading `c:smooth` and calling `.linear()` when appropriate.
- **Multi-line (2–5 series) with a legend — the common case, and it needs Level B.** The
  struct is single-line only; overlaying multiple `LineChart`s does **not** share a
  y-scale. True multi-line means DIY on the raw `Line` primitive + one shared
  `ScaleLinear` + a hand-built legend + a 5-color palette. Feasible, but real work.
- **Markers — approximate.** `.dot()` gives a uniform dot at each point; Excel's
  per-series marker **shape/size/color** (`c:marker`) is not reproduced.
- **Smoothing — approximate.** `.natural()` (a natural cubic spline) is a visual
  stand-in for Excel's `c:smooth`, not a math-identical curve.
- **Stacked / percentStacked line — OUT (rare).** No stacking in any gpui chart type;
  DIY only, and uncommon enough to drop.
- **3D line (`c:line3DChart`) — OUT.** No 3D anywhere in the library; flatten to 2D.

---

## 1. Excel side — the `c:lineChart` data model

`c:lineChart` lives under `c:plotArea` (`excel-chart-data-model.md` §2a/§4a). Skeleton
(children of `CT_LineChart`):

```xml
<c:lineChart>
  <c:grouping val="standard"/>       <!-- standard | stacked | percentStacked -->
  <c:varyColors val="0"/>
  <c:ser> … </c:ser>                 <!-- ONE <c:ser> PER LINE — repeatable (multi-line) -->
  <c:ser> … </c:ser>
  <c:marker val="1"/>                 <!-- chart-group toggle: do series show markers at all -->
  <c:dLbls/>                          <!-- chart-level data labels (optional) -->
  <c:hiLowLines/> <c:dropLines/>      <!-- optional line-connector decorations (rare) -->
  <c:axId val="…"/> <c:axId val="…"/> <!-- links to c:catAx + c:valAx -->
</c:lineChart>
```

Each `c:ser` (the heart of the model — `excel-chart-data-model.md` §4b/§4d):

```xml
<c:ser>
  <c:idx val="0"/> <c:order val="0"/>
  <c:tx>…Revenue…</c:tx>                     <!-- series NAME (legend entry) -->
  <c:spPr><a:ln><a:solidFill>                <!-- PER-SERIES LINE COLOR (a:srgbClr / a:schemeClr) -->
    <a:srgbClr val="4472C4"/></a:solidFill></a:ln></c:spPr>
  <c:marker>                                 <!-- PER-SERIES MARKER -->
    <c:symbol val="circle"/>                 <!-- ST_MarkerStyle: none|auto|circle|square|diamond
                                                  |triangle|x|star|dot|dash|plus -->
    <c:size val="5"/>                         <!-- 2–72 pt -->
    <c:spPr>…marker fill/line color…</c:spPr>
  </c:marker>
  <c:cat>…$A$2:$A$10…</c:cat>                <!-- CATEGORIES / X, SHARED across series -->
  <c:val>…$B$2:$B$10…</c:val>                <!-- VALUES / Y (numCache + c:f range) -->
  <c:smooth val="0"/>                        <!-- PER-SERIES: 1=smoothed spline, 0=straight -->
  <c:dPt/> <c:dLbls/>                        <!-- per-point overrides / labels (optional) -->
</c:ser>
```

Key facts and common cases:

- **Multi-line is the common case.** Each additional line = one more `c:ser` sibling; all
  series repeat the **same `c:cat`** range and each carries its own `c:val` + `c:tx` +
  color (`excel-chart-data-model.md` §4d). Single-line (1 `c:ser`) and multi-line (2–5
  `c:ser`) **with a legend** are both very common in real files.
- **`c:grouping`** (`excel-chart-data-model.md` §5): `standard` (default — lines overlaid
  at true value), `stacked` (values accumulate), `percentStacked` (accumulate to 100%).
  **Stacked/percentStacked line is rare.**
- **`c:smooth`** is per-series. Excel's default line is **straight** — Excel writes
  `c:smooth val="0"` explicitly; `val="1"` gives a smoothed spline through the points.
  (Note: the bare OOXML `CT_Boolean` default is `true`, but Excel-authored files set the
  flag explicitly, so a reader should trust the written value and treat absent as straight
  for Excel files. **[verify default-when-absent on a real file]**)
- **`c:marker`** appears at two levels: a chart-group boolean (`<c:marker val="1"/>`
  toggling the "Line with Markers" family) and a **per-series** `c:marker` with
  `c:symbol` (shape), `c:size` (2–72 pt), and `c:spPr` (marker color). The symbol
  enumeration is ECMA-376 `ST_MarkerStyle` (established knowledge; corroborates the
  Wave-1 "shape/size/color" note in `excel-chart-data-model.md` §5).
- **Per-series color** = `c:ser → c:spPr → a:ln → a:solidFill` (`a:srgbClr` literal or
  `a:schemeClr` theme ref) (`excel-chart-data-model.md` §5).
- **Value axis** (`c:valAx`) auto-scales to the data range (with padding); it does **not**
  force zero, and it **always shows numeric labels**. `c:catAx` carries the x labels.
- **`c:line3DChart`** is the 3D sibling: adds a series axis (`c:serAx`) + `c:view3D` on
  the chart (`excel-chart-data-model.md` §2a/§5). Rare.

---

## 2. gpui side — `LineChart` struct vs `Line`-primitive DIY

### Level A — the `LineChart` struct (`chart/line_chart.rs`)

Signature and knobs (`gpui-component-charts.md` §2 "LineChart"):

```rust
LineChart::new(data)
  .x(|&T| -> X)  .y(|&T| -> Y)            // SINGLE x, SINGLE y; re-calling .y() REPLACES it
  .stroke(Hsla)                           // one line color; default chart_2 (line_chart.rs:171)
  .natural() | .linear() | .step_after()  // curve; DEFAULT = Natural (curved)
  .dot()                                  // uniform dot at each point (markers)
  .grid(bool) .x_axis(bool) .tick_margin(usize)
  .id(..) .name(..)                       // opt-in hover tooltip + series name
```

- **Single line only.** `y: Option<Rc<dyn Fn>>`; `.y()` replaces rather than pushes
  (`gpui-component-charts.md` §2, §4 matrix). **Overlaying several `LineChart` elements
  does NOT share a y-scale** — each normalizes its own domain independently — so true
  multi-line is impossible at Level A (`gpui-component-charts.md` §2 "Multiple series: No").
- **Y-domain forced to include 0** (`.chain(Some(Y::zero()))`, `gpui-component-charts.md`
  §2). Diverges from Excel's auto-range value axis: a series of 100–110 renders squished
  at the top instead of zoomed in.
- **No numeric y-axis labels** — true of *every* gpui chart (`gpui-component-charts.md`
  §3, §4). You get ~4 unlabeled gridlines; you can read the shape but not the magnitudes.
- **No legend, no title** in the module (`gpui-component-charts.md` §3). Titles are faked
  by the story with a surrounding `div`.
- **Value type locked to `f64`** (or `Decimal` via feature) — `Sealed` trait
  (`gpui-component-charts.md` §1). Integer/`f32` series must be widened to `f64`.
- Present and good: curve interpolation, `.dot()` markers, grid/x-axis toggles, tick
  density, opt-in hover tooltip (crosshair + dot + one row), per-line stroke color.

### Level B — DIY on the `plot/` primitives

The Wave-1 doc's headline conclusion: *"the primitives layer (`plot/` —
`Line`/`Bar`/`Area`/`Arc`/`Pie`/`Stack` + scales + axis/grid/tooltip) is the real asset
and is reusable"* (`gpui-component-charts.md` §5). For line specifically, a custom `Plot`
can:

- Draw **N lines** with the raw `Line` shape (`plot/shape/`), each fed from its own
  accessor — this is what unlocks multi-line.
- Build **one shared `ScaleLinear`** over the union min/max of all series so every line is
  measured against the same y-scale (the exact thing overlaid `LineChart`s cannot do), and
  set its domain to the data range (matching Excel's auto-range, *not* forced-zero).
- Assign the **`chart_1..chart_5` palette** (`theme_color.rs:134-147`) per series — note
  the palette does **not** auto-cycle; you wire a `ScaleOrdinal` yourself
  (`gpui-component-charts.md` §1). 5 colors covers the common 2–5 series case exactly.
- Hand-roll a **legend** (swatch + series name row) — there is none in the library
  (`gpui-component-charts.md` §3), so it must be drawn as adjacent elements.
- Draw **markers** and **numeric y ticks** manually via `label`/`axis` helpers if desired.

Cost: this is a bespoke `Plot` impl (the library's own stacked-bar demo does exactly this
for bars, and lives in the story crate, not the chart module — `gpui-component-charts.md`
§3). Straightforward but non-trivial: shared-domain math, palette wiring, legend layout,
optional per-series marker/curve/tooltip handling.

---

## 3. Mapping table — Excel line features → gpui level

Level key: **A** = `LineChart` struct as-is · **B** = DIY on `plot/` primitives ·
**approx** = renders but not faithfully · **OUT** = not renderable, drop/flatten.

| Excel line feature | Data model | Level | Render-quality note |
|---|---|---|---|
| **Single line** (`c:lineChart`, 1 `c:ser`, `grouping=standard`) | `c:cat` + `c:val` | **A** | Clean shape, curve, dots, grid, x-labels, hover. Gaps: no numeric y-axis labels; forced-zero y-domain can distort; must set `.linear()` to honor Excel's straight default. |
| **Multi-line grouped** (2–5 `c:ser`, `grouping=standard`) | N× `c:ser`, shared `c:cat` | **B** | Struct: **NO** (single-line; overlays don't share y-scale). Primitive: **YES** with shared `ScaleLinear` + `chart_1..5` palette + hand-built legend. This is the *common* case, so Level B is effectively mandatory for line. |
| **Stacked line** (`grouping=stacked`) | series accumulate | **B / OUT** | No stacking in any gpui chart; DIY only (pre-sum values, feed `Line`s). Rare — recommend **OUT** for MVP. |
| **percentStacked line** (`grouping=percentStacked`) | series → 100% | **B / OUT** | Same as stacked plus normalize-to-100%. Rarer still — **OUT**. |
| **Per-series color** (`c:spPr → a:ln → a:solidFill`) | per-`c:ser` | **A** (1 line) / **B** (N lines) | Faithful. Struct `.stroke()` = one color; primitive path assigns palette/explicit color per series. `a:schemeClr` theme refs must resolve to RGB first. |
| **Markers** (`c:marker`: `symbol`/`size`/`spPr`) | per-`c:ser` | **approx** | `.dot()` = uniform dot, single shape/size, stroke-tinted. No square/diamond/triangle/x/star shapes, no size, no per-marker color. Shape/size/color **lost**. On Level B you could draw custom marker glyphs. |
| **Smoothed line** (`c:smooth val="1"`) | per-`c:ser` | **approx** | `.natural()` (natural cubic spline) vs Excel's Bézier/Catmull-Rom-style smoothing: both are smooth curves through all points, but control-point math differs → subtly different arcs between points. Faithful *visually*, not pixel-identical. **Flag as approximation.** |
| **Straight line** (`c:smooth val="0"`, Excel default) | per-`c:ser` | **A** | `.linear()`. Faithful. **Caveat: gpui default is `.natural()` (curved)** — the reader MUST map absent/`0` → `.linear()` or straight Excel lines render as curves. |
| **3D line** (`c:line3DChart`, `c:serAx`, `c:view3D`) | 3D | **OUT** | No z-axis/perspective/3D geometry anywhere (`gpui-component-charts.md` §3 "3D: None. Confirmed"). Flatten to a 2D multi-line and drop depth. |
| **Legend** (`c:legend`) | chart-level | **B** | None in library; must be hand-drawn (swatch+name). Needed for multi-line. |
| **Title** (`c:title`) | chart-level | **B** (wrapper) | Not in chart module; render as a `div` label around the plot. |
| **Numeric value axis** (`c:valAx` labels) | chart-level | **B** | Absent on all gpui charts; DIY tick labels on primitives if we want readable magnitudes. |
| **Data labels** (`c:dLbls` showVal) | per-point | **OUT / B** | `LineChart` has no data labels; DIY text placement only. Low priority. |

### On `c:smooth` vs `.natural()` (called out per the ask)

Excel `c:smooth` produces a smoothed curve through the data points using its own spline
(Bézier/Catmull-Rom-family). gpui `.natural()` is a *natural cubic spline* (d3
`curveNatural`-style). Both pass through every data point and look "smooth," so
`.natural()` is a **faithful visual stand-in** for a smoothed Excel line. It is **not**
mathematically identical — the interpolated path *between* points will differ subtly (peak
overshoot, curvature). **Treat as an approximation, acceptable for display.** The more
important correctness issue is the **default mismatch**: gpui defaults to curved, Excel
defaults to straight — so the extractor must drive curve choice from `c:smooth`, not rely
on the gpui default.

---

## 4. Owner's asks

- **Multiple datasets → multi-line.** Struct: **NO** (single `.y()`; overlays don't share
  a y-scale). Primitive: **YES, with work** — one shared `ScaleLinear` over the union of
  all series, the `chart_1..chart_5` palette wired via `ScaleOrdinal` (no auto-cycle), and
  a hand-built legend. This is the common real-world line chart, so plan for Level B.
- **Coloring → per-series stroke color.** **Feasible.** Maps `c:ser → c:spPr → a:ln →
  a:solidFill` to `.stroke()` (single line) or per-series palette/explicit color
  (multi-line). Resolve `a:schemeClr` theme colors to RGB during extraction.
- **3D → OUT.** No 3D in the library. Flatten `c:line3DChart` to a 2D multi-line (drop
  `c:view3D`/`c:serAx`); or decline 3D line entirely for MVP.
- **Markers → approximate.** `.dot()` gives dots but drops Excel's `c:marker`
  shape/size/color. Acceptable as "markers present"; not faithful. Level B could draw real
  marker glyphs if fidelity matters.

---

## 5. Verdict — render line WELL, or major gaps?

**Mixed: single line is a strong pass; the common multi-line-with-legend case is a
Level-B build, not free.**

- **(a) Single line — near clean win, not flawless.** Struct renders it well
  (curve/dots/grid/x-axis/tooltip/per-line color). Held back only by the two library-wide
  gaps every gpui chart shares — **no numeric y-axis labels** and **forced-zero y-domain**
  — plus the **curved-by-default** quirk (must call `.linear()` for Excel's straight
  default). Good enough to ship; honest about the missing y-axis numbers.
- **(b) Multi-line with legend — the common case, and it needs Level B.** The struct
  cannot do it (single-line; overlays don't share a scale). Achievable and not hard on the
  `Line` primitive with a shared `ScaleLinear`, the 5-color palette, and a hand-built
  legend — but it is real, bespoke work, and the legend + numeric axis must be hand-drawn.
- **(c) Markers & smoothing — both approximate.** Markers collapse to a uniform dot
  (shape/size/color lost); smoothing via `.natural()` is a visual, not exact, stand-in for
  `c:smooth`. Fine for display; flag both as approximations.
- **(d) 3D line — dropped.** No 3D in the library; flatten to 2D or decline.

**Bottom line:** gpui can render the **line family well for display**, but the honest split
is single-line = struct-level pass with two known gaps, while the *common* multi-line
case requires committing to the Level-B primitive path (shared scale + palette + legend +
optional numeric axis). Stacked/percentStacked and 3D are out.
