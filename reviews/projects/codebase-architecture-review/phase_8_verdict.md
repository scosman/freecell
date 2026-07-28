# Phase 8: Fresh-Eyes Verdict

*Principal-architect synthesis over Phases 1–7, plus independent re-verification of nine
load-bearing claims against the code. Nothing was built or run. Where I disagree with a phase
reviewer, it is called out by name in its own section.*

**Context for the grade:** 392 commits, 2026-06-30 → 2026-07-27. Twenty-seven days. 99k lines,
1,451 tests, five crates, three-platform packaging, code signing, a dependency-licensing audit,
and a 518-line self-maintained gap register. That is the thing being graded.

---

## The Verdict

**This is good code with a bad relationship to the truth about itself. Grade: B− as a codebase,
F as a shipping product — and the README currently ships it.**

Judged as engineering, the comparison class is not a hobby project. On layering, panic
discipline, algorithmic choices at scale, atomic-save durability, dependency provenance, and
packaging, this is at or above the median seed-stage startup alpha with three engineers and six
months, and it was built in four weeks. Several things here — the `ArcSwap` publication seam, the
`Axis` primitive, the source-preserving chart save patcher, the negative controls on the
architecture guard and the perf harness — are work I would call good in any codebase, at any
funding level. I want that on the record before the rest, because the rest is severe.

Judged as a product, it is not close to shippable, and the reason is not any of the twenty-two
criticals individually. It is that **the codebase's self-reported quality is systematically
better than its actual quality, in a specific and repeating way**: invariants that a compiler or
a test happens to enforce are held rigorously; invariants documented in prose are held until the
code moves and then silently aren't. The most consequential instance is that opening a real
Excel workbook, editing one cell, and saving permanently destroys the user's pivot tables,
macros, comments, data validation, hyperlinks, Excel Tables, images, and in-cell rich text, with
no warning — and the HEAD commit of this repo is a README with three "Download for macOS /
Windows / Linux" badges.

---

## What's Good

These are real and a new owner should protect them. I verified each.

**1. The crate seam is the best asset in the repo, and it is compiler-enforced.** `freecell-core`
and `freecell-chart-model` have no GPUI and no IronCalc; `freecell-engine` has IronCalc but no
GPUI; no `ironcalc_base` type is nameable from `freecell-app` (containment is held by `pub(crate)`
on `WorkbookDocument`, not by convention). If IronCalc had to be replaced, the blast radius is one
crate. This is the difference between a project that can survive its engine bet and one that
can't, and it was drawn correctly on day one. Phase 1, 2 and 5 all independently confirmed it
holds; so did I.

**2. The worker/UI seam's *primary* surface is genuinely well built.** One thread owns the
`UserModel`; the UI does one wait-free `ArcSwap` load per frame; publish-then-bump; no `block_on`,
no blocking `recv`, no engine call on the render path — and that last invariant is enforced by a
process-global counter with a documented negative control, not by a comment. That is the correct
shape for this problem, correctly executed.

**3. `Axis` and the read queries are real Excel-max engineering, not a claim.** Two-level segment
sums (~2k f64 for 1M rows, sizes from a closure, nothing materialized per track); `find_matches`,
`selection_stats`, and `resolve_edge` walk populated cells rather than the selected rectangle, so
a full-column selection on a sparse 1M-row sheet is O(populated). Most spreadsheet UIs get this
wrong. This one doesn't.

**4. Save durability is better than most shipping document editors.** `NamedTempFile` created in
the *destination* directory (same-filesystem rename guaranteed), `sync_all()` on data and
metadata, then `persist()`; both save entry points share the one implementation so they cannot
diverge; failure-mode tests inject EISDIR/ENOTDIR rather than relying on permission bits. Table
stakes, met properly.

**5. The chart save patcher is the single cleverest thing in the codebase.** Re-parse the retained
`chartN.xml`, diff against the model, splice *byte ranges* for changed fields only, in the file's
own namespace prefixes — so unmodeled DrawingML (gradients, effects, `txPr`, `extLst`) survives
byte-for-byte. The `_FOLLOWING` schema-order tables that anchor a fresh insert before the first
present later sibling are a correct solution to a genuinely hard problem. Whoever wrote this
understood that round-trip fidelity is a preservation problem, not a modelling problem.

