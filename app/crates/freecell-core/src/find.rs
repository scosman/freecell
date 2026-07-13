//! Pure find/replace predicates (`functional_spec.md §4.3–4.4`) — the case/whole-cell/substring
//! matching rules, GPUI-free and engine-free so the worker's used-range scan
//! ([`freecell_engine`]) and the chrome's tests share one authoritative implementation.
//!
//! **Match target = raw content** (`functional_spec.md §4.3`): the caller passes the cell's raw
//! content — the literal value for value cells, the **formula text** (`"=A1+A2"`) for formula
//! cells — so a match against a formula is a match against its text (Excel's "Look in: Formulas").
//! Replace edits that same raw content, so a replace inside a formula edits the formula text.
//!
//! Toggles: `match_case` off = case-insensitive; `whole_cell` off = substring.

/// Whether `content` matches `query` under the case / whole-cell rules
/// (`functional_spec.md §4.3`). An **empty** `query` never matches (an empty find field has no
/// matches, `§4.3`).
///
/// - `whole_cell` ⇒ the whole content must equal `query`; else a substring match.
/// - `match_case` off ⇒ case-insensitive; on ⇒ exact case.
pub fn cell_matches(content: &str, query: &str, match_case: bool, whole_cell: bool) -> bool {
    if query.is_empty() {
        return false;
    }
    let case_insensitive = !match_case;
    if whole_cell {
        str_eq(content, query, case_insensitive)
    } else {
        find_from(content, query, 0, case_insensitive).is_some()
    }
}

/// The replaced content when `content` matches `query`, or `None` when it does not match
/// (`functional_spec.md §4.4`). Whole-cell ⇒ the replacement *is* the whole new content; else
/// **every** occurrence of `query` is replaced (non-overlapping, left to right), preserving the
/// surrounding original text (including its case under a case-insensitive match).
pub fn replace_in_cell(
    content: &str,
    query: &str,
    replacement: &str,
    match_case: bool,
    whole_cell: bool,
) -> Option<String> {
    if !cell_matches(content, query, match_case, whole_cell) {
        return None;
    }
    if whole_cell {
        return Some(replacement.to_string());
    }
    let case_insensitive = !match_case;
    let chars: Vec<char> = content.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < chars.len() {
        if matches_at(&chars, i, &needle, case_insensitive) {
            out.push_str(replacement);
            i += needle.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Case-(in)sensitive equality of two whole strings.
fn str_eq(a: &str, b: &str, case_insensitive: bool) -> bool {
    if !case_insensitive {
        return a == b;
    }
    let (mut ca, mut cb) = (a.chars(), b.chars());
    loop {
        match (ca.next(), cb.next()) {
            (Some(x), Some(y)) if chars_eq_ci(x, y) => continue,
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// The char index of the first occurrence of `needle` in `haystack` at or after `start`
/// (case-(in)sensitive), or `None`. Char-based (not byte-based) so a case-insensitive match never
/// splits a multi-byte char.
fn find_from(haystack: &str, needle: &str, start: usize, case_insensitive: bool) -> Option<usize> {
    let hay: Vec<char> = haystack.chars().collect();
    let ndl: Vec<char> = needle.chars().collect();
    if ndl.is_empty() || ndl.len() > hay.len() {
        return None;
    }
    (start..=hay.len() - ndl.len()).find(|&i| matches_at(&hay, i, &ndl, case_insensitive))
}

/// Whether `needle` sits at char position `pos` of `hay` (case-(in)sensitive, char-by-char).
fn matches_at(hay: &[char], pos: usize, needle: &[char], case_insensitive: bool) -> bool {
    if pos + needle.len() > hay.len() {
        return false;
    }
    needle.iter().enumerate().all(|(k, &n)| {
        let h = hay[pos + k];
        if case_insensitive {
            chars_eq_ci(h, n)
        } else {
            h == n
        }
    })
}

/// Case-insensitive single-char equality. ASCII fast path; a Unicode fallback folds both chars to
/// lowercase (handles the common single→single mappings; multi-char foldings like `ß`→`ss` are not
/// matched — a rare edge in spreadsheet content, accepted).
fn chars_eq_ci(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    if a.is_ascii() || b.is_ascii() {
        return a.eq_ignore_ascii_case(&b);
    }
    a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_never_matches() {
        assert!(!cell_matches("anything", "", false, false));
        assert!(!cell_matches("", "", true, true));
    }

    #[test]
    fn substring_case_insensitive_by_default() {
        assert!(cell_matches("Hello World", "world", false, false));
        assert!(cell_matches("Hello World", "WORLD", false, false));
        assert!(!cell_matches("Hello World", "world", true, false)); // case-sensitive
        assert!(cell_matches("Hello World", "World", true, false));
    }

    #[test]
    fn whole_cell_requires_full_equality() {
        assert!(cell_matches("Total", "total", false, true));
        assert!(!cell_matches("Total sum", "total", false, true)); // substring is not whole-cell
        assert!(cell_matches("Total sum", "total", false, false)); // but matches as substring
        assert!(!cell_matches("Total", "total", true, true)); // case-sensitive whole-cell
    }

    #[test]
    fn matches_formula_text() {
        // The caller passes formula TEXT for a formula cell (Excel "Look in: Formulas").
        assert!(cell_matches("=A1+A2", "a1", false, false));
        assert!(cell_matches("=SUM(A1:A2)", "SUM", true, false));
        assert!(!cell_matches("=A1+A2", "B1", false, false));
    }

    #[test]
    fn replace_whole_cell_swaps_entire_content() {
        assert_eq!(
            replace_in_cell("Total", "total", "Sum", false, true).as_deref(),
            Some("Sum")
        );
        assert_eq!(
            replace_in_cell("Total sum", "total", "x", false, true),
            None
        );
    }

    #[test]
    fn replace_substring_replaces_every_occurrence() {
        assert_eq!(
            replace_in_cell("a-a-a", "a", "b", true, false).as_deref(),
            Some("b-b-b")
        );
        assert_eq!(
            replace_in_cell("foofoo", "foo", "bar", true, false).as_deref(),
            Some("barbar")
        );
    }

    #[test]
    fn replace_case_insensitive_preserves_surrounding_case() {
        // The matched substring is replaced verbatim with `replacement`; the untouched text keeps
        // its original case.
        assert_eq!(
            replace_in_cell("Hello WORLD hello", "hello", "Hi", false, false).as_deref(),
            Some("Hi WORLD Hi")
        );
    }

    #[test]
    fn replace_returns_none_when_no_match() {
        assert_eq!(replace_in_cell("abc", "z", "y", false, false), None);
        assert_eq!(replace_in_cell("abc", "", "y", false, false), None);
    }

    #[test]
    fn replace_edits_formula_text() {
        assert_eq!(
            replace_in_cell("=A1+A1", "A1", "B2", true, false).as_deref(),
            Some("=B2+B2")
        );
    }

    #[test]
    fn unicode_substring_does_not_split_multibyte() {
        // café contains "é"; a case-insensitive search for "É" matches without byte slicing.
        assert!(cell_matches("café", "É", false, false));
        assert_eq!(
            replace_in_cell("café au lait", "É", "e", false, false).as_deref(),
            Some("cafe au lait")
        );
    }
}
