# Architecture-Review Remediation

**Status:** Proposed (2026-07-28) — plan of record for closing the findings of the
whole-codebase architecture review.

Source: an 8-phase fresh-eyes architecture review of `app/` at `e4c7afa` (~99k LOC,
five crates), run as independent sub-agents over crate boundaries, engine concurrency,
UI architecture, charts, persistence, testing, and build/shipping posture, plus a
synthesis verdict. Findings: **22 critical, 55 moderate, 39 mild.** The review
artifacts live under `reviews/projects/codebase-architecture-review/`, which is
git-ignored — so this document is written to stand alone.

Reviewers were instructed to treat `specs/`, `experiments/`, `GAPS.md` and `CLAUDE.md`
as evidence of *intent* only, never as justification.

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
| *"P8 threads the actual workbook `clrScheme`"* | P8 shipped; it doesn't; the wrong colour is written into user files (G2) |

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
Which is why most units below are hours, not weeks.

---

## Unit index

Size is **task** (small, one phase) or **project** (multi-phase). Targets follow the
`GAPS.md` v0.5 / v1.0 / v2.0 convention.

| # | Title | Size | Target | Waits on |
|---|---|---|---|---|
| A1 | Pin the IronCalc fork by immutable rev | task | v0.5 | — |
| A2 | `--locked` on every cargo invocation in CI | task | v0.5 | — |
| A4 | IronCalc fork exit strategy | project | v1.0 | A1 (pref) |
| B1 | Guard load/save with `catch_unwind`; surface worker death | task | v0.5 | — |
| B2 | Clamp the frozen-pane band; validate `SetFrozen` | task | v0.5 | — |
| B3 | Cache lock hold-time | task | v0.5 | — |
| B4 | Fuzz `WorkbookDocument::open` | task | v1.0 | B1 (pref) |
| C1 | Part-inventory round-trip test | task | v0.5 | — |
| C2 | Save-fidelity warning dialog | task | v0.5 | **C1** |
| C3 | Default-carry / explicit-drop preservation model | project | v1.0 | **C1** |
| C4 | Comment write-back | task | v1.0 | C1 (pref) |
| D1 | Make the `render` gate actually run | task | v0.5 | — |
| D2 | Tighten pixel tolerance; add exact-match text cases | task | v0.5 | D1 (pref) |
| D3 | macOS `cargo check` on PRs | task | v0.5 | — |
| D4 | Measure real frame cost, not element construction | project | v1.0 | — |
| D5 | Scale tests on the I/O path | project | v1.0 | C3 (pref) |
| E1 | One commit point for the four shared surfaces | project | v1.0 | B3 (pref) |
| E2 | Single source of truth for UI state | project | v1.0 | F1 (strong pref) |
| F1 | Split `chrome/view.rs` and `worker/run.rs` | project (small) | v0.5 | — |
| F2 | Production-line-count ceiling in CI | task | v0.5 | **F1** |
| F3 | Fold `freecell-chart-model` onto `freecell-core` | project | v1.0 | coord. G3 |
| F4 | Collapse the worker protocol | project | v2.0 | E2 (pref) |
| G1 | Detect multi-group (combo) charts | task | v0.5 | — |
| G2 | Stop writing wrong theme colours into user files | task | v0.5 | — |
| G3 | Move the fidelity classifier into the engine | project | v1.0 | **G1** |
| G4 | Parse `theme1.xml`; thread a real `ThemePalette` | project | v1.0 | **G2** |
| G5 | `dLbls` overrides + chart-insert collisions | task | v0.5 / v1.0 | — |
| H1 | Invariant-enforcement sweep | project | v0.5 | B2, F1 (pref) |
| H2 | Reconcile the docs with reality | task | v0.5 | **D1, G1** |
| H3 | Update `CLAUDE.md` to prevent this class of defect | task | v0.5 | H1 (pref) |

**Bold** = required ordering. *(pref)* = preferred, not blocking.

---

## A. Supply-chain & build integrity

### A1. Pin the IronCalc fork by immutable rev
**task · v0.5 · no dependencies**

