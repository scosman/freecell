//! P11 chart perf harness — measures the three exit-criterion chart ops and reports **p50/p99**,
//! environment-stamped (`charts/architecture.md §5` challenge 5, `functional_spec.md §8`, `CLAUDE.md`
//! benchmark conventions).
//!
//! **Headless** (no GPU / no window): all three ops are CPU work off the render path, so this runs
//! directly under a `timeout` — no Xvfb/lavapipe needed:
//!
//! ```text
//! cargo run -p render-tests --release --bin chart_perf
//! ```
//!
//! - **first-paint** — the deferred, off-critical-path parse of a sheet's charts:
//!   `discover_and_parse_for_sheet` + bind + build the published snapshot. (Cells paint first; this
//!   is the work that now happens a frame *after* first paint.)
//! - **edit-rerender** — the engine edit path a source-cell change drives: dirty-set intersection +
//!   re-resolve the intersecting chart + rebuild the snapshot (the `Arc`-shared source keeps this
//!   O(chart values), not O(source bytes)).
//! - **scroll-with-K** — the per-frame ChartLayer cull scan over **K = 1000** charts: map each
//!   placement's anchor to a rect and cull the off-screen ones, materializing only the on-screen few.
//!
//! Every op is FORCE + ASSERTED (the measured work provably happened — a scan that measured nothing
//! would trip an assert), per `CLAUDE.md`. Run FOREGROUND; never background it. No hard target is
//! gated here — the p50/p99 targets are ratified at the post-P11 human checkpoint; the scroll scan is
//! reported against the repo's existing frame budgets for reference.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use freecell_app::grid::chart_layer::{anchor_rect, ChartPlacement, GridGeometry};
use freecell_chart_model::{
    cap_markers_for_paint, downsample_for_paint, Anchor, AnchorCell, Axis, Category, Chart,
    ChartKind, ChartSpec, Grouping, Legend, Series, SourceXml, MAX_PAINT_MARKERS,
    MAX_PAINT_VERTICES,
};
use freecell_core::perf::{fmt_ns, LatencyStats, FRAME_TARGET_NS, FRAME_WORST_NS};
use freecell_core::{CellRange, CellRef, SheetId};
use freecell_engine::chart::binding::{CellData, ChartBindings};
use freecell_engine::chart::{authoring, discover_and_parse, discover_and_parse_for_sheet};

/// Charts held on the stress sheet for the scroll-with-K measurement.
const K: usize = 1000;

/// Line charts on one sheet for the many-line-charts OPEN measurement (real zip + XML parse each).
const MANY_K: usize = 200;

/// Points in the large-series line for the paint-prep / down-sample measurement.
const BIG_N: usize = 100_000;

