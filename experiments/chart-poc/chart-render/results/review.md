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
