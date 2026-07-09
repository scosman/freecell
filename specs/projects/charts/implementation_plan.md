---
status: draft
---

# Implementation Plan: Charts (production)

Rebuilt into **small, CR-sized phases** — each one coherent goal, one clean commit,
reviewable in a sitting. Risk-ordered: **line chart proven end-to-end (component → app →
live → save) before any other type**, with a **blocking human review/tuning checkpoint**
after it. v1 ships after the fidelity/robustness phases; authoring + editing follow as their
own phases (v1 can ship first).

Refs: `functional_spec.md`, `ui_design.md`, `architecture.md`. Reusable PoC assets:
`SYNTHESIS.md §5`; fidelity targets: `ooxml-coverage-matrix.md`. Each phase's detail lives in
those docs — this is the ordered checklist.

---

## Foundation

- [ ] **P1 — Crate scaffolding & placement.** Move PoC crates into homes by charter
  (`chart-model`→core/dedicated crate; file layer→`freecell-engine`; render→`freecell-app`).
  *Exit:* workspace compiles; PoC unit tests pass; zero behavior change for non-chart files.
- [ ] **P2 — Chart data model.** Widen `chart-model` to the OOXML-bounded typed shape; add
  `ChartSpec` (retained `source`, `ranges`, `anchor`, `origin`). *Exit:* model + `ChartSpec`
  with unit tests; nothing renders differently yet.
- [ ] **P3 — Derived fidelity accessor.** `display_fidelity()` (Faithful/Degraded/Unsupported)
  over model+source; 3D→2D normalization; curated "render-affecting unsupported" set.
  *Exit:* pure-logic unit tests (3D⇒Degraded, surface/radar⇒Unsupported).
- [ ] **P4 — Render-test harness.** Lift the capture harness into `render-tests` (headless
  `xvfb`+lavapipe+`xrefresh`; perceptual-diff; container prereqs). *Exit:* one PoC scene
  renders headless + diffs green in CI.

## Line chart — isolated component (no app yet)

- [ ] **P5 — Line renderer.** Production line component from `chart-model` (chrome, nice-tick
  numeric axes, multi-series on one shared scale, legend, title, axis titles). *Exit:* unit
  tests + committed render-test baselines; runs in the test harness, not the app.
- [ ] **P6 — Line P1 fidelity.** Theme colors (`schemeClr`), rotated vertical value-axis title,
  `numFmt` ticks, markers, `smooth`. *Exit:* updated render-test baselines; reviewer sees a
  *real* line chart.

## Engine — load

- [ ] **P7 — Chart load.** `freecell-engine` `discover_and_parse` walks sheet→drawing→chart,
  parses `chartN.xml` → `ChartSpec` (line fixtures) into the P2 model + retained source.
  *Exit:* headless unit tests parse a real line-chart `.xlsx`; no UI.

## App integration — line chart end-to-end

- [ ] **P8 — Render line chart in the spreadsheet.** `ChartLayer` over cells: anchor→pixel via
  grid geometry, clip to viewport, scroll/zoom with the sheet, cull off-screen; corner badge on
  `Degraded`, placeholder on `Unsupported`. Cache values (static). *Exit:* opening a real file
  shows its line chart in place, correctly positioned.
- [ ] **P9 — Live binding.** Parse `c:f`; range→chart index; on recompute, re-resolve the dirty
  charts and publish via the worker seam. *Exit:* editing a source cell re-renders the line
  chart; only intersecting charts recompute.
- [ ] **P10 — Save / restore (source-first).** Byte-preserve unedited; **patch retained source**
  on reflow; multi-sheet part map; fail loudly on missing part. *Exit:* open→edit→save→reopen
  keeps the line chart in **Excel + LibreOffice**; untouched charts bit-stable.
- [ ] **P11 — Line perf + baselines.** Lazy parse off open's critical path; off-screen free;
  coalesced dirty-set recompute. *Exit:* p50/p99 first-paint / edit-rerender / scroll-with-K
  measured vs targets; committed perceptual-diff baselines.