fn main() {
    let commit = std::env::var("FREECELL_COMMIT")
        .or_else(|_| std::env::var("POC_COMMIT"))
        .unwrap_or_else(|_| "unknown".to_string());

    // One on-disk line fixture the file-reading ops share (no `tempfile` — a bin can't use the
    // crate's dev-dependency). Cleaned up before exit.
    let fixture =
        std::env::temp_dir().join(format!("freecell_chart_perf_{}.xlsx", std::process::id()));
    authoring::write_line_fixture(&fixture).expect("write line fixture");

    // The many-line-charts open fixture: K real line charts on one sheet, written once.
    let many_fixture = std::env::temp_dir().join(format!(
        "freecell_chart_perf_many_{}.xlsx",
        std::process::id()
    ));
    authoring::write_many_line_charts_fixture(&many_fixture, MANY_K).expect("write many fixture");

    let first_paint = measure_first_paint(&fixture);
    let edit_rerender = measure_edit_rerender(&fixture);
    let scroll = measure_scroll_with_k();
    let many = measure_many_line_charts(&many_fixture);
    let large = measure_large_series();
    let cloud = measure_bubble_cloud();

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_file(&many_fixture);

    println!("=== chart perf (headless, CPU render/engine path) ===");
    print_op(
        "first-paint (discover+parse+bind+snapshot, 1 line chart)",
        &first_paint,
        None,
    );
    print_op(
        "edit-rerender (dirty-set + reresolve + snapshot)",
        &edit_rerender,
        None,
    );
    print_op(
        &format!("scroll-with-K (per-frame cull scan over K={K} charts)"),
        &scroll.stats,
        Some((FRAME_TARGET_NS, FRAME_WORST_NS)),
    );
    println!(
        "  scroll: {} charts total, ~{} on-screen per frame (rest culled), {} distinct scroll positions",
        K, scroll.max_on_screen, scroll.distinct
    );
    print_op(
        &format!("many-line-charts (open: discover+parse {MANY_K} line charts on one sheet)"),
        &many,
        None,
    );
    println!("  many-line-charts: {MANY_K} charts parsed per open (real zip + XML parse each)");
    print_op(
        &format!("large-series down-sample (N={BIG_N} -> <= {MAX_PAINT_VERTICES} vertices)"),
        &large.downsample,
        None,
    );
    print_op(
        &format!("large-series paint-prep, FULL N={BIG_N} (pre-hardening per-frame CPU)"),
        &large.prep_full,
        Some((FRAME_TARGET_NS, FRAME_WORST_NS)),
    );
    print_op(
        &format!(
            "large-series paint-prep, DOWN-SAMPLED to {} (post-hardening per-frame CPU)",
            large.kept
        ),
        &large.prep_downsampled,
        Some((FRAME_TARGET_NS, FRAME_WORST_NS)),
    );
    println!(
        "  large-series: {} points retained for save; {} vertices painted (down-sample ~{:.0}x fewer)",
        large.n,
        large.kept,
        large.n as f64 / large.kept as f64,
    );
    print_op(
        &format!("bubble/scatter cloud paint-prep, FULL N={BIG_N} (pre-cap per-frame CPU)"),
        &cloud.prep_full,
        Some((FRAME_TARGET_NS, FRAME_WORST_NS)),
    );
    print_op(
        &format!(
            "bubble/scatter cloud paint-prep, CAPPED to {} (post-cap per-frame CPU, C-P25-1)",
            cloud.kept
        ),
        &cloud.prep_capped,
        Some((FRAME_TARGET_NS, FRAME_WORST_NS)),
    );
    println!(
        "  cloud: {} points retained for save; {} marks painted (cap ~{:.0}x fewer)",
        cloud.n,
        cloud.kept,
        cloud.n as f64 / cloud.kept as f64,
    );

    if let Err(e) = write_json(
        &commit,
        &first_paint,
        &edit_rerender,
        &scroll,
        &many,
        &large,
        &cloud,
    ) {
        eprintln!("chart_perf: failed to write results JSON: {e}");
    }
}

/// Report p50/p99 for one op (+ optional reference frame budgets).
fn print_op(name: &str, stats: &LatencyStats, budgets: Option<(u64, u64)>) {
    print!(
        "  {name}: p50 {}  p99 {}  (max {}, n={})",
        fmt_ns(stats.p50_ns),
        fmt_ns(stats.p99_ns),
        fmt_ns(stats.max_ns),
        stats.count,
    );
    if let Some((target, worst)) = budgets {
        print!(
            "  [ref frame budget {} / {}]",
            fmt_ns(target),
            fmt_ns(worst)
        );
    }
    println!();
}

// ---------------------------------------------------------------------------------------------
// first-paint
// ---------------------------------------------------------------------------------------------

fn measure_first_paint(path: &Path) -> LatencyStats {
    let mut samples = Vec::new();
    for i in 0..300 {
        let started = Instant::now();
        // The deferred, off-critical-path work: walk + parse the sheet's charts, bind them, and
        // build the snapshot the UI installs.
        let specs = discover_and_parse_for_sheet(path, "Data").expect("discover");
        let mut bindings = ChartBindings::default();
        let added = bindings.add_missing(vec![(SheetId(0), specs)]);
        let snapshot = bindings.specs_by_sheet();
        let elapsed = started.elapsed().as_nanos() as u64;

        // FORCE + ASSERT: the chart was actually discovered, bound, and published with its cached
        // values — a no-op parse would fail here.
        assert!(added, "first-paint must bind the line chart");
        assert_eq!(snapshot.len(), 1, "one anchor sheet");
        assert_eq!(snapshot[0].1.len(), 1, "one line chart");
        assert_eq!(
            first_values(&snapshot[0].1[0]),
            authoring::WIDGETS.to_vec(),
            "first paint carries the file's cached Widgets values",
        );
        if i >= 20 {
            samples.push(elapsed); // drop warmup iterations
        }
    }
    LatencyStats::from_samples(&samples)
}

