//! The fill-colour palette — the 10 Office default theme colours (`ui_design.md §3.1`),
//! for consistency with existing spreadsheets. The action-row Fill popover renders these
//! as a swatch grid; the "No fill" and "Custom…" entries are UI affordances, not palette
//! members. IronCalc stores an arbitrary `#RRGGBB`, so a custom pick applies like a swatch.

use crate::color::Rgb;
use crate::refs::{CellRange, RefToken};

/// One named palette colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swatch {
    pub name: &'static str,
    pub rgb: Rgb,
}

/// The 10 Office theme colours in canonical theme order (Background/Text pairs, then the
/// six accents), matching Excel's own fill dropdown. Hexes from `ui_design.md §3.1`.
pub const FILL_PALETTE: [Swatch; 10] = [
    Swatch {
        name: "Background 1",
        rgb: Rgb::from_hex(0xFFFFFF),
    },
    Swatch {
        name: "Text 1",
        rgb: Rgb::from_hex(0x000000),
    },
    Swatch {
        name: "Background 2",
        rgb: Rgb::from_hex(0xE7E6E6),
    },
    Swatch {
        name: "Text 2",
        rgb: Rgb::from_hex(0x44546A),
    },
    Swatch {
        name: "Accent 1",
        rgb: Rgb::from_hex(0x4472C4),
    },
    Swatch {
        name: "Accent 2",
        rgb: Rgb::from_hex(0xED7D31),
    },
    Swatch {
        name: "Accent 3",
        rgb: Rgb::from_hex(0xA5A5A5),
    },
    Swatch {
        name: "Accent 4",
        rgb: Rgb::from_hex(0xFFC000),
    },
    Swatch {
        name: "Accent 5",
        rgb: Rgb::from_hex(0x5B9BD5),
    },
    Swatch {
        name: "Accent 6",
        rgb: Rgb::from_hex(0x70AD47),
    },
];

/// One reference-highlight color, with a light- and dark-theme variant (theme-aware, DPM.3).
/// The consumer picks `light` or `dark` from the active window appearance at draw time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefColor {
    /// The color used on a light theme (chosen for contrast against the white cell background).
    pub light: Rgb,
    /// The color used on a dark theme (chosen for contrast against the dark cell background).
    pub dark: Rgb,
}

/// The fixed 7-color reference-highlight cycle (DPM.3). Distinct + legible as a grid fill +
/// border (and, for the future in-editor styling control, as editor text), in both themes.
/// Curated from Excel's colored-refs feel; each slot's `light`/`dark` hex is tuned for contrast
/// against the light and dark cell backgrounds respectively. Beyond 7 distinct references the
/// cycle recycles (see [`ref_color`] / [`assign_ref_colors`]).
pub const REF_HIGHLIGHT_PALETTE: [RefColor; 7] = [
    // Blue
    RefColor {
        light: Rgb::from_hex(0x2563EB),
        dark: Rgb::from_hex(0x93B4FF),
    },
    // Green
    RefColor {
        light: Rgb::from_hex(0x15803D),
        dark: Rgb::from_hex(0x86EFAC),
    },
    // Purple
    RefColor {
        light: Rgb::from_hex(0x7C3AED),
        dark: Rgb::from_hex(0xC4B5FD),
    },
    // Magenta
    RefColor {
        light: Rgb::from_hex(0xC026D3),
        dark: Rgb::from_hex(0xF0ABFC),
    },
    // Orange
    RefColor {
        light: Rgb::from_hex(0xEA580C),
        dark: Rgb::from_hex(0xFDBA74),
    },
    // Teal
    RefColor {
        light: Rgb::from_hex(0x0D9488),
        dark: Rgb::from_hex(0x5EEAD4),
    },
    // Red
    RefColor {
        light: Rgb::from_hex(0xDC2626),
        dark: Rgb::from_hex(0xFCA5A5),
    },
];

/// The palette slot for `index`, recycling past the palette length (`index % 7`).
pub fn ref_color(index: usize) -> RefColor {
    REF_HIGHLIGHT_PALETTE[index % REF_HIGHLIGHT_PALETTE.len()]
}

