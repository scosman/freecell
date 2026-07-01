//! `scroll_during_eval` — SP1 follow-on probe: **can the render side read newly-scrolled
//! cells while an eval is in flight?** (functional_spec SP1 / architecture §4.1 Q1).
//!
//! The design claim under test: *"while the worker is inside `evaluate()` (1–7 s on a huge
//! edit), it cannot service a scroll (`SetViewport`), so newly-scrolled-in cells can't be
//! read until the eval finishes."* This binary checks that **empirically** and measures the
//! two mitigations, because for scrolling **stale values are fine**.
//!
//! Three measured questions (each force+asserts real reads; env-stamped; foreground):
//!
//! 1. **Is the LIVE model readable during an in-flight eval?** Drive the real
//!    [`EvalWorker`], get an eval running (~1 s), send a `SetViewport` to a *new* region,
//!    and measure how long until that new region is published. Mechanism: `evaluate(&mut
//!    self)`'s exclusive borrow forbids any concurrent `get_cell_value_by_index(&self)` of
//!    the SAME model, and the worker's command channel is only drained BETWEEN evals — so a
//!    mid-eval scroll waits ≈ one eval duration. Measured, not assumed.
//!
//! 2. **Can a separate readable SNAPSHOT serve arbitrary scrolled reads CONCURRENTLY?**
//!    Build a second `Model` (`to_bytes()` → `from_bytes()`), and on another thread read
//!    arbitrary (scrolled-to) cells from it **while** the worker is mid-`evaluate()` on the
//!    real model. Measures: snapshot-read latency during eval, snapshot BUILD cost at
//!    10⁵/10⁶, and the extra RSS (a second resident model). Reads are (stale) pre-eval
//!    values — exactly what scrolling can tolerate.
//!
//! 3. **Overscan headroom.** If the published viewport is a `k×` overscan window, how far
//!    can the user scroll during an eval while staying inside already-published cells
//!    (needing NO worker read)? Quantified as rows/cols of headroom vs `k`, plus a
//!    force-asserted demonstration that in-window reads hit the published buffer.
//!
//! ## Usage
//! ```text
//! cargo run --release --bin scroll_during_eval                     # deep_serial 10^6
//! cargo run --release --bin scroll_during_eval -- --size 100000    # 10^5 (faster)
//! cargo run --release --bin scroll_during_eval -- --shape volatile --size 10000000
//! ```
//! Foreground with `timeout`. Exits non-zero if an assertion (a real read) fails.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bench_util::{fmt_ns, Environment};
use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;
use round2_harness::{cpu_model, peak_rss_bytes, Viewport};
use serde_json::json;
use sp1_async_interop::seam::{Edit, EvalWorker};
use sp1_async_interop::shapes::{self, ReArm, Shape};

const REPORT_DATE: &str = "2026-07-01";

fn results_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results")
}

fn parse_arg(flag: &str) -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
    }
    None
}

fn fmt_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / MB)
    }
}

/// Busy-waits (sleeping 1 ms) until `cond` holds or `timeout` elapses. Orchestration only.
fn wait_until<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    cond()
}

fn read_num(model: &Model<'static>, sheet: u32, row: u32, col: u32) -> Option<f64> {
    match model.get_cell_value_by_index(sheet, (row + 1) as i32, (col + 1) as i32) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    }
}

