//! The example scenes: each is a [`chart_model::Chart`] plus the capture metadata the
//! agent-review harness needs (a human-readable `description` of what it should show and an
//! `expectation` the reviewer judges against, functional_spec §6).
//!
//! Phase 0 defines exactly one trivial single-series bar chart — enough to prove the whole
//! capture + review harness end-to-end. Later phases add rows here (multi-series line,
//! grouped/stacked bar, pie, scatter, …), the same way `app/render-tests` grows its case
//! table.

use chart_model::{Axis, BarDir, Category, Chart, ChartKind, Grouping, Legend, Series};

/// A wider viewport for multi-series scenes (title + legend + plot need the room).
const WIDE_VP: (u32, u32) = (720, 460);

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
    vec![
        // Phase 0 / 1.
        bar_single(),
        line_single(),
        line_multi(),
        // Phase 2 — Gate 2 harder layouts.
        bar_horizontal(),
        bar_grouped(),
        bar_stacked(),
        bar_percent_stacked(),
        area_stacked(),
        area_percent_stacked(),
        pie(),
        doughnut(),
    ]
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

/// Four quarters shared by the grouped/stacked bar + area scenes.
fn quarters() -> Vec<Category> {
    ["Q1", "Q2", "Q3", "Q4"]
        .into_iter()
        .map(|q| Category::Text(q.into()))
        .collect()
}

/// The three product series shared by the grouped/stacked bar scenes.
fn product_series() -> Vec<Series> {
    vec![
        Series::category_value(Some("Widgets"), quarters(), vec![120.0, 150.0, 90.0, 170.0]),
        Series::category_value(Some("Gadgets"), quarters(), vec![80.0, 110.0, 140.0, 100.0]),
        Series::category_value(Some("Gizmos"), quarters(), vec![60.0, 70.0, 50.0, 95.0]),
    ]
}

