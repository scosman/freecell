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
//!
//! **[`renders_faithfully`] is the specification of what this module must get right, and it is
//! tested as such.** `freecell-engine`'s `tests/numfmt_agreement.rs` *generates* format codes across
//! the shape space the predicate accepts and asserts every one of them against IronCalc — the
//! formatter the cells beside the chart go through. Widening the predicate therefore widens what
//! must agree; a shape it accepts and this module renders differently is a build failure, not
//! something a later reader has to notice.

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

    // Each unquoted `%` multiplies by 100 **and** is rendered as a literal where it stands — that
    // is what IronCalc's formatter does (`Token::Percent` pushes `Literal('%')` and bumps a
    // `percent` counter), and what Excel does. Appending a single `%` at the end instead is what
    // made `0" %"` (an already-in-percent-units number) scale by 100, and `0% ` print `1 %`.
    let scaled = value * 100f64.powi(spec.percent_scale);
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
/// fill-repeat, `?` digit-align, `@` raw-text — none rendered), a **numeric run** carrying anything
/// but `0`/`#`/`,`/`.`, an integer run that is not `#`…`0`…
/// ([`integer_run_shape_is_hash_then_zero`]), a grouping shape IronCalc groups differently, a
/// doubled quote, or a construct the parser rejects (dates, scientific, fractions). The check is
/// deliberately stricter than
/// [`FormatSpec::parse`], which accepts several of these and renders them wrong — being *called*
/// Faithful is what would hide the mis-render, so this gate rejects them even though the applier
/// still produces (approximate) output.
///
/// **This predicate is the corpus's specification, not a list of remembered cases.**
/// `numfmt_agreement.rs::generated_shape_space_agrees_with_the_cell` enumerates the shape space it
/// accepts and asserts agreement with IronCalc across it, so a shape accepted here that we render
/// differently from the cell fails the build. Three rounds of review each found another such shape
/// (`#` optional digits → a quoted/escaped `%` → required leading zeros) because the corpus was
/// hand-written; it is generated now.
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
    // A bracketed currency token is hoisted to the FRONT of the string by IronCalc's formatter
    // (`Formatted::text` starts as the currency char), and its lexer reads exactly one symbol
    // character. So `[$€-407]#,##0` agrees, but a multi-character symbol or a token that is not at
    // the head of the code does not.
    if let Some((head, rest)) = code.split_once("[$") {
        let Some((token, tail)) = rest.split_once(']') else {
            return false;
        };
        let symbol = token.split('-').next().unwrap_or("");
        if !head.is_empty() || symbol.chars().count() != 1 || tail.contains("[$") {
            return false;
        }
    }
    // A doubled quote is where the two lexers genuinely disagree about the grammar: IronCalc's
    // `consume_string` reads `""` as an ESCAPED quote inside one literal (`"%"" kg"` → `%" kg`),
    // while the applier reads it as two adjacent literals (`% kg`). Excel's own convention here is
    // not something we can settle from the spec, so the code degrades honestly rather than picking
    // a side (`GAPS.md` E9).
    if code.contains("\"\"") {
        return false;
    }
    // A scaling comma or an unhandled control char (`_`/`*`/`?`/`@`) parses but mis-renders —
    // reject it here so it is not called Faithful.
    let body = control_body(code);
    if control_body_is_unrenderable(&body) {
        return false;
    }
    let Some(spec) = FormatSpec::parse(code) else {
        return false;
    };
    // The numeric run must be *only* placeholders, grouping commas and one decimal separator.
    // Anything else in it (a space, a `%`, a stray letter) is a literal IronCalc renders **in
    // place**, inside the digits, which the applier cannot do — it only carries a prefix and a
    // suffix. `%0` was the case that mattered: a leading percent sign.
    let (Some(first), Some(last)) = (body.find(['0', '#']), body.rfind(['0', '#'])) else {
        return false;
    };
    let run = &body[first..=last];
    if run.chars().any(|c| !matches!(c, '0' | '#' | ',' | '.')) {
        return false;
    }
    if run.matches('.').count() > 1 {
        return false;
    }
    let (integer_run, fractional_run) = match run.split_once('.') {
        Some((int_run, frac)) => (int_run, frac),
        None => (run, ""),
    };
    // A comma inside the fractional run is not grouping and not a scale; nothing renders it.
    if fractional_run.contains(',') {
        return false;
    }
    // Without an integer placeholder IronCalc never emits the minus sign at all (it hangs the sign
    // off the first *integer* digit token), so `.00` on -0.5 shows `.50` in the cell and `-.50` on
    // the axis.
    if integer_run.is_empty() {
        return false;
    }
    // `#`…`0`… only: the applier models required integer digits as a *minimum width*, which is what
    // IronCalc's per-token padding comes to **only** when every required `0` sits to the right of
    // every optional `#`. `0#0` would need positional padding.
    if !integer_run_shape_is_hash_then_zero(integer_run) {
        return false;
    }
    // Grouping: IronCalc computes the separator's position from the *token* index rather than the
    // digit index (`use_group_separator(use_thousands, ln - digit_index, …)` in `format.rs`), so it
    // groups correctly only when the value's integer-digit count `ln` is not strictly between 3 and
    // the number of integer digit tokens — `##,##0` on 1234.5 renders `1234.5` in a cell where
    // Excel (and the applier) show `1,234.5`. It also never groups the zeros it *padded* in, so
    // `0,000` on 5 is `0005` in a cell and `0,005` here. Both are IronCalc defects (`GAPS.md` E8);
    // the shapes that cannot reach either are exactly "at most four integer digit tokens, at most
    // three of them required", which covers every everyday grouping code (`#,##0`, `#,###`,
    // `$#,##0.00`, `#,#00`).
    if spec.grouping {
        let digit_tokens = integer_run.chars().filter(|c| *c != ',').count();
        if digit_tokens > 4 || spec.min_integer_digits > 3 {
            return false;
        }
    }
    true
}

