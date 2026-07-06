---
status: complete
---

# Functional Spec: Chart Proof of Concept

## 0. Purpose & framing

This spec defines a **proof-of-concept**, not a shippable feature. Its single job is to
produce a **go/no-go decision** (and a *shape* for a future ship-quality project) on chart
support in FreeCell, by building the cheapest experiments that resolve the real unknowns.

- **Deliverable of the whole project** = a **written assessment** (`SYNTHESIS.md` under
  `experiments/chart-poc/`) that says GO / NO-GO / PARTIAL-GO, backed by rendered PNGs and
  an agent's image review, plus a recommended scope for the follow-on project.
- **Everything lives in `experiments/chart-poc/`.** Nothing in `/app`. No app integration,
  no interactive UI, no persistence beyond writing example `.xlsx` and `.png` files.
- **Relaxed rigor.** Optimize for speed to the decision: light tests (enough to prove a
  claim, not to guard a product), structural code review only (does it work / prove the
  point — not form/style polish), and no perf/robustness bar beyond "renders a handful of
  example charts."

The research phase (`research/`) already settled *what* is worth trying. This PoC settles
*whether we can actually build it* on gpui-component's primitives, and *how hard* it is.

---

## 1. The core unknowns this PoC must resolve

Ranked by how badly a "no" hurts (this ranking drives the ordering in §6):

1. **Render quality on the primitives (make-or-break).** Can we, building on
   gpui-component's `plot/` primitives, produce a **multi-series chart with a title, axis
   titles, a readable numeric value-axis, and a legend** that a user would accept as a real
   chart of their data? The stock gpui chart structs cannot do this
   (`research/gpui-component-charts.md`); this is the whole bet.
2. **Harder layouts.** Do **grouped (clustered)** and **stacked / 100%-stacked** bar,
   **stacked area**, and **pie/doughnut with a sensible color palette** come out right?
   (Research flagged specific traps: `ScaleLinear` has no "nice" tick generation; the
   `Area` primitive has a scalar baseline so true stacked area needs an `Area` fork; pie
   has no auto-palette.)
3. **Scatter feasibility (and bubble as a cheap generalization).** Is a scatter plot (two
   numeric axes + dots), reusing the title/axis/legend scaffolding, actually "just draw
   some dots," or a swamp? Scatter is the highest-value type gpui-component can't do
   (`research/scope-and-gaps.md`). And once scatter works, is **bubble** (the same, with
   marker radius driven by a third value) essentially free, or does sizing/overlap make it
   more than a trivial add?
4. **Load/save stitching.** Can we (a) parse chart definitions out of a real `.xlsx`
   alongside IronCalc's load, and (b) write a valid `.xlsx` that still contains a chart
   after IronCalc's chart-dropping save path runs? (Lower risk — the zip second-pass
   pattern already exists in `open_fixups.rs` — but the save re-injection is unproven.)
5. **Headless validation loop.** Can we screenshot each example to PNG offscreen and have
   an agent reliably eyeball correctness? (Lowest risk — the capture pipeline already
   exists in `app/render-tests/` and `round-3/C-ci-rendering/`.)

---

## 2. Shared artifact: the OOXML-shaped chart data model

A small Rust data model that **mirrors the OOXML `c:` chart structure**
(`research/excel-chart-data-model.md`). It is the **seam** between Experiment 1 (parse
*into* it) and Experiment 2/3 (render *from* it), and is designed so the future
ship-quality project can keep it.

Functional shape (names indicative, not binding):

- `Chart { title: Option<String>, kind: ChartKind, series: Vec<Series>, cat_axis: Axis,
  val_axis: Axis, legend: Option<Legend> }`
- `ChartKind`:
  - `Bar { dir: Col | Bar, grouping: Clustered | Stacked | PercentStacked }`
  - `Line { grouping: Standard | Stacked | PercentStacked, smooth: bool }`
  - `Area { grouping: Standard | Stacked | PercentStacked }`
  - `Pie { doughnut_hole: Option<f32> }`
  - `Scatter { }` (uses `xy` series, below)
  - `Bubble { }` (uses `xy` series with `size`, below — scatter + a third value)
