---
status: complete
---

# Implementation Plan: FreeCell — Phase 1 (Technical De-risking)

Ordered build plan. Details live in `functional_spec.md` and `architecture.md`.
Orchestration, the parallel-editor isolation rule, and review gating are defined in
architecture §2. **Parallel phases run in their own git worktrees** and merge back
(disjoint folders). Detailed per-sub-project design is written into each phase plan
**after the gate**.

## Phases

- [x] **Phase 0 — Scaffolding** *(serial)*: create `experiments/` skeleton,
  `shared/datagen` + `shared/bench_util` lib crates, and `README.md`. Freeze
  `shared/` (read-only to later phases). Commit once.

- [ ] **Phase 1 — Stack Decision GATE (Sub-project A)** *(serial; gating)*:
  Formualizer smoke test (capture real API surface) + web research on engine/UI
  alternatives → `00-stack-decision/findings.md` with a ranked recommendation.
  **→ HUMAN SIGN-OFF (go / pivot) required before any phase below starts.** On
  pivot, re-scope phase plans to the chosen stack.

*After sign-off, Phases 2–4 run in parallel:*

- [ ] **Phase 2 — Datamodel Binding & Engine Perf (Sub-project C)** *(parallel;
  own review + sign-off)*: implement binding designs D1/D2/D3, run the scrolling-
  read, cascade→visible, 1M-cell cascade, write, and memory benchmarks; recommend
  a binding design. Authoritative numbers in-container.

- [ ] **Phase 3 — UI PoC (Sub-project E)** *(parallel; own review + UI sign-off on
  Mac)*: raw-gpui and gpui-component grid variants over the static datamodel
  provider; in-app "Run Test" measured PASS/FAIL harness; macOS build scripts.
  Authoritative numbers on macOS (you run it).

- [ ] **Phase 4 — File Support + Formatting + Round-2 (Sub-projects B, D, F)**
  *(parallel; ONE batched review + sign-off)*: B = xlsx/CSV load→edit→save
  round-trip + recommendation; D = formatting/metadata exposure + storage design;
  F = ranked Round-2 exploration list.

*After Phases 2–4 land:*

- [ ] **Phase 5 — Synthesis (Sub-project G)** *(serial; last)*: roll up all
  findings into `experiments/SYNTHESIS.md` — go / go-with-changes / no-go / pivot
  recommendation feeding the Stage 3 decision, plus the Round-2 pointer.
