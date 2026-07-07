---
status: draft
---

# Implementation Plan: IronCalc Upstreaming

Scope = Option 2. Fork work in `scosman/ironcalc` (`/workspace/ironcalc`); FreeCell work in this
repo on `claude/ironcalc-workarounds-oss-rlt0i1`. See `architecture.md` for per-step design.

## Phases

- [ ] **Phase 0 — Fork setup & baseline.** Add `upstream` remote (`ironcalc/IronCalc`); sync fork
  `main` to `upstream/main`; record base SHA; create `freecell-fixes` off `main`. Baseline green:
  `cargo test` + `make lint`.
- [ ] **Phase 1 — E2: num-fmt table (fork).** `fix/e2-numfmt` off `main`. Correct
  `base/src/number_format.rs` `DEFAULT_NUM_FMTS` (ids 5–8, 37–49) per architecture; formatter
  tests. Green. Merge → `freecell-fixes`. Push.
- [ ] **Phase 2 — E5: `<indexedColors>` override (fork).** `fix/e5-indexed` off `main`. Parse
  `<indexedColors>` in `xlsx/src/import/styles.rs`; thread to `get_color`; resolve when present.
  Crafted-styles tests + guards. Green. Merge → `freecell-fixes`. Push.
- [ ] **Phase 3 — FreeCell upgrade (the migration).** Add `[patch.crates-io]` → `freecell-fixes`.
  Delete `open_fixups.rs` + `open_repair.rs` (+ their `document.rs::open` call sites) and drop
  `roxmltree`/`zip`. Migrate the color-read path (`cache.rs`, `document.rs`) to resolve `Color` via
  `UserModel::resolve_color` (`Color::None` ⇒ no fill); fix any incidental `main` API drift the
  build surfaces. Update tests/fixtures reading `fill.fg_color`. Workspace `cargo test` + checks
  green.
- [ ] **Phase 4 — Validation (the redundancy proof).** Port `open_fixups`' theme + indexed goldens
  into an equivalence test (engine `resolve_color` == the RGBs the hack produced). Owner visual
  pass: open the mortgage (purple theme), Numbers (indexed palette + `xfId`-less), and a
  currency/accounting file (num-fmt) — confirm correct render + that each opens; open→save→reopen
  one affected file. This gate confirms pulling the hacks is correct.
- [ ] **Phase 5 — Sign-off gate → upstream PRs.** On owner approval: rebase `fix/*` on fresh
  `upstream/main`; open one PR per fix (E2, E5) against `ironcalc/IronCalc:main`, PR-first, minimal
  repro + tests in each body. Record in the status table.

## Follow-up (NOT this project)
- Move FreeCell from the git-`main` patch to a **released** IronCalc pin once the fixes ship →
  `projects/ironcalc-upgrade.md` (slimmed to that tail).

## Status table

| Item | Branch | Tests | `freecell-fixes` | FreeCell migrated | Upstream PR | State |
|------|--------|-------|------------------|-------------------|-------------|-------|
| E2 num-fmt | `fix/e2-numfmt` | — | — | — | — | not started |
| E5 indexed | `fix/e5-indexed` | — | — | — | — | not started |
| FreeCell upgrade | (this branch) | — | — | — | n/a | not started |
