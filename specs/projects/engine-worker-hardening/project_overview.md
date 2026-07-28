---
status: complete
---

# engine-worker-hardening

Close the v0.5 engine findings of the architecture review: two crash/hang defects, the
extraction of chart code out of `worker/run.rs`, and the unification of the four shared
worker→UI surfaces under one commit point.

These are grouped into one project because **they all own the same files** —
`engine/src/worker/run.rs` above all — and splitting them across parallel agents would
guarantee merge conflicts.

**Plan of record:**
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md)
§B1, §B2, §E1, and the `worker/run.rs` half of §F1.
**Underlying review:** `reviews/projects/codebase-architecture-review/phase_2_feedback.md`
(concurrency), `phase_5_feedback.md` (the persistence-side view of the same panic gap).

## Motivation

The worker seam's *primary* surface is genuinely well built and should be protected: one
thread owns the `UserModel`, the UI does one wait-free `ArcSwap` load per frame,
publish-then-bump ordering, no `block_on`, no blocking `recv`, and zero engine calls on the
render path — that last invariant enforced by a process-global counter with a negative
control, not a comment.

The problems are all at the **edges of** that good design:

- Two paths escaped an invariant the surrounding code takes seriously (an unclamped loop, two
  unguarded calls).
- Three shared surfaces were added *beside* the publication rather than through it, each with
  its own primitive and version discipline.

This is the review's central pattern in one file: every invariant with a mechanism holds;
every invariant written in a comment has already failed.

## Confirm before building

Every finding below is a hypothesis from an agent that read the code and compiled none of it.
**Confirm each root cause at HEAD before fixing it** — re-derive from source, check
`git log -p`, and go deeper than the review did. One unit elsewhere in this remediation
(chart theme colours) was already disproved this way. A phase that ends "this isn't real,
here's why" is a success; correct the remediation doc and move on.

## Scope — ordered phases

The order matters: the two safety fixes are small and land first, the mechanical move
follows, and the structural unification goes last so it is built on stable ground.

### Phase 1 — B2: Clamp the frozen-pane band; validate `SetFrozen`
Every loop in `build_publication` is bounded by a constant **except** the frozen-pane band,
which iterates `(0..m)×(0..k)` from unvalidated `frozen_rows`/`frozen_cols`. The UI computes
the count as "last row of the header run you right-clicked", so freezing at row 500,000 makes
every subsequent publish do ~500,000 × 256 `formatted_value` calls and the worker never
returns. **Two clicks to a permanent hang**, also reachable from a crafted `.xlsx` `<pane>`
element.

Clamp at the publish site **and** range-validate in `pre_validate`, closing both the click
path and the file path. Note the doc comment directly above the loop asserts it can never be
a sheet-size loop — that comment *is* the bug, and it is the canonical example for the
invariant sweep (H1) that follows this project.

### Phase 2 — B1: Guard load/save with `catch_unwind`; surface worker death
Six mutation regions are guarded; `from_source` and `save_workbook` are not — and the pinned
exporter contains a reachable `panic!("Model needs to be evaluated before saving!")`. Because
the `JoinHandle` is discarded and `send` swallows `SendError` by design, a panic on the file
path produces a **silent zombie**: the window keeps rendering the last publication, edits
vanish, Save does nothing, and there is no dialog, no degraded bar and no log entry.

Wrap both calls into `LoadFailed` / `SaveFailed`, retain the handle, and make the window
treat "event stream ended without a requested `Shutdown`" as fatal and say so. Touches
`app/src/shell/window.rs` for the UI half — the only file outside the engine this project
owns.

*Found independently by the concurrency and persistence reviewers from opposite directions:
it looks like a threading bug from one side and a persistence bug from the other.*

### Phase 3 — Extract chart handling out of `worker/run.rs`
`run.rs` is 9,288 lines (~3,984 production, ~43%), carrying roughly fifteen responsibilities
and 28 `Worker` fields. About 1,200 of those lines are chart machinery sitting next to an
**empty 39-line `worker/charts.rs`**. Move them there. Behaviour-preserving; no logic
changes.

