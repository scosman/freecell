# Excel / OOXML Chart Data Model

Research note for the FreeCell **charts** project (research phase). Goal: catalog the
chart types Excel/OOXML can store, and document the on-disk **data model** we would have
to *read* to render a chart from a `.xlsx`. Scope reminder: we render "at most what
`gpui-component` already does" — this doc is about the *source* format, not our renderer.

> Sourcing note: most schema-reference sites (datypic, c-rex, liquid-technologies,
> openpyxl/XlsxWriter readthedocs, Microsoft Learn) block automated fetches (HTTP 403) in
> this environment, so facts below were corroborated across web-search summaries of those
> same pages **plus a real-world `chart1.xml`** fetched from GitHub. URLs are given for
> human verification. Claims I could not fully pin down are flagged **[verify]**.

---

## 0. TL;DR

- Excel charts come in **two schema families**:
  1. **Classic** — namespace `http://schemas.openxmlformats.org/drawingml/2006/chart`
     (`c:` prefix), standardized in **ECMA-376 Part 1 (DrawingML — Charts)**. ~16
     chart-group elements (commonly summarized as "~17 chart types" at the Excel-UI
     level). This is what almost every `.xlsx` in the wild uses.
  2. **Extended / "chartex"** — namespace
     `http://schemas.microsoft.com/office/drawing/2014/chartex` (`cx:` prefix), a
     **Microsoft extension** (NOT in ECMA-376; documented in **[MS-ODRAWXML]**). Adds the
     newer Excel 2016+ statistical/hierarchical types: sunburst, treemap, waterfall,
     histogram/Pareto, box-&-whisker, funnel, region map.
- Chart data physically lives in `xl/charts/chartN.xml` (classic) or
  `xl/charts/chartExN.xml` (extended), reached from a worksheet via a
  `<drawing r:id>` → `drawingN.xml` (anchor) → chart relationship chain.
- Read model skeleton (classic):
  `c:chartSpace` → `c:chart` → `c:plotArea` → `c:<type>Chart` → one-or-more `c:ser`,
  each `c:ser` carrying `c:tx` (series name), `c:cat` (categories/X), `c:val`
  (values/Y). Each of those wraps a `c:f` range formula (e.g. `Sheet1!$B$2:$B$10`) plus a
  cached `c:numCache`/`c:strCache` copy of the values.

---

## 1. Two chart families & their namespaces

| Family | Prefix | Namespace URI | Spec home | Part naming | Notes |
|---|---|---|---|---|---|
| Classic (DrawingML Charts) | `c:` | `http://schemas.openxmlformats.org/drawingml/2006/chart` | ECMA-376 Part 1, §21.2 (DrawingML — Charts) | `xl/charts/chart1.xml` | The universal, ISO/ECMA-standardized chart format. |
| Extended ("chartex") | `cx:` | `http://schemas.microsoft.com/office/drawing/2014/chartex` | [MS-ODRAWXML] (Microsoft extension, not ECMA-376) | `xl/charts/chartEx1.xml` **[verify: part name]** | Excel 2016+ statistical/hierarchical types. Backward-compat: Excel usually also writes a classic fallback chart in the drawing so older apps show *something*. **[verify: fallback behavior]** |

Supporting prefixes seen inside chart XML: `a:` (DrawingML main, `.../2006/main` — shape
properties/fills/text), `r:` (relationships, `.../2006/relationships` — the `r:id`
references), `xdr:` (SpreadsheetDrawingML, `.../2006/spreadsheetDrawing` — the anchor).

---

## 2. Chart type catalog

### 2a. Classic (`c:`) chart-group elements — the "basic" set

These are the elements allowed as children of `c:plotArea` (from `CT_PlotArea`). At the
XML level there are **16 distinct chart-group elements**; the "~17 chart types" figure
usually cited is the Excel-UI family count (which folds 3D variants together and counts
Column vs. Bar separately, etc.), so treat "17" as an informal headline, not an exact
schema count. **[flag: the exact "17" is soft — schema has 16 group elements.]**