- `Series`:
  - category/value series (bar/line/area/pie): `{ name: Option<String>, categories:
    Vec<Category>, values: Vec<f64>, color: Option<Color> }` — mirrors `c:cat` / `c:val`.
  - xy series (scatter / bubble): `{ name, x: Vec<f64>, y: Vec<f64>, size:
    Option<Vec<f64>>, color }` — mirrors `c:xVal` / `c:yVal` (+ `c:bubbleSize` for bubble;
    `size` is `None` for plain scatter).
- `Axis { title: Option<String> }` (numeric formatting/scale details are the renderer's
  business for the PoC).
- `Legend { position }` (position may be ignored for the PoC; presence is what matters).

Values come from the chart XML's **cached** `<c:numCache>` / `<c:strCache>` so **no
formula evaluation is needed** to render.

This model lives in one place in `experiments/chart-poc/` and is depended on by the other
experiments. (Exact crate boundaries are an architecture detail.)

---

## 3. Experiment 2 — the chart component (make-or-break; specced first because it gates everything)

A FreeCell-owned chart widget rendered with gpui-component's `plot/` primitives
(`Line` / `Bar` / `Area` / `Arc` / `Pie` / `Stack` + scales + axis/grid/tooltip helpers).
We **reuse the primitives for the core marks** and **own the wrapper**: title, axis titles,
numeric value-axis with generated ticks, category axis, legend, and multi-series /
grouped / stacked layout.

**What it must render (from the data model, no app needed):**

| Variation | Why it's here | Known trap (from research) |
|---|---|---|
| Multi-series **line** (2–4 series) + title + axis titles + numeric axis + legend | **Gate 1 — the make-or-break example** | Each stock `LineChart` normalizes its own y-domain; multi-line needs raw `Line` on one shared `ScaleLinear`; must force straight segments (`.linear()`), Excel's default |
| Single-series **column** and **bar** (horizontal) | baseline sanity | none major |
| **Grouped (clustered)** column, multi-series | the single most common business chart | no grouped layout in structs; DIY over `Bar` + band scale |
| **Stacked** and **100%-stacked** column | common | DIY via `Stack` primitive |
| **Stacked area** | common; the nastiest layout | `Area` primitive has scalar baseline → needs an `Area` fork or hand-rolled polygons; percent needs a normalize pass |
| **Pie** and **doughnut** with a real palette | part-to-whole | no auto-palette → synthesize per-slice colors (theme accents / `c:dPt`) |

**Owned pieces the PoC must prove it can build well:** numeric value-axis with **readable,
"nice" tick labels** (the linear scale ships none); a **legend** (swatch + series name,
correct color mapping); **chart title + axis titles**; a **multi-series color cycle** over
`chart_1..chart_5` (extended if >5 series).

**Out (this experiment):** tooltips/hover/interactivity (static render only), animation,
zoom/pan, data labels beyond what's trivially reused, per-point style fidelity, exact
Excel pixel matching.

---

## 4. Experiment 3 — scatter (and bubble)

Add a scatter plot to the same component, **reusing** the title / axis-title / legend /
numeric-axis scaffolding from Experiment 2. The genuinely new part is a **second numeric
axis** (X becomes `ScaleLinear`, not a band scale) and **drawing point marks (dots)** from
`xVal`/`yVal` series.

**Must render:** a single-series scatter and a **multi-series** scatter (distinct colors +
legend) with both numeric axes labeled and a title.

**Question it answers:** is scatter a modest addition once the axis/legend scaffolding
exists, or does the category-vs-value orientation baked into the primitives make it
painful? The answer directly sets whether scatter is in-scope for the follow-on project.

### 4a. Bubble (small generalization of scatter)

A **bubble** chart is scatter with a **third value per point** (`c:bubbleSize`) that drives
the **marker radius**. It reuses the entire scatter render path (two numeric axes, legend,
title); the only new work is varying dot radius by a size series (with a size→radius scale
and a sensible max-radius clamp). It is validated **only if scatter passes** — if scatter is
a swamp, bubble is moot.

