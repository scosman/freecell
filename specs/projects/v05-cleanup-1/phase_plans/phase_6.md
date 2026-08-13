# Phase 6 — F3a: chart-axis / cell number-format and colour agreement

> **This record was rewritten on 2026-07-28 after code review, and again after a second, third and
> fourth review round.** Each round's findings are in its own section at the end; where a later
> section contradicts the text above, the later section wins and says so. Round 4 retracts a claim
> made in Round 3 (that the E7 carve-out could not hide a chart-side presentation bug) and corrects
> Round 3's headline (which was true of the *generated* space, not of the predicate's *accepted*
> space) — both inline, at the sentences concerned.
> The first version's headline was
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

**Round 3 replaced the hand-written corpus with a generated one** — see the Round 3 section. The
counts in this section are the round-2 numbers and are kept as the record of that round.

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
it turned out to be contained, and matched across the corpus plus probed neighbours — **not**
"provably right", as this line claimed until round 3 probed 28 `#`-family codes outside the corpus
and found three residuals (see Round 3):

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
evaluated **after** the `to_precision` pre-round at `:438` (`:437` is the comment line above it),
so the real threshold is
**≈`10^-decimals`** — exactly `10^-d + 5×10^-(2d+1)`. Measured by bisection: `0` → **1.5**,
`0.0` → **0.105**, `0.00` → **0.01005**, `0.000` → **0.0010005**, `0%` → **0.015**. The old reading
overstated the two-decimal band by a factor of ten; a fix written against it would have tested the
wrong boundary.

**E7 — the rounding story was simply wrong.** The first pass recorded "IronCalc is half-to-even
everywhere except 0.5, where it alone rounds up." IronCalc is **not half-to-even** and 0.5 is **not
the only affected value**. `format.rs` rounds in pieces:

1. pre-round (`format.rs:438`; `:437` is the comment above it):
   `to_precision(value, precision + integer_digits)` — significant digits, via `{:.*e}`,
   half-to-**even**;
2. render: `value_abs.round()` at `precision == 0` (half **away from zero**), else `floor()` plus a
   separately rounded fractional string.

**Round 3 correction: this "two corruptions" reading is still narrower than the mechanism.** The
mechanism is one thing — for `|v| < 1` the integer part is `"0"`, so the pre-round keeps
`decimals + 1` **significant** digits and the render rounds *again* to `decimals` decimals: a plain
double round, corrupting anything that ties or crosses a tie at the intermediate precision. For
`|v| >= 1` nothing is corrupted. The two items below are sub-cases of that, and 886 corpus pairs
matched neither of them. See Round 3.

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
adding a variant fails to **compile** until someone classifies it as a fill swatch or not. The list
the match is swept over was still hand-written *in the test* until round 3, which made the round-trip
half exhaustive only over what someone remembered to type there; it is `ThemeSlot::ALL`, owned by
`chart-model`, now.

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

## Round 3 — the corpus now derives from the predicate

