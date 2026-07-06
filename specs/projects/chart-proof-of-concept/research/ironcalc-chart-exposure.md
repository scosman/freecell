# IronCalc 0.7.1 — Chart Data Exposure

**Research question:** To display charts from a `.xlsx`, we need the chart definitions
out of the file. Does IronCalc 0.7.1 **parse/expose** chart data on load, or must
FreeCell parse it itself? And how hard is "roll our own"?

**Bottom line (verified):** IronCalc 0.7.1 exposes **no chart data whatsoever**. Its
data model has no chart/drawing type, its xlsx importer never reads `xl/charts/` or
`xl/drawings/`, and its writer never emits them. Charts are officially a **post-1.0
"Planned"** feature upstream. So FreeCell **must roll its own** chart extraction if it
wants charts at all. The good news: FreeCell **already does exactly this kind of
read-only second pass over the same zip** (`open_fixups.rs`), so a display-only chart
extractor is a **tractable, weekend-to-few-days-sized** parser for the basic in-scope
types — reusing crates already in the dependency tree. **Preserving** charts through
edit+save is a **separate, much harder** problem (owned by the `.xlsx`-preservation
project), because IronCalc's writer regenerates the file from a model that has no charts.

Pin under study: `ironcalc = "=0.7.1"`, `ironcalc_base = "=0.7.1"`
(`app/Cargo.toml:35-36`).

---

## 1. The data model has no chart/drawing type — VERIFIED

At the `v0.7.1` tag, the two model structs that could plausibly hold a chart carry **no
chart, drawing, graphicFrame, image, or shape field**:

`ironcalc_base::types::Worksheet` fields (verbatim, from the v0.7.1 source):

```rust
pub struct Worksheet {
    pub dimension: String,
    pub cols: Vec<Col>,
    pub rows: Vec<Row>,
    pub name: String,
    pub sheet_data: SheetData,
    pub shared_formulas: Vec<String>,
    pub sheet_id: u32,
    pub state: SheetState,
    pub color: Option<String>,
    pub merge_cells: Vec<String>,
    pub comments: Vec<Comment>,
    pub frozen_rows: i32,
    pub frozen_columns: i32,
    pub views: HashMap<u32, WorksheetView>,
    pub show_grid_lines: bool,
}
```

`ironcalc_base::types::Workbook` fields:

```rust
pub struct Workbook {
    pub shared_strings: Vec<String>,
    pub defined_names: Vec<DefinedName>,
    pub worksheets: Vec<Worksheet>,
    pub styles: Styles,
    pub name: String,
    pub settings: WorkbookSettings,
    pub metadata: Metadata,
    pub tables: HashMap<String, Table>,
    pub views: HashMap<u32, WorkbookView>,
}
```

There is **nowhere in the model to store a chart**. This is the decisive fact: even if
the importer wanted to read a chart, it would have nowhere to put it. FreeCell already
reaches this model directly (`document.rs:301-304`, `worksheet()` returns
`&ironcalc_base::types::Worksheet`), so we know first-hand there is no chart accessor to
call.

Source: `https://raw.githubusercontent.com/ironcalc/IronCalc/v0.7.1/base/src/types.rs`
(struct definitions; also on docs.rs at `ironcalc_base/0.7.1/.../struct.Worksheet.html`,
which 403s to automated fetch but matches the tagged source).

---

## 2. The xlsx importer never reads `xl/charts/` or `xl/drawings/` — VERIFIED

IronCalc's reader is its own roxmltree-based importer (not calamine). Reading the v0.7.1
importer source directly:

**`xlsx/src/import/mod.rs`** reads a **fixed, enumerated set of parts** from the zip and
nothing else:
`xl/_rels/workbook.xml.rels`, the shared strings, `xl/workbook.xml`, the worksheets
(via `load_sheets`), `xl/styles.xml`, and metadata. There is **no read of any
`xl/drawings/*` or `xl/charts/*` part**.

