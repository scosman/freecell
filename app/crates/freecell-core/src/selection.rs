//! `SelectionModel` + keyboard-motion rules (`functional_spec.md §3.2`,
//! `components/grid.md §Public interface`).
//!
//! The grid binds every navigation key to [`apply_motion`], a pure function so it is
//! unit-tested headless on Linux (the grid just wires keys → motions and repaints). The
//! model is an `(anchor, active)` pair: `active` is the outlined cell shown in the ref
//! box; `anchor` is the fixed corner a range extends from. Collapsing motions set both to
//! the new cell; extending motions move only `active`.

use crate::limits;
use crate::refs::{column_label, CellRange, CellRef};

/// A sheet's dimensions, used to clamp motions to valid cells. Motions never move past
/// `[0, rows) × [0, cols)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetDims {
    pub rows: u32,
    pub cols: u32,
}

impl SheetDims {
    pub const fn new(rows: u32, cols: u32) -> Self {
        Self { rows, cols }
    }
}

/// A cardinal direction for a motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// A navigation motion. Each maps to one key (or key+modifier) the grid binds:
/// - [`Motion::Move`] — arrow keys (and Tab→`Right`, Enter→`Down`, Shift+Tab→`Left`,
///   Shift+Enter→`Up`, mapped at the keymap layer): move one step, **collapse**.
/// - [`Motion::Extend`] — Shift+arrow: move `active` one step, **keep** the anchor.
/// - [`Motion::JumpEdge`] — Cmd/Ctrl+arrow: jump `active` to the **edge of the data region**,
///   collapse (`functional_spec.md §4`). The occupancy that resolves this lives in the engine, past
///   the published viewport, so the grid routes this motion to an async worker query
///   ([`resolve_edge`], D4.1 Option A) rather than [`apply_motion`]; the synchronous `apply_motion`
///   arm keeps the sheet-edge fallback for headless/occupancy-free callers.
/// - [`Motion::ExtendEdge`] — Cmd/Ctrl+Shift+arrow: jump to the edge-of-data target, keep the
///   anchor (same async resolution as `JumpEdge`).
/// - [`Motion::Page`] / [`Motion::ExtendPage`] — Page Up/Down by `rows` (the grid passes
///   its current page height in rows).
/// - [`Motion::RowStart`] / [`Motion::ExtendRowStart`] — Home / Shift+Home: to column 0 of
///   the active row.
/// - [`Motion::DocumentStart`] / [`Motion::ExtendDocumentStart`] — Cmd/Ctrl+Home /
///   Cmd/Ctrl+Shift+Home: to cell A1 (the sheet origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Move(Direction),
    Extend(Direction),
    JumpEdge(Direction),
    ExtendEdge(Direction),
    Page { direction: Direction, rows: u32 },
    ExtendPage { direction: Direction, rows: u32 },
    RowStart,
    ExtendRowStart,
    DocumentStart,
    ExtendDocumentStart,
}

/// The current selection: an active cell and the anchor a range extends from. A single
/// selection has `anchor == active`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionModel {
    pub anchor: CellRef,
    pub active: CellRef,
}

impl SelectionModel {
    /// A single-cell selection at `cell`.
    pub fn single(cell: CellRef) -> Self {
        Self {
            anchor: cell,
            active: cell,
        }
    }

    /// Whether exactly one cell is selected.
    pub fn is_single(&self) -> bool {
        self.anchor == self.active
    }

    /// The normalized rectangular range spanning anchor→active.
    pub fn range(&self) -> CellRange {
        CellRange::new(self.anchor, self.active)
    }

    /// A1 notation for the ref box: `"B7"` (single) or `"B2:D9"` (range).
    pub fn to_a1(&self) -> String {
        self.range().to_a1()
    }
}

impl Default for SelectionModel {
    /// A1 selected — the state a fresh sheet opens on (`components/grid.md`).
    fn default() -> Self {
        Self::single(CellRef::new(0, 0))
    }
}

/// Whether `range` spans every row of the sheet (a full-column / whole-sheet selection).
pub fn spans_all_rows(range: &CellRange) -> bool {
    range.start.row == 0 && range.end.row == limits::MAX_ROWS - 1
}

/// Whether `range` spans every column of the sheet (a full-row / whole-sheet selection).
pub fn spans_all_cols(range: &CellRange) -> bool {
    range.start.col == 0 && range.end.col == limits::MAX_COLS - 1
}

