//! Formula reference tokenization — the one engine surface the formula point-mode + range
//! highlighting feature needs (`../../../specs/projects/formula-point-mode/architecture.md §1`).
//!
//! [`lex_formula_refs`] tokenizes an in-progress formula with the **real** IronCalc `Lexer` and
//! returns its reference/range tokens as gpui-free [`freecell_core::RefToken`]s. It is pure,
//! synchronous, and `Model`-free (the lexer needs only the process-wide locale/language statics),
//! so it is safe to call on the render/main thread on every keystroke — no worker round-trip.
//! `freecell-engine` is the only crate allowed to touch IronCalc types (`architecture.md §2`);
//! no IronCalc type crosses this function's boundary.

use freecell_core::{CellRange, CellRef, RefToken};
use ironcalc_base::expressions::lexer::{Lexer, LexerMode};
use ironcalc_base::expressions::token::TokenType;
use ironcalc_base::language::get_language;
use ironcalc_base::locale::get_locale;

/// Tokenize `edit_text` (the full pending edit, **including** the leading `=`) and return its
/// reference/range tokens, with byte spans in `edit_text` coordinates and 0-based resolved
/// targets. Non-`=` text, or text with no complete references, returns an empty vec.
/// `active_sheet_name` is the visible sheet's name, used to set each token's `same_sheet` flag
/// (`architecture.md §1.2` step 4 / Q4).
///
/// Partial or invalid references never emit (`functional_spec.md §3`): an incomplete sheet
/// qualifier (`=Sheet2!`) lexes as `Illegal` (terminating the scan), a bare column (`=A`) as
/// `Ident`, and a string literal as `String` — none are `Reference`/`Range`. An unterminated
/// range (`=A1:`) does lex its first endpoint as a bare `Reference`, but it is suppressed here
/// because a range colon immediately follows it (a *complete* range is a single `Range` token).
pub fn lex_formula_refs(edit_text: &str, active_sheet_name: &str) -> Vec<RefToken> {
    // Only formulas tokenize; everything else has no references.
    let Some(body) = edit_text.strip_prefix('=') else {
        return Vec::new();
    };
    // The lexer needs the process-wide locale/language statics. "en" is always registered, but
    // degrade to "no highlights" rather than panic on the render thread if that ever changes.
    let (Ok(locale), Ok(language)) = (get_locale("en"), get_language("en")) else {
        return Vec::new();
    };

    // `Lexer::get_position()` is a CHAR index into `body` (the lexer collects `body.chars()`), so
    // map char indices back to byte offsets for the token span. The stripped leading `=` is one
    // byte and one char, so a span shifts by `+1` from body coordinates into `edit_text`.
    let body_chars: Vec<(usize, char)> = body.char_indices().collect();
    let char_count = body_chars.len();
    let byte_of = |char_idx: usize| -> usize {
        body_chars
            .get(char_idx)
            .map(|(byte, _)| *byte)
            .unwrap_or(body.len())
    };

    let mut lexer = Lexer::new(body, LexerMode::A1, locale, language);
    let mut tokens = Vec::new();
    loop {
        let before = lexer.get_position().max(0) as usize;
        let token = lexer.next_token();
        let after = (lexer.get_position().max(0) as usize).min(char_count);

        let (target, sheet, is_range) = match token {
            // End of input, or the first lexer error — stop contributing (a partial trailing ref
            // such as `=Sheet2!` lexes as `Illegal`, so it is never highlighted).
            TokenType::EOF | TokenType::Illegal(_) => break,
            TokenType::Reference {
                sheet, row, column, ..
            } => (CellRange::single(cell_from(row, column)), sheet, false),
            TokenType::Range { sheet, left, right } => (
                CellRange::new(
                    cell_from(left.row, left.column),
                    cell_from(right.row, right.column),
                ),
                sheet,
                true,
            ),
            // Idents, numbers, strings, operators, parens — not references.
            _ => continue,
        };

        // An unterminated range (`=A1:`, `=A1+B2:`) does not lex to a single `Range` token — the
        // lexer emits a bare `Reference` for the first endpoint followed by a standalone range
        // colon. A *complete* range is always one `Range` token (never `Reference` + colon), so a
        // `Reference` immediately followed by a `:` is a range still being typed and must not
        // highlight (`functional_spec.md §3`). Peek past any whitespace for that colon.
        if !is_range {
            let mut peek = after;
            while peek < char_count && body_chars[peek].1.is_whitespace() {
                peek += 1;
            }
            if peek < char_count && body_chars[peek].1 == ':' {
                continue;
            }
        }

        // The lexer already skips leading whitespace before a token, but defensively advance past
        // any that landed inside [before, after) so the span brackets the token text exactly.
        let mut start = before.min(char_count);
        while start < after && body_chars[start].1.is_whitespace() {
            start += 1;
        }
        let span = (byte_of(start) + 1)..(byte_of(after) + 1);

        let same_sheet = match &sheet {
            None => true,
            Some(name) => name.eq_ignore_ascii_case(active_sheet_name),
        };
        tokens.push(RefToken {
            span,
            target,
            sheet,
            same_sheet,
        });
    }
    tokens
}

