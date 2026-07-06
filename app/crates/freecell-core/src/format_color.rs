//! Number-format presentation helpers: the **date-format heuristic** and the
//! **format-colour index → RGB** table (`architecture.md §1.2`,
//! `components/style_render.md`).
//!
//! Both are pure and engine-free so they unit-test headless. The worker uses them when
//! building a [`PublishedCell`](crate::publication::PublishedCell): the heuristic
//! reclassifies a number-typed cell as a date when its format is date/time-like, and the
//! colour table maps IronCalc's `format_number(...).color` index onto the RGB the grid
//! draws (the pinned engine returns only the raw index and leaves RGB mapping to the
//! consumer — verified: it carries no palette itself).

use crate::color::Rgb;

/// Whether `fmt` is a date/time number format — used to reclassify a Number-typed cell as
/// [`CellKind::Date`](crate::publication::CellKind::Date) (the engine models dates as serial
/// numbers, so it has no distinct date cell type).
///
/// The heuristic (`architecture.md §1.2`): strip bracketed sections (`[Red]`, `[$-409]`,
/// `[h]`, …) and string literals (`"..."` and single `\`-escaped characters), then look for
/// any of the date/time field letters `y m d h s`. Pure number formats (`@`, `#`, `0`,
/// `#,##0.00`) contain none and stay Number.
pub fn is_date_format(fmt: &str) -> bool {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        match c {
            // Bracketed section: locale/colour/condition/elapsed-time markers — never a
            // date field to us. Skip to the closing `]`.
            '[' => {
                for b in chars.by_ref() {
                    if b == ']' {
                        break;
                    }
                }
            }
            // Quoted literal: everything up to the next unescaped `"` is literal text.
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            // Backslash escapes the next single character into a literal.
            '\\' => {
                chars.next();
            }
            // Date/time field letters (case-insensitive: `M`/`m` for month, etc.).
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => return true,
            _ => {}
        }
    }
    false
}

/// Named format colours in the pinned lexer's index order (`base/formatter/lexer.rs`):
/// 0 black, 1 white, 2 red, 3 green, 4 blue, 5 yellow, 6 magenta. `[Red]` — the GAPS #2
/// requirement — is index 2. RGB primaries approximate Excel's named format colours.
const NAMED_COLORS: [u32; 7] = [
    0x000000, // black
    0xFFFFFF, // white
    0xFF0000, // red
    0x00FF00, // green
    0x0000FF, // blue
    0xFFFF00, // yellow
    0xFF00FF, // magenta
];

/// Map an IronCalc number-format colour index (from `format_number(...).color`) to an RGB.
///
/// Named colours 0–6 cover every named format colour (`[Black]`…`[Magenta]`), including the
/// `[Red]` negatives this project targets. `[Color N]` for `N > 6` returns `None` (the cell
/// keeps the default text colour): the classic 56-entry palette has no engine-side RGB
/// reference to match, and `[Color N]` with a high index is vanishingly rare in real files
/// (recorded in `DECISIONS_TO_REVIEW.md`). Out-of-range indices → `None`.
pub fn format_color_rgb(index: i32) -> Option<Rgb> {
    if (0..=6).contains(&index) {
        Some(Rgb::from_hex(NAMED_COLORS[index as usize]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_formats_detected() {
        assert!(is_date_format("m/d/yyyy"));
        assert!(is_date_format("h:mm AM/PM"));
        assert!(is_date_format("yyyy\\-mm")); // escaped '-' is literal, y/m remain
        assert!(is_date_format("[$-409]d-mmm-yy")); // locale section stripped, d/m/y remain
        assert!(is_date_format("[h]:mm:ss")); // elapsed-time bracket stripped, m/s remain
    }

    #[test]
    fn non_date_formats_rejected() {
        assert!(!is_date_format("general"));
        assert!(!is_date_format("@"));
        assert!(!is_date_format("#,##0.00"));
        assert!(!is_date_format("0.00%"));
        assert!(!is_date_format("$#,##0.00"));
        // A colour bracket must not be read as a date field (the 'd' is inside `[Red]`).
        assert!(!is_date_format("[Red]0.00"));
        // A quoted literal must not contribute date letters.
        assert!(!is_date_format("\"months\"@"));
        assert!(!is_date_format("0\"days\""));
    }

    #[test]
    fn format_color_named_indices() {
        assert_eq!(format_color_rgb(0), Some(Rgb::from_hex(0x000000))); // black
        assert_eq!(format_color_rgb(2), Some(Rgb::from_hex(0xFF0000))); // red ([Red])
        assert_eq!(format_color_rgb(4), Some(Rgb::from_hex(0x0000FF))); // blue
        assert_eq!(format_color_rgb(6), Some(Rgb::from_hex(0xFF00FF))); // magenta
    }

    #[test]
    fn format_color_out_of_named_range_is_none() {
        assert_eq!(format_color_rgb(7), None); // [Color 7] — classic palette not carried
        assert_eq!(format_color_rgb(56), None);
        assert_eq!(format_color_rgb(-1), None);
        assert_eq!(format_color_rgb(1000), None);
    }
}
