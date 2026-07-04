//! Phase-12 perf harness — the POC "Run Test" scenario against the **real** grid + a
//! **1M×100 styled fixture**, plus the "zero engine calls on the scroll path" gate
//! (`architecture.md §4, §9`; ported scenario from `experiments/04-ui-poc`).
//!
//! This module owns the two engine-side pieces the driver ([`crate::bin`] `perf_harness`)
//! composes with the real `GridView::measure_frame`:
//!
//! 1. [`build_fixture`] — builds a **1M×100 styled** sheet through the real
//!    [`DocumentClient`]: variable column widths (incl. wide cols), dense **col-band styles**
//!    across all 100 cols (so every visible cell at any scroll depth is styled — a
//!    near-worst-case element build the whole sweep), a spread of row-height overrides
//!    (variable geometry at 1M scale), and a densely valued+styled top band, published so the
//!    heaviest frames carry real text + a large (`O(published)`) publication scan. It keeps
//!    the worker **alive** so the harness can read the engine-call counter and run the
//!    negative control.
//! 2. Environment stamping + JSON recording (`freecell-core` stays serde-free, so the report
//!    is serialized here) + the committed, buffered CI thresholds.
//!
//! ### Measurement reality (lavapipe)
//!
//! This runs under Mesa lavapipe (software Vulkan) — GPU *present* is NOT representative of
//! real hardware, so the harness measures the **CPU render-build path** (data resolution +
//! element construction, exactly what the POC measured) and the **engine-call counter**
//! (fully representative). See `DECISIONS_TO_REVIEW.md` (Phase 12).

use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use arc_swap::ArcSwap;
use std::sync::Arc;

use freecell_app::grid::GridDataSources;
use freecell_core::perf::PerfConfig;
use freecell_core::{Align, CellRange, CellRef, RenderStyle, Rgb, SheetId};
use freecell_engine::{
    Command, DocumentClient, DocumentSource, StyleAttr, WorkerEvent, WorkerEventReceiver,
};

// ---------------------------------------------------------------------------------------
// Committed, buffered CI gate thresholds — CALIBRATED on the pinned Linux runner image
// (Mesa lavapipe, CPU render-build path) at ~2× the measured p99/max (`architecture.md §9`,
// the "buffered gate" product call). Real-hardware budgets (8.33 ms / 16.67 ms / 2 ms) stay
// the product truth (`freecell_core::perf::{FRAME_TARGET_NS, FRAME_WORST_NS,
// CELL_LOAD_TARGET_NS}`), checked on macos-verify. Recalibrate only deliberately (a committed
// change with rationale), never to quiet a regression.
//
// CANONICAL calibration run — one run, cited identically here, in DECISIONS_TO_REVIEW.md
// (Phase 12), and in render-tests/results/perf-runtest.json:
//   2026-07-04, Intel Xeon @ 2.80 GHz, 4 cores, ubuntu-24.04-class, rustc 1.95.0, release,
//   lavapipe — frame-build p50 1.89 ms / p99 5.56 ms / max 5.80 ms; cell-load p99 93.6 µs.
// (These comfortably meet the real-hardware §4 budgets even on this slow shared CPU — the
// buffered gate exists for runner-to-runner variance, not because the budget is at risk.)
// ---------------------------------------------------------------------------------------

/// CI frame-build p99 gate (ns) ≈ 2× the calibrated 5.56 ms p99 (11.5 ms = 2.07× p99).
pub const CI_FRAME_P99_NS: u64 = 11_500_000;
/// CI worst frame-build gate (ns) ≈ 2× the calibrated 5.80 ms max (13 ms = 2.24× max).
pub const CI_FRAME_MAX_NS: u64 = 13_000_000;
/// CI cell-load p99 gate (ns). The calibrated p99 is a ~93.6 µs micro-measurement dominated
/// by scheduler/cache noise, so a strict 2× (~0.19 ms) would be flaky across runner CPUs; this
/// 0.5 ms floor (~5.3× calibrated) still catches a real regression yet stays 4× under the
/// 2 ms product budget. Documented deviation-from-strict-2× (DECISIONS_TO_REVIEW.md, Phase 12).
pub const CI_CELL_LOAD_P99_NS: u64 = 500_000;

/// The value-band depth (rows) of the fixture — the rows carrying real engine values, and the
/// published window's height. Deliberately larger than a typical viewport overscan (a bounded,
/// within-`MAX_PUBLISH` stress of the `O(published-cells)` per-frame publication scan the grid
/// runs — `grid/view.rs`), so the heaviest frames exercise it.
pub const FIXTURE_VALUE_ROWS: u32 = 256;

/// How long to wait for the worker's initial `Loaded` event.
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// The idle gap that signals the worker finished the fixture's commands.
const IDLE_GAP: Duration = Duration::from_millis(250);
/// A hard cap on the total drain, so a misbehaving worker fails the harness instead of hanging.
const DRAIN_CAP: Duration = Duration::from_secs(120);

