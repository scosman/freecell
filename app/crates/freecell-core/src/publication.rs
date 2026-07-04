//! `Publication` / `PublishedCell` — the engine-free value snapshot read model.
//!
//! The worker builds a `Publication` for the active sheet's overscanned viewport after
//! each evaluation, swaps it into an `Arc`, then bumps the generation counter
//! (publish-then-bump, `architecture.md §2`). The grid reads it per frame — one atomic
//! load, no engine call. Display text and its optional colour come pre-formatted from the
//! engine's formatted-value API; FreeCell adds no number-format logic (round-3 B,
//! `functional_spec.md §3.6`).

use std::ops::Range;

use crate::color::Rgb;
use crate::refs::SheetId;
use crate::style::Align;

/// The evaluated type of a published cell (`architecture.md §1.2`). Drives the grid's
/// type-aware default alignment (`§1.3`) and could carry other type-specific presentation
/// later. `Date` is a `Number` cell whose number format is date/time-like (the engine has
/// no distinct date type — dates are serial numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum CellKind {
    Number,
    Date,
    #[default]
    Text,
    Bool,
    Error,
}

impl CellKind {
    /// The type-aware default horizontal alignment for a cell with **no** explicit
    /// alignment (`architecture.md §1.3`, GAPS #1): numbers and dates align right,
    /// booleans and errors center, text left. An explicit [`Align`] on the cell's style
    /// always wins over this default.
    pub fn default_align(self) -> Align {
        match self {
            CellKind::Number | CellKind::Date => Align::Right,
            CellKind::Bool | CellKind::Error => Align::Center,
            CellKind::Text => Align::Left,
        }
    }
}

/// One cell's published value: the display string, its evaluated kind, and its optional
/// resolved text colour. Empty cells inside the viewport are simply omitted from
/// [`Publication::cells`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCell {
    pub row: u32,
    pub col: u32,
    /// The engine's formatted display text (numbers/dates/currency/errors already
    /// rendered to a string).
    pub display_text: String,
    /// The cell's evaluated type — drives type-aware default alignment (`§1.3`).
    pub kind: CellKind,
    /// The fully-resolved text colour: the cell's explicit (non-black) font colour if set,
    /// else the number format's produced colour (e.g. `[Red]` negatives), else `None`
    /// (the grid's near-black default).
    pub text_color: Option<Rgb>,
}

/// A snapshot of the active sheet's overscanned viewport at a given generation. The grid
/// renders values from here; anything outside `rows`×`cols` renders style-only with blank
/// text (beyond-overscan during an eval), never stale wrong values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub sheet: SheetId,
    /// The covered row range (already overscanned by the worker).
    pub rows: Range<u32>,
    /// The covered column range (already overscanned by the worker).
    pub cols: Range<u32>,
    /// The generation this snapshot belongs to — the UI repaints when it changes.
    pub generation: u64,
    /// The non-empty cells in the covered region.
    pub cells: Vec<PublishedCell>,
}

impl Publication {
    /// An empty snapshot for `sheet` at `generation` (the initial pre-load state and the
    /// value the grid falls back to before the first publish).
    pub fn empty(sheet: SheetId, generation: u64) -> Self {
        Self {
            sheet,
            rows: 0..0,
            cols: 0..0,
            generation,
            cells: Vec::new(),
        }
    }

    /// Whether `(row, col)` falls inside the covered region. Cells outside coverage render
    /// blank (the grid still draws their style from the resident cache).
    pub fn covers(&self, row: u32, col: u32) -> bool {
        self.rows.contains(&row) && self.cols.contains(&col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_publication_has_no_cells() {
        let p = Publication::empty(SheetId(0), 7);
        assert!(p.cells.is_empty());
        assert_eq!(p.generation, 7);
        assert!(!p.covers(0, 0), "an empty coverage covers nothing");
    }

    #[test]
    fn covers_reports_membership() {
        let p = Publication {
            sheet: SheetId(1),
            rows: 10..20,
            cols: 3..8,
            generation: 1,
            cells: vec![PublishedCell {
                row: 12,
                col: 4,
                display_text: "42".into(),
                kind: CellKind::Number,
                text_color: None,
            }],
        };
        assert!(p.covers(10, 3));
        assert!(p.covers(19, 7));
        assert!(!p.covers(20, 4)); // row end is exclusive
        assert!(!p.covers(12, 8)); // col end is exclusive
        assert!(!p.covers(9, 4));
    }

    #[test]
    fn cell_kind_default_align() {
        assert_eq!(CellKind::Number.default_align(), Align::Right);
        assert_eq!(CellKind::Date.default_align(), Align::Right);
        assert_eq!(CellKind::Bool.default_align(), Align::Center);
        assert_eq!(CellKind::Error.default_align(), Align::Center);
        assert_eq!(CellKind::Text.default_align(), Align::Left);
        // Text is the default kind (used for empty/unknown cells).
        assert_eq!(CellKind::default(), CellKind::Text);
    }
}