`app/Cargo.toml` patches `ironcalc`/`ironcalc_base` to `branch = "freecell-fixes"`,
whose documented maintenance procedure (`CLAUDE.md` §Engine) is *rebasing*. One
force-push + GC makes every historical FreeCell commit unbuildable, including any tag
needed for a hotfix — while every other git dep in the tree, including four of zed's
own forks, is `rev`-pinned. Replace with `rev = "<sha>"`, push an immutable
`freecell-v<n>` tag on `scosman/ironcalc`, and add a mirror or `cargo vendor` snapshot.
Also fix the `ironcalc = "=0.7.1"` version pin, which is fiction — FreeCell migrated to
the post-0.7.1 style-colour API and provably cannot compile against 0.7.1.

*~2 hours; removes the single-point build-extinction risk. Do it first.*

### A2. `--locked` on every cargo invocation in CI
**task · v0.5 · no dependencies**

No workflow builds with `--locked`, so the lockfile `cargo deny` audits is not provably
the lockfile that ships — including in `release.yml`, which produces signed binaries. A
PR that edits a dependency in `Cargo.toml` without regenerating the lock never fails.
One flag per step across six workflows.

*Optional rider: assert `cargo tree -i ztracing` mentions `vendor/ztracing`. The GPL
license gate itself is already covered by `cargo deny` (`exceptions = []`, GPL absent
from `allow`, runs on every push/PR under an `app/**` paths filter), so this is only
about converting an opaque "GPL rejected" failure into a legible "the patch stopped
applying" signal.*

### A4. IronCalc fork exit strategy
**project · v1.0 · after A1 (preferred)**

The `freecell-fixes` branch carries **ten** `fix/*` merges, not the two the manifest
comment and the project's own status table claim — including an entire merged-cells
feature and the `set_user_inputs` batched-write API that the whole paste/replace path
depends on. Zero are upstream. Several are *features* upstream has not agreed to take,
so the documented exit ("pin a crates.io release once the fixes ship") has no path at
all. Needs a true inventory, a per-fix upstream/keep-forked decision, and an honest
long-term position: permanent maintained fork, or an upstream campaign with a schedule.

→ related: [`projects/ironcalc-upgrade.md`](ironcalc-upgrade.md),
[`specs/projects/ironcalc-upstreaming/`](../specs/projects/ironcalc-upstreaming/)

---

## B. Crash & hang safety

### B1. Guard load/save with `catch_unwind`; surface worker death
**task · v0.5 · no dependencies**

Six mutation regions are guarded; `from_source` and `save_workbook` are not — and the
pinned exporter contains a reachable `panic!("Model needs to be evaluated before
saving!")`. Because the `JoinHandle` is discarded and `send` swallows `SendError` by
design, a panic on the file path produces a **silent zombie**: the window keeps
rendering the last publication, edits vanish, Save does nothing, and there is no dialog,
no degraded bar, and no log entry. Wrap both calls into `LoadFailed`/`SaveFailed`,
retain the handle, and make the window treat "event stream ended without a requested
`Shutdown`" as fatal and say so.

*Highest safety-per-hour item in the list. Found independently by the concurrency and
persistence reviewers from opposite directions — it looks like a threading bug from one
side and a persistence bug from the other, and is neither.*

### B2. Clamp the frozen-pane band; validate `SetFrozen`
**task · v0.5 · no dependencies**

Every loop in `build_publication` is bounded by a constant except the frozen-pane band,
which iterates `(0..m)×(0..k)` from unvalidated `frozen_rows`/`frozen_cols`. The UI
computes the count as "last row of the header run you right-clicked", so freezing at
row 500,000 makes every subsequent publish do ~500,000 × 256 `formatted_value` calls
and the worker never returns — two clicks to a permanent hang, also reachable from a
crafted `.xlsx` `<pane>` element. Clamp at the publish site *and* range-validate in
`pre_validate`, closing both the click path and the file path.

### B3. Cache lock hold-time
**task · v0.5 · no dependencies**

`refresh_cache_cells` holds the cache write lock across up to 100,000 IronCalc reads
while the render thread takes that same lock 23 times per frame. Chunk the refresh, or
build off-lock and swap. Visible-jank/perceived-hang, not theoretical.