// ---------------------------------------------------------------------------------------------
// edit-rerender
// ---------------------------------------------------------------------------------------------

fn measure_edit_rerender(path: &Path) -> LatencyStats {
    let specs = discover_and_parse_for_sheet(path, "Data").expect("discover");
    let mut bindings = ChartBindings::default();
    bindings.add_missing(vec![(SheetId(0), specs)]);

    // A resolver mapping the data sheet's name to its id; a reader that returns fresh values per
    // iteration so every re-resolve is a real change (never a measured no-op).
    let resolver = |name: &str| (name == "Data").then_some(SheetId(0));
    let edited = [(SheetId(0), range("B2"))];

    let mut samples = Vec::new();
    for i in 0..3000u64 {
        let vary = i as f64;
        let reader = move |_sheet: SheetId, cell: CellRef| -> CellData {
            if cell.col == 0 {
                CellData::Text(format!("Q{}", cell.row + 1)) // categories (col A)
            } else {
                CellData::Number(vary + cell.row as f64) // values (cols B/C)
            }
        };

        let started = Instant::now();
        let dirty = bindings.dirty_indices(&edited, &[], &resolver);
        let changed = bindings.reresolve(&dirty, &resolver, &reader);
        let snapshot = bindings.specs_by_sheet();
        let elapsed = started.elapsed().as_nanos() as u64;

        // FORCE + ASSERT: the edit selected the chart, changed it, and republished the new value.
        assert_eq!(dirty, vec![0], "the B2 edit selects the line chart");
        assert!(changed, "the re-resolve changed the chart");
        assert_eq!(
            first_values(&snapshot[0].1[0])[0],
            vary + 1.0,
            "B2 (row 1) reflects the fresh value",
        );
        if i >= 200 {
            samples.push(elapsed);
        }
    }
    LatencyStats::from_samples(&samples)
}

// ---------------------------------------------------------------------------------------------
// scroll-with-K
// ---------------------------------------------------------------------------------------------

/// A uniform grid geometry (80 px columns, 20 px rows) for the anchor→pixel mapping.
struct UniformGeom;
impl GridGeometry for UniformGeom {
    fn col_start(&self, col: u32) -> f64 {
        col as f64 * 80.0
    }
    fn row_start(&self, row: u32) -> f64 {
        row as f64 * 20.0
    }
    fn col_at(&self, x: f64) -> u32 {
        (x.max(0.0) / 80.0).floor() as u32
    }
    fn row_at(&self, y: f64) -> u32 {
        (y.max(0.0) / 20.0).floor() as u32
    }
}

struct ScrollResult {
    stats: LatencyStats,
    max_on_screen: usize,
    distinct: usize,
}

