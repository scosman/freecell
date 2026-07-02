//! Loads golden cases from `data/golden_cases.csv` into [`Case`] structs.
//!
//! CSV columns: `id,category,formula,inputs,expected_kind,expected,tol`
//! - `inputs`: `;`-separated `Cell=literal` seeds, e.g. `A1=10;A2=0`. A literal
//!   beginning with `=` is seeded as a formula (used to seed a precedent error or a
//!   date). Empty `inputs` means no seeds.
//! - `expected_kind`: `number` | `text` | `bool` | `error` | `empty`.
//! - `expected`: the known-correct Excel result. For `error`, an Excel error token
//!   (`#DIV/0!`, `#N/A`, ...). For `bool`, `TRUE`/`FALSE`.
//! - `tol`: numeric tolerance (only meaningful for `number`; blank ⇒ 0).

use anyhow::{anyhow, bail, Context, Result};

use crate::golden::{Case, Expected};
use crate::typed_error::TypedError;

fn parse_inputs(field: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in field.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let eq = part
            .find('=')
            .ok_or_else(|| anyhow!("input {part:?} is not Cell=literal"))?;
        let (cell, literal) = part.split_at(eq);
        out.push((cell.trim().to_string(), literal[1..].to_string()));
    }
    Ok(out)
}

fn parse_expected(kind: &str, value: &str, tol: &str) -> Result<Expected> {
    let tol = if tol.trim().is_empty() {
        0.0
    } else {
        tol.trim()
            .parse::<f64>()
            .with_context(|| format!("bad tol {tol:?}"))?
    };
    Ok(match kind.trim() {
        "number" => Expected::Number {
            value: value
                .trim()
                .parse::<f64>()
                .with_context(|| format!("bad number {value:?}"))?,
            tol,
        },
        "text" => {
            // Strip optional surrounding quotes so an explicit empty string ("") works.
            let v = value.trim();
            let v = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
                &v[1..v.len() - 1]
            } else {
                v
            };
            Expected::Text(v.to_string())
        }
        "bool" => match value.trim().to_ascii_uppercase().as_str() {
            "TRUE" => Expected::Bool(true),
            "FALSE" => Expected::Bool(false),
            other => bail!("bad bool {other:?}"),
        },
        "error" => Expected::Error(
            TypedError::parse(value.trim())
                .ok_or_else(|| anyhow!("expected error token not recognized: {value:?}"))?,
        ),
        "empty" => Expected::Empty,
        other => bail!("unknown expected_kind {other:?}"),
    })
}

/// Parses the golden-cases CSV into [`Case`] structs, validating each row.
pub fn load_cases(path: &str) -> Result<Vec<Case>> {
    let mut rdr = csv::ReaderBuilder::new()
        .trim(csv::Trim::None)
        .from_path(path)
        .with_context(|| format!("open golden cases {path}"))?;

    let headers = rdr.headers()?.clone();
    let idx = |name: &str| -> Result<usize> {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| anyhow!("missing CSV column {name:?}"))
    };
    let (i_id, i_cat, i_formula, i_inputs, i_kind, i_expected, i_tol) = (
        idx("id")?,
        idx("category")?,
        idx("formula")?,
        idx("inputs")?,
        idx("expected_kind")?,
        idx("expected")?,
        idx("tol")?,
    );

    let mut cases = Vec::new();
    for (line, rec) in rdr.records().enumerate() {
        let rec = rec.with_context(|| format!("reading case row {}", line + 2))?;
        let get = |i: usize| rec.get(i).unwrap_or_default();
        let expected = parse_expected(get(i_kind), get(i_expected), get(i_tol))
            .with_context(|| format!("row {} (id={:?})", line + 2, get(i_id)))?;
        let inputs = parse_inputs(get(i_inputs))
            .with_context(|| format!("row {} (id={:?})", line + 2, get(i_id)))?;
        cases.push(Case {
            id: get(i_id).to_string(),
            category: get(i_cat).to_string(),
            formula: get(i_formula).to_string(),
            inputs,
            expected,
        });
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inputs_field() {
        let got = parse_inputs("A1=10;A2=0").unwrap();
        assert_eq!(
            got,
            vec![
                ("A1".to_string(), "10".to_string()),
                ("A2".to_string(), "0".to_string())
            ]
        );
        assert!(parse_inputs("").unwrap().is_empty());
        // A formula seed keeps its leading '='.
        let f = parse_inputs("A1==DATE(2020,1,1)").unwrap();
        assert_eq!(f, vec![("A1".to_string(), "=DATE(2020,1,1)".to_string())]);
    }

    #[test]
    fn parses_each_expected_kind() {
        assert_eq!(
            parse_expected("number", "5", "0").unwrap(),
            Expected::Number {
                value: 5.0,
                tol: 0.0
            }
        );
        assert_eq!(
            parse_expected("text", "\"hi\"", "").unwrap(),
            Expected::Text("hi".into())
        );
        assert_eq!(
            parse_expected("bool", "TRUE", "").unwrap(),
            Expected::Bool(true)
        );
        assert_eq!(
            parse_expected("error", "#DIV/0!", "").unwrap(),
            Expected::Error(TypedError::Div0)
        );
        assert_eq!(parse_expected("empty", "", "").unwrap(), Expected::Empty);
        assert!(parse_expected("bogus", "x", "").is_err());
    }

    /// The committed cases file loads, has >= 100 cases (the GATE), and every row is
    /// well-formed. Guards the data file as strictly as the code.
    #[test]
    fn committed_cases_load_and_meet_gate() {
        let cases = load_cases("data/golden_cases.csv").expect("load golden cases");
        assert!(
            cases.len() >= 100,
            "GATE requires >= ~100 cases, found {}",
            cases.len()
        );
        // No duplicate ids.
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate case ids in golden_cases.csv");
    }
}
