# Architecture-Review Remediation

**Status:** Proposed (2026-07-28) — plan of record for closing the findings of the
whole-codebase architecture review. Revised 2026-07-28 after owner review.

Source: an 8-phase fresh-eyes architecture review of `app/` at `e4c7afa` (~99k LOC,
five crates), run as independent sub-agents over crate boundaries, engine concurrency,
UI architecture, charts, persistence, testing, and build/shipping posture, plus a
synthesis verdict. Findings: **22 critical, 55 moderate, 39 mild.**

The full review — eight phase reports with per-finding `file:line` evidence, plus the
consolidated summary and verdict — is committed at
[`reviews/projects/codebase-architecture-review/`](../reviews/projects/codebase-architecture-review/)
(force-added past `.gitignore` so the detail survives; `reviews/` remains ignored for
future runs). Start with
[`phase_8_verdict.md`](../reviews/projects/codebase-architecture-review/phase_8_verdict.md).
This document nonetheless stands alone — it inlines the load-bearing evidence for every
unit.

Reviewers were instructed to treat `specs/`, `experiments/`, `GAPS.md` and `CLAUDE.md`
as evidence of *intent* only, never as justification.

---

## How to use this document — read before picking up a unit

**Every finding below is a hypothesis from a broad research agent, not an established
fact.** The reviewers were fast and wide; they read a lot of code and did not compile or
run any of it. Several claims have already been corrected by the owner on the first
pass, and at least one was found stale on re-check.

So the first phase of every unit — task or project — is **confirm the root cause still
exists, at HEAD, in the code you are about to change.** Concretely:

- Re-derive the problem from the source. Do not cite this document as evidence; cite the
  code. Line numbers here are from `e4c7afa` and drift.
- Go *deeper* than the research agent did. The claims are broad by construction; you own
  one unit and can afford to trace the actual path, read the surrounding history
  (`git log -p` on the file often shows the fix already landed), and check whether an
  adjacent fix already closed it.
- **Push back.** If the root cause does not exist, is already fixed, is overstated, or
  the proposed direction is wrong for reasons visible only up close, say so and stop.
  Write down what you found and why the unit should be dropped, narrowed, or reshaped.
  A unit closed as "not a real problem, here's the evidence" is a successful outcome,
  not a failed one.
- If the problem is real but *different* from the description, fix the real one and
  correct the entry here.

The one thing not to do is implement a fix for a problem you have not personally
confirmed.

---

## Verdict, in one paragraph

Good code with a bad relationship to the truth about itself. The crate seam, the
`ArcSwap` publication, `Axis` + the O(populated) read queries, the byte-splicing chart
save patcher, the atomic-save mechanics and 22 negative controls in the test suite are
genuine engineering. The architecture is sound enough to keep building on: **nothing
needs tearing out**, two subsystems need redoing (save preservation, UI state
ownership), and the loudest problem — `chrome/view.rs` — is a mechanical split and is
*not* in the top five most dangerous things in the repo.

## The pattern behind the findings

> **The project consistently mistakes having *reasoned about* an invariant for having
> *enforced* it.**

Sort every finding by that axis and it partitions cleanly.

**Enforced by a mechanism → healthy, in every case:** crate boundaries (cargo + a guard
test with negative controls); IronCalc containment (`pub(crate)`); command dispatch
completeness (exhaustive match); publication ordering (a concurrent test with a
non-zero-sample assertion); zero engine calls on the render path (a process-global
counter with a negative control); the GPL removal (verified against `Cargo.lock`).

**Asserted in prose → failed, in every case:**

| Claim | Reality |
|---|---|
| *"the bands are a few leading tracks, never a sheet-size loop"* | the comment directly above the unbounded loop that wedges the worker (B2) |
| *"the worker is the only writer"* | `autogrow_measure_now` takes `caches.write()` from the UI thread in the shipped binary |
| *"this MUST be a required status check"* | `render.yml` is `workflow_dispatch:`-only; zero runs on `main`, ever (D1) |
| *"at most one popover is open"* | eight independent `bool`s, holding only because each popover paints an occluding backdrop |
| *"the undo stacks stay 1:1"* | already violated by `SetFrozen` sending two engine history entries against one worker touch |
| *"P8 threads the actual workbook `clrScheme`"* | P8 shipped; nothing threads it — charts still hardcode the Office palette (G2) |

Two corollaries worth acting on:

- **Zero `TODO`/`FIXME` markers in 99k lines.** Debt is not recorded in code; it is
  externalized into `GAPS.md`, `specs/` and `projects/` — the artifacts that cannot
  fail a build. `GAPS.md` being wrong about combo charts is the predicted outcome, not
  an anomaly.
- **Failures cluster at the boundaries *between* well-built things.** The publication is
  excellent and the three surfaces added beside it have no shared commit point;
  `catch_unwind` is excellent and the two calls added outside the pattern are unguarded;
  the save patcher is excellent and `dLbls` is the one field patched by whole-node
  replacement. Each mechanism enforces itself and nothing adjacent to itself.

The good news: this failure mode is **mechanically fixable and requires no redesign**.

---

## Unit index

