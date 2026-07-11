//! The **ChartLayer** math (charts/architecture §4.2, §5 challenge 1): mapping a chart's
//! `xdr:twoCellAnchor` to a content-local pixel rectangle against the grid's own geometry, and
//! culling charts that fall off-screen. Kept **gpui-free** so the anchor→pixel + cull logic is
//! unit-tested headless — the [`GridView`](super::GridView) chart layer only threads its `Frame`
//! geometry in and paints the resolved rects.
//!
//! Coordinate convention matches the rest of the grid ([`super::layout`]): **content-local** px
//! have their origin at the content area's top-left (i.e. after the row-header gutter / column-
//! header strip). A chart at cell `c` starts at `col_start(c) + emu_to_px(colOff) - scroll_x`
//! content-local px — the same `offset − scroll` mapping cells use, so scroll and variable-geometry
//! ("zoom") are free.

use freecell_chart_model::{Anchor, AnchorCell, ChartSpec, Fidelity};

/// EMU (English Metric Units) per CSS pixel at Excel's 96-DPI screen basis: 914 400 EMU/inch ÷
/// 96 px/inch. An `xdr:*Anchor`'s intra-cell `colOff`/`rowOff` are in EMUs; this converts them to
/// the grid's device-independent pixels.
pub const EMU_PER_PX: f64 = 9525.0;

/// Convert an EMU length to content pixels (see [`EMU_PER_PX`]).
pub fn emu_to_px(emu: i64) -> f64 {
    emu as f64 / EMU_PER_PX
}

/// The grid geometry a chart anchor resolves against: the **content-space** (pre-scroll) start
/// offset of a column / row. A minimal seam so [`anchor_rect`] is unit-tested without a `Frame`
/// (which carries gpui `Axis`es); the view implements it over its per-frame geometry.
pub trait GridGeometry {
    /// The content-space x offset (px, pre-scroll) of column `col`'s left edge.
    fn col_start(&self, col: u32) -> f64;
    /// The content-space y offset (px, pre-scroll) of row `row`'s top edge.
    fn row_start(&self, row: u32) -> f64;
    /// The column whose span contains content-space x offset `x` (px, pre-scroll) — the inverse of
    /// [`col_start`](Self::col_start), used by [`rect_to_anchor`] to turn a dragged pixel rect back
    /// into a cell anchor (P18).
    fn col_at(&self, x: f64) -> u32;
    /// The row whose span contains content-space y offset `y` (px, pre-scroll).
    fn row_at(&self, y: f64) -> u32;
}

/// The visual size (px) of a selection resize handle square drawn at a selected chart's corners +
/// edge midpoints (P18, `ui_design §3.2`).
pub const HANDLE_PX: f32 = 8.0;
/// The half-extent (px) of a handle's **hit** zone — a little larger than the visual square so a
/// handle is easy to grab.
pub const HANDLE_HIT_HALF: f32 = 7.0;
/// The minimum width/height (px) a chart resize clamps to, so a drag can't invert or collapse it.
pub const MIN_CHART_PX: f32 = 40.0;

/// Map a chart's content-local pixel `rect` (as produced by a move/resize drag, current scroll
/// applied) **back** to an [`Anchor`] — the inverse of [`anchor_rect`] (P18). Each corner resolves
/// to the cell whose span contains it plus the intra-cell EMU offset, so a moved/resized chart
/// persists to a `twoCellAnchor` that reproduces the same rect. Offsets are clamped ≥ 0.
pub fn rect_to_anchor(
    rect: ChartRect,
    geom: &impl GridGeometry,
    scroll_x: f64,
    scroll_y: f64,
) -> Anchor {
    let x0 = rect.x as f64 + scroll_x;
    let x1 = (rect.x + rect.w) as f64 + scroll_x;
    let y0 = rect.y as f64 + scroll_y;
    let y1 = (rect.y + rect.h) as f64 + scroll_y;
    Anchor::new(anchor_cell_at(geom, x0, y0), anchor_cell_at(geom, x1, y1))
}