/// Assign a palette **slot** to each token by distinct resolved reference, first-appearance
/// order (DPM.3): two tokens with the same `(sheet, target)` share a slot; a new distinct ref
/// takes the next slot; slot = distinct-index `% 7`. Returns one slot per input token, parallel
/// to `tokens`.
///
/// First-appearance order makes colors **stable** as the user types a later ref (never recolors
/// earlier ones); removing an earlier ref may shift later slots by one (cosmetic, accepted —
/// `functional_spec.md §5`). Pure — unit-tested headless.
pub fn assign_ref_colors(tokens: &[RefToken]) -> Vec<u8> {
    let mut distinct: Vec<(Option<String>, CellRange)> = Vec::new();
    let mut slots = Vec::with_capacity(tokens.len());
    for token in tokens {
        let key = (token.sheet.clone(), token.target);
        let index = match distinct.iter().position(|k| *k == key) {
            Some(i) => i,
            None => {
                distinct.push(key);
                distinct.len() - 1
            }
        };
        slots.push((index % REF_HIGHLIGHT_PALETTE.len()) as u8);
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::CellRef;

    #[test]
    fn palette_has_ten_office_swatches() {
        assert_eq!(FILL_PALETTE.len(), 10);
        // Names are unique.
        for (i, a) in FILL_PALETTE.iter().enumerate() {
            for b in &FILL_PALETTE[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate swatch name {}", a.name);
            }
        }
    }

    #[test]
    fn palette_hexes_match_spec() {
        // Spot-check the load-bearing anchors against ui_design.md §3.1.
        assert_eq!(FILL_PALETTE[0].rgb, Rgb::from_hex(0xFFFFFF)); // Background 1
        assert_eq!(FILL_PALETTE[1].rgb, Rgb::from_hex(0x000000)); // Text 1
        assert_eq!(FILL_PALETTE[4].rgb, Rgb::from_hex(0x4472C4)); // Accent 1
        assert_eq!(FILL_PALETTE[9].rgb, Rgb::from_hex(0x70AD47)); // Accent 6
        assert_eq!(
            FILL_PALETTE[3].rgb,
            Rgb::new(0x44, 0x54, 0x6A),
            "Text 2 channels"
        );
    }

    /// Build a `RefToken` for color-assignment tests (only `sheet` + `target` are load-bearing).
    fn tok(sheet: Option<&str>, range: CellRange) -> RefToken {
        RefToken {
            span: 0..0,
            target: range,
            sheet: sheet.map(str::to_string),
            same_sheet: true,
        }
    }

    fn cell(row: u32, col: u32) -> CellRange {
        CellRange::single(CellRef::new(row, col))
    }

    #[test]
    fn ref_highlight_palette_is_seven_distinct_theme_pairs() {
        assert_eq!(REF_HIGHLIGHT_PALETTE.len(), 7);
        for (i, c) in REF_HIGHLIGHT_PALETTE.iter().enumerate() {
            assert_ne!(c.light, c.dark, "slot {i} light and dark must differ");
        }
        // All light variants distinct, and all dark variants distinct.
        for (i, a) in REF_HIGHLIGHT_PALETTE.iter().enumerate() {
            for b in &REF_HIGHLIGHT_PALETTE[i + 1..] {
                assert_ne!(a.light, b.light, "duplicate light color at slot {i}");
                assert_ne!(a.dark, b.dark, "duplicate dark color at slot {i}");
            }
        }
    }

    #[test]
    fn ref_color_wraps_past_seven() {
        assert_eq!(ref_color(0), REF_HIGHLIGHT_PALETTE[0]);
        assert_eq!(ref_color(6), REF_HIGHLIGHT_PALETTE[6]);
        // The 8th distinct reference recycles to slot 0, the 9th to slot 1, etc.
        assert_eq!(ref_color(7), ref_color(0));
        assert_eq!(ref_color(8), ref_color(1));
        assert_eq!(ref_color(13), ref_color(6));
    }

    #[test]
    fn assign_ref_colors_shares_repeats_and_steps_distinct() {
        // Two occurrences of the same reference share one slot.
        let repeats = [tok(None, cell(0, 0)), tok(None, cell(0, 0))];
        assert_eq!(assign_ref_colors(&repeats), vec![0, 0]);

        // Distinct references step to the next slot, first-appearance order.
        let distinct = [tok(None, cell(0, 0)), tok(None, cell(1, 1))];
        assert_eq!(assign_ref_colors(&distinct), vec![0, 1]);
    }

    #[test]
    fn assign_ref_colors_recycles_past_seven_distinct() {
        // Eight distinct references: the 8th recycles to slot 0.
        let tokens: Vec<RefToken> = (0..8).map(|i| tok(None, cell(i, 0))).collect();
        assert_eq!(
            assign_ref_colors(&tokens),
            vec![0, 1, 2, 3, 4, 5, 6, 0],
            "8th distinct ref recycles to slot 0"
        );
    }

    #[test]
    fn assign_ref_colors_is_first_appearance_stable() {
        // Appending a later reference never changes earlier slots.
        let two = [tok(None, cell(0, 0)), tok(None, cell(1, 1))];
        let three = [
            tok(None, cell(0, 0)),
            tok(None, cell(1, 1)),
            tok(None, cell(2, 2)),
        ];
        assert_eq!(assign_ref_colors(&two), vec![0, 1]);
        assert_eq!(assign_ref_colors(&three), vec![0, 1, 2]);
    }

    #[test]
    fn assign_ref_colors_keys_on_sheet() {
        // A same-target ref on another sheet is a distinct key → its own slot.
        let tokens = [tok(Some("Sheet2"), cell(0, 0)), tok(None, cell(0, 0))];
        assert_eq!(assign_ref_colors(&tokens), vec![0, 1]);
    }
}
