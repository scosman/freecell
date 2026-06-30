---
status: complete
---

# Functional Spec: FreeCell — Phase 1 (Technical De-risking)

## 1. Purpose

Phase 1 is a **research and de-risking phase**, not a product build. No FreeCell
app exists yet and none is built here. The goal is to remove the major technical
uncertainties behind the product vision (a GPU-rendered, Rust, Excel-compatible
spreadsheet that is "stupid fast" on huge sheets) so we can make an informed
**go / no-go decision at Stage 3**.

Concretely, Phase 1 answers: *Is **Formualizer + GPUI** (or some better-ranked
alternative) a stack we can confidently build FreeCell on?* — backed by
reproducible evidence, not vibes.

All Phase 1 work lives in an **`experiments/`** folder at the repo root. Each
sub-project produces a written findings document and, where relevant, runnable
code (benchmarks or a proof-of-concept) plus recorded results.

## 2. Scope

### In scope
- Six research / experiment sub-projects (Section 6).
- An `experiments/` tree containing: findings docs, runnable Rust benchmarks,
  a throwaway GPUI proof-of-concept grid, and recorded results.
- A final **Phase 1 synthesis**: go/no-go recommendation + a proposed Round-2
  exploration list.

### Out of scope (explicitly, for Phase 1)
- Building any part of the real FreeCell application or its production UI.
- Implementing Excel feature coverage, formula functions, or a formatting model
  beyond what's needed to *validate feasibility*.
- Production-grade file import/export, error handling, or persistence.
- Polished UI/UX, menus, dark-mode theming, accessibility (the PoC is a
  perf test rig, deliberately rough but not "ugly as sin").
- Cross-platform packaging/distribution.

### Deliberately deferred
- Anything tagged for "Round 2" by the final synthesis doc.

## 3. Environment & Division of Labor

This is a hard constraint that shapes every sub-project.

| Work type | Where it runs | Who drives |
|-----------|---------------|------------|
| All research / writing | Anywhere | Agent |
| UI-less Rust (engine, file, perf, memory benchmarks) | Headless Linux / CI / container | Agent, autonomously |
| GPUI proof-of-concept (the macOS app, both purposes) | **macOS (Metal)** | Agent writes it + build scripts; **human (you) runs it locally** and reports the logged results |

Rules baked into the spec:
- **Every sub-project except the UI Technical Test is pure, UI-less Rust** that
  the agent can build and run headlessly.
- **The UI Technical Test is a single macOS (Metal) app** serving two purposes
  (Sub-project E): (1) an **interactive scrolling app** for the human visual/feel
  test, and (2) a **"Run Test" menu item** that runs scripted scroll / fast-scroll
  / jump-to-cell sequences, **measures** frame timings and cell-load latencies,
  logs the results, and **applies measured pass/fail gates** — numbers, not vibes.
- There is **no software-rendering path.** All authoritative UI performance comes
  from the real-GPU macOS run; the agent builds the app and its measurement
  harness, the human runs it and reports the logged pass/fail output.

## 4. Gating & Sequencing

The stack decision **gates** the rest of Phase 1 (per explicit decision).

1. **Gate 0 — Stack decision (Sub-project A).** Research + a preliminary hands-on
   smoke test of Formualizer (does it build, load a file, evaluate?), evaluate
   alternatives, produce a **ranked recommendation**. → **Human sign-off required**
   before further experiments begin.
   - If we confirm Formualizer + GPUI (or an explicitly chosen alternative), proceed.
   - If we pivot, the downstream sub-projects are re-scoped to the chosen stack
     before continuing.
2. After sign-off, the remaining sub-projects (B–F) proceed; the UI Technical Test
   (F) can run in parallel with the engine-side work (B–E) since they share little.
3. **Phase 1 synthesis (G)** runs last and consumes all findings.

## 5. Cross-Cutting Conventions

### 5.1 `experiments/` layout
```
experiments/
  README.md                      # index of sub-projects + how to run everything
  00-stack-decision/
    findings.md
    smoke/                       # minimal hands-on Formualizer smoke test (Cargo crate)
  01-file-support/
    findings.md
    <crate(s)>
  02-datamodel-binding-perf/
    findings.md
    <bench crate(s)>
    results/                     # recorded benchmark output (committed)
  03-formatting/
    findings.md
    <crate(s)>
  04-ui-poc/
    findings.md
    raw-gpui/                    # PoC variant on raw gpui
    gpui-component/              # PoC variant on gpui-component
    scripts/                     # macOS build/run (one command)
    results/                     # logged pass/fail from the app's "Run Test"
  05-round-2-proposal/
    round_2_explorations.md
  SYNTHESIS.md                   # go/no-go + Round 2 pointer
```
(Exact crate names/structure are an architecture-step detail; this is the shape.)

