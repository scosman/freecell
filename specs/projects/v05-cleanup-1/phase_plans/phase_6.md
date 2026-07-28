# Phase 6 — F3a: chart-axis / cell number-format and colour agreement

**Verdict: mostly DISPROVED as stated — and it found a serious IronCalc bug instead.**

Three sub-claims, three different answers:

| Sub-claim | Verdict |
|---|---|
| `chart-model`'s numfmt races IronCalc, so the same code produces two strings | **Disproved as a chart-model defect.** They agree everywhere reachable. Every divergence found is **IronCalc's**, including a severe cell-display bug. |
| `rgb_to_hsl` copies drifted (`%` vs `.rem_euclid`), disagreeing on negative hues | **Disproved.** Mathematically identical — both normalise with `.rem_euclid(360.0)` on the way out. Duplication real, divergence not. |
| Two definitions of the Office palette | **Confirmed.** Real duplication, currently in agreement, nothing enforcing it. |

## The number-format half

### Reframing, before testing

`chart-model/src/numfmt.rs` is not an unbounded reimplementation racing IronCalc. It is an
explicitly *bounded subset* with a companion predicate `renders_faithfully(code)`, and
`source_fidelity` degrades any chart whose codes fall outside it — so out-of-subset divergence is
**already disclosed to the user by the ⚠ badge**. The sharp, falsifiable invariant is therefore:

> For every code where `renders_faithfully(code)` is true, the chart's label must equal IronCalc's
> rendering — because the chart is *claiming* to be faithful and the user is comparing the axis
> against the cells.

`renders_faithfully` was `pub(crate)`; it is now exported, because it is the contract that makes a
disagreement a bug rather than a disclosed approximation.

### The test

`app/crates/freecell-engine/tests/numfmt_agreement.rs` — in the engine, the only crate depending on
both; putting it in `chart-model` would break the ironcalc-free boundary the design rests on.

Corpus: every `formatCode` extracted from the real fixtures in-tree (so it is grounded in files we
actually open), plus the everyday codes, crossed with values chosen for the places two
implementations diverge — signs, rounding half-way points, magnitudes crossing grouping and
exponent thresholds.

Reference: `ironcalc_base::formatter::format::format_number(v, code, locale("en"))`. Verified to be
the *exact* call the cell path makes — `model.rs:2815` `get_formatted_cell_value` →
`format_number(value, &format, self.locale).text`.

### What it found — and the mistake I made first

