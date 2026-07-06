//! Pure cumulative-stacking math shared by the stacked / 100%-stacked **bar** and **area**
//! widgets. gpui-free so it is unit-tested without a GPU.
//!
//! gpui-component ships a `Stack` primitive (a d3-shape port) that computes the same
//! cumulative `(y0, y1)` per (series, category) — but it only produces *numbers*, it paints
//! nothing (`plot/shape/stack.rs`). The cumulative math is a few lines, so we inline it here
//! and reuse one implementation across bars (rectangles) and areas (hand-rolled polygons),
//! plus the percent normalize pass the `Stack` primitive has **no** mode for.
//!
//! Negatives: Excel stacks negative values below the baseline. The PoC's in-scope demo data is
//! all-positive, so for simplicity (and to keep a percent stack meaningful) negatives are
//! clamped to zero when accumulating. A ship-quality follow-on would split positive/negative
//! stacks; noted, not built.

/// One stacked segment for a (series, category): the cumulative lower and upper bound in
/// value space (the series sits on the running total of the series below it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub lo: f64,
    pub hi: f64,
}

impl Segment {
    /// The segment's height (its own contribution to the stack).
    pub fn height(&self) -> f64 {
        self.hi - self.lo
    }
}

fn clamp_non_negative(v: f64) -> f64 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

/// Per-category totals across every series (the height of each stacked column).
pub fn category_totals(series_values: &[Vec<f64>], n_categories: usize) -> Vec<f64> {
    let mut totals = vec![0.0_f64; n_categories];
    for values in series_values {
        for (c, total) in totals.iter_mut().enumerate() {
            *total += clamp_non_negative(values.get(c).copied().unwrap_or(0.0));
        }
    }
    totals
}

/// Cumulative stacked segments `segments[series][category]`: each series stacks on the running
/// total of the series before it (input order = bottom→top).
pub fn stacked_segments(series_values: &[Vec<f64>], n_categories: usize) -> Vec<Vec<Segment>> {
    let mut running = vec![0.0_f64; n_categories];
    let mut out = Vec::with_capacity(series_values.len());
    for values in series_values {
        let mut row = Vec::with_capacity(n_categories);
        for (c, run) in running.iter_mut().enumerate() {
            let v = clamp_non_negative(values.get(c).copied().unwrap_or(0.0));
            let lo = *run;
            let hi = lo + v;
            *run = hi;
            row.push(Segment { lo, hi });
        }
        out.push(row);
    }
    out
}

/// Stacked segments normalized so each category's stack spans `0..100` (percent). A category
/// whose total is zero collapses to empty segments (`lo == hi == 0`).
pub fn percent_segments(series_values: &[Vec<f64>], n_categories: usize) -> Vec<Vec<Segment>> {
    let totals = category_totals(series_values, n_categories);
    let mut segs = stacked_segments(series_values, n_categories);
    for row in &mut segs {
        for (c, seg) in row.iter_mut().enumerate() {
            let total = totals[c];
            if total > 0.0 {
                seg.lo = seg.lo / total * 100.0;
                seg.hi = seg.hi / total * 100.0;
            } else {
                seg.lo = 0.0;
                seg.hi = 0.0;
            }
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Vec<f64>> {
        vec![
            vec![10.0, 15.0], // bottom
            vec![20.0, 25.0], // middle
            vec![30.0, 35.0], // top
        ]
    }

    #[test]
    fn stacked_baselines_are_cumulative() {
        let segs = stacked_segments(&sample(), 2);
        // Each series' top is the next series' bottom, per category.
        for c in 0..2 {
            for s in 0..segs.len() - 1 {
                assert_eq!(
                    segs[s][c].hi,
                    segs[s + 1][c].lo,
                    "segment {s} top must equal segment {} bottom at category {c}",
                    s + 1
                );
            }
        }
        // First segment starts at zero; last segment top equals the category total.
        let totals = category_totals(&sample(), 2);
        for c in 0..2 {
            assert_eq!(segs[0][c].lo, 0.0);
            assert_eq!(segs[segs.len() - 1][c].hi, totals[c]);
        }
        assert_eq!(totals, vec![60.0, 75.0]);
    }

    #[test]
    fn percent_stacks_sum_to_100() {
        let segs = percent_segments(&sample(), 2);
        for c in 0..2 {
            // The stack fills exactly 0..100.
            assert_eq!(segs[0][c].lo, 0.0);
            assert!((segs[segs.len() - 1][c].hi - 100.0).abs() < 1e-9);
            // The segment heights sum to 100.
            let sum: f64 = segs.iter().map(|row| row[c].height()).sum();
            assert!((sum - 100.0).abs() < 1e-9, "category {c} sum {sum} != 100");
        }
    }

    #[test]
    fn zero_total_category_collapses() {
        let segs = percent_segments(&[vec![0.0], vec![0.0]], 1);
        for row in &segs {
            assert_eq!(row[0], Segment { lo: 0.0, hi: 0.0 });
        }
    }

    #[test]
    fn negatives_are_clamped_when_stacking() {
        let segs = stacked_segments(&[vec![-5.0], vec![10.0]], 1);
        assert_eq!(segs[0][0], Segment { lo: 0.0, hi: 0.0 });
        assert_eq!(segs[1][0], Segment { lo: 0.0, hi: 10.0 });
    }
}
