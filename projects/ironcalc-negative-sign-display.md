# IronCalc drops the minus sign on small negative numbers

**Status: Filed, unfixed — targeted at v0.5.** Found 2026-07-28 by unit F3a's differential test
(`specs/projects/v05-cleanup-1`). Needs a fork fix per `CLAUDE.md` §Engine — one `fix/<slug>`
branch, one clean upstream PR.

Tracked in `GAPS.md` as **E6** (and **E7** for the rounding outlier) under
*Engine (fork) — negative numbers display without their minus sign*, and listed in the
**v0.5 release tier**.

## The bug

A cell displays a **negative number without its minus sign** whenever the magnitude is small
relative to the number format's decimal places.

Reproduced end-to-end through the shipped app path — worker, `SetStylePath(NumFmt)`, publication —
not just against the formatter in isolation:

```
number format "#,##0", values [-1, 0.5, -0.5, -1234.5, 1]
displayed:                    ["1", "1",  "1",  "-1,234", "1"]
                                ^^^        ^^^
                       -1 displays as 1.   -0.5 displays as 1.
```

## The rule

> **Corrected 2026-07-28.** This document first stated the rule as `|value| < 1.5 × 10^(-decimals)`.
> **That is wrong**, and it is wrong in the direction that matters: it holds only at
> `decimals = 0`. A fork fix written against it would be written against the wrong band. The
> corrected rule and the measured thresholds are below.

`ironcalc_base::formatter::format::format_number` decides the sign at `format.rs:481`:

```rust
let is_negative = value < -(10.0_f64.powf(-(p.precision as f64)));
```

The subtlety is *when*: `value` was already overwritten a few lines earlier (`format.rs:437`) by

```rust
value = to_precision(value, (p.precision as usize) + format!("{}", value.abs().floor()).len());
```

so the test is not "is the value negative" but **"is the value, after pre-rounding to
`decimals + integer_digits` significant digits, still below `-10^-decimals`"**. The sign is dropped
for everything at or inside that bound. The threshold is therefore **≈`10^-decimals`** — exactly
`10^-d + 5×10^-(2d+1)`, the extra term being the pre-round's half-step at the `(d+1)`-th significant
digit.

Measured by bisecting the largest magnitude that still loses its sign:

| Format | Decimals | Sign dropped for `|v|` up to | First value that keeps its sign |
|---|---|---|---|
| `#,##0`, `0` | 0 | **1.5** | `-1.5` → `"-2"` |
| `0.0` | 1 | **0.105** | `-0.106` → `"-0.1"` |
| `0.00`, `$#,##0.00` | 2 | **0.01005** | `-0.0101` → `"-0.01"` |
| `0.000` | 3 | **0.0010005** | `-0.001001` |
| `0%` | 0, ×100 | **0.015** | the percent scale moves the band, not the rule |
| `General` | — | never — a separate code path in IronCalc | |

So the everyday case is the worst one: **a zero-decimal integer format, the most common format in
a spreadsheet, shows `-1` as `1`.** Note also that the old `1.5 ×` reading made the two-decimal band
look ten times wider than it is (`0.01` vs the real `0.01005`), which is the sort of error that
makes a fix's test cases miss the boundary.

## Why it matters

This is silent numeric misinformation in the primary display surface. A budget cell holding −1
reads as 1; a variance column of small negatives reads as if every value were positive. There is
no badge, no warning, and no way for the user to tell — the underlying value is correct, so a
formula over the same cells gives the right answer while the screen does not.

## Not a FreeCell workaround

Per `CLAUDE.md` §Engine, the fix belongs in the fork and then upstream — **do not** add a
compensating sign patch in FreeCell's display path. FreeCell reads the formatted string straight
from `get_formatted_cell_value` (`model.rs:2815`, whose `format_number(value, &format, self.locale).text` call is at `:2826`),
which is exactly the function to fix.

Starting point: `base/src/formatter/format.rs`, `ParsePart::Number` arm. For a **single-section**
format, `format_number` selects `parts[0]` and leaves `value` negative (the 2/3/4-section arms
explicitly negate and pick a different part). The sign is then decided by the `is_negative` line at
`:481` — a magnitude comparison against `10^-precision`, evaluated on the value *after* the
`to_precision` pre-round at `:437`. Rendering the sign from a **magnitude threshold** rather than
from `value_original.is_sign_negative()` (plus "did the rendered digits come out non-zero") is the
defect; the threshold is why -1 under `#,##0` loses its sign.

## This is NOT the `fix/dollar-negative-zero` case you just reverted

Worth stating explicitly, because they look adjacent and are not the same thing.

