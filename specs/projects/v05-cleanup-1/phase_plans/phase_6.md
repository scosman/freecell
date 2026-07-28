# Phase 6 — F3a: chart-axis / cell number-format and colour agreement

> **This record was rewritten on 2026-07-28 after code review.** The first version's headline was
> *"mostly DISPROVED as stated — every divergence is IronCalc's, not chart-model's."* **That
> conclusion was false.** It was produced by a test whose carve-outs were too broad and whose corpus
> was missing a whole format family; once both were tightened, three real `chart-model` defects fell
> out, and the two IronCalc defects turned out to be characterised wrongly as well. The original
> text is not preserved verbatim because it asserted things that are not true; what it got wrong is
> spelled out below, since that is the part worth keeping.

**Verdict: PARTLY CONFIRMED.** The review's *mechanism* ("a second formatter races IronCalc, so the
same code produces two strings") was right. Its *scale* was overstated — the divergence is bounded
by `renders_faithfully`, not unbounded — but the divergence was real on both sides, and the first
pass found only IronCalc's half of it.

| Sub-claim | Verdict |
|---|---|
| `chart-model`'s numfmt races IronCalc, so the same code produces two strings | **Confirmed, both directions.** Three `chart-model` defects (negative zero, `#` treated as required, whitespace trimmed) and two IronCalc defects (sign dropped, rounding corrupted). |
| `rgb_to_hsl` copies drifted (`%` vs `.rem_euclid`), disagreeing on negative hues | **Disproved.** Identical *by construction*, for every input. Duplication real — and now removed. |
| Two definitions of the Office palette | **Confirmed.** Real duplication, currently in agreement, now enforced. |

## What the first pass got wrong, and why

Three mechanisms, all of the same kind: **the test was written so that it could not fail on the
thing it was supposed to catch.**

1. **An over-broad carve-out.** `is_ironcalc_sign_bug` was "chart says `-X`, cell says `X`". That is
   true of the IronCalc sign bug — and equally true of `chart-model` printing `-0.00` where the cell
   correctly prints `0.00`. It suppressed **27** pairs, of which only **12** were IronCalc's. The
   other 15 were a `chart-model` bug, invisible because the predicate that was supposed to *name* a
   defect instead described a *shape* that two different defects share.
2. **A corpus with a hole in it.** `renders_faithfully` returns `true` for `0.##`, `#,##0.##`,
   `#,##0.0#`, `0.###`, `#,###` — none of which were in the corpus. So the gate certified as
   Faithful a family it had never once evaluated.
3. **A carve-out validated against a corpus that could not contradict it.**
   `is_ironcalc_half_up_outlier` tested `|v| == 0.5` and looked sufficient — because `VALUES`
   contained no other value in the band it was carving out.

The general lesson, worth more than the specific fixes: **a differential test's carve-outs and its
corpus have to be adversarial to each other.** A predicate that names a defect must be narrow enough
that it cannot also match the defect on the other side, and the corpus must contain values the
predicate does *not* explain. Neither was true here, and the result was a green gate over a real
bug plus a confidently wrong write-up.

## The number-format half

### Reframing, before testing

`chart-model/src/numfmt.rs` is not an unbounded reimplementation racing IronCalc. It is an
explicitly *bounded subset* with a companion predicate `renders_faithfully(code)`, and
`source_fidelity` degrades any chart whose codes fall outside it — so out-of-subset divergence is
**already disclosed to the user by the ⚠ badge**. The sharp, falsifiable invariant is therefore:

> For every code where `renders_faithfully(code)` is true, the chart's label must equal IronCalc's
> rendering — because the chart is *claiming* to be faithful and the user is comparing the axis
> against the cells.

That reframing still holds. What it does **not** license is treating a green gate as evidence the
subset is correct, when the subset's own predicate is what decides which pairs get asserted.

### The test

`app/crates/freecell-engine/tests/numfmt_agreement.rs` — in the engine, the only crate depending on
both; putting it in `chart-model` would break the ironcalc-free boundary the design rests on.

