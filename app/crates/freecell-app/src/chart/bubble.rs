//! The FreeCell-owned **bubble** widget — the last chart type (functional_spec §4, §7, P26).
//!
//! Bubble is **scatter + a third value per point** (`c:bubbleSize`): each point is `(x, y, size)`
//! over the **same two numeric axes** as scatter, drawn as a filled circle whose **area** (Excel's
//! default) or **width** encodes the size. So the whole frame is scatter's, reused verbatim:
//! - the two numeric `ScaleLinear` axes over the union of every series' x / y ([`NiceScale`]);
//! - the title / both axis titles / legend from [`super::chrome`];
//! - the multi-series color cycle via [`resolve_series_hsla`](super::style::resolve_series_hsla),
//!   matched swatch-for-bubble-cloud by the legend.
//!
//! The one genuinely new variable is **dot size → radius**:
//! - **Area** ([`SizeRepresentation::Area`], the default): `radius ∝ √size`, so equal size *ratios*
//!   read as equal *area* ratios (Excel's convention);
//! - **Width** ([`SizeRepresentation::Width`]): `radius ∝ size`.
//!
//! Both map through a **min/max radius clamp** ([`MIN_BUBBLE_RADIUS`]/[`MAX_BUBBLE_RADIUS`]) so a tiny
//! value stays visible and a huge one can't swamp the plot. Bubbles are drawn with a **translucent
//! fill** + a solid series-colored edge so overlapping bubbles remain readable, and **largest-first**
//! so a small bubble is never hidden behind a big one. Like scatter, the per-frame mark count is
//! bounded by the [`cap_markers_for_paint`] cloud cap (GAPS C-P25-1).

use gpui::{px, size, BorderStyle, Bounds, Hsla, IntoElement, Pixels, TextAlign, Window};
use gpui_component::plot::{
    origin_point,
    scale::{Scale, ScaleLinear},
    AxisLabelSide, AxisText, IntoPlot, Plot, PlotAxis, AXIS_GAP,
};

use freecell_chart_model::{
    cap_markers_for_paint, Chart, ChartKind, SeriesData, SizeRepresentation, MAX_PAINT_MARKERS,
};

