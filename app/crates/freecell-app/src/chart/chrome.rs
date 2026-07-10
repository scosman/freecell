//! The chart **frame** FreeCell owns around a plot element: chart title, axis titles, and a
//! legend. The stock gpui-component chart structs have none of these
//! (`research/gpui-component-charts.md`), so we build them as a plain gpui `div` layout and
//! drop any plot (bar, line, area, pie, …) into the plot slot.
//!
//! The legend is the load-bearing piece for a multi-series (or multi-slice) chart: it lists
//! every series/slice with the exact color that mark uses, so the series→color mapping the §6
//! rubric checks is correct by construction — the plot and the legend read the same palette.
//! For a **pie/doughnut** the "series" are the *categories* of the single series, so the legend
//! keys off the categories + [`slice_color`]; for everything else it keys off the series color,
//! resolved by [`resolve_series_color`] (explicit sRGB / theme reference, else the palette cycle).

use std::hash::{Hash as _, Hasher as _};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, px, rgb, App, Bounds, FontWeight, IntoElement, ParentElement, Pixels,
    SharedString, Styled, TransformationMatrix, Window,
};

use freecell_chart_model::{BarDir, Chart, ChartKind, LegendPosition, SeriesData};

use super::palette::slice_color;
use super::style::{
    hsla, resolve_series_color, AXIS_TITLE_FONT_SIZE, AXIS_TITLE_TEXT, BACKGROUND,
    LEGEND_FONT_SIZE, TITLE_FONT_SIZE, TITLE_TEXT,
};

/// Width (px) of the rotated value-axis title column. Also the SVG viewBox width, so the rotated
/// title renders 1:1 (paint_svg fits the SVG width to the bounds width — matching them keeps the
/// glyphs at their natural `AXIS_TITLE_FONT_SIZE`).
const VTITLE_WIDTH: f32 = 22.0;

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
    let is_line = matches!(chart.kind, ChartKind::Line { .. });
    chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| LegendEntry {
            // Resolve the series' explicit sRGB / theme color (or the palette cycle) to the same
            // color its mark uses, so swatch↔mark match by construction (P6 theme colors included).
            // For a **line** chart the mark is the line, so the swatch follows the `a:ln` stroke
            // color when the series carries one (P13) — the same precedence the line renderer uses.
            color: resolve_series_color(line_mark_color(s, is_line), i).to_hex(),
            name: s
                .name
                .clone()
                .unwrap_or_else(|| format!("Series {}", i + 1)),
        })
        .collect()
}

/// The color reference a series' legend swatch should use: for a line chart, the `a:ln` stroke color
/// if present (matching what the line renderer draws), else the series fill/theme color; for other
/// kinds, the series fill/theme color. Falls through to `None` so the caller applies the palette cycle.
fn line_mark_color(
    series: &freecell_chart_model::Series,
    is_line: bool,
) -> Option<freecell_chart_model::ChartColor> {
    if is_line {
        series.stroke.and_then(|st| st.color).or(series.color)
    } else {
        series.color
    }
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
                .text_size(px(LEGEND_FONT_SIZE))
                .child(SharedString::from(entry.name)),
        )
        .into_any_element()
}

/// Build the legend as a vertical **column** (one row per [`legend_entries`] key) — the Left/Right
/// (and TopRight) placements sit it beside the plot.
fn legend_column(chart: &Chart) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .px_2()
        .children(legend_entries(chart).into_iter().map(legend_row))
        .into_any_element()
}

/// Build the legend as a horizontal **bar** (wrapping row of keys) — the Top/Bottom placements sit
/// it above/below the plot, centered.
fn legend_bar(chart: &Chart) -> gpui::AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .justify_center()
        .gap_3()
        .py_1()
        .children(legend_entries(chart).into_iter().map(legend_row))
        .into_any_element()
}

/// The two axis titles as `(vertical_title, horizontal_title)` — the title of the **vertical** axis
/// (rendered rotated down the left, [`vertical_axis_title`]) and of the **horizontal** axis
/// (rendered as the bottom caption). Normally the value axis is vertical and the category axis
/// horizontal; a **horizontal** bar chart swaps them (its value axis runs along the bottom, its
/// category axis down the left).
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