| OOXML element | Common name | 2D / 3D | Set | Notes |
|---|---|---|---|---|
| `c:barChart` | Column **and** Bar | 2D | basic | Orientation chosen by child `c:barDir` (`col` = vertical Column, `bar` = horizontal Bar). One element covers both Excel "Column" and "Bar" families. |
| `c:bar3DChart` | 3-D Column / 3-D Bar | 3D | basic | 3D variant; also uses `c:barDir`. Adds `c:shape` (box/cylinder/cone/pyramid) + series-axis. |
| `c:lineChart` | Line | 2D | basic | `c:grouping` + optional `c:marker`, `c:smooth`. |
| `c:line3DChart` | 3-D Line | 3D | basic | Adds a series axis (`c:serAx`). |
| `c:pieChart` | Pie | 2D | basic | Single series typically; `c:varyColors`, `c:firstSliceAng`. |
| `c:pie3DChart` | 3-D Pie | 3D | basic | 3D pie. |
| `c:doughnutChart` | Doughnut | 2D | basic | Multi-ring pie; `c:holeSize`, `c:firstSliceAng`. |
| `c:ofPieChart` | Pie of Pie / Bar of Pie | 2D | basic | `c:ofPieType` = `pie` (Pie-of-Pie) or `bar` (Bar-of-Pie); has a secondary plot + `c:splitType`. |
| `c:areaChart` | Area | 2D | basic | `c:grouping`. |
| `c:area3DChart` | 3-D Area | 3D | basic | 3D area. |
| `c:scatterChart` | XY (Scatter) | 2D | basic | Uses `c:xVal`/`c:yVal` (NOT `c:cat`/`c:val`); `c:scatterStyle` (line/marker/smooth). Two value axes. |
| `c:bubbleChart` | Bubble | 2D | basic | `c:xVal`/`c:yVal`/`c:bubbleSize`; `c:bubbleScale`, `c:sizeRepresents`. |
| `c:radarChart` | Radar | 2D | basic | `c:radarStyle` = `standard` / `marker` / `filled`. |
| `c:stockChart` | Stock (Hi-Lo-Close, OHLC…) | 2D | basic | No `barDir`/`grouping`; meaning comes from series *order* (open/high/low/close) + `c:hiLowLines`, `c:upDownBars`. |
| `c:surfaceChart` | Surface (contour / wireframe, top view) | 2D-ish | basic | 2D "contour" projection of a 3D surface; `c:wireframe`. |
| `c:surface3DChart` | 3-D Surface | 3D | basic | Full 3D surface. |

Notes / gotchas:
- **`c:barChart` is doing double duty** for both Excel's "Column" and "Bar" UI families —
  the only difference is `c:barDir`. Easy to miss.
- **Scatter & bubble do not use `c:cat`/`c:val`.** They use `c:xVal`/`c:yVal`
  (+ `c:bubbleSize`). Any reader must branch on chart type here.
- A single `c:plotArea` may contain **multiple different chart-group elements**
  (e.g. `c:barChart` + `c:lineChart`) — that is how Excel **combo charts** are stored.
  There is no dedicated "combo" element.
- 3D types (`*3DChart`) additionally reference a **series axis** (`c:serAx`) and a
  `c:view3D` on the chart, plus per-series `c:shape`.

### 2b. Extended (`cx:`) chart types — the newer set

Extended charts do **not** have one element per type. Instead a single generic structure
is used and the concrete type is selected by a **`layoutId` attribute on `cx:series`**
inside `cx:plotAreaRegion`. (This is a notable structural departure from the classic
family — see §4.)

