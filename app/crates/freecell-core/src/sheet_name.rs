//! Sheet-name validator — the xlsx naming rules (`functional_spec.md §3.7`,
//! `components/app_shell.md §Sheet tab bar`).
//!
//! The same rules run UI-side (inline rename) and worker-side (re-check before applying),
//! so both live here in `freecell-core`. Excel's constraints: non-empty, ≤ 31 chars, none
//! of `: \ / ? * [ ]`, cannot begin or end with an apostrophe, and case-insensitively
//! unique within the workbook.

/// The maximum sheet-name length (Excel's limit).
pub const MAX_SHEET_NAME_LEN: usize = 31;

/// The characters Excel forbids anywhere in a sheet name.
pub const ILLEGAL_CHARS: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

/// Why a proposed sheet name is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetNameError {
    /// Empty or all-whitespace.
    Empty,
    /// Longer than [`MAX_SHEET_NAME_LEN`] characters.
    TooLong { len: usize },
    /// Contains one of [`ILLEGAL_CHARS`].
    IllegalChar(char),
    /// Begins or ends with an apostrophe (`'`).
    EdgeApostrophe,
    /// Case-insensitively duplicates an existing sheet name.
    Duplicate,
}

/// Validates a proposed sheet `name` against the `existing` names of the *other* sheets
/// (the caller excludes the sheet being renamed, so renaming to the same name passes).
/// Uniqueness is case-insensitive, matching Excel.
pub fn validate_sheet_name(name: &str, existing: &[&str]) -> Result<(), SheetNameError> {
    // Empty or all-whitespace (covers Excel's "not just an apostrophe-wrapped blank").
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err(SheetNameError::Empty);
    }

    let len = name.chars().count();
    if len > MAX_SHEET_NAME_LEN {
        return Err(SheetNameError::TooLong { len });
    }

    if let Some(&bad) = ILLEGAL_CHARS.iter().find(|&&c| name.contains(c)) {
        return Err(SheetNameError::IllegalChar(bad));
    }

    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(SheetNameError::EdgeApostrophe);
    }

    if existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
        return Err(SheetNameError::Duplicate);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        assert_eq!(validate_sheet_name("Sheet1", &["Sheet2", "Sales"]), Ok(()));
        assert_eq!(validate_sheet_name("Q1 Budget", &[]), Ok(()));
        // Exactly 31 chars is allowed.
        let at_max = "a".repeat(MAX_SHEET_NAME_LEN);
        assert_eq!(validate_sheet_name(&at_max, &[]), Ok(()));
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(validate_sheet_name("", &[]), Err(SheetNameError::Empty));
    }

    #[test]
    fn rejects_blank() {
        assert_eq!(validate_sheet_name("   ", &[]), Err(SheetNameError::Empty));
    }

    #[test]
    fn rejects_too_long() {
        let too_long = "a".repeat(MAX_SHEET_NAME_LEN + 1);
        assert_eq!(
            validate_sheet_name(&too_long, &[]),
            Err(SheetNameError::TooLong {
                len: MAX_SHEET_NAME_LEN + 1
            })
        );
    }

    #[test]
    fn rejects_each_illegal_char() {
        for c in ILLEGAL_CHARS {
            let name = format!("Sheet{c}1");
            assert_eq!(
                validate_sheet_name(&name, &[]),
                Err(SheetNameError::IllegalChar(c)),
                "should reject {c:?}"
            );
        }
    }

    #[test]
    fn rejects_edge_apostrophe() {
        assert_eq!(
            validate_sheet_name("'Sheet", &[]),
            Err(SheetNameError::EdgeApostrophe)
        );
        assert_eq!(
            validate_sheet_name("Sheet'", &[]),
            Err(SheetNameError::EdgeApostrophe)
        );
        // An apostrophe in the middle is fine.
        assert_eq!(validate_sheet_name("Bob's Sheet", &[]), Ok(()));
    }

    #[test]
    fn rejects_case_insensitive_duplicate() {
        assert_eq!(
            validate_sheet_name("sales", &["Sheet1", "SALES"]),
            Err(SheetNameError::Duplicate)
        );
    }

    #[test]
    fn allows_rename_to_same_name() {
        // The caller passes only the *other* sheets, so renaming to the current name (not
        // in `existing`) is accepted — a same-name commit is a no-op, not a duplicate.
        assert_eq!(validate_sheet_name("Sales", &["Sheet1", "Sheet2"]), Ok(()));
    }
}
