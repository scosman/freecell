# Chart Comparison — Area

Per-type Excel-vs-`gpui-component` comparison for the **Area** chart family, part of the
FreeCell charts research phase. Scope reminder: we render **at most what `gpui-component`
already renders** — this doc assesses whether an `.xlsx` area chart can be shown *well*,
or whether major gaps remain.

Primary inputs: `excel-chart-data-model.md`, `gpui-component-charts.md` (both in this
folder). gpui facts are cited `path:line` at the pinned rev
`a9a7341c35b62f27ff512371c62419342264710c`; area-family sources verified against the
actual `chart/area_chart.rs`, `plot/shape/stack.rs`, and `plot/shape/area.rs`.

**Framing (carried through):** `gpui-component` gives us two levels —
- **Level A** = use the chart **structs** as-is (`AreaChart`).
- **Level B** = **DIY** on the `plot/` primitives (`Stack` + `Area` + scales/axis).
Both are assessed below, because the single most useful Excel area variant is one Level A
does **not** do.

---

## 1. Excel side — `c:areaChart` data model

An Excel area chart is stored as a `c:areaChart` chart-group element inside
`c:plotArea` (2-D) or `c:area3DChart` (3-D). Like bar/line, its behavior is driven by a
single **`c:grouping`** knob and one-or-more `c:ser` (series) children.

### 1a. `c:grouping` — the variant selector

`c:grouping` on `c:areaChart` takes one of (per `excel-chart-data-model.md` §5, ECMA-376
`ST_Grouping`):

| `c:grouping` value | What it draws | Baseline behavior |
|---|---|---|
| `standard` | **Overlaid** areas — every series drawn from the zero axis, layered on top of each other. | All series share baseline `0`; later series **occlude** earlier ones. |
| `stacked` | **Stacked** areas — each series sits *on top of* the cumulative sum of the series below it. | Per-point baseline = running total of lower series; bands add up. |
| `percentStacked` | **100%-stacked** areas — like `stacked` but each category column is normalized so the stack fills 0–100%. | Per-point baseline = running total of the *normalized* shares; total is always 100%. |

**CRITICAL real-world nuance:** for area charts, **`stacked` and `percentStacked` are the
common, deliberate uses; `standard` (overlay) is rare.** Overlaid filled areas occlude one
another (a tall series hides the ones behind it), so people almost never *choose* overlaid
areas on purpose — if they want overlapping trends they use a line chart. The reason to
pick an *area* chart is precisely to show **part-to-whole composition over a category/time
axis**, which is exactly `stacked` / `percentStacked`. So the canonical `.xlsx` area chart
is the stacked one — **the variant `gpui-component` does NOT do natively.**

### 1b. Series & data slots

Standard `c:cat`/`c:val` model (area is not scatter/bubble):

- Multiple **`c:ser`** siblings inside `c:areaChart`, one per dataset. Each carries
  `c:tx` (series name), `c:val` (`c:numRef` → `c:f` range + `c:numCache`), and typically
  **shares the same `c:cat`** category axis across all series (`excel-chart-data-model.md`
  §4d). `c:idx`/`c:order` order them (also the stacking order, bottom→top).
- **Per-series color/fill** via `c:ser` → `c:spPr` → `a:solidFill` (or gradient/pattern
  fill) plus `a:ln` for the top stroke; color is `a:srgbClr val="RRGGBB"` or theme
  `a:schemeClr` (§5). Area series are usually semi-transparent so lower bands show through
  in the overlay case.
- Chrome: `c:catAx`/`c:valAx`, optional `c:title`, `c:legend`, `c:dLbls` — same as other
  cat/val charts.

### 1c. 3-D area — `c:area3DChart`

Adds a `c:view3D` (rotation/perspective) on `c:chart`, a series axis `c:serAx`, and depth.
Rendered as receding 3-D ribbons. Rare, and structurally a superset we cannot honor.

---

## 2. gpui side — struct overlay vs. primitive DIY

### 2a. Level A — the `AreaChart` struct: multi-series **overlay**, not stacking

`AreaChart<T,X,Y>` **is genuinely multi-series** — `y`, `strokes`, `fills`, `names`,
`stroke_styles` are all `Vec`s (`area_chart.rs:29-40`), and each `.y()/.stroke()/.fill()/
.name()` call **pushes** a new series (`area_chart.rs:88-101`). Paint loops over the series
(`area_chart.rs:203-230`). So far it looks like a stacked-area chart.

**But it OVERLAYS; it does not stack.** In the paint loop every series is drawn with a
constant baseline:

```
Area::new()
    .y0(height)                          // area_chart.rs:224  — SAME baseline for every series
    .y1(move |d| y.tick(&y_fn(d)))       // area_chart.rs:225
```

`y0(height)` is the bottom of the plot for **all** series, so each area rises from the zero
axis independently and layers over the others — Excel's `standard` grouping, not `stacked`.
The story pane titled **"Area Chart - Stacked"** is a **misnomer**; there is no stacking
helper anywhere in `AreaChart`.

Two further consequences that make the struct *unable* to fake stacking:

- **The y-scale domain is the max single-series value, not the cumulative total.** The
  domain is built by flat-mapping every series' values and chaining `Y::zero()`
  (`area_chart.rs:150-157`) — i.e. `max(all individual values)`, **not** `max(column
  sums)`. A stacked chart needs the axis to reach the total height of the tallest stack; this
  scale can't represent that even if the baselines were fixed.
- **The builder is positional / index-aligned and fails silently.** Per-series fill/stroke
  are read by index with a fallback: `self.fills.get(i).unwrap_or(&chart_2@0.4)` and
  `self.strokes.get(i).unwrap_or(&chart_2)` (`area_chart.rs:209-219`). If the series `Vec`s
  drift out of sync (e.g. you set a stroke for series 0 but not series 1), it **silently**
  falls back to the default `chart_2` instead of erroring — fragile when mapping N Excel
  series programmatically.

What the struct *does* give (Level A wins):
- Per-series **stroke + fill**, solid **or gradient** (`fill` takes anything `Into<Background>`,
  incl. `linear_gradient`) — `area_chart.rs:93-101`, defaults `chart_2` stroke / `chart_2@0.4`
  fill (`:212-214`).
- **Category (x) axis** with labels (`area_chart.rs:180-191`), `.grid()`, `.x_axis()` toggles.
- **Multi-row hover tooltip** — one dot + one row (swatch/name/value) per series
  (`area_chart.rs:237-311`). This is the only legend-like surface.
- Curve styles per series: `.natural()/.linear()/.step_after()`.

What it does **not** give (same module-wide gaps as the rest of the library,
`gpui-component-charts.md` §3): **no legend**, **no chart title**, **no numeric value-axis
labels** (only ~4 fixed unlabeled gridlines, `area_chart.rs:194-200`), no data labels, no
true stacking, no 3-D.

### 2b. Level B — DIY true stacking on the primitives

The primitive layer has the pieces for stacking math but, crucially, **not a shape that
fills a variable baseline**:

- **`Stack` primitive (`plot/shape/stack.rs`) — correct stacking math.** A d3-shape port:
  give it `data`, `keys` (series), and a `value(&T, key) -> Option<f32>` accessor; `series()`
  returns per-series `StackPoint { y0, y1, data }` with cumulative baselines computed in key
  order (`stack.rs:99-133`, verified by its unit test `stack.rs:149-190`:
  apples 0→10, bananas 10→30, cherries 30→60). This gives you exactly the per-point `(y0,y1)`
  a stacked area needs. **Note:** it does *absolute* stacking only — there is **no
  percentStacked mode**; for 100%-stacked you must normalize each category column to sum=1
  (divide `y0`/`y1` by the column total) *before or after* `Stack`.
- **`Area` primitive (`plot/shape/area.rs`) — CANNOT consume a per-point baseline.** Its
  baseline is a **scalar** `y0: Option<f32>` (`area.rs:11`, setter `:56-59`), and the fill
  path is closed with a **flat horizontal** bottom edge — two `line_to` calls at the single
  `y0` across the whole width (`area.rs:170-176`). There is no `y0` *closure*. So you cannot
  hand a stacked band's lower curve to `Area`.

**This is the important Level-B finding.** The library's known stacking precedent is the
**stacked-BAR** story demo (`Stack` + `Bar`), which works because a `Bar` is a rectangle that
naturally takes its own `y0..y1` per bar. **Stacked *area* has no equivalent easy compose** —
`Stack` gives the numbers but `Area` can't draw a wavy baseline. True stacked area therefore
requires either:
  1. **Fork/extend the `Area` primitive** to accept a per-point `y0` (a small, local patch:
     make `y0` a `Box<dyn Fn(&T)->Option<f32>>` and trace the lower boundary in reverse when
     closing the path), or
  2. **Hand-roll the filled polygons** in a custom `Plot` with `PathBuilder` directly — trace
     each series' upper boundary forward, then the previous series' upper boundary backward,
     and close — using `Stack`'s `(y0,y1)` and a `ScaleLinear` whose domain is the **column
     sums**.
Either is real work beyond wiring existing structs, and beyond the stacked-bar precedent.

