//! Latency statistics: percentiles plus a summary [`LatencyStats`] over a set of
//! samples. Report p50/p99/max wherever a distribution matters (architecture §3),
//! not just means.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The nearest-rank percentile (in nanoseconds) of a **sorted-ascending** slice.
///
/// `pct` is clamped to `0.0..=100.0`. For a non-empty input the rank is
/// `ceil(pct/100 * n)` (1-based), so `percentile_ns(_, 100.0)` is the max and
/// `percentile_ns(_, 0.0)` is the min. Returns `0` for an empty slice.
///
/// The caller is responsible for sorting; this keeps the hot path allocation-free.
pub fn percentile_ns(sorted_ascending: &[u64], pct: f64) -> u64 {
    if sorted_ascending.is_empty() {
        return 0;
    }
    let pct = pct.clamp(0.0, 100.0);
    let n = sorted_ascending.len();
    if pct <= 0.0 {
        return sorted_ascending[0];
    }
    // 1-based nearest rank, then convert to a 0-based index bounded to the slice.
    let rank = (pct / 100.0 * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted_ascending[idx]
}

/// A summary of a latency sample set, all in nanoseconds. Serializable so it can be
/// embedded directly in a recorded [`crate::BenchResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyStats {
    /// Number of samples.
    pub count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub mean_ns: u64,
    /// Median (50th percentile).
    pub p50_ns: u64,
    /// 99th percentile.
    pub p99_ns: u64,
}

impl LatencyStats {
    /// Computes stats over `samples`. Returns `None` for an empty input (there is
    /// no meaningful min/mean/percentile of zero samples).
    pub fn from_durations(samples: &[Duration]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut ns: Vec<u64> = samples.iter().map(duration_ns).collect();
        ns.sort_unstable();
        Self::from_sorted_ns(&ns)
    }

    /// Computes stats over an already-sorted-ascending nanosecond slice. Returns
    /// `None` if empty.
    pub fn from_sorted_ns(sorted_ascending: &[u64]) -> Option<Self> {
        if sorted_ascending.is_empty() {
            return None;
        }
        let count = sorted_ascending.len() as u64;
        let sum: u128 = sorted_ascending.iter().map(|&v| v as u128).sum();
        let mean_ns = (sum / count as u128) as u64;
        Some(Self {
            count,
            min_ns: sorted_ascending[0],
            max_ns: *sorted_ascending.last().unwrap(),
            mean_ns,
            p50_ns: percentile_ns(sorted_ascending, 50.0),
            p99_ns: percentile_ns(sorted_ascending, 99.0),
        })
    }
}

/// Saturating conversion of a [`Duration`] to whole nanoseconds. Durations longer
/// than ~584 years saturate at `u64::MAX` (irrelevant for benchmarks, but keeps the
/// function total).
fn duration_ns(d: &Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_nearest_rank() {
        let sorted: Vec<u64> = (1..=100).collect(); // 1..=100
        assert_eq!(percentile_ns(&sorted, 50.0), 50);
        assert_eq!(percentile_ns(&sorted, 99.0), 99);
        assert_eq!(percentile_ns(&sorted, 100.0), 100);
        assert_eq!(percentile_ns(&sorted, 0.0), 1);
    }

    #[test]
    fn percentile_edge_cases() {
        assert_eq!(percentile_ns(&[], 50.0), 0);
        assert_eq!(percentile_ns(&[7], 50.0), 7);
        assert_eq!(percentile_ns(&[7], 99.0), 7);
        // Out-of-range pct is clamped.
        assert_eq!(percentile_ns(&[1, 2, 3], 150.0), 3);
        assert_eq!(percentile_ns(&[1, 2, 3], -5.0), 1);
    }

    #[test]
    fn latency_stats_empty_is_none() {
        assert!(LatencyStats::from_durations(&[]).is_none());
        assert!(LatencyStats::from_sorted_ns(&[]).is_none());
    }

    #[test]
    fn latency_stats_single_element() {
        let s = LatencyStats::from_durations(&[Duration::from_nanos(42)]).unwrap();
        assert_eq!(s.count, 1);
        assert_eq!(s.min_ns, 42);
        assert_eq!(s.max_ns, 42);
        assert_eq!(s.mean_ns, 42);
        assert_eq!(s.p50_ns, 42);
        assert_eq!(s.p99_ns, 42);
    }

    #[test]
    fn latency_stats_known_set() {
        let durations: Vec<Duration> = (1..=10).map(Duration::from_nanos).collect();
        let s = LatencyStats::from_durations(&durations).unwrap();
        assert_eq!(s.count, 10);
        assert_eq!(s.min_ns, 1);
        assert_eq!(s.max_ns, 10);
        assert_eq!(s.mean_ns, 5); // (1+..+10)/10 = 55/10 = 5 (integer)
        assert_eq!(s.p50_ns, 5); // ceil(0.5*10)=5 -> index 4 -> value 5
        assert_eq!(s.p99_ns, 10); // ceil(0.99*10)=10 -> index 9 -> value 10
    }

    #[test]
    fn latency_stats_sorts_unsorted_input() {
        let durations: Vec<Duration> = [10u64, 1, 5, 3, 8]
            .into_iter()
            .map(Duration::from_nanos)
            .collect();
        let s = LatencyStats::from_durations(&durations).unwrap();
        assert_eq!(s.min_ns, 1);
        assert_eq!(s.max_ns, 10);
    }
}
