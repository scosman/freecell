# IronCalc drops the minus sign on small negative numbers

**Status: Filed, unfixed. Found 2026-07-28 by unit F3a's differential test
(`specs/projects/v05-cleanup-1`).** Needs a fork fix per `CLAUDE.md` §Engine — one
`fix/<slug>` branch, one clean upstream PR.

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

`ironcalc_base::formatter::format::format_number` returns an unsigned string when

> `|value| < 1.5 × 10^(-decimals)`

i.e. whenever the value rounds to a magnitude of 0 or 1 in the format's last digit position.

| Format | Sign dropped for | First value that keeps its sign |
|---|---|---|
| `#,##0`, `0` (0 dp) | `-0.0001` … `-1.49` | `-1.5` → `"-2"` |
| `0.00`, `$#,##0.00`, `0%` (2 dp) | `-0.0001` … `-0.01` | `-0.011` |
| `0.0000` (4 dp) | `-0.0001` | |
| `General` | never — a separate code path in IronCalc | |

So the everyday case is the worst one: **a zero-decimal integer format, the most common format in
a spreadsheet, shows `-1` as `1`.**

## Why it matters

This is silent numeric misinformation in the primary display surface. A budget cell holding −1
reads as 1; a variance column of small negatives reads as if every value were positive. There is
no badge, no warning, and no way for the user to tell — the underlying value is correct, so a
formula over the same cells gives the right answer while the screen does not.

## Not a FreeCell workaround

Per `CLAUDE.md` §Engine, the fix belongs in the fork and then upstream — **do not** add a
compensating sign patch in FreeCell's display path. FreeCell reads the formatted string straight
from `get_formatted_cell_value` (`model.rs:2815` → `format_number(value, &format, locale).text`),
which is exactly the function to fix.

Starting point: `base/src/formatter/format.rs`. For a **single-section** format, `format_number`
selects `parts[0]` and leaves `value` negative (the 2/3/4-section arms explicitly negate and pick a
different part). So the sign is lost further down, in the digit-rendering path, once the rounded
magnitude falls below the format's precision — most likely a rounded-to-zero integer part being
rendered without consulting the original sign.

## Related, same function

`format_number` also rounds **0.5 at zero decimals** to `"1"` while rounding `2.5 → "2"`,
`4.5 → "4"`, `10.5 → "10"` and `1234.5 → "1234"` — half-to-even everywhere except that one value.
Excel rounds half away from zero throughout, so IronCalc is inconsistent with both Excel and
itself. Lower severity than the sign bug; worth folding into the same investigation but **its own
`fix/` branch and PR** if it turns out to be an independent defect.

## Guard already in place

`app/crates/freecell-engine/tests/numfmt_agreement.rs` carves both behaviours out explicitly, with
predicates named `is_ironcalc_sign_bug` and `is_ironcalc_half_up_outlier`. When the fork carries
the fix, **delete the carve-out** — the differential gate then tightens by itself and will fail if
the bug ever returns.