/// The [`AnchorCell`] for a content-space point `(x, y)` (pre-scroll px): the containing cell + the
/// intra-cell EMU offset from that cell's top-left.
fn anchor_cell_at(geom: &impl GridGeometry, x: f64, y: f64) -> AnchorCell {
    let col = geom.col_at(x);
    let row = geom.row_at(y);
    let col_off = ((x - geom.col_start(col)).max(0.0) * EMU_PER_PX).round() as i64;
    let row_off = ((y - geom.row_start(row)).max(0.0) * EMU_PER_PX).round() as i64;
    AnchorCell::with_offsets(col, col_off, row, row_off)
}

/// One of the eight selection resize handles around a chart's rect — four corners + four edge
/// midpoints (P18). Each names the edge(s) a drag on it moves ([`Handle::moves`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

/// Which edges a drag on this handle moves: `(left, right, top, bottom)`.
struct MovedEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Handle {
    /// All eight handles, in a stable order.
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// The handle's center point on `rect` (the point the small square is drawn around).
    pub fn center(self, rect: ChartRect) -> (f32, f32) {
        let (l, r) = (rect.x, rect.x + rect.w);
        let (t, b) = (rect.y, rect.y + rect.h);
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        match self {
            Handle::TopLeft => (l, t),
            Handle::Top => (cx, t),
            Handle::TopRight => (r, t),
            Handle::Right => (r, cy),
            Handle::BottomRight => (r, b),
            Handle::Bottom => (cx, b),
            Handle::BottomLeft => (l, b),
            Handle::Left => (l, cy),
        }
    }

    /// The `HANDLE_PX`-sized square rect drawn for this handle (centered on [`center`](Self::center)).
    pub fn square(self, rect: ChartRect) -> ChartRect {
        let (cx, cy) = self.center(rect);
        ChartRect {
            x: cx - HANDLE_PX / 2.0,
            y: cy - HANDLE_PX / 2.0,
            w: HANDLE_PX,
            h: HANDLE_PX,
        }
    }

    fn moves(self) -> MovedEdges {
        let (mut left, mut right, mut top, mut bottom) = (false, false, false, false);
        match self {
            Handle::TopLeft => (left, top) = (true, true),
            Handle::Top => top = true,
            Handle::TopRight => (right, top) = (true, true),
            Handle::Right => right = true,
            Handle::BottomRight => (right, bottom) = (true, true),
            Handle::Bottom => bottom = true,
            Handle::BottomLeft => (left, bottom) = (true, true),
            Handle::Left => left = true,
        }
        MovedEdges {
            left,
            right,
            top,
            bottom,
        }
    }
}

/// The handle of `rect` a content-local point `(x, y)` grabs, if any (within [`HANDLE_HIT_HALF`]).
/// Corners win over edges when the zones overlap (their order in [`Handle::ALL`] puts corners
/// first at each end). Only the **selected** chart's handles are hit-tested by the caller.
pub fn handle_at(rect: ChartRect, x: f32, y: f32) -> Option<Handle> {
    Handle::ALL.into_iter().find(|h| {
        let (cx, cy) = h.center(rect);
        (x - cx).abs() <= HANDLE_HIT_HALF && (y - cy).abs() <= HANDLE_HIT_HALF
    })
}

/// Apply a drag delta `(dx, dy)` (content px, from the grab point) to `start` by moving the edges
/// [`handle`](Handle) controls, clamped so width/height stay ≥ [`MIN_CHART_PX`] (P18). A corner
/// moves two edges; an edge midpoint moves one. The opposite (fixed) edges never move, so the
/// resize pins the chart's far side.
pub fn resize_rect(start: ChartRect, handle: Handle, dx: f32, dy: f32) -> ChartRect {
    let m = handle.moves();
    let mut left = start.x;
    let mut right = start.x + start.w;
    let mut top = start.y;
    let mut bottom = start.y + start.h;
    if m.left {
        left = (left + dx).min(right - MIN_CHART_PX);
    }
    if m.right {
        right = (right + dx).max(left + MIN_CHART_PX);
    }
    if m.top {
        top = (top + dy).min(bottom - MIN_CHART_PX);
    }
    if m.bottom {
        bottom = (bottom + dy).max(top + MIN_CHART_PX);
    }
    ChartRect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    }
}

