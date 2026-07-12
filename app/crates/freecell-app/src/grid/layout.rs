//! Pure grid geometry — the offset/hit-test/scroll/scrollbar math, with **no gpui and no
//! engine** so it is unit-tested headless (`components/grid.md §Test plan`: `axis_*`,
//! `hit_test_*`, plus scroll/clamp/scrollbar cases). Everything answers from the two
//! `freecell_core::Axis` prefix sums (`components/grid.md §Internal design`); nothing here
//! allocates per sheet.
//!
//! All screen math uses this coordinate convention: **grid-local** pixels have their
//! origin at the grid's top-left. The fixed header strip occupies the top
//! [`COL_HEADER_H`] px and the left `row_header_w` px; the scrollable content area is the
//! remaining rectangle. A cell at index `c` starts at `col_axis.offset_of(c) - scroll_x`
//! within the content area (then `+ row_header_w` to reach grid-local px).

use std::ops::Range;

use freecell_core::refs::{CellRange, CellRef};
use freecell_core::{Align, Axis};

/// Column-header strip height (px) — `ui_design.md §3.3` (~24 px).
pub const COL_HEADER_H: f32 = 24.0;
/// Minimum row-header gutter width (px) — widens for 7-digit row labels (`ui_design.md §3.3`).
pub const ROW_HEADER_MIN_W: f32 = 48.0;
/// Extra visible tracks rendered on each side of the viewport. The *publication* overscan
/// (~3× viewport) is the worker's concern; render overscan is small (`components/grid.md`).
pub const RENDER_OVERSCAN: u32 = 2;
/// Per-digit width estimate (px) for sizing the row-header gutter at the 11.5 px header
/// font. An estimate, not a glyph measurement — the gutter only needs to comfortably fit
/// the label, and over-estimating by a px is harmless (see `row_header_width`).
pub const HEADER_DIGIT_W: f32 = 7.5;
/// Horizontal padding (px) added on each side of a row-header label.
pub const HEADER_LABEL_PAD: f32 = 6.0;

/// Scrollbar inset from the content edge (px) — macOS-style overlay (`ui_design.md §3.3`).
pub const SCROLLBAR_INSET: f32 = 3.0;
/// Scrollbar thumb thickness (px).
pub const SCROLLBAR_THICKNESS: f32 = 8.0;
/// Minimum scrollbar thumb length (px) so a thumb over a huge extent stays grabbable.
pub const SCROLLBAR_MIN_LEN: f32 = 24.0;

/// The scrollable content area (px), i.e. the viewport minus the fixed header strip. Never
/// negative (a viewport smaller than the headers yields a zero-sized content area).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentArea {
    pub row_header_w: f32,
    pub width: f64,
    pub height: f64,
}

/// A scrollbar thumb along one axis: `offset` px from the track start, `length` px long.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thumb {
    pub offset: f32,
    pub length: f32,
}

/// Which zone of the grid a grid-local pixel lands in (`components/grid.md §Input`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridHit {
    /// The top-left corner cap where the two header strips meet.
    Corner,
    /// The column-header strip over column `col`.
    ColHeader { col: u32 },
    /// The row-header gutter beside row `row`.
    RowHeader { row: u32 },
    /// A data cell.
    Cell { row: u32, col: u32 },
}