| `cx:series` `layoutId` value | Common name | Set | Notes |
|---|---|---|---|
| `clusteredColumn` / `waterfall` | Waterfall | extended | Running-total bridge chart. |
| `sunburst` | Sunburst | extended | Hierarchical concentric rings. |
| `treemap` | Treemap | extended | Nested proportional rectangles (hierarchy). |
| `boxWhisker` | Box & Whisker | extended | Quartiles/whiskers/outliers (statistical). |
| `clusteredColumn` (binned) → `histogram` | Histogram | extended | Auto-binned distribution. |
| `paretoLine` | Pareto | extended | Histogram + cumulative % line (a histogram variant). |
| `funnel` | Funnel | extended | Stage/funnel bars. |
| `regionMap` | Filled Map / Region Map | extended | Geographic choropleth (needs geo data service). |

**[flag]** The exact `layoutId` string set above is assembled from web-search summaries of
[MS-ODRAWXML] and LibreOffice/python-pptx sources; the canonical enumeration should be
confirmed against [MS-ODRAWXML] before we rely on specific literals. The *set* of types
(sunburst, treemap, waterfall, histogram, Pareto, box&whisker, funnel, region map) is
well corroborated.

**Relevance to us:** almost none of these are in scope — `gpui-component` renders none of
the extended family, and they are rare in real files. They are cataloged here so
per-type comparison docs can explicitly declare them out of scope.

---

## 3. Where the authoritative specs & the data physically live

### 3a. Authoritative data-model specs (citable)

- **ECMA-376** (Office Open XML File Formats), Part 1, DrawingML → **Charts** (the
  `c:` schema; `dml-chart.xsd` / `CT_Chart`, `CT_PlotArea`, `CT_BarChart`, `CT_LineChart`,
  `CT_Ser`, …). Standard landing page:
  <https://ecma-international.org/publications-and-standards/standards/ecma-376/>
  (also published as **ISO/IEC 29500**).
- **Classic chart namespace:** `http://schemas.openxmlformats.org/drawingml/2006/chart`
- **Extended "chartex" schema / namespace:** `http://schemas.microsoft.com/office/drawing/2014/chartex`,
  documented in **[MS-ODRAWXML]**:
  <https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/e2723b0a-9120-42a5-bd11-c252ccb13c1e>
- **Schema browsers (per-element, human-readable):**
  - datypic OOXML reference (elements `c:chart`, `c:barChart`, `c:ser`, `c:dLbls`, …):
    <http://www.datypic.com/sc/ooxml/> (e.g. `e-draw-chart_ser-1.html`, `e-draw-chart_dLbls-1.html`)
  - c-rex.net ECMA-376 mirror: <https://c-rex.net/samples/ooxml/>
  - liquid-technologies OOXML schema docs: <https://schemas.liquid-technologies.com/OfficeOpenXML/2006/dml-chart_xsd.html>
- **Practical model references (good for shape of the data):**
  - openpyxl charts (Python read/write model): <https://openpyxl.readthedocs.io/en/stable/charts/introduction.html>
    and series module <https://openpyxl.readthedocs.io/en/stable/api/openpyxl.chart.series.html>
  - XlsxWriter "Working with Charts": <https://xlsxwriter.readthedocs.io/working_with_charts.html>
  - Microsoft Open XML SDK — SpreadsheetML structure:
    <https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/structure-of-a-spreadsheetml-document>
- **Real-world chart XML** (verified by fetch — a `c:pieChart` with `c:ser`/`c:cat`/`c:val`/`c:strCache`/`c:numCache`/`c:dPt`/`c:dLbls`):
  <https://github.com/Vitaliy-1/DOCX2JATS/blob/master/stylesheets/charts/chart1.xml>

### 3b. Physical location inside the `.xlsx` (OPC zip) package

An `.xlsx` is a ZIP (Open Packaging Conventions). A chart on a sheet is reached through a
relationship chain — you cannot find charts by scanning the worksheet XML alone; you must
follow `r:id`s through `.rels` parts:

