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
