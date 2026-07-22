---
status: complete
---

# Phase 7 — DOLLAR (§3.7). Verified; branch skipped (one neg-zero divergence, owner decision).

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) — verified, no branch created.**
`fn_dollar` — `base/src/functions/text/string_format.rs:60` (registry: enum `Dollar`
mod.rs:227 · name-map:776 · dispatch:2415).

Verified against functional_spec §3.7 in a scratch module (deleted after verification).

## Vectors run (§3.7)

| Vector | Expected | Result |
|---|---|---|
| `DOLLAR(1234.567)` | `$1,234.57` | PASS |
| `DOLLAR(1234.567,1)` | `$1,234.6` | PASS |
| `DOLLAR(-1234.567,2)` | `($1,234.57)` (negative → parens) | PASS |
| `DOLLAR(99.9,0)` | `$100` | PASS |
| `DOLLAR(12345.67,-2)` | `$12,300` (round left of point) | PASS |
| `DOLLAR(50,-3)` | `$0` (rounds to nearest 1000 = 0) | PASS |
| `DOLLAR(0)` | `$0.00` | PASS |
| `DOLLAR(-0.001,2)` | `$0.00` (negative-zero guard) | **DIVERGENCE → actual `($0.00)`** |

Currency symbol, thousands separator, rounding (half-away), negative→parens, negative-decimals
rounding, and the `$0` integer guard are all correct.

## DIVERGENCE — needs owner decision

`DOLLAR(-0.001,2)` returns `($0.00)` instead of the spec's `$0.00`. The spec §3.7 calls for a
**negative-zero guard**: a negative magnitude that rounds to zero should render as an unsigned
`$0.00`, not parenthesized. The fork applies the parenthesized-negative format before checking
whether the rounded magnitude is zero. Narrow cosmetic divergence on the rounds-to-zero-negative
case only; flagged for owner decision / a possible small `fix/dollar` correctness branch. No fork
source modified.
