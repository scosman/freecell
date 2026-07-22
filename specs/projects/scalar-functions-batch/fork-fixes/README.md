# Fork fixes for the Scalar Functions Batch — upstreaming tracker

This batch is implemented **entirely in our IronCalc fork** (`scosman/ironcalc`) per the standing
"fix the fork, don't hack FreeCell" doctrine (`CLAUDE.md`;
`specs/projects/ironcalc-upstreaming/implementation_plan.md` §Operating model). It adds 11 scalar
functions + fixes TRIM. **One fix = one `fix/<slug>` branch off `main` = one clean upstream PR**,
each integrated onto **`freecell-fixes`** (the branch FreeCell's `[patch.crates-io]` pins,
`app/Cargo.toml` L110-112). FreeCell picks the functions up with **no FreeCell-side code** beyond a
pin bump.

The agent **cannot open upstream `ironcalc/IronCalc` PRs**. For each ready branch it **prepares** the
PR (compare link + title + body below); the **owner opens** them (human-in-loop). This file is the
durable tracker — like `conditional-formatting/fork-fixes/`, if fork **push** is blocked (403 on
`scosman/ironcalc`, as the CF fix hit) each branch's commit is preserved here as a
`NNNN-<slug>.patch` and the owner applies + pushes it.

11 functions + TRIM collapse into **10 branches → 10 PRs** (two justified pairings — see
`../architecture.md` §4).

## Status table (all NOT STARTED — planning artifact)

| # | Branch | Function(s) | Impl fn (module, best-inferred) | Tests | `freecell-fixes` | Upstream PR | State |
|---|--------|-------------|----------------------------------|-------|------------------|-------------|-------|
| 1 | `fix/sumproduct` | SUMPRODUCT | `fn_sumproduct` (math) | — | ⬜ | ⬜ prep | not started |
| 2 | `fix/trim-internal-runs` | TRIM (fix) | `fn_trim` body (`base/src/functions/text/common.rs`) | `cargo test -p ironcalc_base` green + `make lint` | ✅ (merge `0a36e79e`) | ✅ prep (below) | **landed + pushed** — branch `6c894ba2` |
| 3 | `fix/proper` | PROPER | `fn_proper` (text) | — | ⬜ | ⬜ prep | not started |
| 4 | `fix/replace` | REPLACE | `fn_replace` (text) | — | ⬜ | ⬜ prep | not started |
| 5 | `fix/char-code` | CHAR + CODE | `fn_char`, `fn_code` (text) | — | ⬜ | ⬜ prep | not started — **paired** |
| 6 | `fix/clean` | CLEAN | `fn_clean` (text) | — | ⬜ | ⬜ prep | not started |
| 7 | `fix/dollar` | DOLLAR | `fn_dollar` (text/fin) | — | ⬜ | ⬜ prep | not started |
| 8 | `fix/percentile-quartile-inc` | PERCENTILE.INC + PERCENTILE + QUARTILE.INC + QUARTILE | `fn_percentile_inc`, `fn_quartile_inc` (stat) | — | ⬜ | ⬜ prep | not started — **paired** |
| 9 | `fix/address` | ADDRESS | `fn_address` (lookup) | — | ⬜ | ⬜ prep | not started — full R1C1 |
| 10 | `fix/xmatch` | XMATCH | `fn_xmatch` (lookup) | — | ⬜ | ⬜ prep | not started — all modes |

Fill in the branch commit SHA, the test result (`cargo test -p ironcalc_base` + `make lint`), the
`freecell-fixes` merge SHA, and the upstream-PR state as each branch lands. Mirror the
`ironcalc-upstreaming` status-table conventions.

## Pre-branch existence check (do this first, every branch)

Before creating any `fix/*`, confirm the capability isn't **already present** (a stale gap note, or
an upstream landing — hide/unhide was already upstream in `gaps_closing_7_15`):

- `git grep -i "<name>"` at the pinned rev / on `main`, and check the `Function` enum for the name.
- `git merge-base --is-ancestor <upstream-sha> origin/freecell-fixes` when a specific commit is suspected.
- **CHAR/CODE, PERCENTILE/QUARTILE, and TRIM especially** may already exist (common functions; TRIM
  already exists — this is a body fix). If a name already computes correctly → **skip / record "already
  present"**. If it exists but is wrong (CHAR raw-Unicode 128–255; a non-inclusive legacy PERCENTILE) →
  the branch is a **correctness fix** to the existing impl.

