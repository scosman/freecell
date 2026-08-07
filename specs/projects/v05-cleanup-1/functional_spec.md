---
status: complete
---

# Functional Spec: v05-cleanup-1

Seven independent v0.5 remediation units from the whole-codebase architecture review.
Each unit is **a hypothesis to confirm, then either fix or close with evidence**. This
spec states, per unit, what "done" means for *both* outcomes.

Source of truth for unit intent: [`project_overview.md`](project_overview.md) and
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md).

---

## Cross-cutting behaviour

### The confirm-first contract

Every unit begins with a **confirmation step** and ends with a **written verdict**. The
confirmation step must cite live code (`file:line` at HEAD) and, where the claim is about
behaviour rather than text, must be demonstrated by *running* something — a test, a
`cargo` invocation, a git query — not by reading.

Three legal verdicts per unit:

| Verdict | Meaning | Required output |
|---|---|---|
| **Confirmed** | Root cause exists at HEAD as described | The fix, plus a regression guard where one is possible |
| **Confirmed, different** | A real defect exists but not the one described | Fix the real one; correct the remediation-doc entry |
| **Disproved** | No defect, already fixed, or overstated to the point of being wrong | Evidence write-up; correct the remediation-doc entry; **no code change** |

A **Disproved** verdict is a successful phase. It is not a reason to invent adjacent work.

### Where verdicts are recorded

1. **`projects/architecture-review-remediation.md`** — the plan of record. Each unit's
   section gains a short `**Outcome (v05-cleanup-1, <date>):**` note stating the verdict
   and the one-line evidence. Disproved units keep their text but are marked so a future
   reader does not re-litigate them.
2. **The phase plan** (`specs/projects/v05-cleanup-1/phase_plans/phase_N.md`) — the full
   derivation, evidence and reasoning.
3. **`GAPS.md`** — only where a unit *reveals or reclassifies a user-visible gap*
   (C1, G1, G5-insert). Not a general log.

### What is explicitly out of scope for every unit

- `app/crates/freecell-app/src/chrome/view.rs` (chrome-view-split project owns it).
- `app/crates/freecell-engine/src/worker/*` and `core/src/publication.rs`
  (engine-worker-hardening owns them).
- Any fix belonging to a later unit (C2, C3, D2, F2, F3, G1b, G3, H1, H3). Where this
  round *surfaces* such work, it is filed, not built.
- Behaviour changes to chart rendering widgets. This round touches chart **parse/save**
  and **number-format/colour agreement**, never the drawing code.

### Verification per phase

- Crate-scoped `cargo build -p <crate>` and `cargo test -p <crate>` for the crate touched.
- `cargo fmt --all --check` (whole workspace) on every phase, always.
- Docs-only phases (A4, and the doc halves of A2/D1/G1) require no compile, but any phase
  that edits a workflow file must be YAML-validated and reasoned through, since CI cannot
  be run from here for scheduled/release triggers.
- No full render suite. If any change turns out to move pixels, the relevant
  `render_tests.sh test <prefix>` subset only.

---

## A1 — Pin the IronCalc fork by SHA

**Confirmed at HEAD:** `app/Cargo.toml` `[patch.crates-io]` uses
`branch = "freecell-fixes"` for both `ironcalc` and `ironcalc_base`.

### Behaviour required

1. Both patch entries pin `rev = "<40-char sha>"` — the current tip of
   `scosman/ironcalc@freecell-fixes` — replacing `branch`.
2. `Cargo.lock` is regenerated so the locked source URL carries that rev, and the build
   resolves to the *same* code as before the change (no dependency drift is permitted as
   a side effect of this unit).

   > **Deviation, deliberate — recorded 2026-08-07.** Items 1 and 2 turned out to be in
   > conflict: `freecell-fixes`' tip had already moved two commits past the locked SHA, so
   > pinning "the current tip" (item 1) necessarily changed what the build resolves to
   > (item 2). On owner instruction the phase pinned the **tip** (`ecbf6226`) and accepted
   > the engine change — a deliberate revert of `fix/dollar-negative-zero`, which moved
   > `=DOLLAR(-0.001,2)` from `$0.00` to `($0.00)`. Nothing else in `Cargo.lock` moved
   > (the diff is 4 lines). Item 2's real intent — *no **incidental** drift* — holds.
   > Full reasoning: `phase_plans/phase_1.md`.
