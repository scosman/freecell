# Agent image review — chart-render

Per functional_spec §6: each captured PNG is judged by a reviewer agent against the rubric.
For Phase 0 a single verdict is enough (Gate 1 upgrades to a 3-agent panel). Verdicts are
advisory input to the human go/no-go decision, not an automated gate.

Rubric: (1) correct chart type & geometry; (2) series clearly drawn / multi-series
distinguishable; (3) legend present, correct label + series→color mapping; (4) title + axis
titles present and legible; (5) numeric value axis with readable ticks at sensible intervals;
(6) no clipping/overlap/garbage/blank; (7) overall — would a user accept this as a real chart?

## bar_single.png — verdict: PASS

Single-series vertical column chart of quarterly revenue. Reviewed by a fresh reviewer
sub-agent (clean context, viewed the actual pixels).

| # | Rubric point | Result |
|---|---|---|
| 1 | Chart type & geometry | YES — four upright columns on a shared baseline, evenly spaced, heights vary sensibly |
| 2 | Series clearly drawn (single series) | YES — one consistent blue series, each bar solid |
| 3 | Legend + color mapping | YES — right-side "Revenue" legend, blue swatch matches the bar fill |
| 4 | Title + axis titles | YES — "Quarterly Revenue", "USD (thousands)", "Quarter" all legible |
| 5 | Numeric value axis | YES — 0/50/100/150/200 at even 50-unit intervals with gridlines |
| 6 | No clipping/overlap/garbage/blank | YES — clean render, tallest bar (Q4 ≈175) within the 200 gridline |
| 7 | Overall acceptance | YES — reads as a polished, conventional column chart |

**Notes:** Q1≈120, Q2≈90, Q3≈150, Q4≈175 are all legible against the axis; every rubric
element (title, both axis titles, numeric ticks, category labels, legend with matching swatch)
is present. Comfortably clears the Phase 0 bar of "a recognizable, non-blank bar chart" with no
defects observed.

## line_multi.png — GATE 1 (make-or-break) — 3-agent panel majority: **PASS** (3/3 PASS)

The make-or-break multi-series line image (functional_spec §7; §10 decision #3: Gate 1 gets a
3-agent panel, not a single verdict). Three **independent, fresh** reviewer sub-agents each
viewed the actual pixels of `line_multi.png` (a 3-series line — North/South/West — over Jan–Jun
on one shared value axis) and judged it against the §6 seven-point rubric.

| Reviewer | Verdict |
|---|---|
| A | **PASS** |
| B | **PASS** |
| C | **PASS** |
| **Majority** | **PASS** |

Consensus per rubric point (all three agreed YES on every point):

| # | Rubric point | Result |
|---|---|---|
| 1 | Chart type & geometry | YES — three lines with **straight** segments between points (no smoothing), dot markers at every point |
| 2 | Multi-series distinguishable | YES — three distinct colors (blue North, orange South, green West), all on **one shared** 20–100 value scale so heights compare; crossings read correctly |
| 3 | Legend + color mapping | YES — right-side North/South/West, each swatch color matches its line exactly |
| 4 | Title + both axis titles | YES — "Regional Sales by Month", "Units (thousands)", "Month" all present + legible |
| 5 | Numeric value axis | YES — 20/40/60/80/100 evenly spaced (interval 20) with gridlines, readable |
| 6 | No clipping/overlap/garbage/blank | YES — clean; Jun points near the right edge but not clipped; legend clear of the plot; no label collisions |
| 7 | Overall acceptance | YES — reads as a real, "publication-quality" multi-series line chart |

**Panel notes:** Unanimous PASS. Two of three reviewers flagged one **non-defect** cosmetic
observation: the value-axis title is placed horizontally above the axis rather than rotated
vertically alongside it — clearly legible and correctly associated, so not counted against any
rubric point. **Gate 1 is cleared:** building an acceptable multi-series line chart on the raw
`Line` primitive over one shared `ScaleLinear` (with our own nice value axis, category axis,
legend, and multi-series color cycle) is demonstrably achievable. The PoC proceeds to Gate 2.

## line_single.png — verdict: PASS (single-agent sanity)

Supporting single-series line sanity scene (monthly website visitors, Jan–Jun). One reviewer
confirmed: a single straight-segment line rising left-to-right with dot markers, a readable
40–90 numeric value axis, month category labels, chart + axis titles, and a one-entry legend
whose swatch matches the line. Non-blank, no defects — the line widget's axis/grid/marker
scaffolding reads cleanly on its own. (Sanity check only; the graded gate is `line_multi`.)

---

# Phase 2 — Gate 2: harder layouts (single-agent verdicts)

Gate 2 (functional_spec §7) stresses the layouts that carry the research-flagged traps. Per
§10 decision #3, Gate 2 uses a **single** reviewer verdict per image (the 3-agent panel was
Gate 1 only). Each image below was reviewed by an independent, fresh sub-agent that viewed the
actual pixels and judged against the §6 seven-point rubric.

## Gate 2 verdict table

| Scene | Type | Verdict |
|---|---|---|
| `bar_single` (re-review) | single-series column | **PASS** |
| `bar_horizontal` | single-series horizontal bar | **PASS** |
| `bar_grouped` | grouped (clustered) column, 3 series | **PASS** |
| `bar_stacked` | stacked column, 3 series | **PASS** |
| `bar_percent_stacked` | 100%-stacked column, 3 series | **PASS** |
| `area_stacked` | stacked area, 3 series | **PASS** |
| `area_percent_stacked` | 100%-stacked area, 3 series | **PASS** |
| `pie` | single-series pie, 5 slices | **PASS** |
| `doughnut` | single-series doughnut, 5 slices | **PASS** |