```
xl/worksheets/sheet1.xml
    └─ <drawing r:id="rId1"/>                      (worksheet points to ONE drawing part)
xl/worksheets/_rels/sheet1.xml.rels
    └─ rId1  →  ../drawings/drawing1.xml            (.../relationships/drawing)
xl/drawings/drawing1.xml                            (the ANCHOR / positioning)
    └─ <xdr:twoCellAnchor> (or oneCellAnchor)
         └─ <xdr:graphicFrame>
              └─ <a:graphic><a:graphicData uri=".../chart">
                   └─ <c:chart r:id="rId1"/>        (frame points to the chart part)
xl/drawings/_rels/drawing1.xml.rels
    └─ rId1  →  ../charts/chart1.xml                (.../relationships/chart)
xl/charts/chart1.xml                                (THE CHART — chartSpace root)
xl/charts/_rels/chart1.xml.rels
    ├─ →  ../charts/style1.xml                      (style; c14/cs styling)
    ├─ →  ../charts/colors1.xml                     (color mapping / color style)
    └─ (optional) →  ../embeddings/…xlsx            (externalData: a cached mini-workbook,
                                                     mainly when the chart is embedded in a
                                                     Word/PPT doc rather than a live sheet)
```

- **`xl/charts/chartN.xml`** — the chart definition itself (`c:chartSpace`). This is the
  file you parse for data + config.
- **`xl/charts/styleN.xml`** and **`xl/charts/colorsN.xml`** — Excel 2013+ theme-aware
  style/color parts (`cs:` and `.../2011/chartStyle` / color-style namespaces). Optional;
  visual polish, not data. Mostly ignorable for a first-cut renderer.
- **`xl/drawings/drawingN.xml`** — the **anchor**: where on the sheet the chart floats and
  how big it is (`twoCellAnchor` from/to cells, or `oneCellAnchor`/`absoluteAnchor`).
- **`[Content_Types].xml`** declares the chart part content type
  (`application/vnd.openxmlformats-officedocument.drawingml.chart+xml`) — a fast way to
  enumerate charts in a package.
- **Chart sheets** (a whole tab that is just a chart) live under `xl/chartsheets/` and
  reference their drawing/chart the same way. **[minor: verify path for our purposes]**

Package-structure references:
<http://officeopenxml.com/anatomyofOOXML-xlsx.php>,
<http://officeopenxml.com/drwPicInSpread.php>,
<https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/structure-of-a-spreadsheetml-document>

---

## 4. Core XML structure for READING chart data (classic `c:`)

### 4a. Skeleton

```xml
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
              xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    <c:title>…</c:title>                 <!-- optional; see §5 -->
    <c:autoTitleDeleted val="0"/>
    <c:plotArea>
      <c:layout/>                        <!-- manual vs auto plot-area position -->
      <c:barChart>                       <!-- the chart-group element (type lives HERE) -->
        <c:barDir val="col"/>            <!-- per-family config (see §5) -->
        <c:grouping val="clustered"/>
        <c:varyColors val="0"/>
        <c:ser> … </c:ser>               <!-- one <c:ser> PER DATASET/SERIES (repeatable) -->
        <c:ser> … </c:ser>
        <c:dLbls/>                        <!-- chart-level data labels (optional) -->
        <c:axId val="111111111"/>        <!-- links series to axes below -->
        <c:axId val="222222222"/>
      </c:barChart>
      <c:catAx> … </c:catAx>             <!-- category axis -->
      <c:valAx> … </c:valAx>             <!-- value axis -->
    </c:plotArea>
    <c:legend> … </c:legend>            <!-- optional -->
    <c:plotVisOnly val="1"/>
  </c:chart>
</c:chartSpace>
```

### 4b. A single series (`c:ser`) — the heart of the data model

