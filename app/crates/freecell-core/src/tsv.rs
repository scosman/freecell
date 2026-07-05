//! Tab-separated-value geometry helpers for the range clipboard (`components/clipboard.md`,
//! `functional_spec.md §2.2`). Pure + gpui-/IronCalc-free, so the dims parsing and the
//! overflow predicate are unit-tested headless.
//!
//! Only the *dimensions* of a pasted TSV are computed here (for the overflow pre-check and the
//! target `Area`); the actual token parsing + write is the engine's `paste_csv_string`
//! (tab-delimited, values-as-user-input). `tsv_dims` must be a **true upper bound** on the
//! rectangle the engine could write, so the overflow guard (`paste_fits`) always rejects a
//! spill-over *before* any mutation — a partial write would land outside the single undo entry
//! (`functional_spec.md §2.2`: "no partial paste").

use csv::ReaderBuilder;

use crate::limits;
use crate::refs::CellRef;

/// The `(width, height)` of a TSV block, parsed by the **same `csv` reader the engine's
/// `paste_csv_string` uses** (delimiter `\t`, no header row, default `Terminator::CRLF` — `\r`,
/// `\n`, `\r\n` each terminate a record — and default `"` quoting with `""` escapes). Computing
/// dims through the same parser rather than a hand-rolled scan means the bound can never diverge
/// from how the engine actually splits records/fields: quoted fields containing tabs or newlines
/// are ONE field on ONE record, `""` escapes are handled, mixed terminators are honoured. `width`
/// is the max field count over all records, `height` the record count; empty / terminator-only /
/// all-blank text → `(0, 0)` (the csv reader yields no record for a blank line).
///
/// This is a **provable upper bound** on the rectangle the engine writes, in both dimensions:
/// - **Rows:** read with `flexible(true)` so a ragged (mismatched-width) record parses and is
///   counted; the engine (`flexible = false`) instead *drops* such a record via `continue`
///   without advancing the row, so its row count is always `<= height`.
/// - **Columns:** the engine writes only records matching the *first* record's width, so its max
///   column comes from that first width — always `<= max(all record widths) = width`.
///
/// A malformed record (unreachable for valid UTF-8 with `flexible(true)`, handled defensively) is
/// counted toward `height` only — the engine would drop it too, so it contributes no columns.
pub fn tsv_dims(text: &str) -> (u32, u32) {
    let mut reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut height: u32 = 0;
    let mut width: u32 = 0;
    for record in reader.records() {
        height += 1;
        if let Ok(record) = record {
            width = width.max(record.len() as u32);
        }
    }
    (width, height)
}

