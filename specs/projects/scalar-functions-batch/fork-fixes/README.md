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

## Real deliverable (post-verification + fix round)

Phase 0 discovery + Phase 1/3–10 verification found **10 of the 11 functions already present +
registered** on the fork's `main` (hence on `freecell-fixes`). Verification ran every
functional_spec §3.1–§3.11 worked-example vector against the existing impls (via a scratch
integration module, deleted after use; then via CONFIRM-FIRST minimal repros for the divergences).
That surfaced **four narrow divergences**. After a CONFIRM-FIRST re-check with correct syntax,
**three were confirmed real and function-local** and each was fixed on its own branch; the fourth
is an engine-operator issue **deferred** off the critical path.

**The batch's real deliverable = 4 fork branches / PRs:**

- **TRIM fix** (`fix/trim-internal-runs`) — the one net-new behavior fix from the original scope.
- **DOLLAR** (`fix/dollar-negative-zero`) — negative-zero-guard correctness fix.
- **ADDRESS** (`fix/address-empty-sheet`) — empty-`sheet_text` prefix correctness fix (O-4 edge).
- **XMATCH** (`fix/xmatch-array-constant`) — accept array constants as `lookup_array`.

The other **7 functions** (PROPER, REPLACE, CHAR, CODE, CLEAN, PERCENTILE.INC, QUARTILE.INC) are
**already present — verified, skipped** (no branch, no upstream PR). **SUMPRODUCT** is likewise
present + verified in place, but its `--`-idiom divergence turned out to be an **engine unary-minus
operator** bug, not a SUMPRODUCT bug — spun out as a **deferred** off-critical-path follow-on
(`projects/unary-minus-boolean-coercion.md`), not fixed here.

Per-phase records: `phase_plans/phase_{1,3,4,5,6,7,8,9,10}.md`. One additional owner-decided
**SKIP** stands (CHAR/CODE 5 undefined CP1252 slots — acceptable divergence).

## Status table (7 verified-and-skipped · 4 landed = TRIM + DOLLAR + ADDRESS + XMATCH · 1 deferred)

| # | Branch | Function(s) | Impl fn (module) | Verified vs §3 | `freecell-fixes` | Upstream PR | State |
|---|--------|-------------|----------------------------------|-------|------------------|-------------|-------|
| 1 | ~~`fix/sumproduct`~~ | SUMPRODUCT | `fn_sumproduct` `math_and_trigonometry/sumproduct.rs:19` | §3.1 — 7/8 pass; the 1 divergence is a unary-minus **operator** bug, not SUMPRODUCT | inherited | n/a | **already present — verified; divergence DEFERRED (operator, own project)** |
| 2 | `fix/trim-internal-runs` | TRIM (fix) | `fn_trim` body (`text/common.rs`) | `cargo test -p ironcalc_base` green + lint clean | ✅ (merge `0a36e79e`) | ✅ prep (below) | **landed + pushed** — branch `6c894ba2` |
| 3 | ~~`fix/proper`~~ | PROPER | `fn_proper` `text/string_format.rs:193` | §3.2 — all pass | inherited | n/a | **already present — verified, branch skipped** |
| 4 | ~~`fix/replace`~~ | REPLACE | `fn_replace` `text/string_format.rs:207` | §3.3 — all pass | inherited | n/a | **already present — verified, branch skipped** |
| 5 | ~~`fix/char-code`~~ | CHAR + CODE | `fn_char`, `fn_code` `text/char_code.rs` | §3.4/§3.5 — all pass; 5 undefined CP1252 slots = owner SKIP | inherited | n/a | **already present (CP1252) — verified, branch skipped** |
| 6 | ~~`fix/clean`~~ | CLEAN | `fn_clean` `text/char_code.rs:137` | §3.6 — all pass (0–31 only) | inherited | n/a | **already present — verified, branch skipped** |
| 7 | `fix/dollar-negative-zero` | DOLLAR | `fn_dollar` `text/string_format.rs:60` | §3.7 — divergence (neg-zero → `($0.00)`) **confirmed + FIXED** | ✅ (merge `6163e084`) | ✅ prep (below) | **landed + pushed** — branch `aa36a177` |
| 8 | ~~`fix/percentile-quartile-inc`~~ | PERCENTILE(.INC) + QUARTILE(.INC) | `fn_percentile_inc` `statistical/percentile.rs:44`, `fn_quartile_inc` `statistical/quartile.rs` | §3.9/§3.10 — all pass; legacy routes to inclusive | inherited | n/a | **already present — verified, branch skipped** |
| 9 | `fix/address-empty-sheet` | ADDRESS | `fn_address` `lookup_and_reference/address_areas.rs:18` | §3.8 — divergence (empty sheet → `$A$1`) **confirmed + FIXED** | ✅ (merge `582a78b1`) | ✅ prep (below) | **landed + pushed** — branch `09259476` |
| 10 | `fix/xmatch-array-constant` | XMATCH | `fn_xmatch` `lookup_and_reference/xmatch.rs` | §3.11 — divergence (array-constant → `#VALUE!`) **confirmed + FIXED** | ✅ (merge `9161a463`) | ✅ prep (below) | **landed + pushed** — branch `f9d1f9ce` |