/// Whether `sel` is a full-column header selection (spans every row of one or more columns).
/// A whole-sheet selection also qualifies (it is rendered in the column form, `A:XFD`).
pub fn is_full_column_selection(sel: &SelectionModel) -> bool {
    spans_all_rows(&sel.range())
}

/// Whether `sel` is a full-row header selection (spans every column of one or more rows, and is
/// **not** also a full-column/whole-sheet selection, which takes the column form).
pub fn is_full_row_selection(sel: &SelectionModel) -> bool {
    let range = sel.range();
    spans_all_cols(&range) && !spans_all_rows(&range)
}

/// The reference-box text for a selection (`components/grid_structure.md §Public interface`):
/// - a full-column selection → `C:C` / `C:E` (or the whole sheet → `A:XFD`),
/// - a full-row selection → `3:3` / `3:7`,
/// - otherwise ordinary A1 (`A1` / `B2:D9`).
///
/// Full extents render as their band form so a header selection reads like Excel's name box; a
/// bounded selection falls through to [`CellRange::to_a1`].
pub fn format_selection_ref(sel: &SelectionModel) -> String {
    let range = sel.range();
    if spans_all_rows(&range) {
        // Column form (a full column, several full columns, or the whole sheet → A:XFD).
        let c0 = column_label(range.start.col);
        let c1 = column_label(range.end.col);
        format!("{c0}:{c1}")
    } else if spans_all_cols(&range) {
        // Row form (full rows) — 1-based labels.
        format!("{}:{}", range.start.row + 1, range.end.row + 1)
    } else {
        range.to_a1()
    }
}

/// Steps a cell one track in `direction`, clamping to `[0, dims)`. Uses `saturating_add`
/// (like [`step_by`]) so an out-of-range active cell can never panic on overflow.
fn step(cell: CellRef, direction: Direction, dims: SheetDims) -> CellRef {
    match direction {
        Direction::Up => CellRef::new(cell.row.saturating_sub(1), cell.col),
        Direction::Down => CellRef::new(
            cell.row.saturating_add(1).min(dims.rows.saturating_sub(1)),
            cell.col,
        ),
        Direction::Left => CellRef::new(cell.row, cell.col.saturating_sub(1)),
        Direction::Right => CellRef::new(
            cell.row,
            cell.col.saturating_add(1).min(dims.cols.saturating_sub(1)),
        ),
    }
}

/// Steps a cell `n` tracks in `direction`, clamping to `[0, dims)`.
fn step_by(cell: CellRef, direction: Direction, n: u32, dims: SheetDims) -> CellRef {
    match direction {
        Direction::Up => CellRef::new(cell.row.saturating_sub(n), cell.col),
        Direction::Down => CellRef::new(
            (cell.row.saturating_add(n)).min(dims.rows.saturating_sub(1)),
            cell.col,
        ),
        Direction::Left => CellRef::new(cell.row, cell.col.saturating_sub(n)),
        Direction::Right => CellRef::new(
            cell.row,
            (cell.col.saturating_add(n)).min(dims.cols.saturating_sub(1)),
        ),
    }
}

/// Jumps a cell to the **sheet** edge in `direction` — the synchronous fallback for
/// [`Motion::JumpEdge`]/[`ExtendEdge`] in [`apply_motion`]. The real ⌘+arrow uses edge-of-**data**
/// ([`resolve_edge`]), resolved worker-side because occupancy lives in the engine past the published
/// viewport (`functional_spec.md §4`, D4.1); this arm still applies when a caller drives `apply_motion`
/// with those motions directly (e.g. headless).
fn edge(cell: CellRef, direction: Direction, dims: SheetDims) -> CellRef {
    match direction {
        Direction::Up => CellRef::new(0, cell.col),
        Direction::Down => CellRef::new(dims.rows.saturating_sub(1), cell.col),
        Direction::Left => CellRef::new(cell.row, 0),
        Direction::Right => CellRef::new(cell.row, dims.cols.saturating_sub(1)),
    }
}

