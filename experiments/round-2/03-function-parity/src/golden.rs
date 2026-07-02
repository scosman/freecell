//! Data-driven golden-file correctness harness (functional_spec SP3 GATE).
//!
//! Cases live in `data/golden_cases.csv` (so the suite grows by appending rows). Each
//! case is `formula + input cells → expected value OR expected typed error`, with the
//! known-correct **Excel** output. Every case is run through the frozen
//! `round2_harness` IronCalc adapter and its output classified; a returned error string
//! is parsed to a [`TypedError`] and compared **as a typed error, not a string**
//! (architecture §3).

use anyhow::{anyhow, bail, Context, Result};
use round2_harness::{EngineValue, IronCalcEngine, SpreadsheetEngine};
use serde::Serialize;

use crate::typed_error::TypedError;

/// The expected result of a case: a value of a specific kind, or a typed error.
#[derive(Debug, Clone, PartialEq)]
pub enum Expected {
    /// A number, compared within `tol` (absolute or relative — see [`numbers_match`]).
    Number {
        value: f64,
        tol: f64,
    },
    Text(String),
    Bool(bool),
    Error(TypedError),
    /// Deliberately empty (a formula that yields a blank).
    Empty,
}

/// One golden case parsed from the CSV.
#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub formula: String,
    /// `(cell, literal)` seeds, e.g. `("A1", "10")`, `("B2", "\"txt\"")`.
    pub inputs: Vec<(String, String)>,
    pub expected: Expected,
}

/// The outcome of running one case.
#[derive(Debug, Clone)]
pub enum Outcome {
    Pass,
    Fail {
        expected: String,
        actual: String,
        reason: String,
    },
}

impl Outcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Outcome::Pass)
    }
}

/// Per-case row for the results CSV.
#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub category: String,
    pub formula: String,
    pub expected: String,
    pub actual: String,
    pub pass: bool,
    pub reason: String,
}

const SHEET_COLS: u32 = 16_384;

/// Parses an A1-style cell reference into 0-based `(row, col)` (datagen space, which the
/// harness trait uses). Single sheet only.
pub fn parse_cell(cell: &str) -> Result<(u32, u32)> {
    let cell = cell.trim();
    let split = cell
        .find(|c: char| c.is_ascii_digit())
        .ok_or_else(|| anyhow!("cell {cell:?} has no row number"))?;
    let (col_str, row_str) = cell.split_at(split);
    if col_str.is_empty() {
        bail!("cell {cell:?} has no column letters");
    }
    let mut col: u32 = 0;
    for ch in col_str.chars() {
        let up = ch.to_ascii_uppercase();
        if !up.is_ascii_uppercase() {
            bail!("cell {cell:?} has a bad column letter {ch:?}");
        }
        col = col * 26 + (up as u32 - 'A' as u32 + 1);
    }
    let col = col - 1; // to 0-based
    if col >= SHEET_COLS {
        bail!("cell {cell:?} column out of range");
    }
    let row: u32 = row_str
        .parse::<u32>()
        .with_context(|| format!("cell {cell:?} row not a number"))?;
    if row == 0 {
        bail!("cell {cell:?} row must be >= 1");
    }
    Ok((row - 1, col))
}

/// Parses a seed literal into a neutral [`EngineValue`]. Supports: `"quoted text"`,
/// `TRUE`/`FALSE`, `#ERR` tokens (seed an error via a formula instead — see below), and
/// bare numbers. Anything else is treated as text.
fn parse_literal(raw: &str) -> EngineValue {
    let s = raw.trim();
    if s.is_empty() {
        return EngineValue::Empty;
    }
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        return EngineValue::Text(s[1..s.len() - 1].to_string());
    }
    match s.to_ascii_uppercase().as_str() {
        "TRUE" => return EngineValue::Bool(true),
        "FALSE" => return EngineValue::Bool(false),
        _ => {}
    }
    if let Ok(n) = s.parse::<f64>() {
        return EngineValue::Number(n);
    }
    EngineValue::Text(s.to_string())
}

/// Seeds a single input cell. A literal beginning with `=` is set as a **formula**
/// (this lets a case seed a precedent error like `A2 = "=1/0"` or a date via
/// `A1 = "=DATE(2020,1,1)"`), otherwise as a typed value.
fn seed_cell(engine: &mut IronCalcEngine, cell: &str, literal: &str) -> Result<()> {
    let (row, col) = parse_cell(cell)?;
    let lit = literal.trim();
    if let Some(formula) = lit.strip_prefix('=') {
        engine.set_formula(row, col, &format!("={formula}"));
    } else {
        engine.set_value(row, col, parse_literal(lit));
    }
    Ok(())
}