### 5.2 Findings document standard
Each sub-project's `findings.md` must contain, at minimum:
- **Question(s)** being answered.
- **What was done** (approach, code pointers, commands to reproduce).
- **Results / evidence** (numbers with their environment, screenshots/PNGs where
  relevant).
- **Conclusion** — a direct answer, including "we couldn't determine X because Y."
- **Recommended design** + **next-best alternative** (where the brief asks for a
  proposal).
- **Risks / open questions** carried forward.

### 5.3 Benchmark / evidence standard
- Benchmarks are reproducible from a single documented command.
- Use a real benchmarking harness (e.g. Criterion) for micro/throughput numbers;
  report p50/p99 where latency distribution matters, not just means.
- Inputs (large synthetic sheets, sample files) are generated by committed code,
  not hand-made binaries, so anyone can regenerate them.
- Every number records: environment (CPU/OS/rendering path), input size, and date.
- Compare **at least two designs/approaches** wherever the brief calls for it
  (binding patterns, raw-gpui vs gpui-component), and report the winner with why.

### 5.4 Performance targets ("Excel-max & buttery")
These are the **goals we measure toward**. Phase 1 passes if we hit them *or*
establish a credible, evidenced path to them — not necessarily on first try.

| Dimension | Target |
|-----------|--------|
| Grid size (PoC + benchmarks) | Excel max: **1,048,576 rows × 16,384 cols** |
| Scroll *render* frame (macOS/Metal, authoritative) | Sustain **120 fps** (~8.3 ms/frame) while scrolling; never worse than 60 fps under fast scroll/jump |
| Load newly-visible cells (while scrolling) | Pull values+formatting for cells entering the viewport (~viewport + overscan, order 10³–10⁴ cells) in **< ~2 ms**, repeatedly, while panning (must fit inside a frame) |
| Dependency cascade recompute (engine, **not** a frame rate) | **1,000,000-cell** linear chain (`=PREV+1`); edit the head cell → full recompute in **< 100 ms**; report recalc latency p50/p99 |
| "Change cascade → visible update" | Edit a cell that cascades (incl. cross-sheet / offscreen) → visible-cell values refreshed within one frame budget |
| File load | Open a **100 MB+** `.xlsx`; record load time and peak memory (target: seconds, not minutes; memory a sane multiple of file size) |
| Memory | Load a large workbook (order 10⁷ populated cells) + edit; stays within a reasonable RAM envelope (measure & report; flag if it balloons) |

> **On the numbers (the three are independent budgets):** the **120 fps / ~8.3 ms**
> figure is the *render frame* target while scrolling. The **< ~2 ms** figure is the
> cost of *loading the values + formatting for cells newly entering the viewport* as
> you scroll — it must fit comfortably inside one frame. The **< 100 ms** figure is a
> *one-off engine recalculation* triggered by an edit: a 1M-cell dependency chain must
> recompute fast after you change its head. **< 100 ms is a recalc latency, not a frame
> rate** (it is not "10 fps").

Numbers that are "measure and report" (file load time, memory envelope) are
**discovery metrics**: we record them and judge reasonableness, rather than
pass/fail against a pre-fixed threshold.

### 5.5 What "Phase 1 success" means
A clear, evidence-backed answer to: **can we build FreeCell on this stack and hit
the bar?** The output is a recommendation (go / go-with-changes / no-go / pivot),
not a finished app. A well-evidenced "no" or "pivot" is a successful Phase 1.

## 6. Sub-Projects

Each sub-project below lists its **Questions**, **Approach**, **Deliverables**,
and **Pass criteria**.

### A. Stack Decision — "Challenge the design direction"  *(GATING)*
**Questions.** Is Formualizer + GPUI a great base stack? What alternatives exist
(engine and UI layers, separately and as combos)? What are the risks of each
(maturity, function coverage, file fidelity, license, maintenance/bus-factor,
performance ceiling, platform support)?

**Approach.**
- Web research on the engine landscape (Formualizer and alternatives) and the
  GPU/native UI landscape (GPUI and alternatives), using web agents.
- Hands-on **smoke test** of Formualizer: add it as a dependency, build it in
  this environment, load a small `.xlsx`/CSV, evaluate a formula, mutate a cell.
  Just enough to confirm the API and that it's real and usable.
