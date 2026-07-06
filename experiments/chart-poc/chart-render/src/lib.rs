//! `chart-render` — FreeCell chart widgets over gpui-component's plot primitives, example
//! scenes, and the headless capture + agent-review harness (Experiments 2/3, §3-§6).
//!
//! gpui-free logic (the nice-tick generator, the color palette, the scene data) lives in
//! its own modules so it is unit-tested without a GPU; the gpui rendering + capture live in
//! [`bar`], [`line`], [`chrome`], [`render`], and [`capture`].

pub mod palette;
pub mod scenes;
pub mod ticks;

pub mod bar;
pub mod capture;
pub mod chrome;
pub mod line;
pub mod render;
pub mod style;

use chart_model::{Chart, ChartKind};

/// Build the chart element for any supported chart kind, dispatching to the right widget.
/// Returns `None` for a chart no widget can render yet.
pub fn chart_element(chart: &Chart) -> Option<gpui::AnyElement> {
    match chart.kind {
        ChartKind::Line { .. } => line::line_element(chart),
        ChartKind::Bar { .. } => bar::bar_element(chart),
        _ => None,
    }
}
