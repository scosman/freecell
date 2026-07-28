---
status: complete
---

# invariant-enforcement

Convert the codebase's prose-asserted invariants into enforced ones. This is the unit that
addresses the **root cause** the architecture review identified, rather than any individual
symptom.

**Plan of record:**
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md)
§H1.
**Underlying review:**
[`reviews/projects/codebase-architecture-review/phase_8_verdict.md`](../../../reviews/projects/codebase-architecture-review/phase_8_verdict.md)
§"The Pattern Behind the Problems" — read this before planning.

## Motivation

An 8-phase review found 22 critical issues spread across concurrency, persistence, XML schema
handling, GPUI state ownership, CI configuration and supply chain. That spread rules out a
domain-knowledge gap. The synthesis reviewer identified one generator:

> **The project consistently mistakes having *reasoned about* an invariant for having
> *enforced* it.**

Sort every finding by that axis and it partitions cleanly.

**Enforced by a mechanism → healthy, in every case:** crate boundaries (cargo + a guard test
with negative controls); IronCalc containment (`pub(crate)`); command-dispatch completeness
(exhaustive match); publication ordering (a concurrent test with a non-zero-sample assertion);
zero engine calls on the render path (a process-global counter with a negative control); the
GPL removal (verified against `Cargo.lock`).

**Asserted in prose → failed, in every case.** These are the sweep targets:

| Claim in a comment | What is actually true |
|---|---|
| *"the bands are a few leading tracks, never a sheet-size loop"* | sits directly above the unbounded loop that wedges the worker |
| *"the worker is the only writer"* | `autogrow_measure_now` takes `caches.write()` from the UI thread in the shipped binary |
| *"at most one popover is open"* | eight independent `bool`s, holding only because each popover paints an occluding backdrop |
| *"the undo stacks stay 1:1"* | violated by `SetFrozen` sending two engine history entries against one worker touch |
| *"the toggle reads at render / click time, not per frame"* | `render` **is** per frame |
| five "BUG #5" comments on which cross-entity calls need `window.defer` | decided per-call-site by reasoning, with no type or helper enforcing it |
| three hand-written `max()` reconciliations of row height | one already caused a bug its own comment documents |

Two corollaries the sweep should keep in view:

- **Zero `TODO`/`FIXME` markers exist in 99k lines.** Debt is not recorded in code; it is
  externalized into `GAPS.md`, `specs/` and `projects/` — the artifacts that cannot fail a
  build.
- **Failures cluster at the boundaries *between* well-built things.** Each good mechanism
  enforces itself and nothing adjacent to it, so every new neighbour is a fresh coin flip
  resolved by whoever remembers the rule.

## Prerequisites

Preferred, not strictly blocking:

- **engine-worker-hardening** — its Phase 1 fixes the first row of the table above and is the
  worked example for the whole sweep. Read that diff before starting.
- **chrome-view-split** — several targets live in `chrome/view.rs`, and sweeping a file that
  is about to be split wastes the work.

## Scope

**Per target, the deliverable is one of four outcomes** — and three of them are not code:

1. **Enforce it** — a `debug_assert!`, a clamp, a type that makes the illegal state
   unrepresentable (e.g. `Option<OpenPanel>` instead of eight `bool`s), or a test.
2. **Delete the claim** — if the invariant isn't real or isn't worth enforcing, the comment is
   worse than nothing because it is load-bearing for the next reader.
3. **Already enforced** — the invariant holds because of a mechanism the reviewer didn't
   notice. Record where, and move on. **This is a legitimate and expected outcome; the
   findings below are hypotheses from an agent that read the code and compiled none of it.**
4. **Too big for this sweep** — enforcing it is a real project. File it in `GAPS.md` with what
   you found, and do not start it here.

Named starting targets, from the table above:

- Feature-gate or eliminate `autogrow_measure_now`'s UI-thread `caches.write()`, making
  "the worker is the only writer" true or the comment false.
- Replace the eight popover `bool`s with `Option<OpenPanel>`.
- Return the engine-entry count from `apply_one` so the undo 1:1 claim is checked rather than
  asserted.
- Collapse the three hand-written row-height `max()` reconciliations into one function with
  three call sites.
- Add a single `post_to_sibling` helper that always defers, retiring the five "BUG #5"
  comments.

Then **sweep beyond the named list**: search for comments that assert an invariant
("never", "always", "must", "guaranteed", "at most one", "only the X does Y") and apply the
same four-way decision. Report how many you found and how they partitioned — that number is
the real output of this project, because it tells us whether the pattern is contained or
endemic.

## Explicitly not in scope

- **Writing the rule into `CLAUDE.md`** — that is H3, in the parallel `v05-cleanup-2` project.
  This project produces the evidence; H3 writes the convention. If H3 lands first, cite it; if
  this lands first, hand H3 your findings.
- Any structural refactor a target implies. If enforcing an invariant requires E2's view-model
  or F4's protocol collapse, that is outcome 4 — file it.
- Adding `TODO`/`FIXME` markers as a policy change. Note the observation; the policy call
  belongs to H3.

## Working agreement

- **Autonomy:** run the full spec + implement flow in one pass. Do not stop for sign-off
  between spec phases. Ask only if a real unknown surfaces. **Judgment is the substance of
  this project** — deciding "enforce vs. delete vs. already-fine vs. too big" is the work, and
  you are expected to make those calls and defend them in writing, not to escalate them.
- Group targets into coherent phases by area (engine, chrome, grid, core) rather than one phase
  per target — several are one-line changes and shouldn't each carry a phase's overhead.
- Every enforcement added needs a test that would **fail without it**. An assertion nothing
  exercises is another prose invariant wearing a `debug_assert!`. Where practical follow the
  codebase's own best pattern: the existing guard tests ship **negative controls** proving the
  guard trips on a synthetic violation, and there are 22 of them to copy from.
- Crate-scoped checks per phase; `cargo fmt --all --check` (whole workspace) always.
- **Render tests:** targets touching `grid/` render code are pixel-in-scope — iterate with
  `render_tests.sh test <prefix>` subsets and, if any grid-render change lands, run the full
  suite once at the end under a ~10-minute watchdog. Targets in `chrome/` (popovers, sidebar)
  have no baselines and need only gpui view tests plus an Xvfb smoke launch.
- Commit per phase. Push regularly.
