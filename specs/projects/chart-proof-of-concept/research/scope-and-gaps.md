# Chart Support — Scope & Gaps

Research-phase scoping note for the FreeCell **charts** project. It answers the central
scoping question — *how much chart DISPLAY should we build?* — by intersecting what Excel
files can contain with what `gpui-component` can render, then judging that intersection
against how people actually use charts. It feeds a human discussion; it does not decide.

**Guardrail (from the project brief):** render *"at most what `gpui-component` already
renders — not building a charting engine."* This doc treats that guardrail as the thing
to pressure-test: the literal reading (Tier 1) may be too weak to ship, and the useful
reading (Tier 2) is a little more than "just use gpui's chart structs."

**Inputs (Wave-1 research — cited throughout):**
- `excel-chart-data-model.md` — what the `.xlsx` can store (16 classic chart-group
  elements + the extended family) and the on-disk data model.
- `gpui-component-charts.md` — exactly what `gpui-component` can render, at pinned rev
  `a9a7341`. Its **Level A / Level B** framing is the backbone of this doc.
- `ironcalc-chart-exposure.md` — IronCalc exposes **no** chart data; we parse chart XML
  ourselves; **cached values make display-only tractable**; save-preservation is a
  separate hard problem, out of scope.

The two capability levels (from `gpui-component-charts.md §5`):
- **Level A — the five chart *structs* as-is:** `LineChart`, `AreaChart`, `BarChart`,
  `PieChart`, `CandlestickChart`. Single-series column/bar/line/area/pie/donut;
  category-axis labels; per-bar / per-slice color; data labels (bar + pie); hover
  tooltips (line/area/bar only). **Missing:** legend, numeric value-axis labels, chart
  title, real multi-series (Area only *overlays*; Line/Bar are single-series), stacking /
  grouping.
- **Level B — DIY on the `plot/` primitives** (`Line` / `Bar` / `Area` / `Arc` / `Pie` /
  `Stack` + scales + axis/grid/tooltip). We write our own chart widgets to get
  multi-series, grouped/stacked, legends, and numeric axes. More work; edges toward "chart
  rendering business" — but reuses gpui's D3-derived primitives, and the library itself
  ships a hand-rolled stacked-bar example doing exactly this (`gpui-component-charts.md §3`).

---

## 1. The in-scope intersection (Excel chart types ∧ gpui-component)

These are the Excel/OOXML types `gpui-component` can plausibly render. "Level A"
= a stock chart struct covers it; "Level B" = needs DIY on the primitives.

| Excel chart type | OOXML element (`excel-chart-data-model.md §2a`) | gpui-component target | Single-series → level | Multi-series / stacked / legend → level | Notes |
|---|---|---|---|---|---|
| **Column** (vertical) | `c:barChart` `barDir=col` | `BarChart` (`Bottom`/`Top` alignment) | **Level A** | **Level B** (`Stack`+`Bar` for clustered/stacked; legend + numeric axis DIY) | The single most common business chart is a *multi-series* clustered column → Level B. |
| **Bar** (horizontal) | `c:barChart` `barDir=bar` | `BarChart` (`Left`/`Right` alignment) | **Level A** | **Level B** | Horizontal orientation IS supported (4 `BarAlignment`s). |
| **Line** | `c:lineChart` | `LineChart` | **Level A** | **Level B** (multi-line needs raw `Line`; each `LineChart` normalizes its own y-domain) | Single line per `LineChart` struct. No numeric y-axis in either level without DIY. |
| **Area** | `c:areaChart` | `AreaChart` | **Level A** | **Level B for *true* stacking** | `AreaChart` takes multiple `.y()` series but **overlays, does not stack** — the "stacked" story pane is a misnomer (`gpui-component-charts.md §2`). Cumulative stacking = Level B via `Stack`. |
| **Pie** | `c:pieChart` | `PieChart` | **Level A** | n/a (single ring) | Per-slice color + leader-line data labels. No hover/interactivity. |
| **Doughnut** (single ring) | `c:doughnutChart` | `PieChart` + `inner_radius` | **Level A** | n/a | Donut hole well-supported. **Multi-ring** doughnut (multiple `c:ser`) is out — see §2. |
| **Stock ≈ Candlestick** *(borderline)* | `c:stockChart` (High-Low-Close / OHLC order) | `CandlestickChart` | **Level A (weak)** | n/a | Only OHLC maps; **no volume, no color control, no tooltip/interactivity** (`gpui-component-charts.md §2`). Finance-only, rare in general sheets — recommend **defer** even though technically renderable. |

