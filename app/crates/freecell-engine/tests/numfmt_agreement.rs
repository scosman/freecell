//! **Chart-axis vs. cell number-format agreement** (unit F3a —
//! `projects/architecture-review-remediation.md`).
//!
//! `freecell-chart-model` must stay ironcalc-free, so it carries its own bounded
//! implementation of OOXML number formatting (`chart-model/src/numfmt.rs`) for axis ticks and
//! data labels. IronCalc formats the *cells* those charts plot. Two implementations of the same
//! spec, and the user reads both at once — an axis label beside the column it measures.
//!
//! This suite is the differential test. It lives here, in `freecell-engine`, because this is the
//! only crate that depends on **both**; putting it in `chart-model` would violate the
//! ironcalc-free boundary the whole design rests on.
//!
//! # The invariant, and how it is scoped
//!
//! `chart-model`'s formatter is deliberately a *subset*, with a companion predicate
//! `renders_faithfully(code)`. A chart whose format codes fall **outside** that subset is
//! classified `Fidelity::Degraded` and drawn with the ⚠ badge — the disagreement is disclosed.
//! So the sharp, falsifiable claim is:
//!
//! > For every code where `renders_faithfully(code)` is true, the chart's rendering of a value
//! > must equal IronCalc's — because the chart is claiming to be faithful, and the user is
//! > comparing the axis against the cells.
//!
//! A disagreement inside that set is a real bug: a chart labelled Faithful showing a different
//! string than the data it plots. A disagreement outside it is already flagged, and closing it
//! for real is F3 / G3.
//!
//! # The corpus is GENERATED from the predicate, not remembered
//!
//! The invariant above quantifies over *every* code the predicate accepts, so a hand-written corpus
//! can only ever sample it. Four consecutive reviews each found another accepted shape that
//! `chart-model` rendered wrong with no badge — the `#` optional-digit family, then a quoted or
//! `\`-escaped `%` (a 100× error), then required leading zeros, then a placeholder character
//! *inside* a literal (`\#0`, `"Item #"0`) — and each time the fix was to add the missing codes to
//! a list. The list is not the specification; the predicate is.
//!
//! [`generated_shape_space_agrees_with_the_cell`] therefore enumerates the shape space directly,
//! keeps the ones `renders_faithfully` accepts, and asserts agreement across [`VALUES`] — about
//! 266 000 (code, value) pairs. The hand-written [`FIXTURE_CODES`] / [`COMMON_CODES`] corpus is kept
//! as the *grounded* half (codes from real files in-tree, and the everyday codes), but it is no
//! longer the boundary of what is checked.
//!
//! **Round 4: the generated space is not the same thing as the accepted space, and the axes are
//! what closes the gap.** The round-3 generator swept
//! [`INTEGER_RUNS`] × [`FRACTIONAL_RUNS`] × [`PERCENT_FORMS`] × [`AFFIX_FORMS`] × [`PADS`], whose
//! affix axis was `{"", "$", "\$", "\" kg\"", "[$€-407]", "\"USD \""}` — not a placeholder
//! character, not a non-`$` bracket token, not a bare literal, not a doubled separator among them.
//! So a whole class of accepted-and-divergent codes was unreachable *by construction*, and the
//! round's headline ("zero unexplained disagreements across the whole generated space") was true of
//! the generated space while being false of the space the invariant actually ranges over. Three new
//! axes were added rather than the specific codes: [`LITERAL_AFFIXES`], [`BRACKET_TOKENS`] and
//! [`BARE_LITERALS`], plus the non-grouping comma shapes folded into [`INTEGER_RUNS`]. The residual
//! is still real and is stated in `phase_6.md`: axes are a sampling strategy, not a proof.
//!
//! **`General` is the one in-subset code this gate does not assert, and that is a hole, not a
//! property.** `renders_faithfully("General")` returns `true`, so by the invariant above the
//! strings should match — and they do not: `chart-model`'s General is a *tick-label* formatter
//! (three decimals, trimmed), IronCalc's is Excel's (~9 significant digits, scientific outside
//! `[1e-8, 1e11)`). `functional_spec.md` §F3a offers exactly two remedies for a disagreement
//! inside the subset — fix `chart-model` to match IronCalc, or (architecture §6 "Fix policy")
//! make `renders_faithfully` return `false` so the chart degrades honestly. Neither is taken
//! here, deliberately:
//!
//! - Matching IronCalc means axis ticks reading `0.333333333`, which is worse for the user than
//!   the divergence it removes; and
//! - returning `false` would badge **every chart with a General axis** — i.e. the default — as
//!   Degraded, converting a legible rounding difference into a permanent ⚠ on almost every file.
//!
//! The honest fix is the third of the spec's three outcomes ("fix what is reachable, file the
//! rest"): separating tick formatting from data-label formatting, which is chart-project work and
//! is filed in `GAPS.md`. Until then, `general_differs_from_the_cell_only_by_rounding` below pins
//! the *shape* of the divergence so it cannot silently widen, and this paragraph is the record
//! that the exclusion is a known, argued gap rather than an oversight.
//!
//! Codes outside the subset are still *exercised* here — printed as an informational table by
//! [`report_unfaithful_divergence`] — so the size of the disclosed gap is visible rather than
//! assumed.

use freecell_chart_model::{apply_number_format, renders_faithfully};
use ironcalc_base::formatter::format::format_number;
use ironcalc_base::locale::get_locale;
use ironcalc_base::number_format::to_precision;

/// The engine's locale. `WorkbookDocument::open` loads with `"en"`, so this is the formatter the
/// user's cells actually go through — comparing against any other locale would manufacture
/// disagreements nobody can see.
const LOCALE: &str = "en";

/// IronCalc's rendering of `value` under `code` — the reference, because it is what the cell shows.
fn ironcalc(code: &str, value: f64) -> String {
    let locale = get_locale(LOCALE).expect("the engine's locale resolves");
    format_number(value, code, locale).text
}

