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
/// contain — the subset `chart-model` claims to render exactly.
///
/// The `#` **optional-digit** family (`0.##`, `#,##0.##`, `#,##0.0#`, `0.###`, `#,###`) is here
/// because it was missing when the gate was first written, and its absence hid a real
/// `chart-model` defect: `FormatSpec::parse` counted `#` and `0` identically, so `#,##0.0#` on 1.5
/// padded to `"1.50"` while the cell read `"1.5"` — with the chart classified Faithful and drawn
/// with no badge. The whitespace-padded `"0 "` is here for the same reason (the applier used to
/// `trim()` the code and silently drop the literal padding the cell renders).
///
/// `"0.0"` is here because it is the only single-decimal, non-percent code in the corpus, and it is
/// what makes the value `0.96` reach IronCalc's **lost fractional carry** defect (`"0.0"` on 0.96
/// displays `0.0`) — without it that half of E7 would be characterised but never exercised.
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
];

/// Codes deliberately **outside** the bounded subset. Not asserted — they exist so the
/// informational report shows how wide the disclosed (badged) gap actually is.
const OUT_OF_SUBSET_CODES: &[&str] = &[
    "0.00E+00",
    "yyyy-mm-dd",
    "mm/dd/yyyy",
    "h:mm:ss",
    "#,##0.00;[Red](#,##0.00)",
    "[<100]0;0.0",
    "# ?/?",
    "0.00_);(0.00)",
];

/// Values chosen to hit the places two independent implementations diverge: sign handling, the
/// rounding half-way boundary, magnitudes that cross grouping and exponent thresholds.
///
/// `0.45`, `-0.46` and `0.96` are in the **E7 band** — the values IronCalc's double-rounding
/// corrupts (see [`is_ironcalc_rounding_defect`]). They are here because the previous corpus
/// contained nothing in that band except `±0.5`, which made a one-point carve-out look sufficient.
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
    0.001,  // rounds to zero at 2dp — the negative twin is the DOLLAR-style trap
    -0.001, //
    0.45,   // E7: displays as "1" under code `0` (pre-rounded to 1 significant digit first)
    -0.46,  // E7, negative twin
    0.96,   // E7: displays as "0.0" under code `0.0` (the fractional carry is dropped)
    1_000_000.0,
    -1_000_000.0,
    1e-7,
    1e15,
    0.3333333333333333,
];

/// The format's fractional-digit count and percent scale — the two things IronCalc's rounding path
/// is parameterised by, and which [`is_ironcalc_rounding_defect`] needs in order to locate the
/// defective band. Mirrors `chart-model`'s own parse closely enough for the corpus (first section,
/// placeholders between the first and last `0`/`#`).
fn code_precision(code: &str) -> (usize, bool) {
    let section = code.split(';').next().unwrap_or(code);
    let percent = section.contains('%');
    let decimals = match (section.find(['0', '#']), section.rfind(['0', '#'])) {
        (Some(first), Some(last)) => section[first..=last]
            .split_once('.')
            .map(|(_, frac)| frac.chars().filter(|c| *c == '0' || *c == '#').count())
            .unwrap_or(0),
        _ => 0,
    };
    (decimals, percent)
}

/// The three carve-outs below are the *only* permitted disagreements, and each names a specific,
/// characterised defect rather than waving the difference away. Everything else must match exactly.
///
/// **1. IronCalc drops the minus sign on small negatives** (`GAPS.md` E6). `format_number` computes
/// `is_negative = value < -(10^-precision)` **after** pre-rounding, so the sign is dropped whenever
/// `|value| <= 10^-decimals` — a cell formatted `#,##0` holding **-1 displays "1"**, and `0.00`
/// holding -0.005 displays "0.01". (Not `1.5 × 10^-decimals`, as this comment and `GAPS.md` used to
/// say: that expression is only right at `decimals = 0`. Measured thresholds: `0` → ~1.05, `0.0` →
/// ~0.105, `0.00` → ~0.0101, `0.000` → ~0.001.) Verified end-to-end through the real app (worker +
/// `SetStylePath(NumFmt)` + publication), not just through this helper. `General` is unaffected (a
/// different code path in IronCalc), and larger magnitudes are fine: -1.5 renders "-2" and -1234.5
/// renders "-1,234".
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
fn is_ironcalc_sign_bug(value: f64, chart: &str, cell: &str) -> bool {
    value < 0.0
        && chart.starts_with('-')
        && !cell.contains('-')
        && chart[1..] == *cell
        && chart[1..].contains(|c: char| c.is_ascii_digit() && c != '0')
}

