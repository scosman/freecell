---
status: complete
---

# Implementation Plan: Charts (production)

Rebuilt into **small, CR-sized phases** — each one coherent goal, one clean commit,
reviewable in a sitting. Risk-ordered: **line chart proven *and hardened to production
quality*** first, with a **blocking human review/tuning checkpoint** after the line slice.
**Then authoring + editing (insert / drag / resize / edit) land on the line chart — the *real
minimal shipping version* — and v1 ships.** Only after that MVP is proven end-to-end do we go
wide on breadth: each new chart type then slots onto the hardened, now-editable pipeline.
(Rationale: no reason to invest in five more renderers before we know the authoring/editing path
has no blockers.)

Refs: `functional_spec.md`, `ui_design.md`, `architecture.md`. Reusable PoC assets:
`SYNTHESIS.md §5`; fidelity targets: `ooxml-coverage-matrix.md`. Each phase's detail lives in
those docs — this is the ordered checklist.

---

## Foundation

- [x] **P1 — Crate scaffolding & placement.** Move PoC crates into homes by charter
  (`chart-model`→core/dedicated crate; file layer→`freecell-engine`; render→`freecell-app`).
  *Exit:* workspace compiles; PoC unit tests pass; zero behavior change for non-chart files.
- [x] **P2 — Chart data model.** Widen `chart-model` to the OOXML-bounded typed shape; add
  `ChartSpec` (retained `source`, `ranges`, `anchor`, `origin`). *Exit:* model + `ChartSpec`
  with unit tests; nothing renders differently yet.
- [x] **P3 — Derived fidelity accessor.** `display_fidelity()` (Faithful/Degraded/Unsupported)
  over model+source; 3D→2D normalization; curated "render-affecting unsupported" set.
  *Exit:* pure-logic unit tests (3D⇒Degraded, surface/radar⇒Unsupported).
- [x] **P4 — Render-test harness.** Lift the capture harness into `render-tests` (headless
  `xvfb`+lavapipe+`xrefresh`; perceptual-diff; container prereqs). *Exit:* one PoC scene
  renders headless + diffs green in CI.

## Line chart — isolated component (no app yet)

- [x] **P5 — Line renderer.** Production line component from `chart-model` (chrome, nice-tick
  numeric axes, multi-series on one shared scale, legend, title, axis titles). *Exit:* unit
  tests + committed render-test baselines; runs in the test harness, not the app.
- [x] **P6 — Line P1 fidelity.** Theme colors (`schemeClr`), rotated vertical value-axis title,
  `numFmt` ticks, markers, `smooth`. *Exit:* updated render-test baselines; reviewer sees a
  *real* line chart.

## Engine — load

- [x] **P7 — Chart load.** `freecell-engine` `discover_and_parse` walks sheet→drawing→chart,
  parses `chartN.xml` → `ChartSpec` (line fixtures) into the P2 model + retained source.
  *Exit:* headless unit tests parse a real line-chart `.xlsx`; no UI.

## App integration — line chart end-to-end

- [x] **P8 — Render line chart in the spreadsheet.** `ChartLayer` over cells: anchor→pixel,
  clip, scroll/zoom, cull; corner badge on `Degraded`, placeholder on `Unsupported`. Cache
  values (static). *Exit:* opening a real file shows its line chart in place.
- [x] **P9 — Live binding.** Parse `c:f`; range→chart index; re-resolve dirty charts on
  recompute and publish via the worker seam. *Exit:* editing a source cell re-renders the line
  chart; only intersecting charts recompute.
- [x] **P10 — Save / restore (source-first).** Byte-preserve unedited; **patch retained source**
  on reflow; multi-sheet part map; fail loudly on missing part. *Exit:* open→edit→save→reopen
  keeps the line chart in **Excel + LibreOffice**; untouched charts bit-stable.
- [x] **P11 — Line perf + baselines.** Lazy parse off open's critical path; off-screen free;
  coalesced dirty-set recompute. *Exit:* p50/p99 first-paint / edit-rerender / scroll-with-K
  measured vs targets; committed perceptual-diff baselines.

## 🚦 CHECKPOINT — human review & tuning  *(BLOCKING)*
Human review of the whole line slice on real files (render vs Excel, in-grid behavior,
live feel, badge/placeholder, save/restore in both apps, perf); budgeted **tuning** pass;
GO/loop/re-plan decision. **No hardening or type phase starts until this passes.**

## Harden the line chart to production quality  *(before authoring)*
Build the cross-cutting fidelity + robustness + CI machinery **proven on line first**; each is
reusable infra the types then inherit. (Type-specific fills/labels land with each type.)

