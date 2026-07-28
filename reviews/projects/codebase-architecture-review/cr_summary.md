# Code Review Summary: codebase-architecture-review

Whole-codebase architecture review of FreeCell at `e4c7afa`. Eight phases, eight
independent sub-agents, no shared context. ~99k lines of Rust across five crates.
Fresh-eyes mandate: the project's own specs, experiments and prior decisions were
read as evidence of intent only, never as justification.

## What This Codebase Is

A GPU-rendered (GPUI), Excel-compatible spreadsheet in Rust, wrapping the IronCalc
calculation engine behind a single-threaded worker, with a custom virtualized grid,
a chart subsystem spanning three crates, and a pixel-baseline render harness.
392 commits over 27 days (2026-06-30 → 2026-07-27), built agentically.

## The Verdict

**B− as a codebase. F as a shipping product — and the README currently ships it.**

On layering, panic discipline, algorithmic choices at scale, atomic-save durability,
dependency provenance and packaging, this is at or above the median seed-stage alpha
built by three engineers over six months. It was built in four weeks.

What it is not is safe to give to a stranger with a real workbook. The codebase's
self-reported quality is systematically better than its actual quality, and the gap
lands on user data.

## Key Architectural Decisions — Assessed

| Decision | Assessment |
|---|---|
| Four-crate layering (`core` / `chart-model` / `engine` / `app`) | **Sound and compiler-enforced.** No GPUI in core/engine; no IronCalc type nameable from the app (held by `pub(crate)`, not convention). Verified independently by four reviewers. Best asset in the repo. |
| Worker actor + `ArcSwap` publication | **Sound.** One thread owns the model; UI does one wait-free load per frame; no `block_on`, no engine call on the render path — the last enforced by a process-global counter with a negative control. |
| `Axis` segment sums + O(populated) read queries | **Genuinely Excel-max engineering**, not a claim. Full-column selection on a sparse 1M-row sheet is O(populated). |
| Source-preserving chart save (byte-range splicing) | **The cleverest code in the repo.** Unmodeled DrawingML survives byte-for-byte. Correct answer to a hard problem. |
| Allowlist-based xlsx part preservation | **Wrong, and it is the most dangerous thing here.** See Critical #1. |
| UI state mirrored across `GridView` / `ChromeView` / `SinkShared` | **Wrong.** A correctness architecture that holds because nobody has hit the wrong interleaving yet. |
| `freecell-chart-model` as a zero-dependency crate | **Net negative.** The purity boundary bought a clean manifest and paid with a 383-line duplicate number formatter and a text-scanning fidelity classifier that structurally cannot see combo charts. |
| Engine pinned to a mutable branch of a personal fork | **Unsound as a supply-chain position.** Ten unmerged fixes, none upstreamed, no exit. |

## Quality Assessment

**Test suite: genuinely strong, not theatre.** 1,451 tests, ~3.4 assertions each,
exactly three assertion-free. Production:test ratio **1 : 0.75** workspace-wide
(56,857 : 42,431 LOC); `freecell-engine` is 1 : 1.12. Twenty-two **negative controls**
— including one proving the architecture guard trips on a synthetic violation and one
proving the perf harness isn't measuring a dead counter. These are rare practices.

**Verification design: the weak half.** The gates that exist are well built; the gates
that matter most either don't run, or can't see what they claim to. Nothing in the
project validates that the right number is drawn correctly in a cell.

**Code health:** effectively zero unguarded `unwrap`/`expect` in engine production
code. Zero `TODO`/`FIXME` markers in 99k lines — which is not the strength it appears
to be (see Pattern, below).

## Issues Overview

| Severity | Count | Key Themes |
|----------|-------|------------|
| Critical | 22 | Silent data loss on save; silent worker death; unbounded loops reachable from a click; gates that don't run; a mutable-branch engine pin |
| Moderate | 55 | No single commit point across four shared surfaces; no single source of truth for UI state; protocol grown into a 76-variant RPC surface; duplicated foundations |
| Mild | 39 | Doc drift, naming, dead scaffolding, threshold hygiene |

## Critical Issues

**Ranked by danger, not by phase.**

1. **[`engine/src/chart/save.rs:518`] Save is lossy-by-default and silent.**
   `is_carry_part` carries exactly two prefixes (`xl/charts/`, `xl/drawings/`) over a
   package IronCalc regenerates from scratch (eleven parts). Opening a real `.xlsx`,
   editing one cell, and saving **permanently deletes** pivot tables, macros, comments,
   data validation, hyperlinks, Excel Tables, autofilters, sheet protection, print
   setup, images, and in-cell rich text. Invisible to the test suite **by construction**
   — `tests/roundtrip.rs` only round-trips workbooks FreeCell itself authored.
   `GAPS.md:451` records the warn-before-strip dialog was **cut on 2026-07-13**.

2. **[`worker/run.rs:369`, `:757`] Load and save are the only engine calls outside
   `catch_unwind`.** Six guarded mutation regions exist; the two file-path calls have
   none. The `JoinHandle` is discarded and `send` swallows `SendError`, so a panic
   yields a *silent zombie*: window still rendering, edits vanishing, Save doing
   nothing, no dialog and no log. The pinned exporter has a reachable
   `panic!("Model needs to be evaluated before saving!")`.