/// Whether a `width × height` block whose top-left sits at `anchor` (0-based) fits entirely
/// inside the Excel-max sheet — the paste-overflow pre-check (`functional_spec.md §2.2`: a
/// paste that would spill past the sheet edge is rejected, no partial write). A zero-sized
/// block never "fits" (there is nothing to paste).
pub fn paste_fits(anchor: CellRef, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    // Occupied rows are `[anchor.row, anchor.row + height - 1]`; fits iff the last index is in
    // bounds. Do the arithmetic in `u64` so an anchor near the edge can't overflow `u32`.
    let last_row = anchor.row as u64 + height as u64 - 1;
    let last_col = anchor.col as u64 + width as u64 - 1;
    last_row < limits::MAX_ROWS as u64 && last_col < limits::MAX_COLS as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_simple() {
        // A 2×2 grid with a trailing newline is 2 rows × 2 cols.
        assert_eq!(tsv_dims("1\t2\n=A1\ttrue\n"), (2, 2));
        // No trailing newline — same dims.
        assert_eq!(tsv_dims("1\t2\n=A1\ttrue"), (2, 2));
    }

    #[test]
    fn dims_trailing_newline() {
        // The trailing terminator's empty segment is skipped (like the csv reader).
        assert_eq!(tsv_dims("a\n"), (1, 1));
        // A blank line writes no record, so it is not counted (engine skips it).
        assert_eq!(tsv_dims("a\n\n"), (1, 1));
        // CRLF is a single terminator.
        assert_eq!(tsv_dims("a\tb\r\n"), (2, 1));
    }

    #[test]
    fn dims_bare_cr_counts_records() {
        // Regression: `\r` alone is a record terminator (csv `Terminator::CRLF`). The engine
        // writes 3 rows for this, so `tsv_dims` must too — else the overflow guard is bypassed.
        assert_eq!(tsv_dims("a\rb\rc"), (1, 3));
        // Mixed CR / LF / CRLF terminators, each ending one record.
        assert_eq!(tsv_dims("a\rb\nc\r\nd"), (1, 4));
        // A blank line between CRs is skipped (empty record), like the engine.
        assert_eq!(tsv_dims("a\r\rZ"), (1, 2));
    }

    #[test]
    fn dims_ragged() {
        // The width is the widest row; a short row does not shrink it (the engine drops the
        // ragged row, so this stays a conservative upper bound).
        assert_eq!(tsv_dims("1\t2\t3\n9\n"), (3, 2));
        // A trailing `\r` on an interior CRLF row does not add a phantom column.
        assert_eq!(tsv_dims("1\t2\r\n3\t4\r\n"), (2, 2));
    }

    #[test]
    fn dims_quote_aware_width() {
        // Regression (CR Moderate): a quoted field with an embedded newline is ONE field on ONE
        // record — the engine parses `a\t"x\ny"\tb` as a single 3-field record. A physical-line
        // scan would wrongly report width 2 (splitting inside the quote), UNDER-counting the
        // overflow bound. Must be a 3-wide, single record.
        assert_eq!(tsv_dims("a\t\"x\ny\"\tb"), (3, 1));
        // A quoted field with an embedded tab stays one field (not split on the inner tab).
        assert_eq!(tsv_dims("\"a\tb\"\tc"), (2, 1));
        // `""` is an escaped quote inside a quoted field — still one field.
        assert_eq!(tsv_dims("\"a\"\"b\"\td"), (2, 1));
    }

    #[test]
    fn dims_single_token() {
        assert_eq!(tsv_dims("hello"), (1, 1));
        assert_eq!(tsv_dims(""), (0, 0));
        assert_eq!(tsv_dims("\n"), (0, 0)); // only a terminator → nothing
        assert_eq!(tsv_dims("\r"), (0, 0)); // bare CR terminator → nothing
        assert_eq!(tsv_dims("\r\n"), (0, 0)); // a single CRLF terminator → nothing
                                              // A record that is only tabs (empty fields) is still a record.
        assert_eq!(tsv_dims("\t"), (2, 1));
    }

    #[test]
    fn overflow_predicate() {
        let a1 = CellRef::new(0, 0);
        assert!(paste_fits(a1, 1, 1));
        assert!(paste_fits(a1, limits::MAX_COLS, limits::MAX_ROWS)); // whole sheet from A1 fits
        assert!(!paste_fits(a1, 0, 5)); // nothing to paste
        assert!(!paste_fits(a1, 5, 0));

        // One row short of the bottom edge: a 2-row block spills, a 1-row block fits.
        let near_bottom = CellRef::new(limits::MAX_ROWS - 1, 0);
        assert!(paste_fits(near_bottom, 1, 1));
        assert!(!paste_fits(near_bottom, 1, 2));

        // One column short of the right edge.
        let near_right = CellRef::new(0, limits::MAX_COLS - 1);
        assert!(paste_fits(near_right, 1, 1));
        assert!(!paste_fits(near_right, 2, 1));

        // Moderate #1 regression: a CR-only 3-row block anchored on the last row is rejected.
        let (w, h) = tsv_dims("a\rb\rc");
        assert!(
            !paste_fits(near_bottom, w, h),
            "CR-only overflow must be caught"
        );

        // Moderate (width) regression: the quoted-newline payload is a 3-wide record; pasting it
        // with its top-left two columns from the right edge (col 16382, 0-based) must be rejected
        // — a physical-line width of 2 would (16382+2-1 = 16383 < 16384) wrongly pass the guard.
        let (qw, qh) = tsv_dims("a\t\"x\ny\"\tb");
        assert_eq!((qw, qh), (3, 1));
        let two_from_right = CellRef::new(0, limits::MAX_COLS - 2);
        assert!(
            !paste_fits(two_from_right, qw, qh),
            "a quoted-field 3-wide record near the right edge must overflow-reject"
        );
    }
}
