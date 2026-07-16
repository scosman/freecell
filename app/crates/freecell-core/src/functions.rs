//! The **static function catalog** + formula completion-context heuristics powering
//! function autocomplete + signature hints (`gaps_closing_7_15 functional_spec.md §1`,
//! `architecture.md §1`).
//!
//! IronCalc's 345-variant `Function` enum is private and non-enumerable from outside the
//! engine crate, and `freecell-core` may not depend on IronCalc (`tests/dependency_rule.rs`).
//! So the name list lives here as a FreeCell-static `const` array, seeded from the two
//! committed parity CSVs (`experiments/round-2/03-function-parity/data/`): the 345
//! engine-registered names (D1.3), each with an importance `rank` (the canonical CSV's
//! `common` set = rank 0, everything else rank 1) and an argument `template`. The ~150
//! common names carry authored templates; the long tail carries a generic `NAME(…)`
//! fallback (every name still completes — only the arg hint degrades).
//!
//! The completion-context detection ([`fn_edit_context`], [`enclosing_fn_name`]) is a
//! deliberate **lexical heuristic**, not a real parse — sufficient for name completion and a
//! static (D1.1) signature hint, and fully unit-testable without IronCalc.

/// One catalog entry: the function name, its argument template, and an importance rank
/// (lower = more common; drives completion ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FnSig {
    /// The engine-registered function name, uppercase (`"SUMIF"`).
    pub name: &'static str,
    /// The argument template shown in the completion row + signature hint
    /// (`"SUMIF(range, criteria, [sum_range])"`; optional args in `[]`). The long tail uses
    /// the generic `NAME(…)` fallback.
    pub template: &'static str,
    /// Importance rank: `0` for the canonical `common` set, `1` for the rest. Ties break
    /// alphabetically in [`complete`].
    pub rank: u16,
}

/// The active function-name token under the caret (from [`fn_edit_context`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnEditContext {
    /// Byte offset in the edit text where the identifier prefix begins (the replace point on
    /// accept).
    pub token_start: usize,
    /// The identifier chars in `[token_start, caret)` — the completion prefix.
    pub prefix: String,
}

/// Whether `c` is a formula identifier char (letters, digits, `.`, `_`). All ASCII, so a
/// byte-wise left-walk over these never splits a multi-byte char.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_'
}

/// Whether the char immediately before an identifier token puts it in **function position**
/// — start-of-formula handled by the caller; here: an operator/opener/separator/space. A
/// digit or `)` (a value/closing-paren) is *not* function position.
fn is_function_position_prev(b: u8) -> bool {
    matches!(
        b,
        b'(' | b',' | b'+' | b'-' | b'*' | b'/' | b'^' | b'&' | b'<' | b'>' | b'=' | b'%' | b':'
    ) || b.is_ascii_whitespace()
}

/// Whether `token` has the shape of an A1 cell reference: 1–3 leading letters then ≥1 digit,
/// nothing else. Used to reject `=A1` as a reference (unless a real function shares the
/// prefix, e.g. `LOG10`/`ATAN2` — checked by the caller).
fn is_cell_ref_shape(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == 0 || i > 3 || i == bytes.len() {
        return false;
    }
    bytes[i..].iter().all(|b| b.is_ascii_digit())
}

/// Whether any catalog name starts with `prefix` (case-insensitive).
fn any_fn_starts_with(prefix: &str) -> bool {
    let up = prefix.to_ascii_uppercase();
    FUNCTIONS.iter().any(|f| f.name.starts_with(&up))
}

/// Whether the caret sits inside an (unterminated) string literal — scan `text[..caret]`
/// toggling on unescaped `"` (IronCalc doubles `""` to escape).
fn in_string_at(text: &str, caret: usize) -> bool {
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < caret {
        if bytes[i] == b'"' {
            if in_string && i + 1 < caret && bytes[i + 1] == b'"' {
                // A doubled quote inside a string is an escaped quote — stay inside.
                i += 2;
                continue;
            }
            in_string = !in_string;
        }
        i += 1;
    }
    in_string
}

/// Case-insensitive **prefix** completion against the catalog. Order: exact-name match
/// first, then `rank` ascending, then name ascending. An empty prefix returns nothing (the
/// ≥1-char trigger is enforced in [`fn_edit_context`]; this is a belt-and-braces).
pub fn complete(prefix: &str) -> Vec<&'static FnSig> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let up = prefix.to_ascii_uppercase();
    let mut out: Vec<&'static FnSig> = FUNCTIONS
        .iter()
        .filter(|f| f.name.starts_with(&up))
        .collect();
    out.sort_by(|a, b| {
        let a_exact = a.name == up;
        let b_exact = b.name == up;
        // Exact match sorts first (false < true, so invert), then rank, then name.
        b_exact
            .cmp(&a_exact)
            .then(a.rank.cmp(&b.rank))
            .then(a.name.cmp(b.name))
    });
    out
}

/// Exact (case-insensitive) name lookup for the signature hint.
pub fn signature(name: &str) -> Option<&'static FnSig> {
    let up = name.to_ascii_uppercase();
    FUNCTIONS.iter().find(|f| f.name == up)
}

/// The active function-name token under `caret` (a byte offset into `text`), or `None` when
/// the caret is not at the end of a completable function-name prefix in function position
/// inside a formula. A lexical heuristic (`architecture.md §1.1`), not a real parse.
pub fn fn_edit_context(text: &str, caret: usize) -> Option<FnEditContext> {
    if !text.starts_with('=') || caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    if in_string_at(text, caret) {
        return None;
    }
    let bytes = text.as_bytes();
    // Walk left from the caret over identifier bytes to the token start.
    let mut start = caret;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let prefix = &text[start..caret];
    // ≥1 char, leading char a letter (a number/`.`-led token is not a function name).
    let first = prefix.as_bytes().first().copied()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    // Function position: `=` at index 0, or an operator/opener/separator/space before it.
    let in_fn_position = start == 1 || (start > 1 && is_function_position_prev(bytes[start - 1]));
    if !in_fn_position {
        return None;
    }
    // Reject a bare cell reference (`=A1`), unless a real function shares the prefix.
    if is_cell_ref_shape(prefix) && !any_fn_starts_with(prefix) {
        return None;
    }
    Some(FnEditContext {
        token_start: start,
        prefix: prefix.to_string(),
    })
}

