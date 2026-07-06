# Bubble — in-scope by analysis (not a rendered gate)

**Status:** IN for the follow-on. **Decided by code inspection, not by rendering an
example.** Bubble was never drawn or agent-reviewed — this note records *why* that was
unnecessary, and what the (small) residual risk is.

## What a bubble chart is

A bubble chart (`c:bubbleChart`) is a **scatter chart with a third value per point**
(`c:bubbleSize`) that sets the **marker radius**. X and Y are still two numeric axes; the
only new visual variable is dot size. (See `research/excel-chart-data-model.md §4e` and
`research/scope-and-gaps.md`, which rated bubble Low-prevalence but noted it "needs scatter
first, plus sized markers.")

## Why it's in by analysis

Gate 3 already **PASSED** for scatter — both single- and multi-series (`SYNTHESIS.md §1`,
`chart-render/results/scatter_single.png`, `scatter_multi.png`). Bubble is a strict, tiny
generalization of that exact render path:

- **The dot mark is already a sized circle.** `chart-render/src/scatter.rs` draws each point
  as a filled circle via `Window::paint_quad` with `radius = DOT_SIZE / 2.0` — a **constant**
  today. Bubble makes that radius a **per-point function of the size value**. Nothing else
  about the mark changes.
- **Everything around the mark is already built and validated.** Two shared `ScaleLinear`
  axes, the nice-tick numeric axes, axis titles, chart title, the multi-series color cycle,
  and the legend are all reused verbatim from scatter — each already PASS in Gate 3 / Gate 1.
- **The data model already carries the pair.** `SeriesData::Xy { x, y }` mirrors
  `c:xVal`/`c:yVal`; bubble adds one parallel `size` vector (`c:bubbleSize`). No structural
  change — an added optional field plus a `ChartKind::Bubble`.

So the only genuinely new code is: *map a size value to a radius.* There is no new axis
system, no new layout, no new interaction — the things that made scatter itself the
"cheapest new type of the whole PoC" (`SYNTHESIS.md §3`) apply doubly here, because scatter
already did the hard part.

## The exact delta (for the follow-on)

- **`chart-model`:** add `ChartKind::Bubble`; add `size: Option<Vec<f64>>` to
  `SeriesData::Xy` (`None` = plain scatter); add a `Series::bubble(name, x, y, size)`
  constructor.
- **`chart-render`:** in the scatter dot loop, replace the constant `radius` with a
  **√-scaled** size→radius mapping (area ∝ value — the Excel convention, `c:sizeRepresents`
  defaults to area) and a **max-radius clamp** so a large value can't swamp the plot.
- **scene:** one `bubble_single` example. Legend/title/axes need no new work.

## Residual risk (why this is safe to decide without rendering)

The only things a render would have exercised that inspection can't fully settle are
**cosmetic tuning**, not feasibility:

- the size→radius **constant** (how big the biggest bubble is) — a one-line tuning knob;
- **overlap/occlusion legibility** when large bubbles cover small ones — the standard bubble
  trade-off, mitigated by the same thin light outline scatter already draws per dot, plus
  draw-largest-first ordering if needed.

Neither is a go/no-go unknown; both are polish for the ship-quality follow-on. If a visual
confirmation is ever wanted, it is a ~30-minute add (the scene + one capture + one review)
on top of the existing harness — but it would only be confirming the obvious.

## Conclusion

Bubble is **IN** for the follow-on ship-quality project, at effectively the same near-zero
incremental cost as scatter. It does **not** change the PoC's **GO** verdict or the 16/16
rendered-image tally — it is an analysis-based scope addition layered on the passed scatter
gate.
