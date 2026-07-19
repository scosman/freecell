//! Pure merged-region query logic (`architecture.md §2`, merged-cell-ui project).
//!
//! The resident cache stores the sheet's merged regions as a small `Vec<CellRange>` (0-based;
//! merge counts are tiny — a few hundred — so no per-cell index that would blow up on a
//! whole-column merge). These free functions are the synchronous lookups the render + input
//! threads run against that slice, mirroring the old `merge_guard`'s free-predicate style.
//!
//! `region_at` / `regions_intersecting` are linear scans; render bounds their cost by scanning
//! only the viewport's regions (`architecture.md §6`). All coordinates are 0-based `CellRef` /
//! `CellRange` (the file→1-based conversion lives in the engine's `Document`, `§2`).
//!
//! `blocks_fill` moved here from the retired `merge_guard.rs` (fill into a merge stays a rejected
//! edit target — `functional_spec.md F6`); the interim `blocks_row_op`/`blocks_col_op`
//! insert/delete predicates are **deleted** (the engine now displaces merges across insert/delete,
//! so those ops are no longer blocked, `§5`).

use crate::refs::{CellRange, CellRef};

/// The merged region covering `cell` (anchor or any covered cell), or `None` when `cell` is in no
/// region. A covered cell resolves to its region (whose `start` is the anchor); regions never
/// overlap, so the first hit is the only hit.
pub fn region_at(merges: &[CellRange], cell: CellRef) -> Option<CellRange> {
    merges.iter().copied().find(|m| m.contains(cell))
}

/// The anchor (top-left) a covered `cell` edits/selects to — `region_at(cell).map(|r| r.start)`.
/// `None` when `cell` is in no region (it is its own target).
pub fn anchor_of(merges: &[CellRange], cell: CellRef) -> Option<CellRef> {
    region_at(merges, cell).map(|r| r.start)
}

/// Every region that intersects `range` (shares ≥1 cell) — the set a toggle unmerges, the
/// data-loss scan inspects, and render draws for the viewport (`architecture.md §6`, `§8`).
/// Order follows `merges` (stable).
pub fn regions_intersecting(merges: &[CellRange], range: CellRange) -> Vec<CellRange> {
    merges
        .iter()
        .copied()
        .filter(|m| m.intersects(&range))
        .collect()
}

