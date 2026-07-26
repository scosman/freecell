---
status: complete
---

# Phase 7 — DOLLAR (§3.7). Divergence confirmed + FIXED (`fix/dollar-negative-zero`).

## Outcome

**Confirmed a real divergence during verification, then fixed it as its own fork branch.**
`fn_dollar` — `base/src/functions/text/string_format.rs:60` (registry: enum `Dollar`
mod.rs:227 · name-map:776 · dispatch:2415).

The rest of DOLLAR was already present on `freecell-fixes` (inherited from `main`) and correct;
only the **negative-zero** case diverged. That one case is now fixed.

## Vectors run (§3.7)

| Vector | Expected | Before fix | After fix |
|---|---|---|---|
| `DOLLAR(1234.567)` | `$1,234.57` | PASS | PASS |
| `DOLLAR(1234.567,1)` | `$1,234.6` | PASS | PASS |
| `DOLLAR(-1234.567,2)` | `($1,234.57)` (negative → parens) | PASS | PASS |
| `DOLLAR(99.9,0)` | `$100` | PASS | PASS |
| `DOLLAR(12345.67,-2)` | `$12,300` (round left of point) | PASS | PASS |
| `DOLLAR(50,-3)` | `$0` (rounds to nearest 1000 = 0) | PASS | PASS |
| `DOLLAR(0)` | `$0.00` | PASS | PASS |
| `DOLLAR(-0.001,2)` | `$0.00` (negative-zero guard) | **`($0.00)`** | **PASS** |
| `DOLLAR(-50,-3)` | `$0` (negative rounds to 0 left of point) | (n/a) | PASS |

## Confirmed divergence (CONFIRM-FIRST repro)

A minimal repro (`new_empty_model` + `_set`/`evaluate`/`_get_text`) confirmed
`DOLLAR(-0.001,2)` returned `($0.00)` instead of the spec §3.7 `$0.00`, while every regression
vector (real negatives still parenthesized, the `$0` integer guard, negative-decimals rounding)
stayed correct. Real fork behavior, not a harness artifact.

## Fix

Localized to `fn_dollar`. After the magnitude is formatted, detect the rounds-to-zero case
(the formatted string contains only `0`, `,`, `.`) and emit the unsigned `$…` form even when
`value < 0.0`. A value that is still non-zero after rounding stays parenthesized. Spec §3.7:
"If rounding zeroes out the whole magnitude the result is `$0`."

## Tests added

`base/src/test/text_functions/mod.rs`:
- `test_dollar_negative_rounds_to_zero` — `-0.001,2`→`$0.00`, `-1234.567,2`→`($1,234.57)`,
  `-50,-3`→`$0`, `0`→`$0.00`.
- `test_dollar_vectors` — `1234.567`→`$1,234.57`, `99.9,0`→`$100`, `12345.67,-2`→`$12,300`,
  `50,-3`→`$0`.

## Verification

`cargo test -p ironcalc_base` green (2191 unit + 23 doctests, 0 failed); `cargo fmt --check`
clean; `cargo clippy -p ironcalc_base --all-targets --all-features` clean under
`-D warnings` + unwrap/expect/panic lints.

## Fork delivery

- Branch `fix/dollar-negative-zero` off `main`, commit **`aa36a177`**
  ("Fix DOLLAR to not parenthesize a value that rounds to zero").
- Merged into `freecell-fixes` at **`6163e084`** (`--no-ff`, no conflicts).
- Both pushed to `scosman/ironcalc`. Upstream-PR prep recorded in `../fork-fixes/README.md`.