Corpus: every `formatCode` extracted from the real fixtures in-tree, plus the everyday codes
**including the `#` optional-digit family and a whitespace-padded code**, crossed with values chosen
for the places two implementations diverge — signs, rounding bands, magnitudes crossing grouping and
exponent thresholds. 462 in-subset (code, value) pairs.

Reference: `ironcalc_base::formatter::format::format_number(v, code, locale("en"))`. Verified to be
the *exact* call the cell path makes — `model.rs:2815` `get_formatted_cell_value`, whose
`format_number(value, &format, self.locale).text` call is at **`:2826`**.

### The three `chart-model` defects

**1. Negative zero.** `apply_number_format` chose the sign from `scaled < 0.0` *before*
`format_magnitude` rounded, so any negative that rounds to zero at the format's precision printed
`-0`, `-0.00`, `-$0.00`, `-0%`, `-0.00 kg`. Excel suppresses the sign there and so does IronCalc:
**IronCalc was right and `chart-model` was wrong** — the exact inverse of the story the first pass
told. Fixed by taking the sign from the *rendered magnitude* (`has_significant_digit`). `chart-model`
had **no unit test for a small negative at all**, which is why it survived; it has several now. The
same rule was applied to `format_number` (the General / fall-back path), which could also emit `-0`.

**2. `#` counted as a required digit.** `FormatSpec::parse` counted `0` and `#` identically, so
`#,##0.0#` on 1.5 padded to `"1.50"` while the cell read `"1.5"`, and `0.##` on 0.5 gave `"0.50"`
against `"0.5"`. A chart with `formatCode="0.##"` was therefore **Faithful, unbadged, and wrong on
every label**.

*Remedy chosen: **(a) fix `chart-model`.*** `functional_spec.md` §F3a and architecture §6 bless
either fixing the formatter or making `renders_faithfully` reject the code. (a) was chosen because
it turned out to be both contained and *provably* right rather than corpus-fitted:

- `FormatSpec` gained `min_decimals` (placeholders up to the last required `0`) alongside
  `decimals` (all placeholders); `format_magnitude` rounds at `decimals` and trims trailing zeros
  back to `min_decimals`. ~20 lines.
- Two adjacent behaviours had to come with it, and both are *general* rules, not patches for corpus
  entries: the decimal separator is a literal in the format string and survives when every
  fractional digit is trimmed (`0.##` on 1 → `1.`, which IronCalc's own source flags as deliberate
  Excel behaviour), and an all-`#` integer run suppresses a zero integer part (`#,###` on 0 → `""`).
- The second rule was **not** derived from the required corpus: `#,###` needs only the "empty when
  zero" half, but the same rule independently produces IronCalc's `.5` and `.` for `#.##` on 0.5 and
  0.001 — a code nobody asked for. A rule that predicts unrequested cases correctly is a rule, not a
  fit.

Remedy (b) — badging the family — was rejected because `0.##` and `#,##0.0#` are common Excel codes;
degrading them would put a permanent ⚠ on ordinary files to avoid a fix that fits in 20 lines and is
exactly matched against IronCalc across the corpus.

**3. Whitespace padding trimmed.** `apply_number_format` opened with `code.trim()`, so `"0 "`
rendered `"1"` where the cell renders `"1 "`. Only the General/empty test trims now.

### The two IronCalc defects — both re-derived, both previously mischaracterised

**E6 — the sign is dropped on small magnitudes.** Still the most severe finding here, and still a
fork fix (`CLAUDE.md` §Engine: one `fix/` branch, one upstream PR). But the *rule* was wrong. The
first pass recorded `|value| < 1.5 × 10^-decimals`; that expression is correct **only at
`decimals = 0`**. The mechanism is `is_negative = value < -(10^-precision)` (`format.rs:481`)
evaluated **after** the `to_precision` pre-round at `:437`, so the real threshold is
**≈`10^-decimals`** — exactly `10^-d + 5×10^-(2d+1)`. Measured by bisection: `0` → **1.5**,
`0.0` → **0.105**, `0.00` → **0.01005**, `0.000` → **0.0010005**, `0%` → **0.015**. The old reading
overstated the two-decimal band by a factor of ten; a fix written against it would have tested the
wrong boundary.