/// The vertical value-axis title, rotated a **true −90°** (reading bottom-to-top) as Excel draws it
/// (`c:valAx/c:title/a:bodyPr rot="-5400000"`, P13 observation A). The P6 fallback stacked upright
/// characters because gpui's public painters don't rotate a text run — but [`gpui::Window::paint_svg`]
/// *is* the one painter that takes a rotation, and gpui's SVG renderer shapes `<text>` through
/// usvg/resvg + a system font DB. So a [`canvas`] paints an **inline SVG** whose `<text>` is rotated
/// −90°: a real rotated title, no gpui bump, no new deps. The typeface is the SVG renderer's
/// system sans-serif (DejaVu/Liberation), not the app's Inter — an accepted GAP; the weight/size and
/// the rotation match Excel.
fn vertical_axis_title(text: String) -> gpui::AnyElement {
    div()
        .flex_none()
        .w(px(VTITLE_WIDTH))
        .h_full()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| paint_rotated_title(&text, bounds, window, cx),
            )
            .size_full(),
        )
        .into_any_element()
}

/// Paint `text` rotated −90° into `bounds` via an inline SVG (see [`vertical_axis_title`]). The SVG
/// viewBox width equals the column width so `paint_svg` renders it 1:1; its height is a padded
/// estimate of the (rotated) text length, capped to the column height, and the title is centered.
fn paint_rotated_title(text: &str, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
    if text.trim().is_empty() || w < 1.0 || h < 1.0 {
        return;
    }
    let font_size = AXIS_TITLE_FONT_SIZE;
    // Estimate the rotated text's length (≈ glyph count × an average advance) + padding, so the SVG
    // canvas is tall enough not to clip the title; cap at the column height so it can't overflow the
    // whole chart in a pathological (very long title / short chart) case.
    let est_length = (text.chars().count() as f32 * font_size * 0.62 + font_size).max(font_size);
    let vb_h = est_length.min(h);
    let svg = rotated_text_svg(text, VTITLE_WIDTH, vb_h, font_size);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut hasher);
    // Content-hashed cache key so distinct titles don't collide in the sprite atlas (the key is
    // (path, size); same-size charts with different titles must differ here).
    let key: SharedString = format!("freecell://chart/vaxis/{:016x}", hasher.finish()).into();
    let _ = window.paint_svg(
        bounds,
        key,
        Some(svg.as_bytes()),
        TransformationMatrix::unit(),
        hsla(AXIS_TITLE_TEXT),
        cx,
    );
}

/// Build an inline SVG (`w`×`h` user units) whose `<text>` is rotated −90° about the canvas center,
/// so it reads bottom-to-top. The fill is opaque black — [`gpui::Window::paint_svg`] renders the SVG
/// as an alpha mask and recolors it, so only the shape (not the fill color) matters.
fn rotated_text_svg(text: &str, w: f32, h: f32, font_size: f32) -> String {
    let (cx, cy) = (w / 2.0, h / 2.0);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\"><text x=\"{cx}\" y=\"{cy}\" \
         transform=\"rotate(-90 {cx} {cy})\" text-anchor=\"middle\" dominant-baseline=\"central\" \
         font-family=\"DejaVu Sans, Liberation Sans, sans-serif\" font-weight=\"bold\" \
         font-size=\"{font_size}\" fill=\"#000000\">{t}</text></svg>",
        t = escape_xml(text)
    )
}

/// Escape the five XML metacharacters so an arbitrary axis title is safe inside SVG text.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Where the legend sits relative to the plot, derived from `c:legendPos` ([`LegendPosition`], P13).
/// Left/Right (and TopRight) are **side columns** of the body row; Top/Bottom are horizontal
/// **bars** above/below it. (TopRight is approximated as a right column — a true floating top-right
/// overlay is not modeled; a right column is Excel's closest read-only equivalent.)
#[derive(Clone, Copy, PartialEq, Eq)]
enum LegendPlacement {
    Left,
    Right,
    Top,
    Bottom,
}

impl LegendPlacement {
    fn of(position: LegendPosition) -> Self {
        match position {
            LegendPosition::Left => Self::Left,
            LegendPosition::Bottom => Self::Bottom,
            LegendPosition::Top => Self::Top,
            LegendPosition::Right | LegendPosition::TopRight => Self::Right,
        }
    }
}