fn measure_scroll_with_k() -> ScrollResult {
    // K charts spread down a tall sheet: chart i spans rows (i*30)..(i*30+15), cols 0..8 → a 600 px
    // vertical pitch with a 300 px-tall chart, so a 600 px viewport shows ~1 at a time.
    let specs: Arc<[ChartSpec]> = (0..K)
        .map(|i| {
            let top = (i as u32) * 30;
            ChartSpec::loaded(
                stress_line_chart(i),
                SourceXml::new("<c:lineChart/>"),
                Vec::new(),
                Anchor::new(AnchorCell::new(0, top), AnchorCell::new(8, top + 15)),
            )
        })
        .collect();
    // The grid's per-frame scan reads only the tiny placements (anchor + fidelity) — the heavy
    // render `Chart` stays in the shared `specs` and is touched only for on-screen charts.
    let placements: Vec<ChartPlacement> = specs.iter().map(ChartPlacement::from_spec).collect();

    let geom = UniformGeom;
    let (content_w, content_h) = (800.0_f64, 600.0_f64);

    let mut samples = Vec::new();
    let mut max_on_screen = 0usize;
    let mut ever_on_screen = false;
    let mut distinct = std::collections::HashSet::new();
    // Sweep the scroll down the sheet in prime-ish steps so many distinct viewports are visited.
    let step = 137.0_f64;
    let sheet_px = (K as f64) * 30.0 * 20.0;
    let mut scroll_y = 0.0_f64;
    let mut iter = 0;
    while scroll_y < sheet_px {
        let started = Instant::now();
        // The per-frame ChartLayer cull scan: O(K) over placements, materializing only the visible.
        let mut on_screen = 0usize;
        let mut checksum = 0.0_f64;
        for (i, placement) in placements.iter().enumerate() {
            let rect = anchor_rect(&placement.anchor, &geom, 0.0, scroll_y);
            if rect.is_offscreen(content_w, content_h) {
                continue;
            }
            on_screen += 1;
            // Materialize: touch the heavy render `Chart` only for on-screen charts (the "off-screen
            // free" boundary). Summing a value both proves the access and defeats dead-code removal.
            checksum += first_values(&specs[i])[0];
        }
        let elapsed = started.elapsed().as_nanos() as u64;
        std::hint::black_box(checksum);

        max_on_screen = max_on_screen.max(on_screen);
        ever_on_screen |= on_screen > 0;
        distinct.insert(scroll_y.to_bits());
        if iter >= 20 {
            samples.push(elapsed);
        }
        scroll_y += step;
        iter += 1;
    }

    // FORCE + ASSERT: the sweep scanned all K charts, visited many viewports, actually materialized
    // charts (so we didn't measure an empty loop), and — crucially for "off-screen free" — only a
    // handful were ever on-screen at once (the rest culled).
    assert_eq!(placements.len(), K, "all K charts are in the scan");
    assert!(
        ever_on_screen,
        "the sweep materialized on-screen charts (not an empty measurement)"
    );
    assert!(
        max_on_screen <= 5,
        "at most a handful of the {K} charts are ever on-screen (got {max_on_screen}) — the rest are freed by the cull",
    );
    assert!(
        distinct.len() > 100,
        "the sweep visits many distinct scroll positions"
    );

    ScrollResult {
        stats: LatencyStats::from_samples(&samples),
        max_on_screen,
        distinct: distinct.len(),
    }
}

// ---------------------------------------------------------------------------------------------
// many-line-charts (open at scale)
// ---------------------------------------------------------------------------------------------

/// The open cost for a sheet bearing K line charts: `discover_and_parse` walks the package and
/// parses all K chart parts. Reports the whole-open latency, FORCE+ASSERTING all K were parsed as
/// line charts with their cached values (a no-op parse would fail here).
fn measure_many_line_charts(path: &Path) -> LatencyStats {
    let mut samples = Vec::new();
    for i in 0..120 {
        let started = Instant::now();
        let specs = discover_and_parse(path).expect("discover many line charts");
        let elapsed = started.elapsed().as_nanos() as u64;

        assert_eq!(specs.len(), MANY_K, "all K line charts are discovered");
        assert!(
            specs
                .iter()
                .all(|s| matches!(s.chart().map(|c| &c.kind), Some(ChartKind::Line { .. }))),
            "every discovered chart is a line chart"
        );
        assert_eq!(
            first_values(&specs[0]),
            authoring::WIDGETS.to_vec(),
            "each chart carries its cached values"
        );
        if i >= 10 {
            samples.push(elapsed);
        }
    }
    LatencyStats::from_samples(&samples)
}

// ---------------------------------------------------------------------------------------------
// large-series (paint-prep + down-sample)
// ---------------------------------------------------------------------------------------------

struct LargeSeriesResult {
    /// Cost of `downsample_for_paint(N points)`.
    downsample: LatencyStats,
    /// Per-frame paint-prep mapping ALL N points to pixels (pre-hardening).
    prep_full: LatencyStats,
    /// Per-frame paint-prep mapping only the down-sampled points (post-hardening).
    prep_downsampled: LatencyStats,
    /// Points retained (N — the full series kept for save).
    n: usize,
    /// Vertices painted after decimation.
    kept: usize,
}

