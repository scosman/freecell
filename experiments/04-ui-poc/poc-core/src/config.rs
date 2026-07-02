//! PoC configuration and the §5.4 performance thresholds.
//!
//! Both rendering shells (`raw-gpui/`, `gpui-component/`) build their state from a
//! [`PocConfig`] so grid size, viewport, overscan, and the pass/fail gates are shared
//! and consistent. Defaults target the **Excel-max grid** (functional_spec §5.4).

use datagen::{EXCEL_MAX_COLS, EXCEL_MAX_ROWS};

/// Frame-time budget for the sustained 120 fps target (functional_spec §5.4):
/// `1_000_000_000 / 120 ≈ 8_333_333` ns.
pub const FRAME_TARGET_NS: u64 = 8_333_333;

/// Worst-case frame-time budget: never worse than 60 fps under fast scroll / jump
/// (functional_spec §5.4): `1_000_000_000 / 60 ≈ 16_666_667` ns.
pub const FRAME_WORST_NS: u64 = 16_666_667;

/// Newly-visible-cell load budget (functional_spec §5.4): pulling values + formatting
/// for the cells entering the viewport must fit inside a frame — target `< ~2 ms`.
pub const CELL_LOAD_TARGET_NS: u64 = 2_000_000;

/// Default header sizes (logical px). Row headers are a fixed gutter on the left; the
/// column-header row is a fixed strip on top.
pub const DEFAULT_ROW_HEADER_WIDTH: f32 = 56.0;
pub const DEFAULT_COL_HEADER_HEIGHT: f32 = 24.0;

/// Configuration for a PoC run: grid dimensions, viewport, overscan, and seed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PocConfig {
    /// Logical rows in the grid (default: Excel max).
    pub rows: u32,
    /// Logical columns in the grid (default: Excel max).
    pub cols: u32,
    /// Deterministic generator seed for the backing [`datagen::SyntheticSheet`].
    pub seed: u64,
    /// Viewport width in logical px (the scrollable content area, excluding the row
    /// header gutter).
    pub viewport_width: f32,
    /// Viewport height in logical px (excluding the column-header strip).
    pub viewport_height: f32,
    /// Extra rows/cols rendered beyond the viewport on each side so a fast scroll does
    /// not flash blank cells (functional_spec §5.4 overscan).
    pub overscan_rows: u32,
    pub overscan_cols: u32,
    /// Left row-header gutter width (px).
    pub row_header_width: f32,
    /// Top column-header strip height (px).
    pub col_header_height: f32,
}

impl Default for PocConfig {
    /// The canonical PoC config: the full Excel-max grid, a laptop-ish 1440×900
    /// viewport, and a modest overscan.
    fn default() -> Self {
        Self {
            rows: EXCEL_MAX_ROWS,
            cols: EXCEL_MAX_COLS,
            seed: 0xF9EE_C011,
            viewport_width: 1440.0,
            viewport_height: 900.0,
            overscan_rows: 8,
            overscan_cols: 4,
            row_header_width: DEFAULT_ROW_HEADER_WIDTH,
            col_header_height: DEFAULT_COL_HEADER_HEIGHT,
        }
    }
}

impl PocConfig {
    /// The content viewport height available for data rows (viewport minus the column
    /// header strip).
    pub fn content_height(&self) -> f32 {
        (self.viewport_height - self.col_header_height).max(0.0)
    }

    /// The content viewport width available for data columns (viewport minus the row
    /// header gutter).
    pub fn content_width(&self) -> f32 {
        (self.viewport_width - self.row_header_width).max(0.0)
    }
}