A third review found **two more Criticals of the same class** as round 2's, plus unresolved items.
The class is what matters: each round, `renders_faithfully` accepted a format shape that
`chart-model` rendered wrong, unbadged, and the corpus did not contain it. Round 1: `#` optional
digits. Round 2: nothing new in the corpus, but the sign carve-out was hiding a chart-side bug.
Round 3: a quoted or `\`-escaped `%`, and required leading zeros.

### The root fix: generate the corpus from the predicate

The invariant quantifies over *every* code `renders_faithfully` accepts, so a hand-written list can
only sample it — and "add the family we just found" had been the remedy three times running.

`generated_shape_space_agrees_with_the_cell` now enumerates the shape space directly: 13 integer runs
× 10 fractional runs × 6 percent forms × 6 affix forms × 2 paddings = **9 360 codes**, of which
**7 480** are accepted by the predicate, crossed with 30 values covering sign, zero, ties, and the
grouping and `|v| < 1` boundaries — **224 400 asserted pairs**, against 960 for the grounded corpus.
The hand-written `FIXTURE_CODES`/`COMMON_CODES` corpus stays as the readable, real-file half; it is
no longer the boundary of what is checked.

Widening the predicate now widens what must agree. That is the property the phase's own annotation
asked for and did not have.

### Every family the generated corpus surfaced, and the choice made

| Family | What the cell showed | Choice | Where |
|---|---|---|---|
| **Literal `%`** — `0" %"`, `0\%`, `0.00"%"` | `12 %` / `1%` / `0.25%`; the chart scaled by 100 (`1250 %`) | **Fix `chart-model`.** The percent *scale* is counted on the control body (quotes, brackets, `\`-escapes stripped) — the same stripping `control_body_is_unrenderable` already did. One stripper, both callers. | `numfmt.rs` |
| **`%` position / count** — `0% `, `%0`, `0%%` | `100% ` / `%100` / `10000%%`; the chart appended one `%` at the very end | **Fix `chart-model`.** A `%` is a literal rendered *where it stands* that also multiplies by 100 per occurrence — which is literally what IronCalc's `Token::Percent` does (push `Literal('%')`, `percent += 1`). | `numfmt.rs` |
| **Required leading zeros** — `000`, `0000`, `00.0`, `#,#00.0#` | `007` / `0042` / `01.0` / `01.0`; the chart dropped the padding | **Fix `chart-model`.** `min_integer_digits` (count of `0` in the integer run) left-pads the integer part — symmetric with the fractional side. | `numfmt.rs` |
| **Mixed fractional runs** — `0.#0`, `#.0#0` | `1.0` / `1.00`; the chart padded to `1.00` / `1.000` | **Fix `chart-model`.** The single `min_decimals` count took the *last* `0`. Replaced by the per-placeholder kinds: drop the rounded fraction's trailing zeros, then re-add one `0` per **required** placeholder past what survived — IronCalc's `get_fract_part` + per-token padding, and Excel's rule. | `numfmt.rs` |
| **Grouping with ≥5 integer digit tokens** — `##,##0`, `##,##0.###` | `1234.5` (no separator!) where Excel and the chart give `1,234.5` | **Narrow the predicate** (badge). IronCalc positions the separator from the *token* index, not the digit index, so it drops it whenever `3 < ln < digit_count`. Filed as `GAPS.md` **E8**. `chart-model` is correct and must not be "fixed" to match. | `numfmt.rs` + `GAPS.md` E8 |
| **Grouping + ≥4 required zeros** — `0,000` | `0005` where Excel and the chart give `0,005` | **Narrow the predicate** (badge). Same E8 defect, second half: IronCalc never groups the zeros it padded in. | `numfmt.rs` + `GAPS.md` E8 |
| **Doubled quote** — `0"a""b"`, `#.0"%"" kg"` | `a"b` — IronCalc's `consume_string` reads `""` as an escaped quote; the chart reads two adjacent literals | **Narrow the predicate** (badge). The grammar is genuinely ambiguous and ECMA-376 does not settle it. Filed as `GAPS.md` **E9**, to be raised upstream as a question before a fix. | `numfmt.rs` + `GAPS.md` E9 |
| **Literal inside the digit run** — `0 0`, `%` between placeholders | rendered in place by the cell; the applier only has a prefix and a suffix | **Narrow the predicate** (badge). The numeric run must be `0`/`#`/`,`/`.` only. | `numfmt.rs` |
| **Positional required digits** — `0#0`, `00#` | required digits are per-token, not a minimum width | **Narrow the predicate** (badge). The integer run must be `#`…`0`…, the shape a minimum width can express. | `numfmt.rs` |
| **No integer placeholder** — `.00` | the cell hangs the minus sign off the first *integer* digit token, so `-0.5` shows `.50` unsigned | **Narrow the predicate** (badge). | `numfmt.rs` |
| **Bracketed currency out of head position / multi-char** — `0[$€-407]`, `[$USD-409]#,##0` | IronCalc hoists the currency to the front of the string, and its lexer reads exactly one symbol char (rejecting `USD` outright) | **Narrow the predicate** (badge). | `numfmt.rs` |

