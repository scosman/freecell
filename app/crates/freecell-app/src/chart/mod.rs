//! `freecell-app::chart` — the chart **render layer** (charts/architecture §2, §4.2):
//! FreeCell's chart widgets over gpui-component's `plot/` primitives, plus the shared chrome
//! and gpui-free drawing infra.
//!
//! The gpui-free logic (the nice-tick generator [`ticks`], the categorical [`palette`], the
//! cumulative [`stacking`] math) lives in its own modules so it is unit-tested without a GPU;
//! the gpui rendering lives in [`chrome`] + the per-kind widgets ([`mod@line`], [`bar`], [`area`],
//! [`pie`], [`scatter`]). [`chart_element`] dispatches a [`freecell_chart_model::Chart`] to the
//! right widget.
//!
//! Lifted from the chart PoC (`experiments/chart-poc/chart-render`), library-only: the capture
//! harness + example scenes are lifted into `render-tests` in P4, and the production line
//! renderer / in-grid `ChartLayer` land in P5 / P8. This is the placed baseline — dormant
//! library code, not yet wired into the grid.

pub mod palette;
pub mod stacking;
pub mod ticks;

pub mod area;
pub mod bar;
pub mod chrome;
pub mod line;
pub mod pie;
pub mod scatter;
pub mod style;

use freecell_chart_model::{Chart, ChartKind};

/// Build the chart element for any supported chart kind, dispatching to the right widget.
/// Returns `None` for a chart no widget can render yet.
pub fn chart_element(chart: &Chart) -> Option<gpui::AnyElement> {
    match chart.kind {
        ChartKind::Line { .. } => line::line_element(chart),
        ChartKind::Bar { .. } => bar::bar_element(chart),
        ChartKind::Area { .. } => area::area_element(chart),
        ChartKind::Pie { .. } => pie::pie_element(chart),
        ChartKind::Scatter => scatter::scatter_element(chart),
    }
}