/// Translate `rect` by a move drag delta `(dx, dy)` (content px) — the whole chart follows the
/// pointer (P18). Size is unchanged.
pub fn move_rect(rect: ChartRect, dx: f32, dy: f32) -> ChartRect {
    ChartRect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

/// A chart's **content-local** pixel rectangle (origin at the content area's top-left, current
/// scroll already applied), as produced by [`anchor_rect`]. The layer clips it to the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ChartRect {
    /// Whether this rect has nothing to paint inside the content viewport
    /// `[0, content_w) × [0, content_h)` — a degenerate (zero-area) rect, or one lying wholly off
    /// any edge. Such charts are **culled** so an off-screen chart costs ~nothing (functional_spec
    /// §8; the resident value stays, only the paint is skipped).
    pub fn is_offscreen(&self, content_w: f64, content_h: f64) -> bool {
        let (x, y, w, h) = (self.x as f64, self.y as f64, self.w as f64, self.h as f64);
        w <= 0.0 || h <= 0.0 || x + w <= 0.0 || y + h <= 0.0 || x >= content_w || y >= content_h
    }
}

/// Map a chart's `twoCellAnchor` to its content-local pixel rect against the grid geometry, with
/// the current scroll applied. Each corner is `col_start(cell) + emu_to_px(cellOff) − scroll` (and
/// the row analogue), so the chart tracks scroll and variable geometry with no extra bookkeeping
/// (charts/architecture §5 challenge 1). Width/height are clamped ≥ 0 so a degenerate anchor
/// (e.g. a `oneCellAnchor` whose `to` fell back to `from`) yields an empty rect
/// [`is_offscreen`](ChartRect::is_offscreen) culls rather than a negative-size element.
pub fn anchor_rect(
    anchor: &Anchor,
    geom: &impl GridGeometry,
    scroll_x: f64,
    scroll_y: f64,
) -> ChartRect {
    let x0 = geom.col_start(anchor.from.col) + emu_to_px(anchor.from.col_off_emu) - scroll_x;
    let x1 = geom.col_start(anchor.to.col) + emu_to_px(anchor.to.col_off_emu) - scroll_x;
    let y0 = geom.row_start(anchor.from.row) + emu_to_px(anchor.from.row_off_emu) - scroll_y;
    let y1 = geom.row_start(anchor.to.row) + emu_to_px(anchor.to.row_off_emu) - scroll_y;
    ChartRect {
        x: x0 as f32,
        y: y0 as f32,
        w: (x1 - x0).max(0.0) as f32,
        h: (y1 - y0).max(0.0) as f32,
    }
}

/// The **always-resident** per-chart data the ChartLayer needs to place a chart and decide whether
/// it is on-screen: its in-grid [`Anchor`] and its derived [`Fidelity`], both [`Copy`] and tiny.
/// Classified **once** when charts are installed ([`set_sheet_charts`](super::GridView::set_sheet_charts))
/// so the per-frame cull scan never re-parses source XML.
///
/// The heavy render picture ([`Chart`](freecell_chart_model::Chart)) is deliberately **not** held
/// here — it stays in the shared `Arc<[ChartSpec]>` the grid installs, and is touched **only** for
/// the handful of charts actually on-screen (charts/architecture §5 challenge 5, "off-screen
/// free"): a huge sheet with K charts scans K of these tiny placements per frame but materializes
/// only the visible few, and an off-screen chart holds no render resources until it scrolls back in.
#[derive(Clone, Copy, Debug)]
pub struct ChartPlacement {
    pub anchor: Anchor,
    pub fidelity: Fidelity,
}