## Upstream PR prep (agent preps; owner opens)

For each branch, once green on `freecell-fixes`:

- **Compare link:** `https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/<slug>`
- **Title:** single-feature, imperative (see per-branch below).
- **Body:** what/why (one paragraph) + the Excel contract (from `../functional_spec.md` §3) + a minimal
  repro (a formula → expected value) + "tests included: `<module>`". Self-contained; compiles against
  upstream `main`.

Per-branch titles (bodies drafted at land-time from the matching functional-spec §3 subsection):

| Branch | Suggested PR title |
|--------|--------------------|
| `fix/sumproduct` | Add SUMPRODUCT |
| `fix/trim-internal-runs` | Fix TRIM to collapse internal runs of spaces (Excel-compatible) |
| `fix/proper` | Add PROPER |
| `fix/replace` | Add REPLACE |
| `fix/char-code` | Add CHAR and CODE (Windows-1252 for 128–255, inverse pair) |
| `fix/clean` | Add CLEAN |
| `fix/dollar` | Add DOLLAR |
| `fix/percentile-quartile-inc` | Add PERCENTILE.INC / QUARTILE.INC (+ legacy PERCENTILE / QUARTILE) |
| `fix/address` | Add ADDRESS (A1 + full R1C1) |
| `fix/xmatch` | Add XMATCH (all match/search modes, incl. binary + wildcard) |

## Upstream PR prep — completed branches (ready for owner to open)

### `fix/trim-internal-runs` — Fix TRIM to collapse internal runs of spaces (Excel-compatible)

- **Compare link:** https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/trim-internal-runs
- **Branch commit:** `6c894ba2` (off `main`) · **`freecell-fixes` merge:** `0a36e79e` · both pushed to `scosman/ironcalc`.
- **Title:** `Fix TRIM to collapse internal runs of spaces (Excel-compatible)`
- **Body:**

  > TRIM previously returned `s.trim().to_owned()`, which only stripped the ends and — via Rust's
  > `str::trim` — removed *all* Unicode whitespace (tab, NBSP, …), violating Excel's contract. Excel's
  > TRIM operates on the **ASCII space `0x20` only**: it trims leading/trailing spaces **and** collapses
  > each internal run of two or more spaces to a single space, while leaving tab (`0x09`), non-breaking
  > space (`0xA0`), and every other whitespace character untouched. The fix replaces the body with a
  > `0x20`-only `split(' ')/filter(non-empty)/join(" ")` normalization in
  > `base/src/functions/text/common.rs`, which collapses internal runs and preserves the `0x20`-only scope.
  >
  > Minimal repro: `TRIM("a    b")` → `"a b"` (previously `"a    b"`). The `0x20`-only scope is proven by
  > `TRIM("a"&CHAR(9)&CHAR(9)&"b")` keeping its tabs and `TRIM(CHAR(160)&"x"&CHAR(160))` keeping its NBSPs.
  >
  > tests included: `base/src/test/text_functions/mod.rs`

## Owner action

1. If fork push is blocked, apply each preserved `NNNN-<slug>.patch` onto `scosman/ironcalc`
   `freecell-fixes` and push (`git am < NNNN-<slug>.patch`), then re-pin FreeCell
   (`cd app && cargo update -p ironcalc_base -p ironcalc`).
2. Open the 10 upstream PRs from the compare links above (one per branch), on sign-off.
3. As each merges upstream, it returns via the next `main` sync — then drop the local `fix/<slug>`
   and its `freecell-fixes` merge.

## What FreeCell already does (correct without any FreeCell change)

Nothing to hack: these are pure engine functions. Once `freecell-fixes` carries them and FreeCell's
lock is bumped, a formula that calls one of these names **computes** instead of erroring (`#NAME?`).
The only optional FreeCell-side touch is a **data** edit — adding the 11 names + arg templates to the
authored autocomplete catalog `freecell-core/src/functions.rs` so they autocomplete + show signature
hints (see `../architecture.md` §7; deferrable to a GAPS follow-on).
