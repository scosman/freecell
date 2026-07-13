//! `freecell-core` — the GPUI-free, IronCalc-free foundation.
//!
//! This crate holds FreeCell's pure logic: the [`Axis`](axis::Axis) two-level prefix-sum
//! virtualization, [`CellRef`](refs::CellRef)/[`CellRange`](refs::CellRange)/A1 conversion,
//! the [`SelectionModel`](selection::SelectionModel) and keyboard-motion rules, the formula
//! [input-cap](input_cap) and [sheet-name](sheet_name) validators, the fill
//! [palette](palette), the engine-free [`RenderStyle`](style::RenderStyle), the
//! [recent-files](recent) store + formatters, and the read
//! models the grid consumes ([`Publication`](publication::Publication)/[`PublishedCell`](publication::PublishedCell),
//! the [`SheetCaches`](cache::SheetCaches) read model). It imports neither GPUI nor
//! IronCalc, so it builds and tests on any machine with no GPU or display
//! (`architecture.md §1, §3`; enforced by `tests/dependency_rule.rs`).

pub mod axis;
pub mod border;
pub mod cache;
pub mod color;
pub mod data_row;
pub mod eval_indicator;
pub mod find;
pub mod format_color;
pub mod format_ui;
pub mod input_cap;
pub mod merge_guard;
pub mod palette;
pub mod perf;
pub mod publication;
pub mod recent;
pub mod refs;
pub mod selection;
pub mod sheet_name;
pub mod stats;
pub mod style;
pub mod tsv;

// The load-bearing types re-exported at the crate root for ergonomic downstream use
// (the grid, engine, and shell all reach for these constantly).
pub use axis::Axis;
pub use border::{effective_edge, BorderSpec, Edge, LinePattern};
pub use cache::{SheetCache, SheetCacheBuilder, SheetCaches, StyleId};
pub use color::Rgb;
pub use format_ui::{adjust_decimals, font_size_display, num_fmt_category, Category};
pub use merge_guard::{blocks_col_op, blocks_row_op};
pub use publication::{CellKind, Publication, PublishedCell};
pub use recent::{DisplayEntry, RecentEntry, RecentList};
pub use refs::{CellRange, CellRef, SheetId};
pub use selection::{
    apply_motion, format_selection_ref, is_full_column_selection, is_full_row_selection,
    resolve_edge, Direction, Motion, SelectionModel, SheetDims,
};
pub use stats::{format_stat_count, format_stat_value, SelectionStats};
pub use style::{Align, RenderStyle, VAlign};

/// The Excel-max grid dimensions FreeCell targets (`CLAUDE.md`): the engine, geometry
/// cache, and validators are all sized against these hard maxima.
pub mod limits {
    /// Maximum number of rows on a sheet (Excel-max).
    pub const MAX_ROWS: u32 = 1_048_576;

    /// Maximum number of columns on a sheet (Excel-max).
    pub const MAX_COLS: u32 = 16_384;

    /// Formula input length cap (chars). Inputs longer than this are rejected before they
    /// reach the engine (`architecture.md §3` input-cap validator).
    pub const MAX_INPUT_LEN: usize = 8_192;

    /// Formula nesting-depth cap (parenthesis nesting). Deeper inputs are rejected
    /// (`architecture.md §3`).
    pub const MAX_NESTING_DEPTH: usize = 64;
}

#[cfg(test)]
mod tests {
    use super::limits;

    #[test]
    fn excel_max_constants_are_correct() {
        // The whole engine/geometry/validation stack leans on these exact maxima.
        assert_eq!(limits::MAX_ROWS, 1 << 20, "Excel-max rows is 2^20");
        assert_eq!(limits::MAX_COLS, 1 << 14, "Excel-max cols is 2^14");
    }

    #[test]
    fn input_caps_match_spec() {
        assert_eq!(limits::MAX_INPUT_LEN, 8_192);
        assert_eq!(limits::MAX_NESTING_DEPTH, 64);
    }
}
