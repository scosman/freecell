---
status: complete
---

# Phase 9 — ADDRESS (§3.8). Divergence confirmed + FIXED (`fix/address-empty-sheet`).

## Outcome

**Confirmed a real divergence during verification, then fixed it as its own fork branch.**
`fn_address` — `base/src/functions/lookup_and_reference/address_areas.rs:18` (registry: enum
`Address` mod.rs:161 · name-map:710 · dispatch:2359).

The rest of ADDRESS was already present on `freecell-fixes` (inherited from `main`) with full
R1C1 + `sheet_text` support and correct; only the **empty-`sheet_text`** edge (the O-4 edge the
spec flagged for confirmation) diverged. That one case is now fixed.

## Vectors run (§3.8)

| Vector | Expected | Before fix | After fix |
|---|---|---|---|
| `ADDRESS(1,1)` | `$A$1` | PASS | PASS |
| `ADDRESS(2,3)` | `$C$2` | PASS | PASS |
| `ADDRESS(2,3,2)` | `C$2` | PASS | PASS |
| `ADDRESS(2,3,3)` | `$C2` | PASS | PASS |
| `ADDRESS(2,3,4)` | `C2` | PASS | PASS |
| `ADDRESS(2,3,1,FALSE)` | `R2C3` | PASS | PASS |
| `ADDRESS(2,3,2,FALSE)` | `R2C[3]` | PASS | PASS |
| `ADDRESS(2,3,3,FALSE)` | `R[2]C3` | PASS | PASS |
| `ADDRESS(2,3,4,FALSE)` | `R[2]C[3]` | PASS | PASS |
| `ADDRESS(1,1,1,TRUE,"Sheet1")` | `Sheet1!$A$1` | PASS | PASS |
| `ADDRESS(1,1,1,TRUE,"My Sheet")` | `'My Sheet'!$A$1` (quoted) | PASS | PASS |
| `ADDRESS(1,16384)` | `$XFD$1` | PASS | PASS |
| `ADDRESS(0,1)` | `#VALUE!` (row out of range) | PASS | PASS |
| `ADDRESS(1,1,1,TRUE,"")` | `!$A$1` (empty-sheet edge, O-4) | **`$A$1`** | **PASS** |

## Confirmed divergence (CONFIRM-FIRST repro)

A minimal repro confirmed `ADDRESS(1,1,1,TRUE,"")` returned `$A$1` instead of the spec §3.8 / O-4
`!$A$1`, while the omitted-arg case (`ADDRESS(1,1)` → `$A$1`, no `!`), the `'My Sheet'` quoting
predicate, and `$XFD$1` all stayed correct. Real fork behavior. The distinction is
**arg omitted** (no prefix) vs **arg present but empty** (prefix `!`).

## Fix

Localized to `fn_address`. When `sheet_text` is **present** (`args.len() == 5`) always emit
`{quote_name(s)}!`, even for an empty string — `quote_name("")` is `""`, so a present-empty
argument yields the `!` prefix. An omitted argument still yields no prefix. The quoting logic is
unchanged (an empty name needs no quoting).

## Tests added

`base/src/test/lookup_and_reference/test_fn_address_areas.rs`:
- `test_address_empty_sheet_text` — `""`→`!$A$1`, omitted→`$A$1`.
- `test_address_quoted_sheet_text` — `"My Sheet"`→`'My Sheet'!$A$1`, `(1,16384)`→`$XFD$1`.

## Verification

`cargo test -p ironcalc_base` green (2191 unit + 23 doctests, 0 failed); `cargo fmt --check`
clean; `cargo clippy -p ironcalc_base --all-targets --all-features` clean under
`-D warnings` + unwrap/expect/panic lints.

## Fork delivery

- Branch `fix/address-empty-sheet` off `main`, commit **`09259476`**
  ("Fix ADDRESS to prefix ! for an empty sheet_text argument").
- Merged into `freecell-fixes` at **`582a78b1`** (`--no-ff`, no conflicts).
- Both pushed to `scosman/ironcalc`. Upstream-PR prep recorded in `../fork-fixes/README.md`.
