//! Conditional-formatting sidebar state (`components/cf_sidebar.md §3`).
//!
//! The `ChromeView`-owned view-model for the right-docked CF sidebar. P4 carries only the
//! List-mode state (the active sheet + its published rule rows). The rule **editor** sub-state
//! (`CfEditorState`) and its seeded text inputs arrive in P6; List-mode rendering of the rows
//! themselves arrives in P5.

use freecell_core::{CfRuleView, SheetId};

/// The open conditional-formatting sidebar's state — `Some` on [`ChromeView`](super::view::ChromeView)
/// ⇒ the sidebar is open (mirrors the chart panel's `Option<ChartPanel>`).
pub(crate) struct CondFmtPanel {
    /// The sheet whose rules the sidebar is showing.
    pub sheet: SheetId,
    /// The published rules for [`sheet`](Self::sheet), priority-descending, as read from
    /// `client.cond_fmt_rules(sheet)`. Empty when the sheet carries no CF. Rendered as rows in P5.
    pub rows: Vec<CfRuleView>,
}
