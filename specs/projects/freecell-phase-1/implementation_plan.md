---
status: complete
---

# Implementation Plan: FreeCell — Phase 1 (Technical De-risking)

Ordered build plan. Details live in `functional_spec.md` and `architecture.md`.
Orchestration, the parallel-editor isolation rule, and review gating are defined in
architecture §2. **Parallel phases run in their own git worktrees** and merge back
(disjoint folders). Detailed per-sub-project design is written into each phase plan.

## Gate outcome (2026-07-01)

Human decision at the Phase 1 stack gate: **proceed.** **UI = GPUI** — *settled*, no
UI competitor (still validated by the PoC). **Engine = undecided → full bake-off:**
validate **Formualizer** and **IronCalc** the same way, head-to-head, and choose the
engine at a new decision phase. Formualizer + GPUI is the working assumption; the
bake-off may overturn the *engine* only.

## Phases

- [x] **Phase 0 — Scaffolding** *(serial)*: `experiments/` skeleton, `shared/datagen`
  + `shared/bench_util` lib crates, README. `shared/` frozen (read-only downstream).

- [x] **Phase 1 — Stack Decision GATE (Sub-project A)** *(serial; gating)*:
  Formualizer smoke test (real API surface captured) + engine/UI alternatives
  research → `00-stack-decision/findings.md`. **Gate cleared** → proceed; GPUI
  settled; engine bake-off added below.

*After the gate, Phases 2–6 run in parallel (each in its own worktree/folder). Every
engine-dependent phase evaluates **both engines** head-to-head using the shared
`datagen`/`bench_util` harness and identical scenarios, and emits a comparison
`findings.md` covering **API suitability, missing/needed features, perf, and
fidelity**. Engine work lives in per-engine subfolders (`.../formualizer/`,
`.../ironcalc/`) so parallel editors stay isolated.*

- [x] **Phase 2 — Engine bake-off: Datamodel Binding & Perf (Sub-project C →
  `02-datamodel-binding-perf/`)** *(parallel; own review + sign-off — risky)*: shared
  engine-abstraction + benchmark harness; run **Formualizer and IronCalc** through
  binding designs D1/D2/D3 and the scrolling-read, cascade→visible, 1M-cell cascade,
  write, and memory benchmarks. Captures each engine's **binding API surface**
  (IronCalc mirrors the Phase 1 Formualizer smoke). Output: per-engine numbers +
  head-to-head comparison + recommended binding design. In-container authoritative.

- [x] **Phase 3 — Engine bake-off: File Support (Sub-project B → `01-file-support/`)**
  *(parallel; batched review)*: **both engines** — xlsx/CSV load→edit→save round-trip,
  fidelity, missing features, API. Head-to-head comparison + recommendation.

- [x] **Phase 4 — Engine bake-off: Formatting (Sub-project D → `03-formatting/`)**
  *(parallel; batched review)*: **both engines** — styles/metadata exposure
  (read + write + roundtrip) and storage-model design. A key differentiator
  (Formualizer's `CellData` styles gap vs IronCalc's styled xlsx r/w). Head-to-head
  comparison + recommendation.

- [x] **Phase 5 — UI PoC (Sub-project E → `04-ui-poc/`)** *(parallel; own review +
  Mac UI sign-off)*: **engine-neutral, GPUI only** (no engine). raw-gpui vs
  gpui-component grid variants over the static datamodel provider; in-app "Run Test"
  measured PASS/FAIL harness; macOS build scripts. Authoritative on macOS (you run it).

- [x] **Phase 6 — Round-2 Proposal (Sub-project F → `05-round-2-proposal/`)**
  *(parallel; batched review)*: ranked follow-up explorations (incl. the captured
  style read→write roundtrip item).

*After the engine-dependent phases (2–4) land:*

- [x] **Phase 7 — Engine Bake-off Decision (Sub-project G → `06-engine-bakeoff/`)** — **DECIDED (2026-07-01): IronCalc.** (UI grid also decided: raw-gpui, with gpui-component for app chrome.)
  *(own review + HUMAN engine sign-off)*: pull data from Phases 2–4 (+ Phase 1) into
  `06-engine-bakeoff/decision.md`; make the **case for Formualizer** and the **case
  for IronCalc** across API suitability, missing features, perf, file fidelity,
  formatting, and maturity/bus-factor/license; give a **recommendation**.
  **→ HUMAN SIGN-OFF on the engine choice.**

*After everything lands:*

- [ ] **Phase 8 — Final Synthesis (Sub-project H → `experiments/SYNTHESIS.md`)**
  *(serial; last)*: overall go / go-with-changes / no-go / pivot for FreeCell
  (incorporating the engine decision) + Round-2 pointer. Feeds the Stage 3 decision.
