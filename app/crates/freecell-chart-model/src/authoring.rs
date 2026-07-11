//! Authoring templates — [`ChartInsertKind`] and the **near-empty chart** it builds (charts/
//! implementation_plan P17, ui_design §3.1). This is the model half of the action-bar insert flow:
//! the app's chart menu offers a [`ChartInsertKind`] per glyph, and the worker turns the chosen kind
//! into a [`ChartSpec::authored`](crate::ChartSpec::authored) chart via [`near_empty_chart`].
//!
//! [`near_empty_chart`]: ChartInsertKind::near_empty_chart
//!
//! **Why a small placeholder series, not truly empty:** the in-grid renderers draw the grey
//! "Unsupported chart type" placeholder for a bar/area/pie whose only series has no data (only
//! line/scatter frame-render an empty series). A near-empty inserted chart therefore carries **one
//! series with a few placeholder points**, so *every* type renders as its real kind — a visible
//! template the user reshapes (set the data range / title) via the edit panel (P19+). It carries no
//! `c:f` refs, so on save the write path emits literals (no live binding until a range is set).

use crate::{Axis, BarDir, BarLayout, Category, Chart, ChartKind, Grouping, Legend, Series};

/// A chart type the action-bar insert menu can author (charts/ui_design §3.1). Each maps to a
/// [`ChartKind`] and has both an in-grid renderer and a write-path serializer. Bubble is
/// deliberately excluded — it has no [`ChartKind`] variant (it renders as the Unsupported
/// placeholder), so it cannot be authored yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartInsertKind {
    Line,
    /// Vertical bars (`c:barChart` + `barDir=col`).
    Column,
    /// Horizontal bars (`c:barChart` + `barDir=bar`).
    Bar,
    Area,
    Pie,
    /// A pie with a centre hole (`c:doughnutChart`).
    Doughnut,
    Scatter,
}

/// The placeholder category labels of a near-empty template (a small, neutral "1..4" set so the
/// chart renders as a recognizable frame the user then re-ranges).
const PLACEHOLDER_CATEGORIES: [&str; 4] = ["1", "2", "3", "4"];
/// The placeholder values paired with [`PLACEHOLDER_CATEGORIES`] — a gentle shape (all positive so
/// a pie/doughnut draws real slices).
const PLACEHOLDER_VALUES: [f64; 4] = [4.0, 6.0, 5.0, 8.0];
/// The doughnut hole radius (fraction of the outer radius) an authored doughnut starts with.
const DOUGHNUT_HOLE: f32 = 0.5;

