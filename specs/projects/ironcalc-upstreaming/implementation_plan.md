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
- [ ] **Phase 6 — Adopt the fork as FreeCell's permanent engine + establish the ongoing loop.**
  Not a one-shot: this makes "FreeCell rides our fork; fix IronCalc, don't hack FreeCell" the
  standing way of working. See **§Operating model** below for the durable process. Concretely for
  this project: keep FreeCell's `[patch.crates-io]` → the fork's `freecell-fixes` as the **normal**
  dependency (not temporary); land the git-`main` geometry/font reconciliation (Phase-3 finding) so
  the workspace is fully green on the fork; and record the loop so future IronCalc issues follow it.

## Operating model — FreeCell rides our IronCalc fork (standing process)

**This is a permanent way of working, not a one-off.** FreeCell depends on **our fork**
(`scosman/ironcalc`), and when we hit an IronCalc bug or missing capability we **fix it in the
fork** rather than adding a workaround in FreeCell, then contribute that fix back **upstream**
(`ironcalc/IronCalc`) as a clean, single-purpose PR. Upstream wants the patches; we want the fix
in the engine, not compensation code in the app. Both goals are served by the same commit.

**Two repos, one container.** An agent works on both in parallel in the same environment:
FreeCell at `/home/user/freecell`, the fork cloned at `/workspace/ironcalc` (add it to a session
with `add_repo scosman/ironcalc`, then clone; `add_repo ironcalc/IronCalc` too when it's time to
open upstream PRs). FreeCell builds against the fork via `[patch.crates-io]` (git branch for a
committed/reproducible build; a `path = "/workspace/ironcalc/{xlsx,base}"` patch is equivalent for
fast in-container iteration).

**Branch strategy (fork `scosman/ironcalc`):**
- **`main`** — a clean mirror of upstream `ironcalc/IronCalc:main`. Never commit fixes here.
- **`fix/<slug>`** — one branch per fix, off `main`, with upstream-style tests. Each is a single
  logical change so it can be a clean standalone PR (e.g. `fix/e2-numfmt`, `fix/e5-indexed`).
- **`freecell-fixes`** — integration branch that merges every in-flight `fix/*`. **This is the
  branch FreeCell's `[patch.crates-io]` points at** — the sum of our not-yet-upstreamed fixes.

**The loop for every new IronCalc issue:**
1. Hit a bug/limitation while building FreeCell.
2. In the fork, branch `fix/<slug>` off `main`; reproduce + fix; add tests; pass the fork's own
   `cargo test` + `make lint` (fmt + strict clippy). Author as the owner
   (`Steve Cosman <848343+scosman@users.noreply.github.com>`), clean messages, **no internal
   session URLs** in commits bound for a public PR.
3. Merge `fix/<slug>` into `freecell-fixes`; FreeCell builds against it; verify in-app.
4. **On owner sign-off**, open a single-fix PR from `fix/<slug>` against `ironcalc/IronCalc:main`
   (PR-first; the description carries the minimal repro + the tests).
5. When it merges upstream, it returns via the next `main` sync — then drop the local `fix/<slug>`
   and its merge from `freecell-fixes`.

**Syncing the fork from upstream (do periodically, and before opening PRs):**
- `git fetch upstream && git checkout main && git merge --ff-only upstream/main && git push origin main`.
- Rebase each live `fix/*` and rebuild `freecell-fixes` on the new `main`, so PRs apply cleanly and
  FreeCell gets upstream's other improvements.
- Expect **incidental drift** on sync — upstream changes unrelated to our fixes (e.g. the 2026-07
  font/geometry refresh). Reconcile it on the FreeCell side as part of the sync; it's the normal
  cost of tracking an active engine, not a defect.

**Releases (optional optimisation):** when upstream cuts a release containing some of our merged
fixes, we can bump FreeCell's crates.io pin to it and shrink `freecell-fixes` (and the `[patch]`)
to only the fixes not yet released — less to carry, same behaviour. Riding `freecell-fixes`
directly is always valid; a released pin is just leaner when available.

## Phase 3 finding — the fork is ahead of the 0.7.1 release (2026-07-07)

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

**Decision (resolved 2026-07-07 — owner): push through; the fork is FreeCell's permanent engine.**
FreeCell rides `freecell-fixes` as its normal dependency (see §Operating model), so the git-`main`
drift is reconciled here, not decoupled. Remaining work under Phase 6:
- **Reconcile the geometry/font defaults** on the FreeCell side: update `DEFAULT_ROW_HEIGHT_PX` /
  `DEFAULT_COL_WIDTH_PX` (and any derived metrics) and the `default_font` expectation (12 pt Inter)
  to match the fork, so the resident-cache↔engine agreement holds again.
- **Fix the ~21 geometry/`default_font` tests** and **refresh the render baselines** the new metrics
  change.
- Then the FreeCell workspace is fully green on the fork.

The manual verification (`MANUAL_TEST.md`) already confirms the **fix-relevant** behaviour with the
hacks removed: **E1, E2, E4, E5 verified in-app by the owner (2026-07-07); E3 (date/time) pending.**
The 21 failures are geometry-only and don't touch fix correctness — they're the reconciliation task
above, not a blocker to the fixes.

## Optional optimisation (not required)
- When upstream releases a version containing our merged fixes, optionally bump FreeCell's
  crates.io pin to it and shrink the `[patch]` to only the not-yet-released fixes →
  `projects/ironcalc-upgrade.md`. Riding `freecell-fixes` directly stays valid indefinitely.

## Status table

| Item | Branch | Tests | `freecell-fixes` | Owner-verified in-app | Upstream PR | State |
|------|--------|-------|------------------|-----------------------|-------------|-------|
| E2 num-fmt | `fix/e2-numfmt` (`953af32`) | ✅ base 2107 + xlsx 213 green, fmt+clippy clean | ✅ merged | ✅ E2 (mortgage) | ⏳ awaiting sign-off | fix pushed; backup `patches/0001-e2-numfmt.patch` |
| E5 indexed | `fix/e5-indexed` (`1c2c477`) | ✅ 4 new + xlsx 213 green, fmt+clippy clean | ✅ merged (`48b0b23`) | ✅ E5 fills (numbers_table) + borders (FONTS.xlsx) | ⏳ awaiting sign-off | fix pushed; backup `patches/0002-e5-indexed.patch` |
| E1/E4/tint | (already on upstream `main`) | n/a | inherited | ✅ E1 (mortgage) + E4 (numbers_table opens) | n/a | consumed via the fork; no PR needed |
| FreeCell migration | (this branch, WIP `3fc7b1d`) | engine compiles; ~21 geometry tests pending reconcile | — | ✅ E1/E2/E4/E5; ⬜ E3 (date/time) | n/a | Color migration + hacks removed; **remaining: geometry/font reconcile (Phase 6)** |

> **Push access resolved (2026-07-07):** owner granted write to `scosman/ironcalc`; commits are
> authored `Steve Cosman <848343+scosman@users.noreply.github.com>` (noreply, to satisfy email
> privacy). `fix/e2-numfmt` + `freecell-fixes` pushed at `953af32`; `main` clean at `29daa42`.
