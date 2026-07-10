//! The chart scene registry — the chart analogue of the grid [`crate::cases`] table
//! (charts/architecture §7, implementation_plan P4).
//!
//! A [`ChartScene`] is a [`freecell_chart_model::Chart`] fixture plus the capture viewport. The
//! `render_scene` bin looks one up by name (`--chart <name>`), the [`crate::capture::render_charts`]
//! path renders each headless, and the chart pixel-diff test diffs it against a committed
//! baseline. Unlike the grid [`crate::scene::Scene`] (which drives the real engine), a chart
//! fixture is just static data — the chart model holds concrete cached numbers/strings, so no
//! engine or formula evaluation is needed to render it.
//!
//! This is lifted from the chart PoC's `scenes.rs`, trimmed to the render-test need: P4 seeds it
//! with the one make-or-break scene (`chart_line_multi`); later phases add rows the same way the
//! grid case table grows.

use freecell_chart_model::{
    Axis, Category, Chart, ChartColor, ChartKind, Grouping, Legend, Marker, MarkerSymbol, Series,
    ThemeSlot,
};

/// One capturable chart fixture: a chart, and the (tight) capture viewport in device px. `name`
/// is snake_case and IS the baseline PNG filename (`<name>.png`) and the `--chart` key, so a red
/// CI line names the exact scene.
pub struct ChartScene {
    /// snake_case — the baseline filename and the `--chart` lookup key. Chart scenes are prefixed
    /// `chart_` so they never collide with a grid case name and so `render_tests.sh test chart_`
    /// (or `generate --only chart_`) selects only chart scenes.
    pub name: &'static str,
    /// Capture size in device px.
    pub viewport: (u32, u32),
    /// The chart to render, built from the shared gpui-free data model.
    pub chart: Chart,
}

/// A wide viewport for multi-series scenes (title + legend + plot need the room).
const WIDE_VP: (u32, u32) = (720, 460);
/// A roomy default viewport for the simpler single-series / no-legend cases.
const DEFAULT_VP: (u32, u32) = (640, 440);

/// Every chart scene, rebuilt fresh per call (the `render_scene` bin looks one up by name). P4
/// seeded the one make-or-break multi-series scene; P5 adds the production line coverage
/// (single-series, a zero-crossing nice-tick axis, legend-off, and title/axis-title collapse).
pub fn all() -> Vec<ChartScene> {
    vec![
        chart_line_multi(),
        chart_line_single(),
        chart_line_negative(),
        chart_line_no_legend(),
        chart_line_no_titles(),
        chart_line_markers(),
        chart_line_smooth(),
    ]
}

/// Look a chart scene up by name.
pub fn get(name: &str) -> Option<ChartScene> {
    all().into_iter().find(|s| s.name == name)
}

/// The six months the line scene uses as its category axis.
fn months() -> Vec<Category> {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun"]
        .into_iter()
        .map(|m| Category::Text(m.into()))
        .collect()
}

/// The make-or-break Gate-1 scene (functional_spec §3, §7): a **multi-series** line chart (three
/// regions over six months) whose lines cross, drawn against ONE shared value scale, with a
/// title, both axis titles, a numeric value axis with nice ticks, a category axis, and a legend
/// mapping each region to its line color. It exercises the full chart chrome in a single capture,
/// so it is the richest proof that the chart render → capture → diff path works end-to-end.
fn chart_line_multi() -> ChartScene {
    let chart = Chart {
        title: Some("Regional Sales by Month".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("North"),
                months(),
                vec![32.0, 41.0, 55.0, 62.0, 78.0, 91.0],
            ),
            Series::category_value(
                Some("South"),
                months(),
                vec![74.0, 60.0, 48.0, 52.0, 63.0, 85.0],
            ),
            Series::category_value(
                Some("West"),
                months(),
                vec![50.0, 54.0, 49.0, 58.0, 61.0, 66.0],
            ),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Units (thousands)"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_multi",
        viewport: WIDE_VP,
        chart,
    }
}

/// A single-series line (monthly website visitors) with a title, both axis titles, and a
/// **one-entry** legend — proves the single-series render path and a single-row legend read
/// cleanly (the production line's simplest real shape).
fn chart_line_single() -> ChartScene {
    let chart = Chart {
        title: Some("Website Visitors".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("Visitors"),
            months(),
            vec![42.0, 55.0, 51.0, 68.0, 74.0, 90.0],
        )],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Visitors (thousands)"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_single",
        viewport: DEFAULT_VP,
        chart,
    }
}

