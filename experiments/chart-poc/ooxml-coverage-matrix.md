# OOXML Chart Coverage Matrix

**What this is.** A feature-by-feature audit of how much of the OOXML (classic `c:`) chart
data model the Chart PoC actually covers, how much is reachable by *extending* the
prototype (not restarting), and what is out of reach. It answers the question the go/no-go
`SYNTHESIS.md` did **not**: *"how faithful is our chart data-model mapping?"* — the PoC
proved the **render pipeline** on a thin structural spine; this matrix is the honest
fidelity ledger for the follow-on ship-quality project.

Grounded in: [`research/excel-chart-data-model.md`](../../specs/projects/chart-proof-of-concept/research/excel-chart-data-model.md)
(the OOXML feature catalog), the per-type `research/compare-*.md` docs, the implemented
`chart-model/src/lib.rs`, the `load-save/src/load.rs` parser, `chart-render/`, and
`SYNTHESIS.md §4` (sharp edges).

## Rubric

**Priority** (from a "faithfully display real-world business charts" lens):
- **P1** — core function; the chart is wrong/misleading without it.
- **P2** — important but not essential; a knowledgeable user notices it missing.
- **P3** — can live without; niche formatting, spacing, rare use cases.

**Support level:**
- **OK** — **validated in the prototype** (a scene was rendered and/or the parser reads it
  under test).
- **E-OK** — *not* validated, but **reasonable to assume implementable** by extending what
  we built, given what we learned (no architecture change).
- **HEAVY** — achievable, but **major effort** on top of the prototype (new subsystem, new
  geometry/scale, new writer).
