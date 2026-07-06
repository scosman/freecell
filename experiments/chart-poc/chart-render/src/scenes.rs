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
    vec![bar_single()]
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
