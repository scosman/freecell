//! Theme-independent colors + small color helpers shared by every chart widget.
//!
//! The widgets pass **explicit** colors to the plot primitives instead of reading
//! `cx.theme()`, so the headless capture is deterministic and high-contrast regardless of
//! the ambient (possibly dark) gpui-component theme. Keeping them in one module means the
//! bar plot, the line plot, and the surrounding chrome (title / axis titles / legend) all
//! agree on one palette.

use gpui::{rgb, Hsla};

use freecell_chart_model::{ChartColor, Color as ModelColor, Series, ThemePalette};

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

// Font sizes/weights tuned toward Excel's chart proportions (P13, observation B). Excel's default
// line chart draws the TITLE bold + noticeably larger than the axis titles, axis titles **bold** at
// the tick-label size, and tick/legend text regular. We bundle Inter (not Calibri), so we match the
// weight/size proportions, not the typeface (an accepted GAP).
/// Chart title font size — bold, the largest text in the chart (Excel emits ~18pt).
pub const TITLE_FONT_SIZE: f32 = 18.0;
/// Axis-title (value/category caption) font size — drawn **bold** (Excel axis titles are bold).
pub const AXIS_TITLE_FONT_SIZE: f32 = 12.0;
/// Legend entry font size.
pub const LEGEND_FONT_SIZE: f32 = 11.0;

/// Convert a packed `0xRRGGBB` value to a gpui `Hsla`.
pub fn hsla(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

/// Apply an opacity `alpha` (fraction in `0..=1`) to an `Hsla` — the `a:ln/a:alpha` of a line
/// stroke (P13). `alpha` is clamped to `0..=1`.
pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: alpha.clamp(0.0, 1.0),
        ..color
    }
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

/// The resolved fill color for pie/doughnut **slice** `index` (P24). Precedence:
/// 1. a `c:dPt` per-slice override for this index (resolved against the theme), else
/// 2. the **varied** palette color for the slice, when `vary_colors` is on (the pie default), else
/// 3. the single series fill (the `c:varyColors="0"` case — every slice the same color).
///
/// The renderer ([`super::pie`]) and the legend ([`super::chrome`]) both call this, so a slice and
/// its legend swatch match **by construction** — including a dPt override and the `varyColors`-off
/// case.
pub fn resolve_slice_color(series: &Series, index: usize, vary_colors: bool) -> ModelColor {
    if let Some(dp) = series
        .data_points
        .iter()
        .find(|d| d.index as usize == index)
    {
        if let Some(color) = dp.color {
            return color.resolve(&render_theme_palette());
        }
    }
    if vary_colors {
        series_color(index)
    } else {
        // varyColors off: every slice takes the single series fill (or palette slot 0 when unset).
        resolve_series_color(series.color, 0)
    }
}