**Read of the table:** the intersection is exactly the everyday chart set — column, bar,
line, area, pie/donut — plus a weak candlestick. The catch is the **level split**:
*single-series* versions of all of these are cheap Level A, but the *multi-series /
stacked / legend* versions that dominate real reports require Level B. That split, not the
type list, is the real scoping decision (§4).

---

## 2. What we leave out — and how much it hurts

Everything below is an Excel type `gpui-component` **cannot** render as-is. For each: a
one-line "what it is" + a prevalence/importance rating (how often it shows up in real
spreadsheets). Ratings: **High** (common enough to matter for MVP) · **Medium** · **Low** ·
**Rare**.

| Omitted Excel type | What it is | Prevalence / importance | Why it's out |
|---|---|---|---|
| **Scatter (XY)** ⚠️ | Plots paired numeric (X, Y) points on two value axes; the standard tool for correlation / relationship between two variables. | **HIGH — the biggest omission.** Consistently ranked a foundational, top-tier chart type; the canonical "two numeric variables" visual with no substitute. | `gpui-component` has **no scatter struct**, and even the primitives are category-vs-value oriented (`ScaleBand`/`ScalePoint` for X, `Line`/`Bar`/`Area` shapes). Scatter needs points on **two `ScaleLinear` axes** — genuinely net-new plotting, not a wrapper. Data model even differs: scatter uses `c:xVal`/`c:yVal`, not `c:cat`/`c:val` (`excel-chart-data-model.md §4e`). **Flag loudly: a very common, genuinely useful type we cannot show.** |
| **Combo / multi-plot** | Two chart types sharing one plot area (e.g. columns + a line), often with a secondary value axis. Stored as multiple chart-group elements in one `c:plotArea` (`excel-chart-data-model.md §2a`) — there is no "combo" element. | **Medium-High.** Very common in business dashboards (actuals-vs-target, volume + %). | No combo support; needs Level B/C (two plot passes + a second axis). Secondary axes are explicitly in the "swamp" (`ironcalc-chart-exposure.md §4`). |
| **Bubble** | Scatter + a third dimension encoded as marker size (`c:xVal`/`c:yVal`/`c:bubbleSize`). | **Low.** Niche; mostly consulting/portfolio quadrant decks. | Needs scatter first, plus sized markers. |
| **Radar / spider** | Multivariate values on radial spokes (`c:radarChart`). | **Low.** Occasional (skills, KPI comparisons). | No radial category geometry in the primitives. |
| **Stock volume variants** | Volume-HLC / Volume-OHLC — candlestick + a volume bar sub-plot (a combo). | **Rare** outside finance. | Combo + candlestick; candlestick itself is already the weakest struct. |
| **ofPie (Pie-of-Pie / Bar-of-Pie)** | Breaks small slices out into a secondary pie or bar (`c:ofPieChart`). | **Rare.** | Needs a second linked plot + split logic; no support. |
| **Multi-ring doughnut** | Doughnut with several `c:ser` = concentric rings (one ring per series). | **Rare.** | `PieChart` renders one ring only. |
| **Surface** | 3D/contour topographic surface over two independent variables (`c:surfaceChart`/`c:surface3DChart`). | **Rare.** | No 3D and no surface geometry. |
| **All 3D variants** | 3-D column/bar/line/pie/area/surface (`c:bar3DChart`, `c:pie3DChart`, …). | **Low-Rare**, and widely discouraged. | **No 3D anywhere** in the library — confirmed, zero z-axis/perspective code (`gpui-component-charts.md §3`). Best we could do is flatten to 2D. |
| **Extended family** (sunburst, treemap, waterfall, histogram, Pareto, box & whisker, funnel, region map) | Excel 2016+ statistical/hierarchical types in the `cx:` "chartex" schema (`excel-chart-data-model.md §2b`). | **Low-Rare** in typical files; a few (waterfall, treemap) rising. | `gpui-component` renders none; different XML schema; explicitly out (`excel-chart-data-model.md §2b`). |

**The one that hurts: scatter (XY).** It is the only *high-prevalence, genuinely
non-substitutable* type in this table. Column/line/pie/area cover comparison, trend, and
part-to-whole; scatter is the relationship/correlation workhorse with no fallback among
the in-scope types. Combo is the runner-up omission (common, but at least each half is a
type we can draw). Everything else in this table is Low/Rare and cheap to concede.

---

## 3. Prevalence reality check — does the in-scope set cover "most charts people have"?

Two independent lines of evidence say the everyday chart set is small and that our
in-scope types sit at the top of it.

