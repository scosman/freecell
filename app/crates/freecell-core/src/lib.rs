//! `freecell-core` — the GPUI-free, IronCalc-free foundation.
//!
//! This crate holds FreeCell's pure logic: the [`Axis`](axis) two-level prefix-sum
//! virtualization, `CellRange`/A1 conversion, the `SelectionModel` and keyboard-motion
//! rules, the formula input-cap and sheet-name validators, the fill palette, the
//! engine-free `RenderStyle`, and the read models the grid consumes
//! (`Publication`/`PublishedCell`, the `SheetCaches` read model). It imports neither
//! GPUI nor IronCalc, so it builds and tests on any machine with no GPU or display
//! (`architecture.md §1, §3`).
//!
//! Phase 1 (scaffolding) ships only the workspace-wide invariants below; the logic
//! types land in Phase 2 (`implementation_plan.md`).

/// The Excel-max grid dimensions FreeCell targets (`CLAUDE.md`): the engine, geometry
/// cache, and validators are all sized against these hard maxima.
pub mod limits {
    /// Maximum number of rows on a sheet (Excel-max).
    pub const MAX_ROWS: u32 = 1_048_576;

    /// Maximum number of columns on a sheet (Excel-max).
    pub const MAX_COLS: u32 = 16_384;

    /// Formula input length cap (chars). Inputs longer than this are rejected before
    /// they reach the engine (`architecture.md §3` input-cap validator).
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
