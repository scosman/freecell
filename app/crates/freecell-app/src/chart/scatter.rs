//! The FreeCell-owned **scatter (XY)** widget — Gate 3 (functional_spec §4, §7).
//!
//! Scatter is the one type the research (`research/scope-and-gaps.md`) flagged as a genuine
//! step-change, because gpui-component's primitives are category-vs-value oriented
//! (`ScaleBand`/`ScalePoint` for the category axis, `Line`/`Bar`/`Area` shapes). The
//! genuinely new demand is that **BOTH axes are numeric `ScaleLinear`** and the marks are
//! **standalone dots** from `c:xVal`/`c:yVal` pairs — not a connected path.
//!
//! The bet Gate 3 proves: once the axis + legend scaffolding exists, scatter is a *modest*
//! addition. Almost everything here is reused verbatim:
//! - the value-axis pattern from [`super::line`] / [`super::area`] (one shared
//!   [`NiceScale`] → `ScaleLinear`), applied to **both** X and Y;
//! - the shared X/Y domains via [`NiceScale::spanning`] over the union of every series'
//!   values — Excel's scatter auto-ranging (axes zoom to the data, not forced to zero),
//!   exactly the reuse Gate 1 made for the line value axis;
//! - the title / both axis titles / legend from [`super::chrome`] (the Y-axis title lives in
//!   `val_axis`, the X-axis title in `cat_axis`, so both numeric captions render unchanged);
//! - the multi-series color cycle via [`resolve_series_color`](super::style::resolve_series_color),
//!   matched swatch-for-dot-cloud by the legend.
//!
//! The only net-new drawing is the dot mark: gpui-component's `Line` primitive already paints
//! its dots as a rounded quad whose corner radius = half its side (i.e. a filled circle,
//! `plot/shape/line.rs`). Scatter wants those dots *without* the connecting path, so it hand-
//! draws the same quad per point via [`Window::paint_quad`] — the proven dot recipe, no path.

use gpui::{
    point, px, size, Background, BorderStyle, Bounds, Hsla, IntoElement, Pixels, TextAlign, Window,
};
use gpui_component::plot::{
    scale::{Scale, ScaleLinear},
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, AXIS_GAP,
};

use freecell_chart_model::{Chart, ChartKind, SeriesData};

use super::chrome::chart_frame;
use super::style::{hsla, resolve_series_hsla, AXIS_STROKE, GRID_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

/// Pixels reserved at the left of the plot for value-axis (Y) tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top so the highest dot isn't clipped.
const PLOT_TOP_GAP: f32 = 14.0;
/// Pixels reserved at the right so the rightmost dot isn't clipped.
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Roughly how many ticks to aim for on each numeric axis.
const TARGET_TICKS: usize = 5;
/// Dot (marker) diameter.
const DOT_SIZE: f32 = 7.0;
/// Dot outline width — a thin light stroke so overlapping dots stay individually readable.
const DOT_STROKE_WIDTH: f32 = 1.0;

/// One scatter series: its paired x/y values and resolved color.
#[derive(Clone)]
struct ScatterSeries {
    xs: Vec<f64>,
    ys: Vec<f64>,
    color: Hsla,
}

/// A multi-series scatter plot over TWO numeric [`ScaleLinear`] axes, drawing standalone dots
/// at each `(x, y)`.
#[derive(IntoPlot)]
pub struct ScatterPlot {
    series: Vec<ScatterSeries>,
    /// The shared X domain (the union of every series' x-values, nice-d).
    x_scale: NiceScale,
    /// The shared Y domain (the union of every series' y-values, nice-d).
    y_scale: NiceScale,
}

impl ScatterPlot {
    /// Build from a [`ChartKind::Scatter`] chart. Every series contributes to both shared
    /// domains. Returns `None` for a non-scatter chart or one with no xy series.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        if !matches!(chart.kind, ChartKind::Scatter) {
            return None;
        }

        let mut series = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::Xy { x, y } = &s.data else {
                continue;
            };
            let color = resolve_series_hsla(s.color, i);
            series.push(ScatterSeries {
                xs: x.clone(),
                ys: y.clone(),
                color,
            });
        }

        if series.is_empty() {
            return None;
        }

        // The SHARED domains: nice-d over the union of EVERY series' x / y values, so every
        // point of every series is measured against the same two axes.
        let x_scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.xs.iter().copied()),
            TARGET_TICKS,
        );
        let y_scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.ys.iter().copied()),
            TARGET_TICKS,
        );

        Some(Self {
            series,
            x_scale,
            y_scale,
        })
    }

    /// The shared X domain (exposed for tests: it must cover every series' x-values).
    #[cfg(test)]
    fn x_domain(&self) -> NiceScale {
        self.x_scale
    }

    /// The shared Y domain (exposed for tests: it must cover every series' y-values).
    #[cfg(test)]
    fn y_domain(&self) -> NiceScale {
        self.y_scale
    }

    /// Total number of plotted points across all series (exposed for tests: dot count == data).
    #[cfg(test)]
    fn point_count(&self) -> usize {
        self.series.iter().map(|s| s.xs.len().min(s.ys.len())).sum()
    }
}

