---
status: draft
---

# Implementation Plan: IronCalc Upstreaming

Scope = Option 2. Fork work in `scosman/ironcalc` (`/workspace/ironcalc`); FreeCell work in this
repo on `claude/ironcalc-workarounds-oss-rlt0i1`. See `architecture.md` for per-step design.

## Phases

- [x] **Phase 0 — Fork setup & baseline.** Recorded base SHA `29daa42`; created `freecell-fixes`
  off `main`; `ironcalc_base` baseline green. *(Deferred: adding the `upstream` remote + syncing
  `main` to `upstream/main` — `ironcalc/IronCalc` isn't in this session's scope; done at Phase 5
  pre-PR when upstream is added. Fork `main` is already a clean upstream mirror, authored by the
  IronCalc maintainer.)*
- [x] **Phase 1 — E2: num-fmt table (fork).** `fix/e2-numfmt` (`953af32`). **Discovery:** the
  table was structurally misaligned (index ≠ id from id ~18), so the fix is a full ECMA-376
  realignment, not a few-cell edit. `base` 2107 + `xlsx` 213 green, fmt + strict clippy clean.
  Merged → `freecell-fixes`. Pushed. (id 47 `mmss.0` = separate formatter gap, documented.)
- [x] **Phase 2 — E5: `<indexedColors>` override (fork).** `fix/e5-indexed` (`1c2c477`). Parse
  `<indexedColors>` in `styles.rs`, thread through the styles-path colour resolution via
  `get_color_indexed` (fills/fonts/borders/dxfs); tab/CF colours keep the default resolver
  (documented follow-up). 4 tests (end-to-end load_styles ±override + guards), fmt + clippy clean.
  Merged → `freecell-fixes` (`48b0b23`, both fixes; combined suite green). Pushed.
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
| E2 num-fmt | `fix/e2-numfmt` (`953af32`) | ✅ base 2107 + xlsx 213 green, fmt+clippy clean | ✅ merged | — | ⏳ awaiting sign-off | **fix complete + pushed to fork**; patch backup `patches/0001-e2-numfmt.patch` |
| E5 indexed | `fix/e5-indexed` (`1c2c477`) | ✅ 4 new + xlsx 213 green, fmt+clippy clean | ✅ merged (`48b0b23`) | — | ⏳ awaiting sign-off | **fix complete + pushed to fork**; patch backup `patches/0002-e5-indexed.patch` |
| FreeCell upgrade | (this branch) | — | — | — | n/a | not started (Phase 3 — awaiting go-ahead) |

> **Push access resolved (2026-07-07):** owner granted write to `scosman/ironcalc`; commits are
> authored `Steve Cosman <848343+scosman@users.noreply.github.com>` (noreply, to satisfy email
> privacy). `fix/e2-numfmt` + `freecell-fixes` pushed at `953af32`; `main` clean at `29daa42`.