- Assess GPUI's viability as a standalone dependency (it's coupled to Zed;
  evaluate `gpui-component` as the practical component layer).
- Hypothesize 2–4 alternative stacks; rank with reasoning.

**Deliverables.** `00-stack-decision/findings.md` with a **ranked
recommendation** and explicit risks; a minimal smoke-test crate.

**Pass criteria.** A defensible ranked recommendation that you can sign off on
(or use to choose a pivot). Gate cleared by human decision.

### B. File Support
**Questions.** Can Formualizer read/write modern `.xlsx` and CSV? Round-trip
(load → edit → save) fidelity? If not native, what's our plan (e.g. `calamine` /
`rust_xlsxwriter` / `umya-spreadsheet`, or build it)?

**Approach.** Programmatically generate and/or use sample files; load, inspect
values & formulas, mutate, save, reload, diff. Test CSV import/export. Probe
edge cases: formulas, multiple sheets, large files, number formats, shared
strings, dates.

**Deliverables.** `01-file-support/findings.md`; a load/save round-trip test
crate; a recommended design + next-best alternative for file I/O.

**Pass criteria.** Demonstrated load + save of modern `.xlsx` and CSV with known
round-trip behavior documented (what survives, what doesn't), and a credible plan
for anything missing.

### C. Datamodel Binding & Engine Performance
This is the core technical risk: the **engine ↔ UI binding** drives perf/scale.

**Questions.**
- *Writes:* is `set_value` as simple/cheap as it looks? (Challenge it — batching,
  locking, recalc triggering.)
- *Reads / binding:* how do we pull all values for the current viewport as we
  scroll fast, and update as data changes? Per-cell `evaluate_cell` vs range
  APIs? Parallelism? How do we subscribe to updates for visible cells? Is caching
  needed, is it internal, and how is invalidation handled?
- *Arrow:* how does the Apache Arrow backing model affect access patterns and
  the ideal load/subscribe design? (See Formualizer "Large Workbook Performance".)
- *General engine perf:* are cascades fast? (e.g. 1M-cell `=PREV+1` chain; plus
  a few more propagation shapes — wide fan-out, cross-sheet, volatile functions.)
- *Memory:* load a large workbook + edit — is RAM reasonable?

**Approach.**
- Design ≥2 candidate **access/binding patterns** (e.g. naive per-cell pull;
  range/bulk pull; pull + subscription/dirty-tracking cache) and benchmark them.
- **Scrolling read benchmark:** simulate rapid viewport movement across a huge
  sheet; measure per-viewport read latency (target < ~2 ms).
- **Change-cascade → visible-update benchmark:** edit a cell that cascades
  (including via offscreen / cross-sheet cells), then fetch the now-visible
  values; measure end-to-end latency.
- **Cascade/propagation benchmarks:** the 1M-cell chain (< 100 ms) plus
  additional propagation shapes.
- **Memory benchmark:** load order-10⁷-cell workbook, edit, measure peak RSS.
- Iterate in a research loop; compare designs; pick a recommended binding design.

**Deliverables.** `02-datamodel-binding-perf/findings.md`; benchmark crate(s)
with committed `results/`; a **recommended binding-layer design** + next-best
alternative; the answer to "what other perf-critical areas should we validate?"

**Pass criteria.** Evidence that a viable binding pattern hits the read/cascade
targets (or a credible path to them), a clear recommended design, and reasonable
memory behavior — all reproducible.

### D. Formatting — Research & Pre-validation
**Questions.** Does Formualizer (or its underlying XLSX engine) expose formatting
(row/col sizes, bold/italic, fills/lines, font size, number formats)? Does it
offer format/metadata storage on the same Arrow backend, or must we build our own
formatting model? If we load → edit formatting → save, is that easy or hard?

**Approach.** Inspect Formualizer's API/source for formatting & metadata; test
reading formatting from a styled `.xlsx`; test whether edits survive a save;
prototype a minimal external formatting store if native support is absent.

**Deliverables.** `03-formatting/findings.md` with a **recommended formatting
design** (native vs side-table vs custom Arrow-backed store) + next-best
alternative, and the load→edit→save verdict.

**Pass criteria.** A clear picture of what formatting info is available and a
credible, evidenced design for FreeCell's formatting model and its persistence.

### E. UI Technical Test — GPUI Proof-of-Concept  *(only UI sub-project)*
**Questions.** Can GPUI render a giant spreadsheet grid crazy fast? How does raw
`gpui` compare to `gpui-component` for this? Does it hit the perf bar — *measured*?

**It's a single macOS app serving two purposes:**
1. **Interactive scrolling app** — you scroll/jump around to judge speed and feel
   (the visual test).
2. **"Run Test" menu item** — runs scripted scroll / fast-scroll / jump-to-cell
   sequences, **measures** per-frame render timings and newly-visible-cell load
   latency, **logs results to a file**, and **reports measured pass/fail** against
   the Section 5.4 targets. Measured, not vibes.

**Look & feel (deliberately minimal — it's a PoC).** White background, grey
gridlines; cells show text with borders, plus highlighting (fills), bold, and
italic; column/row headers; **variable row/column widths**. Make it look enough
like a spreadsheet to get a real feel — don't sweat minor details. The agent
drives the remaining visual choices.

**Approach.**
- **Static datamodel provider**: code returning values + formatting per cell — a
  reasonable proxy for a big, difficult sheet; **no real engine connected**.
- Size: target the Excel-max grid (Section 5.4).
- Build **two variants — raw `gpui` and `gpui-component` — and compare** perf &
  ergonomics.
- The **"Run Test" harness lives in the app**; pass/fail gates derive from measured
  numbers (frame time vs the 120 fps budget; cell-load latency vs the ~2 ms target).
- **macOS build/run scripts** so you can pull and run with one command, then report
  the logged results.
- Optional: render-correctness sanity via screenshots / known-good PNGs (a
  foretaste of the product's rendering-test strategy).
- Iterate on perf in a research loop.

**Deliverables.** `04-ui-poc/` with both variants, macOS `scripts/`, logged
`results/`, and `findings.md` comparing the two + the **measured** verdict against
the perf bar.

**Pass criteria.** The in-app "Run Test" reports **measured pass** against the
Section 5.4 frame-time and cell-load targets at the target scale on macOS/Metal,
the human confirms it scrolls and feels smooth, and there's a clear
raw-vs-`gpui-component` recommendation.

### F. Round-2 Technical Exploration Proposal
**Questions.** Given Phase 1 findings, what should we de-risk next?

**Approach.** Synthesize gaps/risks surfaced across A–E into a concrete, ordered
list of follow-up explorations.

**Deliverables.** `05-round-2-proposal/round_2_explorations.md` — a ranked list
with rationale per item.

**Pass criteria.** An actionable Round-2 list grounded in Phase 1 evidence.

### G. Phase 1 Synthesis  *(final)*
**Deliverable.** `experiments/SYNTHESIS.md`: a go / go-with-changes / no-go /
pivot recommendation for building FreeCell on the chosen stack, citing the
evidence from A–E, plus a pointer to the Round-2 list (F). This is the artifact
that feeds the Stage 3 decision.

## 7. Edge Cases, Risks & What Could Invalidate the Approach

- **Formualizer maturity.** It's young; Excel compatibility is the product's
  headline feature and rests on it. Function coverage (320+ vs Excel's ~500),
  file fidelity, and maintenance/bus-factor are real risks — stress-tested in
  Sub-projects A/B/D.
- **GPUI as a dependency.** Coupled to Zed, sparse docs, moving target, primarily
  macOS/Linux. The raw-vs-`gpui-component` comparison (E) and the stack research
  (A) probe this.
- **UI perf is measured on a real GPU, by you.** There is no software-render proxy;
  the PoC's in-app "Run Test" harness produces the authoritative pass/fail numbers
  on macOS/Metal, and the agent depends on you to run it and report the log.
- **Network/build friction in-container.** Egress is policy-restricted and there's
  no GPU/display here; if a dependency can't be fetched/built headlessly, that's a
  finding to record, and the work routes to the macOS side or is flagged blocked.
- **Targets are goals, not guarantees.** The "120 fps / <100 ms" bar is what we
  measure toward; Phase 1's real job is to determine whether the bar is reachable
  on this stack, with evidence.
- **Scope creep.** The temptation is to start building FreeCell. Phase 1 stops at
  validation + recommendation.

## 8. Resolved Decisions
- **No formal UI Design step** (confirmed — it's a PoC). The only "UI" is the
  throwaway perf-test grid; its look is specified inline in Sub-project E (white
  background, grey gridlines; cells with text, borders, highlighting, bold, italic;
  variable row/col widths). The agent drives the remaining visual details.
- **Stack decision gates first; then sub-projects B–F run in parallel**, each in its
  own `experiments/` subfolder (Sections 4–5).
- **Implementation uses agent-swarm (nested sub-agents)** — verified available — so
  the parallel sub-projects can be driven concurrently. (If that capability were
  unavailable, the sub-projects would run serially instead.)
