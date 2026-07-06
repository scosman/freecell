//! The chart **frame** FreeCell owns around a plot element: chart title, axis titles, and a
//! legend. The stock gpui-component chart structs have none of these
//! (`research/gpui-component-charts.md`), so we build them as a plain gpui `div` layout and
//! drop any plot (bar, line, …) into the plot slot.
//!
//! The legend is the load-bearing piece for multi-series: it lists **every** series with the
//! exact color that series' marks use (`series[i].color` or the palette cycle), so the
//! series→color mapping the §6 rubric checks is correct by construction — the plot and the
//! legend read the same source.

use gpui::{div, px, rgb, FontWeight, IntoElement, ParentElement, SharedString, Styled};

use chart_model::Chart;

use crate::palette::series_color;
use crate::style::{AXIS_TITLE_TEXT, BACKGROUND, TITLE_TEXT};

/// One legend row: a color swatch + the series name.
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

/// Build the legend: one row per series, each swatch colored exactly like that series' marks
/// (explicit `series.color`, else the palette cycle at the series index).
fn legend(chart: &Chart) -> gpui::AnyElement {
    let rows: Vec<gpui::AnyElement> = chart
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
        .collect();

    div()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .pl_2()
        .children(rows)
        .into_any_element()
}

/// Wrap a plot element in the full chart frame: chart title on top, the value-axis title as a
/// compact caption above the plot, the plot beside its legend, and the category-axis title
/// centered below. Shared by every chart kind so the chrome is identical across them.
pub fn chart_frame(chart: &Chart, plot: gpui::AnyElement) -> gpui::AnyElement {
    let title = chart.title.clone().unwrap_or_default();
    let value_axis_title = chart.val_axis.title.clone().unwrap_or_default();
    let category_axis_title = chart.cat_axis.title.clone().unwrap_or_default();

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
        // Category-axis title.
        .child(
            div().w_full().flex().justify_center().child(
                div()
                    .text_color(rgb(AXIS_TITLE_TEXT))
                    .text_size(px(11.))
                    .child(SharedString::from(category_axis_title)),
            ),
        )
        .into_any_element()
}
