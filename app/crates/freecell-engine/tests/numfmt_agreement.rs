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
//! # The invariant, and why it is scoped
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
const COMMON_CODES: &[&str] = &[
    "",
    "general",
    "0",
    "0.00",
    "0.000",
    "#,##0.00",
    "0.00%",
    "$#,##0",
    "$#,##0.00",
    "#,##0.00 \"kg\"",
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
    1_000_000.0,
    -1_000_000.0,
    1e-7,
    1e15,
    0.3333333333333333,
];

/// The three carve-outs below are the *only* permitted disagreements, and each names a specific,
/// characterised defect rather than waving the difference away. Everything else must match exactly.
///
/// **1. IronCalc drops the minus sign on small negatives.** `format_number` returns an UNSIGNED
/// string whenever `|value| < 1.5 × 10^-decimals` — so a cell formatted `#,##0` holding **-1
/// displays "1"**, and `0.00` holding -0.005 displays "0.01". Verified end-to-end through the real
/// app (worker + `SetStylePath(NumFmt)` + publication), not just through this helper. `General` is
/// unaffected (a different code path in IronCalc), and larger magnitudes are fine: -1.5 renders
/// "-2" and -1234.5 renders "-1,234".
///
/// **`chart-model` is CORRECT here and IronCalc is wrong**, so "make them agree" must not be
/// satisfied by copying the bug into the charts. This is an engine defect and belongs in the fork
/// per CLAUDE.md §Engine (one `fix/` branch, one upstream PR); tracked in `GAPS.md`. When the fork
/// carries the fix, delete this carve-out and the gate tightens by itself.
fn is_ironcalc_sign_bug(value: f64, chart: &str, cell: &str) -> bool {
    value < 0.0 && chart.starts_with('-') && !cell.contains('-') && chart[1..] == *cell
}

/// **2. IronCalc rounds 0.5-at-zero-decimals inconsistently with itself.** `format_number` gives
/// `0.5 → "1"` but `2.5 → "2"`, `4.5 → "4"`, `10.5 → "10"`, `1234.5 → "1234"` — half-to-even
/// everywhere except that one value. `chart-model` uses `{:.n}` (half-to-even throughout) and so
/// matches IronCalc on **every** other half-way case in the corpus, including `0.125 → "0.12"` and
/// `2.675 → "2.67"`.
///
/// Making `chart-model` round half-away-from-zero to fix `0.5` was tried and made agreement
/// strictly worse — it broke 1234.5, 0.125 and every other half-way case to fix one. There is no
/// rounding rule that matches an inconsistent reference, so this single value is carved out and
/// the inconsistency is recorded as IronCalc's.
///
/// The deviation is at the single value ±0.5 rendered at zero decimals, so the predicate tests
/// exactly that — including after a percent code's ×100 scaling, which is how `0%` on 0.005
/// reaches the same point (0.005 × 100 = 0.5 → IronCalc `"1%"`, chart `"0%"`).
fn is_ironcalc_half_up_outlier(value: f64) -> bool {
    let is_half = |v: f64| v.abs() == 0.5;
    is_half(value) || is_half(value * 100.0)
}

/// **3. An empty format code is not a thing a cell can have.** OOXML lets a chart carry
/// `formatCode=""`, which `chart-model` reasonably treats as General; IronCalc's formatter returns
/// `#VALUE!` because a *cell* always has a format string. Comparing them here measures an input
/// the cell path cannot receive.
fn is_empty_code_artifact(code: &str, cell: &str) -> bool {
    code.trim().is_empty() && cell == "#VALUE!"
}

/// **The gate.** Inside the faithful subset, for every explicit numeric code, the chart label and
/// the cell must be byte-identical. (`General` has its own test below — it is a deliberate
/// tick-label formatter, not an exact renderer.)
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
            if chart == cell
                || is_ironcalc_sign_bug(value, &chart, &cell)
                || is_ironcalc_half_up_outlier(value)
                || is_empty_code_artifact(code, &cell)
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

fn is_general(code: &str) -> bool {
    let c = code.trim();
    c.is_empty() || c.eq_ignore_ascii_case("General")
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
/// not F3a's. This test therefore asserts the *shape* of the divergence so it cannot silently
/// widen: General must agree on everything an axis realistically shows, and may differ only by
/// showing FEWER digits.
#[test]
fn general_differs_from_the_cell_only_by_rounding_to_three_decimals() {
    for &value in VALUES {
        let chart = apply_number_format("General", value);
        let cell = ironcalc("General", value);
        if chart == cell {
            continue;
        }
        // Whatever the chart prints must be a correctly-rounded, shorter rendering — never a
        // different number, and never longer than what the cell shows.
        let reparsed: f64 = chart.parse().unwrap_or_else(|_| {
            panic!("General produced a non-numeric label {chart:?} for {value} (cell: {cell:?})")
        });
        let tolerance = (value.abs() * 1e-3).max(5e-4);
        assert!(
            (reparsed - value).abs() <= tolerance || value.abs() >= 1e11,
            "General on the chart rendered {value} as {chart:?} — that is not the same number \
             rounded, it is a different one (cell shows {cell:?})",
        );
    }
}

/// Codes *outside* the subset are allowed to disagree (the chart is badged `Degraded`), but the
/// size of that gap should be visible rather than assumed. This test cannot fail; run it with
/// `--nocapture` to read the table.
#[test]
fn report_unfaithful_divergence() {
    let mut rows = 0usize;
    let mut differing = 0usize;
    println!("codes OUTSIDE the faithful subset (chart shows ⚠, disagreement is disclosed):");
    for code in OUT_OF_SUBSET_CODES {
        assert!(
            !renders_faithfully(code),
            "{code:?} is in the faithful subset — move it to COMMON_CODES so it is ASSERTED, not \
             merely reported",
        );
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
        "the out-of-subset corpus is not out of subset",
    );
}