/// Format codes **taken from the real fixtures in this repo** (extracted from every `formatCode`
/// attribute across `tests/fixtures/**`), so the corpus is grounded in files we actually open
/// rather than invented. XML entities are decoded — these are the codes as the parser hands them
/// to the formatter.
const FIXTURE_CODES: &[&str] = &[
    "#,##0",
    "\"$\"#,##0.00",
    "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
    "0%",
    "0.0%",
    "0.0%;[RED]\\-0.0%",
    "General",
    "\\$#,##0",
    "\\$#,##0.00",
];

/// The everyday codes a chart axis or data label carries, beyond what the fixtures happen to
/// contain. This list is **no longer the boundary of what is asserted** — that is
/// [`generated_shape_space_agrees_with_the_cell`] — but it stays because it is the readable,
/// grounded half of the corpus and because each entry is here for a reason:
///
/// - the `#` **optional-digit** family (`0.##`, `#,##0.##`, `#,##0.0#`, `0.###`, `#,###`) and the
///   whitespace-padded `"0 "` are the round-1 finds (`#` counted as a required digit; the applier
///   trimming the code);
/// - `0" %"` / `0\%` / `%0` / `0%%` are the round-3 percent finds — a `%` inside a quoted literal
///   or behind a `\` is *not* the percent control, and a real one renders where it stands;
/// - `000` / `#,#00.0#` are the round-3 leading-zero finds (`000` on 7 must be `007`);
/// - `0.#0` / `#.0#0` are the mixed fractional runs a single "minimum decimals" count got wrong;
/// - `"0.0"` is the only single-decimal non-percent code here, and it is what makes `0.96` reach
///   IronCalc's lost-fractional-carry defect (`GAPS.md` E7).
const COMMON_CODES: &[&str] = &[
    "",
    "general",
    "0",
    "0.0",
    "0.00",
    "0.000",
    "#,##0.00",
    "0.00%",
    "$#,##0",
    "$#,##0.00",
    "#,##0.00 \"kg\"",
    "0.##",
    "#,##0.##",
    "#,##0.0#",
    "0.###",
    "#,###",
    "0 ",
    "000",
    "0000",
    "#,#00.0#",
    "0.#0",
    "#.0#0",
    "0\" %\"",
    "0\\%",
    "%0",
    "0%%",
    "0% ",
];

/// Codes deliberately **outside** the bounded subset. Not asserted — they exist so the
/// informational report shows how wide the disclosed (badged) gap actually is.
///
/// The round-3 additions (`##,##0` … `[$USD-409]#,##0`) are shapes `chart-model` renders the way
/// *Excel* does and IronCalc does not, so agreeing with the cell would mean copying an engine defect
/// into the charts (`GAPS.md` E8/E9). They are badged instead. The round-4 additions are the three
/// shapes the widened axes turned up where the **cell shows `#VALUE!`** (a lexer error) or scales
/// the value: again chart-model is the correct side, so again the resolution is a badge.
/// `the_faithful_subset_is_actually_a_subset` asserts they really are rejected, so this list cannot
/// rot into a set of silently-faithful codes.
const OUT_OF_SUBSET_CODES: &[&str] = &[
    "0.00E+00",
    "yyyy-mm-dd",
    "mm/dd/yyyy",
    "h:mm:ss",
    "#,##0.00;[Red](#,##0.00)",
    "[<100]0;0.0",
    "# ?/?",
    "0.00_);(0.00)",
    "##,##0",          // E8: the cell drops the group separator on 4-digit values
    "0,000",           // E8: the cell never groups the zeros it padded in
    "0\"a\"\"b\"",     // E9: the two lexers read a doubled quote differently
    "[$USD-409]#,##0", // a multi-character currency symbol the cell's lexer rejects outright
    "[h]0",            // round 4: elapsed time — the cell shows `#VALUE!`
    "[DBNum1]#,##0",   // round 4: not a token the cell's lexer knows — `#VALUE!`
    "0°",              // round 4: a bare literal outside `Token::Literal`'s list — `#VALUE!`
    "#,,##0",          // round 4: a comma that is not between digits is a ÷1000 scale, not grouping
];

/// Values chosen to hit the places two independent implementations diverge: sign handling, the
/// rounding half-way boundary, magnitudes that cross grouping and exponent thresholds.
///
/// The `|v| < 1` cluster is the **E7 band** — the whole sub-unit range is where IronCalc's
/// double-round-plus-dropped-carry pipeline can corrupt a value (see [`is_ironcalc_rounding_defect`]).
/// `0.45` / `-0.46` / `0.96` were added in round 2 for the two *sub-cases* the write-up named;
/// `0.855`, `0.0495`, `-0.8745`, `0.1234567` and `-0.98765` were added in round 3 because the
/// mechanism is not those two sub-cases — it corrupts anything that ties or crosses a tie at the
/// intermediate precision, and a corpus that only contained the named sub-cases could not have
/// falsified a predicate written from them. `0.1234567` / `-0.98765` additionally give
/// `general_differs_from_the_cell_only_by_rounding` values with 4+ significant fractional digits,
/// without which its assertions reached only 3 of 21 values.
///
/// The E6 boundary values (`1.5`, `0.105`, `0.01005`, and `-0.0101` just past it) pin the measured
/// sign-drop thresholds, so a fork fix that moves the band is noticed here and not only in prose.
const VALUES: &[f64] = &[
    0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    0.125,
    1234.5,
    -1234.5,
    999.995, // rounds up across every digit at 2dp
    0.005,   // exact half at 2dp
    -0.005,
    0.001,     // rounds to zero at 2dp — the negative twin is the DOLLAR-style trap
    -0.001,    //
    0.45,      // E7: displays as "1" under code `0` (pre-rounded to 1 significant digit first)
    -0.46,     // E7, negative twin
    0.96,      // E7: displays as "0.0" under code `0.0` (the fractional carry is dropped)
    0.855,     // E7: `0.0` → cell "0.8", chart "0.9" — a plain double round, neither sub-case
    0.0495,    // E7: `0.0` → cell "0.1", chart "0.0"
    -0.8745,   // E7: `0.00` → cell "-0.88", chart "-0.87"
    0.1234567, // 7 significant fractional digits — General's rounding has something to do
    -0.98765,  // ditto, negative, and E7's carry case under `#.#`
    1.5,       // E6: the largest magnitude that still loses its sign under `0`
    0.105,     // E6: ditto under `0.0`
    0.01005,   // E6: ditto under `0.00`
    -0.0101,   // E6: just past the `0.00` threshold — the sign SURVIVES here
    1_000_000.0,
    -1_000_000.0,
    1e-7,
    1e15,
    0.3333333333333333,
];

