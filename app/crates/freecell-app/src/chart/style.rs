//! Theme-independent colors + small color helpers shared by every chart widget.
//!
//! The widgets pass **explicit** colors to the plot primitives instead of reading
//! `cx.theme()`, so the headless capture is deterministic and high-contrast regardless of
//! the ambient (possibly dark) gpui-component theme. Keeping them in one module means the
//! bar plot, the line plot, and the surrounding chrome (title / axis titles / legend) all
//! agree on one palette.

use gpui::{rgb, Hsla};

use freecell_chart_model::{ChartColor, Color as ModelColor, ThemePalette};

use super::palette::series_color;

/// Chart background (behind the whole widget).
pub const BACKGROUND: u32 = 0xFFFFFF;
/// Chart title text.
pub const TITLE_TEXT: u32 = 0x1A1A1A;
/// Axis-title text (value / category captions).
pub const AXIS_TITLE_TEXT: u32 = 0x374151;
/// Tick-label / muted text.
pub const MUTED_TEXT: u32 = 0x6B7280;
/// Axis line stroke.
pub const AXIS_STROKE: u32 = 0x9CA3AF;
/// Gridline stroke.
pub const GRID_STROKE: u32 = 0xE5E7EB;

/// Convert a packed `0xRRGGBB` value to a gpui `Hsla`.
pub fn hsla(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

/// Convert a [`freecell_chart_model::Color`] to a gpui `Hsla`.
pub fn model_hsla(color: ModelColor) -> Hsla {
    Hsla::from(rgb(color.to_hex()))
}

/// The theme palette a standalone chart resolves `schemeClr` references against. The isolated
/// render component (P5/P6) has no workbook, so it uses the default **Office** theme — correct for
/// the common default-theme file; P8 threads the actual workbook `clrScheme` when a chart is drawn
/// in the grid.
pub fn render_theme_palette() -> ThemePalette {
    ThemePalette::office_default()
}

/// Resolve a series' optional model color ([`ChartColor`] — explicit sRGB or a theme reference) to
/// a concrete [`ModelColor`], falling back to the categorical palette cycle at `index` when the
/// series carries no explicit color (functional_spec §4 P1).
pub fn resolve_series_color(color: Option<ChartColor>, index: usize) -> ModelColor {
    color
        .map(|c| c.resolve(&render_theme_palette()))
        .unwrap_or_else(|| series_color(index))
}

/// [`resolve_series_color`] as a gpui `Hsla` — the form the plot primitives consume.
pub fn resolve_series_hsla(color: Option<ChartColor>, index: usize) -> Hsla {
    model_hsla(resolve_series_color(color, index))
}
