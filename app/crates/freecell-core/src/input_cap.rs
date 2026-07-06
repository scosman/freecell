//! Formula **input-cap validator** — the load-bearing robustness gate from round-3 D
//! (`experiments/round-3/D-robustness/findings.md`, `functional_spec.md §3.3`,
//! `architecture.md §3, §5`).
//!
//! IronCalc's formula parser + evaluator are recursive with **no depth cap**, so a
//! sufficiently deep/long formula overflows the worker's stack — an **abort** (SIGABRT via
//! the guard page) that `catch_unwind` cannot catch and that kills the whole process. The
//! only real fix is to reject over-budget formulas *before* they reach the engine. D
//! measured the container-floor ceilings (~490 nesting depth / ~2832 flat terms on a
//! spawned thread's default stack; the app's 64 MiB worker raises these ~30×, but the cap
//! is what eliminates the class). We cap at **Excel's own limits** — length ≤ 8192 chars,
//! nesting depth ≤ 64 — which are both spec-compatible and far under every measured
//! ceiling.
//!
//! The cap is scoped to **formulas** (leading `=`): only formulas run through the recursive
//! parser, so a long plain-text value is stored, not parsed, and cannot overflow (D §2/§5).

use crate::limits;

/// The maximum formula length (chars) — Excel's limit and the length cap.
pub const MAX_INPUT_LEN: usize = limits::MAX_INPUT_LEN;
/// The maximum parenthesis-nesting depth — Excel's limit and the depth cap.
pub const MAX_NESTING_DEPTH: usize = limits::MAX_NESTING_DEPTH;

/// Why an input was rejected. Both variants carry the offending measurement and the cap so
/// the UI can show a precise message ("Formula too long", "Formula too deeply nested").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRejection {
    /// The formula's character length exceeds [`MAX_INPUT_LEN`].
    TooLong { len: usize, max: usize },
    /// The formula's parenthesis-nesting depth exceeds [`MAX_NESTING_DEPTH`].
    TooDeeplyNested { depth: usize, max: usize },
}

impl InputRejection {
    /// The user-facing cap-error message (`functional_spec.md §4.2`) shown in the popover
    /// under the active editor. The cap value is grouped with thousands separators
    /// (8192 → "8,192") to match Excel's phrasing.
    pub fn message(&self) -> String {
        match self {
            InputRejection::TooLong { max, .. } => {
                format!(
                    "Formula too long (max {} characters)",
                    group_thousands(*max)
                )
            }
            InputRejection::TooDeeplyNested { max, .. } => {
                format!(
                    "Formula nested too deeply (max {} levels)",
                    group_thousands(*max)
                )
            }
        }
    }
}