### B4. Fuzz `WorkbookDocument::open`
**task · v1.0 · after B1 (preferred)**

The file parser is the only untrusted input surface in the app and has no fuzzing. Add
`cargo-fuzz` over the open path with a seed corpus from the existing fixtures. Cheap to
stand up, and it catches the class of panic that B1 only *contains*. Sequenced after B1
so findings surface as errors rather than zombies.

---

## C. Save fidelity — the critical path

> The most dangerous defect in the repo is here. Existing design note:
> [`projects/xlsx-preservation.md`](xlsx-preservation.md) — these units supersede its
> sequencing with a test-first order.

### C1. Part-inventory round-trip test
**task · v0.5 · no dependencies · KEYSTONE**

`tests/roundtrip.rs` only round-trips workbooks FreeCell itself authored — a closed
loop over IronCalc's own serializer — which is why unbounded data loss has been
invisible for the project's entire life. Meanwhile five real Excel fixtures sit in the
tree used for *open* assertions only. Add open→save→reopen over
`personal_monthly_budget.xlsx` and siblings, asserting a **part-level inventory** of
both zips. Build the detector before the fix so the fix is measurable and
non-regressing.

*C2, C3, C4 and D5 all sit behind this. If only one unit starts, start here.*

### C2. Save-fidelity warning dialog
**task · v0.5 · after C1 (required)**

Restore the warn-before-strip dialog cut on 2026-07-13 (`GAPS.md` line 451; the
surviving mitigation is a `.back` file with no UI, no menu entry and no mention in any
dialog). At save time, diff the original package's part inventory against what will be
written and name exactly what is being dropped. `reinject` already reads the original
zip and enumerates parts, so this is a dialog over data you already have. Converts
silent permanent loss into an informed choice.

*Highest-value single item in the whole plan.*

### C3. Default-carry / explicit-drop preservation model
**project · v1.0 · after C1 (required)**

`is_carry_part` allowlists exactly two prefixes (`xl/charts/`, `xl/drawings/`) over a
package IronCalc regenerates from scratch — it synthesizes an eleven-part zip from its
model rather than rewriting the input. So opening a real `.xlsx`, editing one cell and
saving permanently deletes pivot tables, macros, data validation, hyperlinks, comments,
Excel Tables, autofilters, sheet protection, print setup, images and in-cell rich text.
Allowlist-over-regenerated-package cannot be incrementally tightened into correctness;
it must invert to *carry everything IronCalc does not authoritatively regenerate*. The
`reinject` seam already does the hard parts (filter parts, merge `[Content_Types]`
overrides, patch worksheet XML). Stale beats deleted: this converts an unbounded loss
into a bounded, enumerable one.

### C4. Comment write-back
**task · v1.0 · after C1 (preferred)**

IronCalc *imports* `Worksheet.comments` but no exporter ever writes them back, so
comments are lost even in the model's own round-trip. Narrow and well-scoped — a good
candidate for a fork fix contributed upstream rather than a FreeCell workaround. May be
subsumed by C3 if comments end up carried as parts.

---

## D. Verification integrity

### D1. Make the `render` gate actually run
**task · v0.5 · no dependencies**

`render.yml`'s own header says it "MUST be a required status check"; its trigger is
`workflow_dispatch:`-only. 29 dispatched runs across 13 branches, **zero on `main`,
ever**, against 44 merged PRs — while failing on 7% of the runs it did get. The only
automated gate on the product's core differentiator is a convention in a markdown file.
Add a `paths` filter covering `grid/`, `chart/`, `chrome/view.rs`, `assets/`,
`Cargo.lock`, plus an always-posting status context so docs-only PRs can still merge.
Then delete the multi-paragraph "the agent must decide when to run it" process from
`CLAUDE.md`.

### D2. Tighten pixel tolerance; add exact-match text cases
**task · v0.5 · after D1 (preferred)**

