# Chart Comparison — Bar / Column

Research note for the FreeCell **charts** project (research phase). Goal: an honest
Excel-vs-`gpui-component` comparison for the **Bar / Column** chart family, scoped to "at
most what gpui-component can render," with a render-quality-vs-gaps verdict.

**Inputs / evidence.** Excel/OOXML facts are from the Wave-1 data-model note
(`specs/projects/chart/research/excel-chart-data-model.md`, cited as `excel:§`/`:line`).
gpui capability facts are from the Wave-1 capability note
(`specs/projects/chart/research/gpui-component-charts.md`, cited as `gpui-doc:line`) and
were re-verified for this family by reading the actual source at the pinned rev
`a9a7341c35b62f27ff512371c62419342264710c`
(`crates/ui/src/chart/bar_chart.rs`, `crates/ui/src/plot/shape/{bar,stack}.rs`,
`crates/ui/src/theme/theme_color.rs`, and the story-crate DIY stacked-bar example
`crates/story/src/stories/chart_story/stacked_bar_chart.rs`). Source citations are
`file:line` at that rev.

**Framing (carried from the brief).** gpui-component offers two levels:
- **Level A — the `BarChart` struct** as-is (a fixed, single-series wrapper).
- **Level B — DIY on the `plot/` primitives** (`Bar` + `Stack` + `ScaleBand`/
  `ScaleLinear`/`ScaleOrdinal` + `PlotAxis`/`Grid`/`Tooltip`), writing a custom `Plot`
  impl. The library's *own* stacked-bar chart is built this way and lives in the story
  crate, **not** the chart module.

---

## 0. TL;DR

- **Single-series column and single-series (horizontal) bar → CLEAN WIN at Level A.**
  `BarChart` covers both, with per-bar color, data labels, category axis, rounded corners,
  and hover tooltip. Only real losses: no numeric value-axis numbers, no built-in title.
- **The most common real Excel bar chart — clustered/grouped column with 2–4 series +
  legend — is NOT covered by the struct.** It requires **Level B** (a custom `Plot`).
  Stacked is the same story but has a `Stack` helper and a ~130-line working reference
  example. This is the family's biggest lift: achievable and can look good, but it is real
  engineering plus a hand-built legend, not a wrapper call.
- **3D (`c:bar3DChart`) → OUT.** No 3D anywhere in the library. Must flatten to the 2D
  equivalent (usually acceptable, sometimes an improvement; lossy only for true
  depth-axis 3D and shaped bars).

---

## 1. Excel side — the OOXML data model for this family

### 1a. One element, two Excel families

Excel's "Column" and "Bar" UI families are **the same OOXML element**, `c:barChart`; the
only difference is a child `c:barDir` (`excel:64`, `excel:82-83`, `excel:332`):

| `c:barDir val` | Excel UI family | Visual |
|---|---|---|
| `col` | **Column** | vertical bars, category axis on the bottom |
| `bar` | **Bar** | **horizontal** bars, category axis on the left |

So a reader must branch on `c:barDir`, not on the element name. The 3D counterpart is a
**separate** element, `c:bar3DChart` (also carrying `c:barDir`) (`excel:65`, `excel:341`).

### 1b. Grouping — what each value means visually

`c:grouping` (`excel:333`) selects how the multiple series relate:

| `c:grouping val` | Meaning | Notes |
|---|---|---|
| `clustered` | Series drawn **side-by-side** within each category ("grouped"). | The Excel default for multi-series bar/column; the single most common form. |
| `stacked` | Series **accumulate** into one bar per category. | Each series is a segment; bar length = sum. |
| `percentStacked` | Stacked, then **each category normalized to 100%**. | Shows composition/share, not magnitude. |
| `standard` | Series overlaid at the same baseline (mainly a line/area value). | For bars this behaves like non-clustered overlap; rare/degenerate for bars. |

