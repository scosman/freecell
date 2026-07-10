//! `freecell-engine::chart` — the chart **file layer** (charts/architecture §2, §4.1):
//! load/save chart data stitching beside IronCalc, which reads and writes no chart data.
//!
//! - [`load`] parses chart definitions **out of** an `.xlsx` zip (worksheet → drawing → chart
//!   relationship chain, cached `numCache`/`strCache`, **no** formula eval) **into**
//!   [`freecell_chart_model::Chart`]. gpui-free and IronCalc-free — a `zip` + `roxmltree`
//!   second pass over the OPC package.
//! - [`save`] re-injects the original chart parts into IronCalc's regenerated zip so a chart
//!   **survives** the chart-dropping writer (byte-preservation).
//! - [`authoring`] programmatically writes example `.xlsx` fixtures (used by the tests).
//! - [`xlsx`] holds the shared zip + OPC-relationship helpers.
//!
//! Lifted from the chart PoC (`experiments/chart-poc/load-save`). Live binding, multi-sheet
//! save mapping, and edit-reflow are later phases (P7/P9/P10); this is the placed baseline.

pub mod authoring;
pub mod load;
pub mod save;
pub mod xlsx;

pub use load::{
    discover, discover_and_parse, load_charts_from_xlsx, parse_chart_xml, DiscoveredChart,
    SheetDrawing,
};
pub use save::{reinject, save_with_charts, SaveReport};
