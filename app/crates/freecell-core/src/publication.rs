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

/// One cell's published value: the display string and its optional format colour. Empty
/// cells inside the viewport are simply omitted from [`Publication::cells`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCell {
    pub row: u32,
    pub col: u32,
    /// The engine's formatted display text (numbers/dates/currency/errors already
    /// rendered to a string).
    pub display_text: String,
    /// A number-format colour override (e.g. `[Red]`), if the format specifies one.
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
                text_color: None,
            }],
        };
        assert!(p.covers(10, 3));
        assert!(p.covers(19, 7));
        assert!(!p.covers(20, 4)); // row end is exclusive
        assert!(!p.covers(12, 8)); // col end is exclusive
        assert!(!p.covers(9, 4));
    }
}
