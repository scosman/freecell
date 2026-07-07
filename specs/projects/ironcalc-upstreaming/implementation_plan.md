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
- [~] **Phase 3 — FreeCell upgrade (the migration). PARTIAL — blocked on engine-default drift.**
  Done: `[patch.crates-io]` → `freecell-fixes`; deleted `open_fixups.rs` + `open_repair.rs` (+ the
  `document.rs::open` call sites), dropped `roxmltree`, moved `zip` to dev-deps; migrated the
  colour-read path (`cache.rs` `resolve_rgb`/`render_style_from`/`border_spec_from`, `document.rs`
  `resolve_text_color` + a `workbook_theme()` accessor) to the new `Color` enum. **`freecell-engine`
  compiles clean against the fork; the `Color` migration is small (4 prod + 6 test sites).**
  **BLOCKER (see finding below):** building against upstream `main` is a full 0.7.1→`main` engine
  upgrade — `main` changed `new_empty`'s **default geometry** (row height, col width) and **default
  font** (12pt Inter vs 13pt Calibri). FreeCell's cache-agreement invariant is pinned to the 0.7.1
  values, so **21/91 engine tests fail — all geometry/default mismatches, zero colour-correctness
  failures.** Decision needed (decouple vs. push through the full upgrade).
- [ ] **Phase 4 — Validation (the redundancy proof).** Port `open_fixups`' theme + indexed goldens
  into an equivalence test (engine `resolve_color` == the RGBs the hack produced). Owner visual
  pass: open the mortgage (purple theme), Numbers (indexed palette + `xfId`-less), and a
  currency/accounting file (num-fmt) — confirm correct render + that each opens; open→save→reopen
  one affected file. This gate confirms pulling the hacks is correct.
- [ ] **Phase 5 — Sign-off gate → upstream PRs.** On owner approval: rebase `fix/*` on fresh
  `upstream/main`; open one PR per fix (E2, E5) against `ironcalc/IronCalc:main`, PR-first, minimal
  repro + tests in each body. Record in the status table.

## Phase 3 finding — "upgrade to main" is a full engine upgrade (2026-07-07)

Building FreeCell against the fork's `freecell-fixes` (= upstream `main` + E2/E5) surfaced that
`main` has drifted from the pinned `0.7.1` in ways **unrelated to the E1–E5 workarounds**:

- **Default geometry changed.** Fork `constants.rs`: `DEFAULT_ROW_HEIGHT = 25`, `DEFAULT_COLUMN_WIDTH
  = 90`; a `new_empty` sheet's rows report **21.43 px** via `get_row_height`. FreeCell hardcodes
  `DEFAULT_ROW_HEIGHT_PX = 24`, `DEFAULT_COL_WIDTH_PX = 100` (tuned to `0.7.1`).
- **Default font changed.** Fork `Font::default()` = **12 pt "Inter"**; FreeCell expects **13 pt
  "Calibri"**.
- **Consequence:** the resident-cache↔engine **agreement invariant** (FreeCell's core correctness
  contract) is pinned to the old defaults → **21/91 `freecell-engine` tests fail, all
  geometry/default mismatches; zero colour/number-format correctness failures.**

**Interpretation:** the workaround *removal itself is correct* — the colour/format hacks the engine
now subsumes (E1/E2/E5) and the `xfId` accept (E4) are gone cleanly, and nothing colour/format
regressed. The failures are the cost of moving from the `0.7.1` release to unreleased git-`main`,
which also carries a font/geometry refresh. The E1/E4/tint fixes are **entangled** with that larger
`main` evolution (the `Color`-enum refactor shipped *with* the theme fix), so there is no clean
"`0.7.1` + only our 5 fixes" base.

**Decision needed (recorded, pending owner):**
- **(A) Decouple (recommended):** ship E2+E5 upstream (done, PR-ready); make the FreeCell
  hack-removal its own **engine-upgrade project** against a *released* IronCalc that bundles all
  five fixes (`projects/ironcalc-upgrade.md`), where reconciling row/col/font defaults + updating the
  geometry tests and render baselines is in scope. Revert this branch's Phase-3 changes (restore the
  `0.7.1` pin + hacks) so FreeCell stays green. The Phase-3 migration is preserved (committed WIP) for
  that project to build on.
- **(B) Push through now:** reconcile all of `main`'s drift (bump the geometry/font defaults, fix the
  21 tests + render baselines, chase any further drift) and pin FreeCell to git-`main`. Larger than
  scoped, and a git-`main` pin is not shippable.

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