**`xlsx/src/import/worksheets.rs`** parses only these worksheet child elements:
`dimension`, `cols`, `sheetPr` (tab color), `sheetViews`, `sheetData` (rows/cells),
`mergeCells` (plus comments/tables handled around it). It contains **no branch for
`<drawing>`**, no drawing-relationship follow, no `graphicFrame`/`oleObject`/picture
handling. The worksheet's `<drawing r:id=.../>` element — the anchor that would point at
a chart — is simply **not looked at**, so the chart is discarded at LOAD time and never
reaches the model.

Sources:
`https://raw.githubusercontent.com/ironcalc/IronCalc/v0.7.1/xlsx/src/import/mod.rs`,
`https://raw.githubusercontent.com/ironcalc/IronCalc/v0.7.1/xlsx/src/import/worksheets.rs`.

**Chart *sheets* (standalone chart tabs) are explicitly dropped.** IronCalc's own reader
documentation states, for a sheet whose relationship type is `chartsheet` (target
`chartsheets/sheet1.xml`): *"In IronCalc we ignore those sheets."*
(`xlsx/documentation/workbook.md`). So a workbook whose chart lives on its own chart
sheet loses the sheet entirely (it does not even appear as an empty worksheet);
a chart *embedded* in a data worksheet keeps the worksheet but loses the chart.

**Upstream confirms this is by design, not an oversight.** IronCalc's docs list Charts
as an explicitly unsupported, *Planned* feature: *"Although charts are an essential
feature for any serious spreadsheet program, they are not planned for version 1.0.
Adding chart support will become a high priority after the release of version 1.0."*
(`docs/src/features/unsupported-features.md`).

---

## 3. The round-trip / save problem — VERIFIED, and it is separate from display

IronCalc's writer **regenerates** the xlsx from the model. Reading the v0.7.1 exporter
(`xlsx/src/export/mod.rs`), the complete set of parts it writes is:
`[Content_Types].xml`, `docProps/app.xml`, `docProps/core.xml`, `_rels/.rels`,
`xl/workbook.xml`, `xl/sharedStrings.xml`, `xl/styles.xml`,
`xl/_rels/workbook.xml.rels`, and one `xl/worksheets/sheet{N}.xml` per sheet.
It writes **no `xl/drawings/`, `xl/charts/`, or chartsheet parts** — it cannot, because
the model has none (§1). This matches the FreeCell repo's standing claim that
"validation, hyperlinks, **charts**, pivots, drawings, VBA are silently dropped on save"
(`specs/projects/mvp/functional_spec.md:261`) and the `.xlsx`-preservation note in
`PROJECTS.md`.

**Implication — keep two problems distinct:**

- **"Render charts we read from the file" (display-only).** Parse the chart parts
  ourselves at open, hand chart definitions to the UI, render with `gpui-component`.
  IronCalc is irrelevant to this path except that it evaluates the cells a chart's
  series may reference. This is the in-scope, tractable problem (§4).
- **"Preserve/write charts on save" (round-trip).** Because IronCalc's writer strips
  every chart, even a perfect load-time parse is thrown away on the next Save. Making
  charts *survive* an edit+save requires either zip-level pass-through of the original
  chart parts (fiddly: `[Content_Types].xml`/rels splicing, and a chart pointing at
  edited cells goes **stale**) or owning the writer (the ~10× trap). This is **not** a
  charts-project problem to solve here — it is exactly the
  `projects/xlsx-preservation.md` decision (options: warn-and-strip / zip pass-through /
  own the writer). **A charts MVP should be display-only and accept that saving drops
  charts**, consistent with the current MVP save-fidelity call.

---

## 4. "Roll our own" assessment — read-only second pass over the same zip

### The infrastructure already exists in FreeCell — VERIFIED