**E7 — the rounding story was simply wrong.** The first pass recorded "IronCalc is half-to-even
everywhere except 0.5, where it alone rounds up." IronCalc is **not half-to-even** and 0.5 is **not
the only affected value**. `format.rs` rounds in pieces:

1. pre-round: `to_precision(value, precision + integer_digits)` — significant digits, via `{:.*e}`,
   half-to-**even**;
2. render: `value_abs.round()` at `precision == 0` (half **away from zero**), else `floor()` plus a
   separately rounded fractional string.

Two corruptions, **both reachable on positives**:

- **Double rounding, `decimals == 0`, `|v| < 1`:** `floor(|v|)` prints `"0"`, so step 1 keeps *one*
  significant digit; the whole band `|v| ∈ [0.45, 0.5]` becomes `0.5` and then rounds away from zero
  to `"1"` (correct: `"0"`). `0.45`, `0.46`, `0.49` and `-0.46` all display as `1` under code `0`.
  `0.5` is merely the top endpoint of that band — and, ironically, the one value in it where
  IronCalc *agrees with Excel*.
- **Lost fractional carry, `decimals ≥ 1`, `|v| < 1`:** the integer part is `floor(|v|) = 0` while
  `get_fract_part` rounds the fraction separately and slices `"0.ddd"[2..]`; when the fraction
  rounds up to `1.0` the slice is empty and the carry is discarded. **`0.96` under `"0.0"` displays
  `"0.0"`.**

And for `|v| ≥ 1` the pre-round happens at the rendered precision, so IronCalc lands on
half-to-even (`2.5 → "2"`, `1234.5 → "1234"`) where **Excel is half-away-from-zero**. So the first
pass's other claim — "switching `chart-model` to half-away-from-zero made agreement strictly worse,
leave it" — is true *against IronCalc* and false *against Excel*. Both implementations are wrong
there, in the same direction. `chart-model` stays half-to-even **because it is pinned to the buggy
reference the user is comparing against**, and the code comment now says exactly that, plus: when
the fork fix lands, half-away-from-zero becomes correct and this must be revisited. The old comment
asserted the false premise and said "Leave it."

The carve-out is now `is_ironcalc_rounding_defect(code, value)` — it takes the **code**, because the
band depends on the decimal count and the percent scale (its predecessor's doc claimed to consider
the code and the signature did not), and it covers both mechanisms rather than one point of one.

### Carve-outs, in full — with counts

Three, each naming a characterised defect. `the_carve_outs_suppress_only_a_handful_of_pairs` prints
the counts on every run and fails if any predicate stops firing (fork fix landed → delete it) or
runs away past a quarter of the corpus.

| Carve-out | Before (old corpus, old predicates) | After (new corpus, new predicates) |
|---|---|---|
| `is_ironcalc_sign_bug` (E6) | **27** — of which **15 were `chart-model`'s own bug** | **18**, all genuine (**12** on the old corpus, i.e. exactly the real ones) |
| `is_ironcalc_rounding_defect` (E7, was `is_ironcalc_half_up_outlier`) | **10** | **27** — the band is wider than one point, and the corpus now reaches it |
| `is_empty_code_artifact` | **0 — dead**, `is_general("")` short-circuited first | **21** — the empty code now reaches the gate, so IronCalc's `#VALUE!` is *pinned* rather than asserted in prose |

Total in-subset pairs: 252 → **462**; agreeing pairs 215 → **396**.

`is_empty_code_artifact` had been documented as one of "three carve-outs" in the module docs, this
phase plan and the commit message while firing zero times. Fixed by pinning rather than deleting:
the gate now skips only `General`, not the empty code.

### `General`, and the one genuine chart-side gap