Size is **task** (small, one phase) or **project** (multi-phase). Targets follow the
`GAPS.md` v0.5 / v1.0 / v2.0 convention, extended with `v3+` for genuine reach items.

| # | Title | Size | Target | Waits on |
|---|---|---|---|---|
| A1 | Pin the IronCalc fork by SHA | task | v0.5 | — |
| A2 | `--locked` on every cargo invocation in CI | task | v0.5 | — |
| A4 | Correct the fork docs; state the real fork strategy | task | v0.5 | — |
| B1 | Guard load/save with `catch_unwind`; surface worker death | task | v0.5 | — |
| B2 | Clamp the frozen-pane band; validate `SetFrozen` | task | v0.5 | — |
| B3 | Cache lock hold-time | task | **v3+** | — |
| B4 | Fuzz `WorkbookDocument::open` | task | **v2.0** | B1 (pref) |
| C1 | Part-inventory round-trip test | task | v0.5 | — |
| C2 | Save-fidelity warning dialog | task | v0.5 | **C1** |
| C3 | Default-carry / explicit-drop preservation model | project | v1.0 | **C1** |
| C4 | Comment write-back | task | v1.0 | C1 (pref) |
| D1 | Render gate weekly on `main` + at release | task | v0.5 | — |
| D2 | Tighten pixel tolerance; add exact-match text cases | task | v0.5 | D1 (pref) |
| ~~D3~~ | ~~Platform-gated PR compile check~~ | — | **dropped** | — |
| D4 | Measure real frame cost, not element construction | project | v1.0 | — |
| D5 | Scale tests on the I/O path | project | v1.0 | C3 (pref) |
| E1 | One commit point for the four shared surfaces | project | **v0.5** | — |
| E2 | Single source of truth for UI state | project | v1.0 | F1 (strong pref) |
| F1 | Split `chrome/view.rs` and `worker/run.rs` | project (small) | v0.5 | — |
| F2 | Production-line ceiling in CI (2,000) | task | v0.5 | **F1** |
| F3a | Chart-axis number formatting must agree with cells | task | **v0.5** | — |
| F3 | Fold `freecell-chart-model` onto `freecell-core` | project | v1.0 | F3a · coord. G3 |
| F4 | Collapse the worker protocol | project | v2.0 | E2 (pref) |
| G1 | Detect multi-group (combo) charts | task | v0.5 | — |
| G1b | Combo / dual-axis chart support | project | v2.0 | G1 |
| G2 | Thread the workbook theme into charts | task | v1.0 | — |
| G3 | Fidelity classifier off the text scanner (in the engine) | project | v1.0 | **G1** |
| G5 | `dLbls` overrides + chart-insert collisions | task | v0.5 / v1.0 | — |
| H1 | Invariant-enforcement sweep | project | v0.5 | B2, F1 (pref) |
| H3 | Update `CLAUDE.md` to prevent this class of defect | task | v0.5 | H1 (pref) |

**Bold** = required ordering. *(pref)* = preferred, not blocking. **↓** = open question,
see the unit body.

*Retired from earlier drafts:* **A3** (GPL reintroduction guard — already covered by
`cargo deny`); **D3** (platform-gated PR compile check — not worth the CI weight); **G4**
(theme parsing — already done; folded into G2); **H2** (doc reconciliation — folded into
the units that make each doc wrong, per below).

**Doc corrections are not a unit.** Every unit that invalidates a doc claim fixes that
claim as part of its own scope: D1 fixes `app/README.md` §CI and the `checks.yml` header
plus the `CLAUDE.md` render process; G1 fixes the `GAPS.md` combo row; G2 fixes the
`chart/style.rs` comment; A4 fixes the manifest comment and the upstreaming status
table; A2 fixes the `deny.toml` header's stale GPL-exception reference; H3 picks up the
orphan from dropped D3 (the "macOS (primary)" claims, which should read as cross-platform).

---

## A. Supply-chain & build integrity

### A1. Pin the IronCalc fork by SHA
**task · v0.5 · no dependencies**

`app/Cargo.toml` patches `ironcalc`/`ironcalc_base` to `branch = "freecell-fixes"`,
whose documented maintenance procedure (`CLAUDE.md` §Engine) is *rebasing*. One
force-push + GC makes every historical FreeCell commit unbuildable, including any tag
needed for a hotfix — while every other git dep in the tree, including four of zed's own
forks, is `rev`-pinned. Replace `branch` with `rev = "<sha>"`.

Deliberately **no** tagging, mirroring or vendoring scheme — that is versioning
complexity for little marginal safety over a SHA, and it would slow the fork loop down.
Bumping the pin is a one-line edit when the fork moves.

Also check the `ironcalc = "=0.7.1"` version line while in the file: FreeCell migrated to
the post-0.7.1 style-colour API, so if that pin is still there it is inert at best and
misleading at worst.

### A2. `--locked` on every cargo invocation in CI
**task · v0.5 · no dependencies**

No workflow builds with `--locked`, so the lockfile `cargo deny` audits is not provably
the lockfile that ships — including in `release.yml`, which produces signed binaries. A
PR that edits a dependency in `Cargo.toml` without regenerating the lock never fails.
One flag per step across the workflows.

