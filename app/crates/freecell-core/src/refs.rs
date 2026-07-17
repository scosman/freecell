//! Cell addressing: zero-based `(row, col)` refs, rectangular ranges, and Excel "A1"
//! notation conversion (both directions).
//!
//! `column_label` / `column_from_label` are the bijective base-26 conversion ported from
//! the frozen `datagen::column_label` (`experiments/shared/datagen/src/cell.rs`), copied
//! not referenced (`architecture.md §1`). Everything here is zero-based internally; A1 is
//! one-based with letter columns, produced only at the UI edge (the ref box).

use crate::limits;

/// A stable, positional identifier for a worksheet. The worker assigns these and keeps an
/// index↔id map so renames don't invalidate per-sheet UI state (`architecture.md §3`,
/// `components/style_cache.md`). Grid scroll/selection maps and the caches key on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SheetId(pub u32);

/// The dominant axis of a drag-fill (`gaps_closing_7_15 §3`, D3.1: one axis per drag).
/// `Vertical` fills up/down over rows (`auto_fill_rows`); `Horizontal` fills left/right over
/// columns (`auto_fill_columns`). Shared by the grid's `GridEvent`, the window→worker mapping,
/// and the engine `Command`/`document.fill_drag` (the engine crate can't see app types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillAxis {
    Vertical,
    Horizontal,
}

/// A zero-based `(row, col)` cell coordinate (`(0, 0)` is `A1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    /// Creates a zero-based cell ref.
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// Renders this ref in Excel "A1" notation: `(0, 0) -> "A1"`, `(0, 26) -> "AA1"`,
    /// `(0, 16383) -> "XFD1"`.
    pub fn to_a1(self) -> String {
        let mut label = column_label(self.col);
        // Excel rows are one-based.
        label.push_str(&(self.row + 1).to_string());
        label
    }

    /// Parses an "A1"-notation ref (`"B7"`, `"$C$3"`, case-insensitive). Absolute-marker
    /// `$` signs are accepted and ignored (MVP has no relative/absolute distinction in the
    /// ref box). Returns `None` for anything malformed or out of the Excel-max range.
    pub fn from_a1(text: &str) -> Option<Self> {
        let s = text.trim();
        let mut chars = s.chars().peekable();

        // Leading optional `$` then one-or-more ASCII letters → column.
        if chars.peek() == Some(&'$') {
            chars.next();
        }
        let mut letters = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() {
                letters.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if letters.is_empty() {
            return None;
        }

        // Optional `$` then one-or-more ASCII digits → row.
        if chars.peek() == Some(&'$') {
            chars.next();
        }
        let mut digits = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(c);
                chars.next();
            } else {
                break;
            }
        }
        // Anything left over (spaces already trimmed) means it wasn't a clean ref.
        if chars.next().is_some() || digits.is_empty() {
            return None;
        }

        let col = column_from_label(&letters)?;
        // Row is one-based in A1; reject 0 and out-of-range.
        let row_one = digits.parse::<u64>().ok()?;
        if row_one == 0 {
            return None;
        }
        let row = u32::try_from(row_one - 1).ok()?;
        if row >= limits::MAX_ROWS || col >= limits::MAX_COLS {
            return None;
        }
        Some(Self { row, col })
    }
}

/// Converts a zero-based column index to its Excel letter label (`0 -> "A"`, `25 -> "Z"`,
/// `26 -> "AA"`, `16383 -> "XFD"`).
///
/// Bijective base-26 ("bijective hexavigesimal"): no zero digit, so `26` maps to `AA`.
pub fn column_label(col: u32) -> String {
    let mut n = col as u64 + 1; // shift to one-based for bijective base-26
    let mut bytes = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        bytes.push(b'A' + rem);
        n = (n - 1) / 26;
    }
    bytes.reverse();
    // Bytes are all ASCII 'A'..='Z', so this is always valid UTF-8.
    String::from_utf8(bytes).expect("column label is ASCII")
}

/// Parses an Excel column label (`"A" -> 0`, `"XFD" -> 16383`, case-insensitive) back to a
/// zero-based index. Returns `None` on empty input, non-letters, or overflow past `u32`.
pub fn column_from_label(label: &str) -> Option<u32> {
    if label.is_empty() {
        return None;
    }
    let mut n: u64 = 0;
    for c in label.chars() {
        let d = match c {
            'A'..='Z' => (c as u8 - b'A') as u64 + 1,
            'a'..='z' => (c as u8 - b'a') as u64 + 1,
            _ => return None,
        };
        n = n.checked_mul(26)?.checked_add(d)?;
        if n > u32::MAX as u64 + 1 {
            return None;
        }
    }
    // n is one-based; shift back to zero-based.
    u32::try_from(n - 1).ok()
}

