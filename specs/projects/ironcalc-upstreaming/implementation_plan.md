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
- [x] **Phase 3 — FreeCell upgrade (the migration). DONE.**
  Done: `[patch.crates-io]` → `freecell-fixes`; deleted `open_fixups.rs` + `open_repair.rs` (+ the
  `document.rs::open` call sites), dropped `roxmltree`, moved `zip` to dev-deps; migrated the
  colour-read path (`cache.rs` `resolve_rgb`/`render_style_from`/`border_spec_from`, `document.rs`
  `resolve_text_color` + a `workbook_theme()` accessor) to the new `Color` enum. The `Color`
  migration is small (4 prod + 6 test sites). **Geometry/font drift reconciled** (see Phase-3
  finding + Phase 6): recalibrated the two unit-conversion reference constants
  (`IRONCALC_DEFAULT_ROW_HEIGHT_PX` 28→25, `IRONCALC_DEFAULT_COL_WIDTH_PX` 125→90) to the fork's
  actual defaults and updated the `default_font` expectation (12pt Inter). **All 91 `freecell-engine`
  lib tests + every integration suite green; fmt + strict clippy clean.**
- [ ] **Phase 4 — Validation (the redundancy proof).** Port `open_fixups`' theme + indexed goldens
  into an equivalence test (engine `resolve_color` == the RGBs the hack produced). Owner visual
  pass: open the mortgage (purple theme), Numbers (indexed palette + `xfId`-less), and a
  currency/accounting file (num-fmt) — confirm correct render + that each opens; open→save→reopen
  one affected file. This gate confirms pulling the hacks is correct.
- [ ] **Phase 5 — Sign-off gate → upstream PRs.** On owner approval: rebase `fix/*` on fresh
  `upstream/main`; open one PR per fix (E2, E5) against `ironcalc/IronCalc:main`, PR-first, minimal
  repro + tests in each body. Record in the status table.
- [x] **Phase 6 — Adopt the fork as FreeCell's permanent engine + establish the ongoing loop.**
  Not a one-shot: this makes "FreeCell rides our fork; fix IronCalc, don't hack FreeCell" the
  standing way of working. See **§Operating model** below for the durable process. Concretely for
  this project: FreeCell's `[patch.crates-io]` → the fork's `freecell-fixes` is now the **normal**
  dependency (not temporary); the git-`main` geometry/font reconciliation landed (constant
  recalibration + `default_font` test); the workspace is fully green on the fork; and the loop is
  recorded in **§Operating model** + `CLAUDE.md` for future IronCalc issues. **Render baselines do
  NOT move** (verified by code analysis, not just left unrun): every `render-tests` scene spawns a
  `NewWorkbook` and injects custom col/row geometry **directly as device px** into the cache
  (`cache.set_col_width`), bypassing the `col_px`/`row_px` conversion the constants feed; default
  cells render at the fixed `CELL_FONT_PX = 13.0` app constant (independent of the engine's default
  font, so the 13→12 change is inert); and the only explicit case font size is 24 pt (≠ 12/13, so
  its default-vs-explicit quantization is unchanged). So no baseline regeneration is required.

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

**Agent operating notes (autonomous runs) — learned `gaps_closing_7_15`, 2026-07-16:**
- **Provision repos UPFRONT.** `add_repo scosman/ironcalc` needs an **interactive permission
  approval**. If a long autonomous run needs the fork, call `add_repo` **while the user is still
  present** to approve it — calling it mid-run after they've left **fails** with
  `AbortError: Tool permission stream closed` (the approval channel is gone). Add every repo you
  might touch at the start.
- **Proxy fallback (no `add_repo` needed).** The container's agent git-proxy already routes
  `scosman/ironcalc`, so you can **clone/branch/push the fork through the proxy URL** even when
  `add_repo` is unavailable: `git clone http://local_proxy@127.0.0.1:<port>/git/scosman/ironcalc`
  (get `<port>` from FreeCell's own `git remote -v` — same `local_proxy@` credential as the
  FreeCell origin). This reaches **`scosman/ironcalc` only**. (The `[patch]` in `app/Cargo.toml`
  pins the fork by a **direct** `github.com/scosman/ironcalc` URL fetched via the outbound HTTPS
  proxy; after pushing `freecell-fixes` through the git-proxy URL, `cargo update -p ironcalc_base
  -p ironcalc` moves the lock to the new rev.)
