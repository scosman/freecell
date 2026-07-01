---
status: draft
---

# Implementation Plan: FreeCell — Phase 2 (Round-2 Technical De-risking)

Ordered build plan. Details live in `functional_spec.md` (sub-projects SP1–SP7 +
pass criteria) and `architecture.md` (layout/reuse, orchestration, the SP1 async
design, methodology). Orchestration, the parallel-editor isolation rule, serialized
commits, and the off-ramp checkpoint are in architecture §2. **Parallel phases run in
their own git worktrees** and merge back (disjoint folders).

## Sequencing

Engine-risk cohort first (SP1–SP3) → **off-ramp checkpoint (human)** → build-out
cohort (SP4, SP5, SP7). SP6 (macOS, human-run) may proceed in parallel throughout.
Synthesis last.

## Phases

- [ ] **Phase 2.0 — Scaffolding** *(serial)*: create `experiments/round-2/`; build
  `round-2/harness/` = verbatim copy of `02/common` (SpreadsheetEngine trait +
  scenarios) + `02/ironcalc` (IronCalc adapter) as a **lib crate**, pinned to the
  Phase-1 IronCalc version; add the `peak_rss()` child-process helper. **Freeze
  `round-2/harness/` (read-only downstream).** (architecture §1, §3)

*After scaffolding, the engine-risk cohort runs in parallel (own worktree/folder):*

- [ ] **Phase SP1 — Recompute cost & async-recompute UX** *(parallel; own review +
  sign-off — highest risk)* → `round-2/01-recompute-async/`. Latency matrix (size ×
  DAG shape); `Model` `Send`/clone cost; the engine-actor async prototype. GATES:
  UI-thread per-edit < frame budget during in-flight recompute; debounce/supersede
  bounds `evaluate()` count. (functional_spec SP1; architecture §4)

- [ ] **Phase SP2 — Large styled `.xlsx` open** *(parallel; batched review)* →
  `round-2/02-xlsx-open/`. ≥100 MB styled-file generator; fresh-process open time +
  peak RSS + stage breakdown; time-to-first-paint. (functional_spec SP2)

- [ ] **Phase SP3 — Function-parity audit** *(parallel; own review — could reopen
  engine choice)* → `round-2/03-function-parity/`. Coverage diff vs canonical Excel
  list + golden-file correctness harness (≥~100 cases; typed error semantics).
  (functional_spec SP3)

- [ ] **⛳ OFF-RAMP CHECKPOINT (human review)**: present SP1–SP3 findings against the
  overview §2 off-ramp. Clean → proceed; triggered → surface for a human engine
  decision before further investment. (architecture §2.4)

*After the checkpoint, the build-out cohort runs in parallel:*

- [ ] **Phase SP4 — Binding layer + native style read at scale** *(parallel; batched
  review)* → `round-2/04-binding-style/`. Viewport read of **value + style** p99
  < 2 ms (GATE); style-API exposure probe (band/empty-cell — may reopen §2);
  cache-invalidation vs `UserModel` diff-list + correctness test. (functional_spec SP4)

- [ ] **Phase SP5 — Long-tail style-roundtrip fidelity** *(parallel; batched review)*
  → `round-2/05-style-fidelity/`. Comprehensive attribute matrix over an `.xlsx`
  round-trip; merges/CF recorded OPEN (not designed). (functional_spec SP5)

- [ ] **Phase SP7 — Residual gaps** *(parallel; batched review)* →
  `round-2/07-residual/`. CSV RFC-4180 hardening; IronCalc load-API ergonomics;
  storage-density extrapolation to Excel-max. (functional_spec SP7)

*Runs in parallel throughout; authoritative numbers require the human on a Mac:*

- [ ] **Phase SP6 — GPUI grid maturation** *(macOS, human-run; own review + Mac UI
  sign-off)* → `round-2/06-gpui-grid/` (evolves `04-ui-poc/raw-gpui`). Inline edit,
  selection ranges, frozen panes; in-app "Run Test" records the still-pending §5.4
  numbers; PNG-baseline render tests; **resolve GPL #55470** (verify via cargo tree).
  (functional_spec SP6; architecture §6)

*After everything lands:*

- [ ] **Phase Synthesis — Stage-3 recommendation** *(serial; last)* →
  `experiments/round-2/SYNTHESIS.md`. Build / adjust / pivot for FreeCell citing
  SP1–SP7; state whether the off-ramp triggered; list Round-3 carry-forward.
  (functional_spec §6 Synthesis)
