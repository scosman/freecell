//! `RenderStyle` — the engine-free, fully-resolved cell style the grid draws.
//!
//! The style cache pre-resolves every IronCalc `Style` into one of these, so the render
//! path does zero engine-type work (`components/style_cache.md`, `architecture.md §6`).
//! It carries exactly what the MVP grid can draw (`functional_spec.md §3.6`); rendering
//! features added later extend it.

use crate::color::Rgb;

/// Horizontal text alignment. `None` on a [`RenderStyle`] means "engine default" (text
/// left, numbers/dates right, booleans/errors center — resolved by the grid per cell type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A resolved, ready-to-draw cell style. All fields describe *only* what the MVP grid
/// paints; anything the engine models but the grid ignores (borders, font family/size,
/// strikethrough, wrap, …) is intentionally absent — it is preserved in the engine and on
/// save, never in this render form (`functional_spec.md §3.6`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Background fill, if any (`None` = the default white cell).
    pub fill: Option<Rgb>,
    /// Explicit font colour, e.g. a number-format `[Red]` override (`None` = near-black
    /// default, chosen by the grid).
    pub font_color: Option<Rgb>,
    /// Explicit horizontal alignment (`None` = engine default by cell type).
    pub h_align: Option<Align>,
    /// Whether the cell uses the default (`General`) number format. `false` marks a
    /// custom format so the grid/engine display path knows the string is engine-formatted.
    pub num_format_is_default: bool,
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            fill: None,
            font_color: None,
            h_align: None,
            num_format_is_default: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_style_default_is_plain() {
        let s = RenderStyle::default();
        assert!(!s.bold && !s.italic && !s.underline);
        assert_eq!(s.fill, None);
        assert_eq!(s.font_color, None);
        assert_eq!(s.h_align, None);
        assert!(
            s.num_format_is_default,
            "a default cell uses the General format"
        );
    }
}
