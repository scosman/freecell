//! GPUI-free conversion of a [`datagen::CellData`] into render-ready primitives.
//!
//! Both rendering shells (`raw-gpui/`, `gpui-component/`) convert cells through this
//! module, so their look — text, fill, borders, bold/italic, alignment — is identical
//! and the visual comparison is apples-to-apples. Colours are plain `0xRRGGBB` `u32`s
//! that map trivially onto `gpui::rgb(...)`; nothing here depends on gpui.

use datagen::{CellData, CellValue, HAlign, Rgb};

/// The look of the minimal spreadsheet (functional_spec §6.E / §8): white background,
/// grey gridlines, dark text.
pub const BG_WHITE: u32 = 0xFFFFFF;
pub const GRIDLINE_GREY: u32 = 0xD0D0D0;
pub const TEXT_DARK: u32 = 0x1A1A1A;
/// Header strip fill (a light grey, distinct from the white cell body).
pub const HEADER_BG: u32 = 0xF2F2F2;
pub const HEADER_TEXT: u32 = 0x555555;

/// Horizontal alignment for a rendered cell (a gpui-free mirror of [`HAlign`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl From<HAlign> for Align {
    fn from(a: HAlign) -> Self {
        match a {
            HAlign::Left => Align::Left,
            HAlign::Center => Align::Center,
            HAlign::Right => Align::Right,
        }
    }
}

/// A fully render-ready cell: display text plus resolved colours and font attributes.
/// The shells position it (via [`crate::layout::Axis`]) and draw exactly these values.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCell {
    /// The text to draw (numbers formatted; empty cells render as `""`).
    pub text: String,
    /// The cell fill colour as `0xRRGGBB` — the highlight if present, else white.
    pub fill: u32,
    /// The text colour as `0xRRGGBB`.
    pub text_color: u32,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// Whether this cell carries a highlight fill (distinct from "fill == white").
    pub highlighted: bool,
}

/// Packs an [`Rgb`] into a `0xRRGGBB` `u32` (the shape `gpui::rgb` expects).
pub fn rgb_hex(c: Rgb) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

/// Formats a [`CellValue`] for display. Numbers drop a trailing `.0` and show at most
/// two decimals; text is passed through; empty is the empty string.
pub fn format_value(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Text(t) => t.clone(),
        CellValue::Number(n) => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                // Trim to two decimals, then strip trailing zeros for a clean look.
                let s = format!("{n:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    }
}

impl RenderCell {
    /// Builds a [`RenderCell`] from a provider [`CellData`].
    pub fn from_cell(cell: &CellData) -> Self {
        let highlighted = cell.format.highlight.is_some();
        let fill = cell.format.highlight.map(rgb_hex).unwrap_or(BG_WHITE);
        Self {
            text: format_value(&cell.value),
            fill,
            text_color: TEXT_DARK,
            bold: cell.format.bold,
            italic: cell.format.italic,
            align: cell.format.h_align.into(),
            highlighted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datagen::{CellFormat, CellSource, SyntheticSheet};

    #[test]
    fn rgb_hex_packs_channels() {
        assert_eq!(rgb_hex(Rgb::new(0x12, 0x34, 0x56)), 0x123456);
        assert_eq!(rgb_hex(Rgb::new(0xFF, 0x00, 0xFF)), 0xFF00FF);
    }

    #[test]
    fn formats_numbers_text_and_empty_distinctly() {
        assert_eq!(format_value(&CellValue::Empty), "");
        assert_eq!(format_value(&CellValue::Text("hi".into())), "hi");
        assert_eq!(format_value(&CellValue::Number(42.0)), "42");
        assert_eq!(format_value(&CellValue::Number(3.5)), "3.5");
        assert_eq!(format_value(&CellValue::Number(3.50)), "3.5");
        assert_eq!(format_value(&CellValue::Number(1234.0)), "1234");
    }

    #[test]
    fn highlight_maps_to_fill_and_flag() {
        let plain = CellData {
            value: CellValue::Text("x".into()),
            format: CellFormat::default(),
        };
        let rc = RenderCell::from_cell(&plain);
        assert_eq!(rc.fill, BG_WHITE);
        assert!(!rc.highlighted);

        let hot = CellData {
            value: CellValue::Number(1.0),
            format: CellFormat {
                highlight: Some(Rgb::new(255, 249, 196)),
                bold: true,
                italic: true,
                h_align: HAlign::Right,
            },
        };
        let rc = RenderCell::from_cell(&hot);
        assert!(rc.highlighted);
        assert_eq!(rc.fill, 0xFFF9C4);
        assert!(rc.bold && rc.italic);
        assert_eq!(rc.align, Align::Right);
    }

    #[test]
    fn carries_provider_attributes_through() {
        // Over a sample of real synthetic cells, every RenderCell reflects its source.
        let sheet = SyntheticSheet::new(7, 200, 40);
        for r in 0..50 {
            for c in 0..40 {
                let cell = sheet.cell(r, c);
                let rc = RenderCell::from_cell(&cell);
                assert_eq!(rc.bold, cell.format.bold);
                assert_eq!(rc.italic, cell.format.italic);
                assert_eq!(rc.highlighted, cell.format.highlight.is_some());
            }
        }
    }
}