/// Parses ONE OOXML `sqref` endpoint that is a *pure column label* (`"A"`, `"$C"`) into its
/// zero-based column index. An optional leading `$` is stripped; the rest must be letters
/// only. Anything with digits is a cell ref (not a bare column), so it returns `None` here
/// and is handled by the rectangle branch instead. Out-of-range labels are rejected like
/// [`CellRef::from_a1`].
fn column_only_label(endpoint: &str) -> Option<u32> {
    let label = endpoint.strip_prefix('$').unwrap_or(endpoint);
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let col = column_from_label(label)?;
    if col >= limits::MAX_COLS {
        return None;
    }
    Some(col)
}

/// Parses ONE OOXML `sqref` endpoint that is a *pure 1-based row number* (`"1"`, `"$7"`) into
/// its zero-based row index. An optional leading `$` is stripped; the rest must be digits
/// only. Rejects row `0` (A1 rows are one-based) and anything out of the Excel-max range,
/// matching [`CellRef::from_a1`].
fn row_only_number(endpoint: &str) -> Option<u32> {
    let digits = endpoint.strip_prefix('$').unwrap_or(endpoint);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let row_one = digits.parse::<u64>().ok()?;
    if row_one == 0 {
        return None;
    }
    let row = u32::try_from(row_one - 1).ok()?;
    if row >= limits::MAX_ROWS {
        return None;
    }
    Some(row)
}