/// Whether an integer digit run (grouping commas included) is zero or more optional `#`
/// placeholders followed by zero or more required `0` placeholders — the shape whose required
/// digits are expressible as a **minimum integer width**. `#,##0`, `#,#00`, `000` and `###` all
/// qualify; `0#0` and `00#` do not (their required digits are positional).
fn integer_run_shape_is_hash_then_zero(integer_run: &str) -> bool {
    let mut seen_required = false;
    for c in integer_run.chars() {
        match c {
            '0' => seen_required = true,
            '#' if seen_required => return false,
            _ => {}
        }
    }
    true
}

/// The format's "control body" — the code with bracket tokens (`[…]`), quoted literals (`"…"`) and
/// `\`-escapes removed, leaving only the characters that act as *format controls*.
///
/// Every check that asks "does this code contain the control character X?" must run on this string,
/// never on the raw code: `0" %"` and `0\%` carry a **literal** percent sign (an
/// already-in-percent-units number, a standard Excel idiom), and treating it as the percent control
/// scaled the value by 100 with no badge. That bug existed because `control_body_is_unrenderable`
/// stripped literals and `FormatSpec::parse`'s `contains('%')` did not — two functions disagreeing
/// about the same character. There is one stripper now, and both callers use it.
fn control_body(code: &str) -> String {
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
    body
}

