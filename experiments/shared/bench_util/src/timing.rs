//! Small end-to-end timing helpers for latencies that Criterion's model doesn't
//! fit (e.g. "edit → cascade → read visible"), per architecture §3.
//!
//! These use the real monotonic clock (`std::time::Instant`) for **measurement**
//! only. Note the deliberate split from results *recording* (see [`crate::record`]),
//! which never calls a wall clock — the report date is always passed in, so
//! recorded artifacts stay deterministic (architecture §3).

use std::time::{Duration, Instant};

/// A minimal monotonic stopwatch over [`Instant`].
///
/// ```
/// use bench_util::Stopwatch;
/// let sw = Stopwatch::start();
/// // ... work ...
/// let _elapsed = sw.elapsed();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch {
    started: Instant,
}

impl Stopwatch {
    /// Starts a stopwatch at "now".
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// Elapsed time since [`Stopwatch::start`].
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Runs `f` once, returning its result alongside how long it took.
///
/// ```
/// use bench_util::time_once;
/// let (sum, _elapsed) = time_once(|| (0..100).sum::<u32>());
/// assert_eq!(sum, 4950);
/// ```
pub fn time_once<T, F: FnOnce() -> T>(f: F) -> (T, Duration) {
    let sw = Stopwatch::start();
    let out = f();
    (out, sw.elapsed())
}

/// Runs `f` `iters` times, returning each iteration's [`Duration`] so callers can
/// feed them to [`crate::LatencyStats::from_durations`]. `iters == 0` returns an
/// empty vector.
pub fn time_iters<F: FnMut()>(iters: usize, mut f: F) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let sw = Stopwatch::start();
        f();
        samples.push(sw.elapsed());
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwatch_is_monotonic_nonnegative() {
        let sw = Stopwatch::start();
        let a = sw.elapsed();
        let b = sw.elapsed();
        assert!(b >= a);
    }

    #[test]
    fn time_once_returns_value() {
        let (value, _elapsed) = time_once(|| 21 * 2);
        assert_eq!(value, 42);
    }

    #[test]
    fn time_iters_count() {
        let mut n = 0;
        let samples = time_iters(5, || n += 1);
        assert_eq!(samples.len(), 5);
        assert_eq!(n, 5);
    }

    #[test]
    fn time_iters_zero_is_empty() {
        let samples = time_iters(0, || unreachable!("closure must not run"));
        assert!(samples.is_empty());
    }
}
