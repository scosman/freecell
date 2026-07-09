---
status: draft
---

# Implementation Plan: Charts (production)

> ⚠️ **STRAWMAN — not yet reviewed.** Written ahead of process during a batch drift; parked
> as raw input pending the step-gated `/spec new_project` review (the last step). Do **not**
> treat as vetted or settled.

Risk-ordered. The governing idea (your guidance): **de-risk the whole pipeline on ONE type —
line — end to end, then stop for a human review/tuning checkpoint**, and only then grind out
the remaining types. The scariest work is *app integration + live binding + save/reflow +
perf*, none of which the PoC touched — so line charts carry all of it first, on the smallest
possible surface.

Details: `functional_spec.md`, `architecture.md`. Reusable PoC assets: `SYNTHESIS.md §5`.
Fidelity targets: `ooxml-coverage-matrix.md`.

Legend: each phase lists **Steps** and an **Exit** (what must be true to move on).

---

## Phase 0 — Foundation & placement  *(enabling; lowest risk)*
Lift the proven PoC code into the real crates without behavior change, and stand up the
in-app render path so Phase 1 starts from a working build.

**Steps**
- [ ] Place **`chart-model`** in `freecell-core` (gpui-free seam) — move as-is; keep tests.
- [ ] Move the **file layer** (`load.rs`/`xlsx.rs`/`save.rs`) into `freecell-engine`, beside
  `open_fixups.rs`; expose a `discover + parse charts` API returning `ChartSpec`s.
- [ ] Move the **render layer** (chrome, palette, ticks, stacking, `chart_element`) into
  `freecell-app` as a module; drop the standalone capture bins.
- [ ] Stand up the **chart render-test suite** in `render-tests` (lift the capture harness;
  provision the container prereqs from `SYNTHESIS §4.4`).
- [ ] Prove the wiring: render one hand-built line `Chart` in a dev/debug view in the app
  build (not yet grid-integrated) → non-blank frame.

**Exit:** all PoC crates compile inside the app workspace; the existing PoC scenes still
render (regression baseline captured); no `/app` behavior change for non-chart files.

---

## Phase 1 — Line chart, END TO END  *(the risk-front-loaded vertical slice)*
One type, every hard axis. Ordered to hit the biggest integration risk first.

**Steps**
- [ ] **1a — Render in the grid at anchor (cache values).** Parse the drawing `anchor`
  (from/to cell + EMU offsets); map to a pixel rect via the grid's coordinate system; paint a
  `ChartLayer` over cells, clipped to the viewport, scrolling/zooming with the sheet; cull
  off-screen. Load a **real Excel-authored** line-chart `.xlsx` and see it in place.
- [ ] **1b — Live binding.** Parse each series' `c:f` range; build the range→chart map; on
  IronCalc recompute, re-resolve the dirty line charts and republish via the worker
  publication seam; editing a source cell re-renders the line. Cache = first-paint/fallback.
- [ ] **1c — Save / restore + reflow.** Save→reopen preserves the line chart (Excel +
  LibreOffice), multi-sheet mapping; **edit-reflow** refreshes its `numCache` from current
  cells; untouched charts bit-stable.
- [ ] **1d — Line-relevant fidelity to production quality.** Series colors incl. **theme
  colors** (`schemeClr`+tint); **rotated** vertical value-axis title; number-formatted axis
  ticks (`c:numFmt`); markers + `c:smooth`; data labels if present. (Enough that a reviewer
  judges a *real* line chart, not a stripped one.)
- [ ] **1e — Performance pass + regression baseline.** Lazy parse off the open path;
  off-screen free; dirty-set recompute coalesced; measure **p50/p99** first-paint, edit
  re-render, and scroll frame time with K line charts (foreground `timeout`, forced+asserted,
  env-stamped). Commit perceptual-diff baselines for the line scenes.

**Exit:** a real line chart opens → renders in-grid at its anchor → updates live on edit →
survives save/reopen in Excel + LibreOffice → meets agreed perf targets. Fully driven
end-to-end, not in a harness.

---

## 🚦 CHECKPOINT — human review & tuning  *(BLOCKING — we stop here)*
The project's central de-risking gate. **Do not start Phase 2 until this passes.**

- **Human review** of the line-chart vertical slice on real files: rendering quality (vs
  Excel side-by-side), in-grid behavior (scroll/zoom/clip/z-order), live-update feel,
  save/restore correctness in Excel + LibreOffice, and the perf numbers vs targets.