/// A two-series line whose values **cross zero** (negative and positive). Proves the nice-tick
/// numeric value axis over a zero-crossing SHARED scale: the auto-ranged domain spans the negative
/// floor to the positive ceiling, with a `0` tick and negative tick labels — not forced to a zero
/// baseline, and not per-series.
fn chart_line_negative() -> ChartScene {
    let chart = Chart {
        title: Some("Temperature Deviation".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("Station A"),
                months(),
                vec![-12.0, -5.0, 3.0, 8.0, 15.0, 22.0],
            ),
            Series::category_value(
                Some("Station B"),
                months(),
                vec![-20.0, -14.0, -2.0, 5.0, 9.0, 18.0],
            ),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Deviation"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_negative",
        viewport: WIDE_VP,
        chart,
    }
}

/// A two-series line with **no legend** (`legend: None`). Proves the legend is model-driven: with
/// no legend the plot uses the full width and no legend column is drawn (the production behavior
/// the seed lacked — it always drew a legend).
fn chart_line_no_legend() -> ChartScene {
    let chart = Chart {
        title: Some("Active Users by Month".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("2023"),
                months(),
                vec![30.0, 34.0, 41.0, 45.0, 52.0, 60.0],
            ),
            Series::category_value(
                Some("2024"),
                months(),
                vec![44.0, 49.0, 55.0, 62.0, 71.0, 83.0],
            ),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Users (thousands)"),
        legend: None,
    };
    ChartScene {
        name: "chart_line_no_legend",
        viewport: WIDE_VP,
        chart,
    }
}

/// A three-series line with **no chart title and untitled axes**, but a legend present. Proves the
/// title row and both axis-title captions collapse (no blank rows) while the legend still renders
/// — the chrome is driven by the model, element by element.
fn chart_line_no_titles() -> ChartScene {
    let chart = Chart {
        title: None,
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("Alpha"),
                months(),
                vec![18.0, 24.0, 30.0, 27.0, 33.0, 39.0],
            ),
            Series::category_value(
                Some("Beta"),
                months(),
                vec![40.0, 36.0, 31.0, 34.0, 29.0, 25.0],
            ),
            Series::category_value(
                Some("Gamma"),
                months(),
                vec![22.0, 26.0, 24.0, 30.0, 35.0, 32.0],
            ),
        ],
        cat_axis: Axis::untitled(),
        val_axis: Axis::untitled(),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_no_titles",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P6 fidelity showcase (markers + theme colors + numFmt): a three-region straight line where
/// each series is an **Office theme color** (`schemeClr` accent1/2/3) carrying a distinct **marker**
/// (circle / square / diamond), and the value axis uses a **currency `numFmt`** (`"$#,##0"`) so its
/// ticks read `$0`, `$20,000`, …. Proves theme-color resolution, per-series marker shapes, numFmt
/// ticks, and the rotated vertical value-axis title in one capture.
fn chart_line_markers() -> ChartScene {
    let chart = Chart {
        title: Some("Regional Revenue".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("North"),
                months(),
                vec![18000.0, 24000.0, 30000.0, 41000.0, 52000.0, 63000.0],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent1))
            .with_marker(Marker::new(MarkerSymbol::Circle)),
            Series::category_value(
                Some("South"),
                months(),
                vec![42000.0, 38000.0, 45000.0, 39000.0, 48000.0, 55000.0],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent2))
            .with_marker(Marker::new(MarkerSymbol::Square)),
            Series::category_value(
                Some("West"),
                months(),
                vec![25000.0, 29000.0, 27000.0, 34000.0, 40000.0, 46000.0],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent3))
            .with_marker(Marker::new(MarkerSymbol::Diamond)),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Revenue (USD)").with_number_format("$#,##0"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_markers",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P6 `smooth` + percent-`numFmt` showcase: a two-series **smooth** (curved, `c:smooth`) line of
