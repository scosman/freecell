---
status: complete
---

# Functional Spec: chrome-view-split

## 1. What this is

A **pure structural move**. `app/crates/freecell-app/src/chrome/view.rs` becomes the
directory `app/crates/freecell-app/src/chrome/view/`, with its contents distributed across
child modules by feature domain. Not one line of behaviour changes.

The "functional" surface of this project is therefore unusual: there are no user-facing
features to specify. What needs specifying is **what must not change**, **what is allowed to
change**, and **how we prove it**.

## 2. Measured baseline (re-measured at HEAD, commit `6e05e76`)

The overview's numbers came from a review agent that read the file without compiling it.
Re-measured:

| Metric | Overview claim | Measured at HEAD | Verdict |
|---|---|---|---|
| Total lines | 16,099 | **16,099** | exact |
| Production lines | 8,249 | **8,248** (lines 1–8248) | off by one |
| Inline test lines | ~7,851 | **7,851** (lines 8249–16099) | exact |
| `ChromeView` fields | 71 | **69** | overview over-counted by 2 |
| Methods (both `impl` blocks incl. tests) | 580 | **580** | exact |
| Production methods | 267 | **265** | ~exact |
| `render_*` methods | 35 | **35** | exact |
| Feature domains | "at least eight" | **nine**, cleanly separable | confirmed |

The file has **not** moved since the review. Structure at HEAD:

- **1–555** — imports, look constants, domain constant tables, `CfMenu`, `Anchor`,
  `ChartPanel`/`ChartPanelSeries`, `TabDrag`/`TabSpan`, two free tab-geometry functions, and
  the `struct ChromeView` definition (69 fields, lines 330–555).
- **556–3927** — `impl ChromeView` #1 (behaviour: `new`, plumbing, all the action methods).
- **3928–4081** — four free helper functions (colour + border-icon + font-size formatting).
- **4082–4115** — `impl Focusable` and `impl Render`.
- **4116–7572** — `impl ChromeView` #2 (all 35 `render_*` methods + find behaviour).
- **7573–8248** — CF free functions and constant tables (labels, validation, spec building).
- **8249–8379** — `#[cfg(test)] impl ChromeView` — 15 test seams.
- **8380–16099** — a single flat `mod tests` — 254 tests (245 `#[gpui::test]`, 9 `#[test]`)
  plus **44 helper functions**, three structs (`Harness` and two grid-body stubs) and one type
  alias, with nothing marking which items are fixtures and which are tests.

### 2.1 The finding that makes this cheap

**The file is already sectioned, and the two halves agree.** Production carries 17 `// ----`
banner comments; the test module carries 27. The section names line up — `// ---- Action
row: SetBorders (pen popover)` exists in both halves, as does `// ---- Sheet-tab reorder
drag`, `// ---- Conditional-formatting rules list (P5)`, and so on.

This is the single most important measurement in the project. It means domain boundaries do
not have to be *invented* — they are already written into the file by the people who grew it,
and the mapping from production section to test section is mechanical rather than a judgement
call. It is the reason this split can be done as a sequence of contiguous-range moves rather
than a per-method triage.

## 3. Target structure

Ten production files — `mod.rs` plus nine domain modules — and a test-support module, under
`chrome/view/`:

| File | Domain | Prod lines (est.) | Test lines (est.) |
|---|---|---:|---:|
| `mod.rs` | struct, `new`, shared consts/types, module decls | ~600 | 0 |
| `shell.rs` | `Render`/`Focusable`, action row, overlays, worker-event router, degrade | ~770 | ~325 |
| `editing.rs` | data row, in-cell editor, quick-edit, autocomplete, sig hints | ~1,130 | ~1,930 |
| `formatting.rs` | style toggles, fill/text colour, number format, font, borders | ~1,460 | ~1,610 |
| `charts.rs` | insert-chart menu, chart edit panel + its chrome | ~850 | ~840 |
| `cf_sidebar.rs` | CF sidebar open/close/refresh, rules list, row rendering | ~465 | ~830 |
| `cf_editor.rs` | CF rule editor: state, operands, formats, scales, save | ~1,730 | ~1,095 |
| `tabs.rs` | sheet tab bar, reorder drag, rename, context menu, delete | ~695 | ~405 |
| `find.rs` | find/replace bar + behaviour | ~410 | ~350 |
| `stats.rs` | selection-stats readout | ~110 | ~195 |
| `test_support.rs` | `Harness` + the cross-domain helpers + 2 body stubs, the 15 test seams | 0 | ~370 |

Every production count is under the 2,000 ceiling that F2 will enforce, with the largest
(`cf_editor.rs`, ~1,730) having ~270 lines of headroom. **No exemption is requested.**

### 3.1 Why conditional formatting splits in two

CF is the one domain that does not fit under 2,000 as a single file (~2,190 production
lines). It is split at a real seam, not an arbitrary line number: the **sidebar** (open,
close, refresh, re-scope, the rules list, per-row raise/lower/delete) versus the **rule
editor** (the modal editing state, its operand/format/scale controls, validation, and save).
These are separate features in `components/cf_sidebar.md` — they shipped as separate phases
(P4/P5 vs P6) and have separate test banner sections.

### 3.2 Naming

`chrome/` already has sibling modules `edit.rs` and `cond_fmt.rs` holding extracted state
types. The new children are named to avoid confusion with them: `editing.rs` (not `edit.rs`),
`cf_sidebar.rs` / `cf_editor.rs` (not `cond_fmt.rs`).

## 4. Invariants — what must not change