impl ChartPlacement {
    /// The placement for a [`ChartSpec`]: keep its [`Anchor`], and snapshot its
    /// [`display_fidelity`](ChartSpec::display_fidelity) so the source is classified once, not per
    /// frame. The spec's render [`Chart`](freecell_chart_model::Chart) is left in the shared spec.
    pub fn from_spec(spec: &ChartSpec) -> Self {
        Self {
            anchor: spec.anchor,
            fidelity: spec.display_fidelity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{
        Anchor, AnchorCell, Axis, Category, Chart, ChartKind, Grouping, Legend, Series, SourceXml,
    };

    /// A uniform mock geometry (every column `col_w` px, every row `row_h` px) — enough to check
    /// the anchor→pixel mapping without the real prefix-sum axes.
    struct Uniform {
        col_w: f64,
        row_h: f64,
    }
    impl GridGeometry for Uniform {
        fn col_start(&self, col: u32) -> f64 {
            col as f64 * self.col_w
        }
        fn row_start(&self, row: u32) -> f64 {
            row as f64 * self.row_h
        }
        fn col_at(&self, x: f64) -> u32 {
            (x.max(0.0) / self.col_w).floor() as u32
        }
        fn row_at(&self, y: f64) -> u32 {
            (y.max(0.0) / self.row_h).floor() as u32
        }
    }

    fn sample_line_chart() -> Chart {
        Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("A"),
                vec![Category::Text("Q1".into()), Category::Text("Q2".into())],
                vec![1.0, 2.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn emu_to_px_converts_at_96_dpi() {
        assert_eq!(emu_to_px(0), 0.0);
        assert_eq!(emu_to_px(9525), 1.0);
        assert_eq!(emu_to_px(19_050), 2.0);
        assert_eq!(emu_to_px(-9525), -1.0);
        // Half a pixel.
        assert!((emu_to_px(4762) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn anchor_rect_maps_corners_with_offsets_and_scroll() {
        let geom = Uniform {
            col_w: 100.0,
            row_h: 24.0,
        };
        // From (col 1 + 1 px, row 2 + 2 px) to (col 6 + 0, row 14 + 0).
        let anchor = Anchor::new(
            AnchorCell::with_offsets(1, 9525, 2, 19_050),
            AnchorCell::with_offsets(6, 0, 14, 0),
        );
        // No scroll: x0 = 100 + 1 = 101; x1 = 600; y0 = 48 + 2 = 50; y1 = 336.
        let r = anchor_rect(&anchor, &geom, 0.0, 0.0);
        assert!((r.x - 101.0).abs() < 1e-3, "x = {}", r.x);
        assert!((r.y - 50.0).abs() < 1e-3, "y = {}", r.y);
        assert!((r.w - 499.0).abs() < 1e-3, "w = {}", r.w);
        assert!((r.h - 286.0).abs() < 1e-3, "h = {}", r.h);

        // Scroll shifts the origin but not the size.
        let s = anchor_rect(&anchor, &geom, 40.0, 10.0);
        assert!((s.x - 61.0).abs() < 1e-3, "scrolled x = {}", s.x);
        assert!((s.y - 40.0).abs() < 1e-3, "scrolled y = {}", s.y);
        assert!((s.w - r.w).abs() < 1e-3 && (s.h - r.h).abs() < 1e-3);
    }

    #[test]
    fn anchor_rect_degenerate_anchor_is_zero_area() {
        let geom = Uniform {
            col_w: 100.0,
            row_h: 24.0,
        };
        // A oneCellAnchor fallback (to == from) → zero width/height, not negative.
        let anchor = Anchor::new(AnchorCell::new(3, 3), AnchorCell::new(3, 3));
        let r = anchor_rect(&anchor, &geom, 0.0, 0.0);
        assert_eq!((r.w, r.h), (0.0, 0.0));
        assert!(
            r.is_offscreen(1000.0, 1000.0),
            "a zero-area chart is culled"
        );
    }

    #[test]
    fn is_offscreen_culls_each_side_and_degenerate() {
        let (cw, ch) = (640.0, 320.0);
        // Fully on-screen → visible.
        assert!(!ChartRect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        // Straddling the left/top edge (partially visible) → still visible.
        assert!(!ChartRect {
            x: -40.0,
            y: -20.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        // Wholly left / above / right / below → culled.
        assert!(ChartRect {
            x: -120.0,
            y: 10.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        assert!(ChartRect {
            x: 10.0,
            y: -120.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        assert!(ChartRect {
            x: cw as f32 + 5.0,
            y: 10.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        assert!(ChartRect {
            x: 10.0,
            y: ch as f32 + 5.0,
            w: 100.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
        // Zero-area is culled even inside the viewport.
        assert!(ChartRect {
            x: 10.0,
            y: 10.0,
            w: 0.0,
            h: 80.0,
        }
        .is_offscreen(cw, ch));
    }

    #[test]
    fn rect_to_anchor_inverts_anchor_rect() {
        let geom = Uniform {
            col_w: 100.0,
            row_h: 24.0,
        };
        // A twoCellAnchor with intra-cell offsets → rect → back to the same anchor.
        let anchor = Anchor::new(
            AnchorCell::with_offsets(1, 9525, 2, 19_050),
            AnchorCell::with_offsets(6, 4762, 14, 4762),
        );
        for (sx, sy) in [(0.0, 0.0), (40.0, 10.0), (250.0, 120.0)] {
            let rect = anchor_rect(&anchor, &geom, sx, sy);
            let back = rect_to_anchor(rect, &geom, sx, sy);
            assert_eq!(
                back.from.col, anchor.from.col,
                "from.col at scroll {sx},{sy}"
            );
            assert_eq!(back.from.row, anchor.from.row);
            assert_eq!(back.to.col, anchor.to.col);
            assert_eq!(back.to.row, anchor.to.row);
            // Offsets round-trip within a rounding EMU (pixel → EMU → pixel).
            assert!((back.from.col_off_emu - anchor.from.col_off_emu).abs() <= EMU_PER_PX as i64);
            assert!((back.to.row_off_emu - anchor.to.row_off_emu).abs() <= EMU_PER_PX as i64);
        }
    }

    #[test]
    fn handle_at_hits_each_handle_and_misses_the_interior() {
        let rect = ChartRect {
            x: 100.0,
            y: 50.0,
            w: 200.0,
            h: 120.0,
        };
        // Each handle's center is grabbed.
        for h in Handle::ALL {
            let (cx, cy) = h.center(rect);
            assert_eq!(handle_at(rect, cx, cy), Some(h), "grab {h:?}");
        }
        // The interior (well inside every handle zone) grabs nothing.
        assert_eq!(handle_at(rect, 200.0, 110.0), None);
    }

    #[test]
    fn resize_rect_moves_the_dragged_edges_and_clamps_min() {
        let start = ChartRect {
            x: 100.0,
            y: 50.0,
            w: 200.0,
            h: 120.0,
        };
        // Dragging the bottom-right corner grows width + height; the top-left stays pinned.
        let r = resize_rect(start, Handle::BottomRight, 30.0, 40.0);
        assert_eq!((r.x, r.y), (100.0, 50.0));
        assert_eq!((r.w, r.h), (230.0, 160.0));

        // Dragging the left edge inward moves x + shrinks width; the right edge stays pinned.
        let r = resize_rect(start, Handle::Left, 50.0, 0.0);
        assert_eq!(r.x, 150.0);
        assert_eq!(r.w, 150.0);
        assert_eq!(r.x + r.w, 300.0, "right edge pinned");

        // An extreme inward left drag clamps to MIN_CHART_PX (never inverts).
        let r = resize_rect(start, Handle::Left, 500.0, 0.0);
        assert_eq!(r.w, MIN_CHART_PX);
        assert_eq!(r.x + r.w, 300.0, "right edge still pinned");
    }

    #[test]
    fn chart_placement_from_spec_derives_fidelity_and_anchor() {
        let anchor = Anchor::new(AnchorCell::new(1, 1), AnchorCell::new(6, 14));
        let spec = |xml: &str| {
            ChartSpec::loaded(sample_line_chart(), SourceXml::new(xml), Vec::new(), anchor)
        };
        assert_eq!(
            ChartPlacement::from_spec(&spec("<c:lineChart/>")).fidelity,
            Fidelity::Faithful
        );
        assert_eq!(
            ChartPlacement::from_spec(&spec("<c:bar3DChart/>")).fidelity,
            Fidelity::Degraded
        );
        assert_eq!(
            ChartPlacement::from_spec(&spec("<c:surfaceChart/>")).fidelity,
            Fidelity::Unsupported
        );
        // The anchor is carried through; the heavy render `Chart` is NOT copied into the placement
        // (it stays in the shared spec — "off-screen free").
        assert_eq!(
            ChartPlacement::from_spec(&spec("<c:lineChart/>")).anchor,
            anchor
        );
    }
}