`fix/dollar-negative-zero` (reverted on `freecell-fixes` by PR #2, 2026-07) changed the **`DOLLAR`
worksheet function** in `base/src/functions/text/string_format.rs` so a value whose magnitude
*rounds to zero* printed `$0.00` instead of `($0.00)`. That is a genuine judgement call about
Excel's parenthesised-negative convention at the rounds-to-zero boundary, and the revert is a
defensible position.

This finding is in a **different function** (`base/src/formatter/format.rs::format_number`), on a
**different surface** (every cell's displayed value, not one worksheet function), and its worst case
is **not** a rounds-to-zero case at all:

> `#,##0` on the value **−1** displays **`1`**.

−1 does not round to zero at zero decimals. It rounds to −1, and the sign is dropped anyway — right
through to `−1.49`. No convention explains that; it is simply the wrong number on screen. Please
don't triage it as a repeat of the DOLLAR decision.

## Related, same function — and also mischaracterised at first (GAPS E7)

> **Corrected 2026-07-28.** This section originally said `format_number` is "half-to-even everywhere
> except 0.5". **It is not half-to-even, and 0.5 is not the only affected value.** The corrected
> account is below; the old one would have sent a fix at a single data point.

`format_number` does not implement one rounding rule. It rounds in pieces:

1. **pre-round**, `format.rs:437`: `to_precision(value, precision + integer_digits)` — that many
   *significant* digits, through Rust's `{:.*e}`, which is half-to-**even**;
2. **render**, `format.rs:459-468`: `value_abs.round()` when `precision == 0` (half **away from
   zero**), otherwise `value_abs.floor()` plus a *separately rounded* fractional string from
   `get_fract_part`.

Two corruptions fall out, **both reachable on positive values**:

- **Double rounding — `decimals == 0`, `|v| < 1`.** `floor(|v|)` prints as `"0"`, so step 1 keeps
  only **one** significant digit. Every value in `|v| ∈ [0.45, 0.5]` becomes `0.5`, which step 2
  then rounds away from zero to `"1"`. Correct answer: `"0"`.
  Measured: `0.45 → "1"`, `0.46 → "1"`, `0.49 → "1"`, `-0.46 → "1"` (the sign additionally lost to
  the E6 bug above). `0.5 → "1"` — the value the old write-up singled out — is just the top endpoint
  of this band, and is in fact the one value in it where IronCalc's answer *matches Excel*.
- **Lost fractional carry — `decimals ≥ 1`, `|v| < 1`.** The integer part is `floor(|v|) = 0` while
  `get_fract_part` rounds the fraction on its own and slices the result as `"0.ddd"[2..]`. When the
  fraction rounds up to `1.0` the slice is empty and the carry is thrown away:
  **`0.96` under `"0.0"` displays `"0.0"`**, and `-0.96` displays `"-0.0"`.

Separately, for `|v| ≥ 1` the pre-round happens at exactly the rendered precision, so IronCalc lands
on half-to-**even** (`2.5 → "2"`, `4.5 → "4"`, `1234.5 → "1234"`) where **Excel is
half-away-from-zero** (`3`, `5`, `1235`). Wrong, but consistently so — and it is the reason
`chart-model`'s formatter deliberately uses half-to-even too, so the axis label matches the cell
beside it. **A fix must change that as well**, and when it lands, `chart-model`'s
`numfmt::format_magnitude` should switch to half-away-from-zero in the same breath.

Lower severity than the sign bug; worth folding into the same investigation but **its own `fix/`
branch and PR** if it turns out to be an independent defect. (Both live in the same
`ParsePart::Number` arm of `format.rs`, so independence is not obvious.)

## Guard already in place

`app/crates/freecell-engine/tests/numfmt_agreement.rs` carves both behaviours out explicitly, with
predicates named `is_ironcalc_sign_bug` and `is_ironcalc_rounding_defect` (the latter renamed from
`is_ironcalc_half_up_outlier`, which tested `|v| == 0.5` and covered one point of one of the two
bands). The corpus now contains values inside both bands (`0.45`, `-0.46`, `0.96`) so the defect is
exercised, not merely described. When the fork carries the fix, **delete the carve-out** — the
differential gate then tightens by itself and will fail if the bug ever returns.

## One thing the carve-out was hiding

The sign carve-out was originally written as "chart says `-X`, cell says `X`". That also matched 15
pairs where **`chart-model` was the buggy side**: it emitted `-0`, `-0.00`, `-$0.00`, `-0%` for
negatives that round to zero at the format's precision, where Excel and IronCalc both print
unsigned. That was a FreeCell defect, not an IronCalc one; it is fixed (`numfmt.rs` now takes the
sign from the *rendered magnitude*), and the carve-out now additionally requires the unsigned
rendering to contain a non-zero digit so it cannot hide that class again.
