# Phase 1: Crate boundaries & module structure

Scope: the `app/` workspace only (`crates/freecell-core`, `crates/freecell-chart-model`,
`crates/freecell-engine`, `crates/freecell-app`, `render-tests`). `experiments/` is correctly
excluded from the workspace and is not judged here.

**Baseline measurements** (production lines = total minus top-level `#[cfg(test)] mod` blocks and
`tests/` targets; measured, not estimated):

| Crate | prod LOC | test LOC | total |
|---|---|---|---|
| freecell-core | 5,678 | 4,067 | 9,720 |
| freecell-chart-model | 2,755 | 1,784 | 4,529 |
| freecell-engine | 15,457 | 16,971 | 32,399 |
| freecell-app | 27,052 | 17,622 | 44,627 |
| render-tests | 6,684 | 1,218 | 7,886 |
| **total** | **57,626** | **41,662** | **99,161** |

---

## What's Good

**The layering rule is real, not aspirational, and it holds.** I went looking for leaks and did not
find them. `freecell-core` and `freecell-chart-model` have zero GPUI and zero IronCalc dependencies;
`freecell-engine` has IronCalc but no GPUI; `freecell-app` has GPUI but no IronCalc. More
importantly, the *type* boundary holds where it matters: `freecell_engine::document`'s
`user_model_mut()` / `user_model()` / `workbook_theme()` are `pub(crate)`
(`crates/freecell-engine/src/document.rs:1142,1889,1903`), so no `ironcalc_base` type is nameable
from `freecell-app`. The only `ironcalc` strings in the app crate are a URL in the About window and
three doc comments. That is a genuinely well-defended seam and it is the single most valuable
structural asset in this codebase — it is what lets ~57% of the code be tested headless.

**The pure-logic extraction pattern is executed well and consistently.** The hard, algorithmic,
regression-prone parts have been lifted out from under GPUI:
`crates/freecell-app/src/grid/layout.rs` (1,402 lines of offset/hit-test/scroll/scrollbar math, no
gpui, no engine), `crates/freecell-app/src/grid/chart_layer.rs` (anchor→pixel + culling behind a
4-method `GridGeometry` trait, so it unit-tests without a `Frame`),
`crates/freecell-app/src/shell/registry.rs` + `lifecycle.rs` (window dedupe, dirty accounting,
quit-prompt ordering as pure decisions), and most of `freecell-core`'s reducers (`data_row`,
`eval_indicator`, `selection`, `input_cap`, `sheet_name`, `merge`). This is the right instinct and
it is not cosmetic — it is why 42% of the repo can be test code.

**The seam traits are narrow and purposeful, with real doubles.**
`crates/freecell-app/src/chrome/client.rs`'s `ChromeClient` is 8 methods, deliberately excludes the
publication/generation (documented as "those are the grid's"), and has a `RecordingClient` double.
`GridGeometry`, `ChromeGridSink`, and `GridEventSink` are similarly minimal. These are the seams a
new owner would actually want, and they exist.

**The intra-app module DAG is clean — cleaner than the file sizes suggest.** `shell` is the only
module that imports both `grid` and `chrome`. The `chrome → grid` edge is exactly one symbol
(`caret_intent_modifiers`); the `grid → chrome` edge is exactly one type (`AutocompleteDisplay`);
`chart` is a leaf that imports nothing from siblings. That is textbook composition-root discipline,
and it means the *crate-level* refactoring cost of splitting `chrome` is near zero.

**`freecell-core::cache` vs `freecell-engine::cache` is a correct split, not drift.** I checked this
specifically because it looks like duplication (1,301 vs 1,476 lines). It is not: core holds the
read model (`SheetCache` data + accessors + `SheetCacheBuilder`), engine holds the IronCalc-facing
builder/mutator and the unit-conversion constants. No shared logic is copied. Same verdict for the
other flagged pairs: `core/palette.rs` (fill swatches) vs `app/chart/palette.rs` (series color
cycle) and `chart-model/authoring.rs` (insert templates) vs `engine/chart/authoring.rs` (`.xlsx`
fixture generator) are unrelated code that happens to share a filename. Those three "duplication"
suspicions are false positives and should not be chased.

**The worker actor model has a single, defensible ownership story for engine state.** One thread
owns the `UserModel`; the UI reads an `ArcSwap<Publication>` and an `RwLock<SheetCaches>` that only
the worker writes. Single-writer, publish-then-bump. That is the right shape for this problem and it
is respected throughout.

---

## Critical (must fix)

