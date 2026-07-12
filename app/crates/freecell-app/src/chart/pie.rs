//! The FreeCell-owned **pie / doughnut** widget, built over gpui-component's `Pie` (angle layout) +
//! `Arc` (slice paint) primitives (`research/compare-pie.md`). Pie/doughnut is the most visually
//! distinct chart type: **radial slices, no cartesian axes** — the chrome is title + legend (by
//! category) + optional on-slice labels.
//!
//! **The crux is color.** gpui-component has **no auto-palette**: the stock `PieChart` paints every
//! slice the same color unless a per-slice color is supplied. A pie's "slices" are the **categories**
//! of its single series, so the per-slice color is resolved (P24) as: a `c:dPt` override for that
//! slice if present, else the **varied** palette color for the slice index (`c:varyColors`, the pie
//! default), else the single series fill — through the shared
//! [`resolve_slice_color`](super::style::resolve_slice_color) the legend also uses, so slice↔swatch
//! match by construction.
//!
//! **Doughnut** (`doughnut_hole`) draws the ring as an annulus (`inner = hole × outer`).
//! **Rotation** (`c:firstSliceAng`) — Excel measures the first slice's angle in **degrees clockwise
//! from 12 o'clock**, which maps directly onto `Pie::start_angle` (angle 0 = 12 o'clock, increasing =
//! clockwise). **Explosion** (`c:dPt/c:explosion`) pulls a slice outward along its bisector.
//! **On-slice % labels** (`c:dLbls`/`showPercent`) give the part-to-whole read a pie has instead of a
//! value axis.

use std::f32::consts::{PI, TAU};

use gpui::{point, px, Bounds, Hsla, IntoElement, Pixels, TextAlign, Window};
use gpui_component::plot::{
    label::{Text, TEXT_SIZE},
    shape::{Arc, Pie},
    IntoPlot, Plot, PlotLabel,
};

use freecell_chart_model::{Chart, ChartKind, SeriesData};

use super::chrome::chart_frame;
use super::style::{model_hsla, resolve_slice_color};

const HALF_PI: f32 = PI / 2.0;
/// Outer radius as a fraction of the smaller plot dimension (the room left for the disc + any
/// exploded slice).
const OUTER_RADIUS_FRAC: f32 = 0.42;
/// Only label a slice whose share is at least this fraction (avoids clutter on tiny slivers).
const MIN_LABEL_FRACTION: f32 = 0.04;

/// One slice: its value, resolved color, and radial explosion (a fraction of the outer radius the
/// slice is pulled out by). The label is carried by the legend + the on-slice %.
#[derive(Clone)]
struct Slice {
    value: f32,
    color: Hsla,
    /// Radial offset as a fraction of the outer radius (`c:explosion / 100`; 0 = flush).
    explosion: f32,
}

/// A pie / doughnut plot over the raw `Pie` (angle layout) + `Arc` (slice paint) primitives, with a
/// resolved per-slice palette (P24).
#[derive(IntoPlot)]
pub struct PiePlot {
    slices: Vec<Slice>,
    /// Hole radius as a fraction of the outer radius (`None` / 0 = solid pie).
    doughnut_hole: Option<f32>,
    /// The first slice's leading-edge angle, in radians (clockwise from 12 o'clock).
    start_angle: f32,
    /// Whether to draw the on-slice percent labels (`c:dLbls/showPercent`).
    show_percent: bool,
}

