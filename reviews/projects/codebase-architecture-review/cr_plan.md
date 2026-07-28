# Code Review Plan: codebase-architecture-review

## Scope

**Not a diff review.** Whole-codebase architecture review of `app/` at HEAD
(`e4c7afa`), ~99k lines of Rust across 5 workspace crates + vendored stubs.

- Branch: `claude/codebase-architecture-review-p9l7l5`
- Mode: fresh-eyes senior-architect assessment — good / bad / ugly
- Explicitly **out of scope**: line-by-line nitpicking; validating the repo's own
  specs, experiments, or prior decisions. Reviewers judge the code as it stands,
  not the reasoning that produced it.

## Spec Context

None used deliberately. `specs/`, `experiments/`, `GAPS.md` and `PROJECTS.md` are
available to reviewers as *evidence about intent*, but are **not** accepted as
justification for what the code does.

## Codebase shape (for phase design)

| Crate | LOC | Role |
|---|---|---|
| `freecell-app` | 44.6k | GPUI UI — grid, chrome, charts, shell |
| `freecell-engine` | 32.4k | IronCalc wrapper, worker thread, xlsx/chart OOXML |
| `freecell-core` | 9.7k | shared pure logic (no gpui, no ironcalc) |
| `freecell-chart-model` | 4.5k | gpui-free/ironcalc-free chart data model |
| `render-tests` | 7.9k | pixel/perf harness |

Largest files: `chrome/view.rs` (16,099), `grid/view.rs` (10,627),
`worker/run.rs` (9,288), `document.rs` (3,833).

## Phases

- [x] Phase 1: Crate boundaries & module structure — layering, dependency direction, the 16k-line-file problem, what belongs where
- [x] Phase 2: Engine core & concurrency — worker seam, protocol, publication/snapshot model, cache invalidation, cancellation, failure modes
- [x] Phase 3: UI architecture (GPUI) — state ownership, render path, virtualization, input, chrome↔grid↔shell coupling, testability
- [x] Phase 4: Chart subsystem vertical slice — chart-model / engine::chart / app::chart across three crates
- [x] Phase 5: Persistence & data fidelity — xlsx roundtrip, styles/cond-fmt conversion, data-loss risk, IronCalc fork coupling
- [x] Phase 6: Testing strategy & confidence — what the suite actually proves, the manual pixel gate, perf harness credibility
- [x] Phase 7: Build, dependencies, licensing & shipping posture — pinned zed rev, GPL patch-out, fork `[patch.crates-io]`, CI gates, packaging
- [x] Phase 8: Fresh-eyes verdict (synthesis) — reads phases 1–7 + independent sampling; answers "is this codebase any good?"

Phases 1–7 run as independent sub-agents. Phase 8 runs last, over their output.
