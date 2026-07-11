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
    Axis, BarDir, BarLayout, Category, Chart, ChartColor, ChartKind, Color, DataLabels, Grouping,
    Legend, LegendPosition, LineStroke, Marker, MarkerSymbol, Series, ThemeSlot,
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
        chart_line_value_labels(),
        chart_line_percent_labels(),
        chart_line_named_labels(),
        chart_line_reversed(),
        chart_line_scaled(),
        chart_line_no_gridlines(),
        chart_line_styled(),
        chart_line_legend_bottom(),
        // P22 — column & bar.
        chart_column_clustered(),
        chart_column_stacked(),
        chart_column_percent(),
        chart_bar_clustered(),
        chart_column_gap_overlap(),
        chart_column_theme_fills(),
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

/// Four quarters, for the label scenes (fewer points → labels don't crowd).
fn quarters() -> Vec<Category> {
    ["Q1", "Q2", "Q3", "Q4"]
        .into_iter()
        .map(|q| Category::Text(q.into()))
        .collect()
}

/// The P12 **value data-labels** scene: a single-series line whose points carry `c:showVal` value
/// labels formatted through a currency `numFmt` (`"$#,##0"`), so each label reads `$12,000`,
/// `$19,000`, …, drawn above its point (the line default position). Proves value labels + label
/// number formatting.
fn chart_line_value_labels() -> ChartScene {
    let chart = Chart {
        title: Some("Monthly Revenue".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("Revenue"),
            months(),
            vec![12000.0, 19000.0, 15000.0, 24000.0, 21000.0, 30000.0],
        )
        .with_color(ChartColor::theme(ThemeSlot::Accent1))
        .with_marker(Marker::new(MarkerSymbol::Circle))
        .with_data_labels(DataLabels::new().value().with_number_format("$#,##0"))],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Revenue (USD)").with_number_format("$#,##0"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_value_labels",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P12 **percent data-labels** scene: a single-series line with `c:showPercent` labels — each
/// point's share of the series total, rendered as `NN%` above its point. Proves the percent path
/// (value ÷ series total), distinct from the value/currency labels above.
fn chart_line_percent_labels() -> ChartScene {
    let chart = Chart {
        title: Some("Traffic Share by Month".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("Sessions"),
            months(),
            vec![30.0, 45.0, 60.0, 40.0, 55.0, 70.0],
        )
        .with_color(ChartColor::theme(ThemeSlot::Accent4))
        .with_marker(Marker::new(MarkerSymbol::Circle))
        .with_data_labels(DataLabels::new().percent())],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Sessions (thousands)"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_percent_labels",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P12 **composed data-labels** scene: a single-series line whose labels combine the series
/// name, category name, and value (`c:showSerName`, `c:showCatName`, `c:showVal`) with a
/// `c:showLegendKey` swatch, so each label is a color swatch followed by the joined text
/// `North, Q1, 12`. Proves the multi-part label composition and the legend-key swatch.
fn chart_line_named_labels() -> ChartScene {
    let chart = Chart {
        title: Some("Units Sold".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("North"),
            quarters(),
            vec![12.0, 19.0, 15.0, 24.0],
        )
        .with_color(ChartColor::theme(ThemeSlot::Accent2))
        .with_marker(Marker::new(MarkerSymbol::Circle))
        .with_data_labels(
            DataLabels::new()
                .series_name()
                .category_name()
                .value()
                .legend_key(),
        )],
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_named_labels",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P13 **reversed category axis** scene: a two-series line whose category (`c:orientation
/// maxMin`) axis runs right→left, so the months read Jun→Jan. Proves the reversed-axis rendering
/// (`c:scaling`), distinct from the default minMax order.
fn chart_line_reversed() -> ChartScene {
    let chart = Chart {
        title: Some("Backlog by Month (reversed)".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("Open"),
                months(),
                vec![88.0, 74.0, 63.0, 51.0, 44.0, 30.0],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent1)),
            Series::category_value(
                Some("Closed"),
                months(),
                vec![20.0, 33.0, 41.0, 55.0, 62.0, 79.0],
            )
            .with_color(ChartColor::theme(ThemeSlot::Accent2)),
        ],
        cat_axis: Axis::titled("Month").reversed(),
        val_axis: Axis::titled("Tickets"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_reversed",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P13 **explicit value-axis scaling** scene: a single-series line whose value axis is pinned to
/// a fixed `0..100` (`c:scaling/c:min` + `c:max`) even though the data only spans ~30..85, so the
/// line sits low in a fixed-range plot. Proves min/max override the auto nice-scale.
fn chart_line_scaled() -> ChartScene {
    let chart = Chart {
        title: Some("Utilization (fixed 0–100)".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("CPU %"),
            months(),
            vec![32.0, 41.0, 38.0, 55.0, 47.0, 61.0],
        )
        .with_color(ChartColor::theme(ThemeSlot::Accent5))
        .with_marker(Marker::new(MarkerSymbol::Circle))],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Percent").with_bounds(Some(0.0), Some(100.0)),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_scaled",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P13 **gridlines-off** scene: a two-series line whose value axis carries no `c:majorGridlines`,
/// so the plot draws no horizontal gridlines (only the axis lines + data). Proves the gridline toggle
/// is honored (distinct from every other scene, which keeps Excel's default gridlines on).
fn chart_line_no_gridlines() -> ChartScene {
    let chart = Chart {
        title: Some("Signal (no gridlines)".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("A"),
                months(),
                vec![12.0, 19.0, 15.0, 24.0, 21.0, 30.0],
            ),
            Series::category_value(
                Some("B"),
                months(),
                vec![22.0, 18.0, 26.0, 20.0, 29.0, 25.0],
            ),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Level").without_major_gridlines(),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_no_gridlines",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P13 **`a:ln` line styling** scene: a two-series line where each series carries an explicit
/// stroke — a heavy 3pt line vs a lighter 1.5pt semi-transparent line (`a:ln w=…` + `a:solidFill`
/// color + `a:alpha`). Proves honored stroke width, color, and alpha.
fn chart_line_styled() -> ChartScene {
    let chart = Chart {
        title: Some("Line Styling".into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(
                Some("Heavy"),
                months(),
                vec![30.0, 42.0, 51.0, 60.0, 69.0, 82.0],
            )
            .with_stroke(
                LineStroke::new()
                    .with_width_emu(38_100) // 3pt
                    .with_color(Color::from_hex(0x4A7EBB)),
            ),
            Series::category_value(
                Some("Light / 40%"),
                months(),
                vec![70.0, 58.0, 61.0, 47.0, 52.0, 40.0],
            )
            .with_stroke(
                LineStroke::new()
                    .with_width_emu(19_050) // 1.5pt
                    .with_color(Color::from_hex(0xBE4B48))
                    .with_alpha(0.4),
            ),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Value"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_line_styled",
        viewport: WIDE_VP,
        chart,
    }
}

/// The P13 **bottom legend** scene: a three-series line whose legend is placed **below** the plot
/// (`c:legendPos val="b"`) as a horizontal bar, rather than the default right column. Proves the
/// legend-position layout (the other placements share the mapping in `chrome::LegendPlacement`).
fn chart_line_legend_bottom() -> ChartScene {
    let chart = Chart {
        title: Some("Throughput".into()),
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
        val_axis: Axis::titled("Units"),
        legend: Some(Legend {
            position: LegendPosition::Bottom,
        }),
    };
    ChartScene {
        name: "chart_line_legend_bottom",
        viewport: WIDE_VP,
        chart,
    }
}

// -------------------------------------------------------------------------------------------------
// P22 — column & bar scenes
// -------------------------------------------------------------------------------------------------

/// Three product series over four quarters — the shared data for the column grouping scenes.
fn products() -> Vec<Series> {
    vec![
        Series::category_value(Some("Widgets"), quarters(), vec![120.0, 150.0, 90.0, 170.0])
            .with_color(Color::from_hex(0x4472C4)),
        Series::category_value(Some("Gadgets"), quarters(), vec![80.0, 110.0, 130.0, 95.0])
            .with_color(Color::from_hex(0xED7D31)),
        Series::category_value(Some("Gizmos"), quarters(), vec![60.0, 70.0, 55.0, 120.0])
            .with_color(Color::from_hex(0xFFC000)),
    ]
}

/// A clustered **column** chart (`barDir=col`, `grouping=clustered`): three series side-by-side per
/// quarter with explicit sRGB fills, title, both axis titles, and a right legend. The bread-and-butter
/// column chart — proves the multi-series clustered geometry + per-series fills.
fn chart_column_clustered() -> ChartScene {
    let chart = Chart {
        title: Some("Quarterly Sales by Product".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        },
        series: products(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_column_clustered",
        viewport: WIDE_VP,
        chart,
    }
}

/// A **stacked** column chart (`grouping=stacked`): the same three product series stacked into one
/// column per quarter, the value axis reaching the tallest stack total. Proves the cumulative-segment
/// geometry.
fn chart_column_stacked() -> ChartScene {
    let chart = Chart {
        title: Some("Quarterly Sales (stacked)".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Stacked,
            layout: BarLayout::default(),
        },
        series: products(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_column_stacked",
        viewport: WIDE_VP,
        chart,
    }
}

/// A **100%-stacked** column chart (`grouping=percentStacked`): each quarter's stack normalized to fill
/// 0–100%, so the value axis is a fixed 0–100% and every column is full height. Proves the percent
/// normalization + `%` tick labels.
fn chart_column_percent() -> ChartScene {
    let chart = Chart {
        title: Some("Product Mix (100% stacked)".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::PercentStacked,
            layout: BarLayout::default(),
        },
        series: products(),
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Share"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_column_percent",
        viewport: WIDE_VP,
        chart,
    }
}

/// A clustered **horizontal bar** chart (`barDir=bar`): two series over four **distinct** categories
/// (Alpha/Bravo/Charlie/Delta) so the **reversed** category order is visually unambiguous — Excel draws
/// the FIRST category (Alpha) at the BOTTOM (`ooxml-coverage-matrix.md` §B; the classic gotcha). The
/// category labels run down the left, the value axis along the bottom.
fn chart_bar_clustered() -> ChartScene {
    let categories = || {
        ["Alpha", "Bravo", "Charlie", "Delta"]
            .into_iter()
            .map(|c| Category::Text(c.into()))
            .collect::<Vec<_>>()
    };
    let chart = Chart {
        title: Some("Scores by Team".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Bar,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        },
        series: vec![
            Series::category_value(Some("Q1"), categories(), vec![45.0, 62.0, 38.0, 74.0])
                .with_color(Color::from_hex(0x4472C4)),
            Series::category_value(Some("Q2"), categories(), vec![58.0, 49.0, 66.0, 52.0])
                .with_color(Color::from_hex(0xED7D31)),
        ],
        cat_axis: Axis::titled("Team"),
        val_axis: Axis::titled("Score"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_bar_clustered",
        viewport: WIDE_VP,
        chart,
    }
}

/// A clustered column with a **non-default** `gapWidth` + `overlap` (`BarLayout::new(40, 50)`): a narrow
/// inter-cluster gap makes the bars wide and a positive overlap makes the two series' bars overlap —
/// visibly different geometry from the default `chart_column_clustered`. Proves the `c:gapWidth` /
/// `c:overlap` geometry is honored (P22).
fn chart_column_gap_overlap() -> ChartScene {
    let chart = Chart {
        title: Some("Tight Gap / 50% Overlap".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::new(40, 50),
        },
        series: vec![
            Series::category_value(Some("Plan"), quarters(), vec![120.0, 150.0, 90.0, 170.0])
                .with_color(Color::from_hex(0x4472C4)),
            Series::category_value(Some("Actual"), quarters(), vec![100.0, 165.0, 110.0, 140.0])
                .with_color(Color::from_hex(0xED7D31)),
        ],
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_column_gap_overlap",
        viewport: WIDE_VP,
        chart,
    }
}

/// A clustered column whose series carry **theme `schemeClr`** fills (accent1/2/3) rather than explicit
/// sRGB — proves per-type fill theme resolution (`ooxml-coverage-matrix.md` §C), the bar analogue of the
/// line marker/theme scene.
fn chart_column_theme_fills() -> ChartScene {
    let chart = Chart {
        title: Some("Revenue by Region".into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        },
        series: vec![
            Series::category_value(Some("North"), quarters(), vec![120.0, 150.0, 90.0, 170.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent1)),
            Series::category_value(Some("South"), quarters(), vec![80.0, 110.0, 130.0, 95.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent2)),
            Series::category_value(Some("West"), quarters(), vec![60.0, 70.0, 55.0, 120.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent3)),
        ],
        cat_axis: Axis::titled("Quarter"),
        val_axis: Axis::titled("Revenue"),
        legend: Some(Legend::default()),
    };
    ChartScene {
        name: "chart_column_theme_fills",
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

    #[test]
    fn p12_scenes_carry_their_data_label_toggles() {
        // Value labels: showVal + a currency label numFmt.
        let value = get("chart_line_value_labels").expect("value-labels scene");
        let dl = value.chart.series[0]
            .data_labels
            .as_ref()
            .expect("value labels");
        assert!(dl.show_value && dl.is_shown());
        assert_eq!(dl.number_format.as_deref(), Some("$#,##0"));

        // Percent labels: showPercent (the share-of-total path).
        let percent = get("chart_line_percent_labels").expect("percent-labels scene");
        assert!(
            percent.chart.series[0]
                .data_labels
                .as_ref()
                .unwrap()
                .show_percent
        );

        // Composed labels: series name + category + value + legend-key swatch.
        let named = get("chart_line_named_labels").expect("named-labels scene");
        let dl = named.chart.series[0].data_labels.as_ref().unwrap();
        assert!(
            dl.show_series_name && dl.show_category_name && dl.show_value && dl.show_legend_key
        );
    }

    #[test]
    fn p13_scenes_carry_their_axis_and_line_features() {
        // Reversed: category axis reversed (c:orientation maxMin).
        let rev = get("chart_line_reversed").expect("reversed scene");
        assert!(rev.chart.cat_axis.reversed, "category axis is reversed");
        assert!(!rev.chart.val_axis.reversed, "value axis stays default");

        // Scaled: explicit value-axis bounds override the auto scale.
        let scaled = get("chart_line_scaled").expect("scaled scene");
        assert_eq!(
            (scaled.chart.val_axis.min, scaled.chart.val_axis.max),
            (Some(0.0), Some(100.0)),
            "value axis pinned to 0..100"
        );

        // No gridlines: value axis major gridlines off.
        let no_grid = get("chart_line_no_gridlines").expect("no-gridlines scene");
        assert!(
            !no_grid.chart.val_axis.major_gridlines,
            "value-axis gridlines are off"
        );

        // Styled: each series carries an a:ln stroke (heavy + alpha).
        let styled = get("chart_line_styled").expect("styled scene");
        let heavy = styled.chart.series[0].stroke.expect("heavy stroke");
        let light = styled.chart.series[1].stroke.expect("light stroke");
        assert!(
            heavy.width_pt.unwrap() > light.width_pt.unwrap(),
            "heavy > light"
        );
        assert_eq!(light.alpha, Some(0.4), "light series is 40% opacity");

        // Legend bottom: legend placed below the plot.
        let bottom = get("chart_line_legend_bottom").expect("legend-bottom scene");
        assert_eq!(
            bottom.chart.legend.map(|l| l.position),
            Some(LegendPosition::Bottom),
            "legend is bottom-placed"
        );
    }

    #[test]
    fn p22_scenes_carry_their_bar_kind_and_layout() {
        // Every column scene is a vertical bar of the intended grouping.
        for (name, grouping) in [
            ("chart_column_clustered", Grouping::Clustered),
            ("chart_column_stacked", Grouping::Stacked),
            ("chart_column_percent", Grouping::PercentStacked),
            ("chart_column_theme_fills", Grouping::Clustered),
        ] {
            let s = get(name).unwrap_or_else(|| panic!("{name} scene"));
            assert_eq!(
                s.chart.kind,
                ChartKind::Bar {
                    dir: BarDir::Col,
                    grouping,
                    layout: BarLayout::default(),
                },
                "{name}"
            );
        }

        // The bar scene is HORIZONTAL (proves the reversed-order path) over distinct categories.
        let bar = get("chart_bar_clustered").expect("bar scene");
        assert!(matches!(
            bar.chart.kind,
            ChartKind::Bar {
                dir: BarDir::Bar,
                ..
            }
        ));

        // The gap/overlap scene carries the NON-default layout.
        let go = get("chart_column_gap_overlap").expect("gap/overlap scene");
        assert!(matches!(
            go.chart.kind,
            ChartKind::Bar {
                layout: BarLayout {
                    gap_width: 40,
                    overlap: 50
                },
                ..
            }
        ));

        // The theme-fills scene resolves every series to a theme color.
        let theme = get("chart_column_theme_fills").expect("theme-fills scene");
        assert!(
            theme
                .chart
                .series
                .iter()
                .all(|s| matches!(s.color, Some(ChartColor::Theme { .. }))),
            "every theme-fills series is a schemeClr theme color"
        );
    }
}
