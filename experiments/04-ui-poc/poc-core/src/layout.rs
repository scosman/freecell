//! Variable-size axis virtualization: map a scroll offset to the visible index range
//! and an index to its pixel offset, for either the row or column axis.
//!
//! **Why not a plain prefix-sum array?** The Excel-max grid is 1,048,576 rows ×
//! 16,384 cols (functional_spec §5.4). A full cumulative-size array over 1M+ rows would
//! be tens of MB and cost O(n) to build. Instead this uses a **two-level segment sum**:
//! we precompute the cumulative size at each *block* boundary (block size `B`), then a
//! lookup binary-searches the O(n/B) block sums and does a short O(B) scan inside the
//! landing block. Build is O(n) additions but O(n/B) memory; `offset_of` / `index_at`
//! are O(log(n/B) + B). Sizes come from a per-index sizer closure
//! (`SyntheticSheet::col_width` / `row_height`), so nothing per-cell is materialized.
//!
//! All arithmetic that accumulates across the whole axis is done in `f64` to avoid
//! `f32` precision loss over ~10^5–10^6 cells; individual sizes stay `f32`.

use std::ops::Range;

/// Number of indices summed per block. Chosen so the block-sum vector stays small
/// (Excel-max rows → ~2,048 blocks) while the in-block scan stays cheap.
const BLOCK: u32 = 512;

/// A virtualized axis over `count` variable-size tracks (rows or columns).
pub struct Axis {
    count: u32,
    /// Cumulative size (px) at the *start* of each block: `block_starts[b]` is the sum
    /// of the sizes of all indices before `b * BLOCK`. Length is `num_blocks + 1`, so
    /// the last entry is the total axis size.
    block_starts: Vec<f64>,
    /// Per-index size in px.
    sizer: Box<dyn Fn(u32) -> f32>,
}

impl Axis {
    /// Builds an axis of `count` tracks whose sizes come from `sizer`.
    pub fn new(count: u32, sizer: impl Fn(u32) -> f32 + 'static) -> Self {
        let num_blocks = count.div_ceil(BLOCK);
        let mut block_starts = Vec::with_capacity(num_blocks as usize + 1);
        let mut acc = 0.0_f64;
        block_starts.push(0.0);
        for b in 0..num_blocks {
            let start = b * BLOCK;
            let end = (start + BLOCK).min(count);
            for i in start..end {
                acc += sizer(i) as f64;
            }
            block_starts.push(acc);
        }
        Self {
            count,
            block_starts,
            sizer: Box::new(sizer),
        }
    }

    /// The number of tracks on this axis.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The total size (px) of the whole axis.
    pub fn total(&self) -> f64 {
        *self.block_starts.last().unwrap_or(&0.0)
    }

    /// The pixel offset of the *start* of track `index`. `offset_of(count)` returns the
    /// total axis size. Indices past `count` clamp to the total.
    pub fn offset_of(&self, index: u32) -> f64 {
        let index = index.min(self.count);
        let block = index / BLOCK;
        let mut acc = self.block_starts[block as usize];
        let block_start = block * BLOCK;
        for i in block_start..index {
            acc += (self.sizer)(i) as f64;
        }
        acc
    }

    /// The size (px) of a single track. Out-of-range indices return `0.0`.
    pub fn size_of(&self, index: u32) -> f32 {
        if index >= self.count {
            0.0
        } else {
            (self.sizer)(index)
        }
    }

    /// The first track index whose *start* is at or after `offset` px — equivalently,
    /// the index of the track containing `offset` (its start `<= offset < end`). Clamped
    /// to `[0, count]`. `index_at(total())` returns `count`.
    pub fn index_at(&self, offset: f64) -> u32 {
        if offset <= 0.0 {
            return 0;
        }
        if offset >= self.total() {
            return self.count;
        }
        // Binary-search the block whose cumulative start bracket contains `offset`.
        // We want the largest `b` with `block_starts[b] <= offset`.
        let mut lo = 0usize;
        let mut hi = self.block_starts.len() - 1;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.block_starts[mid] <= offset {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let block = lo as u32;
        let mut acc = self.block_starts[block as usize];
        let block_start = block * BLOCK;
        let block_end = (block_start + BLOCK).min(self.count);
        for i in block_start..block_end {
            let next = acc + (self.sizer)(i) as f64;
            if next > offset {
                return i;
            }
            acc = next;
        }
        block_end
    }

    /// The half-open range of track indices visible in a viewport of `extent` px
    /// starting at scroll offset `scroll`, expanded by `overscan` tracks on each side
    /// and clamped to `[0, count)`.
    pub fn visible_range(&self, scroll: f64, extent: f64, overscan: u32) -> Range<u32> {
        if self.count == 0 {
            return 0..0;
        }
        let first = self.index_at(scroll);
        // `index_at` returns the track containing `scroll + extent`; include it, hence
        // `+ 1` before clamping. `end` is exclusive.
        let last = self.index_at(scroll + extent).min(self.count - 1);
        let start = first.saturating_sub(overscan);
        let end = (last + 1 + overscan).min(self.count);
        start..end
    }
}

impl std::fmt::Debug for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Axis")
            .field("count", &self.count)
            .field("blocks", &self.block_starts.len())
            .field("total", &self.total())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic, varied sizer: base 20 px plus a small index-derived wiggle, with
    /// every 7th track "wide". Mirrors the shape of the real synthetic sizers.
    fn varied_sizer(i: u32) -> f32 {
        if i % 7 == 0 {
            100.0 + (i % 13) as f32
        } else {
            20.0 + (i % 5) as f32
        }
    }