/// The number of decimal digits in `n` (`0 -> 1`). Used to size the row-header gutter.
fn digit_count(n: u32) -> u32 {
    let mut n = n;
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

/// The row-header gutter width (px): wide enough for the deepest visible row's 1-based
/// label plus padding, floored at [`ROW_HEADER_MIN_W`]. `last_visible_row` is zero-based.
pub fn row_header_width(last_visible_row: u32) -> f32 {
    let label = last_visible_row.saturating_add(1); // rows are 1-based in the header
    let w = digit_count(label) as f32 * HEADER_DIGIT_W + 2.0 * HEADER_LABEL_PAD;
    w.max(ROW_HEADER_MIN_W)
}

/// The maximum scroll offset (px) for an axis whose content is `total` px inside a
/// `content` px viewport. Zero when the content fits (`total <= content`).
pub fn max_scroll(total: f64, content: f64) -> f64 {
    (total - content).max(0.0)
}

/// Clamps a scroll offset to `[0, max_scroll]` on each axis (`components/grid.md §Input`).
pub fn clamp_scroll(
    scroll_x: f64,
    scroll_y: f64,
    total_w: f64,
    total_h: f64,
    content: ContentArea,
) -> (f64, f64) {
    let max_x = max_scroll(total_w, content.width);
    let max_y = max_scroll(total_h, content.height);
    (scroll_x.clamp(0.0, max_x), scroll_y.clamp(0.0, max_y))
}

/// The overlay-scrollbar thumb for one axis, or `None` when the content fits the viewport
/// (nothing to scroll → no bar). `track_len` is the usable track length (px).
pub fn scrollbar_thumb(total: f64, content: f64, scroll: f64, track_len: f32) -> Option<Thumb> {
    if total <= content || track_len <= 0.0 {
        return None;
    }
    let proportion = (content / total) as f32; // (0, 1)
    let length = (track_len * proportion).clamp(SCROLLBAR_MIN_LEN.min(track_len), track_len);
    let max = max_scroll(total, content);
    let fraction = if max <= 0.0 {
        0.0
    } else {
        (scroll / max).clamp(0.0, 1.0) as f32
    };
    let offset = (track_len - length) * fraction;
    Some(Thumb { offset, length })
}

/// Hit-tests a **grid-local** pixel (origin at the grid's top-left) to a [`GridHit`].
/// `scroll_x`/`scroll_y` are the current content scroll offsets (px).
pub fn hit_test(
    local_x: f32,
    local_y: f32,
    row_header_w: f32,
    scroll_x: f64,
    scroll_y: f64,
    row_axis: &Axis,
    col_axis: &Axis,
) -> GridHit {
    let in_header_row = local_y < COL_HEADER_H;
    let in_header_col = local_x < row_header_w;
    match (in_header_col, in_header_row) {
        (true, true) => GridHit::Corner,
        (false, true) => {
            let content_x = scroll_x + (local_x - row_header_w) as f64;
            GridHit::ColHeader {
                col: col_axis
                    .index_at(content_x)
                    .min(col_axis.count().saturating_sub(1)),
            }
        }
        (true, false) => {
            let content_y = scroll_y + (local_y - COL_HEADER_H) as f64;
            GridHit::RowHeader {
                row: row_axis
                    .index_at(content_y)
                    .min(row_axis.count().saturating_sub(1)),
            }
        }
        (false, false) => {
            let content_x = scroll_x + (local_x - row_header_w) as f64;
            let content_y = scroll_y + (local_y - COL_HEADER_H) as f64;
            GridHit::Cell {
                row: row_axis
                    .index_at(content_y)
                    .min(row_axis.count().saturating_sub(1)),
                col: col_axis
                    .index_at(content_x)
                    .min(col_axis.count().saturating_sub(1)),
            }
        }
    }
}

/// The minimal new `(scroll_x, scroll_y)` so cell `(row, col)` is fully inside the content
/// area, clamped to the valid scroll range (`components/grid.md`: `scroll_cell_into_view`).
/// Already-visible cells leave the scroll unchanged.
pub fn scroll_to_reveal(
    row: u32,
    col: u32,
    row_axis: &Axis,
    col_axis: &Axis,
    content: ContentArea,
    scroll_x: f64,
    scroll_y: f64,
) -> (f64, f64) {
    let x = reveal_axis(
        col,
        col_axis,
        content.width,
        scroll_x,
        max_scroll(col_axis.total(), content.width),
    );
    let y = reveal_axis(
        row,
        row_axis,
        content.height,
        scroll_y,
        max_scroll(row_axis.total(), content.height),
    );
    (x, y)
}

/// One axis of [`scroll_to_reveal`]: nudge `scroll` just enough that track `index` is
/// fully within `[scroll, scroll + content)`, then clamp to `[0, max]`.
fn reveal_axis(index: u32, axis: &Axis, content: f64, scroll: f64, max: f64) -> f64 {
    let start = axis.offset_of(index);
    let end = start + axis.size_of(index) as f64;
    let scroll = if start < scroll {
        start // track is above/left of the viewport → align its start
    } else if end > scroll + content {
        end - content // track is below/right → align its end to the viewport end
    } else {
        scroll // already fully visible
    };
    scroll.clamp(0.0, max)
}

/// Maps a **grid-local** pixel to the data cell under it, clamping the point into the content
/// rectangle first — so a drag into the header strips or past a viewport edge still resolves to
/// the nearest data cell (a drag-extend never lands on a header). Used while dragging a
/// selection (`components/grid.md §Input`). Unlike [`hit_test`], the result is always a
/// `CellRef` (headers are folded into the adjacent content cell).
#[allow(clippy::too_many_arguments)]
pub fn cell_at_point(
    local_x: f32,
    local_y: f32,
    row_header_w: f32,
    scroll_x: f64,
    scroll_y: f64,
    row_axis: &Axis,
    col_axis: &Axis,
    content_w: f64,
    content_h: f64,
) -> CellRef {
    let left = row_header_w as f64;
    let top = COL_HEADER_H as f64;
    // Clamp into the content rect, then convert to content-space (scroll + local offset).
    let clamped_x = (local_x as f64).clamp(left, left + content_w);
    let clamped_y = (local_y as f64).clamp(top, top + content_h);
    let content_x = scroll_x + (clamped_x - left);
    let content_y = scroll_y + (clamped_y - top);
    // Inclusive right/bottom clamp: when `content_w`/`content_h` lands exactly on a track
    // boundary, `index_at` can name the track just past the last fully-visible one. This is
    // intentional and benign for drag-extend — the `.min(count - 1)` below keeps it a valid
    // cell, and auto-scroll reveals a cell selected one past the edge.
    let col = col_axis
        .index_at(content_x)
        .min(col_axis.count().saturating_sub(1));
    let row = row_axis
        .index_at(content_y)
        .min(row_axis.count().saturating_sub(1));
    CellRef::new(row, col)
}

/// The per-axis scroll delta (px) for drag-past-edge **auto-scroll**: a fixed `step` toward the
/// origin (`-step`) when the pointer is within `hotzone` px of the left/top content edge, `+step`
/// toward the end when within `hotzone` of the right/bottom edge, and `0` in the interior
/// (`components/grid.md §Input`: "auto-scroll when dragging past edges … fixed 20 px/frame step").
/// The caller adds this to the current scroll and clamps it.
///
/// The `hotzone` inset is load-bearing, not cosmetic: gpui delivers `on_mouse_move` only while
/// the grid element is **hovered** (the pointer is inside its bounds), and the content's
/// right/bottom edges coincide with the window edge — so a pointer *strictly past* them never
/// generates the move event that would START the auto-scroll loop. Firing while the pointer is
/// still `hotzone` px INSIDE each edge lets the loop launch from a real move event; once running
/// it re-reads the (out-of-window, unclamped) pointer directly. This is also the Excel feel — the
/// scroll begins as the pointer nears an edge, not only once it leaves the window.
pub fn edge_autoscroll_delta(
    local_x: f32,
    local_y: f32,
    row_header_w: f32,
    content_w: f64,
    content_h: f64,
    step: f64,
    hotzone: f64,
) -> (f64, f64) {
    let left = row_header_w as f64;
    let right = left + content_w;
    let top = COL_HEADER_H as f64;
    let bottom = top + content_h;
    let (lx, ly) = (local_x as f64, local_y as f64);
    // Each test covers both "within hotzone inside the edge" and "past the edge" (the running
    // loop sees unclamped, out-of-window coordinates); the left/top branch is checked first so a
    // degenerate content area narrower than 2×hotzone resolves deterministically.
    let dx = if lx < left + hotzone {
        -step
    } else if lx > right - hotzone {
        step
    } else {
        0.0
    };
    let dy = if ly < top + hotzone {
        -step
    } else if ly > bottom - hotzone {
        step
    } else {
        0.0
    };
    (dx, dy)
}

/// Decomposes a selection range **minus its active cell** into ≤4 index sub-rectangles
/// (each `rows × cols`, both half-open), which together tile the range exactly once and
/// never cover the active cell — the Excel "white anchor" overlay (`ui_design.md §3.3`,
/// `components/grid.md §Render pass`). A single-cell selection yields an empty vec.
pub fn range_overlay_rects(range: CellRange, active: CellRef) -> Vec<(Range<u32>, Range<u32>)> {
    let (r0, r1) = (range.start.row, range.end.row); // inclusive
    let (c0, c1) = (range.start.col, range.end.col);
    let (ar, ac) = (active.row, active.col);
    let mut rects = Vec::new();

    // If the active cell is outside the range (shouldn't happen for a normal selection),
    // fall back to tinting the whole range so nothing is silently dropped.
    if ar < r0 || ar > r1 || ac < c0 || ac > c1 {
        if r0 <= r1 && c0 <= c1 {
            rects.push((r0..r1 + 1, c0..c1 + 1));
        }
        return rects;
    }

    // Bands above / below the active row span the full column width.
    if ar > r0 {
        rects.push((r0..ar, c0..c1 + 1));
    }
    if ar < r1 {
        rects.push((ar + 1..r1 + 1, c0..c1 + 1));
    }
    // Left / right of the active cell, within the active row only.
    if ac > c0 {
        rects.push((ar..ar + 1, c0..ac));
    }
    if ac < c1 {
        rects.push((ar..ar + 1, ac + 1..c1 + 1));
    }
    rects
}

// --- Text spill / overflow (`functional_spec.md §2`, `architecture.md §2`) ------------------
//
// Pure, gpui-free geometry for horizontal text spill: which direction a wrap-off text cell
// spills, how far it reaches across empty neighbours, and a cheap width gate that keeps
// comfortably-fitting text off the spill path entirely. The render integration
// (`grid/view.rs`) supplies the per-column occupancy probe (mirror / coverage / published
// content) and paints the spill element; everything decision-shaped lives here so it is unit-
// testable without a `Window`.

/// The direction(s) a wrap-off **text** cell spills over empty neighbours, from its effective
/// horizontal alignment (`functional_spec.md §2.2`, Excel-accurate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpillDirection {
    /// General / left-aligned text spills to the right (the common case).
    Right,
    /// Right-aligned text spills to the left.
    Left,
    /// Center-aligned text spills both ways, centred over the empty run on each side.
    Both,
}

