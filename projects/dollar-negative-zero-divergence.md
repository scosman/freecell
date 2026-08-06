# DOLLAR Negative-Zero Divergence (accepted, upstream declined the fix)

**Status: Future** (accepted divergence from `scalar-functions-batch`, reverted 2026-07-28,
recorded here 2026-08-06).

## The divergence

Excel's `DOLLAR` wraps **negative** values in parentheses rather than using a minus sign:
`DOLLAR(-1234.567, 2)` → `($1,234.57)`. But a negative whose magnitude **rounds to zero** is not
a negative number once rounded, so Excel emits it unsigned:

```
DOLLAR(-0.001, 2)   → $0.00     (Excel)
DOLLAR(-50, -3)     → $0        (Excel)
```

IronCalc branches on the *pre-rounding* sign, so both return the parenthesized form:

```
DOLLAR(-0.001, 2)   → ($0.00)   (IronCalc, and therefore FreeCell)
```

Real negatives are unaffected in either engine.

## Why it isn't fixed

The guard was written, tested, and merged as `fix/dollar-negative-zero` (`aa36a177`), then
**backed out** of the fork's `freecell-fixes` branch (`8a79a7f6`, fork PR #2) after the IronCalc
team pushed back on it upstream. Per CLAUDE.md's standing rule we fix things in the engine rather
than compensating in FreeCell — and when upstream declines a fix, that rule cuts the other way
too: we do not re-add a FreeCell-side workaround behind their back. So FreeCell ships IronCalc's
behaviour.

The branch `fix/dollar-negative-zero` still exists on `scosman/IronCalc`, unmerged, if the
question is ever reopened with upstream.

## Blast radius

Narrow. It is one function, at one input class — a negative whose magnitude rounds away entirely
at the requested precision. Nothing else in the currency formatting path is affected, and the
value-level formatter (number formats applied to cells) is a different code path that does not
share this branch.

## What would close it

Reopening the conversation upstream with a compatibility argument (Excel + LibreOffice both emit
the unsigned form), and, if they agree, re-landing `fix/dollar-negative-zero` — at which point the
assertion in `freecell-engine`'s `scalar_functions_batch_computes_through_pinned_engine` flips
back to `"$0.00"`. That test is the tripwire: it pins the current behaviour explicitly, so a
future engine change here shows up as a deliberate, reviewed edit rather than a silent drift.

## Pointers

- Current per-branch state: `specs/projects/scalar-functions-batch/fork-fixes/README.md` row #7.
- The assertion: `app/crates/freecell-engine/src/document.rs`,
  `scalar_functions_batch_computes_through_pinned_engine`.
- The sibling accepted divergence from the same batch:
  [`projects/unary-minus-boolean-coercion.md`](unary-minus-boolean-coercion.md).