Rows #2, #7, #9, #10 are real branches/PRs. Rows #3–6, #8 are inherited from upstream `main` and
verified in place — no `fix/*` branch, no upstream PR. Row #1 (SUMPRODUCT) is verified in place; its
divergence is deferred as an operator-level follow-on (below).

## Divergences found during verification

| Function | §ref | Spec expects | Fork actual (before) | Disposition |
|---|---|---|---|---|
| DOLLAR | §3.7 | `DOLLAR(-0.001,2)` → `$0.00` | `($0.00)` | **FIXED** — `fix/dollar-negative-zero` (`aa36a177`). Rounds-to-zero unsigned guard in `fn_dollar`. |
| ADDRESS | §3.8 (O-4) | `ADDRESS(1,1,1,TRUE,"")` → `!$A$1` | `$A$1` | **FIXED** — `fix/address-empty-sheet` (`09259476`). Present-empty `sheet_text` emits the `!` prefix. |
| XMATCH | §2.5 | accepts array constants | `#VALUE!` on `{...}` | **FIXED** — `fix/xmatch-array-constant` (`f9d1f9ce`). Materialize `CalcResult::Array` as well as `Range`; function-local. |
| SUMPRODUCT | §3.1 | `SUMPRODUCT(--(cond))` counts → `2` | `0` | **DEFERRED** — root cause is the unary-minus operator (`=--TRUE`→`TRUE` not `1`), not SUMPRODUCT. Broad blast radius, out of this batch's scope. → `projects/unary-minus-boolean-coercion.md` (PROJECTS.md). Workarounds `1*(cond)` and `(A=x)*(B)` work today. |
| CHAR/CODE | §3.4/§3.5 | `CODE(CHAR(n))==n` over full 1..=255 | `#VALUE!` on 5 undefined CP1252 slots {129,141,143,144,157} | **Owner-decided SKIP** (acceptable per Phase 0); defined-range round-trip passes. |

## Pre-branch existence check (do this first, every branch)

Before creating any `fix/*`, confirm the capability isn't **already present** (a stale gap note, or
an upstream landing — hide/unhide was already upstream in `gaps_closing_7_15`):

- `git grep -i "<name>"` at the pinned rev / on `main`, and check the `Function` enum for the name.
- `git merge-base --is-ancestor <upstream-sha> origin/freecell-fixes` when a specific commit is suspected.
- **CHAR/CODE, PERCENTILE/QUARTILE, and TRIM especially** may already exist (common functions; TRIM
  already exists — this is a body fix). If a name already computes correctly → **skip / record "already
  present"**. If it exists but is wrong (a missing DOLLAR neg-zero guard, an ADDRESS empty-sheet prefix,
  XMATCH not accepting array constants) → the branch is a **correctness fix** to the existing impl.

## Upstream PR prep (agent preps; owner opens)

For each branch, once green on `freecell-fixes`:

- **Compare link:** `https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/<slug>`
- **Title:** single-feature, imperative (see per-branch below).
- **Body:** what/why (one paragraph) + the Excel contract (from `../functional_spec.md` §3) + a minimal
  repro (a formula → expected value) + "tests included: `<module>`". Self-contained; compiles against
  upstream `main`.

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

### `fix/dollar-negative-zero` — Fix DOLLAR to not parenthesize a value that rounds to zero

- **Compare link:** https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/dollar-negative-zero
- **Branch commit:** `aa36a177` (off `main`) · **`freecell-fixes` merge:** `6163e084` · both pushed to `scosman/ironcalc`.
- **Title:** `Fix DOLLAR to not parenthesize a value that rounds to zero`
- **Body:**

  > DOLLAR formats a number as currency text and wraps **negatives** in parentheses (no minus sign):
  > `DOLLAR(-1234.567, 2)` → `($1,234.57)`. But a negative `number` whose magnitude **rounds to zero**
  > must render as the unsigned `$0.00`, not `($0.00)` — Excel reserves the parenthesized form for values
  > that are still non-zero after rounding. The fork applied the parenthesized-negative format whenever
  > `value < 0.0`, before checking whether the rounded magnitude was zero, so `DOLLAR(-0.001, 2)` returned
  > `($0.00)`.
  >
  > The fix, localized to `fn_dollar` (`base/src/functions/text/string_format.rs`), detects the
  > rounds-to-zero case from the already-formatted magnitude (only `0`, `,`, `.` remain) and emits the
  > non-negative form for it. A value still non-zero after rounding stays parenthesized.
  >
  > Minimal repro: `DOLLAR(-0.001, 2)` → `$0.00` (previously `($0.00)`); `DOLLAR(-50, -3)` → `$0`. Real
  > negatives unchanged: `DOLLAR(-1234.567, 2)` → `($1,234.57)`.
  >
  > tests included: `base/src/test/text_functions/mod.rs`