- [x] **P12 — Data labels & number formats.** `c:dLbls` (val/percent/cat/legendKey) + `numFmt`,
  exercised on line. *Exit:* baselines; the accessor's unsupported set shrinks accordingly.
- [x] **P13 — Axis breadth & line styling.** `scaling` min/max, reversed, gridline toggles;
  `a:ln` stroke width/color; alpha; legend positions. *Exit:* baselines.
- [x] **P14 — Robustness on real files (line + graceful degrade).** A real Excel/LibreOffice
  corpus loads without breakage; line charts render; every *other* type degrades to
  placeholder/warning cleanly; edge cases (unresolved `c:f`, empty ranges, row/col shifts).
  *Exit:* corpus green; workbook open never breaks.
- [x] **P15 — Regression + external round-trip CI + line perf hardening.** Perceptual-diff suite
  + save→reopen (Excel + LibreOffice) wired into CI for line; many-line-charts/large-series
  perf. *Exit:* **a production-robust line chart** — the pipeline (render→fidelity→robust→CI) is
  proven end-to-end on one type.

## Authoring — Stage A  *(the MVP: insert / drag / resize / edit the line chart)*
The real minimal shipping version. Basic authoring + editing land **on the hardened line chart**
before any breadth — a blocker here must surface before we've built five more renderers. The one
genuinely new subsystem, the write path, is isolated to P16.

- [x] **P16 — Write path (component design + impl).** Design doc for the write path + edit
  panel, then **write-from-model** (authored) + **source-patch** (edited). *Exit:* a
  model-built chart serializes to a valid `.xlsx` reopenable in Excel + LibreOffice; round-trip
  tests.
- [x] **P17 — Insert flow.** Action-bar chart-icon menu (type glyphs) → insert a near-empty
  chart of that type → it appears in the grid. *Exit:* insert a line chart via the UI; it
  renders + saves.
- [x] **P18 — Manipulate.** Select (outline + handles), move, resize, delete on the ChartLayer.
  *Exit:* manipulation persists to the anchor and round-trips.
- [x] **P19 — Edit panel + range/type.** Right-docked panel skeleton; set data **range** and
  chart **type**. *Exit:* a near-empty inserted chart can be shaped into a real one.

## Editing — Stage B  *(chrome editing — completes the MVP)*

- [x] **P20 — Chrome editing.** Title, legend on/off + position, axis titles, series colors,
  data-label toggles via the panel. *Exit:* chrome edits apply live + round-trip; the edit
  contract (patch preserves unmodeled styling) holds.

## v1 MVP ship gate

- [x] **P21 — MVP sweep → v1 ships.** Full perceptual-diff + external round-trip suites green for
  the **line chart** across display + live + preserve + **authoring/editing** (insert / move /
  resize / delete / chrome edits) + badge/placeholder; many-charts / huge-sheet perf re-measured.
  *Exit:* **v1 SHIPPABLE** — the line chart, fully authorable & editable. Breadth (other types)
  follows in the next batch.

## New graph types — one phase each  *(breadth, post-MVP — each slots onto the hardened, editable pipeline)*
Each: renderer + type fidelity + reuse anchor/bind/save/**author/edit** + its own regression
baselines + round-trip. Ordered by prevalence/ROI; the final type re-runs the full cross-type
perceptual-diff + external round-trip sweep.

- [x] **P22 — Column & bar.** Both orientations; clustered/stacked/100%; `gapWidth`/`overlap`;
  **Excel horizontal-bar category order**; per-type fills.
- [ ] **P23 — Area.** Standard/stacked/100% (hand-rolled polygon fork); fills.
- [ ] **P24 — Pie & doughnut.** `c:dPt` per-slice colors + `varyColors`; `holeSize`;
  rotation/explosion; on-slice % labels.
- [ ] **P25 — Scatter.** Two numeric axes + dots; `scatterStyle`.
- [ ] **P26 — Bubble.** Scatter + `bubbleSize`→radius (√-area + clamp); the final type re-runs
  the full cross-type perceptual-diff + external round-trip sweep.

---

### Why this order
Line chart goes **component → engine load → in-grid → live → save → perf → checkpoint → full
hardening (fidelity + robustness + CI)** before anything else — so the *entire* production
pipeline, not just rendering, is proven and reviewed on one type. **Authoring + editing (P16–P20)
then land on that hardened line chart — the write path (P16) is the one genuinely new subsystem —
and P21 ships v1 as a fully authorable/editable line-chart product.** Only then do we go wide:
each new type (P22–P26) is a small CR that inherits the hardened, editable machinery and re-runs
its regression + round-trip suites. Deferring breadth until *after* the MVP means a blocker in the
authoring/editing path surfaces before we've spent effort on five more renderers.
