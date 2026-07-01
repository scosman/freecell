---
status: complete
---

# Implementation Plan: FreeCell — Phase 2 (Round-2 Technical De-risking)

Ordered build plan. Details live in `functional_spec.md` (experiments SP1–SP5 + pass
criteria) and `architecture.md` (layout/reuse, orchestration, the SP1 interop design
space, methodology). Orchestration, the parallel-editor isolation rule, serialized
commits, and the off-ramp checkpoint are in architecture §2. **Parallel phases run in
their own git worktrees** and merge back (disjoint folders). **Everything runs
in-container — no macOS dependency.**

## Sequencing

Engine-risk cohort first (SP1–SP3) → **off-ramp checkpoint (human)** → build-out
cohort (SP4, SP5) → synthesis.

## Phases

- [x] **Phase 2.0 — Scaffolding** *(serial)*: create `experiments/round-2/`; build
  `round-2/harness/` = verbatim copy of `02/common` (SpreadsheetEngine trait +
  scenarios) + `02/ironcalc` (IronCalc adapter) as a **lib crate**, pinned to the
  Phase-1 IronCalc version; add the `peak_rss()` child-process helper. **Freeze
  `round-2/harness/` (read-only downstream).** (architecture §1, §3)

*After scaffolding, the engine-risk cohort runs in parallel (own worktree/folder):*

- [x] **Phase SP1 — Non-blocking recompute & engine↔render interop seam** *(parallel;
  own review + sign-off — THE key experiment)* → `round-2/01-async-interop/`.
  Investigate IronCalc's eval lifecycle / `Send`-ability / read-during-eval /
  changed-cells-stream; measure `evaluate()` across sizes × DAG shapes; build a
  minimal non-blocking harness; **lock the interop-seam design**. GATES: render loop
  never blocks on recompute (per-tick < frame budget during a 10⁶–10⁷ eval);
  debounce/coalesce bounds eval count. (functional_spec SP1; architecture §4)

- [x] **Phase SP2 — Large styled `.xlsx` open** *(parallel; batched review)* →
  `round-2/02-xlsx-open/`. ≥100 MB styled-file generator; fresh-process open time +
  peak RSS + stage breakdown; time-to-first-paint. GATE: seconds, sane memory.
  (functional_spec SP2)

- [x] **Phase SP3 — Function-parity audit** *(parallel; own review — could reopen
  engine choice)* → `round-2/03-function-parity/`. Coverage diff vs a committed
  canonical Excel list + golden-file correctness harness (≥~100 cases; typed error
  semantics). (functional_spec SP3)

- [ ] **⛳ OFF-RAMP CHECKPOINT (human review)**: present SP1–SP3 findings against the
  overview §2 off-ramp. Clean → proceed; triggered → surface for a human engine
  decision before further investment. (architecture §2.4)

*After the checkpoint, the build-out cohort runs in parallel:*

- [ ] **Phase SP4 — Styled viewport read at scale + style-API coverage** *(parallel;
  batched review)* → `round-2/04-styled-read/`. Viewport read of **value + style**
  p99 < 2 ms (GATE); style-API exposure probe (band/empty-cell — a miss reopens the
  overview §2 formatting decision). (functional_spec SP4)

- [ ] **Phase SP5 — Long-tail style-roundtrip fidelity** *(parallel; batched review)*
  → `round-2/05-style-fidelity/`. Comprehensive attribute matrix over an `.xlsx`
  round-trip; merges/CF recorded OPEN (not designed). (functional_spec SP5)

*After everything lands:*

- [ ] **Phase Synthesis — Stage-3 recommendation** *(serial; last)* →
  `experiments/round-2/SYNTHESIS.md`. Build / adjust / pivot for FreeCell citing
  SP1–SP5; state whether the off-ramp triggered; list Round-3 carry-forward.
  (functional_spec §6 Synthesis)
