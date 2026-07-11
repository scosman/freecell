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
//! renderer lands in P5/P6. As of **P8** the renderer is wired into the grid — [`in_grid`] adds
//! the in-grid **ChartLayer** dispatch (fidelity → plot + badge / placeholder), which the grid
//! ([`crate::grid`]) paints over cells at each chart's anchor rect.

pub mod palette;
pub mod stacking;
pub mod ticks;

pub mod area;
pub mod bar;
pub mod chrome;
pub mod in_grid;
pub mod line;
pub mod pie;
pub mod scatter;
pub mod style;

pub use in_grid::{in_grid_chart_element, render_mode, RenderMode};

use freecell_chart_model::{Chart, ChartKind};

/// Build the chart element for any supported chart kind, dispatching to the right widget.
/// Returns `None` for a chart no widget can render yet.
pub fn chart_element(chart: &Chart) -> Option<gpui::AnyElement> {
    match chart.kind {
        ChartKind::Line { .. } => line::line_element(chart),
        ChartKind::Bar { .. } => bar::bar_element(chart),
        ChartKind::Area { .. } => area::area_element(chart),
        ChartKind::Pie { .. } => pie::pie_element(chart),
        ChartKind::Scatter { .. } => scatter::scatter_element(chart),
    }
}