3. **[`worker/run.rs:3245-3278`] Unbounded frozen-pane band in `build_publication`.**
   Every loop there is clamped except this one. "Freeze rows" on row 500,000 (or a
   crafted `<pane>`) makes every subsequent publish do ~500,000 × 256 `formatted_value`
   calls. Worker never returns. Two clicks. The doc comment directly above the loop
   asserts it can never be a sheet-size loop.

4. **[`chart/load.rs:605` + `chart-model/src/fidelity.rs:175`] Combo charts truncate
   silently and are badged `Faithful`.** `parse_chart_xml` keeps only the first chart
   group; the fidelity classifier never counts groups. An ordinary Excel bar+line combo
   loads as bars only, line series absent, no badge. `fidelity.rs` — 1,272 lines —
   exists to prevent exactly this and is structurally incapable of catching it.

5. **[`render.yml`] The only automated gate on the product's core differentiator is a
   convention in a markdown file.** The workflow's own header says it "MUST be a
   required status check"; its trigger is `workflow_dispatch:`-only. 29 dispatched runs
   across 13 branches, **zero on `main`**, against 44 merged PRs — and it failed on 7%
   of the runs it did get.

6. **[`render-tests/src/diff.rs`] The pixel gate cannot see what it exists to
   validate.** `fail_fraction: 0.005` permits 384–1,596 differing pixels in scenes whose
   entire non-background content is 8k–25k pixels and whose glyph ink is ~74 pixels. A
   wrong digit passes. Since `#[gpui::test]` installs a `NoopTextSystem`, this suite is
   the *only* place real font metrics are ever exercised.

7. **[`app/Cargo.toml`] The engine is pinned to a mutable branch, and the documented
   maintenance procedure destroys it.** `branch = "freecell-fixes"` on a personal fork
   whose standing procedure is *rebasing*. One force-push + GC makes every historical
   FreeCell commit unbuildable. Every other git dep in the tree is `rev`-pinned. The
   branch carries **ten** `fix/*` merges (the manifest comment claims two), none
   upstreamed, several of them *features* upstream has not agreed to take — so the
   documented exit to a crates.io release has no path. The `= "0.7.1"` pin is fiction.

8. **[`app/crates/freecell-app/src/chrome/view.rs:330`] `ChromeView` is a god object.**
   16,099 lines (8,249 production), one struct, 71 fields, 267 methods, 91 `cx.listener`
   closures, eight independent feature domains. With `grid/view.rs` it is **55% of the
   app crate's production code in two files**. Growth was monotonic — 5,618 → 16,099
   across 34 commits in sixteen days, with one net-negative commit.
   *Loudest problem, not the most dangerous one — and the split is mechanical.*

9. **[UI state] No single source of truth.** Selection lives in three places, active
   sheet in four, pending-edit text in four representations; ten `GridView` fields are
   hand-pushed mirrors through a nine-positional-argument setter. This forced the cyclic
   `Rc<OnceCell<WeakEntity>>` wiring and a `window.defer` convention documented across
   five "BUG #5" comments. Already produced a self-documented wrong-sheet-write hazard
   at `shell/window.rs:2129`.

10. **[scale] The product thesis has no test.** Six committed fixtures, largest 35 KB.
    The 1M-row perf fixture is synthesised in-process and never serialised — parse time,
    peak memory, shared-string blowup and save time at scale are all unmeasured. The
    perf gate that does exist measures element *construction*; there is not one custom
    `Element` impl in `grid/`, so the cost of handing ~2,000 divs to taffy per frame —
    plausibly the dominant term — is never measured.

*(Remaining criticals — the four unversioned shared surfaces, the theme-never-parsed
wrong-color write, `dLbls` whole-node replacement, chart-insert-onto-occupied-sheet
hard failure, missing `--locked` in all six workflows — are detailed in the phase files.)*

## The Pattern Behind the Problems

Twenty-two criticals across concurrency, persistence, XML schema handling, GPUI state,
CI configuration and supply chain. That spread rules out a domain-knowledge gap.