**6. The test suite is well above average at the test-writing altitude.** 1,451 tests, 3.4
assertions each, exactly three with no assertion. Behavioural test names. **Twenty-two negative
controls** — including one proving the dependency guard trips on a synthetic violation and one
proving the perf harness isn't measuring a dead counter. A public-API-only worker integration
surface with deadline-bounded waits. A render harness that *fails* rather than skipping when the
capture stack is missing. Phase 6 is right that these are rare practices, and right that they make
several gates credible rather than decorative.

**7. Production panic discipline is excellent.** Effectively zero unguarded `unwrap`/`expect`
across ~6k lines of engine production code; zero across the whole of FreeCell's own load/save
code. The panic risk on the file path is entirely inherited from IronCalc, not authored here.

**8. Dependency and licensing work exceeds most commercial codebases.** The GPL patch-out is a
real removal — I confirmed `zlog` is absent from `Cargo.lock` entirely and `ztracing` resolves to
the local vendor path — not a paper exception. Every advisory ignore names its entry path. The
packaging and macOS signing scripts are reasoned out in advance to a level people usually reach
only after a failed release.

**9. `GAPS.md` is an unusually honest artifact.** 518 lines, itemized, tiered, with "**NEW**
(checked: …)" provenance on each entry. Most projects at this stage have nothing like it. It is
also — see below — the source of the single most damning fact in this review, which is exactly
what a good gap register is for.

---

## What's Bad

Structural problems that compound. I separate "hurts now" from "hurts in six months."

### Hurts now

**The four shared surfaces have no single commit point.** The publication was designed as a seam;
the style cache, chart snapshot, and CF map were each added afterward with their own primitive and
their own (or no) version discipline. The commit order emits `Published` *between* the value commit
and the style commit, and `commit_chart_op` emits `Published` without publishing. There is no
answer to "what does the UI see at generation N," which is the question you must be able to answer
to reason about this seam at all. Compounding: `refresh_cache_cells` holds the cache write lock
across up to 100,000 IronCalc reads while the render thread takes that same lock 23 times per
frame.

**Load and save are the only engine calls outside `catch_unwind`.** I verified this: six guarded
mutation regions, and `from_source` / `save_workbook` guarded by nothing. The worker's `JoinHandle`
is discarded, `send` swallows `SendError` by design, and the window's event loop just falls out of
`recv().await`. So a panic on the file path produces a *silent zombie*: window still rendering,
edits vanishing, Save doing nothing, no dialog, no log. Phases 2 and 5 found this independently
from opposite directions, which is worth noting — it is the kind of defect that looks like a
concurrency issue from one side and a persistence issue from the other, and is neither.

**Selection, active sheet, and pending-edit text have no single source of truth.** Selection lives
in three places, active sheet in four, the edit text in four, and ten `GridView` fields are
hand-pushed mirrors of chrome state through a nine-positional-argument setter. This forced the
cyclic `Rc<OnceCell<WeakEntity>>` wiring and the `window.defer` discipline that five "BUG #5"
comments document. Phase 3 is right that this is a correctness architecture, not a smell: the code
already documents a **wrong-sheet-write hazard** at `shell/window.rs:2129` and argues it is
acceptable because the alternative was a panic. It isn't acceptable; it's a symptom.

**Every loop in `build_publication` is bounded by a constant except the one that isn't.** The
frozen-pane band reads `frozen_rows`/`frozen_cols` off the cache with no cap, on every publish. The
UI computes the count as "last row of the header run you right-clicked," so freezing at row 500,000
makes every subsequent scroll frame do ~500,000 × 256 `formatted_value` calls. The worker never
returns. Two clicks. I read the code: the doc comment directly above the loop asserts *"the bands
are a few leading tracks, never a sheet-size loop."* That assertion is the bug.

### Hurts in six months

**No module-size discipline anywhere.** `chrome/view.rs` at 16,099 lines (8,249 production, 71
fields, eight independent feature domains) plus `grid/view.rs` at 10,627 are 55% of the app crate's
production code in two files; `worker/run.rs` is another 26% of the engine's, a quarter of it chart
machinery sitting next to the empty 39-line `worker/charts.rs`. The growth curve is monotonic —
5,618 → 16,099 across 34 commits, one net-negative commit — because nothing resists it. Note
carefully: this is the *loudest* problem and the *least dangerous* one. Phase 1 is right that the
mechanical split is near-zero-risk (Rust module-descendant privacy means the 71 private fields need
no visibility changes). Do not let its volume displace the items above it.