The gate went red with ~24 disagreements. My first reading was that `chart-model` rounded wrong
(Rust's `{:.n}` is half-to-**even**; Excel is half-away-from-zero), so I changed
`format_magnitude` to round half away from zero.

**That made agreement strictly worse** — it broke `1234.5`, `0.125`, `#,##0.00` and more, to fix
one case. Sweeping IronCalc's actual behaviour showed why:

```
0.5 → "1"    1.5 → "2"    2.5 → "2"    4.5 → "4"    10.5 → "10"    1234.5 → "1234"
0.125 → "0.12"    0.145 → "0.14"    1.005 → "1.00"    2.675 → "2.67"
```

IronCalc is half-to-even **everywhere except 0.5**, where it alone rounds up — inconsistent with
Excel *and* with itself. Rust's default already matched it on every other half-way case. So the
change was reverted, with a comment recording that it was tried and why it must not be retried.

The lesson generalises: "make them agree" is only well-posed if the reference is self-consistent.

### The real finding: IronCalc drops the minus sign

Stripping the rounding noise, the dominant disagreement class was IronCalc rendering **negative
numbers without a minus sign**:

```
format "#,##0", values [-1, 0.5, -0.5, -1234.5, 1]
displayed              ["1", "1", "1",  "-1,234", "1"]
```

Reproduced **end-to-end through the shipped app** — real worker, `SetStylePath(NumFmt)`, real
publication — not just through the test helper. Characterised precisely: the sign is dropped when
`|value| < 1.5 × 10^(-decimals)`, so a zero-decimal integer format (the most common format in a
spreadsheet) shows **-1 as 1**. `General` is unaffected.

`chart-model` is **correct** here and IronCalc is wrong. That inverts the unit's premise: satisfying
"make the chart agree with the cell" by changing `chart-model` would have copied a display bug into
the charts. So the disagreement is carved out of the gate by a named predicate
(`is_ironcalc_sign_bug`) that documents the defect and says to delete it once the fork carries the
fix — at which point the gate tightens by itself.

Filed as `projects/ironcalc-negative-sign-display.md` + a `PROJECTS.md` entry, with the
investigation starting point. **Not fixed here**: per `CLAUDE.md` §Engine it is a fork fix — one
`fix/` branch, one upstream PR — and the round's working agreement says a unit needing a fork change
flags it rather than folding it in. It is also plainly more severe than the unit that found it.

### `General`, and the one genuine chart-side gap

`chart-model`'s General is a *tick-label* formatter: integers bare, otherwise three decimals trimmed.
IronCalc's is Excel's — ~9 significant digits with a scientific fallback. So a data label reads
`0.333` where the cell reads `0.333333333`.

Not closed, deliberately. Making axis ticks print `0.333333333` is worse, not better; doing it
properly means separating tick formatting from data-label formatting, which is chart-project work.
`general_differs_from_the_cell_only_by_rounding_to_three_decimals` instead pins the *shape* of the
divergence — whatever the chart prints must be the same number correctly rounded, never a different
one — so it cannot silently widen.

### Carve-outs, in full

Three, each naming a characterised defect rather than waving a difference away: the IronCalc sign
bug, IronCalc's 0.5 outlier, and `formatCode=""` (which `chart-model` treats as General and IronCalc
answers `#VALUE!` — an input a *cell* cannot have).

## The colour half

### `rgb_to_hsl` — disproved

The copies are in `freecell-app/src/chart/palette.rs` and `freecell-chart-model/src/theme.rs`, not
`core` as the review said. The textual difference is real: `((g - b) / d) % 6.0` vs
`.rem_euclid(6.0)`.

It changes nothing. **Both functions end with `(h * 60.0).rem_euclid(360.0)`**, which maps the
negative intermediate onto the identical hue. Verified over the entire reachable input space (all
five base colours × every lap rotation) — zero differences. `hsl_to_rgb`'s `%` vs `.rem_euclid` is
likewise moot: its argument is non-negative by construction, where the two operators coincide.

The duplication is real even though the drift is not, so
`hsl_helpers_agree_with_the_chart_model_copy` pins it (helpers exported from `chart-model` for the
purpose). **Not deduplicated:** removing the copy means merging the crates (F3), and refactoring
chart-render colour code risks a baseline move for a difference that produces no different pixel.

### Office palette — confirmed

`freecell-core::palette::FILL_PALETTE` (10 swatches, the fill popover) and
`freecell-chart-model::ThemePalette::office_default()` (10 theme slots, `schemeClr` resolution) are
genuinely two definitions of the same Office colours, in two zero-dependency crates that do not know
each other exist. My architecture-doc guess that they were unrelated was wrong.

They currently agree. `tests/office_palette_agreement.rs` now enforces it slot by slot — including
the `lt1`=Background 1 / `dk1`=Text 1 pairing, which is the most likely way they would silently
diverge. Merging them is F3.

## Verification

- `cargo test -p freecell-engine --test numfmt_agreement` — 4 passed
- `cargo test -p freecell-engine --test office_palette_agreement` — 2 passed
- `cargo test -p freecell-chart-model --lib` — 93 passed
- `cargo test -p freecell-app --lib chart::palette` — 4 passed
- `cargo fmt --all --check` clean

**No render impact.** `apply_number_format`'s output is unchanged (the rounding experiment was
reverted), and no colour value moved — so no `chart_*` baseline can shift and the pixel subset was
not run, per the round's working agreement.