/// Whether a [`control_body`] carries a construct [`apply_number_format`] does **not** render
/// exactly even though [`FormatSpec::parse`] accepts it:
/// - a **scaling comma**: a `,` *after* the last `0`/`#` digit placeholder. Excel divides the value
///   by 1000 per such comma (`#,##0,` = thousands, `#,##0,,` = millions); the applier drops it and
///   shows the unscaled number (1000×/1e6× too big). A comma *before* the last placeholder is normal
///   thousands **grouping**, which the applier does render — so this only fires on the trailing scale.
/// - a comma **before** the first placeholder, which IronCalc renders as a literal `,` and the
///   applier drops.
/// - an **unhandled control/format char**: `_` (column-width align), `*` (fill-repeat), `?`
///   (digit-align space) or `@` (raw text) — none of which the applier renders.
///
/// Running on the stripped body keeps a benign comma or `*`/`_`/`?` *inside* a quoted suffix
/// (e.g. `0" (a*b)"`) from being mistaken for a control construct.
fn control_body_is_unrenderable(body: &str) -> bool {
    if body.contains(['_', '*', '?', '@']) {
        return true;
    }
    match (body.find(['0', '#']), body.rfind(['0', '#'])) {
        // A comma after the last digit placeholder is a ÷1000 scaling comma; one before the first
        // is a literal the applier drops. Both placeholder chars (`0`/`#`) are single-byte ASCII,
        // so the slice boundaries are valid.
        (Some(first), Some(last)) => {
            body[last + 1..].contains(',') || body[..first].contains([',', '.'])
        }
        // No digit placeholder at all — `FormatSpec::parse` will reject it (date/text) anyway.
        _ => false,
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
    /// The *kind* of each fractional placeholder, in order: `'0'` required, `'#'` optional. Length
    /// is [`Self::decimals`]. Trailing zeros of the rounded fraction are dropped, then one `0` is
    /// re-appended for each **required** placeholder past the surviving digits — which is what
    /// makes `#,##0.0#` print `1.5` (not `1.50`) and `0.#0` print `1.0` (not `1.00`). A single
    /// `min_decimals` count could not express the latter: it took the *last* `0`, so `0.#0` padded
    /// to two places and `#.0#0` to three.
    fraction_kinds: Vec<char>,
    /// Whether the section has a decimal separator inside its numeric run. Excel (and IronCalc)
    /// keep the separator even when every fractional digit is suppressed — `0.##` on 1 is `1.`.
    decimal_point: bool,
    /// **Minimum** digits before the decimal point — the count of required (`0`) placeholders in
    /// the integer run. The integer part is left-padded with zeros to this width, so `000` on 7 is
    /// `007` and `#,#00.0#` on 1 is `01.0`. Zero means the integer run is all-`#`, in which case an
    /// integer part that rounds to zero renders as nothing at all (`#,###` on 0 is the empty
    /// string; `#.##` on 0.5 is `.5`).
    min_integer_digits: usize,
    /// Whether to group the integer part in thousands.
    grouping: bool,
    /// How many unquoted `%` the section carries: the value is multiplied by `100^percent_scale`.
    /// The `%` characters themselves are literals rendered **where they stand** (they survive into
    /// [`Self::prefix`]/[`Self::suffix`]), so `%0` is `%100` and `0%%` is `10000%%`, exactly as
    /// IronCalc's `Token::Percent` (push `Literal('%')`, `percent += 1`) does it.
    percent_scale: i32,
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

        // The percent SCALE is counted on the control body — a `%` inside `"…"` or behind a `\` is
        // literal text, not the percent control. Counting it on the raw section is what made
        // `0" %"` (12.5 → `1250 %`) and `0\%` (1 → `100%`) scale by 100 against cells reading
        // `12 %` and `1%`.
        let percent_scale = control_body(section).matches('%').count() as i32;
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
        // "1.5". `decimals` is the rounding precision (all placeholders); the per-placeholder
        // KINDS drive which of the rounded fraction's trailing zeros survive.
        let decimal_point = fractional_run.is_some();
        let fraction_kinds: Vec<char> = fractional_run
            .unwrap_or("")
            .chars()
            .filter(|c| placeholders.contains(*c))
            .collect();
        let decimals = fraction_kinds.len();
        // Each required `0` in the integer run is a digit that must be printed even when the value
        // has none there, so the integer part is left-padded to that width (`000` on 7 → `007`).
        // Zero of them (an all-`#` run) suppresses a zero integer part entirely, the way Excel does.
        let min_integer_digits = integer_run.chars().filter(|c| *c == '0').count();

        // Prefix / suffix are the literal text around the numeric run, minus quotes and escapes.
        // A `%` SURVIVES: it is rendered where it stands (see `percent_scale`).
        let prefix = literal(&cleaned[..first]);
        let suffix = literal(&cleaned[last + 1..]);

        Some(Self {
            prefix,
            suffix,
            decimals,
            fraction_kinds,
            decimal_point,
            min_integer_digits,
            grouping,
            percent_scale,
        })
    }
}