**The project consistently mistakes having *reasoned about* an invariant for having
*enforced* it.** Every invariant a compiler or an existing test happens to check is
held rigorously — crate boundaries, IronCalc containment, dispatch exhaustiveness,
publication ordering, the GPL removal. Every invariant written down in prose instead
has already failed: *"the bands are never a sheet-size loop"* (the comment above the
unbounded loop), *"the worker is the only writer"* (while a UI-thread path takes
`caches.write()`), *"this MUST be a required status check"* (`workflow_dispatch`-only),
*"at most one popover is open"* (eight bools), *"P8 threads the actual `clrScheme`"*
(it doesn't — and the wrong color is written into user files).

Corollary: **zero `TODO`/`FIXME` in 99k lines.** Debt isn't recorded in code; it is
externalized into markdown — the one artifact that cannot fail a build. Which is why
`GAPS.md` is itself wrong about combo charts, in the direction of confidence.

Why this shape is specific to an agentic build: writing an excellent explanatory
comment costs an LLM the same as writing a mediocre one, while building an enforcer is
a separate task nobody asked for. The prose is generated from the same intent as the
code, in the same act — so it describes the intent perfectly and drifts the instant the
intent changes.

**The good news is that this failure mode is mechanically fixable and requires no
redesign.** Every prose-only invariant can become a `debug_assert!`, a clamp, an enum,
a test, or three lines of YAML. Twenty-two negative controls prove the project can
build first-rate enforcers — it just hasn't built one every time it wrote down a rule.

## Is the Architecture Sound Enough to Build On?

**Yes. Keep the architecture; rebuild two subsystems and stop trusting three claims.**

*Keep unchanged:* the crate graph and dependency rule; the worker actor model and
`ArcSwap` publication; `Axis` and the O(populated) read queries; the chart save patcher;
the atomic save mechanics; the pure-logic extractions (`grid/layout.rs`,
`grid/chart_layer.rs`, `shell/lifecycle.rs`, core's reducers).

*Redo, not refactor:* (1) the save preservation model — invert allowlist to
default-carry / explicit-drop; the `reinject` seam already exists. (2) UI state
ownership — one observed document-view-model dissolves the mirrors, `SinkShared`, the
nine-argument setter and most of the re-entrancy class at once. This is a multi-week
job and is deliberately **not** in the 30-day plan.

*Relocate:* `fidelity.rs`'s classifier belongs in the engine, deriving from the DOM the
loader already built.

*Tear out:* nothing. `chrome/view.rs` is a mechanical split with zero visibility
changes (Rust module-descendant privacy); `worker/run.rs`'s chart third moves into the
already-existing empty `worker/charts.rs`. Afternoons, not rewrites.

## Recommendations — Next 30 Days

**Week 1 — stop the bleeding.**
1. Pin the fork by `rev`, push an immutable tag on `scosman/ironcalc`, mirror or vendor
   it. (~2 hours; removes single-point build extinction.)
2. Add `--locked` to every cargo invocation in all six workflows — especially
   `release.yml` and `cargo deny check`.
3. Wrap `from_source` / `save_workbook` in `catch_unwind`; retain the `JoinHandle`;
   treat an unrequested event-stream end as fatal and say so. (A day; converts a silent
   zombie into a dialog.)
4. Clamp `m`/`k` in `build_publication`; range-validate `SetFrozen`. (An hour.)

**Week 2 — make the product honest.**
5. **Ship the save-fidelity warning that was cut on 2026-07-13.** `reinject` already
   enumerates the original package's parts — this is a dialog over data you have.
   *Highest-value item in the plan.*
6. Add the open→save→reopen **part-inventory** test over `personal_monthly_budget.xlsx`
   (already in the repo). Its absence is what let #5 exist for the project's whole life.
7. Generalise `is_carry_part` to default-carry / explicit-drop.
8. Count chart-group children at parse time; force `Degraded` for >1. Correct the
   `GAPS.md` combo row.
9. **Take the download badges off `README.md`,** or point them at a page that says
   alpha and names what save does not preserve. *Highest reputational risk in the repo.*

**Week 3 — make the gates real.**
10. Make `render` auto-run on a `paths` filter; add an always-posting status context.
11. Tighten `fail_fraction` for text-centric cases (29 runs of demonstrated stability).
12. macOS `cargo check --workspace` on PRs — the stated primary platform is gated weekly.
13. Add `cargo tree -i zlog` must-fail / `ztracing` must-resolve-local to `checks.yml`.

**Week 4 — buy back structure, and fix the generator.**
14. Split `chrome/view.rs` into `chrome/view/` child modules; move the ~1,200 chart
    lines from `worker/run.rs` into `worker/charts.rs`. Add a production-line ceiling
    to CI so it stays done.
15. **The pattern fix.** Sweep for comments that assert an invariant; convert each to a
    `debug_assert!`, clamp, enum or test — or delete the claim. Standing rule:
    *a rule you write down is a rule you enforce, or you don't write it down.*

**Deliberately deferred** (right, but not more urgent than 1–9): the shared
document-view-model refactor (schedule days 31–75), the protocol collapse, folding
`chart-model` into `freecell-core`, `cargo-fuzz` over `WorkbookDocument::open`.

## Review Artifacts

| File | Contents |
|---|---|
| `cr_plan.md` | Scope and phase design |
| `phase_1_feedback.md` | Crate boundaries & module structure |
| `phase_2_feedback.md` | Engine core & concurrency |
| `phase_3_feedback.md` | UI architecture (GPUI) |
| `phase_4_feedback.md` | Chart subsystem |
| `phase_5_feedback.md` | Persistence & data fidelity |
| `phase_6_feedback.md` | Testing strategy & confidence |
| `phase_7_feedback.md` | Build, dependencies, licensing, shipping |
| `phase_8_verdict.md` | **Fresh-eyes verdict** — good / bad / ugly, disagreements with the phase reviews, the pattern, the 30-day plan |