/// Large-series perf: a line with **N points**. Measures (1) the down-sample cost, (2) the
/// per-frame point-mapping over ALL N (what the renderer did before the P15 hardening), and (3) the
/// same mapping over the down-sampled vertices (what it does now). FORCE+ASSERTS that the full
/// series (N points) is retained — the down-sample is paint-only, so save fidelity is untouched —
/// while paint touches `<= MAX_PAINT_VERTICES`.
fn measure_large_series() -> LargeSeriesResult {
    // A dense sine wave so the shape (and its extrema) is non-trivial to preserve.
    let values: Vec<f64> = (0..BIG_N)
        .map(|i| ((i as f64) * 0.001).sin() * 100.0 + 100.0)
        .collect();
    // A representative linear value->pixel map (min..max -> plot_bottom..plot_top), the arithmetic
    // the renderer's `ScaleLinear` does per point.
    let (min, max) = (0.0_f64, 200.0_f64);
    let (plot_bottom, plot_h) = (400.0_f32, 380.0_f32);
    let y_px = |v: f64| -> f32 { plot_bottom - (((v - min) / (max - min)) as f32) * plot_h };

    // 1. down-sample cost.
    let mut ds = Vec::new();
    let mut kept = 0usize;
    for i in 0..300 {
        let started = Instant::now();
        let keep = downsample_for_paint(&values, MAX_PAINT_VERTICES);
        let elapsed = started.elapsed().as_nanos() as u64;
        kept = keep.len();
        assert!(
            keep.len() <= MAX_PAINT_VERTICES && *keep.last().unwrap() == BIG_N - 1,
            "decimated to <= budget, keeping the last point"
        );
        std::hint::black_box(keep);
        if i >= 20 {
            ds.push(elapsed);
        }
    }

    // 2. paint-prep over ALL N (pre-hardening).
    let mut full = Vec::new();
    for i in 0..300 {
        let started = Instant::now();
        let pts: Vec<f32> = (0..BIG_N)
            .filter(|&j| values[j].is_finite())
            .map(|j| y_px(values[j]))
            .collect();
        let elapsed = started.elapsed().as_nanos() as u64;
        assert_eq!(pts.len(), BIG_N, "full prep maps every point");
        std::hint::black_box(pts);
        if i >= 20 {
            full.push(elapsed);
        }
    }

    // 3. paint-prep over the down-sampled vertices (post-hardening).
    let keep = downsample_for_paint(&values, MAX_PAINT_VERTICES);
    let mut down = Vec::new();
    for i in 0..300 {
        let started = Instant::now();
        let pts: Vec<f32> = keep
            .iter()
            .filter(|&&j| values[j].is_finite())
            .map(|&j| y_px(values[j]))
            .collect();
        let elapsed = started.elapsed().as_nanos() as u64;
        assert_eq!(
            pts.len(),
            keep.len(),
            "down-sampled prep maps the kept points"
        );
        std::hint::black_box(pts);
        if i >= 20 {
            down.push(elapsed);
        }
    }

    // FORCE + ASSERT: the full series is retained for save (paint-only decimation).
    assert_eq!(
        values.len(),
        BIG_N,
        "the full N-point series is retained for save"
    );
    assert!(kept < BIG_N, "paint decimates below the full series");

    LargeSeriesResult {
        downsample: LatencyStats::from_samples(&ds),
        prep_full: LatencyStats::from_samples(&full),
        prep_downsampled: LatencyStats::from_samples(&down),
        n: BIG_N,
        kept,
    }
}

// ---------------------------------------------------------------------------------------------
// bubble/scatter cloud paint-prep + cloud cap (GAPS C-P25-1)
// ---------------------------------------------------------------------------------------------

struct CloudResult {
    /// Per-frame paint-prep mapping ALL N cloud points to (pixel + radius) (pre-cap).
    prep_full: LatencyStats,
    /// Per-frame paint-prep mapping only the capped subset (post-cap).
    prep_capped: LatencyStats,
    /// Points retained (N — the full cloud kept for save).
    n: usize,
    /// Marks painted after the cloud cap.
    kept: usize,
}

