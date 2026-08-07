---
status: complete
---

# v05-cleanup-1

First of two batched cleanup rounds closing the **v0.5** findings of the whole-codebase
architecture review. This round holds the small, independent fixes that share no files with
each other or with the two structural projects running alongside it.

**Plan of record:**
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md)
— unit IDs below refer to it.
**Underlying review** (per-finding `file:line` evidence):
[`reviews/projects/codebase-architecture-review/`](../../../reviews/projects/codebase-architecture-review/)
— phase files are cited per unit.

## Motivation

An 8-phase fresh-eyes architecture review found 22 critical issues across concurrency,
persistence, chart handling, CI configuration and supply chain. The root cause was
diagnosed as a single generator:

> The project consistently mistakes having *reasoned about* an invariant for having
> *enforced* it.

Most of the criticals are therefore missing **enforcers**, not missing designs — which is
why so many are hours of work rather than refactors, and why they batch well. This round
takes the ones with no ordering dependencies.

## Read this first — the findings are hypotheses, not facts

The reviewers read a lot of code and compiled none of it. **One unit (G2) has already been
disproved on re-check** — it was reported as silent file corruption and turned out to be a
display bug over an already-correct save path.

So **every unit below starts by confirming the root cause still exists at HEAD**, in the
code you are about to change. Re-derive it from source; don't cite the plan as evidence.
Check `git log -p` on the file — the fix may already have landed. Go deeper than the review
did: you own one unit and can afford to trace the real path.

**A unit closed as "not a real problem, here's the evidence" is a successful outcome.** If a
root cause is absent, already fixed, or overstated, say so with evidence, correct the entry
in the remediation doc, and move on. Do not implement a fix for a problem you have not
personally confirmed.

## Scope — one unit per phase

### A1 — Pin the IronCalc fork by SHA
`app/Cargo.toml` patches `ironcalc`/`ironcalc_base` to `branch = "freecell-fixes"`, whose
documented maintenance procedure (`CLAUDE.md` §Engine) is *rebasing*. A force-push + GC
would make every historical commit unbuildable; every other git dep in the tree is
`rev`-pinned. Replace `branch` with `rev = "<sha>"`. **No tagging, mirroring or vendoring
scheme** — explicitly rejected as versioning complexity that would slow the fork loop.
Also check the `ironcalc = "=0.7.1"` line while in the file; FreeCell moved to the
post-0.7.1 style-colour API, so that pin may be inert and misleading.

### A2 — `--locked` on every cargo invocation in CI
No workflow builds with `--locked`, so the lockfile `cargo deny` audits is not provably the
one that ships — including in `release.yml`, which produces signed binaries. A PR that edits
`Cargo.toml` without regenerating the lock never fails. Add the flag across the workflows.
Also fix `deny.toml`'s header, which still references a GPL exception that was replaced with
`exceptions = []`.

*Not in scope: a `cargo tree -i zlog` guard. The GPL reintroduction case is already covered
by `cargo deny` and the extra check was considered and dropped.*

### A4 — Correct the fork docs; state the real fork strategy
Docs only. `app/Cargo.toml`'s comment names two fixes on `freecell-fixes`; the branch
carries substantially more, including a whole merged-cells feature and the `set_user_inputs`
batched-write API. **The review also claimed nothing has been upstreamed — that is wrong;
fixes have been submitted and merged.** Establish the true state from the fork's history,
then correct the comment and the
[`specs/projects/ironcalc-upstreaming/`](../ironcalc-upstreaming/) status table.

Write down the actual standing strategy, which is *not* "eliminate the fork": **we keep the
fork permanently, keep upstreaming fixes as clean single-fix PRs, and keep re-syncing from
upstream `main`.** `CLAUDE.md` §Engine and
[`projects/ironcalc-upgrade.md`](../../../projects/ironcalc-upgrade.md) currently imply a
temporary state with an exit; make them say the real thing.

### C1 — Part-inventory round-trip test  ⭐ keystone
`engine/tests/roundtrip.rs` only round-trips workbooks FreeCell itself authored — a closed
loop over IronCalc's own serializer. That is why unbounded save-time data loss has been
invisible for the project's entire life, while five real Excel fixtures sit in the tree used
for *open* assertions only.

Add open→save→reopen over `personal_monthly_budget.xlsx` and siblings, asserting a
**part-level inventory** of both zips. Expect it to go red immediately and loudly — that is
the point. Record what it proves is being dropped in `GAPS.md`; the fix is a separate v1.0
unit (C3), and the warning dialog (C2) is the next round.

→ evidence: `phase_5_feedback.md`

### D1 — Render gate weekly on `main` + at release
`render.yml`'s header says it "MUST be a required status check"; its trigger is
`workflow_dispatch:`-only. 29 dispatched runs across 13 branches, **zero on `main`, ever**,
against 44 merged PRs — while failing on 7% of the runs it did get.

