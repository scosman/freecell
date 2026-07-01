//! Formula-editing helpers — the formula bar needs: (1) the cell's formula string,
//! (2) a tokenizer for syntax/reference highlighting, (3) reference extraction, and
//! (4) the function list for autocomplete.
//!
//! - Formula string — PRESENT: `UserModel::get_cell_content` (common.rs:466) returns the
//!   `=formula` for the formula bar (empty string / value for non-formula cells).
//! - Tokenizer — PRESENT: `expressions::lexer::Lexer` is a public struct in a `pub mod`
//!   (expressions/mod.rs:2). `Lexer::new(formula, LexerMode::A1, &Locale, &Language)`
//!   (lexer/mod.rs:94) + `next_token() -> TokenType` (lexer/mod.rs:185) +
//!   `get_position()` (lexer/mod.rs:144). `TokenType` (token.rs:219) is public with
//!   `Reference{..}`, `Range{..}`, `Ident`, `String`, `Number`, operators, `EOF` — enough
//!   to color operators/refs/functions/literals and to highlight precedent references.
//!   Locale/Language come from public `get_locale`/`get_language`/`get_default_*`.
//! - Reference extraction — PRESENT: `expressions::parser::Parser` (parser/mod.rs:229) is
//!   public; `Parser::parse(formula, &CellReferenceRC) -> Node` (parser/mod.rs:276), and
//!   `Node` (parser/mod.rs:110) is public with `ReferenceKind{row,column,absolute_*}` and
//!   `RangeKind{..}`, walkable to extract precedents. `new_parser_english(worksheets,
//!   defined_names, tables)` (parser/mod.rs:219) is a public convenience constructor.
//! - Function list — WORKAROUND: the `Function` enum (functions/mod.rs:32) has exactly
//!   **345 variants** (matching SP3), BUT `mod functions;` is PRIVATE (lib.rs:45, not
//!   `pub mod`), so the enum is NOT reachable from an external crate, and IronCalc has no
//!   `strum`/iterator over it (its own test parses the source file to enumerate,
//!   functions/mod.rs:2022). FreeCell must maintain its OWN function-name list for
//!   autocomplete (it can validate a name by parsing: an unknown function parses to
//!   `Node::InvalidFunctionKind`, parser/mod.rs:178).

use ironcalc_base::expressions::lexer::{Lexer, LexerMode};
use ironcalc_base::expressions::parser::{new_parser_english, Node};
use ironcalc_base::expressions::token::TokenType;
use ironcalc_base::expressions::types::CellReferenceRC;
use ironcalc_base::language::get_language;
use ironcalc_base::locale::get_locale;
use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

/// Tokenizes a formula with the public `Lexer` and counts reference/range tokens — the
/// exact operation formula-bar reference highlighting needs.
fn tokenize_reference_count(formula: &str) -> (usize, usize) {
    let locale = get_locale("en").unwrap();
    let language = get_language("en").unwrap();
    let mut lexer = Lexer::new(formula, LexerMode::A1, locale, language);
    let mut total = 0usize;
    let mut refs = 0usize;
    loop {
        match lexer.next_token() {
            TokenType::EOF => break,
            TokenType::Illegal(_) => break,
            TokenType::Reference { .. } | TokenType::Range { .. } => {
                refs += 1;
                total += 1;
            }
            _ => total += 1,
        }
    }
    (total, refs)
}

/// Parses a formula to the public `Node` AST and counts the reference/range leaves by
/// walking the tree — the parser path for precedent extraction.
fn parse_reference_count(formula: &str) -> usize {
    // `Parser::parse` takes the formula WITHOUT the leading '='.
    let mut parser = new_parser_english(vec!["Sheet1".to_string()], vec![], Default::default());
    let ctx = CellReferenceRC {
        sheet: "Sheet1".to_string(),
        row: 1,
        column: 1,
    };
    let node = parser.parse(formula, &ctx);
    count_refs(&node)
}

fn count_refs(node: &Node) -> usize {
    match node {
        Node::ReferenceKind { .. } | Node::RangeKind { .. } => 1,
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. }
        | Node::OpRangeKind { left, right } => count_refs(left) + count_refs(right),
        Node::UnaryKind { right, .. } => count_refs(right),
        Node::FunctionKind { args, .. } | Node::InvalidFunctionKind { args, .. } => {
            args.iter().map(count_refs).sum()
        }
        _ => 0,
    }
}

pub fn probe() -> FormulaHelpersObservation {
    // (1) formula-bar string
    let mut model = UserModel::new_empty("Book", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "=A2+B3*2").unwrap();
    let content = model.get_cell_content(0, 1, 1).unwrap();

    // (2)/(3) tokenizer + parser reference counts on the same formula ("A2" + "B3" = 2).
    let (token_total, token_refs) = tokenize_reference_count("A2+B3*2");
    let parsed_refs = parse_reference_count("A2+B3*2");

    FormulaHelpersObservation {
        content,
        token_total,
        token_refs,
        parsed_refs,
        function_enum_variants: 345,
        function_enum_public: false,
    }
}

#[derive(Debug, Clone)]
pub struct FormulaHelpersObservation {
    pub content: String,
    pub token_total: usize,
    pub token_refs: usize,
    pub parsed_refs: usize,
    pub function_enum_variants: usize,
    pub function_enum_public: bool,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "Formula bar: get a cell's formula string",
            Status::Present,
            format!(
                "UserModel::get_cell_content -> {:?}. (common.rs:466)",
                o.content
            ),
        ),
        AuditRow::new(
            "Formula bar: tokenizer for syntax/reference highlighting",
            Status::Present,
            format!(
                "public Lexer::new + next_token -> TokenType (Reference/Range/Ident/...); \
                 tokenized 'A2+B3*2' = {} tokens, {} references. (lexer/mod.rs:94/185, \
                 token.rs:219)",
                o.token_total, o.token_refs
            ),
        ),
        AuditRow::new(
            "Formula bar: parse to AST + extract references (precedents)",
            Status::Present,
            format!(
                "public Parser::parse -> Node (ReferenceKind/RangeKind walkable); parsed \
                 'A2+B3*2' -> {} references. (parser/mod.rs:110/276)",
                o.parsed_refs
            ),
        ),
        AuditRow::new(
            "Formula bar: enumerate the function list (for autocomplete)",
            Status::Workaround,
            format!(
                "Function enum has {} variants (== SP3's 345) but `mod functions` is \
                 PRIVATE (lib.rs:45) so it's not externally reachable, and there is no \
                 iterator/strum. FreeCell maintains its own function-name list; validate \
                 a name by parsing (unknown -> Node::InvalidFunctionKind). \
                 (functions/mod.rs:32, public={})",
                o.function_enum_variants, o.function_enum_public
            ),
        ),
    ]
}
