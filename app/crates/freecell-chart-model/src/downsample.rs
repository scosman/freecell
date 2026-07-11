//! Paint-time series decimation (charts/architecture §5 challenge 5).
//!
//! A line with **very many points** (a dense time-series) is expensive to paint: the renderer maps
//! every point to a pixel, tessellates an N-vertex stroke path, and draws a marker per point. This
//! module bounds that per-frame cost by decimating a series to at most [`MAX_PAINT_VERTICES`]
//! vertices **for paint only** — [`ChartSpec`](crate::ChartSpec)'s retained `Chart`/source keeps
//! **every** point, so save fidelity and live values are untouched (the down-sample is applied at
//! the render call site, never stored).
//!
//! The decimation is **min/max bucketing** (the standard shape-preserving choice for line/scope
//! rendering): the first and last point are always kept, and the interior is split into buckets
//! that each contribute their local minimum and maximum value index — so peaks and troughs (and the
//! global extrema) survive, unlike naive stride sampling which can skip a spike. A series already
//! within budget is returned unchanged (identity), so ordinary charts paint **byte-identically**.

/// The largest number of vertices the line renderer paints for one series. A series at or under this
/// keeps every point (identity in [`downsample_for_paint`]); a larger one is decimated to `<=` this.
/// Chosen well above any realistic on-screen line resolution (a chart is at most ~1–2k px wide, so
/// more than ~2k vertices cannot be visually distinguished) and far above every committed render
/// scene (all `<= ~12` points), so no baseline moves — only genuinely huge series are decimated.
pub const MAX_PAINT_VERTICES: usize = 2048;

/// The indices of `values` to KEEP when painting a line of at most `max_vertices` vertices, in
/// ascending order, preserving the line's shape.
///
/// - A series already within budget (`len <= max_vertices`), or a degenerate budget (`< 4`), returns
///   the identity `0..len` — small charts paint exactly as before.
/// - Otherwise: the first and last index are always kept, and the interior `1..len-1` is split into
///   buckets, each contributing its local **min-value** and **max-value** index (so the envelope is
///   preserved). The result is strictly increasing and has `<= max_vertices` entries.
///
/// Non-finite (`NaN`/`Inf`) values never win a bucket's min/max, but a bucket that is *entirely*
/// non-finite still contributes its first index, so the renderer's finite-filter still sees the gap
/// (a break in the line) — "render what's valid, blank the rest" survives the decimation.
pub fn downsample_for_paint(values: &[f64], max_vertices: usize) -> Vec<usize> {
    let n = values.len();
    if n <= max_vertices || max_vertices < 4 {
        return (0..n).collect();
    }

    let interior = n - 2; // interior indices are 1..=n-2
    let budget = max_vertices - 2; // reserve one slot each for first + last
    let buckets = (budget / 2).max(1); // each bucket yields up to 2 (min + max) indices

    let mut keep = Vec::with_capacity(max_vertices);
    keep.push(0);
    for b in 0..buckets {
        // Split the interior into `buckets` contiguous ranges [start, end).
        let start = 1 + b * interior / buckets;
        let end = 1 + (b + 1) * interior / buckets;
        if start >= end {
            continue;
        }
        let mut min_i = start;
        let mut max_i = start;
        let mut seen_finite = false;
        for i in start..end {
            let v = values[i];
            if !v.is_finite() {
                continue;
            }
            if !seen_finite {
                min_i = i;
                max_i = i;
                seen_finite = true;
            } else {
                if v < values[min_i] {
                    min_i = i;
                }
                if v > values[max_i] {
                    max_i = i;
                }
            }
        }
        if !seen_finite {
            // An all-non-finite bucket still contributes an index so the gap renders as a break.
            keep.push(start);
            continue;
        }
        let (lo, hi) = (min_i.min(max_i), min_i.max(max_i));
        keep.push(lo);
        if hi != lo {
            keep.push(hi);
        }
    }
    keep.push(n - 1);
    keep.dedup(); // buckets are disjoint + ascending, so this only guards degenerate math
    keep
}

/// The largest number of **markers** (bubble / scatter points) the renderer paints for one series.
/// A scatter/bubble series at or under this keeps every point (identity in
/// [`cap_markers_for_paint`]); a larger one is uniformly sub-sampled to `<=` this. Chosen well above
/// every committed render scene (all `<= ~13` points), so no baseline moves — only a genuinely huge
/// (large-range-bound) point cloud is capped.
pub const MAX_PAINT_MARKERS: usize = 2048;