    fn naive_total(count: u32, sizer: impl Fn(u32) -> f32) -> f64 {
        (0..count).map(|i| sizer(i) as f64).sum()
    }

    #[test]
    fn axis_total_matches_naive_sum() {
        for count in [0u32, 1, 7, 512, 513, 1000, 5000] {
            let axis = Axis::new(count, varied_sizer);
            let naive = naive_total(count, varied_sizer);
            assert!(
                (axis.total() - naive).abs() < 1e-6,
                "count={count}: axis.total()={} naive={naive}",
                axis.total()
            );
        }
    }

    #[test]
    fn offset_and_index_roundtrip() {
        let count = 5000u32;
        let axis = Axis::new(count, varied_sizer);
        for i in [0u32, 1, 6, 7, 8, 511, 512, 513, 2500, 4999] {
            let off = axis.offset_of(i);
            // The track starting exactly at `off` is `i` (index_at returns the track
            // whose start <= off < end).
            assert_eq!(axis.index_at(off), i, "roundtrip failed at index {i}");
            // A point just inside the track still maps back to `i`.
            let mid = off + (axis.size_of(i) as f64) / 2.0;
            assert_eq!(axis.index_at(mid), i, "midpoint of track {i} misclassified");
        }
        // offset_of(count) is the total; index_at(total) is count.
        assert!((axis.offset_of(count) - axis.total()).abs() < 1e-6);
        assert_eq!(axis.index_at(axis.total()), count);
        assert_eq!(axis.index_at(-5.0), 0);
        assert_eq!(axis.index_at(0.0), 0);
    }

    #[test]
    fn visible_range_covers_viewport_and_clamps() {
        let count = 2000u32;
        let axis = Axis::new(count, varied_sizer);
        let scroll = axis.offset_of(300);
        let extent = 400.0;
        let overscan = 3;
        let range = axis.visible_range(scroll, extent, overscan);

        // Covers the viewport: the first data track fully covering `scroll` is inside
        // the range, and the track covering `scroll + extent` is inside too.
        let first_visible = axis.index_at(scroll);
        let last_visible = axis.index_at(scroll + extent);
        assert!(range.start <= first_visible, "range must start at/before first visible");
        assert!(range.end > last_visible, "range must extend past last visible");

        // Overscan applied on the leading side (300 - 3 = 297).
        assert_eq!(range.start, first_visible - overscan);
        // Clamped to [0, count).
        assert!(range.start < count && range.end <= count);
    }

    #[test]
    fn visible_range_clamps_at_edges() {
        let count = 100u32;
        let axis = Axis::new(count, varied_sizer);
        // At scroll 0 the range starts at 0 (overscan can't go negative).
        let top = axis.visible_range(0.0, 50.0, 5);
        assert_eq!(top.start, 0);
        // Scrolled to the very bottom, end clamps to count.
        let bottom = axis.visible_range(axis.total(), 50.0, 5);
        assert_eq!(bottom.end, count);
    }

    #[test]
    fn empty_axis_is_well_behaved() {
        let axis = Axis::new(0, varied_sizer);
        assert_eq!(axis.total(), 0.0);
        assert_eq!(axis.index_at(0.0), 0);
        assert_eq!(axis.index_at(100.0), 0);
        assert_eq!(axis.visible_range(0.0, 100.0, 4), 0..0);
    }

    #[test]
    fn handles_excel_max_rows_without_oom() {
        // 1,048,576 rows. The block-sum vector is ~2,048 f64 (~16 KB), NOT a per-row
        // array. Build + query near the end must be fast and correct.
        let count = datagen::EXCEL_MAX_ROWS;
        let axis = Axis::new(count, |i| if i % 20 == 0 { 40.0 } else { 22.0 });
        // block_starts holds one entry per block plus one: ~n/BLOCK + 1, far below n.
        assert!(
            axis.block_starts.len() < (count as usize / 100),
            "segment structure must be O(n/BLOCK), not O(n)"
        );
        // Query near the end round-trips.
        let near_end = count - 3;
        let off = axis.offset_of(near_end);
        assert_eq!(axis.index_at(off), near_end);
        // A visible range deep in the grid is small (viewport-sized), not millions.
        let range = axis.visible_range(off, 900.0, 8);
        assert!(range.end - range.start < 100, "visible range must be viewport-sized");
    }
}
