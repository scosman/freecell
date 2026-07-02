---
status: complete
---

# Phase 7: Engine Bake-off Decision (Sub-project G)

## Overview

Sub-project G is a **synthesis / writing** phase, not an experiment. The Phase 1
gate (Sub-project A) settled **UI = GPUI** but left the **engine undecided**, so
Phases 2–4 ran the engine-dependent work as a **two-engine bake-off** (Formualizer
0.7.0 vs IronCalc 0.7.1) over the **same** `datagen` inputs, `bench_util` metrics,
and identical scenarios. This phase aggregates that committed evidence into
`experiments/06-engine-bakeoff/decision.md`: an explicit **case for Formualizer**,
an explicit **case for IronCalc**, and a **recommendation the human signs off on**
(functional_spec §6.G; architecture §1.1; implementation_plan Phase 7).

**No new benchmarks are run.** All numbers are cited from already-committed
findings. The one permitted piece of new, cheap evidence is a direct count of each
engine's registered function set in its crate source under `~/.cargo/registry`
(read-only), to put a defensible number on the otherwise-unquantified
function-coverage axis.

**This phase does not decide** — it produces a defensible, evidence-anchored
recommendation framed as *input for the human's engine sign-off*. The chosen engine
then flows into Sub-project H (`SYNTHESIS.md`, Phase 8).

## Evidence inputs (read-only)

- `experiments/00-stack-decision/findings.md` — Sub-project A: engine/UI research;
  Formualizer 0.7.0 API smoke; maturity / bus-factor / license; function-coverage
  claims (320 vs 400); IronCalc as the primary pivot target.
- `experiments/02-datamodel-binding-perf/findings.md` + `results/summary.md` —
  Sub-project C: the **fairness-corrected** perf numbers (load/memory, graph build,
  cascade/fan-out recompute, viewport reads, single/batch writes) for both engines.
- `experiments/01-file-support/findings.md` — Sub-project B: xlsx/CSV round-trip,
  cached-formula-result behavior, styles-on-read behavior.
- `experiments/03-formatting/findings.md` — Sub-project D: formatting exposure per
  engine; the engine-neutral side-table (`FormatStore`) recommendation.
- `functional_spec.md` §5.4 (perf targets) and §6.G (this sub-project's brief);
  `architecture.md` §1.1 (bake-off framing).
- Source-count (new, cheap): registered function set in
  `formualizer-eval-0.7.0/src/builtins` and `ironcalc_base-0.7.1/src/functions`.

## Deliverable

`experiments/06-engine-bakeoff/decision.md`, structured as:

1. **Dimension-by-dimension comparison table** across: huge-sheet load & memory;
   formula-graph build; cascade / fan-out recompute; viewport reads; single / batch
   writes; file fidelity & CSV; formatting exposure; function coverage / missing
   features; storage model; maturity / bus-factor / license; API suitability for the
   binding layer. Every cell cites a real recorded number or a source-backed fact.
2. **The case FOR Formualizer** (Arrow columnar wins the huge-sheet load/memory axis
   the project was scoped around; permissive license; native + Python + WASM;
   binding-relevant API surface) with its risks (0.x, single-maintainer, ~944
   downloads, no styles on read, slow formula-graph build, `write_range` trap, no
   public bulk *formula* ingest, unverified function parity).
3. **The case FOR IronCalc** (more mature/adopted; native styled xlsx r/w; persists
   cached results; ~4× faster graph build; ~45× faster fan-out; simpler HashMap
   model) with its risks (non-columnar = weaker huge-sheet density/scan; no CSV; no
   merges / conditional-formatting API; full-workbook non-incremental eval; pre-1.0).
4. **A clear RECOMMENDATION**, explicitly framed as *input for the human's decision,
   not the decision itself*, weighing the north star ("stupid fast on huge sheets")
   against IronCalc's maturity / formatting / graph-build edge; noting what is
   **decision-neutral** (formatting model is engine-neutral either way; both need
   async recompute; GPUI settled); with a short **"what would change the
   recommendation"** section.

## Steps

1. Read all four findings files + the two `results/summary.md`/spec sections above.
2. Count registered functions in each crate source (read-only, `~/.cargo/registry`):
   Formualizer via the builtin `name()` string literals; IronCalc via the `Function`
   enum variants (cross-check against the name-mapping + eval-dispatch arms). Record
   the numbers and the method so they are reproducible.
3. Write `decision.md` with the four sections above, citing real numbers only.
4. Read-back review for balance (this is a genuine ~toss-up; the doc must read that
   way and end with a defensible *lean*, not an overclaim).

## Pass criteria

A defensible, evidence-backed engine recommendation the human can sign off on —
both cases made fairly, every quantitative claim traceable to committed evidence,
and the recommendation explicitly a *lean for the human to ratify*, not a decision.
**→ HUMAN SIGN-OFF on the engine choice.**

## Non-goals / notes

- **No new benchmarks.** Synthesis only; cite committed numbers.
- Stay strictly inside `experiments/06-engine-bakeoff/` for writes; everything else
  is read-only. No edits outside this folder (and this plan file).
- Manager commits; this phase does not commit and never `git add -A`.
- The final go/no-go/pivot lives in Sub-project H (`SYNTHESIS.md`), not here; G
  produces only the engine recommendation that H consumes.