/// The indices of an **unordered marker cloud** (scatter / bubble points) to PAINT when capping it to
/// at most `max` markers (charts/architecture §5 challenge 5, GAPS C-P25-1).
///
/// Unlike [`downsample_for_paint`] (which decimates an **index-ordered** line by preserving its
/// value extrema), a scatter/bubble cloud has **no 1-D axis to decimate along** — dropping points
/// changes the cloud either way. So the cloud cap is a **uniform stride** subsample spread across
/// `0..n`: it preserves the cloud's overall spatial extent + density far better than "first N", is
/// deterministic, keeps indices ascending (so a scatter's connecting `Line` still threads points in
/// data order over the same subset), and always keeps the **last** point.
///
/// - `n <= max` (or `max < 2`) returns the identity `0..n` — ordinary charts paint every point,
///   byte-identically (a degenerate `max` is too small to subsample meaningfully).
/// - Otherwise returns **at most** `max` strictly-increasing indices linspaced across `[0, n-1]`
///   inclusive — so the first (`0`) and last (`n-1`) points, and thus the cloud's full extent, are
///   always kept, and `keep.len() <= max`.
pub fn cap_markers_for_paint(n: usize, max: usize) -> Vec<usize> {
    if n <= max || max < 2 {
        return (0..n).collect();
    }
    // `max` samples linspaced over [0, n-1]: index k → round(k * (n-1) / (max-1)). k=0 → 0,
    // k=max-1 → n-1. Strictly increasing because n > max ⇒ the step (n-1)/(max-1) > 1; dedup guards
    // the arithmetic corner just in case.
    let mut keep: Vec<usize> = (0..max).map(|k| k * (n - 1) / (max - 1)).collect();
    keep.dedup();
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_series_is_identity() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(
            downsample_for_paint(&v, MAX_PAINT_VERTICES),
            vec![0, 1, 2, 3, 4]
        );
        // Exactly at budget is still identity.
        let v8: Vec<f64> = (0..8).map(|i| i as f64).collect();
        assert_eq!(
            downsample_for_paint(&v8, 8),
            (0..8).collect::<Vec<_>>(),
            "a series exactly at the budget keeps every point"
        );
    }

    #[test]
    fn degenerate_budget_is_identity() {
        let v: Vec<f64> = (0..100).map(|i| i as f64).collect();
        // A budget below the 4-vertex floor can't decimate meaningfully → identity.
        assert_eq!(downsample_for_paint(&v, 3).len(), 100);
    }

    #[test]
    fn large_series_is_bounded_and_ordered() {
        let n = 100_000;
        let v: Vec<f64> = (0..n).map(|i| (i as f64 * 0.01).sin()).collect();
        let keep = downsample_for_paint(&v, MAX_PAINT_VERTICES);
        assert!(
            keep.len() <= MAX_PAINT_VERTICES,
            "decimated to <= budget, got {}",
            keep.len()
        );
        assert!(keep.len() >= 4, "still a meaningful number of vertices");
        // First + last always kept.
        assert_eq!(keep.first(), Some(&0));
        assert_eq!(keep.last(), Some(&(n - 1)));
        // Strictly increasing (order preserved, no reordering, no duplicates).
        assert!(
            keep.windows(2).all(|w| w[0] < w[1]),
            "indices strictly increasing"
        );
    }

    #[test]
    fn preserves_global_peak_and_trough() {
        // A flat line with one tall spike and one deep dip buried in the interior.
        let n = 50_000;
        let mut v = vec![0.0_f64; n];
        let spike = 12_345;
        let dip = 37_777;
        v[spike] = 1000.0;
        v[dip] = -1000.0;
        let keep = downsample_for_paint(&v, MAX_PAINT_VERTICES);
        assert!(
            keep.contains(&spike),
            "the global max (spike) must survive decimation"
        );
        assert!(
            keep.contains(&dip),
            "the global min (dip) must survive decimation"
        );
    }

    #[test]
    fn marker_cap_is_identity_below_cap_and_bounded_above() {
        // Below / at the cap: identity (every point painted, no baseline moves).
        assert_eq!(
            cap_markers_for_paint(0, MAX_PAINT_MARKERS),
            Vec::<usize>::new()
        );
        assert_eq!(
            cap_markers_for_paint(13, MAX_PAINT_MARKERS),
            (0..13).collect::<Vec<_>>()
        );
        assert_eq!(cap_markers_for_paint(64, 64), (0..64).collect::<Vec<_>>());

        // Above the cap: bounded, strictly increasing, keeps the last point, spans the range.
        let n = 100_000;
        let keep = cap_markers_for_paint(n, MAX_PAINT_MARKERS);
        assert!(keep.len() <= MAX_PAINT_MARKERS, "capped to <= budget");
        assert!(
            keep.len() >= MAX_PAINT_MARKERS - 1,
            "uses ~all of the budget"
        );
        assert_eq!(keep.first(), Some(&0), "keeps the first point");
        assert_eq!(keep.last(), Some(&(n - 1)), "keeps the last point");
        assert!(
            keep.windows(2).all(|w| w[0] < w[1]),
            "indices strictly increasing (order preserved for a connecting line)"
        );
    }

    #[test]
    fn all_nonfinite_bucket_still_contributes_a_break() {
        // A long run of NaN in the middle: the bucket(s) over it still contribute indices, so the
        // renderer's finite-filter sees the gap rather than the decimation hiding it.
        let n = 20_000;
        let mut v: Vec<f64> = (0..n).map(|i| i as f64).collect();
        for x in v.iter_mut().take(15_000).skip(5_000) {
            *x = f64::NAN;
        }
        let keep = downsample_for_paint(&v, 64);
        assert!(keep.len() <= 64);
        // At least one kept index falls inside the NaN run (the gap is represented).
        assert!(
            keep.iter().any(|&i| (5_000..15_000).contains(&i)),
            "the NaN gap must be represented in the kept indices"
        );
    }
}
