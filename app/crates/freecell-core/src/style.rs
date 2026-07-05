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
///
/// `Default` (all fields zero/`None`/`false`) is the plain cell whose `num_fmt` index `0` resolves
/// to `"general"` — so a default cell interns to the default style and resolves to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
    /// Index into the owning [`SheetCache`](crate::SheetCache)'s `num_fmts` side table for
    /// this cell's number-format code string; `0` = the default `"general"`. The grid does
    /// not render from this (display text is engine-formatted in the publication) — it is the
    /// action bar's source for the number-format category + decimals ± (`components/action_bar.md`,
    /// `components/style_render.md`). It still participates in interning identity, so cells that
    /// differ only by format get distinct [`StyleId`](crate::StyleId)s.
    pub num_fmt: u16,
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
        assert_eq!(
            s.num_fmt, 0,
            "a default cell uses the General format (index 0)"
        );
    }
}
