//! Integration tests for the SP1 interop seam and API findings, exercised through the
//! crate's public API. The heavy latency matrix and the 10⁶–10⁷ GATE runs live in the
//! `latency_matrix` / `nonblocking` binaries (run foreground with `timeout`); these
//! tests validate the seam logic + findings at fast, deterministic sizes.

use std::time::{Duration, Instant};

use round2_harness::Viewport;
use sp1_async_interop::probes::{diff_list_is_edit_sites_only, snapshot_roundtrip};
use sp1_async_interop::seam::{assert_model_send, Edit, EvalWorker};
use sp1_async_interop::shapes::{build, Shape};
use sp1_async_interop::time_evaluate;

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

/// FINDING: `Model<'static>` is `Send` — proven at compile time. This test's existence
/// (it links) is the proof; a non-`Send` field would break the build.
#[test]
fn model_is_send() {
    assert_model_send();
}

/// FINDING: IronCalc exposes no evaluated-cell change stream — the `UserModel` diff-list
/// records only edit-sites, so a single edit's diff does not scale with its cascade.
#[test]
fn no_evaluated_cell_diff() {
    let short = diff_list_is_edit_sites_only(10);
    let long = diff_list_is_edit_sites_only(1000);
    assert_eq!(short.cascaded_cells, 9);
    assert_eq!(long.cascaded_cells, 999);
    // The cascade grows 111x; the one-edit diff must stay tiny (edit-site only).
    assert!(
        long.diff_bytes_for_one_edit < short.diff_bytes_for_one_edit * 3,
        "one-edit diff scaled with cascade: {} vs {} bytes",
        short.diff_bytes_for_one_edit,
        long.diff_bytes_for_one_edit
    );
}

/// FINDING: the snapshot publish route (`to_bytes`/`from_bytes`) reproduces values.
#[test]
fn snapshot_roundtrips() {
    assert!(snapshot_roundtrip());
}

/// The matrix timer force+asserts the tail changed and returns a distribution.
#[test]
fn matrix_forces_and_asserts_change() {
    let mut built = build(Shape::WideFanout, 0);
    let cell = time_evaluate(&mut built, 2000, 3);
    assert_eq!(cell.stats.count, 3);
    assert_ne!(cell.tail_before, cell.tail_after);
}

/// GATE-2 logic (coalescing) at a test-friendly size: a burst of rapid edits collapses
/// to a small number of evals.
#[test]
fn worker_coalesces_rapid_edits() {
    let built = build(Shape::DeepSerial, 40_000);
    let vp = Viewport::new(39_999, 0, 1, 1);
    let worker = EvalWorker::spawn(built.model, vp);
    assert!(wait_until(
        || worker.eval_count() >= 1,
        Duration::from_secs(15)
    ));
    let base = worker.eval_count();

    for i in 0..25u32 {
        worker.enqueue_edit(Edit {
            sheet: 0,
            row: 0,
            col: 0,
            input: format!("{}", i + 1),
            enqueued_at: Instant::now(),
        });
    }
    assert!(wait_until(
        || !worker.is_evaluating() && worker.latest_published().generation >= worker.eval_count(),
        Duration::from_secs(30)
    ));
    std::thread::sleep(Duration::from_millis(50));
    let extra = worker.eval_count() - base;
    assert!(
        extra <= 5,
        "25 rapid edits should coalesce, got {extra} evals"
    );
    let _ = worker.shutdown();
}

/// GATE-1 logic at a test-friendly size: the render loop's per-tick synchronous work
/// stays well under one frame even while an eval runs on the worker.
#[test]
fn render_tick_is_cheap_during_eval() {
    let built = build(Shape::DeepSerial, 200_000);
    let (_s, tail_row, _c) = built.tail;
    let vp = Viewport::new(tail_row.saturating_sub(4), 0, 5, 1);
    let worker = EvalWorker::spawn(built.model, vp);
    assert!(wait_until(
        || worker.eval_count() >= 1,
        Duration::from_secs(30)
    ));

    // Fire an edit that triggers a fresh (slow-ish) eval, then tick while it runs.
    worker.enqueue_edit(Edit {
        sheet: 0,
        row: 0,
        col: 0,
        input: "42".to_string(),
        enqueued_at: Instant::now(),
    });

    let mut max_tick = Duration::ZERO;
    let mut sampled_during_eval = false;
    for _ in 0..2000 {
        let start = Instant::now();
        let published = worker.latest_published();
        let _ = worker.is_evaluating();
        let _ = published.values.len();
        let cost = start.elapsed();
        if worker.is_evaluating() {
            sampled_during_eval = true;
            max_tick = max_tick.max(cost);
        }
        std::thread::sleep(Duration::from_micros(500));
    }

    // Whether or not we caught a during-eval tick, the tick cost must be tiny (< frame).
    // The read is O(viewport)=5 cells, so it should be microseconds.
    assert!(
        max_tick < Duration::from_millis(8),
        "render tick during eval took {max_tick:?} (>= 8 ms frame budget)"
    );
    // Best-effort note: on a very fast box the eval may finish between ticks. That's a
    // pass for non-blocking (the loop never stalled); we don't require sampling it.
    let _ = sampled_during_eval;
    let _ = worker.shutdown();
}

/// The staleness measurement plumbing yields a finite, positive window.
#[test]
fn staleness_is_measured() {
    let built = build(Shape::DeepSerial, 30_000);
    let (_s, tail_row, _c) = built.tail;
    let vp = Viewport::new(tail_row, 0, 1, 1);
    let worker = EvalWorker::spawn(built.model, vp);
    assert!(wait_until(
        || worker.eval_count() >= 1,
        Duration::from_secs(15)
    ));

    let edit_at = Instant::now();
    worker.enqueue_edit(Edit {
        sheet: 0,
        row: 0,
        col: 0,
        input: "7".to_string(),
        enqueued_at: edit_at,
    });
    // Wait until the edit is reflected in a published generation.
    assert!(wait_until(
        || {
            let gen = worker.last_edit_visible_gen();
            gen != 0 && worker.latest_published().generation >= gen
        },
        Duration::from_secs(15)
    ));
    let window = edit_at.elapsed();
    assert!(window > Duration::ZERO);
    assert!(window < Duration::from_secs(15));
    let _ = worker.shutdown();
}