- **`crates/freecell-app/src/chrome/view.rs` — a 16,099-line god object that is the single largest
  liability in the codebase**

  Hard numbers: 16,099 total lines = **8,249 production** + 7,851 test. That is **30% of
  `freecell-app`'s entire production code in one file**. It contains one struct, `ChromeView`, with
  **71 fields** (lines 330–556) and **580 methods** across two `impl` blocks of 3,372 lines
  (556–3928) and 3,457 lines (4116–7573), plus ~675 lines of free helper functions and constants.

  It is not "large but cohesive." Reading the method list, `ChromeView` is the sole owner of at
  least **fourteen** independent feature domains:

  1. selection-stats readout + debounce (`request_selection_stats`, `stats_seq`)
  2. the formula-bar/in-cell edit controller integration (`begin_typed`, `commit_and_move`,
     `quick_edit`, `committed_cell`, …)
  3. function autocomplete + signature hints (11 methods, its own `Rc<Vec<SharedString>>` cache)
  4. formula point-mode reference insertion + ref highlighting
  5. worker-event routing (`on_worker_event`)
  6. character/paragraph styling (bold/italic/fill/text-color/align/valign)
  7. number formats + decimals/thousands
  8. font family + font size dropdowns
  9. the borders "pen" (target/line/color/picker — 4 fields, 9 methods)
  10. chart insert menu
  11. the chart edit panel (title, legend, axis titles, series colors, data labels, range, type —
      3 `InputState` entities, ~18 methods)
  12. **the entire conditional-formatting sidebar and rule editor** (~55 methods, 4 `InputState`
      entities, a `Vec<Entity<InputState>>`, `cf_build_spec`/`cf_state_from_spec`/`cf_validate`,
      11 dropdown tables — this alone is a 2,000+ line component)
  13. sheet tabs (select/add/rename/drag-reorder/context-menu/delete-confirm — 8 fields,
      ~20 methods)
  14. find/replace (10 fields, ~18 methods)

  The struct's own field block is already comment-sectioned into these clusters (`// ---- Find /
  replace bar`, `// ---- Selection stats`), which is the author admitting the file wants to be
  several files.

  **The growth curve is the damning part.** Across the 34 commits that touched this file it went
  5,618 → 6,281 → 7,425 → 9,365 → 10,718 → 13,865 → 15,472 → **16,099** lines, monotonically, in
  sixteen days. Exactly one commit removed net lines (−134). Nothing in the process — not review, not
  CI, not convention — resists accretion here. Every new chrome feature has been added by appending
  to this file, and the next one will be too.

  What it costs, concretely: no new owner can hold this file. `cargo` cannot tell you which of the
  580 methods are reachable from where. A change to the CF editor recompiles and re-runs the tests
  for find/replace, sheet tabs, and the chart panel. Two people cannot work on chrome concurrently
  without conflicting. `#[cfg(test)]` module privacy is why `WorkbookWindow` has ~20
  `pub(crate) *_for_test` accessors (see Moderate below) — the file boundary *is* the test boundary,
  so tests that need to reach across it force holes in production types.

  **Nothing prevents the split, and the codebase already knows how.** Two state types were already
  extracted to `chrome/cond_fmt.rs` (118 lines) and the shared dock shell to `chrome/sidebar.rs`
  (116 lines) — the extraction was *started and abandoned* after moving the structs. Rust privacy is
  module-and-descendants, so converting `chrome/view.rs` into `chrome/view/mod.rs` with
  `view/cond_fmt_editor.rs`, `view/chart_panel.rs`, `view/find.rs`, `view/tabs.rs`,
  `view/action_row.rs`, `view/borders.rs`, `view/autocomplete.rs` lets each child module carry its
  own `impl ChromeView` block **with zero visibility changes to any of the 71 private fields** and
  each with its own co-located tests. This is a mechanical, low-risk move that can be done in one
  pass. The harder, better follow-up is to give the CF sidebar and chart panel their own gpui
  `Entity` types (the way `EditController` and `HScroller` already are) so their state leaves
  `ChromeView` entirely — but the mechanical split alone would cut this file by ~75% and should be
  done first.