---

## 3. Mapping table — Excel area variants → gpui level

| Excel area variant | gpui level | Render quality |
|---|---|---|
| **Single-series area** (one `c:ser`, any grouping) | **Level A** (`AreaChart` with one `.y()`) | **WELL.** Filled area, category axis, curve, hover, per-series color. Only losses are library-wide: no numeric y-axis labels, no legend/title. A clean win. |
| **Overlaid multi-area** — Excel `grouping="standard"` | **Level A** (`AreaChart`, multiple `.y()`) | **Faithful — but this is the *least common* Excel variant.** The struct's native behavior (all series from a shared baseline, semi-transparent overlays) matches `standard` exactly. Same chrome losses as above. The catch: real files rarely *want* this. |
| **Stacked area** — Excel `grouping="stacked"` | **Level B** (`Stack` math + forked/hand-rolled `Area` fill) — **OUT of Level A** | **GAP.** Not achievable with the struct (constant baseline + single-series-max y-scale). Level B is correct but needs an `Area` fork or hand-rolled polygons; more than the stacked-bar precedent. |
| **percentStacked area** — Excel `grouping="percentStacked"` | **Level B** (`Stack` + a normalization pass + forked/hand-rolled `Area` fill) — **OUT of Level A** | **GAP.** As stacked, plus per-column normalize-to-1.0 (`Stack` has no percent mode). Axis should read 0–100%, but the library has no numeric axis labels anyway. |
| **3-D area** — `c:area3DChart` | **OUT** — flatten | **Approximate.** No 3-D anywhere in the library (`gpui-component-charts.md` §3 "3D"). Flatten to the 2-D equivalent of its grouping (overlay→Level A, stacked→Level B). Depth/perspective dropped. |

**Explicit takeaway:** Level A hands you the **overlay** variant (Excel `standard`) — the
one people rarely pick on purpose — and **misses both common variants** (`stacked`,
`percentStacked`). The "multi-series" headline for `AreaChart` is real but only in the
overlay sense.

---

## 4. Owner's asks

- **Multiple datasets** — Partially. The `AreaChart` struct *is* multi-series, so multiple
  `c:ser` map to multiple `.y()` pushes — **but only as overlays.** Correct multi-series
  *stacking* (the usual reason a file has multiple area series) needs the `Stack` primitive
  plus a variable-baseline fill (Level B). Watch the fragile index-aligned builder when wiring
  N series (`area_chart.rs:209-219`: silent fallback to `chart_2`).
- **Coloring** — Feasible and reasonably good. Per-series **stroke + fill**, solid or gradient
  (`area_chart.rs:93-101`), maps from each series' `c:spPr` fill/line. Caveat: the positional
  builder means you must push colors in lockstep with `.y()` or silently get the default.
- **3-D** — **OUT.** Flatten `c:area3DChart` to its 2-D grouping equivalent; drop perspective,
  depth, and the series axis.

---

## 5. Verdict — render area well, or major gaps?

**Mixed, and it hinges on `c:grouping`.**

- **Single-series area = WIN.** Level A renders it well (fill, category axis, curve, hover,
  color); only library-wide chrome losses (no numeric y-axis, no legend/title).
- **Overlaid multi-area (Excel `standard`) = the struct nails it, but it's rarely what the file
  wants** — overlay is the uncommon, occlusion-prone variant nobody picks deliberately.
- **Stacked / percentStacked area (the COMMON case) = GAP.** Not doable with the struct; needs
  Level B `Stack` math **plus** an `Area` fork or hand-rolled polygons (the `Area` primitive's
  baseline is a scalar, so it's harder than the existing stacked-*bar* demo). percentStacked
  additionally needs a normalization pass (`Stack` has no percent mode).
- **3-D area = dropped**, flattened to its 2-D grouping.

**Do NOT shortcut a stacked file into the overlay struct.** Rendering a `grouping="stacked"`
chart as `AreaChart` overlays is **wrong twice over**: (1) visually — semi-transparent bands
occlude/blend and no longer read as a cumulative composition; and (2) quantitatively — the
picture implies each series is measured from zero, so the *totals mislead* (a viewer reads the
top band's height as its own value rather than its contribution to the stack), and the y-scale
would top out at the largest single series rather than the stack total (`area_chart.rs:150-157`).
For a spreadsheet whose whole point is correct numbers, that is a data-integrity error, not just
a cosmetic one. **Recommendation:** ship single-series and (if wanted) overlaid area at Level A
now; treat stacked/percentStacked as a Level-B item (fork `Area` for a per-point baseline, or
draw the bands by hand), and **never approximate a stacked area as an overlay.**