**Owner decision: keep it off the PR path.** The suite is genuinely expensive and cannot earn
that cost on PRs that can't move a pixel. Add a **weekly schedule on `main`** and a **run at
release**; keep `workflow_dispatch` for agent-driven runs. No bump-token, no `paths` filter,
no per-PR gate — all three were considered and rejected.

Also fix the docs this invalidates: `app/README.md` §CI and the `checks.yml` header both
claim `checks` runs the render suite (it doesn't), and `CLAUDE.md`'s multi-paragraph "the
agent must decide when to run it" process needs rewriting around the real mechanism.

→ evidence: `phase_6_feedback.md`

### F3a — Chart-axis number formatting must agree with cells
`chart-model/src/numfmt.rs` is a 383-line reimplementation of OOXML number formatting,
existing only so `chart-model` can stay ironcalc-free. It races IronCalc's implementation, so
the same format code can produce two different strings — `#,##0.00` on a chart axis vs. on
the cells it plots. Same class: the drifted `rgb_to_hsl` copies between `core` and
`chart-model` (`.rem_euclid` vs `%`, disagreeing on negative hues) and two definitions of the
Office palette.

**Build the differential test first.** The divergence was asserted from reading both
implementations, not running them. A format-code corpus asserting the two agree may show they
already agree everywhere that matters — in which case *the test is the deliverable* and no
fix is needed. Fix the agreement, not the architecture; the crate merge is v1.0 (F3).

### G1 — Detect multi-group (combo) charts
`parse_chart_xml` keeps only the **first** chart-group element in `c:plotArea` and discards
the rest, while `source_fidelity` never counts groups. So an ordinary Excel bar+line combo
loads as bars only — the line series absent from the picture — classified `Faithful`, drawn
with no badge. The whole 1,272-line `fidelity.rs` exists to prevent exactly this.

Count group children at parse time and force `Fidelity::Degraded`. Also fix
`is_extended_chart`'s bare `contains("chartex")`, and correct the `GAPS.md` combo row, which
currently claims placeholder behaviour that does not happen.

This makes the failure **honest, not correct** — real combo support is a v2.0 project (G1b);
add or update its `GAPS.md` entry at that tier.

→ evidence: `phase_4_feedback.md`

### G5 — `dLbls` overrides + chart-insert collisions
Editing data labels whole-node-replaces `c:dLbls`, destroying per-point overrides and label
typography — the one real hole in an otherwise excellent preserve-unknown save path, so this
half is a **data-loss fix and is v0.5**. Separately, inserting a chart onto a sheet that
already carries one is a hard `SaveError`, because the byte-preserve and write-from-model
paths cannot compose on a shared drawing — that half is **v1.0**; if it grows, file it in
`GAPS.md` and ship the `dLbls` fix alone.

→ evidence: `phase_4_feedback.md`

## Out of scope

- Anything touching `app/crates/freecell-app/src/chrome/view.rs` — owned by the parallel
  **chrome-view-split** project.
- Anything touching `engine/src/worker/run.rs`, `client.rs`, `protocol.rs` or
  `core/src/publication.rs` — owned by the parallel **engine-worker-hardening** project.
  This includes the `catch_unwind` and frozen-pane fixes (B1/B2), which live there.
- C2 (save-fidelity warning), D2 (pixel tolerance), F2 (line ceiling), H1, H3 — they depend
  on this round or the structural projects, and land in **v05-cleanup-2** /
  **invariant-enforcement**.

## Working agreement

- **Process — follow the `/spec` loop exactly as defined. Do not improvise a faster one.**
  `/spec implement` puts you in a **manager** role: per phase you spawn a coding sub-agent,
  validate its attestation, spawn a *fresh* CR sub-agent, route feedback back and re-review
  until clean, resume the coding agent to commit, then verify with `git status`. Every code
  change — CR fixes included — needs a clean CR before commit. Do not write the code or run
  the review inline yourself, and do not skip the CR loop because a change looks small.
  "Autonomous" below means **no human sign-off**; it does not mean a shortened process.
- **Autonomy:** run the full spec + implement flow in one pass. Do not stop for sign-off
  between spec phases. Ask only if a *real* unknown surfaces — a decision that changes the
  work materially and cannot be resolved from the code, the remediation doc, or the review
  artifacts. Pushing back on a unit is encouraged; asking permission to proceed is not.
- **Units are independent.** One phase each. If one is disproved or blocked, that phase
  ends with a written finding and the rest continue.
- Existing repo conventions apply: crate-scoped `cargo build -p` / `cargo test -p --lib` per
  phase, `cargo fmt --all --check` (whole workspace) always, commit + push regularly.
- **Render tests:** nothing in this round should move grid/cell/sheet/titlebar or chart-render
  pixels. G1/G5 touch chart *parse and save*, not the chart render widgets. If a change turns
  out to be render-in-scope, run the relevant `render_tests.sh test <prefix>` subset — do not
  run the full suite for this round.
- **Fork policy:** if any unit needs an IronCalc change, it is one fix = one `fix/` branch =
  one clean upstream PR. Do not fold two fork fixes into one branch.
