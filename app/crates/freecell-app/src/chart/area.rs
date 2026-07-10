//! The FreeCell-owned **area** widget — standard (overlaid), **stacked**, and
//! **100%-stacked** — the nastiest Gate-2 layout (`research/compare-area.md`).
//!
//! The trap: gpui-component's `Area` primitive closes its fill with a **flat** bottom edge at
//! a single scalar `y0` (`plot/shape/area.rs`), so it **cannot** draw the wavy per-x baseline a
//! stacked band needs (band `k`'s bottom is the cumulative top of the bands below it). The
//! research-recommended fix is to **hand-roll the filled polygons**: trace each band's upper
//! boundary forward, then its lower boundary back, and close — which is what this module does
//! with `gpui::PathBuilder`, using the shared cumulative math in [`super::stacking`].
//!
//! It reuses the [`ScalePoint`] category axis + one shared [`NiceScale`] value axis pattern
//! from [`super::line`], and the shared title/axis-title/legend chrome from [`super::chrome`].

use gpui::{
    point, px, Background, Bounds, Hsla, IntoElement, PathBuilder, Pixels, SharedString, TextAlign,
    Window,
};
use gpui_component::plot::{
    scale::{Scale, ScaleLinear, ScalePoint},
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, AXIS_GAP,
};

use freecell_chart_model::{Chart, ChartKind, Grouping, SeriesData};