impl PiePlot {
    /// Build from a [`ChartKind::Pie`] chart. A pie is single-series: its slices are the first
    /// series' categories/values. Returns `None` for a non-pie chart or one with no such series.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Pie {
            doughnut_hole,
            first_slice_ang,
            vary_colors,
        } = chart.kind
        else {
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
                color: model_hsla(resolve_slice_color(series, i, vary_colors)),
                explosion: series
                    .data_points
                    .iter()
                    .find(|d| d.index as usize == i)
                    .and_then(|d| d.explosion)
                    .map(|e| e as f32 / 100.0)
                    .unwrap_or(0.0),
            })
            .collect();
        let show_percent = series.data_labels.as_ref().is_some_and(|l| l.show_percent);
        Some(Self {
            slices,
            doughnut_hole,
            start_angle: first_slice_ang as f32 * PI / 180.0,
            show_percent,
        })
    }

    /// The `Pie` angle layout for the current slices (rotated by `start_angle`), shared by `paint`
    /// and the tests.
    fn pie(&self) -> Pie<f32> {
        Pie::new()
            .value(|v: &f32| Some(*v))
            .start_angle(self.start_angle)
            .end_angle(self.start_angle + TAU)
    }

    /// The slice sweep angles (radians), for tests: they must sum to ~2π for positive data.
    #[cfg(test)]
    fn sweep_angles(&self) -> Vec<f32> {
        let data: Vec<f32> = self.slices.iter().map(|s| s.value).collect();
        self.pie()
            .arcs(&data)
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
        // Shrink the base radius so the most-exploded slice still fits the plot box.
        let max_explosion = self
            .slices
            .iter()
            .map(|s| s.explosion)
            .fold(0.0_f32, f32::max);
        let outer = OUTER_RADIUS_FRAC * w.min(h) / (1.0 + max_explosion);
        let inner = self.doughnut_hole.unwrap_or(0.0).clamp(0.0, 0.95) * outer;

        let data: Vec<f32> = self.slices.iter().map(|s| s.value).collect();
        let sum: f32 = data.iter().filter(|v| **v > 0.0).sum();
        let pie = self.pie();
        let arc = Arc::new().inner_radius(inner).outer_radius(outer);

        let mut labels: Vec<Text> = Vec::new();
        for arc_data in pie.arcs(&data) {
            let slice = self.slices.get(arc_data.index);
            let color = slice.map(|s| s.color).unwrap_or_else(gpui::black);

            // Explosion: shift the slice's arc center outward along its mid-angle bisector.
            let mid = (arc_data.start_angle + arc_data.end_angle) / 2.0 - HALF_PI;
            let offset = slice.map(|s| s.explosion).unwrap_or(0.0) * outer;
            let (dx, dy) = (offset * mid.cos(), offset * mid.sin());
            let slice_bounds = Bounds {
                origin: point(bounds.origin.x + px(dx), bounds.origin.y + px(dy)),
                size: bounds.size,
            };
            arc.paint(&arc_data, color, None, None, &slice_bounds, window);

            // On-slice percentage label at the slice mid-angle (gated on showPercent; skip slivers).
            if self.show_percent && sum > 0.0 && arc_data.value / sum >= MIN_LABEL_FRACTION {
                let label_r = if inner > 0.0 {
                    (inner + outer) / 2.0
                } else {
                    outer * 0.62
                };
                let lx = w / 2.0 + dx + label_r * mid.cos();
                let ly = h / 2.0 + dy + label_r * mid.sin();
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
    use freecell_chart_model::{
        Axis, Category, ChartColor, Color, DataLabels, DataPoint, Legend, Series,
    };

    fn pie_chart(doughnut_hole: Option<f32>) -> Chart {
        Chart {
            title: Some("Market Share".into()),
            kind: ChartKind::Pie {
                doughnut_hole,
                first_slice_ang: 0,
                vary_colors: true,
            },
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
    fn rotation_shifts_the_first_slice_start_angle() {
        // A 90° firstSliceAng rotates the whole pie: the first arc starts a quarter-turn later.
        let mut chart = pie_chart(None);
        chart.kind = ChartKind::Pie {
            doughnut_hole: None,
            first_slice_ang: 90,
            vary_colors: true,
        };
        let plot = PiePlot::from_chart(&chart).unwrap();
        let data: Vec<f32> = plot.slices.iter().map(|s| s.value).collect();
        let first = &plot.pie().arcs(&data)[0];
        assert!(
            (first.start_angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "first slice starts at 90° (π/2), got {}",
            first.start_angle
        );
        // The sweeps still sum to a full turn regardless of the rotation.
        let total: f32 = plot.sweep_angles().iter().sum();
        assert!((total - std::f32::consts::TAU).abs() < 1e-4);
    }

    #[test]
    fn doughnut_inner_radius_is_hole_times_outer() {
        let plot = PiePlot::from_chart(&pie_chart(Some(0.5))).unwrap();
        assert_eq!(plot.inner_radius(100.0), 50.0);
        let solid = PiePlot::from_chart(&pie_chart(None)).unwrap();
        assert_eq!(solid.inner_radius(100.0), 0.0);
    }

    #[test]
    fn slices_have_distinct_colors_when_vary_colors() {
        let plot = PiePlot::from_chart(&pie_chart(None)).unwrap();
        assert_eq!(plot.slices.len(), 4);
        assert_ne!(plot.slices[0].color, plot.slices[1].color);
        assert_ne!(plot.slices[1].color, plot.slices[2].color);
    }

    #[test]
    fn dpt_overrides_its_slice_color_and_explosion() {
        // A c:dPt on slice 1 recolors it (custom sRGB) and explodes it, while the others keep the
        // varied palette and stay flush.
        let mut chart = pie_chart(None);
        chart.series[0] = chart.series[0].clone().with_data_points(vec![DataPoint {
            index: 1,
            color: Some(ChartColor::Rgb(Color::from_hex(0x123456))),
            explosion: Some(20),
        }]);
        let plot = PiePlot::from_chart(&chart).unwrap();
        assert_eq!(
            plot.slices[1].color,
            model_hsla(Color::from_hex(0x123456)),
            "the dPt color overrides slice 1"
        );
        assert!(
            (plot.slices[1].explosion - 0.20).abs() < 1e-6,
            "slice 1 is exploded 20%"
        );
        assert_eq!(plot.slices[0].explosion, 0.0, "other slices stay flush");
    }

    #[test]
    fn vary_colors_off_paints_every_slice_the_series_color() {
        let mut chart = pie_chart(None);
        chart.kind = ChartKind::Pie {
            doughnut_hole: None,
            first_slice_ang: 0,
            vary_colors: false,
        };
        chart.series[0] = chart.series[0]
            .clone()
            .with_color(Color::from_hex(0xABCDEF));
        let plot = PiePlot::from_chart(&chart).unwrap();
        // Every slice takes the single series fill (varyColors off), not the palette cycle.
        for slice in &plot.slices {
            assert_eq!(slice.color, model_hsla(Color::from_hex(0xABCDEF)));
        }
    }

    #[test]
    fn show_percent_gates_the_on_slice_labels() {
        // No data labels → labels off; showPercent → labels on.
        assert!(!PiePlot::from_chart(&pie_chart(None)).unwrap().show_percent);
        let mut chart = pie_chart(None);
        chart.series[0] = chart.series[0]
            .clone()
            .with_data_labels(DataLabels::new().percent());
        assert!(PiePlot::from_chart(&chart).unwrap().show_percent);
    }

    #[test]
    fn rejects_non_pie() {
        let mut bar = pie_chart(None);
        bar.kind = ChartKind::Bar {
            dir: freecell_chart_model::BarDir::Col,
            grouping: freecell_chart_model::Grouping::Clustered,
            layout: freecell_chart_model::BarLayout::default(),
        };
        assert!(PiePlot::from_chart(&bar).is_none());
    }
}
