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

**Inverted after Phase 0 discovery + Phase 1/3–10 verification: 10 of the 11 functions were
already present + registered on the fork's `main` (hence on `freecell-fixes`), so they are
**verified-and-skipped** (no upstream PR needed). The batch collapses to **1 real branch = TRIM**
(`fix/trim-internal-runs`, landed).** Verification ran every functional_spec §3.1–§3.11
worked-example vector against the existing impls via a scratch integration module (deleted
after use); see the per-phase plans `phase_plans/phase_{1,3,4,5,6,7,8,9,10}.md`.

Four narrow **DIVERGENCES** surfaced during verification (recorded below; each flagged for owner
decision — none fixed): SUMPRODUCT `--` idiom, DOLLAR negative-zero, ADDRESS empty-sheet prefix,
XMATCH array-constant acceptance. Plus one owner-decided **SKIP** (CHAR/CODE 5 undefined CP1252
slots).

## Status table (10 verified-and-skipped · 1 landed = TRIM)

| # | Branch | Function(s) | Impl fn (module) | Verified vs §3 | `freecell-fixes` | Upstream PR | State |
|---|--------|-------------|----------------------------------|-------|------------------|-------------|-------|
| 1 | ~~`fix/sumproduct`~~ | SUMPRODUCT | `fn_sumproduct` `math_and_trigonometry/sumproduct.rs:19` | §3.1 — 7/8 pass (1 divergence: `--` idiom) | inherited | n/a | **already present — verified, branch skipped** |
| 2 | `fix/trim-internal-runs` | TRIM (fix) | `fn_trim` body (`text/common.rs`) | `cargo test -p ironcalc_base` green + `make lint` | ✅ (merge `0a36e79e`) | ✅ prep (below) | **landed + pushed** — branch `6c894ba2` |
| 3 | ~~`fix/proper`~~ | PROPER | `fn_proper` `text/string_format.rs:193` | §3.2 — all pass | inherited | n/a | **already present — verified, branch skipped** |
| 4 | ~~`fix/replace`~~ | REPLACE | `fn_replace` `text/string_format.rs:207` | §3.3 — all pass | inherited | n/a | **already present — verified, branch skipped** |
| 5 | ~~`fix/char-code`~~ | CHAR + CODE | `fn_char`, `fn_code` `text/char_code.rs` | §3.4/§3.5 — all pass; 5 undefined CP1252 slots = owner SKIP | inherited | n/a | **already present (CP1252) — verified, branch skipped** |
| 6 | ~~`fix/clean`~~ | CLEAN | `fn_clean` `text/char_code.rs:137` | §3.6 — all pass (0–31 only) | inherited | n/a | **already present — verified, branch skipped** |
| 7 | ~~`fix/dollar`~~ | DOLLAR | `fn_dollar` `text/string_format.rs:60` | §3.7 — 7/8 pass (1 divergence: neg-zero → `($0.00)`) | inherited | n/a | **already present — verified, branch skipped** |
| 8 | ~~`fix/percentile-quartile-inc`~~ | PERCENTILE(.INC) + QUARTILE(.INC) | `fn_percentile_inc` `statistical/percentile.rs:44`, `fn_quartile_inc` `statistical/quartile.rs` | §3.9/§3.10 — all pass; legacy routes to inclusive | inherited | n/a | **already present — verified, branch skipped** |
| 9 | ~~`fix/address`~~ | ADDRESS | `fn_address` `lookup_and_reference/address_areas.rs:18` | §3.8 — 13/14 pass (1 divergence: empty sheet → `$A$1`) | inherited | n/a | **already present (full R1C1) — verified, branch skipped** |
| 10 | ~~`fix/xmatch`~~ | XMATCH | `fn_xmatch` `lookup_and_reference/xmatch.rs` | §3.11 — all pass on ranges (1 divergence: array-constant → `#VALUE!`) | inherited | n/a | **already present (all modes) — verified, branch skipped** |

Only row #2 (TRIM) is a real branch/PR. Rows #1,3–10 are inherited from upstream `main` and
verified in place — no `fix/*` branch, no upstream PR.

## Divergences found during verification (owner decision — none fixed)

| Function | §ref | Spec expects | Fork actual | Note |
|---|---|---|---|---|
| SUMPRODUCT | §3.1 | `SUMPRODUCT(--(cond))` counts → `2` | `0` | Root cause = unary-minus operator: `=--TRUE`→`TRUE` not `1`; SUMPRODUCT itself correct, `1*(cond)` idiom works. Operator-level fix, out of a SUMPRODUCT branch's scope. |
| DOLLAR | §3.7 | `DOLLAR(-0.001,2)` → `$0.00` | `($0.00)` | Missing negative-zero guard: a negative that rounds to 0 is parenthesized instead of unsigned. |
| ADDRESS | §3.8 (O-4) | `ADDRESS(1,1,1,TRUE,"")` → `!$A$1` | `$A$1` | Empty `sheet_text` should still emit the `!` prefix; fork drops it. |
| XMATCH | §2.5 | accepts array constants | `#VALUE!` on `{...}` | Only range lookup_array accepted; all core logic correct on ranges. |
| CHAR/CODE | §3.4/§3.5 | `CODE(CHAR(n))==n` over full 1..=255 | `#VALUE!` on 5 undefined CP1252 slots {129,141,143,144,157} | **Owner-decided SKIP** (acceptable divergence per Phase 0); defined-range round-trip passes. |

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
2. Open the **1** upstream PR from the compare link above (`fix/trim-internal-runs`), on sign-off.
   (Rows #1,3–10 need no PR — they are already upstream, verified in place.)
3. As it merges upstream, it returns via the next `main` sync — then drop the local
   `fix/trim-internal-runs` and its `freecell-fixes` merge.
4. **Optional — the four divergences** (SUMPRODUCT `--`, DOLLAR neg-zero, ADDRESS empty-sheet,
   XMATCH array-constant): decide whether any warrants its own `fix/*` correctness branch. None
   were changed during verification.

## What FreeCell already does (correct without any FreeCell change)

Nothing to hack: these are pure engine functions. Once `freecell-fixes` carries them and FreeCell's
lock is bumped, a formula that calls one of these names **computes** instead of erroring (`#NAME?`).
The only optional FreeCell-side touch is a **data** edit — adding the 11 names + arg templates to the
authored autocomplete catalog `freecell-core/src/functions.rs` so they autocomplete + show signature
hints (see `../architecture.md` §7; deferrable to a GAPS follow-on).