impl Plot for ScatterPlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        // Both axes are numeric ScaleLinear. X increases left→right; Y is inverted (min at the
        // bottom). The nice outward-rounding pads the data inside each domain, so dots land
        // inset from the frame without any manual pixel inset — ticks/grid stay at the edges.
        let x_axis = ScaleLinear::new(
            vec![self.x_scale.min, self.x_scale.max],
            vec![plot_left, plot_right],
        );
        let y_axis = ScaleLinear::new(
            vec![self.y_scale.min, self.y_scale.max],
            vec![plot_bottom, plot_top],
        );

        let x_ticks = self.x_scale.ticks();
        let y_ticks = self.y_scale.ticks();

        // Grid: vertical lines at each X tick, horizontal lines at each Y tick.
        let grid_xs: Vec<Pixels> = x_ticks
            .iter()
            .filter_map(|t| x_axis.tick(t).map(px))
            .collect();
        let grid_ys: Vec<Pixels> = y_ticks
            .iter()
            .filter_map(|t| y_axis.tick(t).map(px))
            .collect();
        Grid::new()
            .stroke(hsla(GRID_STROKE))
            .dash_array(&[px(4.), px(2.)])
            .x(grid_xs)
            .y(grid_ys)
            .paint(&bounds, window);

        // Axes + numeric tick labels: Y labels right-aligned left of the value axis, X labels
        // centered below the bottom axis (both numeric — where line/area put category text).
        let value_labels = y_ticks.iter().filter_map(|t| {
            y_axis.tick(t).map(|y| {
                AxisText::new(format_tick(*t), px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
            })
        });
        let x_labels = x_ticks.iter().filter_map(|t| {
            x_axis.tick(t).map(|x| {
                AxisText::new(format_tick(*t), px(x), hsla(MUTED_TEXT)).align(TextAlign::Center)
            })
        });
        PlotAxis::new()
            .x(px(plot_bottom))
            .x_label(x_labels)
            .y(px(plot_left))
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(hsla(AXIS_STROKE))
            .paint(&bounds, window, cx);

        // Dots. Each point is a filled circle (a rounded quad, radius = half its side — the
        // exact shape `Line` paints its dot markers with) at the mapped pixel, offset by the
        // plot origin, with a thin light outline so overlapping dots stay readable.
        let origin = bounds.origin;
        let radius = DOT_SIZE / 2.0;
        let diameter = px(DOT_SIZE);
        let dot_stroke = hsla(0xFFFFFF);
        for s in &self.series {
            let fill: Background = s.color.into();
            let n = s.xs.len().min(s.ys.len());
            for i in 0..n {
                let (Some(cx_px), Some(cy_px)) = (x_axis.tick(&s.xs[i]), y_axis.tick(&s.ys[i]))
                else {
                    continue;
                };
                let top_left = point(px(cx_px - radius), px(cy_px - radius)) + origin;
                window.paint_quad(gpui::quad(
                    gpui::bounds(top_left, size(diameter, diameter)),
                    diameter / 2.0,
                    fill,
                    px(DOT_STROKE_WIDTH),
                    dot_stroke,
                    BorderStyle::default(),
                ));
            }
        }
    }
}

/// Build the full scatter chart element (title, both axis titles, plot, legend). Returns
/// `None` for a chart this widget can't render.
pub fn scatter_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = ScatterPlot::from_chart(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Grouping, Legend, Series};

    fn two_series_scatter() -> Chart {
        Chart {
            title: Some("Measurements".into()),
            kind: ChartKind::Scatter,
            series: vec![
                Series::xy(
                    Some("Group A"),
                    vec![1.0, 2.5, 4.0, 5.5],
                    vec![10.0, 22.0, 18.0, 31.0],
                ),
                Series::xy(
                    Some("Group B"),
                    vec![2.0, 3.0, 6.0, 7.5, 9.0],
                    vec![40.0, 55.0, 48.0, 62.0, 70.0],
                ),
            ],
            cat_axis: Axis::titled("X value"),
            val_axis: Axis::titled("Y value"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn shared_domains_cover_all_points() {
        let chart = two_series_scatter();
        let plot = ScatterPlot::from_chart(&chart).expect("scatter plot");
        let x = plot.x_domain();
        let y = plot.y_domain();
        // Both shared domains must contain EVERY point of EVERY series — the core "one shared
        // numeric domain per axis over the union of all series" property Gate 3 hinges on.
        for s in &chart.series {
            if let SeriesData::Xy { x: xs, y: ys } = &s.data {
                for &vx in xs {
                    assert!(
                        x.min <= vx && vx <= x.max,
                        "x {vx} outside shared X domain [{}, {}]",
                        x.min,
                        x.max
                    );
                }
                for &vy in ys {
                    assert!(
                        y.min <= vy && vy <= y.max,
                        "y {vy} outside shared Y domain [{}, {}]",
                        y.min,
                        y.max
                    );
                }
            }
        }
    }

    #[test]
    fn point_count_matches_data() {
        let plot = ScatterPlot::from_chart(&two_series_scatter()).expect("scatter plot");
        // 4 points in Group A + 5 in Group B.
        assert_eq!(plot.point_count(), 9);
    }

    #[test]
    fn multi_series_has_distinct_colors() {
        let plot = ScatterPlot::from_chart(&two_series_scatter()).expect("scatter plot");
        assert_eq!(plot.series.len(), 2);
        assert_ne!(plot.series[0].color, plot.series[1].color);
    }

    #[test]
    fn rejects_non_scatter_and_empty() {
        // A line chart is not a scatter chart.
        let mut line = two_series_scatter();
        line.kind = ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        };
        assert!(ScatterPlot::from_chart(&line).is_none());

        // A scatter chart with no xy series has nothing to draw.
        let mut empty = two_series_scatter();
        empty.series.clear();
        assert!(ScatterPlot::from_chart(&empty).is_none());
    }
}