Spacing knobs (`excel:334`): **`c:gapWidth`** = gap between category clusters (as a % of
bar width); **`c:overlap`** = how much series bars overlap within a cluster (0 = touching,
100 = fully overlapping, negative = extra gap). For `stacked`, Excel writes `overlap=100`.

### 1c. Multiple series (the multi-dataset case)

A multi-dataset chart is simply **multiple `<c:ser>` siblings** inside `c:barChart`
(`excel:213`, `excel:289-295`). Each `c:ser` carries:
- `c:tx` → the **series name** (the dataset label; a `strRef` to a cell or literal),
- `c:cat` → the **category/X labels** (typically the *same* range repeated per series),
- `c:val` → the **numeric values** (`numRef` → live `c:f` range + cached `c:numCache`),
- `c:idx`/`c:order` → identity/draw order.

FreeCell can render straight from the `numCache`/`strCache` snapshot with zero formula
evaluation (`excel:281-286`); staying live means resolving each `c:f` against IronCalc.

**The common case is explicitly the multi-series one:** clustered column with 2–4 series
plus a legend is one of the single most common Excel charts in the wild. Single-series
column/bar is common too, but the multi-series form is the workhorse.

### 1d. Per-series and per-point color

- **Per-series** fill: `c:ser → c:spPr → a:solidFill` (`excel:347`). Color is either
  `a:srgbClr val="RRGGBB"` (literal) or `a:schemeClr` (theme reference; needs theme
  resolution). Optional `a:ln` outline.
- **Per-point override:** `c:ser → c:dPt` with its own `c:idx` + `c:spPr` (`excel:348`) —
  recolors a single bar. Common for "highlight one column."