After the fixes and the narrowings: **0 unexplained disagreements** over the whole generated space
(and 0 over the grounded corpus), with only the E6 and E7 carve-outs firing.

> **Round-4 correction to the sentence above.** "The whole generated space" is **not** "the
> predicate's accepted space", and the invariant quantifies over the second. The round-3 generator's
> affix axis was `{"", "$", "\$", "\" kg\"", "[$€-407]", "\"USD \""}` — no placeholder character, no
> non-`$` bracket token, no bare literal, no doubled separator — so an entire class of
> accepted-and-divergent codes was unreachable *by construction*, and round 4 found seven divergent
> shapes inside it (607 of 767 sampled codes disagreed). This is round 2's "corpus with a hole in it"
> one level up: the hole moved from the list of codes to the list of axes. **Residual scope, stated
> rather than totalled:** the corpus is now a sample over more axes, and a sample over axes is a
> sampling strategy, not a proof. What can be said is that every disagreement reachable from
> *these* axes is resolved; what cannot be said is that no accepted code disagrees. Three rounds of
> confident totals were retracted; this one is not offered.

### The `#` claim, corrected

Round 2 called the `#` remedy "provably right rather than corpus-fitted". Probing 28 `#`-family codes
outside the corpus found three residuals, all now resolved: `0.#0` and `#.0#0` were **`chart-model`
bugs** (fixed above, the `min_decimals` → per-placeholder-kinds change), and `##,##0.###` is
**IronCalc's** grouping defect (E8, badged). The claim is now: *matched across the corpus plus the
generated shape space, with the residuals resolved as fixes or badges.*

### E7, characterised properly

Round 2's write-up named two bands. Over 21 corpus codes × 14 001 values, **886 pairs disagreed and
matched neither**, and a decimal-exact reference showed **chart-only-wrong = 0** for them — so they
were IronCalc's, and the predicate was incomplete, not the implementation.

The mechanism, stated once: **for `|v| < 1` the integer part is `"0"`, so the pre-round
`to_precision(value, precision + 1)` keeps `decimals + 1` significant digits and the render rounds
again to `decimals` decimals — a plain double round; anything that ties or crosses a tie at the
intermediate precision is corrupted. For `|v| >= 1` nothing is.** The `[0.45, 0.5]` band and the
lost-carry case are sub-cases.

`is_ironcalc_rounding_defect` is derived from that: it reconstructs the sub-unit pipeline and fires
only when the reconstruction reproduces the cell **byte for byte**. Because the reconstruction is
rendered through `chart-model` itself, it can only match when `chart-model`'s presentation is already
identical to IronCalc's and the rounding is the sole difference — so a chart-side presentation bug
makes the carve-out *stop* firing and the gate fail. It cannot fire at all for `|v| >= 1`.

> **RETRACTED in round 4.** The clause "a chart-side presentation bug makes the carve-out stop firing
> and the gate fail" was asserted here, in `numfmt_agreement.rs` twice and in `GAPS.md` E7, and it is
> **false for sign suppression** — the one presentation bug the carve-out was written not to hide.
> Measured, not argued: re-introducing round 2's exact defect (`if scaled < 0.0` without
> `has_significant_digit`) on the round-3 tree left **all six tests green** and moved the census
> `211156 agree / 8008 E7` → `204732 / 14432`, i.e. 6 424 pairs absorbed, comfortably under the
> `total/4` runaway guard. Mechanism: the reconstruction renders the *positive* predicted magnitude
> and the negative fallback renders `-predicted`, which for the whole lost-carry class is `-0.0` —
> and `-0.0 < 0.0` is `false`, so the defective branch is never reached. The claim is now **enforced
> instead of asserted**; see Round 4 below.
Representative values (`0.855`, `0.0495`, `-0.8745`, `-0.98765`) are in `VALUES` so the predicate is
forced to stay honest, alongside the E6 thresholds (`1.5`, `0.105`, `0.01005`, `-0.0101`).

### The other round-3 items

