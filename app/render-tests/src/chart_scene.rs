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

use freecell_chart_model::{Axis, Category, Chart, ChartKind, Grouping, Legend, Series};

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

/// Every chart scene, rebuilt fresh per call (the `render_scene` bin looks one up by name). P4
/// seeds exactly the one scene the exit criterion needs; later phases append rows.
pub fn all() -> Vec<ChartScene> {
    vec![chart_line_multi()]
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
        viewport: (720, 460),
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
}