/// conversion rates in **Office theme colors** (`schemeClr` accent5/accent6), with a **percent
/// `numFmt`** (`"0%"`) value axis so its fractional values (0.12, 0.34, …) render as `12%`, `34%`.
/// Proves the curved stroke and percent tick formatting (distinct from the straight, currency
/// `chart_line_markers`).
fn chart_line_smooth() -> ChartScene {
    let chart = Chart {
        title: Some("Conversion Rate".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: true,
        },
        series: vec![
            Series::category_value(
                Some("Desktop"),
                months(),
                vec![0.12, 0.18, 0.22, 0.31, 0.4, 0.52],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent5)),
            Series::category_value(
                Some("Mobile"),
                months(),
                vec![0.08, 0.11, 0.19, 0.24, 0.29, 0.38],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent6)),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Rate").with_number_format("0%"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_smooth",
        viewport: WIDE_VP,
        chart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_lookupable_and_nonempty() {
        let scenes = all();
        assert!(!scenes.is_empty());
        for s in &scenes {
            assert!(get(s.name).is_some(), "{} not found by name", s.name);
            assert!(
                s.name.starts_with("chart_"),
                "{} needs the chart_ prefix",
                s.name
            );
            assert!(s.viewport.0 > 0 && s.viewport.1 > 0);
            assert!(!s.chart.series.is_empty());
        }
    }

    #[test]
    fn line_multi_is_a_multi_series_line() {
        let s = get("chart_line_multi").expect("chart_line_multi scene");
        assert!(matches!(
            s.chart.kind,
            ChartKind::Line { smooth: false, .. }
        ));
        assert!(
            s.chart.series.len() >= 2,
            "Gate 1 needs a multi-series line, got {}",
            s.chart.series.len()
        );
        // Every series shares the same category count (one point per month).
        let cats = s.chart.series[0].len();
        assert!(cats > 0);
        for series in &s.chart.series {
            assert_eq!(series.len(), cats, "series must share the category axis");
        }
    }

    #[test]
    fn production_line_scenes_cover_their_features() {
        // Every chart_line_* scene is a line chart.
        for name in [
            "chart_line_multi",
            "chart_line_single",
            "chart_line_negative",
            "chart_line_no_legend",
            "chart_line_no_titles",
        ] {
            let s = get(name).unwrap_or_else(|| panic!("{name} scene"));
            assert!(
                matches!(s.chart.kind, ChartKind::Line { .. }),
                "{name} must be a line chart"
            );
        }

        // Single-series line: exactly one series (one-entry legend).
        assert_eq!(get("chart_line_single").unwrap().chart.series.len(), 1);

        // Negative scene: carries a value below zero (the zero-crossing shared domain).
        let neg = get("chart_line_negative").unwrap();
        let has_negative = neg.chart.series.iter().any(|s| match &s.data {
            freecell_chart_model::SeriesData::CategoryValue { values, .. } => {
                values.iter().any(|&v| v < 0.0)
            }
            _ => false,
        });
        assert!(has_negative, "chart_line_negative must cross zero");

        // Legend-off scene: no legend in the model (proves the legend is model-driven).
        assert!(
            get("chart_line_no_legend").unwrap().chart.legend.is_none(),
            "chart_line_no_legend must have no legend"
        );

        // Titles-off scene: no chart title and both axes untitled (chrome collapse).
        let bare = get("chart_line_no_titles").unwrap();
        assert!(bare.chart.title.is_none(), "no chart title");
        assert!(
            bare.chart.cat_axis.title.is_none(),
            "untitled category axis"
        );
        assert!(bare.chart.val_axis.title.is_none(), "untitled value axis");
        // ...but a legend is still present (it should render even with titles gone).
        assert!(bare.chart.legend.is_some(), "legend still present");
    }

    #[test]
    fn p6_scenes_carry_the_new_fidelity_features() {
        // Markers scene: theme colors, distinct marker symbols, and a currency value-axis numFmt.
        let markers = get("chart_line_markers").expect("chart_line_markers scene");
        assert!(matches!(
            markers.chart.kind,
            ChartKind::Line { smooth: false, .. }
        ));
        assert_eq!(
            markers.chart.val_axis.number_format.as_deref(),
            Some("$#,##0"),
            "markers scene must format ticks as currency"
        );
        let symbols: Vec<_> = markers
            .chart
            .series
            .iter()
            .map(|s| s.marker.map(|m| m.symbol))
            .collect();
        assert_eq!(
            symbols,
            vec![
                Some(MarkerSymbol::Circle),
                Some(MarkerSymbol::Square),
                Some(MarkerSymbol::Diamond)
            ],
            "each series carries its own marker shape"
        );
        assert!(
            markers
                .chart
                .series
                .iter()
                .all(|s| matches!(s.color, Some(ChartColor::Theme { .. }))),
            "every markers-scene series is a theme color"
        );

        // Smooth scene: curved line and a percent value-axis numFmt.
        let smooth = get("chart_line_smooth").expect("chart_line_smooth scene");
        assert!(
            matches!(smooth.chart.kind, ChartKind::Line { smooth: true, .. }),
            "smooth scene must be a curved line"
        );
        assert_eq!(
            smooth.chart.val_axis.number_format.as_deref(),
            Some("0%"),
            "smooth scene must format ticks as percentages"
        );
    }
}
