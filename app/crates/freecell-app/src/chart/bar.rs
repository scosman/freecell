//! The FreeCell-owned **bar / column** widget family, built over gpui-component's `plot/`
//! **primitives** (`Bar` + `ScaleLinear` + `PlotAxis` + `Grid`) rather than the stock chart
//! structs — which do only single-series and expose no numeric value axis, no legend, no title
//! (`research/compare-bar-column.md`). One `BarPlot` renders every in-scope bar variant:
//!
//! - **Direction** ([`BarDir`]): vertical **columns** (`Col`) or horizontal **bars** (`Bar`,
//!   axes swapped — category on Y, value on X).
//! - **Grouping** ([`Grouping`]): **clustered** (side-by-side sub-bars per category),
//!   **stacked** (cumulative segments), and **100%-stacked** (segments normalized so each
//!   category fills 0–100%).
//!
//! There is **no grouped/stacked helper** in the primitives, so the layout is DIY: each
//! category owns a slot (`plot span / n_categories`); a clustered slot is sub-divided across
//! series, a stacked slot holds one column of cumulative segments (via [`super::stacking`]).
//! `ScaleBand` is deliberately **not** used for the slot geometry — its `band_width` is capped
//! at 30px (`plot/scale/band.rs`), too narrow for a grouped cluster — so slots are computed
//! manually for full control. The value axis is our own [`NiceScale`] (nice ticks the linear
//! scale ships none of), whose domain reflects the grouping: single values for clustered, the
//! per-category **sum** for stacked, a fixed 0–100 for percent.
//!
//! Colors are explicit (see [`super::style`]) so the headless capture is deterministic
//! regardless of the ambient light/dark theme; the shared title/axis-title/legend chrome lives
//! in [`super::chrome`].

use gpui::{
    px, Background, Bounds, Corners, Hsla, IntoElement, Pixels, SharedString, TextAlign, Window,
};
use gpui_component::plot::{
    scale::{Scale, ScaleLinear},
    shape::{Bar, BarAlignment},
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, AXIS_GAP,
};

use freecell_chart_model::{BarDir, Chart, ChartKind, Grouping, SeriesData};

use super::chrome::chart_frame;
use super::palette::series_color;
use super::stacking::{category_totals, percent_segments, stacked_segments, Segment};
use super::style::{hsla, model_hsla, AXIS_STROKE, GRID_STROKE, MUTED_TEXT};
use super::ticks::{format_tick, NiceScale};

/// Left gutter for the value-axis tick labels on a vertical column chart.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Left gutter for the category labels on a horizontal bar chart (roomier: category text runs
/// down the left edge instead of short numeric ticks).
const CATEGORY_AXIS_GUTTER: f32 = 64.0;
/// Reserved space at the top of the plot so the tallest bar/segment isn't clipped.
const PLOT_TOP_GAP: f32 = 12.0;
/// Reserved space at the right of the plot.
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Fraction of a category slot the bars/cluster occupy (the rest is the inter-category gap).
const GROUP_FILL: f32 = 0.7;
/// Fraction of a clustered sub-slot the bar itself occupies (the rest is the inter-bar gap).
const SUB_BAR_FILL: f32 = 0.86;
/// Roughly how many value-axis ticks to aim for.
const TARGET_TICKS: usize = 5;

/// One series ready to draw: its per-category values and resolved color.
#[derive(Clone)]
struct BarSeries {
    values: Vec<f64>,
    color: Hsla,
}

/// A bar/column plot over the raw `Bar` primitive, covering every in-scope direction ×
/// grouping combination on a numeric value axis we own via [`NiceScale`].
#[derive(IntoPlot)]
pub struct BarPlot {
    categories: Vec<SharedString>,
    series: Vec<BarSeries>,
    dir: BarDir,
    grouping: Grouping,
    /// The value domain: single-value span (clustered), stacked-total span (stacked), or the
    /// fixed 0–100 percent span.
    scale: NiceScale,
    /// Whether the value axis is a percentage (labels get a `%`).
    percent: bool,
}

impl BarPlot {
    /// Build from any [`ChartKind::Bar`] chart. Categories come from the first series; every
    /// series contributes its values. Returns `None` for a non-bar chart or one with no
    /// category/value data.
    pub fn from_chart(chart: &Chart) -> Option<Self> {
        let ChartKind::Bar { dir, grouping } = chart.kind else {
            return None;
        };

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
            series.push(BarSeries {
                values: values.clone(),
                color: model_hsla(s.color.unwrap_or_else(|| series_color(i))),
            });
        }

        let categories = categories?;
        if series.is_empty() || categories.is_empty() {
            return None;
        }