/// The format's fractional-digit count and percent scale — the two things IronCalc's rounding path
/// is parameterised by, and which [`is_ironcalc_rounding_defect`] needs in order to reconstruct it.
///
/// Both are read off the **control body** (bracket tokens, quoted literals and `\`-escapes
/// stripped), because a `%` inside `"…"` or behind a `\` is a literal percent sign and not the
/// percent control — the same distinction that `chart-model`'s parser got wrong until round 3.
fn code_precision(code: &str) -> (usize, i32) {
    let section = code.split(';').next().unwrap_or(code);
    let mut body = String::new();
    let mut chars = section.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                for bracket_char in chars.by_ref() {
                    if bracket_char == ']' {
                        break;
                    }
                }
            }
            '"' => {
                for quote_char in chars.by_ref() {
                    if quote_char == '"' {
                        break;
                    }
                }
            }
            '\\' => {
                chars.next();
            }
            _ => body.push(c),
        }
    }
    let percent_scale = body.matches('%').count() as i32;
    let decimals = match (body.find(['0', '#']), body.rfind(['0', '#'])) {
        (Some(first), Some(last)) => body[first..=last]
            .split_once('.')
            .map(|(_, frac)| frac.chars().filter(|c| *c == '0' || *c == '#').count())
            .unwrap_or(0),
        _ => 0,
    };
    (decimals, percent_scale)
}

/// The three carve-outs below are the *only* permitted disagreements, and each names a specific,
/// characterised defect rather than waving the difference away. Everything else must match exactly.
///
/// **1. IronCalc drops the minus sign on small negatives** (`GAPS.md` E6). `format_number` computes
/// `is_negative = value < -(10^-precision)` (`format.rs:481`) **after** the `to_precision` pre-round
/// at `format.rs:438`, so the sign is dropped whenever the pre-rounded magnitude is at or below
/// `10^-decimals` — a cell formatted `#,##0` holding **-1 displays "1"**, and `0.00` holding -0.005
/// displays "0.01".
///
/// The exact threshold is **`10^-d + 5×10^-(2d+1)`** (the extra term is the pre-round's half-step at
/// the `(d+1)`-th significant digit), **not** `1.5 × 10^-decimals` — those two coincide only at
/// `d = 0`. Measured by bisection, and identical in `GAPS.md` E6,
/// `projects/ironcalc-negative-sign-display.md` and `phase_6.md`: `0` → **1.5**, `0.0` → **0.105**,
/// `0.00` → **0.01005**, `0.000` → **0.0010005**, `0%` → **0.015** (the ×100 scale moves the band,
/// not the rule). This doc comment carried a different, self-contradictory set of numbers
/// (`|value| <= 10^-decimals`, `0` → ~1.05, `0.00` → ~0.0101) after the other four documents were
/// corrected; the values above are the measured ones, and `VALUES` now contains 1.5, 0.105, 0.01005
/// and -0.0101 so the boundary is exercised rather than asserted in prose.
///
/// Verified end-to-end through the real app (worker + `SetStylePath(NumFmt)` + publication), not
/// just through this helper. `General` is unaffected (a different code path in IronCalc).
///
/// **`chart-model` is CORRECT here and IronCalc is wrong**, so "make them agree" must not be
/// satisfied by copying the bug into the charts. This is an engine defect and belongs in the fork
/// per CLAUDE.md §Engine (one `fix/` branch, one upstream PR); tracked in `GAPS.md`. When the fork
/// carries the fix, delete this carve-out and the gate tightens by itself.
///
/// **The predicate is deliberately narrow.** It requires the chart's *unsigned* rendering to carry
/// a non-zero digit, so it can only fire where a real number lost its sign. Without that clause it
/// also swallowed 15 pairs where **`chart-model`** was the buggy side — it emitted `-0`, `-0.00`,
/// `-$0.00`, `-0%`, `-0.00 kg` for negatives that round to zero, which Excel and IronCalc both
/// print unsigned. That defect is fixed in `chart-model`; this clause makes sure the carve-out
/// cannot hide its like again.
///
/// **Round 4: anchored to the sign POSITION, and to IronCalc's own sign test.** The clause was
/// `!cell.contains('-')`, which is not a statement about the sign at all — it also excluded every
/// pair whose *format code* carries a literal `-` (`-0`, `0-`, `$-0` on -1: cell `-1` / `1-` /
/// `$-1`, chart `--1` / `-1-` / `-$-1`). Those are genuine E6 pairs — IronCalc dropped the sign and
/// what remains is the code's own literal — and the gate would have reported them as hard failures
/// once the corpus reached a `-`-bearing code. The sign's position is already pinned by
/// `chart[1..] == *cell` (the cell is the chart minus exactly the leading character), so the
/// `contains` test was doing nothing else.
///
/// In its place the predicate now **reconstructs IronCalc's own decision**
/// ([`ironcalc_keeps_the_sign`]) and fires only when that decision was "drop it". That turns a
/// shape match into a named-defect match: a pair where IronCalc kept the sign and the strings still
/// differ by a leading `-` is not this defect, and is not suppressed.
fn is_ironcalc_sign_bug(code: &str, value: f64, chart: &str, cell: &str) -> bool {
    value < 0.0
        && chart.starts_with('-')
        && chart[1..] == *cell
        && chart[1..].contains(|c: char| c.is_ascii_digit() && c != '0')
        && !ironcalc_keeps_the_sign(code, value)
}

