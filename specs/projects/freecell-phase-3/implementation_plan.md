---
status: complete
---

# Implementation Plan: FreeCell — Phase 3 (Pre-Build De-risking / "Round 3")

Ordered build plan. Details live in `project_overview.md` (the self-contained handoff:
locked decisions, inherited evidence, the four investigations with pass criteria, file
map, conventions), `functional_spec.md` (investigations A–D formalized + pass criteria),
and `architecture.md` (layout/reuse, orchestration, methodology, the Investigation-A
cache-sync design space §4). Orchestration, the parallel-editor isolation rule,
serialized commits, and the build-readiness checkpoint are in architecture §2.
**Parallel phases run in their own git worktrees** and merge back (disjoint folders).
**A, B, D run in-container; C's demonstrable half is macOS/human-run.**

## Sequencing

Scaffolding → in-container cohort A/B/D in parallel (C's in-container investigation
alongside; its render→PNG→diff harness authored for the human macOS run) →
**build-readiness checkpoint (human)** → synthesis.

## Phases

- [ ] **Phase 3.0 — Scaffolding** *(serial)*: create `experiments/round-3/` with
  `{A-cache-sync, B-api-audit, C-ci-rendering, D-robustness}/` skeletons as **independent
  Cargo projects**; wire each by relative path, **read-only**, to `../../round-2/harness`
  and `../../shared/*`; pin `ironcalc`/`ironcalc_base` to **0.7.1**. **No new frozen
  harness** — A/B probe `UserModel` in their own crates (architecture §1). (architecture
  §1, §2.1)

*After scaffolding, the in-container cohort runs in parallel (own worktree/folder):*

- [ ] **Phase A — Style/geometry cache sync + structural editing** *(parallel; own
  review — THE key investigation)* → `round-3/A-cache-sync/`. Probe `UserModel`
  (insert/delete rows/cols, undo/redo, copy/paste, diff-list, `Send`-ness, does the SP1
  seam hold); correctness harness asserting insert/delete row/col shift references + band
  styles + sizes; undo/redo across value/style/structural; **cache-sync prototype that
  shifts and provably agrees with IronCalc** (architecture §4.4 contract); structural-edit
  + cache-shift cost at 10⁵–10⁶; `.xlsx` round-trip. **Lock the cache-sync design +
  `Model`-vs-`UserModel` recommendation.** GATES: structural correctness + undo/redo
  coverage; validated cache-sync design agreeing with IronCalc at acceptable cost.
  (functional_spec A; architecture §4)

- [ ] **Phase B — Needed-API audit** *(parallel; batched review)* →
  `round-3/B-api-audit/`. Probe IronCalc 0.7.1's public API against the checklist
  (display formatting **[headline — who owns number-format rendering]**, diff-list shape,
  sheet ops, defined names, view state, cell extras, tokenizer; re-confirm merges/CF and
  dynamic arrays as OPEN). Produce the **present/absent/workaround matrix**, each entry
  probe- or source-cited, with a plan per gap. GATE (judgment): no surprise load-bearing
  gap buried. (functional_spec B; architecture §5)

- [ ] **Phase D — Engine robustness** *(parallel; batched review; cheap)* →
  `round-3/D-robustness/`. Feed circular refs (`A1=A1`; `A1=B1,B1=A1`) + malformed/
  pathological formulas; assert typed errors, **no hang** (foreground `timeout`), **no
  panic**; test worker-panic-recovery (catch_unwind / restart) for the SP1-style worker.
  GATE: circular refs error without hanging; malformed → error not panic. DELIVERABLE:
  worker-robustness recommendation. (functional_spec D; architecture §5)

- [ ] **Phase C — CI snapshot rendering** *(in-container investigation + macOS human-run)*
  → `round-3/C-ci-rendering/`. In-container: investigate GPUI's offscreen/headless
  capture surface and attempt it (expected fail, no GPU — the failure mode is the
  finding). Author the **render→PNG→perceptual-diff** harness (evolves Phase-1
  `04-ui-poc`); **human runs it on macOS**, commits a baseline PNG, confirms a re-render
  passes within tolerance and a changed scene fails (discriminating power), reports the
  confirmed CI mechanism. GATE: a confirmed working "snapshot the grid in CI" mechanism
  demonstrated end-to-end. (functional_spec C; architecture §5)

*After the cohort lands (C folded in when the human reports the macOS run):*

- [ ] **⛳ BUILD-READINESS CHECKPOINT (human review)** — present A–D findings against each
  investigation's pass criteria. Any GATE fail or off-ramp (structural edits broken/slow,
  undo/redo missing, cache-shift intractable, surprise load-bearing API gap, no viable
  CI-snapshot mechanism, circular-ref hang) → surface for a human "change-first vs accept"
  decision **before the build commit**. Clean → "clear to build." (architecture §2.4)

*After the checkpoint:*

- [ ] **Phase Synthesis — Stage-3 "clear to build" recommendation** *(serial; last)* →
  `experiments/round-3/SYNTHESIS.md`. Cite A–D; state whether any off-ramp fired; give
  the **"clear to build"** verdict or the precise **must-change-first** list, with
  build-time carry-forward. (functional_spec §6 Synthesis)