`chart-model`'s General is a *tick-label* formatter: integers bare, otherwise three decimals
trimmed. IronCalc's is Excel's — ~9 significant digits with a scientific fallback. So a data label
reads `0.333` where the cell reads `0.333333333`.

`renders_faithfully("General")` returns `true`, so by the module's own invariant this **should** be
asserted and is not. That is a hole, and the module docs now say so and argue it explicitly against
`functional_spec.md` §F3a's remedies rather than leaving it implicit: matching IronCalc means axis
ticks reading `0.333333333` (worse than the divergence); returning `false` badges every chart with a
General axis — the default — as Degraded. The honest fix is the spec's third outcome, "fix what is
reachable, file the rest": separating tick formatting from data-label formatting, which is
chart-project work.

`general_differs_from_the_cell_only_by_rounding` pins the shape, and now actually constrains it. The
old version reached its assertion for only 2 of 18 values and used a relative tolerance that would
have accepted `1233.3` for `1234.5`. It now asserts the exact three-decimal rounding, at most three
fractional digits with no trailing zero, and no signed zero.

## The colour half

### `rgb_to_hsl` — disproved, and deduplicated

The copies were in `freecell-app/src/chart/palette.rs` and `freecell-chart-model/src/theme.rs`, not
`core` as the review said. The textual difference was real: `((g - b) / d) % 6.0` vs
`.rem_euclid(6.0)`.

It changes nothing, and the reason is stronger than a sweep: `rgb_to_hsl` returns
`(h * 60.0).rem_euclid(360.0)`, so `h ∈ [0, 360)` **always**; `hsl_to_rgb`'s `hp = h / 60` is
therefore never negative, and `%` and `rem_euclid` coincide on non-negative dividends. The
equivalence holds **by construction, for every input** — the first write-up claimed verification
"over the entire reachable input space", which was an overstatement (the test covered laps 0–7).

Architecture §6 prescribed *export from `chart-model` and delete the app copy*. The first pass did
only the export, which left the tree with **more public API and still two copies**. The copy is now
deleted and the app imports the helpers; the equivalence test is replaced by
`series_color_is_pinned_for_the_first_three_laps`, which pins the actual per-index colours. That is
strictly stronger — the old test passed however both copies moved, as long as they moved together.
The pinned values are byte-identical to what the deleted copy produced, so **no pixel can move**.

### Office palette — confirmed

`freecell-core::palette::FILL_PALETTE` (10 swatches, the fill popover) and
`freecell-chart-model::ThemePalette::office_default()` (10 theme slots, `schemeClr` resolution) are
genuinely two definitions of the same Office colours, in two zero-dependency crates that do not know
each other exist. They agree; `tests/office_palette_agreement.rs` enforces it slot by slot,
including the `lt1`=Background 1 / `dk1`=Text 1 pairing. Merging them is F3.

The slot-coverage test was `assert_eq!(FILL_PALETTE.len(), EXPECTED.len())` — both sides the
compile-time constant `10`, so it was a tautology. It is now an exhaustive `match` over `ThemeSlot`:
adding a variant fails to **compile** until someone classifies it as a fill swatch or not.

## Verification

- `cargo test -p freecell-engine --test numfmt_agreement` — 5 passed
- `cargo test -p freecell-engine --test office_palette_agreement` — 2 passed
- `cargo test -p freecell-chart-model --lib` — 105 passed
- `cargo test -p freecell-app --lib chart::palette` — 4 passed
- `cargo fmt --all --check` clean

**No render impact, checked rather than assumed.** `apply_number_format`'s behaviour *did* change
this time, so the `chart_*` baselines' format codes were inspected: the render scenes use only
`"$#,##0"` and `"0%"` (`render-tests/src/chart_scene.rs`), on positive values. Neither code has an
optional-`#` fractional run or an all-`#` integer run, neither carries whitespace padding, and no
scene value is a small negative — so none of the three changed behaviours is reachable from a
baseline. Colour output is unchanged and pinned. The pixel subset was therefore not run, per the
round's working agreement.
