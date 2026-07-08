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

/// Vertical text alignment (parallel to the horizontal [`Align`]). `None` on a [`RenderStyle`]
/// means "no explicit vertical alignment" — the grid keeps its default vertical placement, so a
/// cell only moves when it opts in (`functional_spec.md §1.3`). Excel's `Justify`/`Distributed`
/// are out of scope and resolve to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

/// A resolved, ready-to-draw cell style. All fields describe *only* what the grid paints;
/// anything the engine models but the grid ignores (diagonal borders, justify/distributed
/// alignment, …) is intentionally absent — it is preserved in the engine and on save, never in
/// this render form (`functional_spec.md §3.6`).
///
/// `Default` (all fields zero/`None`/`false`) is the plain cell whose `num_fmt` index `0` resolves
/// to `"general"` and whose `font_size_q`/`font_family` `0` mean "the workbook default font" — so a
/// default cell interns to the default style and resolves to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RenderStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Strikethrough: a horizontal line through the vertical middle of the cell text (IronCalc
    /// `font.strike`). Toggled like [`bold`](Self::bold); combines with `underline`.
    pub strikethrough: bool,
    /// Wrap text: when set, the cell's text wraps to the column width and renders as multiple
    /// lines clipped to the row height (IronCalc `alignment.wrap_text`); when unset, the current
    /// single-line behavior is unchanged (`functional_spec.md §1.2`).
    pub wrap: bool,
    /// Background fill, if any (`None` = the default white cell).
    pub fill: Option<Rgb>,
    /// Explicit font colour, e.g. a number-format `[Red]` override (`None` = near-black
    /// default, chosen by the grid).
    pub font_color: Option<Rgb>,
    /// Explicit horizontal alignment (`None` = engine default by cell type).
    pub h_align: Option<Align>,
    /// Explicit vertical alignment (`None` = the grid's default vertical placement — unset cells
    /// don't move, `functional_spec.md §1.3`).
    pub v_align: Option<VAlign>,
    /// Index into the owning [`SheetCache`](crate::SheetCache)'s `num_fmts` side table for
    /// this cell's number-format code string; `0` = the default `"general"`. The grid does
    /// not render from this (display text is engine-formatted in the publication) — it is the
    /// action bar's source for the number-format category + decimals ± (`components/action_bar.md`,
    /// `components/style_render.md`). It still participates in interning identity, so cells that
    /// differ only by format get distinct [`StyleId`](crate::StyleId)s.
    pub num_fmt: u16,
    /// Font size in **quarter-points**; `0` = the workbook default font size (rendered at the
    /// grid's default text size). A non-zero value renders the cell at `q/4` pt
    /// (`components/style_render.md`). `0` is the workbook default — **not** a hardcoded 11pt —
    /// so every default cell (new-workbook 13pt Calibri or an opened file's own default) stays the
    /// default style (the engine resolves it relative to the workbook default, like `font.color`
    /// vs black).
    pub font_size_q: u16,
    /// Index into the owning [`SheetCache`](crate::SheetCache)'s `font_families` side table for
    /// this cell's font-family name; `0` = the workbook default font (rendered in the grid's
    /// default family). Non-zero renders that family (missing families fall back via gpui's
    /// fallback stack — display-only, the style is preserved).
    pub font_family: u16,
    /// Index into the owning [`SheetCache`](crate::SheetCache)'s `border_specs` side table for this
    /// cell's resolved [`BorderSpec`](crate::BorderSpec); `0` = [`BorderSpec::NONE`](crate::BorderSpec::NONE)
    /// (no borders). Non-zero draws that cell's edges (the grid resolves the spec from the side
    /// table and paints each effective edge — `components/style_render.md §Border painting`). Like
    /// the other side-table indices it participates in interning identity, so cells that differ
    /// only by border get distinct [`StyleId`](crate::StyleId)s.
    pub border: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_style_default_is_plain() {
        let s = RenderStyle::default();
        assert!(!s.bold && !s.italic && !s.underline);
        assert!(!s.strikethrough && !s.wrap);
        assert_eq!(s.fill, None);
        assert_eq!(s.font_color, None);
        assert_eq!(s.h_align, None);
        assert_eq!(s.v_align, None);
        assert_eq!(
            s.num_fmt, 0,
            "a default cell uses the General format (index 0)"
        );
        assert_eq!(
            s.font_size_q, 0,
            "a default cell uses the workbook default size"
        );
        assert_eq!(
            s.font_family, 0,
            "a default cell uses the workbook default family"
        );
        assert_eq!(
            s.border, 0,
            "a default cell has no borders (index 0 = NONE)"
        );
    }
}