use super::chrome::chart_frame;
use super::stacking::{category_totals, percent_segments, stacked_segments, Segment};
use super::style::{hsla, resolve_series_hsla, AXIS_STROKE, GRID_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

const VALUE_AXIS_GUTTER: f32 = 46.0;
const PLOT_TOP_GAP: f32 = 14.0;
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Inset of the first/last category point from the plot edges so end labels aren't clipped.
const POINT_INSET: f32 = 8.0;
const TARGET_TICKS: usize = 5;
/// Fill opacity for the area bands (a solid top stroke sits on each band).
const FILL_ALPHA: f32 = 0.82;
const TOP_STROKE_WIDTH: f32 = 1.5;

/// One area band ready to draw: its per-category cumulative `(lo, hi)` segments and color.
#[derive(Clone)]
struct AreaSeries {
    segments: Vec<Segment>,
    color: Hsla,
}

/// A multi-series area plot with hand-rolled stacked-band polygons over one shared value scale.
#[derive(IntoPlot)]
pub struct AreaPlot {
    categories: Vec<SharedString>,
    series: Vec<AreaSeries>,
    scale: NiceScale,
    percent: bool,
}

impl AreaPlot {
    /// Build from any [`ChartKind::Area`] chart. Returns `None` for a non-area chart or one
    /// with no category/value data.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Area { grouping } = chart.kind else {
            return None;
        };

        let mut categories: Option<Vec<SharedString>> = None;
        let mut colors = Vec::new();
        let mut values: Vec<Vec<f64>> = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::CategoryValue {
                categories: cats,
                values: vals,
            } = &s.data
            else {
                continue;
            };
            if categories.is_none() {
                categories = Some(cats.iter().map(|c| c.label().into()).collect());
            }
            colors.push(resolve_series_hsla(s.color, i));
            values.push(vals.clone());
        }

        let categories = categories?;
        if values.is_empty() || categories.is_empty() {
            return None;
        }
        let n = categories.len();
        let percent = matches!(grouping, Grouping::PercentStacked);

        // Per-series cumulative (lo, hi) segments per category.
        let seg_rows = match grouping {
            // Overlaid: every band rises from zero independently.
            Grouping::Standard | Grouping::Clustered => values
                .iter()
                .map(|vals| {
                    (0..n)
                        .map(|c| {
                            let v = vals.get(c).copied().unwrap_or(0.0).max(0.0);
                            Segment { lo: 0.0, hi: v }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            Grouping::Stacked => stacked_segments(&values, n),
            Grouping::PercentStacked => percent_segments(&values, n),
        };

        let scale = match grouping {
            Grouping::Standard | Grouping::Clustered => {
                NiceScale::for_values(values.iter().flatten().copied(), TARGET_TICKS)
            }
            Grouping::Stacked => NiceScale::for_values(category_totals(&values, n), TARGET_TICKS),
            Grouping::PercentStacked => NiceScale::new(0.0, 100.0, TARGET_TICKS),
        };

        let series = colors
            .into_iter()
            .zip(seg_rows)
            .map(|(color, segments)| AreaSeries { segments, color })
            .collect();

        Some(Self {
            categories,
            series,
            scale,
            percent,
        })
    }

    /// The cumulative segments per series (exposed for tests).
    #[cfg(test)]
    fn segment_rows(&self) -> Vec<Vec<Segment>> {
        self.series.iter().map(|s| s.segments.clone()).collect()
    }
}

impl Plot for AreaPlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        let point_scale = ScalePoint::new(
            self.categories.clone(),
            vec![plot_left + POINT_INSET, plot_right - POINT_INSET],
        );
        let xs: Vec<f32> = self
            .categories
            .iter()
            .map(|c| point_scale.tick(c).unwrap_or(plot_left))
            .collect();

        let value_scale = ScaleLinear::new(
            vec![self.scale.min, self.scale.max],
            vec![plot_bottom, plot_top],
        );
        let ticks = self.scale.ticks();

        // Gridlines at each nice tick (horizontal).
        let grid_ys: Vec<Pixels> = ticks
            .iter()
            .filter_map(|t| value_scale.tick(t).map(px))
            .collect();
        Grid::new()
            .stroke(hsla(GRID_STROKE))
            .dash_array(&[px(4.), px(2.)])
            .y(grid_ys)
            .paint(&bounds, window);

        // Axes + labels.
        let value_labels = ticks.iter().filter_map(|t| {
            value_scale.tick(t).map(|y| {
                let text = if self.percent {
                    format!("{}%", format_tick(*t))
                } else {
                    format_tick(*t)
                };
                AxisText::new(text, px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
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

        // Bands. Paint bottom→top so a later (upper) band sits over the one below it. Each band
        // is a filled polygon: upper boundary forward, lower boundary back, closed; with a solid
        // stroke tracing the top edge (the "line" of the area).
        let origin = bounds.origin;
        for s in &self.series {
            let mut fill = PathBuilder::fill();
            let mut top = PathBuilder::stroke(px(TOP_STROKE_WIDTH));

            let upper: Vec<gpui::Point<Pixels>> = s
                .segments
                .iter()
                .enumerate()
                .filter_map(|(i, seg)| {
                    let x = *xs.get(i)?;
                    let y = value_scale.tick(&seg.hi)?;
                    Some(point(px(x) + origin.x, px(y) + origin.y))
                })
                .collect();
            if upper.len() < 2 {
                continue;
            }
            let lower: Vec<gpui::Point<Pixels>> = s
                .segments
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, seg)| {
                    let x = *xs.get(i)?;
                    let y = value_scale.tick(&seg.lo)?;
                    Some(point(px(x) + origin.x, px(y) + origin.y))
                })
                .collect();

            // Fill polygon: forward along the top, back along the bottom.
            fill.move_to(upper[0]);
            for p in &upper[1..] {
                fill.line_to(*p);
            }
            for p in &lower {
                fill.line_to(*p);
            }
            fill.close();

            // Top stroke along the upper boundary only.
            top.move_to(upper[0]);
            for p in &upper[1..] {
                top.line_to(*p);
            }

            let fill_bg: Background = Hsla {
                a: FILL_ALPHA,
                ..s.color
            }
            .into();
            let stroke_bg: Background = s.color.into();
            if let Ok(path) = fill.build() {
                window.paint_path(path, fill_bg);
            }
            if let Ok(path) = top.build() {
                window.paint_path(path, stroke_bg);
            }
        }
    }
}

/// Build the full area chart element (title, axis titles, plot, legend). Returns `None` for a
/// chart this widget can't render.
pub fn area_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = AreaPlot::from_chart(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Category, Legend, Series};

    fn months() -> Vec<Category> {
        ["Jan", "Feb", "Mar"]
            .into_iter()
            .map(|m| Category::Text(m.into()))
            .collect()
    }

    fn area_chart(grouping: Grouping) -> Chart {
        Chart {
            title: Some("Traffic".into()),
            kind: ChartKind::Area { grouping },
            series: vec![
                Series::category_value(Some("Direct"), months(), vec![10.0, 20.0, 30.0]),
                Series::category_value(Some("Search"), months(), vec![20.0, 25.0, 15.0]),
                Series::category_value(Some("Social"), months(), vec![5.0, 15.0, 25.0]),
            ],
            cat_axis: Axis::titled("Month"),
            val_axis: Axis::titled("Visits"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn stacked_baselines_are_cumulative() {
        let plot = AreaPlot::from_chart(&area_chart(Grouping::Stacked)).unwrap();
        let rows = plot.segment_rows();
        for c in 0..3 {
            for s in 0..rows.len() - 1 {
                assert_eq!(
                    rows[s][c].hi,
                    rows[s + 1][c].lo,
                    "band {s} top must equal band {} bottom at category {c}",
                    s + 1
                );
            }
            assert_eq!(rows[0][c].lo, 0.0, "bottom band starts at zero");
        }
    }

    #[test]
    fn percent_sums_to_100_and_axis_is_0_100() {
        let plot = AreaPlot::from_chart(&area_chart(Grouping::PercentStacked)).unwrap();
        assert_eq!(plot.scale.min, 0.0);
        assert_eq!(plot.scale.max, 100.0);
        assert!(plot.percent);
        let rows = plot.segment_rows();
        for c in 0..3 {
            let sum: f64 = rows.iter().map(|row| row[c].height()).sum();
            assert!((sum - 100.0).abs() < 1e-9, "category {c} sum {sum} != 100");
        }
    }

    #[test]
    fn standard_bands_all_start_at_zero() {
        let plot = AreaPlot::from_chart(&area_chart(Grouping::Standard)).unwrap();
        for row in plot.segment_rows() {
            for seg in row {
                assert_eq!(seg.lo, 0.0, "overlaid areas rise from zero");
            }
        }
    }

    #[test]
    fn rejects_non_area() {
        let mut bar = area_chart(Grouping::Stacked);
        bar.kind = ChartKind::Bar {
            dir: freecell_chart_model::BarDir::Col,
            grouping: Grouping::Stacked,
        };
        assert!(AreaPlot::from_chart(&bar).is_none());
    }
}
