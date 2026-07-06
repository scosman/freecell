# load-save — agent image review (Gate 4, load half)

Each PNG under `results/` was rendered by `chart-render`'s widgets **FROM a `chart_model::Chart`
that was parsed out of the authored `.xlsx` fixture** (`fixtures/charts_basic.xlsx`) — i.e. the
image is proof the full seam **parse → chart-model → render** works on a real file, not a
hand-built in-memory chart. One independent reviewer agent per image (functional_spec §6,
decision #3: single verdict per image), judged against the §6 rubric.

| Image | Chart (loaded from xlsx) | Verdict | Notes |
|---|---|---|---|
| `loaded_column.png` | Clustered column "Quarterly Sales by Product" (Widgets/Gadgets × Q1–Q4) | **PASS** | Correct clustered geometry; Widgets (blue) / Gadgets (orange) side-by-side across Q1–Q4; legend + both axis titles legible; numeric axis 0–200 at 50-unit ticks; no clipping. Series colors are the ones parsed from each `c:ser/c:spPr` fill. |
| `loaded_line.png` | Multi-series line "Sales Trend by Product" (Widgets/Gadgets × Q1–Q4) | **PASS** | Two straight-segment lines on one shared value axis with dot markers; lines cross between Q2–Q3 as the data implies; legend + axis titles legible; ticks at 20-unit intervals; no garbage/blank. |
| `loaded_pie.png` | Pie "Quarterly Totals" (Q1–Q4 slices of the Total column) | **PASS** | Four distinct-colored wedges of differing sizes; per-slice percentage labels 21% / 27% / 24% / 28% match the cached totals (200/260/230/270 of 960); legend maps Q1–Q4 to slice colors; no clipping. |

**All three PASS.** The loaded values, series names, colors, titles, axis titles, and legend all
came straight out of the chart XML's cached `numCache`/`strCache` (no formula evaluation), and
render identically to the hand-built scenes from Phases 1–2.
