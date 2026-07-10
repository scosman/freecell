---
status: draft
---

# Implementation Plan: Excel Table-Style Import + Resolution

Ordered by dependency: fork parsing → resolver (custom dxfs) → built-in/theme-derived →
FreeCell consumption → render validation. Fork work in `scosman/ironcalc` (`/workspace/
ironcalc`) per `specs/projects/ironcalc-upstreaming/` §Operating model (branch-per-fix,
`freecell-fixes` integration, upstream PR on sign-off). See `architecture.md` for per-step
design; `functional_spec.md` for behaviors + the Open Questions the owner must settle before
Phase 2 locks (built-in v1/v2, direct-vs-table precedence, stripes-in-v1).

## Phases

- [ ] **Phase 0 — Decisions.** Owner settles the functional-spec Open Questions:
  built-in styles v1 or v2 (recommended: v2); direct-vs-table precedence option a/b/c
  (recommended: b); stripes in v1 (recommended: yes). These set Phase 2/3 scope.

- [ ] **Phase 1 — Fork: `tableStyleInfo` parse fixes** (`fix/table-style-info-parse`).
  Fix the `<tableStyleInfo>` wrong-tag bug (name + stripe flags), the `dataDxfId`←
  `headerRowDxfId` copy-paste (table + column), verify the `showRowStripes` semantics. Unit
  tests (synthetic XML). Fork `cargo test` + `make lint` clean; merge → `freecell-fixes`. A
  clean standalone upstream PR candidate.

- [ ] **Phase 2 — Fork: `<tableStyles>` parse + custom-dxf resolver** (`fix/table-style-
  resolve`). Add the `TableStyles`/`TableStyle`/`TableStyleElement`/`TableStyleType` model +
  `load_table_styles`; add `Model::apply_table_styles` (region membership, precedence,
  `dxf_for_region` named-style + per-table override, `Dxf::apply_to` reuse, `direct_wins`
  reconcile per Phase-0 choice); wire into `get_style_for_cell` with the no-table fast path.
  Unit tests (membership truth table, precedence, override-beats-named, no-op, out-of-range).
  Fork tests + lint clean; merge → `freecell-fixes`.

- [ ] **Phase 3 — Fork: built-in/theme-derived styles** *(only if Phase-0 puts it in v1;
  else defer as `fix/table-style-builtin`, track in GAPS.md).* Encode the built-in catalog +
  theme derivation into synthetic per-region dxfs plugged into the same resolver seam. Its
  own tests; merge → `freecell-fixes`.

- [ ] **Phase 4 — FreeCell consumption.** Point `[patch.crates-io]` at the updated
  `freecell-fixes` (or local path for iteration). Feed the cache-build path the table-aware
  resolved style for in-table cells + enumerate table rectangles (empty-boxed-cell fix);
  keep the resident-cache agreement contract green. Un-ignore the three fixture tests
  (`personal_monthly_budget_fixture.rs`) → green; existing guards stay green. Add the
  empty-boxed-cell cache test. Workspace `cargo test` + fmt + strict clippy clean. Iterate
  render with the **subset** filter only (`render_tests.sh test cell_|border_|fill_`).

- [ ] **Phase 5 — Render validation (dedicated late phase, once).** Add the table-style
  render case backed by `personal_monthly_budget.xlsx` (harness-loads-xlsx vs synthetic —
  decide per `architecture.md §10`). Run the **full** pixel suite under a ~10-min watchdog;
  regenerate + **eyeball** baselines (intentional rendering change), confirm no unrelated
  baseline moved, commit refreshed baselines. **Dispatch the CI `render` gate** on the
  branch, poll to green.

- [ ] **Phase 6 — Owner validation + upstream (sign-off gate).** Owner eyeballs the budget
  file rendering in-app. On sign-off, open the upstream PRs (one per `fix/*` branch) against
  `ironcalc/IronCalc:main`, PR-first with minimal repro + tests, per the operating model.
  Update GAPS.md (mark resolved / adjust residuals; note built-in styles if still deferred).