/// The spill direction implied by a cell's **effective** horizontal alignment (the explicit
/// `h_align` else the cell type's default). Only text cells spill, and text defaults to
/// [`Align::Left`], so a plain/general text cell spills [`SpillDirection::Right`].
pub fn spill_direction(align: Align) -> SpillDirection {
    match align {
        Align::Left => SpillDirection::Right,
        Align::Right => SpillDirection::Left,
        Align::Center => SpillDirection::Both,
    }
}

/// Whether a candidate neighbour column can be spilled over. `Empty` = no content **and**
/// coverage is known (the spill may extend across it); `Blocked` = has content, is being
/// edited, or its coverage is unknown (the spill stops *before* it — never treat "beyond the
/// covered region" as reliably empty, `functional_spec.md §2.5`). Fill/border alone does NOT
/// make a cell `Blocked` (content-only), which the caller's probe reflects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
    Empty,
    Blocked,
}

/// The inclusive column span `[left, right]` a spilling cell's text is painted across; it always
/// contains the origin column. `left == right == origin` means there is no empty neighbour in the
/// spill direction, i.e. the cell does not spill (see [`SpillSpan::spills`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpillSpan {
    pub left: u32,
    pub right: u32,
}

impl SpillSpan {
    /// Whether the span actually extends beyond the origin column (there is something to spill
    /// over). A no-op span (`left == right == origin`) is not a spill.
    pub fn spills(&self, origin: u32) -> bool {
        self.left < origin || self.right > origin
    }
}