- **The test's own E6 numbers** still said `|value| <= 10^-decimals` with `0` → ~1.05 and
  `0.00` → ~0.0101 — self-contradictory at `d = 0` — while the other four documents had been
  corrected. The doc comment now carries the measured thresholds (`0` → **1.5**, `0.0` → **0.105**,
  `0.00` → **0.01005**, `0.000` → **0.0010005**, `0%` → **0.015**; closed form
  `10^-d + 5×10^-(2d+1)`), and those values are in `VALUES`.
- **`general_differs_from_the_cell_only_by_rounding`** reached its assertions for 3 of 21 values.
  `0.1234567` and `-0.98765` were added, and the test now **asserts its own reach** (`>= 5`), so it
  cannot silently go back to constraining nothing.
- **`office_palette_agreement.rs`'s `all_slots`** was a hand-written list in the test. It is
  `ThemeSlot::ALL`, a const on the type in `chart-model`, with a duplicate check.
- **`format.rs:437`** is the comment line; the `to_precision` call is at **`:438`**. Corrected in
  `GAPS.md`, this file and `projects/ironcalc-negative-sign-display.md`.

### Carve-out counts after round 3

| Carve-out | Grounded corpus (960 pairs) | Generated space (224 400 pairs) |
|---|---|---|
| `is_ironcalc_sign_bug` (E6) | **48** | **5 236** |
| `is_ironcalc_rounding_defect` (E7) | **61** | **8 008** |
| `is_empty_code_artifact` | **30** | 0 (the generator emits no empty code) |
| agreeing | 821 | 211 156 |

`the_carve_outs_suppress_only_a_handful_of_pairs` now runs over **both** corpora and still fails if a
predicate stops firing or runs past a quarter of the pairs; the empty-code pin is asserted directly
so it cannot go dead again.

### Verification (round 3)

- `cargo test -p freecell-engine --test numfmt_agreement --test office_palette_agreement` — 6 + 2 passed
- `cargo test -p freecell-chart-model --lib` — 109 passed
- `cargo test -p freecell-app --lib chart::palette` — 4 passed
- `cargo clippy --locked -p freecell-engine -p freecell-chart-model --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean

**Render impact, checked rather than assumed.** `apply_number_format` changed again, so the
`chart_*` baselines' format codes were re-inspected: `render-tests/src/chart_scene.rs` uses only
`"$#,##0"` and `"0%"`, on positive values. `"$#,##0"` has no fractional run, no required leading zero
beyond the single trailing `0`, four integer digit tokens (so the E8 narrowing does not touch it) and
no literal `%`; `"0%"` has a single trailing percent control, which renders in the same position
before and after the change. Neither reaches any changed behaviour, so no baseline can move and the
pixel subset was not run.

## Round 4 — the two functions now read the code the same way, and the carve-out is enforced

A fourth review found three Criticals, three Moderates and a Mild. **Every one of them reproduced
exactly as reported** — nothing in this round is a disproof, which is itself worth recording after
three rounds in which a confident claim was retracted each time. What changed structurally is that
two of the findings were *about the previous round's fixes* rather than about new format families:
the round-3 corpus could not reach the space it claimed to cover, and the round-3 carve-out's safety
argument was false.

### Critical 1 — a placeholder inside a literal (`\#0`, `"Item #"0`) — **CONFIRMED, fixed**