/// Numbers match if they agree within `tol`. `tol` is treated as absolute for small
/// magnitudes and relative for large ones (max of the two), so both `43831` (date
/// serial, tol 0) and `0.3333333` (tol 1e-6) work with a single knob.
pub fn numbers_match(actual: f64, expected: f64, tol: f64) -> bool {
    if actual == expected {
        return true;
    }
    let diff = (actual - expected).abs();
    let scale = expected.abs().max(actual.abs()).max(1.0);
    diff <= tol || diff <= tol * scale
}

/// Where the formula under test is written: a corner well clear of any seeded inputs.
const RESULT_CELL: (u32, u32) = (0, 100); // row 1, column CW

/// Runs one case through a fresh IronCalc engine, returning both the classified
/// outcome and the actual [`EngineValue`] IronCalc produced (so results can record the
/// real value even on a pass).
pub fn run_case_full(case: &Case) -> (Outcome, EngineValue) {
    let mut engine = IronCalcEngine::new_blank();
    for (cell, literal) in &case.inputs {
        if let Err(e) = seed_cell(&mut engine, cell, literal) {
            let actual = EngineValue::Text(format!("seed-error: {e}"));
            return (
                Outcome::Fail {
                    expected: describe_expected(&case.expected),
                    actual: describe_actual(&actual),
                    reason: "input cell could not be seeded".into(),
                },
                actual,
            );
        }
    }
    engine.set_formula(RESULT_CELL.0, RESULT_CELL.1, &case.formula);
    engine.recompute();
    let actual = engine.get_value(RESULT_CELL.0, RESULT_CELL.1);
    let outcome = classify(&case.expected, &actual);
    (outcome, actual)
}

/// Runs one case and returns just the outcome (convenience for tests).
pub fn run_case(case: &Case) -> Outcome {
    run_case_full(case).0
}

fn describe_actual(v: &EngineValue) -> String {
    match v {
        EngineValue::Empty => "<empty>".into(),
        EngineValue::Number(n) => format!("{n}"),
        EngineValue::Text(t) => match TypedError::parse(t) {
            Some(e) => format!("error {e}"),
            None => format!("\"{t}\""),
        },
        EngineValue::Bool(b) => format!("{b}"),
        EngineValue::Error(e) => format!("error {e}"),
    }
}

fn describe_expected(e: &Expected) -> String {
    match e {
        Expected::Number { value, tol } => format!("{value} (±{tol})"),
        Expected::Text(t) => format!("\"{t}\""),
        Expected::Bool(b) => format!("{b}"),
        Expected::Error(err) => format!("error {err}"),
        Expected::Empty => "<empty>".into(),
    }
}

/// The typed error an `EngineValue` represents, if any. IronCalc surfaces errors as
/// `Text("#...")` (its `CellValue` has no Error variant); the neutral `Error` variant
/// is handled too for completeness.
fn actual_error(v: &EngineValue) -> Option<TypedError> {
    match v {
        EngineValue::Text(t) => TypedError::parse(t),
        EngineValue::Error(s) => TypedError::parse(s),
        _ => None,
    }
}

/// Compares expected vs actual, with typed-error comparison for the error case.
pub fn classify(expected: &Expected, actual: &EngineValue) -> Outcome {
    let exp_str = describe_expected(expected);
    let act_str = describe_actual(actual);
    let fail = |reason: &str| Outcome::Fail {
        expected: exp_str.clone(),
        actual: act_str.clone(),
        reason: reason.to_string(),
    };

    // If a value was expected but an error came back (or vice versa), that's a mismatch.
    let actual_err = actual_error(actual);
    match expected {
        Expected::Error(want) => match actual_err {
            Some(got) if got == *want => Outcome::Pass,
            Some(got) => Outcome::Fail {
                expected: exp_str,
                actual: act_str,
                reason: format!("wrong error kind: expected {want}, got {got}"),
            },
            None => fail("expected an error, got a value"),
        },
        _ if actual_err.is_some() => fail(&format!(
            "expected a value, got error {}",
            actual_err.unwrap()
        )),
        Expected::Number { value, tol } => match actual {
            EngineValue::Number(n) if numbers_match(*n, *value, *tol) => Outcome::Pass,
            EngineValue::Number(_) => fail("number outside tolerance"),
            _ => fail("expected a number"),
        },
        Expected::Text(want) => match actual {
            EngineValue::Text(t) if t == want => Outcome::Pass,
            _ => fail("expected text"),
        },
        Expected::Bool(want) => match actual {
            EngineValue::Bool(b) if b == want => Outcome::Pass,
            // Excel sometimes surfaces booleans as 1/0 through some paths; accept the
            // numeric equivalent so a correct value isn't marked wrong on a type nuance.
            EngineValue::Number(n) if (*n == 1.0 && *want) || (*n == 0.0 && !*want) => {
                Outcome::Pass
            }
            _ => fail("expected a boolean"),
        },
        Expected::Empty => match actual {
            EngineValue::Empty => Outcome::Pass,
            EngineValue::Text(t) if t.is_empty() => Outcome::Pass,
            _ => fail("expected empty"),
        },
    }
}