- **The same failure mode in `grid/view.rs` and `worker/run.rs` — there is no module-size
  discipline anywhere in the codebase**

  `crates/freecell-app/src/grid/view.rs`: 10,627 lines = **6,576 production** + 4,052 test.
  `GridView` has **45 fields** and hosts, in one file: frame/quadrant geometry resolution
  (`resolve_frame`, ~250 lines), five separate drag state machines (cell-selection, header,
  resize, fill-handle, point-mode, chart move/resize), edge auto-scroll, three context menus
  (header/cell/chart) with their item tables and element builders, autofit width *and* height
  measurement, wrap auto-grow measurement, chart hit-testing, and the actual painting
  (`build_quadrant` ~500 lines, `build_grid_layers` ~425 lines).

  `crates/freecell-engine/src/worker/run.rs`: 9,288 lines = **3,985 production** + 5,304 test — 26%
  of the engine crate's production code. `Worker` has ~24 fields and ~90 methods spanning the
  command loop, edit application, clipboard, find/replace, cache maintenance, wrap auto-grow,
  publication building, save, and — occupying lines 2242–3213, roughly a quarter of the production
  file — the entire chart CRUD + chart undo/redo subsystem, which has its own eight-variant
  `ChartUndo` enum and eight chart-specific fields on `Worker`.

  Together, `chrome/view.rs` + `grid/view.rs` are **55% of `freecell-app`'s production code in two
  files**. This is not three unlucky files; it is the codebase's default shape. Whatever process
  produced it (phase-by-phase feature addition into the existing view) has no counter-pressure, and
  it will keep producing it. The concrete ask: pick a hard ceiling (e.g. 1,500 production lines per
  file), enforce it in CI, and pay down the three files above. Without a mechanical gate this will
  regress within a month — the growth curve above is the evidence.

---

## Moderate (should fix)

- **`crates/freecell-core` — the seam is drawn on the *technology* axis, not the *responsibility*
  axis, and it has become a junk drawer as a result**

  The crate's charter is literally negative: "GPUI-free, IronCalc-free." That is a purity property,
  not a responsibility, so anything pure lands here regardless of what layer it belongs to. The
  result mixes spreadsheet domain (`refs`, `selection`, `style`, `cache`, `merge`, `publication`),
  grid rendering geometry (`axis`), UI view-models (`data_row`, `eval_indicator`, `format_ui`,
  `functions`), OS-level application state (`recent`), and benchmark tooling (`perf`).

  Concretely, ~2,000 of core's 5,678 production lines have exactly **one** downstream consumer and
  do not belong in a shared domain crate:
  - `perf.rs` (647 lines) — a benchmark-harness config/script contract, consumed only by
    `render-tests` bins and a `pub` perf hook bolted onto `GridView` (`grid/view.rs:5117`).
  - `recent.rs` (610 lines) — the recent-files store, consumed only by `freecell-app::shell`. It
    also does `std::fs::write` / file-stat I/O (`recent.rs:16,108`), which directly contradicts the
    crate's own "pure logic" module doc. A crate whose stated value is "builds and tests anywhere
    with no GPU or display" should not be the one touching the filesystem.
  - `functions.rs` (800 lines) — the formula-function autocomplete catalog, consumed only by
    `freecell-app::chrome`.
  - `format_ui.rs` (770 lines) — action-row dropdown labels and decimals arithmetic, likewise
    chrome-only.

  Direction: move the single-consumer UI/tooling modules down into their consumer (`app::chrome`,
  `app::shell`, `render-tests`), and let `freecell-core` mean *spreadsheet domain model + read
  models*. The purity guarantee survives — those modules are still pure — but the crate regains a
  describable responsibility. Today, if you ask "what is `freecell-core`?", the only honest answer
  is "the pure stuff", which is not an architecture.

- **`crates/freecell-chart-model` is a second, disconnected foundation crate — and the duplication
  it causes is real**

  `freecell-chart-model` has **zero dependencies**, including on `freecell-core`. Its justification
  (a gpui-free/ironcalc-free seam between `engine::chart` and `app::chart`) describes a property
  `freecell-core` already has and already provides for the non-chart half of the app. Having two
  parallel, mutually-unaware "pure shared model" crates guarantees primitive duplication, and it has
  already happened:

  - `freecell_chart_model::Color { r, g, b: u8 }` with `from_hex`/`to_hex` is byte-for-byte the same
    type as `freecell_core::Rgb { r, g, b: u8 }` with `from_hex`/`to_hex`. The app pays for it with a
    manual bridge (`shell/window.rs:1650 fn chart_color_rgb`), and the engine has to speak both.
  - **More serious:** `chart-model/src/numfmt.rs` (383 lines) is an independent reimplementation of
    OOXML number-format-code application — affixes, thousands grouping, decimals, percent scale —
    while *cell* display formatting goes through IronCalc's `format_number` in the engine. The same
    `#,##0.00` on a chart axis and on the cells it plots is rendered by two different
    implementations, one of which explicitly falls back to general formatting for dates/scientific.
    That is a visible-divergence bug waiting to happen and a second grammar to maintain.

  Direction: either fold `chart-model` in as `freecell_core::chart` (my preference — it costs
  nothing, it is already pure, and it kills the primitive duplication), or at minimum make
  `chart-model` depend on `freecell-core` for shared primitives and route chart number formatting
  through the same implementation the cells use. Keeping two independent foundation crates is
  premature fragmentation that is already charging interest.