This is the `worker/run.rs` half of the review's F1 unit — the `chrome/view.rs` half runs as
a separate parallel project. Aim for the same **2,000 production-line** ceiling that CI will
enforce next round (F2); if `run.rs` still exceeds it after the chart extraction, say so and
propose what else should move rather than silently leaving it over.

### Phase 4 — E1: One commit point for the four shared worker→UI surfaces
**Promoted to v0.5** — a correctness problem reachable today, and one that gets cheaper the
sooner it lands, since every additional shared surface is one more thing to unify later.

The `ArcSwap` publication was designed as a seam; the **style cache**, **chart snapshot** and
**CF map** were each added beside it with their own primitive and their own (or no) version
discipline. The commit order emits `Published` *between* the value commit and the style
commit, and `commit_chart_op` emits `Published` without publishing. The consequence is that
there is no answer to *"what does the UI see at generation N"* — which is the question you
must be able to answer to reason about this seam at all.

Unify under one generation counter and one commit point. Add ordering tests with
**non-zero-sample assertions** — the existing publication test does this correctly and is the
pattern to follow; a concurrency test that can pass without ever observing the interleaving
is not a test.

## Out of scope

- `app/crates/freecell-app/src/chrome/view.rs` — owned by the parallel **chrome-view-split**
  project. Do not touch it.
- **B3 (cache lock hold-time).** `refresh_cache_cells` holds the cache write lock across up to
  100,000 IronCalc reads while the render thread takes it 23 times per frame. Deferred to
  **v3+** by owner call: perf is validated and this is a theoretical contention win, not an
  observed one. If Phase 4 makes it trivially free, mention it — do not chase it.
- **F4 (protocol collapse).** 53 `Command` + 23 `WorkerEvent` variants with UI concerns
  leaking into the engine (column widths in device pixels). Real, but v2.0 and downstream of
  the UI state work.
- The invariant sweep (H1) — its own project, next round, and this project's Phase 1 is its
  worked example.

## Working agreement

- **Process — follow the `/spec` loop exactly as defined. Do not improvise a faster one.**
  `/spec implement` puts you in a **manager** role: per phase you spawn a coding sub-agent,
  validate its attestation, spawn a *fresh* CR sub-agent, route feedback back and re-review
  until clean, resume the coding agent to commit, then verify with `git status`. Every code
  change — CR fixes included — needs a clean CR before commit. Do not write the code or run
  the review inline yourself, and do not skip the CR loop because a change looks small. This
  project touches concurrency and the file path; an unreviewed "obvious" edit here is exactly
  the class of change that produced the findings you are fixing. "Autonomous" below means
  **no human sign-off**; it does not mean a shortened process.
- **Autonomy:** run the full spec + implement flow in one pass. Do not stop for sign-off
  between spec phases. Ask only if a real unknown surfaces. The plausible one here is Phase 4:
  if unifying the four surfaces turns out to require changing what the UI reads (rather than
  when it reads it), that is a bigger change than scoped — raise it rather than expanding
  silently.
- **Phases 1–3 must not change observable behaviour** except for the new error paths in
  Phase 2. Phase 4 changes ordering, which is the point — that is where the new tests go.
- Crate-scoped checks per phase: `cargo build -p freecell-engine` +
  `cargo test -p freecell-engine --lib`, plus `cargo test -p freecell-engine --test
  worker_seam` for phases 1, 2 and 4. `cargo fmt --all --check` (whole workspace) always.
  Add `-p freecell-app` for Phase 2's window changes.
- **Render tests:** this project does not touch grid-render code, so the pixel suite is out of
  scope. Phase 4 changes *what is published and when*, which the render harness consumes — if
  a phase turns out to alter published content, run the relevant `render_tests.sh test
  <prefix>` subset rather than the full suite.
- Commit per phase. Push regularly.
