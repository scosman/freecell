//! Runtime probe: empirically confirm which canonical functions IronCalc actually
//! recognizes, by evaluating a minimal call and checking for `#NAME?` (the token
//! IronCalc emits for an unknown function).
//!
//! This corroborates the source-extracted static list (functional_spec §7: "345
//! registered ≠ 345 correct; a count is not an audit"). The static list stays
//! authoritative for the coverage %; the probe catches any name that is *declared* but
//! not actually wired (or a name the static extraction mis-handled). Discrepancies are
//! reported, not silently reconciled.

use round2_harness::{EngineValue, IronCalcEngine, SpreadsheetEngine};

use crate::typed_error::TypedError;

/// Result of probing one function name.
#[derive(Debug, Clone, PartialEq)]
pub enum Recognition {
    /// Engine recognized the function (returned a value or a non-`#NAME?` error).
    Recognized,
    /// Engine returned `#NAME?` — the function is not implemented.
    Unknown,
}

/// Builds a probe call for `name`. We don't need the *right* answer, only to
/// distinguish "IronCalc knows this name" from "it doesn't". Verified empirically
/// against IronCalc 0.7.1:
///
/// - an **unknown** function name → `#NAME?`
/// - a **known** function with wrong arity/args → `#ERROR!` (a parse/arg error, NOT
///   `#NAME?`) or a real value/typed error.
///
/// So a single generic call suffices: `#NAME?` ⇒ Unknown, anything else ⇒ Recognized.
fn probe_formula(name: &str) -> String {
    format!("={name}(1,1,1)")
}

/// Probes a single function name against a fresh engine. Only `#NAME?` counts as
/// unknown; `#ERROR!` (wrong-arity), `#VALUE!`, `#NUM!`, a value, etc. all mean the name
/// is registered.
pub fn probe_one(name: &str) -> Recognition {
    let mut engine = IronCalcEngine::new_blank();
    engine.set_formula(0, 0, &probe_formula(name));
    engine.recompute();
    match engine.get_value(0, 0) {
        EngineValue::Text(t) => match TypedError::parse(&t) {
            Some(TypedError::Name) => Recognition::Unknown,
            _ => Recognition::Recognized,
        },
        _ => Recognition::Recognized,
    }
}

/// A row comparing the static (source-extracted) verdict with the runtime probe.
#[derive(Debug, Clone)]
pub struct ProbeRow {
    pub name: String,
    pub static_supported: bool,
    pub probe_recognized: bool,
    pub agree: bool,
}

/// Probes every canonical function and compares to the static supported-set.
pub fn probe_all(
    canonical: &[String],
    static_supported: &std::collections::BTreeSet<String>,
) -> Vec<ProbeRow> {
    canonical
        .iter()
        .map(|name| {
            let probe_recognized = probe_one(name) == Recognition::Recognized;
            let static_supported = static_supported.contains(name);
            ProbeRow {
                name: name.clone(),
                static_supported,
                probe_recognized,
                agree: static_supported == probe_recognized,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Core functions the engine definitely has are probed as recognized; a deliberately
    /// fake name is probed as unknown.
    #[test]
    fn probe_agrees_with_static_on_core() {
        assert_eq!(probe_one("SUM"), Recognition::Recognized);
        assert_eq!(probe_one("IF"), Recognition::Recognized);
        assert_eq!(probe_one("VLOOKUP"), Recognition::Recognized);
        assert_eq!(probe_one("ZZ_NOT_A_FUNCTION"), Recognition::Unknown);
    }

    /// A function IronCalc lacks (dynamic-array SEQUENCE) probes as unknown.
    #[test]
    fn missing_function_probes_unknown() {
        assert_eq!(probe_one("SEQUENCE"), Recognition::Unknown);
    }
}
