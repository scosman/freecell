---
status: complete
---

# Phase 8: Phase 1 Final Synthesis (Sub-project H)

## Overview

Sub-project H is the **capstone synthesis / writing** phase, not an experiment. It
consumes all prior Phase-1 findings (Sub-projects A–G) and the two locked decisions —
**engine = IronCalc** (human sign-off, Sub-project G) and **UI = GPUI** (raw-gpui grid
+ gpui-component chrome, human-validated on macOS) — into a single **decision-ready**
document, `experiments/SYNTHESIS.md`, that feeds the human's **Stage-3 "do we keep
going" decision** (functional_spec §6.H; project_overview Stage-3).

**No new benchmarks are run.** Every quantitative claim is cited from an
already-committed findings doc and traceable to a recorded result under
`experiments/*/results/`. This document must be **self-contained and accurate** — it
cites real numbers so a reader deciding on Stage-2 investment needs nothing else.

## Evidence inputs (read-only)

- `specs/projects/freecell-phase-1/project_overview.md` — the product thesis
  (GPU-rendered, Rust, Excel-compatible, stupid-fast on huge sheets; deep testing).
- `specs/projects/freecell-phase-1/functional_spec.md` — §1 purpose (de-risking),
  §5.4 perf targets, §5.5 what success means.
- `experiments/00-stack-decision/findings.md` (A) — engine/UI landscape, ranked stacks.
- `experiments/01-file-support/findings.md` (B) — xlsx/CSV round-trip, cached results,
  styles-on-read.
- `experiments/02-datamodel-binding-perf/findings.md` (C) + `results/` — the
  fairness-corrected perf numbers (viewport reads, 1M cascade, memory, writes).
- `experiments/03-formatting/findings.md` (D) — formatting exposure; `FormatStore`.
- `experiments/04-ui-poc/findings.md` (E) — GPUI PoC (raw-gpui vs gpui-component).
- `experiments/06-engine-bakeoff/decision.md` (G) — engine = IronCalc, rationale,
  IronCalc-specific consequences, function counts (345 vs 410).
- `experiments/05-round-2-proposal/round_2_explorations.md` (F) — the ranked next steps.

## Deliverable

`experiments/SYNTHESIS.md`, structured as a decision-ready document:

1. **Verdict up top** — a clear GO / go-with-conditions / no-go / pivot recommendation
   for proceeding past Stage 1, in 2–3 sentences, with confidence level.
2. **What Phase 1 set out to de-risk, and the answers** — a compact table mapping each
   core uncertainty (engine viability, huge-sheet perf, GPU UI at scale, file I/O,
   formatting, Excel-compat foundation) → what we found → proven / partly-proven /
   unproven.
3. **The stack we're recommending** — IronCalc engine + GPUI (raw-gpui grid,
   gpui-component chrome): one paragraph each on *why*, grounded in evidence and honest
   about what the choice costs.
4. **What is PROVEN** (with key numbers) — viewport reads <2 ms both engines; 10⁷ cells
   reached; GPUI grid renders fast (human-confirmed + poc-core virtualization tested);
   xlsx/CSV round-trip; IronCalc native styled I/O + persists cached results; file/format
   matrix.
5. **What is UNPROVEN / top risks into Stage 2** — lead with IronCalc's
   no-incremental-recalc full-`evaluate()` cost (1M cascade ~2 s, async mandatory +
   unvalidated at scale), then end-to-end large-`.xlsx` open (§5.4's un-run target),
   function-parity audit (345 vs ~500), GPUI/Zed git-pin + GPL #55470 legal sign-off.
   Point to Round-2 (F) as the agenda.
6. **Meta-outcome** — the reusable engine-neutral test/benchmark harness (shared
   `datagen` + `bench_util` + `SpreadsheetEngine` trait) and the adversarial-review
   process (caught a backwards perf conclusion before it drove the decision).
7. **Honest caveats** — all perf on a 4-core Linux box (Mac faster); several
   conclusions rest on Phase-1-scale point samples; the engine call was a genuine
   toss-up decided on delivery risk.

## Steps

1. Read all Phase-1 findings (A–G) + project_overview + functional_spec §1/§5.4/§5.5.
2. Write `experiments/SYNTHESIS.md` with the seven sections above, citing real numbers
   only — verdict-first, evidence-anchored, honest about the unknowns.
3. Read-back review for accuracy (every number traceable) and for the intended
   posture: a defensible GO-with-conditions, not a hedge.

## Pass criteria

A self-contained, decision-ready `SYNTHESIS.md` that opens with a defensible verdict,
maps each core uncertainty to a proven/partly-proven/unproven answer with real cited
numbers, names the top Stage-2 risks (led by IronCalc's async-recompute gap), and
points at Round-2 (F) as the agenda — the artifact the human uses for the Stage-3
go/no-go decision.

## Non-goals / notes

- **No new benchmarks.** Synthesis only; cite committed numbers.
- Only two files are written: `experiments/SYNTHESIS.md` and this plan. Everything else
  is read-only.
- Manager commits; this phase does not commit and never `git add -A`.
- This is the final Phase-1 deliverable; the go/no-go/pivot decision itself belongs to
  the human at Stage 3, informed by this document.