use super::cartesian::PlotRect;
use super::chrome::chart_frame;
use super::style::{hsla, resolve_series_hsla, with_alpha, AXIS_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

/// Pixels reserved at the left of the plot for value-axis (Y) tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top so the highest bubble isn't clipped (allows for a near-max radius).
const PLOT_TOP_GAP: f32 = 30.0;
/// Pixels reserved at the right so the rightmost bubble isn't clipped.
const PLOT_RIGHT_GAP: f32 = 32.0;
/// Roughly how many ticks to aim for on each numeric axis.
const TARGET_TICKS: usize = 5;

/// The smallest bubble radius (px) — a tiny/zero size still draws a visible dot.
const MIN_BUBBLE_RADIUS: f32 = 4.0;
/// The largest bubble radius (px) — the biggest size is clamped here so it can't swamp the plot.
const MAX_BUBBLE_RADIUS: f32 = 26.0;
/// The radius (px) used when a bubble series carries no usable size data (all sizes absent / ≤ 0).
const DEFAULT_BUBBLE_RADIUS: f32 = 9.0;
/// Fill opacity for a bubble's disc — translucent so overlapping bubbles stay readable.
const BUBBLE_FILL_ALPHA: f32 = 0.45;
/// The solid series-colored edge width (px) around each bubble.
const BUBBLE_EDGE_WIDTH: f32 = 1.5;

/// The px radius a bubble of `size` gets, given the max size across the chart and the size
/// representation. Pure (unit-tested): area → `radius ∝ √size`, width → `radius ∝ size`, both scaled
/// so the max size hits [`MAX_BUBBLE_RADIUS`] and clamped to `[MIN_BUBBLE_RADIUS, MAX_BUBBLE_RADIUS]`.
/// A non-positive `max_size` (no usable sizes) yields [`DEFAULT_BUBBLE_RADIUS`].
fn bubble_radius(size: f64, max_size: f64, representation: SizeRepresentation) -> f32 {
    // `max_size` is a fold over finite positive sizes starting at 0.0, so it is always finite here;
    // `<= 0.0` means "no usable sizes".
    if max_size <= 0.0 {
        return DEFAULT_BUBBLE_RADIUS;
    }
    // Negative sizes aren't drawn as negative bubbles (showNegBubbles is off by default); clamp ≤ 0
    // to the smallest bubble.
    let s = size.max(0.0);
    let frac = match representation {
        SizeRepresentation::Area => (s / max_size).sqrt(),
        SizeRepresentation::Width => s / max_size,
    } as f32;
    (frac * MAX_BUBBLE_RADIUS).clamp(MIN_BUBBLE_RADIUS, MAX_BUBBLE_RADIUS)
}

/// One bubble series: its x/y points, its per-point sizes (parallel to x/y; empty when unbound), and
/// its resolved color.
#[derive(Clone)]
struct BubbleSeries {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// `c:bubbleSize` values, one per point; empty (a bubble whose size range is unbound) → every
    /// bubble draws at [`DEFAULT_BUBBLE_RADIUS`].
    sizes: Vec<f64>,
    color: Hsla,
}

/// A multi-series bubble plot over TWO numeric [`ScaleLinear`] axes, drawing a sized filled circle at
/// each `(x, y)` whose area/width encodes its `size`.
#[derive(IntoPlot)]
pub struct BubblePlot {
    series: Vec<BubbleSeries>,
    /// `c:sizeRepresents` — whether the bubble **area** (default) or **width** encodes the size.
    representation: SizeRepresentation,
    /// The largest (positive) size across every series — the normalizer for the radius mapping.
    max_size: f64,
    /// The shared X domain (the union of every series' x-values, nice-d).
    x_scale: NiceScale,
    /// The shared Y domain (the union of every series' y-values, nice-d).
    y_scale: NiceScale,
}

impl BubblePlot {
    /// Build from a [`ChartKind::Bubble`] chart. Every series contributes to both shared domains and
    /// to the shared max-size normalizer. Returns `None` for a non-bubble chart or one with no xy
    /// series.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Bubble {
            size_representation,
        } = chart.kind
        else {
            return None;
        };

        let mut series = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::Xy { x, y, size } = &s.data else {
                continue;
            };
            let color = resolve_series_hsla(s.color, i);
            series.push(BubbleSeries {
                xs: x.clone(),
                ys: y.clone(),
                sizes: size.clone().unwrap_or_default(),
                color,
            });
        }
        if series.is_empty() {
            return None;
        }

        // The SHARED domains: nice-d over the union of EVERY series' x / y values.
        let x_scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.xs.iter().copied()),
            TARGET_TICKS,
        );
        let y_scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.ys.iter().copied()),
            TARGET_TICKS,
        );
        // The shared max size (finite, positive) across all series — the radius normalizer, so a
        // bubble's radius is comparable across series (Excel scales all series to one size range).
        let max_size = series
            .iter()
            .flat_map(|s| s.sizes.iter().copied())
            .filter(|v| v.is_finite() && *v > 0.0)
            .fold(0.0_f64, f64::max);

        Some(Self {
            series,
            representation: size_representation,
            max_size,
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

    /// Total number of plotted bubbles across all series (exposed for tests: count == data).
    #[cfg(test)]
    fn point_count(&self) -> usize {
        self.series.iter().map(|s| s.xs.len().min(s.ys.len())).sum()
    }
}

impl Plot for BubblePlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        // Both axes are numeric ScaleLinear — X increases left→right, Y is inverted (min at bottom),
        // exactly as scatter maps its dots.
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

        // The plot rect the gridlines + axis lines are clipped to (the shared cartesian chrome).
        let rect = PlotRect {
            left: plot_left,
            right: plot_right,
            top: plot_top,
            bottom: plot_bottom,
        };

        // Grid: vertical lines at each X tick, horizontal at each Y tick — both bounded to the plot
        // rect.
        let grid_xs: Vec<f32> = x_ticks.iter().filter_map(|t| x_axis.tick(t)).collect();
        let grid_ys: Vec<f32> = y_ticks.iter().filter_map(|t| y_axis.tick(t)).collect();
        rect.paint_vertical_gridlines(&bounds, &grid_xs, window);
        rect.paint_horizontal_gridlines(&bounds, &grid_ys, window);
        // The solid category (X) + value (Y) axis lines at the plot's bottom/left boundaries.
        rect.paint_axes(&bounds, window);

        // Numeric tick labels (same layout as scatter); the axis LINES are drawn above (bounded), so
        // `PlotAxis` only paints labels here.
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
            .x_axis(false)
            .x_label(x_labels)
            .y(px(plot_left))
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(hsla(AXIS_STROKE))
            .paint(&bounds, window, cx);

        // Marks: one sized, translucent, solid-edged disc per finite point.
        let origin = bounds.origin;
        for s in &self.series {
            let n = s.xs.len().min(s.ys.len());
            // Cloud paint cap (GAPS C-P25-1): bound the per-frame disc count for a large-range bubble.
            let keep = cap_markers_for_paint(n, MAX_PAINT_MARKERS);

            // Resolve each kept point's center + radius, dropping non-finite positions.
            let mut discs: Vec<(f32, f32, f32)> = keep
                .iter()
                .filter_map(|&i| {
                    let mx = s.xs[i]
                        .is_finite()
                        .then(|| x_axis.tick(&s.xs[i]))
                        .flatten()?;
                    let my = s.ys[i]
                        .is_finite()
                        .then(|| y_axis.tick(&s.ys[i]))
                        .flatten()?;
                    let size = s.sizes.get(i).copied().unwrap_or(f64::NAN);
                    let r = bubble_radius(size, self.max_size, self.representation);
                    Some((mx, my, r))
                })
                .collect();

            // Draw LARGEST-first so smaller bubbles land on top and stay visible.
            discs.sort_by(|a, b| b.2.total_cmp(&a.2));

            let fill = with_alpha(s.color, BUBBLE_FILL_ALPHA);
            let edge = s.color;
            for (cx_px, cy_px, r) in discs {
                let center = origin_point(px(cx_px), px(cy_px), origin);
                let radius = px(r);
                let top_left = gpui::point(center.x - radius, center.y - radius);
                // A quad with corner radius == radius is a circle; a translucent fill + a solid
                // series-colored edge (so overlaps read, and each bubble has a crisp outline).
                window.paint_quad(gpui::quad(
                    gpui::bounds(top_left, size(radius * 2.0, radius * 2.0)),
                    radius,
                    fill,
                    px(BUBBLE_EDGE_WIDTH),
                    edge,
                    BorderStyle::default(),
                ));
            }
        }
    }
}

