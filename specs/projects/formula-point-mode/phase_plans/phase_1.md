---
status: complete
---

# Phase 1: Tokenization seam + pure foundation

## Overview

Front-loads the **pure, no-pixel, no-gpui** foundation for formula point-mode + range
highlighting (architecture.md §1, §2). Adds the gpui-free data types + predicates in
`freecell-core` and the single engine tokenization free function in `freecell-engine`. No
`ChromeView`/grid wiring, no rendering — those are Phases 2–3. Everything here is unit-testable
headless.

Field bindings against the pinned fork (`scosman/ironcalc` @ `freecell-fixes`) were confirmed
directly against source (`base/src/expressions/token.rs`, `.../types.rs`, `.../lexer/mod.rs`):

- `TokenType::Reference { sheet: Option<String>, row: i32, column: i32, absolute_column: bool,
  absolute_row: bool }`
- `TokenType::Range { sheet: Option<String>, left: ParsedReference, right: ParsedReference }`
- `ParsedReference { column: i32, row: i32, absolute_column: bool, absolute_row: bool }` (no
  `sheet` field — the qualifier lives on the `Range` variant).
- `Lexer::get_position() -> i32` returns a **char index** (the lexer collects `formula.chars()`),
  and `next_token()` calls `consume_whitespace()` first, so `get_position()` after a `next_token()`
  points just past the token, with no trailing whitespace. `row`/`column` are 1-based absolute A1
  coordinates in `LexerMode::A1`.
- Incomplete refs lex to `Illegal` (`=A1:`, `=Sheet2!`) or `Ident` (bare `=A`); a string is a
  `String` token — none emit a `Reference`/`Range`, satisfying "partial refs never highlight".

## Steps

1. **`freecell-core/src/refs.rs` — add `RefToken`.** New public struct (Debug/Clone/PartialEq/Eq):
   `span: std::ops::Range<usize>` (byte span in the full edit text, leading `=` included),
   `target: CellRange` (0-based, normalized), `sheet: Option<String>` (qualifier, part of the color
   key), `same_sheet: bool` (target resolves to the visible sheet). Doc comment per architecture.md
   §2.1.

2. **`freecell-core/src/functions.rs` — add `is_reference_ready`.** `pub fn is_reference_ready(text:
   &str, caret: usize) -> bool`, beside `fn_edit_context`. Guards: `text.starts_with('=')`, `caret <=
   len`, char boundary, `!in_string_at`. Skip the run of ASCII spaces immediately before `caret`;
   if none remain (`i == 0`), false; else return `is_function_position_prev(bytes[i-1])` (the char is
   guaranteed non-whitespace by the skip, so this is true only for the operator/opener/comma/colon/`=`
   set). Reuses the existing private `is_function_position_prev` + `in_string_at`.

3. **`freecell-core/src/palette.rs` — add `RefColor`, `REF_HIGHLIGHT_PALETTE`, `ref_color`,
   `assign_ref_colors`.**
   - `RefColor { light: Rgb, dark: Rgb }` (Copy).
   - `pub const REF_HIGHLIGHT_PALETTE: [RefColor; 7]` — 7 authored theme-aware pairs (light for the
     white cell bg, dark for the dark cell bg), all-distinct.
   - `pub fn ref_color(index: usize) -> RefColor` = `REF_HIGHLIGHT_PALETTE[index % 7]`.
   - `pub fn assign_ref_colors(tokens: &[RefToken]) -> Vec<u8>` — walk left-to-right, key an
     insertion-ordered `Vec<(Option<String>, CellRange)>` on `(sheet, target)`; slot = distinct-index
     `% 7`; one slot per input token (parallel).

4. **`freecell-core/src/lib.rs` — re-exports.** `pub use refs::{… RefToken …}`;
   `pub use palette::assign_ref_colors;`; `pub use functions::is_reference_ready;` (match the
   `freecell_core::{RefToken, assign_ref_colors, is_reference_ready}` surface the component doc uses;
   `RefColor`/`REF_HIGHLIGHT_PALETTE`/`ref_color` stay reachable via `palette::`).