- **`pub` is used as the default reach, not as a deliberate API decision**

  `freecell-app` — a binary crate — declares **471 `pub`** items against only 87 `pub(crate)`. Its
  one external consumer, `render-tests`, uses exactly **six** symbols: `grid::GridView`,
  `grid::GridDataSources`, `grid::chart_layer`, `chart::chart_element`, `shell::register_fonts`,
  `shell::titlebar::titlebar_row`. Everything else marked `pub` is either cross-*module* access
  within the crate (which should be `pub(crate)`) or genuinely unreachable. The cost is not
  theoretical: `dead_code` analysis is disabled for all 471 items, and there is no way to read the
  crate and know what its real contract is. `freecell-core` is the mirror image — 316 `pub`, **zero**
  `pub(crate)` — so it has no internal encapsulation at all and every helper is API.

  Direction: default to `pub(crate)`, promote to `pub` only for the six render-tests symbols in the
  app crate, and let the compiler tell you what is dead.

- **Test-only API is drilled into production types (27 `*_for_test` functions)**

  `WorkbookWindow` alone exposes ~20 `pub(crate) fn *_for_test` accessors
  (`shell/window.rs:1111–1298`): `set_dirty_for_test`, `inject_worker_event_for_test`,
  `arm_pending_edge_for_test`, `grid_for_test`, `client_for_test`, `chrome_for_test`,
  `route_selection_changed_for_test`, `switch_sheet_for_test`, … Their only callers live in
  `shell/app.rs`'s test module. This is a direct symptom of the god-file problem: because the tests
  for `window.rs` live in a *different file*, module privacy forces production holes. It also means
  the production type carries a second, parallel, test-shaped interface that the compiler will
  happily let production code call.

  Fixing the file split (Critical) largely dissolves this; the residue should move behind a
  `#[cfg(test)] impl` block or a dedicated test-support module.

- **`crates/freecell-app/src/shell/window.rs` is the app's coupling hub and is almost untested
  in-file**

  2,185 lines, of which **2,154 are production and 31 are test**. It is simultaneously the
  composition root (`build` wires grid + chrome + worker), the worker-event router (`on_worker_event`
  is ~250 lines), the save/save-as/export-CSV flow, the modal state machine, the degraded-mode bar,
  and the home of three free-function sink factories (`make_grid_sink` 311 lines,
  `make_chrome_grid_sink` 120 lines, `switch_grid_to_sheet`). Its tests live remotely in `app.rs`.

  A composition root this dense is where cross-cutting bugs live, and it currently has neither
  co-located tests nor sub-structure. At minimum the event router, the save/export flows, and the
  sink factories should be separate modules under `shell/window/` with their own tests.

- **The dependency guard is real, but its coverage does not match its billing**

  `crates/freecell-core/tests/dependency_rule.rs` is better than most such guards: it parses inline,
  dotted-sub-table, and target-scoped dependency forms, correctly skips dev-dependencies, and — the
  part that makes it not theatre — carries **negative controls** (`guard_detects_a_forbidden_dependency`,
  `guard_catches_dotted_subtable_and_target_forms`) that prove the scanner and the assertion both
  trip. Credit where due.

  But it only asserts two facts: core has no `gpui*`/`ironcalc*`, engine has no `gpui*`. It does not
  cover `freecell-chart-model` at all (which today has zero deps and so is one careless commit from
  silently gaining one), and it enforces nothing about layering *inside* `freecell-app`, which is
  where all the structural decay actually is. It is a guard on the one boundary that is already
  healthy and no guard on the boundaries that are failing. Extend it to chart-model, and add the
  file-size / module-boundary gate that the Critical findings call for — that is where enforcement
  buys something.

---

## Mild (consider fixing)

