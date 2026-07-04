//! `SelectionModel` + keyboard-motion rules (`functional_spec.md §3.2`,
//! `components/grid.md §Public interface`).
//!
//! The grid binds every navigation key to [`apply_motion`], a pure function so it is
//! unit-tested headless on Linux (the grid just wires keys → motions and repaints). The
//! model is an `(anchor, active)` pair: `active` is the outlined cell shown in the ref
//! box; `anchor` is the fixed corner a range extends from. Collapsing motions set both to
//! the new cell; extending motions move only `active`.

use crate::refs::{CellRange, CellRef};

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
/// - [`Motion::JumpEdge`] — Cmd/Ctrl+arrow: jump `active` to the sheet edge, collapse.
/// - [`Motion::ExtendEdge`] — Cmd/Ctrl+Shift+arrow: jump to the edge, keep the anchor.
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

/// Jumps a cell to the sheet edge in `direction` (MVP: edge of sheet, not edge-of-data —
/// `functional_spec.md §3.2`).
fn edge(cell: CellRef, direction: Direction, dims: SheetDims) -> CellRef {
    match direction {
        Direction::Up => CellRef::new(0, cell.col),
        Direction::Down => CellRef::new(dims.rows.saturating_sub(1), cell.col),
        Direction::Left => CellRef::new(cell.row, 0),
        Direction::Right => CellRef::new(cell.row, dims.cols.saturating_sub(1)),
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
}
