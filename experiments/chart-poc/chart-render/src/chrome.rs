//! The chart **frame** FreeCell owns around a plot element: chart title, axis titles, and a
//! legend. The stock gpui-component chart structs have none of these
//! (`research/gpui-component-charts.md`), so we build them as a plain gpui `div` layout and
//! drop any plot (bar, line, area, pie, …) into the plot slot.
//!
//! The legend is the load-bearing piece for a multi-series (or multi-slice) chart: it lists
//! every series/slice with the exact color that mark uses, so the series→color mapping the §6
//! rubric checks is correct by construction — the plot and the legend read the same palette.
//! For a **pie/doughnut** the "series" are the *categories* of the single series, so the legend
//! keys off the categories + [`slice_color`]; for everything else it keys off the series +
//! [`series_color`].

use gpui::{div, px, rgb, FontWeight, IntoElement, ParentElement, SharedString, Styled};

use chart_model::{BarDir, Chart, ChartKind, SeriesData};

use crate::palette::{series_color, slice_color};
use crate::style::{AXIS_TITLE_TEXT, BACKGROUND, TITLE_TEXT};

/// One legend row: a color swatch + the series/slice name.
fn legend_row(color: u32, name: String) -> gpui::AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(div().w(px(11.)).h(px(11.)).rounded(px(2.)).bg(rgb(color)))
        .child(
            div()
                .text_color(rgb(TITLE_TEXT))
                .text_size(px(11.))
                .child(SharedString::from(name)),
        )
        .into_any_element()
}

/// The legend rows: one per slice (categories of the first series) for a pie/doughnut, else one
/// per series. Each swatch is colored by the same palette function the marks use.
fn legend_rows(chart: &Chart) -> Vec<gpui::AnyElement> {
    if matches!(chart.kind, ChartKind::Pie { .. }) {
        if let Some(SeriesData::CategoryValue { categories, .. }) =
            chart.series.first().map(|s| &s.data)
        {
            return categories
                .iter()
                .enumerate()
                .map(|(i, c)| legend_row(slice_color(i).to_hex(), c.label()))
                .collect();
        }
    }
    chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let color = s.color.unwrap_or_else(|| series_color(i)).to_hex();
            let name = s
                .name
                .clone()
                .unwrap_or_else(|| format!("Series {}", i + 1));
            legend_row(color, name)
        })
        .collect()
}

/// Build the legend column.
fn legend(chart: &Chart) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .pl_2()
        .children(legend_rows(chart))
        .into_any_element()
}

/// The two axis-title captions (above the plot, below the plot). For a **horizontal** bar chart
/// the value axis is at the bottom and the category axis on the left, so the captions swap:
/// value title goes below (under the bottom value axis), category title above. Every other kind
/// keeps value-title-above / category-title-below.
fn captions(chart: &Chart) -> (String, String) {
    let value = chart.val_axis.title.clone().unwrap_or_default();
    let category = chart.cat_axis.title.clone().unwrap_or_default();
    if matches!(
        chart.kind,
        ChartKind::Bar {
            dir: BarDir::Bar,
            ..
        }
    ) {
        (category, value)
    } else {
        (value, category)
    }
}

/// Wrap a plot element in the full chart frame: chart title on top, one axis-title caption above
/// the plot, the plot beside its legend, and the other axis-title caption centered below. Shared
/// by every chart kind so the chrome is identical across them.
pub fn chart_frame(chart: &Chart, plot: gpui::AnyElement) -> gpui::AnyElement {
    let title = chart.title.clone().unwrap_or_default();
    let (top_caption, bottom_caption) = captions(chart);

    let body = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        // Top axis-title caption (compact caption above the plot).
        .child(
            div().pl(px(6.)).child(
                div()
                    .text_color(rgb(AXIS_TITLE_TEXT))
                    .text_size(px(11.))
                    .child(SharedString::from(top_caption)),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_row()
                .child(div().flex_1().min_w(px(0.)).child(plot))
                .child(legend(chart)),
        );

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
        // Bottom axis-title caption.
        .child(
            div().w_full().flex().justify_center().child(
                div()
                    .text_color(rgb(AXIS_TITLE_TEXT))
                    .text_size(px(11.))
                    .child(SharedString::from(bottom_caption)),
            ),
        )
        .into_any_element()
}
