//! **Number-format application** for axis ticks (P6) and data labels (P12) — a *bounded* subset
//! of the OOXML/ECMA-376 number-format grammar (charts/functional_spec §4 P2; coverage-matrix §D
//! `c:numFmt` + §F `c:dLbls/c:numFmt`).
//!
//! A `formatCode` comes from `<c:numFmt formatCode="…">` on an axis or inside a `c:dLbls`;
//! [`apply_number_format`] turns a numeric tick/value into its label text under that code. The
//! supported subset is the everyday chart cases — General, percent, thousands grouping, required
//! (`0`) and optional (`#`) decimal placeholders, and a currency/text affix — and it **falls back
//! to general formatting** for anything it does not parse (dates, scientific, fractions, section
//! conditionals), so an unknown code
//! degrades to a readable number rather than misformatting or panicking. [`renders_faithfully`]
//! reports whether a code is inside that subset — the fidelity accessor uses it so a code we
//! render exactly is Faithful while one we fall back on still degrades.

use crate::format_number;

/// Format `value` under an OOXML `formatCode`. Empty or `General` (case-insensitive) uses the
/// crate's general number formatting; otherwise the first `;`-section is parsed for an affix,
/// thousands grouping, decimal places, and a percent scale (see the module docs for the supported
/// subset). Unsupported constructs (scientific, dates, fractions, section conditionals) fall back
/// to general formatting.
pub fn apply_number_format(code: &str, value: f64) -> String {
    // Only the *General* test trims: leading/trailing whitespace elsewhere in a format code is
    // literal padding that Excel and IronCalc both render (`"0 "` on 1 shows `1 `), so the parsed
    // section keeps it. Trimming here used to drop that character silently.
    let trimmed = code.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("General") {
        return format_number(value);
    }

    // Only the positive section governs the rendered text; the negative/zero/text sections are
    // outside the bounded subset (a multi-section code is not `renders_faithfully`).
    let section = code.split(';').next().unwrap_or(code);
    let Some(spec) = FormatSpec::parse(section) else {
        return format_number(value);
    };

    let scaled = if spec.percent { value * 100.0 } else { value };
    let magnitude = format_magnitude(scaled.abs(), &spec);
    let mut out = String::new();
    // The sign is decided from the **rendered magnitude**, not from the raw value: a negative that
    // rounds to zero at the format's precision must print unsigned, because that is what Excel
    // shows (`-0.001` under `0.00` is `0.00`, not `-0.00`) and what IronCalc shows. Testing
    // `scaled < 0.0` *before* rounding is what made this emit `-0`, `-0.00`, `-$0.00`, `-0%` —
    // a chart-model defect that the differential gate's sign carve-out hid for a while.
    if scaled < 0.0 && has_significant_digit(&magnitude) {
        out.push('-');
    }
    out.push_str(&spec.prefix);
    out.push_str(&magnitude);
    out.push_str(&spec.suffix);
    if spec.percent {
        out.push('%');
    }
    out
}

/// Whether a rendered magnitude carries any non-zero digit — i.e. whether it is a number a minus
/// sign should be attached to. `"0.00"`, `"0"`, `""` and `"."` are all "rendered zero".
fn has_significant_digit(magnitude: &str) -> bool {
    magnitude.chars().any(|c| c.is_ascii_digit() && c != '0')
}

