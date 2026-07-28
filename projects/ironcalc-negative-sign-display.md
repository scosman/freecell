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
