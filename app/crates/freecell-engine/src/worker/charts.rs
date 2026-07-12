//! [`ChartSnapshot`] — the charts half of the worker publication seam (charts/architecture §4.1).
//!
//! Charts ride the **same lock-free snapshot path as cells**, not a bespoke channel: the worker
//! stores a `ChartSnapshot` into an [`ArcSwap`](arc_swap::ArcSwap) in
//! [`Shared`](super::client) and signals it with the *existing*
//! [`WorkerEvent::Published`](super::WorkerEvent) — the one that already fires per edit. The UI
//! loads the snapshot wait-free on each `Published` (and on `Loaded`) and installs it only when the
//! [`version`](ChartSnapshot::version) changed, so a scroll-only publish — or an edit that touches
//! no chart — never re-installs.

use std::sync::Arc;

use freecell_chart_model::ChartSpec;
use freecell_core::SheetId;

/// The published set of live-bound charts, grouped by the sheet they're anchored on. Each
/// [`ChartSpec`]'s `chart` field carries the **current** (live-resolved) values; the rest of the
/// envelope (retained source, anchor, origin) is unchanged from load, so the UI derives fidelity and
/// places the chart exactly as in P8.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartSnapshot {
    /// Bumped **only** when the bound charts change (on load, and on each dirty re-resolve). The UI
    /// installs into the grid iff this differs from what it last installed. The empty initial
    /// snapshot is version `0`; a file with charts publishes version `1` on load.
    pub version: u64,
    /// Charts to paint, keyed by their anchor [`SheetId`]. Each per-sheet list is an
    /// `Arc<[ChartSpec]>` so the UI installs it into `GridView::set_sheet_charts` by **sharing the
    /// same allocation** (a refcount bump, not a deep copy) — the grid holds no independent copy of
    /// its charts' render pictures or retained source (charts/architecture §5 challenge 5,
    /// "off-screen free").
    pub sheets: Vec<(SheetId, Arc<[ChartSpec]>)>,
}

impl ChartSnapshot {
    /// The pre-load / chart-less snapshot (version `0`, no charts).
    pub fn empty() -> Self {
        Self::default()
    }
}