/// **2. IronCalc's rounding is not a rounding rule** (`GAPS.md` E7). `formatter/format.rs` does not
/// round once; it rounds in pieces, and the pieces disagree:
///
/// 1. it pre-rounds with `to_precision(value, precision + integer_digits)` — round to that many
///    **significant** digits, via Rust's `{:.*e}`, which is half-to-**even**; then
/// 2. it renders the integer part with `value_abs.round()` when `precision == 0` (half **away from
///    zero**) or `value_abs.floor()` plus a separately-rounded fractional string when
///    `precision > 0`.
///
/// Two distinct corruptions fall out, and **both hit positives** — this is not, as the carve-out
/// previously claimed, "half-to-even everywhere except the single value ±0.5":
///
/// - **Double rounding, `decimals == 0`, `|v| < 1`.** `floor(|v|)` prints as `"0"`, so step 1 keeps
///   just **one** significant digit: `0.45`, `0.46`, `0.49` all become `0.5`, which step 2 then
///   rounds away from zero to `"1"`. The correct answer is `"0"`. The band is `|v| ∈ [0.45, 0.5]`,
///   whose endpoint `0.5` is the single value the old predicate tested — it looked sufficient only
///   because `VALUES` contained nothing else in the band.
/// - **Lost fractional carry, `decimals >= 1`, `|v| < 1`.** The integer part is `floor(|v|) = 0`
///   while `get_fract_part` rounds the fraction separately and slices it as `"0.ddd"[2..]`; when the
///   fraction rounds up to `1.0` the slice is empty and the carry is simply discarded. So `0.96`
///   under `0.0` displays **`"0.0"`**, and `-0.96` displays `"-0.0"`.
///
/// For `|v| >= 1` step 1 rounds at exactly the rendered precision, so IronCalc lands on half-to-even
/// — which is why `2.5 → "2"`, `4.5 → "4"`, `1234.5 → "1234"`. Excel is half-away-from-zero
/// throughout (`2.5 → 3`), so **IronCalc and `chart-model` are both wrong there, in the same
/// direction**; `chart-model`'s `{:.n}` is pinned to IronCalc's behaviour on purpose so the axis
/// matches the cell beside it. A fork fix must therefore change more than `0.5`, and when it lands,
/// half-away-from-zero becomes correct in `chart-model` too (see the comment on
/// `numfmt::format_magnitude`).
///
/// The predicate takes the **code**, not just the value, because the band depends on the format's
/// decimal count and percent scale — `0%` reaches the `decimals == 0` band at value `0.005`
/// (0.005 × 100 = 0.5). The old predicate ignored `code` entirely while its doc claimed otherwise.
fn is_ironcalc_rounding_defect(code: &str, value: f64) -> bool {
    let (decimals, percent) = code_precision(code);
    let magnitude = if percent { value * 100.0 } else { value }.abs();
    if decimals == 0 {
        // Pre-rounded to one significant digit, then rounded away from zero.
        return (0.45..=0.5).contains(&magnitude);
    }
    // The fraction rounds up to 1.0 and the carry into the (zero) integer part is dropped.
    magnitude < 1.0
        && format!("{magnitude:.*}", decimals)
            .parse::<f64>()
            .is_ok_and(|rounded| rounded >= 1.0)
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

/// **The gate.** Inside the faithful subset, for every code but `General`, the chart label and the
/// cell must be byte-identical. (`General` has its own test below, and the module docs argue why it
/// is excluded rather than fixed or badged.)
#[test]
fn chart_and_cell_agree_on_every_faithfully_rendered_code() {
    let mut disagreements: Vec<String> = Vec::new();

    for code in FIXTURE_CODES.iter().chain(COMMON_CODES) {
        if !renders_faithfully(code) || is_general(code) {
            continue;
        }
        for &value in VALUES {
            let chart = apply_number_format(code, value);
            let cell = ironcalc(code, value);
            // `is_empty_code_artifact` is tested FIRST: an empty code never reaches a numeric
            // format path at all, so the other two predicates have nothing to say about it.
            if chart == cell
                || is_empty_code_artifact(code, &cell)
                || is_ironcalc_sign_bug(value, &chart, &cell)
                || is_ironcalc_rounding_defect(code, value)
            {
                continue;
            }
            disagreements.push(format!(
                "    code {code:?} value {value}\n      chart axis: {chart:?}\n      cell:       {cell:?}"
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "chart-model claims to render these codes faithfully, but the axis label and the cell it \
         measures disagree — the chart is drawn WITHOUT a fidelity badge while showing a \
         different string than its data:\n{}\n\
         Fix `chart-model::numfmt` to match IronCalc (the cell is the reference the user compares \
         against), or — if a code cannot be made to agree without growing the reimplementation — \
         make `renders_faithfully` return false for it, so the chart degrades honestly instead. \
         If the disagreement is IRONCALC's fault, fix it in the fork (CLAUDE.md §Engine) rather \
         than copying it into the charts.",
        disagreements.join("\n"),
    );
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
#[test]
fn general_differs_from_the_cell_only_by_rounding() {
    for &value in VALUES {
        let chart = apply_number_format("General", value);
        let cell = ironcalc("General", value);
        if chart == cell {
            continue;
        }
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
}

/// Each carve-out must stay a *carve-out*: a named, characterised defect that suppresses a handful
/// of pairs, not a blanket that quietly swallows new disagreements as the corpus grows. This test
/// reports the suppression counts and fails if any one of them runs away.
#[test]
fn the_carve_outs_suppress_only_a_handful_of_pairs() {
    let (mut sign, mut rounding, mut empty, mut agree, mut total) = (0, 0, 0, 0, 0);
    for code in FIXTURE_CODES.iter().chain(COMMON_CODES) {
        if !renders_faithfully(code) || is_general(code) {
            continue;
        }
        for &value in VALUES {
            total += 1;
            let chart = apply_number_format(code, value);
            let cell = ironcalc(code, value);
            if chart == cell {
                agree += 1;
            } else if is_empty_code_artifact(code, &cell) {
                empty += 1;
            } else if is_ironcalc_sign_bug(value, &chart, &cell) {
                sign += 1;
            } else if is_ironcalc_rounding_defect(code, value) {
                rounding += 1;
            }
        }
    }
    println!(
        "carve-outs over {total} in-subset pairs: {agree} agree, \
         {sign} IronCalc sign bug (E6), {rounding} IronCalc rounding defect (E7), \
         {empty} empty-code `#VALUE!`",
    );
    assert!(
        sign > 0 && rounding > 0 && empty > 0,
        "a carve-out fired ZERO times (sign {sign}, rounding {rounding}, empty {empty}) — either \
         the fork landed the fix (delete the carve-out and let the gate tighten) or the corpus no \
         longer reaches the defect it documents",
    );
    assert!(
        sign + rounding + empty < total / 4,
        "the carve-outs now suppress {} of {total} in-subset pairs — that is no longer a carve-out \
         for a characterised defect, it is a hole. Re-derive each predicate before widening it.",
        sign + rounding + empty,
    );
}