1. **Public API.** `chrome::view` currently exports `ChromeView`, `ChartPanel`,
   `ChartPanelSeries` (re-exported from `chrome/mod.rs`). After the split, `chrome/mod.rs`'s
   `pub use view::{ChartPanel, ChartPanelSeries, ChromeView};` line is **unchanged**, and no
   other crate or module sees a different path for anything.
2. **`ChromeView` stays one struct with all 69 fields.** No field is moved, renamed, split,
   consolidated, or re-typed. Splitting state is E2's job.
3. **No logic changes.** Method bodies move verbatim. No renames, no signature changes, no
   extracted helpers, no "while I'm here" simplifications, no clippy-suggested rewrites.
4. **No new abstractions.** No traits, no delegation layer, no builder, no `impl` splitting a
   method across modules.
5. **Test count does not drop.** 254 tests before, 254 after. Every test keeps its name.
6. **Rendering is untouched.** No pixel baseline may move.

## 5. What is allowed to change

Exactly three categories. Anything outside them is out of scope.

1. **Item visibility, widening only, and only where the compiler demands it.** Rust's
   module-descendant privacy gives child modules free access to the *parent's* private items
   — which is what makes the 69 fields cost nothing. It does **not** work sideways: a private
   `fn` that moves into `view::tabs` becomes invisible to `view::formatting`. Every such item
   gets `pub(super)`, which scopes it to the `view` subtree exactly as `private-to-view.rs`
   scoped it before. Nothing becomes `pub` or `pub(crate)` that was not already.

   This is the one place the overview is imprecise: "zero visibility changes to any of the 71
   fields" is true, and remains true — but it is a statement about *fields*, not about
   methods and free functions. Cross-domain private callees will need annotating. The set is
   discovered by the compiler, not guessed.

2. **Import placement.** Each child gets the imports it needs. `use super::*;` carries the
   shared set down from `mod.rs`.

3. **Module-level doc comments.** Each new file gets a `//!` header naming its domain. This
   is additive documentation, not a behaviour change.

## 6. Verification contract

Per the overview: verification is the whole point. At the end of **every** phase:

- `cargo build -p freecell-app` — clean.
- `cargo test -p freecell-app --lib` — green, and the reported test count is **exactly** the
  baseline. A dropped test is a silently-lost `mod`; the count is the tripwire that catches
  it, and the reason it is checked per-phase rather than at the end.
- `cargo fmt --all --check` — clean (whole workspace, per `CLAUDE.md`).
- `cargo clippy -p freecell-app --all-targets -- -D warnings` — clean. CI gates on this, and
  a split is exactly the change that strands an unused import.

At the end of the project, additionally:

- `cargo test -p freecell-app` (all targets) and a workspace build, once.
- An Xvfb smoke launch: `xvfb-run -a cargo run -p freecell-app` opens the welcome window.

### 6.1 The strongest available check

Because this is a pure move, we can assert something stronger than "tests pass": the multiset
of test **names** before and after must be identical.

The measured baseline, captured at `6e05e76` before any move:

- **610** tests in the `freecell-app` lib target, all passing.
- **254** of them under `chrome::view::tests` — the ones this project relocates.

The comparison is on the **leaf name** (the segment after the last `::`), not the full path,
because the full paths change by design: `chrome::view::tests::tab_drag_reorders` becomes
`chrome::view::tabs::tests::tab_drag_reorders`. Leaf names must be preserved exactly. (Three
leaf names are duplicated elsewhere in the crate; comparing sorted multisets handles that.)

Each phase runs the check as: list the test binary's tests, strip module paths, sort, and
diff against the stored baseline. An empty diff proves no test was dropped, renamed, or
silently `cfg`'d out of the build — a guarantee a passing *count* alone does not give, since
one test lost and one gained would net to zero.

## 7. Render tests

`chrome/view.rs` renders the action row, data/formula row, find bar, sheet tabs, the CF
sidebar and the chart panel. Per `CLAUDE.md` §Render tests → Scope, **none of these have
pixel baselines**. The pixel suite covers grid/cell/sheet rendering, the macOS titlebar row,
and standalone chart *render scenes* (`freecell_app::chart`, a different module from
`chrome::view`'s chart *panel*).

So: **the pixel suite is not run for this project**, full or subset. Nothing here can move a
baseline. If a phase finds itself touching `grid/` or `chart/` render code, that is a signal
the move was not pure — stop and reconsider rather than regenerate a baseline.

## 8. Out of scope

- Any state-model change (E2): field ownership, mirrored fields, the
  `Rc<OnceCell<WeakEntity>>` wiring.
- `worker/run.rs` and its chart extraction — owned by the parallel
  **engine-worker-hardening** project.
- `engine/chart/*`, manifests, workflows — owned by parallel projects.
- `grid/view.rs`. Measured at HEAD: **10,627 lines, 6,575 production** — over three times the
  F2 ceiling and the obvious next unit. Recorded in the backlog (§9), not touched.
- The F2 CI ceiling itself.
- Fixing any bug found along the way. Bugs get written down in `findings.md`, not fixed —
  a mixed diff is the one failure mode that makes this project not worth doing.

## 9. Deliverables beyond the code

- `specs/projects/chrome-view-split/findings.md` — anything noticed and deliberately not
  acted on: suspected bugs, dead code, duplicated logic, oddities. Written as observations
  with file/line references, so a later project can pick them up.
- A `projects/` backlog entry for `grid/view.rs` per `CLAUDE.md` §Projects backlog, since the
  overview asks to "note where it stands and file it if it warrants its own unit" — at 6,575
  production lines it plainly does.
