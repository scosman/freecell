//! The FreeCell-owned chart widget, built over gpui-component's `plot/` **primitives**
//! (`Bar` + `ScaleBand` + `ScaleLinear` + `PlotAxis` + `Grid`) rather than the stock chart
//! structs — the approach the make-or-break Gate 1 needs (`research/gpui-component-charts.md`:
//! the structs have no numeric value axis, no legend, no title).
//!
//! Phase 0 renders one trivial single-series column chart. The pieces FreeCell owns and this
//! module proves it can build:
//! - a **numeric value axis** with readable "nice" tick labels + gridlines, using our own
//!   [`crate::ticks::NiceScale`] as the value domain so the bars and the ticks share one
//!   scale (the stock charts normalize their own domain and expose no nice ticks);
//! - a **chart title** + **axis titles**;
//! - a **legend** (swatch + series name) driven by our palette.
//!
//! Colors are chosen explicitly (not from the gpui-component theme) so the headless capture
//! is deterministic and high-contrast regardless of the ambient light/dark theme.

use gpui::{
    div, px, rgb, Background, Bounds, Corners, FontWeight, Hsla, IntoElement, ParentElement,
    Pixels, SharedString, Styled, TextAlign, Window,
};
use gpui_component::plot::{
    scale::{Scale, ScaleBand, ScaleLinear},
    shape::{Bar, BarAlignment},
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, AXIS_GAP,
};

use chart_model::{BarDir, Chart, ChartKind, Color as ModelColor, SeriesData};

use crate::palette::series_color;
use crate::ticks::{format_tick, NiceScale};

// Explicit, theme-independent colors (see module docs).
const BACKGROUND: u32 = 0xFFFFFF;
const TITLE_TEXT: u32 = 0x1A1A1A;
const AXIS_TITLE_TEXT: u32 = 0x374151;
const MUTED_TEXT: u32 = 0x6B7280;
const AXIS_STROKE: u32 = 0x9CA3AF;
const GRID_STROKE: u32 = 0xE5E7EB;

/// Pixels reserved at the left of the plot for value-axis tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top of the plot so the tallest bar/label isn't clipped.
const PLOT_TOP_GAP: f32 = 12.0;
/// Pixels reserved at the right of the plot.
const PLOT_RIGHT_GAP: f32 = 12.0;
/// Roughly how many value-axis ticks to aim for.
const TARGET_TICKS: usize = 5;

