//! The FreeCell-owned **multi-series line** widget — Gate 1, the make-or-break example
//! (functional_spec §3, §7). Built over gpui-component's raw `Line` primitive rather than the
//! stock `LineChart` struct, because the struct is single-line only and, critically, each
//! `LineChart` normalizes its **own** y-domain, so overlaying several does not share a value
//! scale (`research/compare-line.md`).
//!
//! The pieces this proves FreeCell can build:
//! - **N lines over ONE shared value scale.** All series measure against a single
//!   [`NiceScale`] computed over the union of every series' values, so their heights are
//!   directly comparable (the exact thing overlaid `LineChart`s cannot do).
//! - **Straight segments** ([`StrokeStyle::Linear`]) — Excel's default line, not the
//!   primitive's `Natural` (curved) default.
//! - a **numeric value axis** with readable "nice" tick labels + gridlines (our
//!   [`NiceScale`]; the linear scale ships no tick generator);
//! - a **category axis** (via [`ScalePoint`], which — unlike [`ScaleBand`] — honors its range
//!   start, so no gutter fix-up is needed);
//! - the **multi-series color cycle** ([`series_color`]), matched swatch-for-swatch by the
//!   legend the surrounding [`super::chrome`] frame draws.

use gpui::{px, Background, Bounds, Hsla, IntoElement, Pixels, SharedString, TextAlign, Window};
use gpui_component::plot::{
    scale::{Scale, ScaleLinear, ScalePoint},
    shape::Line,
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, StrokeStyle, AXIS_GAP,
};

use freecell_chart_model::{Chart, ChartKind, SeriesData};

use super::chrome::chart_frame;
use super::palette::series_color;
use super::style::{hsla, model_hsla, AXIS_STROKE, GRID_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

/// Pixels reserved at the left of the plot for value-axis tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top so the highest point/dot isn't clipped.
const PLOT_TOP_GAP: f32 = 14.0;
/// Pixels reserved at the right so the last point/label isn't clipped.
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Inset of the first/last category point from the plot edges, so end dots and their centered
/// labels have room to breathe instead of sitting on the axis line / frame edge.
const POINT_INSET: f32 = 10.0;
/// Roughly how many value-axis ticks to aim for.
const TARGET_TICKS: usize = 5;
/// Line stroke width.
const LINE_WIDTH: f32 = 2.0;
/// Marker (dot) diameter.
const DOT_SIZE: f32 = 6.0;

/// One line: its per-category values and its color (already resolved from the series' explicit
/// color or the palette cycle).
#[derive(Clone)]
struct LineSeries {
    values: Vec<f64>,
    color: Hsla,
}

/// A multi-series line plot over the raw `Line` primitive with ONE shared value scale
/// ([`Self::scale`]) covering every series.
#[derive(IntoPlot)]
pub struct LinePlot {
    categories: Vec<SharedString>,
    series: Vec<LineSeries>,
    /// The single value domain shared by every series (the union of all their values, nice-d).
    scale: NiceScale,
}

impl LinePlot {
    /// Build the plot from a [`ChartKind::Line`] chart. Categories come from the first series;
    /// every series contributes to the shared value domain. Returns `None` for a non-line
    /// chart or one with no category/value data.
    pub fn multi_series(chart: &Chart) -> Option<Self> {
        if !matches!(chart.kind, ChartKind::Line { .. }) {
            return None;
        }

        let mut categories: Option<Vec<SharedString>> = None;
        let mut series = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::CategoryValue {
                categories: cats,
                values,
            } = &s.data
            else {
                continue;
            };
            if categories.is_none() {
                categories = Some(cats.iter().map(|c| c.label().into()).collect());
            }
            let color = model_hsla(s.color.unwrap_or_else(|| series_color(i)));
            series.push(LineSeries {
                values: values.clone(),
                color,
            });
        }

        let categories = categories?;
        if series.is_empty() {
            return None;
        }

        // The SHARED value domain: nice-d over the union of EVERY series' values, so all lines
        // are drawn against one scale and their heights compare directly.
        let scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.values.iter().copied()),
            TARGET_TICKS,
        );

        Some(Self {
            categories,
            series,
            scale,
        })
    }

    /// The shared value domain (exposed for tests: it must cover every series' values).
    #[cfg(test)]
    fn shared_scale(&self) -> NiceScale {
        self.scale
    }
}

