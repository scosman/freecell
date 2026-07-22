---
status: complete
---

# Phase 9 — ADDRESS (§3.8). Verified; branch skipped (one empty-sheet divergence, owner decision).

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) with full R1C1 + sheet_text —
verified, no branch created.** `fn_address` —
`base/src/functions/lookup_and_reference/address_areas.rs:18` (registry: enum `Address`
mod.rs:161 · name-map:710 · dispatch:2359).

Verified against functional_spec §3.8 in a scratch module (deleted after verification).

## Vectors run (§3.8)

| Vector | Expected | Result |
|---|---|---|
| `ADDRESS(1,1)` | `$A$1` | PASS |
| `ADDRESS(2,3)` | `$C$2` | PASS |
| `ADDRESS(2,3,2)` | `C$2` | PASS |
| `ADDRESS(2,3,3)` | `$C2` | PASS |
| `ADDRESS(2,3,4)` | `C2` | PASS |
| `ADDRESS(2,3,1,FALSE)` | `R2C3` | PASS |
| `ADDRESS(2,3,2,FALSE)` | `R2C[3]` | PASS |
| `ADDRESS(2,3,3,FALSE)` | `R[2]C3` | PASS |
| `ADDRESS(2,3,4,FALSE)` | `R[2]C[3]` | PASS |
| `ADDRESS(1,1,1,TRUE,"Sheet1")` | `Sheet1!$A$1` | PASS |
| `ADDRESS(1,1,1,TRUE,"My Sheet")` | `'My Sheet'!$A$1` (quoted) | PASS |
| `ADDRESS(1,16384)` | `$XFD$1` | PASS |
| `ADDRESS(0,1)` | `#VALUE!` (row out of range) | PASS |
| `ADDRESS(1,1,1,TRUE,"")` | `!$A$1` (empty-sheet edge, O-4) | **DIVERGENCE → actual `$A$1`** |

Full abs_num 1–4 markers, complete R1C1 style, column→letters (`16384`→`XFD`), out-of-range
`#VALUE!`, and the `'My Sheet'` quoting predicate all behave per spec.

## DIVERGENCE — needs owner decision

`ADDRESS(1,1,1,TRUE,"")` returns `$A$1` instead of the spec's `!$A$1`. Excel emits the `!`
prefix even for an empty `sheet_text`; the fork drops the prefix (and the `!`) when the name is
empty. This is exactly the O-4 edge the spec flagged "for confirmation." Narrow divergence on the
empty-string sheet argument only; flagged for owner decision / a possible small `fix/address`
correctness branch. No fork source modified.
