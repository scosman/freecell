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
    /// The covered **body** row range (already overscanned by the worker).
    pub rows: Range<u32>,
    /// The covered **body** column range (already overscanned by the worker).
    pub cols: Range<u32>,
    /// Frozen-pane leading-row band `M` (`freeze-panes`): the worker **always** publishes rows
    /// `0..M` alongside the body window, so a frozen band shows its values even when the body is
    /// scrolled deep past it. The covered region is therefore the union `(0..M ∪ rows)` on this
    /// axis; `0` when the sheet is unfrozen (the union reduces to `rows`).
    pub frozen_rows: u32,
    /// Frozen-pane leading-column band `K` (the column analog of [`frozen_rows`](Self::frozen_rows)).
    pub frozen_cols: u32,
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
            frozen_rows: 0,
            frozen_cols: 0,
            generation,
            cells: Vec::new(),
        }
    }

    /// Whether `(row, col)` falls inside the covered region — the union of the body window and the
    /// leading frozen bands (`(rows ∪ 0..M) × (cols ∪ 0..K)`, `freeze-panes`). Cells outside
    /// coverage render blank (the grid still draws their style from the resident cache). With
    /// `M=K=0` this reduces to plain body-window membership.
    pub fn covers(&self, row: u32, col: u32) -> bool {
        (self.rows.contains(&row) || row < self.frozen_rows)
            && (self.cols.contains(&col) || col < self.frozen_cols)
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
            frozen_rows: 0,
            frozen_cols: 0,
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
    fn covers_includes_frozen_bands() {
        // A body window scrolled deep past M=2 frozen rows / K=1 frozen col: the leading bands are
        // covered even though they sit outside the body range (`freeze-panes`), while a non-band
        // track scrolled out of the body is not covered.
        let p = Publication {
            sheet: SheetId(0),
            rows: 100..120,
            cols: 5..10,
            frozen_rows: 2,
            frozen_cols: 1,
            generation: 1,
            cells: vec![],
        };
        // Frozen band rows/cols are covered regardless of the deep body window.
        assert!(p.covers(0, 0), "corner band cell covered");
        assert!(p.covers(1, 7), "top band (frozen row, body col) covered");
        assert!(p.covers(110, 0), "left band (body row, frozen col) covered");
        assert!(p.covers(110, 7), "body cell covered");
        // A non-band track that scrolled out of the body window is NOT covered.
        assert!(
            !p.covers(2, 7),
            "row 2 is past the band and outside the body"
        );
        assert!(
            !p.covers(110, 1),
            "col 1 is past the band and outside the body"
        );
        // With no freeze the union reduces to plain body-window membership.
        let plain = Publication {
            frozen_rows: 0,
            frozen_cols: 0,
            ..p.clone()
        };
        assert!(!plain.covers(0, 0));
        assert!(plain.covers(110, 7));
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
