//! **Number-format application** for axis tick and (later) data labels — a *bounded* subset of
//! the OOXML/ECMA-376 number-format grammar (charts/functional_spec §4 P2; coverage-matrix §D
//! `c:numFmt`).
//!
//! The value axis carries a `formatCode` from `<c:numFmt formatCode="…">`; [`apply_number_format`]
//! turns a numeric tick into its label text under that code. The full number-format engine (all
//! sections, conditionals, dates, scientific, fractions) is **P12**; this phase handles the
//! everyday chart-axis cases — General, percent, thousands grouping, fixed decimals, and a
//! currency/text affix — and **falls back to general formatting** for anything it does not parse,
//! so an unknown code degrades to a readable number rather than misformatting or panicking.

use crate::format_number;

/// Format `value` under an OOXML `formatCode`. Empty or `General` (case-insensitive) uses the
/// crate's general number formatting; otherwise the first `;`-section is parsed for an affix,
/// thousands grouping, decimal places, and a percent scale (see the module docs for the supported
/// subset). Unsupported constructs (scientific, dates, fractions, section conditionals) fall back
/// to general formatting.
pub fn apply_number_format(code: &str, value: f64) -> String {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("General") {
        return format_number(value);
    }

    // Only the positive section governs axis ticks; the negative/zero sections are P12.
    let section = code.split(';').next().unwrap_or(code);
    let Some(spec) = FormatSpec::parse(section) else {
        return format_number(value);
    };

    let scaled = if spec.percent { value * 100.0 } else { value };
    let mut out = String::new();
    if scaled < 0.0 {
        out.push('-');
    }
    out.push_str(&spec.prefix);
    out.push_str(&format_magnitude(
        scaled.abs(),
        spec.decimals,
        spec.grouping,
    ));
    out.push_str(&spec.suffix);
    if spec.percent {
        out.push('%');
    }
    out
}

/// The pieces of a single number-format section we honor.
struct FormatSpec {
    /// Literal text before the number (e.g. a currency symbol).
    prefix: String,
    /// Literal text after the number (excluding a trailing `%`, handled separately).
    suffix: String,
    /// Digits after the decimal point.
    decimals: usize,
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
        let decimals = numeric
            .split_once('.')
            .map(|(_, frac)| frac.chars().filter(|c| placeholders.contains(*c)).count())
            .unwrap_or(0);

        // Prefix / suffix are the literal text around the numeric run, minus quotes, escapes, and
        // the percent sign (appended separately).
        let prefix = literal(&cleaned[..first]);
        let suffix = literal(&cleaned[last + 1..]).replace('%', "");

        Some(Self {
            prefix,
            suffix,
            decimals,
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

/// Format a non-negative magnitude with `decimals` fractional digits and optional thousands
/// grouping of the integer part.
fn format_magnitude(value: f64, decimals: usize, grouping: bool) -> String {
    let fixed = format!("{value:.decimals$}");
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (fixed.as_str(), None),
    };
    let int_out = if grouping {
        group_thousands(int_part)
    } else {
        int_part.to_string()
    };
    match frac_part {
        Some(f) => format!("{int_out}.{f}"),
        None => int_out,
    }
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
    }

    #[test]
    fn unsupported_codes_fall_back_to_general() {
        // Date and scientific are out of the P6 subset — a readable number, not a misformat.
        assert_eq!(apply_number_format("yyyy-mm-dd", 45000.0), "45000");
        assert_eq!(apply_number_format("0.00E+00", 12345.0), "12345");
    }
}