## 🚦 CHECKPOINT — human review & tuning  *(BLOCKING)*
Human review of the whole line slice on real files (render vs Excel, in-grid behavior,
live feel, badge/placeholder, save/restore in both apps, perf); budgeted **tuning** pass;
GO/loop/re-plan decision. **No type phase starts until this passes.**

## New graph types — one phase each (render + type fidelity + render-tests; reuse anchor/bind/save)

- [ ] **P12 — Column & bar.** Both orientations; clustered/stacked/100%; `gapWidth`/`overlap`;
  **Excel horizontal-bar category order**. *Exit:* renders/binds/saves in-grid; baselines.
- [ ] **P13 — Area.** Standard/stacked/100% (hand-rolled polygon fork). *Exit:* same bar.
- [ ] **P14 — Pie & doughnut.** `c:dPt` per-slice colors + `varyColors`; `holeSize`;
  rotation/explosion; on-slice % labels. *Exit:* same bar.
- [ ] **P15 — Scatter.** Two numeric axes + dots; `scatterStyle`. *Exit:* same bar.
- [ ] **P16 — Bubble.** Scatter + `bubbleSize`→radius (√-area + clamp). *Exit:* same bar.

## Fidelity & robustness → v1 ships

- [ ] **P17 — Data labels & number formats.** Cross-cutting `c:dLbls`
  (val/percent/cat/legendKey) + `numFmt`, shared by all types. *Exit:* baselines per type.
- [ ] **P18 — Axis & fill breadth.** `scaling` min/max, reversed, gridline toggles; gradient
  fills, `a:ln` stroke, alpha; legend positions. *Exit:* baselines; unsupported set curated so
  handled features drop their warning.
- [ ] **P19 — Robustness on real files.** Real Excel/LibreOffice corpus loads without breakage;
  correct Degraded/Unsupported; edge cases (unresolved `c:f`, empty ranges, row/col shifts).
  *Exit:* corpus green; open never breaks.
- [ ] **P20 — Regression + external round-trip CI + perf hardening.** Perceptual-diff suite
  green across all types (+ badge/placeholder); save→reopen in Excel + LibreOffice in CI;
  many-charts/large-series perf. *Exit:* **v1 SHIPPABLE** (display + preserve).

## Authoring — Stage A  *(end-phase; v1 can ship first)*

- [ ] **P21 — Write path (component design + impl).** Design doc for the write path + edit
  panel, then **write-from-model** (authored) + **source-patch** (edited). *Exit:* a
  model-built chart serializes to a valid `.xlsx` reopenable in Excel + LibreOffice; round-trip
  tests.
- [ ] **P22 — Insert flow.** Action-bar chart-icon menu (type glyphs) → insert a near-empty
  chart of that type → it appears in the grid. *Exit:* insert a line chart via the UI; it
  renders + saves.
- [ ] **P23 — Manipulate.** Select (outline + handles), move, resize, delete on the ChartLayer.
  *Exit:* manipulation persists to the anchor and round-trips.
- [ ] **P24 — Edit panel + range/type.** Right-docked panel skeleton; set data **range** and
  chart **type**. *Exit:* a near-empty inserted chart can be shaped into a real one.

## Editing — Stage B  *(end-phase)*

- [ ] **P25 — Chrome editing.** Title, legend on/off + position, axis titles, series colors,
  data-label toggles via the panel. *Exit:* chrome edits apply live + round-trip; edit contract
  (patch preserves unmodeled styling) holds.

---

### Why this order
Line chart goes **component → engine load → in-grid → live → save → perf → checkpoint**
before any other type, so the one-time integration risks (anchor mapping, live binding,
source-first save, perf) are paid once and *reviewed* before replication. After the
checkpoint, each type is a small, independent CR on the proven pipeline. The one genuinely new
subsystem — the write path — is isolated to P21, gating authoring/editing only.