impl ChartInsertKind {
    /// The fully-specified [`ChartKind`] an authored chart of this menu type starts as (the type's
    /// default grouping / orientation / hole).
    pub fn chart_kind(self) -> ChartKind {
        match self {
            ChartInsertKind::Line => ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            ChartInsertKind::Column => ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered,
                layout: BarLayout::default(),
            },
            ChartInsertKind::Bar => ChartKind::Bar {
                dir: BarDir::Bar,
                grouping: Grouping::Clustered,
                layout: BarLayout::default(),
            },
            ChartInsertKind::Area => ChartKind::Area {
                grouping: Grouping::Standard,
            },
            ChartInsertKind::Pie => ChartKind::Pie {
                doughnut_hole: None,
            },
            ChartInsertKind::Doughnut => ChartKind::Pie {
                doughnut_hole: Some(DOUGHNUT_HOLE),
            },
            ChartInsertKind::Scatter => ChartKind::Scatter,
        }
    }

    /// The [`ChartInsertKind`] a fully-specified [`ChartKind`] came from — the inverse of
    /// [`chart_kind`](Self::chart_kind). Used by the edit panel (P19) to show a chart's **current**
    /// type and by the worker to map a spec back to a menu kind for a type switch. `None` only for a
    /// [`ChartKind`] no menu entry authors (there is none today — every variant maps back).
    pub fn from_chart_kind(kind: &ChartKind) -> Option<Self> {
        Some(match kind {
            ChartKind::Line { .. } => ChartInsertKind::Line,
            ChartKind::Bar {
                dir: BarDir::Col, ..
            } => ChartInsertKind::Column,
            ChartKind::Bar {
                dir: BarDir::Bar, ..
            } => ChartInsertKind::Bar,
            ChartKind::Area { .. } => ChartInsertKind::Area,
            ChartKind::Pie {
                doughnut_hole: None,
            } => ChartInsertKind::Pie,
            ChartKind::Pie {
                doughnut_hole: Some(_),
            } => ChartInsertKind::Doughnut,
            ChartKind::Scatter => ChartInsertKind::Scatter,
        })
    }

    /// Whether an authored chart of this type carries **xy** series ([`SeriesData::Xy`]) rather than
    /// category/value — `true` only for [`Scatter`](ChartInsertKind::Scatter). Drives the data-shape a
    /// re-range / type-switch builds its series in (P19).
    pub fn is_xy(self) -> bool {
        matches!(self, ChartInsertKind::Scatter)
    }

    /// A **near-empty** authored [`Chart`] of this type: one placeholder series over a small sample
    /// grid, a generic title, default axes, and a right legend. It carries no data references —
    /// live binding is set later when the chart is re-ranged (P19). Because bar/area/pie render the
    /// Unsupported placeholder for a dataless series, the placeholder points guarantee the chart
    /// draws as its real kind the moment it is inserted.
    pub fn near_empty_chart(self) -> Chart {
        let series = self.placeholder_series();
        Chart {
            title: Some("Chart".to_string()),
            kind: self.chart_kind(),
            series: vec![series],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        }
    }

    /// The one placeholder series — an xy pair set for scatter, a category/value set otherwise.
    fn placeholder_series(self) -> Series {
        match self {
            ChartInsertKind::Scatter => Series::xy(
                Some("Series 1"),
                PLACEHOLDER_CATEGORIES
                    .iter()
                    .enumerate()
                    .map(|(i, _)| (i + 1) as f64)
                    .collect(),
                PLACEHOLDER_VALUES.to_vec(),
            ),
            _ => Series::category_value(
                Some("Series 1"),
                PLACEHOLDER_CATEGORIES
                    .iter()
                    .map(|c| Category::Text((*c).to_string()))
                    .collect(),
                PLACEHOLDER_VALUES.to_vec(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeriesData;

    const ALL: [ChartInsertKind; 7] = [
        ChartInsertKind::Line,
        ChartInsertKind::Column,
        ChartInsertKind::Bar,
        ChartInsertKind::Area,
        ChartInsertKind::Pie,
        ChartInsertKind::Doughnut,
        ChartInsertKind::Scatter,
    ];

    #[test]
    fn chart_kind_maps_each_menu_type() {
        assert!(matches!(
            ChartInsertKind::Line.chart_kind(),
            ChartKind::Line { smooth: false, .. }
        ));
        assert!(matches!(
            ChartInsertKind::Column.chart_kind(),
            ChartKind::Bar {
                dir: BarDir::Col,
                ..
            }
        ));
        assert!(matches!(
            ChartInsertKind::Bar.chart_kind(),
            ChartKind::Bar {
                dir: BarDir::Bar,
                ..
            }
        ));
        assert!(matches!(
            ChartInsertKind::Area.chart_kind(),
            ChartKind::Area { .. }
        ));
        assert_eq!(
            ChartInsertKind::Pie.chart_kind(),
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        assert!(matches!(
            ChartInsertKind::Doughnut.chart_kind(),
            ChartKind::Pie {
                doughnut_hole: Some(h)
            } if h > 0.0
        ));
        assert_eq!(ChartInsertKind::Scatter.chart_kind(), ChartKind::Scatter);
    }

    #[test]
    fn near_empty_chart_has_one_nonempty_series_and_a_title() {
        for kind in ALL {
            let chart = kind.near_empty_chart();
            assert_eq!(chart.kind, kind.chart_kind(), "{kind:?}");
            assert_eq!(chart.title.as_deref(), Some("Chart"), "{kind:?}");
            assert_eq!(chart.series.len(), 1, "{kind:?}");
            // A non-empty series so bar/area/pie render their real kind, not the placeholder box.
            assert!(
                !chart.series[0].is_empty(),
                "{kind:?} placeholder has points"
            );
            assert!(chart.legend.is_some(), "{kind:?}");
        }
    }

    #[test]
    fn scatter_uses_xy_others_use_category_value() {
        assert!(matches!(
            ChartInsertKind::Scatter.near_empty_chart().series[0].data,
            SeriesData::Xy { .. }
        ));
        for kind in [
            ChartInsertKind::Line,
            ChartInsertKind::Column,
            ChartInsertKind::Pie,
        ] {
            assert!(
                matches!(
                    kind.near_empty_chart().series[0].data,
                    SeriesData::CategoryValue { .. }
                ),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn from_chart_kind_inverts_chart_kind() {
        for kind in ALL {
            let round = ChartInsertKind::from_chart_kind(&kind.chart_kind());
            assert_eq!(round, Some(kind), "{kind:?} must round-trip its ChartKind");
        }
        // `is_xy` is true only for scatter (the one xy-series menu type).
        assert!(ChartInsertKind::Scatter.is_xy());
        assert!(!ChartInsertKind::Line.is_xy());
        assert!(!ChartInsertKind::Pie.is_xy());
    }

    #[test]
    fn pie_and_doughnut_placeholder_values_are_positive() {
        // A pie/doughnut only draws slices for a positive-sum series (else the arcs collapse).
        for kind in [ChartInsertKind::Pie, ChartInsertKind::Doughnut] {
            match &kind.near_empty_chart().series[0].data {
                SeriesData::CategoryValue { values, .. } => {
                    assert!(values.iter().sum::<f64>() > 0.0, "{kind:?}");
                }
                other => panic!("{kind:?} expected CategoryValue, got {other:?}"),
            }
        }
    }
}