**The protocol grew into an RPC surface.** 53 `Command` + 23 `WorkerEvent` variants that mirror UI
menus one-for-one, several carrying UI concerns into the engine outright (column widths in *device
pixels*; `AutoGrowRowHeights` shipping render-thread text measurements into the worker). Adding one
feature is a six-site change. The exhaustive matches keep it safe; they don't keep it small.

**Two zero-dependency "pure" foundation crates that don't know about each other.** This has already
produced a duplicated `Color`/`Rgb`, drifted `rgb_to_hsl` copies (`.rem_euclid` vs `%` — they differ
on negatives), two definitions of the Office palette, and — worst — a 383-line reimplementation of
OOXML number formatting in `chart-model` racing IronCalc's, so `#,##0.00` on an axis and on the
cells it plots go through two different implementations.

---

## What's Ugly

The owner asked for this category by name. These are the things that are misleading, and the fact
that none of them is *intentionally* dishonest is what makes them worth naming.

**1. The README sells a product whose save path destroys user data, and the warning was cut on
purpose.** `README.md` (the HEAD commit, "Revise README with updated app features") carries three
download badges and states: *"XLSX file support: open and edit Excel files."* Under "What's not
included (yet)" it lists *"Pivot tables."* A reasonable person reads that as "I can't create a pivot
table." What it actually means is **"if your file has one, we delete it."** And this is not an
oversight: `GAPS.md` line 451 records that xlsx part-preservation is a **v1.0** item and that
*"v0.5 relies on the write-once `.back` backup, not a warn dialog — **the warn-before-strip idea
was cut 2026-07-13**."* The mitigation that survived is a `.back` file with no UI, no menu entry,
no mention in any dialog, that most users will find next to their spreadsheet and delete as junk.
A project that describes itself internally as *"alpha with a full-strip writer"* has download
badges on its front page.

**2. `render.yml` says it "MUST be a required status check." It is `workflow_dispatch:`-only.** I
verified the trigger block. `CLAUDE.md` builds an entire multi-paragraph process on the assumption
of enforcement and then admits, in the same document, *"there is no safety net."* Phase 6 measured
29 dispatched runs across 13 branches, zero on `main`, against 44 merged PRs — I could not
re-verify the Actions history (no `gh` in this container) but the in-repo evidence is sufficient on
its own: a `workflow_dispatch`-only workflow with no `paths` filter cannot have been a satisfied
required check on 31 merges. **The only automated gate on the product's core differentiator is a
convention in a markdown file.**