/// A live perf fixture: the shared read-surfaces the grid renders from, plus the still-running
/// worker (kept alive so the harness can read `engine_call_count()` and run the negative
/// control). The `active_sheet` is the sheet the publication + cache cover.
pub struct Fixture {
    client: DocumentClient,
    events: WorkerEventReceiver,
    caches: Arc<parking_lot::RwLock<freecell_core::SheetCaches>>,
    publication: Arc<ArcSwap<freecell_core::Publication>>,
    pub sheet: SheetId,
}

impl Fixture {
    /// The grid data sources over this fixture's live read-surfaces (the same `Arc`s the worker
    /// writes — the render path reads them wait-free / under a brief lock, never the engine).
    pub fn sources(&self) -> GridDataSources {
        GridDataSources {
            publication: Arc::clone(&self.publication),
            caches: Arc::clone(&self.caches),
        }
    }

    /// The live client — used by the driver's negative control (send one real edit and watch the
    /// engine-call counter climb).
    pub fn client(&self) -> &DocumentClient {
        &self.client
    }

    /// The worker event receiver (drained after the negative-control edit).
    pub fn events(&self) -> &WorkerEventReceiver {
        &self.events
    }

    /// Drain worker events until the queue stays empty for [`IDLE_GAP`] (the worker went idle).
    pub fn drain_to_idle(&self) -> Result<()> {
        drain_to_idle(&self.events)
    }
}

/// Builds the **1M×100 styled** perf fixture through the real engine. `value_rows` is the top
/// band of real engine values (and the published window height); the real harness passes
/// [`FIXTURE_VALUE_ROWS`], unit tests pass a tiny value.
///
/// The worker is left **running** (no `Shutdown`) so the caller can read `engine_call_count()`
/// and run the negative control; drop the returned [`Fixture`] to release it.
pub fn build_fixture(cfg: &PerfConfig, value_rows: u32) -> Result<Fixture> {
    let cols = cfg.cols;
    let (client, events) = DocumentClient::spawn(DocumentSource::NewWorkbook);

    let sheet = loop {
        match events.recv_timeout(LOAD_TIMEOUT) {
            Some(WorkerEvent::Loaded { sheets }) => {
                break sheets.first().map(|m| m.id).unwrap_or(SheetId(0));
            }
            Some(WorkerEvent::LoadFailed { error }) => bail!("worker load failed: {error}"),
            Some(_) => continue,
            None => bail!("worker never emitted Loaded within {LOAD_TIMEOUT:?}"),
        }
    };

    // Real engine VALUES across the top band (a mix of numbers / short + long text so the frame
    // build shapes real, varied strings — wide columns land on the long ones). Every cell in the
    // 0..cols region is populated so the published window is dense.
    for row in 0..value_rows {
        for col in 0..cols {
            client.send(Command::SetCellInput {
                sheet,
                cell: CellRef::new(row, col),
                input: cell_value(row, col),
            });
        }
    }

    // Publish the whole valued band (clamped worker-side to MAX_PUBLISH = 512×256).
    client.send(Command::SetViewport {
        sheet,
        rows: 0..value_rows,
        cols: 0..cols,
    });

    drain_to_idle(&events)?;

    let publication = client.publication_swap();
    let caches = client.caches();

    // Command-less render features injected into the real cache the grid consumes (the same
    // public mutators the worker uses; alignment / explicit geometry have no MVP edit command —
    // in the product they arrive from an opened file). This is how `render-tests/scene.rs` also
    // realizes geometry/alignment.
    apply_dense_styling(&caches, sheet, cols);

    Ok(Fixture {
        client,
        events,
        caches,
        publication,
        sheet,
    })
}

/// The value for a fixture cell: mostly numbers, some short words, and a long string on a
/// rotating column so wide columns carry text-shaping-heavy content (`architecture.md §10`).
fn cell_value(row: u32, col: u32) -> String {
    match (row.wrapping_mul(31).wrapping_add(col)) % 7 {
        0 => format!("{}", row * 100 + col),
        1 => format!("{}.{:02}", row, col % 100),
        2 => "Item".to_string(),
        3 => "Total".to_string(),
        4 => format!("R{row}C{col}"),
        5 => "the quick brown fox jumps".to_string(),
        _ => format!("=-{}", row + col + 1), // a literal negative number (no cross-refs to keep eval cheap)
    }
}

/// Inject the dense, whole-sheet styling + geometry into the real `SheetCache`: per-column band
/// styles (so EVERY row's cells in `0..cols` are styled, cheaply — `cols` band entries, not
/// `1M×cols` cell entries), variable column widths incl. wide cols, and a spread of row-height
/// overrides across the 1M rows (variable geometry exercising the two-level axis).
fn apply_dense_styling(
    caches: &parking_lot::RwLock<freecell_core::SheetCaches>,
    sheet: SheetId,
    cols: u32,
) {
    let mut guard = caches.write();
    let Some(cache) = guard.get_mut(sheet) else {
        return;
    };

    for col in 0..cols {
        cache.set_col_band_style(col, band_style(col));
        cache.set_col_width(col, col_width(col));
    }

    // ~1000 row-height overrides spread across the whole 1M range (every 1000th row), so the
    // row axis is genuinely variable at scale (not a flat prefix sum).
    let overrides: Vec<(u32, Option<f32>)> = (0..1000)
        .map(|i| (i * 1000, Some(24.0 + (i % 3) as f32 * 8.0)))
        .collect();
    cache.set_row_heights(&overrides);
}

