//! The FreeCell-owned **scatter (XY)** widget — the first XY type (functional_spec §4, §7, P25).
//!
//! Scatter is the one type the research (`research/scope-and-gaps.md`) flagged as a genuine
//! step-change, because gpui-component's primitives are category-vs-value oriented
//! (`ScaleBand`/`ScalePoint` for the category axis). The genuinely new demand is that **BOTH axes
//! are numeric `ScaleLinear`** and the marks come from `c:xVal`/`c:yVal` pairs — each point maps
//! `(x, y) → pixel` through two independent nice-tick numeric scales.
//!
//! Almost everything else is reused verbatim:
//! - the value-axis pattern from [`super::line`] / [`super::area`] (one shared [`NiceScale`] →
//!   `ScaleLinear`), applied to **both** X and Y over the union of every series' values — Excel's
//!   scatter auto-ranging (axes zoom to the data, not forced to zero);
//! - the title / both axis titles / legend from [`super::chrome`] (the Y-axis title lives in
//!   `val_axis`, the X-axis title in `cat_axis`, so both numeric captions render unchanged);
//! - the multi-series color cycle via [`resolve_series_hsla`](super::style::resolve_series_hsla),
//!   matched swatch-for-dot-cloud by the legend;
//! - the **shared marker painter** [`paint_marker`](super::line::paint_marker) — scatter dots are the
//!   *same* mark the line renderer paints for `c:marker`, so a scatter marker honors the full OOXML
//!   symbol set (circle/square/diamond/…), not a fixed circle.
//!
//! **`c:scatterStyle`** ([`ScatterStyle`]) governs the combination: `marker` draws dots only, `line`
//! draws straight connecting segments only, `lineMarker` draws both (Excel's insert default), and
//! `smooth`/`smoothMarker` fall back to **straight** segments (an honest fidelity choice — the chart
//! is badged Degraded). The connecting segments reuse gpui-component's `Line` primitive, connecting a
//! series' points **in data order** (Excel's scatter-with-lines behavior).

use gpui::{px, Background, Bounds, Hsla, IntoElement, Pixels, TextAlign, Window};
use gpui_component::plot::{
    origin_point,
    scale::{Scale, ScaleLinear},
    shape::Line,
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, StrokeStyle, AXIS_GAP,
};

use freecell_chart_model::{
    cap_markers_for_paint, Chart, ChartKind, Marker, ScatterStyle, SeriesData, MAX_PAINT_MARKERS,
};