fn hsla(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

/// Convert a model color to a gpui `Hsla`.
fn model_hsla(color: ModelColor) -> Hsla {
    Hsla::from(rgb(color.to_hex()))
}

/// A single-series vertical bar (column) plot over the raw `Bar` primitive, with a numeric
/// value axis we control via [`NiceScale`].
#[derive(IntoPlot)]
pub struct BarPlot {
    categories: Vec<SharedString>,
    values: Vec<f64>,
    scale: NiceScale,
    bar_color: Hsla,
}

impl BarPlot {
    /// Build the plot from a single category/value series. Returns `None` if the chart is not
    /// a single-series column chart (Phase 0 scope; later phases add the other kinds).
    pub fn single_series(chart: &Chart) -> Option<Self> {
        if !matches!(
            chart.kind,
            ChartKind::Bar {
                dir: BarDir::Col,
                ..
            }
        ) {
            return None;
        }
        let series = chart.series.first()?;
        let SeriesData::CategoryValue { categories, values } = &series.data else {
            return None;
        };
        let scale = NiceScale::for_values(values.iter().copied(), TARGET_TICKS);
        let bar_color = model_hsla(series.color.unwrap_or_else(|| series_color(0)));
        Some(Self {
            categories: categories.iter().map(|c| c.label().into()).collect(),
            values: values.clone(),
            scale,
            bar_color,
        })
    }
}

impl Plot for BarPlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        // Category (band) axis across the plot width. `ScaleBand::tick` positions from 0
        // (it ignores the range's start offset), so we give it the available *width* and add
        // `plot_left` ourselves to every band position — otherwise the bars slide left into
        // the value-axis gutter and paint over the tick labels there.
        let band = ScaleBand::new(
            self.categories.clone(),
            vec![0.0, (plot_right - plot_left).max(1.0)],
        )
        .padding_inner(0.4)
        .padding_outer(0.2);
        let band_width = band.band_width();

        // Value axis: OUR nice domain -> pixel range (inverted: min at the bottom). Sharing
        // this scale between the ticks and the bars is what keeps them aligned.
        let value_scale = ScaleLinear::new(
            vec![self.scale.min, self.scale.max],
            vec![plot_bottom, plot_top],
        );

        let ticks = self.scale.ticks();

        // Gridlines at each nice tick (horizontal lines for vertical bars).
        let grid_ys: Vec<f32> = ticks.iter().filter_map(|t| value_scale.tick(t)).collect();
        Grid::new()
            .stroke(hsla(GRID_STROKE))
            .dash_array(&[px(4.), px(2.)])
            .y(grid_ys)
            .paint(&bounds, window);

        // Axes + labels: value labels left of the value-axis line, category labels below the
        // baseline.
        let value_labels = ticks.iter().filter_map(|t| {
            value_scale.tick(t).map(|y| {
                AxisText::new(format_tick(*t), px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
            })
        });
        let cat_labels = self.categories.iter().filter_map(|c| {
            band.tick(c).map(|x| {
                AxisText::new(
                    c.clone(),
                    px(plot_left + x + band_width / 2.),
                    hsla(MUTED_TEXT),
                )
                .align(TextAlign::Center)
            })
        });
        PlotAxis::new()
            .x(px(plot_bottom))
            .x_label(cat_labels)
            .y(px(plot_left))
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(hsla(AXIS_STROKE))
            .paint(&bounds, window, cx);

        // Bars. Baseline is the pixel row of value 0 (the bottom for all-positive data).
        let baseline_px = value_scale.tick(&0.0).unwrap_or(plot_bottom);
        let categories = self.categories.clone();
        let values = self.values.clone();
        let band_for_cross = band.clone();
        let value_for_val = value_scale.clone();
        let bar_bg: Background = self.bar_color.into();
        let n = self.values.len();

        Bar::new()
            .data((0..n).collect::<Vec<usize>>())
            .alignment(BarAlignment::Bottom)
            .band_width(band_width)
            .cross(move |i: &usize| band_for_cross.tick(&categories[*i]).map(|x| plot_left + x))
            .base(move |_| baseline_px)
            .value(move |i: &usize| value_for_val.tick(&values[*i]))
            .fill(move |_, _, _| bar_bg)
            .corner_radii(Corners::all(px(2.)))
            .paint(&bounds, window, cx);
    }
}

/// Build the full chart element for a [`Chart`]: title, axis titles, the plot, and a legend.
/// Returns `None` for a chart this phase can't render.
pub fn chart_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = BarPlot::single_series(chart)?;

    let title = chart.title.clone().unwrap_or_default();
    let value_axis_title = chart.val_axis.title.clone().unwrap_or_default();
    let category_axis_title = chart.cat_axis.title.clone().unwrap_or_default();

    // Legend swatches follow the same series colors the bars use.
    //
    // KNOWN single-series limitation (Phase 0): `BarPlot::single_series` draws only
    // `series[0]`, but this legend lists ALL series. With one series (Phase 0 scope) they
    // agree. When the widget grows to multi-series in Phase 1 (grouped/stacked bars,
    // multi-line), the plot must draw every series so the legend and the marks stay in sync —
    // resolve this together with that work, not by trimming the legend to `series[0]` here.
    let legend_items: Vec<gpui::AnyElement> = chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let color = s.color.unwrap_or_else(|| series_color(i));
            let name = s
                .name
                .clone()
                .unwrap_or_else(|| format!("Series {}", i + 1));
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .w(px(11.))
                        .h(px(11.))
                        .rounded(px(2.))
                        .bg(rgb(color.to_hex())),
                )
                .child(
                    div()
                        .text_color(rgb(TITLE_TEXT))
                        .text_size(px(11.))
                        .child(SharedString::from(name)),
                )
                .into_any_element()
        })
        .collect();

    let legend = div()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .pl_2()
        .children(legend_items);

    let body = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        // Value-axis title (compact caption above the axis gutter).
        .child(
            div().pl(px(6.)).child(
                div()
                    .text_color(rgb(AXIS_TITLE_TEXT))
                    .text_size(px(11.))
                    .child(SharedString::from(value_axis_title)),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_row()
                .child(div().flex_1().min_w(px(0.)).child(plot))
                .child(legend),
        );

    Some(
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BACKGROUND))
            .p_3()
            .gap_1()
            // Chart title.
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(TITLE_TEXT))
                        .text_size(px(16.))
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(title)),
                ),
            )
            .child(body)
            // Category-axis title.
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(AXIS_TITLE_TEXT))
                        .text_size(px(11.))
                        .child(SharedString::from(category_axis_title)),
                ),
            )
            .into_any_element(),
    )
}