/// Gate 2 — single-series **horizontal bar** (`BarDir::Bar`): axes swapped, category labels
/// down the left, value axis along the bottom. The baseline "same data, other orientation" case.
fn bar_horizontal() -> Scene {
    let chart = Chart {
        title: Some("Revenue by Region".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Bar,
            grouping: Grouping::Clustered,
        },
        series: vec![Series::category_value(
            Some("Revenue"),
            vec![
                Category::Text("North".into()),
                Category::Text("South".into()),
                Category::Text("East".into()),
                Category::Text("West".into()),
                Category::Text("Central".into()),
            ],
            vec![145.0, 98.0, 176.0, 132.0, 60.0],
        )],
        cat_axis: Axis::titled("Region"),
        val_axis: Axis::titled("USD (thousands)"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "bar_horizontal",
        description: "A single-series horizontal bar chart of revenue by region (five regions), \
                      category labels down the left, a numeric value axis along the bottom, a \
                      title, and a legend.",
        expectation: "Five horizontal bars of differing lengths growing left-to-right from a \
                      value axis on the LEFT, region names (North/South/East/West/Central) down \
                      the left as category labels, readable numeric tick labels along the bottom, \
                      a chart title, and a one-entry legend. Non-blank, no clipping.",
        viewport: DEFAULT_VP,
        chart,
    }
}

/// Gate 2 — **grouped (clustered) column**, three series side-by-side within each quarter. The
/// single most common business chart; no grouped helper in the primitives (DIY sub-band offsets).
fn bar_grouped() -> Scene {
    let chart = Chart {
        title: Some("Quarterly Sales by Product".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
        },
        series: product_series(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units (thousands)"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "bar_grouped",
        description: "A grouped (clustered) column chart: three product series \
                      (Widgets/Gadgets/Gizmos) drawn side-by-side within each of four quarters, \
                      with a title, numeric value axis, and a legend.",
        expectation: "Four quarter groups (Q1-Q4), each containing THREE side-by-side columns in \
                      three distinct colors (one per product), all measured against one numeric \
                      value axis on the left with readable tick labels; a chart title; a legend \
                      whose three swatch colors match the three column colors. Bars within a group \
                      do not overlap. Non-blank, no clipping.",
        viewport: WIDE_VP,
        chart,
    }
}

/// Gate 2 — **stacked column**, three series accumulating into one column per quarter; the value
/// axis reaches the tallest stacked total.
fn bar_stacked() -> Scene {
    let chart = Chart {
        title: Some("Quarterly Sales (Stacked)".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Stacked,
        },
        series: product_series(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units (thousands)"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "bar_stacked",
        description: "A stacked column chart: three product series stacked into one column per \
                      quarter (four quarters), value axis reflecting the stacked totals, with a \
                      title and legend.",
        expectation: "Four single columns (Q1-Q4), each split into THREE stacked colored segments \
                      (one per product, same color order as the legend), the segments summing to \
                      the column total; a numeric value axis on the left whose top covers the \
                      tallest stack; a chart title and a three-entry legend with matching colors. \
                      Non-blank, no clipping.",
        viewport: WIDE_VP,
        chart,
    }
}

/// Gate 2 — **100%-stacked column**: same stack normalized so each quarter fills 0–100%.
fn bar_percent_stacked() -> Scene {
    let chart = Chart {
        title: Some("Sales Mix by Quarter".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::PercentStacked,
        },
        series: product_series(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Share of quarter"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "bar_percent_stacked",
        description: "A 100%-stacked column chart: each quarter's three product series normalized \
                      so the column fills 0-100%, with a percentage value axis, title, and legend.",
        expectation: "Four equal-height full columns (Q1-Q4), each divided into THREE colored \
                      segments whose sizes are each product's SHARE of that quarter (segments sum \
                      to a full column); a value axis labeled 0%/20%/…/100% on the left; a chart \
                      title and a three-entry legend with matching colors. Non-blank, no clipping.",
        viewport: WIDE_VP,
        chart,
    }
}

/// The three traffic-source series shared by the area scenes.
fn traffic_series() -> Vec<Series> {
    vec![
        Series::category_value(Some("Direct"), quarters(), vec![30.0, 42.0, 38.0, 55.0]),
        Series::category_value(Some("Search"), quarters(), vec![50.0, 55.0, 60.0, 58.0]),
        Series::category_value(Some("Social"), quarters(), vec![20.0, 28.0, 45.0, 40.0]),
    ]
}

/// Gate 2 — **stacked area**, the nastiest layout (hand-rolled per-x baseline polygons because
/// the `Area` primitive has a scalar baseline).
fn area_stacked() -> Scene {
    let chart = Chart {
        title: Some("Traffic by Source (Stacked)".into()),
        kind: ChartKind::Area {
            grouping: Grouping::Stacked,
        },
        series: traffic_series(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Visits (thousands)"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "area_stacked",
        description: "A stacked area chart: three traffic-source series (Direct/Search/Social) \
                      stacked as filled bands over four quarters, value axis reflecting the \
                      stacked totals, with a title and legend.",
        expectation: "Three filled area bands stacked on top of one another (each band sitting on \
                      the cumulative top of the ones below, NOT all from zero) across four \
                      quarters (Q1-Q4), in three distinct colors matching the legend; a numeric \
                      value axis on the left covering the stacked total; straight segments; a \
                      chart title and a three-entry legend. Non-blank, no clipping.",
        viewport: WIDE_VP,
        chart,
    }
}

/// Gate 2 — **100%-stacked area**: the same bands normalized so each quarter fills 0–100%.
fn area_percent_stacked() -> Scene {
    let chart = Chart {
        title: Some("Traffic Mix by Quarter".into()),
        kind: ChartKind::Area {
            grouping: Grouping::PercentStacked,
        },
        series: traffic_series(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Share of traffic"),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "area_percent_stacked",
        description:
            "A 100%-stacked area chart: three traffic-source series normalized so each \
                      quarter's bands fill 0-100%, with a percentage value axis, title, and legend.",
        expectation: "Three filled area bands that together fill the full plot height at every \
                      quarter (Q1-Q4), each band's thickness = that source's SHARE of the quarter; \
                      a value axis labeled 0%/20%/…/100% on the left; three distinct colors \
                      matching the legend; a chart title and a three-entry legend. Non-blank.",
        viewport: WIDE_VP,
        chart,
    }
}

/// The market-share slices shared by the pie + doughnut scenes.
fn share_series() -> Vec<Series> {
    vec![Series::category_value(
        Some("Share"),
        vec![
            Category::Text("Alpha".into()),
            Category::Text("Beta".into()),
            Category::Text("Gamma".into()),
            Category::Text("Delta".into()),
            Category::Text("Other".into()),
        ],
        vec![38.0, 26.0, 18.0, 12.0, 6.0],
    )]
}

/// Gate 2 — **pie** (single series, one slice per category) with a synthesized per-slice palette
/// (gpui-component has no auto-palette) and a slice→color legend.
fn pie() -> Scene {
    let chart = Chart {
        title: Some("Market Share".into()),
        kind: ChartKind::Pie {
            doughnut_hole: None,
        },
        series: share_series(),
        cat_axis: Axis::untitled(),
        val_axis: Axis::untitled(),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "pie",
        description: "A single-series pie chart of market share across five companies, each slice \
                      a distinct synthesized color, with on-slice percentage labels, a title, and \
                      a slice→color legend.",
        expectation: "A round pie divided into five wedges of differing sizes in five DISTINCT \
                      colors (NOT a monochrome disc), each wedge labeled with its percentage; a \
                      chart title ('Market Share'); a legend mapping each company \
                      (Alpha/Beta/Gamma/Delta/Other) to its slice color. Non-blank, no clipping. \
                      (A pie has no numeric value axis — this is expected.)",
        viewport: (640, 460),
        chart,
    }
}

/// Gate 2 — **doughnut** (pie with an inner radius = `doughnut_hole × outer_radius`).
fn doughnut() -> Scene {
    let chart = Chart {
        title: Some("Market Share (Doughnut)".into()),
        kind: ChartKind::Pie {
            doughnut_hole: Some(0.55),
        },
        series: share_series(),
        cat_axis: Axis::untitled(),
        val_axis: Axis::untitled(),
        legend: Some(Legend::default()),
    };
    Scene {
        name: "doughnut",
        description: "A single-series doughnut chart (pie with a center hole) of market share \
                      across five companies, distinct per-slice colors, on-slice percentage \
                      labels, a title, and a slice→color legend.",
        expectation: "A doughnut ring (a pie with a hollow center) divided into five arcs of \
                      differing sizes in five DISTINCT colors, each arc labeled with its \
                      percentage; a chart title; a legend mapping each company to its arc color. \
                      Non-blank, no clipping. (No numeric value axis — expected for a doughnut.)",
        viewport: (640, 460),
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

    #[test]
    fn gate2_scenes_have_the_expected_kinds() {
        let horizontal = get("bar_horizontal").unwrap();
        assert!(matches!(
            horizontal.chart.kind,
            ChartKind::Bar {
                dir: BarDir::Bar,
                ..
            }
        ));

        for (name, grouping) in [
            ("bar_grouped", Grouping::Clustered),
            ("bar_stacked", Grouping::Stacked),
            ("bar_percent_stacked", Grouping::PercentStacked),
        ] {
            let s = get(name).unwrap();
            match s.chart.kind {
                ChartKind::Bar { dir, grouping: g } => {
                    assert_eq!(dir, BarDir::Col);
                    assert_eq!(g, grouping, "{name} grouping");
                }
                other => panic!("{name} expected a Bar, got {other:?}"),
            }
            assert!(s.chart.series.len() >= 2, "{name} should be multi-series");
        }

        for (name, grouping) in [
            ("area_stacked", Grouping::Stacked),
            ("area_percent_stacked", Grouping::PercentStacked),
        ] {
            let s = get(name).unwrap();
            assert_eq!(s.chart.kind, ChartKind::Area { grouping }, "{name}");
            assert!(s.chart.series.len() >= 2, "{name} should be multi-series");
        }

        assert_eq!(
            get("pie").unwrap().chart.kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        assert!(matches!(
            get("doughnut").unwrap().chart.kind,
            ChartKind::Pie {
                doughnut_hole: Some(_)
            }
        ));
    }
}