impl Plot for LinePlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        // Category axis: evenly spaced points, first/last inset from the plot edges. `ScalePoint`
        // (unlike `ScaleBand`) honors its range start, so we can hand it the true pixel range.
        let point_scale = ScalePoint::new(
            self.categories.clone(),
            vec![plot_left + POINT_INSET, plot_right - POINT_INSET],
        );
        // Precompute each category's x pixel once and share it across every series.
        let xs: Vec<f32> = self
            .categories
            .iter()
            .map(|c| point_scale.tick(c).unwrap_or(plot_left))
            .collect();

        // Value axis: the ONE shared nice domain -> pixel range (inverted: min at the bottom).
        let value_scale = ScaleLinear::new(
            vec![self.scale.min, self.scale.max],
            vec![plot_bottom, plot_top],
        );
        let ticks = self.scale.ticks();

        // Gridlines at each nice tick (horizontal, for the value axis).
        let grid_ys: Vec<f32> = ticks.iter().filter_map(|t| value_scale.tick(t)).collect();
        Grid::new()
            .stroke(hsla(GRID_STROKE))
            .dash_array(&[px(4.), px(2.)])
            .y(grid_ys)
            .paint(&bounds, window);

        // Axes + labels: value labels left of the value axis, category labels below the baseline.
        let value_labels = ticks.iter().filter_map(|t| {
            value_scale.tick(t).map(|y| {
                AxisText::new(format_tick(*t), px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
            })
        });
        let cat_labels = self.categories.iter().enumerate().map(|(i, c)| {
            AxisText::new(c.clone(), px(xs[i]), hsla(MUTED_TEXT)).align(TextAlign::Center)
        });
        PlotAxis::new()
            .x(px(plot_bottom))
            .x_label(cat_labels)
            .y(px(plot_left))
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(hsla(AXIS_STROKE))
            .paint(&bounds, window, cx);

        // One `Line` per series, all sharing `xs` (category positions) and `value_scale`
        // (the shared value domain). Straight segments (Excel's default), with small dot
        // markers to keep crossing lines readable.
        for s in &self.series {
            let xs = xs.clone();
            let values = s.values.clone();
            let value_scale = value_scale.clone();
            let stroke: Background = s.color.into();
            let n = values.len().min(xs.len());

            Line::new()
                .data((0..n).collect::<Vec<usize>>())
                .x(move |i: &usize| Some(xs[*i]))
                .y(move |i: &usize| value_scale.tick(&values[*i]))
                .stroke(stroke)
                .stroke_width(px(LINE_WIDTH))
                .stroke_style(StrokeStyle::Linear)
                .dot()
                .dot_size(px(DOT_SIZE))
                .dot_fill_color(s.color)
                .dot_stroke_color(hsla(0xFFFFFF))
                .paint(&bounds, window);
        }
    }
}

/// Build the full multi-series line chart element (title, axis titles, plot, legend). Returns
/// `None` for a chart this widget can't render.
pub fn line_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = LinePlot::multi_series(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Category, Grouping, Legend, Series};

    fn q_categories() -> Vec<Category> {
        vec![
            Category::Text("Q1".into()),
            Category::Text("Q2".into()),
            Category::Text("Q3".into()),
        ]
    }

    fn three_series_line() -> Chart {
        Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![
                Series::category_value(Some("North"), q_categories(), vec![32.0, 55.0, 91.0]),
                Series::category_value(Some("South"), q_categories(), vec![74.0, 48.0, 63.0]),
                Series::category_value(Some("West"), q_categories(), vec![50.0, 49.0, 66.0]),
            ],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("Units"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn shared_scale_covers_all_series() {
        let chart = three_series_line();
        let plot = LinePlot::multi_series(&chart).expect("line plot");
        let scale = plot.shared_scale();
        // The single shared domain must contain EVERY value of EVERY series — the core
        // "one shared value scale across all series" property Gate 1 hinges on.
        for s in &chart.series {
            if let SeriesData::CategoryValue { values, .. } = &s.data {
                for &v in values {
                    assert!(
                        scale.min <= v && v <= scale.max,
                        "value {v} outside shared domain [{}, {}]",
                        scale.min,
                        scale.max
                    );
                }
            }
        }
        // Zoomed to the data (not forced to zero) — the min value is 32.
        assert!(
            scale.min > 0.0,
            "line axis should not force zero: {}",
            scale.min
        );
    }

    #[test]
    fn multi_series_reads_all_series_and_categories() {
        let chart = three_series_line();
        let plot = LinePlot::multi_series(&chart).expect("line plot");
        assert_eq!(plot.series.len(), 3, "all three series must be kept");
        assert_eq!(plot.categories.len(), 3, "all categories must be kept");
        // Each series carries its own resolved color (distinct palette entries).
        assert_ne!(plot.series[0].color, plot.series[1].color);
        assert_ne!(plot.series[1].color, plot.series[2].color);
    }

    #[test]
    fn rejects_non_line_and_empty() {
        // A bar chart is not a line chart.
        let mut bar = three_series_line();
        bar.kind = ChartKind::Bar {
            dir: freecell_chart_model::BarDir::Col,
            grouping: Grouping::Clustered,
        };
        assert!(LinePlot::multi_series(&bar).is_none());

        // A line chart with no series has nothing to draw.
        let mut empty = three_series_line();
        empty.series.clear();
        assert!(LinePlot::multi_series(&empty).is_none());
    }
}