**All nine Gate-2 images PASS.** No wholesale FAIL of grouped/stacked → this is a **GO** signal
for the harder-layout capability (not a PARTIAL-GO). The bar family (both orientations, all
three groupings), hand-rolled stacked/percent **area**, and the synthesized-palette
**pie/doughnut** all render as charts a user would accept.

## Per-scene notes

- **`bar_single`** (re-reviewed because Phase 2 generalized `bar.rs`, widening the columns from
  the old 30px `ScaleBand` cap): PASS on all 7 points — four upright bars, 0/50/100/150/200
  axis with gridlines, Q1–Q4 labels, title + both axis titles, matching legend. No regression.
- **`bar_horizontal`**: PASS. Correct swapped geometry — five bars grow left-to-right from a
  left value axis, region names down the left, numeric ticks 0–200 along the bottom, legend
  matches. (Point 4 PARTIAL note: the left category axis shows a "Region" caption rather than a
  rotated axis title — the same cosmetic non-defect flagged at Gate 1, not docked.)
- **`bar_grouped`**: PASS. Four quarter groups, three side-by-side non-overlapping columns each,
  three distinct colors, one shared 0–200 axis, legend Widgets/Gadgets/Gizmos matches.
- **`bar_stacked`**: PASS. Four single columns each split into three cumulative segments; value
  axis 0–400 covers the tallest stack (~365); stacking order matches the legend. (Point 4
  PARTIAL: value-axis caption, as above.)
- **`bar_percent_stacked`**: PASS. Four full-height columns of share segments summing to 100%;
  value axis labeled 0%–100% at 20% intervals; legend matches. (Point 4 PARTIAL: caption.)
- **`area_stacked`**: PASS — the nastiest layout. Three filled bands stack cumulatively (bottom
  band from the axis, upper bands on the running total, wavy per-x baselines), value axis 0–200
  reaches the ~153 total, legend Direct/Search/Social matches. The hand-rolled polygon fork of
  the scalar-baseline `Area` primitive works.
- **`area_percent_stacked`**: PASS. Three bands together fill 0–100% at every quarter; value
  axis 0%–100%; legend matches.
- **`pie`**: PASS. Five DISTINCT-colored wedges (not the monochrome disc an unset gpui palette
  would give), on-slice percentage labels summing to 100%, title, slice→color legend
  (Alpha/Beta/Gamma/Delta/Other). The synthesized-palette color-mapping crux is solved.
- **`doughnut`**: PASS. Same as pie with a hollow center (inner radius = 0.55 × outer); five
  distinct arcs, percentage labels, matching legend.

**Cosmetic non-defect (carried from Gate 1):** several reviewers noted the value-axis *title*
is a horizontal caption above/below the plot rather than a rotated vertical axis title. It is
legible and correctly associated, so no rubric point was docked; a ship-quality follow-on could
rotate it. Not worth it for the PoC.

---

# Phase 3 — Gate 3: scatter (single-agent verdicts)

Gate 3 (functional_spec §4, §7) tests scatter — the one type the research flagged as a genuine
step-change, because it needs **two numeric axes** (X is a `ScaleLinear`, not a band/point
category scale) and **standalone dot marks** from `c:xVal`/`c:yVal` pairs. Per §10 decision #3
this uses a **single** reviewer verdict per image. Each image below was reviewed by an
independent, fresh sub-agent that viewed the actual pixels and judged against the §6 seven-point
rubric. Checkpoint semantics are lower-stakes: a FAIL would record scatter *out-of-scope for the
follow-on*, not a whole-project NO-GO.

## Gate 3 verdict table

| Scene | Type | Verdict |
|---|---|---|
| `scatter_single` | single-series scatter, 10 points | **PASS** |
| `scatter_multi` | multi-series scatter, 3 species clusters | **PASS** |

**Both Gate-3 images PASS.** Scatter renders as a chart a user would accept, on two numeric axes
with dots, reusing the Gate 1/2 title / axis-title / legend / nice-tick / palette scaffolding.
→ **Scatter is IN-scope for the follow-on.**

## Per-scene notes

- **`scatter_single`** (Ad spend vs Revenue, 10 points): PASS on all 7 points. Standalone dots
  (no connecting line) trending clearly up-and-to-the-right; a numeric **X** axis along the
  bottom (0/10/20/30/40/50) *and* a numeric **Y** axis on the left (20/40/60/80/100), each with
  evenly spaced readable ticks and dashed gridlines; chart title plus **both** axis titles
  ("Ad spend (USD thousands)" / "Revenue (USD thousands)"); one-entry legend whose blue swatch
  matches the dots. No clipping. The reviewer confirmed it reads as a legitimate scatter of the
  data.
- **`scatter_multi`** (iris petal length vs width, Setosa/Versicolor/Virginica): PASS on all 7
  points — the graded Gate-3 case. Three **distinct-colored** dot clusters (blue low-left,
  orange middle, green upper-right), all sharing ONE pair of numeric axes (X 0/2/4/6/8, Y 0/1/2/3
  with gridlines); chart title plus both numeric axis titles; a three-entry legend whose swatch
  colors match the three clusters exactly. Dots, not lines; no clipping. The multi-series color
  cycle + legend mapping carry over to two-numeric-axis dot marks unchanged.

**Cosmetic non-defect (carried from Gates 1–2):** the value-axis title is a horizontal caption
rather than a rotated vertical title — legible and correctly associated, not docked. Neither
reviewer flagged any new scatter-specific defect.