/// Grows `range` until it fully contains every region it touches — the range-selection fixpoint
/// (`architecture.md §7`, `functional_spec.md F4`). Monotonic (the box only grows) and bounded by
/// the sheet, so it terminates; a region pulled in at a new edge can extend that edge onto a
/// *different* region, which the outer loop then also pulls in (chained pull-in). O(n²) worst with
/// `n` = merge count (small). A `range` already containing (or disjoint from) every region is
/// returned unchanged.
pub fn expand_to_regions(merges: &[CellRange], range: CellRange) -> CellRange {
    let mut result = range;
    loop {
        let mut changed = false;
        for m in merges {
            if m.intersects(&result) && !contains_range(&result, m) {
                result = bounding_box(&result, m);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    result
}

/// Whether a fill (⌘D / ⌘R) over `target` (0-based, inclusive) touches any region — i.e. any merge
/// **intersects** `target` (`functional_spec.md F6` documented limitation). Fill into a merge is
/// rejected (the engine rejects covered-cell writes), so this predicate gates the fill.
///
/// `target` is the whole **selection rectangle**, a conservative *superset* of the cells the fill
/// actually writes: it also contains the seed line (top row for ⌘D / left column for ⌘R), which is
/// read, not overwritten. The one intentionally-unguarded read is the single-cell pull-from-
/// neighbor seed (the cell above / to the left of a lone selected cell), which lies **outside** the
/// selection rectangle — reading a value out of a merged neighbor is harmless. (Moved verbatim from
/// the retired `merge_guard.rs`.)
pub fn blocks_fill(merges: &[CellRange], target: CellRange) -> bool {
    merges.iter().any(|m| m.intersects(&target))
}

/// Whether `outer` fully contains `inner` (both corners inside). Used by the fixpoint to tell
/// "already swallowed" from "cuts through".
fn contains_range(outer: &CellRange, inner: &CellRange) -> bool {
    outer.contains(inner.start) && outer.contains(inner.end)
}

/// The smallest range covering both `a` and `b` (their corner-wise union).
fn bounding_box(a: &CellRange, b: &CellRange) -> CellRange {
    CellRange::new(
        CellRef::new(a.start.row.min(b.start.row), a.start.col.min(b.start.col)),
        CellRef::new(a.end.row.max(b.end.row), a.end.col.max(b.end.col)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// K7:L10 in A1 → 0-based rows 6..=9, cols 10..=11 (the long-standing merge fixture).
    fn k7_l10() -> CellRange {
        CellRange::new(CellRef::new(6, 10), CellRef::new(9, 11))
    }

    #[test]
    fn region_at_and_anchor_of() {
        let merges = [k7_l10()];
        // The anchor resolves to its own region + anchor.
        assert_eq!(region_at(&merges, CellRef::new(6, 10)), Some(k7_l10()));
        assert_eq!(
            anchor_of(&merges, CellRef::new(6, 10)),
            Some(CellRef::new(6, 10))
        );
        // A covered (non-anchor) cell resolves to the same region / anchor.
        assert_eq!(region_at(&merges, CellRef::new(9, 11)), Some(k7_l10()));
        assert_eq!(
            anchor_of(&merges, CellRef::new(8, 11)),
            Some(CellRef::new(6, 10))
        );
        // A cell outside every region → None.
        assert_eq!(region_at(&merges, CellRef::new(0, 0)), None);
        assert_eq!(anchor_of(&merges, CellRef::new(5, 10)), None);
        // An empty merge list never resolves.
        assert_eq!(region_at(&[], CellRef::new(6, 10)), None);
    }

    #[test]
    fn regions_intersecting_edge_disjoint_and_multiple() {
        let a = k7_l10(); // rows 6..=9, cols 10..=11
        let b = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1)); // A1:B2
        let merges = [a, b];
        // A range clipping the merge's top-left corner hits it (edge-touch counts as intersection).
        let clip = CellRange::new(CellRef::new(0, 0), CellRef::new(6, 10));
        assert_eq!(regions_intersecting(&merges, clip), vec![a, b]);
        // A range strictly between the two hits neither.
        let gap = CellRange::new(CellRef::new(3, 5), CellRef::new(4, 6));
        assert!(regions_intersecting(&merges, gap).is_empty());
        // A range fully inside `a` hits only `a`.
        let inside = CellRange::single(CellRef::new(8, 11));
        assert_eq!(regions_intersecting(&merges, inside), vec![a]);
    }

    #[test]
    fn expand_to_regions_single_and_contained() {
        let merges = [k7_l10()];
        // A range cutting through the merge grows to swallow the whole region.
        let cut = CellRange::new(CellRef::new(6, 10), CellRef::new(6, 10)); // just the anchor
        assert_eq!(expand_to_regions(&merges, cut), k7_l10());
        // A range already containing the region is unchanged.
        let big = CellRange::new(CellRef::new(0, 0), CellRef::new(20, 20));
        assert_eq!(expand_to_regions(&merges, big), big);
        // A range disjoint from every region is unchanged.
        let far = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1));
        assert_eq!(expand_to_regions(&merges, far), far);
        // No merges → identity.
        assert_eq!(expand_to_regions(&[], big), big);
    }

    #[test]
    fn expand_to_regions_chains_pull_in() {
        // Two regions staggered so swallowing the first extends an edge onto the second, which the
        // fixpoint must then also pull in.
        let r1 = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 3)); // rows 0..=1, cols 0..=3
        let r2 = CellRange::new(CellRef::new(3, 3), CellRef::new(5, 5)); // rows 3..=5, cols 3..=5
        let merges = [r1, r2];
        // Start touching only r1; expanding to r1 reaches col 3, which reaches r2 → both swallowed.
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0));
        let out = expand_to_regions(&merges, seed);
        assert_eq!(out, CellRange::new(CellRef::new(0, 0), CellRef::new(5, 5)));
    }

    #[test]
    fn blocks_fill_on_intersection() {
        let merges = [k7_l10()];
        // A fill target overlapping the merge blocks; a disjoint one doesn't.
        assert!(blocks_fill(
            &merges,
            CellRange::new(CellRef::new(6, 10), CellRef::new(9, 10)) // inside the merge
        ));
        assert!(blocks_fill(
            &merges,
            CellRange::new(CellRef::new(0, 11), CellRef::new(6, 11)) // clips the top edge
        ));
        assert!(!blocks_fill(
            &merges,
            CellRange::new(CellRef::new(0, 0), CellRef::new(5, 9)) // strictly above-left, disjoint
        ));
        // An empty merge list never blocks a fill.
        assert!(!blocks_fill(
            &[],
            CellRange::new(CellRef::new(0, 0), CellRef::new(9, 9))
        ));
    }
}
