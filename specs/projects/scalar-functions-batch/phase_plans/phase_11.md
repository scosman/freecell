---
status: complete
---

# Phase 11: Integration + FreeCell pickup + PR prep

> **Correction (2026-08-06):** one of the four fixes below, DOLLAR
> (`fix/dollar-negative-zero`), was **wrong** — Excel returns `($0.00)` for
> `DOLLAR(-0.001,2)` (upstream ironcalc/IronCalc#1293, closed; Google Sheets is what returns
> `$0.00`). It was reverted out of `freecell-fixes` and its branch deleted deliberately, so the
> real deliverable is **3** branches / PRs (TRIM + ADDRESS + XMATCH) and the smoke test asserts
> `($0.00)`. The record below stands as history.

## Overview

The final phase. All fork work landed in earlier phases on the fork's `freecell-fixes`
branch (HEAD `9161a463`). This phase brings the batch into FreeCell with **no FreeCell-side
code beyond a pin bump**: re-pin the lock onto the new `freecell-fixes` HEAD, prove the pin
picks up the batch end-to-end with a `freecell-engine` smoke test, and finalize the upstream
PR preps for the fork branches that actually landed.

The batch's real deliverable narrowed during verification: **4 fork branches / PRs**
(`fix/trim-internal-runs`, `fix/dollar-negative-zero`, `fix/address-empty-sheet`,
`fix/xmatch-array-constant`) rather than the 10 the plan front-loaded — **7 of the 11
functions were already present upstream + verified in place** (PROPER, REPLACE, CHAR, CODE,
CLEAN, PERCENTILE.INC, QUARTILE.INC), and the SUMPRODUCT `--` divergence turned out to be an
**engine unary-minus operator** issue, deferred off the critical path
(`projects/unary-minus-boolean-coercion.md`).

## Steps

1. **Re-pin the lock.** From `app/`: `cargo update -p ironcalc_base -p ironcalc`. Moved
   `app/Cargo.lock`'s fork git rev for both `ironcalc` and `ironcalc_base` from `81feec40`
   to the new `freecell-fixes` HEAD `9161a463` (the XMATCH-array-constant merge). Cargo
   fetched the fork through the container git-proxy — no extra redirect config needed. (The
   lockfile regen incidentally nudged a transitive `windows-core` 0.57→0.58 line via
   `iana-time-zone`; harmless, accepted as-is.)

2. **Add the FreeCell-side smoke test.** New `#[test]
   scalar_functions_batch_computes_through_pinned_engine` in the existing `#[cfg(test)] mod
   tests` of `app/crates/freecell-engine/src/document.rs`, matching the crate's convention
   (`WorkbookDocument::new_empty()` → `set_cell_input` → `evaluate()` → `formatted_value()`).
   Sets each function as a formula in A1 and asserts the computed, formatted value — a
   `#NAME?`/`#VALUE!`/wrong value would fail. 13 assertions:
   - **9 presence** (compute, not `#NAME?`): `SUMPRODUCT({1,2,3},{4,5,6})`=32,
     `PROPER("john smith")`="John Smith", `REPLACE("abcdefg",3,2,"XY")`="abXYefg",
     `CHAR(65)`="A", `CODE("A")`=65, `CLEAN("Hello"&CHAR(7)&"World")`="HelloWorld",
     `PERCENTILE.INC({1,2,3,4},0.5)`=2.5, `QUARTILE.INC({1,2,4,7,8,9,10,12},2)`=7.5,
     `XMATCH(30,{10,20,30,40,50})`=3.
   - **4 fixes** (prove the pin carries each landed branch): `TRIM("a    b")`="a b"
     (`fix/trim-internal-runs`), `DOLLAR(-0.001,2)`="$0.00" (`fix/dollar-negative-zero`)
     (now `($0.00)` — see the correction),
     `ADDRESS(1,1,1,TRUE,"")`="!$A$1" (`fix/address-empty-sheet`),
     `XMATCH("ban*",{"apple","banana","cherry"},2)`=2 (`fix/xmatch-array-constant`).

3. **Finalize PR preps.** Verified `fork-fixes/README.md` already carries complete,
   self-contained upstream-PR preps (compare link + branch/merge commits + title + body) for
   the fork branches. Nothing missing; no edits needed. (The DOLLAR prep has since been
   removed — see the correction at the top; 3 preps remain.)

## Tests

- `scalar_functions_batch_computes_through_pinned_engine` — 13 formula→value assertions
  (9 presence + 4 fork fixes) proving the batch computes end-to-end through the real FreeCell
  engine seam under the new pin.

## Verification

- `cargo test -p freecell-engine --lib` (from `app/`): **372 passed, 0 failed, 1 ignored**
  (the pre-existing CF-undo ignore, unrelated); includes the new smoke.
- `cargo fmt --all --check`: clean.
- No pixel/render suite, no benchmarks — engine-only, no UI surface (arch §6; CLAUDE.md
  render gate does not apply).

## Outcome

- **Pin bump:** `81feec40` → `9161a463` (`freecell-fixes` HEAD).
- **Real fork deliverable:** 3 branches / PRs — TRIM + ADDRESS + XMATCH (a 4th, DOLLAR, was
  withdrawn — see the correction at the top); preps finalized in `fork-fixes/README.md` for the
  owner to open (human-in-loop).
- **7 functions already present + verified** upstream, no branch/PR.
- **SUMPRODUCT `--` / unary-minus deferred** off critical path
  (`projects/unary-minus-boolean-coercion.md`).
- No FreeCell-side code beyond the lock bump + smoke test; the optional autocomplete-catalog
  touch (arch §7) remains deferrable.