/// Cloud paint-prep perf (GAPS C-P25-1): a scatter/bubble with **N points**. Unlike the line
/// renderer, the scatter/bubble mark loop maps + draws one mark **per point** with no cap — so this
/// measures (1) mapping ALL N points to a pixel + radius (what the renderer did before the P26 cap),
/// and (2) the same over the `cap_markers_for_paint` subset (what it does now). FORCE+ASSERTS that
/// the full cloud (N points) is retained (the cap is paint-only) while paint touches
/// `<= MAX_PAINT_MARKERS`.
fn measure_bubble_cloud() -> CloudResult {
    // A dense spiral cloud with a size per point — a non-trivial (x, y, size) set.
    let xs: Vec<f64> = (0..BIG_N)
        .map(|i| ((i as f64) * 0.01).cos() * 500.0)
        .collect();
    let ys: Vec<f64> = (0..BIG_N)
        .map(|i| ((i as f64) * 0.01).sin() * 500.0)
        .collect();
    let sizes: Vec<f64> = (0..BIG_N).map(|i| (i % 97) as f64 + 1.0).collect();
    let max_size = sizes.iter().copied().fold(0.0_f64, f64::max);
    let (xmin, xmax) = (-500.0_f64, 500.0_f64);
    let (ymin, ymax) = (-500.0_f64, 500.0_f64);
    let (plot_left, plot_w) = (46.0_f32, 700.0_f32);
    let (plot_bottom, plot_h) = (400.0_f32, 380.0_f32);
    let x_px = |v: f64| -> f32 { plot_left + (((v - xmin) / (xmax - xmin)) as f32) * plot_w };
    let y_px = |v: f64| -> f32 { plot_bottom - (((v - ymin) / (ymax - ymin)) as f32) * plot_h };
    // The √-area radius map (the bubble renderer's arithmetic), clamped like the widget.
    let r_px = |s: f64| -> f32 { ((s / max_size).sqrt() as f32 * 26.0).clamp(4.0, 26.0) };

    // 1. paint-prep over ALL N (pre-cap): map every point to (x, y, r).
    let mut full = Vec::new();
    for i in 0..300 {
        let started = Instant::now();
        let marks: Vec<(f32, f32, f32)> = (0..BIG_N)
            .filter(|&j| xs[j].is_finite() && ys[j].is_finite())
            .map(|j| (x_px(xs[j]), y_px(ys[j]), r_px(sizes[j])))
            .collect();
        let elapsed = started.elapsed().as_nanos() as u64;
        assert_eq!(marks.len(), BIG_N, "full prep maps every cloud point");
        std::hint::black_box(marks);
        if i >= 20 {
            full.push(elapsed);
        }
    }

    // 2. paint-prep over the CAPPED subset (post-cap).
    let keep = cap_markers_for_paint(BIG_N, MAX_PAINT_MARKERS);
    let kept = keep.len();
    let mut capped = Vec::new();
    for i in 0..300 {
        let started = Instant::now();
        let marks: Vec<(f32, f32, f32)> = keep
            .iter()
            .filter(|&&j| xs[j].is_finite() && ys[j].is_finite())
            .map(|&j| (x_px(xs[j]), y_px(ys[j]), r_px(sizes[j])))
            .collect();
        let elapsed = started.elapsed().as_nanos() as u64;
        assert_eq!(marks.len(), kept, "capped prep maps the kept marks");
        std::hint::black_box(marks);
        if i >= 20 {
            capped.push(elapsed);
        }
    }

    // FORCE + ASSERT: the full cloud is retained for save (the cap is paint-only), and the cap
    // bounds the mark count below the full cloud.
    assert_eq!(
        xs.len(),
        BIG_N,
        "the full N-point cloud is retained for save"
    );
    assert!(
        kept <= MAX_PAINT_MARKERS,
        "the cap bounds the marks to the budget"
    );
    assert!(kept < BIG_N, "the cap actually reduced the full cloud");

    CloudResult {
        prep_full: LatencyStats::from_samples(&full),
        prep_capped: LatencyStats::from_samples(&capped),
        n: BIG_N,
        kept,
    }
}