While in `deny.toml`: its header still references a GPL exception that was deliberately
replaced with `exceptions = []`. Fix the comment.

*Note: the GPL reintroduction case is already covered — `cargo deny` runs on every push
and PR under an `app/**` paths filter, with GPL absent from `allow` and `exceptions = []`,
so a reappearing GPL crate fails the license gate. No extra guard needed.*

### A4. Correct the fork docs; state the real fork strategy
**task · v0.5 · no dependencies**

Docs-only. The review flagged a discrepancy between what `app/Cargo.toml`'s comment says
the `freecell-fixes` branch contains (it names two fixes) and what is actually merged
there — the branch carries substantially more, including at least one whole feature
(merged cells) and the `set_user_inputs` batched-write API the paste/replace path depends
on. **The review's further claim that none of these are upstreamed is wrong** — fixes have
been submitted and merged upstream. Do not repeat that claim; establish the true state
first.

Scope:
1. Inventory what is actually on `freecell-fixes` and what its upstream status is
   (merged / open PR / fork-only), from the fork's history rather than from any status
   table.
2. Correct the `Cargo.toml` comment and the
   [`specs/projects/ironcalc-upstreaming/`](../specs/projects/ironcalc-upstreaming/)
   status table to match.
3. Write down the actual standing strategy, which is not "eliminate the fork": **we keep
   our fork permanently; we keep upstreaming fixes as clean single-fix PRs; we keep
   re-syncing the fork from upstream `main`.** The fork is a normal operating position,
   not a temporary state with an exit. `CLAUDE.md` §Engine and
   [`projects/ironcalc-upgrade.md`](ironcalc-upgrade.md) currently imply the latter.

---

## B. Crash & hang safety

### B1. Guard load/save with `catch_unwind`; surface worker death
**task · v0.5 · no dependencies**

Six mutation regions are guarded; `from_source` and `save_workbook` are not — and the
pinned exporter contains a reachable `panic!("Model needs to be evaluated before
saving!")`. Because the `JoinHandle` is discarded and `send` swallows `SendError` by
design, a panic on the file path produces a **silent zombie**: the window keeps rendering
the last publication, edits vanish, Save does nothing, and there is no dialog, no degraded
bar, and no log entry. Wrap both calls into `LoadFailed`/`SaveFailed`, retain the handle,
and make the window treat "event stream ended without a requested `Shutdown`" as fatal and
say so.

*Found independently by the concurrency and persistence reviewers from opposite
directions — it looks like a threading bug from one side and a persistence bug from the
other, and is neither.*

### B2. Clamp the frozen-pane band; validate `SetFrozen`
**task · v0.5 · no dependencies**

Every loop in `build_publication` is bounded by a constant except the frozen-pane band,
which iterates `(0..m)×(0..k)` from unvalidated `frozen_rows`/`frozen_cols`. The UI
computes the count as "last row of the header run you right-clicked", so freezing at row
500,000 makes every subsequent publish do ~500,000 × 256 `formatted_value` calls and the
worker never returns — two clicks to a permanent hang, also reachable from a crafted
`.xlsx` `<pane>` element. Clamp at the publish site *and* range-validate in
`pre_validate`, closing both the click path and the file path.

### B3. Cache lock hold-time
**task · v3+ · no dependencies**

`refresh_cache_cells` holds the cache write lock across up to 100,000 IronCalc reads while
the render thread takes that same lock 23 times per frame. Chunk the refresh, or build
off-lock and swap.

*Deferred to v3+ by owner call: perf is validated by the existing harness and this is a
theoretical contention win, not an observed one. Revisit only if a real measurement (see
D4) shows lock contention on the frame path.*

### B4. Fuzz `WorkbookDocument::open`
**task · v2.0 · after B1 (preferred)**

The file parser is the only untrusted input surface in the app and has no fuzzing. Add
`cargo-fuzz` over the open path with a seed corpus from the existing fixtures. Catches the
class of panic that B1 only *contains*, so sequence after B1 — findings then surface as
errors rather than zombies.

---

## C. Save fidelity — the critical path

> The most dangerous defect in the repo is here. Existing design note:
> [`projects/xlsx-preservation.md`](xlsx-preservation.md) — these units supersede its
> sequencing with a test-first order.

### C1. Part-inventory round-trip test
**task · v0.5 · no dependencies · KEYSTONE**

`tests/roundtrip.rs` only round-trips workbooks FreeCell itself authored — a closed loop
over IronCalc's own serializer — which is why unbounded data loss has been invisible for
the project's entire life. Meanwhile five real Excel fixtures sit in the tree used for
*open* assertions only. Add open→save→reopen over `personal_monthly_budget.xlsx` and
siblings, asserting a **part-level inventory** of both zips. Build the detector before the
fix so the fix is measurable and non-regressing.

*C2, C3, C4 and D5 all sit behind this. If only one unit starts, start here.*

### C2. Save-fidelity warning dialog
**task · v0.5 · after C1 (required)**