- **Filename collisions across layers defeat navigation.** `authoring.rs` exists in
  `chart-model/src/` (insert templates) and `engine/src/chart/` (an `.xlsx` fixture writer) meaning
  entirely different things. Same for `chrome.rs` (`engine/src/chart/chrome.rs` = OOXML chrome
  serializers; `app/src/chart/chrome.rs` = the gpui title/legend frame — the word "chrome" also
  already means the app's action/data/tab bars), `cache.rs` ×2, `palette.rs` ×2, and four
  CF-related files named `cond_fmt*`. None of these are duplication, but grepping for a concept
  lands you in the wrong layer, and a new owner will mis-edit at least once. Rename toward
  responsibility: `chart_xml_fixtures.rs`, `chart_chrome_xml.rs`, `chart_frame.rs`,
  `series_colors.rs`.

- **Test-fixture generators ship in production crates.**
  `freecell_engine::chart::authoring` is a `pub mod` with 557 production lines whose only callers
  are `#[cfg(test)]` blocks and `tests/*.rs` inside the same crate. `engine::fixtures` (also `pub`)
  is at least justified — `render-tests` needs it cross-crate — but `chart::authoring` is not; it
  should be `#[cfg(test)]`-gated or moved to a `tests/support/` module. As-is it is compiled into
  every release build of the app.

- **`freecell_core::perf` reaching back into `GridView`.** The perf contract in core forces a `pub`
  test/bench hook onto the production `GridView` (`perf_scroll_step`, `grid/view.rs:5117`), which
  returns a `freecell_core::perf::FrameSample`. A benchmark harness should not be able to make the
  production view type grow methods. Move the contract to `render-tests` and have the harness drive
  the view through its normal API.

- **`Command` at 53 variants and `WorkerEvent` at 23** in a single `protocol.rs` is at the edge of
  what one enum should carry, and `run.rs`'s `process_batch` has to match all of them. Not wrong
  yet — a spreadsheet protocol is genuinely wide — but if the chart commands (currently ~10
  variants driving ~970 lines of `run.rs`) grow further, splitting charts into a sub-protocol +
  sub-handler would relieve both the enum and the `Worker` struct.

---

## Phase Summary

The crate-level architecture of this workspace is genuinely good, and the file-level architecture is
genuinely bad, and those two facts are almost independent. The four-way dependency rule
(core/chart-model → engine → app, GPU-free below the app, IronCalc-sealed inside the engine) is
correct, is enforced with a guard that actually works, and — I checked — has no leaks: no
`ironcalc_base` type is nameable from `freecell-app`, and the worker's single-writer ownership of the
`UserModel` is respected. The pure-logic extraction into `layout.rs`, `chart_layer.rs`, `registry.rs`,
`lifecycle.rs`, and core's reducers is the right instinct executed consistently, and it is why 42% of
this repo can be tests. Whoever drew the crate seams knew what they were doing.

But below the crate line there is no structure discipline at all. `chrome/view.rs` (8,249 production
lines, 71 fields, 580 methods, ~14 unrelated feature domains) and `grid/view.rs` (6,576 production
lines, 45 fields) are **55% of the app crate's production code in two files**, and `worker/run.rs`
is another 26% of the engine's. The `chrome/view.rs` growth curve — 5,618 to 16,099 lines,
monotonically, over 34 commits and sixteen days, with a single net-negative commit — shows this is
not a backlog item that happened to slip; it is the codebase's steady state, and it will not
self-correct. The crate boundaries are healthy precisely because they are enforced by the compiler
and a test; the module boundaries are unhealthy precisely because nothing enforces them.

On the two secondary questions: `freecell-core` is a purity boundary masquerading as a domain layer,
and has drifted into a junk drawer (~2,000 of its 5,678 production lines have one consumer, and
`recent.rs` does filesystem I/O in the crate whose whole charter is "pure"). `freecell-chart-model`
is premature fragmentation — as a *second* zero-dependency foundation crate that does not know
`freecell-core` exists, it has already produced a duplicated `Color`/`Rgb` primitive and, more
seriously, a second independent OOXML number-format implementation that can visibly disagree with the
cell formatter. Of the four duplication suspicions I was asked to verify, three (core/engine `cache`,
the two `palette`s, the two `authoring`s) are false positives and should not be pursued; these two
are the real ones.

Would this survive doubling in size? The crate graph would. The files would not — doubling means
`chrome/view.rs` at ~16,000 production lines and `grid/view.rs` at ~13,000, at which point the app
crate becomes effectively unownable by anyone who did not write it. The single highest-leverage
action available is mechanical and low-risk: convert `chrome/view.rs` into a `view/` directory of
per-domain modules (Rust's module-descendant privacy means the 71 private fields need no visibility
changes), do the same for `grid/view.rs` and the chart half of `worker/run.rs`, and then put a
production-line ceiling in CI so it stays done.