This is the key finding for feasibility. FreeCell **already** does a read-only,
best-effort second pass over the opened `.xlsx` to fix things IronCalc imports wrong:
`app/crates/freecell-engine/src/open_fixups.rs` re-opens the zip and reads
`xl/theme/theme1.xml` and `xl/styles.xml` with **`roxmltree` + `zip`**, parsing OOXML
nodes by tag name and reconciling child-index ordering with IronCalc's tables. It is
wired into the load path at `document.rs:184` (`apply_open_fixups(&mut model, path)`),
runs during `WorkbookDocument::open(path)`, is fully unit-tested against crafted zips,
and is explicitly "best-effort: any parse/read failure leaves the model as IronCalc
imported it (never fails the open)."

A chart extractor is **the same shape of code**: open the zip, walk a few XML parts by
tag name, build FreeCell-owned structs, never fail the open on a parse error. The
helper `read_zip_entry(path, name)` (`open_fixups.rs:305-313`) is a 9-line
`zip::ZipArchive::by_name` reader we can reuse directly.

Crates needed are **already in the tree** (no new dependencies):

- `zip = "0.6"` — already a direct dep of `freecell-engine` (`Cargo.toml`), used by
  `open_fixups`.
- `roxmltree = "0.19"` — already a direct dep, used by `open_fixups`. (IronCalc's own
  importer is roxmltree-based too, so the parsing style is consistent.)

### What the parser has to walk (the OOXML chain)

For an **embedded** chart the resolution chain is:

1. `xl/worksheets/sheet{N}.xml` → `<drawing r:id="rIdD"/>` (the anchor pointer).
2. `xl/worksheets/_rels/sheet{N}.xml.rels` → maps `rIdD` → `../drawings/drawing{M}.xml`.
3. `xl/drawings/drawing{M}.xml` → `<xdr:twoCellAnchor>`/`<xdr:oneCellAnchor>` giving the
   cell-anchored position + size, containing `<xdr:graphicFrame>` →
   `<a:graphic>/<a:graphicData>/<c:chart r:id="rIdC"/>`.
4. `xl/drawings/_rels/drawing{M}.xml.rels` → maps `rIdC` → `../charts/chart{K}.xml`.
5. `xl/charts/chart{K}.xml` — the actual DrawingML chart
   (`<c:chartSpace>/<c:chart>/<c:plotArea>` with a `<c:barChart>` / `<c:lineChart>` /
   `<c:pieChart>` / `<c:areaChart>` / `<c:scatterChart>` and one `<c:ser>` per series;
   each series has `<c:tx>` name, `<c:cat>` categories, `<c:val>` values). Each
   `<c:cat>`/`<c:val>` is a `<c:numRef>`/`<c:strRef>` holding a `<c:f>` range formula
   (e.g. `Sheet1!$B$2:$B$9`) **and** a `<c:numCache>`/`<c:strCache>` of cached `<c:pt>`
   values.

A **series' data can be obtained two ways**, and this is a real design choice:
   - **Cached values** — read the `<c:numCache>`/`<c:strCache>` `<c:pt>` values straight
     out of `chart{K}.xml`. Zero dependence on IronCalc; simplest; but the cache can be
     **stale** relative to the (recomputed) cells.
   - **Live values** — parse the `<c:f>` range reference and pull the values from
     IronCalc's evaluated model (`get_formatted_cell_value` / cell reads). Live and
     consistent with what the grid shows, at the cost of a small A1-range parser
     (cross-sheet, absolute refs, and — the annoying tail — named ranges and
     multi-area unions).

Chart *sheets* (standalone) skip steps 1–2: `xl/workbook.xml.rels` points a
`chartsheet` relationship at `xl/chartsheets/sheet{N}.xml`, which carries its own
`<drawing>` → `xl/drawings/...` → `xl/charts/...`. Since IronCalc drops the chartsheet
entirely (§2), FreeCell would also need to read `xl/workbook.xml.rels` to discover these
if we want to support standalone chart sheets (lower priority — most real charts are
embedded).

