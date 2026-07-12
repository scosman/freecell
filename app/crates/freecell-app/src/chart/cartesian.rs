//! Shared **cartesian plot chrome** — the axis lines + gridlines every cartesian chart (line,
//! column, bar, area, scatter, bubble) draws INSIDE its plot rect. Pie/doughnut are radial and use
//! none of this.
//!
//! Centralizing it fixes two things once, for every type:
//! - **gridlines + axis lines are clipped to the plot rect.** gpui-component's `Grid`/`PlotAxis`
//!   primitives draw full-`bounds` lines (`0..width` / `0..height`), so a horizontal value gridline
//!   ran left *through* the y-axis tick labels and on past the plot's right edge. Here every line is
//!   bounded to the inset plot rectangle (the y-axis on the left, the plot's right edge on the
//!   right), so nothing bleeds into the axis-label gutters.
//! - **the value (Y) axis renders as a solid line, like the X axis.** `PlotAxis` never drew the
//!   y-axis line (its `y_axis` flag defaults off), so [`PlotRect::paint_axes`] draws both the
//!   category (X) and value (Y) axis lines at the plot's bottom/left boundaries, at the same weight
//!   and color.
//!
//! The gpui-component `PlotAxis` is still used by each renderer for the tick **labels** (with its own
//! axis line suppressed via `.x_axis(false)`).

use gpui::{px, Bounds, Hsla, PathBuilder, Pixels, Window};
use gpui_component::plot::origin_point;

use super::style::{hsla, AXIS_STROKE, GRID_STROKE};

/// Solid axis-line stroke width (px) — the same 1px the gpui-component `PlotAxis` used for the X
/// axis, so the value (Y) axis we now add reads at the identical weight.
const AXIS_LINE_WIDTH: f32 = 1.0;
/// Dashed gridline stroke width (px) — the gpui-component `Grid` default.
const GRID_LINE_WIDTH: f32 = 1.0;
/// The gridline dash pattern (px on, px off) — matches the `Grid` dashing the renderers used, so only
/// the *bounding* of each gridline changes, not its look.
const GRID_DASH: [f32; 2] = [4.0, 2.0];

/// The plot rectangle (device px, plot-element-relative): the inset area bounded by the value-axis
/// gutter (`left`) + top gap and the plot's right / bottom edges. Axis lines + gridlines are clipped
/// to it.
#[derive(Clone, Copy)]
pub(super) struct PlotRect {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl PlotRect {
    /// The solid **category (X)** + **value (Y)** axis lines, at the bottom and left plot boundaries,
    /// each bounded to the plot rect. Both in [`AXIS_STROKE`] at [`AXIS_LINE_WIDTH`], so the Y axis
    /// (which the gpui-component `PlotAxis` never drew) matches the X axis's weight and color.
    pub(super) fn paint_axes(&self, bounds: &Bounds<Pixels>, window: &mut Window) {
        // X axis: along the bottom, left→right.
        paint_line(
            bounds,
            (self.left, self.bottom),
            (self.right, self.bottom),
            AXIS_LINE_WIDTH,
            None,
            hsla(AXIS_STROKE),
            window,
        );
        // Y axis: up the left, top→bottom.
        paint_line(
            bounds,
            (self.left, self.top),
            (self.left, self.bottom),
            AXIS_LINE_WIDTH,
            None,
            hsla(AXIS_STROKE),
            window,
        );
    }

    /// Horizontal (value) gridlines at each `y` (plot-relative px), each bounded to `left..right` —
    /// so a gridline starts at the Y axis and stops at the plot's right edge, never running under the
    /// tick-label gutter or past the plot.
    pub(super) fn paint_horizontal_gridlines(
        &self,
        bounds: &Bounds<Pixels>,
        ys: &[f32],
        window: &mut Window,
    ) {
        for &y in ys {
            paint_line(
                bounds,
                (self.left, y),
                (self.right, y),
                GRID_LINE_WIDTH,
                Some(&GRID_DASH),
                hsla(GRID_STROKE),
                window,
            );
        }
    }

    /// Vertical gridlines at each `x` (plot-relative px), each bounded to `top..bottom`.
    pub(super) fn paint_vertical_gridlines(
        &self,
        bounds: &Bounds<Pixels>,
        xs: &[f32],
        window: &mut Window,
    ) {
        for &x in xs {
            paint_line(
                bounds,
                (x, self.top),
                (x, self.bottom),
                GRID_LINE_WIDTH,
                Some(&GRID_DASH),
                hsla(GRID_STROKE),
                window,
            );
        }
    }
}

/// Paint one straight line between two plot-relative points `(x, y)`, optionally dashed, in `color`.
/// The points are mapped into absolute window coordinates via [`origin_point`], exactly as the
/// gpui-component `Grid`/`PlotAxis` primitives do.
fn paint_line(
    bounds: &Bounds<Pixels>,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    dash: Option<&[f32]>,
    color: Hsla,
    window: &mut Window,
) {
    let mut builder = PathBuilder::stroke(px(width));
    if let Some(dash) = dash {
        let dashes: Vec<Pixels> = dash.iter().map(|d| px(*d)).collect();
        builder = builder.dash_array(dashes.as_slice());
    }
    builder.move_to(origin_point(px(from.0), px(from.1), bounds.origin));
    builder.line_to(origin_point(px(to.0), px(to.1), bounds.origin));
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}