/// A normalized rectangular range of cells. `start` is always the top-left corner and
/// `end` the bottom-right (both inclusive), regardless of which corner the user anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    /// Builds a range from two arbitrary corners, normalizing to top-left / bottom-right.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Self {
            start: CellRef::new(a.row.min(b.row), a.col.min(b.col)),
            end: CellRef::new(a.row.max(b.row), a.col.max(b.col)),
        }
    }

    /// A single-cell range.
    pub fn single(cell: CellRef) -> Self {
        Self {
            start: cell,
            end: cell,
        }
    }

    /// Whether the range covers exactly one cell.
    pub fn is_single(&self) -> bool {
        self.start == self.end
    }

    /// The range's width in columns (inclusive; `>= 1` for a normalized range).
    pub fn width(&self) -> u32 {
        self.end.col - self.start.col + 1
    }

    /// The range's height in rows (inclusive; `>= 1` for a normalized range).
    pub fn height(&self) -> u32 {
        self.end.row - self.start.row + 1
    }

    /// Inclusive row span.
    pub fn rows(&self) -> std::ops::RangeInclusive<u32> {
        self.start.row..=self.end.row
    }

    /// Inclusive column span.
    pub fn cols(&self) -> std::ops::RangeInclusive<u32> {
        self.start.col..=self.end.col
    }

    /// Whether `cell` lies inside the range (inclusive).
    pub fn contains(&self, cell: CellRef) -> bool {
        cell.row >= self.start.row
            && cell.row <= self.end.row
            && cell.col >= self.start.col
            && cell.col <= self.end.col
    }

    /// Whether this range overlaps `other` — i.e. they share at least one cell, which holds iff
    /// their row spans overlap **and** their column spans overlap (inclusive rectangle
    /// intersection). Reflexive and symmetric. Ranges that share a boundary row/column (e.g.
    /// `A1:C6` and `C6:E9`) overlap on that shared line; ranges that are merely *adjacent* with no
    /// shared cell (e.g. cols `2..=6` and `7..=9`) do **not**. Both ranges are normalized
    /// (`start` ≤ `end` on each axis), so this is the standard 1-D interval-overlap test per axis.
    /// Used to selection-scope the conditional-formatting rule list (only rules whose target range
    /// intersects the selection are shown).
    pub fn intersects(&self, other: &CellRange) -> bool {
        self.start.row <= other.end.row
            && other.start.row <= self.end.row
            && self.start.col <= other.end.col
            && other.start.col <= self.end.col
    }

    /// A1 notation: `"B7"` for a single cell, `"B2:D9"` for a rectangle.
    pub fn to_a1(&self) -> String {
        if self.is_single() {
            self.start.to_a1()
        } else {
            format!("{}:{}", self.start.to_a1(), self.end.to_a1())
        }
    }

    /// Parses an A1-notation rectangle: `"B2:D9"` (two corners), or a single `"A1"`. Corners
    /// are normalized to top-left / bottom-right. Case-insensitive; absolute `$` markers are
    /// accepted and ignored (via [`CellRef::from_a1`]). Returns `None` for anything malformed or
    /// out of the Excel-max range. The shared parser for the file's merge ranges (the cache build
    /// + the worker's merge guard both read `worksheet.merge_cells`, which are A1 strings).
    pub fn from_a1(text: &str) -> Option<Self> {
        let s = text.trim();
        match s.split_once(':') {
            Some((a, b)) => Some(Self::new(CellRef::from_a1(a)?, CellRef::from_a1(b)?)),
            None => Some(Self::single(CellRef::from_a1(s)?)),
        }
    }

    /// Parses ONE OOXML `sqref` sub-area into a normalized range, handling every shape Excel
    /// writes verbatim into a conditional-formatting target's `sqref`:
    /// - **single cell** (`"B2"`) → that one cell,
    /// - **cell rectangle** (`"B2:D9"`) → the rectangle (delegates to [`CellRange::from_a1`]),
    /// - **whole column(s)** (`"A:A"`, `"A:C"`) → those columns spanning ALL rows,
    /// - **whole row(s)** (`"1:1"`, `"3:7"`) → those rows spanning ALL columns.
    ///
    /// Absolute `$` markers on either endpoint are accepted and ignored. Endpoint order does
    /// not matter (`CellRange::new` normalizes). Returns `None` for anything else — a mixed
    /// `col:cell` form, garbage, or empty input. This is the counterpart to `from_a1` that
    /// additionally understands the column-only / row-only forms `from_a1` rejects (it requires
    /// both letters and digits), so whole-column/whole-row CF rules parse instead of vanishing.
    pub fn from_sqref_area(area: &str) -> Option<CellRange> {
        let s = area.trim();
        let (a, b) = match s.split_once(':') {
            // No colon: a lone cell reference.
            None => return CellRef::from_a1(s).map(CellRange::single),
            Some(parts) => parts,
        };
        // Cell rectangle: both endpoints are complete cell refs.
        if let (Some(start), Some(end)) = (CellRef::from_a1(a), CellRef::from_a1(b)) {
            return Some(CellRange::new(start, end));
        }
        // Whole column(s): both endpoints are pure column labels → span all rows.
        if let (Some(col_a), Some(col_b)) = (column_only_label(a), column_only_label(b)) {
            return Some(CellRange::new(
                CellRef::new(0, col_a),
                CellRef::new(limits::MAX_ROWS - 1, col_b),
            ));
        }
        // Whole row(s): both endpoints are pure row numbers → span all columns.
        if let (Some(row_a), Some(row_b)) = (row_only_number(a), row_only_number(b)) {
            return Some(CellRange::new(
                CellRef::new(row_a, 0),
                CellRef::new(row_b, limits::MAX_COLS - 1),
            ));
        }
        // Mixed (col:cell), garbage, or otherwise unrecognized.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_label_known_values() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(27), "AB");
        assert_eq!(column_label(701), "ZZ");
        assert_eq!(column_label(702), "AAA");
        assert_eq!(column_label(16383), "XFD"); // Excel's last column.
    }

    #[test]
    fn column_label_roundtrip() {
        for col in [0u32, 1, 25, 26, 27, 51, 52, 700, 701, 702, 16383] {
            let label = column_label(col);
            assert_eq!(
                column_from_label(&label),
                Some(col),
                "roundtrip failed for {col}"
            );
        }
        // Case-insensitive.
        assert_eq!(column_from_label("xfd"), Some(16383));
        // Rejects junk.
        assert_eq!(column_from_label(""), None);
        assert_eq!(column_from_label("A1"), None);
        assert_eq!(column_from_label("!"), None);
    }

    #[test]
    fn a1_roundtrip() {
        for (row, col) in [(0u32, 0u32), (6, 1), (0, 26), (1_048_575, 16_383), (41, 3)] {
            let cell = CellRef::new(row, col);
            let a1 = cell.to_a1();
            assert_eq!(
                CellRef::from_a1(&a1),
                Some(cell),
                "roundtrip failed for {a1}"
            );
        }
        assert_eq!(CellRef::new(6, 1).to_a1(), "B7");
        assert_eq!(CellRef::from_a1("B7"), Some(CellRef::new(6, 1)));
        // Absolute markers accepted and ignored; whitespace trimmed; case-insensitive.
        assert_eq!(CellRef::from_a1("$C$3"), Some(CellRef::new(2, 2)));
        assert_eq!(CellRef::from_a1("  aa1 "), Some(CellRef::new(0, 26)));
    }

    #[test]
    fn from_a1_rejects_junk() {
        for bad in [
            "", "1", "A", "A0", "B7C", "1A", "A1.5", "$$A1", "AAAA1", "B7 D9",
        ] {
            assert_eq!(CellRef::from_a1(bad), None, "should reject {bad:?}");
        }
        // Out of the Excel-max range.
        assert_eq!(CellRef::from_a1("A1048577"), None); // one past the last row
        assert_eq!(CellRef::from_a1("XFE1"), None); // one past the last column
    }

    #[test]
    fn cell_range_normalizes_corners() {
        // Anchor at bottom-right, active at top-left — still normalizes.
        let r = CellRange::new(CellRef::new(9, 3), CellRef::new(2, 1));
        assert_eq!(r.start, CellRef::new(2, 1));
        assert_eq!(r.end, CellRef::new(9, 3));
        assert!(!r.is_single());
        assert!(CellRange::single(CellRef::new(4, 4)).is_single());
    }

    #[test]
    fn cell_range_width_and_height() {
        let r = CellRange::new(CellRef::new(2, 1), CellRef::new(9, 3)); // rows 2..=9, cols 1..=3
        assert_eq!(r.height(), 8);
        assert_eq!(r.width(), 3);
        let one = CellRange::single(CellRef::new(4, 4));
        assert_eq!((one.width(), one.height()), (1, 1));
    }

    #[test]
    fn cell_range_contains() {
        let r = CellRange::new(CellRef::new(2, 1), CellRef::new(9, 3));
        assert!(r.contains(CellRef::new(2, 1)));
        assert!(r.contains(CellRef::new(9, 3)));
        assert!(r.contains(CellRef::new(5, 2)));
        assert!(!r.contains(CellRef::new(1, 2)));
        assert!(!r.contains(CellRef::new(5, 4)));
    }

    #[test]
    fn cell_range_intersects() {
        let base = CellRange::new(CellRef::new(2, 2), CellRef::new(6, 6)); // rows 2..=6, cols 2..=6

        // Overlapping rectangles (share an interior region).
        let overlap = CellRange::new(CellRef::new(4, 4), CellRef::new(9, 9));
        assert!(base.intersects(&overlap));
        assert!(overlap.intersects(&base), "intersection is symmetric");

        // Edge-touching on the shared boundary column (col 6 belongs to both) → overlap.
        let share_col = CellRange::new(CellRef::new(2, 6), CellRef::new(6, 10));
        assert!(base.intersects(&share_col));
        assert!(share_col.intersects(&base));
        // Edge-touching on the shared boundary row (row 6 belongs to both) → overlap.
        let share_row = CellRange::new(CellRef::new(6, 2), CellRef::new(10, 6));
        assert!(base.intersects(&share_row));

        // Merely adjacent columns (2..=6 vs 7..=9) share no cell → NOT an intersection.
        let adjacent_col = CellRange::new(CellRef::new(2, 7), CellRef::new(6, 9));
        assert!(!base.intersects(&adjacent_col));

        // Disjoint by row: columns overlap but the row spans do not (rows 2..=6 vs 8..=10).
        let disjoint_row = CellRange::new(CellRef::new(8, 2), CellRef::new(10, 6));
        assert!(!base.intersects(&disjoint_row));
        assert!(!disjoint_row.intersects(&base));

        // Disjoint by col: rows overlap but the column spans do not (cols 2..=6 vs 8..=10).
        let disjoint_col = CellRange::new(CellRef::new(2, 8), CellRef::new(6, 10));
        assert!(!base.intersects(&disjoint_col));

        // Single cell inside → intersects; single cell outside → does not.
        let inside = CellRange::single(CellRef::new(4, 4));
        assert!(base.intersects(&inside));
        assert!(inside.intersects(&base));
        let outside = CellRange::single(CellRef::new(0, 0));
        assert!(!base.intersects(&outside));
        assert!(!outside.intersects(&base));

        // A range always intersects itself.
        assert!(base.intersects(&base));
    }

    #[test]
    fn range_to_a1_single_vs_rect() {
        assert_eq!(CellRange::single(CellRef::new(6, 1)).to_a1(), "B7");
        let r = CellRange::new(CellRef::new(1, 1), CellRef::new(8, 3));
        assert_eq!(r.to_a1(), "B2:D9");
    }

    #[test]
    fn range_from_a1_valid_and_hostile() {
        // A single cell parses to a single-cell range.
        assert_eq!(
            CellRange::from_a1("A1"),
            Some(CellRange::single(CellRef::new(0, 0)))
        );
        // A rectangle; K7:L10 → rows 6..=9, cols 10..=11 (the merge-guard fixture).
        assert_eq!(
            CellRange::from_a1("K7:L10"),
            Some(CellRange::new(CellRef::new(6, 10), CellRef::new(9, 11)))
        );
        // Reversed / mixed corners normalize.
        assert_eq!(
            CellRange::from_a1("D9:B2"),
            Some(CellRange::new(CellRef::new(1, 1), CellRef::new(8, 3)))
        );
        // Full-sheet range round-trips within Excel-max.
        assert_eq!(
            CellRange::from_a1("A1:XFD1048576"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(limits::MAX_ROWS - 1, limits::MAX_COLS - 1)
            ))
        );
        // Hostile / malformed inputs never panic → None.
        for bad in [
            "",
            "A",
            "1:2",
            "A1:",
            ":B2",
            "A1:B2:C3",
            "ZZZ9:A1",
            "A1048577:B2",
        ] {
            assert_eq!(CellRange::from_a1(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn from_sqref_area_all_shapes() {
        // Single cell → single-cell range.
        assert_eq!(
            CellRange::from_sqref_area("B2"),
            Some(CellRange::single(CellRef::new(1, 1)))
        );
        // Cell rectangle → delegates to from_a1: B2:D9 → rows 1..=8, cols 1..=3.
        assert_eq!(
            CellRange::from_sqref_area("B2:D9"),
            Some(CellRange::new(CellRef::new(1, 1), CellRef::new(8, 3)))
        );
        // Whole column: "A:A" → col A (0) over every row.
        assert_eq!(
            CellRange::from_sqref_area("A:A"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(limits::MAX_ROWS - 1, 0)
            ))
        );
        // Whole columns: "A:C" → cols A..=C (0..=2) over every row.
        assert_eq!(
            CellRange::from_sqref_area("A:C"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(limits::MAX_ROWS - 1, 2)
            ))
        );
        // Whole row: "1:1" → row 0 over every column.
        assert_eq!(
            CellRange::from_sqref_area("1:1"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(0, limits::MAX_COLS - 1)
            ))
        );
        // Whole rows: "3:7" → rows 2..=6 over every column.
        assert_eq!(
            CellRange::from_sqref_area("3:7"),
            Some(CellRange::new(
                CellRef::new(2, 0),
                CellRef::new(6, limits::MAX_COLS - 1)
            ))
        );
        // Absolute `$` markers accepted and ignored on either endpoint.
        assert_eq!(
            CellRange::from_sqref_area("$A:$A"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(limits::MAX_ROWS - 1, 0)
            ))
        );
        assert_eq!(
            CellRange::from_sqref_area("$1:$1"),
            Some(CellRange::new(
                CellRef::new(0, 0),
                CellRef::new(0, limits::MAX_COLS - 1)
            ))
        );
        // Endpoint order does not matter (normalized).
        assert_eq!(
            CellRange::from_sqref_area("C:A"),
            CellRange::from_sqref_area("A:C")
        );
    }

    #[test]
    fn from_sqref_area_whole_col_row_intersection() {
        // "A:A" intersects a selection anywhere in column A, but NOT one confined to column B.
        let col_a = CellRange::from_sqref_area("A:A").unwrap();
        assert!(col_a.intersects(&CellRange::single(CellRef::new(500, 0))));
        assert!(!col_a.intersects(&CellRange::new(CellRef::new(0, 1), CellRef::new(9, 1))));

        // "1:1" intersects a row-1 selection (row index 0) but not a row-5 (index 4) selection.
        let row_1 = CellRange::from_sqref_area("1:1").unwrap();
        assert!(row_1.intersects(&CellRange::single(CellRef::new(0, 42))));
        assert!(!row_1.intersects(&CellRange::new(CellRef::new(4, 0), CellRef::new(4, 9))));
    }

    #[test]
    fn from_sqref_area_rejects_junk() {
        // Mixed col:cell / cell:col, garbage, empty, incomplete → None (never panics).
        for bad in [
            "###", "", "A1:B", "A:B5", "1:B2", "A:", ":A", "0:0", "$$A:$$A",
        ] {
            assert_eq!(
                CellRange::from_sqref_area(bad),
                None,
                "should reject {bad:?}"
            );
        }
    }
}