- `c:varyColors` (`excel:335`) can color each point differently, but for bar charts this
  is uncommon (it's a pie/doughnut default).

### 1e. Chrome the model carries

Legend (`c:legend` + `c:legendPos` t/b/l/r/tr, `excel:356`); title
(`c:title`/`c:autoTitleDeleted`, `excel:355`); category axis `c:catAx` and value axis
`c:valAx` with scaling/gridlines/number format (`excel:357-358`); data labels `c:dLbls`
(`showVal`/`showSerName`/etc., `excel:360`).

### 1f. 3D (`c:bar3DChart`)

Same data skeleton as 2D (`barDir` + `grouping` + repeated `c:ser` with `cat`/`val`), plus
3D-only knobs: `c:view3D` (rotation/perspective) on the chart, per-series `c:shape`
(box/cylinder/cone/pyramid), and an extra **series axis `c:serAx`** so series can be
arranged along a *depth* axis instead of clustering in the plane (`excel:65`, `excel:90`,
`excel:341`). It is comparatively rare and widely regarded as poor dataviz.

### 1g. Prevalence (real files)

- **Very common:** single-series column; **clustered column, 2–4 series, with legend**.
- **Common:** stacked / percentStacked column; single-series horizontal bar; per-point
  color highlight (`c:dPt`).
- **Rarer:** 3D column/bar; `standard` grouping on bars; horizontal stacked bar.

---

## 2. gpui side — what can actually be drawn

### 2a. Level A — the `BarChart` struct (verified)

`BarChart::new(data).band(|d| …).value(|d| …)` (`bar_chart.rs:24-99`). Facts, re-verified
against source:

- **Single value series only.** There is exactly one `value` accessor
  (`bar_chart.rs:32`, `.value()` at `:96-99`); calling `.value()` again *replaces* it.
  **No grouped, no stacked** built in.
- **4 orientations incl. horizontal.** `.alignment(BarAlignment)` with
  `Bottom`/`Top` (vertical) and `Left`/`Right` (horizontal) (`shape/bar.rs:12-23`,
  `is_horizontal` at `:26`). Horizontal bars are fully supported.
- **Rich per-bar coloring — the strongest feature.** `.fill(|datum, bar_bounds,
  chart_bounds, alignment| -> Background)` runs **once per bar** (`bar_chart.rs:120-132`,
  applied at `:446-450`), so per-point colors, chart-wide gradients, patterns, and sampled
  colormaps are all expressible. `.fill_gradient(...)` gives an auto-oriented base→tip
  2-stop gradient with clip-to-bar interpolation (`bar_chart.rs:169-176`, `clip_stops_to_bar`
  at `:577-614`). Default fill is a single theme color `chart_2` (`bar_chart.rs:394`).
- **Data labels: yes.** `.label(|d| …)` draws value text at each bar end, alignment
  auto-picked per orientation (`bar_chart.rs:183-189`, painted `:452-461`).
- **Category-axis labels: yes.** Band labels drawn on the category axis, with
  `.tick_margin()` density control (`bar_chart.rs:340-364`).
- **Hover tooltip: yes**, for all four orientations (highlight band + title + one value
  row) (`bar_chart.rs:470-564`), opt-in via `.id()`.
- **Rounded corners:** `.corner_radii(...)` (`bar_chart.rs:216-219`).
- **Hard limits:** **no numeric value-axis labels** — the value axis is only ~4 unlabeled,
  dashed gridlines, count hard-coded `(0..4)` (`bar_chart.rs:376-388`; gpui-doc:191-192).
  **No legend** anywhere in the module (gpui-doc:194). **No title** (gpui-doc:195). Bar
  padding is **hard-coded** `padding_inner(0.4)`/`padding_outer(0.2)` (`bar_chart.rs:235-236`)
  — not user-settable, so `c:gapWidth` cannot be honored at Level A.

### 2b. Level B — the primitives (verified)

The reusable toolkit under `plot/`:
- **`Bar<T>`** (`shape/bar.rs`): draws a set of bars given `.cross(|d|→x)`,
  `.band_width(w)`, `.base(|d|→pixel)`, `.value(|d|→pixel)`, `.fill(...)`, optional
  `.label(...)`, `.corner_radii(...)`, and a `BarAlignment`. Because `cross`, `base`, and
  `band_width` are all caller-supplied, you can place bars anywhere — this is what makes
  both grouping and stacking possible.
- **`Stack<T>`** (`shape/stack.rs`): a d3-shape port. `.keys([...])` + `.value(|d,key|→f32)`
  → `.series()` returns `Vec<StackSeries>` where each point carries cumulative `y0`/`y1`
  (`stack.rs:99-133`). This is exactly the cumulative math a **stacked** bar needs. It does
  **not** help with clustered bars (those need side-by-side offset math, which is simpler
  and manual).
- **Scales:** `ScaleBand` (categories→pixel bands, with padding), `ScaleLinear`
  (min/max→pixel, **no "nice" rounding, no tick generation** — gpui-doc:36), `ScaleOrdinal`
  (key→color, the auto-palette mechanism).
- **Palette:** the theme exposes exactly **5** categorical colors `chart_1..chart_5`
  (`theme_color.rs:134-147`; verified `chart_1..chart_5` fields). No auto-cycling inside
  the chart structs — you wire `ScaleOrdinal` yourself to get per-series colors.
- **Axis/grid/tooltip helpers:** `PlotAxis`, `Grid`, `Tooltip`/`CrossLine`, `AxisText`.

**Proof that multi-series bars are Level-B-achievable:** the library's own
`StackedBarChart` (`chart_story/stacked_bar_chart.rs`) is a **~130-line custom `Plot`**
that: computes cumulative segments with `Stack`; builds `ScaleBand`(x)/`ScaleLinear`(y);
assigns per-series colors with `ScaleOrdinal` over `chart_1..chart_4`; draws x-axis labels
and 4 gridlines; loops over series drawing one `Bar` per series with `.base(y0)`/
`.value(y1)`; and even implements a **multi-row hover tooltip** (one row per series). Its
header comment: *"You can draw any chart you want by using the `Plot`."* That multi-row
tooltip (swatch + series name + value) is the closest thing to a legend the library
has — but it is **hover-only**, not a persistent legend.

**Clustered/grouped bars** have **no** helper analogous to `Stack`. You'd subdivide each
category band into N sub-bands (`sub_w = band_width / n_series`), offset each series'
`.cross()` by `series_index * sub_w`, set `.band_width(sub_w)`, and color via
`ScaleOrdinal`. The math is *simpler* than stacking; the effort is comparable (a
similar-sized custom `Plot`). No built-in overlap/gap control, so `c:overlap`/`c:gapWidth`
would be approximated by your chosen sub-band spacing.

---

## 3. Mapping table — Excel variant → gpui level + quality

| Excel variant | Path | Level | Render-quality note |
|---|---|---|---|
| **Single-series Column** (`barDir=col`, 1 `ser`) | `BarChart` `.alignment(Bottom)` | **A** | **Clean win.** Per-bar color, data labels, category axis, tooltip, rounded corners all map. Losses: no numeric y-axis numbers, no title. |
| **Single-series Bar** (`barDir=bar`, horizontal) | `BarChart` `.alignment(Left)` (or `Right`) | **A** | **Clean win.** Horizontal is first-class; category labels move to the value axis and are width-measured (`bar_chart.rs:243-270`). Same losses as above. |
| **Clustered / grouped** (`grouping=clustered`, ≥2 `ser`) | custom `Plot`: `Bar` per series into subdivided bands + `ScaleOrdinal` colors | **B** | **Achievable, real work.** Looks good; per-series colors from palette or Excel `solidFill`. Needs hand-built legend + no numeric axis. `gapWidth`/`overlap` approximated. **This is the common case and it is NOT Level A.** |
| **Stacked** (`grouping=stacked`, ≥2 `ser`) | custom `Plot` using `Stack` + `Bar` loop | **B** | **Achievable, with a helper + a proven ~130-line reference.** Faithful segments, per-series color, multi-row hover. Needs hand-built legend + no numeric axis. |
| **percentStacked** (`grouping=percentStacked`) | as stacked, then normalize each category to its total | **B** | Same as stacked + one normalization pass (divide each series value by the category sum). Low incremental effort once stacked exists. |
| **3D Column / 3D Bar** (`c:bar3DChart`) | read data, **render as 2D** clustered/stacked column | **OUT → flatten** | No 3D in the library (gpui-doc:204-208). Flatten to the 2D equivalent. Acceptable for the common 3D-clustered-column; lossy for depth-axis (`serAx`) arrangement and box/cylinder/cone `c:shape`. |
| **`standard` grouping on bars** (rare) | `BarChart` (if 1 ser) or custom overlay | A/B | Degenerate/rare; treat as single-series or overlap. Low priority. |

---

## 4. The project owner's specific asks

**(1) Multiple datasets → grouped/stacked.** Struct: **NO** (single `value`,
`bar_chart.rs:32`). Primitives: **YES, with work.** Stacked is de-risked — the `Stack`
helper (`shape/stack.rs`) plus the library's own ~130-line `StackedBarChart` reference
prove it end to end (including tooltip). Clustered has no helper but is *simpler* math
(offset within the band); expect a similarly sized custom `Plot`. Both are a genuine
engineering task, not a config flag. Since the multi-series clustered column is the single
most common Excel bar chart, plan for Level B from the start if we want the common case to
look right.

**(2) Coloring → strong.** `.fill()` is per-bar (`bar_chart.rs:120-132`), so mapping Excel
colors is feasible and clean:
- Excel **per-series** `c:spPr → a:solidFill` `srgbClr RRGGBB` → set that color for every
  bar of the series (Level A: constant `.fill`; Level B: `ScaleOrdinal` key→color).
- Excel **per-point** `c:dPt` override → branch the `.fill` closure on the datum's index
  and return the override color. Direct, no library gap.
- Caveat: `a:schemeClr` (theme references) must be resolved against the workbook/chart
  theme to an RGB before use — an approximation step, not a blocker. Also note the auto
  palette is only **5** colors (`chart_1..chart_5`); beyond 5 series we must supply colors
  explicitly (which, when reading `solidFill`, we do anyway).

**(3) 3D options → OUT, flatten to 2D.** No z-axis/perspective/geometry exists
(gpui-doc:204-208). Assessment of flattening: **generally acceptable, sometimes an
improvement.** The `c:bar3DChart` data model is the same `barDir`+`grouping`+`ser`/`cat`/
`val` as 2D, so we read it and render the 2D clustered/stacked column. Because 3D bars
distort magnitude via perspective, a clean 2D rendering is often *more* readable. Real
losses: (a) true **depth-axis** 3D — where `c:serAx` arranges series front-to-back instead
of clustering — collapses; approximate it as clustered. (b) `c:shape` (cylinder/cone/
pyramid) and `c:view3D` rotation are dropped. Verdict: flatten, and drop the 3D chrome.

**(4) Legends / numeric axis labels → missing, need Level B.**
- **Legend:** absent from the whole module (gpui-doc:194). Must be **hand-built** — but this
  is cheap in GPUI: a `div` row/column of colored swatches + series names derived from each
  `c:ser`'s `c:tx`, placed outside the plot. Not a charting-engine problem.
- **Numeric value-axis labels:** absent on every chart type (gpui-doc:191). Hover tooltips
  show values, but static axis numbers require DIY: `ScaleLinear` gives min/max→pixel with
  **no tick generation and no "nice" rounding** (gpui-doc:36), so we'd generate ticks
  ourselves (respecting `c:valAx` scaling / number format) and paint them with `AxisText`.
  Feasible, but real work, and needed for any chart where reading magnitudes matters.

---

## 5. Verdict

**(a) Simple single-series column / bar — CLEAN WIN (Level A).** `BarChart` renders both
vertical (`barDir=col`) and horizontal (`barDir=bar`) well: per-bar/per-point color from
`solidFill`/`dPt`, data labels, category-axis labels, rounded corners, and a polished hover
tooltip, all as direct wrapper calls. The only gaps are a missing numeric value axis (4
unlabeled gridlines instead of numbers) and no built-in title (wrap in a `div`). For a lot
of real spreadsheets this is genuinely good.

**(b) Common multi-series clustered / stacked — ACHIEVABLE BUT REAL WORK (Level B), the
family's main lift.** The `BarChart` struct does **not** do grouped or stacked, so the most
common Excel bar chart (clustered column, 2–4 series, legend) needs a custom `Plot`.
Stacked is well de-risked (the `Stack` primitive + a working ~130-line reference incl.
tooltip); clustered is comparable effort with simpler math; percentStacked is stacked plus
a normalization pass. On top of that we must **hand-build a legend** (cheap) and, if we want
readable magnitudes, **hand-build numeric axis ticks** (moderate). Net: it will look good
and is entirely doable, but budget it as engineering, not configuration — and `c:gapWidth`/
`c:overlap` will be approximated (bar padding is hard-coded).

**(c) 3D column / bar — DROP, FLATTEN TO 2D.** No 3D in the library and none coming.
Flatten `c:bar3DChart` to its 2D clustered/stacked equivalent (same data model), which is
usually acceptable and often more readable; accept losing depth-axis (`serAx`) separation,
`c:shape` geometry, and `c:view3D` rotation.

**Bottom line:** we can render the Bar/Column family *well* — single-series is a clean
wrapper win; the common multi-series case is a solid, good-looking result that costs a
custom `Plot` + legend + optional axis-ticks; only true 3D must be dropped/approximated. The
per-bar `.fill` API makes faithful Excel color mapping (`solidFill`/`dPt`) a strength rather
than a gap.