/// Wrap a plot element in the full chart frame: chart title on top, the **vertical**-axis title
/// rotated down the left of the plot, the plot with its legend placed per `c:legendPos`, and the
/// **horizontal**-axis title centered below. Shared by every chart kind so the chrome is identical
/// across them.
///
/// The chrome is **driven by the model, not always-on**: the title row, each axis title, and the
/// legend are each rendered only when the model carries them (a non-empty title / axis title,
/// `chart.legend.is_some()`). An untitled, legend-less chart is just its plot — no blank
/// rows/columns, no stray legend.
pub fn chart_frame(chart: &Chart, plot: gpui::AnyElement) -> gpui::AnyElement {
    let title = chart.title.clone().unwrap_or_default();
    let (vertical_title, bottom_caption) = captions(chart);
    let placement = chart.legend.map(|l| LegendPlacement::of(l.position));

    let body = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_row()
        // A Left-placed legend column starts the body row.
        .when(placement == Some(LegendPlacement::Left), |row| {
            row.child(legend_column(chart))
        })
        // The vertical-axis title, rotated down the left of the value axis — only when there is one.
        .when(!vertical_title.is_empty(), |body| {
            body.child(vertical_axis_title(vertical_title))
        })
        .child(div().flex_1().min_w(px(0.)).child(plot))
        // A Right-placed legend column ends the body row; otherwise the plot fills the width.
        .when(placement == Some(LegendPlacement::Right), |row| {
            row.child(legend_column(chart))
        });

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(BACKGROUND))
        .p_3()
        .gap_1()
        // Chart title — only when there is one. Bold + the largest chart text (Excel proportions).
        .when(!title.is_empty(), |frame| {
            frame.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(TITLE_TEXT))
                        .text_size(px(TITLE_FONT_SIZE))
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(title)),
                ),
            )
        })
        // A Top-placed legend bar sits between the title and the plot.
        .when(placement == Some(LegendPlacement::Top), |frame| {
            frame.child(legend_bar(chart))
        })
        .child(body)
        // A Bottom-placed legend bar sits below the plot (above the bottom caption).
        .when(placement == Some(LegendPlacement::Bottom), |frame| {
            frame.child(legend_bar(chart))
        })
        // Bottom axis-title caption — only when there is one. Bold, like the (rotated) vertical title.
        .when(!bottom_caption.is_empty(), |frame| {
            frame.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_color(rgb(AXIS_TITLE_TEXT))
                        .text_size(px(AXIS_TITLE_FONT_SIZE))
                        .font_weight(FontWeight::BOLD)
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
    fn legend_swatch_follows_line_stroke_color() {
        use freecell_chart_model::LineStroke;
        // On a line chart the legend swatch matches the `a:ln` stroke color (what the line draws),
        // not the palette — so a series styled only via `a:ln` reads coherently (P13).
        let mut chart = multi_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_stroke(LineStroke::new().with_color(Color::from_hex(0xBE4B48)));
        let entries = legend_entries(&chart);
        assert_eq!(
            entries[0].color, 0xBE4B48,
            "line legend swatch follows the a:ln stroke color"
        );
        // A series with no stroke still uses its fill/palette color.
        assert_eq!(entries[1].color, series_color(1).to_hex());
    }

    #[test]
    fn legend_entry_resolves_theme_colored_series() {
        use freecell_chart_model::{ChartColor, ThemePalette, ThemeSlot};
        let mut chart = multi_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_color(ChartColor::theme(ThemeSlot::Accent2));
        let entries = legend_entries(&chart);
        assert_eq!(
            entries[0].color,
            ThemePalette::office_default().accent2.to_hex(),
            "a schemeClr swatch must resolve to the same Office accent color its mark uses"
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
    fn legend_placement_maps_every_position() {
        // Left/Right → side columns; Top/Bottom → bars; TopRight approximated as a right column
        // (P13). This is the whole c:legendPos → layout mapping the frame keys off.
        assert!(LegendPlacement::of(LegendPosition::Left) == LegendPlacement::Left);
        assert!(LegendPlacement::of(LegendPosition::Right) == LegendPlacement::Right);
        assert!(LegendPlacement::of(LegendPosition::Top) == LegendPlacement::Top);
        assert!(LegendPlacement::of(LegendPosition::Bottom) == LegendPlacement::Bottom);
        assert!(LegendPlacement::of(LegendPosition::TopRight) == LegendPlacement::Right);
    }

    #[test]
    fn rotated_title_svg_is_well_formed_and_escaped() {
        // The inline SVG carries a −90° rotation and XML-escapes the title (so an ampersand/quote
        // can't break the document) — the true-rotation path (P13 observation A).
        let svg = rotated_text_svg("R&D \"units\"", VTITLE_WIDTH, 120.0, AXIS_TITLE_FONT_SIZE);
        assert!(svg.contains("rotate(-90"), "title is rotated −90°");
        assert!(
            svg.contains("R&amp;D &quot;units&quot;"),
            "metachars escaped: {svg}"
        );
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
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