/// Scans outward from `origin` in `direction`, extending across [`Occupancy::Empty`] neighbour
/// columns and stopping at the first [`Occupancy::Blocked`] column or the inclusive scan bounds
/// `[min_col, max_col]` (the visible frame edge, which is always within publication coverage).
/// `occupancy(col)` classifies a neighbour column in the origin's row; it is invoked only for
/// candidate columns and MUST return `Blocked` for anything whose emptiness/coverage is unknown.
pub fn spill_span(
    origin: u32,
    direction: SpillDirection,
    min_col: u32,
    max_col: u32,
    mut occupancy: impl FnMut(u32) -> Occupancy,
) -> SpillSpan {
    let mut span = SpillSpan {
        left: origin,
        right: origin,
    };
    if matches!(direction, SpillDirection::Right | SpillDirection::Both) {
        let mut c = origin;
        while c < max_col && occupancy(c + 1) == Occupancy::Empty {
            c += 1;
            span.right = c;
        }
    }
    if matches!(direction, SpillDirection::Left | SpillDirection::Both) {
        let mut c = origin;
        while c > min_col && occupancy(c - 1) == Occupancy::Empty {
            c -= 1;
            span.left = c;
        }
    }
    span
}

/// Average glyph advance as a fraction of the font size, used only by [`estimated_text_width`].
/// A deliberate UNDER-estimate for a proportional UI font (Excel-ish ≈ 0.5em) so a cell whose
/// text comfortably fits its column is never treated as a spill candidate.
const SPILL_AVG_GLYPH_EM: f32 = 0.5;

/// A conservative estimate of `text`'s rendered width (px) at `font_px`, used **only** as the
/// spill width gate — never for layout (the actual clip bounds do the real work). Deliberately an
/// under-estimate (see [`SPILL_AVG_GLYPH_EM`]): a comfortably-fitting cell must not spill (that
/// keeps its non-spill render path — and thus its pixels — untouched), while genuinely long text
/// still exceeds any column. No allocation, O(chars).
pub fn estimated_text_width(text: &str, font_px: f32) -> f32 {
    text.chars().count() as f32 * font_px * SPILL_AVG_GLYPH_EM
}