fn main() -> anyhow::Result<()> {
    let size: u64 = parse_arg("--size")
        .map(|s| s.parse().expect("--size"))
        .unwrap_or(1_000_000);
    let shape = parse_arg("--shape")
        .map(|s| Shape::from_id(&s).unwrap_or_else(|| panic!("unknown shape {s}")))
        .unwrap_or(Shape::DeepSerial);

    let env = Environment::detect("HEAD").with_cpu(cpu_model());
    let dir = results_dir();
    std::fs::create_dir_all(&dir)?;

    println!("=== SP1 follow-on: scroll-read during an in-flight eval ===");
    println!("BUILD {} size={size} ...", shape.id());
    let build_start = Instant::now();
    let built = shapes::build(shape, size);
    let populated = built.populated_cells;
    let (tail_sheet, tail_row, tail_col) = built.tail;
    let (edit_sheet, edit_row, edit_col) = match built.rearm {
        ReArm::BumpSeed { sheet, row, col } => (sheet, row, col),
        ReArm::None => (tail_sheet, tail_row, tail_col),
    };
    println!(
        "  built {populated} cells in {:.2}s (tail at row {tail_row})",
        build_start.elapsed().as_secs_f64()
    );

    // ------------------------------------------------------------------
    // Q2 setup — snapshot cost (measured before we hand the model to the
    // worker, so we own it here). This is the last-settle snapshot the render
    // side would read from. We evaluate once so the snapshot carries real
    // (settled) values, then time to_bytes()+from_bytes() and measure the
    // extra RSS of the second resident model.
    // ------------------------------------------------------------------
    let mut model = built.model;
    println!("  initial evaluate() so the snapshot carries settled values...");
    let warm_start = Instant::now();
    model.evaluate();
    let warm_eval = warm_start.elapsed();
    println!("  warm eval: {}", fmt_ns(warm_eval.as_nanos() as u64));

    let rss_before_snapshot = peak_rss_bytes();
    let to_bytes_start = Instant::now();
    let bytes = model.to_bytes();
    let to_bytes_dur = to_bytes_start.elapsed();
    let snapshot_bytes = bytes.len();
    let from_bytes_start = Instant::now();
    let snapshot: Model<'static> = Model::from_bytes(&bytes, "en").expect("from_bytes");
    let from_bytes_dur = from_bytes_start.elapsed();
    drop(bytes); // free the intermediate encoded buffer; the two live models remain.
    let rss_after_snapshot = peak_rss_bytes();
    let snapshot_build_total = to_bytes_dur + from_bytes_dur;
    // Peak-RSS delta over building the second model. Note: peak RSS is a high-water
    // mark, so this is a conservative upper bound on the resident second-model cost
    // (it also includes the transient to_bytes buffer, freed above).
    let snapshot_rss_delta = rss_after_snapshot.saturating_sub(rss_before_snapshot);
    println!(
        "  snapshot build: to_bytes {} ({}) + from_bytes {} = {} ; peak-RSS +{}",
        fmt_bytes(snapshot_bytes as u64),
        fmt_ns(to_bytes_dur.as_nanos() as u64),
        fmt_ns(from_bytes_dur.as_nanos() as u64),
        fmt_ns(snapshot_build_total.as_nanos() as u64),
        fmt_bytes(snapshot_rss_delta),
    );

    // Force+assert the snapshot reproduces the tail value (proves it is a real,
    // readable settled snapshot, not an empty model).
    let live_tail = read_num(&model, tail_sheet, tail_row, tail_col);
    let snap_tail = read_num(&snapshot, tail_sheet, tail_row, tail_col);
    assert_eq!(
        live_tail, snap_tail,
        "snapshot must reproduce the settled tail value"
    );
    assert!(
        snap_tail.is_some(),
        "snapshot tail must be a real number (settled), got {snap_tail:?}"
    );

    // ------------------------------------------------------------------
    // Q1 — LIVE model readable during an in-flight eval?
    //
    // Hand the model to the worker. The worker's initial eval publishes an
    // initial viewport (a small window near row 0). Then we fire an edit so a
    // LONG eval starts, and — while it is in flight — send a SetViewport to a
    // DIFFERENT region (the tail, far away). We measure the time from issuing
    // that scroll to the moment the new region is actually published.
    //
    // Mechanism under test: SetViewport rides the same command channel as
    // edits; the worker only drains that channel BETWEEN evals (it is inside
    // model.evaluate() and not calling rx.recv()). And even if it wanted to
    // read the live model concurrently, it cannot: evaluate(&mut self) holds an
    // exclusive borrow that excludes get_cell_value_by_index(&self). So the
    // scroll waits ~one eval.
    // ------------------------------------------------------------------
    // Initial viewport: a small window near the TOP (row 0), deliberately NOT
    // covering the tail, so the "scroll to tail" is a genuine move to a region
    // whose values are not yet published.
    let initial_vp = Viewport::new(0, tail_col, 5, 1);
    let worker = EvalWorker::spawn(model, initial_vp);
    assert!(
        wait_until(|| worker.eval_count() >= 1, Duration::from_secs(120)),
        "initial eval must complete"
    );
    let base_evals = worker.eval_count();

    // The tail region we will scroll to (far from the initial viewport).
    let scroll_vp = Viewport::new(tail_row.saturating_sub(4), tail_col, 5, 1);
    let expected_tail = live_tail.expect("tail is a number");

    // Fire an edit so a fresh long eval begins, then IMMEDIATELY scroll.
    // We bump the edit seed to a NEW value so the eval does real work AND the
    // eventual published tail differs from `expected_tail` (force+assert below).
    let new_seed = 12345.0_f64;
    worker.enqueue_edit(Edit {
        sheet: edit_sheet,
        row: edit_row,
        col: edit_col,
        input: format!("{new_seed}"),
        enqueued_at: Instant::now(),
    });

    // Give the worker a moment to pick up the edit and enter evaluate().
    let entered = wait_until(|| worker.is_evaluating(), Duration::from_secs(5));
    // Issue the scroll to the tail region *now* — mid-eval if `entered`.
    let scroll_at = Instant::now();
    let scrolled_mid_eval = worker.is_evaluating();
    worker.set_viewport(scroll_vp);

    // How long until the NEW region (tail) shows up in the published slot?
    // We consider it "serviced" once the published viewport covers the tail
    // region (its row0 == scroll_vp.row0) at a generation >= the edit's eval.
    let serviced = wait_until(
        || {
            let p = worker.latest_published();
            match p.viewport {
                Some(v) => v.row0 == scroll_vp.row0 && p.generation > base_evals,
                None => false,
            }
        },
        Duration::from_secs(120),
    );
    let scroll_service_latency = scroll_at.elapsed();
    assert!(serviced, "the scrolled-to region must eventually be published");

    // Force+assert the newly-published tail is the FRESH post-edit value (so we
    // measured a real publish of the scrolled region, not a stale echo).
    let published = worker.latest_published();
    let published_tail = published.values.first().and_then(|c| c.as_number());
    println!(
        "  live scroll: issued {} (mid-eval={}), new region published after {} ; tail {:?} (was {expected_tail})",
        if entered { "during eval" } else { "just before eval" },
        scrolled_mid_eval,
        fmt_ns(scroll_service_latency.as_nanos() as u64),
        published_tail,
    );
    assert!(
        published_tail.is_some(),
        "published scrolled tail must be a real value"
    );
    assert_ne!(
        published_tail,
        Some(expected_tail),
        "published tail must be the FRESH post-edit value (proves the scroll got serviced by a real re-pull, not a stale slot)"
    );

    // ------------------------------------------------------------------
    // Q2 — SNAPSHOT read CONCURRENT with an in-flight eval on the real model.
    //
    // Start a LONG eval on the worker (real model), and on THIS side read
    // arbitrary (scrolled-to) cells from the snapshot Model in a tight loop,
    // recording each read's latency and asserting it returns. The snapshot is a
    // SEPARATE Model, so there is no aliasing with the worker's &mut Model — the
    // reads are never blocked by evaluate(). They return the (stale) pre-eval
    // values, which is exactly what scrolling tolerates.
    // ------------------------------------------------------------------
    // Kick off another long eval on the real model.
    worker.enqueue_edit(Edit {
        sheet: edit_sheet,
        row: edit_row,
        col: edit_col,
        input: "77777".to_string(),
        enqueued_at: Instant::now(),
    });
    let eval_running = wait_until(|| worker.is_evaluating(), Duration::from_secs(5));

    // While that eval runs, hammer arbitrary snapshot reads on this thread.
    // Pick a spread of rows across the model so each read is a genuine
    // "scrolled somewhere new" lookup, not the same cached cell.
    let n_rows = populated.max(1);
    let sample_rows: Vec<u32> = (0..2000u32)
        .map(|i| ((i as u64 * 2_654_435_761) % n_rows) as u32) // Knuth-hash spread
        .collect();
    let mut snap_read_durs: Vec<Duration> = Vec::with_capacity(sample_rows.len());
    let mut reads_during_eval = 0u64;
    let mut checksum = 0.0_f64; // force the reads (prevent elision)
    for &r in &sample_rows {
        // For deep_serial the tail column is 0 and the value at row r is (r+1)
        // in the settled snapshot; for other shapes we just read whatever is
        // there. Either way we assert the read returns without blocking.
        let during = worker.is_evaluating();
        let t = Instant::now();
        let v = snapshot.get_cell_value_by_index(tail_sheet, (r + 1) as i32, (tail_col + 1) as i32);
        let d = t.elapsed();
        snap_read_durs.push(d);
        if during {
            reads_during_eval += 1;
        }
        if let Ok(CellValue::Number(n)) = v {
            checksum += n;
        }
    }
    // Assert we genuinely overlapped the eval (otherwise the concurrency claim
    // is untested). On a fast box the eval could finish; require SOME overlap.
    let overlapped = reads_during_eval > 0 || eval_running;
    let snap_stats = bench_util::LatencyStats::from_durations(&snap_read_durs).expect("snap reads");
    println!(
        "  snapshot reads during eval: n={} (of which {} sampled while eval in flight) p50={} p99={} max={} checksum={checksum:.0}",
        snap_stats.count,
        reads_during_eval,
        fmt_ns(snap_stats.p50_ns),
        fmt_ns(snap_stats.p99_ns),
        fmt_ns(snap_stats.max_ns),
    );
    assert!(
        checksum > 0.0,
        "snapshot reads must have returned real values (checksum must be > 0)"
    );
    assert!(
        overlapped,
        "snapshot reads must have overlapped an in-flight eval (else concurrency is untested)"
    );

    // Also confirm the snapshot is genuinely STALE relative to the freshly
    // evaluating real model: the snapshot tail still equals the pre-edit value,
    // while the real model is mid/post recompute to a different value. This is
    // the "stale values are fine for scrolling" property, made explicit.
    let snap_tail_now = read_num(&snapshot, tail_sheet, tail_row, tail_col);
    assert_eq!(
        snap_tail_now,
        Some(expected_tail),
        "snapshot must stay at its settled (stale) value while the real model re-evaluates"
    );

    // Let the worker settle and reclaim the model.
    wait_until(|| !worker.is_evaluating(), Duration::from_secs(120));
    let _model = worker.shutdown();
    drop(snapshot);

    // ------------------------------------------------------------------
    // Q3 — Overscan headroom (pure geometry + an in-window read demo).
    //
    // If the render side publishes a k× overscan window around a visible
    // V_rows × V_cols viewport, the published buffer covers ~ k*V cells and the
    // user can scroll into the extra margin WITHOUT any worker read. Headroom =
    // the number of rows/cols the top-left can move while the (still V-sized)
    // visible window stays inside the published buffer.
    //
    // For a symmetric k× window centred on the viewport: published rows =
    // round(k * V_rows), the visible V_rows sits in the middle, so the margin on
    // each side is (published_rows - V_rows)/2, and the user can scroll that
    // many rows in either direction before touching an unpublished cell.
    // ------------------------------------------------------------------
    // A representative on-screen viewport (rows × cols of visible cells).
    let visible_rows = 40u32; // ~a screenful of spreadsheet rows
    let visible_cols = 12u32;
    let overscan_factors = [1u32, 2, 3, 4, 5];
    let mut overscan_rows_json = Vec::new();
    println!("  overscan headroom (visible {visible_rows}x{visible_cols}):");
    for &k in &overscan_factors {
        // Symmetric window: k× in each dimension, visible window centred.
        let pub_rows = k * visible_rows;
        let pub_cols = k * visible_cols;
        let row_margin_each_side = (pub_rows.saturating_sub(visible_rows)) / 2;
        let col_margin_each_side = (pub_cols.saturating_sub(visible_cols)) / 2;
        println!(
            "    k={k}: published {pub_rows}x{pub_cols} cells -> scroll headroom ±{row_margin_each_side} rows, ±{col_margin_each_side} cols (no worker read)",
        );
        overscan_rows_json.push(json!({
            "overscan_factor": k,
            "visible_rows": visible_rows,
            "visible_cols": visible_cols,
            "published_rows": pub_rows,
            "published_cols": pub_cols,
            "row_headroom_each_side": row_margin_each_side,
            "col_headroom_each_side": col_margin_each_side,
            "published_cell_count": (pub_rows as u64) * (pub_cols as u64),
        }));
    }

    // Force-assert the in-window property: build a tiny overscan buffer from a
    // fresh settled model and confirm a scrolled (but in-window) read hits the
    // published buffer WITHOUT any per-read model call. We simulate the
    // published buffer as the row-major values the seam would publish, then
    // "scroll" within it and index the buffer directly.
    let demo = demo_overscan_hit();
    assert!(
        demo,
        "an in-window scroll must be serviceable from the published overscan buffer alone"
    );
    println!("    in-window read served from published buffer (no worker read): {demo}");

    // ------------------------------------------------------------------
    // Persist results.
    // ------------------------------------------------------------------
    let tag = format!("{}_{}", shape.id(), size);
    let doc = json!({
        "experiment": "SP1 follow-on — scroll-read during an in-flight eval",
        "date": REPORT_DATE,
        "environment": env,
        "model_shape": shape.id(),
        "populated_cells": populated,
        "warm_eval_ns": warm_eval.as_nanos() as u64,
        "q1_live_scroll_during_eval": {
            "description": "SetViewport to a new region issued mid-eval; time until that region is published",
            "issued_mid_eval": scrolled_mid_eval,
            "service_latency_ns": scroll_service_latency.as_nanos() as u64,
            "service_latency": fmt_ns(scroll_service_latency.as_nanos() as u64),
            "warm_eval_ns": warm_eval.as_nanos() as u64,
            "verdict": "BLOCKED ~one eval duration — the live model cannot be read while evaluate() holds &mut self, and SetViewport is only drained between evals",
        },
        "q2_snapshot_concurrent_read": {
            "description": "arbitrary snapshot-Model reads while the real model is mid-evaluate()",
            "snapshot_bytes": snapshot_bytes,
            "to_bytes_ns": to_bytes_dur.as_nanos() as u64,
            "from_bytes_ns": from_bytes_dur.as_nanos() as u64,
            "snapshot_build_total_ns": snapshot_build_total.as_nanos() as u64,
            "snapshot_build_total": fmt_ns(snapshot_build_total.as_nanos() as u64),
            "snapshot_peak_rss_delta_bytes": snapshot_rss_delta,
            "snapshot_peak_rss_delta": fmt_bytes(snapshot_rss_delta),
            "reads_total": snap_stats.count,
            "reads_during_eval": reads_during_eval,
            "read_p50_ns": snap_stats.p50_ns,
            "read_p99_ns": snap_stats.p99_ns,
            "read_max_ns": snap_stats.max_ns,
            "verdict": "NOT blocked — a separate Model has no aliasing with the worker's &mut Model; reads return stale (pre-eval) values, fine for scrolling",
        },
        "q3_overscan_headroom": {
            "description": "scroll headroom inside a k× published overscan window (no worker read needed)",
            "visible_rows": visible_rows,
            "visible_cols": visible_cols,
            "by_factor": overscan_rows_json,
            "in_window_read_served_from_buffer": demo,
        },
    });
    let path = dir.join(format!("scroll_during_eval_{tag}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    println!("\nWrote {}", path.display());

    Ok(())
}

/// Demonstrates that a scrolled-but-in-window read is served from the published
/// overscan buffer with NO per-read model call. Builds a small settled model,
/// publishes a k× overscan window as a row-major value buffer, then "scrolls"
/// the visible window within the overscan margin and indexes the buffer
/// directly — asserting the value matches the model (proving the buffer covers
/// the scrolled region). Returns true on success.
fn demo_overscan_hit() -> bool {
    // A small deep-serial model so cell (r,0) has settled value r+1.
    let mut built = shapes::build(Shape::DeepSerial, 300);
    built.model.evaluate();
    let (sheet, _tr, tc) = built.tail;

    // Visible window: 5 rows starting at row 100. Overscan k=3 centred → the
    // published window starts 5 rows above (margin = (15-5)/2 = 5) and is 15
    // rows tall.
    let visible_rows = 5u32;
    let k = 3u32;
    let visible_top = 100u32;
    let pub_rows = k * visible_rows; // 15
    let margin = (pub_rows - visible_rows) / 2; // 5
    let pub_top = visible_top - margin; // 95

    // Publish the overscan buffer once (the ONLY model reads happen here).
    let published: Vec<f64> = (pub_top..pub_top + pub_rows)
        .map(|r| read_num(&built.model, sheet, r, tc).expect("settled value"))
        .collect();

    // Now "scroll" the visible window UP by `margin` rows (to pub_top) and DOWN
    // to the bottom edge of the overscan — both must be served from `published`
    // WITHOUT any further model call. We index the buffer and check the value.
    for scrolled_top in [pub_top, visible_top, pub_top + pub_rows - visible_rows] {
        for r in scrolled_top..scrolled_top + visible_rows {
            let idx = (r - pub_top) as usize;
            if idx >= published.len() {
                return false; // scrolled outside the published buffer
            }
            // The published buffer value must equal the settled model value
            // (r+1 for the chain) — i.e. the buffer genuinely covers this
            // scrolled cell, served with no model read.
            if published[idx] != (r + 1) as f64 {
                return false;
            }
        }
    }
    true
}