/// Whether **`chart-model`** attached a minus sign to this rendering.
///
/// Measured by rendering the same magnitude and comparing, **not** by looking for a leading `-`:
/// a format code may carry a literal `-` of its own, so a leading dash is not evidence of a sign
/// decision. `#-` on **+0.5** renders `-` (the integer part is suppressed and the dash is the
/// code's), and a `starts_with('-')` gate read that as "the chart signed a positive number".
fn chart_is_signed(code: &str, value: f64) -> bool {
    value < 0.0 && apply_number_format(code, value) != apply_number_format(code, -value)
}

/// Whether **IronCalc** attached a minus sign, measured the same way. `format.rs` emits the sign as
/// `text = format!("-{text}")` at integer digit index 0 and takes every digit from `value_abs`, so
/// the signed rendering is exactly `-` prepended to the unsigned one — which is what makes this
/// comparison exact rather than approximate.
fn cell_is_signed(code: &str, value: f64) -> bool {
    value < 0.0 && ironcalc(code, value) != ironcalc(code, -value)
}

/// IronCalc's own `is_negative`, reconstructed from `format.rs`: scale by the percent factor,
/// pre-round with `to_precision(value, precision + integer_digits)` (`:438`), then test
/// `value < -(10^-precision)` (`:481`). `true` means the cell is expected to carry a minus sign.
///
/// The scaling-comma factor (`/1000^comma`) is omitted deliberately: `renders_faithfully` rejects
/// every code that has one, so no pair reaching this gate can have `comma > 0`.
fn ironcalc_keeps_the_sign(code: &str, value: f64) -> bool {
    let (decimals, percent_scale) = code_precision(code);
    let scaled = value * 100f64.powi(percent_scale);
    let integer_digits = format!("{}", scaled.abs().floor()).len();
    let pre_rounded = to_precision(scaled, decimals + integer_digits);
    pre_rounded < -(10f64.powf(-(decimals as f64)))
}

/// **2. IronCalc's rounding is not a rounding rule** (`GAPS.md` E7). `formatter/format.rs` does not
/// round once; it rounds in pieces, and the pieces disagree. **The mechanism is one thing, not a
/// list of cases:**
///
/// > For `|v| < 1` the integer part is `"0"`, so the pre-round
/// > `to_precision(value, precision + integer_digits)` (`format.rs:438`) keeps **`decimals + 1`
/// > significant digits**, and the render then rounds *again* to `decimals` decimals. That is a
/// > plain **double round**, and anything that ties or crosses a tie at the intermediate precision
/// > is corrupted. For `|v| >= 1` the pre-round happens at exactly the rendered precision and
/// > nothing is corrupted.
///
/// The two sub-cases the earlier write-ups enumerated are consequences of it, not the extent of it:
///
/// - **`decimals == 0`, the band `|v| ∈ [0.45, 0.5]`** — one significant digit survives the
///   pre-round, so the whole band becomes `0.5` and then `value_abs.round()` (half **away from
///   zero**) makes it `"1"`. Correct: `"0"`.
/// - **`decimals >= 1`, the lost fractional carry** — `get_fract_part` rounds the fraction on its
///   own and slices `"0.ddd"[2..]`; when the fraction rounds up to `1.0` the slice is empty and the
///   carry into the (zero) integer part is discarded. `0.96` under `0.0` displays **`"0.0"`**.
///
/// Neither describes `0.855` under `"0.0"` (cell `0.8`, chart `0.9`), `0.0495` under `"0.0"` (cell
/// `0.1`, chart `0.0`) or `-0.8745` under `"0.00"` (cell `-0.88`, chart `-0.87`) — 886 such pairs
/// were left unexplained by the sub-case predicate, and a decimal-exact reference showed
/// `chart-model` right on every one of them. **The predicate is therefore derived from the
/// mechanism**: it reconstructs IronCalc's sub-unit pipeline and fires only when that reconstruction
/// reproduces the cell **byte for byte**.
///
/// That confirmation is what keeps it a carve-out rather than a hole. The reconstruction renders
/// its predicted magnitude through `chart-model` itself, so it can only match the cell when
/// `chart-model`'s *presentation* (padding, grouping, affixes, percent placement) is already
/// identical to IronCalc's and the sub-unit **rounding** is the sole difference.
///
/// **That argument had one hole, and round 4 found it: sign suppression.** The reconstruction feeds
/// the *positive* predicted magnitude to `apply_number_format`, and the negative fallback feeds
/// `-predicted` — which for the whole lost-carry class is `-0.0`, and `-0.0 < 0.0` is `false`. So
/// re-introducing round 2's exact chart-side defect (deciding the sign from `scaled < 0.0` before
/// rounding, i.e. dropping `has_significant_digit`) left every reconstruction untouched: all six
/// tests still passed, and the census simply moved 6 424 pairs from "agree" into "E7". A carve-out
/// written specifically so it could not hide a sign bug was hiding a sign bug.
///
/// The fix is not prose but a **gate**: the chart and the cell must have made the *same* sign
/// decision before this predicate may fire at all. Under the re-introduced defect the chart prints
/// `-0` where the cell prints `0`, the gate blocks the carve-out, and the pair becomes a hard
/// failure — verified by running exactly that mutation (round-4 remediation, `phase_6.md`).
///
/// For `|v| >= 1` the predicate cannot fire at all, so any disagreement there is a hard failure.
/// Excel is half-away-from-zero throughout (`2.5 → 3`) while IronCalc lands on half-to-even there
/// (`2.5 → "2"`), so **both implementations are wrong versus Excel, in the same direction**;
/// `chart-model`'s `{:.n}` is pinned to IronCalc's behaviour on purpose so the axis matches the cell
/// beside it. When the fork fix lands, half-away-from-zero becomes correct in `chart-model` too (see
/// the comment on `numfmt::format_magnitude`).
///
/// The predicate takes the **code**, not just the value, because the pipeline is parameterised by
/// the format's decimal count and percent scale — `0%` reaches the sub-unit regime at value 0.005
/// (0.005 × 100 = 0.5).
fn is_ironcalc_rounding_defect(code: &str, value: f64, cell: &str) -> bool {
    // **The sign gate.** A rounding carve-out may only explain a pair on which both sides already
    // agree about the sign; a difference in the *sign* decision is E6's business or a chart bug, and
    // either way must not be absorbed here. See the doc comment above for the mutation this closes.
    if chart_is_signed(code, value) != cell_is_signed(code, value) {
        return false;
    }
    let (decimals, percent_scale) = code_precision(code);
    let scale = 100f64.powi(percent_scale);
    let magnitude = (value * scale).abs();
    if magnitude >= 1.0 {
        return false;
    }
    let predicted = ironcalc_sub_unit_magnitude(magnitude, decimals) / scale;
    let rendered = apply_number_format(code, predicted);
    // IronCalc decides the sign from a magnitude threshold *before* the digits exist (E6), so it can
    // also print a signed rendering of a magnitude that came out as nothing at all — `#.#` on
    // -0.98765 is `-.` in the cell. Both spellings count as reproducing it.
    cell == rendered
        || (value < 0.0
            && (cell == format!("-{rendered}") || apply_number_format(code, -predicted) == cell))
}