/// Whether [`apply_number_format`] renders `code` **exactly as authored** (rather than
/// mis-rendering or falling back to general formatting). The fidelity accessor
/// ([`source_fidelity`](crate::source_fidelity)) uses this so a chart whose only `c:numFmt` codes
/// are ones we render is Faithful, while a code we only approximate stays Degraded (⚠ badge).
///
/// `true` for the supported subset: empty / `General`, and a **single**, non-conditional section
/// the applier parses exactly (percent, thousands **grouping**, required (`0`) **and optional
/// (`#`)** digit placeholders, currency/text affix, literal whitespace padding). `false` for codes
/// outside it — a **multi-section** code (`;`, whose negative/zero/text sections the applier
/// drops), a **conditional** section (`[<`/`[>`/`[=`, which selects a format
/// by value), a **scaling comma** (a `,` after the last digit placeholder — Excel's ÷1000-per-comma
/// "in thousands / millions" scale, which the applier silently ignores, e.g. `#,##0,` → `1,235`
/// but we'd emit `1,234,567`), an **unhandled control/format char** (`_` column-align, `*`
/// fill-repeat, `?` digit-align — none rendered), or a construct the parser rejects (dates,
/// scientific, fractions). The check is deliberately stricter than [`FormatSpec::parse`], which
/// accepts several of these and renders them wrong — being *called* Faithful is what would hide the
/// mis-render, so this gate rejects them even though the applier still produces (approximate) output.
pub fn renders_faithfully(code: &str) -> bool {
    // Only the General test trims — everything below inspects the code the applier actually
    // parses, whitespace padding included (see [`apply_number_format`]).
    let trimmed = code.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("General") {
        return true;
    }
    // The applier honors only the positive section, so a multi-section code renders its negative /
    // zero / text values differently than authored.
    if code.contains(';') {
        return false;
    }
    // A conditional section ([>=100], [<0], …) changes which format applies by value; the applier
    // strips the bracket and ignores the condition, so it is not an exact render.
    if code.contains("[>") || code.contains("[<") || code.contains("[=") {
        return false;
    }
    // A scaling comma or an unhandled control char (`_`/`*`/`?`) parses but mis-renders — reject
    // it here so it is not called Faithful.
    if control_body_is_unrenderable(code) {
        return false;
    }
    FormatSpec::parse(code).is_some()
}

/// Whether the format's "control body" — the code with bracket tokens (`[…]`), quoted literals
/// (`"…"`), and `\`-escapes removed — carries a construct [`apply_number_format`] does **not**
/// render exactly even though [`FormatSpec::parse`] accepts it:
/// - a **scaling comma**: a `,` *after* the last `0`/`#` digit placeholder. Excel divides the value
///   by 1000 per such comma (`#,##0,` = thousands, `#,##0,,` = millions); the applier drops it and
///   shows the unscaled number (1000×/1e6× too big). A comma *before* the last placeholder is normal
///   thousands **grouping**, which the applier does render — so this only fires on the trailing scale.
/// - an **unhandled control/format char**: `_` (column-width align), `*` (fill-repeat), or `?`
///   (digit-align space) — none of which the applier renders.
///
/// Stripping quoted literals/escapes first keeps a benign comma or `*`/`_`/`?` *inside* a quoted
/// suffix (e.g. `0" (a*b)"`) from being mistaken for a control construct.
fn control_body_is_unrenderable(code: &str) -> bool {
    let mut body = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            // Skip a bracket token ([Red], [$-409], …) — brackets don't nest in a number format.
            '[' => {
                for bracket_char in chars.by_ref() {
                    if bracket_char == ']' {
                        break;
                    }
                }
            }
            // Skip a quoted literal — its characters are literal text, not format controls.
            '"' => {
                for quote_char in chars.by_ref() {
                    if quote_char == '"' {
                        break;
                    }
                }
            }
            // Skip the escaped character (a literal).
            '\\' => {
                chars.next();
            }
            _ => body.push(c),
        }
    }

    if body.contains(['_', '*', '?']) {
        return true;
    }
    match body.rfind(['0', '#']) {
        // A comma after the last digit placeholder is a ÷1000 scaling comma. Both placeholder chars
        // (`0`/`#`) are single-byte ASCII, so `+ 1` is a valid slice boundary.
        Some(last_placeholder) => body[last_placeholder + 1..].contains(','),
        // No digit placeholder at all — `FormatSpec::parse` will reject it (date/text) anyway.
        None => false,
    }
}