```xml
<c:ser>
  <c:idx val="0"/>                        <!-- series index -->
  <c:order val="0"/>                      <!-- draw/legend order -->

  <c:tx>                                  <!-- SERIES NAME (the dataset label) -->
    <c:strRef>
      <c:f>Sheet1!$B$1</c:f>              <!-- formula: where the name comes from -->
      <c:strCache><c:ptCount val="1"/>
        <c:pt idx="0"><c:v>Revenue</c:v></c:pt>   <!-- cached literal -->
      </c:strCache>
    </c:strRef>
  </c:tx>

  <c:spPr>…</c:spPr>                       <!-- optional per-series fill/line (color) -->

  <c:cat>                                  <!-- CATEGORIES / X labels (shared across series) -->
    <c:strRef>                             <!-- strRef for text cats; numRef/multiLvlStrRef also legal -->
      <c:f>Sheet1!$A$2:$A$10</c:f>
      <c:strCache><c:ptCount val="9"/>
        <c:pt idx="0"><c:v>Jan</c:v></c:pt> … </c:strCache>
    </c:strRef>
  </c:cat>

  <c:val>                                  <!-- VALUES / Y (the numbers plotted) -->
    <c:numRef>
      <c:f>Sheet1!$B$2:$B$10</c:f>         <!-- the data range -->
      <c:numCache>
        <c:formatCode>General</c:formatCode>
        <c:ptCount val="9"/>
        <c:pt idx="0"><c:v>43</c:v></c:pt> … <!-- cached numeric values -->
      </c:numCache>
    </c:numRef>
  </c:val>
</c:ser>
```

### 4c. How a data reference works (the key concept)

Every data slot (`c:tx`, `c:cat`, `c:val`, `c:xVal`, `c:yVal`, `c:bubbleSize`) wraps a
**reference object** that pairs a *formula* with a *cache*:

- **`c:f`** — a range/formula string in A1 notation, sheet-qualified and absolute:
  e.g. `Sheet1!$B$2:$B$10`. This is the *live* link to worksheet cells.
- **`c:numCache`** (numeric) or **`c:strCache`** (string) — a **snapshot** of the values
  Excel last computed for that range: `c:ptCount` + a list of `c:pt idx=".."`/`c:v`
  (plus `c:formatCode` for numeric). Sparse points are allowed (idx gaps for blanks).
- Reference wrapper types: `c:numRef` (numbers-from-range), `c:strRef`
  (strings-from-range), `c:multiLvlStrRef` (hierarchical categories), and literal
  variants `c:numLit`/`c:strLit` (values embedded directly, no range).

**Practical consequence for FreeCell:** we can render a chart **purely from the cache**
(`numCache`/`strCache`) without evaluating any formula — the values are already there.
To stay *live* (re-render when the sheet changes) we would instead parse `c:f`, resolve
the range against IronCalc, and read the current cell values. The cache is the fast,
zero-dependency path; the `c:f` range is the live path.

### 4d. Multiple series (multiple datasets)

A multi-dataset chart is simply **multiple `<c:ser>` siblings** inside the chart-group
element. Each `c:ser` has its own `c:tx` (name), its own `c:val` range, and typically
**shares the same `c:cat`** categories (each series repeats the same `c:cat` range).
`c:idx`/`c:order` disambiguate/order them. This maps cleanly onto a "multi-series" chart
in any renderer (`gpui-component` included). Per-series color comes from each series'
`c:spPr` (§5).

### 4e. Scatter / bubble (different slots)

Scatter and bubble series replace `c:cat`/`c:val` with paired numeric dimensions:

```xml
<c:ser>
  <c:xVal><c:numRef><c:f>Sheet1!$A$2:$A$10</c:f><c:numCache>…</c:numCache></c:numRef></c:xVal>
  <c:yVal><c:numRef><c:f>Sheet1!$B$2:$B$10</c:f><c:numCache>…</c:numCache></c:numRef></c:yVal>
  <c:bubbleSize><c:numRef><c:f>Sheet1!$C$2:$C$10</c:f>…</c:numRef></c:bubbleSize> <!-- bubble only -->
</c:ser>
```