3. A comment states the maintenance procedure: bumping the pin is a one-line edit; the
   SHA must be re-pinned whenever the fork moves.
4. The `ironcalc = "=0.7.1"` / `ironcalc_base = "=0.7.1"` workspace lines are
   investigated. If they are inert under the patch, that is stated in a comment rather
   than left implying a real version constraint. **They are not removed** unless removing
   them provably leaves the resolve unchanged — `[patch]` requires a version requirement
   to patch against, so the expected outcome is a corrected comment, not a deletion.

### Explicitly rejected

Tagging, mirroring, vendoring, or any release-versioning scheme for the fork.

### Done when

`cargo metadata` (or the lockfile) shows the rev-pinned source for both crates, the
workspace still builds, and no other dependency changed in `Cargo.lock`.

---

## A2 — `--locked` in CI; `deny.toml` header

**Confirmed at HEAD:** no `cargo build`/`test`/`clippy` step in any workflow passes
`--locked`; the only `--locked` occurrences are on `cargo install` of tooling.

### Behaviour required

1. Every cargo invocation in every workflow that *builds or tests this workspace* gains
   `--locked`. In scope: `checks.yml`, `macos-verify.yml`, `render.yml`, `roundtrip.yml`,
   `perf-gates.yml`, `release.yml`, plus any cargo call reached indirectly through a
   script those workflows run.