### `fix/address-empty-sheet` — Fix ADDRESS to prefix ! for an empty sheet_text argument

- **Compare link:** https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/address-empty-sheet
- **Branch commit:** `09259476` (off `main`) · **`freecell-fixes` merge:** `582a78b1` · both pushed to `scosman/ironcalc`.
- **Title:** `Fix ADDRESS to prefix ! for an empty sheet_text argument`
- **Body:**

  > ADDRESS builds a text cell reference; a present `sheet_text` argument prefixes `sheet_text!`. In Excel
  > the `!` separator is emitted whenever the argument is **present**, even when it is an empty string:
  > `ADDRESS(1,1,1,TRUE,"")` → `!$A$1`. Only an **omitted** `sheet_text` produces no prefix
  > (`ADDRESS(1,1)` → `$A$1`). The fork treated a present-but-empty argument like an omitted one and
  > dropped the `!`, returning `$A$1`.
  >
  > The fix, localized to `fn_address` (`base/src/functions/lookup_and_reference/address_areas.rs`),
  > always emits `{quote_name(s)}!` when the argument is present (`args.len() == 5`); `quote_name("")` is
  > `""`, so a present-empty argument yields the bare `!` prefix. Quoting of non-empty names is unchanged.
  >
  > Minimal repro: `ADDRESS(1,1,1,TRUE,"")` → `!$A$1` (previously `$A$1`). Unchanged: `ADDRESS(1,1)` →
  > `$A$1`, `ADDRESS(1,1,1,TRUE,"My Sheet")` → `'My Sheet'!$A$1`.
  >
  > tests included: `base/src/test/lookup_and_reference/test_fn_address_areas.rs`

### `fix/xmatch-array-constant` — Add array-constant support to XMATCH lookup_array

- **Compare link:** https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/xmatch-array-constant
- **Branch commit:** `f9d1f9ce` (off `main`) · **`freecell-fixes` merge:** `9161a463` · both pushed to `scosman/ironcalc`.
- **Title:** `Add array-constant support to XMATCH lookup_array`
- **Body:**

  > XMATCH returns the 1-based position of a lookup value within `lookup_array`. The fork only accepted a
  > **range** reference for `lookup_array`; an in-formula **array constant** such as
  > `XMATCH("ban*", {"apple","banana","cherry"}, 2)` returned `#VALUE!`, because `fn_xmatch`'s final match
  > handled only `CalcResult::Range` and an array literal evaluates to `CalcResult::Array`.
  >
  > The fix materializes `lookup_array` into a one-dimensional `Vec<CalcResult>` from **either** a range
  > (unchanged path) **or** a `CalcResult::Array` (flattened, single-row/column enforced), then runs the
  > existing linear/binary search over that vector. A 2-D array constant still yields `#VALUE!`. The
  > change is function-local (`base/src/functions/lookup_and_reference/xmatch.rs`).
  >
  > **Reviewer note:** the diff is mostly **de-indentation whitespace** — the search block moved out of
  > the `Range` match arm to run over the shared vector. View it whitespace-insensitive
  > (`git diff -w` / "Hide whitespace") for a readable diff.
  >
  > Minimal repro: `XMATCH("ban*", {"apple","banana","cherry"}, 2)` → `2` and
  > `XMATCH(30, {10,20,30,40,50})` → `3` (both previously `#VALUE!`); ranges unchanged.
  >
  > tests included: `base/src/test/lookup_and_reference/test_fn_xmatch.rs`

## Owner action

1. If fork push is blocked, apply each preserved `NNNN-<slug>.patch` onto `scosman/ironcalc`
   `freecell-fixes` and push (`git am < NNNN-<slug>.patch`), then re-pin FreeCell
   (`cd app && cargo update -p ironcalc_base -p ironcalc`). (All 4 branches pushed cleanly this
   round — no patches needed.)
2. Open the **4** upstream PRs from the compare links above (`fix/trim-internal-runs`,
   `fix/dollar-negative-zero`, `fix/address-empty-sheet`, `fix/xmatch-array-constant`), on sign-off.
   (Rows #3–6, #8 need no PR — already upstream, verified in place.)
3. As each merges upstream, it returns via the next `main` sync — then drop the local `fix/*`
   branch and its `freecell-fixes` merge.
4. **Deferred — SUMPRODUCT `--` / unary-minus** (`projects/unary-minus-boolean-coercion.md`): an
   engine-operator fix with broad blast radius; decide whether/when to open it as its own project.
   Not changed in this batch.

## What FreeCell already does (correct without any FreeCell change)

Nothing to hack: these are pure engine functions. Once `freecell-fixes` carries them and FreeCell's
lock is bumped, a formula that calls one of these names **computes** instead of erroring (`#NAME?`).
The only optional FreeCell-side touch is a **data** edit — adding the 11 names + arg templates to the
authored autocomplete catalog `freecell-core/src/functions.rs` so they autocomplete + show signature
hints (see `../architecture.md` §7; deferrable to a GAPS follow-on).