/// The magnitude IronCalc's `ParsePart::Number` arm actually lands on for `magnitude < 1`, straight
/// from `format.rs`: pre-round to `decimals + 1` significant digits (the integer part contributes
/// exactly one, `"0"`), then `value_abs.round()` at `precision == 0`, or a separately-rounded
/// fraction whose carry into the integer part is thrown away.
fn ironcalc_sub_unit_magnitude(magnitude: f64, decimals: usize) -> f64 {
    let pre_rounded = to_precision(magnitude, decimals + 1);
    if decimals == 0 {
        return pre_rounded.round();
    }
    let fraction = format!("{:.*}", decimals, pre_rounded.fract());
    match fraction.split_once('.') {
        // "0.ddd" — the fraction stayed below 1 and every digit survives.
        Some(("0", digits)) => format!("0.{digits}").parse().unwrap_or(0.0),
        // "1.000" — the fraction carried, and `get_fract_part`'s `[2..]` slice discards it.
        _ => 0.0,
    }
}

/// **3. An empty format code is not a thing a cell can have.** OOXML lets a chart carry
/// `formatCode=""`, which `chart-model` reasonably treats as General; IronCalc's formatter returns
/// `#VALUE!` because a *cell* always has a format string. Comparing them here measures an input the
/// cell path cannot receive.
///
/// This is a **pin, not a waiver**: the empty code is fed through the gate (it is not skipped with
/// `General`) precisely so that `#VALUE!` is asserted rather than assumed. Written the other way —
/// skipping the empty code alongside General — the predicate fired zero times and the claim that
/// IronCalc answers `#VALUE!` was documented but untested.
fn is_empty_code_artifact(code: &str, cell: &str) -> bool {
    code.trim().is_empty() && cell == "#VALUE!"
}

/// Whether this (code, value) pair is allowed to disagree, and why. `None` means "must match".
fn carve_out(code: &str, value: f64, chart: &str, cell: &str) -> Option<&'static str> {
    // `is_empty_code_artifact` is tested FIRST: an empty code never reaches a numeric format path
    // at all, so the other two predicates have nothing to say about it.
    if is_empty_code_artifact(code, cell) {
        Some("empty-code `#VALUE!`")
    } else if is_ironcalc_sign_bug(code, value, chart, cell) {
        Some("IronCalc sign bug (E6)")
    } else if is_ironcalc_rounding_defect(code, value, cell) {
        Some("IronCalc rounding defect (E7)")
    } else {
        None
    }
}

/// **The gate.** Inside the faithful subset, for every code but `General`, the chart label and the
/// cell must be byte-identical. (`General` has its own test below, and the module docs argue why it
/// is excluded rather than fixed or badged.)
#[test]
fn chart_and_cell_agree_on_every_faithfully_rendered_code() {
    let corpus: Vec<String> = FIXTURE_CODES
        .iter()
        .chain(COMMON_CODES)
        .map(|c| c.to_string())
        .collect();
    let pairs = assert_agreement(&corpus, "the grounded corpus (fixtures + everyday codes)");
    assert!(
        pairs > 500,
        "the grounded corpus contributed only {pairs} asserted pairs",
    );
}

/// **The same gate, over the shape space the predicate accepts rather than the codes we remembered.**
///
/// Every previous round of this work ended with "add the missing family to the corpus"; every next
/// round found another one. The corpus is generated from the same dimensions `renders_faithfully`
/// reasons about, so a shape it accepts and `chart-model` renders differently from the cell is a
/// build failure instead of the next reviewer's find.
#[test]
fn generated_shape_space_agrees_with_the_cell() {
    let corpus = generated_corpus();
    let pairs = assert_agreement(&corpus, "the generated shape space");
    // **The degeneracy guard measures what is ASSERTED, not what is generated.** It used to check
    // `corpus.len() > 5_000` — the *unfiltered* generator output, a number no change to
    // `renders_faithfully` can move. Narrowing the predicate to accept nothing at all would have
    // left it green while the gate asserted zero pairs. The count below is the number of
    // (accepted code, value) pairs actually compared against IronCalc, which is the quantity the
    // invariant ranges over.
    assert!(
        pairs > 100_000,
        "the generated shape space contributed only {pairs} ASSERTED (code, value) pairs — either \
         the generator stopped sweeping the shape space or `renders_faithfully` was narrowed to \
         near-nothing, and this gate is no longer checking the invariant it claims to",
    );
}

