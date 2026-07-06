//! The example scenes: each is a [`chart_model::Chart`] plus the capture metadata the
//! agent-review harness needs (a human-readable `description` of what it should show and an
//! `expectation` the reviewer judges against, functional_spec §6).
//!
//! Phase 0 defines exactly one trivial single-series bar chart — enough to prove the whole
//! capture + review harness end-to-end. Later phases add rows here (multi-series line,
//! grouped/stacked bar, pie, scatter, …), the same way `app/render-tests` grows its case
//! table.

use chart_model::{Axis, BarDir, Category, Chart, ChartKind, Grouping, Legend, Series};

/// One capturable example: a chart, its viewport, and the review metadata.
pub struct Scene {
    /// snake_case — IS the PNG filename (`results/<name>.png`) and the `--scene` key.
    pub name: &'static str,
    /// One line describing what the image should show (goes in `manifest.json`).
    pub description: &'static str,
    /// What a correct render must contain, for the reviewer agent to judge against.
    pub expectation: &'static str,
    /// Capture size in device px.
    pub viewport: (u32, u32),
    /// The chart to render, built from the shared data model.
    pub chart: Chart,
}

/// A roomy default viewport — big enough for a title, axis titles, legend, and a plot area.
const DEFAULT_VP: (u32, u32) = (640, 440);

/// Every scene, rebuilt fresh per call (the `render_scene` bin looks one up by name).
pub fn all() -> Vec<Scene> {
    vec![bar_single(), line_single(), line_multi()]
}

/// Look a scene up by name.
pub fn get(name: &str) -> Option<Scene> {
    all().into_iter().find(|s| s.name == name)
}

/// Phase 0 — the trivial single-series column chart that proves the harness end-to-end.
fn bar_single() -> Scene {
    let chart = Chart {
        title: Some("Quarterly Revenue".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
        },
        series: vec![Series::category_value(
            Some("Revenue"),
            vec![
                Category::Text("Q1".into()),
                Category::Text("Q2".into()),
                Category::Text("Q3".into()),
                Category::Text("Q4".into()),
            ],
            vec![120.0, 90.0, 150.0, 175.0],
        )],
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("USD (thousands)"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "bar_single",
        description: "A single-series vertical bar (column) chart of quarterly revenue \
                      with four bars (Q1-Q4), a title, axis titles, a numeric value axis, \
                      and a legend.",
        expectation: "A vertical bar chart with four upright bars of differing heights, a \
                      readable numeric value axis on the left, category labels Q1-Q4 along \
                      the bottom, a chart title, and a legend. Non-blank.",
        viewport: DEFAULT_VP,
        chart,
    }
}

/// The six months the line scenes share as their category axis.
fn months() -> Vec<Category> {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun"]
        .into_iter()
        .map(|m| Category::Text(m.into()))
        .collect()
}

/// Phase 1 supporting sanity scene — a single line, to confirm the line widget's axis / grid /
/// marker scaffolding reads cleanly on its own before the multi-series case.
fn line_single() -> Scene {
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
    Scene {
        name: "line_single",
        description: "A single-series line chart of monthly website visitors (Jan-Jun) with a \
                      title, axis titles, a numeric value axis, and a legend.",
        expectation: "One straight-segment line rising left-to-right across six months \
                      (Jan-Jun), with dot markers at each point, a readable numeric value axis \
                      on the left, a chart title, axis titles, and a one-entry legend. Non-blank.",
        viewport: DEFAULT_VP,
        chart,
    }
}

/// Phase 1 — GATE 1, the make-or-break scene (functional_spec §3, §7): a **multi-series** line
/// chart (three regions over six months) whose lines cross, drawn against ONE shared value
/// scale, with a title, both axis titles, a numeric value axis with nice ticks, a category
/// axis, and a legend mapping each region to its line color.
fn line_multi() -> Scene {
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
    Scene {
        name: "line_multi",
        description: "A three-series line chart of regional sales (North/South/West) over six \
                      months (Jan-Jun): three straight-segment lines in distinct colors sharing \
                      one value axis, with a title, axis titles, a numeric value axis, and a \
                      legend.",
        expectation: "Three distinctly colored straight-segment lines (North, South, West) that \
                      cross over six months (Jan-Jun), all measured against ONE shared numeric \
                      value axis on the left with readable, evenly spaced tick labels; dot \
                      markers at each data point; a chart title ('Regional Sales by Month'); a \
                      value-axis title and a 'Month' category-axis title; and a legend whose \
                      three swatch colors match the three line colors. No clipping or overlap. \
                      Non-blank.",
        viewport: (720, 460),
        chart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_lookupable_and_has_metadata() {
        let scenes = all();
        assert!(!scenes.is_empty());
        for s in &scenes {
            assert!(get(s.name).is_some(), "{} not found by name", s.name);
            assert!(!s.description.is_empty());
            assert!(!s.expectation.is_empty());
            assert!(s.viewport.0 > 0 && s.viewport.1 > 0);
            assert!(!s.chart.series.is_empty());
        }
    }

    #[test]
    fn gate1_line_scene_is_multi_series_line() {
        let s = get("line_multi").expect("line_multi scene");
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
    fn phase0_bar_scene_is_a_single_series_column() {
        let s = get("bar_single").expect("bar_single scene");
        assert!(matches!(
            s.chart.kind,
            ChartKind::Bar {
                dir: BarDir::Col,
                ..
            }
        ));
        assert_eq!(s.chart.series.len(), 1);
        assert_eq!(s.chart.series[0].len(), 4);
    }
}