5. **`freecell-engine/src/formula_refs.rs` — new module.** `pub fn lex_formula_refs(edit_text: &str,
   active_sheet_name: &str) -> Vec<freecell_core::RefToken>`.
   - Return empty unless `edit_text.starts_with('=')`. Strip the leading `=` (1 byte/char) → `body`.
   - `let (Ok(locale), Ok(language)) = (get_locale("en"), get_language("en")) else { return vec![] };`
     `Lexer::new(body, LexerMode::A1, locale, language)`.
   - Build a char-index → byte-offset table from `body` for span mapping.
   - Loop: capture `before = get_position()`, `tok = next_token()`, `after = get_position()`; break on
     `EOF | Illegal(_)`. For `Reference`/`Range`, compute the token's char span `[start, after)` where
     `start = before + (leading whitespace count in body chars [before, after))` (defensive — the
     lexer already skips leading whitespace, so this is usually `before`); map to byte span via the
     table; the final `RefToken.span` is `(byte_start + 1)..(byte_end + 1)` (the stripped `=`).
   - `Reference` → `CellRange::single(cell_from(row, column))`, `sheet` from the variant.
     `Range` → `CellRange::new(cell_from(left.row, left.column), cell_from(right.row, right.column))`
     (normalized; drag-direction independent), `sheet` from the variant.
   - `cell_from(row, col)` = `CellRef::new((row.max(1) - 1) as u32, (column.max(1) - 1) as u32)`
     (1-based → 0-based, defensive clamp).
   - `same_sheet = sheet.is_none() || sheet.eq_ignore_ascii_case(active_sheet_name)`.

6. **`freecell-engine/src/lib.rs` — re-export.** `pub mod formula_refs;` +
   `pub use formula_refs::lex_formula_refs;`.

## Tests

**`freecell-core` (headless):**
- `is_reference_ready` truth table (functions.rs): `=|`→true; `=A1+|`→true; `=SUM(|`→true;
  `=A1:|`→true; `= |` (trailing space)→true; `=A1|`→false; `=SUM(A1)|`→false; `=SU|`→false;
  `=12|`→false; `="A1+|`→false; `A1` (no `=`)→false; `=`|caret 0→false.
- `assign_ref_colors` (palette.rs): repeats share (`A1,A1`→[0,0]); distinct step (`A1,B2`→[0,1]);
  >7 distinct recycle (8th distinct → slot 0); first-appearance stability (appending a later ref
  never changes earlier slots); `Sheet2!A1` vs `A1` distinct slots.
- palette invariants (palette.rs): `REF_HIGHLIGHT_PALETTE.len() == 7`; `ref_color(i)` wraps at 7;
  `light != dark` per slot; all light distinct + all dark distinct.

**`freecell-engine` (headless, real IronCalc lexer):**
- `=A1+B2` → 2 tokens; spans are `1..3`/`4..6` (i.e. `edit_text[span] == "A1"/"B2"`); targets
  `A1`=`(0,0)` / `B2`=`(1,1)`; both `same_sheet` on "Sheet1".
- `=SUM(C3:E7)` → 1 `Range` token; target normalized `C3:E7` = `(2,2)..(6,4)`; span == `"C3:E7"`.
- `=Sheet2!A1` on "Sheet1" → `same_sheet=false`, `sheet=Some("Sheet2")`.
- `=Sheet1!A1` on "Sheet1" → `same_sheet=true` (self-qualified, case-insensitive).
- `=A1:` / `=Sheet2!` / `="A1"` / `=` / `hello` → no tokens (empty vec).
- drag-direction independence: `=B2:A1` normalizes to target `(0,0)..(1,1)`.

## Implementation notes

- **Lexer deviation from architecture.md §1.2 (verified empirically).** The doc assumed an
  unterminated range `=A1:` lexes to a single `Illegal` token (so nothing is emitted). The pinned
  fork actually emits a bare `Reference(A1)` followed by a standalone range colon, so `A1` would
  leak through — contradicting the locked `functional_spec.md §3` ("`=A1:` gets no highlight").
  `lex_formula_refs` therefore suppresses a `Reference` when a `:` immediately follows it (peeking
  past whitespace): a *complete* range is always one `Range` token (never `Reference` + colon,
  confirmed by `top_level_complete_range_lexes_as_one_range` and `lexes_range_token_normalized`),
  so a trailing colon unambiguously marks a range still being typed. Behaviour matches the locked
  spec; only the mechanism differs from the doc's assumption. (`=Sheet2!` does lex to `Illegal` as
  the doc assumed.)

## Checks (run cargo from `app/`)

- `cargo build -p freecell-core -p freecell-engine`
- `cargo test -p freecell-core --lib -p freecell-engine --lib`
- `cargo fmt --all --check`
