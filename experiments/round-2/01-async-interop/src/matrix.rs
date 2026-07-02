//! The `evaluate()` latency matrix (functional_spec SP1 DISCOVERY).
//!
//! Times a single full `Model::evaluate()` across sizes × DAG shapes, reporting
//! p50/p99, and — per the benchmark discipline — **force + asserts** that the tail
//! cell's value actually changed each timed sample (so we never record the latency of
//! a no-op re-eval). Build time is kept out of the measured op: the model is built
//! once, then each sample re-arms (a cheap seed edit) and times only `evaluate()`.

use bench_util::{LatencyStats, Stopwatch};
use ironcalc_base::Model;

use crate::shapes::{self, BuiltShape, ReArm, Shape};

/// The outcome of timing one (shape, size) cell of the matrix.
pub struct MatrixCell {
    pub shape: Shape,
    /// Requested target size (10⁴…10⁷).
    pub requested_size: u64,
    /// Actual populated-cell count (the honest input size; eval is O(all cells)).
    pub populated_cells: u64,
    pub tail_a1: String,
    /// The tail value before the *last* timed eval and after it — proof it changed.
    pub tail_before: f64,
    pub tail_after: f64,
    pub stats: LatencyStats,
}

/// Times `samples` full evaluations of an already-built shape, force+asserting the tail
/// changed each sample. Returns the per-eval latency distribution plus change proof.
///
/// `requested_size` is the matrix target (10⁴…10⁷) stamped onto the result for
/// reporting; `populated_cells` (the honest input size) comes from the built shape.
///
/// Between samples the model is re-armed (a single seed edit for the deterministic
/// shapes; nothing for volatile, which changes on its own). The re-arm edit is **not**
/// timed — only the `evaluate()` call is.
pub fn time_evaluate(built: &mut BuiltShape, requested_size: u64, samples: usize) -> MatrixCell {
    assert!(samples >= 1, "need at least one sample");

    // Warm-up eval so the first *timed* sample isn't paying one-time setup (parsing was
    // already done at set_user_input; this primes the cell cache / allocator).
    built.model.evaluate();

    let mut durations = Vec::with_capacity(samples);
    let mut last_before = f64::NAN;
    let mut last_after = f64::NAN;

    for i in 0..samples {
        // Re-arm so this eval produces a genuinely different tail (skip for volatile).
        if built.rearm != ReArm::None {
            shapes::rearm(&mut built.model, built.rearm, i as u64);
        }
        let before = read_tail(&built.model, built);

        let sw = Stopwatch::start();
        built.model.evaluate();
        durations.push(sw.elapsed());

        let after = read_tail(&built.model, built);
        assert_tail_changed(built, before, after);

        last_before = before;
        last_after = after;
    }

    let stats = LatencyStats::from_durations(&durations).expect("non-empty samples");
    MatrixCell {
        shape: built.shape,
        requested_size,
        populated_cells: built.populated_cells,
        tail_a1: built.tail_a1.clone(),
        tail_before: last_before,
        tail_after: last_after,
        stats,
    }
}

fn read_tail(model: &Model<'static>, built: &BuiltShape) -> f64 {
    shapes::read_number(model, built.tail.0, built.tail.1, built.tail.2)
        .expect("tail cell must be numeric")
}

/// Force+assert the eval did real work.
///
/// - **Deterministic shapes** (sparse/chain/fanout/cross-sheet): after a monotonic seed
///   re-arm the tail MUST take a new, predictable value — a strict `before != after`.
/// - **Volatile** (`=RAND()`): two complementary checks. The **range** check
///   (`after ∈ [0,1)`) proves the tail holds a **well-formed `RAND()` result** — but note
///   it does NOT by itself prove a re-roll (a skipped/no-op eval would leave the previous
///   RAND value, also in `[0,1)`). The **re-roll witness** is `before != after`: two
///   independent `RAND()` draws collide with probability ~2⁻⁵², so a change is
///   overwhelming evidence the eval actually re-evaluated the cell. Both asserts are kept.
fn assert_tail_changed(built: &BuiltShape, before: f64, after: f64) {
    if built.changes_without_rearm {
        assert!(
            (0.0..1.0).contains(&after),
            "volatile tail must be a well-formed RAND() result in [0,1) after eval (got {after})"
        );
        assert_ne!(
            before, after,
            "volatile tail should differ across consecutive RAND() evals"
        );
    } else {
        assert_ne!(
            before,
            after,
            "re-armed eval must change the tail (shape={}, tail={})",
            built.shape.id(),
            built.tail_a1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::build;

    #[test]
    fn times_and_asserts_change_deep_serial() {
        let mut built = build(Shape::DeepSerial, 20);
        let cell = time_evaluate(&mut built, 20, 3);
        assert_eq!(cell.stats.count, 3);
        assert_eq!(cell.requested_size, 20);
        assert_eq!(cell.shape, Shape::DeepSerial);
        assert_ne!(cell.tail_before, cell.tail_after);
        assert!(cell.populated_cells == 20);
    }

    #[test]
    fn times_and_asserts_change_volatile() {
        let mut built = build(Shape::Volatile, 16);
        let cell = time_evaluate(&mut built, 16, 3);
        assert_eq!(cell.stats.count, 3);
        assert_eq!(cell.shape, Shape::Volatile);
        // Volatile: before/after of the last sample differ.
        assert_ne!(cell.tail_before, cell.tail_after);
    }

    #[test]
    fn stats_are_positive() {
        let mut built = build(Shape::Sparse, 100);
        let cell = time_evaluate(&mut built, 100, 2);
        assert!(cell.stats.p50_ns > 0);
        assert!(cell.stats.max_ns >= cell.stats.p50_ns);
    }
}
