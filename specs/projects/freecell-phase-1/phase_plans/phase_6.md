---
status: complete
---

# Phase 6: Round-2 Technical Exploration Proposal (Sub-project F)

## Overview

Sub-project F (functional_spec §6.F; project_overview "Round 2 technical
exploration") is a **synthesis / writing** phase, not an experiment. It produces a
**ranked list of follow-up technical explorations** to de-risk next, grounded in the
committed Phase-1 evidence (Sub-projects A–E) and the decisions already made at the
gate:

- **UI = GPUI**, human-confirmed on macOS: a **raw-gpui custom grid** for the sheet
  itself, `gpui-component` for surrounding chrome (Sub-project E).
- **Engine = IronCalc** (Sub-project G, `06-engine-bakeoff/decision.md`), a **human
  decision** overriding that document's soft go-with-conditions lean toward
  Formualizer. This single decision reshapes the Round-2 list: the ranked items are
  now **IronCalc-specific** (its weaknesses, not Formualizer's, are what must be
  de-risked), and three validations were **explicitly deferred to Round 2** by the
  decision doc.

**No benchmarks are run in this phase.** It is pure synthesis: read the committed
findings + the engine decision, fold the existing "Captured notes" from the gate into
a coherent ranked list, and write each item with *what/why*, *rough approach*, and
*what it de-risks*.

## Evidence inputs (read-only)

- `experiments/06-engine-bakeoff/decision.md` — **DECISION: engine = IronCalc.** The
  "✅ HUMAN DECISION" section is the primary driver: it names the IronCalc-specific
  design consequences (no native range read → per-cell viewport loop; **no
  incremental recalc** → every edit = full-workbook `evaluate()` off the UI thread;
  no merges/CF API → FreeCell side-store; engine-neutral `FormatStore` stays the
  render model) and the **three validations deferred to Round 2** (end-to-end large
  `.xlsx` open with styles; IronCalc full-`evaluate()` cost at Excel-max scale;
  function-parity audit of IronCalc's 345 registered builtins).
- `experiments/00-stack-decision/findings.md` (A) — GPUI/Zed coupling + GPL-linkage
  (#55470) risks; `gpui-component` spreadsheet gaps; engine landscape.
- `experiments/02-datamodel-binding-perf/findings.md` (C) — the fairness-corrected
  perf numbers: IronCalc 10M-cell load 6.13 s / 1.63 GB (~162 B/cell); no incremental
  recalc; 1M serial cascade ~2.1 s (FAIL <100 ms); fan-out 77.5 ms; viewport read
  392 µs (PASS <2 ms) via per-cell loop; `UserModel` diff-list change feed.
- `experiments/01-file-support/findings.md` (B) — IronCalc native styled xlsx r/w,
  persists cached results; no CSV; two-step + four-arg load; the un-run 100 MB+ open.
- `experiments/03-formatting/findings.md` (D) — IronCalc native styles; **no merges /
  conditional-formatting API**; the engine-neutral `FormatStore` design; deferred
  style-roundtrip-fidelity sweep.
- `experiments/04-ui-poc/findings.md` (E) — raw-gpui vs gpui-component; the **pending
  Mac Run Test numbers**; PNG rendering-baseline as a foretaste of the product's
  rendering-regression strategy.
- `functional_spec.md` §5.4 (targets, incl. the un-run "open a 100 MB+ `.xlsx`" one)
  and §6.F; `project_overview.md` "Round 2 technical exploration".

## Deliverable

Rewrite `experiments/05-round-2-proposal/round_2_explorations.md`:

- Preserve the existing **"Captured notes" from the gate** (do not lose them): the
  style read→write roundtrip item stays a Round-2 exploration; the parallel-IronCalc
  bake-off note is now **DONE** (it became Sub-project G and produced the IronCalc
  decision) and is folded in as resolved history, not a live item.
- Produce a **RANKED list**, ordered by impact on the eventual go/no-go, each item
  with **what/why**, **rough approach**, and **what it de-risks**. Rank so the chosen
  engine's core weakness leads. Planned order:

  1. **IronCalc full-`evaluate()` cost at Excel-max scale** (highest — the chosen
     engine has no incremental recalc; every edit is a full-workbook eval) + the
     off-thread/debounced recompute + "recalculating" UX design.
  2. **End-to-end large `.xlsx` open** (time + peak RSS, **with styles**) — closes
     §5.4's un-run target and the Arrow-ingest-≠-file-open parse-cost gap.
  3. **Function-parity audit** — IronCalc 345 builtins vs Excel ~500: what's missing +
     Excel-correctness on edge cases / errors / locale (raw count ≠ parity).
  4. **Binding layer for IronCalc specifically** — per-cell viewport loop holds <2 ms
     at scale under real formatting; `UserModel` diff-list cache invalidation.
  5. **FreeCell `FormatStore` design/prototype** + row/col insert-delete interaction.
  6. **Style read→write roundtrip fidelity** (kept from captured notes) + **merges /
     conditional-formatting side-store** design (IronCalc has no API for either).
  7. **GPUI grid maturation** — full raw-gpui grid (in-cell editing, selection
     ranges, frozen panes); capture the quantitative Mac Run Test numbers if not yet
     done; PNG rendering-baseline tests.
  8. Residual A–E gaps rolled up (GPUI/Zed pin + GPL sign-off; CSV bridge;
     cross-sheet / volatile / parallel-DAG recompute shapes; IronCalc storage-density
     headroom toward Excel-max).

  (Exact final ordering/merging decided while writing, but the chosen-engine core
  weakness leads and the three explicitly-deferred validations rank in the top few.)

## Steps

1. Rewrite `round_2_explorations.md` per §5.2-adjacent structure: a short framing
   paragraph (decisions locked: UI = GPUI raw-grid + gpui-component chrome; engine =
   IronCalc), then the ranked list, then a folded-in "Captured notes / resolved
   history" section that preserves the two gate notes (one now DONE, one still live).
2. Cite specifics from A–E and the decision doc for each item (numbers with their
   source), so the list is evidence-grounded, not generic.
3. No code, no benchmarks, no new crates. Stay strictly inside
   `experiments/05-round-2-proposal/` (+ this phase-plan file).

## Guardrails

- Foreground only; no background monitors.
- Write only `specs/.../phase_plans/phase_6.md` and
  `experiments/05-round-2-proposal/round_2_explorations.md`; everything else
  read-only.
- Git ops path-scoped; never `git add -A`; do **not** commit.

## Tests

- NA — documentation-only phase. No code, no benchmarks. Verification is that the
  deliverable is a ranked, evidence-cited list covering the required items and
  preserving the captured gate notes.