`fail_fraction: 0.005` permits 384–1,596 differing pixels in scenes whose entire
non-background content is 8k–25k pixels and whose glyph ink is on the order of 74
pixels — a wrong digit in a cell passes silently. Because `#[gpui::test]` installs a
`NoopTextSystem`, this suite is the *only* place real font metrics are ever exercised,
so **nothing in the project currently validates that the right number is drawn
correctly**. Tighten for text-centric cases (`cell_*`, `spill_*`, `autogrow_*`);
lavapipe has 29 runs of demonstrated stability, and `diff.rs`'s own comment says to
tighten when it does.

### D3. macOS `cargo check` on PRs
**task · v0.5 · no dependencies**

The stated primary platform is currently gated weekly. Add `cargo check --workspace` on
macOS to the PR set.

### D4. Measure real frame cost, not element construction
**project · v1.0 · no hard dependencies**

The 8.33 ms perf gate measures `resolve_frame` + `build_grid_layers`. There is **not one
custom `Element` impl in `grid/`** — the grid hands ~2,000 absolutely-positioned `div`s
to taffy every frame, and that cost, plausibly the dominant term, is never measured. A
rigorously-executed measurement of the wrong quantity is worse than none, because its
rigor makes the number unquestioned. Needs an end-to-end frame measurement, a
re-derived budget, and trend tracking rather than a flat 2× threshold. Worth doing
*before* any perf-motivated refactor, since its result may reframe E1/F1 priorities.

### D5. Scale tests on the I/O path
**project · v1.0 · after C3 (preferred)**

Excel-max is the product thesis and there is no scale test on load or save: six
committed fixtures, largest 35 KB, and the 1M-row perf fixture is synthesised
in-process and never serialised. Parse time, peak memory, shared-string blowup and save
time at scale are all unmeasured. Needs a generated large-workbook fixture (committed or
reproducibly built) plus gates on parse/save time and peak RSS. Sequenced after C3 —
no point baselining save cost on a model about to change.

---

## E. State & consistency architecture

### E1. One commit point for the four shared worker→UI surfaces
**project · v1.0 · after B3 (preferred)**

The `ArcSwap` publication was designed as a seam; the style cache, chart snapshot and CF
map were each added beside it with their own primitive and their own (or no) version
discipline. The commit order emits `Published` *between* the value commit and the style
commit, and `commit_chart_op` emits `Published` without publishing — so there is no
answer to "what does the UI see at generation N", which is the question you must be able
to answer to reason about this seam at all. Unify under one generation counter and one
commit point; add ordering tests with non-zero-sample assertions.

### E2. Single source of truth for UI state
**project · v1.0 · after F1 (strongly preferred)**

Selection lives in three places (`GridView.selection`, `ChromeView.selection`,
`SinkShared.last_selection`), active sheet in four, pending-edit text in four
representations, and ten `GridView` fields are hand-pushed mirrors of chrome state
through a nine-positional-argument `set_edit_state`. This forced the cyclic
`Rc<OnceCell<WeakEntity>>` grid↔chrome wiring and a `window.defer` convention documented
across five "BUG #5" comments, and it has already produced a self-documented
wrong-sheet-write hazard at `shell/window.rs:2129`. One observed document-view-model
that both views read dissolves the mirrors, `SinkShared`, the setter and most of the
re-entrancy class at once.

*Multi-week. Deliberately not in the first 30 days. **Freeze rule while it waits: no new
mirrored fields.***

---

## F. Structure & maintainability

### F1. Split `chrome/view.rs` and `worker/run.rs`
**project (small, 2–3 phases) · v0.5 · no dependencies**

`chrome/view.rs` is 16,099 lines — **8,249 production** — one struct with 71 fields, 267
methods, 91 `cx.listener` closures, spanning eight independent feature domains (CF
sidebar ~1,510 lines, formatting ~1,450, sheet tabs ~1,350, edit/formula ~990, charts
~790, find, stats, shell). With `grid/view.rs` (10,627) that is **55% of the app crate's
production code in two files**. The growth curve is the damning part: 5,618 → 16,099
across 34 commits in sixteen days, monotonically, with exactly one net-negative commit —
because nothing resists it. Rust's module-descendant privacy means a `chrome/view/`
directory needs **zero visibility changes** to the 71 private fields, so this is
mechanical. Same for moving `worker/run.rs`'s ~1,200 chart lines into the
already-existing, currently-empty 39-line `worker/charts.rs`.