`renders_faithfully` located the numeric run on `control_body(code)` (quotes, brackets and
`\`-escapes **stripped**) while `FormatSpec::parse` located it on `cleaned`, which **retained** them.
So a `0`/`#`/`,`/`.` inside a quoted literal or behind a `\` was invisible to the predicate and
consumed as a digit placeholder by the applier. Round 3 had made the two agree about `%` only.

Reproduced verbatim, all with `renders_faithfully == true` and therefore no ⚠ badge:

| code | value | chart (before) | cell |
|---|---|---|---|
| `0" of 100"` | 7 | `007` | `7 of 100` |
| `\#0` | 7 | `7` | `#7` |
| `"Item #"0` | 7 | `Item 7` | `Item #7` |
| `#,##0" #"` | 1234.5 | `1,234` | `1,234 #` |
| `0" (0)"` | 7 | `07)` | `7 (0)` |
| `0"0"` | 1 | `01` | `10` |
| `0.0%" #"` | 0 | `0.0` | `0.0% #` |

**Fix: one reading of the code, shared.** `numfmt.rs` gained `lex()` → `Lexed`, which splits a format
code into `Tok { ch, control }` — control characters, literal characters (from `"…"`, `\x`, and a
`[$…]` symbol) and the bracket tokens. `control_body`, the placeholder positions, the numeric run,
the percent scale, the prefix and the suffix all now come from that one structure, so the predicate
and the applier agree about **every** character rather than about `%`. Literal text is emitted as
literal text: `\#0` on 7 is `#7`, `0"0"` on 1 is `10`.

The general shape of the bug is worth more than the fix: **two functions deriving the same positions
from two different strings**. That is the third time this module has produced a defect of exactly
that form (`%` in a literal was the second). There is one stripper and one position source now.

### Critical 2 — the corpus did not cover the predicate's accepted space — **CONFIRMED, widened**

`AFFIX_FORMS` was `{"", "$", "\$", "\" kg\"", "[$€-407]", "\"USD \""}` — no `0`/`#`/`,`/`.`, no
non-`$` bracket token, no bare literal, no doubled separator — so Critical 1's whole class was
unreachable *by construction*. The round-3 headline was true of the generated space and false of the
accepted space. The Round-3 section above now carries that correction inline.

**Widened by axis, not by code.** Three new axes plus one extension:

| Axis | What it sweeps | What it caught |
|---|---|---|
| `LITERAL_AFFIXES` (14) | quoted / escaped literals containing `0`, `#`, `,`, `.` — `"Item #"`, `" of 100"`, `\#`, `"0"` … | Critical 1's whole class |
| `BRACKET_TOKENS` (20) | colours in both cases, `[Color N]` in both spellings, currency, elapsed time, `[DBNum1]`, conditions, `[t]` | `[h]0`, `[DBNum1]#,##0`, `[color 5]0.00` → cell `#VALUE!`, chart a number |
| `BARE_LITERALS` (31) | every character in IronCalc's `Token::Literal` list, plus ones outside it (`°£¥µ×¤`, tab, letters) | `0°`, `0£`, `0¥`, `0µ`, `0×`, `0¤`, `0\t` → cell `#VALUE!`; and `-0` / `0-` / `$-0`, the sign-bearing literals |
| `INTEGER_RUNS` (+6) | non-grouping comma placements — `#,,##0`, `0,,0`, `#,##,##0` | `#,,##0` on 1 → chart `1`, cell `0` (IronCalc scales by 1000 per non-grouping comma) |

Each family placed in **both** positions (before and after the digits), because position is what
several of the divergences turn on. Corpus: 9 360 → **14 850** codes, **224 400 → 265 860** asserted
pairs.

Resolutions, one per family — and note that on **every** round-4 family chart-model is the correct
side, so all four are predicate narrowings (badges), not chart changes:

- **Bracket tokens.** `renders_faithfully` now accepts only a *head-position*, single-symbol `[$…]`
  currency or one of the seven plain colour names `formatter/lexer.rs::consume_color` accepts.
  Conditions, elapsed time, `[DBNum1]`, `[Color N]` (whose lexer arm is case-sensitive, so
  `[color 5]` is a hard error) are all rejected.
- **Bare literals.** A *control* character must now be one of `0 # , . %` or a character in
  IronCalc's explicit `Token::Literal` list. `/` and `:` are held out of that list deliberately —
  `/` is the fraction separator the parser already rejects, `:` flips IronCalc's section to a time
  format.
- **Repeated separators.** A comma is grouping only **between two digit placeholders**, which is
  IronCalc's own rule (`use_thousands = last_token_is_digit && next_token_is_digit`) and Excel's;
  anywhere else it is another ÷1000 scale the applier ignores.
- **Sign-bearing literals.** Not a narrowing — these are genuine E6 pairs, and the Mild finding below
  is what let the carve-out see them.

One more shape fell out of the widened corpus that the reviewer had not listed: **`#-` on +0.5**,
where the chart renders `-` (all-`#` integer run suppressed, plus the code's own literal dash) and
the cell renders `1-`. It is E7 (half-to-even vs `.round()` at `|v| < 1`), and it is the reason the
sign gate below is *measured* rather than sniffed from a leading `-`.

### Critical 3 — "a presentation bug makes the carve-out stop firing" was FALSE — **CONFIRMED, enforced**

Verified on the pristine round-3 tree (`git stash` of all round-4 work), re-introducing the round-2
defect verbatim — `if scaled < 0.0` with `has_significant_digit` removed:

```
all 6 tests PASSED
grounded  corpus: 774 agree, 48 E6, 108 E7, 30 empty   (was 821 / 48 / 61 / 30)
generated space:  204732 agree, 5236 E6, 14432 E7      (was 211156 / 5236 / 8008)
```

Byte-identical to the reviewer's numbers: 6 424 pairs of a re-introduced chart-model bug silently
absorbed, well under the `total/4` runaway guard. Mechanism as reported — the reconstruction renders
`apply_number_format(code, predicted)` with the *positive* magnitude and its negative fallback
renders `-predicted`, which for the entire lost-carry class is `-0.0`; `-0.0 < 0.0` is `false`, so
the defective branch is never exercised.

**Fix: a gate, not prose.** `is_ironcalc_rounding_defect` may not fire unless the chart and the cell
made the **same sign decision**. Both are *measured* by re-rendering the same magnitude
(`chart_is_signed` / `cell_is_signed`) rather than by testing `starts_with('-')` — the `#-` case
above is exactly why: a leading dash can be the code's own literal, and a `starts_with` gate read it
as a sign decision on a positive number and turned a legitimate E7 pair into a hard failure.

**Proof, run both ways:**

| tree | mutation | result |
|---|---|---|
| round-3 (pristine `a1f8577`) | round-2 sign defect | **all 6 green** — the hole, confirmed |
| round-4 (this work) | round-2 sign defect | **RED** — `chart_and_cell_agree_on_every_faithfully_rendered_code` *and* `generated_shape_space_agrees_with_the_cell` both fail, on `#,##0` −0.5 → chart `-0` / cell `1`, `"$"#,##0.00` −0.001 → chart `-$0.00` / cell `$0.00`, … |
| round-4 | none | all 6 green |

The four places asserting the false claim (`numfmt_agreement.rs` ×2, `phase_6.md` Round 3,
`GAPS.md` E7) are corrected in place, each saying what was measured rather than just dropping the
sentence.

### Moderate — the degeneracy guard measured the wrong quantity — **CONFIRMED, fixed**

`assert!(corpus.len() > 5_000)` counted the *unfiltered* generator output (a constant 9 360), which
no change to `renders_faithfully` can move: narrowing the predicate to accept nothing would have left
it green while the gate asserted zero pairs. `assert_agreement` is now `#[must_use]` and returns the
**asserted-pair** count; the generated gate requires `> 100_000` (265 860 today) and the grounded one
`> 500` (960). The failure message names the quantity it actually checks.

### Moderate — `ThemeSlot::ALL` exhaustiveness — **CONFIRMED, strengthened; residual stated**

`ThemeSlot::index` is a new wildcard-free `const fn` on the type in `chart-model`, and a
`const _: () = { … }` block asserts at **compile time** that `ALL` is dense and in index order.
`office_palette_agreement.rs` adds the runtime round-trip (`ALL[s.index()] == s`, `s.index() == i`)
and pins `ALL.len() == max_index + 1`.

Verified by probe, both directions:

- adding a 13th variant (`ThemeSlot::ProbeThirteenth`): `error[E0004]: non-exhaustive patterns` —
  `chart-model` itself stops compiling (two sites), and so does the test's `is_ui_fill_swatch`;
- swapping `ALL[0]` and `ALL[1]`: `error[E0080]: evaluation panicked: assertion failed:
  ThemeSlot::ALL[i].index() == i` — the const block, at compile time.

**Residual, stated rather than claimed away.** This proves `ALL` is duplicate-free, dense and
ordered, and that no variant can be added without an author editing `lib.rs`. It does **not** prove
`ALL` is exhaustive: an author who adds an `index` arm and forgets `ALL` still gets a green build,
because `ALL.len() == 1 + max index over ALL` is satisfied by the shortened array. Rust cannot
enumerate a foreign enum without `variant_count` (unstable) or a macro that defines the enum itself,
and `ThemeSlot`'s definition lives in `theme.rs`, outside this unit's file scope. The compile break
is the trip-wire; the density check is the consistency behind it; the gap is documented at both the
`index` definition and the test.

### Moderate — record what E8 does not confirm — **CONFIRMED, recorded**

`GAPS.md` E8 now separates the two halves. The *token-index* half is independently verified
(`###,##0` on 12345 → cell `1,2345` vs chart `12,345`; `00,000` on 1234 → cell `0,1234` vs chart
`01,234`; every accepted grouping code agrees at every corpus value; every mis-grouping code is
rejected). The *padded-zeros* half — `0,000` on 5 → Excel `0,005` — rests on a reading of ECMA-376
§18.8.31 and **has not been checked against a real Excel**: IronCalc's `0005` is measured, Excel's
`0,005` is inferred. Nothing depends on resolving it (the shape is rejected either way), but it must
not be cited as measured.

### Mild — `is_ironcalc_sign_bug`'s `!cell.contains('-')` — **CONFIRMED, anchored**

The clause was not a statement about the sign: it excluded every genuine E6 pair whose *format code*
carries a literal `-` (`-0`, `0-`, `$-0` on −1 → cell `-1` / `1-` / `$-1`, chart `--1` / `-1-` /
`-$-1`), which the widened `BARE_LITERALS` axis now reaches. The sign's position was already pinned
by `chart[1..] == *cell`, so the `contains` test was doing nothing else. It is gone, and in its place
the predicate **reconstructs IronCalc's own `is_negative`** (`format.rs:438` pre-round, then `:481`)
and fires only when that reconstruction says the sign was dropped — a named-defect match rather than
a shape match. The reconstruction is itself asserted against IronCalc's *measured* behaviour on every
pair the gate sees (266 820 assertions), so a fork fix that moves the E6 band fails there first, with
a pointer to the line that has to change.

### Carve-out counts after round 4

| Carve-out | Grounded corpus (960 pairs) | Generated space (265 860 pairs) |
|---|---|---|
| `is_ironcalc_sign_bug` (E6) | **48** | **6 960** |
| `is_ironcalc_rounding_defect` (E7) | **61** | **10 686** |
| `is_empty_code_artifact` | **30** | 0 (the generator emits no empty code) |
| agreeing | 821 | 248 214 |

5 988 of the 14 850 generated codes are rejected by the predicate (badged) — the mix
`the_faithful_subset_is_actually_a_subset` requires.

### Verification (round 4)

- `cargo test -p freecell-engine --test numfmt_agreement --test office_palette_agreement` — 6 + 2 passed
- `cargo test -p freecell-chart-model --lib` — 113 passed (4 new tests: the literal-placeholder fix,
  the bracket-token narrowing, the bare-literal narrowing, the non-grouping comma)
- `cargo clippy -p freecell-chart-model -p freecell-engine --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean
- **Mutation testing**, the round's central check — see the Critical 3 table: the round-2 sign defect
  passes on the round-3 tree and fails on this one.

**Render impact: none, and this time the reasoning is about the narrowings.** `apply_number_format`
changed again, so `render-tests/src/chart_scene.rs`'s codes were re-checked: it uses only `"$#,##0"`
and `"0%"`, on positive values. Neither carries a quoted literal, a `\`-escape, a bracket token, a
bare literal outside `$`, or any comma but the single grouping one — so neither reaches the new
lexer path in a way that can change its output, and both remain inside the (narrowed) faithful
subset. The `chart_*` baselines cannot move; the pixel subset was not run, per the round's working
agreement.
