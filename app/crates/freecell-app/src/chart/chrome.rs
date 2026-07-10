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

use gpui::prelude::FluentBuilder as _;
use gpui::{div, px, rgb, FontWeight, IntoElement, ParentElement, SharedString, Styled};

use freecell_chart_model::{BarDir, Chart, ChartKind, SeriesData};

use super::palette::{series_color, slice_color};
use super::style::{AXIS_TITLE_TEXT, BACKGROUND, TITLE_TEXT};

/// One legend key: the swatch color (packed `0xRRGGBB`) and the label it sits beside. This is the
/// load-bearing series↔swatch mapping (module docs) — a plain, gpui-free struct so the mapping is
/// unit-tested without a GPU, then rendered by [`legend_row`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegendEntry {
    pub color: u32,
    pub name: String,
}

/// The legend entries: one per slice (the categories of the first series) for a pie/doughnut, else
/// one per series. Each entry's color is the *same* palette function the marks use, so the
/// legend↔mark colors match by construction. Pure (no gpui) so it is unit-tested directly.
pub(crate) fn legend_entries(chart: &Chart) -> Vec<LegendEntry> {
    if matches!(chart.kind, ChartKind::Pie { .. }) {
        if let Some(SeriesData::CategoryValue { categories, .. }) =
            chart.series.first().map(|s| &s.data)
        {
            return categories
                .iter()
                .enumerate()
                .map(|(i, c)| LegendEntry {
                    color: slice_color(i).to_hex(),
                    name: c.label(),
                })
                .collect();
        }
    }
    chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| LegendEntry {
            color: s.color.unwrap_or_else(|| series_color(i)).to_hex(),
            name: s
                .name
                .clone()
                .unwrap_or_else(|| format!("Series {}", i + 1)),
        })
        .collect()
}

/// One legend row: a color swatch + the series/slice name.
fn legend_row(entry: LegendEntry) -> gpui::AnyElement {
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
                .bg(rgb(entry.color)),
        )
        .child(
            div()
                .text_color(rgb(TITLE_TEXT))
                .text_size(px(11.))
                .child(SharedString::from(entry.name)),
        )
        .into_any_element()
}

/// Build the legend column (one row per [`legend_entries`] key).
fn legend(chart: &Chart) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .pl_2()
        .children(legend_entries(chart).into_iter().map(legend_row))
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
///
/// The chrome is **driven by the model, not always-on**: the title row, each axis-title caption,
/// and the legend column are each rendered only when the model carries them (a non-empty title /
/// caption, `chart.legend.is_some()`). An untitled, legend-less chart is just its plot — no blank
/// rows, no stray legend column.
pub fn chart_frame(chart: &Chart, plot: gpui::AnyElement) -> gpui::AnyElement {
    let title = chart.title.clone().unwrap_or_default();
    let (top_caption, bottom_caption) = captions(chart);
    let has_legend = chart.legend.is_some();

    let body = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        // Top axis-title caption (compact caption above the plot) — only when there is one.
        .when(!top_caption.is_empty(), |body| {
            body.child(
                div().pl(px(6.)).child(
                    div()
                        .text_color(rgb(AXIS_TITLE_TEXT))
                        .text_size(px(11.))
                        .child(SharedString::from(top_caption)),
                ),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_row()
                .child(div().flex_1().min_w(px(0.)).child(plot))
                // The legend column only when the model has a legend; otherwise the plot fills
                // the full width.
                .when(has_legend, |row| row.child(legend(chart))),
        );

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(BACKGROUND))
        .p_3()
        .gap_1()
        // Chart title — only when there is one.
        .when(!title.is_empty(), |frame| {
            frame.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(TITLE_TEXT))
                        .text_size(px(16.))
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(title)),
                ),
            )
        })
        .child(body)
        // Bottom axis-title caption — only when there is one.
        .when(!bottom_caption.is_empty(), |frame| {
            frame.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(AXIS_TITLE_TEXT))
                        .text_size(px(11.))
                        .child(SharedString::from(bottom_caption)),
                ),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::super::palette::{series_color, slice_color};
    use super::*;
    use freecell_chart_model::{Axis, Category, Color, Grouping, Legend, Series};

    fn months() -> Vec<Category> {
        ["Jan", "Feb", "Mar"]
            .into_iter()
            .map(|m| Category::Text(m.into()))
            .collect()
    }

    fn multi_series_line() -> Chart {
        Chart {
            title: Some("Regional Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![
                Series::category_value(Some("North"), months(), vec![1.0, 2.0, 3.0]),
                Series::category_value(Some("South"), months(), vec![3.0, 2.0, 1.0]),
                Series::category_value(Some("West"), months(), vec![2.0, 2.0, 2.0]),
            ],
            cat_axis: Axis::titled("Month"),
            val_axis: Axis::titled("Units"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn legend_entries_map_multi_series_line() {
        let chart = multi_series_line();
        let entries = legend_entries(&chart);
        // One entry per series, in order, named by the series.
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["North", "South", "West"]
        );
        // Each swatch is the palette color the mark uses — matched by construction, and distinct.
        assert_eq!(entries[0].color, series_color(0).to_hex());
        assert_eq!(entries[1].color, series_color(1).to_hex());
        assert_eq!(entries[2].color, series_color(2).to_hex());
        assert_ne!(entries[0].color, entries[1].color);
        assert_ne!(entries[1].color, entries[2].color);
    }

    #[test]
    fn legend_entry_honors_explicit_series_color() {
        let mut chart = multi_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_color(Color::from_hex(0x123456));
        let entries = legend_entries(&chart);
        assert_eq!(
            entries[0].color, 0x123456,
            "an explicit series color must win over the palette"
        );
    }

    #[test]
    fn legend_entry_names_unnamed_series() {
        let mut chart = multi_series_line();
        chart.series[1].name = None;
        let entries = legend_entries(&chart);
        assert_eq!(
            entries[1].name, "Series 2",
            "an unnamed series falls back to its 1-based position"
        );
    }

    #[test]
    fn legend_entries_are_per_slice_for_pie() {
        let chart = Chart {
            title: Some("Market Share".into()),
            kind: ChartKind::Pie {
                doughnut_hole: None,
            },
            series: vec![Series::category_value(
                Some("Share"),
                vec![
                    Category::Text("Alpha".into()),
                    Category::Text("Beta".into()),
                    Category::Text("Gamma".into()),
                ],
                vec![50.0, 30.0, 20.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        let entries = legend_entries(&chart);
        // A pie is single-series: the legend keys off the CATEGORIES (slices), one per slice,
        // colored by the slice palette — not one entry for the single series.
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Beta", "Gamma"]
        );
        assert_eq!(entries[0].color, slice_color(0).to_hex());
        assert_eq!(entries[2].color, slice_color(2).to_hex());
    }

    #[test]
    fn captions_swap_for_horizontal_bar() {
        // A normal (non-horizontal) chart: value caption on top, category caption below.
        let mut chart = multi_series_line();
        assert_eq!(captions(&chart), ("Units".to_string(), "Month".to_string()));

        // A horizontal bar swaps them (value axis is along the bottom, category down the left).
        chart.kind = ChartKind::Bar {
            dir: BarDir::Bar,
            grouping: Grouping::Clustered,
        };
        assert_eq!(captions(&chart), ("Month".to_string(), "Units".to_string()));

        // Untitled axes yield empty captions (so the frame collapses those rows).
        chart.cat_axis = Axis::untitled();
        chart.val_axis = Axis::untitled();
        assert_eq!(captions(&chart), (String::new(), String::new()));
    }
}