*This is the **loudest** problem and one of the **least dangerous**. Do it — it is the
top velocity and bus-factor problem — but do not let its volume displace B and C.*

### F2. Production-line-count ceiling in CI
**task · v0.5 · after F1 (required)**

Nothing has ever resisted file growth, which is why the curve is monotonic. Add a CI
check on per-file production line count so the F1 split stays done.

### F3. Fold `freecell-chart-model` onto `freecell-core`
**project · v1.0 · coordinate with G3**

Two zero-dependency foundation crates that do not know each other exist. This has
already produced a byte-identical duplicate `Color`/`Rgb`, drifted `rgb_to_hsl` copies
(`.rem_euclid` vs `%` — they disagree on negatives), two definitions of the Office
palette, and a **383-line reimplementation of OOXML number formatting** racing
IronCalc's, so `#,##0.00` on a chart axis and on the cells it plots go through two
different implementations. A purity boundary is not free: it charges rent in
reimplementation. Decide the target shape with G3, which moves code *out* of
`chart-model` and may shrink the problem.

### F4. Collapse the worker protocol
**project · v2.0 · after E2 (preferred)**

53 `Command` + 23 `WorkerEvent` variants mirroring UI menus one-for-one, several
carrying UI concerns into the engine outright — column widths in *device pixels*,
`AutoGrowRowHeights` shipping render-thread text measurements into the worker. Adding
one feature is a six-site change. The exhaustive matches keep it safe; they do not keep
it small. Partly a symptom of E2's state sprawl, so sequence after it.

---

## G. Chart correctness

### G1. Detect multi-group (combo) charts
**task · v0.5 · no dependencies**

`parse_chart_xml` takes only the *first* chart-group element in `c:plotArea` and
discards the rest, while `source_fidelity` never counts groups — so an ordinary Excel
bar+line combo loads as bars only, the line series **absent from the picture**,
classified `Faithful`, and drawn with no badge. Count group children at parse time and
force `Fidelity::Degraded`. Also fix `is_extended_chart`'s bare `contains("chartex")`,
and correct the `GAPS.md` combo row, which currently claims placeholder behaviour that
does not happen.

*Stopgap for G3, which fixes the structural cause.*

### G2. Stop writing wrong theme colours into user files
**task · v0.5 · no dependencies**

The workbook theme is never parsed, so every `schemeClr` resolves against a hardcoded
Office palette — and three call sites bypass the model's own `ChartColor::resolve` to
write that wrong RGB, **tint stripped**, back into the user's file on a series-colour
edit. This is silent file corruption, not a display bug. The v0.5 half is the write
side only: preserve the original `schemeClr` when the user did not change it, rather
than round-tripping a guess.

### G3. Move the fidelity classifier into the engine
**project · v1.0 · after G1 (required)**

`fidelity.rs` is 1,272 lines including ~130 lines of hand-rolled XML lexing, in that
form solely so `chart-model` can stay dependency-free. As a whole-part *text scanner* it
cannot bind an element to its enclosing chart group — which is precisely why it is
structurally incapable of catching G1, the thing it exists to catch. It belongs in the
engine, deriving from the DOM the loader already built.

### G4. Parse `theme1.xml`; thread a real `ThemePalette`
**project · v1.0 · after G2 (required)**

The full fix behind G2: parse the workbook theme in the engine and thread a real palette
through load, render and save so `schemeClr` + tint resolve correctly everywhere. Also
retires the `chart/style.rs` claim that "P8 threads the actual workbook `clrScheme`",
which shipped false.

### G5. `dLbls` overrides + chart-insert collisions
**task · v0.5 (dLbls) / v1.0 (insert) · no dependencies**

Editing data labels whole-node-replaces `c:dLbls`, destroying per-point overrides and
label typography — the one real hole in an otherwise excellent preserve-unknown save
path, so the v0.5 half is a data-loss fix. Separately, inserting a chart onto a sheet
that already carries one is a hard `SaveError`, because the byte-preserve and
write-from-model paths cannot compose on a shared drawing. Same file; naturally done
together.

