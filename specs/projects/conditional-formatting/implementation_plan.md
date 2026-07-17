---
status: complete
---

# Implementation Plan: Conditional Formatting

CR-sized, risk-ordered phases: prove the **engine seam + value-dependent rendering** first (the real
technical risk), then the reusable container + sidebar shell, then list management, then the
authoring forms, then persistence, then the late render/perf validation. Each phase is one coherent
commit. Refs: `functional_spec.md`, `ui_design.md`, `architecture.md`,
`components/engine_cf.md`, `components/cf_sidebar.md`.

Scope reminder (confirmed 2026-07-17): **highlight rules + color scales** with **full rule
management** are the first pass. **Data bars, icon sets, ratings** are planned here as later phases
(P11–P13) but are **not built in the first pass** — the first pass ends after P10 and is presented
for review. Every skipped option is logged in `GAPS.md` (done in P9).

## Core first pass

- [x] **P1 — Engine-free CF types + wrapper + conversions (headless).** `freecell-core::cond_fmt`
  types; `freecell-engine` `cond_fmt_convert` (spec↔input, format↔dxf, rule→view) +
  `WorkbookDocument` methods (add/update/delete/raise/lower/list/`has_cond_fmt`/
  `extended_render_style`). No protocol/UI. *Exit:* `engine_cf.md §7` engine tests green
  (add→list, update-merge, delete, reorder, extended-style reflects a rule + **value change flips
  it**, conversions incl. deferred→Badge). Crate-scoped: `-p freecell-core -p freecell-engine`.

- [x] **P2 — Worker protocol + published rule list.** `Command` CF variants + `WorkerEvent::
  CondFmtUpdated`; `apply_one` dispatch (+ `Err` surfacing); `Shared.cond_fmt` map +
  `DocumentClient::cond_fmt_rules`; refresh + emit on mutation/undo/redo/open. *Exit:* worker-seam
  tests — `AddCondFmt`→`cond_fmt_rules` reflects it + `CondFmtUpdated`/`StyleCacheUpdated`;
  update/delete/reorder; undo/redo; bad-range `Err` surfaces.

- [x] **P3 — Value-dependent render cache.** Thread `has_cond_fmt` gate into
  `build_sheet_cache`/`refresh_cell` (extended read for CF sheets); value-publish → style-cache
  rebuild for CF sheets in the worker publish path. *Exit:* cache tests — a `> 100` rule paints a
  fill in the render cache; editing a source value flips a Top-N/threshold cell **with no CF
  command**; color scale interpolates; non-CF sheets unchanged (fast path). Grid paint untouched.

- [x] **P4 — Reusable sidebar container + action-bar button + empty sidebar.** Extract
  `chrome/sidebar.rs::docked_sidebar` + shared `section` helpers; refactor `render_chart_panel` onto
  it (no visual change); add the lucide **`split`** button, `cond_fmt` state, toggle/open/close,
  selection-change exemption, chart↔CF mutual exclusion; render a minimal **List-mode shell** (intro
  + Add-rule button, no rows yet). *Exit:* view tests — button toggles sidebar; opening closes the
  chart panel; selection change doesn't close it; chart-panel tests still pass. Smoke launch.

- [x] **P5 — Rules list (List mode).** Build rows from `client.cond_fmt_rules`; render rows
  (preview swatch / summary / range / reorder / edit / delete); wire delete + raise/lower; refresh
  on `CondFmtUpdated` + sheet switch; deferred-family rows non-editable but deletable. *Exit:* view
  tests — list renders rows; delete + reorder send commands; sheet switch refreshes; Badge row edit
  disabled.

- [ ] **P6 — Rule editor: highlight rules + format editor.** Editor mode; rule-type dropdown;
  per-type operands (Cell value, Text, Dates, Top/Bottom, Above/Below, Duplicate/Unique,
  Blanks/Errors, Formula); the **format editor** (fill/text color + bold/italic + presets +
  preview); validation; Save → `AddCondFmt`/`UpdateCondFmt`; edit seeds from `spec`. *Exit:* view
  tests — add a Cell-value rule; edit it (seeded + `UpdateCondFmt`); validation blocks bad input;
  engine `Err` keeps editor open.

- [ ] **P7 — Color-scale editor.** 2/3-color editor (stop kind/value/color + presets + gradient
  preview); add/edit `ColorScale` rules. *Exit:* view tests — add a 3-color scale (spec has 3
  stops); edit stops; the list shows a gradient preview.

- [ ] **P8 — Persistence + deferred-rule handling + round-trip.** Confirm CF saves/loads via the
  engine writer (add a `WorkbookDocument` round-trip test: author highlight + color-scale → save →
  reopen → rules + effective styles survive); ensure loaded deferred-family rules appear in the list
  (Badge, delete-only) and don't break the cache. *Exit:* round-trip test green; a loaded data-bar
  rule lists as a Badge and its cell isn't corrupted.

- [ ] **P9 — GAPS + docs.** Append a **"Conditional Formatting — deferred behaviors"** section to
  `GAPS.md` (data bars, icon sets, ratings; TimePeriod date-range variant; color-scale formula
  thresholds; Dxf attrs: underline/strike/border/num-fmt/alignment; per-cell-vs-range perf
  follow-up) + tag them in the release-target tier table. No code. *Exit:* GAPS updated; cross-links
  to this project.

- [ ] **P10 — Render validation + perf (late phase, per CLAUDE.md).** Add render-test baselines for
  a CF **highlight** scene + a **color-scale** scene over the real GridView; iterate with the
  `cond_`/`cf_` subset; run the **full** suite once (watchdog); eyeball + commit baselines; dispatch
  the CI `render` gate and confirm green. Benchmark edit→repaint + scroll on a CF-heavy sheet vs a
  no-CF baseline (foreground, force+assert, p50/p99). Final `cargo fmt --all --check` + a
  workspace build. *Exit:* full render suite + CI `render` green; perf within range; **first pass
  complete → present for review.**

## Deferred families — later phases (planned, not built in the first pass)

> Built only after the first-pass review. Each adds a new in-cell **grid draw primitive** (in-scope
> for the pixel suite) + its authoring UI, reusing the P1–P10 seams (engine already returns the
> decoration in `ExtendedStyle`).

- [ ] **P11 — Data bars.** `RenderStyle`/cache decoration field for `ExtendedStyle.data_bar`; grid
  in-cell bar primitive (positive/negative color, gradient, axis position, show-value); data-bar
  editor. Render baselines.
- [ ] **P12 — Icon sets.** `ExtendedStyle.icon` decoration + in-cell icon glyph primitive (the
  engine's `Icon` enum → lucide glyphs); icon-set editor (presets + per-threshold). Render baselines.
- [ ] **P13 — Ratings.** `ExtendedStyle.rating` decoration + repeated-glyph primitive; rating
  editor. Render baselines. Final cross-family render + round-trip sweep.

### Why this order
The engine seam + value-dependent cache (P1–P3) is the only real technical risk and touches every
later phase, so it is proven headless first. The reusable container (P4) unblocks both panels and is
a behavior-preserving refactor. List (P5) before the editor (P6–P7) so management works against
real rules early. Persistence (P8) and GAPS (P9) close the first pass; render/perf validation (P10)
is a dedicated late phase per CLAUDE.md (grid pixels change). Data bars / icons / ratings (P11–P13)
are deferred because each needs a new draw primitive and the requester wants to review the
highlight+color-scale first pass before we invest in them.