/// Formats `n` with `,` thousands separators (`8192` → `"8,192"`). Used only for the small
/// cap values in [`InputRejection::message`].
fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    // The first group has `len % 3` digits (or 3 when the length is a multiple of 3); a
    // separator precedes every full group of three after it.
    let first = digits.len() % 3;
    for (i, ch) in digits.chars().enumerate() {
        if i != 0 && i >= first && (i - first).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Validates a cell input before it reaches the engine. Non-formula inputs (not starting
/// with `=`) always pass — they never touch the recursive parser. Formulas are rejected if
/// they exceed the length or nesting-depth cap.
pub fn validate_input(input: &str) -> Result<(), InputRejection> {
    // Only formulas run through the recursive parser (round-3 D §2/§5). A leading `=`
    // marks a formula; Excel allows no leading whitespace before it.
    if !input.starts_with('=') {
        return Ok(());
    }

    let len = input.chars().count();
    if len > MAX_INPUT_LEN {
        return Err(InputRejection::TooLong {
            len,
            max: MAX_INPUT_LEN,
        });
    }

    let depth = max_paren_depth(input);
    if depth > MAX_NESTING_DEPTH {
        return Err(InputRejection::TooDeeplyNested {
            depth,
            max: MAX_NESTING_DEPTH,
        });
    }

    Ok(())
}

/// The maximum parenthesis-nesting depth of a formula, ignoring parentheses inside
/// double-quoted string literals (so `="((("` doesn't count as nesting). IronCalc's
/// grammar quotes strings with `"` and escapes an embedded quote by doubling it (`""`).
fn max_paren_depth(input: &str) -> usize {
    let mut depth: usize = 0;
    let mut max = 0usize;
    let mut in_string = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '"' {
                // A doubled quote is an escaped quote, still inside the string.
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' => {
                depth += 1;
                max = max.max(depth);
            }
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_formula() {
        assert_eq!(validate_input("=SUM(A1:A5)"), Ok(()));
        assert_eq!(validate_input("=IF(A1>0, B1*2, C1)"), Ok(()));
        assert_eq!(validate_input("=42"), Ok(()));
    }

    #[test]
    fn accepts_non_formula_text() {
        // Non-formulas never reach the recursive parser, so even a very long one passes.
        let long_text = "x".repeat(MAX_INPUT_LEN * 4);
        assert_eq!(validate_input(&long_text), Ok(()));
        // A plain value with many parens is text, not a formula → allowed.
        let parens = "(".repeat(MAX_NESTING_DEPTH * 10);
        assert_eq!(validate_input(&parens), Ok(()));
    }

    #[test]
    fn rejects_over_length() {
        // `=` + 8192 `1`s = 8193 chars, one over the cap.
        let formula = format!("={}", "1".repeat(MAX_INPUT_LEN));
        assert_eq!(
            validate_input(&formula),
            Err(InputRejection::TooLong {
                len: MAX_INPUT_LEN + 1,
                max: MAX_INPUT_LEN,
            })
        );
    }

    #[test]
    fn rejects_over_nesting_depth() {
        // 65 nested parens around a 1 → depth 65, one over the cap. Length stays small.
        let formula = format!(
            "={}1{}",
            "(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1)
        );
        assert_eq!(
            validate_input(&formula),
            Err(InputRejection::TooDeeplyNested {
                depth: MAX_NESTING_DEPTH + 1,
                max: MAX_NESTING_DEPTH,
            })
        );
    }

    #[test]
    fn rejects_round3_d_deep_parens_reproducer() {
        // The D "nested parens" abort shape (`=((…1…))`) at its ~2755 abort depth — the
        // depth cap rejects it long before the engine can overflow the stack.
        let d = 2755;
        let formula = format!("={}1{}", "(".repeat(d), ")".repeat(d));
        assert!(matches!(
            validate_input(&formula),
            Err(InputRejection::TooDeeplyNested { .. })
        ));
    }

    #[test]
    fn rejects_round3_d_flat_chain_reproducer() {
        // The D "wide flat" abort shape (`=1+1+…`) at ~11897 terms. It has no nesting, so
        // the *length* cap is what rejects it (≈23793 chars).
        let terms = 11_897;
        let mut formula = String::from("=1");
        for _ in 1..terms {
            formula.push_str("+1");
        }
        assert!(matches!(
            validate_input(&formula),
            Err(InputRejection::TooLong { .. })
        ));
    }

    #[test]
    fn paren_in_string_literal_not_counted() {
        // Deeply-"nested" parens that live inside a string literal are not real nesting.
        let inner = "(".repeat(MAX_NESTING_DEPTH * 4);
        let formula = format!("=\"{inner}\"");
        assert_eq!(validate_input(&formula), Ok(()));
        // A doubled-quote escape keeps us inside the string; the trailing `(` is literal.
        assert_eq!(validate_input("=\"a\"\"(b\""), Ok(()));
    }

    #[test]
    fn boundary_at_exactly_the_caps() {
        // Exactly at the cap is allowed; one over is not (guards an off-by-one).
        let at_len = format!("={}", "1".repeat(MAX_INPUT_LEN - 1)); // total = MAX_INPUT_LEN
        assert_eq!(validate_input(&at_len), Ok(()));

        let at_depth = format!(
            "={}1{}",
            "(".repeat(MAX_NESTING_DEPTH),
            ")".repeat(MAX_NESTING_DEPTH)
        );
        assert_eq!(validate_input(&at_depth), Ok(()));
    }

    #[test]
    fn rejection_messages_match_spec() {
        // `functional_spec.md §4.2` pins these exact strings.
        assert_eq!(
            InputRejection::TooLong {
                len: 9000,
                max: MAX_INPUT_LEN
            }
            .message(),
            "Formula too long (max 8,192 characters)"
        );
        assert_eq!(
            InputRejection::TooDeeplyNested {
                depth: 70,
                max: MAX_NESTING_DEPTH
            }
            .message(),
            "Formula nested too deeply (max 64 levels)"
        );
    }

    #[test]
    fn group_thousands_inserts_separators() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(64), "64");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(8192), "8,192");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(1_000_000), "1,000,000");
    }
}