/// Convert a 1-based IronCalc A1 coordinate to a 0-based [`CellRef`]. In `LexerMode::A1` these are
/// always absolute 1-based coordinates; the `max(1)` is a defensive clamp that keeps the
/// subtraction from underflowing on an unexpected non-positive value.
fn cell_from(row: i32, column: i32) -> CellRef {
    CellRef::new((row.max(1) - 1) as u32, (column.max(1) - 1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_two_simple_refs_with_spans_and_targets() {
        let text = "=A1+B2";
        let toks = lex_formula_refs(text, "Sheet1");
        assert_eq!(toks.len(), 2);
        // Spans are in `edit_text` coordinates (the leading `=` is byte 0).
        assert_eq!(toks[0].span, 1..3);
        assert_eq!(toks[1].span, 4..6);
        assert_eq!(&text[toks[0].span.clone()], "A1");
        assert_eq!(&text[toks[1].span.clone()], "B2");
        assert_eq!(toks[0].target, CellRange::from_a1("A1").unwrap());
        assert_eq!(toks[1].target, CellRange::from_a1("B2").unwrap());
        assert!(toks[0].same_sheet && toks[1].same_sheet);
        assert!(toks[0].sheet.is_none());
    }

    #[test]
    fn lexes_range_token_normalized() {
        let text = "=SUM(C3:E7)";
        let toks = lex_formula_refs(text, "Sheet1");
        assert_eq!(
            toks.len(),
            1,
            "one Range token; SUM/parens are not references"
        );
        assert_eq!(&text[toks[0].span.clone()], "C3:E7");
        assert_eq!(toks[0].target, CellRange::from_a1("C3:E7").unwrap());
    }

    #[test]
    fn range_target_is_drag_direction_independent() {
        let toks = lex_formula_refs("=B2:A1", "Sheet1");
        assert_eq!(toks.len(), 1);
        assert_eq!(
            toks[0].target,
            CellRange::from_a1("A1:B2").unwrap(),
            "endpoints normalize to top-left..bottom-right"
        );
    }

    #[test]
    fn top_level_complete_range_lexes_as_one_range() {
        // Guards the colon-suppression: a *complete* range is a single Range token, not a bare
        // Reference-followed-by-colon, so it survives.
        let text = "=A1:B2";
        let toks = lex_formula_refs(text, "Sheet1");
        assert_eq!(toks.len(), 1);
        assert_eq!(&text[toks[0].span.clone()], "A1:B2");
        assert_eq!(toks[0].target, CellRange::from_a1("A1:B2").unwrap());
    }

    #[test]
    fn cross_sheet_ref_is_not_same_sheet() {
        let toks = lex_formula_refs("=Sheet2!A1", "Sheet1");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].sheet.as_deref(), Some("Sheet2"));
        assert!(
            !toks[0].same_sheet,
            "cross-sheet ref draws no grid highlight"
        );
        // The color map still resolves its target.
        assert_eq!(toks[0].target, CellRange::from_a1("A1").unwrap());
    }

    #[test]
    fn self_qualified_ref_is_same_sheet_case_insensitive() {
        let toks = lex_formula_refs("=sheet1!A1", "Sheet1");
        assert_eq!(toks.len(), 1);
        assert!(
            toks[0].same_sheet,
            "a qualifier naming the visible sheet (any case) is same-sheet"
        );
    }

    #[test]
    fn partial_and_non_formula_yield_no_tokens() {
        // Partial range, incomplete qualifier, string literal, bare `=`, a non-formula, empty.
        for text in ["=A1:", "=Sheet2!", "=\"A1\"", "=", "=A", "hello", ""] {
            assert!(
                lex_formula_refs(text, "Sheet1").is_empty(),
                "expected no reference tokens for {text:?}"
            );
        }
    }

    #[test]
    fn refs_before_a_partial_tail_still_lex() {
        // A complete ref preceding an unfinished one still highlights (the scan collects it before
        // hitting the trailing `Illegal`).
        let toks = lex_formula_refs("=A1+B2:", "Sheet1");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].target, CellRange::from_a1("A1").unwrap());
    }
}