        let n = categories.len();
        let all_values: Vec<Vec<f64>> = series.iter().map(|s| s.values.clone()).collect();
        let percent = matches!(grouping, Grouping::PercentStacked);
        let scale = match grouping {
            // Clustered: every bar is independent, so cover the largest single value (zero-based).
            Grouping::Clustered | Grouping::Standard => NiceScale::for_values(
                series.iter().flat_map(|s| s.values.iter().copied()),
                TARGET_TICKS,
            ),
            // Stacked: the axis must reach the tallest column total.
            Grouping::Stacked => {
                NiceScale::for_values(category_totals(&all_values, n), TARGET_TICKS)
            }
            // Percent: always 0–100.
            Grouping::PercentStacked => NiceScale::new(0.0, 100.0, TARGET_TICKS),
        };

        Some(Self {
            categories,
            series,
            dir,
            grouping,
            scale,
            percent,
        })
    }

    /// The per-category cumulative segments for a stacked / percent chart (`None` for
    /// clustered, which draws independent zero-based bars). Exposed for tests.
    fn segments(&self) -> Option<Vec<Vec<Segment>>> {
        let all_values: Vec<Vec<f64>> = self.series.iter().map(|s| s.values.clone()).collect();
        let n = self.categories.len();
        match self.grouping {
            Grouping::Stacked => Some(stacked_segments(&all_values, n)),
            Grouping::PercentStacked => Some(percent_segments(&all_values, n)),
            _ => None,
        }
    }
}

/// A value-axis → pixel map plus the category-slot geometry, resolved for one orientation.
struct Geometry {
    /// Maps a value to a pixel along the value axis.
    value_scale: ScaleLinear<f64>,
    /// Pixel of value 0 along the value axis (the bar baseline).
    baseline_px: f32,
    /// Center pixel of each category along the category axis.
    centers: Vec<f32>,
    /// Width of one category slot along the category axis.
    slot: f32,
}

impl Geometry {
    fn category_center(span_start: f32, slot: f32, i: usize) -> f32 {
        span_start + slot * (i as f32 + 0.5)
    }
}

impl Plot for BarPlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();
        let horizontal = matches!(self.dir, BarDir::Bar);

        // Plot rect. Horizontal bars put the category labels down the (roomier) left gutter and
        // the value ticks along the bottom; vertical columns do the opposite.
        let plot_left = if horizontal {
            CATEGORY_AXIS_GUTTER
        } else {
            VALUE_AXIS_GUTTER
        };
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        let n = self.categories.len().max(1);
        let ticks = self.scale.ticks();

        let geo = if horizontal {
            // Value axis runs left→right; categories stack top→bottom.
            let value_scale = ScaleLinear::new(
                vec![self.scale.min, self.scale.max],
                vec![plot_left, plot_right],
            );
            let baseline_px = value_scale.tick(&0.0).unwrap_or(plot_left);
            let slot = (plot_bottom - plot_top) / n as f32;
            let centers = (0..n)
                .map(|i| Geometry::category_center(plot_top, slot, i))
                .collect();
            Geometry {
                value_scale,
                baseline_px,
                centers,
                slot,
            }
        } else {
            // Value axis runs bottom→top (inverted); categories run left→right.
            let value_scale = ScaleLinear::new(
                vec![self.scale.min, self.scale.max],
                vec![plot_bottom, plot_top],
            );
            let baseline_px = value_scale.tick(&0.0).unwrap_or(plot_bottom);
            let slot = (plot_right - plot_left) / n as f32;
            let centers = (0..n)
                .map(|i| Geometry::category_center(plot_left, slot, i))
                .collect();
            Geometry {
                value_scale,
                baseline_px,
                centers,
                slot,
            }
        };

        // Gridlines + axes + labels (orientation-aware).
        self.paint_chrome(
            &bounds,
            window,
            cx,
            horizontal,
            plot_left,
            plot_bottom,
            &geo,
            &ticks,
        );

        // Bars. `cross` = category-axis position, `value` = value-axis position; the alignment
        // tells the primitive which screen axis each maps to.
        let alignment = if horizontal {
            BarAlignment::Left
        } else {
            BarAlignment::Bottom
        };
        match self.grouping {
            Grouping::Stacked | Grouping::PercentStacked => {
                self.paint_stacked(&bounds, window, cx, alignment, &geo)
            }
            _ => self.paint_clustered(&bounds, window, cx, alignment, &geo),
        }
    }
}

