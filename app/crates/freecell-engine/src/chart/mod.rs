//! `freecell-engine::chart` — the chart **file layer** (charts/architecture §2, §4.1):
//! load/save chart data stitching beside IronCalc, which reads and writes no chart data.
//!
//! - [`load`] parses chart definitions **out of** an `.xlsx` zip (worksheet → drawing → chart
//!   relationship chain, cached `numCache`/`strCache`, **no** formula eval) **into**
//!   [`freecell_chart_model::Chart`]. gpui-free and IronCalc-free — a `zip` + `roxmltree`
//!   second pass over the OPC package.
//! - [`save`] re-injects the original chart parts into IronCalc's regenerated zip so a chart
//!   **survives** the chart-dropping writer (byte-preservation).
//! - [`binding`] (P9) turns the retained `c:f` refs into structured ranges + a range→chart index,
//!   and re-resolves a dirty chart's values from the current model — the live-binding machinery the
//!   worker drives (charts/architecture §4.1). gpui-free and IronCalc-free (it reads through
//!   closures).
//! - [`authoring`] programmatically writes example `.xlsx` fixtures (used by the tests).
//! - [`xlsx`] holds the shared zip + OPC-relationship helpers.
//!
//! Lifted from the chart PoC (`experiments/chart-poc/load-save`). Multi-sheet save mapping and
//! edit-reflow are later phases (P10); this is the placed baseline plus live binding (P9).

pub mod authoring;
pub mod binding;
pub mod load;
pub mod save;
pub mod xlsx;

pub use binding::{parse_cf, CellData, ChartBinding, ChartBindings};
pub use load::{
    discover, discover_and_parse, load_charts_from_xlsx, parse_chart_xml, DiscoveredChart,
    SheetDrawing,
};
pub use save::{reinject, save_with_charts, SaveReport};