/// Whether `text` at `font_px` is wide enough to overflow a `col_w`-px column (minus the cell's
/// horizontal padding `h_pad` on both sides) — i.e. the cell is a spill candidate
/// (`functional_spec.md §2.1`). Uses the conservative [`estimated_text_width`], so a snug-fitting
/// label reads as "fits" and takes the unchanged render path.
pub fn text_overflows_column(text: &str, font_px: f32, col_w: f32, h_pad: f32) -> bool {
    estimated_text_width(text, font_px) > (col_w - 2.0 * h_pad).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_core::refs::{CellRange, CellRef};
    use freecell_core::{limits, SelectionModel};

    /// A uniform axis of `count` tracks each `size` px — enough for the clamp/scrollbar/
    /// reveal cases that don't need variable geometry.
    fn uniform(count: u32, size: f32) -> Axis {
        Axis::new(count, move |_| size)
    }

    /// A varied axis (every 7th track wide) for hit-test-under-variable-geometry.
    fn varied(count: u32) -> Axis {
        Axis::new(count, |i| if i.is_multiple_of(7) { 120.0 } else { 24.0 })
    }

    fn content(w: f64, h: f64) -> ContentArea {
        ContentArea {
            row_header_w: ROW_HEADER_MIN_W,
            width: w,
            height: h,
        }
    }

    #[test]
    fn row_header_width_widens_for_deep_rows() {
        // Shallow rows sit at the 48 px minimum.
        assert_eq!(row_header_width(0), ROW_HEADER_MIN_W);
        assert_eq!(row_header_width(8), ROW_HEADER_MIN_W); // label "9"
                                                           // A 7-digit Excel-max label widens the gutter past the minimum, monotonically.
        let deep = row_header_width(limits::MAX_ROWS - 1); // label 1048576 (7 digits)
        assert!(
            deep > ROW_HEADER_MIN_W,
            "deep rows must widen the gutter: {deep}"
        );
        // A 6-digit label (row index 99_999 → "100000") is narrower than the 7-digit max.
        assert!(row_header_width(99_999) < deep, "more digits ⇒ wider");
    }

    #[test]
    fn max_scroll_never_negative() {
        assert_eq!(max_scroll(100.0, 400.0), 0.0); // content bigger than the sheet
        assert_eq!(max_scroll(1000.0, 400.0), 600.0);
    }

    #[test]
    fn clamp_scroll_bounds() {
        let axis_total_w = 1000.0;
        let axis_total_h = 2000.0;
        let c = content(400.0, 300.0);
        // Below zero clamps to zero; past the end clamps to total - content.
        let (x, y) = clamp_scroll(-50.0, -10.0, axis_total_w, axis_total_h, c);
        assert_eq!((x, y), (0.0, 0.0));
        let (x, y) = clamp_scroll(9999.0, 9999.0, axis_total_w, axis_total_h, c);
        assert_eq!((x, y), (600.0, 1700.0));
        // A sheet that fits the viewport pins scroll at 0.
        let small = content(4000.0, 4000.0);
        assert_eq!(
            clamp_scroll(123.0, 456.0, axis_total_w, axis_total_h, small),
            (0.0, 0.0)
        );
    }

    #[test]
    fn scrollbar_thumb_none_when_fits() {
        assert_eq!(scrollbar_thumb(300.0, 400.0, 0.0, 380.0), None);
        assert_eq!(scrollbar_thumb(400.0, 400.0, 0.0, 380.0), None);
    }

    #[test]
    fn scrollbar_thumb_proportional_and_positioned() {
        // Half the content is visible → thumb is half the track; at mid-scroll it is centred.
        let track = 400.0;
        let t = scrollbar_thumb(800.0, 400.0, 200.0, track).unwrap();
        assert!((t.length - 200.0).abs() < 1e-3, "length {}", t.length);
        // max scroll = 800-400 = 400; at scroll 200 (half) offset = (400-200)*0.5 = 100.
        assert!((t.offset - 100.0).abs() < 1e-3, "offset {}", t.offset);
    }

    #[test]
    fn scrollbar_thumb_at_extremes() {
        let track = 400.0;
        let top = scrollbar_thumb(4000.0, 400.0, 0.0, track).unwrap();
        assert_eq!(top.offset, 0.0);
        let bottom = scrollbar_thumb(4000.0, 400.0, 3600.0, track).unwrap();
        assert!((bottom.offset - (track - bottom.length)).abs() < 1e-3);
    }

    #[test]
    fn scrollbar_thumb_min_length() {
        // A 1M-row extent yields a tiny proportional thumb, floored at the minimum.
        let track = 400.0;
        let t = scrollbar_thumb(25_000_000.0, 400.0, 0.0, track).unwrap();
        assert_eq!(t.length, SCROLLBAR_MIN_LEN);
    }

    #[test]
    fn hit_test_zones() {
        let rows = uniform(100, 24.0);
        let cols = uniform(100, 100.0);
        let rhw = ROW_HEADER_MIN_W;
        // Corner.
        assert_eq!(
            hit_test(10.0, 10.0, rhw, 0.0, 0.0, &rows, &cols),
            GridHit::Corner
        );
        // Column header strip over the first column.
        assert_eq!(
            hit_test(rhw + 5.0, 10.0, rhw, 0.0, 0.0, &rows, &cols),
            GridHit::ColHeader { col: 0 }
        );
        // Row header gutter beside the first row.
        assert_eq!(
            hit_test(10.0, COL_HEADER_H + 5.0, rhw, 0.0, 0.0, &rows, &cols),
            GridHit::RowHeader { row: 0 }
        );
        // A1 cell: just inside the content origin.
        assert_eq!(
            hit_test(rhw + 1.0, COL_HEADER_H + 1.0, rhw, 0.0, 0.0, &rows, &cols),
            GridHit::Cell { row: 0, col: 0 }
        );
    }

    #[test]
    fn hit_test_scrolled_variable_geometry() {
        let rows = varied(1000);
        let cols = varied(1000);
        let rhw = ROW_HEADER_MIN_W;
        // Scroll to the start of row 50 / col 40, then click the top-left content pixel:
        // it must resolve to exactly (50, 40).
        let scroll_y = rows.offset_of(50);
        let scroll_x = cols.offset_of(40);
        assert_eq!(
            hit_test(
                rhw + 0.5,
                COL_HEADER_H + 0.5,
                rhw,
                scroll_x,
                scroll_y,
                &rows,
                &cols
            ),
            GridHit::Cell { row: 50, col: 40 }
        );
        // A pixel one row-height down lands on the next row.
        let y = COL_HEADER_H + rows.size_of(50) + 0.5;
        assert_eq!(
            hit_test(rhw + 0.5, y, rhw, scroll_x, scroll_y, &rows, &cols),
            GridHit::Cell { row: 51, col: 40 }
        );
    }

    #[test]
    fn scroll_to_reveal_directions_and_clamp() {
        let rows = uniform(1000, 24.0);
        let cols = uniform(1000, 100.0);
        let c = content(400.0, 300.0); // 4 cols, 12.5 rows visible
                                       // Already visible near the origin → unchanged.
        let (x, y) = scroll_to_reveal(2, 2, &rows, &cols, c, 0.0, 0.0);
        assert_eq!((x, y), (0.0, 0.0));
        // A cell below the viewport aligns its end to the viewport bottom.
        let (_x, y) = scroll_to_reveal(20, 0, &rows, &cols, c, 0.0, 0.0);
        assert!((y - (21.0 * 24.0 - 300.0)).abs() < 1e-6, "reveal down: {y}");
        // A cell to the right aligns its end to the viewport right edge.
        let (x, _y) = scroll_to_reveal(0, 10, &rows, &cols, c, 0.0, 0.0);
        assert!(
            (x - (11.0 * 100.0 - 400.0)).abs() < 1e-6,
            "reveal right: {x}"
        );
        // Scrolling back up to an above-viewport cell aligns its start.
        let (_x, y) = scroll_to_reveal(5, 0, &rows, &cols, c, 1000.0, 1000.0);
        assert!((y - (5.0 * 24.0)).abs() < 1e-6, "reveal up: {y}");
        // Reveal never exceeds the clamp range.
        let (x, y) = scroll_to_reveal(999, 999, &rows, &cols, c, 0.0, 0.0);
        assert!(x <= max_scroll(cols.total(), c.width) + 1e-6);
        assert!(y <= max_scroll(rows.total(), c.height) + 1e-6);
    }

    #[test]
    fn cell_at_point_inside_and_clamped() {
        let rows = uniform(100, 24.0);
        let cols = uniform(100, 100.0);
        let rhw = ROW_HEADER_MIN_W;
        let (cw, ch) = (400.0, 300.0);
        // Inside the content: A1 at the top-left content pixel.
        assert_eq!(
            cell_at_point(
                rhw + 1.0,
                COL_HEADER_H + 1.0,
                rhw,
                0.0,
                0.0,
                &rows,
                &cols,
                cw,
                ch
            ),
            CellRef::new(0, 0)
        );
        // A point in the column-header strip clamps DOWN into the top content row (row 0).
        assert_eq!(
            cell_at_point(rhw + 150.0, 3.0, rhw, 0.0, 0.0, &rows, &cols, cw, ch),
            CellRef::new(0, 1)
        );
        // A point in the row-header gutter clamps RIGHT into the first content column (col 0).
        assert_eq!(
            cell_at_point(
                5.0,
                COL_HEADER_H + 50.0,
                rhw,
                0.0,
                0.0,
                &rows,
                &cols,
                cw,
                ch
            ),
            CellRef::new(2, 0)
        );
        // Far above/left of the whole grid clamps to A1 (the corner).
        assert_eq!(
            cell_at_point(-99.0, -99.0, rhw, 0.0, 0.0, &rows, &cols, cw, ch),
            CellRef::new(0, 0)
        );
        // Far past the bottom-right edge clamps into the last visible cell of the content rect.
        let past = cell_at_point(9999.0, 9999.0, rhw, 0.0, 0.0, &rows, &cols, cw, ch);
        // Content is 400×300 → ~4 cols (0..=3/4) and ~12 rows visible from the origin.
        assert!(
            past.col <= 4 && past.row <= 12,
            "clamped into view: {past:?}"
        );
    }

    #[test]
    fn cell_at_point_scrolled_variable_geometry() {
        let rows = varied(1000);
        let cols = varied(1000);
        let rhw = ROW_HEADER_MIN_W;
        // Scroll to the start of row 50 / col 40; the top-left content pixel is that cell.
        let scroll_y = rows.offset_of(50);
        let scroll_x = cols.offset_of(40);
        assert_eq!(
            cell_at_point(
                rhw + 0.5,
                COL_HEADER_H + 0.5,
                rhw,
                scroll_x,
                scroll_y,
                &rows,
                &cols,
                400.0,
                300.0
            ),
            CellRef::new(50, 40)
        );
    }

    /// Auto-scroll hot-zone inset used in the tests (a cell height, matching the app constant).
    const HZ: f64 = 24.0;

    #[test]
    fn edge_autoscroll_delta_zero_inside() {
        let rhw = ROW_HEADER_MIN_W;
        let (cw, ch) = (400.0, 300.0);
        // A pointer comfortably inside the content (well outside every hot-zone) → no scroll.
        assert_eq!(
            edge_autoscroll_delta(rhw + 200.0, COL_HEADER_H + 150.0, rhw, cw, ch, 20.0, HZ),
            (0.0, 0.0)
        );
    }

    #[test]
    fn edge_autoscroll_delta_starts_inside_hotzone() {
        // The critical case (the CR bug): the pointer is still INSIDE the content — so a real
        // `on_mouse_move` fires — but within the hot-zone of an edge, and auto-scroll must START.
        let rhw = ROW_HEADER_MIN_W;
        let (cw, ch) = (400.0, 300.0);
        let right = rhw as f64 + cw;
        let bottom = COL_HEADER_H as f64 + ch;
        // Just inside the right edge (within the hot-zone) → positive x step, no y.
        assert_eq!(
            edge_autoscroll_delta(
                (right - 5.0) as f32,
                COL_HEADER_H + 150.0,
                rhw,
                cw,
                ch,
                20.0,
                HZ
            ),
            (20.0, 0.0)
        );
        // Just inside the bottom edge (within the hot-zone) → positive y step, no x.
        assert_eq!(
            edge_autoscroll_delta(rhw + 200.0, (bottom - 5.0) as f32, rhw, cw, ch, 20.0, HZ),
            (0.0, 20.0)
        );
        // Just inside the left/top edges → negative steps.
        assert_eq!(
            edge_autoscroll_delta(rhw + 5.0, COL_HEADER_H + 5.0, rhw, cw, ch, 20.0, HZ),
            (-20.0, -20.0)
        );
        // One px further inside than the hot-zone (interior side) → no scroll on that axis.
        assert_eq!(
            edge_autoscroll_delta(
                (right - HZ - 1.0) as f32,
                COL_HEADER_H + 150.0,
                rhw,
                cw,
                ch,
                20.0,
                HZ,
            ),
            (0.0, 0.0)
        );
    }

    #[test]
    fn edge_autoscroll_delta_past_each_edge() {
        // Once running, the loop re-reads the unclamped, out-of-window pointer — same steps.
        let rhw = ROW_HEADER_MIN_W;
        let (cw, ch) = (400.0, 300.0);
        let right = rhw as f64 + cw;
        let bottom = COL_HEADER_H as f64 + ch;
        // Past the left/top edges → negative (scroll toward the origin).
        assert_eq!(
            edge_autoscroll_delta(rhw - 50.0, COL_HEADER_H - 50.0, rhw, cw, ch, 20.0, HZ),
            (-20.0, -20.0)
        );
        // Past the right/bottom edges → positive (scroll toward the end).
        assert_eq!(
            edge_autoscroll_delta(
                (right + 50.0) as f32,
                (bottom + 50.0) as f32,
                rhw,
                cw,
                ch,
                20.0,
                HZ,
            ),
            (20.0, 20.0)
        );
        // Only one axis past an edge → only that axis scrolls.
        assert_eq!(
            edge_autoscroll_delta(
                (right + 50.0) as f32,
                COL_HEADER_H + 150.0,
                rhw,
                cw,
                ch,
                20.0,
                HZ
            ),
            (20.0, 0.0)
        );
    }

    #[test]
    fn range_overlay_rects_single_is_empty() {
        let sel = SelectionModel::single(CellRef::new(3, 3));
        assert!(range_overlay_rects(sel.range(), sel.active).is_empty());
    }

    #[test]
    fn range_overlay_rects_row_excludes_active() {
        // A single-row selection B2:E2 with the active cell at the E2 corner → the overlay
        // is the left segment B2:D2 (the active cell is excluded).
        let sel = SelectionModel {
            anchor: CellRef::new(1, 1),
            active: CellRef::new(1, 4),
        };
        let rects = range_overlay_rects(sel.range(), sel.active);
        assert_eq!(rects, vec![(1..2, 1..4)]);
        // With the active cell mid-row (a general case), both flanking segments appear.
        let mid = range_overlay_rects(sel.range(), CellRef::new(1, 2));
        assert_eq!(mid, vec![(1..2, 1..2), (1..2, 3..5)]);
    }

    #[test]
    fn range_overlay_rects_block_tiles_without_active() {
        // A 3×3 block B2:D4, active at C3 (centre). The ≤4 rects must tile all 9 cells
        // except the active one, with no overlap.
        let range = CellRange::new(CellRef::new(1, 1), CellRef::new(3, 3));
        let active = CellRef::new(2, 2);
        let rects = range_overlay_rects(range, active);
        let mut covered = std::collections::BTreeSet::new();
        for (rows, cols) in &rects {
            for r in rows.clone() {
                for col in cols.clone() {
                    assert!(covered.insert((r, col)), "overlap at ({r},{col})");
                }
            }
        }
        // Every range cell except the active one is covered exactly once.
        for r in 1..=3 {
            for col in 1..=3 {
                let want = (r, col) != (2, 2);
                assert_eq!(covered.contains(&(r, col)), want, "cell ({r},{col})");
            }
        }
        assert!(
            !covered.contains(&(2, 2)),
            "active cell must stay uncovered"
        );
    }

    // --- Text spill (`functional_spec.md §2`) -------------------------------------------

    /// A row occupancy map for the scan tests: `Blocked` iff the column is in `blocked`, else
    /// `Empty`. Mirrors what the render probe produces (content/coverage → `Blocked`).
    fn occ(blocked: &[u32]) -> impl Fn(u32) -> Occupancy + '_ {
        move |c| {
            if blocked.contains(&c) {
                Occupancy::Blocked
            } else {
                Occupancy::Empty
            }
        }
    }

    #[test]
    fn spill_direction_follows_alignment() {
        assert_eq!(spill_direction(Align::Left), SpillDirection::Right);
        assert_eq!(spill_direction(Align::Right), SpillDirection::Left);
        assert_eq!(spill_direction(Align::Center), SpillDirection::Both);
    }

    #[test]
    fn spill_span_extends_right_over_empties_stops_at_content() {
        // Origin at col 1; col 4 holds content. Rightward spill covers 1..=3 and stops before 4.
        let span = spill_span(1, SpillDirection::Right, 0, 20, occ(&[4]));
        assert_eq!(span, SpillSpan { left: 1, right: 3 });
        assert!(span.spills(1));
    }

    #[test]
    fn spill_span_extends_left_for_right_aligned() {
        // Origin at col 5; col 1 holds content. Leftward spill covers 2..=5 and stops before 1.
        let span = spill_span(5, SpillDirection::Left, 0, 20, occ(&[1]));
        assert_eq!(span, SpillSpan { left: 2, right: 5 });
        assert!(span.spills(5));
    }

    #[test]
    fn spill_span_center_extends_both_bounded_each_side() {
        // Origin at col 4; blockers at col 1 (left) and col 7 (right). Center spill covers 2..=6,
        // each side bounded independently by the nearest content cell.
        let span = spill_span(4, SpillDirection::Both, 0, 20, occ(&[1, 7]));
        assert_eq!(span, SpillSpan { left: 2, right: 6 });
    }

    #[test]
    fn spill_span_stops_at_scan_bound() {
        // No content anywhere, but the inclusive scan bound clamps both directions (the visible
        // frame / coverage edge — never spill into the unknown region past it, §2.5).
        let span = spill_span(5, SpillDirection::Both, 3, 7, occ(&[]));
        assert_eq!(span, SpillSpan { left: 3, right: 7 });
        // Rightward-only from the last in-bounds column cannot extend.
        let at_edge = spill_span(7, SpillDirection::Right, 3, 7, occ(&[]));
        assert!(!at_edge.spills(7));
    }

    #[test]
    fn spill_span_no_empty_neighbor_is_no_spill() {
        // The immediate neighbour in the spill direction has content → no spill.
        let span = spill_span(2, SpillDirection::Right, 0, 20, occ(&[3]));
        assert_eq!(span, SpillSpan { left: 2, right: 2 });
        assert!(!span.spills(2));
    }

    #[test]
    fn estimated_width_and_overflow_gate() {
        // A comfortably-fitting short label does NOT overflow a default-ish column…
        assert!(!text_overflows_column("Exactly", 13.0, 62.0, 4.0));
        // …while genuinely long text does, in the same column.
        assert!(text_overflows_column(
            "clipped-very-long-text-abcdefghijklmnop",
            13.0,
            100.0,
            4.0
        ));
        // The estimate scales with length and font size (monotone), and empty text is zero.
        assert_eq!(estimated_text_width("", 13.0), 0.0);
        assert!(estimated_text_width("aaaa", 26.0) > estimated_text_width("aaaa", 13.0));
        assert!(estimated_text_width("aaaaaaaa", 13.0) > estimated_text_width("aaaa", 13.0));
    }
}
