# Chart Comparison — Stock / Candlestick

Research note for the FreeCell **charts** project (research phase). Per-family
Excel-vs-`gpui-component` comparison, scoped to "at most what `gpui-component` already
renders." This family is **borderline in-scope** — the doc's job is to decide whether
stock/candlestick is worth including at all.

**Inputs:** `research/excel-chart-data-model.md` (Wave-1 OOXML data model),
`research/gpui-component-charts.md` (Wave-1 gpui capability audit, pinned rev
`a9a7341c35b62f27ff512371c62419342264710c`), `research/ironcalc-chart-exposure.md`
(extraction feasibility). Structural OOXML facts below were corroborated across
web-search summaries of the Open XML SDK `StockChart` class, openpyxl, and XlsxWriter
stock-chart docs (the canonical pages — Microsoft Learn, openpyxl RTD — **403 to the
automated fetcher**, so exact cardinalities are flagged **[verify]** where inferred).

---

## 0. TL;DR

- Excel's `c:stockChart` is the **single most awkward classic chart type to consume**,
  because its on-disk shape is the *inverse* of what gpui wants. OOXML stores stock data
  as **one `c:ser` per metric** (Open is a whole series, High another, Low another, Close
  another; Volume a bar series) — parallel arrays. gpui's `CandlestickChart` wants **one
  item per period carrying all of O/H/L/C together**. Consuming a stock chart therefore
  requires a **series-transpose** (zip N parallel value-series into per-period OHLC
  tuples). This transpose is the crux of the whole family.
- gpui's `CandlestickChart` is the **weakest struct in the library** (Wave-1): OHLC-only,
  **hard-coded bull/bear colors** (no color control), **zero interactivity** (paint-only,
  no tooltip), no data labels, no legend, no numeric y-axis. There is **no combined
  price+volume view** (no dual-axis).
- Net: the pure **OHLC candlestick renders at Level A** (after the transpose) but loses
  color + interactivity; **HLC (no open) is a poor fit** (fake-open hacks); the **Volume
  variants' bar sub-plot is categorically OUT** (gpui has no dual-axis price+volume
  combo). Combined with the family's **rarity** (finance/trading niche), the
  recommendation is **DEFER / drop from MVP**, and if ever added, treat as a low-priority
  stretch limited to OHLC price-only.

---

## 1. Excel side — the mapping is awkward; dig in

### 1a. Four sub-styles