Restore the warn-before-strip dialog cut on 2026-07-13 (`GAPS.md` line 451; the surviving
mitigation is a `.back` file with no UI, no menu entry and no mention in any dialog). At
save time, warn the user that content in the original file will not be preserved.

**Priority split:**
- **P1 — the warning exists at all.** Saving over a file that came from Excel tells the
  user, before it happens, that unmodelled content will be dropped. This is the shippable
  minimum.
- **P2 — enumerate exactly what is being dropped.** `reinject` already reads the original
  zip and enumerates parts, so a naive part-name list is close at hand; a *user-meaningful*
  list ("2 pivot tables, 14 comments, 1 macro module") is more work. **This unit is allowed
  to punt P2 to v1.0** — if the mapping from part names to user-facing nouns balloons the
  scope, ship P1, file a `GAPS.md` entry for the enumeration, and move on.

### C3. Default-carry / explicit-drop preservation model
**project · v1.0 · after C1 (required)**

`is_carry_part` allowlists exactly two prefixes (`xl/charts/`, `xl/drawings/`) over a
package IronCalc regenerates from scratch — it synthesizes an eleven-part zip from its
model rather than rewriting the input. So opening a real `.xlsx`, editing one cell and
saving permanently deletes pivot tables, macros, data validation, hyperlinks, comments,
Excel Tables, autofilters, sheet protection, print setup, images and in-cell rich text.
Allowlist-over-regenerated-package cannot be incrementally tightened into correctness; it
must invert to *carry everything IronCalc does not authoritatively regenerate*. The
`reinject` seam already does the hard parts (filter parts, merge `[Content_Types]`
overrides, patch worksheet XML). Stale beats deleted: this converts an unbounded loss into
a bounded, enumerable one.

### C4. Comment write-back
**task · v1.0 · after C1 (preferred)**

IronCalc *imports* `Worksheet.comments` but no exporter ever writes them back, so comments
are lost even in the model's own round-trip. Narrow and well-scoped — a good candidate for
a fork fix contributed upstream rather than a FreeCell workaround. May be subsumed by C3
if comments end up carried as parts.

---

## D. Verification integrity

### D1. Run the render gate weekly on `main` and at release
**task · v0.5 · no dependencies**

`render.yml`'s own header says it "MUST be a required status check"; its trigger is
`workflow_dispatch:`-only. 29 dispatched runs across 13 branches, **zero on `main`, ever**,
against 44 merged PRs — while failing on 7% of the runs it did get. So `main` has never
been render-verified.

The suite is genuinely expensive (software lavapipe, many minutes), and it cannot be worth
that cost on the many PRs that can't move a pixel. **Decided (owner, 2026-07-28): keep it
off the PR path entirely.** Add two triggers instead:

- a **weekly schedule on `main`** — catches anything that slipped through merges, at a
  cadence where a regression is still cheap to bisect
- a **run at release** — nothing ships without a green pixel suite

Keep `workflow_dispatch` for the agent-driven case: an agent making a rendering change
still runs the relevant subset locally and can dispatch the full suite on its branch when
it wants confirmation. What changes is that forgetting no longer means *nobody* checked.

No bump-token, no `paths` filter, no per-PR gate — those were considered and rejected as
ceremony that buys little over a weekly backstop.

