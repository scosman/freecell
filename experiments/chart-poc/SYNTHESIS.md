# Chart Proof of Concept — SYNTHESIS (go/no-go)

**The deliverable of the whole PoC** (functional_spec §0): the evidence-backed go/no-go call
on chart support in FreeCell, plus a recommended scope and rough shape for a follow-on
ship-quality project. It aggregates the committed PNGs, the per-image agent-review tables, and
each experiment's `findings.md` from Phases 0–4. Every claim below is cross-checked against
those sources; relative links point at the committed evidence.

Spec: [`specs/projects/chart-proof-of-concept/`](../../specs/projects/chart-proof-of-concept/).
Build record: 5 phase commits, `65f0751` (Phase 0) → `c754613` (Phase 4), all gates passed.

> **Fidelity caveat / read this alongside the verdict:** the GO below is a go on *render
> feasibility*, proven on a **thin structural slice** of the OOXML model (type + series data
> + one color + title/legend/axis-titles + grouping) and only **3 agent-authored** load
> fixtures. It is **not** a proof of OOXML *fidelity*. The feature-by-feature ledger — what's
> validated vs. extendable vs. out, by priority — is in
> [`ooxml-coverage-matrix.md`](ooxml-coverage-matrix.md), and building a faithful model is the
> follow-on's largest job.

---

## 1. The verdict: **GO**

Building acceptable charts on gpui-component's `plot/` primitives is **demonstrably
achievable**, and it is not close. All four early-bail gates cleared cleanly, in order of how
badly a "no" would have hurt:

- **Gate 1 — the make-or-break (§7).** The multi-series line chart with title, both axis
  titles, a readable numeric value axis (our own "nice" ticks), a category axis, and a legend —
  the user's stated bail example — was judged by a **3-agent panel, unanimous PASS (3/3)**, with
  all three reviewers agreeing YES on every one of the seven §6 rubric points.
  ([`line_multi.png`](chart-render/results/line_multi.png))
- **Gate 2 — harder layouts (§7).** **All nine** research-trap layouts (grouped, stacked,
  100%-stacked column; stacked and 100%-stacked area; pie; doughnut; horizontal bar; the
  regenerated single column) returned single-agent **PASS**. No wholesale grouped/stacked FAIL —
  a GO signal, not the PARTIAL-GO fallback.
- **Gate 3 — scatter (§7).** Both single- and multi-series scatter (two numeric axes + dots)
  returned **PASS** → scatter is IN-scope for the follow-on. **Bubble** rides on this same
  path and is IN too — **by code analysis, not a rendered gate** (it is scatter with a
  per-point marker radius from a third `c:bubbleSize` value); see
  [`bubble-analysis.md`](bubble-analysis.md).
- **Gate 4 — load/save (§7).** Parsing charts out of a real `.xlsx` into the shared model and
  rendering them back returned **PASS** on all three loaded charts (**LOAD PASS**); and
  byte-preservation re-injection survives IronCalc's chart-dropping writer, verified three ways
  (**SAVE PASS**) → display **+ save-preservation** is justified, not display-only.

**Tally: 16/16 reviewed images PASS — zero MARGINAL, zero FAIL** (13 hand-built `chart-render`
scenes + 3 loaded-from-xlsx scenes), plus a save round-trip proven programmatically. Mapped to
functional_spec §9, this is the **GO** branch verbatim: "Gate 1 passes and
grouped/stacked/area/pie mostly PASS → a follow-on ship-quality project is justified with the
full in-scope type set," with scatter (Gate 3) and save-preservation (Gate 4) both **in**.

**Honest tempering.** The verdict is a confident GO, but a PoC's value is an accurate decision,
so the caveats are real (§4 below). Every PASS was against a **relaxed-rigor** bar — "would a
user accept this as a real chart of their data," judged by a Claude reviewer viewing the pixels,
not perceptual-diff-vs-Excel. Two things in particular are *proven feasible* but *not proven at
ship quality*: (a) several capabilities are our own hand-rolled code the library does not
provide (the stacked-**area** polygon fork, grouped/stacked slot math, the pie palette, the nice
tick generator), so the follow-on maintains them; and (b) **nothing here touches `/app`** —
interactive rendering, live re-render on edit, and huge-sheet performance are entirely
unexercised (functional_spec §8, by design). The GO is a green light for a *ship-quality
project*, and that project starts with real unknowns, not a finished feature.

