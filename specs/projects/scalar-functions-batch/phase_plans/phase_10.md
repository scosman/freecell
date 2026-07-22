---
status: complete
---

# Phase 10 — XMATCH (§3.11). Divergence confirmed + FIXED (`fix/xmatch-array-constant`).

## Outcome

**Confirmed a real, function-local divergence during verification, then fixed it as its own fork
branch.** `fn_xmatch` — `base/src/functions/lookup_and_reference/xmatch.rs` (registry: enum
`Xmatch` mod.rs:184 · name-map:733 · dispatch:2383).

All four `match_mode`s × four `search_mode`s were already present on `freecell-fixes` (inherited
from `main`) and correct on **ranges**; the divergence was that an **inline array constant**
lookup_array yielded `#VALUE!`. That is now fixed — XMATCH accepts array constants as well.

## Vectors run (§3.11)

| Vector | Expected | Before fix | After fix |
|---|---|---|---|
| `XMATCH(30,arr)` range | `3` (exact) | PASS | PASS |
| `XMATCH(35,arr)` range | `#N/A` (exact not found) | PASS | PASS |
| `XMATCH(35,arr,-1)` range | `3` (next smaller = 30) | PASS | PASS |
| `XMATCH(35,arr,1)` range | `4` (next larger = 40) | PASS | PASS |
| `XMATCH(20,{10,20,20,30},0,1)` range | `2` (first match) | PASS | PASS |
| `XMATCH(20,{10,20,20,30},0,-1)` range | `3` (last match, reverse) | PASS | PASS |
| `XMATCH(30,arr,0,2)` range | `3` (binary ascending) | PASS | PASS |
| `XMATCH(30,{50,40,30,20,10},0,-2)` range | `3` (binary descending) | PASS | PASS |
| `XMATCH("BANANA",words)` range | `2` (case-insensitive) | PASS | PASS |
| `XMATCH(5,{"5","x"})` range | `#N/A` (type-sensitive: 5 ≠ "5") | PASS | PASS |
| `XMATCH(3,J1:K2)` 2-D lookup_array | `#VALUE!` | PASS | PASS |
| `XMATCH("ban*",{"apple","banana","cherry"},2)` **array-const** | `2` (wildcard) | **`#VALUE!`** | **PASS** |
| `XMATCH(30,{10,20,30,40,50})` **array-const** | `3` (exact) | **`#VALUE!`** | **PASS** |
| `XMATCH(35,{10,20,30,40,50},1)` **array-const** | `4` (next larger) | **`#VALUE!`** | **PASS** |
| `XMATCH(30,{10;20;30;40;50})` **array-const (column)** | `3` | **`#VALUE!`** | **PASS** |
| `XMATCH(99,{10,20,30})` **array-const** | `#N/A` (not found, not `#VALUE!`) | **`#VALUE!`** | **PASS** |

## Confirmed divergence + function-local investigation (CONFIRM-FIRST)

A minimal repro confirmed both `XMATCH("ban*", {"apple","banana","cherry"}, 2)` and
`XMATCH(30, {10,20,30,40,50})` returned `#VALUE!` — real fork behavior, not a harness artifact.

**Investigated whether the fix was function-local or broader arg-marshaling.** `fn_xmatch`'s
final `match` only handled `CalcResult::Range` for `lookup_array`; an array literal evaluates to
`CalcResult::Array`, which fell through to the `#VALUE!` arm. The parent module
`lookup_and_reference` already has a private `array_node_to_calc_result` helper (reachable from
the `xmatch` child module via `super::`), so materializing an array into a `Vec<CalcResult>` is
**entirely local to `xmatch.rs`** — not a broad shared arg-marshaling refactor. Fixed as
branch C.

## Fix

Localized to `fn_xmatch`. Materialize `lookup_array` into a one-dimensional `Vec<CalcResult>`
from **either** a `CalcResult::Range` (unchanged path: dimension check, entire-col/row clamp,
`prepare_array`) **or** a `CalcResult::Array` (new: single-row/column enforced, flattened via
`super::array_node_to_calc_result`), then run the existing linear/binary search over that vector.
A 2-D array constant still yields `#VALUE!`.

Diff note: mostly de-indentation whitespace (the search block moved out of the `Range` match arm
to run over the shared vector) — review whitespace-insensitive.

## Tests added

`base/src/test/lookup_and_reference/test_fn_xmatch.rs`:
- `test_xmatch_array_constant_wildcard` — `"ban*"` against `{"apple","banana","cherry"}` → `2`.
- `test_xmatch_array_constant_numeric` — exact `30`→`3`, next-larger `35,1`→`4`, column
  `{10;20;30}`→`3`, not-found `99`→`#N/A`.

## Verification

`cargo test -p ironcalc_base` green (2191 unit + 23 doctests, 0 failed); `cargo fmt --check`
clean; `cargo clippy -p ironcalc_base --all-targets --all-features` clean under
`-D warnings` + unwrap/expect/panic lints.

## Fork delivery

- Branch `fix/xmatch-array-constant` off `main`, commit **`f9d1f9ce`**
  ("Add array-constant support to XMATCH lookup_array").
- Merged into `freecell-fixes` at **`9161a463`** (`--no-ff`, no conflicts).
- Both pushed to `scosman/ironcalc`. Upstream-PR prep recorded in `../fork-fixes/README.md`.
