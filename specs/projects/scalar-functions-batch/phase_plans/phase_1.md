---
status: complete
---

# Phase 1 — SUMPRODUCT (§3.1). Verified; branch skipped.

## Outcome

**Already present on the fork's `freecell-fixes` (inherited from `main`) — verified, no branch
created.** `fn_sumproduct` — `base/src/functions/math_and_trigonometry/sumproduct.rs:19`
(registry: enum `Sumproduct` mod.rs:132 · name-map:681 · dispatch:2645).

Verified against functional_spec §3.1 by running every worked-example vector through the fork's
test harness (`new_empty_model` / `_set` / `evaluate` / `_get_text`) in a scratch integration
module (`base/src/test/verify_scalar_batch.rs`, deleted after verification per Phase 0).

## Vectors run (§3.1)

| Vector | Expected | Result |
|---|---|---|
| `SUMPRODUCT({1,2,3},{4,5,6})` | `32` | PASS |
| `SUMPRODUCT({1,2,3})` | `6` | PASS |
| `SUMPRODUCT({1,2,3},{4,5})` (dim mismatch) | `#VALUE!` | PASS |
| `SUMPRODUCT((A1:A3="x")*(B1:B3))`, A={x,y,x} B={10,20,30} | `40` | PASS |
| `SUMPRODUCT((A1:A3="x"))` (booleans direct) | `0` | PASS |
| `SUMPRODUCT(D1:D3,E1:E3)`, D={1,"text",3} E={4,5,6} (text→0) | `22` | PASS |
| `SUMPRODUCT(1*(A1:A3="x"))` (arithmetic-coerced count) | `2` | PASS |
| `SUMPRODUCT(--(A1:A3="x"))` (double-unary count idiom) | `2` | **DIVERGENCE → actual `0`** |

**SUMPRODUCT itself is correct** — dimension check → `#VALUE!`, non-numeric elements (text /
blank / boolean) treated as `0`, error elements propagate, and both the multi-array and the
single-expression `(A=x)*(B)` array-context forms behave per spec.

## DIVERGENCE — needs owner decision (not a SUMPRODUCT bug)

The `--(condition)` boolean-counting idiom returns `0` instead of `2`. **Root cause is the
unary-minus operator, not SUMPRODUCT:** in the fork `=--TRUE` evaluates to `TRUE` (the `--`
folds to identity on a boolean) rather than `1`, so the double-unary leaves booleans as
booleans, which SUMPRODUCT then (correctly) treats as `0`. The arithmetic `1*(cond)` and
`(cond)*(range)` idioms DO coerce and return the right counts, so the SUMPRODUCT contract is
met; only the specific `--` idiom is affected. Fixing it is an operator-level change (unary
minus should coerce a boolean to a number), out of scope for a SUMPRODUCT branch — flagged for
owner decision / a possible separate correctness branch. No fork source modified.
