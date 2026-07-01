//! Typed spreadsheet errors, so the golden-file harness compares errors as **typed
//! errors, not strings** (functional_spec SP3, architecture §3).
//!
//! IronCalc's public read path (`get_cell_value_by_index` → `CellValue`, and thus the
//! harness's `EngineValue`) has **no `Error` variant**: an error cell comes back as a
//! *string* like `"#DIV/0!"`. Excel's own error tokens are likewise textual. Comparing
//! those strings directly is brittle (spelling/locale/format drift). Instead we parse
//! both the engine's output and the case's expected error into this enum and compare
//! the **variants**. The variant set mirrors `ironcalc_base::expressions::token::Error`
//! plus Excel's `#GETTING_DATA` (which IronCalc has no equivalent for).

use std::fmt;

/// A spreadsheet error kind, independent of the exact textual spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedError {
    /// `#NULL!` — a null intersection of ranges.
    Null,
    /// `#DIV/0!` — division by zero.
    Div0,
    /// `#VALUE!` — wrong type of argument/operand.
    Value,
    /// `#REF!` — invalid cell reference.
    Ref,
    /// `#NAME?` — unrecognized name (e.g. an unimplemented function).
    Name,
    /// `#NUM!` — invalid numeric value.
    Num,
    /// `#N/A` — value not available.
    Na,
    /// `#SPILL!` — a dynamic array cannot spill.
    Spill,
    /// `#CALC!` — a calculation engine error (e.g. empty array).
    Calc,
    /// `#CIRC!` — a circular reference (IronCalc-specific spelling).
    Circ,
    /// `#GETTING_DATA` — Excel's "external data loading" placeholder (no IronCalc peer).
    GettingData,
    /// `#ERROR!` / `#N/IMPL!` — a generic parse/not-implemented error IronCalc emits
    /// when it cannot even build the formula. Kept distinct so "the function is missing"
    /// is visible in results rather than masquerading as a real Excel error.
    GenericOrNotImpl,
}

impl TypedError {
    /// Parses a spreadsheet-error string (either an IronCalc or an Excel spelling) into
    /// a [`TypedError`]. Returns `None` for a non-error string (a normal text value).
    ///
    /// Recognizes the canonical English tokens; whitespace is trimmed and matching is
    /// case-insensitive so minor formatting never turns a real error into a miss.
    pub fn parse(s: &str) -> Option<TypedError> {
        let t = s.trim().to_ascii_uppercase();
        // Must look like an error token to avoid classifying ordinary text as an error.
        if !t.starts_with('#') {
            return None;
        }
        let normalized = t.replace(' ', "");
        Some(match normalized.as_str() {
            "#NULL!" => TypedError::Null,
            "#DIV/0!" | "#DIV0!" => TypedError::Div0,
            "#VALUE!" => TypedError::Value,
            "#REF!" => TypedError::Ref,
            "#NAME?" => TypedError::Name,
            "#NUM!" => TypedError::Num,
            "#N/A" | "#NA" => TypedError::Na,
            "#SPILL!" => TypedError::Spill,
            "#CALC!" => TypedError::Calc,
            "#CIRC!" => TypedError::Circ,
            "#GETTING_DATA" | "#GETTINGDATA" => TypedError::GettingData,
            "#ERROR!" | "#N/IMPL!" | "#N/IMPL" | "#NIMPL!" => TypedError::GenericOrNotImpl,
            _ => return None,
        })
    }

    /// The canonical Excel spelling for reporting.
    pub fn canonical_str(self) -> &'static str {
        match self {
            TypedError::Null => "#NULL!",
            TypedError::Div0 => "#DIV/0!",
            TypedError::Value => "#VALUE!",
            TypedError::Ref => "#REF!",
            TypedError::Name => "#NAME?",
            TypedError::Num => "#NUM!",
            TypedError::Na => "#N/A",
            TypedError::Spill => "#SPILL!",
            TypedError::Calc => "#CALC!",
            TypedError::Circ => "#CIRC!",
            TypedError::GettingData => "#GETTING_DATA",
            TypedError::GenericOrNotImpl => "#ERROR!",
        }
    }
}

impl fmt::Display for TypedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.canonical_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::expressions::token::Error as IcError;

    /// Every error string IronCalc's `Display for Error` can emit parses to a
    /// `TypedError` — so an error the engine returns is never silently unrecognized.
    #[test]
    fn parses_all_ironcalc_tokens() {
        let all = [
            IcError::REF,
            IcError::NAME,
            IcError::VALUE,
            IcError::DIV,
            IcError::NA,
            IcError::NUM,
            IcError::ERROR,
            IcError::NIMPL,
            IcError::SPILL,
            IcError::CALC,
            IcError::CIRC,
            IcError::NULL,
        ];
        for e in all {
            let s = e.to_string();
            assert!(
                TypedError::parse(&s).is_some(),
                "IronCalc token {s:?} did not parse to a TypedError"
            );
        }
    }

    /// Excel and IronCalc spellings of the six everyday errors map to one variant each.
    #[test]
    fn excel_and_ironcalc_aliases_agree() {
        assert_eq!(TypedError::parse("#DIV/0!"), Some(TypedError::Div0));
        assert_eq!(TypedError::parse("#N/A"), Some(TypedError::Na));
        assert_eq!(TypedError::parse("#NA"), Some(TypedError::Na));
        assert_eq!(TypedError::parse("#NAME?"), Some(TypedError::Name));
        assert_eq!(TypedError::parse("#NUM!"), Some(TypedError::Num));
        assert_eq!(TypedError::parse("#REF!"), Some(TypedError::Ref));
        assert_eq!(TypedError::parse("#VALUE!"), Some(TypedError::Value));
    }

    /// Non-error text is not misclassified as an error.
    #[test]
    fn ordinary_text_is_not_an_error() {
        assert_eq!(TypedError::parse("hello"), None);
        assert_eq!(TypedError::parse(""), None);
        assert_eq!(TypedError::parse("#hashtag"), None);
        assert_eq!(TypedError::parse("42"), None);
    }
}