impl BarPlot {
    #[allow(clippy::too_many_arguments)]
    fn paint_chrome(
        &self,
        bounds: &Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::App,
        horizontal: bool,
        plot_left: f32,
        plot_bottom: f32,
        geo: &Geometry,
        ticks: &[f64],
    ) {
        let value_pixels: Vec<f32> = ticks
            .iter()
            .filter_map(|t| geo.value_scale.tick(t))
            .collect();
        let value_labels = ticks.iter().zip(&value_pixels).map(|(t, p)| {
            let text = if self.percent {
                format!("{}%", format_tick(*t))
            } else {
                format_tick(*t)
            };
            (text, *p)
        });

        if horizontal {
            // Value axis on the bottom (vertical gridlines); category axis on the left.
            Grid::new()
                .stroke(hsla(GRID_STROKE))
                .dash_array(&[px(4.), px(2.)])
                .x(value_pixels.iter().map(|x| px(*x)).collect())
                .paint(bounds, window);
            let value_axis_labels = value_labels.map(|(text, x)| {
                AxisText::new(text, px(x), hsla(MUTED_TEXT)).align(TextAlign::Center)
            });
            let cat_labels = self.categories.iter().enumerate().map(|(i, c)| {
                AxisText::new(c.clone(), px(geo.centers[i]), hsla(MUTED_TEXT))
                    .align(TextAlign::Right)
            });
            PlotAxis::new()
                .x(px(plot_bottom))
                .x_label(value_axis_labels)
                .y(px(plot_left))
                .y_label_side(AxisLabelSide::Start)
                .y_label(cat_labels)
                .stroke(hsla(AXIS_STROKE))
                .paint(bounds, window, cx);
        } else {
            // Value axis on the left (horizontal gridlines); category axis on the bottom.
            Grid::new()
                .stroke(hsla(GRID_STROKE))
                .dash_array(&[px(4.), px(2.)])
                .y(value_pixels.iter().map(|y| px(*y)).collect())
                .paint(bounds, window);
            let value_axis_labels = value_labels.map(|(text, y)| {
                AxisText::new(text, px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
            });
            let cat_labels = self.categories.iter().enumerate().map(|(i, c)| {
                AxisText::new(c.clone(), px(geo.centers[i]), hsla(MUTED_TEXT))
                    .align(TextAlign::Center)
            });
            PlotAxis::new()
                .x(px(plot_bottom))
                .x_label(cat_labels)
                .y(px(plot_left))
                .y_label_side(AxisLabelSide::Start)
                .y_label(value_axis_labels)
                .stroke(hsla(AXIS_STROKE))
                .paint(bounds, window, cx);
        }
    }

    /// Clustered / single-series: independent zero-based bars, series side-by-side within the
    /// category slot. The same `cross`/`value` closures serve both orientations — the `Bar`
    /// primitive reads `cross` as the category-axis position and `value` as the value-axis
    /// position for whichever [`BarAlignment`] it is given, and `geo` already maps each axis
    /// to the correct screen direction.
    fn paint_clustered(
        &self,
        bounds: &Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::App,
        alignment: BarAlignment,
        geo: &Geometry,
    ) {
        let n_series = self.series.len().max(1);
        let group_width = geo.slot * GROUP_FILL;
        let sub_w = group_width / n_series as f32;
        let bar_w = sub_w * SUB_BAR_FILL;
        let baseline = geo.baseline_px;

        for (j, s) in self.series.iter().enumerate() {
            let centers = geo.centers.clone();
            let values = s.values.clone();
            let value_scale = geo.value_scale.clone();
            let bar_bg: Background = s.color.into();
            let n = values.len().min(centers.len());
            // Center this series' sub-bar within its sub-slot of the category group.
            let offset =
                -group_width / 2.0 + sub_w * j as f32 + (sub_w - bar_w) / 2.0 + bar_w / 2.0;

            Bar::new()
                .data((0..n).collect::<Vec<usize>>())
                .alignment(alignment)
                .band_width(bar_w)
                .base(move |_| baseline)
                // `cross` is the bar's near edge, so subtract half the bar width from the center.
                .cross(move |i: &usize| Some(centers[*i] + offset - bar_w / 2.0))
                .value(move |i: &usize| value_scale.tick(&values[*i]))
                .fill(move |_, _, _| bar_bg)
                .corner_radii(Corners::all(px(2.)))
                .paint(bounds, window, cx);
        }
    }

    /// Stacked / percent: one column per category, each series a cumulative segment drawn from
    /// its lower to its upper cumulative bound.
    fn paint_stacked(
        &self,
        bounds: &Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::App,
        alignment: BarAlignment,
        geo: &Geometry,
    ) {
        let Some(segments) = self.segments() else {
            return;
        };
        let group_width = geo.slot * GROUP_FILL;
        let offset = -group_width / 2.0;

        for (s, row) in segments.iter().enumerate() {
            let centers = geo.centers.clone();
            let value_scale = geo.value_scale.clone();
            let seg_row = row.clone();
            let seg_lo = row.clone();
            let vs_lo = value_scale.clone();
            let bar_bg: Background = self.series[s].color.into();
            let n = seg_row.len().min(centers.len());

            Bar::new()
                .data((0..n).collect::<Vec<usize>>())
                .alignment(alignment)
                .band_width(group_width)
                .cross(move |i: &usize| Some(centers[*i] + offset))
                .base(move |i: &usize| vs_lo.tick(&seg_lo[*i].lo).unwrap_or(0.0))
                .value(move |i: &usize| value_scale.tick(&seg_row[*i].hi))
                .fill(move |_, _, _| bar_bg)
                .paint(bounds, window, cx);
        }
    }
}

/// Build the full bar/column chart element (title, axis titles, plot, legend). Returns `None`
/// for a chart this widget can't render.
pub fn bar_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = BarPlot::from_chart(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Category, Legend, Series};