/// Integer runs: optional-only, required-only, mixed, every grouping shape — including the two
/// (`##,##0`, `0,000`) that `renders_faithfully` must *reject* because IronCalc groups them wrong —
/// and (round 4) the **separator axis**: comma placements that are not grouping at all. IronCalc
/// counts a comma as a thousands separator only when it sits *between two digit tokens*
/// (`parser.rs`: `use_thousands = last_token_is_digit && next_token_is_digit`) and as a ÷1000 scale
/// otherwise, so `#,,##0` on 1 shows `0` in the cell where the applier shows `1`.
const INTEGER_RUNS: &[&str] = &[
    "#",
    "0",
    "##",
    "00",
    "000",
    "0000",
    "###",
    "#,##0",
    "#,###",
    "##,##0",
    "#,#00",
    "#,000",
    "0,000",
    "#,,##0",
    "#,,#0",
    "0,,0",
    "#,#,#0",
    "#,##,##0",
    "##,###,##0",
];
/// Fractional runs: absent, optional-only, required-only, and every interleaving that a single
/// "minimum decimals" count got wrong (`.#0`, `.0#0`).
const FRACTIONAL_RUNS: &[&str] = &[
    "", ".#", ".0", ".##", ".0#", ".00", ".#0", ".0#0", ".###", ".000",
];
/// Percent forms: absent, the control, the control twice, and the three spellings of a **literal**
/// percent sign — quoted with and without a leading space, and `\`-escaped.
const PERCENT_FORMS: &[&str] = &["", "%", "%%", "\" %\"", "\"%\"", "\\%"];
/// Affixes: none, a bare `$`, an escaped `\$`, a quoted suffix, a bracketed currency token, and a
/// quoted prefix.
const AFFIX_FORMS: &[&str] = &["", "$", "\\$", "suffix", "[$€-407]", "prefix"];
/// Trailing literal whitespace — the padding the applier used to `trim()` away.
const PADS: &[&str] = &["", " "];

/// A small set of everyday numeric shapes, used as the carrier for the three round-4 axes below.
/// Crossing those axes against the *full* `INTEGER_RUNS × FRACTIONAL_RUNS × …` space would multiply
/// the corpus by 60× for no extra discrimination — the axes are independent of the digit shape.
const CORE_NUMERIC: &[&str] = &[
    "0", "0.0", "0.00", "#,##0", "#,##0.00", "#", "#.##", "000", "0%",
];

/// **The literal-affix axis** (round 4). A literal that *contains* a placeholder character is the
/// case the predicate and the applier disagreed about for three rounds: `renders_faithfully` looked
/// for the numeric run on the stripped control body while `FormatSpec::parse` looked for it on a
/// string that still contained quotes and `\`-escapes, so a `0`/`#`/`,`/`.` inside a literal was
/// invisible to one and a digit placeholder to the other. `\#0` and `"Item #"0` are standard Excel
/// idioms; both rendered wrong, unbadged, and neither was reachable from the round-3 axes (whose
/// affix forms were `{"", "$", "\$", "\" kg\"", "[$€-407]", "\"USD \""}` — not a placeholder
/// character among them).
const LITERAL_AFFIXES: &[&str] = &[
    "\"0\"",
    "\"#\"",
    "\",\"",
    "\".\"",
    "\" of 100\"",
    "\"Item #\"",
    "\" #\"",
    "\" (0)\"",
    "\"0.0\"",
    "\\0",
    "\\#",
    "\\,",
    "\\.",
    "\" kg\"",
];

/// **The bracket-token axis** (round 4). Only `[$…]` was swept before. IronCalc's lexer accepts a
/// currency token, a condition, elapsed time, and exactly seven lower-cased colour names; every
/// other bracket token is a hard lexer **error** and the cell shows `#VALUE!` while the applier
/// renders a number. `[Color 5]` is spelled both ways on purpose — the lexer's arm is
/// case-sensitive (`chars.starts_with("Color")`), so the two spellings are not interchangeable.
const BRACKET_TOKENS: &[&str] = &[
    "[Red]",
    "[red]",
    "[Blue]",
    "[Black]",
    "[Green]",
    "[Magenta]",
    "[Yellow]",
    "[White]",
    "[Color 5]",
    "[color 5]",
    "[Color 99]",
    "[$€-407]",
    "[$$-409]",
    "[$USD-409]",
    "[h]",
    "[mm]",
    "[DBNum1]",
    "[>=100]",
    "[<0]",
    "[t]",
];

/// **The bare-literal axis** (round 4). An unquoted, unescaped literal character is a
/// `Token::Literal` only if it is in IronCalc's explicit list (`formatter/lexer.rs`: `$ € ( ) / :
/// + - ^ ' { } < = ! ~ >` and space); a date letter starts a date token, and anything else is a
/// lexer error. So `0°` / `0£` / `0¥` / `0µ` / `0×` / `0¤` / `0\t` show `#VALUE!` in the cell while
/// the applier renders `7°`, `7£`, … Here **chart-model is the correct side**, so the resolution is
/// a predicate narrowing, not a chart change. `-` and `+` are in the list for a second reason: a
/// **sign-bearing literal** interacts with IronCalc's sign handling (`-0` on -1 → cell `-1`, chart
/// `--1`), which is the E6 defect and was not reachable while `is_ironcalc_sign_bug` required
/// `!cell.contains('-')`.
const BARE_LITERALS: &[&str] = &[
    "$", "€", "(", ")", "/", ":", "+", "-", "^", "'", "{", "}", "<", "=", "!", "~", ">", " ", "°",
    "£", "¥", "µ", "×", "¤", "\t", "k", "z", "E", "m", "y", "AM/PM",
];

/// Every code in the shape space, faithful or not. Filtering is the caller's job.
///
/// Four families, not one cross product: the digit-shape space, then one family per round-4 axis
/// carried on [`CORE_NUMERIC`]. Each family places its axis value in **both** positions (before and
/// after the digits), because position is exactly what several of the divergences turned on — a
/// `[$…]` currency is hoisted to the head of the string, and a literal ahead of the run lands in the
/// prefix rather than the suffix.
fn generated_corpus() -> Vec<String> {
    let mut codes = Vec::new();
    for integer_run in INTEGER_RUNS {
        for fractional_run in FRACTIONAL_RUNS {
            for percent in PERCENT_FORMS {
                for affix in AFFIX_FORMS {
                    for pad in PADS {
                        let numeric = format!("{integer_run}{fractional_run}{percent}");
                        codes.push(match *affix {
                            "$" => format!("${numeric}{pad}"),
                            "\\$" => format!("\\${numeric}{pad}"),
                            "[$€-407]" => format!("[$€-407]{numeric}{pad}"),
                            "suffix" => format!("{numeric}\" kg\"{pad}"),
                            "prefix" => format!("\"USD \"{numeric}{pad}"),
                            _ => format!("{numeric}{pad}"),
                        });
                    }
                }
            }
        }
    }
    for numeric in CORE_NUMERIC {
        for axis in LITERAL_AFFIXES
            .iter()
            .chain(BRACKET_TOKENS)
            .chain(BARE_LITERALS)
        {
            codes.push(format!("{axis}{numeric}"));
            codes.push(format!("{numeric}{axis}"));
        }
    }
    codes
}