/// The name of the function call whose parentheses enclose `caret` (for the signature hint's
/// "caret inside a call" trigger), or `None`. Scans `text[..caret]` (skipping string
/// literals), tracking open parens; the innermost still-open `(` names the enclosing call.
pub fn enclosing_fn_name(text: &str, caret: usize) -> Option<&str> {
    if !text.starts_with('=') || caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut open_stack: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < caret {
        let b = bytes[i];
        if in_string {
            if b == b'"' {
                if i + 1 < caret && bytes[i + 1] == b'"' {
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'(' => open_stack.push(i),
            b')' => {
                open_stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    let open = *open_stack.last()?;
    // Read the identifier immediately left of the open paren.
    let mut start = open;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == open {
        return None;
    }
    let name = &text[start..open];
    if !name.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    Some(name)
}

// The static catalog, built from the two committed parity CSVs
// (`experiments/round-2/03-function-parity/data/{ironcalc_functions.csv,
// excel_functions_canonical.csv}`): the 345 engine-registered names (D1.3). The 154
// `common`-importance names carry `rank: 0` + an authored argument template (Excel/ECMA-376
// argument names, optional args in `[]`); the remaining 191 carry `rank: 1` + the generic
// `NAME(…)` fallback. The unit tests below pin the count + the common-set template invariant.
pub const FUNCTIONS: &[FnSig] = &[
    FnSig { name: "ABS", template: "ABS(number)", rank: 0 },
    FnSig { name: "ACOS", template: "ACOS(number)", rank: 0 },
    FnSig { name: "ACOSH", template: "ACOSH(…)", rank: 1 },
    FnSig { name: "ACOT", template: "ACOT(…)", rank: 1 },
    FnSig { name: "ACOTH", template: "ACOTH(…)", rank: 1 },
    FnSig { name: "AND", template: "AND(logical1, [logical2], …)", rank: 0 },
    FnSig { name: "ARABIC", template: "ARABIC(…)", rank: 1 },
    FnSig { name: "ASIN", template: "ASIN(number)", rank: 0 },
    FnSig { name: "ASINH", template: "ASINH(…)", rank: 1 },
    FnSig { name: "ATAN", template: "ATAN(number)", rank: 0 },
    FnSig { name: "ATAN2", template: "ATAN2(x_num, y_num)", rank: 0 },
    FnSig { name: "ATANH", template: "ATANH(…)", rank: 1 },
    FnSig { name: "AVEDEV", template: "AVEDEV(…)", rank: 1 },
    FnSig { name: "AVERAGE", template: "AVERAGE(number1, [number2], …)", rank: 0 },
    FnSig { name: "AVERAGEA", template: "AVERAGEA(value1, [value2], …)", rank: 0 },
    FnSig { name: "AVERAGEIF", template: "AVERAGEIF(range, criteria, [average_range])", rank: 0 },
    FnSig { name: "AVERAGEIFS", template: "AVERAGEIFS(average_range, criteria_range1, criteria1, …)", rank: 0 },
    FnSig { name: "BASE", template: "BASE(…)", rank: 1 },
    FnSig { name: "BESSELI", template: "BESSELI(…)", rank: 1 },
    FnSig { name: "BESSELJ", template: "BESSELJ(…)", rank: 1 },
    FnSig { name: "BESSELK", template: "BESSELK(…)", rank: 1 },
    FnSig { name: "BESSELY", template: "BESSELY(…)", rank: 1 },
    FnSig { name: "BETA.DIST", template: "BETA.DIST(…)", rank: 1 },
    FnSig { name: "BETA.INV", template: "BETA.INV(…)", rank: 1 },
    FnSig { name: "BIN2DEC", template: "BIN2DEC(…)", rank: 1 },
    FnSig { name: "BIN2HEX", template: "BIN2HEX(…)", rank: 1 },
    FnSig { name: "BIN2OCT", template: "BIN2OCT(…)", rank: 1 },
    FnSig { name: "BINOM.DIST", template: "BINOM.DIST(…)", rank: 1 },
    FnSig { name: "BINOM.DIST.RANGE", template: "BINOM.DIST.RANGE(…)", rank: 1 },
    FnSig { name: "BINOM.INV", template: "BINOM.INV(…)", rank: 1 },
    FnSig { name: "BITAND", template: "BITAND(…)", rank: 1 },
    FnSig { name: "BITLSHIFT", template: "BITLSHIFT(…)", rank: 1 },
    FnSig { name: "BITOR", template: "BITOR(…)", rank: 1 },
    FnSig { name: "BITRSHIFT", template: "BITRSHIFT(…)", rank: 1 },
    FnSig { name: "BITXOR", template: "BITXOR(…)", rank: 1 },
    FnSig { name: "CEILING", template: "CEILING(number, significance)", rank: 0 },
    FnSig { name: "CEILING.MATH", template: "CEILING.MATH(number, [significance], [mode])", rank: 0 },
    FnSig { name: "CEILING.PRECISE", template: "CEILING.PRECISE(…)", rank: 1 },
    FnSig { name: "CELL", template: "CELL(info_type, [reference])", rank: 0 },
    FnSig { name: "CHISQ.DIST", template: "CHISQ.DIST(…)", rank: 1 },
    FnSig { name: "CHISQ.DIST.RT", template: "CHISQ.DIST.RT(…)", rank: 1 },
    FnSig { name: "CHISQ.INV", template: "CHISQ.INV(…)", rank: 1 },
    FnSig { name: "CHISQ.INV.RT", template: "CHISQ.INV.RT(…)", rank: 1 },
    FnSig { name: "CHISQ.TEST", template: "CHISQ.TEST(…)", rank: 1 },
    FnSig { name: "CHOOSE", template: "CHOOSE(index_num, value1, [value2], …)", rank: 0 },
    FnSig { name: "COLUMN", template: "COLUMN([reference])", rank: 0 },
    FnSig { name: "COLUMNS", template: "COLUMNS(array)", rank: 0 },
    FnSig { name: "COMBIN", template: "COMBIN(…)", rank: 1 },
    FnSig { name: "COMBINA", template: "COMBINA(…)", rank: 1 },
    FnSig { name: "COMPLEX", template: "COMPLEX(…)", rank: 1 },
    FnSig { name: "CONCAT", template: "CONCAT(text1, [text2], …)", rank: 0 },
    FnSig { name: "CONCATENATE", template: "CONCATENATE(text1, [text2], …)", rank: 0 },
    FnSig { name: "CONFIDENCE.NORM", template: "CONFIDENCE.NORM(…)", rank: 1 },
    FnSig { name: "CONFIDENCE.T", template: "CONFIDENCE.T(…)", rank: 1 },
    FnSig { name: "CONVERT", template: "CONVERT(…)", rank: 1 },
    FnSig { name: "CORREL", template: "CORREL(…)", rank: 1 },
    FnSig { name: "COS", template: "COS(number)", rank: 0 },
    FnSig { name: "COSH", template: "COSH(…)", rank: 1 },
    FnSig { name: "COT", template: "COT(…)", rank: 1 },
    FnSig { name: "COTH", template: "COTH(…)", rank: 1 },
    FnSig { name: "COUNT", template: "COUNT(value1, [value2], …)", rank: 0 },
    FnSig { name: "COUNTA", template: "COUNTA(value1, [value2], …)", rank: 0 },
    FnSig { name: "COUNTBLANK", template: "COUNTBLANK(range)", rank: 0 },
    FnSig { name: "COUNTIF", template: "COUNTIF(range, criteria)", rank: 0 },
    FnSig { name: "COUNTIFS", template: "COUNTIFS(criteria_range1, criteria1, …)", rank: 0 },
    FnSig { name: "COVARIANCE.P", template: "COVARIANCE.P(…)", rank: 1 },
    FnSig { name: "COVARIANCE.S", template: "COVARIANCE.S(…)", rank: 1 },
    FnSig { name: "CSC", template: "CSC(…)", rank: 1 },
    FnSig { name: "CSCH", template: "CSCH(…)", rank: 1 },
    FnSig { name: "CUMIPMT", template: "CUMIPMT(…)", rank: 1 },
    FnSig { name: "CUMPRINC", template: "CUMPRINC(…)", rank: 1 },
    FnSig { name: "DATE", template: "DATE(year, month, day)", rank: 0 },
    FnSig { name: "DATEDIF", template: "DATEDIF(start_date, end_date, unit)", rank: 0 },
    FnSig { name: "DATEVALUE", template: "DATEVALUE(date_text)", rank: 0 },
    FnSig { name: "DAVERAGE", template: "DAVERAGE(…)", rank: 1 },
    FnSig { name: "DAY", template: "DAY(serial_number)", rank: 0 },
    FnSig { name: "DAYS", template: "DAYS(end_date, start_date)", rank: 0 },
    FnSig { name: "DAYS360", template: "DAYS360(…)", rank: 1 },
    FnSig { name: "DB", template: "DB(…)", rank: 1 },
    FnSig { name: "DCOUNT", template: "DCOUNT(…)", rank: 1 },
    FnSig { name: "DCOUNTA", template: "DCOUNTA(…)", rank: 1 },
    FnSig { name: "DDB", template: "DDB(…)", rank: 1 },
    FnSig { name: "DEC2BIN", template: "DEC2BIN(…)", rank: 1 },
    FnSig { name: "DEC2HEX", template: "DEC2HEX(…)", rank: 1 },
    FnSig { name: "DEC2OCT", template: "DEC2OCT(…)", rank: 1 },
    FnSig { name: "DECIMAL", template: "DECIMAL(…)", rank: 1 },
    FnSig { name: "DEGREES", template: "DEGREES(angle)", rank: 0 },
    FnSig { name: "DELTA", template: "DELTA(…)", rank: 1 },
    FnSig { name: "DEVSQ", template: "DEVSQ(…)", rank: 1 },
    FnSig { name: "DGET", template: "DGET(…)", rank: 1 },
    FnSig { name: "DMAX", template: "DMAX(…)", rank: 1 },
    FnSig { name: "DMIN", template: "DMIN(…)", rank: 1 },
    FnSig { name: "DOLLARDE", template: "DOLLARDE(…)", rank: 1 },
    FnSig { name: "DOLLARFR", template: "DOLLARFR(…)", rank: 1 },
    FnSig { name: "DPRODUCT", template: "DPRODUCT(…)", rank: 1 },
    FnSig { name: "DSTDEV", template: "DSTDEV(…)", rank: 1 },
    FnSig { name: "DSTDEVP", template: "DSTDEVP(…)", rank: 1 },
    FnSig { name: "DSUM", template: "DSUM(…)", rank: 1 },
    FnSig { name: "DVAR", template: "DVAR(…)", rank: 1 },
    FnSig { name: "DVARP", template: "DVARP(…)", rank: 1 },
    FnSig { name: "EDATE", template: "EDATE(start_date, months)", rank: 0 },
    FnSig { name: "EFFECT", template: "EFFECT(…)", rank: 1 },
    FnSig { name: "EOMONTH", template: "EOMONTH(start_date, months)", rank: 0 },
    FnSig { name: "ERF", template: "ERF(…)", rank: 1 },
    FnSig { name: "ERF.PRECISE", template: "ERF.PRECISE(…)", rank: 1 },
    FnSig { name: "ERFC", template: "ERFC(…)", rank: 1 },
    FnSig { name: "ERFC.PRECISE", template: "ERFC.PRECISE(…)", rank: 1 },
    FnSig { name: "ERROR.TYPE", template: "ERROR.TYPE(…)", rank: 1 },
    FnSig { name: "EVEN", template: "EVEN(number)", rank: 0 },
    FnSig { name: "EXACT", template: "EXACT(text1, text2)", rank: 0 },
    FnSig { name: "EXP", template: "EXP(number)", rank: 0 },
    FnSig { name: "EXPON.DIST", template: "EXPON.DIST(…)", rank: 1 },
    FnSig { name: "F.DIST", template: "F.DIST(…)", rank: 1 },
    FnSig { name: "F.DIST.RT", template: "F.DIST.RT(…)", rank: 1 },
    FnSig { name: "F.INV", template: "F.INV(…)", rank: 1 },
    FnSig { name: "F.INV.RT", template: "F.INV.RT(…)", rank: 1 },
    FnSig { name: "F.TEST", template: "F.TEST(…)", rank: 1 },
    FnSig { name: "FACT", template: "FACT(number)", rank: 0 },
    FnSig { name: "FACTDOUBLE", template: "FACTDOUBLE(…)", rank: 1 },
    FnSig { name: "FALSE", template: "FALSE()", rank: 0 },
    FnSig { name: "FIND", template: "FIND(find_text, within_text, [start_num])", rank: 0 },
    FnSig { name: "FISHER", template: "FISHER(…)", rank: 1 },
    FnSig { name: "FISHERINV", template: "FISHERINV(…)", rank: 1 },
    FnSig { name: "FLOOR", template: "FLOOR(number, significance)", rank: 0 },
    FnSig { name: "FLOOR.MATH", template: "FLOOR.MATH(number, [significance], [mode])", rank: 0 },
    FnSig { name: "FLOOR.PRECISE", template: "FLOOR.PRECISE(…)", rank: 1 },
    FnSig { name: "FORMULATEXT", template: "FORMULATEXT(…)", rank: 1 },
    FnSig { name: "FV", template: "FV(rate, nper, pmt, [pv], [type])", rank: 0 },
    FnSig { name: "GAMMA", template: "GAMMA(…)", rank: 1 },
    FnSig { name: "GAMMA.DIST", template: "GAMMA.DIST(…)", rank: 1 },
    FnSig { name: "GAMMA.INV", template: "GAMMA.INV(…)", rank: 1 },
    FnSig { name: "GAMMALN", template: "GAMMALN(…)", rank: 1 },
    FnSig { name: "GAMMALN.PRECISE", template: "GAMMALN.PRECISE(…)", rank: 1 },
    FnSig { name: "GAUSS", template: "GAUSS(…)", rank: 1 },
    FnSig { name: "GCD", template: "GCD(…)", rank: 1 },
    FnSig { name: "GEOMEAN", template: "GEOMEAN(…)", rank: 1 },
    FnSig { name: "GESTEP", template: "GESTEP(…)", rank: 1 },
    FnSig { name: "HARMEAN", template: "HARMEAN(…)", rank: 1 },
    FnSig { name: "HEX2BIN", template: "HEX2BIN(…)", rank: 1 },
    FnSig { name: "HEX2DEC", template: "HEX2DEC(…)", rank: 1 },
    FnSig { name: "HEX2OCT", template: "HEX2OCT(…)", rank: 1 },
    FnSig { name: "HLOOKUP", template: "HLOOKUP(lookup_value, table_array, row_index_num, [range_lookup])", rank: 0 },
    FnSig { name: "HOUR", template: "HOUR(serial_number)", rank: 0 },
    FnSig { name: "HYPGEOM.DIST", template: "HYPGEOM.DIST(…)", rank: 1 },
    FnSig { name: "IF", template: "IF(logical_test, value_if_true, [value_if_false])", rank: 0 },
    FnSig { name: "IFERROR", template: "IFERROR(value, value_if_error)", rank: 0 },
    FnSig { name: "IFNA", template: "IFNA(value, value_if_na)", rank: 0 },
    FnSig { name: "IFS", template: "IFS(logical_test1, value1, …)", rank: 0 },
    FnSig { name: "IMABS", template: "IMABS(…)", rank: 1 },
    FnSig { name: "IMAGINARY", template: "IMAGINARY(…)", rank: 1 },
    FnSig { name: "IMARGUMENT", template: "IMARGUMENT(…)", rank: 1 },
    FnSig { name: "IMCONJUGATE", template: "IMCONJUGATE(…)", rank: 1 },
    FnSig { name: "IMCOS", template: "IMCOS(…)", rank: 1 },
    FnSig { name: "IMCOSH", template: "IMCOSH(…)", rank: 1 },
    FnSig { name: "IMCOT", template: "IMCOT(…)", rank: 1 },
    FnSig { name: "IMCSC", template: "IMCSC(…)", rank: 1 },
    FnSig { name: "IMCSCH", template: "IMCSCH(…)", rank: 1 },
    FnSig { name: "IMDIV", template: "IMDIV(…)", rank: 1 },
    FnSig { name: "IMEXP", template: "IMEXP(…)", rank: 1 },
    FnSig { name: "IMLN", template: "IMLN(…)", rank: 1 },
    FnSig { name: "IMLOG10", template: "IMLOG10(…)", rank: 1 },
    FnSig { name: "IMLOG2", template: "IMLOG2(…)", rank: 1 },
    FnSig { name: "IMPOWER", template: "IMPOWER(…)", rank: 1 },
    FnSig { name: "IMPRODUCT", template: "IMPRODUCT(…)", rank: 1 },
    FnSig { name: "IMREAL", template: "IMREAL(…)", rank: 1 },
    FnSig { name: "IMSEC", template: "IMSEC(…)", rank: 1 },
    FnSig { name: "IMSECH", template: "IMSECH(…)", rank: 1 },
    FnSig { name: "IMSIN", template: "IMSIN(…)", rank: 1 },
    FnSig { name: "IMSINH", template: "IMSINH(…)", rank: 1 },
    FnSig { name: "IMSQRT", template: "IMSQRT(…)", rank: 1 },
    FnSig { name: "IMSUB", template: "IMSUB(…)", rank: 1 },
    FnSig { name: "IMSUM", template: "IMSUM(…)", rank: 1 },
    FnSig { name: "IMTAN", template: "IMTAN(…)", rank: 1 },
    FnSig { name: "INDEX", template: "INDEX(array, row_num, [column_num])", rank: 0 },
    FnSig { name: "INDIRECT", template: "INDIRECT(ref_text, [a1])", rank: 0 },
    FnSig { name: "INFO", template: "INFO(…)", rank: 1 },
    FnSig { name: "INT", template: "INT(number)", rank: 0 },
    FnSig { name: "INTERCEPT", template: "INTERCEPT(…)", rank: 1 },
    FnSig { name: "IPMT", template: "IPMT(rate, per, nper, pv, [fv], [type])", rank: 0 },
    FnSig { name: "IRR", template: "IRR(values, [guess])", rank: 0 },
    FnSig { name: "ISBLANK", template: "ISBLANK(value)", rank: 0 },
    FnSig { name: "ISERR", template: "ISERR(value)", rank: 0 },
    FnSig { name: "ISERROR", template: "ISERROR(value)", rank: 0 },
    FnSig { name: "ISEVEN", template: "ISEVEN(number)", rank: 0 },
    FnSig { name: "ISFORMULA", template: "ISFORMULA(reference)", rank: 0 },
    FnSig { name: "ISLOGICAL", template: "ISLOGICAL(value)", rank: 0 },
    FnSig { name: "ISNA", template: "ISNA(value)", rank: 0 },
    FnSig { name: "ISNONTEXT", template: "ISNONTEXT(value)", rank: 0 },
    FnSig { name: "ISNUMBER", template: "ISNUMBER(value)", rank: 0 },
    FnSig { name: "ISO.CEILING", template: "ISO.CEILING(…)", rank: 1 },
    FnSig { name: "ISODD", template: "ISODD(number)", rank: 0 },
    FnSig { name: "ISOWEEKNUM", template: "ISOWEEKNUM(…)", rank: 1 },
    FnSig { name: "ISPMT", template: "ISPMT(…)", rank: 1 },
    FnSig { name: "ISREF", template: "ISREF(…)", rank: 1 },
    FnSig { name: "ISTEXT", template: "ISTEXT(value)", rank: 0 },
    FnSig { name: "KURT", template: "KURT(…)", rank: 1 },
    FnSig { name: "LARGE", template: "LARGE(array, k)", rank: 0 },
    FnSig { name: "LCM", template: "LCM(…)", rank: 1 },
    FnSig { name: "LEFT", template: "LEFT(text, [num_chars])", rank: 0 },
    FnSig { name: "LEN", template: "LEN(text)", rank: 0 },
    FnSig { name: "LN", template: "LN(number)", rank: 0 },
    FnSig { name: "LOG", template: "LOG(number, [base])", rank: 0 },
    FnSig { name: "LOG10", template: "LOG10(number)", rank: 0 },
    FnSig { name: "LOGNORM.DIST", template: "LOGNORM.DIST(…)", rank: 1 },
    FnSig { name: "LOGNORM.INV", template: "LOGNORM.INV(…)", rank: 1 },
    FnSig { name: "LOOKUP", template: "LOOKUP(lookup_value, lookup_vector, [result_vector])", rank: 0 },
    FnSig { name: "LOWER", template: "LOWER(text)", rank: 0 },
    FnSig { name: "MATCH", template: "MATCH(lookup_value, lookup_array, [match_type])", rank: 0 },
    FnSig { name: "MAX", template: "MAX(number1, [number2], …)", rank: 0 },
    FnSig { name: "MAXA", template: "MAXA(value1, [value2], …)", rank: 0 },
    FnSig { name: "MAXIFS", template: "MAXIFS(max_range, criteria_range1, criteria1, …)", rank: 0 },
    FnSig { name: "MEDIAN", template: "MEDIAN(number1, [number2], …)", rank: 0 },
    FnSig { name: "MID", template: "MID(text, start_num, num_chars)", rank: 0 },
    FnSig { name: "MIN", template: "MIN(number1, [number2], …)", rank: 0 },
    FnSig { name: "MINA", template: "MINA(value1, [value2], …)", rank: 0 },
    FnSig { name: "MINIFS", template: "MINIFS(min_range, criteria_range1, criteria1, …)", rank: 0 },
    FnSig { name: "MINUTE", template: "MINUTE(serial_number)", rank: 0 },
    FnSig { name: "MIRR", template: "MIRR(…)", rank: 1 },
    FnSig { name: "MOD", template: "MOD(number, divisor)", rank: 0 },
    FnSig { name: "MONTH", template: "MONTH(serial_number)", rank: 0 },
    FnSig { name: "MROUND", template: "MROUND(number, multiple)", rank: 0 },
    FnSig { name: "N", template: "N(value)", rank: 0 },
    FnSig { name: "NA", template: "NA()", rank: 0 },
    FnSig { name: "NEGBINOM.DIST", template: "NEGBINOM.DIST(…)", rank: 1 },
    FnSig { name: "NETWORKDAYS", template: "NETWORKDAYS(start_date, end_date, [holidays])", rank: 0 },
    FnSig { name: "NETWORKDAYS.INTL", template: "NETWORKDAYS.INTL(start_date, end_date, [weekend], [holidays])", rank: 0 },
    FnSig { name: "NOMINAL", template: "NOMINAL(…)", rank: 1 },
    FnSig { name: "NORM.DIST", template: "NORM.DIST(…)", rank: 1 },
    FnSig { name: "NORM.INV", template: "NORM.INV(…)", rank: 1 },
    FnSig { name: "NORM.S.DIST", template: "NORM.S.DIST(…)", rank: 1 },
    FnSig { name: "NORM.S.INV", template: "NORM.S.INV(…)", rank: 1 },
    FnSig { name: "NOT", template: "NOT(logical)", rank: 0 },
    FnSig { name: "NOW", template: "NOW()", rank: 0 },
    FnSig { name: "NPER", template: "NPER(rate, pmt, pv, [fv], [type])", rank: 0 },
    FnSig { name: "NPV", template: "NPV(rate, value1, [value2], …)", rank: 0 },
    FnSig { name: "OCT2BIN", template: "OCT2BIN(…)", rank: 1 },
    FnSig { name: "OCT2DEC", template: "OCT2DEC(…)", rank: 1 },
    FnSig { name: "OCT2HEX", template: "OCT2HEX(…)", rank: 1 },
    FnSig { name: "ODD", template: "ODD(number)", rank: 0 },
    FnSig { name: "OFFSET", template: "OFFSET(reference, rows, cols, [height], [width])", rank: 0 },
    FnSig { name: "OR", template: "OR(logical1, [logical2], …)", rank: 0 },
    FnSig { name: "PDURATION", template: "PDURATION(…)", rank: 1 },
    FnSig { name: "PEARSON", template: "PEARSON(…)", rank: 1 },
    FnSig { name: "PHI", template: "PHI(…)", rank: 1 },
    FnSig { name: "PI", template: "PI()", rank: 0 },
    FnSig { name: "PMT", template: "PMT(rate, nper, pv, [fv], [type])", rank: 0 },
    FnSig { name: "POISSON.DIST", template: "POISSON.DIST(…)", rank: 1 },
    FnSig { name: "POWER", template: "POWER(number, power)", rank: 0 },
    FnSig { name: "PPMT", template: "PPMT(rate, per, nper, pv, [fv], [type])", rank: 0 },
    FnSig { name: "PRODUCT", template: "PRODUCT(number1, [number2], …)", rank: 0 },
    FnSig { name: "PV", template: "PV(rate, nper, pmt, [fv], [type])", rank: 0 },
    FnSig { name: "QUOTIENT", template: "QUOTIENT(numerator, denominator)", rank: 0 },
    FnSig { name: "RADIANS", template: "RADIANS(angle)", rank: 0 },
    FnSig { name: "RAND", template: "RAND()", rank: 0 },
    FnSig { name: "RANDBETWEEN", template: "RANDBETWEEN(bottom, top)", rank: 0 },
    FnSig { name: "RANK.AVG", template: "RANK.AVG(number, ref, [order])", rank: 0 },
    FnSig { name: "RANK.EQ", template: "RANK.EQ(number, ref, [order])", rank: 0 },
    FnSig { name: "RATE", template: "RATE(nper, pmt, pv, [fv], [type], [guess])", rank: 0 },
    FnSig { name: "REPT", template: "REPT(text, number_times)", rank: 0 },
    FnSig { name: "RIGHT", template: "RIGHT(text, [num_chars])", rank: 0 },
    FnSig { name: "ROMAN", template: "ROMAN(…)", rank: 1 },
    FnSig { name: "ROUND", template: "ROUND(number, num_digits)", rank: 0 },
    FnSig { name: "ROUNDDOWN", template: "ROUNDDOWN(number, num_digits)", rank: 0 },
    FnSig { name: "ROUNDUP", template: "ROUNDUP(number, num_digits)", rank: 0 },
    FnSig { name: "ROW", template: "ROW([reference])", rank: 0 },
    FnSig { name: "ROWS", template: "ROWS(array)", rank: 0 },
    FnSig { name: "RRI", template: "RRI(…)", rank: 1 },
    FnSig { name: "RSQ", template: "RSQ(…)", rank: 1 },
    FnSig { name: "SEARCH", template: "SEARCH(find_text, within_text, [start_num])", rank: 0 },
    FnSig { name: "SEC", template: "SEC(…)", rank: 1 },
    FnSig { name: "SECH", template: "SECH(…)", rank: 1 },
    FnSig { name: "SECOND", template: "SECOND(serial_number)", rank: 0 },
    FnSig { name: "SHEET", template: "SHEET(…)", rank: 1 },
    FnSig { name: "SHEETS", template: "SHEETS(…)", rank: 1 },
    FnSig { name: "SIGN", template: "SIGN(number)", rank: 0 },
    FnSig { name: "SIN", template: "SIN(number)", rank: 0 },
    FnSig { name: "SINH", template: "SINH(…)", rank: 1 },
    FnSig { name: "SKEW", template: "SKEW(…)", rank: 1 },
    FnSig { name: "SKEW.P", template: "SKEW.P(…)", rank: 1 },
    FnSig { name: "SLN", template: "SLN(…)", rank: 1 },
    FnSig { name: "SLOPE", template: "SLOPE(…)", rank: 1 },
    FnSig { name: "SMALL", template: "SMALL(array, k)", rank: 0 },
    FnSig { name: "SQRT", template: "SQRT(number)", rank: 0 },
    FnSig { name: "SQRTPI", template: "SQRTPI(…)", rank: 1 },
    FnSig { name: "STANDARDIZE", template: "STANDARDIZE(…)", rank: 1 },
    FnSig { name: "STDEV.P", template: "STDEV.P(number1, [number2], …)", rank: 0 },
    FnSig { name: "STDEV.S", template: "STDEV.S(number1, [number2], …)", rank: 0 },
    FnSig { name: "STDEVA", template: "STDEVA(…)", rank: 1 },
    FnSig { name: "STDEVPA", template: "STDEVPA(…)", rank: 1 },
    FnSig { name: "STEYX", template: "STEYX(…)", rank: 1 },
    FnSig { name: "SUBSTITUTE", template: "SUBSTITUTE(text, old_text, new_text, [instance_num])", rank: 0 },
    FnSig { name: "SUBTOTAL", template: "SUBTOTAL(function_num, ref1, [ref2], …)", rank: 0 },
    FnSig { name: "SUM", template: "SUM(number1, [number2], …)", rank: 0 },
    FnSig { name: "SUMIF", template: "SUMIF(range, criteria, [sum_range])", rank: 0 },
    FnSig { name: "SUMIFS", template: "SUMIFS(sum_range, criteria_range1, criteria1, …)", rank: 0 },
    FnSig { name: "SUMSQ", template: "SUMSQ(…)", rank: 1 },
    FnSig { name: "SUMX2MY2", template: "SUMX2MY2(…)", rank: 1 },
    FnSig { name: "SUMX2PY2", template: "SUMX2PY2(…)", rank: 1 },
    FnSig { name: "SUMXMY2", template: "SUMXMY2(…)", rank: 1 },
    FnSig { name: "SWITCH", template: "SWITCH(expression, value1, result1, …, [default])", rank: 0 },
    FnSig { name: "SYD", template: "SYD(…)", rank: 1 },
    FnSig { name: "T", template: "T(…)", rank: 1 },
    FnSig { name: "T.DIST", template: "T.DIST(…)", rank: 1 },
    FnSig { name: "T.DIST.2T", template: "T.DIST.2T(…)", rank: 1 },
    FnSig { name: "T.DIST.RT", template: "T.DIST.RT(…)", rank: 1 },
    FnSig { name: "T.INV", template: "T.INV(…)", rank: 1 },
    FnSig { name: "T.INV.2T", template: "T.INV.2T(…)", rank: 1 },
    FnSig { name: "T.TEST", template: "T.TEST(…)", rank: 1 },
    FnSig { name: "TAN", template: "TAN(number)", rank: 0 },
    FnSig { name: "TANH", template: "TANH(…)", rank: 1 },
    FnSig { name: "TBILLEQ", template: "TBILLEQ(…)", rank: 1 },
    FnSig { name: "TBILLPRICE", template: "TBILLPRICE(…)", rank: 1 },
    FnSig { name: "TBILLYIELD", template: "TBILLYIELD(…)", rank: 1 },
    FnSig { name: "TEXT", template: "TEXT(value, format_text)", rank: 0 },
    FnSig { name: "TEXTAFTER", template: "TEXTAFTER(text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found])", rank: 0 },
    FnSig { name: "TEXTBEFORE", template: "TEXTBEFORE(text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found])", rank: 0 },
    FnSig { name: "TEXTJOIN", template: "TEXTJOIN(delimiter, ignore_empty, text1, [text2], …)", rank: 0 },
    FnSig { name: "TIME", template: "TIME(hour, minute, second)", rank: 0 },
    FnSig { name: "TIMEVALUE", template: "TIMEVALUE(time_text)", rank: 0 },
    FnSig { name: "TODAY", template: "TODAY()", rank: 0 },
    FnSig { name: "TRIM", template: "TRIM(text)", rank: 0 },
    FnSig { name: "TRUE", template: "TRUE()", rank: 0 },
    FnSig { name: "TRUNC", template: "TRUNC(number, [num_digits])", rank: 0 },
    FnSig { name: "TYPE", template: "TYPE(value)", rank: 0 },
    FnSig { name: "UNICODE", template: "UNICODE(…)", rank: 1 },
    FnSig { name: "UPPER", template: "UPPER(text)", rank: 0 },
    FnSig { name: "VALUE", template: "VALUE(text)", rank: 0 },
    FnSig { name: "VALUETOTEXT", template: "VALUETOTEXT(…)", rank: 1 },
    FnSig { name: "VAR.P", template: "VAR.P(number1, [number2], …)", rank: 0 },
    FnSig { name: "VAR.S", template: "VAR.S(number1, [number2], …)", rank: 0 },
    FnSig { name: "VARA", template: "VARA(…)", rank: 1 },
    FnSig { name: "VARPA", template: "VARPA(…)", rank: 1 },
    FnSig { name: "VLOOKUP", template: "VLOOKUP(lookup_value, table_array, col_index_num, [range_lookup])", rank: 0 },
    FnSig { name: "WEEKDAY", template: "WEEKDAY(serial_number, [return_type])", rank: 0 },
    FnSig { name: "WEEKNUM", template: "WEEKNUM(serial_number, [return_type])", rank: 0 },
    FnSig { name: "WEIBULL.DIST", template: "WEIBULL.DIST(…)", rank: 1 },
    FnSig { name: "WORKDAY", template: "WORKDAY(start_date, days, [holidays])", rank: 0 },
    FnSig { name: "WORKDAY.INTL", template: "WORKDAY.INTL(start_date, days, [weekend], [holidays])", rank: 0 },
    FnSig { name: "XIRR", template: "XIRR(…)", rank: 1 },
    FnSig { name: "XLOOKUP", template: "XLOOKUP(lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode])", rank: 0 },
    FnSig { name: "XNPV", template: "XNPV(…)", rank: 1 },
    FnSig { name: "XOR", template: "XOR(logical1, [logical2], …)", rank: 0 },
    FnSig { name: "YEAR", template: "YEAR(serial_number)", rank: 0 },
    FnSig { name: "YEARFRAC", template: "YEARFRAC(start_date, end_date, [basis])", rank: 0 },
    FnSig { name: "Z.TEST", template: "Z.TEST(…)", rank: 1 },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A template is the generic fallback when it is exactly `NAME(…)` for that name.
    fn is_fallback(sig: &FnSig) -> bool {
        sig.template == format!("{}(…)", sig.name)
    }

    #[test]
    fn catalog_has_345_unique_nonempty_names() {
        assert_eq!(
            FUNCTIONS.len(),
            345,
            "the 345 engine-registered names (D1.3)"
        );
        let mut seen = std::collections::HashSet::new();
        for f in FUNCTIONS {
            assert!(!f.name.is_empty(), "name is non-empty");
            assert!(
                !f.template.is_empty(),
                "template is non-empty for {}",
                f.name
            );
            assert!(
                f.template.contains('('),
                "template has a paren for {}",
                f.name
            );
            assert!(seen.insert(f.name), "name {} is unique", f.name);
        }
    }

    #[test]
    fn common_names_have_real_templates() {
        // Every rank-0 (common) entry carries an authored template, not the `NAME(…)` fallback.
        let common = FUNCTIONS.iter().filter(|f| f.rank == 0).count();
        assert!(common >= 150, "the common set is ~150 names, got {common}");
        for f in FUNCTIONS.iter().filter(|f| f.rank == 0) {
            assert!(
                !is_fallback(f),
                "common name {} must have a real template",
                f.name
            );
        }
    }

    #[test]
    fn complete_orders_exact_then_rank_then_alpha() {
        // Exact full-name match sorts first even against shorter-name siblings.
        let sum = complete("sum");
        assert_eq!(sum.first().map(|f| f.name), Some("SUM"), "exact SUM first");

        // "su" groups rank-0 (common) before rank-1, alphabetical within a rank.
        let su = complete("su");
        assert!(su.len() >= 3, "several SU* functions");
        // All matches share the prefix.
        assert!(su.iter().all(|f| f.name.starts_with("SU")));
        // Ranks are non-decreasing across the ordered list.
        let ranks: Vec<u16> = su.iter().map(|f| f.rank).collect();
        assert!(
            ranks.windows(2).all(|w| w[0] <= w[1]),
            "rank non-decreasing: {ranks:?}"
        );
        // Within the leading rank-0 block, names are alphabetical.
        let common_block: Vec<&str> = su
            .iter()
            .take_while(|f| f.rank == 0)
            .map(|f| f.name)
            .collect();
        let mut sorted = common_block.clone();
        sorted.sort();
        assert_eq!(common_block, sorted, "alphabetical within a rank");

        // Case-insensitive.
        assert_eq!(
            complete("SU").iter().map(|f| f.name).collect::<Vec<_>>(),
            complete("su").iter().map(|f| f.name).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn complete_empty_and_no_match_are_empty() {
        assert!(complete("").is_empty());
        assert!(complete("zzz").is_empty());
    }

    #[test]
    fn signature_is_case_insensitive() {
        let a = signature("sumif").expect("SUMIF");
        let b = signature("SUMIF").expect("SUMIF");
        assert_eq!(a, b);
        assert_eq!(a.template, "SUMIF(range, criteria, [sum_range])");
        assert!(signature("definitely_not_a_function").is_none());
    }

    #[test]
    fn fn_edit_context_truth_table() {
        // Simple prefix after `=`.
        let c = fn_edit_context("=su", 3).expect("=su");
        assert_eq!(c.token_start, 1);
        assert_eq!(c.prefix, "su");

        // Inside a 2nd argument of a call.
        let text = "=SUM(A1,su";
        let c = fn_edit_context(text, text.len()).expect("2nd arg");
        assert_eq!(c.prefix, "su");
        assert_eq!(c.token_start, text.len() - 2);

        // After an operator.
        assert_eq!(
            fn_edit_context("=1+su", 5).map(|c| c.prefix),
            Some("su".into())
        );

        // Bare cell reference → None (no function shares "A1").
        assert!(fn_edit_context("=A1", 3).is_none());

        // Real function whose prefix looks ref-shaped → Some.
        assert_eq!(
            fn_edit_context("=LOG10", 6).map(|c| c.prefix),
            Some("LOG10".into())
        );
        assert_eq!(
            fn_edit_context("=ATAN2", 6).map(|c| c.prefix),
            Some("ATAN2".into())
        );

        // Inside a string literal → None.
        assert!(fn_edit_context("=\"su", 4).is_none());

        // No leading `=` → None.
        assert!(fn_edit_context("su", 2).is_none());

        // Caret in the middle of the token still completes on the left part.
        let c = fn_edit_context("=su+1", 3).expect("caret after su");
        assert_eq!(c.prefix, "su");

        // A token that is not in function position (follows a digit) → None.
        assert!(fn_edit_context("=1su", 4).is_none());

        // Bare `=` (no identifier char) → None.
        assert!(fn_edit_context("=", 1).is_none());
    }

    #[test]
    fn enclosing_fn_name_finds_call() {
        assert_eq!(enclosing_fn_name("=SUM(A1,", 8), Some("SUM"));

        // Nested: caret inside the inner call names the inner function.
        let text = "=IF(SUM(A1,";
        assert_eq!(enclosing_fn_name(text, text.len()), Some("SUM"));

        // After the inner call closes, the outer call encloses the caret.
        let text = "=IF(SUM(A1,A2),";
        assert_eq!(enclosing_fn_name(text, text.len()), Some("IF"));

        // Not inside any call.
        assert!(enclosing_fn_name("=A1", 3).is_none());
        assert!(enclosing_fn_name("=A1+B2", 6).is_none());

        // A paren with no preceding identifier (grouping) is not a call.
        assert!(enclosing_fn_name("=(A1+", 5).is_none());
    }
}