/// The pieces of a single number-format section we honor.
struct FormatSpec {
    /// Literal text before the number (e.g. a currency symbol).
    prefix: String,
    /// Literal text after the number (excluding a trailing `%`, handled separately).
    suffix: String,
    /// **Maximum** digits after the decimal point — every `0`/`#` placeholder in the fractional
    /// run. The value is rounded to this many places.
    decimals: usize,
    /// **Minimum** digits after the decimal point — the placeholders up to and including the last
    /// required (`0`) one. Fractional digits beyond this are `#` *optional* digits and are dropped
    /// when they are trailing zeros, which is what makes `#,##0.0#` print `1.5`, not `1.50`.
    min_decimals: usize,
    /// Whether the section has a decimal separator inside its numeric run. Excel (and IronCalc)
    /// keep the separator even when every fractional digit is suppressed — `0.##` on 1 is `1.`.
    decimal_point: bool,
    /// Whether the integer run is made only of optional (`#`) placeholders, in which case an
    /// integer part that rounds to zero renders as nothing at all (`#,###` on 0 is the empty
    /// string; `#.##` on 0.5 is `.5`).
    integer_optional: bool,
    /// Whether to group the integer part in thousands.
    grouping: bool,
    /// Whether the code is a percentage (scales the value by 100 and appends `%`).
    percent: bool,
}

impl FormatSpec {
    /// Parse one format section into a [`FormatSpec`], or `None` if it contains a construct outside
    /// the supported subset (scientific / date / fraction), signalling the caller to fall back.
    fn parse(section: &str) -> Option<Self> {
        // Strip bracket tokens ([Red], [>=100], [$-409], …): color/condition/locale hints we don't
        // apply. A `[$sym-locale]` currency token keeps its symbol (the text before the `-`).
        let mut cleaned = String::new();
        let mut chars = section.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c == '[' {
                let mut token = String::new();
                chars.next();
                for tc in chars.by_ref() {
                    if tc == ']' {
                        break;
                    }
                    token.push(tc);
                }
                if let Some(sym) = token.strip_prefix('$') {
                    // [$€-407] → keep "€" (currency symbol up to the locale separator).
                    cleaned.push_str(sym.split('-').next().unwrap_or(""));
                }
            } else {
                cleaned.push(c);
                chars.next();
            }
        }

        // Scientific / fraction are out of the P6 subset → fall back to general.
        let lower = cleaned.to_ascii_lowercase();
        if lower.contains("e+") || lower.contains("e-") || cleaned.contains('/') {
            return None;
        }
        // A date/time code has no digit placeholder to drive; general handles it more sensibly.
        let has_placeholder = cleaned.contains('0') || cleaned.contains('#');
        if !has_placeholder {
            return None;
        }

        let percent = cleaned.contains('%');
        let placeholders = "0#";

        // The numeric run spans the first to the last digit placeholder.
        let first = cleaned.find(|c| placeholders.contains(c))?;
        let last = cleaned.rfind(|c| placeholders.contains(c))?;
        let numeric = &cleaned[first..=last];
        let grouping = numeric.contains(',');
        let (integer_run, fractional_run) = match numeric.split_once('.') {
            Some((int_run, frac)) => (int_run, Some(frac)),
            None => (numeric, None),
        };
        // `0` is a REQUIRED digit, `#` an OPTIONAL one (ECMA-376 §18.8.31). Counting them
        // identically is what made `#,##0.0#` pad 1.5 to "1.50" where Excel and IronCalc show
        // "1.5". `decimals` is the rounding precision (all placeholders); `min_decimals` stops at
        // the last required one, and anything past it is trimmed if it is a trailing zero.
        let decimal_point = fractional_run.is_some();
        let (decimals, min_decimals) = match fractional_run {
            Some(frac) => {
                let digits: Vec<char> =
                    frac.chars().filter(|c| placeholders.contains(*c)).collect();
                let required = digits.iter().rposition(|c| *c == '0').map_or(0, |i| i + 1);
                (digits.len(), required)
            }
            None => (0, 0),
        };
        // An all-`#` integer run suppresses a zero integer part entirely, the way Excel does.
        let integer_optional = !integer_run.contains('0');

