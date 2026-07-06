//! Theme-independent colors + small color helpers shared by every chart widget.
//!
//! The widgets pass **explicit** colors to the plot primitives instead of reading
//! `cx.theme()`, so the headless capture is deterministic and high-contrast regardless of
//! the ambient (possibly dark) gpui-component theme. Keeping them in one module means the
//! bar plot, the line plot, and the surrounding chrome (title / axis titles / legend) all
//! agree on one palette.

use gpui::{rgb, Hsla};

use chart_model::Color as ModelColor;

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

/// Convert a [`chart_model::Color`] to a gpui `Hsla`.
pub fn model_hsla(color: ModelColor) -> Hsla {
    Hsla::from(rgb(color.to_hex()))
}