/// Extract the literal text from a format fragment: unwrap `"…"` quotes, honor `\x` escapes, and
/// drop the format placeholder punctuation (`0 # , .`) so only genuine literal characters (a
/// currency symbol, a unit, a percent sign) survive. `%` is **kept** — it is a literal that also
/// scales the value, and it renders in the position it was written (`%0` → `%100`).
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
            '0' | '#' | ',' | '.' | '?' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Format a non-negative magnitude under `spec`: round to `spec.decimals` places, drop the
/// fraction's trailing zeros and re-add one per **required** (`0`) placeholder past them, left-pad
/// the integer part to `spec.min_integer_digits`, group it if asked, and suppress a zero integer
/// part when the integer run is all-`#`.
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

    // Trailing zeros of the rounded fraction are dropped wholesale, then one `0` is re-appended for
    // every REQUIRED placeholder past the digits that survived. That is IronCalc's `get_fract_part`
    // (trim, then per-token `'0'` padding) and Excel's rule, and unlike a single "minimum decimals"
    // count it gets the mixed runs right: `0.#0` on 1 is `1.0`, `#.0#0` on 1 is `1.00`.
    let kept = frac_part.trim_end_matches('0').len();
    let mut frac = String::from(&frac_part[..kept]);
    for kind in &spec.fraction_kinds[kept.min(spec.decimals)..] {
        if *kind == '0' {
            frac.push('0');
        }
    }

    // Required integer placeholders are a minimum width: `000` on 7 renders `007`.
    let padded;
    let int_digits = if int_part.len() < spec.min_integer_digits {
        padded = format!("{:0>width$}", int_part, width = spec.min_integer_digits);
        padded.as_str()
    } else {
        int_part
    };
    let mut out = if spec.min_integer_digits == 0 && int_digits == "0" {
        String::new()
    } else if spec.grouping {
        group_thousands(int_digits)
    } else {
        int_digits.to_string()
    };
    // The separator is a literal in the format string: Excel and IronCalc both emit it even when
    // every fractional digit was optional and got trimmed (`0.##` on 1 → `1.`).
    if spec.decimal_point {
        out.push('.');
        out.push_str(&frac);
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

    /// A `%` is a literal that renders **where it stands** and scales once per occurrence — not a
    /// flag that appends one `%` at the very end. Appending is what made `0% ` print `1 %` where
    /// the cell prints `1% `, and `%0` print `100%` where the cell prints `%100`.
    #[test]
    fn percent_renders_in_place_and_scales_once_per_sign() {
        assert_eq!(apply_number_format("0% ", 1.0), "100% ");
        assert_eq!(apply_number_format("%0", 1.0), "%100");
        assert_eq!(apply_number_format("0%%", 1.0), "10000%%");
        assert_eq!(apply_number_format("0%\" kg\"", 1.0), "100% kg");
    }

    /// A `%` inside a quoted literal or behind a `\` escape is a **literal percent sign**, not the
    /// percent control: `0" %"` and `0\%` are the standard Excel codes for a number that is
    /// *already* in percent units. Testing `contains('%')` on the raw section scaled them by 100 —
    /// `0" %"` on 12.5 rendered `1250 %` beside a cell reading `12 %` — with the chart classified
    /// Faithful and no badge. The percent count runs on the [`control_body`] now, which is the same
    /// stripping [`control_body_is_unrenderable`] already did for `,`/`_`/`*`/`?`.
    #[test]
    fn a_quoted_or_escaped_percent_is_a_literal_not_a_scale() {
        assert_eq!(apply_number_format("0\" %\"", 12.5), "12 %");
        assert_eq!(apply_number_format("#,##0\" %\"", 12.5), "12 %");
        assert_eq!(apply_number_format("0\\%", 1.0), "1%");
        assert_eq!(apply_number_format("0.00\"%\"", 0.25), "0.25%");
        // …and the real percent control still scales when it is not quoted.
        assert_eq!(apply_number_format("0\\%", 1.0), "1%");
        assert_eq!(apply_number_format("0%", 1.0), "100%");
    }

    /// Required (`0`) integer placeholders are a **minimum width**: `000` zero-pads to three digits
    /// the way Excel does for part numbers and zero-padded IDs. Recording only "is the integer run
    /// all-`#`" threw the width away, so `000` on 7 rendered `7` beside a cell reading `007`.
    #[test]
    fn required_integer_placeholders_zero_pad() {
        assert_eq!(apply_number_format("000", 7.0), "007");
        assert_eq!(apply_number_format("0000", 42.0), "0042");
        assert_eq!(apply_number_format("00.0", 1.0), "01.0");
        assert_eq!(apply_number_format("#,#00.0#", 1.0), "01.0");
        assert_eq!(apply_number_format("000", -7.0), "-007");
        // A value wider than the minimum is untouched.
        assert_eq!(apply_number_format("000", 12345.0), "12345");
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

    /// The surviving fractional zeros are decided **per placeholder**, not by a single "minimum
    /// decimals" count taken from the *last* required `0`: the rounded fraction loses all its
    /// trailing zeros and then one `0` comes back for each required placeholder past what
    /// survived. A `min_decimals` count padded `0.#0` on 1 to `1.00` and `#.0#0` on 1 to `1.000`,
    /// where the cells read `1.0` and `1.00`.
    #[test]
    fn optional_and_required_fractional_placeholders_interleave() {
        assert_eq!(apply_number_format("0.#0", 1.0), "1.0");
        assert_eq!(apply_number_format("#.0#0", 1.0), "1.00");
        assert_eq!(apply_number_format("0.#0", 0.125), "0.12");
        assert_eq!(apply_number_format("0.0#0", 1.5), "1.50");
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
            "",
            "General",
            "general",
            "0",
            "0.00",
            "#,##0",
            "#,##0.00",
            "0%",
            "0.0%",
            "$#,##0",
            // The optional-digit (`#`) family renders exactly too, now that `#` is honored as an
            // optional placeholder rather than counted as a required one.
            "0.##",
            "#,##0.##",
            "#,##0.0#",
            "0.###",
            "#,###",
            "0 ",
            // Required leading zeros (zero-padded IDs), and the mixed fractional runs.
            "000",
            "0000",
            "00.0",
            "#,#00.0#",
            "0.#0",
            "#.0#0",
            // Percent in every position/spelling the applier now renders in place.
            "0% ",
            "%0",
            "0%%",
            "0\" %\"",
            "0\\%",
            "0.00\"%\"",
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
            "0;@",               // `@` raw-text section
            // Shapes the applier renders the way EXCEL does and IronCalc does not, so a chart
            // carrying them would disagree with the cells beside it (`GAPS.md` E8/E9). They are
            // badged rather than "fixed" by copying the engine's defect into the charts.
            "##,##0", // 5 integer digit tokens: the cell drops the separator on 4-digit values
            "0,000",  // grouping + 4 required zeros: the cell never groups the digits it padded
            "0\"a\"\"b\"", // doubled quote: the two lexers disagree about what it means
            // Shapes outside the applier's model of "prefix + digits + suffix".
            "0#0",             // required digits are positional, not a minimum width
            ".00",             // no integer placeholder: the cell hangs the minus sign off one
            "0 0",             // a literal inside the digit run
            "0[$€-407]",       // the cell hoists a bracketed currency to the front of the string
            "[$USD-409]#,##0", // a multi-character currency symbol the cell's lexer rejects
        ] {
            assert!(!renders_faithfully(code), "{code:?} should NOT be faithful");
        }
    }
}
