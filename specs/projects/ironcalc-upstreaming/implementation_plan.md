---
status: draft
---

# Implementation Plan: IronCalc Upstreaming

Scope = Option 1. Work happens in the fork `scosman/ironcalc` (`/workspace/ironcalc`); this repo
holds only specs. See `architecture.md` for the per-fix design.

## Phases

- [ ] **Phase 0 — Fork setup & baseline.** Add `upstream` remote (`ironcalc/IronCalc`); sync fork
  `main` to `upstream/main`; create `freecell-fixes` off `main`. Confirm the workspace is green at
  baseline: `cargo test` + `make lint`. Record the upstream base SHA.
- [ ] **Phase 1 — E2: num-fmt table.** Branch `fix/e2-numfmt` off `main`. Correct
  `base/src/number_format.rs` `DEFAULT_NUM_FMTS` for the locale-independent block (ids 5–8, 37–49)
  per the architecture table; add formatter tests (each corrected id → ECMA code + a value no
  longer `#VALUE!`). `cargo test` + `make lint` green. Merge into `freecell-fixes`. Push.
- [ ] **Phase 2 — E5: `<indexedColors>` override.** Branch `fix/e5-indexed` off `main`. Parse
  `<colors><indexedColors>` in `xlsx/src/import/styles.rs`; thread the override to `get_color`
  (via a `ColorContext`/added param) and resolve `indexed=` against it when present; unchanged
  otherwise. Port the crafted-styles tests + guards. `cargo test` + `make lint` green. Merge into
  `freecell-fixes`. Push.
- [ ] **Phase 3 — Owner sign-off gate → upstream PRs.** Present both fixes (diff + tests) for the
  human validation/sign-off. **Only on approval:** rebase branches on a fresh `upstream/main`;
  open one PR per fix against `ironcalc/IronCalc:main` (PR-first, minimal repro in each body).
  Record fix ↔ branch ↔ PR in the status table below.

## Deferred (NOT this project)

- **FreeCell upgrade + `Color` migration + hack deletion** → `projects/ironcalc-upgrade.md`
  (Future), gated on an IronCalc release carrying all five fixes.

## Status table

| Fix | Fork branch | Tests | `freecell-fixes` | Upstream PR | State |
|-----|-------------|-------|------------------|-------------|-------|
| E2 num-fmt | `fix/e2-numfmt` | — | — | — | not started |
| E5 indexed | `fix/e5-indexed` | — | — | — | not started |