2. `cargo install` of external tooling keeps its existing `--locked` (that flag means the
   tool's own lockfile; it is already correct).
3. The functional consequence: a PR that edits a dependency in `Cargo.toml` without
   committing the regenerated `Cargo.lock` **fails CI** instead of silently building
   against a different graph than `cargo deny` audited.
4. `deny.toml`'s header comment is corrected: it still describes a GPL license exception
   that was replaced by `exceptions = []` + the vendored no-op stubs.

### Edge cases

- If any workflow step legitimately *needs* to update the lock (none is expected), it is
  exempted with an inline comment saying why.
- `--locked` fails the build if `Cargo.lock` is stale. This unit must therefore confirm
  the committed lock is current at HEAD before landing, or it turns CI red on merge.
  Confirming this is part of the unit, not a follow-up.

### Out of scope

A `cargo tree -i zlog` GPL-reintroduction guard — considered and dropped; `cargo deny`
already covers it.

### Done when

Every workspace-building cargo step carries `--locked`, `cargo build --locked
--workspace` succeeds locally at HEAD (proving the lock is current), and the `deny.toml`
header matches its actual configuration.

---

## A4 — Correct the fork docs; state the real fork strategy

Docs only. No code, no manifest semantics (the `rev` edit belongs to A1).

### Behaviour required

1. **Establish the true state from the fork's history**, not from any existing status
   table: what commits are on `freecell-fixes` beyond upstream `main`, and for each, its
   upstream status (merged / open PR / fork-only). The overview states plainly that the
   review's "nothing has been upstreamed" claim is **wrong** — fixes have been submitted
   and merged. The inventory must establish this from evidence.
2. Correct `app/Cargo.toml`'s `[patch.crates-io]` comment, which names two fixes while
   the branch carries substantially more (including merged-cells support and the
   `set_user_inputs` batched-write API).
3. Correct the status table in `specs/projects/ironcalc-upstreaming/`.
4. Write down the **actual standing strategy**, in `CLAUDE.md` §Engine and
   `projects/ironcalc-upgrade.md`:
   > We keep the fork permanently. We keep upstreaming fixes as clean single-fix PRs. We
   > keep re-syncing the fork from upstream `main`.
   Both documents currently frame the fork as a temporary state with an exit
   (`Cargo.toml`'s comment literally says "TEMPORARY: revert to a released crates.io pin
   once the fixes ship"). That framing must go.

### Constraint

The fork lives at `scosman/ironcalc`, a separate repo. If it cannot be read from this
container, the unit degrades to: state what *can* be established, mark the inventory
explicitly incomplete, and do not fabricate merge statuses. A guess presented as an
inventory is worse than the current wrong comment.

### Done when

The manifest comment, the upstreaming status table, `CLAUDE.md` §Engine and
`projects/ironcalc-upgrade.md` all describe the same, evidence-backed, permanent-fork
strategy, and no document still claims the fork is temporary or that nothing has been
upstreamed.

---

## C1 — Part-inventory round-trip test ⭐ keystone

### Behaviour required

A new test (in `app/crates/freecell-engine/tests/`) that, for each real-Excel fixture in
the tree:

1. **Opens** the fixture through the production open path.
2. **Saves** it to a temp path through the production save path, with **no edits** (the
   pure-preservation case) and, where cheap, with one trivial cell edit (the realistic
   case).
3. **Reopens** the saved file and asserts it opens without error.
4. Asserts a **part-level inventory** comparison of the two zips: the set of part names
   in the original vs. the set in the saved file.

### The assertion's shape

- The comparison is **name-level**, not byte-level. Byte equality is not the goal and is
  not achievable; the question this test answers is *"what did we drop on the floor?"*.
- Parts that FreeCell legitimately rewrites or that OOXML permits to differ (e.g.
  `docProps/*` timestamps, calc chain) must not be reported as loss — but the exclusion
  list is **explicit and small**, each entry justified inline. A broad exclusion defeats
  the test.
- Failure output must **name the dropped parts**. A bare `assert_eq!` of two sets is a
  usable but poor diagnostic; the test's whole value is telling a human exactly what is
  missing.

### Expected outcome

**The test is expected to fail on first run, loudly.** That is the deliverable, not a
problem. This unit therefore has a mandatory decision:

- The test must land in a state where **CI is green**. It cannot land as a red required
  check.
- Resolution: the test asserts against a **committed, reviewed expectation** of currently
  dropped parts — i.e. the known loss is encoded as a baseline the test pins. Adding a new
  dropped part fails; the existing loss is recorded, visible, and named in the file.
  This makes the loss non-regressing (the C1 goal) without shipping a red gate or
  pretending the loss is acceptable.
- The measured loss is written into `GAPS.md` at the tier the remediation doc assigns
  (the *fix* is C3, v1.0; the *warning* is C2, next round).

### Fixtures

All real-Excel fixtures under `app/crates/freecell-engine/tests/fixtures/`, starting with
`personal_monthly_budget.xlsx`. Fixtures that are FreeCell-authored are not in scope —
they are the closed loop this unit exists to escape.

### Out of scope

Fixing any dropped part. C1 is a detector.

### Done when

The test exists, runs in the normal `cargo test -p freecell-engine` path, passes at HEAD
against a committed baseline, fails if a new part starts being dropped, and `GAPS.md`
records what the baseline proves is being lost.

---

## D1 — Render gate weekly on `main` + at release

**Confirmed at HEAD:** `render.yml` is `on: workflow_dispatch:` only, while its header
says it "MUST be a required status check".

### Behaviour required

1. `render.yml` gains:
   - a **weekly `schedule:`** trigger, running on `main`;
   - a trigger that makes it **run at release** — either by `release.yml` invoking it, or
     by a trigger on the same event `release.yml` uses. The architecture step picks the
     mechanism; the functional requirement is *nothing ships without a green pixel suite*,
     and if the render run fails, that must be visible on the release, not buried.
   - `workflow_dispatch` retained unchanged for agent-driven runs.
2. `render.yml`'s header stops claiming it is a required status check and describes the
   real mechanism.
3. **Docs this invalidates are fixed in the same phase:**
   - `app/README.md` §CI and the `checks.yml` header, both of which claim `checks` runs
     the render suite;
   - `CLAUDE.md`'s multi-paragraph "the agent must decide when to run it" section,
     rewritten around the real mechanism — including that `main` now has a scheduled
     backstop, so a missed agent-driven run is no longer *nobody* checking.

### Constraints

- Scheduled workflows only run from the default branch — the schedule is inherently on
  `main`, which is what is wanted.
- The weekly run must not be silently ignorable. If the suite fails on schedule, the
  failure has to be discoverable (GitHub's default failed-scheduled-run notification to
  the repo owner is acceptable; nothing more elaborate is in scope).
- The release path must not become able to *hang* a release for many minutes without that
  being an intentional, documented cost.

### Explicitly rejected

Bump-token, `paths` filter, per-PR gate.

### Done when

`render.yml` runs weekly on `main` and at release, still dispatches manually, and no
document in the repo claims either that `checks` runs the render suite or that `render`
is a required per-PR check.

---

## F3a — Chart-axis number formatting must agree with cells

### Behaviour required — the test is the primary deliverable

1. Build a **differential test over a format-code corpus** asserting that
   `freecell-chart-model`'s `numfmt` and IronCalc's formatter produce the **same string**
   for the same (format code, value) pair.
   - The test lives where it can see both — i.e. in `freecell-engine` (which depends on
     both ironcalc and chart-model), not in `chart-model` (which must stay ironcalc-free).
     Preserving that crate seam is a hard constraint; the test must not violate it.
   - The corpus covers at minimum: the format codes actually reachable on a chart axis
     from the fixtures in-tree, the general format, `#,##0.00` and its family,
     percentages, currency, scientific, dates/times, negative-number sections, and text
     sections. Values must include negatives, zero, very large/small magnitudes.
     > **Annotation (2026-07-28, post-code-review — this section is `status: complete` but this
     > requirement was too weak.)** "`#,##0.00` and its family" was read as the *required-digit*
     > family and the **`#` optional-digit** codes (`0.##`, `#,##0.##`, `#,##0.0#`, `0.###`,
     > `#,###`) were left out, as was any whitespace-padded code. `renders_faithfully` accepts all
     > of them, so the gate certified them Faithful without ever evaluating them — and that hid a
     > real `chart-model` defect. The binding rule is: **the corpus must cover every code shape the
     > faithfulness predicate accepts**, and the value set must contain points the carve-out
     > predicates do *not* explain. See `phase_plans/phase_6.md`.
2. **Only then** decide the fix size. Three outcomes, all acceptable:
   - **They agree** on everything reachable → *the test is the deliverable*; record that
     the divergence claim was overstated, and correct the remediation doc.
   - **They disagree on a few codes** → fix `chart-model`'s implementation to match
     IronCalc for those codes. IronCalc is the reference; the cells are what the user
     compares against.
   - **They disagree structurally** → do not attempt the crate merge (that is F3). Fix
     what is reachable, file the rest.
3. **`rgb_to_hsl` divergence**: the review located the copies in `core` and `chart-model`;
   at HEAD they are in `freecell-app/src/chart/palette.rs` and
   `freecell-chart-model/src/theme.rs`. The unit must confirm the `.rem_euclid` vs `%`
   difference actually changes an output, with a test, before changing anything. Same for
   the two Office-palette definitions: assert they are equal; if they are, the assertion
   is the fix.

### Non-goals

Merging the crates. Changing the crate dependency graph. Making `chart-model` depend on
ironcalc.

### Done when

A differential test exists and passes, any confirmed disagreement in reachable format
codes is fixed or explicitly filed, and the colour-helper duplication is either proven
equivalent (with a guard test) or reconciled.

> **Annotation (2026-07-28, post-code-review).** "Exists and passes" is not sufficient on its own:
> a differential test passes trivially if its carve-outs are broad enough. Phase 6's first pass met
> this bar with a green gate over a live `chart-model` bug. Read it as: **passes, with every
> carve-out narrow enough that it cannot also match a defect on our side of the comparison, and
> with counts reported.** On the colour half, "reconciled" was the outcome architecture §6 actually
> prescribed (export **and** delete); a guard test over a duplicate that could simply be deleted is
> the weaker option, not an equal one.

---

## G1 — Detect multi-group (combo) charts

### Behaviour required

1. **Confirm** that `parse_chart_xml` retains only the first chart-group child of
   `c:plotArea`, and that `source_fidelity` does not count groups — by parsing a real
   multi-group chart XML and observing the result, not by reading the parser.
2. When a `c:plotArea` contains **more than one chart-group element**, the resulting
   chart's fidelity is forced to `Fidelity::Degraded` with a reason that names the
   dropped groups. The user sees the existing degraded-fidelity badge; the chart is not
   silently presented as faithful.
3. `is_extended_chart`'s bare `contains("chartex")` is fixed — a substring test over
   arbitrary XML text can match content that is not an extended-chart declaration. It must
   test the actual condition (namespace/part-type), and the confirmation step must show
   the bare version can produce a wrong answer.
4. `GAPS.md`'s combo-chart row is corrected: it currently describes placeholder behaviour
   that does not happen. It should describe what actually happens (first group only,
   flagged Degraded after this unit) at the honest tier, and a v2.0 entry for real combo
   support (G1b) is added or updated.

### Explicit non-goal

Rendering the additional groups. This unit makes the failure **honest, not correct**.

### Edge cases

- A plot area with one group plus non-group children (axes, `c:dTable`, `c:spPr`) must
  **not** be flagged. The count is of chart-group elements specifically.
- Fidelity must not be *downgraded from* something worse — if the chart is already
  `Unsupported`, detecting combo must not upgrade it.

### Done when

A multi-group chart parses to `Degraded` with a naming reason, a unit test covers it,
`is_extended_chart` tests the real condition, and `GAPS.md` describes reality.

---

## G5 — `dLbls` overrides (v0.5) + chart-insert collisions (v1.0)

### In scope for this round — the `dLbls` data-loss fix

1. **Confirm** that editing data labels replaces the whole `c:dLbls` node, and that this
   destroys per-point overrides (`c:dLbl` children) and label typography (`c:txPr`,
   `c:spPr`, `c:numFmt`) — by round-tripping a chart XML that carries such children and
   observing them gone.
2. The save path must **preserve unknown/unmodelled children of `c:dLbls`** while applying
   the modelled edit — matching the preserve-unknown discipline the rest of the patcher
   already follows. Concretely: the fields FreeCell models are updated in place; every
   other child element is carried through byte-preserved.
3. A regression test asserts a `c:dLbls` carrying per-point overrides survives an edit to
   a modelled label field.

### Out of scope for this round — chart-insert collisions

Inserting a chart onto a sheet that already carries one returns a hard `SaveError`. That
is **v1.0** per the remediation doc. This unit confirms the behaviour, files it in
`GAPS.md` if it is not already there accurately, and **ships the `dLbls` fix alone**.

### Done when

Per-point label overrides and label typography survive a data-label edit, a test pins it,
and the insert-collision limitation is recorded rather than fixed.

---

## Out of scope for the project as a whole

- C2 (save-fidelity warning dialog), C3 (preservation model), D2 (pixel tolerance),
  F2 (line ceiling), F3 (crate merge), G1b (combo support), G3 (fidelity classifier),
  H1/H3 — later rounds.
- B1/B2 — owned by engine-worker-hardening.
- F1 chrome split — owned by chrome-view-split.
- Any IronCalc fork change. If a unit needs one, it is one `fix/<slug>` branch, one
  upstream PR, per `CLAUDE.md` §Engine — and it is flagged, not folded into this round.