        // Prefix / suffix are the literal text around the numeric run, minus quotes, escapes, and
        // the percent sign (appended separately).
        let prefix = literal(&cleaned[..first]);
        let suffix = literal(&cleaned[last + 1..]).replace('%', "");

        Some(Self {
            prefix,
            suffix,
            decimals,
            min_decimals,
            decimal_point,
            integer_optional,
            grouping,
            percent,
        })
    }
}

/// Extract the literal text from a format fragment: unwrap `"…"` quotes, honor `\x` escapes, and
/// drop the format placeholder punctuation (`0 # , . %`) so only genuine literal characters (a
/// currency symbol, a unit) survive.
fn literal(fragment: &str) -> String {
    let mut out = String::new();
    let mut chars = fragment.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for qc in chars.by_ref() {
                    if qc == '"' {
                        break;
                    }
                    out.push(qc);
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            '0' | '#' | ',' | '.' | '%' | '?' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Format a non-negative magnitude under `spec`: round to `spec.decimals` places, trim optional
/// (`#`) trailing zeros back to `spec.min_decimals`, group the integer part if asked, and suppress
/// a zero integer part when the integer run is all-`#`.
fn format_magnitude(value: f64, spec: &FormatSpec) -> String {
    // ROUNDING: `{:.n}` is half-to-EVEN, and that is **pinned to IronCalc's current behaviour, not
    // to correctness**. Excel rounds half AWAY from zero (2.5→3, 1234.5→1235); IronCalc does not
    // implement a single rule at all — `formatter/format.rs` pre-rounds through
    // `to_precision(value, precision + integer_digits)`, whose `{:.*e}` is half-to-even, and only
    // then rounds/floors again, so it lands on half-to-even for |v| ≥ 1 while a separate
    // double-rounding path corrupts |v| < 1 (`0.45`, `0.46`, `0.49` all display as `1` under code
    // `0`). Half-to-even here therefore matches the *cells beside the chart* on 2.5→"2",
    // 1234.5→"1234", 0.125→"0.12" — which is why switching to half-away-from-zero made the
    // differential gate strictly worse when it was tried.
    //
    // It also means both sides are wrong versus Excel on those ties. That is deliberate and
    // TEMPORARY: the IronCalc rounding defect is `GAPS.md` E7 and is a fork fix. **When the fork
    // lands it, half-away-from-zero becomes the correct rule here and this line should be revisited
    // together with the gate's `is_ironcalc_rounding_defect` carve-out** — do not treat the
    // half-to-even choice as settled on its own merits.
    let fixed = format!("{value:.*}", spec.decimals);
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i, f),
        None => (fixed.as_str(), ""),
    };

    // `#` placeholders past the last required `0` are optional: drop them when they are zeros.
    let mut frac = frac_part;
    while frac.len() > spec.min_decimals && frac.ends_with('0') {
        frac = &frac[..frac.len() - 1];
    }

    let mut out = if spec.integer_optional && int_part == "0" {
        String::new()
    } else if spec.grouping {
        group_thousands(int_part)
    } else {
        int_part.to_string()
    };
    // The separator is a literal in the format string: Excel and IronCalc both emit it even when
    // every fractional digit was optional and got trimmed (`0.##` on 1 → `1.`).
    if spec.decimal_point {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// Insert `,` thousands separators into a run of integer digits (ASCII digits only, as produced
/// by `format!("{:.*}")`).
fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        // A separator precedes every position whose distance from the right is a multiple of 3.
        if i != 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_and_empty_use_plain_number() {
        assert_eq!(apply_number_format("General", 42.0), "42");
        assert_eq!(apply_number_format("general", 42.0), "42");
        assert_eq!(apply_number_format("", 42.5), "42.5");
    }

    #[test]
    fn percent_scales_by_100() {
        assert_eq!(apply_number_format("0%", 0.25), "25%");
        assert_eq!(apply_number_format("0.0%", 0.25), "25.0%");
        assert_eq!(apply_number_format("0.00%", 0.4), "40.00%");
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(apply_number_format("#,##0", 1234567.0), "1,234,567");
        assert_eq!(apply_number_format("#,##0.00", 1234.5), "1,234.50");
        assert_eq!(apply_number_format("#,##0", 12.0), "12");
        assert_eq!(apply_number_format("#,##0", 1000.0), "1,000");
    }

    #[test]
    fn fixed_decimals_without_grouping() {
        assert_eq!(apply_number_format("0.00", 9.876), "9.88");
        assert_eq!(apply_number_format("0.0", 9.876), "9.9");
        assert_eq!(apply_number_format("0", 3.7), "4");
    }

    #[test]
    fn currency_prefix() {
        assert_eq!(apply_number_format("$#,##0", 2500.0), "$2,500");
        assert_eq!(apply_number_format("$#,##0.00", 2500.5), "$2,500.50");
        assert_eq!(apply_number_format("\"$\"#,##0", 1000.0), "$1,000");
        assert_eq!(apply_number_format("[$€-407]#,##0", 1000.0), "€1,000");
    }

    #[test]
    fn suffix_literal_is_kept() {
        assert_eq!(apply_number_format("0\" kg\"", 5.0), "5 kg");
    }

    #[test]
    fn negatives_get_a_leading_sign() {
        assert_eq!(apply_number_format("#,##0", -1500.0), "-1,500");
        assert_eq!(apply_number_format("0.0%", -0.05), "-5.0%");
        // The sign survives as soon as the rendered magnitude has a digit to carry it.
        assert_eq!(apply_number_format("0.00", -0.005), "-0.01");
        assert_eq!(apply_number_format("0.000", -0.001), "-0.001");
    }

    /// A negative that **rounds to zero at the format's precision** must print unsigned — Excel
    /// shows `0.00` for -0.001 under `0.00`, never `-0.00`, and so does IronCalc. Deciding the sign
    /// from the raw value before rounding emitted `-0`, `-0.00`, `-$0.00`, `-0%`, `-0.00 kg`; the
    /// F3a differential gate's sign carve-out was broad enough to hide all of them, and this crate
    /// had no small-negative test of its own to catch it.
    #[test]
    fn negatives_that_round_to_zero_print_unsigned() {
        assert_eq!(apply_number_format("0", -0.4), "0");
        assert_eq!(apply_number_format("#,##0", -0.005), "0");
        assert_eq!(apply_number_format("0.00", -0.001), "0.00");
        assert_eq!(apply_number_format("$#,##0.00", -0.001), "$0.00");
        assert_eq!(apply_number_format("0%", -0.001), "0%");
        assert_eq!(apply_number_format("0.00\" kg\"", -0.0001), "0.00 kg");
        assert_eq!(apply_number_format("0.000", -1e-7), "0.000");
        // Zero itself is unsigned whichever way it arrives.
        assert_eq!(apply_number_format("0.00", -0.0), "0.00");
    }

    /// `#` is an **optional** digit: trailing zeros it would fill are suppressed down to the last
    /// required `0`. Counting `#` and `0` identically padded `1.5` to `"1.50"` under `#,##0.0#`
    /// while the cell beside it read `1.5`.
    #[test]
    fn optional_hash_digits_suppress_trailing_zeros() {
        assert_eq!(apply_number_format("#,##0.0#", 1.5), "1.5");
        assert_eq!(apply_number_format("#,##0.0#", 1.25), "1.25");
        assert_eq!(apply_number_format("#,##0.0#", 1.0), "1.0");
        assert_eq!(apply_number_format("0.##", 0.5), "0.5");
        assert_eq!(apply_number_format("0.##", 0.125), "0.12");
        assert_eq!(apply_number_format("0.###", 0.3333333333333333), "0.333");
        // Every fractional digit optional and all of them zero: the separator is a literal in the
        // format string, so Excel and IronCalc still emit it.
        assert_eq!(apply_number_format("0.##", 1.0), "1.");
        assert_eq!(apply_number_format("#,##0.##", 1000000.0), "1,000,000.");
    }

    /// An all-`#` integer run has no required digit, so an integer part that rounds to zero renders
    /// as nothing at all — `#,###` on 0 is blank in Excel, and `#.##` on 0.5 is `.5`.
    #[test]
    fn optional_integer_run_suppresses_a_zero_integer_part() {
        assert_eq!(apply_number_format("#,###", 0.0), "");
        assert_eq!(apply_number_format("#,###", 0.4), "");
        assert_eq!(apply_number_format("#,###", -0.4), "");
        assert_eq!(apply_number_format("#,###", 1234.0), "1,234");
        assert_eq!(apply_number_format("#.##", 0.5), ".5");
        assert_eq!(apply_number_format("#.##", -0.5), "-.5");
        assert_eq!(apply_number_format("#.##", -0.001), ".");
        // A required `0` anywhere in the integer run keeps it.
        assert_eq!(apply_number_format("#,##0", 0.0), "0");
    }

    /// Whitespace in a format code is literal padding the cell renders, so the applier must not
    /// trim it away. (Only the `General`/empty test trims.)
    #[test]
    fn literal_whitespace_padding_is_kept() {
        assert_eq!(apply_number_format("0 ", 1.0), "1 ");
        assert_eq!(apply_number_format("0 ", -1234.5), "-1234 ");
        assert_eq!(apply_number_format("   ", 42.0), "42");
        assert_eq!(apply_number_format("  General  ", 42.0), "42");
    }

    #[test]
    fn unsupported_codes_fall_back_to_general() {
        // Date and scientific are out of the P6 subset — a readable number, not a misformat.
        assert_eq!(apply_number_format("yyyy-mm-dd", 45000.0), "45000");
        assert_eq!(apply_number_format("0.00E+00", 12345.0), "12345");
    }

    #[test]
    fn renders_faithfully_covers_the_supported_subset() {
        // General / empty and the everyday single-section codes render exactly. Normal thousands
        // GROUPING (a comma between placeholders) stays faithful — only a TRAILING scaling comma is
        // rejected (see the reject test).
        for code in [
            "", "General", "general", "0", "0.00", "#,##0", "#,##0.00", "0%", "0.0%", "$#,##0",
            // The optional-digit (`#`) family renders exactly too, now that `#` is honored as an
            // optional placeholder rather than counted as a required one.
            "0.##", "#,##0.##", "#,##0.0#", "0.###", "#,###", "0 ",
        ] {
            assert!(
                renders_faithfully(code),
                "{code:?} should render faithfully"
            );
        }
        // Bracketed currency/locale token whose symbol we keep is still exact.
        assert!(renders_faithfully("[$€-407]#,##0"));
        // A comma inside a quoted suffix is literal text, not a scaling comma → still faithful.
        assert!(renders_faithfully(r#"#,##0" units, ea""#));
    }

    #[test]
    fn renders_faithfully_rejects_codes_we_only_approximate() {
        for code in [
            "yyyy-mm-dd",        // date — no digit placeholder
            "0.00E+00",          // scientific
            "# ?/?",             // fraction
            "#,##0;(#,##0)",     // multi-section (negatives dropped)
            "[Red][>1000]#,##0", // conditional section
            "[<0]0",             // conditional section
            "#,##0,",            // scaling comma → ÷1000 (emits 1,234,567 vs Excel 1,235)
            "#,##0,,",           // scaling comma → ÷1,000,000
            r#"0.0," k""#,       // trailing scaling comma before a quoted suffix
            "0.00_)",            // `_` column-align (unrendered)
            "0.??",              // `?` digit-align (unrendered)
        ] {
            assert!(!renders_faithfully(code), "{code:?} should NOT be faithful");
        }
    }
}
