---
status: draft
---

# Implementation Plan: Charts (production)

Risk-ordered. The governing idea (your guidance): **de-risk the whole pipeline on ONE type —
line — end to end, then stop for a human review/tuning checkpoint**, and only then grind out
the remaining types. The scariest work is *app integration + live binding + source-first
save + perf*, none of which the PoC touched — so line charts carry all of it first.

**Shape:** Phases 0–4 build the **shippable v1 core** (display + live + preserve). Phases 5–6
are the **authoring/editing extensions** — planned here, but v1 can ship before they land.

Refs: `functional_spec.md`, `ui_design.md`, `architecture.md`. Reusable PoC assets:
`SYNTHESIS.md §5`. Fidelity targets: `ooxml-coverage-matrix.md`.

Legend: each phase lists **Steps** and an **Exit**.

---

## Phase 0 — Foundation & placement  *(enabling; lowest risk)*
Lift proven PoC code into the real crates; establish the production data model.

**Steps**
- [ ] Place **`chart-model`** in `freecell-core` (or a dedicated `freecell-chart-model` crate)
  — the gpui-free seam.
- [ ] Widen the model to **OOXML-shaped + bounded** (typed fields for what we render/edit) and
  add the **retained `source` XML** + `origin` to `ChartSpec` (architecture §3).
- [ ] Implement the **derived fidelity accessor** `display_fidelity()` (Faithful / Degraded /
  Unsupported) over model+source, with the curated "render-affecting unsupported" set + the
  3D→2D normalization (architecture §3.3).
- [ ] Move the **file layer** (`load`/`xlsx`/`save`) into `freecell-engine` beside
  `open_fixups.rs`; expose `discover_and_parse → Vec<ChartSpec>`.
- [ ] Move the **render layer** (chrome, palette, ticks, stacking, `chart_element`) into
  `freecell-app`; stand up the **chart render-test suite** in `render-tests` (lift the capture
  harness; provision the `SYNTHESIS §4.4` container prereqs).

**Exit:** PoC crates compile in the app workspace; existing PoC scenes still render (baseline
captured); `display_fidelity()` unit-tested (3D→2D ⇒ Degraded, surface/radar ⇒ Unsupported);
no `/app` behavior change for non-chart files.

---

## Phase 1 — Line chart, END TO END  *(the risk-front-loaded vertical slice)*
One type, every hard axis. Ordered to hit the biggest integration risk first.

**Steps**
- [ ] **1a — Render in the grid at anchor + fidelity UI.** Parse the drawing `anchor` → pixel
  rect via the grid coordinate system; paint a `ChartLayer` over cells, clipped, scrolling with
  the sheet, culling off-screen. Wire the **corner "⚠ May not display as intended" badge** on
  `Degraded` and the **placeholder** on `Unsupported`. Load a **real** line-chart `.xlsx` and
  see it in place.
- [ ] **1b — Live binding.** Parse each series' `c:f`; build the range→chart index; on IronCalc
  recompute, re-resolve dirty line charts and republish via the worker publication seam;
  editing a source cell re-renders. Cache = first-paint / fallback.
- [ ] **1c — Save / restore (source-first).** Save→reopen preserves the line chart (Excel +
  LibreOffice), multi-sheet mapping; unedited → byte-preserve source; edited → **patch the
  retained source** (reflow `numCache`, keep unmodeled styling); untouched charts bit-stable.
- [ ] **1d — Line-relevant P1 fidelity.** Series colors incl. **theme colors** (`schemeClr`);
  **rotated** vertical value-axis title; number-formatted ticks (`c:numFmt`); markers +
  `c:smooth`; data labels if present. (A reviewer should see a *real* line chart.)
- [ ] **1e — Performance + regression baseline.** Lazy parse off the open path; off-screen
  free; dirty-set recompute coalesced; measure **p50/p99** first-paint, edit re-render, scroll
  frame time with K line charts (repo bench convention). Commit perceptual-diff baselines.

**Exit:** a real line chart opens → renders in-grid at its anchor (with badge/placeholder as
apt) → updates live on edit → survives save/reopen in Excel + LibreOffice → meets agreed perf
targets. Driven end-to-end, not in a harness.

---

## 🚦 CHECKPOINT — human review & tuning  *(BLOCKING — we stop here)*
The project's central de-risking gate. **Do not start Phase 2 until this passes.**

- **Human review** of the line-chart slice on real files: rendering vs Excel, in-grid behavior
  (scroll/zoom/clip/z-order), live-update feel, the fidelity badge/placeholder, save/restore in
  Excel + LibreOffice, and perf vs targets.
- **Tuning** (expected, budgeted): colors/spacing/tick density/anchor-mapping/axis-title
  rotation/perf knobs.
- **Decision:** GO to grind remaining types on the proven pipeline; loop on tuning; or re-plan
  if an integration/perf wall appears.

**Exit criteria (all hold):** human-accepted line quality; perf targets met; save/restore
verified in both external apps; live binding correct; the pipeline
(anchor→render→bind→save-patch) judged sound to replicate.