`c:stockChart` is a first-class chart-group element under `c:plotArea` (Wave-1
data-model §2a, line 77: *"No `barDir`/`grouping`; meaning comes from series order
(open/high/low/close) + `c:hiLowLines`, `c:upDownBars`."*). Excel's UI exposes **four**
sub-styles, distinguished by *which* metric-series are present and whether a volume bar
is added:

| Excel sub-style | Metrics (in order) | Price series | Volume series | Excel's native look |
|---|---|---|---|---|
| **HLC** — High-Low-Close | High, Low, Close | 3 | 0 | vertical hi-low line + right-side close tick; **no body** |
| **OHLC** — Open-High-Low-Close (candlestick) | Open, High, Low, Close | 4 | 0 | candle: `upDownBars` = body (white=up/black=down), `hiLowLines` = wick |
| **VHLC** — Volume-High-Low-Close | Volume, High, Low, Close | 3 | 1 (bar) | HLC chart + volume column sub-plot on a 2nd axis |
| **VOHLC** — Volume-Open-High-Low-Close | Volume, Open, High, Low, Close | 4 | 1 (bar) | candlestick + volume column sub-plot on a 2nd axis |

### 1b. The critical data-model mismatch (the transpose problem)

In OOXML there is **no "OHLC point" type**. Stock data is stored the same way any
multi-series line chart is: as **separate `c:ser` siblings, one per metric**, each a
full CT_LineSer-style series carrying its own `c:tx` (name), shared `c:cat`
(the dates/periods), and its own `c:val` range + `c:numCache`. So a 20-day OHLC chart is
**four parallel 20-element value arrays** (Open[0..20], High[0..20], Low[0..20],
Close[0..20]) that all share one category axis. Confirmed against the Open XML SDK
`StockChart` class (children: `LineChartSeries` ×N, `DropLines`, `HiLowLines`,
`UpDownBars`, `AxisId`) and openpyxl/XlsxWriter, which both build stock charts by adding
High/Low/Close (+Open, +Volume) as **separate series references**. **[verify: exact SDK
child cardinalities blocked by 403.]**

**Which series is which metric is positional, not tagged.** OOXML does not label a series
"this is Open." The meaning comes from **series order** within the `c:stockChart`
(`c:idx`/`c:order`), by the convention Open→High→Low→Close (or High→Low→Close for HLC).
A reader must therefore branch on the **price-series count**: 3 ⇒ (High, Low, Close);
4 ⇒ (Open, High, Low, Close). This positional decoding is fragile if a producer deviates
from the canonical order.

**gpui wants the transpose.** `CandlestickChart::new(Vec<T>)` with accessors
`.x(|&T|)`, `.open(|&T|)`, `.high`, `.low`, `.close` (all `-> f64`) expects **one `T`
per period** yielding all four values (gpui doc §2, lines 141-147). So consuming an Excel
stock chart requires building, for each period `i`:

```
T_i = { x: cat[i],
        open:  openSer.cache[i],   // absent for HLC/VHLC — see edge cases
        high:  highSer.cache[i],
        low:   lowSer.cache[i],
        close: closeSer.cache[i] }
```

i.e. **zip N parallel value-series (indexed by category) into per-period OHLC tuples.**
This is a genuine structural transform, not a field rename — the only such transform
across the whole chart-family comparison set. It runs after the normal cached-value read
(`ironcalc-chart-exposure.md` §4: read each `c:ser`'s `c:numCache` `c:pt` values; a
stock chart is just several of these that must then be interleaved).

### 1c. `c:upDownBars` / `c:hiLowLines` (and the volume combo)

- **`c:hiLowLines`** — a vertical line per period connecting the High and Low series
  (the candlestick **wick**/range). Styling lives in its `c:spPr`.
- **`c:upDownBars`** — a filled bar per period spanning Open↔Close (the candlestick
  **body**), with **separate `c:upBars`/`c:downBars` fills** (Excel default white/black,
  user-recolorable) chosen by up (close≥open) vs down. These two elements are the
  *semantic source* of "what a candle looks like," and they carry Excel's **color
  styling** for the body.
- **Volume (VHLC/VOHLC)** is **not** part of `c:stockChart`. It is a **sibling
  `c:barChart`** in the same `c:plotArea`, on a **secondary value axis** (volume
  magnitudes dwarf prices, so it needs its own scale) — i.e. the volume variants are a
  **combo chart** (`c:barChart` + `c:stockChart` sharing the category axis, two value
  axes). **[verify: secondary-axis detail inferred from Excel behavior + combo model in
  data-model §2a note "a single `c:plotArea` may contain multiple chart-group
  elements."]**

---

## 2. gpui side — `CandlestickChart` (the weakest struct)

From the Wave-1 gpui audit (`candlestick_chart.rs`, doc §2 lines 140-157, capability
matrix line 219):

```rust
pub struct CandlestickChart<T, X, Y>          // Y bound to f64 (sealed trait)
CandlestickChart::new(data)
  .x(|&T| -> X) .open(..).high(..).low(..).close(..)   // 4 OHLC accessors, all -> f64
  .body_width_ratio(f32)   // default 0.8
  .grid(bool) .x_axis(bool) .tick_margin(usize)
```

- **OHLC only.** Four accessors; one candle per item. No notion of "just HLC."
- **Coloring is hard-coded and un-settable.** Bullish `chart_bullish` / bearish
  `chart_bearish` from theme, chosen by `close > open` (candlestick_chart.rs:199-204).
  **No color setter, no per-candle override** — Excel's `upBars`/`downBars` fills and
  `hiLowLines` styling **cannot be honored**.
- **Zero interactivity.** No `id` field; the `Plot` impl is **paint-only** (no
  `id()`/`tooltip_state()`/`tooltip()`). Candlesticks are completely non-interactive — no
  hover tooltip showing the O/H/L/C numbers.
- **No data labels, no legend, no numeric y-axis labels** (the last is a library-wide gap
  — gpui gives 4 unlabeled gridlines on every chart). Only config knobs: body width
  ratio, grid on/off, x-axis on/off, tick density. Y-domain is raw OHLC min/max (does not
  force 0 — correct for prices).
- **No combined price+volume view.** There is no dual-axis / second-plot capability
  anywhere in the module (gpui doc §3: no secondary axis, no combo). So the volume bar
  sub-plot has **nowhere to go**.

**DIY-on-primitives note:** the `plot/` primitives (`Bar`, `Line`, scales, axis) could in
principle hand-roll a price+volume combo or a colored candle, but that is building a chart
engine, not reusing a struct — out of the "at most what gpui renders" envelope for this
borderline family. Not worth it given prevalence (§4).

---

## 3. Mapping table

Levels: **A struct** = renderable via `CandlestickChart` as-is; **approximate** =
renderable only with a lossy hack; **OUT** = no path within the gpui envelope.

| Excel construct | Level | Render note |
|---|---|---|
| **OHLC candlestick** (4 price series) | **A struct** | Transpose Open/High/Low/Close series → per-period OHLC `T`; feed `CandlestickChart`. gpui reconstructs body+wick geometrically. **Lost:** Excel `upBars`/`downBars` fill colors (gpui hard-codes bull/bear), all interactivity, data labels. |
| **HLC** (High-Low-Close, **no Open**) | **approximate → lean OUT** | gpui *requires* `.open()`. Fakes: `open = close` collapses every candle to a doji (body → a line) **and breaks coloring** (`close>open` never true ⇒ everything renders bearish); `open = prev-period close` is a hack that still draws a full body Excel never shows. Excel renders HLC as a hi-low line + close tick with **no body**, so candlestick is the wrong shape regardless. Approximate at best; recommend OUT. |
| **Volume-HLC (VHLC)** | **approximate (price only) / volume OUT** | Volume bar sub-plot **OUT** (no dual-axis). Remaining 3 price series are the HLC-no-open case above ⇒ same fake-open cost. So VHLC degrades to "approximate HLC, minus volume." |
| **Volume-OHLC (VOHLC)** | **A struct (price) / volume OUT** | Drop the sibling `c:barChart` volume series; transpose the 4 price series → OHLC candlestick (Level A). **Volume column sub-plot OUT** — gpui cannot render the price+volume dual-axis combo. |
| **`c:upDownBars` / `c:hiLowLines` styling** | **OUT** | gpui *derives* body+wick geometry from OHLC values but **ignores Excel's explicit styling** — up/down bar fill colors, hi-low line formatting, drop lines. Result is always gpui's hard-coded `chart_bullish`/`chart_bearish` look, not the file's. |

---

## 4. Prevalence

Stock/candlestick charts are **rare** in real-world spreadsheets — a **finance/trading
niche** (equity/FX/commodity OHLC price action, technical analysis). The vast majority of
business, scientific, and reporting workbooks never use one; the common families are
column/bar, line, pie, and area. Within the ~16 classic chart-group elements,
`c:stockChart` is among the **least-encountered** in the wild (comparable to
radar/surface). For a spreadsheet MVP whose charting bar is "at most what gpui renders,"
stock charts will affect a **small fraction** of files, and the users who *do* rely on
them (traders/analysts) are also the users most likely to notice the **lost color coding,
lost interactivity, and missing volume** — i.e. the population that cares is the
population we serve worst.

---

## 5. Verdict

**Is stock/candlestick worth including? Recommendation: DEFER (drop from MVP); if ever
added, a low-priority stretch limited to OHLC/VOHLC price-only.**

Rationale — the pure OHLC candlestick *is* renderable via `CandlestickChart`, but every
qualifier cuts against it:

1. **Awkward mapping (the crux).** Uniquely in this family, consuming the chart needs a
   **series-transpose**: OOXML stores one `c:ser` per metric (parallel arrays, positional
   role decoding), while gpui wants one item per period with all four values. This is real
   transform code, not a field rename, plus count-based role assignment (3 ⇒ HLC,
   4 ⇒ OHLC) that is fragile if a producer deviates from canonical order.
2. **Lossy even on the happy path.** gpui's candlestick is the library's weakest struct:
   **hard-coded bull/bear colors** (Excel's `upBars`/`downBars` and hi-low styling
   ignored) and **zero interactivity** (no hover to read O/H/L/C — painful for exactly the
   numeric-precision audience that uses these charts).
3. **Whole sub-styles are out.** **HLC (no open)** only renders via a fake-open hack that
   distorts the shape and breaks coloring; the **Volume variants' bar sub-plot is
   categorically OUT** (gpui has no dual-axis price+volume combo).
4. **Rarity.** Finance-only niche; smallest ROI of the classic families, and the users who
   need it are the ones hurt most by the gaps.

**Concrete scoping if it's ever done:** support **OHLC and VOHLC price-only** (drop
volume), cached-values, transpose-then-`CandlestickChart`, accept gpui's fixed colors and
no interactivity; treat **HLC/VHLC and all volume sub-plots as explicitly unsupported.**
This is a post-MVP stretch, not a launch feature.

---

### Sources
- Wave-1 docs (this repo): `research/excel-chart-data-model.md` (§2a line 77
  `c:stockChart`; §4 series/cache model), `research/gpui-component-charts.md`
  (§2 CandlestickChart lines 140-157; capability matrix line 219; §3 no-combo/no-legend),
  `research/ironcalc-chart-exposure.md` (§4 cached-value extraction path).
- Open XML SDK `StockChart` class (children `LineChartSeries`/`HiLowLines`/`UpDownBars`/
  `DropLines`/`AxisId`): `https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.charts.stockchart`
  **[403 to fetcher; corroborated via search summary]**.
- openpyxl Stock Charts (separate High/Low/Close/Open/Volume series):
  `https://openpyxl.readthedocs.io/en/stable/charts/stock.html` **[403; search summary]**.
- XlsxWriter stock chart example (multi-series, `add_series` per metric):
  `https://xlsxwriter.readthedocs.io/example_chart_stock.html`.
- ECMA-376 Part 1, DrawingML Charts (`CT_StockChart`, `CT_UpDownBars`, `CT_HiLowLines`):
  `https://ecma-international.org/publications-and-standards/standards/ecma-376/`.
