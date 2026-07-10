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

use freecell_chart_model::{Anchor, ChartSpec, Fidelity};

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