**Guidance / "which charts to use" consensus.** Bar/column, line, and pie are named the
three most-used chart types in business reporting across multiple references, with area
and scatter rounding out the "foundational" set. Bar charts are repeatedly called the
default/first choice for comparison, line charts the default for trends, pie for
part-to-whole ([Atlassian — Essential Chart Types](https://www.atlassian.com/data/charts/essential-chart-types-for-data-visualization);
[Microsoft — Available chart types in Office](https://support.microsoft.com/en-us/office/available-chart-types-in-office-a6187218-807e-4103-9e0a-27cdb19afb90);
[HubSpot — types of graphs for data visualization](https://blog.hubspot.com/marketing/types-of-graphs-for-data-visualization)).

**Actual usage data (Datawrapper's yearly "what did users make" counts).** Across
published visualizations, **line and bar charts are consistently the top two chart types**
(tables and maps aside); in 2024 tables were 21.6% of all visualizations, and split/paired
bars alone held ~5% (6th place) — i.e. a *handful* of types account for the large majority
of real charts ([Datawrapper — popular chart types 2025](https://www.datawrapper.de/blog/popular-chart-types-2025);
[Datawrapper — popular chart types 2024](https://www.datawrapper.de/blog/popular-chart-types-2024)).
**Caveat:** Datawrapper's audience skews newsroom/journalism, which inflates tables and
locator maps and *under*-counts scatter/combo relative to a spreadsheet population — so
read it as directional support for "bar/line/pie dominate," not as spreadsheet ground
truth.

**Multi-series is the norm, not the exception.** The everyday reporting chart is a
*clustered* (or stacked) column with **2–4 series and a legend** — clustered columns are
called "the workhorse of everyday reporting," and Excel adds a legend by default precisely
because multi-series needs one ([DataCamp — clustered column charts in Excel](https://www.datacamp.com/tutorial/clustered-column-chart-in-excel)).
This is the crux: the *types* people use are in scope, but they routinely use the
*multi-series* form of them.

**Scatter is common and has no substitute.** It's the standard visual for correlation /
relationship between two numeric variables — a distinct analytical job none of the
in-scope types can do ([Atlassian — what is a scatter plot](https://www.atlassian.com/data/charts/what-is-a-scatter-plot)).

**Verdict:** the in-scope *type list* (column/bar/line/area/pie/donut) genuinely covers
**most charts people actually have** — comparison, trend, and part-to-whole. Two real
holes remain: **scatter** (high prevalence, no substitute) and **combo** (medium-high).
And covering those in-scope types *usefully* means handling their **multi-series** form —
which is the Level A vs Level B question.

---

## 4. The central scoping question — Level A vs Level B

The type list is nearly settled by §1–§3; the live decision is **how much of Level B we
buy**. Concretely: single-series charts are ~free (Level A wrappers exist), but the common
real chart (multi-series clustered column + legend + a readable numeric y-axis) is Level B.
Three tiers:

### Tier 1 — Level A only (the literal reading of the guardrail)
- **Covers:** single-series **column, bar, line, area, pie, donut**; category-axis labels;
  per-bar / per-slice color; data labels (bar + pie); hover tooltips (line/area/bar).
  Optionally the weak candlestick.
- **Effort:** **Lowest.** Thin adapters from our parsed chart structs onto the five
  existing gpui structs. No new rendering code.
- **Honest gap:** **No legend, no numeric value-axis, no chart title, no multi-series, no
  stacking** (`gpui-component-charts.md §3` — "no numeric value-axis labels on ANY chart
  type," "no legend anywhere"). The killer: a **multi-series clustered column with a
  legend — the single most common real business chart — cannot be rendered.** Tier 1 must
  either draw only the first series (misleading) or show a "can't display" placeholder for
  every multi-series file. For a spreadsheet, unreadable magnitudes (no y-axis numbers) and
  no legend make even the charts it *does* draw feel like previews, not the real thing.

### Tier 2 — Level A + targeted Level B (multi-series bar/line/area, legend, numeric axis)
- **Covers:** Tier 1, **plus** for column/bar/line/area: **true multi-series**,
  **grouped (clustered) and stacked / 100%-stacked**, a **legend**, and **numeric
  value-axis labels** — built on the `Stack` / `Line` / `Bar` / `Area` primitives + scales
  + the axis/grid helpers (the sanctioned path; the library's own stacked-bar demo does
  this — `gpui-component-charts.md §3`). Pie/donut stay Level A.
- **Effort:** **Moderate.** A bounded amount of custom `Plot` code: a legend widget, a
  numeric-axis tick generator (the linear scale has **no "nice" ticks** today —
  `gpui-component-charts.md §1`), a multi-series color cycle over `chart_1..chart_5`, and
  clustered/stacked layout via `Stack`. Real work, but bounded and reusing primitives.
- **Honest gap:** still **out** — scatter, bubble, radar, surface, 3D, combo, stock/volume,
  ofPie, multi-ring doughnut. And it is honestly **"more than just use gpui's charts"**:
  we're authoring custom widgets, which pushes slightly toward the "charting engine" line
  the guardrail warns about (though it stops well short — no new chart *types*, just the
  multi-series/legend/axis form of types gpui already draws).

### Out of both tiers (render a graceful "unsupported chart type" placeholder)
Scatter (XY), bubble, radar, surface, all 3D, combo/multi-plot, stock/volume variants,
ofPie, multi-ring doughnut, and the entire extended (`cx:`) family. For these, a clean
placeholder that still shows the chart title + anchor rectangle beats a wrong picture.

**Signal for the decision:** the jump from Tier 1 → Tier 2 is the jump from "renders the
*single-series subset* of the common types" to "renders the common types *as people
actually build them*." The jump from Tier 2 → supporting scatter is a genuine step-change
(net-new two-numeric-axis plotting) — that's where "charting engine" really begins, which
is why scatter sits **Out** despite its High prevalence.

---

## 5. Recommendation

**Target Tier 2, scoped to column/bar/line/area/pie/donut, and hold the line firmly there
(scatter and combo Out for MVP).** Reasoning:

1. **Tier 1 is the literal reading of the guardrail but is probably too weak to ship.**
   Most real spreadsheet charts are multi-series (clustered/stacked column, multi-line)
   with a legend, and a spreadsheet audience expects to **read values off the axis**. A
   chart renderer that silently drops all but the first series and shows no legend or
   numeric axis will read as broken more often than not. Shipping Tier 1 risks looking
   worse than "no charts."

2. **Tier 2 covers the charts people overwhelmingly have** (§3): comparison and trend in
   their real multi-series form, plus part-to-whole. Combined with cached-value extraction
   being tractable (`ironcalc-chart-exposure.md §4`), it delivers a genuinely useful,
   recognizable chart display for the large majority of files.

3. **It respects "don't build a charting engine" — barely, and deliberately.** The Level B
   work is bounded to *legend + numeric axis + multi-series/stacking* on gpui's own
   primitives — the exact thing the library demonstrates in its stacked-bar example. We add
   **no new chart types** beyond what gpui draws; we stop hard **before** scatter, radar,
   surface, 3D, and combo, which are where real charting-engine work (new axis systems, new
   geometry) starts.

**State the tension plainly for the discussion:** Tier 1 is the *literal* guardrail but may
be too weak to be worth shipping; Tier 2 delivers the common cases but is *more than "just
use gpui-component's chart structs."* The recommendation resolves that tension toward
usefulness, and pays for it with a bounded, primitives-based Level B investment plus an
explicit, loud concession on **scatter** — the one high-value type we're choosing not to
build.

**Two smaller calls to confirm in discussion:**
- **Numeric value-axis + legend are non-optional even in Tier 2's minimum.** They are the
  two Level B pieces that separate "a readable spreadsheet chart" from "a pretty preview";
  the gpui doc flags the missing numeric axis as "a major shortfall" (`gpui-component-charts.md §5`).
- **Candlestick/stock: defer.** Technically Level A, but finance-only, no color control, no
  interactivity, OHLC-only (no volume). Low prevalence; not worth the surface area for MVP.

---

### Sources

Capability facts: `specs/projects/chart/research/gpui-component-charts.md`,
`…/excel-chart-data-model.md`, `…/ironcalc-chart-exposure.md` (Wave-1).

Prevalence:
- [Atlassian — Essential Chart Types for Data Visualization](https://www.atlassian.com/data/charts/essential-chart-types-for-data-visualization)
- [Atlassian — What is a Scatter Plot?](https://www.atlassian.com/data/charts/what-is-a-scatter-plot)
- [Microsoft Support — Available chart types in Office](https://support.microsoft.com/en-us/office/available-chart-types-in-office-a6187218-807e-4103-9e0a-27cdb19afb90)
- [Datawrapper — Which chart types did our users create in 2025?](https://www.datawrapper.de/blog/popular-chart-types-2025)
- [Datawrapper — Which chart types did our users create in 2024?](https://www.datawrapper.de/blog/popular-chart-types-2024)
- [DataCamp — Clustered Column Charts in Excel](https://www.datacamp.com/tutorial/clustered-column-chart-in-excel)
- [HubSpot — Types of graphs for data visualization](https://blog.hubspot.com/marketing/types-of-graphs-for-data-visualization)
- [Alchemer — Pie Chart or Bar Graph?](https://www.alchemer.com/resources/blog/pie-chart-or-bar-graph/)
</content>
</invoke>