use super::chrome::chart_frame;
use super::line::{line_width_px, paint_marker};
use super::style::{hsla, resolve_series_hsla, with_alpha, AXIS_STROKE, GRID_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

/// Pixels reserved at the left of the plot for value-axis (Y) tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top so the highest dot isn't clipped.
const PLOT_TOP_GAP: f32 = 14.0;
/// Pixels reserved at the right so the rightmost dot isn't clipped.
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Roughly how many ticks to aim for on each numeric axis.
const TARGET_TICKS: usize = 5;

/// One scatter series: its paired x/y values, its resolved color, marker, and (connecting) line width.
#[derive(Clone)]
struct ScatterSeries {
    xs: Vec<f64>,
    ys: Vec<f64>,
    color: Hsla,
    /// `c:marker` symbol/size (painted via the shared [`paint_marker`]); `None` = the default dot.
    marker: Option<Marker>,
    /// Connecting-line width in px (from `a:ln@w`, else the Excel-like default) — used only when the
    /// style [`draws_line`](ScatterStyle::draws_line).
    width_px: f32,
}

/// A multi-series scatter plot over TWO numeric [`ScaleLinear`] axes, drawing dots at each `(x, y)`
/// (and, per [`ScatterStyle`], connecting segments).
#[derive(IntoPlot)]
pub struct ScatterPlot {
    series: Vec<ScatterSeries>,
    /// The plotting style (`c:scatterStyle`): whether to draw markers, connecting segments, or both.
    style: ScatterStyle,
    /// The shared X domain (the union of every series' x-values, nice-d).
    x_scale: NiceScale,
    /// The shared Y domain (the union of every series' y-values, nice-d).
    y_scale: NiceScale,
}

impl ScatterPlot {
    /// Build from a [`ChartKind::Scatter`] chart. Every series contributes to both shared
    /// domains. Returns `None` for a non-scatter chart or one with no xy series.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Scatter { style } = chart.kind else {
            return None;
        };

        let mut series = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::Xy { x, y, .. } = &s.data else {
                continue;
            };
            // Color resolution mirrors the line renderer: prefer the `a:ln` stroke color, then the
            // series fill/theme color, then the palette cycle, applying any `a:ln` alpha; the
            // connecting-line width comes from `a:ln@w` (else the Excel-like default).
            let stroke = s.stroke;
            let base = resolve_series_hsla(stroke.and_then(|st| st.color).or(s.color), i);
            let color = match stroke.and_then(|st| st.alpha) {
                Some(alpha) => with_alpha(base, alpha),
                None => base,
            };
            series.push(ScatterSeries {
                xs: x.clone(),
                ys: y.clone(),
                color,
                marker: s.marker,
                width_px: line_width_px(stroke.and_then(|st| st.width_pt)),
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
            style,
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

        // Marks. Per series: the connecting line first (so markers sit on top), then the markers.
        let origin = bounds.origin;
        let draws_line = self.style.draws_line();
        let draws_markers = self.style.draws_markers();
        for s in &self.series {
            let n = s.xs.len().min(s.ys.len());
            // Cloud paint cap (GAPS C-P25-1): a scatter bound to a large range paints one mark (and
            // one connecting-line vertex) per point — unbounded per frame. `cap_markers_for_paint`
            // uniformly sub-samples to <= MAX_PAINT_MARKERS (identity below the cap, so no committed
            // scene moves); both the connecting `Line` and the markers draw over the SAME kept subset,
            // so a huge scatter stays a bounded-cost frame.
            let keep = cap_markers_for_paint(n, MAX_PAINT_MARKERS);
            // Each kept point's mapped plot-relative pixel (`None` for a non-finite value — dropped).
            let mapped: Vec<(Option<f32>, Option<f32>)> = keep
                .iter()
                .map(|&i| {
                    let mx = s.xs[i].is_finite().then(|| x_axis.tick(&s.xs[i])).flatten();
                    let my = s.ys[i].is_finite().then(|| y_axis.tick(&s.ys[i])).flatten();
                    (mx, my)
                })
                .collect();

            // Connecting segments (`line`/`lineMarker`; `smooth`/`smoothMarker` fall back to straight):
            // one `Line` per series through its (kept) points in data order.
            if draws_line {
                let stroke: Background = s.color.into();
                let xs = mapped.clone();
                let ys = mapped.clone();
                Line::new()
                    .data((0..mapped.len()).collect::<Vec<usize>>())
                    .x(move |j: &usize| xs[*j].0)
                    .y(move |j: &usize| ys[*j].1)
                    .stroke(stroke)
                    .stroke_width(px(s.width_px))
                    .stroke_style(StrokeStyle::Linear)
                    .paint(&bounds, window);
            }

            // Markers (`marker`/`lineMarker`/`smoothMarker`): the shared marker mark at each finite
            // point (default = the white-edged dot; honors the series' `c:marker` symbol/size).
            if draws_markers {
                for (mx, my) in &mapped {
                    if let (Some(cx_px), Some(cy_px)) = (mx, my) {
                        let center = origin_point(px(*cx_px), px(*cy_px), origin);
                        paint_marker(window, center, s.marker, s.color);
                    }
                }
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
    use freecell_chart_model::{Axis, Grouping, Legend, MarkerSymbol, Series};

    fn two_series_scatter(style: ScatterStyle) -> Chart {
        Chart {
            title: Some("Measurements".into()),
            kind: ChartKind::Scatter { style },
            series: vec![
                Series::xy(
                    Some("Group A"),
                    vec![1.0, 2.5, 4.0, 5.5],
                    vec![10.0, 22.0, 18.0, 31.0],
                )
                .with_marker(Marker::new(MarkerSymbol::Circle)),
                Series::xy(
                    Some("Group B"),
                    vec![2.0, 3.0, 6.0, 7.5, 9.0],
                    vec![40.0, 55.0, 48.0, 62.0, 70.0],
                )
                .with_marker(Marker::new(MarkerSymbol::Diamond)),
            ],
            cat_axis: Axis::titled("X value"),
            val_axis: Axis::titled("Y value"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn shared_domains_cover_all_points() {
        let chart = two_series_scatter(ScatterStyle::Marker);
        let plot = ScatterPlot::from_chart(&chart).expect("scatter plot");
        let x = plot.x_domain();
        let y = plot.y_domain();
        // Both shared domains must contain EVERY point of EVERY series — the core "one shared
        // numeric domain per axis over the union of all series" property scatter hinges on.
        for s in &chart.series {
            if let SeriesData::Xy { x: xs, y: ys, .. } = &s.data {
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
        let plot = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::Marker))
            .expect("scatter plot");
        // 4 points in Group A + 5 in Group B.
        assert_eq!(plot.point_count(), 9);
    }

    #[test]
    fn multi_series_has_distinct_colors() {
        let plot = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::Marker))
            .expect("scatter plot");
        assert_eq!(plot.series.len(), 2);
        assert_ne!(plot.series[0].color, plot.series[1].color);
    }

    #[test]
    fn series_markers_carry_into_the_plot() {
        let plot = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::LineMarker))
            .expect("scatter plot");
        assert_eq!(
            plot.series[0].marker,
            Some(Marker::new(MarkerSymbol::Circle))
        );
        assert_eq!(
            plot.series[1].marker,
            Some(Marker::new(MarkerSymbol::Diamond))
        );
    }

    #[test]
    fn style_gates_line_and_markers() {
        // marker: dots only; line: segments only; lineMarker: both; smooth*: straight-fallback line.
        let marker = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::Marker)).unwrap();
        assert!(marker.style.draws_markers() && !marker.style.draws_line());

        let line = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::Line)).unwrap();
        assert!(line.style.draws_line() && !line.style.draws_markers());

        let both = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::LineMarker)).unwrap();
        assert!(both.style.draws_line() && both.style.draws_markers());

        let smooth = ScatterPlot::from_chart(&two_series_scatter(ScatterStyle::Smooth)).unwrap();
        assert!(smooth.style.draws_line() && smooth.style.is_smooth());
    }

    #[test]
    fn rejects_non_scatter_and_empty() {
        // A line chart is not a scatter chart.
        let mut line = two_series_scatter(ScatterStyle::Marker);
        line.kind = ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        };
        assert!(ScatterPlot::from_chart(&line).is_none());

        // A scatter chart with no xy series has nothing to draw.
        let mut empty = two_series_scatter(ScatterStyle::Marker);
        empty.series.clear();
        assert!(ScatterPlot::from_chart(&empty).is_none());
    }
}