### Complexity: weekend-sized for basic types, with a well-defined swamp beyond

**Tractable (days, reusing `open_fixups` infrastructure) — INFERENCE, well-grounded:**
the anchor/rels traversal is a handful of `roxmltree` tag walks; the basic chart types
in scope (bar/column, line, pie, area, scatter — the `gpui-component`-supported set per
`project_overview.md`) share the same `<c:ser>` → `<c:cat>`/`<c:val>` shape; reading
**cached** series values needs no cell resolution at all. A read-only extractor that
yields `{chart_type, title, series[{name, categories, values}], anchor_rect}` for those
types is a focused, testable module in the exact style FreeCell already ships.

**The swamp (out of MVP scope) — INFERENCE:** the long tail is large — combo charts
(multiple plot types in one), secondary axes, stacked/100%-stacked variants, 3-D
(`bar3DChart` etc.), stock/radar/surface/doughnut/bubble, per-point/per-series color and
`<c:dPt>` overrides, data labels, legends positioning, axis number formats & scaling,
trendlines, error bars, and robust `<c:f>` reference resolution (named ranges,
non-contiguous unions, cross-sheet). Trying to render *all* of these faithfully — or to
*write* them back — is where it becomes a multi-week effort. Scoping the MVP to
"basic types, cached-or-simple-range values, approximate anchoring, no round-trip" keeps
it firmly on the tractable side.

---

## 5. Does IronCalc hand us the bytes / zip? No — but FreeCell already re-opens the path — VERIFIED

IronCalc does **not** expose the raw file bytes or a zip handle. `WorkbookDocument`
loads via `load_from_xlsx(path_str, ...)` (`document.rs:179`), a **path-based** API, and
stores only the `UserModel` — it does **not** retain the source path or bytes as a
field (`WorkbookDocument { model }`, `document.rs:132-135`). There is no
`get_original_bytes()` on the model.

