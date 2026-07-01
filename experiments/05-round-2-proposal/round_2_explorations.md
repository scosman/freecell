# Sub-project F — Round-2 Technical Exploration Proposal

> Status: **not started** — placeholder created during Phase 0 scaffolding.
> Owned and filled by Phase 4 (functional_spec §6.F). Do not edit from other phases.

A ranked list of follow-up technical explorations to de-risk next, each with
rationale, synthesized from the gaps/risks surfaced across Sub-projects A–E.

## Ranked explorations

1. _(to be filled by Phase 4)_

## Captured notes (during Phase 1 gate, 2026-07-01)

Seed items recorded from the human review at the stack-decision gate. Phase 4 will
rank/expand these alongside gaps surfaced by Sub-projects A–E.

- **[Round 2] Style read → write roundtrip fidelity experiment.** Explicitly a
  *next-round* experiment (per human): load a styled `.xlsx`, edit formatting, save,
  reload, and verify which styles survive (bold/italic/fills/borders/number formats/
  merges/conditional formatting/themes). Phase 1 already pinned that Formualizer's
  `CellData` read path returns `style: None` in 0.7.0 and styles must come from the
  umya layer directly or a FreeCell-side store; this Round-2 item is the *hands-on
  roundtrip-fidelity* validation of that path (Sub-project D does the design; the
  full roundtrip experiment is deferred here).

- **[Proposed for THIS round — pending gate sign-off] Parallel IronCalc engine
  evaluation.** Human is leaning toward adding a parallel task *this* round to
  hands-on evaluate **IronCalc** alongside Formualizer (an engine bake-off), since
  IronCalc emerged as the strongest engine alternative (more mature/adopted, real
  xlsx r/w with styles) — its main weakness vs Formualizer is non-columnar
  `HashMap` storage. If confirmed at the gate, this becomes an added Round-1 phase
  (not Round 2): a smoke test + the same Sub-project C benchmarks run against
  IronCalc so the engine choice is measured, not assumed. Recorded here so it is not
  lost even if deferred.