**Must render:** a single-series bubble chart (points sized by `size`) with both numeric
axes labeled, a title, and legibly differentiated bubble sizes.

**Question it answers:** is bubble genuinely "scatter + a radius" (near-free once scatter
works), or does marker sizing / overlap / occlusion make it more than a trivial add? Sets
whether bubble rides along with scatter in the follow-on scope.

**Out (scatter & bubble):** trendlines, log axes, `c:bubble3D`, area-vs-width size
semantics (`c:sizeRepresents`), negative-size handling — a fixed size→radius mapping is fine
for the PoC.

---

## 5. Experiment 1 — load/save data stitching

A read + write pass over the `.xlsx` zip, **beside** the IronCalc-owned model (IronCalc
reads/writes no chart data — `research/ironcalc-chart-exposure.md`).

- **Load:** open the zip (the second-pass pattern from `open_fixups.rs`), follow the
  worksheet → `xl/drawings/drawingN.xml` → `xl/charts/chartN.xml` relationship chain, and
  parse each chart into the §2 data model. **Prove it** by loading a **real Excel-authored
  `.xlsx`** containing at least a couple of the in-scope chart types and rendering them
  through Experiment 2.
- **Save:** produce a valid `.xlsx` that **still contains the chart** after IronCalc's
  (chart-dropping) writer runs. The PoC's minimum bar is **byte-preservation re-injection**
  — carry the original `xl/charts/*`, `xl/drawings/*`, their rels, and the worksheet
  `<drawing>` reference through into IronCalc's output zip so `open → save → reopen` (in
  Excel/LibreOffice or our own loader) still shows the chart. Writing a chart *from our
  data model* (synthesizing chart XML) is a **stretch goal**, noted if time allows.

**Question it answers:** is the load stitching as quick as hoped, and is save re-injection
tractable or a swamp? This sets whether the follow-on project can offer **display + save
preservation**, or **display-only** (charts dropped on save, as today).

**Out:** editing chart definitions, reflowing a chart when the underlying data changes,
adjusting cached values, chartsheets, the extended `cx:` family.

---

## 6. Validation: screenshots + agentic image review

On top of light unit tests, each rendered example is **captured to a PNG offscreen** and
**reviewed by an agent** that judges whether the image is a correct rendering.

- **Capture.** A **CLI / test binary** renders each example (from §3–§5) to a PNG headless,
  reusing the repo's proven path (`app/render-tests/src/capture.rs`: gpui window under
  `xvfb-run` + lavapipe, `xrefresh` to force presentation, capture by window id; and the
  `round-3/C-ci-rendering` render→PNG pipeline). Output PNGs are committed under the
  experiment's `results/`.
- **Agentic review.** An agent is given each PNG (and a one-line description of what it
  *should* show) and returns a per-example verdict — **PASS / MARGINAL / FAIL** + notes —
  against this rubric:
  1. Correct chart type & geometry (marks in the right places; grouped vs stacked correct).
  2. Multi-series distinguishable (distinct colors; correct grouping/stacking/overlay).
  3. Legend present, correct labels, correct series→color mapping.
  4. Title and axis titles present and legible.
  5. Numeric axis present with readable tick labels at sensible intervals.
  6. No clipping / overlap / garbage / blank output.
  7. **Overall:** would a user accept this as a real chart of their data?
- **Assessment.** The PoC ends with a written per-variation table (PASS/MARGINAL/FAIL +
  the agent's notes + the human's spot-check) feeding the go/no-go call.

The agent review is **advisory input to a human decision**, not an automated gate that
blocks. (Perceptual-diff-vs-baseline from `round-3` may be reused for *stability* checks
but is not required for the PoC.)

---

## 7. Ordering & early-bail gates

Ordered so the worst dealbreaker is hit first. **After each gate, stop and reassess; a
failed gate can end the PoC with a NO-GO or PARTIAL-GO — we do not grind through later
examples once a core capability is shown unachievable.**

- **M0 — Enablement.** Shared data model (§2) + the capture/agent-review harness (§6),
  proven on one trivial single-series bar (renders to a non-blank PNG; agent sees a bar
  chart).