---

## Phase 2 — Remaining types on the proven pipeline
Each type: lift its PoC renderer into the `ChartLayer` dispatch + its type-specific fidelity;
reuse anchor/binding/source-save unchanged. Ordered by prevalence/ROI.

**Steps** (each: render in-grid + live + save + type fidelity + regression baseline)
- [ ] **Column / bar** (both orientations; clustered/stacked/100%): grouped/stacked slot math;
  **fix horizontal-bar category order to Excel's** (bottom-up); `gapWidth`/`overlap`.
- [ ] **Area** (standard/stacked/100%): the hand-rolled stacked polygon fork.
- [ ] **Pie / doughnut:** **`c:dPt` per-slice colors + `varyColors`** (P1 coloring crux);
  `holeSize`; rotation/explosion; on-slice % labels.
- [ ] **Scatter:** two numeric axes + dots; `scatterStyle`.
- [ ] **Bubble:** scatter + `bubbleSize`→radius (√-area + clamp) — the analysis-only type, now built.

**Exit:** every in-scope type renders/binds/saves at the checkpoint bar; per-type
perceptual-diff baselines committed.

---

## Phase 3 — OOXML fidelity sweep (coverage-matrix P2)
**Steps**
- [ ] Data labels breadth (`showVal/showPercent/showCatName/showLegendKey`, `numFmt`, position).
- [ ] Axis breadth: explicit min/max (`c:scaling`), reversed (`orientation`), gridline toggles.
- [ ] Fills breadth: gradient fills; `a:ln` stroke width/color; alpha.
- [ ] Legend position (`t/b/l/r/tr`); `autoTitleDeleted`; title rich-text basics.
- [ ] **Curate the `display_fidelity()` unsupported set** as features move into support (each
  implemented feature drops out ⇒ its warning auto-clears).

**Exit:** the coverage-matrix P1+P2 set for in-scope types is honored or explicitly deferred
(a deferred P2 leaves the chart `Degraded` with the honest warning — never silently wrong).

---

## Phase 4 — Robustness & production hardening → **v1 SHIPPABLE**
**Steps**
- [ ] **Real-file corpus** (Excel + LibreOffice-authored) — load without breakage; correct
  Degraded/Unsupported classification; workbook open never breaks (PoC #10/#11).
- [ ] **External round-trip CI** — save→reopen in Excel + LibreOffice across the corpus.
- [ ] **Perceptual-diff regression suite** green across all types/variations (incl. badge +
  placeholder).
- [ ] **Perf hardening** — many charts/sheet, large-series down-sampling, huge-sheet scroll;
  p50/p99 re-measured.
- [ ] Edge cases (functional_spec §7): unresolved `c:f`, empty/non-numeric ranges, row/col
  insert-delete shifting anchors, multi-sheet remap failing loudly.

**Exit:** **v1 ships** — all in-scope types, live, source-faithful save, robust on real files,
perf-guarded, honest fidelity warnings.

---

## Phase 5 — Authoring, Stage A  *(end-phase extension; v1 can ship first)*
First user-authoring cut (functional_spec §6.A, ui_design §3–§4).

**Steps**
- [ ] **Component design** for the **write path** (template-synthesizer for authored charts +
  source-patcher for edited-loaded) and the **edit panel** — written at the start of this phase
  (architecture deferred it here).
- [ ] **Write-from-model** on save for `Authored` charts (synthesize `chartN.xml` + drawing +
  anchor + rels + content-types); **source-patch** on save for edited-loaded charts.
- [ ] **Insert flow:** action-bar chart-icon menu (type glyphs) → insert a **near-empty** chart
  of that type → open its **right-docked edit panel**.
- [ ] **Manipulate:** select (outline + handles), move, resize, delete on the `ChartLayer`.
- [ ] **Edit panel (structural):** set data **range** + chart **type**; the panel's detail is
  specced in the component-design step above.

**Exit:** insert / move / resize / delete / change-type / re-range produce valid charts that
render, live-bind, and round-trip (Excel + LibreOffice).

---

## Phase 6 — Editing, Stage B  *(end-phase extension)*
Chrome editing in the edit panel (functional_spec §6.B).

**Steps**
- [ ] Edit **title**, **legend** on/off + position, **axis titles**, **series colors**,
  **data-label** toggles — applied via the model, persisted via source-patch / synthesize.

**Exit:** chrome edits apply live and round-trip; the edit contract (patch preserves unmodeled
styling) holds.

---

### Ordering rationale
Phase 1 front-loads **all** unproven ship risk (in-grid render, live binding, source-first
save, perf) onto the simplest type, so the checkpoint validates the *pipeline*, not a picture.
Types (Phase 2) are then near-mechanical lifts onto that pipeline; fidelity (3) and robustness
(4) harden it to a **shippable v1**. Authoring/editing (5–6) build on the same seam — the
write path is the one genuinely new subsystem, and it's isolated to those end phases.
