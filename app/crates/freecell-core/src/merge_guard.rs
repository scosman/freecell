//! The merged-cell insert/delete guard predicate (`functional_spec.md §5.3`,
//! `components/grid_structure.md §Insert/delete + merge guard`).
//!
//! Merged cells are not yet supported (a deferred project), so an insert/delete that would
//! **displace** a merge must be blocked before it corrupts the sheet. This is the single shared
//! predicate used by both guard layers: the UI (disable the menu item) and the worker
//! (authoritative re-check → dialog). It is engine-free — merges are parsed into
//! [`CellRange`]s (0-based) by the caller from `worksheet.merge_cells`.
//!
//! Conservative by design: an insert at row `r` displaces everything at/after `r`, and a delete
//! of a run starting at `r` likewise — so **one** predicate ("any merge at/after the affected
//! index") serves both, keyed on the operation's start index. Merges strictly above/left of the
//! edit don't block.

use crate::refs::CellRange;

/// Whether a row insert/delete whose first affected row is `row` (0-based) would displace any
/// merge — i.e. any merge extends to or past `row`. Insert-above at header `R` affects `R`;
/// insert-below affects `R+1`; delete of a run starting at `R` affects `R`.
pub fn blocks_row_op(merges: &[CellRange], row: u32) -> bool {
    merges.iter().any(|m| m.end.row >= row)
}

/// Whether a column insert/delete whose first affected column is `col` (0-based) would displace
/// any merge (the column analog of [`blocks_row_op`]).
pub fn blocks_col_op(merges: &[CellRange], col: u32) -> bool {
    merges.iter().any(|m| m.end.col >= col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::CellRef;

    #[test]
    fn merge_guard_predicate() {
        // K7:L10 in A1 → 0-based rows 6..=9, cols 10..=11 (the component-doc fixture; the doc's
        // "7/10/11 blocks/blocks/allows" is 1-based — 0-based that is 6/9/10).
        let merges = [CellRange::new(CellRef::new(6, 10), CellRef::new(9, 11))];
        // Row op at/before the merge's last row (9) blocks; strictly past it allows.
        assert!(blocks_row_op(&merges, 6)); // 1-based row 7
        assert!(blocks_row_op(&merges, 9)); // 1-based row 10 (the merge's bottom edge)
        assert!(!blocks_row_op(&merges, 10)); // 1-based row 11 — below the merge, allowed
                                              // Column op: last col is 11 → blocks at/before 11, allows at 12.
        assert!(blocks_col_op(&merges, 9)); // 1-based col J(10)
        assert!(blocks_col_op(&merges, 11)); // 1-based col L(12) — the merge's right edge
        assert!(!blocks_col_op(&merges, 12)); // 1-based col M(13) — right of the merge
                                              // An empty merge list never blocks anything.
        assert!(!blocks_row_op(&[], 0));
        assert!(!blocks_col_op(&[], 0));
    }
}