- `c:xVal` = X (both numeric), `c:yVal` = Y, `c:bubbleSize` = third dimension (bubble
  area/width). A reader must **switch on chart type** to know which slots exist.

### 4f. Extended (`cx:`) read model (contrast)

The extended family separates *data* from *presentation*:
`cx:chartSpace` → `cx:chartData` (holds `cx:data` blocks with `cx:numDim`/`cx:strDim`
dimensions, each with `cx:f` + cached `cx:lvl`/`cx:pt`) **and** `cx:chart` →
`cx:plotArea` → `cx:plotAreaRegion` → `cx:series` (with the `layoutId` that names the
type, referencing data via `cx:dataId`). So type selection is an **attribute**, and the
data is referenced by ID rather than embedded per-series. Out of scope for us, but worth
knowing it is *not* a drop-in of the `c:` model.

---

## 5. Per-family config / style knob inventory

Concise inventory (not exhaustive) of the knobs that affect *rendering* and that a reader
would want to surface. All live inside the relevant chart-group element or `c:ser`.

**Type / layout knobs (per chart-group element):**

| Knob | Element(s) | Values / meaning |
|---|---|---|
| Bar orientation | `c:barDir` | `col` (Column) / `bar` (Bar). `barChart`, `bar3DChart`. |
| Grouping | `c:grouping` | `standard` / `clustered` / `stacked` / `percentStacked`. On bar/line/area (bar uses `clustered`; line/area use `standard`). Drives whether series stack. |
| Bar spacing | `c:gapWidth`, `c:overlap` | gap between category clusters / bar overlap %. |
| Vary colors | `c:varyColors` | `1` = color each point differently (pie/doughnut default). |
| Scatter style | `c:scatterStyle` | `none`/`line`/`lineMarker`/`marker`/`smooth`/`smoothMarker`. |
| Radar style | `c:radarStyle` | `standard`/`marker`/`filled`. |
| Line smoothing / markers | `c:smooth`, `c:marker` | curved lines; marker symbol/size. |
| Doughnut hole | `c:holeSize`, `c:firstSliceAng` | ring thickness / rotation. |
| Of-pie split | `c:ofPieType`, `c:splitType`, `c:splitPos` | Pie-of-Pie vs Bar-of-Pie + how points split. |
| 3D | `c:view3D` (on `c:chart`), `c:shape`, `c:serAx` | rotation/perspective; box/cylinder/cone shapes; extra series axis. **gpui-component has no 3D — expect to flatten to 2D.** |

**Series / point styling:**

| Knob | Element | Meaning |
|---|---|---|
| Per-series color/line | `c:ser` → `c:spPr` → `a:solidFill` (+ `a:ln`) | fill & outline for the whole series. Color can be `a:srgbClr val="RRGGBB"` or theme `a:schemeClr`. |
| Per-point override | `c:ser` → `c:dPt` (has `c:idx` + its own `c:spPr`) | override color/format for a single data point (e.g. one pie slice / one bar). |
| Series marker | `c:ser` → `c:marker` | marker shape/size/fill for line/scatter. |

**Chart chrome:**

| Knob | Element | Meaning |
|---|---|---|
| Title | `c:chart` → `c:title` (rich text in `c:tx`/`a:r`, or `c:strRef` to a cell); `c:autoTitleDeleted` | chart title text/visibility. |
| Legend | `c:chart` → `c:legend` | `c:legendPos` = `t`/`b`/`l`/`r`/`tr`; optional `c:legendEntry` deletions. |
| Category axis | `c:plotArea` → `c:catAx` | `c:delete`, `c:title`, `c:numFmt`, tick/label settings; linked via `c:axId`. |
| Value axis | `c:plotArea` → `c:valAx` | scale (`c:scaling` min/max/`c:orientation`), `c:majorGridlines`, `c:numFmt`. |
| Date/series axis | `c:dateAx`, `c:serAx` | date axis for time categories; series axis for 3D. |
| Data labels | `c:dLbls` (chart-level and/or per-`c:ser`/per-`c:dPt`) | `c:showVal`, `c:showCatName`, `c:showSerName`, `c:showPercent`, `c:showLegendKey`, `c:numFmt`, position. |