/// A rotating per-column band style: a spread of fills / bold / italic / underline / alignment
/// so the sweep builds fully-styled cells at every depth.
fn band_style(col: u32) -> RenderStyle {
    match col % 5 {
        0 => RenderStyle {
            fill: Some(Rgb::from_hex(0xFFF2CC)),
            ..RenderStyle::default()
        },
        1 => RenderStyle {
            bold: true,
            ..RenderStyle::default()
        },
        2 => RenderStyle {
            italic: true,
            ..RenderStyle::default()
        },
        3 => RenderStyle {
            h_align: Some(Align::Right),
            font_color: Some(Rgb::from_hex(0x1155CC)),
            ..RenderStyle::default()
        },
        _ => RenderStyle {
            underline: true,
            ..RenderStyle::default()
        },
    }
}

/// A rotating column width in px: every 7th column is wide (300 px) to stress text shaping;
/// the rest vary 90–150 px.
fn col_width(col: u32) -> f32 {
    if col.is_multiple_of(7) {
        300.0
    } else {
        90.0 + (col % 6) as f32 * 12.0
    }
}

/// Drain worker events until idle for [`IDLE_GAP`]; a `DRAIN_CAP` overrun is a hard error (the
/// fixture may be incomplete). No well-behaved fixture churns events for `DRAIN_CAP`.
fn drain_to_idle(events: &WorkerEventReceiver) -> Result<()> {
    let deadline = Instant::now() + DRAIN_CAP;
    loop {
        match events.recv_timeout(IDLE_GAP) {
            Some(_) => {
                if Instant::now() >= deadline {
                    bail!("worker still emitting events after {DRAIN_CAP:?}; fixture may be incomplete");
                }
            }
            None => return Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------------------
// A style-attr edit for the negative control (proves the engine-call counter CAN climb).
// ---------------------------------------------------------------------------------------

/// A single real edit the driver sends as the negative control for the zero-engine-calls gate.
pub fn negative_control_edit(sheet: SheetId) -> Command {
    // A style toggle over one cell: applies + publishes on the worker (touches the engine), so
    // `engine_call_count()` must climb — proving the sweep's zero-delta assertion isn't vacuous.
    Command::SetStyleAttr {
        sheet,
        range: CellRange::single(CellRef::new(0, 0)),
        attr: StyleAttr::Bold,
    }
}

// ---------------------------------------------------------------------------------------
// Environment stamp + JSON recording.
// ---------------------------------------------------------------------------------------

/// The environment stamp for the recorded report (`CLAUDE.md`: environment-stamped numbers).
pub fn environment_json(commit: &str) -> serde_json::Value {
    serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "cpu": cpu_model(),
        "rustc": env!("FREECELL_RUSTC_VERSION"),
        "renderer": "assumed Mesa lavapipe / software Vulkan (ICD presence enforced by perf.sh) — CPU render-build path measured; GPU present NOT measured",
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "commit": commit,
    })
}

/// Best-effort CPU model name (Linux `/proc/cpuinfo`), else empty.
fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|txt| {
            txt.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture builder yields a densely styled 1M-dim cache + a non-empty engine-produced
    /// publication — the substrate the perf sweep measures. Tiny config so it is fast in
    /// `cargo test --workspace`.
    #[test]
    fn fixture_is_dense_styled_and_published() {
        let cfg = PerfConfig {
            cols: 8,
            ..PerfConfig::default()
        };
        let fixture = build_fixture(&cfg, 6).expect("fixture builds");
        let sources = fixture.sources();

        // The sheet is 1M rows (Excel-max) — the virtualization stress.
        let caches = sources.caches.read();
        let cache = caches
            .get(fixture.sheet)
            .expect("active-sheet cache resident");
        assert_eq!(cache.dims().0, 1_048_576, "the fixture is 1M rows deep");

        // Dense col-band styling: a FAR row (500,000) still resolves a style in the styled cols
        // (proves the band, not per-cell — the whole sweep builds styled cells).
        assert!(
            cache.render_style(500_000, 0).is_some(),
            "col-band styling must cover every row"
        );
        // Variable geometry: a wide column + a row-height override are present.
        assert_eq!(cache.col_width(0), 300.0, "col 0 is a wide column");
        assert!(cache.col_width(1) < 300.0, "col 1 is a normal width");

        // The publication is engine-produced and non-empty (real values in the top band).
        let publication = sources.publication.load();
        assert!(
            !publication.cells.is_empty(),
            "the valued band must publish real cells"
        );
        assert!(
            publication.cells.iter().all(|c| c.row < 6 && c.col < 8),
            "published cells are inside the valued band"
        );
    }
}