- **Tuning** pass: colors/spacing/tick density/anchor-mapping/axis-title rotation/perf knobs —
  whatever the review surfaces. This is expected, budgeted work, not a failure.
- **Decision:** GO to grind the remaining types on the now-proven pipeline; or loop on tuning;
  or, if an integration/perf wall is found, re-plan before spending on 6 more types.

**Exit criteria (all must hold):** human-accepted line render quality; perf targets met;
save/restore verified in both external apps; live binding correct under edits; the pipeline
(anchor→render→bind→save→reflow) judged sound to replicate.

---

## Phase 2 — Remaining types on the proven pipeline
Each type = lift its PoC renderer into the `ChartLayer` dispatch + its type-specific fidelity;
reuse anchor/binding/save unchanged. Ordered by prevalence/ROI.

**Steps** (each: render in-grid + live + save-reflow + type fidelity + regression baseline)
- [ ] **Column / bar** (both orientations; clustered/stacked/100%): grouped/stacked slot math
  from PoC; **fix horizontal-bar category order to Excel's** (bottom-up); `gapWidth`/`overlap`.
- [ ] **Area** (standard/stacked/100%): the hand-rolled stacked polygon fork from PoC.
- [ ] **Pie / doughnut:** **`c:dPt` per-slice colors + `varyColors`** (the P1 coloring crux —
  honor the file's slice colors, else synthesized palette); `holeSize`; rotation/explosion;
  on-slice % labels.
- [ ] **Scatter:** two numeric axes + dots; `scatterStyle`.
- [ ] **Bubble:** scatter + `bubbleSize`→radius (√-area scale + clamp) — the analysis-only
  type, now actually built (`bubble-analysis.md`).

**Exit:** every in-scope type renders/binds/saves at the checkpoint bar; per-type
perceptual-diff baselines committed.

---

## Phase 3 — OOXML fidelity sweep (coverage-matrix P2)
The important-but-not-P1 features, once all types render.
**Steps**
- [ ] Data labels breadth (`showVal/showPercent/showCatName/showLegendKey`, `numFmt`, position).
- [ ] Axis breadth: explicit min/max (`c:scaling`), reversed (`orientation`), gridline toggles.
- [ ] Fills breadth: gradient fills; `a:ln` stroke width/color; alpha.
- [ ] Legend position (all of `t/b/l/r/tr`); `autoTitleDeleted`; title rich-text basics.

**Exit:** the coverage-matrix P1+P2 set for in-scope types is honored or explicitly deferred
with a note.

---

## Phase 4 — Robustness & production hardening
**Steps**
- [ ] **Real-file corpus** (Excel + LibreOffice-authored) — load without breakage; graceful
  placeholder for out-of-scope/malformed charts; workbook open never breaks (PoC #10/#11).
- [ ] **External round-trip CI** — save→reopen in Excel + LibreOffice across the corpus.
- [ ] **Perceptual-diff regression suite** green in CI across all types/variations.
- [ ] **Perf hardening** — many-charts-on-a-sheet, large-series down-sampling, huge-sheet
  scroll with charts; p50/p99 re-measured vs targets.
- [ ] Edge cases (functional_spec §4): unresolved `c:f`, empty/non-numeric ranges, row/col
  insert-delete shifting anchors, multi-sheet part-order remap failing loudly.

**Exit:** v1 acceptance — all in-scope types, live, save-faithful, robust on real files,
perf-guarded. Ship candidate.

---

## Phase 5 — Interaction  *(DEFERRED — post-v1, not in scope for this project's core)*
Select / move / resize / delete chart objects in the grid. Placed here for sequencing only;
it is a **named fast-follow**, explicitly out of v1 (functional_spec §2/§6). Do not start
before Phase 4 ships unless re-scoped.

---

### Ordering rationale
Phase 1 deliberately front-loads **all** the unproven ship risk (in-grid render, live
binding, save-reflow, perf) onto the single simplest type, so the checkpoint validates the
*pipeline*, not just a picture. Types (Phase 2) are then near-mechanical lifts of proven PoC
renderers onto that pipeline; fidelity (Phase 3) and robustness (Phase 4) harden what works.
If Phase 1 hits a wall, we learn it after ~1 type of cost, not 7.