- **Pushes go to `scosman` only.** The agent **cannot open upstream `ironcalc/IronCalc` PRs**
  (not in session repo scope, no push creds there). Instead, when a `fix/<slug>` is ready, the
  agent **prepares** the PR for the owner to open in one click: a **compare link**
  `https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/<slug>` plus a
  suggested **title** and **description** (minimal repro + the tests). This is exactly step 4's
  "PR-first on owner sign-off" — the agent preps, the owner clicks.
- **Check before you branch.** Before creating a `fix/*`, confirm the capability isn't **already
  in** `freecell-fixes` (upstream may have landed it): `git merge-base --is-ancestor <upstream-sha>
  origin/freecell-fixes` + a `git grep` for the API at the pinned rev. In `gaps_closing_7_15` the
  planned row/column-hidden fixes were **already present** (upstream `a520f48f`), so no branches
  were needed — a stale GAPS.md audit note had claimed otherwise.

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
drift is reconciled here, not decoupled.

**Resolved (2026-07-08).** The reconciliation was a small, self-contained recalibration, not a
metrics overhaul. FreeCell keeps its **own** render defaults (`DEFAULT_ROW_HEIGHT_PX = 24`,
`DEFAULT_COL_WIDTH_PX = 100`, `CELL_FONT_PX = 13`) — those are FreeCell's, not IronCalc's to dictate
(owner: "FreeCell owns the defaults… their values are just values, not the 'right value'"). What had
to track the engine is the **unit-conversion reference** — the IronCalc default the px conversion
maps *onto* FreeCell's default, and the sentinel that marks a non-custom track. So the fix was:
- `IRONCALC_DEFAULT_ROW_HEIGHT_PX` 28 → **25**, `IRONCALC_DEFAULT_COL_WIDTH_PX` 125 → **90** (the
  fork's real defaults, probe-verified), with a comment that they must track the pinned engine.
- `default_font` test expectation → **12 pt Inter** (the value only feeds the cache's "is this the
  default?" detection; default cells still render bundled Inter at `CELL_FONT_PX`).
- `unit_conversion_goldens` re-expressed via the constants so it stays correct on future drift.
All 91 lib tests + integration suites pass; fmt + strict clippy clean. **Render baselines don't move**
(see Phase 6). Inter stays FreeCell's default font (`GRID_FONT_FAMILY` untouched). Two follow-on
ideas — persisting FreeCell's defaults into saved files for cross-app fidelity, and render-time
fallback for unavailable explicit fonts — are tracked in `GAPS.md`, deliberately **out of scope** here.

Verification with the hacks removed is **complete**: **E1, E2, E4, E5 confirmed in-app by the owner
(2026-07-07); E3 covered by the `dates_fixture` integration test** (built-in date/time ids 14–22
render as dates, not serials — `tests/fixtures/dates.xlsx`). All five fixes confirmed. The former 21
geometry failures were the reconciliation task above, now **resolved (2026-07-08)** — the workspace
is green on the fork.

## Not an exit — the fork is permanent

Riding `freecell-fixes` is the **standing operating position**, not a waiting room. We keep the
fork, keep upstreaming fixes as clean single-fix PRs, and keep re-syncing from upstream `main`.
Moving to a released crates.io pin is an *optional* simplification if the day ever comes when
every fix we carry is released — never a goal that shapes the work
(`projects/ironcalc-upgrade.md`).

## Status table

**Rebuilt 2026-07-28 from the fork's history** (v05-cleanup-1 / unit A4). The previous version of
this table listed two fixes as "awaiting sign-off" and was three months stale; the review that
prompted the rebuild went further and claimed *nothing* had been upstreamed, which is **wrong** —
six of the eleven are merged.

Method, so it can be re-run: enumerate the first-parent merges on `freecell-fixes`
(`git log --first-parent`), take each merge's second parent as the fix branch, and classify with
`git cherry upstream/main <branch-head> <merge-base>` — patch-id equivalence, which finds a fix
upstream even when it landed with a rewritten SHA or a reworded subject. Subject matching alone
is not reliable and disagreed with patch-id on two rows.

Verified against fork `freecell-fixes` @ `cee2859d` (the SHA `app/Cargo.toml` pins) and upstream
`ironcalc/IronCalc` `main` @ `2e2465c`.

| Fix | Branch / head | Upstream status | Notes |
|---|---|---|---|
| Built-in `numFmtId` table (E2) | `fix/e2-numfmt` (`953af32`) | ✅ **merged** — upstream `5b98252` | Absorbed by a fork rebase, so it no longer appears as a fork-only merge |
| Negative `indexedColors` guard (E5) | `fix/e5-indexed` (`5df8c277`) | ✅ **merged** — upstream `e481dc6` | |
| `UserModel::set_worksheet_index` | `fix/sheet-reorder` (`21cde336`) | ✅ **merged** — upstream `2f53937` | ⚠️ **upstream then RENAMED it to `move_sheet` (`7ca43c7`)** — see the drift note below |
| ECMA-376 `true/false` for `xsd:boolean` | `fix/xlsx-bool-import` (`2cd099e9`) | ✅ **merged** — upstream `13fb8f4b` | |
| TRIM collapses internal runs | `fix/trim-internal-runs` (`6c894ba2`) | ✅ **merged** — upstream `2e2465c` | |
| ADDRESS `!` prefix for empty sheet | `fix/address-empty-sheet` (`09259476`) | ✅ **merged** — upstream `2b8672a` | |
| `UserModel::set_user_inputs` (batched single-undo) | `fix/batch-set-inputs` (`a51cf46c`) | **fork-only** | Load-bearing: `document.rs` Replace-All rides it |
| Merged cells (core/model + bindings + xlsx) | merged-cells (`a9fc9fa0`, 5 commits) | **fork-only** | A whole feature, not a fix — adds `base/src/merge_cells.rs`, absent upstream |
| DOLLAR: no parens when the value rounds to zero | `fix/dollar-negative-zero` (`aa36a177`) | **fork-only** | ⚠️ **reverted on the fork's own branch tip** (`8a79a7f`), while the pinned SHA still carries it — see below |
| XMATCH array-constant `lookup_array` | `fix/xmatch-array-constant` (`f9d1f9ce`) | **fork-only** | |
| Frozen pane tracked on insert/delete row/col | `fix/structural-edits-adjust-frozen-pane` (`507fe6c7`) | **fork-only** | The `freecell-fixes` tip commit |

E1 / E4 / the tint fix are not rows here: they were already on upstream `main` and are inherited,
never carried by us.

### Two live discrepancies this rebuild surfaced

1. **`set_worksheet_index` → `move_sheet`.** Upstream merged our API and then renamed it.
   `freecell-engine/src/document.rs:1514` still calls `set_worksheet_index`, so the **next re-sync
   of the fork onto upstream `main` will break FreeCell's build at that call site**. It is a
   one-line rename, but it must be expected rather than discovered.
2. **The fork's branch tip has moved past the pin, and reverts a fix we depend on.**
   `freecell-fixes` is at `ecbf6226`, two commits past the pinned `cee2859d`, and those commits
   revert `fix/dollar-negative-zero` — which `freecell-engine/src/document.rs:2186` asserts
   (`=DOLLAR(-0.001,2)` → `"$0.00"`). Since v05-cleanup-1/A1 the manifest pins a SHA, so nothing
   moves under us; but the next deliberate bump has to reconcile it — either FreeCell drops that
   assertion or the revert is reverted on the fork.

**Also stale:** the fork's `main` mirror is at `cedba4e`, **99 commits behind** upstream `main`. A
re-sync is overdue and is the normal maintenance this project's operating model calls for.

> **Push access resolved (2026-07-07):** owner granted write to `scosman/ironcalc`; commits are
> authored `Steve Cosman <848343+scosman@users.noreply.github.com>` (noreply, to satisfy email
> privacy).