---

## 2. Per-variation results table (§6 / §9 backbone)

Aggregated from [`chart-render/results/review.md`](chart-render/results/review.md) and
[`load-save/results/review.md`](load-save/results/review.md). Verdicts are advisory input to
this decision (§6), not an automated gate. **G1 = 3-agent panel; all others single-agent.**

### Rendered from a hand-built `chart-model::Chart` (Experiments 2 & 3)

| # | Variation | Gate | Verdict | Key agent note / what it proved |
|---|---|---|---|---|
| 1 | **Multi-series line** (3 series, shared axis) | **1 (make-or-break)** | **PASS (3/3 panel)** | Straight segments on one shared `ScaleLinear`; distinct hues; legend↔line color exact; "publication-quality" ([png](chart-render/results/line_multi.png)) |
| 2 | Single-series line | sanity | PASS | Straight line + dots, nice value axis, matching legend ([png](chart-render/results/line_single.png)) |
| 3 | Single-series **column** | 0 / 2 (re-review) | PASS | Own nice-tick value axis (0/50/…/200); no regression after `bar.rs` was generalized ([png](chart-render/results/bar_single.png)) |
| 4 | **Horizontal bar** | 2 | PASS | Swapped geometry correct; bars grow left→right, categories down the left ([png](chart-render/results/bar_horizontal.png)) |
| 5 | **Grouped (clustered)** column, 3 series | 2 | PASS | DIY slot math (the library `ScaleBand` 30px cap avoided); non-overlapping side-by-side bars ([png](chart-render/results/bar_grouped.png)) |
| 6 | **Stacked** column, 3 series | 2 | PASS | Cumulative segments; value axis reaches the stack total (~365 within 0–400) ([png](chart-render/results/bar_stacked.png)) |
| 7 | **100%-stacked** column, 3 series | 2 | PASS | Full-height columns; 0%–100% axis at 20% intervals ([png](chart-render/results/bar_percent_stacked.png)) |
| 8 | **Stacked area**, 3 series | 2 | PASS | The nastiest layout: hand-rolled polygon **fork** of the scalar-baseline `Area` primitive; bands stack on the running total, not from zero ([png](chart-render/results/area_stacked.png)) |
| 9 | **100%-stacked area**, 3 series | 2 | PASS | Bands fill 0–100% at every x; normalize pass correct ([png](chart-render/results/area_percent_stacked.png)) |
| 10 | **Pie**, 5 slices | 2 | PASS | The no-auto-palette crux solved: 5 **distinct** synthesized colors (not a monochrome disc), on-slice % labels, slice↔legend by construction ([png](chart-render/results/pie.png)) |
| 11 | **Doughnut**, 5 slices | 2 | PASS | Pie with `inner = 0.55 × outer`; distinct arcs + % labels ([png](chart-render/results/doughnut.png)) |
| 12 | **Single-series scatter**, 10 pts | 3 | PASS | Two numeric axes + standalone dots; positive trend reads; both axis titles ([png](chart-render/results/scatter_single.png)) |
| 13 | **Multi-series scatter**, 3 clusters | 3 | PASS | Three distinct dot clusters on one shared X/Y numeric-axis pair; legend↔cluster exact ([png](chart-render/results/scatter_multi.png)) |

### Rendered from a `chart-model::Chart` parsed OUT of a real `.xlsx` (Experiment 1, LOAD)

| # | Variation (loaded from `fixtures/charts_basic.xlsx`) | Gate | Verdict | Key agent note |
|---|---|---|---|---|
| 14 | Clustered column "Quarterly Sales by Product" | 4 (LOAD) | PASS | Series **colors parsed from each `c:ser/c:spPr`**; correct geometry, both axis titles ([png](load-save/results/loaded_column.png)) |
| 15 | Multi-series line "Sales Trend by Product" | 4 (LOAD) | PASS | Two lines cross as the cached data implies; ticks at 20-unit intervals ([png](load-save/results/loaded_line.png)) |
| 16 | Pie "Quarterly Totals" | 4 (LOAD) | PASS | On-slice % (21/27/24/28) match the **cached** totals 200/260/230/270 of 960 ([png](load-save/results/loaded_pie.png)) |