/// The **edge-of-data** target index along one line of travel, applying the exact Excel Ctrl+Arrow
/// rule (`functional_spec.md §4`). `pos` is the active cell's index on the line (its row for a
/// vertical motion, its column for a horizontal one); `forward` is `true` for Down/Right (increasing
/// index) and `false` for Up/Left. `len` is the number of cells on the line (the sheet's rows or
/// cols); `occupied` is the line's populated indices **sorted ascending, distinct**. The result is
/// always in `[0, len)`.
///
/// The rule, from `pos` moving one step (`+1`/`-1`) in the direction:
/// - already at the boundary edge (no next cell) → stay on `pos`;
/// - active cell **and** its neighbour both occupied → jump to the **last** occupied cell of the
///   contiguous run (the cell before the first gap, or the boundary edge if the run reaches it);
/// - otherwise (active empty, or neighbour empty) → **skip to the next** occupied cell, or the
///   boundary edge if none exists before it.
///
/// **Complexity: O(log occupied)** — every step is a binary search over the sorted slice, never a
/// per-index walk across empty space. Crucially, jumping through a gap (e.g. ⌘+Down in an empty
/// 1M-row column) is a single lookup, not ~1M probes — the huge-sheet guarantee (`§4` "correctness /
/// responsiveness"). The contiguous-run end is found by binary-searching the run's constant
/// `occupied[j] - j` signature (consecutive indices share it; a gap strictly increases it), so it is
/// also O(log occupied), not O(run length).
fn edge_of_data_index(pos: u32, forward: bool, len: u32, occupied: &[u32]) -> u32 {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    // Already at the sheet edge in this direction — no move (whatever the occupancy).
    if (forward && pos >= last) || (!forward && pos == 0) {
        return pos;
    }
    let is_occupied = |i: u32| occupied.binary_search(&i).is_ok();
    let adj = if forward { pos + 1 } else { pos - 1 };

    if is_occupied(pos) && is_occupied(adj) {
        // On a contiguous run: jump to its far end. Along a run of consecutive indices the signature
        // `d = occupied[j] - j` is constant; a gap strictly increases it, and (indices being sorted +
        // distinct) `d` is non-decreasing — so the run's index range is the maximal equal-`d` block,
        // found by binary search. `k` is `pos`'s slot (both `pos` and `adj` are occupied ⇒ present).
        let k = occupied.binary_search(&pos).expect("pos is occupied");
        let d = |j: usize| occupied[j] as i64 - j as i64;
        let dk = d(k);
        if forward {
            // Far end = last slot whose `d == dk` (the equal block ends where `d` first exceeds it).
            let mut lo = k;
            let mut hi = occupied.len();
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if d(mid) <= dk {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            occupied[lo - 1]
        } else {
            // Far end = first slot whose `d == dk` (the equal block starts where `d` reaches it).
            let mut lo = 0;
            let mut hi = k;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if d(mid) < dk {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            occupied[lo]
        }
    } else if forward {
        // Active empty, or the neighbour empty: skip to the next occupied index strictly past `pos`,
        // else the boundary edge.
        let idx = occupied.partition_point(|&v| v <= pos);
        if idx < occupied.len() {
            occupied[idx]
        } else {
            last
        }
    } else {
        let idx = occupied.partition_point(|&v| v < pos);
        if idx > 0 {
            occupied[idx - 1]
        } else {
            0
        }
    }
}

/// Resolves the **edge-of-data** target cell for [`Motion::JumpEdge`]/[`ExtendEdge`] — the exact
/// Excel/Sheets ⌘+arrow behavior (`functional_spec.md §4`). `occupied_line` is the populated indices
/// **on the active cell's line of travel** — its column's occupied rows for Up/Down, its row's
/// occupied cols for Left/Right — **sorted ascending, distinct**. The worker builds this from the
/// engine (occupancy lives there, past the published viewport, `architecture.md §4`, D4.1 Option A).
/// Pure and total (O(log occupied), see [`edge_of_data_index`]): the result is clamped to a valid
/// `[0, dims)` coordinate. The caller applies it — collapse for `JumpEdge` (`single(target)`),
/// keep-anchor for `ExtendEdge`.
pub fn resolve_edge(
    from: CellRef,
    dir: Direction,
    dims: SheetDims,
    occupied_line: &[u32],
) -> CellRef {
    match dir {
        Direction::Up => CellRef::new(
            edge_of_data_index(from.row, false, dims.rows, occupied_line),
            from.col,
        ),
        Direction::Down => CellRef::new(
            edge_of_data_index(from.row, true, dims.rows, occupied_line),
            from.col,
        ),
        Direction::Left => CellRef::new(
            from.row,
            edge_of_data_index(from.col, false, dims.cols, occupied_line),
        ),
        Direction::Right => CellRef::new(
            from.row,
            edge_of_data_index(from.col, true, dims.cols, occupied_line),
        ),
    }
}

/// Applies `motion` to `sel` on a sheet of `dims`, returning the new selection. Pure and
/// total: every result cell is clamped to a valid `[0, dims)` coordinate.
pub fn apply_motion(sel: SelectionModel, motion: Motion, dims: SheetDims) -> SelectionModel {
    // Guard against a zero-sized sheet (nothing to select) — keep A1.
    if dims.rows == 0 || dims.cols == 0 {
        return SelectionModel::single(CellRef::new(0, 0));
    }

    match motion {
        Motion::Move(d) => SelectionModel::single(step(sel.active, d, dims)),
        Motion::Extend(d) => SelectionModel {
            anchor: sel.anchor,
            active: step(sel.active, d, dims),
        },
        Motion::JumpEdge(d) => SelectionModel::single(edge(sel.active, d, dims)),
        Motion::ExtendEdge(d) => SelectionModel {
            anchor: sel.anchor,
            active: edge(sel.active, d, dims),
        },
        Motion::Page { direction, rows } => {
            SelectionModel::single(step_by(sel.active, direction, rows, dims))
        }
        Motion::ExtendPage { direction, rows } => SelectionModel {
            anchor: sel.anchor,
            active: step_by(sel.active, direction, rows, dims),
        },
        Motion::RowStart => SelectionModel::single(CellRef::new(sel.active.row, 0)),
        Motion::ExtendRowStart => SelectionModel {
            anchor: sel.anchor,
            active: CellRef::new(sel.active.row, 0),
        },
        Motion::DocumentStart => SelectionModel::single(CellRef::new(0, 0)),
        Motion::ExtendDocumentStart => SelectionModel {
            anchor: sel.anchor,
            active: CellRef::new(0, 0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::Direction::*;
    use super::*;
    use crate::limits;

    fn dims() -> SheetDims {
        SheetDims::new(100, 50)
    }

    fn cell(r: u32, c: u32) -> CellRef {
        CellRef::new(r, c)
    }

    #[test]
    fn move_each_direction_collapses() {
        let start = SelectionModel::single(cell(5, 5));
        for (dir, expected) in [
            (Up, cell(4, 5)),
            (Down, cell(6, 5)),
            (Left, cell(5, 4)),
            (Right, cell(5, 6)),
        ] {
            let out = apply_motion(start, Motion::Move(dir), dims());
            assert_eq!(out.active, expected, "{dir:?}");
            assert!(out.is_single(), "Move must collapse the selection");
        }
    }

    #[test]
    fn move_from_range_collapses_to_stepped_active() {
        // A range selection: a plain arrow collapses and steps from the active cell.
        let sel = SelectionModel {
            anchor: cell(2, 2),
            active: cell(6, 8),
        };
        let out = apply_motion(sel, Motion::Move(Down), dims());
        assert_eq!(out, SelectionModel::single(cell(7, 8)));
    }

    #[test]
    fn move_clamps_at_edges() {
        // Top-left corner: Up and Left stay put.
        let tl = SelectionModel::single(cell(0, 0));
        assert_eq!(
            apply_motion(tl, Motion::Move(Up), dims()).active,
            cell(0, 0)
        );
        assert_eq!(
            apply_motion(tl, Motion::Move(Left), dims()).active,
            cell(0, 0)
        );
        // Bottom-right corner: Down and Right stay put.
        let br = SelectionModel::single(cell(99, 49));
        assert_eq!(
            apply_motion(br, Motion::Move(Down), dims()).active,
            cell(99, 49)
        );
        assert_eq!(
            apply_motion(br, Motion::Move(Right), dims()).active,
            cell(99, 49)
        );
    }

    #[test]
    fn extend_keeps_anchor() {
        let sel = SelectionModel::single(cell(5, 5));
        let out = apply_motion(sel, Motion::Extend(Right), dims());
        assert_eq!(out.anchor, cell(5, 5), "anchor stays fixed while extending");
        assert_eq!(out.active, cell(5, 6));
        assert!(!out.is_single());
        // The range spans anchor→active.
        assert_eq!(out.range(), CellRange::new(cell(5, 5), cell(5, 6)));
    }

    #[test]
    fn jump_edge_goes_to_sheet_bound() {
        let sel = SelectionModel::single(cell(5, 5));
        assert_eq!(
            apply_motion(sel, Motion::JumpEdge(Up), dims()).active,
            cell(0, 5)
        );
        assert_eq!(
            apply_motion(sel, Motion::JumpEdge(Down), dims()).active,
            cell(99, 5)
        );
        assert_eq!(
            apply_motion(sel, Motion::JumpEdge(Left), dims()).active,
            cell(5, 0)
        );
        assert_eq!(
            apply_motion(sel, Motion::JumpEdge(Right), dims()).active,
            cell(5, 49)
        );
        assert!(apply_motion(sel, Motion::JumpEdge(Down), dims()).is_single());
    }

    #[test]
    fn extend_edge_keeps_anchor() {
        let sel = SelectionModel::single(cell(5, 5));
        let out = apply_motion(sel, Motion::ExtendEdge(Down), dims());
        assert_eq!(out.anchor, cell(5, 5));
        assert_eq!(out.active, cell(99, 5));
    }

    #[test]
    fn page_moves_by_rows_clamped() {
        let sel = SelectionModel::single(cell(50, 3));
        let up = apply_motion(
            sel,
            Motion::Page {
                direction: Up,
                rows: 20,
            },
            dims(),
        );
        assert_eq!(up.active, cell(30, 3));
        assert!(up.is_single());
        // Page down past the bottom clamps to the last row.
        let down = apply_motion(
            sel,
            Motion::Page {
                direction: Down,
                rows: 1000,
            },
            dims(),
        );
        assert_eq!(down.active, cell(99, 3));
        // Extend variant keeps the anchor.
        let ext = apply_motion(
            sel,
            Motion::ExtendPage {
                direction: Up,
                rows: 20,
            },
            dims(),
        );
        assert_eq!(ext.anchor, cell(50, 3));
        assert_eq!(ext.active, cell(30, 3));
    }

    #[test]
    fn row_start_goes_to_col_zero() {
        let sel = SelectionModel::single(cell(7, 40));
        assert_eq!(
            apply_motion(sel, Motion::RowStart, dims()).active,
            cell(7, 0)
        );
        let ext = apply_motion(sel, Motion::ExtendRowStart, dims());
        assert_eq!(ext.anchor, cell(7, 40));
        assert_eq!(ext.active, cell(7, 0));
    }

    #[test]
    fn document_start_goes_to_a1() {
        // Cmd/Ctrl+Home collapses to the sheet origin regardless of the current cell.
        let sel = SelectionModel::single(cell(7, 40));
        let out = apply_motion(sel, Motion::DocumentStart, dims());
        assert_eq!(out, SelectionModel::single(cell(0, 0)));
        assert!(out.is_single());
    }

    #[test]
    fn extend_document_start_keeps_anchor() {
        // Cmd/Ctrl+Shift+Home extends the range back to A1, keeping the anchor fixed.
        let sel = SelectionModel::single(cell(7, 40));
        let out = apply_motion(sel, Motion::ExtendDocumentStart, dims());
        assert_eq!(out.anchor, cell(7, 40));
        assert_eq!(out.active, cell(0, 0));
        assert_eq!(out.range(), CellRange::new(cell(0, 0), cell(7, 40)));
    }

    #[test]
    fn single_selection_is_single() {
        assert!(SelectionModel::single(cell(3, 3)).is_single());
        assert_eq!(SelectionModel::default().active, cell(0, 0));
    }

    #[test]
    fn selection_to_a1_single_and_range() {
        assert_eq!(SelectionModel::single(cell(6, 1)).to_a1(), "B7");
        let range = SelectionModel {
            anchor: cell(1, 1),
            active: cell(8, 3),
        };
        assert_eq!(range.to_a1(), "B2:D9");
    }

    #[test]
    fn format_selection_ref_all_shapes() {
        let full_col = |c0, c1| SelectionModel {
            anchor: cell(0, c0),
            active: cell(limits::MAX_ROWS - 1, c1),
        };
        let full_row = |r0, r1| SelectionModel {
            anchor: cell(r0, 0),
            active: cell(r1, limits::MAX_COLS - 1),
        };
        // Single cell + rectangle fall through to A1.
        assert_eq!(
            format_selection_ref(&SelectionModel::single(cell(0, 0))),
            "A1"
        );
        assert_eq!(
            format_selection_ref(&SelectionModel {
                anchor: cell(1, 1),
                active: cell(8, 3),
            }),
            "B2:D9"
        );
        // Full columns.
        assert_eq!(format_selection_ref(&full_col(2, 2)), "C:C");
        assert_eq!(format_selection_ref(&full_col(2, 4)), "C:E");
        // Full rows (1-based labels).
        assert_eq!(format_selection_ref(&full_row(2, 2)), "3:3");
        assert_eq!(format_selection_ref(&full_row(2, 6)), "3:7");
        // Select-all → the column form A:XFD (full rows takes precedence).
        let all = SelectionModel {
            anchor: cell(0, 0),
            active: cell(limits::MAX_ROWS - 1, limits::MAX_COLS - 1),
        };
        assert_eq!(format_selection_ref(&all), "A:XFD");
        assert!(is_full_column_selection(&all));
        assert!(!is_full_row_selection(&all));
        assert!(is_full_row_selection(&full_row(2, 2)));
    }

    #[test]
    fn clamps_at_excel_max_bounds() {
        // At the very last cell of an Excel-max sheet, every forward motion stays put.
        let dims = SheetDims::new(limits::MAX_ROWS, limits::MAX_COLS);
        let last = SelectionModel::single(cell(limits::MAX_ROWS - 1, limits::MAX_COLS - 1));
        assert_eq!(
            apply_motion(last, Motion::Move(Down), dims).active,
            last.active
        );
        assert_eq!(
            apply_motion(last, Motion::Move(Right), dims).active,
            last.active
        );
        assert_eq!(
            apply_motion(last, Motion::JumpEdge(Right), dims).active,
            cell(limits::MAX_ROWS - 1, limits::MAX_COLS - 1)
        );
    }

    // ---- Edge-of-data (⌘+arrow) pure algorithm (`functional_spec.md §4`) --------------------
    //
    // `edge_of_data_index`/`resolve_edge` take the line's populated indices as a slice **sorted
    // ascending, distinct** (what the worker collects from the engine); the tests pass those slices
    // directly.

    #[test]
    fn edge_index_active_empty_jumps_to_next_occupied() {
        // Active cell (2) empty, data at 5: land on the first non-empty ahead.
        assert_eq!(edge_of_data_index(2, true, 10, &[5, 6]), 5);
        // Backward: from 8 (empty), data at 3 → 3.
        assert_eq!(edge_of_data_index(8, false, 10, &[1, 3]), 3);
    }

    #[test]
    fn edge_index_active_empty_no_data_goes_to_boundary() {
        // Active empty, nothing ahead → the sheet edge (len-1 forward, 0 backward).
        assert_eq!(edge_of_data_index(2, true, 10, &[]), 9);
        assert_eq!(edge_of_data_index(2, false, 10, &[]), 0);
    }

    #[test]
    fn edge_index_run_stops_at_last_of_run() {
        // Active (2) and neighbour (3) occupied; run is 2..=4, gap at 5 → last of run = 4.
        assert_eq!(edge_of_data_index(2, true, 10, &[2, 3, 4, 7]), 4);
        // Backward: run 6,5,4 from 6 → 4.
        assert_eq!(edge_of_data_index(6, false, 10, &[4, 5, 6]), 4);
    }

    #[test]
    fn edge_index_run_to_boundary_lands_on_edge() {
        // A run reaching the sheet edge → the edge.
        assert_eq!(
            edge_of_data_index(6, true, 10, &[6, 7, 8, 9]),
            9,
            "run to the last row lands on the last row"
        );
        // Backward: a run reaching row 0.
        assert_eq!(edge_of_data_index(3, false, 10, &[0, 1, 2, 3]), 0);
    }

    #[test]
    fn edge_index_gap_crosses_to_next_block() {
        // Active (2) occupied, neighbour (3) empty → cross the gap to the next occupied (7).
        assert_eq!(edge_of_data_index(2, true, 10, &[2, 7, 8]), 7);
        // Backward analog.
        assert_eq!(edge_of_data_index(7, false, 10, &[1, 2, 7]), 2);
    }

    #[test]
    fn edge_index_gap_with_no_further_data_goes_to_boundary() {
        // Active occupied, neighbour empty, nothing else ahead → boundary edge.
        assert_eq!(edge_of_data_index(2, true, 10, &[2]), 9);
        assert_eq!(edge_of_data_index(7, false, 10, &[7]), 0);
    }

    #[test]
    fn edge_index_at_boundary_does_not_move() {
        // Already at the edge in the direction of travel → stay put, whatever the occupancy.
        assert_eq!(edge_of_data_index(9, true, 10, &[9]), 9);
        assert_eq!(edge_of_data_index(0, false, 10, &[]), 0);
    }

    #[test]
    fn edge_index_adjacent_occupied_is_single_step_run() {
        // Active (2) and neighbour (3) occupied but 4 empty → stop at 3 (the run is just 2,3).
        assert_eq!(edge_of_data_index(2, true, 10, &[2, 3]), 3);
    }

    #[test]
    fn edge_index_jumps_across_a_huge_gap_in_one_lookup() {
        // The huge-sheet guarantee: crossing ~1M empty cells is a binary search, not a per-index walk
        // (an O(line-length) implementation would still be *correct* here but pathologically slow).
        let len = limits::MAX_ROWS;
        // Empty column: ⌘+Down from the top lands on the last row.
        assert_eq!(edge_of_data_index(0, true, len, &[]), len - 1);
        // A single far-off cell: ⌘+Down crosses straight to it.
        assert_eq!(edge_of_data_index(0, true, len, &[1_000_000]), 1_000_000);
        // A long contiguous run reaching the boundary: the run-end binary search lands on the last row.
        let run: Vec<u32> = (0..len).collect();
        assert_eq!(edge_of_data_index(5, true, len, &run), len - 1);
    }

    #[test]
    fn resolve_edge_maps_direction_to_the_right_axis() {
        let dims = SheetDims::new(100, 50);
        // A vertical run in column 5 (occupied rows 3,4,5): from (3,5) Down → last of run (5,5).
        let col_run = [3, 4, 5];
        assert_eq!(resolve_edge(cell(3, 5), Down, dims, &col_run), cell(5, 5));
        // Up from (5,5) → top of run (3,5).
        assert_eq!(resolve_edge(cell(5, 5), Up, dims, &col_run), cell(3, 5));
        // A horizontal run in row 7 (occupied cols 2,3,4): Right from (7,2) → (7,4); Left → (7,2).
        let row_run = [2, 3, 4];
        assert_eq!(resolve_edge(cell(7, 2), Right, dims, &row_run), cell(7, 4));
        assert_eq!(resolve_edge(cell(7, 4), Left, dims, &row_run), cell(7, 2));
    }

    #[test]
    fn resolve_edge_empty_sheet_goes_to_sheet_edge() {
        let dims = SheetDims::new(100, 50);
        assert_eq!(resolve_edge(cell(5, 5), Down, dims, &[]), cell(99, 5));
        assert_eq!(resolve_edge(cell(5, 5), Up, dims, &[]), cell(0, 5));
        assert_eq!(resolve_edge(cell(5, 5), Right, dims, &[]), cell(5, 49));
        assert_eq!(resolve_edge(cell(5, 5), Left, dims, &[]), cell(5, 0));
    }

    #[test]
    fn resolve_edge_across_gap_and_off_the_end() {
        let dims = SheetDims::new(100, 50);
        // Column 0: data at rows 0 and 10 (gap 1..9). From (0,0) Down → cross the gap to (10,0).
        let col0 = [0, 10];
        assert_eq!(resolve_edge(cell(0, 0), Down, dims, &col0), cell(10, 0));
        // From (10,0) Down → nothing further → the sheet's last row.
        assert_eq!(resolve_edge(cell(10, 0), Down, dims, &col0), cell(99, 0));
    }

    #[test]
    fn resolve_edge_at_excel_max_bounds() {
        // At the very last cell, every forward edge motion stays put (Excel-max sheet, occupied cell).
        let dims = SheetDims::new(limits::MAX_ROWS, limits::MAX_COLS);
        let last = cell(limits::MAX_ROWS - 1, limits::MAX_COLS - 1);
        assert_eq!(
            resolve_edge(last, Down, dims, &[limits::MAX_ROWS - 1]),
            last
        );
        assert_eq!(
            resolve_edge(last, Right, dims, &[limits::MAX_COLS - 1]),
            last
        );
    }
}
