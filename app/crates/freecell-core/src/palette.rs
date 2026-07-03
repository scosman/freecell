//! The fill-colour palette — the 10 Office default theme colours (`ui_design.md §3.1`),
//! for consistency with existing spreadsheets. The action-row Fill popover renders these
//! as a swatch grid; the "No fill" and "Custom…" entries are UI affordances, not palette
//! members. IronCalc stores an arbitrary `#RRGGBB`, so a custom pick applies like a swatch.

use crate::color::Rgb;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