- **NO** — can't be done, or so hard it's effectively out (needs 3D, a new schema family,
  or from-scratch geometry the reuse-gpui frame won't give us).

"Applies to" lists the chart type(s) a feature is shared across (many OOXML features are
shared plumbing — the schema is not per-type siloed).

---

## A. Data model & references (the series/data spine)

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| `c:chartSpace`→`c:chart`→`c:plotArea` navigation | All | P1 | **OK** | Parser walks it; namespace/prefix-agnostic (local-name match). |
| `c:ser` — multiple series (datasets) | All | P1 | **OK** | Multi-series line/grouped-bar/area rendered; parsed as repeated `<c:ser>`. |
| `c:tx` series name (from `strCache`) | All | P1 | **OK** | Read + drives the legend. |
| `c:cat` text categories (`strCache`) | bar·line·area·pie | P1 | **OK** | Rendered on the category axis. |
| `c:cat` numeric categories (`numCache`) | bar·line·area·pie | P2 | **OK** | `Category::Number` path exists + tested. |
| `c:val` values (`numCache`) | bar·line·area·pie | P1 | **OK** | The plotted numbers. |
| `c:xVal`/`c:yVal` | scatter·bubble | P1 | **OK** | Scatter parsed + rendered on two numeric axes. |
| `c:bubbleSize` (third value) | bubble | P2 | **E-OK** | Bubble judged IN *by analysis* only (`bubble-analysis.md`) — never rendered; size→radius is a small add. |
| Cached values `numCache`/`strCache` (`c:pt idx`) | All | P1 | **OK** | idx-sorted read; the whole "render without eval" premise. |
| `c:f` range formula → **live** re-render from cells | All | P2 | **E-OK** | We read cache only. Storing the `c:f` string is trivial; resolving it against IronCalc to stay live on edit is a bounded but real effort (currently unused). |
| `c:numLit`/`c:strLit` (inline literals, no range) | All | P3 | **E-OK** | Same cache-reading shape; not exercised. |
| `c:multiLvlStrRef` (hierarchical categories) | bar·line·area | P3 | **E-OK** | Read leaf level easily; a *true* multi-level grouped category axis is more (→ HEAVY for full fidelity). |
| Sparse points / blanks (`idx` gaps, `c:dispBlanksAs`) | All | P2 | **E-OK** | `cache_points` sorts by idx; gap→blank/zero/gap policy not implemented. |

## B. Chart-group types (`c:<type>Chart`)

| OOXML element | Common name | Priority | Support | Notes |
|---|---|---|---|---|
| `c:barChart` `barDir=col` | Column | P1 | **OK** | single + clustered + stacked + 100%-stacked all rendered. |
| `c:barChart` `barDir=bar` | Bar (horizontal) | P1 | **OK** | `bar_horizontal` scene. (Category order is data-order, not Excel's bottom-up — `SYNTHESIS §4.3`.) |
| `c:lineChart` | Line | P1 | **OK** | single + multi-series on a shared scale. |
| `c:areaChart` | Area | P1 | **OK** | stacked + 100%-stacked (hand-rolled polygon fork). |
| `c:pieChart` | Pie | P1 | **OK** | `pie` scene. |
| `c:doughnutChart` | Doughnut | P1 | **OK** | `holeSize` read + rendered. |
| `c:scatterChart` | XY Scatter | P1 | **OK** | single + multi-series, two numeric axes. |
| `c:bubbleChart` | Bubble | P2 | **E-OK** | Analysis-only (see A). |
| multiple groups in one `c:plotArea` | **Combo** | P2 | **HEAVY** | Parser reads only the **first** group (`.find`) — silently drops the rest today. Real combo = two group renderers in one plot + usually a secondary axis. |
| `c:stockChart` | Stock / OHLC | P3 | **HEAVY** | Awkward: OHLC stored as *separate* series → must transpose; no volume combo. Never built. |
| `c:radarChart` | Radar | P3 | **HEAVY** | Needs radial (polar) category geometry — none in the primitives; hand-drawn from scratch. |
| `c:ofPieChart` | Pie-of-Pie / Bar-of-Pie | P3 | **HEAVY** | Secondary linked plot + split logic; no concept in proto. |
| `c:bar3DChart` / `line3D` / `pie3D` / `area3D` | 3-D variants | P3 | **NO** | Zero 3D in gpui at the pinned rev. Best case = flatten to the 2-D equivalent (which is really rendering the 2-D type). |
| `c:surfaceChart` / `surface3DChart` | Surface | P3 | **NO** | 3D / contour geometry; no support. |
| `cx:` extended family | Sunburst, treemap, waterfall, histogram, Pareto, box-&-whisker, funnel, region map | P3 | **NO** | Different schema (`2014/chartex`), none in gpui, rare. Out of scope entirely. |

## C. Series & data-point styling

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| `c:spPr`→`a:solidFill`→`a:srgbClr` (per-series color) | All | P1 | **OK** | Read + applied; else palette cycle. |
| `a:schemeClr` theme color (+ tint/shade) | All | P2 | **E-OK** | FreeCell already parses the theme palette (`open_fixups.rs`); resolving `schemeClr`+lumMod/lumOff is bounded. |
| `a:gradFill` gradient fill | bar·area | P3 | **E-OK** | gpui `Bar`/`Area` accept gradients; parsing `gradFill` is moderate. |
| `a:pattFill` pattern fill | bar·area·pie | P3 | **HEAVY** | No pattern rendering; custom tiling. |
| `a:ln` stroke (width/dash/color) | line·borders | P2 | **E-OK** | Width/color easy; dash patterns need custom stroke work. |
| **`c:dPt` per-point color override** | pie·doughnut·bar | **P1** (pie) / P2 (bar) | **E-OK** | **Not read today** — pie slice colors come from our synthesized palette, not the file. The renderer already takes a per-slice color closure, so wiring `dPt`→color is small; it's the "coloring crux" the pie research flagged, so P1 for pie. |
| `c:varyColors` | pie·doughnut·bar | P2 | **E-OK** | We already vary a palette; honoring the flag is trivial. |
| `c:marker` (shape/size/color) | line·scatter | P2 | **E-OK** | Round dot markers exist; non-circle shapes (square/diamond/triangle) need custom marks. |
| `a:alpha` transparency | All | P3 | **E-OK** | `Hsla` carries alpha. |

## D. Chart chrome (title / legend / axes)

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| `c:title` text | All | P1 | **OK** | Rendered above the plot. |
| `c:autoTitleDeleted` | All | P2 | **E-OK** | We read a present title; honoring the "deleted" flag is trivial. |
| Title rich formatting (font/size/color runs, `c:strRef` to a cell) | All | P3 | **E-OK** | We extract concatenated text only; styled runs / cell-linked titles dropped. |
| `c:legend` presence | All | P1 | **OK** | Legend widget built + rendered (swatch↔series). |
| `c:legendPos` (`t`/`b`/`l`/`r`/`tr`) | All | P2 | **E-OK** | Presence honored; arbitrary placement is layout work (proto places it consistently). |
| `c:legendEntry` deletions | All | P3 | **E-OK** | Skip named entries; modest. |
| `c:catAx` / `c:valAx` **title** | bar·line·area·scatter | P1 | **OK** | Rendered. (Value-axis title is a *horizontal* caption, not rotated — gpui text-rotation limit; cosmetic, `SYNTHESIS §4.2`.) |
| Numeric value axis with readable ("nice") ticks | All | P1 | **OK** | Our `NiceScale` (Heckbert) — `ScaleLinear` ships none. |
| `c:numFmt` axis/label number format (currency/%/date/thousands) | All | P2 | **E-OK** | We own tick text; adding format application is moderate. Not honored today. |
| `c:scaling` explicit min/max | All | P2 | **E-OK** | We auto-fit a domain; honoring a manual min/max is an easy override. |
| `c:orientation` reversed axis (`maxMin`) | All | P2 | **E-OK** | Flip the scale mapping; modest. |
| `c:majorGridlines` / `c:minorGridlines` toggles | All | P2 | **E-OK** | We draw fixed gridlines; making them configurable + honoring flags is modest. |
| Log axis (`c:logBase`) | scatter·line·bar | P3 | **HEAVY** | Needs a log scale + log-nice ticks; new scale code beyond `ScaleLinear`. |
| `c:dateAx` date/time axis | line·area·bar | P2 | **HEAVY** | A whole axis type (date scaling + date tick formatting). |
| `c:catAx` tick interval / label skip | bar·line·area | P3 | **E-OK** | `tick_margin` exists; interval/skip is small. Label **rotation** is HEAVY (text-rotation limit). |
| `c:delete` (hidden axis) | All | P3 | **E-OK** | Don't draw it. |
| `c:serAx` series axis | 3D | P3 | **NO** | 3D only. |

## E. Type-specific layout knobs

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| `c:barDir` | bar | P1 | **OK** | col/bar. |
| `c:grouping` (standard/clustered/stacked/percentStacked) | bar·line·area | P1 | **OK** | All four rendered for bar/area; stacked line is E-OK (rare, not built). |
| `c:gapWidth` / `c:overlap` (bar spacing) | bar | P3 | **E-OK** | Bar slot math is ours (padding hard-coded today); adding gap/overlap params is easy. |
| `c:holeSize` (doughnut) | doughnut | P1 | **OK** | Read + rendered. |
| `c:firstSliceAng` (pie rotation) | pie·doughnut | P3 | **E-OK** | `Pie::start_angle` exists (found in `bubble-analysis`). |
| `c:explosion` (exploded slices) | pie·doughnut | P3 | **E-OK** | Per-slice radius/offset; `Arc` bounds shift. |
| `c:scatterStyle` (line/marker/smooth) | scatter | P2 | **E-OK** | We draw markers; adding the connecting/smoothed line reuses the line renderer. |
| `c:smooth` (curved line) | line | P2 | **E-OK** | Natural-cubic curve exists — visual stand-in, not math-identical to Excel. |
| `c:bubbleScale` / `c:sizeRepresents` (area vs width) | bubble | P3 | **E-OK** | A size→radius formula choice. |
| `c:radarStyle` | radar | P3 | **HEAVY** | Radar itself is HEAVY (B). |
| `c:ofPieType`/`c:splitType`/`c:splitPos` | ofPie | P3 | **HEAVY** | ofPie itself is HEAVY (B). |
| `c:view3D` / `c:shape` | 3D | P3 | **NO** | No 3D. |

## F. Data labels (`c:dLbls`)

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| `c:dLbls` `c:showVal` (value labels on marks) | All | P2 | **E-OK** | **Not modeled today.** gpui `Bar` has `.label()`, `Pie` draws leader-line labels — the primitives support it; wiring `dLbls`→labels is moderate. |
| `c:showCatName` / `c:showSerName` / `c:showLegendKey` | All | P3 | **E-OK** | Compose the label string. |
| `c:showPercent` (pie %) | pie·doughnut | P2 | **E-OK** | Needs a total→percent pass; modest. |
| `c:dLbls` `c:numFmt` | All | P3 | **E-OK** | Same number-format work as axes. |
| Data-label position (inside/outside/center/bestFit) | All | P3 | **E-OK** | Some positions free from the primitives; bestFit is more. |

## G. Anchor, layout & packaging

| OOXML feature | Applies to | Priority | Support | Notes |
|---|---|---|---|---|
| Relationship chain sheet→drawing→chart | All | P1 | **OK** | Full walk, multi-sheet, multi-chart, in document order. |
| **Load** chart XML → model | All | P1 | **OK** | Validated Gate 4 on real-shaped fixtures (3 charts). |
| Drawing **anchor** position/size (`xdr:twoCellAnchor` from/to) | All | P2 | **E-OK** | We follow the anchor part but ignore its geometry (proto renders standalone); placing a chart at its cell anchor is a bounded parse for app integration. |
| Manual `c:layout` (explicit plot-area rect) | All | P3 | **E-OK** | Usually auto; ignore→auto is acceptable, honoring it is small. |
| `styleN.xml` / `colorsN.xml` (theme style/color parts) | All | P3 | **E-OK** (ignore) / **HEAVY** (full fidelity) | Ignorable for a correct-but-plain chart; full Excel-2013 style fidelity is a large separate effort. |
| Chartsheet (whole-tab chart, `xl/chartsheets/`) | All | P3 | **E-OK** | Same chain, different part path. |
| **Save**: byte-preservation re-injection | All | P1 | **OK** | Validated Gate 4 (patch `<drawing>`, worksheet `_rels`, content-types). |
| Save: multi-sheet part mapping (`workbook.xml.rels`) | All | P2 | **E-OK** | Proto is single-sheet 1:1; mapping is modest (`SYNTHESIS §4.8`). |
| Save: **write** chart XML from our model (synthesis) | All | P2 | **HEAVY** | The stretch goal never built — emit valid `chartN.xml` from the model. |
| Save: refresh stale `numCache` after a data edit | All | P2 | **HEAVY** | Byte-preservation keeps the old cache; reflow from IronCalc's evaluated cells is a separate effort (`SYNTHESIS §4.9`). |

---

## Summary

### How close does the *prototype* get to the OOXML spec?
**The core spine of the six common 2-D classic types — and not much beyond it.** In
feature-row terms, the **OK** column is ~25 rows: the relationship chain, series/cat/val/xy
data from cache, one solid series color, chart title + axis titles, a legend, a nice
numeric axis, `barDir`, all four `grouping`s, and the doughnut hole — enough to render a
**recognizable, quantitatively-correct** single- **and** multi-series column/bar/line/area/
pie/doughnut/scatter. That is the **P1 core**, and it is genuinely done.

But measured against the *whole* `c:` feature surface it is a **minority** — call it the
**~20–25% that carries ~80% of everyday charts**. Everything that makes a real Excel chart
look like *that specific file* — per-slice/`dPt` colors, data labels, number formats, axis
scaling, theme/gradient fills, markers, gap/overlap, rotation/explosion — is **absent** from
the model today. And even the mapping that exists was exercised on only **3 agent-authored
fixtures** (no real Excel/LibreOffice corpus — `SYNTHESIS §4.10/§4.11`), so it's a
*structural* proof, not a fidelity proof.

### How close can we get by *extending* (not restarting)?
**Most of P1 + P2 — the large majority of real-world 2-D classic charts — is E-OK.** The
architecture holds: the OOXML-shaped model "held across all four gates without a shape
change" (`SYNTHESIS §5`), and we *own* the renderer, so the high-value gaps are additive:
`dPt`/theme/gradient colors, `dLbls` data labels, `numFmt` number formats, axis min/max +
reversed + gridline toggles, markers, `smooth`, `scatterStyle`, `gapWidth`/`overlap`, pie
rotation/explosion, bubble, and live `c:f` rendering are all **E-OK** — bounded work on the
existing primitives + parser. A follow-on that budgets for these reaches **faithful display
of the vast majority of `.xlsx` charts people actually have.** The one common-ish item that
is **HEAVY** rather than E-OK is **combo** (multi-group + secondary axis), plus **date axis**
and **log axis**.

### What are we *not* getting?
A clear tail stays **HEAVY/NO**, and it clusters:
- **NO (needs 3D, new schema, or from-scratch geometry):** all **3-D** variants, **surface**,
  **radar** (polar geometry), and the entire **`cx:` extended family** (sunburst, treemap,
  waterfall, histogram, Pareto, box-&-whisker, funnel, region map).
- **HEAVY (possible, real subsystem each):** **combo**, **stock/OHLC**, **ofPie**, **date
  axis**, **log axis**, **pattern fills**, rotated vertical axis titles (gpui text-rotation
  limit — cosmetic), full `styleN`/`colorsN` theme fidelity, and the two save-side hard
  problems (**writing** chart XML from the model, and **refreshing stale caches** on edit).

**Net:** the prototype proved the *floor* (the P1 core renders and round-trips); extending it
reaches a strong, useful *ceiling* (most P1+P2, i.e. the everyday chart in its real
multi-series/styled form); and a well-defined tail (3-D, surface, radar, chartex, combo,
save-write) is either out or a deliberate, separately-budgeted lift for the ship project.
