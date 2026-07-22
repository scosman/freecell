# Unary-Minus Boolean/Text Coercion (SUMPRODUCT `--` count idiom)

**Status: Future** (deferred from `scalar-functions-batch`, 2026-07-22).

## The divergence

Excel's double-unary idiom `--(condition)` coerces a boolean array to `1`/`0` so it can be summed —
the canonical way to **count** matches with SUMPRODUCT:

```
SUMPRODUCT(--(A1:A3="x"))   with A = {"x","y","x"}   → 2   (Excel)
```

In our IronCalc fork this returns **`0`**. Reduced further, the operator itself is the tell:

```
=--TRUE      → TRUE   (should be 1)
=-TRUE       → TRUE   (should be -1)
```

SUMPRODUCT then sums a boolean array it (correctly, per its own contract) treats as `0`, so the
count comes out `0`. This is the SUMPRODUCT §3.1 divergence recorded in
`specs/projects/scalar-functions-batch/functional_spec.md` §3.1 (and the batch's
`fork-fixes/README.md`).

## Root cause

**The bug is in the unary-minus operator, not SUMPRODUCT.** Unary minus negates its operand without
first coercing a **boolean** (or numeric-looking **text**) to a number. So `-TRUE` stays a boolean
rather than becoming `-1`, and the doubled `--TRUE` stays `TRUE` rather than `1`. Arithmetic
**binary** operators already coerce (see below), so only the unary path is affected.

SUMPRODUCT itself is verified-correct: in its multiple-array form it treats every non-numeric
element (text, blank, **boolean**) as `0` by design (§3.1) — which is exactly *why* the `--` idiom
exists and why the operator's failure to coerce surfaces here as a `0`.

## Impact

- **Broken:** the `SUMPRODUCT(--(cond))` / `SUMPRODUCT(--range)` count idiom, and any formula that
  negates a boolean or numeric text and expects a number (`-TRUE`, `--"3"`, `-(A1>0)`).
- **Works today (workarounds):** multiply-by-one and product forms coerce via the binary `*`:
  - `SUMPRODUCT(1*(A1:A3="x"))` → `2`
  - `SUMPRODUCT((A1:A3="x")*(B1:B3))` → the classic conditional-sum idiom (§3.1) — correct.

  So users have working alternatives; only the specific `--` spelling misbehaves.

## Why it's deferred (out of the scalar-functions batch's scope)

The scalar-functions batch is **function-local** engine coverage/correctness — new registry entries
plus small in-function fixes (DOLLAR neg-zero guard, ADDRESS empty-sheet prefix, XMATCH
array-constant acceptance). This is different in kind: it's an **engine operator** fix with a
**broad blast radius** — every formula that applies unary `-`/`--` to a non-number runs through the
changed path, so the risk surface and required regression coverage are much larger than a single
function. Bundling it into a SUMPRODUCT branch would also violate "one fix = one branch = one clean
upstream PR" (it isn't a SUMPRODUCT change at all). It gets its own project when prioritized.

## Fix sketch (when picked up)

1. In the unary-minus evaluation path (the `Node::UnaryKind`/negation arm of the engine's
   expression evaluator — the same place binary `+ - * /` live), **coerce the operand to a number
   first** using the engine's existing number-coercion helper (`TRUE`→1, `FALSE`→0, numeric text
   `"3"`→3, empty→0), then negate. Non-numeric text → `#VALUE!`; an error operand propagates.
2. Mirror whatever coercion the binary arithmetic operators already apply, so unary and binary
   minus agree (`-TRUE` == `0 - TRUE` == `-1`).
3. Guard with the **full** `cargo test -p ironcalc_base` suite plus targeted new cases: `=-TRUE`→
   `-1`, `=--TRUE`→`1`, `=--FALSE`→`0`, `SUMPRODUCT(--(A="x"))`→count, `=--"3"`→`3`, `=-"x"`→
   `#VALUE!`, and a spot-check that no existing operator/precedence test regresses (broad blast
   radius — this is the main risk).
4. Ship as its own `fix/<slug>` branch off the fork's `main` with upstream-style tests, per the
   standard fork-upstreaming loop (`CLAUDE.md`; `specs/projects/ironcalc-upstreaming/`).

## References

- `specs/projects/scalar-functions-batch/functional_spec.md` §3.1 (SUMPRODUCT — the `--` idiom and
  the single-expression vs multiple-array coercion distinction).
- `specs/projects/scalar-functions-batch/fork-fixes/README.md` (divergence table — SUMPRODUCT row,
  deferred).