/// Build the full bubble chart element (title, both axis titles, plot, legend). Returns `None` for a
/// chart this widget can't render.
pub fn bubble_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = BubblePlot::from_chart(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Color, Grouping, Legend, ScatterStyle, Series};

    fn two_series_bubble(representation: SizeRepresentation) -> Chart {
        Chart {
            title: Some("Bubbles".into()),
            kind: ChartKind::Bubble {
                size_representation: representation,
            },
            series: vec![
                Series::bubble(
                    Some("Group A"),
                    vec![1.0, 2.5, 4.0, 5.5],
                    vec![10.0, 22.0, 18.0, 31.0],
                    vec![4.0, 16.0, 9.0, 25.0],
                )
                .with_color(Color::from_hex(0x4472C4)),
                Series::bubble(
                    Some("Group B"),
                    vec![2.0, 3.0, 6.0, 7.5, 9.0],
                    vec![40.0, 55.0, 48.0, 62.0, 70.0],
                    vec![36.0, 12.0, 20.0, 8.0, 30.0],
                )
                .with_color(Color::from_hex(0xED7D31)),
            ],
            cat_axis: Axis::titled("X value"),
            val_axis: Axis::titled("Y value"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn shared_domains_cover_all_points() {
        let chart = two_series_bubble(SizeRepresentation::Area);
        let plot = BubblePlot::from_chart(&chart).expect("bubble plot");
        let (x, y) = (plot.x_domain(), plot.y_domain());
        for s in &chart.series {
            if let SeriesData::Xy { x: xs, y: ys, .. } = &s.data {
                for &vx in xs {
                    assert!(
                        x.min <= vx && vx <= x.max,
                        "x {vx} outside [{}, {}]",
                        x.min,
                        x.max
                    );
                }
                for &vy in ys {
                    assert!(
                        y.min <= vy && vy <= y.max,
                        "y {vy} outside [{}, {}]",
                        y.min,
                        y.max
                    );
                }
            }
        }
    }

    #[test]
    fn point_count_matches_data() {
        let plot = BubblePlot::from_chart(&two_series_bubble(SizeRepresentation::Area))
            .expect("bubble plot");
        // 4 points in Group A + 5 in Group B.
        assert_eq!(plot.point_count(), 9);
    }

    #[test]
    fn multi_series_has_distinct_colors_and_shared_max_size() {
        let plot = BubblePlot::from_chart(&two_series_bubble(SizeRepresentation::Area))
            .expect("bubble plot");
        assert_eq!(plot.series.len(), 2);
        assert_ne!(plot.series[0].color, plot.series[1].color);
        // The max size (36 in Group B) is shared across series (one normalizer).
        assert_eq!(plot.max_size, 36.0);
    }

    #[test]
    fn radius_clamps_and_encodes_area() {
        // Area: radius ∝ √size, clamped to [MIN, MAX]. The max size hits MAX_BUBBLE_RADIUS.
        let max = 100.0;
        assert_eq!(
            bubble_radius(100.0, max, SizeRepresentation::Area),
            MAX_BUBBLE_RADIUS,
            "the biggest bubble is the max radius"
        );
        // A quarter of the max size ⇒ half the radius (√(25/100) = 0.5), i.e. a quarter of the area.
        let quarter = bubble_radius(25.0, max, SizeRepresentation::Area);
        assert!(
            (quarter - MAX_BUBBLE_RADIUS * 0.5).abs() < 0.01,
            "√-area: 1/4 the size ⇒ 1/2 the radius, got {quarter}"
        );
        // A tiny size clamps up to MIN so it stays visible.
        assert_eq!(
            bubble_radius(0.0001, max, SizeRepresentation::Area),
            MIN_BUBBLE_RADIUS
        );
        // No usable sizes ⇒ the default radius.
        assert_eq!(
            bubble_radius(5.0, 0.0, SizeRepresentation::Area),
            DEFAULT_BUBBLE_RADIUS
        );
        // A bigger size never yields a smaller radius (monotonic).
        let small = bubble_radius(10.0, max, SizeRepresentation::Area);
        let big = bubble_radius(80.0, max, SizeRepresentation::Area);
        assert!(big >= small, "bigger size ⇒ ≥ radius: {big} vs {small}");
    }

    #[test]
    fn width_representation_maps_size_linearly() {
        // Width: radius ∝ size (not √). Half the max size ⇒ half the max radius.
        let half = bubble_radius(50.0, 100.0, SizeRepresentation::Width);
        assert!(
            (half - MAX_BUBBLE_RADIUS * 0.5).abs() < 0.01,
            "width: 1/2 the size ⇒ 1/2 the radius, got {half}"
        );
        // For the same non-extreme size, width gives a SMALLER radius than area (since s/max < √(s/max)).
        let w = bubble_radius(25.0, 100.0, SizeRepresentation::Width);
        let a = bubble_radius(25.0, 100.0, SizeRepresentation::Area);
        assert!(
            w < a,
            "width radius < area radius for a sub-max size: {w} vs {a}"
        );
    }

    #[test]
    fn rejects_non_bubble_and_empty() {
        // A scatter chart is not a bubble chart.
        let mut scatter = two_series_bubble(SizeRepresentation::Area);
        scatter.kind = ChartKind::Scatter {
            style: ScatterStyle::Marker,
        };
        assert!(BubblePlot::from_chart(&scatter).is_none());
        // A line chart is not a bubble chart.
        let mut line = two_series_bubble(SizeRepresentation::Area);
        line.kind = ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        };
        assert!(BubblePlot::from_chart(&line).is_none());
        // A bubble chart with no xy series has nothing to draw.
        let mut empty = two_series_bubble(SizeRepresentation::Area);
        empty.series.clear();
        assert!(BubblePlot::from_chart(&empty).is_none());
    }
}