---

## H. The generator fix

### H1. Invariant-enforcement sweep
**project · v0.5 (known-failed invariants), ongoing thereafter · after B2, F1 (preferred)**

The root cause behind most of the above, and the highest-leverage unit in the plan.
Sweep for comments that assert an invariant and convert each to a `debug_assert!`, a
clamp, an enum, or a test — or delete the claim. Start with the six in the table at the
top of this document: feature-gate `autogrow_measure_now`; replace the eight popover
`bool`s with `Option<OpenPanel>`; return the engine-entry count from `apply_one` so the
undo 1:1 claim is checked; collapse the three hand-written row-height `max()`
reconciliations into one function; add a single `post_to_sibling` helper that always
defers, retiring the five "BUG #5" comments.

*Sequenced after B2 (its first instance) and F1 (which changes where the comments live).
The convention itself can start immediately.*

### H2. Reconcile the docs with reality
**task · v0.5 · after D1, G1 (required)**

`app/README.md` §CI and `checks.yml`'s own header both claim `checks` runs the render
suite — it does not, render was split out. `deny.toml`'s header still references a GPL
exception that was deliberately replaced with `exceptions = []`. `chart/style.rs` claims
P8 threads the workbook `clrScheme`. `GAPS.md` is wrong about combo charts in the
direction of confidence. Sequenced after D1 and G1 because those change what is true.

### H3. Update `CLAUDE.md` to prevent this class of defect
**task · v0.5 · after H1's first pass (preferred)**

Add standing rules targeting the generator, **each paired with its enforcer rather than
stated as intent** — a `CLAUDE.md` rule is itself prose, which is exactly the failure
mode, so a rule without a named enforcer must be explicitly marked as a judgment call.
Core rule: *an invariant you write down is one you enforce — `debug_assert!`, clamp,
type, or test — or you delete the claim.* Supporting rules, each traceable to a real
defect: every loop over sheet-derived dimensions is bounded by a constant, never by a
value from a file or a click (B2); every engine call from the worker goes through the
`catch_unwind` pattern, no exceptions (B1); a round-trip test over a workbook you
authored yourself is not a fidelity test (C1); a doc claim about a CI gate is checked
against the workflow's trigger block when written (D1); a new feature domain gets a
module, not an append (F1); adding a mirrored copy of existing state requires explicit
justification (E2). Also decide deliberately whether debt may continue to live only in
markdown.

*Keep to rules that changed because of this review. A wholesale rewrite would bury the
six that matter in a document that is already 12k characters and — per H2 — currently
wrong in at least four places about what CI protects.*

---

## Suggested waves

**Wave 0 — parallel, no dependencies, mostly hours (13 units).**
A1 · A2 · B1 · B2 · B3 · C1 · D1 · D3 · F1 · G1 · G2 · G5 · *(start the H3 convention)*

Unusually wide, and that is the payoff of the pattern diagnosis: most criticals are
missing *enforcers*, not missing designs. These can be run by independent agents.

**Wave 1 — unblocked by Wave 0.**
C2 · D2 · F2 · H1 · H2 · H3 · B4

**Wave 2 — the real projects.**
C3 · C4 · E1 · G3 · G4 · D4 · A4

**Wave 3 — after the ground is stable.**
E2 · F3 · D5

**Wave 4.**
F4

## What to protect while doing all of this

Verified strengths that must survive the remediation: the four-crate dependency rule and
its guard test with negative controls; IronCalc containment behind `pub(crate)`; the
worker actor model and the `ArcSwap` publish-then-bump publication; `Axis` two-level
segment sums and the O(populated) read queries; the chart save patcher's byte-range
splicing and `_FOLLOWING` schema-order tables; the atomic save mechanics (`NamedTempFile`
in the destination directory, `sync_all`, `persist`, shared by both save paths); the
pure-logic extractions (`grid/layout.rs`, `grid/input.rs`, `grid/chart_layer.rs`,
`shell/lifecycle.rs`, `shell/registry.rs`); the 22 negative controls; and the GPL
patch-out, which is a real removal verified in `Cargo.lock`.