/// Assert the invariant over a corpus, reporting every disagreement at once rather than the first.
/// Returns the number of in-subset (code, value) pairs actually asserted, so a caller can guard
/// against the corpus or the predicate degenerating.
#[must_use]
fn assert_agreement(corpus: &[String], what: &str) -> usize {
    let mut disagreements: Vec<String> = Vec::new();
    let mut pairs = 0usize;
    for code in corpus {
        if !renders_faithfully(code) || is_general(code) {
            continue;
        }
        for &value in VALUES {
            pairs += 1;
            let chart = apply_number_format(code, value);
            let cell = ironcalc(code, value);
            if chart == cell || carve_out(code, value, &chart, &cell).is_some() {
                continue;
            }
            if disagreements.len() < 40 {
                disagreements.push(format!(
                    "    code {code:?} value {value}\n      chart axis: {chart:?}\n      cell:       {cell:?}"
                ));
            }
        }
    }
    assert!(
        pairs > 0,
        "{what} contributed no in-subset pairs — the gate is asserting nothing",
    );
    assert!(
        disagreements.is_empty(),
        "over {what} ({pairs} in-subset pairs) chart-model claims to render these codes faithfully, \
         but the axis label and the cell it measures disagree — the chart is drawn WITHOUT a \
         fidelity badge while showing a different string than its data (first 40 shown):\n{}\n\
         Fix `chart-model::numfmt` to match IronCalc (the cell is the reference the user compares \
         against), or — if a code cannot be made to agree without growing the reimplementation — \
         make `renders_faithfully` return false for it, so the chart degrades honestly instead. \
         If the disagreement is IRONCALC's fault, fix it in the fork (CLAUDE.md §Engine) rather \
         than copying it into the charts.",
        disagreements.join("\n"),
    );
    pairs
}

/// `General` only — the empty code is **not** skipped, so that `is_empty_code_artifact` pins
/// IronCalc's `#VALUE!` instead of documenting it.
fn is_general(code: &str) -> bool {
    code.trim().eq_ignore_ascii_case("General")
}

/// **`General` is deliberately different, and this test pins how.**
///
/// `chart-model`'s General is a *tick-label* formatter: integers print bare, everything else keeps
/// three decimals trimmed of trailing zeros. IronCalc's General is Excel's — up to ~9 significant
/// digits, with a scientific fallback outside `[1e-8, 1e11)`. So a **data label** reading `0.333`
/// sits beside a cell reading `0.333333333`.
///
/// That is a real fidelity gap for data labels (tracked in `GAPS.md`), but it is **not** a bug to
/// close by making axis ticks print `0.333333333` — nobody wants that on an axis. Closing it
/// properly means separating tick formatting from label formatting, which is chart-project work,
/// not F3a's (the module docs argue the exclusion in full). This test therefore asserts the *shape*
/// of the divergence so it cannot silently widen: the chart must print **the same number with
/// fewer digits** — never a different number, never more than three decimals, never a signed zero.
///
/// It reached its assertions for only 3 of the 21 corpus values while every other value round-tripped
/// identically; `VALUES` now carries `0.1234567` and `-0.98765` (4+ significant fractional digits)
/// so the constraints are actually exercised. The reach is asserted at the end.
#[test]
fn general_differs_from_the_cell_only_by_rounding() {
    let mut reached = 0usize;
    for &value in VALUES {
        let chart = apply_number_format("General", value);
        let cell = ironcalc("General", value);
        if chart == cell {
            continue;
        }
        reached += 1;
        let reparsed: f64 = chart.parse().unwrap_or_else(|_| {
            panic!("General produced a non-numeric label {chart:?} for {value} (cell: {cell:?})")
        });

        // 1. Same number: exactly `value` rounded to three decimals, not merely "close to" it.
        //    (The old relative tolerance would have accepted 1233.3 for 1234.5, and only two of
        //    the corpus values ever reached it.)
        let expected: f64 = format!("{value:.3}").parse().expect("`{:.3}` parses back");
        assert_eq!(
            reparsed, expected,
            "General on the chart rendered {value} as {chart:?} — that is not {value} rounded to \
             three decimals, it is a different number (cell shows {cell:?})",
        );

        // 2. Fewer digits, never more: at most three fractional digits, and no trailing zeros.
        let fractional = chart.split_once('.').map_or("", |(_, frac)| frac);
        assert!(
            fractional.len() <= 3 && !fractional.ends_with('0'),
            "General on the chart rendered {value} as {chart:?} — a tick label must be the SHORTER \
             rendering (<= 3 fractional digits, trimmed), not a longer or padded one (cell shows \
             {cell:?})",
        );

        // 3. Never a signed zero: `-0` is not "the same number with fewer digits".
        // Read as "if it is signed, it must carry a non-zero digit" — clippy rejects the
        // literal `!(signed && !has_digit)` spelling as a non-minimal boolean.
        assert!(
            !chart.starts_with('-') || chart.contains(|c: char| c.is_ascii_digit() && c != '0'),
            "General on the chart rendered {value} as {chart:?} — a magnitude that rounds away to \
             zero must print unsigned (cell shows {cell:?})",
        );
    }
    assert!(
        reached >= 5,
        "the three assertions above ran for only {reached} of {} values — the corpus no longer \
         contains enough values with more fractional precision than a tick label keeps, so this \
         test is passing without constraining anything",
        VALUES.len(),
    );
}