    fn cats() -> Vec<Category> {
        vec![
            Category::Text("Q1".into()),
            Category::Text("Q2".into()),
            Category::Text("Q3".into()),
        ]
    }

    fn bar_chart(dir: BarDir, grouping: Grouping) -> Chart {
        Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Bar { dir, grouping },
            series: vec![
                Series::category_value(Some("A"), cats(), vec![10.0, 20.0, 30.0]),
                Series::category_value(Some("B"), cats(), vec![15.0, 25.0, 35.0]),
                Series::category_value(Some("C"), cats(), vec![5.0, 10.0, 15.0]),
            ],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("Units"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn grouped_offsets_partition_the_band() {
        // The clustered sub-bars for a category must be disjoint and lie within the group.
        let plot = BarPlot::from_chart(&bar_chart(BarDir::Col, Grouping::Clustered)).unwrap();
        let n_series = plot.series.len();
        let slot = 120.0_f32;
        let group_width = slot * GROUP_FILL;
        let sub_w = group_width / n_series as f32;
        let bar_w = sub_w * SUB_BAR_FILL;

        let mut spans = Vec::new();
        for j in 0..n_series {
            let offset = -group_width / 2.0 + sub_w * j as f32 + (sub_w - bar_w) / 2.0;
            spans.push((offset, offset + bar_w));
        }
        // Each bar sits within the group half-widths.
        for (lo, hi) in &spans {
            assert!(*lo >= -group_width / 2.0 - 1e-3, "bar spills left of group");
            assert!(*hi <= group_width / 2.0 + 1e-3, "bar spills right of group");
        }
        // Bars don't overlap (next bar starts after the previous ends).
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0 + 1e-3, "clustered bars overlap");
        }
    }

    #[test]
    fn stacked_baselines_are_cumulative() {
        let plot = BarPlot::from_chart(&bar_chart(BarDir::Col, Grouping::Stacked)).unwrap();
        let segs = plot.segments().expect("stacked chart has segments");
        for c in 0..3 {
            for s in 0..segs.len() - 1 {
                assert_eq!(segs[s][c].hi, segs[s + 1][c].lo);
            }
        }
        // The value axis reaches the tallest column total.
        let totals = category_totals(
            &plot
                .series
                .iter()
                .map(|s| s.values.clone())
                .collect::<Vec<_>>(),
            3,
        );
        let max_total = totals.iter().cloned().fold(0.0_f64, f64::max);
        assert!(
            plot.scale.max >= max_total,
            "axis must cover the stack total"
        );
    }

    #[test]
    fn percent_stacks_sum_to_100_and_axis_is_0_100() {
        let plot = BarPlot::from_chart(&bar_chart(BarDir::Col, Grouping::PercentStacked)).unwrap();
        assert_eq!(plot.scale.min, 0.0);
        assert_eq!(plot.scale.max, 100.0);
        assert!(plot.percent);
        let segs = plot.segments().unwrap();
        for c in 0..3 {
            let sum: f64 = segs.iter().map(|row| row[c].height()).sum();
            assert!((sum - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn clustered_value_domain_covers_max_single_value() {
        let plot = BarPlot::from_chart(&bar_chart(BarDir::Col, Grouping::Clustered)).unwrap();
        assert_eq!(plot.scale.min, 0.0, "columns are zero-based");
        assert!(
            plot.scale.max >= 35.0,
            "must cover the largest single value (35)"
        );
        // Not inflated to the stack total (90) — clustered bars are independent.
        assert!(plot.scale.max < 90.0);
    }

    #[test]
    fn horizontal_direction_is_carried_through() {
        let plot = BarPlot::from_chart(&bar_chart(BarDir::Bar, Grouping::Clustered)).unwrap();
        assert_eq!(plot.dir, BarDir::Bar);
        assert_eq!(plot.series.len(), 3);
        assert_eq!(plot.categories.len(), 3);
    }

    #[test]
    fn rejects_non_bar() {
        let mut line = bar_chart(BarDir::Col, Grouping::Clustered);
        line.kind = ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        };
        assert!(BarPlot::from_chart(&line).is_none());
    }
}