But this is a non-problem, because the open path **already has the `Path` in hand** and
already re-opens the zip: `open(path)` passes `path` to `apply_open_fixups(&mut model,
path)` (`document.rs:184`), which re-reads zip entries from that same path. So the
natural fit is: **extract chart definitions during `open()`, right beside
`apply_open_fixups`, while the `Path` is available**, producing FreeCell-owned chart
structs to publish to the UI (charts don't change during recompute, so a once-at-open
extraction mirrors the resident style-cache pattern). Note the file is *already* read
twice today (once by IronCalc's importer, once by `open_fixups`); a chart pass is a
third cheap read of the same local file, or can be folded into the existing
`open_fixups` zip pass. If we ever need to re-extract after open, we'd start persisting
the source path on `WorkbookDocument` — a trivial field add — but display-at-open needs
nothing new.

---

## Adversarial check on "IronCalc reads no chart data at all"

This is the load-bearing surprising claim, so I tried to falsify it:

- **Whole-repo code search for chart/drawing readers** (`xl/drawings`, `xl/charts`,
  `chartsheet`, `graphicFrame`): **0 hits**. A Rust-only search for `drawing`/`drawings`:
  **0 hits**. (GitHub code search covers the default branch and under-indexes very short
  tokens, so this is corroborating, not sole, evidence.)
- **False positives ruled out.** The only `drawing` hits anywhere are (a) a *webapp*
  TypeScript canvas helper (`cfRenderer.ts`, "Border drawing helpers") and (b) the
  `http://schemas.openxmlformats.org/drawingml/2006/main` **namespace** inside the
  export **theme** template (`xlsx/src/export/theme1.xml`). DrawingML-the-namespace is
  shared by themes/fonts/colors; neither is spreadsheet chart/drawing *part* handling.
- **Primary evidence is the tagged source, not just search:** the v0.7.1 `types.rs`
  (no chart field), `import/worksheets.rs` (no `<drawing>` branch), `import/mod.rs`
  (fixed part list), and `export/mod.rs` (fixed part list) were each read directly at
  the `v0.7.1` tag and independently confirm the claim from three angles (model, reader,
  writer).
- **Upstream intent agrees:** charts are documented as unsupported/Planned-post-1.0, and
  chartsheets are documented as explicitly ignored.

I found **no counter-evidence**. The claim holds. Residual uncertainty is low: the one
part I did not read line-by-line is `load_sheets`' exact relationship-type filter, but
its behavior is pinned down by IronCalc's own docs ("we ignore those sheets") and by the
model having no chart field regardless.

---

## Verdict

- **Does IronCalc 0.7.1 expose chart data? No — categorically.** No model type, no
  importer read, no writer emit; charts are an explicit post-1.0 upstream feature.
  FreeCell must roll its own if it wants charts.
- **Display-only "roll our own": tractable / weekend-to-few-days for basic types.**
  FreeCell already does the identical read-only zip second-pass in `open_fixups.rs`
  using `zip` + `roxmltree` (both already deps), already holds the file `Path` at open,
  and the basic-chart OOXML shape is uniform. Scope it to the `gpui-component`-supported
  types with cached-or-simple-range series values and approximate anchoring.
- **Save-preservation "roll our own": hard / out of scope here.** IronCalc's writer
  regenerates the file and strips all charts, so round-tripping charts is the separate
  `.xlsx`-preservation problem (zip pass-through vs owning the writer, plus the
  stale-chart-on-edited-data issue). A charts MVP should be **display-only** and accept
  that Save drops charts.

## Open questions for the discussion phase

1. **Cached vs live series values** — read `<c:numCache>` (simple, possibly stale) or
   resolve `<c:f>` against IronCalc's evaluated model (live, needs an A1-range parser)?
   Recommend starting with cached, upgrading to live for contiguous single-sheet refs.
2. **Which chart types are truly in scope** — gated by the *separate* gpui-component
   capability research; this doc only establishes we can *extract* them.
3. **Standalone chart sheets** — support (needs `xl/workbook.xml.rels` reading, since
   IronCalc drops the sheet) or defer to embedded-only for MVP?
4. **Where extracted charts live** — publish once-at-open to the UI (like the style
   cache), or persist the source path on `WorkbookDocument` for re-extraction? Once-at-
   open is the smaller change.

### Sources

- Repo: `app/Cargo.toml:35-36` (pins), `app/crates/freecell-engine/Cargo.toml`
  (`zip`/`roxmltree` deps), `app/crates/freecell-engine/src/open_fixups.rs` (existing
  zip second-pass; `read_zip_entry` at :305), `app/crates/freecell-engine/src/document.rs`
  (:132-135 struct, :179 `load_from_xlsx`, :184 `apply_open_fixups`, :301-304
  `worksheet()`), `specs/projects/mvp/functional_spec.md:259-264`,
  `PROJECTS.md` + `projects/xlsx-preservation.md` (preservation project),
  `experiments/01-file-support/findings.md` (IronCalc native reader/writer).
- IronCalc v0.7.1 source (raw.githubusercontent.com, `v0.7.1` tag): `base/src/types.rs`
  (Worksheet/Workbook structs), `xlsx/src/import/mod.rs`, `xlsx/src/import/worksheets.rs`,
  `xlsx/src/export/mod.rs`.
- IronCalc docs/repo (github.com/ironcalc/IronCalc): `docs/src/features/unsupported-features.md`
  ("Charts — Planned, not in 1.0"), `xlsx/documentation/workbook.md` ("chartsheet … In
  IronCalc we ignore those sheets"), `xlsx/documentation/README.md`.
- docs.rs: `docs.rs/ironcalc_base/0.7.1/` and `docs.rs/ironcalc/0.7.1/` (403 to automated
  fetch; content matches the tagged source above).
