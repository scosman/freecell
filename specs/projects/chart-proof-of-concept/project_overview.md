---
status: complete
---

# Chart Proof of Concept

A **proof-of-concept** project to reach a **go/no-go decision** on adding charts to
FreeCell. This is **not** shipping code — it is a set of `experiments/` variations that
de-risk the key unknowns cheaply. If the PoC succeeds, a *separate* project will bring
chart support up to ship quality.

## Guardrails

- **All work lives in `experiments/`** (a new `experiments/chart-poc/` group). **Nothing
  in `/app`.** No app integration, no interactive UI.
- **Relaxed rigor** (it's a PoC, optimize for speed to a decision): fewer tests, lighter
  code review (keep *structural* review — does it actually work / prove the point — but
  relax on form/polish), move fast.
- **PNG capture is enough** for validation — we render examples offscreen to images; we do
  not wire charts into the real app.
- **Scope = at most what we can build on `gpui-component`'s `plot/` primitives.** We reuse
  gpui-component heavily for core rendering; we are not writing a charting engine from
  scratch.

## The three experiments

1. **Load/save data PoC.** Can we extract chart definitions from an `.xlsx` alongside
   IronCalc's load, and stitch a save path that re-injects them? IronCalc reads/writes no
   chart data (see `research/ironcalc-chart-exposure.md`), so this is our own read + write
   pass over the zip, sitting beside the IronCalc-owned model. Hopefully a quick PoC.

2. **Our own chart component** (the make-or-break). A FreeCell chart widget built on
   gpui-component's `plot/` pieces, validating the key unknowns that the stock chart
   structs lack: **titles, axis titles, multi-series, grouped/stacked, legend, numeric
   axes.** We reuse a ton for core rendering but **own the legend / title / wrapper / axis
   labeling**. Its **data model is designed to match the OOXML (`c:`) chart data model**,
   so it is the seam between experiment 1 (parse into it) and this component (render from
   it).

3. **Our own scatter plot**, inside the same chart component. Reuse axis / title / legend;
   the new part is two numeric axes + drawing dots (`c:xVal`/`c:yVal`). Research flagged
   scatter as the highest-value type gpui-component *cannot* do — this PoC tests whether
   "just draw some dots" on the primitives is actually reasonable.

**Punting the long tail:** stock/candlestick, combo, bubble, radar, surface, 3D,
pie-of-pie, multi-ring doughnut, and the extended `cx:` family are out of the PoC.

## Validation: screenshots + agentic image review

On top of (light) tests, the chart component ships a **CLI or test** that **screenshots
each chart-type example to PNG**, and an **agent eyeballs each image** to judge whether it
renders correctly. The PoC ends with a **written assessment** per chart type/variation.
(The repo already has a headless gpui→PNG capture path under `app/render-tests/` and
`experiments/round-3/C-ci-rendering/` to build on.)

## Early-bail

The plan is **ordered so dealbreakers surface first** and we can **stop early**. Example:
if we cannot render a **multi-series line chart with a title, axis labels, and a legend**
well, we do not need to grind through six more failing examples — that is a no-go signal
on its own.

## Prior research

Full research (Excel/OOXML data model, gpui-component capability audit, IronCalc exposure,
per-type comparisons, scope & gaps) lives in [`research/`](research/). Key takeaways that
set up this PoC:

- **IronCalc exposes zero chart data** — we roll our own read (and, for this PoC, write)
  pass over the `.xlsx` zip; chart XML carries cached values so display needs no eval.
- **gpui-component has two levels:** the 5 chart *structs* (single-series, no legend / no
  numeric axis / no real multi-series) vs. the `plot/` **primitives** (`Line`/`Bar`/`Area`/
  `Arc`/`Pie`/`Stack` + scales) we can build richer charts on. This PoC lives at the
  primitives level.
- **The common real chart is multi-series with a legend and a readable numeric axis** —
  exactly what the stock structs lack, which is why experiment 2 is the make-or-break.