**Rendering-priority takeaway for the charts project:** to render a recognizable chart we
mostly need **type (chart-group element + `barDir`/`grouping`)**, **series data
(`c:cat`/`c:val` or `c:xVal`/`c:yVal` via cache)**, **per-series/point color (`c:spPr`/`c:dPt`)**,
and **title/legend/axis titles**. The style/color parts (`styleN.xml`/`colorsN.xml`) and
3D/`view3D` knobs are lower priority (and 3D is likely un-renderable via gpui-component).

---

## 6. Adversarial sanity checks & open flags

- **"17 chart types"** — soft number. The `c:` schema exposes **16 chart-group elements**
  (§2a). "17" is an Excel-UI-family headline, not a schema count. Do not over-index on it.
- **`c:barChart` = Column *and* Bar** — verified across ECMA `CT_BarChart` summaries;
  orientation is `c:barDir`, not a separate element. Confirmed.
- **Combo charts have no element** — they are just multiple chart-group elements sharing a
  `c:plotArea`. Confirmed by `CT_PlotArea` allowing repeated/heterogeneous groups.
- **Scatter/bubble use `xVal`/`yVal`(/`bubbleSize`), not `cat`/`val`** — verified
  (openpyxl scatter docs + ECMA `CT_ScatterSer`/`CT_BubbleSer`).
- **chartex is NOT ECMA-376** — it is a Microsoft extension ([MS-ODRAWXML], `2014/chartex`
  namespace). Verified via Microsoft Learn + EPPlus ChartEx API. Excel typically also
  emits a classic fallback chart for compatibility **[verify exact fallback behavior]**.
- **`layoutId` literals for extended types** ("sunburst"/"treemap"/"waterfall"/
  "boxWhisker"/"paretoLine"/"funnel"/"regionMap") — the *set* is well corroborated; the
  *exact strings* are **[verify against MS-ODRAWXML]** before coding against them.
- **`chartEx1.xml` part name** and **chart-sheet paths** — standard but **[verify]** on a
  real Excel-produced file before relying on the literal names.
- Could not fetch datypic/c-rex/liquid-technologies/openpyxl/Microsoft-Learn directly (403
  to the automated fetcher); the structural facts were corroborated across their
  web-search summaries **plus a fetched real `chart1.xml`**. Anyone hardening this into a
  spec should open the cited pages in a browser to confirm exact cardinalities/attributes.

### Key sources
- ECMA-376: <https://ecma-international.org/publications-and-standards/standards/ecma-376/>
- Classic chart ns: `http://schemas.openxmlformats.org/drawingml/2006/chart`
- chartex [MS-ODRAWXML]: <https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/e2723b0a-9120-42a5-bd11-c252ccb13c1e>
- datypic OOXML: <http://www.datypic.com/sc/ooxml/>
- c-rex ECMA-376 mirror: <https://c-rex.net/samples/ooxml/>
- liquid-technologies schema: <https://schemas.liquid-technologies.com/OfficeOpenXML/2006/dml-chart_xsd.html>
- openpyxl charts: <https://openpyxl.readthedocs.io/en/stable/charts/introduction.html>
- XlsxWriter charts: <https://xlsxwriter.readthedocs.io/working_with_charts.html>
- Open XML SDK / SpreadsheetML: <https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/structure-of-a-spreadsheetml-document>
- OPC anatomy: <http://officeopenxml.com/anatomyofOOXML-xlsx.php>
- Real chart XML: <https://github.com/Vitaliy-1/DOCX2JATS/blob/master/stylesheets/charts/chart1.xml>