Also in scope: fix `app/README.md` §CI and the `checks.yml` header (both claim `checks` runs
the render suite — it doesn't), and rewrite the multi-paragraph "the agent must decide when
to run it" process in `CLAUDE.md` to describe the real mechanism, including that `main` is
now covered by a schedule.

### D2. Tighten pixel tolerance; add exact-match text cases
**task · v0.5 · after D1 (preferred)**

`fail_fraction: 0.005` permits 384–1,596 differing pixels in scenes whose entire
non-background content is 8k–25k pixels and whose glyph ink is on the order of 74 pixels —
a wrong digit in a cell passes silently. Because `#[gpui::test]` installs a
`NoopTextSystem`, this suite is the *only* place real font metrics are ever exercised, so
**nothing in the project currently validates that the right number is drawn correctly**.
Tighten for text-centric cases (`cell_*`, `spill_*`, `autogrow_*`); lavapipe has 29 runs of
demonstrated stability, and `diff.rs`'s own comment says to tighten when it does.

**Don't force it.** If tightened thresholds prove flaky across runners or driver versions,
back off rather than fighting it — a flaky gate is worse than a loose one. Land whatever
tightening holds cleanly, record the rest in `GAPS.md`, and stop.

### D3. ~~Platform-gated code has no PR compile check~~ — DROPPED
**retired 2026-07-28 (owner call)**

The review's premise ("macOS is the stated primary platform, gated weekly") is dropped:
FreeCell is cross-platform, and Linux CI is the better default gate.

The narrower version — that ~250 lines behind `#[cfg(target_os = "macos")]`
(`shell/default_app.rs` CoreFoundation/LaunchServices FFI, `shell/open_files.rs` Apple-Event
bridge) are compiled by no PR — was considered and **rejected as not worth the cost**:
platform-specific branches are expected to be exercised when the PR that touches them is
written and merged, and release builds cover all three OSes. Adding CI weight to 99.9% of
PRs that contain no platform code is the wrong trade.

*The one residue is a doc fix: the repo still calls macOS "primary" in four places
(`app/README.md:8`, `specs/projects/mvp/architecture.md:53`, `functional_spec.md:21`, the
`macos-verify.yml` header). Folded into H3.*

### D4. Measure real frame cost, not element construction
**project · v1.0 · no hard dependencies**

The 8.33 ms perf gate measures `resolve_frame` + `build_grid_layers`. There is **not one
custom `Element` impl in `grid/`** — the grid hands ~2,000 absolutely-positioned `div`s to
taffy every frame, and that cost, plausibly the dominant term, is never measured. A
rigorously-executed measurement of the wrong quantity is worse than none, because its rigor
makes the number unquestioned. Needs an end-to-end frame measurement, a re-derived budget,
and trend tracking rather than a flat 2× threshold. Worth doing before any perf-motivated
refactor, since its result may reframe B3/E1/F1 priorities.

### D5. Scale tests on the I/O path
**project · v1.0 · after C3 (preferred)**

Excel-max is the product thesis and there is no scale test on load or save: six committed
fixtures, largest 35 KB, and the 1M-row perf fixture is synthesised in-process and never
serialised. Parse time, peak memory, shared-string blowup and save time at scale are all
unmeasured. Needs a generated large-workbook fixture (committed or reproducibly built) plus
gates on parse/save time and peak RSS. Sequenced after C3 — no point baselining save cost
on a model about to change.

---

## E. State & consistency architecture

### E1. One commit point for the four shared worker→UI surfaces
**project · v0.5 · no blockers**

**Promoted to v0.5 (2026-07-28).** This is not a scale problem, it is a correctness problem
reachable today, and it gets cheaper the sooner it happens — every additional shared surface
is one more thing to unify later.

The `ArcSwap` publication was designed as a seam; the style cache, chart snapshot and CF
map were each added beside it with their own primitive and their own (or no) version
discipline. The commit order emits `Published` *between* the value commit and the style
commit, and `commit_chart_op` emits `Published` without publishing — so there is no answer
to "what does the UI see at generation N", which is the question you must be able to answer
to reason about this seam at all. Unify under one generation counter and one commit point;
add ordering tests with non-zero-sample assertions.

### E2. Single source of truth for UI state
**project · v1.0 · after F1 (strongly preferred)**

Selection lives in three places (`GridView.selection`, `ChromeView.selection`,
`SinkShared.last_selection`), active sheet in four, pending-edit text in four
representations, and ten `GridView` fields are hand-pushed mirrors of chrome state through
a nine-positional-argument `set_edit_state`. This forced the cyclic
`Rc<OnceCell<WeakEntity>>` grid↔chrome wiring and a `window.defer` convention documented
across five "BUG #5" comments, and it has already produced a self-documented
wrong-sheet-write hazard at `shell/window.rs:2129`. One observed document-view-model that
both views read dissolves the mirrors, `SinkShared`, the setter and most of the re-entrancy
class at once.

***Freeze rule while it waits: no new mirrored fields.*** This unit gets strictly harder
every phase that adds one — there are already ten, and the v1.0 tier is only defensible if
that number stops growing. If the freeze can't hold, promote this to v0.5, because waiting
will only cost more.

---

## F. Structure & maintainability

### F1. Split `chrome/view.rs` and `worker/run.rs`
**project (small, 2–3 phases) · v0.5 · no dependencies**

`chrome/view.rs` is 16,099 lines — **8,249 production** — one struct with 71 fields, 267
methods, 91 `cx.listener` closures, spanning eight independent feature domains (CF sidebar
~1,510 lines, formatting ~1,450, sheet tabs ~1,350, edit/formula ~990, charts ~790, find,
stats, shell). With `grid/view.rs` (10,627) that is **55% of the app crate's production
code in two files**. The growth curve is the damning part: 5,618 → 16,099 across 34 commits
in sixteen days, monotonically, with exactly one net-negative commit — because nothing
resists it. Rust's module-descendant privacy means a `chrome/view/` directory needs **zero
visibility changes** to the 71 private fields, so this is mechanical. Same for moving
`worker/run.rs`'s ~1,200 chart lines into the already-existing, currently-empty 39-line
`worker/charts.rs`.

*This is the **loudest** problem and one of the **least dangerous**. Do it — it is the top
velocity and bus-factor problem — but do not let its volume displace B and C.*

### F2. Production-line ceiling in CI
**task · v0.5 · after F1 (required)**

Nothing has ever resisted file growth, which is why the curve is monotonic. Add a CI check
on per-file **production** line count (excluding `#[cfg(test)]` blocks — several files are
40–50% inline tests and that is fine) with a ceiling of **2,000**. Verify after F1 that
every file actually lands under it; anything that can't be split cleanly gets an explicit,
listed exemption rather than a raised global ceiling.

### F3a. Make chart-axis number formatting agree with cells
**task · v0.5 · no dependencies**

**Split out of F3 (2026-07-28)** — the user-visible bug shouldn't wait on the structural
refactor.

`chart-model/src/numfmt.rs` is a 383-line reimplementation of OOXML number formatting that
exists only because `chart-model` must stay ironcalc-free. It races IronCalc's
implementation, so the *same format code* can produce two different strings: `#,##0.00` on a
chart axis and on the cells that chart plots go through different code. Users see the
disagreement.

Same class, same unit: the drifted `rgb_to_hsl` copies between `core` and `chart-model`
(`.rem_euclid` vs `%` — they disagree on negative hues), and the two definitions of the
Office palette.

Fix the *agreement*, not the architecture — a shared implementation, a delegation, or at
minimum a differential test over a format-code corpus asserting the two agree. The crate
merge that removes the duplication for good is F3.

*Confirm first: the review asserted the divergence from reading both implementations, not
from running them. Build the differential test before deciding how big the fix is — it may
turn out they agree everywhere that matters, in which case the test is the deliverable.*

### F3. Fold `freecell-chart-model` onto `freecell-core`
**project · v1.0 · after F3a · coordinate with G3**

Two zero-dependency foundation crates that do not know each other exist, which produced the
byte-identical duplicate `Color`/`Rgb`, the drifted `rgb_to_hsl`, the two Office palettes and
the duplicate number formatter behind F3a. A purity boundary is not free: it charges rent in
reimplementation.

This is the structural fix that stops the class from recurring. Sequenced after F3a (which
removes the user-visible pain) and coordinated with G3 (which moves the classifier *out* of
`chart-model` and may shrink what's left to fold).

### F4. Collapse the worker protocol
**project · v2.0 · after E2 (preferred)**

53 `Command` + 23 `WorkerEvent` variants mirroring UI menus one-for-one, several carrying
UI concerns into the engine outright — column widths in *device pixels*,
`AutoGrowRowHeights` shipping render-thread text measurements into the worker. Adding one
feature is a six-site change. The exhaustive matches keep it safe; they do not keep it
small. Partly a symptom of E2's state sprawl, so sequence after it.

---

## G. Chart correctness

### G1. Detect multi-group (combo) charts
**task · v0.5 · no dependencies**

`parse_chart_xml` takes only the *first* chart-group element in `c:plotArea` and discards
the rest, while `source_fidelity` never counts groups — so an ordinary Excel bar+line combo
loads as bars only, the line series **absent from the picture**, classified `Faithful`, and
drawn with no badge. Count group children at parse time and force `Fidelity::Degraded`.
Also fix `is_extended_chart`'s bare `contains("chartex")`, and correct the `GAPS.md` combo
row, which currently claims placeholder behaviour that does not happen.

*This makes the failure honest, not correct. Actual support is G1b.*

### G1b. Combo / dual-axis chart support
**project · v2.0 · after G1**

Render multi-group charts properly: multiple chart groups in one plot area, per-group
types, and secondary value axes. Bar+line with a secondary axis is an ordinary Excel
construct, not an exotic one, so this is a real feature gap rather than a fidelity
footnote. Add a `GAPS.md` entry at the v2.0 tier.

### G2. Thread the workbook theme into charts
**task · v1.0 · no dependencies**

**Downgraded after verification (2026-07-28). The review's framing was wrong on the
premise and wrong on the severity** — a good example of why the "confirm the root cause"
rule at the top of this document exists.

What the review claimed: the workbook theme is never parsed, and several call sites write
guessed RGB with tint stripped back into the user's file — "silent file corruption."

What is actually true at HEAD:

- **The theme *is* parsed.** The fork's `xlsx/src/import/theme.rs` reads `clrScheme` into a
  `Theme` with `resolve(idx, tint)`; FreeCell exposes it via `document.rs` `workbook_theme()`
  and already consumes it for **cell** colours in `engine/src/cache.rs` (five sites). This is
  the E1 fork fix, and it landed.
- **It was simply never threaded into charts.** Four sites hardcode
  `ThemePalette::office_default()`: `app/chart/style.rs:65`,
  `engine/chart/save.rs:1129`, `engine/chart/write.rs:309`, `app/shell/window.rs:1654`. So a
  chart `schemeClr` in a custom-theme workbook renders the **wrong colour on screen**. Real
  bug, display-only.
- **There is no file corruption.** The loaded-chart save path is a targeted byte splice
  gated on an actual change (`save.rs:1302` — `if series.color != cached_series.color`), so
  untouched series keep their original `<a:schemeClr>` with `lumMod`/`lumOff` byte-for-byte.
  The only mutation path is an explicit `SeriesColor` edit, and writing `<a:srgbClr>` for a
  colour the user deliberately picked is correct behaviour.
- **Both cited "bypass" sites are unreachable.** `ChartColor::Theme` is constructed only in
  `chart/load.rs:860` (the reader); no edit path can produce it, so those match arms are dead
  defensive code. `write.rs` handles *authored* charts exclusively, which never carry a theme
  colour.

**Revised scope:**
1. Thread `workbook_theme()` through to the chart render path, replacing the four
   `office_default()` hardcodes. This is the actual fix.
2. Harden the two dead arms to call `ChartColor::resolve` (which applies tint) rather than
   bare `.color(slot)` — a latent trap if an edit path ever produces a `Theme` colour, not an
   active bug.
3. Fix the `chart/style.rs:62-65` comment claiming "P8 threads the actual workbook
   `clrScheme`". P8 shipped; nothing threads it. This one the review got right.

*Absorbs the former G4 (theme parsing), which turned out to be already done.*

### G3. Fidelity classifier off the text scanner
**project · v1.0 · after G1 (required)**

`fidelity.rs` is 1,272 lines including ~130 lines of hand-rolled XML lexing, in that form
solely so `chart-model` can stay dependency-free. As a whole-part *text scanner* it cannot
bind an element to its enclosing chart group — which is precisely why it is structurally
incapable of catching G1, the thing it exists to catch. Rebuild it to derive from the DOM
the loader already builds.

**Decided (2026-07-28): it lands in the engine, not IronCalc.** "How much of this chart did
*FreeCell* fail to represent?" is not a fact about the workbook — it is a fact about the gap
between the file and FreeCell's renderer, so it is not a concept IronCalc should carry, and
it would not survive the single-fix-PR upstreaming model. If the engine turns out not to see
enough of the parsed chart to classify properly, *that* gap is a legitimate upstream
capability request and becomes its own `fix/` branch — which fits the fork workflow far
better than shipping a FreeCell concept upstream.

### G5. `dLbls` overrides + chart-insert collisions
**task · v0.5 (dLbls) / v1.0 (insert) · no dependencies**

Editing data labels whole-node-replaces `c:dLbls`, destroying per-point overrides and label
typography — the one real hole in an otherwise excellent preserve-unknown save path, so the
v0.5 half is a data-loss fix. Separately, inserting a chart onto a sheet that already
carries one is a hard `SaveError`, because the byte-preserve and write-from-model paths
cannot compose on a shared drawing. Same file; naturally done together.

---

## H. The generator fix

### H1. Invariant-enforcement sweep
**project · v0.5 (known-failed invariants), ongoing thereafter · after B2, F1 (preferred)**

The root cause behind most of the above, and the highest-leverage unit in the plan. Sweep
for comments that assert an invariant and convert each to a `debug_assert!`, a clamp, an
enum, or a test — or delete the claim. Start with the ones in the table at the top of this
document: feature-gate `autogrow_measure_now`; replace the eight popover `bool`s with
`Option<OpenPanel>`; return the engine-entry count from `apply_one` so the undo 1:1 claim
is checked; collapse the three hand-written row-height `max()` reconciliations into one
function; add a single `post_to_sibling` helper that always defers, retiring the five
"BUG #5" comments.

*Each of these is itself a claim from the review — confirm before converting. An invariant
comment that is actually enforced somewhere you didn't look is a no-op, and saying so is
the right outcome.*

### H3. Update `CLAUDE.md` to prevent this class of defect
**task · v0.5 · after H1's first pass (preferred)**

Add standing rules targeting the generator, **each paired with its enforcer rather than
stated as intent** — a `CLAUDE.md` rule is itself prose, which is exactly the failure mode,
so a rule without a named enforcer must be explicitly marked as a judgment call. Core rule:
*an invariant you write down is one you enforce — `debug_assert!`, clamp, type, or test —
or you delete the claim.* Supporting rules, each traceable to a real defect: every loop over
sheet-derived dimensions is bounded by a constant, never by a value from a file or a click
(B2); every engine call from the worker goes through the `catch_unwind` pattern, no
exceptions (B1); a round-trip test over a workbook you authored yourself is not a fidelity
test (C1); a doc claim about a CI gate is checked against the workflow's trigger block when
written (D1); a new feature domain gets a module, not an append (F1); adding a mirrored copy
of existing state requires explicit justification (E2).

Also worth encoding the review-hygiene rule from the top of this document: **a plan is a
hypothesis; confirm the root cause before implementing the fix, and push back when it isn't
there.**

Also picks up the orphan doc fix from dropped D3: `app/README.md:8`,
`specs/projects/mvp/architecture.md:53`, `functional_spec.md:21` and the `macos-verify.yml`
header all call macOS the "primary" platform. FreeCell is cross-platform; make them say so.

*Keep to rules that changed because of this review. A wholesale rewrite would bury the ones
that matter in a document that is already long and — per D1/G1/G2 — currently wrong in
several places about what CI protects and what shipped.*

---

## Suggested waves

**Wave 0 — parallel, no dependencies (12 units).**
A1 · A2 · A4 · B1 · B2 · C1 · D1 · F1 · F3a · G1 · G5 · *(start the H3 convention)*

Most criticals are missing *enforcers*, not missing designs, which is why this wave is wide
and shallow. These can be run by independent agents.

**Wave 1 — unblocked by Wave 0.**
C2 · D2 · E1 · F2 · H1 · H3

**Wave 2 — the real projects.**
C3 · C4 · D4 · F3 · G2 · G3

**Wave 3 — after the ground is stable.**
E2 · D5

**Later.** B4 · F4 · G1b *(v2.0)* · B3 *(v3+)*

**v0.5 scope** is therefore: A1 · A2 · A4 · B1 · B2 · C1 · C2 · D1 · D2 · E1 · F1 · F2 ·
F3a · G1 · G5(dLbls) · H1 · H3.

---

## Rollout — how to invoke this

The 17 v0.5 units are grouped into **five projects, invoked in two waves.** Grouping is by
*file ownership*, not by theme: the three Wave 1 projects touch disjoint sets of files, so
they can run as parallel agents without merge conflicts.

Each project has a **complete `project_overview.md` already written** (`status: complete`),
so an agent should go straight to functional spec → architecture → implementation plan →
implement, without re-asking what the project is.

### Wave 1 — three parallel agents, then merge all three

| Project | Units | Owns these files |
|---|---|---|
| [`v05-cleanup-1`](../specs/projects/v05-cleanup-1/project_overview.md) | A1, A2, A4, C1, D1, F3a, G1, G5 | `Cargo.toml`, workflows, `deny.toml`, docs, `engine/tests/`, `engine/chart/`, `chart-model/` |
| [`chrome-view-split`](../specs/projects/chrome-view-split/project_overview.md) | F1 (app half) | `app/src/chrome/view.rs` → `chrome/view/` |
| [`engine-worker-hardening`](../specs/projects/engine-worker-hardening/project_overview.md) | B2, B1, F1 (engine half), E1 | `engine/src/worker/*`, `core/publication.rs`, `app/src/shell/window.rs` |

**Why these three don't collide:** cleanup-1 deliberately excludes `worker/run.rs` and
`chrome/view.rs`; the two structural projects each own exactly one of them. B1/B2 live with
E1 rather than in cleanup-1 for this reason — all three edit `run.rs`.

**Merge all three before starting Wave 2.** Wave 2 measures against the post-split file sizes
and the new tests.

### Wave 2 — two parallel agents

| Project | Units | Needs from Wave 1 |
|---|---|---|
| [`v05-cleanup-2`](../specs/projects/v05-cleanup-2/project_overview.md) | C2, D2, F2, H3 | C1 (C2 must not regress it), D1 (D2 tightens it), both splits (F2's ceiling) |
| [`invariant-enforcement`](../specs/projects/invariant-enforcement/project_overview.md) | H1 | B2 as the worked example; the splits, so the sweep isn't wasted |

These two overlap only in `CLAUDE.md`: H3 writes the *rule*, H1 does the *sweep*. Whichever
lands second cites the first — noted in both overviews.

### After v0.5

The v1.0 units — C3, C4, D4, D5, E2, F3, G2, G3 — are **deliberately not specced yet.**
Several will change shape based on what Wave 1 finds: C1's part inventory determines C3's real
scope, F3a may shrink or dissolve F3, and G3 depends on what G1 exposes about the parser. Spec
them once v0.5 has landed and the facts are in.

Later still: B4 and F4 (v2.0), G1b (v2.0), B3 (v3+).

---

## Decisions log

Open questions raised during owner review, and how they were settled.

| Question | Decision (2026-07-28) |
|---|---|
| **D1** — bump-token trigger, `paths` filter, or per-PR gate? | **None of them.** Weekly on `main` + at release. Keep it off the PR path; `workflow_dispatch` stays for agent-driven runs. |
| **D3** — add a macOS compile check to PRs? | **Dropped.** Platform branches are expected to be tested when their PR is written; release covers all three OSes. Not worth taxing 99.9% of PRs. Doc fix folded into H3. |
| **E1** — is the shared-surface commit point v0.5? | **Yes, promoted.** Correctness problem reachable today, and it gets cheaper the sooner it lands. |
| **E2** — is the UI view-model v0.5? | **No, stays v1.0** — genuinely multi-week and competes with everything ahead of it. Conditional on the no-new-mirrored-fields freeze holding. |
| **F3** — is folding `chart-model` v0.5? | **Split.** The user-visible half (formatter disagreement) becomes F3a at v0.5; the crate merge stays v1.0. |
| **G2** — is the theme claim real? | **Verified and downgraded.** Theme is parsed and wired for cells; save is a change-gated splice, so no file corruption. Display-fidelity bug at v1.0. |
| **G3** — classifier into IronCalc or the engine? | **The engine.** Fidelity is a fact about FreeCell's rendering gap, not about the workbook, so it doesn't belong upstream. Stays v1.0. |

---

## What to protect while doing all of this

Verified strengths that must survive the remediation: the four-crate dependency rule and its
guard test with negative controls; IronCalc containment behind `pub(crate)`; the worker actor
model and the `ArcSwap` publish-then-bump publication; `Axis` two-level segment sums and the
O(populated) read queries; the chart save patcher's byte-range splicing and `_FOLLOWING`
schema-order tables; the atomic save mechanics (`NamedTempFile` in the destination directory,
`sync_all`, `persist`, shared by both save paths); the pure-logic extractions
(`grid/layout.rs`, `grid/input.rs`, `grid/chart_layer.rs`, `shell/lifecycle.rs`,
`shell/registry.rs`); the 22 negative controls; and the GPL patch-out, which is a real
removal verified in `Cargo.lock`.