// ---------------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------------

fn stress_line_chart(i: usize) -> Chart {
    Chart {
        title: Some(format!("Chart {i}")),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![Series::category_value(
            Some("Series"),
            vec![
                Category::Text("A".into()),
                Category::Text("B".into()),
                Category::Text("C".into()),
            ],
            vec![i as f64, (i + 1) as f64, (i + 2) as f64],
        )],
        cat_axis: Axis::untitled(),
        val_axis: Axis::untitled(),
        legend: Some(Legend::default()),
    }
}

/// The first series' value list of a spec's chart (category/value → values, scatter → y).
fn first_values(spec: &ChartSpec) -> Vec<f64> {
    match &spec.chart().unwrap().series[0].data {
        freecell_chart_model::SeriesData::CategoryValue { values, .. } => values.clone(),
        freecell_chart_model::SeriesData::Xy { y, .. } => y.clone(),
    }
}

fn range(a1: &str) -> CellRange {
    CellRange::from_a1(a1).expect("valid A1 range")
}

#[allow(clippy::too_many_arguments)]
fn write_json(
    commit: &str,
    first_paint: &LatencyStats,
    edit_rerender: &LatencyStats,
    scroll: &ScrollResult,
    many: &LatencyStats,
    large: &LargeSeriesResult,
    cloud: &CloudResult,
) -> std::io::Result<()> {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("chart-perf.json");
    let json = serde_json::json!({
        "name": "freecell-chart-perf",
        "environment": render_tests::perf::environment_json(commit),
        "note": "chart perf: headless CPU render/engine path. P11 ops (first-paint/edit-rerender/scroll) + P15 many-line-charts (open at scale) and large-series (paint-prep + down-sample). Scroll/large-series prep are reported vs the repo's frame budgets for reference.",
        "ops": {
            "first_paint": stats_json(first_paint),
            "edit_rerender": stats_json(edit_rerender),
            "scroll_with_k": {
                "k": K,
                "max_on_screen": scroll.max_on_screen,
                "distinct_scroll_positions": scroll.distinct,
                "reference_frame_target_ns": FRAME_TARGET_NS,
                "reference_frame_worst_ns": FRAME_WORST_NS,
                "stats": stats_json(&scroll.stats),
            },
            "many_line_charts_open": {
                "k": MANY_K,
                "stats": stats_json(many),
            },
            "large_series": {
                "n_points_retained_for_save": large.n,
                "vertices_painted": large.kept,
                "max_paint_vertices": MAX_PAINT_VERTICES,
                "reference_frame_target_ns": FRAME_TARGET_NS,
                "reference_frame_worst_ns": FRAME_WORST_NS,
                "downsample": stats_json(&large.downsample),
                "paint_prep_full_n": stats_json(&large.prep_full),
                "paint_prep_downsampled": stats_json(&large.prep_downsampled),
            },
            "bubble_scatter_cloud": {
                "n_points_retained_for_save": cloud.n,
                "marks_painted": cloud.kept,
                "max_paint_markers": MAX_PAINT_MARKERS,
                "reference_frame_target_ns": FRAME_TARGET_NS,
                "reference_frame_worst_ns": FRAME_WORST_NS,
                "note": "GAPS C-P25-1: scatter/bubble paint one mark per point; cap_markers_for_paint bounds it to <= max_paint_markers (identity below the cap, so no baseline moves).",
                "paint_prep_full_n": stats_json(&cloud.prep_full),
                "paint_prep_capped": stats_json(&cloud.prep_capped),
            },
        },
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&json)?)?;
    println!("results written to {}", path.display());
    Ok(())
}

fn stats_json(s: &LatencyStats) -> serde_json::Value {
    serde_json::json!({
        "count": s.count,
        "p50_ns": s.p50_ns,
        "p99_ns": s.p99_ns,
        "max_ns": s.max_ns,
        "p50": fmt_ns(s.p50_ns),
        "p99": fmt_ns(s.p99_ns),
        "max": fmt_ns(s.max_ns),
    })
}