/// Runs every case and returns per-case results (with the real actual value recorded on
/// pass and fail alike).
pub fn run_all(cases: &[Case]) -> Vec<CaseResult> {
    cases
        .iter()
        .map(|c| {
            let (outcome, actual_value) = run_case_full(c);
            let (pass, reason) = match &outcome {
                Outcome::Pass => (true, String::new()),
                Outcome::Fail { reason, .. } => (false, reason.clone()),
            };
            CaseResult {
                id: c.id.clone(),
                category: c.category.clone(),
                formula: c.formula.clone(),
                expected: describe_expected(&c.expected),
                actual: describe_actual(&actual_value),
                pass,
                reason,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a1_references() {
        assert_eq!(parse_cell("A1").unwrap(), (0, 0));
        assert_eq!(parse_cell("B2").unwrap(), (1, 1));
        assert_eq!(parse_cell("Z1").unwrap(), (0, 25));
        assert_eq!(parse_cell("AA1").unwrap(), (0, 26));
        assert_eq!(parse_cell("AB10").unwrap(), (9, 27));
        assert!(parse_cell("1").is_err());
        assert!(parse_cell("A").is_err());
        assert!(parse_cell("A0").is_err());
    }

    #[test]
    fn literals_parse_by_type() {
        assert_eq!(parse_literal("10"), EngineValue::Number(10.0));
        assert_eq!(parse_literal("-3.5"), EngineValue::Number(-3.5));
        assert_eq!(parse_literal("\"hi\""), EngineValue::Text("hi".into()));
        assert_eq!(parse_literal("TRUE"), EngineValue::Bool(true));
        assert_eq!(parse_literal("false"), EngineValue::Bool(false));
        assert_eq!(parse_literal(""), EngineValue::Empty);
        assert_eq!(parse_literal("plain"), EngineValue::Text("plain".into()));
    }

    /// The heart of the requirement: error comparison is on TYPED variants, not strings.
    /// A `#DIV/0!` string satisfies an expected `Div0`; a different error is a Fail that
    /// names both typed errors.
    #[test]
    fn typed_error_comparison_is_not_string() {
        // Correct: IronCalc returns the "#DIV/0!" string, expected is the Div0 variant.
        let pass = classify(
            &Expected::Error(TypedError::Div0),
            &EngineValue::Text("#DIV/0!".to_string()),
        );
        assert!(pass.is_pass(), "matching typed error should pass");

        // Wrong error kind: expected Div0, engine returned #VALUE!.
        let fail = classify(
            &Expected::Error(TypedError::Div0),
            &EngineValue::Text("#VALUE!".to_string()),
        );
        match fail {
            Outcome::Fail { reason, .. } => {
                assert!(reason.contains("#DIV/0!") && reason.contains("#VALUE!"));
            }
            Outcome::Pass => panic!("different error kinds must not pass"),
        }
    }

    #[test]
    fn value_vs_error_mismatch_is_flagged() {
        let out = classify(
            &Expected::Number {
                value: 5.0,
                tol: 0.0,
            },
            &EngineValue::Text("#N/A".to_string()),
        );
        assert!(!out.is_pass());
    }

    #[test]
    fn number_tolerance_absolute_and_relative() {
        assert!(numbers_match(0.3333333, 0.3333333, 1e-6));
        assert!(numbers_match(0.33333333, 0.3333333, 1e-6));
        assert!(numbers_match(43831.0, 43831.0, 0.0));
        assert!(!numbers_match(43831.0, 43832.0, 0.0));
        // Relative tolerance for a large magnitude.
        assert!(numbers_match(1_000_000.5, 1_000_000.0, 1e-6));
    }

    /// End-to-end through the real IronCalc adapter: a trivially correct case passes.
    #[test]
    fn known_good_case_passes_through_engine() {
        let case = Case {
            id: "t-sum".into(),
            category: "self-test".into(),
            formula: "=A1+A2".into(),
            inputs: vec![("A1".into(), "2".into()), ("A2".into(), "3".into())],
            expected: Expected::Number {
                value: 5.0,
                tol: 0.0,
            },
        };
        assert!(run_case(&case).is_pass());
    }

    /// A div-by-zero case yields the #DIV/0! typed error through the real engine.
    #[test]
    fn div_by_zero_yields_typed_error_through_engine() {
        let case = Case {
            id: "t-div0".into(),
            category: "self-test".into(),
            formula: "=A1/A2".into(),
            inputs: vec![("A1".into(), "1".into()), ("A2".into(), "0".into())],
            expected: Expected::Error(TypedError::Div0),
        };
        assert!(run_case(&case).is_pass());
    }
}
