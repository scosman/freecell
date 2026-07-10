//! The FreeCell-owned **pie / doughnut** widget, built over gpui-component's `Pie` + `Arc`
//! primitives (`research/compare-pie.md`).
//!
//! **The crux is color.** gpui-component has **no auto-palette**: the stock `PieChart` paints
//! every slice the same `chart_2`, i.e. a monochrome, unreadable disc, unless a per-slice color
//! closure is supplied. A pie's "slices" are the **categories** of a single series, so we
//! synthesize a distinct color per slice from the same categorical
//! [`series_color`](super::palette::series_color) cycle the
//! rest of the charts use — and the legend (in [`super::chrome`]) keys slice→color off that
//! exact same cycle, so the mapping is correct by construction.
//!
//! A **doughnut** is a pie with an inner radius: `inner = doughnut_hole × outer_radius`
//! (`ChartKind::Pie { doughnut_hole }`). On-slice percentage labels give the part-to-whole
//! read a pie has instead of a numeric value axis.

use std::f32::consts::PI;

use gpui::{point, px, Bounds, Hsla, IntoElement, Pixels, TextAlign, Window};
use gpui_component::plot::{
    label::{Text, TEXT_SIZE},
    shape::{Arc, Pie},
    IntoPlot, Plot, PlotLabel,
};

use freecell_chart_model::{Chart, ChartKind, SeriesData};

use super::chrome::chart_frame;
use super::palette::slice_color;
use super::style::model_hsla;

const HALF_PI: f32 = PI / 2.0;
/// Outer radius as a fraction of the smaller plot dimension.
const OUTER_RADIUS_FRAC: f32 = 0.42;
/// Only label a slice whose share is at least this fraction (avoids clutter on tiny slivers).
const MIN_LABEL_FRACTION: f32 = 0.04;

/// One slice: its value and resolved color (the label is carried by the legend + on-slice %).
#[derive(Clone)]
struct Slice {
    value: f32,
    color: Hsla,
}

/// A pie / doughnut plot over the raw `Pie` (angle layout) + `Arc` (slice paint) primitives,
/// with a synthesized per-slice palette.
#[derive(IntoPlot)]
pub struct PiePlot {
    slices: Vec<Slice>,
    /// Hole radius as a fraction of the outer radius (`None` / 0 = solid pie).
    doughnut_hole: Option<f32>,
}

impl PiePlot {
    /// Build from a [`ChartKind::Pie`] chart. A pie is single-series: slices come from the first
    /// category/value series. Returns `None` for a non-pie chart or one with no such series.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Pie { doughnut_hole } = chart.kind else {
            return None;
        };
        let series = chart.series.first()?;
        let SeriesData::CategoryValue { values, .. } = &series.data else {
            return None;
        };
        if values.is_empty() {
            return None;
        }
        let slices = values
            .iter()
            .enumerate()
            .map(|(i, v)| Slice {
                value: *v as f32,
                color: model_hsla(slice_color(i)),
            })
            .collect();
        Some(Self {
            slices,
            doughnut_hole,
        })
    }

    /// The slice sweep angles (radians), for tests: they must sum to ~2π for positive data.
    #[cfg(test)]
    fn sweep_angles(&self) -> Vec<f32> {
        let data: Vec<f32> = self.slices.iter().map(|s| s.value).collect();
        let pie = Pie::new().value(|v: &f32| Some(*v));
        pie.arcs(&data)
            .iter()
            .map(|a| a.end_angle - a.start_angle)
            .collect()
    }

    #[cfg(test)]
    fn inner_radius(&self, outer: f32) -> f32 {
        self.doughnut_hole.unwrap_or(0.0) * outer
    }
}

impl Plot for PiePlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();
        let outer = OUTER_RADIUS_FRAC * w.min(h);
        let inner = self.doughnut_hole.unwrap_or(0.0).clamp(0.0, 0.95) * outer;

        let data: Vec<f32> = self.slices.iter().map(|s| s.value).collect();
        let sum: f32 = data.iter().filter(|v| **v > 0.0).sum();
        let pie = Pie::new().value(|v: &f32| Some(*v));
        let arc = Arc::new().inner_radius(inner).outer_radius(outer);

        let mut labels: Vec<Text> = Vec::new();
        for arc_data in pie.arcs(&data) {
            let color = self
                .slices
                .get(arc_data.index)
                .map(|s| s.color)
                .unwrap_or_else(gpui::black);
            arc.paint(&arc_data, color, None, None, &bounds, window);

            // On-slice percentage label at the slice mid-angle (skip tiny slivers).
            if sum > 0.0 && arc_data.value / sum >= MIN_LABEL_FRACTION {
                let mid = (arc_data.start_angle + arc_data.end_angle) / 2.0 - HALF_PI;
                let label_r = if inner > 0.0 {
                    (inner + outer) / 2.0
                } else {
                    outer * 0.62
                };
                let lx = w / 2.0 + label_r * mid.cos();
                let ly = h / 2.0 + label_r * mid.sin();
                let pct = (arc_data.value / sum * 100.0).round() as i32;
                labels.push(
                    Text::new(
                        format!("{pct}%"),
                        point(px(lx), px(ly - TEXT_SIZE / 2.0)),
                        super::style::hsla(0xFFFFFF),
                    )
                    .align(TextAlign::Center),
                );
            }
        }
        PlotLabel::new(labels).paint(&bounds, window, cx);
    }
}

/// Build the full pie/doughnut chart element (title, plot, per-slice legend). Returns `None`
/// for a chart this widget can't render.
pub fn pie_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = PiePlot::from_chart(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Category, Legend, Series};

    fn pie_chart(doughnut_hole: Option<f32>) -> Chart {
        Chart {
            title: Some("Market Share".into()),
            kind: ChartKind::Pie { doughnut_hole },
            series: vec![Series::category_value(
                Some("Share"),
                vec![
                    Category::Text("A".into()),
                    Category::Text("B".into()),
                    Category::Text("C".into()),
                    Category::Text("D".into()),
                ],
                vec![40.0, 25.0, 20.0, 15.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn slice_angles_sum_to_tau() {
        let plot = PiePlot::from_chart(&pie_chart(None)).unwrap();
        let total: f32 = plot.sweep_angles().iter().sum();
        assert!(
            (total - std::f32::consts::TAU).abs() < 1e-4,
            "slice sweeps sum to {total}, expected 2π"
        );
    }

    #[test]
    fn doughnut_inner_radius_is_hole_times_outer() {
        let plot = PiePlot::from_chart(&pie_chart(Some(0.5))).unwrap();
        assert_eq!(plot.inner_radius(100.0), 50.0);
        let solid = PiePlot::from_chart(&pie_chart(None)).unwrap();
        assert_eq!(solid.inner_radius(100.0), 0.0);
    }

    #[test]
    fn slices_have_distinct_colors() {
        let plot = PiePlot::from_chart(&pie_chart(None)).unwrap();
        assert_eq!(plot.slices.len(), 4);
        assert_ne!(plot.slices[0].color, plot.slices[1].color);
        assert_ne!(plot.slices[1].color, plot.slices[2].color);
    }

    #[test]
    fn rejects_non_pie() {
        let mut bar = pie_chart(None);
        bar.kind = ChartKind::Bar {
            dir: freecell_chart_model::BarDir::Col,
            grouping: freecell_chart_model::Grouping::Clustered,
        };
        assert!(PiePlot::from_chart(&bar).is_none());
    }
}
