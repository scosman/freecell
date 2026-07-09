---
status: complete
---

# Charts

Bring chart support in FreeCell from **proof-of-concept to production quality**. The
[Chart Proof of Concept](../chart-proof-of-concept/) answered *"can we render `.xlsx`
charts on gpui-component's primitives, and preserve them through save?"* — the verdict was
**GO** (`chart-proof-of-concept/SYNTHESIS.md`). This project builds the real, shippable
feature on top of that proven core.

## What the PoC already gives us (do not rebuild)

Per `SYNTHESIS.md §5`, these are proven and lift-and-keep:

- **`chart-model`** — the gpui-free, OOXML-`c:`-shaped data model. The stable **seam**
  between the file layer and the render layer. Kept as the contract.
- **Render core** — shared chrome (title/axis-titles/legend), the categorical palette with
  by-construction legend↔mark mapping, the `NiceScale` tick generator, `stacking.rs`, and
  per-type renderers for line/bar/area/pie/doughnut/scatter built on gpui's `plot/`
  primitives.
- **File layer** — the `zip`+`roxmltree` load parser (namespace-agnostic; whole
  `c:ser → c:cat/c:val` family) and byte-preservation **save re-injection** (single-sheet
  case solved).
- **Validation harness** — headless render→PNG capture + agentic image review.

## What this project must actually build (the real ship risk — `SYNTHESIS.md §5`)

None of this was touched by the PoC (it was static-PNG-only, nothing in `/app`):

1. **App integration** — render charts *inside the real grid* (anchored position, scroll,
   z-order over cells), at production quality.
2. **Live data binding** — charts re-render as the user edits the underlying cells (resolve
   `c:f` ranges against IronCalc), not just from a stale cache.
3. **Performance** — many charts on a sheet, large series, huge-sheet scaling, on the
   scroll/edit path — FreeCell's whole reason for being.
4. **Save/restore at production fidelity** — beyond single-sheet byte-preservation:
   multi-sheet mapping, edit-reflow of cached values, real Excel/LibreOffice round-trips.
5. **OOXML fidelity** — the P1/P2 features the PoC's thin model skipped (per-point/`dPt`
   colors, data labels, number formats, axis scaling), per
   `chart-proof-of-concept/ooxml-coverage-matrix.md`.

## Scope decisions (confirmed at Step 1 review)

- **v1 core = DISPLAY + PRESERVE. Authoring + editing are in this project, as END phases.**
  The shippable v1 renders charts from opened `.xlsx` and keeps them correct through
  edit+save. **Authoring** (insert a chart, pick data range/type) **and editing** (change
  type, re-range, restyle an existing chart) are **later phases in this same project** —
  *extensions, not required to ship*: we may ship v1 (display + preserve) before they are
  complete, but they are planned here, not spun off.
- **Live binding.** Charts reflect *current* cell values and re-render on edit (cache is the
  initial paint / fallback). This subsumes the stale-cache problem.
- **Read-only, in-place for the v1 core.** Charts render at their anchor and scroll with the
  sheet; **select / move / resize / delete** and the authoring/editing UI arrive in the
  end-phase extensions above — *not* in the first cut. The line-chart checkpoint validates
  rendering, perf, and save/restore, not manipulation.
- **Type scope = the PoC's GO set:** line (first), then column/bar, area, pie/doughnut,
  scatter, bubble. **Out:** stock, combo, radar, surface, all 3D, ofPie, multi-ring
  doughnut, and the `cx:` extended family (per the coverage matrix).

## Approach (per your guidance)

- **Risk-aware order:** de-risk app integration + perf + live binding + save/restore on
  **one type end-to-end (line chart) first**, rather than building all types then
  discovering an integration wall.
- **Line chart end-to-end, then a human review/tuning checkpoint.** We complete line charts
  fully (load → live render in-grid → save/restore → perf), **stop**, and do a human
  review + tuning pass validating the *whole vertical slice* before grinding out the
  remaining types. That checkpoint is the project's central de-risking gate.
- **Then the rest of the types** ride the proven pipeline, followed by fidelity + robustness
  hardening — this completes the **shippable v1** (display + preserve).
- **Finally, end-phase extensions: authoring + editing** (create/insert charts, edit an
  existing chart's type/range/style, plus the select/move/resize interaction they require).
  Planned here, but v1 can ship before they land.

Details: `functional_spec.md`, `architecture.md`, and the phased `implementation_plan.md`
(all three are being (re)written under the step-gated review — currently strawman).
