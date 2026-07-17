//! Conditional-formatting sidebar state (`components/cf_sidebar.md §3`).
//!
//! The `ChromeView`-owned view-model for the right-docked CF sidebar: the active sheet + its
//! published rule rows (List mode) plus the [`CfEditorState`] sub-state that drives the rule
//! **Editor** mode. The editor's seeded text inputs (range / operands / formula) live on
//! [`ChromeView`](super::view::ChromeView) itself (mirroring the chart title/axis inputs).

use freecell_core::{CfFormat, CfPeriod, CfRuleView, CfTextOp, CfValueOp, SheetId};

/// The open conditional-formatting sidebar's state — `Some` on [`ChromeView`](super::view::ChromeView)
/// ⇒ the sidebar is open (mirrors the chart panel's `Option<ChartPanel>`).
pub(crate) struct CondFmtPanel {
    /// The sheet whose rules the sidebar is showing.
    pub sheet: SheetId,
    /// The published rules for [`sheet`](Self::sheet), priority-descending, as read from
    /// `client.cond_fmt_rules(sheet)`. Empty when the sheet carries no CF.
    pub rows: Vec<CfRuleView>,
    /// The rule editor's sub-state: `None` ⇒ **List mode**; `Some` ⇒ **Editor mode** (add / edit).
    pub editor: Option<CfEditorState>,
}

/// Which highlight rule family/variant the editor is authoring (`components/cf_sidebar.md §3`).
/// Drives the operand controls + the assembled [`CfRuleSpec`](freecell_core::CfRuleSpec). The
/// color-scale variants (`ColorScale2`/`ColorScale3`) are added by P7 (its dedicated editor); they
/// are omitted here so no never-constructed variant trips the `-D warnings` dead-code lint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CfEditorKind {
    CellValue,
    Text,
    Dates,
    TopBottom,
    Average,
    Duplicate,
    Blanks,
    Errors,
    Formula,
}

/// The rule editor's working state (`components/cf_sidebar.md §3`). The A1 range + the value / text
/// / formula operands live in `ChromeView`'s seeded text inputs; everything else — the chosen
/// family/variant, the operators, the boolean sub-toggles, the differential format, and the
/// validation/engine messages — lives here.
pub(crate) struct CfEditorState {
    /// `None` = adding a new rule; `Some(index)` = editing the rule at that stable storage index.
    pub edit_index: Option<u32>,
    /// The selected rule family/variant (drives which operand controls render + the built spec).
    pub kind: CfEditorKind,
    /// The *Cell value* operator (used when [`kind`](Self::kind) is `CellValue`).
    pub value_op: CfValueOp,
    /// The *Text* operator (used when `kind` is `Text`).
    pub text_op: CfTextOp,
    /// The *A date occurring* period (used when `kind` is `Dates`).
    pub period: CfPeriod,
    /// The Top/Bottom rank seed (the live value is the operand-1 input; this seeds it on open).
    pub top_rank: u32,
    /// Top/Bottom counts by percent of range rather than by item count.
    pub top_percent: bool,
    /// Top/Bottom picks the **bottom** N rather than the top N.
    pub top_bottom: bool,
    /// Above/Below-average picks **below** the average rather than above.
    pub average_below: bool,
    /// Duplicate/Unique targets **unique** values rather than duplicates.
    pub duplicate_unique: bool,
    /// Blanks targets **non-blank** cells rather than blanks.
    pub blanks_no: bool,
    /// Errors targets **non-error** cells rather than errors.
    pub errors_no: bool,
    /// The differential format the highlight applies (fill / text color / bold / italic).
    pub format: CfFormat,
    /// Halt lower-priority rules for a cell this rule matches.
    pub stop_if_true: bool,
    /// Engine `Err` messages surfaced inline (client-side validation is computed live, not stored).
    /// Rendered red above Save; cleared on any form edit / re-open.
    pub errors: Vec<String>,
    /// True between a Save-send and its outcome: a success `CondFmtUpdated` returns to List mode,
    /// an engine `Err` clears this and keeps the editor open with the message
    /// (`components/cf_sidebar.md §4/§6`).
    pub pending_save: bool,
}

impl CfEditorState {
    /// A fresh editor state: add-defaults when `edit_index` is `None`, or the shell an edit then
    /// overwrites from the rule's spec. Defaults to a *Cell value* `> ` rule with an empty format.
    pub fn new(edit_index: Option<u32>) -> Self {
        Self {
            edit_index,
            kind: CfEditorKind::CellValue,
            value_op: CfValueOp::Gt,
            text_op: CfTextOp::Contains,
            period: CfPeriod::Today,
            top_rank: 10,
            top_percent: false,
            top_bottom: false,
            average_below: false,
            duplicate_unique: false,
            blanks_no: false,
            errors_no: false,
            format: CfFormat::default(),
            stop_if_true: false,
            errors: Vec::new(),
            pending_save: false,
        }
    }
}