- **Gate 1 — MAKE-OR-BREAK (§3).** Multi-series **line** with title + axis titles + numeric
  axis + legend, agent-judged **PASS**. **If FAIL → stop, write NO-GO.** (This is the
  user's stated bail example.)
- **Gate 2 — Harder layouts (§3).** Grouped column, stacked + 100%-stacked column, stacked
  area, pie/doughnut with palette. Some MARGINAL is acceptable; **wholesale FAIL of
  grouped/stacked → likely PARTIAL-GO** (e.g. "single-series only" recommendation).
- **Gate 3 — Scatter (§4).** Multi-series scatter, agent-judged. **FAIL → scatter recorded
  as out-of-scope for the follow-on**, PoC continues (not a whole-project NO-GO).
- **Gate 3b — Bubble (§4a).** Resolved **by code analysis** rather than a rendered example:
  because Gate 3 (scatter) passed and bubble is a strict, tiny generalization of that render
  path (a per-point marker radius from `c:bubbleSize`), bubble is recorded **IN**. Reasoning
  in [`experiments/chart-poc/bubble-analysis.md`](../../../experiments/chart-poc/bubble-analysis.md).
  (Originally planned as a small render gate; downgraded to analysis once the scatter code
  made the outcome unambiguous.)
- **Gate 4 — Load/save (§5).** Load a real `.xlsx` and render it; re-inject on save and
  reopen. Load FAIL is serious (there's no chart data without it); **save FAIL →
  display-only recommendation**, not a whole-project NO-GO.

Experiment 1 (load/save) is lower-risk and **may run in parallel** with Experiments 2/3;
but the **go/no-go pivot is Gate 1**, so the render component leads. Gate ordering is a
recommendation — confirm or reorder in review.

---

## 8. Out of scope (whole PoC)

- Chart types: **stock/candlestick, combo/multi-plot, radar, surface, all 3D,
  pie-of-pie, multi-ring doughnut**, and the entire extended `cx:` family (sunburst,
  treemap, waterfall, histogram, box-&-whisker, funnel, region map).
- Any `/app` integration, interactive UI, hover/tooltips, selection, live re-render on data
  change, editing charts, chart creation UI.
- Ship-quality concerns: performance, huge-sheet scaling, exhaustive style fidelity, exact
  Excel pixel parity, full test coverage, accessibility.
- Save beyond chart re-injection (no rewrite of IronCalc's writer; no reflow of cached
  values).

---

## 9. Success criteria (the go/no-go rubric)

The PoC is **successful as an experiment** if it produces a confident, evidence-backed
decision — a clear NO-GO is a successful outcome. The recommendation is one of:

- **GO** — Gate 1 passes and grouped/stacked/area/pie mostly PASS: a follow-on ship-quality
  project is justified with the full in-scope type set. Scatter, bubble, and
  save-preservation are in or out per Gates 3–4.
- **PARTIAL-GO** — the core renders but with a bounded limitation (e.g. single-series only;
  or display-only because save re-injection is intractable; or scatter out). Recommend the
  follow-on with that scope.
- **NO-GO** — Gate 1 fails: building acceptable charts on the primitives costs more than the
  value; recommend not pursuing (or revisiting the "use a different charting approach"
  option).

Each verdict is backed by: committed example PNGs, the agent review table (§6), a
`findings.md` per experiment, and a top-level `SYNTHESIS.md` with the recommendation and
the recommended scope + known risks for the follow-on project.

---

## 10. Resolved decisions (confirmed at review)

1. **Gate ordering (§7):** Experiment 1 (load/save) **may run in parallel** with the render
   component, but the **render component leads** — the go/no-go pivot is Gate 1.
2. **Save PoC bar (§5):** **byte-preservation re-injection is the accepted proof.**
   Synthesizing chart XML from our data model is a **stretch goal** only.
3. **Agentic review (§6):** **single agent verdict per image**, except the make-or-break
   **Gate 1 image gets a 3-agent panel** (majority) to reduce single-agent misjudgment.
4. **Load fixtures (§5):** **the agent authors the example `.xlsx` fixtures** (script /
   LibreOffice / a Rust xlsx writer); real-world files can be dropped in later.