**3. The pixel gate cannot see the thing it exists to validate.** `per_channel_tolerance: 12`,
`fail_fraction: 0.005`. Phase 6 measured that against the committed baselines: 384–1,596 pixels of
allowed difference in scenes whose *entire* non-background content is 8k–25k pixels and whose glyph
ink is on the order of **74 pixels**. A wrong digit in a cell passes. And because `#[gpui::test]`
installs a `NoopTextSystem`, the pixel suite is the *only* place real font metrics are ever
exercised. So for a spreadsheet, **nothing in this project validates that the right number is
drawn correctly** — and the gate that would has an unenforced trigger (see #2).

**4. A badge that says "Faithful" over a chart with a third of its data missing.** I verified both
halves. `parse_chart_xml` does `.find(|n| is_chart_group(...))` — first group only, rest discarded.
`source_fidelity` never counts groups; it asks "is there a `lineChart`?", never "how many groups?".
So an ordinary Excel combo chart (bar + line, secondary axis) loads as bars only, the line series
**absent from the picture**, classified `Faithful`, drawn with no badge. The whole `fidelity.rs`
module — 1,272 lines — exists to prevent exactly this, and is structurally incapable of catching
it because it is a text scanner that cannot bind an element to its enclosing group.

**5. The gap register is wrong about a gap, in the direction of confidence.** `GAPS.md` lists
combo/dual-axis under **v2.0** with the note *"unsupported kinds → placeholder."* That is true for
`radar`/`waterfall` (unrecognized group names) and **false for combo**, whose groups *are*
recognized — so it silently truncates instead of placeholder-ing. The single most carefully
maintained honesty artifact in the repo is wrong precisely where the classifier is textual. This
one finding is the whole review in miniature.

**6. Scale is the product thesis and there is no scale test on I/O.** Six committed fixtures,
largest 35 KB. The 1M-row perf fixture is synthesised in-process and never serialised. So parse
time, peak memory, shared-string blowup, and save time at scale are all unmeasured. And the perf
gate that does exist measures `resolve_frame` + `build_grid_layers` — I confirmed there is **not a
single custom `Element` impl in `grid/`**; the grid hands ~2,000 absolutely-positioned `div`s to
taffy per frame, and that cost, plausibly the dominant term, is the one thing the 8.33 ms budget
never sees. On top of which the gate sits at 2× with no trend tracking.

**7. `[patch.crates-io]` says two fixes; the branch carries ten.** The manifest comment names E2
and E5. Phase 5 counted ten `fix/*` merges on `freecell-fixes`, including an entire merged-cells
feature and the `set_user_inputs` batched-write API that the whole paste/replace path depends on.
Zero are upstream. The manifest also pins `ironcalc = "=0.7.1"` — a version FreeCell provably
cannot compile against, since it migrated to the post-0.7.1 style-colour API.

**8. Doc drift on what CI protects.** `app/README.md` §CI and `checks.yml`'s own header both claim
`checks` runs the render suite. It doesn't — render was split out. `deny.toml`'s header still
references a GPL exception that was deliberately replaced with `exceptions = []`. `chart/style.rs`
says *"P8 threads the actual workbook `clrScheme`"* — P8 shipped; it doesn't; and three call sites
hand-roll theme resolution against a hardcoded Office palette and write the resulting wrong RGB,
tint stripped, back into the user's file.

---

## The Most Dangerous Thing in the Repo

**The save path's preservation model: an allowlist of two prefixes over a package that IronCalc
regenerates from scratch — combined with the decision, on record, not to warn the user.**

`chart/save.rs:518`, verified verbatim:

```rust
fn is_carry_part(name: &str) -> bool {
    name.starts_with("xl/charts/") || name.starts_with("xl/drawings/")
}
```

IronCalc's exporter synthesizes an eleven-part zip from its model. Everything else in the user's
original file survives only if it matches one of those two prefixes. Nothing else does.

Why this over the other candidates:

- **vs. the fork branch pin (Phase 7 C1).** Losing the build is a catastrophe for *the owner*, and
  it is recoverable — someone has a clone, a cargo cache, a `target/` dir. Losing a stranger's
  macro workbook is not recoverable by anyone.
- **vs. the silent-zombie worker death (Phase 2/5).** Severe, but it is *loud in effect* — nothing
  works, so the user knows something is wrong and still has their file on disk. The lossy save is
  silent, succeeds, and reports success.
- **vs. the frozen-pane wedge (Phase 2 C1).** Two clicks to a hung app, which is embarrassing, but
  the document on disk is intact. Force-quit and reopen.
- **vs. the combo-chart Faithful badge (Phase 4 C1).** Same *class* of defect — silent wrongness
  wearing a correctness badge — but one subsystem, one chart shape, and partially visible (a user
  may notice the missing line).
- **vs. `chrome/view.rs`.** Costs velocity, not users. Mechanically fixable. Loudest ≠ most
  dangerous.

The save defect is the only one with all six of: triggered by the single most common user action;
permanent and irreversible; completely silent; **invisible to the entire test suite by
construction** (the round-trip suite only round-trips workbooks FreeCell itself authored — a closed
loop over IronCalc's own serializer, so no test can ever go red for this); it destroys precisely
the content the user did *not* create in FreeCell and therefore trusts most; and the mitigation
that would have made it survivable was considered and cut.

One test would have converted this from unbounded to bounded on day one: open
`personal_monthly_budget.xlsx` (already in the repo), save it, and assert a part-level inventory
of the two zips. It does not exist. Five real Excel fixtures sit in the tree used for *open*
assertions only.

---

## Where I Disagree With the Phase Reviews

**Phase 4 praises `freecell-chart-model`'s zero-dependency manifest as "the strongest boundary in
the repo." It is the boundary that caused two of the subsystem's three worst modules.** Phase 4
itself documents both: `numfmt.rs` (383 lines reimplementing OOXML number formatting, existing
"only because `chart-model` must be ironcalc-free") and `fidelity.rs` (~130 lines of hand-rolled
XML lexing "solely so the model crate can stay dependency-free"). It never connects them to the
boundary it praised in the same document. Phase 1 gave the correct diagnosis for `freecell-core` —
*the seam is drawn on the technology axis, not the responsibility axis* — and did not notice it
applies verbatim to `chart-model`. **A purity boundary is not free: it charges rent in
reimplementation.** Here it bought a clean manifest and paid for it with a second number formatter
that can visibly disagree with the cell formatter, and a second XML parser that structurally cannot
see combo charts. That is the most expensive `[dependencies]` section in the repo. Phase 1 is right
about the direction (fold it into `freecell-core`, or at minimum let it depend on it); Phase 4 is
right that the *layering* between engine and app is excellent. Both, not either.

**Phase 6 grades the perf harness on rigor and under-weights that it measures the wrong quantity.**
Phase 6 calls the methodology "sound and non-vacuous — genuinely better than most," which is true:
forced work, asserted fixture, negative control. Phase 3 found that the thing being measured
excludes gpui layout and paint, and the grid is div-per-cell. I verified there are zero custom
`Element` impls in `grid/`. **A rigorously-executed measurement of the wrong quantity is not
evidence, and its rigor makes it more misleading, not less** — it produces a confident number that
nobody thinks to question. On this specific point Phase 3 is more right than Phase 6, and the two
findings should be merged: the harness's credibility is exactly what makes its blind spot
dangerous.

**Phase 5 understates its own Critical #1 by treating it as a defect.** Phase 5 was correctly
scoped to code and did not read `GAPS.md`. I did. The lossy save is not an oversight that testing
missed — it is a **shipped decision** ("the warn-before-strip idea was cut 2026-07-13") whose
mitigation is a backup file with no user-facing surface. That moves it from "expensive bug" to
"the product's central integrity commitment was traded away and the trade is not visible to
users." It is the reason it is my #1 and not my #3.

**Phase 7's C1 framing is slightly off, and Phase 5's version of the same finding is better.**
Phase 7 leads with "a rebase orphans `cee2859d` and every historical commit becomes unbuildable."
True and worth fixing this week. But the sharper problem is Phase 5's: the branch carries **ten**
fixes, several of which are *features* (merged cells, `set_user_inputs`) that upstream has not
agreed to take, so the documented exit — "pin a crates.io release once the fixes ship" — has no
path at all. The pin is a git-hygiene problem with a one-line fix; the *dependency* is a strategic
position with no exit, and that is the one that should be on the roadmap.

**Where a reviewer was too harsh: `chrome/view.rs`.** Phase 1 calls it "the single largest liability
in the codebase" and Phase 3 "the single biggest liability in the UI." Measured against a save path
that silently deletes user data and a fidelity badge that lies, it is not in the top five most
dangerous things here. It is the top *velocity* problem and the top *bus-factor* problem, and
Phase 1's own analysis shows the fix is a mechanical one-pass move. Rank it accordingly.

---

## The Pattern Behind the Problems

Twenty-two criticals across seven very different domains — concurrency, persistence, XML schema
handling, GPUI state ownership, CI configuration, supply chain. That spread rules out a
domain-knowledge gap. There is one generator, and it is not any of the four the brief offered.

**The generator is: this project consistently mistakes having *reasoned about* an invariant for
having *enforced* it. Invariants that a compiler or an already-existing test happens to check are
held rigorously. Invariants that would require building an enforcer are written down in prose
instead — excellent prose, correct at the moment of writing — and the code moves and the prose
doesn't.**

Sort every finding in this review by that axis and it partitions cleanly.

**Enforced by a mechanism → healthy, in every case:**
- Crate boundaries → cargo + a guard test *with negative controls*. Zero leaks found by three
  reviewers and by me.
- IronCalc containment → `pub(crate)`. 139 references, all inside one crate.
- Command dispatch completeness → exhaustive match, no catch-all. A new variant fails to compile.
- Publication ordering → a real concurrent test with a non-zero-sample assertion.
- Zero engine calls on the render path → a process-global counter with a negative control.
- The GPL removal → verified against `Cargo.lock`, not against a README.

**Asserted in prose → failed, in every case:**
- *"the bands are a few leading tracks, never a sheet-size loop"* — the comment directly above the
  unbounded loop that wedges the worker.
- *"the worker is the only writer"* — while `pub fn autogrow_measure_now` takes `caches.write()`
  from the UI thread in the shipped binary.
- *"this MUST be a required status check"* — it is `workflow_dispatch`-only.
- *"at most one popover is open"* — eight independent `bool`s, holding only because each popover
  paints an occluding backdrop.
- *"the undo stacks stay 1:1"* — already latently violated by `SetFrozen` sending two engine
  history entries against one worker touch; the comment at `run.rs:3580` asserts the opposite.
- *"P8 threads the actual workbook `clrScheme`"* — P8 shipped; it doesn't; the wrong colour gets
  written into user files.
- *"the toggle reads at render / click time, not per frame"* — `render` **is** per frame.
- Five "BUG #5" comments explaining which cross-entity calls need `window.defer` — decided
  per-call-site by reasoning, with no type or helper enforcing it.
- Three hand-written `max()` reconciliations of row height, one of which already caused a bug the
  comment documents.

This also explains three things no single reviewer could explain:

**Why the docs are so good and so wrong at once.** There are **zero `TODO`/`FIXME` markers in
99k lines** — I checked. Debt is not recorded in code; it is externalized into `GAPS.md`, `specs/`,
`projects/`, and long doc comments. Prose scales in volume but not in enforcement, so the artifact
that carries the project's confidence is the one artifact that cannot fail a build. `GAPS.md` being
wrong about combo charts is not an anomaly — it is the predicted outcome.

**Why the failures cluster at *boundaries between* well-built things.** The publication is
excellent; the three surfaces added beside it have no shared commit point. `catch_unwind` is
excellent; the two calls added outside the pattern are unguarded. The publication clamp is
excellent; the band added beside it escaped. The save patcher is excellent; `dLbls` is the one
field patched by whole-node replacement. Each well-built mechanism enforces itself and *nothing
adjacent to itself*, so every new neighbour is a fresh coin flip resolved by whoever remembers the
rule.

**Why this shape is specific to an agentic build.** Writing an excellent explanatory comment costs
an LLM the same as writing a mediocre one; building an enforcer is a separate task nobody asked
for. The prose is generated *from the same intent as the code*, in the same act — so it describes
the intent perfectly and drifts the instant the intent changes. Twenty-seven days of that, with no
counter-pressure, produces exactly this artifact: unusually good local reasoning, unusually good
documentation of that reasoning, and a widening gap between what the repo says about itself and
what is true.

The corollary is the good news, and it is why my verdict is B− and not worse: **this failure mode
is mechanically fixable and does not require rethinking the design.** Every one of the prose-only
invariants above can become a `debug_assert!`, a clamp, an enum, a test, or three lines of YAML.
The project has already demonstrated it can build first-rate enforcers — twenty-two negative
controls prove it. It just hasn't been building one every time it wrote down a rule.

---

## Is the Architecture Sound Enough to Build On?

**Yes. Keep the architecture; rebuild two subsystems and stop trusting three claims.**

Keep, unchanged: the crate graph and dependency rule; the worker actor model and the `ArcSwap`
publication; `Axis` and the O(populated) read queries; the chart save patcher and the retain-badge-
placeholder failure posture; the atomic save mechanics; the pure-logic extractions (`grid/layout.rs`,
`grid/chart_layer.rs`, `shell/lifecycle.rs`, `shell/registry.rs`, core's reducers). None of these
needs to be redone and all of them would be expensive to recreate.

**Two things must be redone, not refactored:**

1. **The save preservation model.** Allowlist-over-regenerated-package is not a policy that can be
   incrementally tightened into correctness — it must invert to default-carry / explicit-drop. The
   seam already exists: `chart/save.rs::reinject` reads the original package, filters parts, merges
   `[Content_Types]` overrides, and patches worksheet XML. Generalising it from two prefixes to
   "carry everything IronCalc does not authoritatively regenerate" converts an *unbounded* loss into
   a *bounded, enumerable* one. Stale beats deleted.

2. **UI state ownership.** Three copies of selection, four of active sheet, four of the pending edit,
   a cyclic `OnceCell<WeakEntity>` wiring made correct by remembering to `defer`, and a
   self-documented wrong-sheet-write hazard. This is a correctness architecture that holds because
   nobody has hit the wrong interleaving yet, and Phase 3 is right that it gets strictly harder every
   phase that adds another mirrored field. One observed document-view-model that both views read
   dissolves the mirrors, `SinkShared`, the nine-argument `set_edit_state`, and most of the
   re-entrancy class at once. **This is a multi-week job and it is not in the 30-day plan** — see
   below for why, honestly stated.

**One thing must be relocated:** `fidelity.rs`'s classifier belongs in the engine, deriving from the
DOM the loader already built. As a text scanner in a deliberately parser-free crate it is
structurally incapable of doing its job.

**Nothing needs to be torn out.** `chrome/view.rs` is a mechanical split with no visibility changes.
`worker/run.rs`'s chart third moves into the already-existing empty `worker/charts.rs`. Those are
afternoons, not rewrites.

---

## The Next 30 Days

Ordered. Everything in weeks 1–3 is hours-to-days; nothing before week 4 is a refactor.

**Week 1 — stop the bleeding.**
1. Pin the fork by `rev = "cee2859d…"` in `app/Cargo.toml`, push an immutable `freecell-v1` tag on
   `scosman/ironcalc`, and mirror or `cargo vendor` it. *(~2 hours; removes the single-point build
   extinction risk.)*
2. Add `--locked` to every `cargo` invocation in all six workflows — especially `release.yml` and
   `cargo deny check`, which currently audits a graph nobody reviewed. *(one word per step.)*
3. Wrap `WorkbookDocument::from_source` and `save_workbook` in `catch_unwind` → `LoadFailed` /
   `SaveFailed`; retain the `JoinHandle`; make the window treat "event stream ended without a
   requested `Shutdown`" as fatal and say so. *(a day; converts a silent zombie into a dialog.)*
4. Clamp `m`/`k` in `build_publication` and range-validate `SetFrozen` in `pre_validate`. *(an
   hour; closes a two-click permanent hang, also reachable from a crafted file.)*

**Week 2 — make the product honest.**
5. **Ship the save-fidelity warning that was cut on 2026-07-13.** At save time, diff the original
   package's part inventory against what will be written and name what is being dropped. `reinject`
   already reads the original zip and enumerates parts — this is a dialog on top of data you already
   have. *(~a day. This is the single highest-value item in the plan.)*
6. Add the open→save→reopen **part-inventory** test over `personal_monthly_budget.xlsx`. It makes #5
   non-regressing and is the test whose absence let #5 exist for the project's whole life.
7. Then generalise `is_carry_part` to default-carry / explicit-drop.
8. Count chart-group children at parse time; force `Degraded` for >1. Fix `is_extended_chart`'s bare
   `contains("chartex")`. Correct the `GAPS.md` combo row. *(~2 hours.)*
9. **Take the download badges off `README.md`,** or point them at a page that says alpha and names
   what save does not preserve. *(minutes; the highest reputational-risk item in the repo.)*

**Week 3 — make the gates real.**
10. Make `render` auto-run on a `paths` filter covering `grid/`, `chart/`, `chrome/view.rs`,
    `assets/`, `Cargo.lock`; add an always-posting status context so docs-only PRs can merge.
11. Tighten `fail_fraction` for the text-centric cases (`cell_*`, `spill_*`, `autogrow_*`) — lavapipe
    has 29 runs of demonstrated stability and `diff.rs`'s own comment says to tighten when it does.
12. macOS `cargo check --workspace` on PRs. The stated primary platform is currently gated weekly.
13. Add `cargo tree -i zlog` must-fail / `cargo tree -i ztracing` must-resolve-local to `checks.yml`
    — three lines that remove the last manual step from the most consequential licensing control.

**Week 4 — buy back structure, mechanically, and fix the generator.**
14. Split `chrome/view.rs` into `chrome/view/` child modules (one per feature domain; zero visibility
    changes) and move the ~1,200 chart lines from `worker/run.rs` into the empty `worker/charts.rs`.
    Then add a production-line-count ceiling to CI so it stays done.
15. **The pattern fix.** Sweep the codebase for comments that assert an invariant and convert each to
    a `debug_assert!`, a clamp, an enum, or a test — or delete the claim. Start with the five that
    already failed: "the worker is the only writer" (feature-gate `autogrow_measure_now`), "at most
    one popover open" (`Option<OpenPanel>`), the undo-stack 1:1 claim (return the engine-entry count
    from `apply_one`), the three hand-written row-height `max()`es (one function, three call sites),
    and the `window.defer` rule (one `post_to_sibling` helper that always defers). Then make it a
    standing rule: **a rule you write down is a rule you enforce, or you don't write it down.**

**Deliberately not in the 30 days,** so the plan is honest about what it defers: the shared
document-view-model refactor (weeks, and it competes with everything above), the protocol collapse,
folding `chart-model` into `freecell-core`, and `cargo-fuzz` over `WorkbookDocument::open`. All four
are right; none is more urgent than items 1–9. Schedule the view-model work for days 31–75 and do
not let another feature add a mirrored field before it starts.