/// Codes *outside* the subset are allowed to disagree (the chart is badged `Degraded`), but the
/// size of that gap should be visible rather than assumed. Informational only — the assertion that
/// `OUT_OF_SUBSET_CODES` really is out of subset lives in
/// [`the_faithful_subset_is_actually_a_subset`], so this test genuinely cannot fail. Run it with
/// `--nocapture` to read the table.
#[test]
fn report_unfaithful_divergence() {
    let mut rows = 0usize;
    let mut differing = 0usize;
    println!("codes OUTSIDE the faithful subset (chart shows ⚠, disagreement is disclosed):");
    for code in OUT_OF_SUBSET_CODES {
        for &value in VALUES {
            rows += 1;
            let chart = apply_number_format(code, value);
            let cell = ironcalc(code, value);
            if chart != cell {
                differing += 1;
                if differing <= 12 {
                    println!("  {code:>26?} {value:>14} → chart {chart:?} / cell {cell:?}");
                }
            }
        }
    }
    println!("  … {differing} of {rows} (code, value) pairs differ outside the subset.");

    // The same, for the generated space: how much of the shape space the badge is carrying.
    let corpus = generated_corpus();
    let rejected = corpus.iter().filter(|c| !renders_faithfully(c)).count();
    println!(
        "generated shape space: {} of {} codes are rejected by `renders_faithfully` (badged)",
        rejected,
        corpus.len(),
    );
}

/// A guard on the scoping itself: if `renders_faithfully` ever returned true for everything, the
/// gate above would still pass while asserting nothing meaningful about the subset's boundary.
#[test]
fn the_faithful_subset_is_actually_a_subset() {
    assert!(
        FIXTURE_CODES
            .iter()
            .chain(COMMON_CODES)
            .any(|c| renders_faithfully(c)),
        "no code in the corpus is inside the faithful subset — the gate is asserting nothing",
    );
    assert!(
        OUT_OF_SUBSET_CODES.iter().all(|c| !renders_faithfully(c)),
        "the out-of-subset corpus is not out of subset — move any faithful code to COMMON_CODES so \
         it is ASSERTED, not merely reported",
    );
    // The generated space must be a genuine mix: all-accepted would mean the predicate stopped
    // discriminating, all-rejected would mean the generator stopped producing renderable codes.
    let corpus = generated_corpus();
    let faithful = corpus.iter().filter(|c| renders_faithfully(c)).count();
    assert!(
        faithful > corpus.len() / 2 && faithful < corpus.len(),
        "the generated shape space is {faithful} faithful of {} — it should be mostly faithful with \
         a rejected remainder (the grouping shapes IronCalc mis-groups, and the doubled-quote \
         codes); one-sided means either the predicate or the generator has stopped discriminating",
        corpus.len(),
    );
}

/// Each carve-out must stay a *carve-out*: a named, characterised defect that suppresses a handful
/// of pairs, not a blanket that quietly swallows new disagreements as the corpus grows. This test
/// reports the suppression counts over BOTH corpora and fails if any one of them runs away.
#[test]
fn the_carve_outs_suppress_only_a_handful_of_pairs() {
    for (corpus, what) in [
        (
            FIXTURE_CODES
                .iter()
                .chain(COMMON_CODES)
                .map(|c| c.to_string())
                .collect::<Vec<_>>(),
            "grounded corpus",
        ),
        (generated_corpus(), "generated shape space"),
    ] {
        let (mut sign, mut rounding, mut empty, mut agree, mut total) = (0, 0, 0, 0, 0);
        for code in &corpus {
            if !renders_faithfully(code) || is_general(code) {
                continue;
            }
            for &value in VALUES {
                total += 1;
                let chart = apply_number_format(code, value);
                let cell = ironcalc(code, value);
                // `is_ironcalc_sign_bug` fires on IronCalc's *reconstructed* `is_negative`
                // (`ironcalc_keeps_the_sign`) rather than on the shape of the two strings. That
                // reconstruction is only worth anything if it is right, so pin it against the
                // measured answer on every pair the gate sees. A fork fix that moves the E6 band
                // fails here first, with a pointer to the line that has to change.
                // (An empty code never reaches the numeric path — `#VALUE!` carries no sign — so it
                // is excluded here and pinned by `is_empty_code_artifact` instead.)
                assert!(
                    code.trim().is_empty()
                        || ironcalc_keeps_the_sign(code, value) == cell_is_signed(code, value),
                    "the reconstruction of IronCalc's `is_negative` (format.rs:438 + :481) \
                     disagrees with what IronCalc actually did for code {code:?} value {value} \
                     (cell {cell:?}) — `is_ironcalc_sign_bug` is no longer anchored to the defect \
                     it names",
                );
                if chart == cell {
                    agree += 1;
                    continue;
                }
                match carve_out(code, value, &chart, &cell) {
                    Some("empty-code `#VALUE!`") => empty += 1,
                    Some("IronCalc sign bug (E6)") => sign += 1,
                    Some("IronCalc rounding defect (E7)") => rounding += 1,
                    _ => {}
                }
            }
        }
        println!(
            "{what}: carve-outs over {total} in-subset pairs: {agree} agree, \
             {sign} IronCalc sign bug (E6), {rounding} IronCalc rounding defect (E7), \
             {empty} empty-code `#VALUE!`",
        );
        assert!(
            sign > 0 && rounding > 0,
            "a defect carve-out fired ZERO times over the {what} (sign {sign}, rounding \
             {rounding}) — either the fork landed the fix (delete the carve-out and let the gate \
             tighten) or the corpus no longer reaches the defect it documents",
        );
        assert!(
            sign + rounding + empty < total / 4,
            "the carve-outs now suppress {} of {total} in-subset pairs of the {what} — that is no \
             longer a carve-out for a characterised defect, it is a hole. Re-derive each predicate \
             before widening it.",
            sign + rounding + empty,
        );
    }
    // The empty code only exists in the grounded corpus, so pin it there specifically rather than
    // letting the loop's `> 0` check pass on the generated space.
    assert!(
        is_empty_code_artifact("", &ironcalc("", 1.0)),
        "IronCalc no longer answers `#VALUE!` for an empty format code — the carve-out is dead",
    );
}