### SAVE (Experiment 1, byte-preservation re-injection — not an image)

| Property | Result | How verified |
|---|---|---|
| Chart survives `open → IronCalc save → reopen` | **PASS** | `save_with_charts` re-injects `xl/charts/*` + `xl/drawings/*` byte-for-byte after IronCalc's writer; `roundtrip_preserves_charts` test + `fixtures` bin ([xlsx](load-save/results/roundtrip_charts_basic.xlsx)) |
| Cached values identical after round-trip | **PASS** (`after == before`) | Our own loader re-finds all three charts with identical `numCache` values |
| Output not corrupted | **PASS** | The re-injected `.xlsx` **reopens in IronCalc** without error |
| Structural validity | **PASS** | Worksheet carries `<drawing>`; `[Content_Types].xml` declares the chart parts; all chart/drawing parts present |

---

## 3. Recommended scope for the follow-on ship-quality project

### Chart types — **IN** (all Gate 1–3 PASS)
- **Line** (single + multi-series, straight; smoothing is a small add).
- **Bar / column**, both orientations, all three groupings: **single, clustered, stacked,
  100%-stacked**.
- **Area**: standard, stacked, 100%-stacked (carrying the hand-rolled polygon fork).
- **Pie & doughnut** with the synthesized per-slice palette.
- **Scatter** (Gate 3 PASS): **IN** — it was the cheapest new type of the whole PoC, almost
  entirely reuse (the "second numeric axis" is just the value axis applied twice; dots are the
  `Line` primitive's dot mark drawn without the path).
- **Bubble** (**IN by analysis**, not a rendered gate): scatter with a per-point marker radius
  from a third `c:bubbleSize` value — a tiny generalization of the passed scatter path (only
  new code: a √-scaled size→radius mapping + a max clamp). Full reasoning + residual risk in
  [`bubble-analysis.md`](bubble-analysis.md).

### Chart types — **OUT** (permanent, functional_spec §8)
Stock/candlestick, combo/multi-plot, radar, surface, all
3D, pie-of-pie, multi-ring doughnut, and the entire extended `cx:` family (sunburst, treemap,
waterfall, histogram, box-&-whisker, funnel, region map). Also out per §4/§8: trendlines and log
axes on scatter.

### Save behavior — **display + save-preservation** (Gate 4 PASS)
Recommend the follow-on ship **display + byte-preservation save** (charts survive save), not
display-only. Load is cheap and reliable; save re-injection is tractable with the three sharp
edges in §4 handled. Scope the save side to: single- **and** multi-sheet worksheet↔part mapping
via `xl/_rels/workbook.xml.rels`; carry-by-prefix for all chart-aux parts (`colorsN`/`styleN`);
and a **documented stale-on-edit caveat** (with cache-refresh as a later enhancement, not v1).

### Permanently out (whole follow-on, §8)
No exhaustive Excel pixel parity, no editing/creation UI in this scope discussion's frame beyond
what the ship project separately specs, no reflow of cached values on the byte-preservation path,
no chartsheets, no `cx:` family.

---

## 4. Known risks / sharp edges carried forward

Mostly pulled from the two `findings.md` files, plus an Excel-fidelity item noted by analysis
here (labelled *fidelity observation*). None was a blocker for the PoC; each is a bounded item
the follow-on must budget for.

**Render side** ([`chart-render/findings.md`](chart-render/findings.md)):

1. **We own more than we reuse for the hard layouts.** The library gives primitives, not
   charts. **Stacked area is a hand-rolled polygon fork** because the `Area` primitive closes
   its fill at a *scalar* `y0` and cannot draw a wavy stacked baseline (trace upper boundary
   forward, lower boundary back, `close()`, paint bottom→top). **Grouped/stacked geometry is DIY
   slot math**, not `ScaleBand` (whose `band_width()` is hard-capped at 30px). **Pie has no
   auto-palette** so we synthesize per-slice colors. **The nice value axis is ours** (`NiceScale`
   Heckbert ticks) — `ScaleLinear` ships none. All work, but the follow-on maintains them.
2. **Value-axis title is a horizontal caption, not a rotated vertical title.** Flagged by
   reviewers at every gate as a **cosmetic non-defect** (legible, correctly associated, no rubric
   point docked). gpui text has no cheap rotation at the pinned rev; a ship-quality follow-on
   should rotate it.
3. **Horizontal-bar category order is data order (top→bottom), not Excel's convention.**
   *(Fidelity observation added here — not stated in either `findings.md`/`review.md`; the
   `bar_horizontal` review PASSed the render.)* Excel puts the first category at the *bottom* of
   a horizontal bar; `bar.rs` renders first-at-top (data order). A fidelity item to match Excel,
   not a defect.
4. **Headless capture is on-screen, not windowless — and needs container prerequisites.** gpui
   has **no windowless GPU capture on Linux** at the pinned rev, so capture is a real gpui window
   under `xvfb-run` + Mesa **lavapipe** + `xrefresh` (force presentation) + ImageMagick `import`
   (grab by window id), with a blank-guard. The base container was **missing**
   `mesa-vulkan-drivers` (the lavapipe ICD — the Vulkan *loader* ships but there was no driver),
   `x11-xserver-utils`/`x11-utils`/`imagemagick`, and `libxkbcommon-dev` + wayland/xcb/x11 dev
   libs (the gpui link step needs `-lxkbcommon`). This is a build/CI-harness concern, not app
   code, but any capture-based regression suite must reproduce that setup.
5. **Library sharp edges (solved, but real):** `ScaleBand::tick` ignores its range start (slid
   bars into the axis gutter until fixed); primitive accessor closures are `'static` (widgets
   move owned clones in); `ScaleLinear` value type is `f64`-only (a non-issue — the model stores
   `f64`); paint order (grid → axis → marks) matters because marks paint last.

**File side** ([`load-save/findings.md`](load-save/findings.md)):

6. **IronCalc's naive parsers reject pretty-printed XML — the single biggest save gotcha.**
   `load_sheet_rels` and the worksheet `sheetData` parser iterate **raw** children and read an
   attribute on each; a whitespace/newline text node between elements trips `Missing "Type"` /
   `Missing "r"`. **Authored fixtures and re-injected `_rels` must be whitespace-free between
   elements** (real Excel is, for the same reason). Flag loudly for the follow-on.
7. **Save re-injection must *patch*, not just carry, three parts.** IronCalc regenerates the
   worksheet without a `<drawing>`, emits no worksheet `_rels`, and omits the chart/drawing
   content-type Overrides. Re-injection carries `xl/charts/*` + `xl/drawings/*` verbatim but must
   also (a) inject `<drawing r:id=…/>` + bind `xmlns:r` into IronCalc's worksheet, (b) write a
   worksheet `_rels`, and (c) merge the Overrides into `[Content_Types].xml`.
8. **Single-sheet-only mapping in the PoC.** Fixtures map 1:1 (`sheet1.xml` ↔ `sheet1.xml`).
   Multi-sheet workbooks need a proper sheet-index→part mapping via `workbook.xml.rels`
   (IronCalc's output part order isn't guaranteed to match the original). Out of PoC scope; the
   re-injection **fails loudly** if a targeted worksheet part is absent rather than silently
   dropping the chart.
9. **Stale-cache-on-edit.** Byte-preservation keeps the chart *as it was*, including its cached
   `numCache`; if the user edits the data cells before saving, the re-injected cache goes stale
   (and `c:f` still points at the old range). This is the accepted §8 limitation
   ("no reflow of cached values"); refreshing the cache from IronCalc's evaluated cells is a
   separate, larger effort (the "synthesize chart XML" stretch goal).
10. **No external-tool validation was possible in this container.** `soffice --headless
    --convert` fails with `source file could not be loaded` on **every** `.xlsx` here — including
    the app's known-good `numbers_table.xlsx` — so LibreOffice is broken/unavailable in this
    environment (a Java/headless-profile issue), **not** a problem with our files. Validity was
    confirmed instead by our own loader reopen + **IronCalc reopen** + structural zip inspection.
    A ship-quality follow-on must round-trip against **real Excel and LibreOffice**.
11. **Fixtures were agent-authored (§10 #4), not real-world Excel exports.** The accepted PoC
    approach, but the long tail of real-world chart XML variety (odd namespaces, richer styling,
    combo/scatter edge shapes) is untested; the follow-on needs a real-file corpus.
12. **Agent review is advisory and subjective.** Single verdict per image (except Gate 1's
    3-panel), Claude reviewers viewing pixels — not perceptual-diff-vs-baseline. Mitigated by the
    3-agent panel on the make-or-break image, but a ship pipeline should add stability
    (perceptual-diff) checks — which (my inference, not a findings statement) could reuse
    `round-3/C-ci-rendering`'s pipeline as a starting point.

---

## 5. Rough shape for the follow-on ship-quality project

### The seam that survives
**`chart-model`** — the small, gpui-free, ironcalc-free, **OOXML-`c:`-shaped** data model
(functional_spec §2) — is the load-bearing abstraction and is designed to be **kept as-is**.
Experiment 1 parses *into* it (from cached `numCache`/`strCache`, no formula eval); Experiments
2 & 3 render *from* it. It held across all four gates without a shape change. The follow-on
should treat it as the stable contract between the file layer and the render layer.

### Proven reusable (lift-and-keep, not rewrite)
- **Shared chrome** — `chrome::chart_frame` (title + axis-title captions + `[plot | legend]`
  row), one dispatch point in `lib.rs::chart_element` over `ChartKind`.
- **Categorical palette** — the `series_color`/`slice_color` Tableau-style cycle (extends past
  5 by hue rotation); legend↔mark mapping is correct **by construction** (same index, same
  source) for every type.
- **`NiceScale` tick generator** (`ticks.rs`) with two modes: `for_values` (force zero — bars)
  and `spanning` (auto-range to data — line/area/scatter, matching Excel).
- **`stacking.rs`** — one gpui-free module (`stacked_segments` / `percent_segments` /
  `category_totals`) shared by stacked bars **and** stacked areas + the percent normalize pass.
- **Capture harness** — `render.rs` (`run_render_chart`) + `capture.rs` (`capture_window`);
  reused unchanged by `load-save`'s `capture_loaded`.
- **Load parser** — `xlsx.rs` + `load.rs` (~570 non-test lines total; `load-save/findings.md`'s
  "~450 lines" for `load.rs` + `xlsx.rs` is an undercount) — `zip 0.6` + `roxmltree 0.19`,
  local-name matching so it's namespace/prefix-agnostic; covers the whole `c:ser → c:cat/c:val`
  family (bar/line/area/pie/doughnut/scatter) in one parser.
- **Save re-injection** — `save.rs` (IronCalc writer → byte-preservation splice), the
  single-sheet case fully solved.

### Biggest remaining unknowns for ship quality
1. **App integration — entirely unproven.** The PoC is static PNG only, nothing in `/app`.
   Rendering a chart *inside the real grid* (positioning, hit-testing, z-order over cells),
   **selection/move/resize**, hover/tooltips, and **live re-render on data change** are all
   net-new and are where the real ship risk now lives.
2. **Performance / huge-sheet scaling — untested.** The relaxed-rigor bar was "renders a handful
   of examples"; no perf, no many-charts-on-a-sheet, no large-series-count work was done.
3. **Cache refresh / edit-reflow** (the stale-on-edit problem, risk #9) — needed if charts must
   track edits; this is the "synthesize chart XML from our model" stretch goal, a separate effort.
4. **Real-file load robustness + external round-trip validation** (risks #10, #11) — a real
   Excel/LibreOffice corpus, never exercised here.
5. **Fidelity polish** (all bounded, known): rotate the value-axis title (#2), reverse
   horizontal-bar category order (#3), multi-sheet save mapping + chart-aux parts (#8).

**Bottom line:** the render-and-file *feasibility* question the PoC set out to answer is
answered **yes** — the primitives, the seam, and the byte-preservation save all work, and a good
chunk of the follow-on's code already exists here. The follow-on's real work is **app
integration and ship-quality robustness**, not proving the charts can be drawn.
