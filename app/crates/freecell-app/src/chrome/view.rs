//! [`ChromeView`] — the action row, data row (formula bar), and sheet tab bar as one GPUI
//! entity (`components/app_shell.md`, `ui_design.md §3.1–3.4`).
//!
//! Thin plumbing over the Phase-2 pure logic: the [`DataRow`] reducer drives the content
//! field, the [`EvalIndicator`] drives the evaluating spinner, [`FILL_PALETTE`] the fill
//! swatches, and [`validate_sheet_name`] the inline rename. Every user action is a plain
//! method here (so it is unit-testable without pixel clicks); the widget handlers just call
//! those methods, and the reducers' effects are performed as [`ChromeClient`] commands and
//! [`ChromeGridRequest`]s.
//!
//! The fill popover, tab context menu, and delete-confirm modal are lightweight
//! `ChromeView`-owned panels (controlled by view state) rather than the stock
//! gpui-component `Popover`/`ContextMenu`/`Modal` — their content closures run in a foreign
//! entity context, which would force cross-entity dispatch for what is a functional-POC
//! surface (`ui_design.md`: "this is chrome — don't over-invest"). Buttons, the text inputs,
//! the color picker, and the spinner are stock gpui-component controls as specced.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, rgb, App, ClickEvent, Context, CursorStyle, Entity, FocusHandle,
    Focusable, Hsla, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Rgba, SharedString, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState, Position};
use gpui_component::spinner::Spinner;
use gpui_component::{Disableable as _, Icon, Selectable as _, Sizable as _};

use freecell_core::data_row::{DataRow, DataRowEffect, DataRowEvent, FieldMode};
use freecell_core::eval_indicator::{EvalEffect, EvalEvent, EvalIndicator};
use freecell_core::format_ui::{
    adjust_decimals_cell, displayed_decimals, font_size_display, is_more_only_num_fmt,
    num_fmt_category, toggle_thousands, Category, BASIC_FORMATS, NUM_FMT_GROUPS,
};
use freecell_core::functions::{self, FnSig};
use freecell_core::input_cap::InputRejection;
use freecell_core::palette::FILL_PALETTE;
use freecell_core::selection::{Direction, Motion};
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::{
    format_stat_count, format_stat_value, limits, Align, CellKind, CellRef, CfFormat, CfPeriod,
    CfPreview, CfRuleSpec, CfRuleView, CfTextOp, CfValueOp, RenderStyle, Rgb, SelectionModel,
    SelectionStats, SheetId, VAlign,
};

use crate::grid::caret_intent_modifiers;

use freecell_chart_model::{Anchor as ChartAnchor, AnchorCell, ChartId, LegendPosition};

use freecell_engine::{
    BorderLine, BorderPreset, ChartAxisKind, ChartChromeEdit, ChartInsertKind, Command,
    DataLabelToggles, EditRejectedReason, StyleAttr, StylePath, WorkerEvent,
};

use super::cond_fmt::{CfEditorKind, CfEditorState, CondFmtPanel};
use super::h_scroller::{h_scroller, HScroller};
use super::sidebar::{close_button, docked_sidebar, section};
use super::{
    AutocompleteDisplay, AutocompleteRow, ChromeClient, ChromeGridRequest, ChromeGridSink,
    EditController, EditOrigin, SheetTab,
};

/// The function-autocomplete list's live state while the dropdown is open (`ChromeView`-owned,
/// `gaps_closing_7_15 §1`). Cleared to `None` when the list dismisses.
struct Autocomplete {
    /// The current filtered, ordered match list (from [`functions::complete`]).
    matches: Vec<&'static FnSig>,
    /// The highlighted row index into [`matches`](Self::matches).
    highlight: usize,
    /// The byte offset in the edit text where the typed prefix begins — the replace point on
    /// accept.
    token_start: usize,
}

/// The 250 ms no-flash delay for both the content-fetch and evaluating spinners
/// (`ui_design.md §3.1/§3.2`, mirrored from the grid's own delayed hooks).
const SPINNER_DELAY: Duration = Duration::from_millis(250);

/// Debounce before a selection-change fires a `SelectionStats` query — a drag-select emits many
/// selection changes, so the readout waits for the drag to settle (`architecture.md §1`).
const STATS_DEBOUNCE: Duration = Duration::from_millis(120);

// --- Chrome look constants (functional POC greys; `ui_design.md §3`) -----------------
const CHROME_BG: u32 = 0xF3F3F3;
// `HAIRLINE`, `ACTIVE_TAB_BG`, `TEXT`, `MUTED_TEXT` are `pub(crate)` so the shared docked-sidebar
// container (`chrome::sidebar`) paints the identical card + section labels.
pub(crate) const HAIRLINE: u32 = 0xD9D9D9;
const DIVIDER: u32 = 0xC8C8C8;
pub(crate) const ACTIVE_TAB_BG: u32 = 0xFFFFFF;
pub(crate) const TEXT: u32 = 0x1F1F1F;
pub(crate) const MUTED_TEXT: u32 = 0x555555;
/// Danger border/text for cap-rejected input + invalid rename (theme danger, `#DC2626`).
const DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the cap-error popover (`ui_design.md §4`).
const TOOLTIP_BG: u32 = 0x2B2B2B;
const TOOLTIP_TEXT: u32 = 0xF5F5F5;
/// The highlighted-row tint in the function-completion list (a light accent wash,
/// `gaps_closing_7_15 §1`).
const AUTOCOMPLETE_HL_BG: u32 = 0xE8F0FE;
/// The completion list's minimum width so argument templates fit without jitter.
const AUTOCOMPLETE_MIN_W: f32 = 300.0;
/// Accent ring around the borders popover's selected color swatch (Office Accent 1 — reads over a
/// black or white swatch, unlike a grey/dark ring; `ui_design.md §2.1`).
const SWATCH_SELECTED_RING: u32 = 0x4472C4;
/// The borders target-icon 2×2 diagram: light-grey context gridlines vs. the solid-dark affected
/// edges (`ui_design.md §2.2`). Drawn from `div` rectangles, the same primitive as the grid's edges.
const TARGET_ICON_PX: f32 = 22.0;
const TARGET_ICON_GREY: u32 = 0xC8C8C8;
const TARGET_ICON_DARK: u32 = 0x1F1F1F;
/// The conditional-formatting list-row preview swatch (`components/cf_sidebar.md §5`): a small
/// hairline chip carrying a highlight rule's fill+text, a colour scale's banded gradient, or a
/// deferred-family Badge tag.
const CF_SWATCH_W: f32 = 22.0;
const CF_SWATCH_H: f32 = 16.0;
/// The muted tag background behind a deferred-family Badge preview (a light grey).
const CF_BADGE_BG: u32 = 0xEDEDED;
/// The tint behind an open CF-editor inline dropdown's option list (a faint grey wash so the
/// expanded options read as a group under their trigger).
const CF_MENU_BG: u32 = 0xF7F7F7;
/// A colour swatch's side in the CF format editor's fill/text palette.
const CF_SWATCH_SIDE: f32 = 18.0;
/// The "range required" validation message — shown inline under the Applies-to field (so it is
/// filtered out of the general message list). One source so the field hint + the filter agree.
const CF_RANGE_REQUIRED: &str = "Enter a range in Applies to.";

/// Which of the CF rule editor's inline dropdowns is expanded (`components/cf_sidebar.md §5`). Each
/// trigger toggles its own menu; only one is open at a time (a scrolling sidebar body, so the
/// options render inline below the trigger rather than as an anchored popover).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CfMenu {
    RuleType,
    ValueOp,
    TextOp,
    Period,
}

// `pub(crate)` so the shared docked-sidebar container (`chrome::sidebar`) positions the card
// between the data row and the tab bar (the right-docked width is `sidebar::SIDEBAR_W`).
pub(crate) const ACTION_ROW_H: f32 = 36.0;

/// The default footprint (in cells) of a chart inserted from the action bar — a typical Excel
/// default chart size (~8 columns × 15 rows), anchored at the active cell (`ui_design §3.1`).
const CHART_INSERT_COLS: u32 = 8;
const CHART_INSERT_ROWS: u32 = 15;

/// The action-bar chart-insert menu entries — `(kind, icon path, label)`, in menu order
/// (`ui_design §3.1`). Every [`ChartInsertKind`] authors a near-empty chart of that type (bubble
/// landed as the final type in P26). Order matches how Excel groups the types: **Area sits right
/// after Line** (both are trend charts) before the Column/Bar pair (post-v1 Batch 2, item 13). This
/// is the single canonical order shared by the action-bar dropdown ([`render_chart_menu`]) and the
/// edit panel's Type row ([`render_chart_type_row`]).
const CHART_MENU: [(ChartInsertKind, &str, &str); 8] = [
    (ChartInsertKind::Line, "icons/chart-line.svg", "Line"),
    (ChartInsertKind::Area, "icons/chart-area.svg", "Area"),
    (ChartInsertKind::Column, "icons/chart-column.svg", "Column"),
    (ChartInsertKind::Bar, "icons/chart-bar.svg", "Bar"),
    (ChartInsertKind::Pie, "icons/chart-pie.svg", "Pie"),
    (
        ChartInsertKind::Doughnut,
        "icons/chart-doughnut.svg",
        "Doughnut",
    ),
    (
        ChartInsertKind::Scatter,
        "icons/chart-scatter.svg",
        "Scatter",
    ),
    (ChartInsertKind::Bubble, "icons/chart-bubble.svg", "Bubble"),
];

/// The action-row dropdown/popover triggers whose panel anchors under the button. The buttons are
/// content-sized (their labels — font family, size, number-format category — change width), so a
/// popover's x-offset can't be a fixed constant (BUG 2c); each trigger's real laid-out left edge is
/// captured into [`ChromeView::anchor_x`] by a `canvas` probe and the panel renders at that x.
/// Discriminants are the `anchor_x` indices.
#[derive(Clone, Copy)]
enum Anchor {
    FontFamily = 0,
    FontSize = 1,
    TextColor = 2,
    Fill = 3,
    Borders = 4,
    NumFmt = 5,
    Chart = 6,
}
const ANCHOR_COUNT: usize = 7;

impl Anchor {
    fn idx(self) -> usize {
        self as usize
    }
}

/// The fixed font-size dropdown list in points (`functional_spec.md §3.2`).
const FONT_SIZES: [f64; 12] = [8., 9., 10., 11., 12., 14., 16., 18., 20., 24., 28., 36.];
/// The top "clear the family override" entry in the font-family dropdown (`ui_design.md §2`).
const SYSTEM_DEFAULT_FAMILY: &str = "Default (Inter)";
// `DATA_ROW_H` / `TAB_BAR_H` are `pub(crate)` so the shared docked-sidebar container
// (`chrome::sidebar`) can dock the card between the data row and the tab bar.
pub(crate) const DATA_ROW_H: f32 = 32.0;
/// The formula-bar content entry's height: [`DATA_ROW_H`] minus 2 px breathing room above **and**
/// below (BUG C), so the row's `items_center` insets the entry within the bar without changing the
/// bar height. gpui-component's single-line `Input` otherwise renders at its fixed control height
/// (`Size::Medium` → 32 px) and fills the row edge-to-edge, which reads as cramped.
const DATA_ROW_FIELD_H: f32 = DATA_ROW_H - 4.0;
pub(crate) const TAB_BAR_H: f32 = 30.0;
/// A tab press that moves less than this (device px) is a click (select / rename), not a drag —
/// only past it does the lift + drop indicator appear (`ui_design.md §3`).
const TAB_DRAG_THRESHOLD_PX: f32 = 4.0;
/// The reorder drop indicator + dragged-tab outline accent (Office Accent 1, matching the
/// borders popover's selected-swatch ring). `ui_design.md §3`: a 2 px accent vertical bar.
const TAB_DROP_ACCENT: u32 = 0x4472C4;
/// Half the inter-tab gap (`gap_1` = 4 px), used to place the drop indicator in the gap when it
/// lands before the first / after the last tab.
const TAB_GAP_HALF: f32 = 2.0;
const REF_BOX_W: f32 = 72.0;
/// The find/replace bar's two text fields' width (`ui_design.md §1`: ~220 px each).
const FIND_FIELD_W: f32 = 220.0;
/// The match counter's min width so "3 of 12" ↔ "No results" doesn't jitter the trailing group.
const FIND_COUNTER_MIN_W: f32 = 72.0;
/// Muted counter text (`ui_design.md §1`: "No results" in a muted color).
const FIND_COUNTER_MUTED: u32 = 0x777777;
/// The content field's left edge inside the data row = padding + ref box + gap + divider +
/// gap (`render_data_row` layout); the cap-error popover anchors here.
const DATA_ROW_CONTENT_LEFT: f32 = 8.0 + REF_BOX_W + 8.0 + 1.0 + 8.0;

/// One series in the chart **edit panel** — its display name + current color, for the per-series
/// color swatch row (P20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartPanelSeries {
    pub name: String,
    /// The series' current explicit color (`None` = the renderer's palette pick), for the highlighted
    /// swatch.
    pub color: Option<Rgb>,
}

/// The right-docked chart **edit panel**'s state (P19 skeleton + P20 chrome) — the chart it is
/// shaping. `kind`/`ranges` drive the (authored-only) Type + Data-range sections; the chrome fields
/// (`title`, `legend`, axis titles, `series`, `labels`) drive the P20 controls. All are updated
/// optimistically on an edit, then reconciled by the window from the republished snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartPanel {
    /// The chart's host sheet (the `SetChart*` target).
    pub sheet: SheetId,
    /// The chart's stable id.
    pub id: ChartId,
    /// Whether the chart was authored in-app — authored charts expose the Type + Data-range controls
    /// (loaded re-type/re-range is not P20; the worker ignores those for loaded ids). Both provenances
    /// expose the chrome controls.
    pub is_authored: bool,
    /// The chart's current type (for the highlighted type glyph).
    pub kind: ChartInsertKind,
    /// A short summary of the current bound data range(s), `None` if unset.
    pub ranges: Option<String>,
    /// The current chart title (`None` = no title).
    pub title: Option<String>,
    /// The current legend position (`None` = legend off).
    pub legend: Option<LegendPosition>,
    /// The current category / value axis titles (`None` = untitled).
    pub cat_axis_title: Option<String>,
    pub val_axis_title: Option<String>,
    /// One entry per series (name + current color), for the color swatch rows.
    pub series: Vec<ChartPanelSeries>,
    /// The chart's current data-label toggles (read from its first series).
    pub labels: DataLabelToggles,
}

impl ChartPanel {
    /// A panel for `chart` with the chrome fields defaulted — a convenience for tests + the
    /// near-empty authored insert case (the window fills the chrome from the snapshot).
    #[cfg(test)]
    pub fn skeleton(sheet: SheetId, id: ChartId, is_authored: bool, kind: ChartInsertKind) -> Self {
        Self {
            sheet,
            id,
            is_authored,
            kind,
            ranges: None,
            title: None,
            legend: None,
            cat_axis_title: None,
            val_axis_title: None,
            series: Vec::new(),
            labels: DataLabelToggles::default(),
        }
    }
}

/// A potential or in-flight sheet-tab reorder drag (`functional_spec.md §6.1`, `ui_design.md §3`).
/// Recorded on a tab mouse-down as a *potential* drag; `dragging` flips true only once the pointer
/// crosses [`TAB_DRAG_THRESHOLD_PX`] from `start_x`, at which point the lift + drop indicator
/// appear. Modeled off the grid's `ResizeDrag`. All coordinates are window-space device px.
#[derive(Debug, Clone, Copy)]
struct TabDrag {
    /// The sheet being dragged. The active sheet follows this **id** across the move (not the
    /// slot), so a reorder never changes which sheet is active.
    sheet: SheetId,
    /// Window x at mouse-down — the threshold origin.
    start_x: f32,
    /// Live window x, updated on every move.
    cur_x: f32,
    /// Whether the pointer has crossed the movement threshold (past it = a real drag, not a click).
    dragging: bool,
}

/// One tab's captured window-space horizontal span, written by a per-tab `canvas` bounds probe
/// during paint (the Window-free geometry the pure insertion computation reads). Keyed by
/// [`SheetId`] and read back in `self.sheets` order, so a stale/partial capture is simply ignored.
#[derive(Debug, Clone, Copy)]
struct TabSpan {
    sheet: SheetId,
    left: f32,
    right: f32,
}

/// The insertion gap a tab drop would land in: the count of tab centers at/left of `cursor_x`
/// (`tab_centers` ordered left→right, in the same coordinate space as `cursor_x`). Returns an
/// index in `0..=n` — the gap the 2 px drop indicator snaps to, already clamped so a drop cannot
/// pass the trailing `+` button. Pure (no `Window`), so the drag geometry is unit-testable.
fn tab_insertion_index(cursor_x: f32, tab_centers: &[f32]) -> usize {
    tab_centers.iter().filter(|&&c| cursor_x >= c).count()
}

/// Convert an insertion `gap` (`0..=n`, from [`tab_insertion_index`]) into the fork's final
/// `to_index` for a sheet currently at `from_slot`, or `None` when the drop is a no-op (lands back
/// on the origin slot). Removing the dragged tab shifts every later gap left by one, so a gap past
/// the origin maps to `gap - 1`; both gaps adjacent to the origin (`from` and `from + 1`) resolve
/// to `from` — a no-op. Pure, so it is unit-testable alongside [`tab_insertion_index`].
fn move_target_for_gap(gap: usize, from_slot: usize) -> Option<usize> {
    let to = if gap <= from_slot { gap } else { gap - 1 };
    (to != from_slot).then_some(to)
}

/// The chrome around the grid: action row + data row + sheet tab bar.
pub struct ChromeView {
    client: Rc<dyn ChromeClient>,
    grid: ChromeGridSink,
    focus_handle: FocusHandle,

    /// The active sheet (mirrors the grid); commands + fetches are scoped to it.
    active_sheet: SheetId,
    /// The current selection (mirrored from the grid) — drives the ref box, toggle states,
    /// and the content fetch.
    selection: SelectionModel,
    /// The active cell's resolved style, cached at selection-change time for the toggles.
    active_style: Option<RenderStyle>,
    /// The active cell's number-format code, cached alongside `active_style` — drives the
    /// number-format dropdown's category label + the decimals ± enabled/computed state
    /// (`components/action_bar.md`). `None` on a multi-cell selection (matches `active_style`).
    active_num_fmt: Option<String>,
    /// The active cell's font-family name (`""` = the workbook default = "Default (Inter)"), cached
    /// alongside `active_style` for the family dropdown's label. `None` on a multi-cell selection.
    active_font_family: Option<String>,
    /// The active cell's evaluated kind + displayed value from the latest publication, cached
    /// alongside `active_num_fmt` — lets the decimals ± buttons enable on a *numeric* General cell
    /// (`200000`) while staying disabled on a text/date General cell (BUG 3). `None` on a multi-cell
    /// selection or an empty/off-viewport active cell.
    active_published: Option<(CellKind, String)>,
    /// The workbook's default font size in points, cached from the resident cache — the size box
    /// labels a **default** cell (`font_size_q == 0`) with this instead of a hardcoded value, so the
    /// label reflects the real default (13pt for a new workbook, the file's default otherwise —
    /// `components/action_bar.md`). `None` until a cache is resident. Workbook-global, so it is
    /// refreshed unconditionally (not gated on a single-cell selection).
    default_font_size_pt: Option<f64>,
    /// Whether the worker is degraded (read-only): every mutating action-bar control disables
    /// (`functional_spec.md §6`). Set by the window on `WorkerDegraded`.
    degraded: bool,

    /// The formula-bar state machine (`freecell-core`).
    data_row: DataRow,
    /// The content field's text buffer (stock gpui-component input).
    content_input: Entity<InputState>,
    /// The in-cell editor + cross-editor sync (`components/edit_controller.md`). Owns the reused
    /// in-cell overlay `InputState`; the data-row half is `content_input` + the `DataRow` reducer.
    edit: EditController,
    /// Whether the last edit-state push to the grid was non-empty (a mirror / overlay was shown),
    /// so an idle selection move doesn't re-push an all-`None` clear on every keystroke.
    edit_state_shown: bool,
    /// Whether the current pending edit is in **quick-edit** mode (`functional_spec.md §5`). Set by
    /// `begin_typed` (type-to-replace entry); cleared by `begin_in_cell`, by any caret-intent signal
    /// (mouse-down in the field, Home/End, a modified arrow — see [`leave_quick_edit`](Self::leave_quick_edit)),
    /// and on commit/cancel. While set + editing, an unmodified arrow commits + moves the active cell
    /// instead of the caret.
    quick_edit: bool,
    /// The `(sheet, cell)` whose fetched content currently lives in the reducer's `committed`
    /// field. The in-cell editor seeds from `committed` **only** for this exact sheet+cell — the
    /// single shared reducer keeps a previous cell's `committed` across a single→single selection
    /// change, and its content is not sheet-scoped, so seeding by `(sheet, cell)` prevents opening
    /// the editor with another cell's/sheet's stale content while the target's fetch is in flight
    /// (`components/edit_controller.md §Grid integration`; data-corruption guard). Reset to `None`
    /// whenever `committed` is cleared or invalidated (multi-select, sheet switch); `None` until the
    /// first reply lands.
    committed_cell: Option<(SheetId, CellRef)>,
    /// A worker `EditRejected{InputCap}` backstop (the UI validates first, so this is rare);
    /// carries the rejection so the popover shows the same message as a local cap reject.
    cap_error_external: Option<InputRejection>,

    /// The function-autocomplete dropdown's live state while open, else `None`
    /// (`gaps_closing_7_15 §1`). Driven off the active editor's text + caret after each
    /// keystroke ([`recompute_autocomplete`](Self::recompute_autocomplete)); rendered under the
    /// data row directly, and under the in-cell overlay by the grid (via `EditState`).
    autocomplete: Option<Autocomplete>,
    /// The active passive signature-hint template (the whole `NAME(args…)` line), or `None`
    /// when the caret is not inside a recognized call. Static (D1.1 — no current-arg tracking).
    sig_hint: Option<&'static str>,

    /// The evaluating-spinner state machine (`freecell-core`).
    eval: EvalIndicator,

    /// The fill popover's open state (a `ChromeView`-owned panel).
    fill_open: bool,
    /// The stock color picker for the fill popover's "Custom…" entry.
    color_picker: Entity<ColorPickerState>,
    /// The text-color popover's open state (mirrors the fill popover, with "Automatic" in place
    /// of "No fill" — `components/action_bar.md`).
    text_color_open: bool,
    /// The stock color picker for the text-color popover's "Custom…" entry.
    text_color_picker: Entity<ColorPickerState>,
    /// The number-format dropdown's open state (a `ChromeView`-owned menu panel).
    num_fmt_open: bool,
    /// The number-format dropdown's **drill-in** state (`functional_spec.md §10.1`, D10.1). `false`
    /// = the basics-first view (the 7 [`BASIC_FORMATS`] flat + a trailing "More ▸" row); `true` =
    /// the full grouped [`NUM_FMT_GROUPS`] view (with a "◂ Back" row). Reset to `false` at every
    /// popover-close so the dropdown always reopens basics-first (except when it opens directly onto
    /// a More-only active format — see [`toggle_num_fmt_popover`](Self::toggle_num_fmt_popover)).
    num_fmt_more_open: bool,
    /// The chart-insert menu's open state (the action-bar chart-type glyph menu, P17). Like the
    /// other formatting popovers it closes on click-away / a type pick / degrade.
    chart_menu_open: bool,
    /// The right-docked **chart edit panel** (P19, `ui_design §4`), open while a chart is being
    /// shaped. It closes on its × button, on **click-away** (a grid click on a cell/empty area,
    /// routed through [`on_selection_changed`](Self::on_selection_changed) — post-v1 Batch 2, item
    /// 12), on the chart's deletion, or on degrade. Clicking *another* chart re-points it (a switch).
    /// The window drives open/close/refresh (`shell::window`); the panel's controls send
    /// `SetChartType` / `SetChartRange` / `SetChartChrome` for its `(sheet, id)`.
    chart_panel: Option<ChartPanel>,
    /// The chart edit-panel's text inputs (P20 chrome): title + category/value axis titles. Seeded
    /// when the panel opens for a NEW chart id (never on a live republish — so an in-progress edit
    /// isn't clobbered), committed **live per keystroke** (`Change`), with Enter/blur as redundant
    /// commit points (post-v1 Batch 2, item 6).
    chart_title_input: Entity<InputState>,
    chart_cat_axis_input: Entity<InputState>,
    chart_val_axis_input: Entity<InputState>,
    /// The panel target `(sheet, id)` captured when a chart text input **gained focus** — the
    /// staleness guard for a deferred `Blur`. If the panel re-points to a different chart between
    /// focus and the field's commit (a rapid selection switch while a field holds unsaved text), the
    /// captured key no longer matches the panel and the stale commit is dropped, so a field's text can
    /// never be sent to the wrong chart. `None` when no chart input is focused.
    chart_input_focus: Option<(SheetId, ChartId)>,
    /// The right-docked **conditional-formatting sidebar** (`components/cf_sidebar.md`), open while
    /// managing a sheet's CF rules. `Some` ⇒ open (mirrors [`chart_panel`](Self::chart_panel)); the
    /// two share the right dock and are **mutually exclusive** (opening one closes the other). Closes
    /// on its × / the action-bar toggle / degrade; does **not** close on grid selection change, and
    /// re-scopes to the new sheet on a sheet switch. P4 renders List mode only (rows P5, editor P6).
    cond_fmt: Option<CondFmtPanel>,
    /// The CF rule-editor's seeded text inputs (`components/cf_sidebar.md §3`), seeded when the
    /// editor opens (mirrors the chart title/axis inputs). Read at Save time — not live-committed:
    /// the Applies-to range, the value operand(s) (operand-1 also carries the Text value + the
    /// Top/Bottom rank), and the custom formula.
    cf_range_input: Entity<InputState>,
    cf_operand1_input: Entity<InputState>,
    cf_operand2_input: Entity<InputState>,
    cf_formula_input: Entity<InputState>,
    /// Which of the editor's inline dropdowns (rule-type / operator / period) is currently expanded,
    /// or `None`. Only one opens at a time; opening the editor / saving / cancelling clears it.
    cf_menu_open: Option<CfMenu>,
    /// The installed font-family names for the family dropdown, fetched once at build
    /// (`cx.text_system().all_font_names()`), sorted-unique with "Default (Inter)" prepended
    /// (`components/action_bar.md`). `Rc` so the render closure can clone it cheaply.
    font_names: Rc<Vec<SharedString>>,
    /// The font-family dropdown's open state (a `ChromeView`-owned scrolling menu panel).
    font_family_open: bool,
    /// The font-size dropdown's open state.
    font_size_open: bool,
    /// The borders popover's open state (the pen-model card — target icons + line gallery +
    /// color; `ui_design.md §2`). Only click-away / Esc closes it; a target click paints and
    /// keeps it open.
    borders_open: bool,
    /// The pen's **selected target** — which set of edges the line/color controls paint right now
    /// (`functional_spec.md §2.1`). `None` on open (and after `None`/click-away); reset every open.
    border_target: Option<BorderPreset>,
    /// The pen's **line style**, default thin solid, reset every open (`ui_design.md §2.4`).
    border_line: BorderLine,
    /// The pen's **color**, default black, reset every open.
    border_color: Rgb,
    /// The stock color picker for the borders popover's "Custom…" entry (reused pattern, like the
    /// fill/text-color pickers).
    border_color_picker: Entity<ColorPickerState>,
    /// The captured chrome-local left-x (device px) of each action-row dropdown trigger, so its
    /// popover anchors under the real (content-sized) button rather than a hardcoded offset (BUG
    /// 2c). Written by a per-button `canvas` bounds probe during paint; indexed by [`Anchor`].
    anchor_x: [f32; ANCHOR_COUNT],

    /// The sheet tabs (the chrome's mirror of the worker's sheet list).
    sheets: Vec<SheetTab>,
    /// The sheet being inline-renamed, if any.
    rename_target: Option<SheetId>,
    /// The inline-rename text input (reused across renames).
    rename_input: Entity<InputState>,
    /// Whether the pending rename failed validation (danger border, stays editing).
    rename_error: bool,
    /// The tab whose right-click context menu is open, if any.
    context_menu: Option<SheetId>,
    /// The sheet pending a delete confirmation (non-empty sheet), if any.
    confirm_delete: Option<SheetId>,
    /// A potential or in-flight tab reorder drag (`functional_spec.md §6`, `ui_design.md §3`).
    tab_drag: Option<TabDrag>,
    /// Each tab's captured window-space horizontal span, refreshed by a per-tab `canvas` probe on
    /// every paint — the geometry the pure insertion-index computation reads (a `Window`-free
    /// snapshot). Keyed by [`SheetId`]; read back in `self.sheets` order.
    tab_spans: Vec<TabSpan>,

    // ---- Find / replace bar (`functional_spec.md §4`, `ui_design.md §1`) -------------------
    /// Whether the find/replace bar is open (rendered below the data row, pushing the grid down).
    find_open: bool,
    /// The Find field's text buffer.
    find_input: Entity<InputState>,
    /// The Replace field's text buffer.
    replace_input: Entity<InputState>,
    /// The **match-case** toggle (`Aa`): off = case-insensitive (default), on = exact case.
    match_case: bool,
    /// The **match-entire-cell** toggle: off = substring (default), on = whole-cell equality.
    whole_cell: bool,
    /// The current match set (row-major `CellRef`s from the worker's `FindResults`); empty = no
    /// matches / empty find field.
    matches: Vec<CellRef>,
    /// The index into [`matches`](Self::matches) of the current match, or `None` when there are no
    /// matches. Drives the "N of M" counter + which cell is selected/revealed.
    match_idx: Option<usize>,
    /// Set while a `ReplaceAll` reply is awaited, so its `ReplacedCount` shows the "Replaced N"
    /// notice (a single `ReplaceOne`'s count is not surfaced — `functional_spec.md §4.4`).
    pending_replace_all: bool,
    /// A transient "Replaced N" notice shown in the counter after a Replace All until the user next
    /// edits the find field / steps matches (`functional_spec.md §4.4`).
    replaced_notice: Option<usize>,

    // ---- Selection stats (the tab-bar status readout, `functional_spec.md §1`) --------------
    /// The latest worker-computed aggregate for the current selection, or `None` when there is
    /// nothing to show (a single-cell/empty selection, or no reply yet). Rendered right-aligned in
    /// the tab bar; only shown when it has ≥1 numeric cell (`SelectionStats::has_numeric`).
    selection_stats: Option<SelectionStats>,
    /// Whether the readout is expanded to also show Min / Max (a **session-only** toggle, flipped by
    /// clicking the readout — `functional_spec.md §1`).
    stats_show_minmax: bool,
    /// Monotonic tag for the debounced stats query: it both debounces (only the most-recently armed
    /// timer fires the send) and stamps the request's `req_id`, so a reply for a superseded
    /// selection is dropped.
    stats_seq: u64,

    /// Horizontal-scroller state for the **action row** — its button groups scroll (with chevrons)
    /// when the window is too small to fit them (`functional_spec.md §9B`, call site 1).
    action_scroller: HScroller,
    /// Horizontal-scroller state for the **sheet-tab strip** — the tabs scroll while the
    /// selection-stats group stays pinned static to the right (`functional_spec.md §9B`, call site 2
    /// → §9A.4 always-visible).
    tab_scroller: HScroller,

    /// The grid, hosted as the chrome's body so the layout is action-row → data-row → **grid**
    /// → tab-bar (`ui_design.md §3`). `None` in the standalone Phase-9 demo/tests; the Phase-11
    /// window installs the real `GridView` via [`set_grid_body`](Self::set_grid_body).
    body: Option<gpui::AnyView>,

    _subscriptions: Vec<gpui::Subscription>,
}

impl ChromeView {
    /// Builds the chrome over `client`, delivering grid requests to `grid`. Starts on
    /// `active_sheet` with an A1 selection and the given tabs; the content field begins Idle
    /// and fetches on the first `on_selection_changed`.
    pub fn new(
        client: Rc<dyn ChromeClient>,
        grid: ChromeGridSink,
        active_sheet: SheetId,
        sheets: Vec<SheetTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content_input = cx.new(|cx| InputState::new(window, cx).placeholder(""));
        let in_cell_input = cx.new(|cx| InputState::new(window, cx).placeholder(""));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let chart_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Chart title"));
        let chart_cat_axis_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Category axis"));
        let chart_val_axis_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Value axis"));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let text_color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let border_color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let cf_range_input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. B2:B20"));
        let cf_operand1_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let cf_operand2_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let cf_formula_input = cx.new(|cx| InputState::new(window, cx).placeholder("=A1>0"));

        // Installed font families for the dropdown, fetched once (`all_font_names` is verified
        // available). "Default (Inter)" is prepended as the clear-the-override entry.
        let mut names: Vec<SharedString> =
            std::iter::once(SharedString::from(SYSTEM_DEFAULT_FAMILY))
                .chain(
                    cx.text_system()
                        .all_font_names()
                        .into_iter()
                        .map(SharedString::from),
                )
                .collect();
        names.dedup();
        let font_names = Rc::new(names);

        // The data-row edit keys (Tab and — in quick-edit — the unmodified arrows) must be seen
        // *before* the gpui-component single-line `Input` acts on them. That `Input` binds
        // Left/Right to caret actions (`MoveLeft`/`MoveRight`) via the keymap; in this gpui build,
        // action bindings dispatch *before* any `capture_key_down`/`on_key_down` listener and stop
        // propagation once handled, so an ancestor capture listener can never preempt the input's
        // Left/Right (Up/Down happen to be unbound in single-line mode, which is the only reason
        // they used to work). A keystroke *interceptor* is the one phase that runs before the
        // input's action bindings, and `stop_propagation` inside it prevents that action dispatch
        // (`feature-gaps-7-11/DECISIONS_TO_REVIEW.md`). It is guarded to this view's focused
        // data-row input, so it never touches other inputs or the in-cell overlay, and it delegates
        // to the same [`handle_data_row_edit_key`](Self::handle_data_row_edit_key) the direct-call
        // unit tests exercise.
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(&content_input, window, Self::on_content_event),
            cx.subscribe_in(&in_cell_input, window, Self::on_incell_event),
            cx.subscribe_in(&rename_input, window, Self::on_rename_event),
            cx.subscribe_in(&color_picker, window, Self::on_color_picker_event),
            cx.subscribe_in(&text_color_picker, window, Self::on_text_color_picker_event),
            cx.subscribe_in(
                &border_color_picker,
                window,
                Self::on_border_color_picker_event,
            ),
            cx.subscribe_in(&chart_title_input, window, Self::on_chart_title_event),
            cx.subscribe_in(&chart_cat_axis_input, window, Self::on_chart_cat_axis_event),
            cx.subscribe_in(&chart_val_axis_input, window, Self::on_chart_val_axis_event),
            cx.subscribe_in(&find_input, window, Self::on_find_input_event),
            cx.subscribe_in(&replace_input, window, Self::on_replace_input_event),
            cx.subscribe_in(&cf_range_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_operand1_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_operand2_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_formula_input, window, Self::on_cf_input_event),
            cx.intercept_keystrokes(move |event, window, cx| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // Only when this view's data-row input is the focused editor — never the
                    // in-cell overlay (its own input) or an unrelated field.
                    let focused = this
                        .content_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window);
                    if !focused {
                        return;
                    }
                    let keystroke = &event.keystroke;
                    let key = keystroke.key.as_str();
                    if this.handle_data_row_edit_key(key, keystroke.modifiers, window, cx) {
                        // Suppress the input's competing caret action for this keystroke.
                        cx.stop_propagation();
                    } else if matches!(key, "left" | "right" | "home" | "end")
                        && this.data_mode() == FieldMode::Editing
                    {
                        // A caret-only key falls through to the input; recompute the list/hint once
                        // the input has moved the caret (`functional_spec.md §1`).
                        this.schedule_autocomplete_recompute(window, cx);
                    }
                });
            }),
        ];

        Self {
            client,
            grid,
            focus_handle: cx.focus_handle(),
            active_sheet,
            selection: SelectionModel::default(),
            active_style: None,
            active_num_fmt: None,
            active_font_family: None,
            active_published: None,
            default_font_size_pt: None,
            degraded: false,
            data_row: DataRow::default(),
            content_input,
            edit: EditController::new(in_cell_input),
            edit_state_shown: false,
            quick_edit: false,
            committed_cell: None,
            cap_error_external: None,
            autocomplete: None,
            sig_hint: None,
            eval: EvalIndicator::default(),
            fill_open: false,
            color_picker,
            text_color_open: false,
            text_color_picker,
            num_fmt_open: false,
            num_fmt_more_open: false,
            chart_menu_open: false,
            chart_panel: None,
            chart_title_input,
            chart_cat_axis_input,
            chart_val_axis_input,
            chart_input_focus: None,
            cond_fmt: None,
            cf_range_input,
            cf_operand1_input,
            cf_operand2_input,
            cf_formula_input,
            cf_menu_open: None,
            anchor_x: [0.0; ANCHOR_COUNT],
            font_names,
            font_family_open: false,
            font_size_open: false,
            borders_open: false,
            border_target: None,
            border_line: BorderLine::ThinSolid,
            border_color: Rgb::new(0, 0, 0),
            border_color_picker,
            sheets,
            rename_target: None,
            rename_input,
            rename_error: false,
            context_menu: None,
            confirm_delete: None,
            tab_drag: None,
            tab_spans: Vec::new(),
            find_open: false,
            find_input,
            replace_input,
            match_case: false,
            whole_cell: false,
            matches: Vec::new(),
            match_idx: None,
            pending_replace_all: false,
            replaced_notice: None,
            selection_stats: None,
            stats_show_minmax: false,
            stats_seq: 0,
            action_scroller: HScroller::new(),
            tab_scroller: HScroller::new(),
            body: None,
            _subscriptions: subscriptions,
        }
    }

    /// Installs the grid as the chrome's body (the Phase-11 window calls this once), so the
    /// chrome renders action-row → data-row → grid (flex-fill) → tab-bar in one stack.
    pub fn set_grid_body(&mut self, body: gpui::AnyView, cx: &mut Context<Self>) {
        self.body = Some(body);
        cx.notify();
    }

    /// Re-reads the active cell's resolved style (the action-row toggle pressed states) without
    /// disturbing the data row — for a `StyleCacheUpdated` after a formatting edit that didn't
    /// move the selection (`components/app_shell.md §Action row`).
    pub fn refresh_active_style(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_single() {
            let cell = self.selection.active;
            self.active_style = self.client.render_style(self.active_sheet, cell);
            self.active_num_fmt = self.client.num_fmt_code(self.active_sheet, cell);
            self.active_font_family = self.client.font_family_name(self.active_sheet, cell);
            self.active_published = self.client.published_cell(self.active_sheet, cell);
        } else {
            self.active_style = None;
            self.active_num_fmt = None;
            self.active_font_family = None;
            self.active_published = None;
        }
        // The workbook default size is selection-independent (used to label a default cell).
        self.default_font_size_pt = self.client.default_font_size_pt(self.active_sheet);
        cx.notify();
    }

    // ---- Selection + data-row plumbing ----------------------------------------------------

    /// The grid's selection changed: refresh the ref box + toggle states, and drive the
    /// content field's fetch/disable via the reducer.
    pub fn on_selection_changed(
        &mut self,
        selection: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = selection;
        self.cap_error_external = None;
        // A multi-cell selection clears the reducer's `committed` (data_row multi arm), so the
        // seed tag it named is no longer valid — reset it (else a later collapse-to-single +
        // in-cell open would seed the just-cleared empty content; data-corruption guard).
        if !selection.is_single() {
            self.committed_cell = None;
        }
        if selection.is_single() {
            self.active_style = self
                .client
                .render_style(self.active_sheet, selection.active);
            self.active_num_fmt = self
                .client
                .num_fmt_code(self.active_sheet, selection.active);
            self.active_font_family = self
                .client
                .font_family_name(self.active_sheet, selection.active);
            self.active_published = self
                .client
                .published_cell(self.active_sheet, selection.active);
        } else {
            self.active_style = None;
            self.active_num_fmt = None;
            self.active_font_family = None;
            self.active_published = None;
        }
        // The workbook default size is selection-independent (used to label a default cell).
        self.default_font_size_pt = self.client.default_font_size_pt(self.active_sheet);
        let effects = self.data_row.reduce(DataRowEvent::SelectionChanged {
            single: selection.is_single(),
        });
        // begin_fetch / disable cleared the field; mirror the reducer's text into the widget.
        self.sync_input_from_reducer(window, cx);
        self.apply_data_effects(effects, window, cx);
        // A selection change ends any pending edit — close the in-cell overlay + clear the mirror.
        self.edit.close();
        self.refresh_edit_grid_state(window, cx);
        // Click-away closes the chart edit panel (post-v1 Batch 2, item 12): a grid click on a
        // cell/header/empty area (or a paste / sheet switch) routes here and dismisses the panel.
        // A click on *another chart* does NOT route here — the grid emits `ChartSelected` instead,
        // which re-points the panel (a switch, not a close) — and the panel's own controls never
        // change the grid selection, so they can't dismiss it either.
        //
        // The CF sidebar is deliberately EXEMPT from this click-away close (`components/cf_sidebar.md
        // §4`): it stays open across selection changes so the user can pick the "Applies to" range by
        // selecting cells. It re-scopes to a new sheet via the sheet-switch path, not here.
        self.close_chart_panel(cx);
        // Refresh the tab-bar selection-stats readout for the new selection (debounced).
        self.request_selection_stats(cx);
        cx.notify();
    }

    /// Re-request the selection-stats readout — the window calls this on `WorkerEvent::Published`
    /// so an edit that changes a value **inside** a still-active multi-cell selection re-aggregates
    /// (`functional_spec.md §1` live-update). Debounced + deduped like the selection-change path.
    pub fn refresh_selection_stats(&mut self, cx: &mut Context<Self>) {
        self.request_selection_stats(cx);
    }

    /// Issue the debounced `SelectionStats` query for the current selection (`functional_spec.md
    /// §1`). Bumps [`stats_seq`](Self::stats_seq) (which invalidates any in-flight reply); a
    /// single-cell / empty selection shows nothing, so it clears the readout and sends no query.
    /// A multi-cell selection arms a [`STATS_DEBOUNCE`] timer that fires the query only if no newer
    /// selection change has superseded it, tagging the request with `seq` so a stale reply is
    /// dropped on arrival.
    fn request_selection_stats(&mut self, cx: &mut Context<Self>) {
        self.stats_seq = self.stats_seq.wrapping_add(1);
        let seq = self.stats_seq;
        if self.selection.is_single() {
            if self.selection_stats.take().is_some() {
                cx.notify();
            }
            return;
        }
        let sheet = self.active_sheet;
        let range = self.selection.range();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATS_DEBOUNCE).await;
            this.update(cx, |this, _cx| {
                // Only the most-recently armed timer sends — an intervening selection change bumped
                // `stats_seq`, superseding this one.
                if this.stats_seq == seq {
                    this.client.send(Command::SelectionStats {
                        sheet,
                        range,
                        req_id: seq,
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Flip the session-only Min / Max expansion of the stats readout (`functional_spec.md §1`).
    pub fn toggle_stats_minmax(&mut self, cx: &mut Context<Self>) {
        self.stats_show_minmax = !self.stats_show_minmax;
        cx.notify();
    }

    /// The labeled parts of the selection-stats readout, or `None` when nothing should show — a
    /// single-cell/empty selection (no stats), or a selection with no numeric cell. Default form is
    /// `Sum · Average · Count`; the session toggle appends `Min · Max` (`functional_spec.md §1`).
    /// Pure — the render + the tests read the same source.
    pub fn stats_readout_parts(&self) -> Option<Vec<String>> {
        let stats = self.selection_stats?;
        if !stats.has_numeric() {
            return None;
        }
        let mut parts = vec![
            format!("Sum: {}", format_stat_value(stats.sum)),
            format!(
                "Average: {}",
                format_stat_value(stats.average().unwrap_or_default())
            ),
            format!("Count: {}", format_stat_count(stats.count)),
        ];
        if self.stats_show_minmax {
            parts.push(format!(
                "Min: {}",
                format_stat_value(stats.min.unwrap_or_default())
            ));
            parts.push(format!(
                "Max: {}",
                format_stat_value(stats.max.unwrap_or_default())
            ));
        }
        Some(parts)
    }

    /// The full selection-stats readout as one string (`"Sum: … Average: … Count: …"`), or `None`
    /// when hidden — a test accessor mirroring what the tab bar renders.
    pub fn selection_stats_text(&self) -> Option<String> {
        self.stats_readout_parts().map(|parts| parts.join("   "))
    }

    /// The grid asks the field to commit a pending edit before a click-away selection change
    /// (`components/grid.md`). Returns whether the field is now committable (a cap-rejected
    /// edit blocks — the caller keeps the field editing and cancels the pending move).
    pub fn on_edit_commit_requested(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let was_editing = self.data_row.mode() == FieldMode::Editing;
        let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
        self.apply_data_effects(effects, window, cx);
        let committed = self.data_row.mode() != FieldMode::Editing;
        self.note_commit(was_editing);
        // A committed (or absent) edit closes the overlay + leaves quick-edit; a cap-rejected one
        // stays open + editing.
        if committed {
            self.edit.close();
            self.quick_edit = false;
        }
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
        committed
    }

    /// Commits any pending data-row edit (Excel click-away) and then adopts `selection` — but
    /// only if the commit succeeded. Returns whether the selection was adopted; a cap-rejected
    /// edit blocks it (`false`), leaving the field Editing so the caller keeps the grid on the
    /// last accepted cell (`functional_spec.md §3.3`). This is the single choke point every
    /// non-emitter selection-adoption path routes through, so [`on_selection_changed`] is never
    /// reached while the field is `Editing` — the invariant its `data_row` `debug_assert` guards
    /// (`components/grid.md`; a violation would silently discard the pending edit).
    ///
    /// [`on_selection_changed`]: Self::on_selection_changed
    pub fn commit_then_adopt_selection(
        &mut self,
        selection: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let committed = self.on_edit_commit_requested(window, cx);
        if committed {
            self.on_selection_changed(selection, window, cx);
        }
        committed
    }

    /// Escape while editing: revert the field to the last-fetched content, close any in-cell
    /// overlay, and hand focus back to the grid.
    pub fn escape_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_row.mode() != FieldMode::Editing {
            return;
        }
        let effects = self.data_row.reduce(DataRowEvent::Escape);
        self.sync_input_from_reducer(window, cx);
        self.mirror_to_in_cell(window, cx);
        self.apply_data_effects(effects, window, cx);
        self.edit.close();
        self.quick_edit = false;
        self.autocomplete = None;
        self.sig_hint = None;
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    // ---- Pending edit: type-to-replace, in-cell editor, Tab, mirror -----------------------
    // (`components/edit_controller.md`; the single pending edit lives in `content_input` + the
    // `DataRow` reducer, with `edit` adding the in-cell overlay + two-editor sync.)

    /// The reused in-cell editor input — the window hands a clone to the grid so it can render the
    /// overlay (`components/edit_controller.md §4.4`).
    pub fn in_cell_input(&self) -> Entity<InputState> {
        self.edit.in_cell_input()
    }

    /// Type-to-replace (`functional_spec.md §1.1`): a printable keystroke on the focused grid
    /// starts an edit of the active cell whose content is **replaced** by `text`, caret at end, in
    /// the data row (never the in-cell overlay). Works from Idle **or** a multi-cell selection
    /// (targets the active cell — the grid collapses the range first).
    pub fn begin_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit.close();
        self.edit.set_origin(EditOrigin::DataRow);
        self.cap_error_external = None;
        // Type-to-replace is the sole entry into quick-edit (`functional_spec.md §5.1`): an
        // unmodified arrow now commits + moves the active cell instead of the caret.
        self.quick_edit = true;
        // Force Editing with the typed char (supersedes any pending fetch / disabled multi state).
        let effects = self.data_row.reduce(DataRowEvent::Edited {
            text: text.to_string(),
        });
        self.content_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx);
            input.focus(window, cx);
        });
        self.apply_data_effects(effects, window, cx);
        self.recompute_autocomplete(cx);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Open the in-cell editor over `cell` (`functional_spec.md §1.3`). Double-click / F2 route
    /// here. Seeds from the reducer's **committed** content (the last fetched raw), so it shows the
    /// real content even if a redundant re-select cleared the live field but the reply already
    /// landed. If a first content fetch is still in flight the overlay opens empty and
    /// [`on_worker_event`](Self::on_worker_event) promotes it once the reply arrives
    /// (empty-with-spinner, `§Grid integration`).
    pub fn begin_in_cell(&mut self, cell: CellRef, window: &mut Window, cx: &mut Context<Self>) {
        // Don't relocate the overlay onto a different cell while another cell's edit is still
        // pending (e.g. a cap-rejected click-away, whose selection revert is deferred) — the
        // reducer + selection remain on the old cell, so opening here would diverge (review #2).
        if self.data_row.mode() == FieldMode::Editing && cell != self.selection.active {
            return;
        }
        self.cap_error_external = None;
        // The in-cell editor (double-click / F2) is never quick-edit — arrows control the caret
        // (`functional_spec.md §5.1`), even if this promotes an in-progress type-to-replace.
        self.quick_edit = false;
        // Enter Editing seeded with the committed raw content, unless already editing this cell
        // (F2 mid-edit keeps the pending text) or the fetch for THIS cell hasn't landed yet. The
        // reducer keeps a previous cell's `committed` across a single→single selection change, so
        // seed only when `committed` is known to belong to `cell`; otherwise open empty and let the
        // in-flight reply promote it (guards a cross-cell stale-content commit, review New Critical).
        // Only seed when not already editing this cell AND `committed` is known to hold THIS
        // sheet+cell's fetched content; otherwise leave the reducer Idle-awaiting and let the
        // in-flight reply promote the overlay.
        if self.data_row.mode() != FieldMode::Editing
            && self.committed_cell == Some((self.active_sheet, cell))
        {
            let committed = self.data_row.committed().to_string();
            self.content_input.update(cx, |input, cx| {
                input.set_value(committed.clone(), window, cx);
            });
            let effects = self
                .data_row
                .reduce(DataRowEvent::Edited { text: committed });
            self.apply_data_effects(effects, window, cx);
        }
        let text = self.content_input.read(cx).value().to_string();
        self.edit.set_syncing(true);
        self.edit.in_cell().update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        self.edit.set_syncing(false);
        self.edit.open_on(cell);
        self.recompute_autocomplete(cx);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Tab / Shift+Tab from the in-cell overlay (routed via the grid): commit + move
    /// right / left (`functional_spec.md §1.4`).
    pub fn commit_incell_move(
        &mut self,
        dir: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_and_move(dir, window, cx);
    }

    /// Escape from the in-cell overlay (routed via the grid): cancel the edit, revert, close. When
    /// the overlay is open but no edit has started yet (a first fetch is still in flight), there is
    /// nothing to revert — just close the overlay and return focus to the grid.
    pub fn cancel_incell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_row.mode() == FieldMode::Editing {
            self.escape_edit(window, cx);
        } else if self.edit.is_open() {
            self.edit.close();
            self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
            self.refresh_edit_grid_state(window, cx);
            cx.notify();
        }
    }

    /// Commit the pending edit and move the active cell in `dir` (Enter → Down, Shift+Enter → Up,
    /// Tab → Right, Shift+Tab → Left). A cap-rejected commit keeps the edit (no move). Shared by
    /// both editors' Enter/Tab paths.
    fn commit_and_move(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let was_editing = self.data_row.mode() == FieldMode::Editing;
        let mut effects = self.data_row.reduce(DataRowEvent::Commit);
        // The reducer's Commit hardcodes a Down move; retarget it to `dir`.
        for effect in &mut effects {
            if matches!(
                effect,
                DataRowEffect::MoveActive(Motion::Move(Direction::Down))
            ) {
                *effect = DataRowEffect::MoveActive(Motion::Move(dir));
            }
        }
        self.apply_data_effects(effects, window, cx);
        self.note_commit(was_editing);
        // A successful commit ends the edit → close the overlay + leave quick-edit; a cap-rejected
        // one stays open (and stays in quick-edit so a re-arrow retries the commit).
        if self.data_row.mode() != FieldMode::Editing {
            self.edit.close();
            self.quick_edit = false;
            self.autocomplete = None;
            self.sig_hint = None;
        }
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// After a `Commit`/`EditCommitRequested` reduce, keep the [`committed_cell`](Self::committed_cell)
    /// tag consistent with the reducer's `committed`. When an edit that was in progress
    /// (`was_editing`) just committed (now no longer Editing), the reducer set `committed` to the
    /// **active cell's** just-committed content — so re-tag it to `(active_sheet, active)`. In the
    /// click-away path `selection.active` is still the edited cell here (the selection moves only
    /// afterwards), so the tag names the right cell (data-corruption guard).
    fn note_commit(&mut self, was_editing: bool) {
        if was_editing && self.data_row.mode() != FieldMode::Editing {
            self.committed_cell = Some((self.active_sheet, self.selection.active));
        }
    }

    /// A caret-intent signal ended quick-edit (`functional_spec.md §5.3`): a mouse-down in the
    /// field, Home/End, or a modified arrow. For the remainder of this edit, arrows control the text
    /// caret, not the active cell. Idempotent; re-pushes the grid's edit state so its copy tracks.
    fn leave_quick_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quick_edit {
            return;
        }
        self.quick_edit = false;
        self.refresh_edit_grid_state(window, cx);
    }

    /// The data-row edit-key handler for a live edit (`functional_spec.md §5.2–5.3`), factored out
    /// so it is unit-testable without routing a keystroke through the nested input. Driven by the
    /// keystroke interceptor registered in [`ChromeView::new`] (which sees the key before the
    /// gpui-component `Input`'s caret action bindings). Returns whether the key was **consumed**
    /// (the caller must then `stop_propagation` so the input doesn't also act on it); `false` lets
    /// the key fall through to the input (caret op).
    ///
    /// - Tab / Shift+Tab always commit + move right / left (unchanged, quick-edit or not).
    /// - In quick-edit, an **unmodified** arrow commits + moves the active cell in that direction.
    /// - A caret-intent modified arrow (Shift/Cmd/Ctrl/Alt — see [`caret_intent_modifiers`]) or
    ///   Home/End signals caret intent: it leaves quick-edit and falls through to the caret, and
    ///   (for a modified arrow) does **not** move the active cell.
    fn handle_data_row_edit_key(
        &mut self,
        key: &str,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.data_mode() != FieldMode::Editing {
            return false;
        }
        // When the completion list is open it preempts navigation/accept/dismiss keys
        // (`functional_spec.md §1`); every other key falls through to update/dismiss the list via
        // the normal `Change` recompute.
        if self.autocomplete.is_some() {
            match key {
                "down" => {
                    self.autocomplete_nav(true, cx);
                    return true;
                }
                "up" => {
                    self.autocomplete_nav(false, cx);
                    return true;
                }
                "enter" | "tab" => {
                    self.autocomplete_accept(window, cx);
                    return true;
                }
                "escape" => {
                    self.autocomplete_dismiss(window, cx);
                    return true;
                }
                _ => {}
            }
        }
        if key == "tab" {
            let dir = if modifiers.shift {
                Direction::Left
            } else {
                Direction::Right
            };
            self.commit_and_move(dir, window, cx);
            return true;
        }
        if !self.quick_edit {
            return false;
        }
        match key {
            "left" | "right" | "up" | "down" => {
                if caret_intent_modifiers(&modifiers) {
                    // Modified arrow = caret/selection op: leave quick-edit, do NOT move the active
                    // cell, and let the key reach the input.
                    self.leave_quick_edit(window, cx);
                    false
                } else {
                    let dir = match key {
                        "left" => Direction::Left,
                        "right" => Direction::Right,
                        "up" => Direction::Up,
                        _ => Direction::Down,
                    };
                    self.commit_and_move(dir, window, cx);
                    true
                }
            }
            "home" | "end" => {
                // Explicit caret positioning: leave quick-edit; the input moves the caret.
                self.leave_quick_edit(window, cx);
                false
            }
            _ => false,
        }
    }

    /// The in-cell overlay input emitted an event: `Change` drives the shared edit (mirrored to the
    /// data row); `PressEnter` commits + moves; `Focus` makes the in-cell editor the driver.
    fn on_incell_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.edit.is_syncing() {
                    return; // the echo of our own push into this editor — ignore (guard the loop)
                }
                self.cap_error_external = None;
                let text = self.edit.in_cell().read(cx).value().to_string();
                // Push into the data-row editor (events suppressed) and drive the shared reducer.
                self.edit.set_syncing(true);
                self.content_input.update(cx, |input, cx| {
                    input.set_value(text.clone(), window, cx);
                });
                self.edit.set_syncing(false);
                let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                self.apply_data_effects(effects, window, cx);
                self.recompute_autocomplete(cx);
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                self.commit_and_move(
                    if *shift {
                        Direction::Up
                    } else {
                        Direction::Down
                    },
                    window,
                    cx,
                );
            }
            InputEvent::Focus => {
                self.edit.set_origin(EditOrigin::InCell);
                // The active editor drives which side shows the cap popover — re-push so the grid
                // reflects the flip (avoids a transient double popover, review #4).
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::Blur => {}
        }
    }

    /// Mirrors the data-row editor's current text into the in-cell editor (events suppressed) when
    /// the overlay is open — the other half of the two-editor sync.
    fn mirror_to_in_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edit.is_open() || self.edit.is_syncing() {
            return;
        }
        let text = self.content_input.read(cx).value().to_string();
        self.edit.set_syncing(true);
        self.edit.in_cell().update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
        self.edit.set_syncing(false);
    }

    /// Pushes the current edit's grid-facing state (live mirror, in-cell overlay cell, in-cell cap
    /// message) to the grid. Called after every edit transition
    /// (`components/edit_controller.md §4.3–4.4`). The overlay is opened/closed explicitly by the
    /// edit entry/exit methods (not auto-closed here), so the in-cell editor can stay open while an
    /// initial content fetch is still in flight (empty-with-spinner, `§Grid integration`).
    fn refresh_edit_grid_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.data_row.mode() == FieldMode::Editing;
        let mirror = editing.then(|| {
            let text: SharedString = self.content_input.read(cx).value().to_string().into();
            (self.active_sheet, self.selection.active, text)
        });
        let in_cell = self.edit.open_cell();
        let cap = (self.edit.origin() == EditOrigin::InCell)
            .then(|| self.cap_error_message())
            .flatten()
            .map(SharedString::from);
        // Quick-edit is meaningful only while the edit is live; gate on `editing` so the grid's copy
        // auto-clears the instant the edit ends (`functional_spec.md §5`).
        let quick_edit = editing && self.quick_edit;
        // The autocomplete list + signature hint render under the in-cell overlay only when it is
        // the driving editor (the data row renders its own — §1). Cleared otherwise so a data-row
        // list never leaks into the grid.
        let in_cell_driving = self.edit.origin() == EditOrigin::InCell;
        let autocomplete = in_cell_driving
            .then(|| self.autocomplete_display())
            .flatten();
        let sig_hint = in_cell_driving
            .then(|| self.sig_hint.map(SharedString::from))
            .flatten();
        let nonempty = mirror.is_some() || in_cell.is_some();
        // Skip an all-`None` clear when nothing was shown (idle selection moves would otherwise
        // re-push every keystroke); always push when something is/was shown so the clear lands.
        if !nonempty && !self.edit_state_shown {
            return;
        }
        self.edit_state_shown = nonempty;
        self.grid.emit(
            ChromeGridRequest::EditState {
                mirror,
                in_cell,
                cap,
                quick_edit,
                autocomplete,
                sig_hint,
            },
            window,
            cx,
        );
    }

    // ---- Function autocomplete + signature hints (`gaps_closing_7_15 §1`) ------------------

    /// The `InputState` currently driving the shared pending edit (the editor the user types in),
    /// so autocomplete reads the right caret.
    fn driving_input(&self) -> &Entity<InputState> {
        match self.edit.origin() {
            EditOrigin::DataRow => &self.content_input,
            EditOrigin::InCell => self.edit.in_cell(),
        }
    }

    /// Recomputes the autocomplete list + signature hint from the **driving** editor's text and
    /// caret after a keystroke (`architecture.md §1.3`). Sets both
    /// [`autocomplete`](Self::autocomplete) and [`sig_hint`](Self::sig_hint); the caller pushes
    /// grid state + notifies. A visible cap error takes precedence — both are cleared while it
    /// shows.
    fn recompute_autocomplete(&mut self, cx: &mut Context<Self>) {
        if self.cap_error_visible() {
            self.autocomplete = None;
            self.sig_hint = None;
            return;
        }
        let input = self.driving_input().read(cx);
        let text = input.value().to_string();
        let caret = input.cursor();

        self.autocomplete = match functions::fn_edit_context(&text, caret) {
            Some(ctx) => {
                let matches = functions::complete(&ctx.prefix);
                if matches.is_empty() {
                    None
                } else {
                    // Preserve the highlight across a same-token refresh (typing more of the same
                    // name); reset to the top when the token moved or the list shrank past it.
                    let highlight = match &self.autocomplete {
                        Some(prev) if prev.token_start == ctx.token_start => {
                            prev.highlight.min(matches.len() - 1)
                        }
                        _ => 0,
                    };
                    Some(Autocomplete {
                        matches,
                        highlight,
                        token_start: ctx.token_start,
                    })
                }
            }
            None => None,
        };

        self.sig_hint = functions::enclosing_fn_name(&text, caret)
            .and_then(functions::signature)
            .map(|s| s.template);
    }

    /// Recompute the list/hint and re-push grid state + notify (the full per-keystroke effect,
    /// used by the deferred caret-move recompute below).
    fn recompute_autocomplete_and_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_autocomplete(cx);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Schedule a recompute for *after* a caret-only key (←/→/Home/End) has moved the caret. The
    /// pinned `InputState` fires no event on a pure caret move, and the intercept/`capture_key_down`
    /// seams run *before* the input moves the caret — so a synchronous recompute would read the
    /// stale (pre-move) caret. Deferring to the next cycle reads the moved caret, so the list
    /// updates/dismisses and the signature hint tracks the caret (`functional_spec.md §1`).
    fn schedule_autocomplete_recompute(&self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |this, cx| {
                    this.recompute_autocomplete_and_refresh(window, cx);
                });
            }
        });
    }

    /// The autocomplete list as grid-renderable display state (all matches; the render caps the
    /// visible height + scrolls per `functional_spec.md §1`), or `None` when the list is closed.
    fn autocomplete_display(&self) -> Option<AutocompleteDisplay> {
        let ac = self.autocomplete.as_ref()?;
        let rows = ac
            .matches
            .iter()
            .map(|f| AutocompleteRow {
                name: f.name.into(),
                template: f.template.into(),
            })
            .collect();
        Some(AutocompleteDisplay {
            rows,
            highlight: ac.highlight,
        })
    }

    /// Move the highlighted row down/up, clamped (no wrap — `functional_spec.md §1`).
    pub fn autocomplete_nav(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some(ac) = self.autocomplete.as_mut() {
            let last = ac.matches.len().saturating_sub(1);
            ac.highlight = if down {
                (ac.highlight + 1).min(last)
            } else {
                ac.highlight.saturating_sub(1)
            };
            cx.notify();
        }
    }

    /// Close the list only — the edit continues, nothing is committed or reverted
    /// (`functional_spec.md §1`, Esc). The signature hint stays as-is.
    pub fn autocomplete_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete.take().is_some() {
            self.refresh_edit_grid_state(window, cx);
            cx.notify();
        }
    }

    /// Accept the highlighted completion (Tab / Enter / mouse click on the highlighted row).
    pub fn autocomplete_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.accept_autocomplete(window, cx);
    }

    /// A caret-only key (←/→/Home/End) moved the caret in the **in-cell** overlay (routed from the
    /// grid, which sees these keys but does not own the list state). Recompute the list/hint after
    /// the move (`functional_spec.md §1`), mirroring the data-row intercept path.
    pub fn autocomplete_caret_moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_autocomplete_recompute(window, cx);
    }

    /// Accept the completion at `index` (a mouse click on a specific in-cell list row).
    pub fn autocomplete_accept_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ac) = self.autocomplete.as_mut() {
            if index < ac.matches.len() {
                ac.highlight = index;
            }
        }
        self.accept_autocomplete(window, cx);
    }

    /// Replace the typed prefix with `NAME(` and place the caret just after the paren (D1.2), then
    /// show the accepted function's signature hint. Drives both editors + the reducer so the
    /// pending edit stays consistent (`architecture.md §1.5`).
    fn accept_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ac) = self.autocomplete.take() else {
            return;
        };
        let Some(sig) = ac.matches.get(ac.highlight).copied() else {
            return;
        };
        let origin = self.edit.origin();
        let text = self.driving_input().read(cx).value().to_string();
        let caret = self.driving_input().read(cx).cursor();
        // Re-derive the token span from the CURRENT text + caret (never the stored `token_start`):
        // the caret may have moved within the token since the list opened, so accepting must replace
        // the WHOLE identifier token, not just `[token_start, caret)`. Without this, `=sum` with the
        // caret moved to offset 2 would splice to `=SUM(um` instead of `=SUM(`. If the caret is no
        // longer in a name token, there is nothing to complete.
        let Some(ctx) = functions::fn_edit_context(&text, caret) else {
            self.refresh_edit_grid_state(window, cx);
            cx.notify();
            return;
        };
        let token_start = ctx.token_start;
        // Extend right over the remaining identifier chars (letters/digits/`.`/`_`) to the token end.
        let bytes = text.as_bytes();
        let mut token_end = caret;
        while token_end < bytes.len()
            && (bytes[token_end].is_ascii_alphanumeric()
                || bytes[token_end] == b'.'
                || bytes[token_end] == b'_')
        {
            token_end += 1;
        }
        let insertion = format!("{}(", sig.name);
        let new_caret = token_start + insertion.len();
        let mut new_text = String::with_capacity(text.len() + insertion.len());
        new_text.push_str(&text[..token_start]);
        new_text.push_str(&insertion);
        new_text.push_str(&text[token_end..]);

        // Drive the shared reducer with the new canonical text (keeps cap-validation/commit
        // consistent), exactly as the programmatic-text paths do.
        let effects = self.data_row.reduce(DataRowEvent::Edited {
            text: new_text.clone(),
        });
        // Position the caret just after the inserted `(` (single-line editor → line 0, char col).
        let char_col = new_text[..new_caret].chars().count() as u32;
        self.set_driving_text_and_caret(origin, &new_text, char_col, window, cx);
        self.apply_data_effects(effects, window, cx);
        // Mirror into the other editor (its caret at end is fine — it is not the one being typed in).
        self.mirror_other_editor(origin, &new_text, window, cx);

        self.sig_hint = Some(sig.template);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Sets the driving editor's text (events suppressed) and moves its caret to `char_col`.
    fn set_driving_text_and_caret(
        &mut self,
        origin: EditOrigin,
        text: &str,
        char_col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match origin {
            EditOrigin::DataRow => self.content_input.clone(),
            EditOrigin::InCell => self.edit.in_cell().clone(),
        };
        input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx);
            input.set_cursor_position(Position::new(0, char_col), window, cx);
        });
    }

    /// Pushes `text` into the editor that is **not** driving (its caret lands at end), under the
    /// sync guard so the echo is ignored.
    fn mirror_other_editor(
        &mut self,
        origin: EditOrigin,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit.set_syncing(true);
        match origin {
            EditOrigin::DataRow => {
                if self.edit.is_open() {
                    self.edit.in_cell().update(cx, |input, cx| {
                        input.set_value(text.to_string(), window, cx);
                    });
                }
            }
            EditOrigin::InCell => {
                self.content_input.update(cx, |input, cx| {
                    input.set_value(text.to_string(), window, cx);
                });
            }
        }
        self.edit.set_syncing(false);
    }

    /// Folds a worker event into the chrome (Phase 11 calls this from the event task; tests
    /// call it directly).
    pub fn on_worker_event(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkerEvent::CellContent { req_id, raw } => {
                let was_awaiting = self.data_row.is_awaiting();
                self.data_row
                    .reduce(DataRowEvent::ContentFetched { req_id, raw });
                // Sync the widget only when the reducer populated the field (fresh reply,
                // still Idle) — never mid-edit, so a late reply can't reset the caret.
                if self.data_row.mode() == FieldMode::Idle {
                    self.sync_input_from_reducer(window, cx);
                    // A reply that actually landed (cleared `awaiting`) is the current active
                    // cell's content — record which cell `committed` now belongs to, and, if the
                    // in-cell editor opened before its content arrived (empty-with-spinner),
                    // promote it to an edit seeded with it (`§Grid integration`; review #3).
                    let landed = was_awaiting && !self.data_row.is_awaiting();
                    if landed {
                        self.committed_cell = Some((self.active_sheet, self.selection.active));
                        if self.edit.is_open() {
                            let text = self.content_input.read(cx).value().to_string();
                            let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                            self.apply_data_effects(effects, window, cx);
                            self.mirror_to_in_cell(window, cx);
                            self.refresh_edit_grid_state(window, cx);
                        }
                    }
                }
                cx.notify();
            }
            WorkerEvent::EvalStarted => {
                let effects = self.eval.reduce(EvalEvent::Started);
                self.apply_eval_effects(effects, cx);
            }
            WorkerEvent::EvalFinished => {
                self.eval.reduce(EvalEvent::Finished);
                cx.notify();
            }
            WorkerEvent::Loaded { sheets } | WorkerEvent::SheetsChanged { sheets } => {
                self.merge_sheet_metas(&sheets);
                cx.notify();
            }
            WorkerEvent::EditRejected {
                reason: EditRejectedReason::InputCap(rejection),
            } => {
                self.cap_error_external = Some(rejection);
                cx.notify();
            }
            // Only honor results while the bar is open (a late reply after close is dropped).
            WorkerEvent::FindResults { matches } if self.find_open => {
                self.matches = matches;
                self.match_idx = self.first_match_from_selection();
                self.select_current_match(window, cx);
                cx.notify();
            }
            WorkerEvent::ReplacedCount { n } => {
                if self.pending_replace_all {
                    self.pending_replace_all = false;
                    self.replaced_notice = Some(n);
                }
                // Re-scan so the match set + counter reflect the post-replace state and the cursor
                // advances past a (now-changed) cell (`functional_spec.md §4.4`).
                if self.find_open {
                    self.recompute_matches(cx);
                }
                cx.notify();
            }
            // Keep only the reply for the latest request — a superseded selection bumped
            // `stats_seq`, so an older reply (or one after a collapse to a single cell) falls
            // through the guard to the `_` arm and is dropped.
            WorkerEvent::SelectionStats { req_id, stats } if req_id == self.stats_seq => {
                self.selection_stats = Some(stats);
                cx.notify();
            }
            // Published/Saved/SaveFailed/StyleCacheUpdated/other EditRejected reasons /
            // degraded are the window's concern (Phase 11 dirty state + modals).
            _ => {}
        }
    }

    /// Mirrors the reducer's current text into the content widget (suppressing the widget's
    /// change event — `InputState::set_value` sets `emit_events = false`).
    fn sync_input_from_reducer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.data_row.text().to_string();
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// The content input emitted an event: typing enters Editing; Enter commits (+ moves the
    /// active cell); Shift+Enter commits + moves up.
    fn on_content_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.edit.is_syncing() {
                    return; // the echo of an in-cell → data-row push — ignore (guard the loop)
                }
                // A keystroke dismisses the cap-error popover (`functional_spec.md §4.2`): the
                // reducer clears its own rejection in `Edited`; the worker backstop is cleared
                // here so both sources dismiss on the next keystroke.
                self.cap_error_external = None;
                let text = self.content_input.read(cx).value().to_string();
                let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                self.apply_data_effects(effects, window, cx);
                self.mirror_to_in_cell(window, cx);
                self.recompute_autocomplete(cx);
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                // Enter commits + moves down, Shift+Enter up (the reducer's Commit hardcodes Down).
                self.commit_and_move(
                    if *shift {
                        Direction::Up
                    } else {
                        Direction::Down
                    },
                    window,
                    cx,
                );
            }
            InputEvent::Blur => {
                // Focus leaving the field dismisses the cap-error popover
                // (`functional_spec.md §4.2`). The reducer clears its own rejection on the
                // next edit/escape; the worker backstop is cleared here.
                if self.cap_error_external.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::Focus => {
                self.edit.set_origin(EditOrigin::DataRow);
                // Re-push so the in-cell cap popover (grid-side) clears when focus flips to the data
                // row and the data-row popover takes over (avoids a transient double, review #4).
                self.refresh_edit_grid_state(window, cx);
            }
        }
    }

    /// Performs the reducer's data-row effects: fetch/commit as client commands, move/focus as
    /// grid requests, and arm the 250 ms fetch-spinner timer.
    fn apply_data_effects(
        &mut self,
        effects: Vec<DataRowEffect>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for effect in effects {
            match effect {
                DataRowEffect::Fetch { req_id } => {
                    self.client.send(Command::GetCellContent {
                        sheet: self.active_sheet,
                        cell: self.selection.active,
                        req_id,
                    });
                    self.arm_fetch_timer(req_id, cx);
                }
                DataRowEffect::Commit { input } => {
                    self.client.send(Command::SetCellInput {
                        sheet: self.active_sheet,
                        cell: self.selection.active,
                        input,
                    });
                }
                DataRowEffect::MoveActive(motion) => {
                    self.grid
                        .emit(ChromeGridRequest::MoveActive(motion), window, cx);
                }
                DataRowEffect::FocusGrid => {
                    self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
                }
                // The danger border + fetch spinner render directly from the reducer's state.
                DataRowEffect::ShowCapError | DataRowEffect::SetSpinner(_) => {}
            }
        }
    }

    /// Arms the 250 ms content-fetch spinner timer for `req_id` (`ui_design.md §3.2`).
    fn arm_fetch_timer(&mut self, req_id: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SPINNER_DELAY).await;
            this.update(cx, |this, cx| {
                this.data_row.reduce(DataRowEvent::FetchTimeout { req_id });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Performs the evaluating-spinner effects, arming the 250 ms timer when asked.
    fn apply_eval_effects(&mut self, effects: Vec<EvalEffect>, cx: &mut Context<Self>) {
        for effect in effects {
            if let EvalEffect::ArmTimer { epoch } = effect {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SPINNER_DELAY).await;
                    this.update(cx, |this, cx| {
                        this.eval.reduce(EvalEvent::Timeout { epoch });
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    // ---- Action row: formatting -----------------------------------------------------------

    /// Toggles a character style over the selection; commits any pending edit first (the same
    /// rule as clicking another cell). A cap-rejected pending edit blocks the toggle.
    pub fn toggle_style(&mut self, attr: StyleAttr, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the format, keeping the field editing
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr,
        });
    }

    /// Applies a fill colour (`Some`) or clears it (`None`) over the selection; commits any
    /// pending edit first, and closes the fill popover.
    pub fn apply_fill(&mut self, fill: Option<Rgb>, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_open = false;
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr: StyleAttr::Fill(fill),
        });
        cx.notify();
    }

    /// Commits a pending data-row edit if any. Returns whether the field is now committable
    /// (`false` = a cap-rejected edit is still open).
    fn commit_pending_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.data_row.mode() == FieldMode::Editing {
            let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
            self.apply_data_effects(effects, window, cx);
            self.note_commit(true);
            if self.data_row.mode() != FieldMode::Editing {
                self.edit.close();
            }
            self.refresh_edit_grid_state(window, cx);
        }
        self.data_row.mode() != FieldMode::Editing
    }

    fn toggle_fill_popover(&mut self, cx: &mut Context<Self>) {
        self.fill_open = !self.fill_open;
        cx.notify();
    }

    fn on_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_fill(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Sends one `SetStylePath` over the selection after committing any pending edit (the same
    /// rule as clicking another cell). Fire-and-forget: a cap-rejected pending edit blocks it, and
    /// the worker logs any engine rejection (the UI only ever sends valid paths/values).
    fn apply_style_path(
        &mut self,
        path: StylePath,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // No mutating control may dispatch while degraded/read-only (`functional_spec.md §6`) — a
        // backstop to the disabled buttons, covering a swatch/entry clicked in a popover that was
        // open at the instant of degradation (also closed by `set_degraded`).
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStylePath {
            sheet: self.active_sheet,
            range: self.selection.range(),
            path,
            value,
        });
        cx.notify();
    }

    /// Applies a text colour (`Some`) or clears it to Automatic (`None`, value `""`), closing the
    /// text-color popover.
    pub fn apply_text_color(
        &mut self,
        color: Option<Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text_color_open = false;
        let value = match color {
            Some(rgb) => format!("#{:06X}", rgb.to_hex()),
            None => String::new(),
        };
        self.apply_style_path(StylePath::FontColor, value, window, cx);
    }

    /// Applies a horizontal alignment; re-pressing the active one clears the explicit alignment
    /// back to the type default (value `"general"` — clears horizontal only, never wrap/vertical).
    pub fn apply_alignment(&mut self, align: Align, window: &mut Window, cx: &mut Context<Self>) {
        let value = if self.align_active(align) {
            "general".to_string()
        } else {
            match align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
            }
            .to_string()
        };
        self.apply_style_path(StylePath::AlignHorizontal, value, window, cx);
    }

    /// Applies a vertical alignment (top/center/bottom) over the selection — a plain radio-style
    /// set (`functional_spec.md §1.3`, `architecture.md §2`). Unlike horizontal align there is no
    /// re-press-to-clear: IronCalc's vertical default is `bottom` and the grid's default placement
    /// is also bottom (decision C — Excel-faithful), so there is no independent "unset" value to
    /// clear back to; the group is purely one-of-N (top / center / bottom).
    pub fn apply_valign(&mut self, valign: VAlign, window: &mut Window, cx: &mut Context<Self>) {
        let value = match valign {
            VAlign::Top => "top",
            VAlign::Center => "center",
            VAlign::Bottom => "bottom",
        }
        .to_string();
        self.apply_style_path(StylePath::AlignVertical, value, window, cx);
    }

    /// Applies a number-format code over the selection, closing the number-format dropdown.
    pub fn apply_num_fmt(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.num_fmt_open = false;
        self.num_fmt_more_open = false;
        self.apply_style_path(StylePath::NumFmt, code.to_string(), window, cx);
    }

    /// Adjusts the active cell's number of decimal places by `delta` (`+1` / `-1`). Computed
    /// UI-side from the cached format string and the active cell's kind/display: a real numeric
    /// format is rewritten directly, and a *numeric* General cell (`200000`) is switched to a
    /// `0.0…` format (BUG 3); a no-op (`adjust_decimals_cell` → `None`) does nothing (the buttons
    /// also render disabled in that case).
    pub fn bump_decimals(&mut self, delta: i8, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.active_num_fmt.clone();
        let (numeric, displayed) = self.active_numeric_decimals();
        if let Some(new_code) = current
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
        {
            self.apply_num_fmt(&new_code, window, cx);
        }
    }

    /// Whether the thousands-separator toggle is enabled: not degraded, and the active cell's
    /// format can be safely re-grouped (`toggle_thousands` — a single-section numeric code with an
    /// integer `0` placeholder). General/Text/Date/Time/Scientific/multi-section customs disable it.
    pub fn toggle_thousands_enabled(&self) -> bool {
        if self.degraded {
            return false;
        }
        self.active_num_fmt
            .as_deref()
            .and_then(toggle_thousands)
            .is_some()
    }

    /// Whether the active cell's format currently carries a thousands separator (the toggle renders
    /// pressed). Only true when the toggle is also actionable, so a disabled button never shows as
    /// selected.
    pub fn thousands_active(&self) -> bool {
        self.toggle_thousands_enabled()
            && self
                .active_num_fmt
                .as_deref()
                .is_some_and(|c| c.contains("#,##0"))
    }

    /// Toggles the thousands separator on the active cell's number format, applying the rewritten
    /// code over the selection (one undo step). A no-op when the format can't be toggled (the
    /// button also renders disabled in that case).
    pub fn toggle_thousands_separator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(new_code) = self.active_num_fmt.as_deref().and_then(toggle_thousands) {
            self.apply_num_fmt(&new_code, window, cx);
        }
    }

    fn toggle_text_color_popover(&mut self, cx: &mut Context<Self>) {
        self.text_color_open = !self.text_color_open;
        cx.notify();
    }

    fn toggle_num_fmt_popover(&mut self, cx: &mut Context<Self>) {
        self.num_fmt_open = !self.num_fmt_open;
        // Basics-first on open — except when the active cell's format lives only under "More ▸":
        // then land directly on the grouped view so the current format is visible/highlighted
        // (`architecture.md §10`, "open onto the matched group"). Always reset when closing.
        self.num_fmt_more_open =
            self.num_fmt_open && is_more_only_num_fmt(&self.num_fmt_active_code());
        cx.notify();
    }

    /// The active cell's number-format code, normalized so `general` (which the engine may echo as
    /// `"General"`) compares lowercase against the preset codes. `None` → the default `"general"`.
    fn num_fmt_active_code(&self) -> String {
        let c = self.active_num_fmt.as_deref().unwrap_or("general");
        if c.eq_ignore_ascii_case("general") {
            "general".to_string()
        } else {
            c.to_string()
        }
    }

    // ---- Action row: insert chart (P17) ---------------------------------------------------

    fn toggle_chart_menu(&mut self, cx: &mut Context<Self>) {
        self.chart_menu_open = !self.chart_menu_open;
        cx.notify();
    }

    /// Inserts a **near-empty authored chart** of `kind` onto the active sheet, anchored at the
    /// active cell (`ui_design §3.1`). This is a **mutating action-row control**, so it follows the
    /// same contract as every sibling (toggle style, fill, text color, decimals, font, borders): it
    /// closes the menu, then **commits any pending in-cell edit first and bails if the commit is
    /// blocked** (a cap-rejected edit stays editing), so the worker's subsequent publish + grid
    /// refresh can't clobber a dangling uncommitted cell edit. Degraded-guarded (a backstop to the
    /// disabled trigger). Fire-and-forget: the worker holds the authored chart + republishes it.
    pub fn insert_chart(
        &mut self,
        kind: ChartInsertKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chart_menu_open = false;
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the insert, keeping the field editing
        }
        // Post-v1 Batch 3, item 8: seed the new chart's data range from the current selection when it
        // is a **real range** (more than one cell) — the worker binds it at creation, so the chart is
        // born as a live chart of the chosen type, no follow-up "Use selection" click. A single-cell
        // (or trivial) selection passes `None`, keeping the near-empty placeholder behavior. Reuses
        // the P19 `SetChartRange` binding (`series_refs_from_block`), now on the freshly-inserted id.
        let data = (!self.selection.is_single()).then(|| self.selection.range());
        self.client.send(Command::InsertChart {
            sheet: self.active_sheet,
            kind,
            anchor: self.default_chart_anchor(),
            data,
        });
        cx.notify();
    }

    /// A default chart placement: the [`CHART_INSERT_COLS`]×[`CHART_INSERT_ROWS`] rectangle from the
    /// active cell, clamped to the sheet edge so the anchor stays on-sheet.
    fn default_chart_anchor(&self) -> ChartAnchor {
        let active = self.selection.active;
        let to_col = active
            .col
            .saturating_add(CHART_INSERT_COLS)
            .min(limits::MAX_COLS - 1);
        let to_row = active
            .row
            .saturating_add(CHART_INSERT_ROWS)
            .min(limits::MAX_ROWS - 1);
        ChartAnchor::new(
            AnchorCell::new(active.col, active.row),
            AnchorCell::new(to_col, to_row),
        )
    }

    /// Whether the chart-insert menu is open (test/render introspection).
    pub fn chart_menu_open(&self) -> bool {
        self.chart_menu_open
    }

    // ---- Chart edit panel (P19 skeleton + P20 chrome) -------------------------------------

    /// Open (or re-point / reconcile) the right-docked chart **edit panel** on `panel`'s chart
    /// (`ui_design §4`). The window calls this when a chart is selected or freshly inserted, and again
    /// on each republish to reconcile the shown state with the worker's snapshot. The text inputs
    /// (title + axis titles) are seeded **only when the panel's chart id changes** — never on a live
    /// republish of the same chart — so a republish (e.g. a source-cell edit re-resolving the chart)
    /// never clobbers an in-progress panel edit.
    pub fn open_chart_panel(
        &mut self,
        panel: ChartPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The chart panel and the CF sidebar share the right dock — opening one closes the other
        // (`components/cf_sidebar.md §2`). A no-op when the CF sidebar is already closed (the common
        // case: this also runs to reconcile an already-open chart panel on republish).
        self.close_cond_fmt(cx);
        let new_chart = self.chart_panel.as_ref().map(|p| p.id) != Some(panel.id);
        if new_chart {
            self.seed_chart_input(
                &self.chart_title_input.clone(),
                panel.title.clone(),
                window,
                cx,
            );
            self.seed_chart_input(
                &self.chart_cat_axis_input.clone(),
                panel.cat_axis_title.clone(),
                window,
                cx,
            );
            self.seed_chart_input(
                &self.chart_val_axis_input.clone(),
                panel.val_axis_title.clone(),
                window,
                cx,
            );
        }
        self.chart_panel = Some(panel);
        cx.notify();
    }

    /// Seed a chart-panel text input with the chart's current value (`set_value` suppresses the
    /// widget's change event, so seeding never triggers a spurious commit).
    fn seed_chart_input(
        &self,
        input: &Entity<InputState>,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |i, cx| {
            i.set_value(value.unwrap_or_default(), window, cx)
        });
    }

    /// Close the chart edit panel (its × button, the chart's deletion, or a degrade).
    pub fn close_chart_panel(&mut self, cx: &mut Context<Self>) {
        if self.chart_panel.take().is_some() {
            cx.notify();
        }
    }

    /// The chart the edit panel is currently shaping, if any (window introspection: refresh / close).
    pub fn chart_panel_target(&self) -> Option<ChartId> {
        self.chart_panel.as_ref().map(|p| p.id)
    }

    // ---- Conditional-formatting sidebar (P4, `components/cf_sidebar.md`) -------------------

    /// Whether the CF sidebar is open (the action-bar `split` button's `selected` state + the render
    /// gate + tests).
    pub fn cond_fmt_open(&self) -> bool {
        self.cond_fmt.is_some()
    }

    /// The sheet the open CF sidebar targets, if any (window introspection: the `CondFmtUpdated`
    /// refresh gate).
    pub fn cond_fmt_sheet(&self) -> Option<SheetId> {
        self.cond_fmt.as_ref().map(|p| p.sheet)
    }

    /// Toggle the right-docked CF sidebar (the action-bar `split` button): open it in List mode if
    /// closed, else close it.
    pub fn toggle_cond_fmt_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.cond_fmt.is_some() {
            self.close_cond_fmt(cx);
        } else {
            self.open_cond_fmt(cx);
        }
    }

    /// Open the CF sidebar in List mode for the active sheet, building its rows from the published
    /// rules. Closes the chart panel first (they share the right dock — `components/cf_sidebar.md §2`).
    fn open_cond_fmt(&mut self, cx: &mut Context<Self>) {
        self.close_chart_panel(cx);
        let sheet = self.active_sheet;
        let rows = self.client.cond_fmt_rules(sheet);
        self.cond_fmt = Some(CondFmtPanel {
            sheet,
            rows,
            editor: None,
        });
        cx.notify();
    }

    /// Close the CF sidebar (its ×, the action-bar toggle, or a degrade).
    pub fn close_cond_fmt(&mut self, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        if self.cond_fmt.take().is_some() {
            cx.notify();
        }
    }

    /// Rebuild the open CF sidebar's rows from the latest published rules for its own sheet — the
    /// `WorkerEvent::CondFmtUpdated` refresh (and, from P5, after a CF command). A no-op when the
    /// sidebar is closed.
    pub fn refresh_cond_fmt(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.cond_fmt.as_ref().map(|p| p.sheet) else {
            return;
        };
        let rows = self.client.cond_fmt_rules(sheet);
        // A refresh that lands while a Save is in flight is that save's success signal — the rule
        // was accepted (an engine `Err` would instead route to `show_cf_editor_error`), so return
        // to List mode (`components/cf_sidebar.md §4`).
        let save_landed = self
            .cond_fmt
            .as_ref()
            .and_then(|p| p.editor.as_ref())
            .is_some_and(|e| e.pending_save);
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.rows = rows;
            if save_landed {
                panel.editor = None;
            }
            cx.notify();
        }
        if save_landed {
            self.cf_menu_open = None;
        }
    }

    /// Re-scope the open CF sidebar to the newly active sheet (rebuild its rows), if open — the
    /// sheet-switch path (`components/cf_sidebar.md §4/§9`: the sidebar does **not** close on a sheet
    /// change, it re-scopes). A no-op when the sidebar is closed. (P6 also cancels any open editor here.)
    fn rescope_cond_fmt_if_open(&mut self, cx: &mut Context<Self>) {
        if self.cond_fmt.is_none() {
            return;
        }
        let sheet = self.active_sheet;
        let rows = self.client.cond_fmt_rules(sheet);
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.sheet = sheet;
            panel.rows = rows;
            // A sheet switch cancels any open editor (it was scoped to the old sheet) —
            // `components/cf_sidebar.md §4`.
            panel.editor = None;
            cx.notify();
        }
    }

    /// Raise the priority of the CF rule at storage `index` — the row's ▲ control
    /// (`components/cf_sidebar.md §5`). Fire-and-forget; the worker republishes the reordered list
    /// and the sidebar refreshes via `CondFmtUpdated` → [`refresh_cond_fmt`](Self::refresh_cond_fmt).
    /// A no-op when the sidebar is closed (no target sheet). `index` is the rule's stable storage
    /// index, not its display position.
    pub fn raise_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client
                .send(Command::RaiseCondFmtPriority { sheet, index });
        }
    }

    /// Lower the priority of the CF rule at storage `index` — the row's ▼ control (the mirror of
    /// [`raise_cf_rule`](Self::raise_cf_rule)).
    pub fn lower_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client
                .send(Command::LowerCondFmtPriority { sheet, index });
        }
    }

    /// Delete the CF rule at storage `index` — the row's 🗑 control. Enabled for every row,
    /// including deferred-family/Badge rows (which are non-editable but deletable —
    /// `functional_spec.md §9`). A no-op when the sidebar is closed.
    pub fn delete_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client.send(Command::DeleteCondFmt { sheet, index });
        }
    }

    /// Whether the CF rule **editor** is open (Editor mode) — introspection for the window's engine
    /// error routing + tests.
    pub fn cf_editor_open(&self) -> bool {
        self.cond_fmt.as_ref().is_some_and(|p| p.editor.is_some())
    }

    /// Open the rule editor (List → Editor mode) for `edit_index` (`Some` = edit an existing rule,
    /// `None` = add). Add mode seeds a fresh [`CfEditorState`] + the Applies-to range from the
    /// current selection; edit mode seeds both the state and the text inputs from the row's `spec`
    /// (`Some` for an authorable highlight rule; a color-scale / non-editable row is a no-op — its
    /// edit control is disabled in P6). `components/cf_sidebar.md §4`.
    fn open_cf_editor(
        &mut self,
        edit_index: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.cond_fmt.as_ref() else {
            return;
        };
        // Assemble the editor state + the operand/formula seeds without holding a borrow of `panel`.
        let (state, range, op1, op2, formula) = match edit_index {
            None => (
                CfEditorState::new(None),
                self.selection.to_a1(),
                String::new(),
                String::new(),
                String::new(),
            ),
            Some(index) => {
                let Some(spec) = panel
                    .rows
                    .iter()
                    .find(|r| r.index == index)
                    .and_then(|r| r.spec.clone())
                else {
                    return; // no such row / a deferred-family (Badge) row: not editable.
                };
                let range = panel
                    .rows
                    .iter()
                    .find(|r| r.index == index)
                    .map(|r| r.range.clone())
                    .unwrap_or_default();
                let Some((state, op1, op2, formula)) = cf_state_from_spec(index, &spec) else {
                    return; // a color-scale spec: authored by P7's editor, not here.
                };
                (state, range, op1, op2, formula)
            }
        };
        self.seed_cf_inputs(range, op1, op2, formula, window, cx);
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.editor = Some(state);
            cx.notify();
        }
    }

    /// Cancel the editor (Editor → List mode) — the back-chevron / Cancel button
    /// (`components/cf_sidebar.md §4`).
    pub fn cancel_cf_editor(&mut self, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            if panel.editor.take().is_some() {
                cx.notify();
            }
        }
    }

    /// Seed the four editor text inputs (`set_value` suppresses the widgets' change events, so
    /// seeding never trips the live re-render/validation subscription spuriously).
    fn seed_cf_inputs(
        &self,
        range: String,
        operand1: String,
        operand2: String,
        formula: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cf_range_input
            .clone()
            .update(cx, |i, cx| i.set_value(range, window, cx));
        self.cf_operand1_input
            .clone()
            .update(cx, |i, cx| i.set_value(operand1, window, cx));
        self.cf_operand2_input
            .clone()
            .update(cx, |i, cx| i.set_value(operand2, window, cx));
        self.cf_formula_input
            .clone()
            .update(cx, |i, cx| i.set_value(formula, window, cx));
    }

    /// Re-render the chrome so the editor's Save `disabled` state + inline validation track the
    /// field text live as the user types (the inputs are read at render, not committed per
    /// keystroke). A no-op while the editor is closed.
    fn on_cf_input_event(
        &mut self,
        _input: &Entity<InputState>,
        _event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cf_editor_open() {
            cx.notify();
        }
    }

    /// The current text of the four editor inputs `(range, operand1, operand2, formula)` — read at
    /// render (for validation) and at Save (to assemble the spec).
    fn cf_input_texts(&self, cx: &Context<Self>) -> (String, String, String, String) {
        (
            self.cf_range_input.read(cx).value().to_string(),
            self.cf_operand1_input.read(cx).value().to_string(),
            self.cf_operand2_input.read(cx).value().to_string(),
            self.cf_formula_input.read(cx).value().to_string(),
        )
    }

    /// Mutate the open editor's state through `f`, clearing any stale engine error and re-rendering.
    /// A no-op while the editor is closed.
    fn with_cf_editor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut CfEditorState)) {
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            f(editor);
            editor.errors.clear();
            cx.notify();
        }
    }

    /// Toggle the editor's inline dropdown `menu` (open it, or close it if already open).
    fn toggle_cf_menu(&mut self, menu: CfMenu, cx: &mut Context<Self>) {
        self.cf_menu_open = (self.cf_menu_open != Some(menu)).then_some(menu);
        cx.notify();
    }

    /// Select the rule family/variant from the rule-type dropdown, closing the menu. Reseeds the
    /// operand inputs so a switch starts each kind fresh (the Top/Bottom rank seeds to its default).
    fn select_cf_kind(&mut self, kind: CfEditorKind, window: &mut Window, cx: &mut Context<Self>) {
        let rank = self
            .cond_fmt
            .as_ref()
            .and_then(|p| p.editor.as_ref())
            .map(|e| e.top_rank)
            .unwrap_or(10);
        let operand1 = if kind == CfEditorKind::TopBottom {
            rank.to_string()
        } else {
            String::new()
        };
        self.seed_cf_inputs(
            self.cf_range_input.read(cx).value().to_string(),
            operand1,
            String::new(),
            String::new(),
            window,
            cx,
        );
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.kind = kind);
    }

    /// Select the *Cell value* operator, closing the menu.
    fn select_cf_value_op(&mut self, op: CfValueOp, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.value_op = op);
    }

    /// Select the *Text* operator, closing the menu.
    fn select_cf_text_op(&mut self, op: CfTextOp, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.text_op = op);
    }

    /// Select the *A date occurring* period, closing the menu.
    fn select_cf_period(&mut self, period: CfPeriod, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.period = period);
    }

    /// Apply an entire format preset (`CF_PRESETS`) to the editor's differential format.
    fn set_cf_format(&mut self, format: CfFormat, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format = format);
    }

    /// Set (or clear, `None` = "No fill") the highlight fill colour.
    fn set_cf_fill(&mut self, fill: Option<Rgb>, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.fill = fill);
    }

    /// Set (or clear, `None` = "Automatic") the highlight text colour.
    fn set_cf_text_color(&mut self, color: Option<Rgb>, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.text_color = color);
    }

    /// Toggle the highlight's bold attribute.
    fn toggle_cf_bold(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.bold = !e.format.bold);
    }

    /// Toggle the highlight's italic attribute.
    fn toggle_cf_italic(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.italic = !e.format.italic);
    }

    /// Set the *Stop if true* flag (the checkbox).
    fn set_cf_stop_if_true(&mut self, on: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.stop_if_true = on);
    }

    /// Set Top/Bottom to pick the bottom N (`true`) or the top N (`false`).
    fn set_cf_top_bottom(&mut self, bottom: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.top_bottom = bottom);
    }

    /// Toggle Top/Bottom counting by percent of range rather than item count.
    fn toggle_cf_top_percent(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.top_percent = !e.top_percent);
    }

    /// Set Above/Below-average to pick below (`true`) or above (`false`) the average.
    fn set_cf_average_below(&mut self, below: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.average_below = below);
    }

    /// Set Duplicate/Unique to target unique values (`true`) or duplicates (`false`).
    fn set_cf_duplicate_unique(&mut self, unique: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.duplicate_unique = unique);
    }

    /// Set Blanks to target non-blank cells (`true`) or blanks (`false`).
    fn set_cf_blanks_no(&mut self, no_blanks: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.blanks_no = no_blanks);
    }

    /// Set Errors to target non-error cells (`true`) or errors (`false`).
    fn set_cf_errors_no(&mut self, no_errors: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.errors_no = no_errors);
    }

    /// Surface an engine `Err` inline and keep the editor **open** (`functional_spec.md §8`): push
    /// the message into the editor's errors + clear the pending-save flag. Called by the window when
    /// an `EditRejectedReason::Engine` arrives while the CF editor is open. A no-op if the editor
    /// closed in the meantime.
    pub fn show_cf_editor_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            editor.pending_save = false;
            editor.errors.push(message);
            cx.notify();
        }
    }

    /// Validate + commit the editor (the Save button). Client-side validation blocks a bad form
    /// (`components/cf_sidebar.md §6`); a valid form assembles a [`CfRuleSpec`] and sends
    /// `AddCondFmt` (add) / `UpdateCondFmt` (edit). The editor stays open with `pending_save` set —
    /// a success `CondFmtUpdated` returns it to List mode ([`refresh_cond_fmt`](Self::refresh_cond_fmt)),
    /// an engine `Err` keeps it open ([`show_cf_editor_error`](Self::show_cf_editor_error)).
    pub fn save_cf_editor(&mut self, cx: &mut Context<Self>) {
        if self.degraded {
            return;
        }
        let (range, op1, op2, formula) = self.cf_input_texts(cx);
        let Some(panel) = self.cond_fmt.as_ref() else {
            return;
        };
        let Some(editor) = panel.editor.as_ref() else {
            return;
        };
        // Don't re-send while a prior Save is still awaiting its outcome (the button is disabled
        // then; this guards a direct call).
        if editor.pending_save {
            return;
        }
        // Never send a Save the live validation would block (Save is disabled anyway; this guards a
        // direct call). The inline hints already explain what's missing.
        if !cf_validate(editor, &range, &op1, &op2, &formula).is_empty() {
            return;
        }
        let sheet = panel.sheet;
        let edit_index = editor.edit_index;
        let spec = cf_build_spec(editor, &op1, &op2, &formula);
        let range = range.trim().to_string();
        match edit_index {
            None => self.client.send(Command::AddCondFmt { sheet, range, spec }),
            Some(index) => self.client.send(Command::UpdateCondFmt {
                sheet,
                index,
                range,
                spec,
            }),
        }
        self.cf_menu_open = None;
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            editor.errors.clear();
            editor.pending_save = true;
        }
        cx.notify();
    }

    /// Switch the panel's chart to `kind` (P19). A mutating chart control — like `insert_chart` it
    /// commits any pending in-cell edit first (bailing if blocked) and degrade-guards. Updates the
    /// panel's shown `kind` optimistically; the worker republishes the reshaped chart and the window
    /// reconciles.
    pub fn set_chart_type_from_panel(
        &mut self,
        kind: ChartInsertKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let (sheet, id) = (panel.sheet, panel.id);
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetChartType { sheet, id, kind });
        if let Some(panel) = self.chart_panel.as_mut() {
            panel.kind = kind;
        }
        cx.notify();
    }

    /// Bind the panel's chart to the **current grid selection** as its data range (P19). The skeleton
    /// range-picker: the user selects the data block in the grid, then applies it here. Commits any
    /// pending edit first + degrade-guards, then sends `SetChartRange` for the current selection
    /// rectangle; the worker re-resolves the chart live + republishes.
    ///
    /// The command's `sheet` is the **active** sheet — the one the selection's coordinates live in —
    /// **not** the chart's host sheet: the worker finds the chart by `id` and qualifies the emitted
    /// `c:f` with `sheet`, so pairing the selection with its own sheet is what keeps the binding
    /// honest (and enables valid cross-sheet data, e.g. a chart on Sheet1 bound to Sheet2's cells).
    /// Every other range command in the chrome pairs `self.selection.range()` with `self.active_sheet`.
    pub fn apply_chart_range_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let id = panel.id;
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetChartRange {
            sheet: self.active_sheet,
            id,
            data: self.selection.range(),
        });
        cx.notify();
    }

    // ---- Chart edit panel: chrome (P20) ---------------------------------------------------

    /// Send one chrome edit for the panel's chart, on either provenance (`Command::SetChartChrome`).
    /// A mutating chart control — like every panel/action-row control it degrade-guards and commits
    /// any pending in-cell edit first (bailing if blocked), then hands the panel to `update_panel` to
    /// reflect the change optimistically (the window reconciles from the republished snapshot).
    fn send_chart_chrome(
        &mut self,
        edit: ChartChromeEdit,
        update_panel: impl FnOnce(&mut ChartPanel),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let (sheet, id) = (panel.sheet, panel.id);
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client
            .send(Command::SetChartChrome { sheet, id, edit });
        if let Some(panel) = self.chart_panel.as_mut() {
            update_panel(panel);
        }
        cx.notify();
    }

    /// Set (or clear, `None`) the chart title (P20).
    pub fn set_chart_title_from_panel(
        &mut self,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::Title(title.clone());
        self.send_chart_chrome(edit, |p| p.title = title, window, cx);
    }

    /// Turn the legend on at `position`, or off (`None`) (P20).
    pub fn set_chart_legend_from_panel(
        &mut self,
        position: Option<LegendPosition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::Legend(position);
        self.send_chart_chrome(edit, |p| p.legend = position, window, cx);
    }

    /// Set (or clear, `None`) an axis title (P20).
    pub fn set_chart_axis_title_from_panel(
        &mut self,
        axis: ChartAxisKind,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::AxisTitle {
            axis,
            title: title.clone(),
        };
        self.send_chart_chrome(
            edit,
            |p| match axis {
                ChartAxisKind::Category => p.cat_axis_title = title,
                ChartAxisKind::Value => p.val_axis_title = title,
            },
            window,
            cx,
        );
    }

    /// Set (or clear back to the palette, `None`) one series' color (P20).
    pub fn set_chart_series_color_from_panel(
        &mut self,
        series: usize,
        color: Option<Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::SeriesColor { series, color };
        self.send_chart_chrome(
            edit,
            |p| {
                if let Some(s) = p.series.get_mut(series) {
                    s.color = color;
                }
            },
            window,
            cx,
        );
    }

    /// Apply the data-label toggles across the chart's series (P20).
    pub fn set_chart_data_labels_from_panel(
        &mut self,
        labels: DataLabelToggles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::DataLabels(labels);
        self.send_chart_chrome(edit, |p| p.labels = labels, window, cx);
    }

    /// The title-input's current value as a chart title (`None` for empty/blank).
    fn chart_input_title(input: &Entity<InputState>, cx: &Context<Self>) -> Option<String> {
        let text = input.read(cx).value().to_string();
        (!text.trim().is_empty()).then_some(text)
    }

    /// The panel's current target `(sheet, id)` — the key the input-focus staleness guard compares.
    fn chart_panel_key(&self) -> Option<(SheetId, ChartId)> {
        self.chart_panel.as_ref().map(|p| (p.sheet, p.id))
    }

    /// Classify a chart-input event into the shared handling: capture the target on `Focus`, or
    /// return `true` to **commit** the field's current text as a live chrome edit.
    ///
    /// The title + axis-title fields commit **live, per keystroke** (`Change`) so the chart updates
    /// as the user types (post-v1 Batch 2, item 6) — not only on Enter/blur. `Change` reads the panel
    /// *synchronously* with the keystroke, so it always targets the chart currently shown. Enter/blur
    /// remain commit points too (a redundant safety net once live commits have already synced).
    ///
    /// Every commit is guarded against the **cross-chart clobber**: it fires only if the panel still
    /// points at the chart the field was focused for. A `Change` keeps the captured focus (typing
    /// continues); Enter/blur consume it (`take`). Seeding uses `set_value`, which suppresses `Change`
    /// (`InputState::emit_events`), so re-seeding a field on a chart switch never emits a spurious
    /// live edit — and a live republish of the *same* chart never re-seeds (only an id change does,
    /// [`open_chart_panel`]), so it can't clobber in-progress typing.
    fn chart_input_should_commit(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::Focus => {
                self.chart_input_focus = self.chart_panel_key();
                false
            }
            // Live per-keystroke commit: keep the focus capture (more keystrokes will follow) and
            // commit only while the panel still points at the focused chart.
            InputEvent::Change => match self.chart_input_focus {
                Some(focused) => self.chart_panel_key() == Some(focused),
                // No captured focus (e.g. a direct test call) → allow if a panel is open.
                None => self.chart_panel_key().is_some(),
            },
            InputEvent::PressEnter { .. } | InputEvent::Blur => match self.chart_input_focus.take()
            {
                // Commit only if the panel still points at the chart the field was focused for; a
                // re-point (or a closed panel) since focus drops the stale text.
                Some(focused) => self.chart_panel_key() == Some(focused),
                // No captured focus (e.g. a direct call) → allow (send_chart_chrome still guards).
                None => true,
            },
        }
    }

    /// Commit the chart title input live per keystroke (`Change`), and on Enter / blur — only when it
    /// differs from the panel's current title (so a redundant event doesn't re-send the same value)
    /// and the panel hasn't been re-pointed since focus (the staleness guard).
    fn on_chart_title_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_title_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.title.clone()) != Some(title.clone()) {
            self.set_chart_title_from_panel(title, window, cx);
        }
    }

    fn on_chart_cat_axis_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_cat_axis_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.cat_axis_title.clone()) != Some(title.clone()) {
            self.set_chart_axis_title_from_panel(ChartAxisKind::Category, title, window, cx);
        }
    }

    fn on_chart_val_axis_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_val_axis_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.val_axis_title.clone()) != Some(title.clone()) {
            self.set_chart_axis_title_from_panel(ChartAxisKind::Value, title, window, cx);
        }
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    /// Sends one `SetFont` over the selection after committing any pending edit (fire-and-forget,
    /// degraded-guarded — the same rule as the `SetStylePath` controls).
    fn apply_set_font(
        &mut self,
        family: Option<String>,
        size_pt: Option<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetFont {
            sheet: self.active_sheet,
            range: self.selection.range(),
            family,
            size_pt,
        });
        cx.notify();
    }

    /// Applies a font family over the selection, closing the family dropdown. "Default (Inter)"
    /// clears the override (sent as `Some("")`); any other name sets it.
    pub fn apply_font_family(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.font_family_open = false;
        let family = if name == SYSTEM_DEFAULT_FAMILY {
            String::new()
        } else {
            name.to_string()
        };
        self.apply_set_font(Some(family), None, window, cx);
    }

    /// Applies a font size (points) over the selection, closing the size dropdown.
    pub fn apply_font_size(&mut self, pt: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.font_size_open = false;
        self.apply_set_font(None, Some(pt), window, cx);
    }

    fn toggle_font_family_popover(&mut self, cx: &mut Context<Self>) {
        self.font_family_open = !self.font_family_open;
        cx.notify();
    }

    fn toggle_font_size_popover(&mut self, cx: &mut Context<Self>) {
        self.font_size_open = !self.font_size_open;
        cx.notify();
    }

    // ---- Action row: SetBorders (pen popover) ---------------------------------------------

    /// Paints the current pen (`border_line` + `border_color`) onto `preset`'s edges over the
    /// selection. Degraded-guards + commits any pending edit first, the same rule as the other
    /// action-row controls (`components/action_bar.md`); returns whether it dispatched. Shared by
    /// [`select_border_target`](Self::select_border_target) and the pen-tweak repaints. For
    /// [`BorderPreset::None`] the engine clears the selection's borders (line/color unused).
    /// Fire-and-forget: the worker logs any engine rejection.
    fn send_border_paint(
        &mut self,
        preset: BorderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.degraded {
            return false;
        }
        if !self.commit_pending_edit(window, cx) {
            return false;
        }
        self.client.send(Command::SetBorders {
            sheet: self.active_sheet,
            range: self.selection.range(),
            preset,
            line: self.border_line,
            color: Some(self.border_color),
        });
        true
    }

    /// Selects a border **target** and paints the current pen onto just its edges — the pen model
    /// (`functional_spec.md §2.1`, `ui_design.md §2.4`). The popover **stays open** (unlike the old
    /// apply-and-close preset path): only click-away / Esc closes it. `None` clears all borders in
    /// the selection and leaves **no** target selected (there is nothing left to keep styling).
    pub fn select_border_target(
        &mut self,
        preset: BorderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.send_border_paint(preset, window, cx) {
            return;
        }
        // `None` is an action, not a paintable target — it deselects; every other preset becomes
        // the selected target so subsequent pen tweaks repaint it.
        self.border_target = (preset != BorderPreset::None).then_some(preset);
        cx.notify();
    }

    /// Sets the pen's **line style**. If a target is selected, repaints that target with the new
    /// pen; with no target, updates the pen only (MVP — no sheet change until a target is picked;
    /// P2 restyle-all is deferred, GAPS F2). The pen carries across target switches.
    pub fn set_border_line(
        &mut self,
        line: BorderLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.border_line = line;
        if let Some(preset) = self.border_target {
            self.send_border_paint(preset, window, cx);
        }
        cx.notify();
    }

    /// Sets the pen's **color** (symmetric to [`set_border_line`](Self::set_border_line)):
    /// repaints the selected target, or updates the pen only when no target is selected.
    pub fn set_border_color(&mut self, color: Rgb, window: &mut Window, cx: &mut Context<Self>) {
        self.border_color = color;
        if let Some(preset) = self.border_target {
            self.send_border_paint(preset, window, cx);
        }
        cx.notify();
    }

    /// Toggles the borders popover. **Opening resets the transient pen state** — no target
    /// selected, pen back to thin solid black — even if the selection already has borders (border
    /// state is never derived from existing cell borders; `functional_spec.md §2.1`).
    fn toggle_borders_popover(&mut self, cx: &mut Context<Self>) {
        self.borders_open = !self.borders_open;
        if self.borders_open {
            self.border_target = None;
            self.border_line = BorderLine::ThinSolid;
            // The pen color is our source of truth; resetting it re-rings the black swatch. We
            // deliberately do NOT reach into the stock `border_color_picker`'s internal display
            // state, so its "Custom…" preview can still show the previous custom color until the
            // user picks again — cosmetic, and identical to the fill/text-color pickers by precedent.
            self.border_color = Rgb::new(0, 0, 0);
        }
        cx.notify();
    }

    /// Whether the borders popover is open (test/render introspection).
    pub fn borders_open(&self) -> bool {
        self.borders_open
    }

    /// The pen's selected target, if any (test introspection).
    #[cfg(test)]
    pub fn border_target(&self) -> Option<BorderPreset> {
        self.border_target
    }

    /// The pen's current line style (test introspection).
    #[cfg(test)]
    pub fn border_line(&self) -> BorderLine {
        self.border_line
    }

    /// The pen's current color (test introspection).
    #[cfg(test)]
    pub fn border_color(&self) -> Rgb {
        self.border_color
    }

    /// The font-family dropdown's active label: the active cell's family, or "Default (Inter)" for a
    /// default-font (or multi-cell) selection (`components/action_bar.md`).
    pub fn font_family_label(&self) -> &str {
        match self.active_font_family.as_deref() {
            Some(name) if !name.is_empty() => name,
            _ => SYSTEM_DEFAULT_FAMILY,
        }
    }

    /// The font-size dropdown's active label. An explicit size (`font_size_q != 0`) shows `q/4` pt;
    /// a **default** cell shows the workbook's real default size (13pt for a new workbook, the file's
    /// default otherwise) — never a hardcoded value that would mismatch the cell. Re-picking that
    /// shown default from the list is a visual no-op (the engine maps size == the workbook default
    /// back to the sentinel), so no surprising size jump. `13` is the fallback before a cache loads
    /// (IronCalc's default; `DECISIONS_TO_REVIEW` records the residual pt↔px seam).
    pub fn font_size_label(&self) -> String {
        let q = self.active_style.map(|s| s.font_size_q).unwrap_or(0);
        if q != 0 {
            font_size_display(q)
        } else {
            format_size_pt(self.default_font_size_pt.unwrap_or(13.0))
        }
    }

    fn on_text_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_text_color(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    /// The borders "Custom…" picker changed → set the pen color (repaints the selected target, if
    /// any). Mirrors [`on_color_picker_event`](Self::on_color_picker_event).
    fn on_border_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.set_border_color(hsla_to_rgb(*hsla), window, cx);
        }
    }

    /// Marks the worker degraded/read-only (or clears it) — disables every mutating action-bar
    /// control (`functional_spec.md §6`). Called by the window on `WorkerDegraded`. Closes any open
    /// formatting popover so a swatch/entry can't be clicked after the controls lock.
    pub fn set_degraded(&mut self, degraded: bool, cx: &mut Context<Self>) {
        if self.degraded != degraded {
            self.degraded = degraded;
            if degraded {
                self.fill_open = false;
                self.text_color_open = false;
                self.num_fmt_open = false;
                self.num_fmt_more_open = false;
                self.font_family_open = false;
                self.font_size_open = false;
                self.borders_open = false;
                self.chart_menu_open = false;
                self.chart_panel = None;
                self.cond_fmt = None;
            }
            cx.notify();
        }
    }

    // ---- Sheet tab bar --------------------------------------------------------------------

    /// Replaces the tab list + active sheet (fixtures / Phase-11 init).
    pub fn set_sheets(&mut self, sheets: Vec<SheetTab>, active: SheetId, cx: &mut Context<Self>) {
        self.sheets = sheets;
        self.active_sheet = active;
        self.prune_tab_spans();
        cx.notify();
    }

    /// Drops captured tab spans for sheets that no longer exist (deleted / reloaded), so the
    /// insertion geometry never reads a stale slot. Survivors are re-measured on the next paint.
    fn prune_tab_spans(&mut self) {
        self.tab_spans
            .retain(|span| self.sheets.iter().any(|t| t.id == span.sheet));
    }

    /// Merges a worker sheet-meta list into the tab mirror. `has_content` is now sourced
    /// directly from the worker's `SheetMeta` (Phase 11 populated it), so the delete-confirm
    /// gate is correct against the real workbook.
    fn merge_sheet_metas(&mut self, metas: &[freecell_engine::SheetMeta]) {
        self.sheets = metas
            .iter()
            .map(|meta| SheetTab {
                id: meta.id,
                name: meta.name.clone(),
                has_content: meta.has_content,
            })
            .collect();
        if !self.sheets.iter().any(|t| t.id == self.active_sheet) {
            if let Some(first) = self.sheets.first() {
                self.active_sheet = first.id;
            }
        }
        self.prune_tab_spans();
    }

    /// Adopts `id` as the active sheet because the *window* (not a tab click) switched it — the
    /// worker added a sheet, a sheet was deleted, or the initial load resolved. Unlike
    /// [`select_sheet`](Self::select_sheet) this does **not** re-emit a `SetActiveSheet` grid
    /// request (that would re-enter the window's `defer` loop); it only re-points the chrome's
    /// active sheet so every subsequent command/fetch and the tab highlight target the right
    /// sheet, and refreshes the action-row toggle state. Load-bearing: without this, adding a
    /// sheet left the chrome pointing at the OLD sheet and routed edits there (`functional_spec.md
    /// §3.7`).
    pub fn adopt_active_sheet(&mut self, id: SheetId, cx: &mut Context<Self>) {
        if id == self.active_sheet {
            return;
        }
        self.active_sheet = id;
        // The committed content belongs to the old sheet — invalidate its seed tag (the tag is also
        // sheet-qualified, so this is belt-and-braces against a cross-sheet stale seed).
        self.committed_cell = None;
        self.context_menu = None;
        // An open find bar re-scopes to the new sheet (`functional_spec.md §4.5`).
        self.rescope_find_if_open(cx);
        // An open CF sidebar re-scopes to the new sheet (`components/cf_sidebar.md §9`).
        self.rescope_cond_fmt_if_open(cx);
        self.refresh_active_style(cx);
    }

    /// Switches the active sheet (tab click) and asks the grid to follow.
    pub fn select_sheet(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.active_sheet {
            return;
        }
        // grid.md invariant: commit any pending data-row edit BEFORE the switch (Excel click-away).
        // The commit targets the CURRENT sheet's edited cell, so it must precede the `active_sheet`
        // change here — otherwise the deferred switch would commit against the new sheet, and its
        // `on_selection_changed` would run while Editing. A cap-rejected edit blocks the switch
        // (stay on this sheet, keep editing).
        if !self.on_edit_commit_requested(window, cx) {
            return;
        }
        self.active_sheet = id;
        self.committed_cell = None;
        self.context_menu = None;
        // An open find bar re-scopes to the new sheet (`functional_spec.md §4.5`).
        self.rescope_find_if_open(cx);
        // An open CF sidebar re-scopes to the new sheet (`components/cf_sidebar.md §9`).
        self.rescope_cond_fmt_if_open(cx);
        self.grid
            .emit(ChromeGridRequest::SetActiveSheet(id), window, cx);
        cx.notify();
    }

    /// Adds a sheet (the worker names it and republishes; the UI switches on `SheetsChanged`).
    pub fn add_sheet(&self) {
        self.client.send(Command::AddSheet);
    }

    // ---- Sheet-tab reorder drag (`functional_spec.md §6`, `ui_design.md §3`) ---------------

    /// Records a *potential* tab reorder drag on mouse-down at window `x` (no movement yet). A
    /// plain click / double-click never crosses the threshold, so this stays a no-op until then.
    fn tab_press(&mut self, sheet: SheetId, x: f32) {
        self.tab_drag = Some(TabDrag {
            sheet,
            start_x: x,
            cur_x: x,
            dragging: false,
        });
    }

    /// Advances a live tab drag to window `x`; crosses into `dragging` past the threshold, at which
    /// point the lift + drop indicator repaint. No-op when no press is pending.
    fn tab_drag_move(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.as_mut() else {
            return;
        };
        drag.cur_x = x;
        if !drag.dragging && (x - drag.start_x).abs() > TAB_DRAG_THRESHOLD_PX {
            drag.dragging = true;
        }
        if drag.dragging {
            cx.notify();
        }
    }

    /// Ends a tab drag at window `x`: a real drag to a new slot sends `MoveSheet`; a sub-threshold
    /// press (a click) or a drop back on the origin slot sends nothing (the click-select path fires
    /// separately). Always clears the drag state.
    fn tab_drag_end(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.take() else {
            return;
        };
        if drag.dragging {
            if let Some(to_index) = self.tab_move_target(drag.sheet, x) {
                self.client.send(Command::MoveSheet {
                    sheet: drag.sheet,
                    to_index,
                });
            }
        }
        cx.notify();
    }

    /// The current tabs' captured centers (window x), in `self.sheets` slot order. Empty unless
    /// every tab has a captured span — the caller treats an incomplete capture as "geometry not
    /// ready" and skips the move.
    fn ordered_tab_centers(&self) -> Vec<f32> {
        self.sheets
            .iter()
            .filter_map(|t| self.tab_spans.iter().find(|s| s.sheet == t.id))
            .map(|s| (s.left + s.right) / 2.0)
            .collect()
    }

    /// The fork `to_index` a drop at window `cursor_x` maps to for the dragged `sheet`, or `None`
    /// for a no-op (drop on the origin slot) or when the tab geometry is not fully captured yet.
    fn tab_move_target(&self, sheet: SheetId, cursor_x: f32) -> Option<u32> {
        let centers = self.ordered_tab_centers();
        if centers.len() != self.sheets.len() {
            return None; // some tab hasn't been measured — don't guess a reorder
        }
        let from_slot = self.sheets.iter().position(|t| t.id == sheet)?;
        let gap = tab_insertion_index(cursor_x, &centers);
        move_target_for_gap(gap, from_slot).map(|to| to as u32)
    }

    /// The window-x at which to paint the 2 px drop indicator for the live drag, or `None` when
    /// not dragging / the geometry is not fully captured. Snaps to the midpoint of the neighboring
    /// tab edges (outer edges offset by half the inter-tab gap).
    fn tab_drop_indicator_x(&self) -> Option<f32> {
        let drag = self.tab_drag?;
        if !drag.dragging {
            return None;
        }
        let spans: Vec<(f32, f32)> = self
            .sheets
            .iter()
            .filter_map(|t| self.tab_spans.iter().find(|s| s.sheet == t.id))
            .map(|s| (s.left, s.right))
            .collect();
        if spans.is_empty() || spans.len() != self.sheets.len() {
            return None;
        }
        let centers: Vec<f32> = spans.iter().map(|(l, r)| (l + r) / 2.0).collect();
        let gap = tab_insertion_index(drag.cur_x, &centers);
        let n = spans.len();
        let x = if gap == 0 {
            spans[0].0 - TAB_GAP_HALF
        } else if gap >= n {
            spans[n - 1].1 + TAB_GAP_HALF
        } else {
            (spans[gap - 1].1 + spans[gap].0) / 2.0
        };
        Some(x)
    }

    /// Whether a tab reorder drag has crossed the threshold (drives the lift + cursor + indicator).
    fn tab_drag_active(&self) -> bool {
        self.tab_drag.is_some_and(|d| d.dragging)
    }

    /// Starts an inline rename of `id`, seeding + focusing the rename input.
    pub fn rename_start(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        self.rename_target = Some(id);
        self.rename_error = false;
        self.context_menu = None;
        self.rename_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Commits the pending rename (Enter): validates against the other sheet names; invalid
    /// keeps editing with a danger border.
    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.validated_rename(cx) {
            Some((id, name)) => {
                self.client.send(Command::RenameSheet { sheet: id, name });
                self.rename_target = None;
                self.rename_error = false;
                self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
            }
            None => {
                if self.rename_target.is_some() {
                    self.rename_error = true;
                }
            }
        }
        cx.notify();
    }

    /// Cancels the pending rename (Escape / blur-when-invalid), reverting to the tab label.
    pub fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_target.is_none() {
            return;
        }
        self.rename_target = None;
        self.rename_error = false;
        self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
        cx.notify();
    }

    /// The pending rename resolved to `(id, name)` iff it validates, else `None`.
    fn validated_rename(&self, cx: &Context<Self>) -> Option<(SheetId, String)> {
        let id = self.rename_target?;
        let name = self.rename_input.read(cx).value().trim().to_string();
        let others: Vec<&str> = self
            .sheets
            .iter()
            .filter(|t| t.id != id)
            .map(|t| t.name.as_str())
            .collect();
        validate_sheet_name(&name, &others)
            .ok()
            .map(|()| (id, name))
    }

    fn on_rename_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.commit_rename(window, cx),
            InputEvent::Blur => {
                // Blur commits if valid, otherwise reverts (never traps focus in a bad name).
                if self.validated_rename(cx).is_some() {
                    self.commit_rename(window, cx);
                } else {
                    self.cancel_rename(window, cx);
                }
            }
            _ => {}
        }
    }

    fn open_context_menu(&mut self, id: SheetId, cx: &mut Context<Self>) {
        self.context_menu = Some(id);
        cx.notify();
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
    }

    /// Whether a sheet can be deleted (not the last one).
    pub fn delete_enabled(&self) -> bool {
        self.sheets.len() > 1
    }

    /// Requests deletion of `id`: a non-empty sheet opens the confirm modal; an empty one is
    /// deleted immediately. The last sheet cannot be deleted.
    pub fn request_delete(&mut self, id: SheetId, cx: &mut Context<Self>) {
        self.context_menu = None;
        if !self.delete_enabled() {
            cx.notify();
            return;
        }
        let has_content = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.has_content)
            .unwrap_or(false);
        if has_content {
            self.confirm_delete = Some(id);
        } else {
            self.client.send(Command::DeleteSheet { sheet: id });
        }
        cx.notify();
    }

    /// Confirms the pending delete.
    pub fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.confirm_delete.take() {
            self.client.send(Command::DeleteSheet { sheet: id });
            cx.notify();
        }
    }

    /// Cancels the pending delete.
    pub fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        cx.notify();
    }

    // ---- Read accessors (tests + render) --------------------------------------------------

    /// The ref box text: `B7` / `B2:D9` for cells, and the band forms `C:C` / `3:7` / `A:XFD`
    /// for header selections (`components/grid_structure.md §5.2`).
    pub fn ref_box_text(&self) -> String {
        freecell_core::format_selection_ref(&self.selection)
    }

    /// The content field's current text.
    pub fn content_text(&self, cx: &App) -> String {
        self.content_input.read(cx).value().to_string()
    }

    /// The formula-bar mode.
    pub fn data_mode(&self) -> FieldMode {
        self.data_row.mode()
    }

    /// Whether the bold toggle is pressed (active cell is bold).
    pub fn bold_active(&self) -> bool {
        self.active_style.map(|s| s.bold).unwrap_or(false)
    }

    /// Whether the italic toggle is pressed.
    pub fn italic_active(&self) -> bool {
        self.active_style.map(|s| s.italic).unwrap_or(false)
    }

    /// Whether the underline toggle is pressed.
    pub fn underline_active(&self) -> bool {
        self.active_style.map(|s| s.underline).unwrap_or(false)
    }

    /// Whether the strikethrough toggle is pressed.
    pub fn strikethrough_active(&self) -> bool {
        self.active_style.map(|s| s.strikethrough).unwrap_or(false)
    }

    /// Whether the wrap-text toggle is pressed.
    pub fn wrap_active(&self) -> bool {
        self.active_style.map(|s| s.wrap).unwrap_or(false)
    }

    /// Whether an alignment button is pressed — the **explicit** alignment only (a number aligned
    /// right by type default shows no pressed button, matching Excel; `components/action_bar.md`).
    pub fn align_active(&self, align: Align) -> bool {
        self.active_style.and_then(|s| s.h_align) == Some(align)
    }

    /// Whether a vertical-alignment button is pressed — the active cell's resolved vertical
    /// alignment (`functional_spec.md §1.3`). Under decision C the resolver reports a defaulted
    /// bottom as `Some(Bottom)`, so a cell whose vertical is merely defaulted (e.g. only horizontal
    /// set, or loaded from `.xlsx`) lights **Align bottom**; a truly-clean cell (no alignment
    /// record at all) lights nothing but still renders bottom. Accepted Excel-ish behavior.
    pub fn valign_active(&self, valign: VAlign) -> bool {
        self.active_style.and_then(|s| s.v_align) == Some(valign)
    }

    /// The active cell's number-format [`Category`] (General on a multi-cell selection / no cache).
    pub fn num_fmt_category(&self) -> Category {
        num_fmt_category(self.active_num_fmt.as_deref().unwrap_or("general"))
    }

    /// The number-format dropdown's button label (the active cell's category name).
    pub fn num_fmt_category_label(&self) -> &'static str {
        self.num_fmt_category().label()
    }

    /// Whether the "increase decimals" button is enabled (not degraded, single cell, and the
    /// active format has an adjustable decimal group).
    pub fn increase_decimals_enabled(&self) -> bool {
        self.decimals_enabled(1)
    }

    /// Whether the "decrease decimals" button is enabled.
    pub fn decrease_decimals_enabled(&self) -> bool {
        self.decimals_enabled(-1)
    }

    fn decimals_enabled(&self, delta: i8) -> bool {
        if self.degraded {
            return false;
        }
        let (numeric, displayed) = self.active_numeric_decimals();
        self.active_num_fmt
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
            .is_some()
    }

    /// Whether the active cell is a *number* (not text/date/bool/error/empty) and, if so, how many
    /// decimals its value currently displays — the inputs the decimals ± need to enable/adjust a
    /// General-formatted number (BUG 3). Both come from the cached publication of the active cell.
    fn active_numeric_decimals(&self) -> (bool, Option<u8>) {
        match &self.active_published {
            Some((CellKind::Number, display)) => (true, displayed_decimals(display)),
            _ => (false, None),
        }
    }

    /// Whether the worker is degraded (read-only) — all mutating action-bar controls disable.
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// Whether the text-color popover is open.
    pub fn text_color_open(&self) -> bool {
        self.text_color_open
    }

    /// Whether the number-format dropdown is open.
    pub fn num_fmt_open(&self) -> bool {
        self.num_fmt_open
    }

    /// Whether the evaluating spinner is shown.
    pub fn eval_spinner_visible(&self) -> bool {
        self.eval.spinner()
    }

    /// Whether the content-fetch spinner is shown.
    pub fn fetch_spinner_visible(&self) -> bool {
        self.data_row.spinner()
    }

    /// Whether the content field shows the cap-rejection danger state.
    pub fn cap_error_visible(&self) -> bool {
        self.data_row.cap_error() || self.cap_error_external.is_some()
    }

    /// The cap-error popover message (`functional_spec.md §4.2`), if a cap rejection is
    /// active. A local reject (the reducer's) takes precedence over the worker backstop.
    pub fn cap_error_message(&self) -> Option<String> {
        self.data_row
            .cap_rejection()
            .or(self.cap_error_external)
            .map(|r| r.message())
    }

    /// The active sheet id.
    pub fn active_sheet(&self) -> SheetId {
        self.active_sheet
    }

    /// The current tab list.
    pub fn sheets(&self) -> &[SheetTab] {
        &self.sheets
    }

    /// The sheet being renamed, if any.
    pub fn rename_target(&self) -> Option<SheetId> {
        self.rename_target
    }

    /// Whether the pending rename is showing the invalid-name state.
    pub fn rename_error(&self) -> bool {
        self.rename_error
    }

    /// The sheet awaiting delete confirmation, if any.
    pub fn confirm_delete_target(&self) -> Option<SheetId> {
        self.confirm_delete
    }

    /// The tab whose context menu is open, if any.
    pub fn context_menu_target(&self) -> Option<SheetId> {
        self.context_menu
    }

    /// Whether the fill popover is open.
    pub fn fill_open(&self) -> bool {
        self.fill_open
    }
}

/// Converts a gpui `Hsla` to a 24-bit [`Rgb`] (the color picker's "Custom…" pick).
fn hsla_to_rgb(hsla: Hsla) -> Rgb {
    let rgba: Rgba = hsla.into();
    Rgb::new(
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
    )
}

/// Which of the target icon's six segments `(top, bottom, left, right, inner_h, inner_v)` a
/// `preset` paints **dark** (affected), the rest staying light-grey context (`ui_design.md §2.2`).
/// The mask mirrors IronCalc's per-`BorderType` edges: All = all six, Inner = the inner cross,
/// Outer = the perimeter, None = nothing, and each of Top/Bottom/Left/Right = its one outer edge.
/// Split out from [`border_target_icon`] so this affordance-defining table is unit-testable (the
/// render harness doesn't cover the chrome popover).
fn border_target_icon_mask(preset: BorderPreset) -> (bool, bool, bool, bool, bool, bool) {
    match preset {
        BorderPreset::All => (true, true, true, true, true, true),
        BorderPreset::Inner => (false, false, false, false, true, true),
        BorderPreset::Outer => (true, true, true, true, false, false),
        BorderPreset::None => (false, false, false, false, false, false),
        BorderPreset::Top => (true, false, false, false, false, false),
        BorderPreset::Bottom => (false, true, false, false, false, false),
        BorderPreset::Left => (false, false, true, false, false, false),
        BorderPreset::Right => (false, false, false, true, false, false),
    }
}

/// A borders **target icon** (`ui_design.md §2.2`): a ~22px 2×2 mini-grid drawn from `div`
/// rectangles. Every gridline is context light-grey (1px); the segments the `preset` affects are
/// solid dark (2px, heavier). The six segments are the four outer edges + the inner cross (mid-H,
/// mid-V); the per-preset dark mask ([`border_target_icon_mask`]) mirrors IronCalc's per-`BorderType`
/// edges. Grey segments paint first so a dark segment always wins at a crossing.
fn border_target_icon(preset: BorderPreset) -> gpui::AnyElement {
    let (top, bottom, left, right, inner_h, inner_v) = border_target_icon_mask(preset);
    let near = 1.0;
    let far = TARGET_ICON_PX - 1.0;
    let mid = TARGET_ICON_PX / 2.0;
    // A horizontal / vertical segment centered on `nominal`, spanning the inset box `[near, far]`
    // extended by its own thickness `t` at each end so it reaches the OUTER edge of the
    // perpendicular lines: corners meet flush (dark t=2 → full extent) with no gap or overhang.
    let hline = |nominal: f32, dark: bool| {
        let t = if dark { 2.0 } else { 1.0 };
        div()
            .absolute()
            .left(px(near - t / 2.0))
            .top(px(nominal - t / 2.0))
            .w(px(far - near + t))
            .h(px(t))
            .bg(rgb(if dark {
                TARGET_ICON_DARK
            } else {
                TARGET_ICON_GREY
            }))
    };
    let vline = |nominal: f32, dark: bool| {
        let t = if dark { 2.0 } else { 1.0 };
        div()
            .absolute()
            .top(px(near - t / 2.0))
            .left(px(nominal - t / 2.0))
            .h(px(far - near + t))
            .w(px(t))
            .bg(rgb(if dark {
                TARGET_ICON_DARK
            } else {
                TARGET_ICON_GREY
            }))
    };
    // Each segment as (is_horizontal, nominal, dark).
    let segments = [
        (true, near, top),
        (true, far, bottom),
        (true, mid, inner_h),
        (false, near, left),
        (false, far, right),
        (false, mid, inner_v),
    ];
    let mut icon = div()
        .relative()
        .flex_none()
        .w(px(TARGET_ICON_PX))
        .h(px(TARGET_ICON_PX));
    // Grey first, then dark on top (so a dark segment wins where it crosses a grey one).
    for &(is_h, nominal, _) in segments.iter().filter(|s| !s.2) {
        icon = icon.child(if is_h {
            hline(nominal, false)
        } else {
            vline(nominal, false)
        });
    }
    for &(is_h, nominal, _) in segments.iter().filter(|s| s.2) {
        icon = icon.child(if is_h {
            hline(nominal, true)
        } else {
            vline(nominal, true)
        });
    }
    icon.into_any_element()
}

/// A borders **line-style preview** (`ui_design.md §2.3`): a short horizontal sample of the real
/// line, vertically centered in a ~34px box. Solid weights are one dark bar (1/2/3px); dashed is a
/// row of short dark dashes; double is two 1px dark bars with a gap.
fn border_line_preview(line: BorderLine) -> gpui::AnyElement {
    const SAMPLE_W: f32 = 34.0;
    let box_ = || {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .w(px(SAMPLE_W))
            .h(px(12.0))
    };
    let bar = |weight: f32| {
        div()
            .w(px(SAMPLE_W))
            .h(px(weight))
            .bg(rgb(TARGET_ICON_DARK))
    };
    match line {
        BorderLine::ThinSolid => box_().child(bar(1.0)).into_any_element(),
        BorderLine::MediumSolid => box_().child(bar(2.0)).into_any_element(),
        BorderLine::ThickSolid => box_().child(bar(3.0)).into_any_element(),
        BorderLine::Dashed => {
            // A run of short dark dashes with gaps (5 dashes across the sample).
            let mut dashes = div().flex().items_center().gap(px(2.0)).h(px(2.0));
            for _ in 0..5 {
                dashes = dashes.child(div().w(px(4.0)).h(px(2.0)).bg(rgb(TARGET_ICON_DARK)));
            }
            box_().child(dashes).into_any_element()
        }
        BorderLine::Double => box_()
            .gap(px(1.0))
            .child(bar(1.0))
            .child(bar(1.0))
            .into_any_element(),
    }
}

/// Formats a font size in points for the size box, trimming a trailing `.0` (`13.0` → `"13"`,
/// `10.5` → `"10.5"`) — the same look as [`font_size_display`] for explicit sizes.
fn format_size_pt(pt: f64) -> String {
    format!("{pt}")
}

/// A vertical divider between action-row control groups (`ui_design.md §2`, existing styling).
/// `pub(super)` so the sibling [`super::h_scroller`] reuses the exact same divider for the
/// horizontal scroller's chevron section (`functional_spec.md §9B`, D9.3) and the tab bar's
/// leading stats divider (§9A.3).
pub(super) fn action_divider() -> gpui::Div {
    div().w(px(1.0)).h(px(20.0)).mx_1().bg(rgb(DIVIDER))
}

impl Focusable for ChromeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChromeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("freecell-chrome")
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            // Fill the available height when hosting the grid, so the grid slot can flex.
            .when(self.body.is_some(), |d| d.flex_1().min_h_0())
            // `window` threads to the two `h_scroller` call sites (action row + tab bar) so a chevron
            // click can drive an animated slide via `request_animation_frame` (D10.2).
            .child(self.render_action_row(window, cx))
            .child(self.render_data_row(cx))
            // The find/replace bar sits directly below the data row and above the grid, pushing the
            // grid down when open (`functional_spec.md §4.1`, `ui_design.md §1`).
            .children(self.find_open.then(|| self.render_find_bar(cx)))
            // The grid body fills the space between the data row and the tab bar
            // (`ui_design.md §3`: action → data → grid → tabs).
            .when_some(self.body.clone(), |d, body| {
                d.child(div().flex_1().min_h_0().w_full().child(body))
            })
            .child(self.render_tab_bar(window, cx))
            .children(self.render_overlays(cx))
    }
}

impl ChromeView {
    /// Wraps a dropdown/popover trigger `button` so its panel can anchor under the real, laid-out
    /// button position instead of a guessed pixel offset (BUG 2c). A zero-size `canvas` probe
    /// fills the wrapper and records the button's window-x into `anchor_x[which]` on each paint —
    /// chrome-local x equals window x (the chrome fills the window width from x = 0), and only the
    /// x is needed (the panel's y is the fixed action-row height). It notifies only on a real
    /// change, so a stable layout captures once and never render-loops.
    fn anchored_trigger(
        &self,
        which: Anchor,
        button: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let probe = cx.entity().downgrade();
        let idx = which.idx();
        div().relative().child(button).child(
            canvas(
                move |bounds, _window, app| {
                    probe
                        .update(app, |this, cx| {
                            let x = f32::from(bounds.origin.x);
                            if (this.anchor_x[idx] - x).abs() > 0.5 {
                                this.anchor_x[idx] = x;
                                cx.notify();
                            }
                        })
                        .ok();
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }

    fn render_action_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every mutating control disables in degraded/read-only mode (`functional_spec.md §6`).
        let disabled = self.degraded;

        // Each button renders a FreeCell-vendored Lucide icon (`shell::assets`) via
        // gpui-component's `Icon` (`icons/<name>.svg`); `Icon` tints it to the button's
        // foreground so the pressed/disabled states read the same as the former text glyphs.
        let toggle = |id: &'static str,
                      icon_path: &'static str,
                      tooltip: &'static str,
                      pressed: bool,
                      attr: StyleAttr,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(pressed)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.toggle_style(attr, window, cx);
                }))
        };

        // An alignment toggle (pressed = the cell's *explicit* alignment).
        let align_btn = |id: &'static str,
                         tooltip: &'static str,
                         align: Align,
                         icon_path: &'static str,
                         cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(self.align_active(align))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_alignment(align, window, cx);
                }))
        };

        // A vertical-alignment button (pressed = the cell's explicit vertical alignment). Mirrors
        // `align_btn` but drives the vertical group (`ui_design.md §1.1`).
        let valign_btn = |id: &'static str,
                          tooltip: &'static str,
                          valign: VAlign,
                          icon_path: &'static str,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(self.valign_active(valign))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_valign(valign, window, cx);
                }))
        };

        // The button groups are the horizontal scroller's *content*: they sit at their exact
        // natural width so a small window makes them overflow + scroll (chevrons) rather than
        // compressing the controls (`functional_spec.md §9B`, call site 1). `flex_shrink_0` is what
        // holds that natural width — flexbox's default shrink=1 would otherwise squish the buttons
        // to fit; it (not a hand-estimated `min_w`) is the "scroll, don't squish" guarantee, so the
        // chevrons appear ONLY when the controls genuinely don't fit (`functional_spec.md §10.2`).
        let groups = div()
            .flex()
            .items_center()
            .gap_1()
            .debug_selector(|| "action-row-groups".to_string())
            // Never wrap or shrink; the scroller scrolls the groups when they don't fit.
            .flex_shrink_0()
            // Font family · size (`ui_design.md §2`):
            .child(
                self.anchored_trigger(
                    Anchor::FontFamily,
                    Button::new("font-family")
                        .label(format!("{} ▾", self.font_family_label()))
                        .tooltip("Font")
                        .ghost()
                        .small()
                        .w(px(140.0))
                        .disabled(disabled)
                        .selected(self.font_family_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_font_family_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                self.anchored_trigger(
                    Anchor::FontSize,
                    Button::new("font-size")
                        .label(format!("{} ▾", self.font_size_label()))
                        .tooltip("Font size")
                        .ghost()
                        .small()
                        .w(px(56.0))
                        .disabled(disabled)
                        .selected(self.font_size_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_font_size_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // B I U:
            .child(toggle(
                "bold",
                "icons/bold.svg",
                "Bold ⌘B",
                self.bold_active(),
                StyleAttr::Bold,
                cx,
            ))
            .child(toggle(
                "italic",
                "icons/italic.svg",
                "Italic ⌘I",
                self.italic_active(),
                StyleAttr::Italic,
                cx,
            ))
            .child(toggle(
                "underline",
                "icons/underline.svg",
                "Underline ⌘U",
                self.underline_active(),
                StyleAttr::Underline,
                cx,
            ))
            // Strikethrough, appended to the B/I/U toggle group
            // (`ui_design.md §1.1`, `functional_spec.md §1`).
            .child(toggle(
                "strikethrough",
                "icons/strikethrough.svg",
                "Strikethrough",
                self.strikethrough_active(),
                StyleAttr::Strikethrough,
                cx,
            ))
            .child(action_divider())
            // Text color · Fill:
            .child(
                self.anchored_trigger(
                    Anchor::TextColor,
                    Button::new("text-color")
                        .icon(Icon::empty().path("icons/baseline.svg"))
                        .label("▾")
                        .tooltip("Text color")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.text_color_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_text_color_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                self.anchored_trigger(
                    Anchor::Fill,
                    Button::new("fill")
                        .icon(Icon::empty().path("icons/paint-bucket.svg"))
                        .label("▾")
                        .tooltip("Fill color")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.fill_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_fill_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // Borders preset popover:
            .child(
                self.anchored_trigger(
                    Anchor::Borders,
                    Button::new("borders")
                        .icon(Icon::empty().path("icons/grid-2x2.svg"))
                        .label("▾")
                        .tooltip("Borders")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.borders_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_borders_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // Alignment L / C / R:
            .child(align_btn(
                "align-left",
                "Align left",
                Align::Left,
                "icons/text-align-start.svg",
                cx,
            ))
            .child(align_btn(
                "align-center",
                "Align center",
                Align::Center,
                "icons/text-align-center.svg",
                cx,
            ))
            .child(align_btn(
                "align-right",
                "Align right",
                Align::Right,
                "icons/text-align-end.svg",
                cx,
            ))
            .child(action_divider())
            // Vertical alignment — its own group after horizontal align (`ui_design.md §1.1`):
            .child(valign_btn(
                "valign-top",
                "Align top",
                VAlign::Top,
                "icons/arrow-up-to-line.svg",
                cx,
            ))
            .child(valign_btn(
                "valign-middle",
                "Align middle",
                VAlign::Center,
                "icons/separator-horizontal.svg",
                cx,
            ))
            .child(valign_btn(
                "valign-bottom",
                "Align bottom",
                VAlign::Bottom,
                "icons/arrow-down-from-line.svg",
                cx,
            ))
            // Wrap text — grouped with vertical alignment, right of Align bottom.
            .child(toggle(
                "wrap",
                "icons/text-wrap.svg",
                "Wrap text",
                self.wrap_active(),
                StyleAttr::WrapText,
                cx,
            ))
            .child(action_divider())
            // Number format dropdown + decimals ±:
            .child(
                self.anchored_trigger(
                    Anchor::NumFmt,
                    Button::new("num-fmt")
                        .label(format!("{} ▾", self.num_fmt_category_label()))
                        .tooltip("Number format")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.num_fmt_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_num_fmt_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                Button::new("decimals-inc")
                    .icon(Icon::empty().path("icons/decimals-arrow-right.svg"))
                    .tooltip("Increase decimals")
                    .ghost()
                    .small()
                    .disabled(!self.increase_decimals_enabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.bump_decimals(1, window, cx);
                    })),
            )
            .child(
                Button::new("decimals-dec")
                    .icon(Icon::empty().path("icons/decimals-arrow-left.svg"))
                    .tooltip("Decrease decimals")
                    .ghost()
                    .small()
                    .disabled(!self.decrease_decimals_enabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.bump_decimals(-1, window, cx);
                    })),
            )
            // Thousands-separator toggle (Phase 6, D6.2): adds/removes the `,` grouping on the
            // active cell's format; `selected` when grouping is on, disabled when it can't be
            // safely toggled (General/Text/Date/Time/multi-section).
            .child(
                Button::new("thousands-sep")
                    .icon(Icon::empty().path("icons/thousands-separator.svg"))
                    .tooltip("Thousands separator")
                    .ghost()
                    .small()
                    .disabled(!self.toggle_thousands_enabled())
                    .selected(self.thousands_active())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_thousands_separator(window, cx);
                    })),
            )
            .child(action_divider())
            // Insert-chart menu — the action-bar chart-type glyph menu (`ui_design.md §3.1`, P17).
            .child(
                self.anchored_trigger(
                    Anchor::Chart,
                    Button::new("insert-chart")
                        .icon(Icon::empty().path("icons/chart-column.svg"))
                        .label("▾")
                        .tooltip("Insert chart")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.chart_menu_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_chart_menu(cx);
                        })),
                    cx,
                ),
            )
            // Conditional formatting — toggles the right-docked CF sidebar directly (no menu, unlike
            // the chart-insert glyph). `selected` (accent) while the sidebar is open; disabled in
            // degraded/read-only mode (`components/cf_sidebar.md §5`, `ui_design.md §1`).
            .child(
                Button::new("cond-fmt")
                    .icon(Icon::empty().path("icons/split.svg"))
                    .tooltip("Conditional formatting")
                    .ghost()
                    .small()
                    .disabled(disabled)
                    .selected(self.cond_fmt_open())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_cond_fmt_sidebar(cx);
                    })),
            )
            .child(action_divider())
            // Find & Replace trigger (`ui_design.md §2`): toggles the find bar; `selected` (accent)
            // while it is open, so it reads as a toggle. `icons/search.svg` resolves from the
            // gpui-component bundle (the magnifier the bundle already ships + tints).
            .child(
                // Find is a *read* — it stays available in degraded/read-only mode (only the bar's
                // Replace / Replace All are gated on `degraded`).
                Button::new("find")
                    .icon(Icon::empty().path("icons/search.svg"))
                    .tooltip("Find & Replace (⌘F)")
                    .ghost()
                    .small()
                    .selected(self.find_open)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_find(window, cx);
                    })),
            );

        // Frame: the fixed-height, full-width action bar hosting the scrollable groups + the
        // right-docked evaluating spinner (`ui_design.md §3.1`). The scroller is `flex_1`, so it
        // fills up to the spinner — the spinner stays docked right exactly as the old `flex_1`
        // spacer did — and shows chevrons only when the groups overflow (`functional_spec.md §9B`).
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(ACTION_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            .child(h_scroller(
                "action-row",
                &self.action_scroller,
                window,
                groups,
            ))
            .when(self.eval.spinner(), |row| row.child(Spinner::new().small()))
    }

    fn render_data_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.data_row.mode() == FieldMode::Disabled;
        let cap_error = self.cap_error_visible();

        // Inset the entry to `DATA_ROW_H - 4` so the row's `items_center` leaves 2 px above and
        // below it (BUG C), without shrinking the 32 px bar. gpui-component's single-line `Input`
        // pins a fixed 32 px control height (`Size::Medium` → `h_8`) that otherwise fills the row
        // edge-to-edge; `Input::h()` is multi-line-only, so pin the single-line control via
        // `min_h`/`max_h` (applied after `input_h` through `refine_style`). The 20 px line box fits
        // the 28 px control, so the normal-size text stays centered and un-clipped.
        let mut content = Input::new(&self.content_input)
            .disabled(disabled)
            .w_full()
            .min_h(px(DATA_ROW_FIELD_H))
            .max_h(px(DATA_ROW_FIELD_H));
        if self.fetch_spinner_visible() {
            content = content.suffix(Spinner::new().small());
        }

        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .h(px(DATA_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            // Escape reverts the edit (the InputState propagates Escape up to here).
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.escape_edit(window, cx);
                }
            }))
            // Tab / Shift+Tab commit + move right/left (`functional_spec.md §1.4`), and — in
            // quick-edit — the unmodified arrows commit + move the active cell while Home/End or a
            // modified arrow leave quick-edit (`functional_spec.md §5.2–5.3`). These are handled by
            // the keystroke interceptor registered in [`ChromeView::new`], NOT a `capture_key_down`
            // here: the gpui-component `Input` binds Left/Right to caret actions that dispatch
            // before any key-down listener and stop propagation, so only an interceptor (which runs
            // before action bindings) can preempt them (`components/edit_controller.md §Tab
            // interception`; `feature-gaps-7-11/DECISIONS_TO_REVIEW.md`).
            // Ref box: read-only A1 address.
            .child(
                div()
                    .w(px(REF_BOX_W))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .text_size(px(12.0))
                    .text_color(rgb(TEXT))
                    .child(self.ref_box_text()),
            )
            .child(div().w(px(1.0)).h(px(20.0)).bg(rgb(DIVIDER)))
            // Content field (danger border on cap reject). The row's `items_center` centers this
            // (input-height) field so the 28 px entry sits 2 px inside the 32 px bar (BUG C).
            .child(
                div()
                    .flex_1()
                    .debug_selector(|| "data-content-field".into())
                    // Clicking to place the caret in the field ends quick-edit (`functional_spec.md
                    // §5.3`): arrows then move the caret, not the active cell. The gpui-component
                    // `Input` does not `stop_propagation` on mouse-down, so this bubble-phase
                    // listener still fires on a click into the field.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.leave_quick_edit(window, cx);
                        }),
                    )
                    .when(cap_error, |d| {
                        d.border_1().border_color(rgb(DANGER)).rounded_md()
                    })
                    .child(content),
            )
    }

    /// The find/replace bar (`functional_spec.md §4.1`, `ui_design.md §1`) — rendered directly below
    /// the data row while [`find_open`](Self::find_open). Left→right: find field · replace field ·
    /// match-case + match-entire-cell toggles · divider · prev/next · counter · spacer · Replace +
    /// Replace All · dismiss. Escape (on the bar) closes it (mirrors the data row's Escape).
    fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_matches = !self.matches.is_empty();
        let has_current = self.match_idx.is_some();
        let can_replace = has_current && !self.degraded;
        let can_replace_all = has_matches && !self.degraded;

        // Counter (`ui_design.md §1`): a Replace All notice wins; else empty for an empty find field,
        // "No results" (muted) for a non-empty query with no matches, else "N of M".
        let find_query = self.find_input.read(cx).value().to_string();
        let (counter_text, counter_muted) = if let Some(n) = self.replaced_notice {
            (format!("Replaced {n}"), true)
        } else if find_query.is_empty() {
            (String::new(), false)
        } else if !has_matches {
            ("No results".to_string(), true)
        } else {
            let pos = self.match_idx.map(|i| i + 1).unwrap_or(0);
            (format!("{pos} of {}", self.matches.len()), false)
        };

        // A small ghost toggle (`Aa` / match-entire-cell), pressed = on.
        let toggle =
            |id: &'static str,
             label: &'static str,
             tooltip: &'static str,
             on: bool,
             cx: &mut Context<Self>,
             handler: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                Button::new(id)
                    .label(label)
                    .tooltip(tooltip)
                    .ghost()
                    .small()
                    .selected(on)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        handler(this, window, cx);
                    }))
            };

        div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(DATA_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            .debug_selector(|| "find-bar".into())
            // Escape closes the bar and returns focus to the grid (`functional_spec.md §4.2`), the
            // same idiom as the data row's Escape.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_find(window, cx);
                }
            }))
            .child(
                div()
                    .w(px(FIND_FIELD_W))
                    .child(Input::new(&self.find_input).small()),
            )
            .child(
                div()
                    .w(px(FIND_FIELD_W))
                    .child(Input::new(&self.replace_input).small()),
            )
            .child(toggle(
                "find-match-case",
                "Aa",
                "Match case",
                self.match_case,
                cx,
                Self::toggle_match_case,
            ))
            .child(toggle(
                "find-whole-cell",
                "Whole cell",
                "Match entire cell",
                self.whole_cell,
                cx,
                Self::toggle_whole_cell,
            ))
            .child(action_divider())
            .child(
                Button::new("find-prev")
                    .icon(Icon::empty().path("icons/chevron-up.svg"))
                    .tooltip("Previous match (⇧⏎)")
                    .ghost()
                    .small()
                    .disabled(!has_matches)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.prev_match(window, cx);
                    })),
            )
            .child(
                Button::new("find-next")
                    .icon(Icon::empty().path("icons/chevron-down.svg"))
                    .tooltip("Next match (⏎)")
                    .ghost()
                    .small()
                    .disabled(!has_matches)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.next_match(window, cx);
                    })),
            )
            .child(
                div()
                    .min_w(px(FIND_COUNTER_MIN_W))
                    .text_size(px(13.0))
                    .text_color(rgb(if counter_muted {
                        FIND_COUNTER_MUTED
                    } else {
                        TEXT
                    }))
                    .child(counter_text),
            )
            .child(div().flex_1())
            .child(
                Button::new("find-replace")
                    .label("Replace")
                    .tooltip("Replace this match")
                    .ghost()
                    .small()
                    .disabled(!can_replace)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.replace_current(window, cx);
                    })),
            )
            .child(
                Button::new("find-replace-all")
                    .label("Replace All")
                    .tooltip("Replace every match")
                    .ghost()
                    .small()
                    .disabled(!can_replace_all)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.replace_all(window, cx);
                    })),
            )
            .child(
                close_button(
                    "find-close",
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.close_find(window, cx);
                    }),
                )
                .tooltip("Close (Esc)"),
            )
    }

    // ---- Find / replace behavior (`functional_spec.md §4`) --------------------------------

    /// Toggle the find bar open/closed (⌘F, Esc, or the action-row search button).
    pub fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open {
            self.close_find(window, cx);
        } else {
            self.open_find(window, cx);
        }
    }

    /// Open the bar and focus the find field, retaining any prior find/replace text
    /// (`functional_spec.md §4.2`). A recompute picks up retained text so the counter is live on open.
    /// Existing find text is **selected** on open (`§4.2`) by dispatching gpui-component's `SelectAll`
    /// to the field's focus handle **after the next paint** (`on_next_frame`) — the field must be in
    /// the rendered dispatch tree for the action to reach it (a `defer` runs before the repaint, so it
    /// would fizzle on a freshly-opened bar).
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        self.replaced_notice = None;
        self.find_input
            .update(cx, |input, cx| input.focus(window, cx));
        if !self.find_query(cx).is_empty() {
            let handle = self.find_input.read(cx).focus_handle(cx);
            window.on_next_frame(move |window, cx| {
                handle.dispatch_action(&gpui_component::input::SelectAll, window, cx);
            });
        }
        self.recompute_matches(cx);
        cx.notify();
    }

    /// Close the bar, clear the transient match state, and return focus to the grid; the
    /// find/replace **text** is retained for the next open (`functional_spec.md §4.2`).
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        self.find_open = false;
        self.matches.clear();
        self.match_idx = None;
        self.replaced_notice = None;
        self.pending_replace_all = false;
        self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
        cx.notify();
    }

    /// Whether the find bar is open (window action-handler / tests).
    pub fn find_is_open(&self) -> bool {
        self.find_open
    }

    /// The current find-field text.
    fn find_query(&self, cx: &Context<Self>) -> String {
        self.find_input.read(cx).value().to_string()
    }

    /// Send a `Find` for the current query + toggles (results arrive via `FindResults`). An empty
    /// query clears the local match state (no worker round-trip).
    fn recompute_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query(cx);
        if query.is_empty() {
            self.matches.clear();
            self.match_idx = None;
            cx.notify();
            return;
        }
        self.client.send(Command::Find {
            sheet: self.active_sheet,
            query,
            match_case: self.match_case,
            whole_cell: self.whole_cell,
        });
    }

    /// Re-scope an **open** find bar to the (already-updated) active sheet: reset the match cursor and
    /// re-send `Find` for the new sheet (`functional_spec.md §4.5`). Called from the sheet-switch
    /// entry points after `active_sheet` changes.
    fn rescope_find_if_open(&mut self, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        self.match_idx = None;
        self.matches.clear();
        self.replaced_notice = None;
        self.recompute_matches(cx);
    }

    /// The `CellRef` of the current match, if any.
    fn current_match_cell(&self) -> Option<CellRef> {
        self.match_idx.and_then(|i| self.matches.get(i).copied())
    }

    /// The index of the first match at or after the current selection (row-major), wrapping to the
    /// first match — so opening / recomputing lands on the nearest match ahead of the cursor.
    fn first_match_from_selection(&self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let key = (self.selection.active.row, self.selection.active.col);
        let idx = self
            .matches
            .iter()
            .position(|c| (c.row, c.col) >= key)
            .unwrap_or(0);
        Some(idx)
    }

    /// Advance to the next match, wrapping around (`functional_spec.md §4.3`).
    fn next_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.replaced_notice = None;
        let n = self.matches.len();
        let i = self.match_idx.map(|i| (i + 1) % n).unwrap_or(0);
        self.match_idx = Some(i);
        self.select_current_match(window, cx);
        cx.notify();
    }

    /// Retreat to the previous match, wrapping around (`functional_spec.md §4.3`).
    fn prev_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.replaced_notice = None;
        let n = self.matches.len();
        let i = self.match_idx.map(|i| (i + n - 1) % n).unwrap_or(n - 1);
        self.match_idx = Some(i);
        self.select_current_match(window, cx);
        cx.notify();
    }

    /// Select + scroll the current match into view (the find field keeps focus).
    fn select_current_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cell) = self.current_match_cell() {
            // grid.md invariant: commit any pending data-row edit BEFORE emitting the deferred
            // reveal — otherwise the deferred `SelectAndReveal` would drive `on_selection_changed`
            // while the field is Editing. A cap-rejected edit blocks navigation (keep editing).
            if !self.on_edit_commit_requested(window, cx) {
                return;
            }
            self.grid
                .emit(ChromeGridRequest::SelectAndReveal(cell), window, cx);
        }
    }

    /// Toggle match-case and recompute matches.
    fn toggle_match_case(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.match_case = !self.match_case;
        self.replaced_notice = None;
        self.recompute_matches(cx);
        cx.notify();
    }

    /// Toggle match-entire-cell and recompute matches.
    fn toggle_whole_cell(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.whole_cell = !self.whole_cell;
        self.replaced_notice = None;
        self.recompute_matches(cx);
        cx.notify();
    }

    /// Replace the current match (`Command::ReplaceOne`, `functional_spec.md §4.4`): the worker
    /// recomputes the replacement from fresh content and commits it; a follow-up `ReplacedCount`
    /// re-runs `Find` so the cursor advances past the (now-changed) cell.
    fn replace_current(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.degraded {
            return;
        }
        let Some(cell) = self.current_match_cell() else {
            return;
        };
        let query = self.find_query(cx);
        if query.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).value().to_string();
        self.client.send(Command::ReplaceOne {
            sheet: self.active_sheet,
            cell,
            query,
            replacement,
            match_case: self.match_case,
            whole_cell: self.whole_cell,
        });
    }

    /// Replace every match (`Command::ReplaceAll`, `functional_spec.md §4.4`); the `ReplacedCount`
    /// reply shows "Replaced N" and re-runs `Find`.
    fn replace_all(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.degraded || self.matches.is_empty() {
            return;
        }
        let query = self.find_query(cx);
        if query.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).value().to_string();
        self.pending_replace_all = true;
        self.client.send(Command::ReplaceAll {
            sheet: self.active_sheet,
            query,
            replacement,
            match_case: self.match_case,
            whole_cell: self.whole_cell,
        });
    }

    /// The find field emitted an event: typing recomputes matches; Enter / Shift+Enter step
    /// next / prev (`ui_design.md §1`).
    fn on_find_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.replaced_notice = None;
                self.recompute_matches(cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                if *shift {
                    self.prev_match(window, cx);
                } else {
                    self.next_match(window, cx);
                }
            }
            _ => {}
        }
    }

    /// The replace field emitted an event: Enter replaces the current match.
    fn on_replace_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter { .. } = event {
            self.replace_current(window, cx);
        }
    }

    fn render_tab_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dragging = self.tab_drag_active();
        let mut row = div()
            // `relative` so the drop indicator (an absolute child, positioned in window x — the
            // tab bar's origin x is 0) lands in the right gap.
            .relative()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(TAB_BAR_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_t_1()
            .border_color(rgb(HAIRLINE))
            // The move / up handlers live on the full-width container, not the individual tabs: a
            // per-tab `on_mouse_move` only fires while *that* tab is hovered, so it would go dead
            // the instant the pointer crossed onto a neighbor mid-drag. The container spans the
            // whole strip, so it tracks the drag across tabs and the release anywhere in the bar.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                this.tab_drag_move(f32::from(event.position.x), cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.tab_drag_end(f32::from(event.position.x), cx);
                }),
            )
            // `grabbing` while a reorder drag is live (`ui_design.md §6`).
            .when(dragging, |d| d.cursor(CursorStyle::ClosedHand));

        // The tabs + the "new sheet" button are the horizontal scroller's *content* — a long tab
        // strip scrolls (chevrons) instead of pushing the stats group off-screen (`functional_spec.md
        // §9B`, call site 2 → §9A.4). They keep their natural width and left-align.
        let mut tabs = div().flex().items_center().gap_1();
        for tab in &self.sheets {
            tabs = tabs.child(self.render_tab(tab, cx));
        }
        tabs = tabs.child(
            Button::new("add-sheet")
                .label("+")
                .tooltip("New sheet")
                .ghost()
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.add_sheet();
                    cx.notify();
                })),
        );

        // The scroller (flex_1) fills the row up to the *static* right section: a leading divider
        // (§9A.3 — only when the readout is shown, so it never floats alone) + the selection-stats
        // group (§9A.4 — pinned right, outside the scroller, so a long tab strip can't push it off).
        row = row.child(h_scroller("tab-bar", &self.tab_scroller, window, tabs));
        if self.stats_readout_parts().is_some() {
            row = row.child(action_divider());
        }
        row = row.child(self.render_selection_stats(cx));

        // The 2 px accent drop indicator at the insertion gap while dragging (`ui_design.md §3`).
        if let Some(x) = self.tab_drop_indicator_x() {
            row = row.child(
                div()
                    .absolute()
                    .left(px(x - 1.0))
                    .top_0()
                    .h_full()
                    .w(px(2.0))
                    .bg(rgb(TAB_DROP_ACCENT)),
            );
        }

        row
    }

    /// The right-aligned selection-stats readout in the tab bar (`functional_spec.md §1`). Empty
    /// when hidden (single-cell / all-text / empty selection) so the row's height stays stable;
    /// when shown, the whole group is clickable to toggle the Min / Max expansion.
    fn render_selection_stats(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut group = div()
            .id("selection-stats")
            .debug_selector(|| "selection-stats".into())
            .flex()
            .items_center()
            // Fill the bar height + track the line-height to it, so the readout sits vertically
            // centered in `TAB_BAR_H` rather than hugging the text box (`functional_spec.md §9A.2`).
            .h_full()
            .line_height(px(TAB_BAR_H))
            .gap_4()
            .pr_1()
            .text_size(px(12.0))
            .text_color(rgb(MUTED_TEXT));
        if let Some(parts) = self.stats_readout_parts() {
            group = group.cursor_pointer().on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.toggle_stats_minmax(cx);
                },
            ));
            for part in parts {
                group = group.child(div().child(part));
            }
        }
        group
    }

    fn render_tab(&self, tab: &SheetTab, cx: &mut Context<Self>) -> gpui::AnyElement {
        let id = tab.id;
        let is_active = id == self.active_sheet;

        if self.rename_target == Some(id) {
            // Inline rename input in the tab's footprint.
            return div()
                .w(px(100.0))
                .when(self.rename_error, |d| {
                    d.border_1().border_color(rgb(DANGER)).rounded_md()
                })
                .child(Input::new(&self.rename_input).small())
                .into_any_element();
        }

        // The dragged tab lifts while a reorder drag is live on it (`ui_design.md §3`): stronger
        // bg, a 1 px accent outline, ~90 % opacity.
        let lifted = self.tab_drag.is_some_and(|d| d.dragging && d.sheet == id);
        // A per-tab zero-cost `canvas` probe records the tab's window-space span into `tab_spans`
        // each paint — the geometry the pure insertion computation reads. No `notify` (the value
        // is consumed on the next mouse event, not this frame), so it never render-loops.
        let probe = cx.entity().downgrade();
        let span_probe = canvas(
            move |bounds, _window, app| {
                probe
                    .update(app, |this, _cx| {
                        let left = f32::from(bounds.origin.x);
                        let right = left + f32::from(bounds.size.width);
                        if let Some(span) = this.tab_spans.iter_mut().find(|s| s.sheet == id) {
                            span.left = left;
                            span.right = right;
                        } else {
                            this.tab_spans.push(TabSpan {
                                sheet: id,
                                left,
                                right,
                            });
                        }
                    })
                    .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .id(gpui::ElementId::Name(format!("tab-{}", id.0).into()))
            // `relative` so the span probe (`absolute().size_full()`) fills the tab exactly.
            .relative()
            .px_3()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded_t_md()
            .bg(rgb(if is_active || lifted {
                ACTIVE_TAB_BG
            } else {
                CHROME_BG
            }))
            .text_size(px(13.0))
            .text_color(rgb(if is_active { TEXT } else { MUTED_TEXT }))
            .when(is_active && !lifted, |d| {
                d.border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_color(rgb(HAIRLINE))
            })
            .when(lifted, |d| {
                d.border_1().border_color(rgb(TAB_DROP_ACCENT)).opacity(0.9)
            })
            .child(tab.name.clone())
            .child(span_probe)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.rename_start(id, window, cx);
                } else {
                    this.select_sheet(id, window, cx);
                }
            }))
            // Record a potential drag; movement past the threshold (tracked on the container) turns
            // it into a real drag. No `stop_propagation`, so the `on_click` above still forms for a
            // plain click / double-click (gpui gates that click on releasing over this same tab, so
            // a real drag — which releases over a different tab — never fires it).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, _cx| {
                    this.tab_press(id, f32::from(event.position.x));
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                    this.open_context_menu(id, cx);
                }),
            )
            .into_any_element()
    }

    /// The floating overlays (fill popover, tab context menu, delete-confirm modal), each a
    /// `ChromeView`-owned panel over a dismiss backdrop.
    fn render_overlays(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut overlays: Vec<gpui::AnyElement> = Vec::new();

        // The right-docked chart edit panel is pushed FIRST so it is the **bottom-most** overlay:
        // gpui paints sibling overlays in vector order (later = on top), so every action-bar
        // dropdown/popover below — the new-chart menu in particular (post-v1 Batch 3, item 10) —
        // floats ABOVE the docked panel instead of dropping behind it. The panel is a persistent
        // docked surface; the transient popovers layer on top of it.
        if self.chart_panel.is_some() {
            overlays.push(self.render_chart_panel(cx));
        }
        // The CF sidebar shares the right dock and is mutually exclusive with the chart panel, so at
        // most one of these ever renders; like the chart panel it is a persistent docked surface, so
        // it is pushed early to stay below the transient action-bar popovers.
        if self.cond_fmt.is_some() {
            overlays.push(self.render_cond_fmt_sidebar(cx));
        }

        // The data-row cap popover anchors under the data row only when it is the active editor;
        // an in-cell cap error is shown under the overlay by the grid (`edit_controller.md §4.2`).
        // The completion list + signature hint anchor at the same spot but yield to the cap error
        // (it means the edit can't commit — `functional_spec.md §1`).
        if self.edit.origin() == EditOrigin::DataRow {
            if let Some(message) = self.cap_error_message() {
                overlays.push(self.render_cap_error_popover(message));
            } else if let Some(list) = self.autocomplete_display() {
                overlays.push(self.render_autocomplete_popover(&list, cx));
            } else if let Some(template) = self.sig_hint {
                overlays.push(self.render_sig_hint_popover(template));
            }
        }
        if self.fill_open {
            overlays.push(self.render_fill_popover(cx));
        }
        if self.text_color_open {
            overlays.push(self.render_text_color_popover(cx));
        }
        if self.num_fmt_open {
            overlays.push(self.render_num_fmt_popover(cx));
        }
        if self.chart_menu_open {
            overlays.push(self.render_chart_menu(cx));
        }
        if self.font_family_open {
            overlays.push(self.render_font_family_popover(cx));
        }
        if self.font_size_open {
            overlays.push(self.render_font_size_popover(cx));
        }
        if self.borders_open {
            overlays.push(self.render_borders_popover(cx));
        }
        if let Some(id) = self.context_menu {
            overlays.push(self.render_context_menu(id, cx));
        }
        if let Some(id) = self.confirm_delete {
            overlays.push(self.render_delete_confirm(id, cx));
        }
        overlays
    }

    fn backdrop(
        &self,
        on_dismiss: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Occlude the grid behind the popover: `BlockMouse` makes every hitbox behind this one
            // (the grid) un-hovered and un-scrollable, so a click on the overlay no longer also
            // moves the grid selection (BUG 2a) and scrolling anywhere over it no longer scrolls the
            // grid underneath (BUG 2b). The popover card, painted *after* this backdrop, still gets
            // its own clicks/scroll (it is in front, not behind).
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    on_dismiss(this, window, cx);
                }),
            )
    }

    /// The cap-error popover (`functional_spec.md §4.2`, `ui_design.md §4`): a small dark
    /// tooltip anchored just below the data-row content field's left edge. No backdrop — it
    /// auto-dismisses on the next keystroke (reducer clears its rejection) or focus change.
    fn render_cap_error_popover(&self, message: String) -> gpui::AnyElement {
        div()
            .absolute()
            .top(px(ACTION_ROW_H + DATA_ROW_H + 2.0))
            .left(px(DATA_ROW_CONTENT_LEFT))
            .px_2()
            .py_1()
            .bg(rgb(TOOLTIP_BG))
            .text_color(rgb(TOOLTIP_TEXT))
            .text_size(px(11.0))
            .rounded_md()
            .shadow_md()
            .whitespace_nowrap()
            .child(message)
            .into_any_element()
    }

    /// The function-completion dropdown under the data-row field (`functional_spec.md §1`): a
    /// passive (no-backdrop) list anchored below the content entry, capped to ~10 rows with
    /// internal scroll. Each row accepts on click; the highlighted row is tinted. Mirrors the
    /// in-cell list the grid draws from the same [`AutocompleteDisplay`].
    fn render_autocomplete_popover(
        &self,
        list: &AutocompleteDisplay,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id("autocomplete-list")
            .absolute()
            .top(px(ACTION_ROW_H + DATA_ROW_H + 2.0))
            .left(px(DATA_ROW_CONTENT_LEFT))
            .occlude()
            .debug_selector(|| "autocomplete-list".into())
            .flex()
            .flex_col()
            .min_w(px(AUTOCOMPLETE_MIN_W))
            .max_h(px(320.0))
            .overflow_y_scroll()
            .bg(rgb(ACTIVE_TAB_BG))
            .border_1()
            .border_color(rgb(HAIRLINE))
            .rounded_md()
            .shadow_md()
            .children(
                list.rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| self.autocomplete_row(i, row, i == list.highlight, cx)),
            )
            .into_any_element()
    }

    /// One completion row (shared shape with the grid's in-cell list): name + argument template,
    /// tinted when highlighted, accepting on click.
    fn autocomplete_row(
        &self,
        index: usize,
        row: &AutocompleteRow,
        highlighted: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(gpui::ElementId::Name(
                format!("autocomplete-row-{index}").into(),
            ))
            .flex()
            .items_baseline()
            .gap_2()
            .px_2()
            .py(px(2.0))
            .when(highlighted, |d| d.bg(rgb(AUTOCOMPLETE_HL_BG)))
            // Hover highlights a row too (`functional_spec.md §1`, Mouse), matching the keyboard tint.
            .hover(|s| s.bg(rgb(AUTOCOMPLETE_HL_BG)))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(TEXT))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED_TEXT))
                    .whitespace_nowrap()
                    .child(row.template.clone()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.autocomplete_accept_at(index, window, cx);
                }),
            )
    }

    /// The passive one-line signature hint under the data-row field (D1.1 — the whole template,
    /// no current-arg tracking). Shown only when the list is not covering the same anchor.
    fn render_sig_hint_popover(&self, template: &str) -> gpui::AnyElement {
        div()
            .absolute()
            .top(px(ACTION_ROW_H + DATA_ROW_H + 2.0))
            .left(px(DATA_ROW_CONTENT_LEFT))
            .px_2()
            .py_1()
            .bg(rgb(ACTIVE_TAB_BG))
            .text_color(rgb(MUTED_TEXT))
            .text_size(px(11.0))
            .border_1()
            .border_color(rgb(HAIRLINE))
            .rounded_md()
            .shadow_md()
            .whitespace_nowrap()
            .child(template.to_string())
            .into_any_element()
    }

    fn render_fill_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // 5×2 swatch grid.
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("swatch-{}", swatch.name).into(),
                        ))
                        .debug_selector(|| format!("fill-swatch-{}", swatch.name))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_fill(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.fill_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Fill.idx()]))
                    // Occlude the card so a mouse-down on it can't reach the backdrop's dismiss
                    // listener painted behind it (BUG A/B): the card's `BlockMouse` hitbox drops
                    // the backdrop out of the hit-test under the pointer, so `is_hovered` is false
                    // there and the backdrop's `on_mouse_down` never fires. Without this, clicking
                    // an item dismissed the popover on mouse-DOWN, tearing it down before the item's
                    // `on_click` (mouse-UP) could apply. Items inside paint above the card, so their
                    // own clicks are unaffected; a click OUTSIDE the card still dismisses.
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("no-fill")
                            .label("No fill")
                            .debug_selector(|| "fill-no-fill".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_fill(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The text-color popover: the same palette as Fill, with **Automatic** (clear) in place of
    /// "No fill" (`components/action_bar.md`, `ui_design.md §2`).
    fn render_text_color_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("text-swatch-{}", swatch.name).into(),
                        ))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_text_color(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.text_color_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::TextColor.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("text-automatic")
                            .label("Automatic")
                            .debug_selector(|| "text-automatic".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_text_color(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.text_color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The number-format dropdown (`functional_spec.md §10.1`, D10.1). Basics-first: the default
    /// view is the seven [`BASIC_FORMATS`] flat (no scroll) plus a trailing "More ▸" row; drilling
    /// in ([`num_fmt_more_open`](Self::num_fmt_more_open)) swaps to the full grouped
    /// [`NUM_FMT_GROUPS`] inventory with a "◂ Back" row. The preset matching the active cell's exact
    /// format code is highlighted at whichever level it lives; when the active format is a More-only
    /// preset the "More ▸" row is marked active (and the dropdown opens straight onto the grouped
    /// view — see [`toggle_num_fmt_popover`](Self::toggle_num_fmt_popover)).
    ///
    /// **Drill-in over flyout (D10.1):** the popover is a single fixed-anchor occluded card over one
    /// full-screen backdrop; a flyout would need a second card anchored to the dynamically-positioned
    /// "More ▸" row (offset + card width unknown without measurement), so drill-in — reusing the same
    /// card/backdrop/occlude/dismiss machinery — is the clean fit.
    fn render_num_fmt_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Highlight the preset whose code exactly matches the active cell's, normalizing `general`
        // case (the engine may echo "General"). Presets can share a category but never a code, so an
        // exact match selects at most one preset (at whichever level it lives).
        let active_code = self.num_fmt_active_code();
        let body = if self.num_fmt_more_open {
            self.num_fmt_more_menu(&active_code, cx)
        } else {
            self.num_fmt_basic_menu(&active_code, cx)
        };

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.num_fmt_open = false;
                        this.num_fmt_more_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("numfmt-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::NumFmt.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .debug_selector(|| "numfmt-card".into())
                    .flex()
                    .flex_col()
                    .p_1()
                    // The grouped "More" inventory is tall — cap the height and scroll it (like the
                    // font-family popover). The basic list is short and never scrolls.
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(body),
            )
            .into_any_element()
    }

    /// The basics-first view: the seven [`BASIC_FORMATS`] flat, then a trailing "More ▸" row that
    /// drills into the full grouped inventory. `active_code` is the normalized active-cell format.
    fn num_fmt_basic_menu(&self, active_code: &str, cx: &mut Context<Self>) -> gpui::Div {
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for preset in BASIC_FORMATS {
            let code = preset.code.to_string();
            let selector = preset.code;
            menu = menu.child(
                Button::new(gpui::ElementId::Name(
                    format!("numfmt-{}", preset.code).into(),
                ))
                .label(preset.label)
                .debug_selector(move || format!("numfmt-{selector}"))
                .ghost()
                .small()
                .selected(preset.code == active_code)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_num_fmt(&code, window, cx);
                })),
            );
        }
        // "More ▸" reveals the full grouped inventory (drill-in). Marked active when the active
        // cell's format lives only under it, so the current format stays discoverable.
        menu.child(
            Button::new("numfmt-more")
                .label("More ▸")
                .debug_selector(|| "numfmt-more".into())
                .ghost()
                .small()
                .selected(is_more_only_num_fmt(active_code))
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.num_fmt_more_open = true;
                    cx.notify();
                })),
        )
    }

    /// The drilled-in "More" view: a "◂ Back" row that restores the basics, then the full grouped
    /// [`NUM_FMT_GROUPS`] inventory (section headers for multi-preset groups, each preset highlighted
    /// by exact code). This is the verbatim Phase-6 grouped render, relocated behind "More ▸".
    fn num_fmt_more_menu(&self, active_code: &str, cx: &mut Context<Self>) -> gpui::Div {
        let mut menu = div().flex().flex_col().gap(px(1.0)).child(
            Button::new("numfmt-back")
                .label("◂ Back")
                .debug_selector(|| "numfmt-back".into())
                .ghost()
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.num_fmt_more_open = false;
                    cx.notify();
                })),
        );
        for group in NUM_FMT_GROUPS {
            // A multi-preset group gets a muted section header; single-preset groups (General,
            // Text, …) read as plain top-level items, so no redundant header.
            if group.presets.len() > 1 {
                menu = menu.child(
                    div()
                        .px_1()
                        .pt(px(3.0))
                        .pb(px(1.0))
                        .text_xs()
                        .text_color(rgb(MUTED_TEXT))
                        .child(group.category.label()),
                );
            }
            for preset in group.presets {
                let code = preset.code.to_string();
                let selector = preset.code;
                menu = menu.child(
                    Button::new(gpui::ElementId::Name(
                        format!("numfmt-{}", preset.code).into(),
                    ))
                    .label(preset.label)
                    .debug_selector(move || format!("numfmt-{selector}"))
                    .ghost()
                    .small()
                    .selected(preset.code == active_code)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.apply_num_fmt(&code, window, cx);
                        },
                    )),
                );
            }
        }
        menu
    }

    /// The chart-insert menu (P17, `ui_design.md §3.1`): a small panel of chart-type glyphs; picking
    /// one inserts a near-empty authored chart of that type. Same backdrop/occlude/anchor pattern as
    /// the number-format dropdown.
    fn render_chart_menu(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // `items_start` keeps each entry content-sized (not stretched to the widest row), so its
        // icon + label pack at the LEFT edge instead of being centered in a full-width button
        // (post-v1 Batch 3, item 14: left-align the dropdown items like a normal menu). Without it
        // the flex column's default `stretch` widens every button to "Doughnut" and the inner
        // label flex (which gpui-component hardcodes to `justify_center`) centers the glyph + text.
        let mut menu = div().flex().flex_col().items_start().gap(px(2.0));
        for (kind, icon_path, label) in CHART_MENU {
            menu = menu.child(
                Button::new(gpui::ElementId::Name(format!("chart-{label}").into()))
                    .icon(Icon::empty().path(icon_path))
                    .label(label)
                    .debug_selector(move || format!("chart-menu-{label}"))
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.insert_chart(kind, window, cx);
                    })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.chart_menu_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Chart.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .debug_selector(|| "chart-menu-card".into())
                    .flex()
                    .flex_col()
                    .p_1()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The right-docked chart **edit panel** (P19, `ui_design §4`): a floating card on the right side
    /// of the sheet with a **Type** row (the `CHART_MENU` glyphs, current kind highlighted) and a
    /// **Data range** section ("use the current selection" applies it as the chart's range). It is a
    /// chrome overlay (no pixel baseline), not a popover on the chart. It closes on its × button, on
    /// **click-away** (a click on a cell / header / empty grid, routed via `on_selection_changed` —
    /// post-v1 Batch 2, item 12), on the chart's deletion, or on a degrade; clicking **another chart**
    /// re-points the panel to it (a switch, not a close). Its body scrolls + clips to its own bounds
    /// (item 7).
    fn render_chart_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panel = self
            .chart_panel
            .as_ref()
            .expect("render_chart_panel only runs while the panel is open");

        // The scrollable body sections (the docked-sidebar header stays pinned above them, so the ×
        // is always reachable — post-v1 Batch 2, item 7).
        let mut sections: Vec<gpui::AnyElement> = Vec::new();

        // Type + Data range — authored charts only (loaded re-type/re-range is not P20).
        if panel.is_authored {
            sections.push(
                section("Type", self.render_chart_type_row(panel.kind, cx)).into_any_element(),
            );
            sections.push(
                section("Data range", self.render_chart_range_body(panel, cx)).into_any_element(),
            );
        }

        // Title (a committed-on-Enter/blur text input).
        sections.push(
            section(
                "Title",
                Input::new(&self.chart_title_input).small().w_full(),
            )
            .into_any_element(),
        );

        // Legend on/off + position.
        sections.push(
            section("Legend", self.render_chart_legend_row(panel.legend, cx)).into_any_element(),
        );

        // Axis titles.
        sections.push(
            section(
                "Axis titles",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Input::new(&self.chart_cat_axis_input).small().w_full())
                    .child(Input::new(&self.chart_val_axis_input).small().w_full()),
            )
            .into_any_element(),
        );

        // Series colors.
        if !panel.series.is_empty() {
            sections.push(
                section("Series colors", self.render_chart_series_colors(panel, cx))
                    .into_any_element(),
            );
        }

        // Data-label toggles.
        sections.push(
            section(
                "Data labels",
                self.render_chart_data_labels(panel.labels, cx),
            )
            .into_any_element(),
        );

        let body = div().flex().flex_col().gap_3().children(sections);

        docked_sidebar(
            "chart-panel",
            "Edit chart",
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close_chart_panel(cx);
            }),
            body,
        )
        .into_any_element()
    }

    /// The right-docked **conditional-formatting sidebar** (`components/cf_sidebar.md §5`): the
    /// **List mode** (intro + rows + "+ Add rule") or, when `panel.editor` is `Some`, the rule
    /// **Editor mode** (`render_cf_editor`) — both inside the same [`docked_sidebar`] container.
    fn render_cond_fmt_sidebar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panel = self
            .cond_fmt
            .as_ref()
            .expect("render_cond_fmt_sidebar only runs while the sidebar is open");

        let body = match panel.editor.as_ref() {
            Some(editor) => self.render_cf_editor(editor, cx),
            None => self.render_cf_list(panel, cx),
        };

        docked_sidebar(
            "cond-fmt",
            "Conditional formatting",
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close_cond_fmt(cx);
            }),
            body,
        )
        .into_any_element()
    }

    /// The CF sidebar's **List mode** body (`ui_design.md §2.1`): an intro line naming the active
    /// sheet, the priority-descending rule rows (or the empty state), and the primary "+ Add rule".
    fn render_cf_list(&self, panel: &CondFmtPanel, cx: &mut Context<Self>) -> gpui::AnyElement {
        let sheet_name = self
            .sheets
            .iter()
            .find(|t| t.id == panel.sheet)
            .map(|t| t.name.clone())
            .unwrap_or_default();

        let intro = div()
            .text_size(px(12.0))
            .text_color(rgb(MUTED_TEXT))
            .child(format!("Rules on {sheet_name}"));

        // The priority-descending rule rows, or the empty state — placed above the Add button so
        // "No rules on this sheet yet." reads as the list's own placeholder (`ui_design.md §2.1`).
        let content = if panel.rows.is_empty() {
            div()
                .debug_selector(|| "cf-empty".to_string())
                .text_size(px(12.0))
                .text_color(rgb(MUTED_TEXT))
                .child("No rules on this sheet yet.")
                .into_any_element()
        } else {
            let last = panel.rows.len() - 1;
            let mut list = div().flex().flex_col().gap_1();
            for (i, row) in panel.rows.iter().enumerate() {
                list = list.child(self.render_cf_row(row, i == 0, i == last, cx));
            }
            list.into_any_element()
        };

        // "+ Add rule" opens the editor in add mode (`open_cf_editor(None)`).
        let add_rule = Button::new("cond-fmt-add-rule")
            .label("+ Add rule")
            .primary()
            .small()
            .disabled(self.degraded)
            .debug_selector(|| "cond-fmt-add-rule".to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_cf_editor(None, window, cx);
            }));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(intro)
            .child(content)
            .child(add_rule)
            .into_any_element()
    }

    /// One conditional-formatting rule as a List-mode row (`ui_design.md §2.1`,
    /// `components/cf_sidebar.md §5`): the preview swatch, a two-line summary/range, and the
    /// reorder/edit/delete controls. `is_first`/`is_last` are the row's ends in the
    /// priority-descending list (they gate the ▲/▼ reorder buttons); `row.index` is the rule's
    /// stable storage index (the handle the index-based mutators take).
    fn render_cf_row(
        &self,
        row: &CfRuleView,
        is_first: bool,
        is_last: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let index = row.index;
        let controls = cf_row_controls(row, is_first, is_last);

        let summary = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(1.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(TEXT))
                    .child(row.summary.clone()),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(MUTED_TEXT))
                    .child(row.range.clone()),
            );

        let controls_row = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(1.0))
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-up").into()))
                    .icon(Icon::empty().path("icons/chevron-up.svg"))
                    .tooltip("Move up")
                    .ghost()
                    .small()
                    .disabled(!controls.move_up)
                    .debug_selector(move || format!("cf-row-{index}-up"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, _cx| {
                        this.raise_cf_rule(index);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-down").into()))
                    .icon(Icon::empty().path("icons/chevron-down.svg"))
                    .tooltip("Move down")
                    .ghost()
                    .small()
                    .disabled(!controls.move_down)
                    .debug_selector(move || format!("cf-row-{index}-down"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, _cx| {
                        this.lower_cf_rule(index);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-edit").into()))
                    .icon(Icon::empty().path("icons/pencil.svg"))
                    .tooltip("Edit rule")
                    .ghost()
                    .small()
                    .disabled(!controls.edit)
                    .debug_selector(move || format!("cf-row-{index}-edit"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_cf_editor(Some(index), window, cx);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(
                    format!("cf-row-{index}-delete").into(),
                ))
                .icon(Icon::empty().path("icons/trash-2.svg"))
                .tooltip("Delete rule")
                .ghost()
                .small()
                // Always enabled — even a deferred-family/Badge rule is deletable
                // (`functional_spec.md §9`); driven off the helper so every control has one source.
                .disabled(!controls.delete)
                .debug_selector(move || format!("cf-row-{index}-delete"))
                .on_click(cx.listener(
                    move |this, _: &ClickEvent, _window, _cx| {
                        this.delete_cf_rule(index);
                    },
                )),
            );

        div()
            .debug_selector(move || format!("cf-row-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .py(px(4.0))
            .child(render_cf_preview(&row.preview))
            .child(summary)
            .child(controls_row)
            .into_any_element()
    }

    /// The CF sidebar's **Editor mode** body (`components/cf_sidebar.md §5`, `ui_design.md §2.2`):
    /// a back-row, the Applies-to range, the rule-type dropdown, the per-kind operands, the format
    /// editor, the Stop-if-true checkbox, the inline errors, and Save/Cancel.
    fn render_cf_editor(&self, editor: &CfEditorState, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (range, op1, op2, formula) = self.cf_input_texts(cx);
        let problems = cf_validate(editor, &range, &op1, &op2, &formula);
        let range_problem = range.trim().is_empty();
        let save_disabled = self.degraded || editor.pending_save || !problems.is_empty();

        // Back-row: a chevron-left + the mode title, whole row returns to List mode.
        let title = if editor.edit_index.is_some() {
            "Edit rule"
        } else {
            "New rule"
        };
        let back = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("cf-editor-back")
                    .icon(Icon::empty().path("icons/chevron-left.svg"))
                    .ghost()
                    .small()
                    .debug_selector(|| "cf-editor-back".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.cancel_cf_editor(cx);
                    })),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child(title),
            );

        // Applies-to: the A1 range input + an inline error when empty.
        let mut applies = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(Input::new(&self.cf_range_input).small().w_full());
        if range_problem {
            applies = applies.child(cf_inline_error(CF_RANGE_REQUIRED));
        }

        // Inline validation hints (operand problems, excluding the range one already shown) +
        // engine errors, red above Save.
        let mut messages = div().flex().flex_col().gap_1();
        for p in problems.iter().filter(|p| *p != CF_RANGE_REQUIRED) {
            messages = messages.child(cf_inline_error(p.clone()));
        }
        for e in &editor.errors {
            messages = messages.child(cf_inline_error(e.clone()));
        }

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cf-editor-save")
                    .label("Save")
                    .primary()
                    .small()
                    .disabled(save_disabled)
                    .debug_selector(|| "cf-editor-save".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.save_cf_editor(cx);
                    })),
            )
            .child(
                Button::new("cf-editor-cancel")
                    .label("Cancel")
                    .ghost()
                    .small()
                    .debug_selector(|| "cf-editor-cancel".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.cancel_cf_editor(cx);
                    })),
            );

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(back)
            .child(section("Applies to", applies))
            .child(section(
                "Rule type",
                self.render_cf_type_dropdown(editor, cx),
            ))
            .child(self.render_cf_operands(editor, cx))
            .child(section("Format", self.render_cf_format_editor(editor, cx)))
            .child(
                Checkbox::new("cf-stop-if-true")
                    .label("Stop if true")
                    .checked(editor.stop_if_true)
                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                        this.set_cf_stop_if_true(*checked, cx);
                    })),
            )
            .child(messages)
            .child(actions)
            .into_any_element()
    }

    /// A generic inline dropdown for the editor: a full-width trigger showing `current`, and — while
    /// this `menu` is the open one — the `options` listed directly below it (`components/cf_sidebar.md
    /// §5`). `tag` scopes the `cf-{tag}-trigger` / `cf-{tag}-{opt}` debug selectors + element ids.
    /// `on_select(this, opt_index, cx)` applies the chosen option.
    fn cf_dropdown(
        &self,
        menu: CfMenu,
        tag: &'static str,
        current: &str,
        options: &[(&'static str, String, bool)],
        on_select: impl Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + Copy + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.cf_menu_open == Some(menu);
        let trigger = Button::new(SharedString::from(format!("cf-{tag}-trigger")))
            .label(current.to_string())
            .small()
            .w_full()
            .debug_selector(move || format!("cf-{tag}-trigger"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.toggle_cf_menu(menu, cx);
            }));

        let mut col = div().flex().flex_col().gap_1().child(trigger);
        if open {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .p_1()
                .rounded_md()
                .bg(rgb(CF_MENU_BG))
                .border_1()
                .border_color(rgb(HAIRLINE));
            for (i, (opt_tag, label, selected)) in options.iter().enumerate() {
                let opt_tag = *opt_tag;
                list = list.child(
                    Button::new(SharedString::from(format!("cf-{tag}-{opt_tag}")))
                        .label(label.clone())
                        .ghost()
                        .small()
                        .w_full()
                        .selected(*selected)
                        .debug_selector(move || format!("cf-{tag}-{opt_tag}"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            on_select(this, i, window, cx);
                        })),
                );
            }
            col = col.child(list);
        }
        col.into_any_element()
    }

    /// The rule-type dropdown (the highlight families — color scales are P7's editor).
    fn render_cf_type_dropdown(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let current = cf_kind_label(editor.kind);
        let options: Vec<(&'static str, String, bool)> = CF_KIND_MENU
            .iter()
            .map(|(kind, tag, label)| (*tag, label.to_string(), *kind == editor.kind))
            .collect();
        self.cf_dropdown(
            CfMenu::RuleType,
            "type",
            current,
            &options,
            |this, i, window, cx| this.select_cf_kind(CF_KIND_MENU[i].0, window, cx),
            cx,
        )
    }

    /// The per-kind operand controls (`ui_design.md §2.2` step 3).
    fn render_cf_operands(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let body = match editor.kind {
            CfEditorKind::CellValue => {
                let op_current = cf_value_op_label(editor.value_op);
                let options: Vec<(&'static str, String, bool)> = CF_VALUE_OP_MENU
                    .iter()
                    .map(|(op, tag, label)| (*tag, label.to_string(), *op == editor.value_op))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::ValueOp,
                    "valueop",
                    op_current,
                    &options,
                    |this, i, _window, cx| this.select_cf_value_op(CF_VALUE_OP_MENU[i].0, cx),
                    cx,
                );
                let two = matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween);
                let mut inputs = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Input::new(&self.cf_operand1_input).small().w_full());
                if two {
                    inputs = inputs
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(MUTED_TEXT))
                                .child("and"),
                        )
                        .child(Input::new(&self.cf_operand2_input).small().w_full());
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(dropdown)
                    .child(inputs)
            }
            CfEditorKind::Text => {
                let op_current = cf_text_op_label(editor.text_op);
                let options: Vec<(&'static str, String, bool)> = CF_TEXT_OP_MENU
                    .iter()
                    .map(|(op, tag, label)| (*tag, label.to_string(), *op == editor.text_op))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::TextOp,
                    "textop",
                    op_current,
                    &options,
                    |this, i, _window, cx| this.select_cf_text_op(CF_TEXT_OP_MENU[i].0, cx),
                    cx,
                );
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(dropdown)
                    .child(Input::new(&self.cf_operand1_input).small().w_full())
            }
            CfEditorKind::Dates => {
                let current = cf_period_label(editor.period);
                let options: Vec<(&'static str, String, bool)> = CF_PERIOD_MENU
                    .iter()
                    .map(|(p, tag, label)| (*tag, label.to_string(), *p == editor.period))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::Period,
                    "period",
                    current,
                    &options,
                    |this, i, _window, cx| this.select_cf_period(CF_PERIOD_MENU[i].0, cx),
                    cx,
                );
                div().flex().flex_col().gap_1().child(dropdown)
            }
            CfEditorKind::TopBottom => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(cf_segmented(
                    [
                        ("cf-top-top", "Top", !editor.top_bottom),
                        ("cf-top-bottom", "Bottom", editor.top_bottom),
                    ],
                    cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_top_bottom(false, cx)),
                    cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_top_bottom(true, cx)),
                ))
                .child(Input::new(&self.cf_operand1_input).small().w_full())
                .child(
                    Button::new("cf-top-percent")
                        .label("% of range")
                        .ghost()
                        .small()
                        .selected(editor.top_percent)
                        .debug_selector(|| "cf-top-percent".to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.toggle_cf_top_percent(cx);
                        })),
                ),
            CfEditorKind::Average => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-avg-above", "Above average", !editor.average_below),
                    ("cf-avg-below", "Below average", editor.average_below),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_average_below(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_average_below(true, cx)),
            )),
            CfEditorKind::Duplicate => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-dup-duplicate", "Duplicate", !editor.duplicate_unique),
                    ("cf-dup-unique", "Unique", editor.duplicate_unique),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_duplicate_unique(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_duplicate_unique(true, cx)),
            )),
            CfEditorKind::Blanks => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-blank-blank", "Blank", !editor.blanks_no),
                    ("cf-blank-noblank", "No blanks", editor.blanks_no),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_blanks_no(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_blanks_no(true, cx)),
            )),
            CfEditorKind::Errors => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-err-error", "Error", !editor.errors_no),
                    ("cf-err-noerror", "No errors", editor.errors_no),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_errors_no(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_errors_no(true, cx)),
            )),
            CfEditorKind::Formula => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(Input::new(&self.cf_formula_input).small().w_full()),
        };
        section("Condition", body).into_any_element()
    }

    /// The differential-format editor (`components/cf_sidebar.md §7`): preset chips, fill + text
    /// swatch palettes, Bold/Italic toggles, and a live preview cell.
    fn render_cf_format_editor(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // Preset chips — each paints its own look and sets the whole format in one click.
        let mut presets = div().flex().flex_wrap().gap(px(4.0));
        for (i, (label, fmt)) in CF_PRESETS.iter().enumerate() {
            let fmt = *fmt;
            let bg = fmt.fill.map(cf_color).unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
            let fg = fmt.text_color.map(cf_color).unwrap_or_else(|| rgb(TEXT));
            presets = presets.child(
                div()
                    .id(SharedString::from(format!("cf-preset-{i}")))
                    .debug_selector(move || format!("cf-preset-{i}"))
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .bg(bg)
                    .text_size(px(10.5))
                    .text_color(fg)
                    .when(fmt.bold, |d| d.font_weight(gpui::FontWeight::BOLD))
                    .when(fmt.italic, |d| d.italic())
                    .cursor_pointer()
                    .child(SharedString::from(*label))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.set_cf_format(fmt, cx);
                        }),
                    ),
            );
        }

        let fill_row = self.render_cf_color_row(
            "fill",
            "Fill",
            editor.format.fill,
            "No fill",
            |this, color, cx| this.set_cf_fill(color, cx),
            cx,
        );
        let text_row = self.render_cf_color_row(
            "text",
            "Text",
            editor.format.text_color,
            "Automatic",
            |this, color, cx| this.set_cf_text_color(color, cx),
            cx,
        );

        let bi = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("cf-bold")
                    .icon(Icon::empty().path("icons/bold.svg"))
                    .tooltip("Bold")
                    .ghost()
                    .small()
                    .selected(editor.format.bold)
                    .debug_selector(|| "cf-bold".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.toggle_cf_bold(cx))),
            )
            .child(
                Button::new("cf-italic")
                    .icon(Icon::empty().path("icons/italic.svg"))
                    .tooltip("Italic")
                    .ghost()
                    .small()
                    .selected(editor.format.italic)
                    .debug_selector(|| "cf-italic".to_string())
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _w, cx| this.toggle_cf_italic(cx)),
                    ),
            );

        // Live preview: a sample cell painted with the working format.
        let preview_bg = editor
            .format
            .fill
            .map(cf_color)
            .unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
        let preview_fg = editor
            .format
            .text_color
            .map(cf_color)
            .unwrap_or_else(|| rgb(TEXT));
        let preview = div()
            .debug_selector(|| "cf-format-preview".to_string())
            .px(px(8.0))
            .py(px(4.0))
            .rounded_sm()
            .border_1()
            .border_color(rgb(HAIRLINE))
            .bg(preview_bg)
            .text_color(preview_fg)
            .text_size(px(12.0))
            .when(editor.format.bold, |d| {
                d.font_weight(gpui::FontWeight::BOLD)
            })
            .when(editor.format.italic, |d| d.italic())
            .child("123 Abc");

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(presets)
            .child(fill_row)
            .child(text_row)
            .child(bi)
            .child(preview)
            .into_any_element()
    }

    /// A labelled compact colour palette (fill or text): the `FILL_PALETTE` swatches (the current
    /// one ringed) + a "none" entry (`No fill` / `Automatic`). `on_pick(this, Option<Rgb>, cx)`
    /// applies the choice.
    fn render_cf_color_row(
        &self,
        tag: &'static str,
        label: &'static str,
        current: Option<Rgb>,
        none_label: &'static str,
        on_pick: impl Fn(&mut Self, Option<Rgb>, &mut Context<Self>) + Copy + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut swatches = div().flex().flex_wrap().items_center().gap(px(3.0));
        for sw in FILL_PALETTE {
            let selected = current == Some(sw.rgb);
            let color = sw.rgb;
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("cf-{tag}-{}", sw.name)))
                    .debug_selector(move || format!("cf-{tag}-{}", sw.name))
                    .w(px(CF_SWATCH_SIDE))
                    .h(px(CF_SWATCH_SIDE))
                    .rounded_sm()
                    .bg(rgb(sw.rgb.to_hex()))
                    .border_1()
                    .border_color(rgb(if selected {
                        SWATCH_SELECTED_RING
                    } else {
                        HAIRLINE
                    }))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            on_pick(this, Some(color), cx);
                        }),
                    ),
            );
        }
        swatches = swatches.child(
            Button::new(SharedString::from(format!("cf-{tag}-none")))
                .label(none_label)
                .ghost()
                .small()
                .selected(current.is_none())
                .debug_selector(move || format!("cf-{tag}-none"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    on_pick(this, None, cx);
                })),
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(MUTED_TEXT))
                    .child(label),
            )
            .child(swatches)
            .into_any_element()
    }

    /// The Type row: one glyph button per authorable kind, the current one selected (authored only).
    fn render_chart_type_row(
        &self,
        current: ChartInsertKind,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut type_row = div().flex().flex_wrap().gap(px(2.0));
        for (kind, icon_path, label) in CHART_MENU {
            type_row = type_row.child(
                Button::new(gpui::ElementId::Name(
                    format!("chart-panel-type-{label}").into(),
                ))
                .icon(Icon::empty().path(icon_path))
                .tooltip(label)
                .debug_selector(move || format!("chart-panel-type-{label}"))
                .ghost()
                .small()
                .selected(kind == current)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_type_from_panel(kind, window, cx);
                })),
            );
        }
        type_row.into_any_element()
    }

    /// The Data-range body: the current bound range summary + a "use selection" apply button.
    fn render_chart_range_body(
        &self,
        panel: &ChartPanel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let range_status = match &panel.ranges {
            Some(r) => r.clone(),
            None => "No data range set".to_string(),
        };
        let selection_a1 = self.selection.range().to_a1();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(TEXT))
                    .debug_selector(|| "chart-panel-range-status".into())
                    .child(range_status),
            )
            .child(
                Button::new("chart-panel-apply-range")
                    .label(format!("Use selection ({selection_a1})"))
                    .debug_selector(|| "chart-panel-apply-range".into())
                    .small()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.apply_chart_range_from_selection(window, cx);
                    })),
            )
            .into_any_element()
    }

    /// The Legend row: one lucide icon per position (`panel-top` / `panel-right` / `panel-left` /
    /// `panel-bottom`, showing where the legend docks) + `square-x` for Off, the current one selected
    /// (post-v1 Batch 2, item 11). Same behavior as before — each button sets the legend position or
    /// turns it off — just iconized. `panel-top` + `square-x` are FreeCell-vendored; the other three
    /// resolve from the gpui-component bundle (`shell::assets`).
    fn render_chart_legend_row(
        &self,
        current: Option<LegendPosition>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // `(target position, icon path, id/selector tag, tooltip)`. `None` = Off. Debug-selector
        // tags stay stable (`off` / `R`/`T`/`B`/`L`) so existing selectors keep resolving.
        let entries: [(Option<LegendPosition>, &str, &str, &str); 5] = [
            (Some(LegendPosition::Top), "icons/panel-top.svg", "T", "Top"),
            (
                Some(LegendPosition::Right),
                "icons/panel-right.svg",
                "R",
                "Right",
            ),
            (
                Some(LegendPosition::Left),
                "icons/panel-left.svg",
                "L",
                "Left",
            ),
            (
                Some(LegendPosition::Bottom),
                "icons/panel-bottom.svg",
                "B",
                "Bottom",
            ),
            (None, "icons/square-x.svg", "off", "Off"),
        ];
        let mut row = div().flex().flex_wrap().gap(px(2.0));
        for (pos, icon_path, tag, tooltip) in entries {
            row = row.child(
                Button::new(gpui::ElementId::Name(
                    format!("chart-panel-legend-{tag}").into(),
                ))
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .debug_selector(move || format!("chart-panel-legend-{tag}"))
                .ghost()
                .small()
                .selected(current == pos)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_legend_from_panel(pos, window, cx);
                })),
            );
        }
        row.into_any_element()
    }

    /// The per-series color rows: each series' name + a palette of swatches (the current one ringed)
    /// + an Automatic (clear) button.
    fn render_chart_series_colors(
        &self,
        panel: &ChartPanel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut col = div().flex().flex_col().gap_2();
        for (i, series) in panel.series.iter().enumerate() {
            let mut swatches = div().flex().flex_wrap().items_center().gap(px(3.0));
            for sw in FILL_PALETTE {
                let selected = series.color == Some(sw.rgb);
                let color = sw.rgb;
                swatches = swatches.child(
                    div()
                        .id(gpui::ElementId::NamedInteger(
                            format!("chart-series-{i}-swatch-{:06X}", sw.rgb.to_hex()).into(),
                            i as u64,
                        ))
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(2.0))
                        .bg(rgb(sw.rgb.to_hex()))
                        .border_1()
                        .border_color(rgb(if selected {
                            SWATCH_SELECTED_RING
                        } else {
                            HAIRLINE
                        }))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.set_chart_series_color_from_panel(i, Some(color), window, cx);
                            }),
                        ),
                );
            }
            swatches = swatches.child(
                Button::new(gpui::ElementId::NamedInteger(
                    "chart-series-auto".into(),
                    i as u64,
                ))
                .label("Auto")
                .debug_selector(move || format!("chart-series-{i}-auto"))
                .ghost()
                .small()
                .selected(series.color.is_none())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_series_color_from_panel(i, None, window, cx);
                })),
            );
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(TEXT))
                            .child(series.name.clone()),
                    )
                    .child(swatches),
            );
        }
        col.into_any_element()
    }

    /// The data-label toggle row: Value / Category / Percent, each reflecting the chart's current
    /// state; clicking flips that flag and applies the whole toggle set.
    fn render_chart_data_labels(
        &self,
        labels: DataLabelToggles,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let toggle = |id: &'static str,
                      label: &'static str,
                      on: bool,
                      next: DataLabelToggles,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .label(label)
                .debug_selector(move || id.into())
                .small()
                .selected(on)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_data_labels_from_panel(next, window, cx);
                }))
        };
        div()
            .flex()
            .flex_wrap()
            .gap(px(2.0))
            .child(toggle(
                "chart-panel-label-value",
                "Value",
                labels.show_value,
                DataLabelToggles {
                    show_value: !labels.show_value,
                    ..labels
                },
                cx,
            ))
            .child(toggle(
                "chart-panel-label-category",
                "Category",
                labels.show_category_name,
                DataLabelToggles {
                    show_category_name: !labels.show_category_name,
                    ..labels
                },
                cx,
            ))
            .child(toggle(
                "chart-panel-label-percent",
                "Percent",
                labels.show_percent,
                DataLabelToggles {
                    show_percent: !labels.show_percent,
                    ..labels
                },
                cx,
            ))
            .into_any_element()
    }

    /// The font-family dropdown: a scrolling menu of the installed families (fetched once at build),
    /// "Default (Inter)" first, the active cell's family highlighted (`components/action_bar.md`).
    fn render_font_family_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_family_label().to_string();
        let names = Rc::clone(&self.font_names);
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for (i, name) in names.iter().enumerate() {
            let pick = name.to_string();
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger(
                    "font-family".into(),
                    i as u64,
                ))
                .label(name.clone())
                .debug_selector(move || format!("font-family-{i}"))
                .ghost()
                .small()
                .selected(name.as_ref() == active)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_font_family(&pick, window, cx);
                })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_family_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-family-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontFamily.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    // The installed-font list is long — cap the height and scroll it.
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The font-size dropdown: the fixed point list, the active cell's size highlighted.
    fn render_font_size_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_size_label();
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for pt in FONT_SIZES {
            let label = format!("{pt}");
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger("font-size".into(), pt as u64))
                    .label(label.clone())
                    .debug_selector(move || format!("font-size-{pt}"))
                    .ghost()
                    .small()
                    .selected(label == active)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.apply_font_size(pt, window, cx);
                    })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_size_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-size-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontSize.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The borders **pen** popover (`ui_design.md §2`): three stacked regions — "Borders"
    /// target icons, a "Line" style gallery, and a "Color" swatch grid + custom picker. A target
    /// click paints the pen onto just those edges and keeps the popover open; only click-away / Esc
    /// closes it. The current target/pen is shown `.selected`.
    fn render_borders_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Region A — the eight "Borders" target icons (icon-only, so each carries a tooltip).
        let target_btn = |id: &'static str,
                          name: &'static str,
                          preset: BorderPreset,
                          this: &Self,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .debug_selector(move || id.to_string())
                .ghost()
                .small()
                .w(px(40.0))
                .h(px(34.0))
                .tooltip(name)
                .selected(this.border_target == Some(preset))
                .child(border_target_icon(preset))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.select_border_target(preset, window, cx);
                }))
        };
        let row1 = div()
            .flex()
            .gap_1()
            .child(target_btn("border-all", "All", BorderPreset::All, self, cx))
            .child(target_btn(
                "border-inner",
                "Inner",
                BorderPreset::Inner,
                self,
                cx,
            ))
            .child(target_btn(
                "border-outer",
                "Outer",
                BorderPreset::Outer,
                self,
                cx,
            ))
            .child(target_btn(
                "border-none",
                "None",
                BorderPreset::None,
                self,
                cx,
            ));
        let row2 = div()
            .flex()
            .gap_1()
            .child(target_btn("border-top", "Top", BorderPreset::Top, self, cx))
            .child(target_btn(
                "border-bottom",
                "Bottom",
                BorderPreset::Bottom,
                self,
                cx,
            ))
            .child(target_btn(
                "border-left",
                "Left",
                BorderPreset::Left,
                self,
                cx,
            ))
            .child(target_btn(
                "border-right",
                "Right",
                BorderPreset::Right,
                self,
                cx,
            ));

        // Region B — the line-style gallery (each button previews the real line).
        let line_btn = |id: &'static str,
                        name: &'static str,
                        line: BorderLine,
                        this: &Self,
                        cx: &mut Context<Self>| {
            Button::new(id)
                .debug_selector(move || id.to_string())
                .ghost()
                .small()
                .h(px(28.0))
                .tooltip(name)
                .selected(this.border_line == line)
                .child(border_line_preview(line))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_border_line(line, window, cx);
                }))
        };
        let gallery = div()
            .flex()
            .gap_1()
            .child(line_btn(
                "border-line-thin",
                "Thin",
                BorderLine::ThinSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-medium",
                "Medium",
                BorderLine::MediumSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-thick",
                "Thick",
                BorderLine::ThickSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-dashed",
                "Dashed",
                BorderLine::Dashed,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-double",
                "Double",
                BorderLine::Double,
                self,
                cx,
            ));

        // Region C — the color swatches (verbatim reuse of the fill popover's `FILL_PALETTE` grid;
        // the current pen color's swatch is ringed) + the inline "Custom…" picker.
        let mut swatches = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                let selected = color == self.border_color;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("border-swatch-{}", swatch.name).into(),
                        ))
                        .debug_selector(|| format!("border-swatch-{}", swatch.name))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        // Ring the pen's current swatch (a 2px accent border) so the selected color
                        // reads over any swatch fill; others keep the hairline outline.
                        .border_2()
                        .border_color(rgb(if selected {
                            SWATCH_SELECTED_RING
                        } else {
                            HAIRLINE
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.set_border_color(color, window, cx);
                            }),
                        ),
                );
            }
            swatches = swatches.child(r);
        }
        let color_region = div().flex().flex_col().gap_1().child(swatches).child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(MUTED_TEXT))
                        .child("Custom…"),
                )
                .child(ColorPicker::new(&self.border_color_picker).small()),
        );

        let section_label = |text: &'static str| {
            div()
                .text_size(px(11.0))
                .text_color(rgb(MUTED_TEXT))
                .child(text)
        };
        let divider = || div().h(px(1.0)).bg(rgb(HAIRLINE));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.borders_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Borders.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(section_label("Borders"))
                    .child(row1)
                    .child(row2)
                    .child(divider())
                    .child(section_label("Line"))
                    .child(gallery)
                    .child(section_label("Color"))
                    .child(color_region),
            )
            .into_any_element()
    }

    fn render_context_menu(&self, id: SheetId, cx: &mut Context<Self>) -> gpui::AnyElement {
        let delete_enabled = self.delete_enabled();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(self.backdrop(|this, _w, cx| this.close_context_menu(cx), cx))
            .child(
                div()
                    .absolute()
                    .bottom(px(TAB_BAR_H))
                    .left(px(16.0))
                    // Occlude the card so Rename/Delete clicks don't trip the backdrop dismiss on
                    // mouse-down before their `on_click` (mouse-up) fires (BUG A/B, same root cause
                    // as the action-bar popovers).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(
                        Button::new("ctx-rename")
                            .label("Rename")
                            .ghost()
                            .small()
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.rename_start(id, window, cx);
                            })),
                    )
                    .child(
                        Button::new("ctx-delete")
                            .label("Delete")
                            .ghost()
                            .small()
                            .disabled(!delete_enabled)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.request_delete(id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_delete_confirm(&self, id: SheetId, cx: &mut Context<Self>) -> gpui::AnyElement {
        let name = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000).opacity(0.3))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .w(px(320.0))
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_lg()
                    .shadow_lg()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(TEXT))
                            .child(format!("Delete sheet “{name}”? This can't be undone.")),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("delete-cancel")
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.cancel_delete(cx);
                                    })),
                            )
                            .child(
                                Button::new("delete-confirm")
                                    .label("Delete")
                                    .danger()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.confirm_delete(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// Which of a CF rule row's controls are enabled (`components/cf_sidebar.md §5`). Move-up/down are
/// disabled at the ends of the priority-descending list; edit is disabled for a non-editable
/// (deferred-family/Badge) rule; delete is always enabled (even deferred-family rules are
/// deletable — `functional_spec.md §9`). Pure so the row test asserts the enablement logic without
/// pixel clicks, and [`render_cf_row`](ChromeView::render_cf_row) derives its `.disabled(...)`
/// flags from the same source.
struct CfRowControls {
    move_up: bool,
    move_down: bool,
    edit: bool,
    delete: bool,
}

fn cf_row_controls(row: &CfRuleView, is_first: bool, is_last: bool) -> CfRowControls {
    CfRowControls {
        move_up: !is_first,
        move_down: !is_last,
        // A color-scale rule is `editable` in the engine model, but its editor is P7 — until then
        // its edit control is disabled here (it stays deletable). P7 removes this gate.
        edit: row.editable && !matches!(row.spec, Some(CfRuleSpec::ColorScale { .. })),
        delete: true,
    }
}

/// The rule-type dropdown menu (`components/cf_sidebar.md §5`): the highlight families in Excel's
/// grouping order. `(kind, debug tag, label)`. Color scales are omitted (P7's dedicated editor).
const CF_KIND_MENU: [(CfEditorKind, &str, &str); 9] = [
    (CfEditorKind::CellValue, "cellvalue", "Cell value"),
    (CfEditorKind::Text, "text", "Text"),
    (CfEditorKind::Dates, "dates", "Date occurring"),
    (CfEditorKind::TopBottom, "topbottom", "Top / Bottom"),
    (CfEditorKind::Average, "average", "Above / Below average"),
    (CfEditorKind::Duplicate, "duplicate", "Duplicate / Unique"),
    (CfEditorKind::Blanks, "blanks", "Blank / No blanks"),
    (CfEditorKind::Errors, "errors", "Error / No errors"),
    (CfEditorKind::Formula, "formula", "Formula"),
];

/// The *Cell value* operator dropdown menu. `(op, debug tag, label)`.
const CF_VALUE_OP_MENU: [(CfValueOp, &str, &str); 8] = [
    (CfValueOp::Gt, "gt", "greater than"),
    (CfValueOp::Lt, "lt", "less than"),
    (CfValueOp::Ge, "ge", "greater than or equal to"),
    (CfValueOp::Le, "le", "less than or equal to"),
    (CfValueOp::Eq, "eq", "equal to"),
    (CfValueOp::Ne, "ne", "not equal to"),
    (CfValueOp::Between, "between", "between"),
    (CfValueOp::NotBetween, "notbetween", "not between"),
];

/// The *Text* operator dropdown menu. `(op, debug tag, label)`.
const CF_TEXT_OP_MENU: [(CfTextOp, &str, &str); 5] = [
    (CfTextOp::Contains, "contains", "contains"),
    (CfTextOp::NotContains, "notcontains", "does not contain"),
    (CfTextOp::BeginsWith, "beginswith", "begins with"),
    (CfTextOp::EndsWith, "endswith", "ends with"),
    (CfTextOp::Equals, "equals", "equal to"),
];

/// The *A date occurring* period dropdown menu (the parameterless periods). `(period, tag, label)`.
const CF_PERIOD_MENU: [(CfPeriod, &str, &str); 13] = [
    (CfPeriod::Today, "today", "Today"),
    (CfPeriod::Yesterday, "yesterday", "Yesterday"),
    (CfPeriod::Tomorrow, "tomorrow", "Tomorrow"),
    (CfPeriod::Last7Days, "last7days", "In the last 7 days"),
    (CfPeriod::LastWeek, "lastweek", "Last week"),
    (CfPeriod::ThisWeek, "thisweek", "This week"),
    (CfPeriod::NextWeek, "nextweek", "Next week"),
    (CfPeriod::LastMonth, "lastmonth", "Last month"),
    (CfPeriod::ThisMonth, "thismonth", "This month"),
    (CfPeriod::NextMonth, "nextmonth", "Next month"),
    (CfPeriod::LastYear, "lastyear", "Last year"),
    (CfPeriod::ThisYear, "thisyear", "This year"),
    (CfPeriod::NextYear, "nextyear", "Next year"),
];

/// The differential-format presets (`components/cf_sidebar.md §7`, the classic Excel highlight
/// looks). Each sets the whole [`CfFormat`] in one click. `(label, format)`.
const CF_PRESETS: [(&str, CfFormat); 5] = [
    (
        "Light red / dark red",
        CfFormat {
            fill: Some(Rgb::from_hex(0xFFC7CE)),
            text_color: Some(Rgb::from_hex(0x9C0006)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Yellow / dark yellow",
        CfFormat {
            fill: Some(Rgb::from_hex(0xFFEB9C)),
            text_color: Some(Rgb::from_hex(0x9C6500)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Green / dark green",
        CfFormat {
            fill: Some(Rgb::from_hex(0xC6EFCE)),
            text_color: Some(Rgb::from_hex(0x006100)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Red text",
        CfFormat {
            fill: None,
            text_color: Some(Rgb::from_hex(0x9C0006)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Bold",
        CfFormat {
            fill: None,
            text_color: None,
            bold: true,
            italic: false,
        },
    ),
];

fn cf_kind_label(kind: CfEditorKind) -> &'static str {
    CF_KIND_MENU
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, _, label)| *label)
        .unwrap_or("Cell value")
}

fn cf_value_op_label(op: CfValueOp) -> &'static str {
    CF_VALUE_OP_MENU
        .iter()
        .find(|(o, _, _)| *o == op)
        .map(|(_, _, label)| *label)
        .unwrap_or("greater than")
}

fn cf_text_op_label(op: CfTextOp) -> &'static str {
    CF_TEXT_OP_MENU
        .iter()
        .find(|(o, _, _)| *o == op)
        .map(|(_, _, label)| *label)
        .unwrap_or("contains")
}

fn cf_period_label(period: CfPeriod) -> &'static str {
    CF_PERIOD_MENU
        .iter()
        .find(|(p, _, _)| *p == period)
        .map(|(_, _, label)| *label)
        .unwrap_or("Today")
}

/// A small red inline error/hint line (`ui_design.md §2.2`).
fn cf_inline_error(text: impl Into<SharedString>) -> gpui::AnyElement {
    div()
        .text_size(px(10.5))
        .text_color(rgb(DANGER))
        .child(text.into())
        .into_any_element()
}

/// A two-option segmented toggle (e.g. Top/Bottom, Above/Below) — two small selectable buttons.
/// `left`/`right` are `(debug tag, label, selected)`; the click handlers are supplied by the caller.
fn cf_segmented(
    options: [(&'static str, &'static str, bool); 2],
    on_left: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_right: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let [(ltag, llabel, lsel), (rtag, rlabel, rsel)] = options;
    div()
        .flex()
        .gap_1()
        .child(
            Button::new(ltag)
                .label(llabel)
                .ghost()
                .small()
                .selected(lsel)
                .debug_selector(move || ltag.to_string())
                .on_click(on_left),
        )
        .child(
            Button::new(rtag)
                .label(rlabel)
                .ghost()
                .small()
                .selected(rsel)
                .debug_selector(move || rtag.to_string())
                .on_click(on_right),
        )
}

/// Client-side validation of the editor form (`components/cf_sidebar.md §6`): the messages that
/// block Save (empty = valid). Range non-empty; Cell-value operand(s) present (a second for
/// Between/NotBetween); Text value present; Formula present; Top rank ≥ 1.
fn cf_validate(
    editor: &CfEditorState,
    range: &str,
    operand1: &str,
    operand2: &str,
    formula: &str,
) -> Vec<String> {
    let mut errs = Vec::new();
    if range.trim().is_empty() {
        errs.push(CF_RANGE_REQUIRED.to_string());
    }
    match editor.kind {
        CfEditorKind::CellValue => {
            if operand1.trim().is_empty() {
                errs.push("Enter a value.".to_string());
            }
            if matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween)
                && operand2.trim().is_empty()
            {
                errs.push("Enter a second value.".to_string());
            }
        }
        CfEditorKind::Text => {
            if operand1.trim().is_empty() {
                errs.push("Enter text to match.".to_string());
            }
        }
        CfEditorKind::Formula => {
            if formula.trim().is_empty() {
                errs.push("Enter a formula.".to_string());
            }
        }
        CfEditorKind::TopBottom => {
            if !matches!(operand1.trim().parse::<u32>(), Ok(n) if n >= 1) {
                errs.push("Enter a rank of 1 or more.".to_string());
            }
        }
        CfEditorKind::Dates
        | CfEditorKind::Average
        | CfEditorKind::Duplicate
        | CfEditorKind::Blanks
        | CfEditorKind::Errors => {}
    }
    errs
}

/// Assemble the [`CfRuleSpec`] the editor commits (add/update). Assumes [`cf_validate`] passed, so
/// the operands are present + a Top rank parses (falls back to the seed rank otherwise).
fn cf_build_spec(
    editor: &CfEditorState,
    operand1: &str,
    operand2: &str,
    formula: &str,
) -> CfRuleSpec {
    let format = editor.format;
    let stop_if_true = editor.stop_if_true;
    match editor.kind {
        CfEditorKind::CellValue => {
            let operand2 = matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween)
                .then(|| operand2.trim().to_string());
            CfRuleSpec::CellIs {
                op: editor.value_op,
                operand: operand1.trim().to_string(),
                operand2,
                format,
                stop_if_true,
            }
        }
        CfEditorKind::Text => CfRuleSpec::Text {
            op: editor.text_op,
            value: operand1.trim().to_string(),
            format,
            stop_if_true,
        },
        CfEditorKind::Dates => CfRuleSpec::TimePeriod {
            period: editor.period,
            format,
            stop_if_true,
        },
        CfEditorKind::TopBottom => CfRuleSpec::Top {
            rank: operand1.trim().parse().unwrap_or(editor.top_rank),
            percent: editor.top_percent,
            bottom: editor.top_bottom,
            format,
            stop_if_true,
        },
        CfEditorKind::Average => CfRuleSpec::Average {
            below: editor.average_below,
            format,
            stop_if_true,
        },
        CfEditorKind::Duplicate => CfRuleSpec::DuplicateValues {
            unique: editor.duplicate_unique,
            format,
            stop_if_true,
        },
        CfEditorKind::Blanks => CfRuleSpec::Blanks {
            no_blanks: editor.blanks_no,
            format,
            stop_if_true,
        },
        CfEditorKind::Errors => CfRuleSpec::Errors {
            no_errors: editor.errors_no,
            format,
            stop_if_true,
        },
        CfEditorKind::Formula => CfRuleSpec::Formula {
            formula: formula.trim().to_string(),
            format,
            stop_if_true,
        },
    }
}

/// Seed a [`CfEditorState`] + the operand/formula input texts `(operand1, operand2, formula)` from
/// an authorable highlight rule's `spec` (edit mode). Returns `None` for a `ColorScale` — that
/// editor is P7, so a color-scale row's edit is disabled here and never reaches this.
fn cf_state_from_spec(
    index: u32,
    spec: &CfRuleSpec,
) -> Option<(CfEditorState, String, String, String)> {
    let mut state = CfEditorState::new(Some(index));
    let (op1, op2, formula) = match spec {
        CfRuleSpec::CellIs {
            op,
            operand,
            operand2,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::CellValue;
            state.value_op = *op;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (
                operand.clone(),
                operand2.clone().unwrap_or_default(),
                String::new(),
            )
        }
        CfRuleSpec::Text {
            op,
            value,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Text;
            state.text_op = *op;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (value.clone(), String::new(), String::new())
        }
        CfRuleSpec::TimePeriod {
            period,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Dates;
            state.period = *period;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Top {
            rank,
            percent,
            bottom,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::TopBottom;
            state.top_rank = *rank;
            state.top_percent = *percent;
            state.top_bottom = *bottom;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (rank.to_string(), String::new(), String::new())
        }
        CfRuleSpec::Average {
            below,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Average;
            state.average_below = *below;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::DuplicateValues {
            unique,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Duplicate;
            state.duplicate_unique = *unique;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Blanks {
            no_blanks,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Blanks;
            state.blanks_no = *no_blanks;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Errors {
            no_errors,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Errors;
            state.errors_no = *no_errors;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Formula {
            formula,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Formula;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), formula.clone())
        }
        CfRuleSpec::ColorScale { .. } => return None,
    };
    Some((state, op1, op2, formula))
}

/// A `freecell_core::Rgb` as a gpui fill/text colour (the grid maps `Rgb` the same way at draw
/// time — `color.rs`).
fn cf_color(c: Rgb) -> Rgba {
    rgb(c.to_hex())
}

/// The List-mode preview swatch for a rule's effect (`components/cf_sidebar.md §5`): a highlight
/// rule's fill + text-colour chip (an "A" glyph so both read), a colour scale's banded gradient,
/// or a deferred-family/variant Badge tag (the first pass can't author it, only show + delete it).
fn render_cf_preview(preview: &CfPreview) -> gpui::AnyElement {
    match preview {
        CfPreview::Highlight { fill, text_color } => {
            // "No fill" reads as a white chip; "no text colour" as the default text colour — so the
            // chip always renders on the sidebar's white card.
            let bg = (*fill).map(cf_color).unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
            let fg = (*text_color).map(cf_color).unwrap_or_else(|| rgb(TEXT));
            div()
                .flex_shrink_0()
                .w(px(CF_SWATCH_W))
                .h(px(CF_SWATCH_H))
                .rounded_sm()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child("A"),
                )
                .into_any_element()
        }
        CfPreview::ColorScale { colors } => {
            // Equal-width bands across the stop colours — a stepped horizontal gradient the width
            // of one swatch. `overflow_hidden` clips the bands to the rounded chip.
            let mut chip = div()
                .flex_shrink_0()
                .w(px(CF_SWATCH_W))
                .h(px(CF_SWATCH_H))
                .rounded_sm()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .flex();
            for c in colors {
                chip = chip.child(div().flex_1().h_full().bg(cf_color(*c)));
            }
            chip.into_any_element()
        }
        CfPreview::Badge(label) => div()
            .flex_shrink_0()
            .px(px(4.0))
            .py(px(1.0))
            .rounded_sm()
            .bg(rgb(CF_BADGE_BG))
            .border_1()
            .border_color(rgb(HAIRLINE))
            .text_size(px(9.5))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(MUTED_TEXT))
            .child(label.clone())
            .into_any_element(),
    }
}

#[cfg(test)]
impl ChromeView {
    /// Test seam: simulate the user typing `text` into the content field (sets the widget
    /// text, then delivers the `Change` event the subscription would).
    fn test_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let handle = self.content_input.clone();
        self.on_content_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the content field.
    fn test_press_enter(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.content_input.clone();
        self.on_content_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: set the rename input's text.
    fn test_rename_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Test seam: simulate typing `text` into the find field (sets the widget text, then delivers
    /// the `Change` event the subscription would).
    fn test_find_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.find_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let handle = self.find_input.clone();
        self.on_find_input_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: set the replace field's text (no event needed — replace reads it on demand).
    fn test_replace_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the find field.
    fn test_find_press_enter(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.find_input.clone();
        self.on_find_input_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: the find field's current text.
    fn find_field_text(&self, cx: &App) -> String {
        self.find_input.read(cx).value().to_string()
    }

    /// Test seam: the find field's current selection range (for the select-on-open check).
    fn find_selection(&self, cx: &App) -> std::ops::Range<usize> {
        self.find_input.read(cx).selected_range()
    }

    /// Test seam: simulate typing `text` into the in-cell editor (sets the widget text, then
    /// delivers the `Change` event the subscription would).
    fn test_incell_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.edit.in_cell().clone();
        handle.update(cx, |input, cx| input.set_value(text, window, cx));
        self.on_incell_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the in-cell editor.
    fn test_incell_press_enter(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.edit.in_cell().clone();
        self.on_incell_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: replicate the data-row Tab handler (commit + move right/left) without the
    /// widget-level `capture_key_down`.
    fn test_data_row_tab(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_mode() == FieldMode::Editing {
            let dir = if shift {
                Direction::Left
            } else {
                Direction::Right
            };
            self.commit_and_move(dir, window, cx);
        }
    }

    /// Test seam: the in-cell editor's current text.
    fn incell_text(&self, cx: &App) -> String {
        self.edit.in_cell().read(cx).value().to_string()
    }

    /// Test seam: the open in-cell overlay cell, if any.
    fn incell_open(&self) -> Option<CellRef> {
        self.edit.open_cell()
    }

    /// Test seam: which editor currently drives the edit.
    fn edit_origin(&self) -> EditOrigin {
        self.edit.origin()
    }

    /// Test seam: the captured chrome-local left-x of a dropdown trigger (BUG 2c anchoring).
    fn anchor_x_of(&self, which: Anchor) -> f32 {
        self.anchor_x[which.idx()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::{ChromeClient, RecordingClient};
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use freecell_core::{CellRange, CellRef, SelectionModel};
    use freecell_engine::{BorderPreset, Command, SheetMeta, StyleAttr, WorkerEvent};
    use gpui::{px, size, Modifiers, TestAppContext};
    use gpui_component::Root;
    use std::cell::RefCell;

    /// A window hosting a `ChromeView` over a `RecordingClient`, plus a recording grid sink.
    struct Harness {
        chrome: Entity<ChromeView>,
        client: Rc<RecordingClient>,
        grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>>,
        window: gpui::WindowHandle<Root>,
    }

    fn cell(row: u32, col: u32) -> CellRef {
        CellRef::new(row, col)
    }

    fn build(cx: &mut TestAppContext, sheets: Vec<SheetTab>, active: SheetId) -> Harness {
        build_win(cx, sheets, active, 200.0)
    }

    /// [`build`] with a caller-chosen window height — the popover-click tests want a tall enough
    /// window that every dropdown item lays out on-screen and can be hit by a simulated click. The
    /// window matches the real document width (1200 px), wider than the action row's ~1152 px
    /// natural width, so the row fits (no scroller chevrons) and its triggers lay out on-screen.
    fn build_win(
        cx: &mut TestAppContext,
        sheets: Vec<SheetTab>,
        active: SheetId,
        height: f32,
    ) -> Harness {
        build_sized(cx, sheets, active, 1200.0, height)
    }

    /// [`build_win`] with a caller-chosen window **width** too — the horizontal-scroller tests open
    /// a narrow window so the action row / tab strip overflow and show chevrons.
    fn build_sized(
        cx: &mut TestAppContext,
        sheets: Vec<SheetTab>,
        active: SheetId,
        width: f32,
        height: f32,
    ) -> Harness {
        let client = Rc::new(RecordingClient::new());
        let grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>> = Rc::new(RefCell::new(Vec::new()));

        cx.update(gpui_component::init);

        let client_for_window = client.clone();
        let reqs_for_window = grid_requests.clone();
        let mut chrome_out: Option<Entity<ChromeView>> = None;
        let chrome_slot = &mut chrome_out;

        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let client_dyn: Rc<dyn ChromeClient> = client_for_window;
            let reqs = reqs_for_window;
            let sink = ChromeGridSink::new(move |req, _w, _cx| reqs.borrow_mut().push(req.clone()));
            let chrome = cx.new(|cx| ChromeView::new(client_dyn, sink, active, sheets, window, cx));
            *chrome_slot = Some(chrome.clone());
            Root::new(chrome, window, cx)
        });

        Harness {
            chrome: chrome_out.expect("chrome built"),
            client,
            grid_requests,
            window,
        }
    }

    fn one_sheet(cx: &mut TestAppContext) -> Harness {
        build(cx, vec![SheetTab::new(SheetId(0), "Sheet1")], SheetId(0))
    }

    /// A stand-in for the hosted grid: an empty full-size body. Its only job is to make the chrome
    /// **fill the window** (`render` flexes only when a body is present), so a popover's full-window
    /// backdrop really spans the window height — the condition under which BUG A/B bites. With a
    /// bodyless chrome the backdrop is only ~3 rows tall and the dropdown items lay out *below* it,
    /// never overlapping it, so the regression would hide.
    struct BodyStub;
    impl gpui::Render for BodyStub {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // A concrete-height body so the chrome content — and thus the popover backdrop, which is
            // `size_full` of the chrome — spans well past the dropdown items. (`flex_1` alone won't
            // stretch it: the test Root sizes the chrome to its content, not the window.)
            div().h(px(500.0)).w_full()
        }
    }

    /// A short stand-in grid body, so the chrome (and thus the absolutely-positioned chart panel,
    /// sized between the data row and the tab bar) is **height-constrained** — the condition under
    /// which the panel's control stack overflows and must scroll + clip (item 7).
    struct ShortBodyStub;
    impl gpui::Render for ShortBodyStub {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().h(px(40.0)).w_full()
        }
    }

    /// One sheet in a tall window with a (stub) grid body, for the popover-click tests: every item
    /// lays out on-screen over a full-height backdrop.
    fn tall_sheet(cx: &mut TestAppContext) -> Harness {
        let h = build_win(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            600.0,
        );
        upd(&h, cx, |c, _w, cx| {
            let body: gpui::AnyView = cx.new(|_| BodyStub).into();
            c.set_grid_body(body, cx);
        });
        h
    }

    /// Runs `f` against the chrome with a live `Window`.
    fn upd<R>(
        h: &Harness,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>) -> R,
    ) -> R {
        h.window
            .update(cx, |_root, window, cx| {
                h.chrome.update(cx, |c, cx| f(c, window, cx))
            })
            .unwrap()
    }

    fn tick(cx: &mut TestAppContext, ms: u64) {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
    }

    // ---- Data row: fetch / reply / disable -------------------------------------------------

    #[gpui::test]
    fn data_row_content_field_is_inset_within_bar(cx: &mut TestAppContext) {
        // BUG C: the formula-bar content entry must sit 2 px inside the 32 px bar (top and bottom)
        // — i.e. render at `DATA_ROW_H - 4` = 28 px — without changing the bar height. The field
        // wrapper hugs the hosted `Input`'s height (the bar is `items_center`, not stretch), so its
        // painted height is the control height. Without the `min_h`/`max_h` inset on the `Input`
        // the control renders at gpui-component's fixed 32 px and fills the bar edge-to-edge; this
        // asserts 28 px and fails if the inset is removed (verified fail-without / pass-with).
        let h = one_sheet(cx);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let field = vcx
            .debug_bounds("data-content-field")
            .expect("the data-row content field was painted");
        let field_h = f32::from(field.size.height);
        assert!(
            (field_h - DATA_ROW_FIELD_H).abs() < 0.5,
            "content field must render at DATA_ROW_H - 4 = {DATA_ROW_FIELD_H}px, got {field_h}"
        );
        // The inset must not have changed the bar height.
        assert_eq!(DATA_ROW_H, 32.0, "the data-row bar height must stay 32px");
        assert!(
            field_h + 3.5 < DATA_ROW_H,
            "the field must be shorter than the bar so items_center leaves breathing room"
        );
    }

    #[gpui::test]
    fn selection_single_fetches_content(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::GetCellContent { cell: cc, req_id: 1, .. }] if *cc == cell(1, 1)),
            "expected one GetCellContent for B2, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.ref_box_text()), "B2");
    }

    #[gpui::test]
    fn content_reply_populates_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "=SUM(A1:A2)".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=SUM(A1:A2)");
    }

    #[gpui::test]
    fn stale_content_reply_dropped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx); // req 1
            c.on_selection_changed(SelectionModel::single(cell(1, 0)), window, cx);
            // req 2
        });
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "stale".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "fresh".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "fresh");
    }

    // ---- Selection stats (tab-bar status readout, `functional_spec.md §1`) -----------------

    /// A ready-made numeric aggregate for the reply-plumbing tests.
    fn numeric_stats() -> SelectionStats {
        SelectionStats {
            count: 5,
            numeric_count: 2,
            sum: 30.0,
            min: Some(10.0),
            max: Some(20.0),
        }
    }

    /// A1:A3 (a 3-cell column selection).
    fn multi_a1_a3() -> SelectionModel {
        SelectionModel {
            anchor: cell(0, 0),
            active: cell(2, 0),
        }
    }

    #[gpui::test]
    fn multi_cell_selection_requests_debounced_stats(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        // Debounced: nothing is sent until the timer fires (a drag-select would otherwise spam).
        assert!(
            h.client.take_commands().is_empty(),
            "the stats query is debounced, not sent synchronously"
        );
        tick(cx, 150);
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SelectionStats { range, req_id: 1, .. }]
                    if *range == CellRange::new(cell(0, 0), cell(2, 0))
            ),
            "expected one debounced SelectionStats for A1:A3, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn single_cell_selection_issues_no_stats(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        tick(cx, 150);
        let cmds = h.client.take_commands();
        assert!(
            cmds.iter()
                .all(|c| !matches!(c, Command::SelectionStats { .. })),
            "a single-cell selection issues no stats query, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()), None);
    }

    #[gpui::test]
    fn stats_reply_renders_readout_with_minmax_toggle(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        h.client.take_commands(); // drain the SelectionStats query (req_id 1)
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            Some("Sum: 30   Average: 15   Count: 5".to_string())
        );
        // Clicking the readout expands it to also show Min / Max (session-only toggle).
        upd(&h, cx, |c, _w, cx| c.toggle_stats_minmax(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            Some("Sum: 30   Average: 15   Count: 5   Min: 10   Max: 20".to_string())
        );
    }

    #[gpui::test]
    fn stale_stats_reply_dropped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // Two multi-cell selections back-to-back → the latest request is req_id 2.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx);
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(3, 0),
                },
                window,
                cx,
            );
        });
        tick(cx, 150);
        h.client.take_commands();
        // A superseded (req_id 1) reply is dropped.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            None,
            "a stale reply for a superseded selection is ignored"
        );
        // The current (req_id 2) reply lands.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 2,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()).is_some());
    }

    #[gpui::test]
    fn all_text_reply_hides_readout(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        h.client.take_commands();
        // A selection with content but no numeric cell shows no readout.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: SelectionStats {
                        count: 3,
                        numeric_count: 0,
                        sum: 0.0,
                        min: None,
                        max: None,
                    },
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()), None);
    }

    #[gpui::test]
    fn tab_bar_paints_stats_readout_when_present(cx: &mut TestAppContext) {
        // Real render coverage for the tab-bar refactor: with a numeric multi-cell selection the
        // right-aligned readout element paints (its Sum/Average/Count text gives it real width).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let bounds = vcx
            .debug_bounds("selection-stats")
            .expect("the selection-stats readout paints in the tab bar");
        assert!(
            f32::from(bounds.size.width) > 20.0,
            "the readout should paint its Sum/Average/Count text, got width {}",
            f32::from(bounds.size.width)
        );
    }

    // ---- Horizontal scroller (action bar + tab strip, `functional_spec.md §9B`) ------------

    /// A tab list long enough to overflow a narrow window.
    fn many_sheets(n: u32) -> Vec<SheetTab> {
        (0..n)
            .map(|i| SheetTab::new(SheetId(i), format!("Sheet{i}")))
            .collect()
    }

    /// Paint the window twice: the scroll handle measures overflow on the first paint, so the
    /// chevron affordance only appears on the second (the one-frame gpui scroll-handle lag noted in
    /// `h_scroller`).
    fn paint_twice(vcx: &mut gpui::VisualTestContext) {
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }

    /// Advance the chevron slide by `n` animation frames. `request_animation_frame`'s `on_next_frame`
    /// callback only fires from the platform's frame loop, which the test window stubs out — so a
    /// test drives each frame manually with a `refresh` + `run_until_parked` (each redraw runs
    /// `h_scroller`'s one-frame `anim_step`).
    fn pump_frames(vcx: &mut gpui::VisualTestContext, n: usize) {
        for _ in 0..n {
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
        }
    }

    /// Drive a numeric multi-cell selection so the tab-bar stats readout is shown (mirrors the
    /// Phase-1 reply plumbing).
    fn show_stats(h: &Harness, cx: &mut TestAppContext) {
        upd(h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        upd(h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
    }

    #[gpui::test]
    fn action_row_fits_has_no_chevrons(cx: &mut TestAppContext) {
        // A wide window fits the whole action row → the scroller is invisible (no chevrons, no
        // behaviour change vs. today — `functional_spec.md §9B` "fits horizontally").
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            1400.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        assert!(
            vcx.debug_bounds("action-row-chevrons").is_none(),
            "a window wider than the action row shows no scroll chevrons"
        );
    }

    #[gpui::test]
    fn action_row_overflow_shows_chevrons(cx: &mut TestAppContext) {
        // A window narrower than the ~1152 px action row → the scroller shows its chevron section.
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            480.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        assert!(
            vcx.debug_bounds("action-row-chevrons").is_some(),
            "a narrow window overflows the action row → scroll chevrons appear"
        );
    }

    #[gpui::test]
    fn action_row_no_chevrons_when_the_controls_actually_fit(cx: &mut TestAppContext) {
        // Regression for `functional_spec.md §10.2`: the button group's natural width is well under
        // the old hand-estimated `ACTION_ROW_MIN_W = 1152`, so at a viewport that comfortably fits
        // the real controls (but is narrower than 1152) the scroller must report NO overflow — the
        // pre-fix `min_w(1152)` padded the scroll content with trailing empty space and tripped the
        // chevrons here.
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            1400.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        // The painted natural width of the button group (`flex_shrink_0`, so it never compresses).
        let natural = vcx
            .debug_bounds("action-row-groups")
            .expect("the action-row button group paints")
            .size
            .width;
        let natural = f32::from(natural);
        assert!(
            natural < 1152.0,
            "the old min_w=1152 over-estimated the controls; true natural width is {natural}"
        );
        // A viewport a hair above the natural width — still < 1152 — must NOT overflow.
        let fits = natural + 40.0;
        assert!(
            fits < 1152.0,
            "the fit viewport {fits} is below the old estimate"
        );
        let h2 = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            fits,
            200.0,
        );
        let mut vcx2 = gpui::VisualTestContext::from_window(h2.window.into(), cx);
        paint_twice(&mut vcx2);
        assert!(
            vcx2.debug_bounds("action-row-chevrons").is_none(),
            "a viewport ({fits}) wide enough for the real controls shows no chevrons (§10.2 fix)"
        );
    }

    #[gpui::test]
    fn tab_bar_overflow_shows_chevrons_and_keeps_stats_static(cx: &mut TestAppContext) {
        // Many tabs in a narrow window overflow the tab strip → chevrons appear, AND the stats
        // group stays pinned static to the RIGHT of the scroller (never pushed off — §9A.4).
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        show_stats(&h, cx);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        let chevrons = vcx
            .debug_bounds("tab-bar-chevrons")
            .expect("a long tab strip overflows → tab-bar scroll chevrons appear");
        let stats = vcx
            .debug_bounds("selection-stats")
            .expect("the stats group is still painted, never scrolled away");
        assert!(
            f32::from(stats.origin.x) > f32::from(chevrons.origin.x),
            "the static stats group sits to the right of the (scrolling) tab chevrons"
        );
    }

    #[gpui::test]
    fn tab_bar_leading_divider_gated_on_stats(cx: &mut TestAppContext) {
        // The leading divider (§9A.3) renders only when the stats readout is shown, so it never
        // floats alone — the render gates it on the same `stats_readout_parts().is_some()` this test
        // exercises directly.
        let h = one_sheet(cx);
        assert!(
            upd(&h, cx, |c, _w, _cx| c.stats_readout_parts().is_none()),
            "a single-cell selection hides the readout → no leading divider"
        );
        show_stats(&h, cx);
        assert!(
            upd(&h, cx, |c, _w, _cx| c.stats_readout_parts().is_some()),
            "a numeric multi-cell selection shows the readout → leading divider renders"
        );
    }

    #[gpui::test]
    fn chevron_click_animates_to_target(cx: &mut TestAppContext) {
        // The tab strip starts scrolled to the left: the left chevron is a no-op there, and the
        // right chevron ANIMATES the content toward the end (offset slides negative over frames —
        // D10.2, replacing the D9.2 instant jump).
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        let at_start = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            at_start.abs() < 1.0,
            "a fresh scroller starts at offset 0, got {at_start}"
        );

        // Left chevron at the start is disabled → clicking it arms no slide and does not scroll.
        let left = vcx
            .debug_bounds("tab-bar-chevron-left")
            .expect("left chevron painted");
        vcx.simulate_click(left.center(), Modifiers::default());
        vcx.run_until_parked();
        assert!(
            !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "the disabled left chevron at the start arms no animation"
        );
        let after_left = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            after_left.abs() < 1.0,
            "the left chevron at the start is a no-op, got {after_left}"
        );

        // Right chevron arms an animated slide (`target` set, `is_animating`) rather than an instant
        // jump: even after the first redraw it's still mid-flight (one 60%-step is not arrival), so
        // the reader sees a slide, not a teleport to the destination.
        let right = vcx
            .debug_bounds("tab-bar-chevron-right")
            .expect("right chevron painted");
        vcx.simulate_click(right.center(), Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "clicking the right chevron arms an in-flight slide"
        );

        // Each frame steps the offset monotonically toward the (negative) clamped target, then the
        // slide settles and clears `target`.
        let mut prev = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        let mut moved_negative = false;
        for _ in 0..20 {
            pump_frames(&mut vcx, 1);
            let now = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
            assert!(
                now <= prev + 0.01,
                "the slide only moves toward the end (never backward): {now} > {prev}"
            );
            if now < -1.0 {
                moved_negative = true;
            }
            prev = now;
            if !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()) {
                break;
            }
        }
        assert!(
            moved_negative,
            "the animated slide moved the content toward the end, got final {prev}"
        );
        assert!(
            !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "the slide self-terminates once it reaches the target"
        );

        // It lands at the clamped `scroll_step` destination (0.8 × viewport from the start), within
        // the scroll range.
        let landed = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(landed < -1.0, "settled toward the end, got {landed}");
    }

    #[gpui::test]
    fn chevron_animation_clamps_at_end(cx: &mut TestAppContext) {
        // Repeated right-chevron clicks (each fully animated) drive the tab scroller to the end and
        // no further; the right chevron then disables (`at_end`) and arms no more slides.
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        // Click + fully settle several times; each click resolves before the next.
        for _ in 0..12 {
            if let Some(right) = vcx.debug_bounds("tab-bar-chevron-right") {
                vcx.simulate_click(right.center(), Modifiers::default());
                vcx.run_until_parked();
                // Settle this slide before the next click.
                for _ in 0..20 {
                    pump_frames(&mut vcx, 1);
                    if !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()) {
                        break;
                    }
                }
            }
        }

        // At the end, the right chevron is disabled: clicking it arms no further slide and the
        // offset does not move past the limit.
        let at_limit = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        if let Some(right) = vcx.debug_bounds("tab-bar-chevron-right") {
            vcx.simulate_click(right.center(), Modifiers::default());
            vcx.run_until_parked();
            pump_frames(&mut vcx, 3);
        }
        let after = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            (after - at_limit).abs() < 1.0,
            "at the end the content stays pinned at the limit ({at_limit} → {after})"
        );
        assert!(
            at_limit < -1.0,
            "the strip did scroll to a non-trivial end, got {at_limit}"
        );
    }

    #[gpui::test]
    fn multiselect_disables_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(1, 1),
                    active: cell(3, 3),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Disabled);
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.ref_box_text()), "B2:D4");
    }

    // ---- Data row: edit / commit / escape / cap ------------------------------------------

    #[gpui::test]
    fn enter_commits_and_moves_down(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1+1", window, cx);
            c.test_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=1+1"),
            "expected SetCellInput, got {cmds:?}"
        );
        let reqs = h.grid_requests.borrow();
        assert!(reqs.iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Down))
        )));
        assert!(reqs
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn shift_enter_moves_up(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(5, 0)), window, cx);
            c.test_type("v", window, cx);
            c.test_press_enter(true, window, cx);
        });
        let reqs = h.grid_requests.borrow();
        assert!(reqs.iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Up))
        )));
    }

    #[gpui::test]
    fn escape_reverts_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.test_type("999", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        upd(&h, cx, |c, window, cx| c.escape_edit(window, cx));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn cap_reject_keeps_editing_and_flags_error(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.test_type(&huge, window, cx);
            c.test_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            !cmds
                .iter()
                .any(|cmd| matches!(cmd, Command::SetCellInput { .. })),
            "a cap-rejected formula must not be committed"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert!(upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        // The popover shows the length-specific message (`functional_spec.md §4.2`).
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cap_error_message()),
            Some("Formula too long (max 8,192 characters)".to_string())
        );
        // The next keystroke clears the danger state + popover.
        upd(&h, cx, |c, window, cx| c.test_type("=1", window, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.cap_error_message()), None);
    }

    #[gpui::test]
    fn edit_commit_requested_commits_without_moving(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        let committed = upd(&h, cx, |c, window, cx| {
            c.on_edit_commit_requested(window, cx)
        });
        assert!(committed);
        let cmds = h.client.take_commands();
        assert!(matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=A1"));
        assert!(
            !h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))),
            "click-away commit does not move the active cell itself"
        );
    }

    #[gpui::test]
    fn commit_then_adopt_commits_pending_edit_and_adopts(cx: &mut TestAppContext) {
        // The shared choke point (used by the paste / grid-selection / sheet-switch consumers):
        // a pending edit is committed to the OLD cell, then the new selection is adopted — never
        // reaching `on_selection_changed` while Editing (`components/grid.md`).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=9", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        h.client.take_commands();
        let adopted = upd(&h, cx, |c, window, cx| {
            c.commit_then_adopt_selection(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(
            adopted,
            "a valid pending edit commits, so the selection is adopted"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        let cmds = h.client.take_commands();
        // The edit is committed to the OLD cell (A1)...
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Command::SetCellInput { cell: cc, input, .. } if *cc == cell(0, 0) && input == "=9"
            )),
            "pending edit must commit to the edited cell (not be lost), got {cmds:?}"
        );
        // ...and the NEW cell (B2) is fetched (selection adopted).
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Command::GetCellContent { cell: cc, .. } if *cc == cell(1, 1)
            )),
            "adopted selection must fetch the new active cell, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn commit_then_adopt_blocks_on_cap_reject(cx: &mut TestAppContext) {
        // A cap-rejected edit blocks adoption: the field stays Editing and the new selection is
        // NOT adopted (no fetch for it), so the caller can keep the grid on the last accepted cell.
        let h = one_sheet(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type(&huge, window, cx);
        });
        h.client.take_commands();
        let adopted = upd(&h, cx, |c, window, cx| {
            c.commit_then_adopt_selection(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(!adopted, "a cap-rejected edit must block adoption");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        let cmds = h.client.take_commands();
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, Command::SetCellInput { .. })),
            "a cap-rejected edit must not commit, got {cmds:?}"
        );
        assert!(
            !cmds.iter().any(|c| matches!(
                c,
                Command::GetCellContent { cell: cc, .. } if *cc == cell(1, 1)
            )),
            "the blocked selection must not be adopted/fetched, got {cmds:?}"
        );
    }

    // ---- Action row: toggles + fill --------------------------------------------------------

    #[gpui::test]
    fn toggle_bold_sends_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Bold, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Bold,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn toggles_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                bold: true,
                italic: false,
                underline: true,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.bold_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.italic_active()));
        assert!(upd(&h, cx, |c, _w, _cx| c.underline_active()));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_toggles_send_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Strikethrough, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Strikethrough,
                ..
            }]
        ));
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::WrapText, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::WrapText,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                strikethrough: true,
                wrap: false,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.strikethrough_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.wrap_active()));
    }

    #[gpui::test]
    fn fill_swatch_and_no_fill(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        let accent = FILL_PALETTE[4].rgb; // Accent 1
        upd(&h, cx, |c, window, cx| {
            c.apply_fill(Some(accent), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if *rgb == accent
        ));
        upd(&h, cx, |c, window, cx| c.apply_fill(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Fill(None),
                ..
            }]
        ));
    }

    #[gpui::test]
    fn formatting_commits_pending_edit_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Italic, window, cx)
        });
        let cmds = h.client.take_commands();
        // Commit first, then the style.
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { input, .. }) if input == "=A1"),
            "pending edit committed first, got {cmds:?}"
        );
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::SetStyleAttr {
                attr: StyleAttr::Italic,
                ..
            }
        )));
    }

    // ---- Action row: insert chart (P17) ---------------------------------------------------

    /// Criterion #1: inserting a chart is a mutating action-row control — it commits any pending
    /// in-cell edit FIRST (the same rule as every sibling), so the worker's subsequent publish +
    /// grid refresh can't clobber a dangling edit.
    #[gpui::test]
    fn insert_chart_commits_pending_edit_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        // The pending edit commits FIRST, then the chart insert.
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { input, .. }) if input == "=A1"),
            "pending edit committed first, got {cmds:?}"
        );
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::InsertChart {
                kind: ChartInsertKind::Line,
                ..
            }
        )));
    }

    /// Criterion #1 (blocking half): a cap-rejected pending edit blocks the insert — no
    /// `InsertChart` is sent and the field stays editing, so the invalid edit isn't clobbered.
    #[gpui::test]
    fn cap_rejected_edit_blocks_insert_chart(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type(&huge, window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, Command::InsertChart { .. })),
            "a cap-rejected pending edit blocks the insert, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    /// Inserting anchors the chart at the active cell (8×15 cells) on the active sheet.
    #[gpui::test]
    fn insert_chart_sends_command_for_active_sheet_from_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(3, 2)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Column, window, cx)
        });
        let cmds = h.client.take_commands();
        let inserted = cmds.iter().find_map(|cmd| match cmd {
            Command::InsertChart {
                sheet,
                kind,
                anchor,
                data,
            } => Some((*sheet, *kind, *anchor, *data)),
            _ => None,
        });
        let (sheet, kind, anchor, data) = inserted.expect("an InsertChart command was sent");
        assert_eq!(sheet, SheetId(0));
        assert_eq!(kind, ChartInsertKind::Column);
        // From the active cell (col 2, row 3), spanning 8 cols × 15 rows.
        assert_eq!((anchor.from.col, anchor.from.row), (2, 3));
        assert_eq!((anchor.to.col, anchor.to.row), (2 + 8, 3 + 15));
        // A single-cell selection carries no data range (the chart stays near-empty).
        assert_eq!(data, None, "a single-cell selection seeds no data range");
    }

    /// Batch 3 item 8: inserting a chart with a **range** selection (more than one cell) threads that
    /// range into `InsertChart` as its data, so the worker binds it at creation (no "Use selection"
    /// click). A single-cell selection threads `None` (covered above), keeping the near-empty chart.
    #[gpui::test]
    fn insert_chart_with_range_selection_seeds_data_range(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // Select a real B2:D9 block, then insert.
        let sel = SelectionModel {
            anchor: cell(1, 1),
            active: cell(8, 3),
        };
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(sel, window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        let data = cmds.iter().find_map(|cmd| match cmd {
            Command::InsertChart { data, .. } => Some(*data),
            _ => None,
        });
        assert_eq!(
            data,
            Some(Some(sel.range())),
            "a range selection is threaded into InsertChart as its data, got {cmds:?}"
        );
    }

    /// Criterion #2 (disabled-in-degraded parity): OPEN the chart menu, THEN degrade — the menu
    /// must close (so a type glyph can't be clicked after the trigger disables), mirroring how the
    /// other formatting popovers close on degrade.
    #[gpui::test]
    fn degrade_closes_open_chart_menu(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_chart_menu(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open()),
            "the chart menu opened"
        );
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.chart_menu_open()),
            "degrading closes the open chart menu"
        );
        assert!(upd(&h, cx, |c, _w, _cx| c.is_degraded()));
    }

    // ---- Conditional-formatting sidebar (P4, `components/cf_sidebar.md`) -------------------

    /// A minimal published rule row for a given range (P4 doesn't render rows, but the sidebar
    /// carries them, so the re-scope / refresh tests assert on `rows`).
    fn cf_rule(range: &str) -> freecell_core::CfRuleView {
        freecell_core::CfRuleView {
            index: 0,
            range: range.to_string(),
            priority: 1,
            editable: true,
            summary: format!("Cell value > 100 ({range})"),
            preview: freecell_core::CfPreview::Highlight {
                fill: None,
                text_color: None,
            },
            spec: None,
        }
    }

    fn cf_rows_len(h: &Harness, cx: &mut TestAppContext) -> Option<usize> {
        upd(h, cx, |c, _w, _cx| {
            c.cond_fmt.as_ref().map(|p| p.rows.len())
        })
    }

    #[gpui::test]
    fn cond_fmt_button_toggles_sidebar(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "the toggle opens the sidebar"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(0)),
            "it opens on the active sheet"
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "toggling again closes it"
        );
    }

    #[gpui::test]
    fn opening_cond_fmt_closes_chart_panel(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7))
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            None,
            "opening the CF sidebar closes the chart panel (shared right dock)"
        );
    }

    #[gpui::test]
    fn opening_chart_panel_closes_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7))
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "opening the chart panel closes the CF sidebar (shared right dock)"
        );
    }

    #[gpui::test]
    fn selection_change_does_not_close_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(3, 2)), window, cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "a grid selection change must NOT close the CF sidebar (the range-pick exemption)"
        );
    }

    #[gpui::test]
    fn sheet_switch_rescopes_cond_fmt(cx: &mut TestAppContext) {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Sheet2"),
            ],
            SheetId(0),
        );
        h.client
            .set_cond_fmt_rules(SheetId(1), vec![cf_rule("B2:B20")]);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(0))
        );
        assert_eq!(cf_rows_len(&h, cx), Some(0), "sheet 0 has no CF rules");
        // A window-driven sheet switch re-scopes the open sidebar to the new sheet + rebuilds rows.
        upd(&h, cx, |c, _w, cx| c.adopt_active_sheet(SheetId(1), cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(1)),
            "the sidebar re-scopes to the new sheet"
        );
        assert_eq!(
            cf_rows_len(&h, cx),
            Some(1),
            "and rebuilds its rows from the new sheet's published rules"
        );
    }

    #[gpui::test]
    fn cond_fmt_updated_refreshes_rows(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(cf_rows_len(&h, cx), Some(0), "opens with no rules");
        // A CF mutation republishes the rule list; `refresh_cond_fmt` (the `CondFmtUpdated` handler)
        // rebuilds the sidebar's rows from the published map.
        h.client
            .set_cond_fmt_rules(SheetId(0), vec![cf_rule("A1:A10")]);
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert_eq!(
            cf_rows_len(&h, cx),
            Some(1),
            "the sidebar rebuilt its rows from the published map"
        );
    }

    #[gpui::test]
    fn degrade_closes_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "degrade closes the CF sidebar (like the chart panel)"
        );
    }

    // ---- Conditional-formatting rules list (P5, `components/cf_sidebar.md §5`) --------------

    /// A no-fill/no-text highlight preview — the common shape for the list-rendering tests.
    fn cf_highlight() -> CfPreview {
        CfPreview::Highlight {
            fill: None,
            text_color: None,
        }
    }

    /// A published rule row with a chosen storage `index`, `summary`, editability, and preview.
    /// `priority` is irrelevant to rendering (the client publishes the list already
    /// priority-sorted; the row order is the vec order), so it is fixed at 0.
    fn cf_view(
        index: u32,
        range: &str,
        summary: &str,
        editable: bool,
        preview: CfPreview,
    ) -> CfRuleView {
        CfRuleView {
            index,
            range: range.to_string(),
            priority: 0,
            editable,
            summary: summary.to_string(),
            preview,
            spec: None,
        }
    }

    /// The open sidebar's row summaries, top-to-bottom (priority order).
    fn cf_row_summaries(h: &Harness, cx: &mut TestAppContext) -> Vec<String> {
        upd(h, cx, |c, _w, _cx| {
            c.cond_fmt
                .as_ref()
                .expect("sidebar open")
                .rows
                .iter()
                .map(|r| r.summary.clone())
                .collect()
        })
    }

    #[test]
    fn cf_row_controls_reflect_position_and_editability() {
        let editable = cf_view(1, "A1:A10", "Cell value > 100", true, cf_highlight());
        let badge = cf_view(
            4,
            "C1:C9",
            "Data bar",
            false,
            CfPreview::Badge("Data bar".to_string()),
        );

        let first = cf_row_controls(&editable, true, false);
        assert!(
            !first.move_up,
            "the first (highest-priority) row can't move up"
        );
        assert!(first.move_down);
        let last = cf_row_controls(&editable, false, true);
        assert!(last.move_up);
        assert!(
            !last.move_down,
            "the last (lowest-priority) row can't move down"
        );
        let middle = cf_row_controls(&editable, false, false);
        assert!(middle.move_up && middle.move_down);
        assert!(
            first.edit && first.delete,
            "an editable highlight rule can be edited AND deleted"
        );

        let badge_controls = cf_row_controls(&badge, false, false);
        assert!(
            !badge_controls.edit,
            "a deferred-family Badge rule can't be edited"
        );
        assert!(
            badge_controls.delete,
            "but a deferred-family Badge rule can still be deleted"
        );
    }

    #[gpui::test]
    fn cf_list_renders_one_row_per_rule(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Cell value > 100", true, cf_highlight()),
                cf_view(
                    2,
                    "B2:B20",
                    "3-color scale",
                    true,
                    CfPreview::ColorScale {
                        colors: vec![Rgb::from_hex(0x63BE7B), Rgb::from_hex(0xF8696B)],
                    },
                ),
                cf_view(
                    9,
                    "C1:C9",
                    "Data bar",
                    false,
                    CfPreview::Badge("Data bar".to_string()),
                ),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // One row painted per published rule, keyed by its stable storage index (5/2/9).
        for sel in ["cf-row-5", "cf-row-2", "cf-row-9"] {
            assert!(
                vcx.debug_bounds(sel).is_some(),
                "{sel} must render one row per rule"
            );
        }
        assert!(
            vcx.debug_bounds("cf-empty").is_none(),
            "no empty state renders while rules exist"
        );
    }

    #[gpui::test]
    fn cf_empty_state_shown_when_no_rules(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-empty").is_some(),
            "the empty state shows when the sheet carries no rules"
        );
        assert!(vcx.debug_bounds("cf-row-0").is_none(), "no rows render");
        assert!(
            vcx.debug_bounds("cond-fmt-add-rule").is_some(),
            "the '+ Add rule' button stays available in the empty state"
        );
    }

    #[gpui::test]
    fn cf_delete_sends_delete_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(3, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.delete_cf_rule(3));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::DeleteCondFmt {
                    sheet: SheetId(0),
                    index: 3
                }]
            ),
            "delete sends DeleteCondFmt for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_move_up_sends_raise(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(2, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.raise_cf_rule(2));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::RaiseCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "move-up sends RaiseCondFmtPriority for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_move_down_sends_lower(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(2, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.lower_cf_rule(2));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::LowerCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "move-down sends LowerCondFmtPriority for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_commands_noop_when_sidebar_closed(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The sidebar is closed → no target sheet → the mutators are no-ops (they never target the
        // active sheet blindly).
        upd(&h, cx, |c, _w, _cx| {
            c.raise_cf_rule(0);
            c.lower_cf_rule(0);
            c.delete_cf_rule(0);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a closed CF sidebar sends no CF command"
        );
    }

    #[gpui::test]
    fn cf_delete_button_click_sends_delete(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let del = vcx
            .debug_bounds("cf-row-5-delete")
            .expect("the row's delete control is painted");
        vcx.simulate_click(del.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::DeleteCondFmt {
                    sheet: SheetId(0),
                    index: 5
                }]
            ),
            "clicking a row's delete button sends DeleteCondFmt for that row (button → method wiring)"
        );
    }

    #[gpui::test]
    fn cf_first_row_move_up_disabled(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // First row (highest priority) — its move-up is disabled, so a click sends nothing.
        let up_first = vcx
            .debug_bounds("cf-row-5-up")
            .expect("first row move-up painted");
        vcx.simulate_click(up_first.center(), Modifiers::default());
        assert!(
            h.client.take_commands().is_empty(),
            "the first row's move-up is disabled (a click is inert)"
        );
        // A lower row's move-up is enabled and raises its priority.
        let up_second = vcx
            .debug_bounds("cf-row-2-up")
            .expect("second row move-up painted");
        vcx.simulate_click(up_second.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::RaiseCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "a non-first row's move-up raises its priority"
        );
    }

    #[gpui::test]
    fn cf_last_row_move_down_disabled(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // Last row (lowest priority) — its move-down is disabled, so a click sends nothing.
        let down_last = vcx
            .debug_bounds("cf-row-2-down")
            .expect("last row move-down painted");
        vcx.simulate_click(down_last.center(), Modifiers::default());
        assert!(
            h.client.take_commands().is_empty(),
            "the last row's move-down is disabled (a click is inert)"
        );
        // A higher row's move-down is enabled and lowers its priority.
        let down_first = vcx
            .debug_bounds("cf-row-5-down")
            .expect("first row move-down painted");
        vcx.simulate_click(down_first.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::LowerCondFmtPriority {
                    sheet: SheetId(0),
                    index: 5
                }]
            ),
            "a non-last row's move-down lowers its priority"
        );
    }

    #[gpui::test]
    fn cf_list_reorders_after_cond_fmt_updated(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(0, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(1, "B1:B10", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(
            cf_row_summaries(&h, cx).join(","),
            "Rule A,Rule B",
            "rows render in the published (priority) order"
        );
        // A raise/lower swapped their priority; the worker republishes the reordered list. The
        // `CondFmtUpdated` refresh rebuilds the rows in the new order.
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(1, "B1:B10", "Rule B", true, cf_highlight()),
                cf_view(0, "A1:A10", "Rule A", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert_eq!(
            cf_row_summaries(&h, cx).join(","),
            "Rule B,Rule A",
            "the list reflects the republished order after CondFmtUpdated"
        );
    }

    // ---- CF rule editor (P6) --------------------------------------------------------------

    /// A B2:B20 selection (anchor B2 → active B20).
    fn selection_b2_b20() -> SelectionModel {
        SelectionModel {
            anchor: cell(1, 1),
            active: cell(19, 1),
        }
    }

    /// A published, editable highlight row carrying `spec` (so edit mode can seed from it). The
    /// preview mirrors the spec's format; `cf_view` (spec: None) is the deferred-family case.
    fn cf_spec_view(index: u32, range: &str, summary: &str, spec: CfRuleSpec) -> CfRuleView {
        let preview = match spec.format() {
            Some(f) => CfPreview::Highlight {
                fill: f.fill,
                text_color: f.text_color,
            },
            None => CfPreview::ColorScale { colors: vec![] },
        };
        CfRuleView {
            index,
            range: range.to_string(),
            priority: 0,
            editable: true,
            summary: summary.to_string(),
            preview,
            spec: Some(spec),
        }
    }

    /// The open editor's text-input values `(range, operand1)`.
    fn cf_input_values(h: &Harness, cx: &mut TestAppContext) -> (String, String) {
        upd(h, cx, |c, _w, cx| {
            (
                c.cf_range_input.read(cx).value().to_string(),
                c.cf_operand1_input.read(cx).value().to_string(),
            )
        })
    }

    #[gpui::test]
    fn cf_add_editor_seeds_range_from_selection(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.selection = selection_b2_b20();
            c.open_cf_editor(None, window, cx);
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "opening the add editor enters Editor mode"
        );
        assert_eq!(
            cf_input_values(&h, cx).0,
            "B2:B20",
            "the Applies-to range seeds from the current selection"
        );
    }

    #[gpui::test]
    fn cf_add_cell_value_rule_saves_add_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.selection = selection_b2_b20();
            c.open_cf_editor(None, window, cx);
            // Default kind = Cell value, op = greater than. Enter the operand + pick a preset.
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("100", window, cx));
            c.set_cf_format(CF_PRESETS[0].1, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        let expected = CfRuleSpec::CellIs {
            op: CfValueOp::Gt,
            operand: "100".to_string(),
            operand2: None,
            format: CF_PRESETS[0].1,
            stop_if_true: false,
        };
        match h.client.take_commands().as_slice() {
            [Command::AddCondFmt { sheet, range, spec }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(range, "B2:B20");
                assert_eq!(*spec, expected);
            }
            other => panic!("expected one AddCondFmt, got {other:?}"),
        }
    }

    #[gpui::test]
    fn cf_save_success_returns_to_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("5", window, cx));
            c.save_cf_editor(cx);
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "the editor stays open pending the save's CondFmtUpdated"
        );
        // The worker accepted the rule + republished → CondFmtUpdated → refresh returns to List mode.
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "a successful save returns to List mode"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "the sidebar itself stays open"
        );
    }

    #[gpui::test]
    fn cf_edit_row_seeds_form_and_saves_update(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let spec = CfRuleSpec::CellIs {
            op: CfValueOp::Ge,
            operand: "50".to_string(),
            operand2: None,
            format: CF_PRESETS[2].1,
            stop_if_true: false,
        };
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_spec_view(4, "C1:C9", "Cell value ≥ 50", spec.clone())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(Some(4), window, cx);
        });
        // The form seeds from the row's spec: range, operand, and the editor state.
        let (range, operand1) = cf_input_values(&h, cx);
        assert_eq!(range, "C1:C9", "the range seeds from the edited row");
        assert_eq!(operand1, "50", "the operand seeds from the spec");
        upd(&h, cx, |c, _w, _cx| {
            let e = c
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .expect("editor open");
            assert_eq!(e.edit_index, Some(4));
            assert_eq!(e.kind, CfEditorKind::CellValue);
            assert_eq!(e.value_op, CfValueOp::Ge);
            assert_eq!(e.format, CF_PRESETS[2].1);
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        match h.client.take_commands().as_slice() {
            [Command::UpdateCondFmt {
                sheet,
                index,
                range,
                spec: got,
            }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(*index, 4);
                assert_eq!(range, "C1:C9");
                assert_eq!(*got, spec);
            }
            other => panic!("expected one UpdateCondFmt, got {other:?}"),
        }
    }

    #[gpui::test]
    fn cf_empty_operand_blocks_save(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx); // Cell value, operand left empty
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        assert!(
            h.client.take_commands().is_empty(),
            "an empty Cell-value operand blocks Save (no command sent)"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "the editor stays open when Save is blocked"
        );
    }

    #[gpui::test]
    fn cf_rule_type_dropdown_selects_kind(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // Open the rule-type dropdown, then pick "Text".
        let trigger = vcx
            .debug_bounds("cf-type-trigger")
            .expect("the rule-type trigger is painted");
        vcx.simulate_click(trigger.center(), Modifiers::default());
        let text_opt = vcx
            .debug_bounds("cf-type-text")
            .expect("the Text option is painted while the dropdown is open");
        vcx.simulate_click(text_opt.center(), Modifiers::default());
        let kind = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .kind
        });
        assert_eq!(kind, CfEditorKind::Text, "the dropdown drives the kind");
    }

    #[gpui::test]
    fn cf_preset_sets_format(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let preset = vcx
            .debug_bounds("cf-preset-0")
            .expect("the first format preset chip is painted");
        vcx.simulate_click(preset.center(), Modifiers::default());
        let format = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .format
        });
        assert_eq!(
            format, CF_PRESETS[0].1,
            "clicking a preset chip sets the whole differential format"
        );
    }

    #[gpui::test]
    fn cf_preview_reflects_format(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            // A preset with a fill; the live preview cell must paint.
            c.set_cf_format(CF_PRESETS[0].1, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-format-preview").is_some(),
            "the live preview cell renders for the current format"
        );
        let fill = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .format
                .fill
        });
        assert_eq!(
            fill,
            Some(Rgb::from_hex(0xFFC7CE)),
            "the preview is driven by the editor's working fill"
        );
    }

    #[gpui::test]
    fn cf_cancel_returns_to_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.cf_editor_open()));
        upd(&h, cx, |c, _w, cx| c.cancel_cf_editor(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "Cancel leaves Editor mode"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "Cancel returns to List mode, sidebar still open"
        );
    }

    #[gpui::test]
    fn cf_engine_error_keeps_editor_open(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("5", window, cx));
            c.save_cf_editor(cx);
        });
        // The worker refuses the rule (e.g. a bad range) → the window routes it inline.
        upd(&h, cx, |c, _w, cx| {
            c.show_cf_editor_error("Invalid range".to_string(), cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "an engine error keeps the editor open"
        );
        let errors = upd(&h, cx, |c, _w, _cx| {
            c.cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .errors
                .clone()
        });
        assert_eq!(errors, vec!["Invalid range".to_string()]);
    }

    #[test]
    fn cf_color_scale_row_edit_disabled() {
        // A color-scale rule is editable in the engine model, but P6 has no color-scale editor —
        // its edit control is disabled here (P7 re-enables it), while delete stays available.
        let scale = CfRuleView {
            index: 3,
            range: "A1:A9".to_string(),
            priority: 0,
            editable: true,
            summary: "3-color scale".to_string(),
            preview: CfPreview::ColorScale {
                colors: vec![Rgb::from_hex(0x63BE7B), Rgb::from_hex(0xF8696B)],
            },
            spec: Some(CfRuleSpec::ColorScale { stops: vec![] }),
        };
        let controls = cf_row_controls(&scale, false, false);
        assert!(
            !controls.edit,
            "a color-scale row's edit is disabled until P7"
        );
        assert!(controls.delete, "but it stays deletable");
    }

    /// A minimal editor state (fresh add-defaults, `edit_index == Some(1)`) with only the
    /// kind-relevant fields set — the exact shape [`cf_state_from_spec`] reconstructs, so a
    /// build→seed round-trip can assert full equality.
    fn cf_state(mutate: impl FnOnce(&mut CfEditorState)) -> CfEditorState {
        let mut s = CfEditorState::new(Some(1));
        mutate(&mut s);
        s
    }

    /// The exhaustive mapping + round-trip guard: for **every** highlight `CfEditorKind` and its
    /// sub-toggles, `cf_build_spec` must produce the expected `CfRuleSpec` variant+fields, and
    /// `cf_state_from_spec(build(state))` must reproduce the state + operand strings exactly. This
    /// red-flags any future field transposition (e.g. `blanks_no`↔`errors_no`, or a swapped
    /// `bottom`/`percent`) that the per-kind mappings would otherwise hide. Pure — no gpui context.
    #[test]
    fn cf_build_spec_and_state_round_trip_cover_every_highlight_kind() {
        use CfEditorKind::*;
        // A non-default format proves the fill/text/bold/italic + stop_if_true seed round-trips
        // across non-CellValue kinds (used by Text + Formula below).
        let fancy = CfFormat {
            fill: Some(Rgb::from_hex(0xC6EFCE)),
            text_color: Some(Rgb::from_hex(0x006100)),
            bold: true,
            italic: true,
        };
        let plain = CfFormat::default();

        // (label, state, operand1, operand2, formula, expected spec)
        let cases: Vec<(&str, CfEditorState, &str, &str, &str, CfRuleSpec)> = vec![
            (
                "cell value gt",
                cf_state(|s| s.value_op = CfValueOp::Gt),
                "100",
                "",
                "",
                CfRuleSpec::CellIs {
                    op: CfValueOp::Gt,
                    operand: "100".to_string(),
                    operand2: None,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "cell value between",
                cf_state(|s| {
                    s.value_op = CfValueOp::Between;
                    s.stop_if_true = true;
                }),
                "10",
                "20",
                "",
                CfRuleSpec::CellIs {
                    op: CfValueOp::Between,
                    operand: "10".to_string(),
                    operand2: Some("20".to_string()),
                    format: plain,
                    stop_if_true: true,
                },
            ),
            (
                "text contains (fancy format)",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::Contains;
                    s.format = fancy;
                    s.stop_if_true = true;
                }),
                "foo",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::Contains,
                    value: "foo".to_string(),
                    format: fancy,
                    stop_if_true: true,
                },
            ),
            (
                "text not-contains",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::NotContains;
                }),
                "bar",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::NotContains,
                    value: "bar".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text begins-with",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::BeginsWith;
                }),
                "pre",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::BeginsWith,
                    value: "pre".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text ends-with",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::EndsWith;
                }),
                "post",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::EndsWith,
                    value: "post".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text equals",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::Equals;
                }),
                "exact",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::Equals,
                    value: "exact".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "dates today",
                cf_state(|s| {
                    s.kind = Dates;
                    s.period = CfPeriod::Today;
                }),
                "",
                "",
                "",
                CfRuleSpec::TimePeriod {
                    period: CfPeriod::Today,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "dates last-month",
                cf_state(|s| {
                    s.kind = Dates;
                    s.period = CfPeriod::LastMonth;
                }),
                "",
                "",
                "",
                CfRuleSpec::TimePeriod {
                    period: CfPeriod::LastMonth,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "top N",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 25;
                    s.top_percent = false;
                    s.top_bottom = false;
                }),
                "25",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 25,
                    percent: false,
                    bottom: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "top percent",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 10;
                    s.top_percent = true;
                    s.top_bottom = false;
                }),
                "10",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 10,
                    percent: true,
                    bottom: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "bottom N",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 5;
                    s.top_percent = false;
                    s.top_bottom = true;
                }),
                "5",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 5,
                    percent: false,
                    bottom: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "bottom percent",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 15;
                    s.top_percent = true;
                    s.top_bottom = true;
                }),
                "15",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 15,
                    percent: true,
                    bottom: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "above average",
                cf_state(|s| {
                    s.kind = Average;
                    s.average_below = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Average {
                    below: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "below average",
                cf_state(|s| {
                    s.kind = Average;
                    s.average_below = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Average {
                    below: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "duplicate values",
                cf_state(|s| {
                    s.kind = Duplicate;
                    s.duplicate_unique = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::DuplicateValues {
                    unique: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "unique values",
                cf_state(|s| {
                    s.kind = Duplicate;
                    s.duplicate_unique = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::DuplicateValues {
                    unique: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "blank",
                cf_state(|s| {
                    s.kind = Blanks;
                    s.blanks_no = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Blanks {
                    no_blanks: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "no blanks",
                cf_state(|s| {
                    s.kind = Blanks;
                    s.blanks_no = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Blanks {
                    no_blanks: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "error",
                cf_state(|s| {
                    s.kind = Errors;
                    s.errors_no = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Errors {
                    no_errors: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "no errors",
                cf_state(|s| {
                    s.kind = Errors;
                    s.errors_no = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Errors {
                    no_errors: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "formula (fancy format)",
                cf_state(|s| {
                    s.kind = Formula;
                    s.format = fancy;
                    s.stop_if_true = true;
                }),
                "",
                "",
                "=A1>0",
                CfRuleSpec::Formula {
                    formula: "=A1>0".to_string(),
                    format: fancy,
                    stop_if_true: true,
                },
            ),
        ];

        // Sanity: every highlight kind is represented at least once.
        let covered: Vec<CfEditorKind> = cases.iter().map(|(_, s, ..)| s.kind).collect();
        for kind in [
            CellValue, Text, Dates, TopBottom, Average, Duplicate, Blanks, Errors, Formula,
        ] {
            assert!(
                covered.contains(&kind),
                "the table must exercise every highlight kind (missing {kind:?})"
            );
        }

        for (label, state, op1, op2, formula, expected) in cases {
            // Forward: the editor state maps to the expected engine spec.
            let built = cf_build_spec(&state, op1, op2, formula);
            assert_eq!(built, expected, "cf_build_spec mismatch for `{label}`");
            // Round-trip: seeding the editor back from that spec reproduces the state exactly.
            let (state2, r1, r2, rf) =
                cf_state_from_spec(1, &built).expect("highlight specs are authorable");
            assert_eq!(state2, state, "state round-trip mismatch for `{label}`");
            assert_eq!(
                (r1.as_str(), r2.as_str(), rf.as_str()),
                (op1, op2, formula),
                "operand round-trip mismatch for `{label}`"
            );
        }
    }

    // ---- Chart edit panel (P19) -----------------------------------------------------------

    #[gpui::test]
    fn chart_panel_opens_and_closes(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target()), None);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7)),
            "the panel opens on the given chart"
        );
        upd(&h, cx, |c, _w, cx| c.close_chart_panel(cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target()), None);
    }

    /// A type glyph in the panel sends `SetChartType` for the panel's chart and updates the shown
    /// kind optimistically.
    #[gpui::test]
    fn chart_panel_type_glyph_sends_set_chart_type(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.set_chart_type_from_panel(ChartInsertKind::Column, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetChartType {
                id: ChartId(7),
                kind: ChartInsertKind::Column,
                ..
            }]
        ));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel.as_ref().map(|p| p.kind)),
            Some(ChartInsertKind::Column),
            "the shown type updates optimistically"
        );
    }

    /// The "use selection" button binds the chart to the current grid selection as its data range.
    #[gpui::test]
    fn chart_panel_apply_range_uses_current_selection(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(4, 3),
                },
                window,
                cx,
            )
        });
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.apply_chart_range_from_selection(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetChartRange { id: ChartId(7), data, .. }]
                    if *data == freecell_core::CellRange::new(cell(0, 0), cell(4, 3))
            ),
            "the current selection is applied as the chart's range, got {cmds:?}"
        );
    }

    /// Moderate-fix regression: "Use selection" binds to the sheet the SELECTION is on (the active
    /// sheet), not the chart's host sheet — so a chart hosted on one sheet can be bound to another
    /// sheet's data (valid cross-sheet), and a stale host sheet never silently mis-qualifies the c:f.
    #[gpui::test]
    fn chart_panel_apply_range_binds_the_active_sheet_not_the_host(cx: &mut TestAppContext) {
        // The user is on sheet 1 ("Data"); the panel edits a chart HOSTED on sheet 0 ("Host").
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Host"),
                SheetTab::new(SheetId(1), "Data"),
            ],
            SheetId(1),
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(4, 1),
                },
                window,
                cx,
            )
        });
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.apply_chart_range_from_selection(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetChartRange { sheet: SheetId(1), id: ChartId(7), data }]
                    if *data == freecell_core::CellRange::new(cell(0, 0), cell(4, 1))
            ),
            "the range binds the active/data sheet (1), not the chart's host sheet (0): {cmds:?}"
        );
    }

    /// Degrading closes the panel and makes its controls inert (like the action-bar popovers).
    #[gpui::test]
    fn degrade_closes_chart_panel(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_none()),
            "degrading closes the edit panel"
        );
        upd(&h, cx, |c, window, cx| {
            c.set_chart_type_from_panel(ChartInsertKind::Bar, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a closed/degraded panel sends no command"
        );
    }

    // ---- Chart edit panel: chrome (P20) ---------------------------------------------------

    /// The canonical chrome-editing [`ChartPanel`] over chart 7 (host sheet 0) with one series —
    /// authored, seeded title "Chart". Spread with `..chart_7_panel()` to vary a field.
    fn chart_7_panel() -> ChartPanel {
        ChartPanel {
            sheet: SheetId(0),
            id: ChartId(7),
            is_authored: true,
            kind: ChartInsertKind::Line,
            ranges: None,
            title: Some("Chart".into()),
            legend: Some(LegendPosition::Right),
            cat_axis_title: None,
            val_axis_title: None,
            series: vec![ChartPanelSeries {
                name: "Widgets".into(),
                color: Some(Rgb::from_hex(0x4472C4)),
            }],
            labels: DataLabelToggles::default(),
        }
    }

    /// Open a chrome-editing panel over chart 7 (host sheet 0) with one series.
    fn open_chrome_panel(h: &Harness, cx: &mut TestAppContext, is_authored: bool) {
        upd(h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    is_authored,
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        h.client.take_commands();
    }

    /// The single `SetChartChrome` edit sent for chart 7, or a panic.
    fn last_chrome_edit(h: &Harness) -> ChartChromeEdit {
        match h.client.take_commands().as_slice() {
            [Command::SetChartChrome {
                id: ChartId(7),
                edit,
                ..
            }] => edit.clone(),
            other => panic!("expected one SetChartChrome for chart 7, got {other:?}"),
        }
    }

    #[gpui::test]
    fn chart_panel_title_sends_chrome_and_updates_optimistically(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_title_from_panel(Some("Sales".into()), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Sales".into()))
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c
                .chart_panel
                .as_ref()
                .and_then(|p| p.title.clone())),
            Some("Sales".into()),
            "the shown title updates optimistically",
        );
    }

    /// Typing in the title field + pressing Enter commits the title as a chrome edit.
    #[gpui::test]
    fn chart_panel_title_input_commits_on_enter(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("Renamed", window, cx));
            c.on_chart_title_event(
                &handle,
                &InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                },
                window,
                cx,
            );
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Renamed".into())),
        );
    }

    /// Clearing the title field to empty commits `Title(None)` (remove the title).
    #[gpui::test]
    fn chart_panel_empty_title_input_clears_the_title(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // seeded title = "Chart"
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Title(None));
    }

    #[gpui::test]
    fn chart_panel_legend_off_and_position(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(None, window, cx)
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Legend(None));
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(Some(LegendPosition::Bottom), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Legend(Some(LegendPosition::Bottom))
        );
    }

    #[gpui::test]
    fn chart_panel_axis_title_sends_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_axis_title_from_panel(
                ChartAxisKind::Value,
                Some("Units".into()),
                window,
                cx,
            )
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Value,
                title: Some("Units".into()),
            },
        );
    }

    #[gpui::test]
    fn chart_panel_series_color_sends_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_series_color_from_panel(0, Some(Rgb::from_hex(0x70AD47)), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: Some(Rgb::from_hex(0x70AD47)),
            },
        );
        // Clearing back to Auto sends None.
        upd(&h, cx, |c, window, cx| {
            c.set_chart_series_color_from_panel(0, None, window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: None
            },
        );
    }

    #[gpui::test]
    fn chart_panel_data_labels_send_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        let toggles = DataLabelToggles {
            show_value: true,
            show_category_name: false,
            show_percent: true,
        };
        upd(&h, cx, |c, window, cx| {
            c.set_chart_data_labels_from_panel(toggles, window, cx)
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::DataLabels(toggles));
    }

    /// A **loaded** chart's panel still edits chrome (the same commands), while its provenance is
    /// recorded so the render hides the authored-only Type + Data-range sections.
    #[gpui::test]
    fn loaded_chart_panel_edits_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, false);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c
                .chart_panel
                .as_ref()
                .map(|p| p.is_authored)),
            Some(false),
            "the panel records the loaded provenance",
        );
        upd(&h, cx, |c, window, cx| {
            c.set_chart_title_from_panel(Some("Reviewed".into()), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Reviewed".into())),
        );
    }

    /// The full chrome panel actually **paints** without panicking — both the authored variant (Type +
    /// Data range + every chrome section incl. the per-series swatches) and the loaded variant
    /// (chrome-only). Forces a real draw through the test renderer (the chrome is out of pixel scope,
    /// so this + the Xvfb smoke launch are its render validation).
    #[gpui::test]
    fn chart_panel_paints_for_both_provenances(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        open_chrome_panel(&h, cx, true);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the authored panel painted and stayed open"
        );

        upd(&h, cx, |c, _w, cx| c.close_chart_panel(cx));
        open_chrome_panel(&h, cx, false);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the loaded (chrome-only) panel painted and stayed open"
        );
    }

    /// Batch 3 item 10: the action-bar new-chart dropdown and the right-docked edit panel can be open
    /// **at the same time** and the chrome paints without panicking. The panel is pushed as the
    /// bottom-most overlay so the dropdown (pushed later) paints ABOVE it; this forces a real draw of
    /// that coexistence path (chrome is out of pixel scope — z-order itself is verified in the Xvfb
    /// smoke launch, this guards the both-open render path).
    #[gpui::test]
    fn chart_menu_and_panel_coexist_and_paint(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        open_chrome_panel(&h, cx, true); // right-docked edit panel open (authored chart 7)
        upd(&h, cx, |c, _w, cx| c.toggle_chart_menu(cx)); // action-bar new-chart dropdown open
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open())
                && upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the dropdown and the edit panel are both open"
        );
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        // Both survived the paint (the overlay-ordering path drew cleanly, no panic).
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open())
                && upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the dropdown-over-panel overlay stack painted and both stayed open"
        );
    }

    /// Degrading makes the chrome setters inert (a closed panel sends nothing).
    #[gpui::test]
    fn degrade_makes_chrome_setters_inert(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(None, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a degraded panel sends no chrome edit"
        );
    }

    /// CR guard: a text field that gained focus for chart A must NOT commit its (stale) text to a
    /// DIFFERENT chart if the panel re-points before the field's Blur is processed (rapid selection
    /// switch). The captured focus target (A) no longer matches the panel (B), so the commit is dropped.
    #[gpui::test]
    fn chart_panel_stale_field_commit_is_dropped_after_repoint(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // panel on chart 7
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            // Focus captures the target (sheet 0, chart 7); then the field holds stale text for it.
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Stale for chart 7", window, cx));
        });
        // The panel re-points to a DIFFERENT chart under the still-focused field (the event-ordering
        // race: a direct re-point that does NOT re-seed, so the field keeps its stale text).
        upd(&h, cx, |c, _w, _cx| {
            c.chart_panel = Some(ChartPanel::skeleton(
                SheetId(0),
                ChartId(9),
                true,
                ChartInsertKind::Line,
            ));
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "stale field text must not be sent to the re-pointed chart",
        );
    }

    /// The counterpart: a focused field whose panel is UNCHANGED commits normally (proving the guard
    /// drops only stale, re-pointed commits — not every focused edit).
    #[gpui::test]
    fn chart_panel_focused_field_commits_when_panel_unchanged(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Kept", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Kept".into())),
            "a same-chart focus→blur commits the field",
        );
    }

    // ---- Chart edit panel: post-v1 Batch 2 (live titles / click-away / scroll / order) ----

    /// Item 6: typing in the Title field commits the chart title **live, per keystroke** (`Change`) —
    /// no Enter/blur needed. Each keystroke sends the current text as a chrome edit.
    #[gpui::test]
    fn chart_panel_title_input_commits_live_on_each_keystroke(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // seeded title = "Chart"
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("S", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("S".into())),
            "the first keystroke commits live",
        );
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("Sa", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Sa".into())),
            "the next keystroke commits live too",
        );
    }

    /// Item 6: the axis-title fields also commit live per keystroke.
    #[gpui::test]
    fn chart_panel_axis_title_input_commits_live_on_change(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // cat/val axis titles seeded empty
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_cat_axis_input.clone();
            c.on_chart_cat_axis_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Month", window, cx));
            c.on_chart_cat_axis_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Category,
                title: Some("Month".into()),
            },
        );
    }

    /// Item 6 anti-clobber: a live **republish of the same chart** (a same-id reconcile) must NOT
    /// re-seed the field, so it never overwrites what the user is actively typing.
    #[gpui::test]
    fn chart_panel_same_chart_republish_does_not_clobber_typing(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7, title "Chart" seeded
                                         // The user is mid-type: the field holds unsaved text (set_value stands in for typing).
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("My draft title", window, cx));
        });
        // A worker republish reconciles the SAME chart (id 7) with the stale snapshot title.
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    title: Some("Chart".into()),
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c
                .chart_title_input
                .read(cx)
                .value()
                .to_string()),
            "My draft title",
            "a same-chart republish must not re-seed / clobber the in-progress edit",
        );
    }

    /// Item 6 anti-clobber counterpart: **switching** the selected chart (a new id) DOES re-seed the
    /// fields to the new chart's values.
    #[gpui::test]
    fn chart_panel_switch_reseeds_the_title_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7, title "Chart"
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    id: ChartId(9),
                    title: Some("Nine".into()),
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c
                .chart_title_input
                .read(cx)
                .value()
                .to_string()),
            "Nine",
            "switching to another chart re-seeds the field to its value",
        );
    }

    /// Item 12: a grid selection change (a click on a cell / empty grid) closes the edit panel.
    #[gpui::test]
    fn chart_panel_closes_on_grid_click_away(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        assert!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(2, 2)), window, cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_none()),
            "a grid click (selection change) closes the edit panel",
        );
    }

    /// Item 12: selecting **another chart** re-points the panel (the window routes a chart click
    /// through `open_chart_panel`, not `on_selection_changed`), so it switches rather than closing.
    #[gpui::test]
    fn chart_panel_another_chart_switches_not_closes(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(9), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(9)),
            "clicking another chart switches the panel to it, not close",
        );
    }

    /// Item 11: the legend icon buttons set the right position / off (behavior unchanged, just
    /// iconized). Exercises the same setter the icon `on_click`s call.
    #[gpui::test]
    fn chart_panel_legend_icons_set_position_and_off(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        for pos in [
            Some(LegendPosition::Top),
            Some(LegendPosition::Right),
            Some(LegendPosition::Left),
            Some(LegendPosition::Bottom),
            None,
        ] {
            upd(&h, cx, |c, window, cx| {
                c.set_chart_legend_from_panel(pos, window, cx)
            });
            assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Legend(pos));
        }
    }

    /// Item 13: the shared chart-type order places **Area right after Line** (Excel grouping), then
    /// the Column/Bar pair — the single canonical order used by both the panel Type row and the
    /// action-bar dropdown.
    #[test]
    fn chart_type_order_places_area_after_line() {
        let kinds: Vec<ChartInsertKind> = CHART_MENU.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(
            &kinds[..4],
            &[
                ChartInsertKind::Line,
                ChartInsertKind::Area,
                ChartInsertKind::Column,
                ChartInsertKind::Bar,
            ],
            "Line → Area → Column → Bar",
        );
    }

    /// Item 7: the panel paints (scrollable body + clipped to its bounds) on a **short** window where
    /// its control stack overflows — without panicking and while staying open.
    #[gpui::test]
    fn chart_panel_paints_scrollable_on_a_short_window(cx: &mut TestAppContext) {
        let h = build_win(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            160.0,
        );
        upd(&h, cx, |c, _w, cx| {
            let body: gpui::AnyView = cx.new(|_| ShortBodyStub).into();
            c.set_grid_body(body, cx);
        });
        open_chrome_panel(&h, cx, true);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the panel paints (scrollable + clipped) on a short window and stays open",
        );
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Select `cell` as a single-cell selection and drain the resulting fetch command.
    fn select_single(h: &Harness, cx: &mut TestAppContext, r: u32, c: u32) {
        upd(h, cx, |chrome, window, cx| {
            chrome.on_selection_changed(SelectionModel::single(cell(r, c)), window, cx)
        });
        h.client.take_commands();
    }

    #[gpui::test]
    fn alignment_toggle_emits_clear_on_repress(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly right-aligned.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                h_align: Some(Align::Right),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.align_active(Align::Right)));

        // Re-pressing the pressed alignment clears horizontal only (value "general").
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Right, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "general"
            ),
            "re-press clears with general, got {cmds:?}"
        );

        // Pressing a different (unpressed) alignment sets it directly.
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Left, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "left"
        ));
    }

    #[gpui::test]
    fn vertical_alignment_sets_and_reflects(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly top-aligned → the Top button reads pressed, others not.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Top),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Top)));
        assert!(!upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Bottom)));

        // Pressing a vertical-align button is a plain set (no re-press-to-clear).
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Bottom, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "bottom"
        ));

        // Re-pressing the already-active alignment re-applies it (no clear value).
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Center),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Center, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "center"
        ));
    }

    #[gpui::test]
    fn text_color_automatic_and_swatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        // Automatic clears the color (empty value).
        upd(&h, cx, |c, window, cx| c.apply_text_color(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()
            ),
            "Automatic clears color, got {cmds:?}"
        );

        // A swatch sends its #RRGGBB hex.
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value == "#FF0000"
        ));
    }

    #[gpui::test]
    fn num_fmt_pick_emits_code_and_category_reflects_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00%");
        select_single(&h, cx, 1, 1);
        // The dropdown label reflects the active cell's format category.
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Percent"
        );

        upd(&h, cx, |c, window, cx| {
            c.apply_num_fmt("$#,##0.00", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"
        ));
    }

    #[gpui::test]
    fn num_fmt_category_label_reflects_new_categories(cx: &mut TestAppContext) {
        // The grouped model added Scientific / Accounting categories; the dropdown button label must
        // reverse-map an active cell's code to the new category names.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00E+00");
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Scientific"
        );

        h.client
            .set_num_fmt(SheetId(0), cell(1, 1), "$#,##0.00;($#,##0.00)");
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Accounting"
        );
    }

    #[gpui::test]
    fn num_fmt_preset_pick_emits_grouped_code(cx: &mut TestAppContext) {
        // Picking a preset from a multi-preset group routes that exact code to the set-format command.
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_num_fmt("yyyy-mm-dd", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "yyyy-mm-dd"
            ),
            "a grouped Date preset routes its exact code, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn thousands_toggle_adds_and_removes_grouping(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // An ungrouped numeric format: toggle enabled, not active → adds grouping.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.00"
            ),
            "toggling on adds grouping, got {cmds:?}"
        );

        // A grouped format: enabled + active → removes grouping.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.00"
            ),
            "toggling off removes grouping, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn thousands_toggle_disabled_for_date_and_degraded(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // A date format has no integer digit placeholder → disabled + no-op.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "m/d/yyyy");
        select_single(&h, cx, 1, 1);
        assert!(!upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a non-toggleable format sends nothing"
        );

        // A toggleable format disables once degraded.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
    }

    #[gpui::test]
    fn decimals_buttons_emit_adjusted_code(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.000"
            ),
            "increase decimals rewrites the code, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_for_date_format(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "m/d/yyyy");
        select_single(&h, cx, 1, 1);
        // A date format has no adjustable decimal group → both buttons disabled + no-op.
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a no-op decimals adjust sends nothing"
        );
    }

    #[gpui::test]
    fn dropdown_anchors_capture_button_positions_left_to_right(cx: &mut TestAppContext) {
        // BUG 2c: each dropdown popover anchors under its real (content-sized) trigger button, not
        // a hardcoded x. After a paint, the `canvas` probes capture each button's laid-out left
        // edge; they must land in left-to-right action-row order and be strictly increasing.
        let h = one_sheet(cx);
        // Force the window to paint so the canvas probes capture each button's laid-out x.
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }

        let xs = upd(&h, cx, |c, _w, _cx| {
            [
                c.anchor_x_of(Anchor::FontFamily),
                c.anchor_x_of(Anchor::FontSize),
                c.anchor_x_of(Anchor::TextColor),
                c.anchor_x_of(Anchor::Fill),
                c.anchor_x_of(Anchor::Borders),
                c.anchor_x_of(Anchor::NumFmt),
            ]
        });
        assert!(
            xs[0] >= 0.0 && xs.windows(2).all(|w| w[1] > w[0]),
            "trigger anchors must be captured in strictly increasing left-to-right order, got {xs:?}"
        );
    }

    #[gpui::test]
    fn decimals_enabled_on_general_numeric_cell(cx: &mut TestAppContext) {
        // BUG 3: a plain number like `200000` is stored with the General format. The ± must still
        // be adjustable — increase applies `0.0`; decrease is a no-op at zero decimals (disabled).
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Number, "200000");
        select_single(&h, cx, 1, 1);

        assert!(
            upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
            "increase must be enabled on a General-formatted number"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            "decrease is a no-op on a General integer (0 decimals)"
        );

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.0"
            ),
            "increase on a General number applies a 0.0 format, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_on_general_text_cell(cx: &mut TestAppContext) {
        // A text cell under General is not numeric → the ± stay disabled and no-op.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Text, "hello");
        select_single(&h, cx, 1, 1);

        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a text General cell must not emit a number-format change"
        );
    }

    #[gpui::test]
    fn decimals_gating_for_custom_formats_matches_spec(cx: &mut TestAppContext) {
        // BUG C audit: for a cell with an explicit *custom* number format, ± must be enabled iff the
        // format is safely adjustable — single-section, no exponent (`E`/`e`), no quoted/escaped
        // literal (`functional_spec.md §3.4`, the deliberate Phase-4 gate). This locks the exact
        // enable/disable set so it can be reconciled against what the owner observed.
        let h = one_sheet(cx);
        fn gate(h: &Harness, cx: &mut TestAppContext, code: &str) -> (bool, bool) {
            h.client.set_num_fmt(SheetId(0), cell(1, 1), code);
            select_single(h, cx, 1, 1);
            (
                upd(h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
                upd(h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            )
        }
        // Safe single-section customs ARE enabled: increase always, decrease when ≥1 decimal.
        assert_eq!(gate(&h, cx, "0.00"), (true, true), "0.00");
        assert_eq!(gate(&h, cx, "#,##0.00"), (true, true), "#,##0.00");
        assert_eq!(gate(&h, cx, "0.00%"), (true, true), "0.00%");
        // `#,##0` has zero decimals → increase enabled, decrease a correct no-op (Excel: can't go
        // below 0). This is NOT a bug: the format IS adjustable, there is just nothing to remove.
        assert_eq!(gate(&h, cx, "#,##0"), (true, false), "#,##0");
        // Only exponent / quoted / multi-section customs are (correctly) disabled both ways.
        assert_eq!(gate(&h, cx, "0.00E+00"), (false, false), "0.00E+00");
        assert_eq!(gate(&h, cx, "0.0\"x\""), (false, false), "0.0\"x\"");
        assert_eq!(
            gate(&h, cx, "0.00;[Red]0.00"),
            (false, false),
            "0.00;[Red]0.00"
        );
    }

    #[gpui::test]
    fn controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        // Enabled before degradation.
        assert!(!upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));

        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        // The decimals gate folds in the degraded flag (the other controls disable via
        // `.disabled(self.is_degraded())` in the render path).
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
    }

    #[gpui::test]
    fn degraded_closes_popovers_and_blocks_dispatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Open the text-color popover, then degrade.
        upd(&h, cx, |c, _w, cx| c.toggle_text_color_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a swatch click can no longer dispatch a command.
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetStylePath dispatches while degraded"
        );
    }

    // ---- BUG A/B: popover item clicks APPLY (real mouse dispatch, not direct `apply_*`) -----
    //
    // These drive real mouse events through the rendered popover with a `VisualTestContext` over a
    // full-height backdrop (`tall_sheet` mounts a tall body stub so the backdrop — `size_full` of
    // the chrome — actually spans the dropdown items) — the path the part-1 anchor test and the
    // `apply_*` unit tests never exercised. Pre-fix, EVERY mouse-down inside the card reached the
    // backdrop: the menu `Button`s insert a plain (Normal) hitbox and only `prevent_default()` on
    // down (never `.occlude()`/`stop_propagation`), and the backdrop's `on_mouse_down` is not gated
    // on `default_prevented`, so a down directly on an item — as well as on the p_1/p_2 padding and
    // the gaps between rows — fired the backdrop's dismiss, tearing the popover down before the
    // item's `on_click` (mouse-UP) could dispatch. Wrapping the card in `.occlude()` inserts a
    // BlockMouse hitbox that breaks the hit-test before the backdrop for ALL in-card presses, so no
    // in-popover press can dismiss it. The mouse-DOWN is the discriminating signal (a full
    // `simulate_click` would not catch the regression: it sends down+up with no intervening repaint,
    // so the doomed button's `on_click` still fires); each per-item test below asserts the down
    // keeps the popover open — and fails without the card `.occlude()`.

    /// Opens a popover via `open`, paints, presses mouse **down** on the item registered under
    /// debug-selector `item`, asserts `open_flag` still holds (the down did not reach the backdrop
    /// dismiss — the BUG A/B guard), then releases and returns the dispatched commands.
    fn press_popover_button(
        h: &Harness,
        cx: &mut TestAppContext,
        open: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>),
        item: &'static str,
        open_flag: impl Fn(&ChromeView) -> bool,
    ) -> Vec<Command> {
        upd(h, cx, |c, w, cx| open(c, w, cx));
        h.client.take_commands(); // drop anything incidental to opening; isolate the click
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds(item)
            .unwrap_or_else(|| panic!("popover item {item:?} was not painted"))
            .center();
        let mods = gpui::Modifiers::default();
        vcx.simulate_mouse_down(center, MouseButton::Left, mods);
        let alive = vcx.update(|_w, cx| open_flag(h.chrome.read(cx)));
        assert!(
            alive,
            "popover item {item:?}: a mouse-DOWN must not dismiss the popover"
        );
        vcx.simulate_mouse_up(center, MouseButton::Left, mods);
        h.client.take_commands()
    }

    #[gpui::test]
    fn card_padding_click_keeps_popover_open(cx: &mut TestAppContext) {
        // Covers the card region a press can land on that ISN'T an item — the p_1 padding ring and
        // the gaps between rows. Like the buttons, pre-fix this reached the backdrop's dismiss
        // listener and closed the popover; the card `.occlude()` shields it too. Verified to fail
        // without the fix.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // The card's top-left padding corner (inside the p_1 border, above the first menu button).
        let pad = gpui::point(card.origin.x + px(1.0), card.origin.y + px(1.0));
        vcx.simulate_mouse_down(pad, MouseButton::Left, gpui::Modifiers::default());
        assert!(
            vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "a press on the card's padding must not dismiss the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "a press on the card padding dispatches no command"
        );
    }

    #[gpui::test]
    fn numfmt_currency_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Grouped presets are keyed by their exact code (`numfmt-<code>`); the `$1,234.56` Currency
        // preset sends `$#,##0.00`.
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_num_fmt_popover(cx),
            "numfmt-$#,##0.00",
            |c| c.num_fmt_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"),
            "clicking the Currency preset must dispatch its num-fmt, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying"
        );
    }

    // ---- Phase 10.1: number-format dropdown basics-first + "More ▸" drill-in ----------------

    #[gpui::test]
    fn num_fmt_basic_menu_paints_seven_and_more_row(cx: &mut TestAppContext) {
        // Basics-first: opening the dropdown paints the seven basic presets flat + a "More ▸" row,
        // and NONE of the More-only grouped inventory (`0.00` — the "1234.56" Number preset) nor a
        // "◂ Back" row. This is the regression fix — the common formats are visible without scroll.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("numfmt-card").is_some(),
            "the number-format card must be painted when open"
        );
        // The seven basic presets (`debug_bounds` needs a `'static` selector, so enumerate them).
        for sel in [
            "numfmt-general",
            "numfmt-#,##0.00",
            "numfmt-$#,##0.00",
            "numfmt-0.00%",
            "numfmt-m/d/yyyy",
            "numfmt-h:mm AM/PM",
            "numfmt-@",
        ] {
            assert!(
                vcx.debug_bounds(sel).is_some(),
                "basic preset {sel} must be painted in the basics-first view"
            );
        }
        assert!(
            vcx.debug_bounds("numfmt-more").is_some(),
            "the 'More ▸' row must be painted"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_none(),
            "a More-only preset (0.00) must NOT be painted in the basic view"
        );
        assert!(
            vcx.debug_bounds("numfmt-back").is_none(),
            "the '◂ Back' row must NOT be painted in the basic view"
        );
    }

    #[gpui::test]
    fn num_fmt_more_drilldown_and_back(cx: &mut TestAppContext) {
        // Clicking "More ▸" drills into the full grouped inventory (a More-only preset + the
        // "◂ Back" row now paint); clicking "◂ Back" restores the basics-first view.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let more = vcx
            .debug_bounds("numfmt-more")
            .expect("the 'More ▸' row was painted")
            .center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_more_open),
            "clicking 'More ▸' must enter the drill-in view"
        );
        assert!(
            vcx.debug_bounds("numfmt-back").is_some(),
            "the '◂ Back' row must be painted in the More view"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_some(),
            "a More-only preset (0.00) must be painted in the More view"
        );

        let back = vcx
            .debug_bounds("numfmt-back")
            .expect("the '◂ Back' row was painted")
            .center();
        vcx.simulate_click(back, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            !vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_more_open),
            "clicking '◂ Back' must restore the basics-first view"
        );
        assert!(
            vcx.debug_bounds("numfmt-more").is_some(),
            "the 'More ▸' row must be painted again after Back"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_none(),
            "the More-only preset must be gone after Back"
        );
    }

    #[gpui::test]
    fn num_fmt_basic_pick_applies_and_closes(cx: &mut TestAppContext) {
        // Selecting a basic preset from the basics-first view routes its exact code and closes the
        // popover (and resets the drill-in state).
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_num_fmt_popover(cx),
            "numfmt-#,##0.00",
            |c| c.num_fmt_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.00"),
            "the basic Number preset must dispatch #,##0.00, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "the drill-in state must reset after applying"
        );
    }

    #[gpui::test]
    fn num_fmt_more_pick_applies_and_closes(cx: &mut TestAppContext) {
        // Drilling into "More ▸" and selecting a More-only preset routes its exact code and closes.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let more = vcx
            .debug_bounds("numfmt-more")
            .expect("the 'More ▸' row was painted")
            .center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        vcx.run_until_parked();
        h.client.take_commands(); // isolate the preset click
        let preset = vcx
            .debug_bounds("numfmt-0.00")
            .expect("the More-only preset (0.00) was painted")
            .center();
        vcx.simulate_click(preset, gpui::Modifiers::default());
        vcx.run_until_parked();
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.00"),
            "the More-only preset must dispatch 0.00, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying from the More view"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "the drill-in state must reset after applying"
        );
    }

    #[gpui::test]
    fn num_fmt_opens_onto_more_for_more_only_active(cx: &mut TestAppContext) {
        // Discoverability (D10.1): when the active cell's format lives only under "More ▸", opening
        // the dropdown lands directly on the grouped view so the current format is visible; a basic
        // active format opens basics-first.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00E+00"); // Scientific — More-only
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "a More-only active format must open onto the grouped view"
        );
        // Close, then a basic active format opens basics-first.
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "$#,##0.00"); // Currency — basic
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must be open"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "a basic active format must open basics-first"
        );
    }

    #[gpui::test]
    fn text_color_automatic_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_text_color_popover(cx),
            "text-automatic",
            |c| c.text_color_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()),
            "Automatic must clear the font colour (empty value), got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open));
    }

    #[gpui::test]
    fn fill_no_fill_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_fill_popover(cx),
            "fill-no-fill",
            |c| c.fill_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr {
                    attr: StyleAttr::Fill(None),
                    ..
                }]
            ),
            "No fill must clear the fill, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn fill_swatch_click_applies_and_closes(cx: &mut TestAppContext) {
        // A swatch applies on `on_mouse_down` (the backdrop also dismissed on that same down pre-fix,
        // but the swatch's own listener still ran, so the command went out either way). This is
        // positive coverage that the card `.occlude()` doesn't break a swatch's own down-to-apply. A
        // single down suffices to dispatch its command.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_fill_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds("fill-swatch-Background 1")
            .expect("the first fill swatch was painted")
            .center();
        vcx.simulate_mouse_down(center, MouseButton::Left, gpui::Modifiers::default());
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if rgb.to_hex() == 0xFFFFFF
            ),
            "the first swatch must apply its colour, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn font_family_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Item 0 is always "Default (Inter)" → clears the family override (sent as `Some("")`).
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_family_popover(cx),
            "font-family-0",
            |c| c.font_family_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()),
            "Default (Inter) must clear the font family, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_family_open));
    }

    #[gpui::test]
    fn font_size_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_size_popover(cx),
            "font-size-14",
            |c| c.font_size_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 14.0).abs() < 1e-6),
            "clicking 14 must set the font size to 14 pt, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_size_open));
    }

    #[gpui::test]
    fn border_target_icon_click_paints_and_stays_open(cx: &mut TestAppContext) {
        // Pen model (`functional_spec.md §2.1`): a real click on the "All" target icon paints the
        // pen onto those edges AND — unlike the old apply-and-close preset path — leaves the
        // popover open with the target selected. `press_popover_button` already asserts the
        // mouse-DOWN doesn't dismiss; here we additionally assert it is still open after mouse-UP.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_borders_popover(cx),
            "border-all",
            |c| c.borders_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetBorders {
                    preset: BorderPreset::All,
                    line: BorderLine::ThinSolid,
                    color: Some(rgb),
                    ..
                }] if rgb.to_hex() == 0x000000
            ),
            "clicking All must paint the default thin-solid-black pen onto All, got {cmds:?}"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open),
            "the popover must STAY OPEN after a target click (pen model)"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            Some(BorderPreset::All),
            "the clicked target must become selected"
        );
    }

    #[gpui::test]
    fn popover_backdrop_outside_click_dismisses_without_dispatch(cx: &mut TestAppContext) {
        // The occluded card must still let a click OUTSIDE it hit the backdrop → dismiss (and never
        // dispatch a command).
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // A point on the backdrop but clear of the card: same top strip as the card (so it is within
        // the backdrop, which only spans the chrome height when no grid body is hosted) but far to
        // its left (the number-format trigger anchors the card on the right).
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );
        vcx.simulate_click(outside, gpui::Modifiers::default());

        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "a click outside the card dismisses the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "dismissing via the backdrop dispatches no command"
        );
    }

    #[gpui::test]
    fn popover_outside_click_removes_card_on_next_render_without_hover(cx: &mut TestAppContext) {
        // BUG B: the backdrop's dismiss closure must `cx.notify()` so the view repaints on the
        // very next frame. Without the notify the open-flag flips false but the view is never
        // marked dirty, so the popover card stays painted until some *unrelated* later event (a
        // hover/mouse-move) happens to repaint it — exactly the reported "won't close until the
        // mouse moves" symptom.
        //
        // The element-level discriminator: `debug_bounds` reads `window.rendered_frame`, which
        // only changes on an actual draw, and `simulate_event` ends in `run_until_parked`, which
        // redraws a window ONLY if something marked it dirty. So a single outside mouse-DOWN that
        // clears the flag but does not notify leaves the *previous* frame — card still present —
        // standing, with no intervening mouse-move. This asserts the card is GONE on that next
        // frame. Reverting the `cx.notify()` in `render_num_fmt_popover`'s backdrop closure makes
        // this fail (card still painted on the next render). Verified fail-without / pass-with.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted while open");
        // A point on the backdrop but clear of the card (top strip, far left of the right-anchored
        // card) — same geometry the sibling outside-click test uses.
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );

        // A single mouse-DOWN on the backdrop, and crucially NO following mouse-move / hover.
        vcx.simulate_mouse_down(outside, MouseButton::Left, gpui::Modifiers::default());

        // The flag flipped false...
        assert!(
            !vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "the outside press must clear the open flag"
        );
        // ...AND the dismiss notified, so the view repainted on the very next frame and the card
        // element is gone — no intervening hover needed. This is the assertion that fails without
        // the `cx.notify()`.
        assert!(
            vcx.debug_bounds("numfmt-card").is_none(),
            "the popover card must be gone on the very next render (the dismiss must cx.notify)"
        );
    }

    // ---- Action row: SetBorders (pen popover) ---------------------------------------------

    /// The pen dispatched by one `SetBorders`, asserting it is the single command and returning its
    /// `(preset, line, color)` for the test to check. Also asserts the range is the whole selection.
    fn one_border_cmd(cmds: &[Command]) -> (BorderPreset, BorderLine, Option<Rgb>) {
        match cmds {
            [Command::SetBorders {
                preset,
                line,
                color,
                range,
                ..
            }] => {
                assert_eq!(
                    *range,
                    freecell_core::CellRange::single(cell(1, 1)),
                    "the paint must cover the selection"
                );
                (*preset, *line, *color)
            }
            other => panic!("expected exactly one SetBorders, got {other:?}"),
        }
    }

    #[gpui::test]
    fn borders_popover_toggles(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
    }

    #[gpui::test]
    fn select_border_target_paints_and_stays_open(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        let (preset, line, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Outer);
        assert_eq!(line, BorderLine::ThinSolid, "the default pen line");
        assert_eq!(
            color.map(|c| c.to_hex()),
            Some(0x000000),
            "the default pen color (explicit black)"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open()),
            "a target click keeps the popover open (pen model)"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            Some(BorderPreset::Outer)
        );
    }

    #[gpui::test]
    fn set_border_line_with_target_repaints(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        // Changing the line with a target selected repaints that target with the new pen.
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::Dashed, window, cx)
        });
        let (preset, line, _) = one_border_cmd(&h.client.take_commands());
        assert_eq!((preset, line), (BorderPreset::Outer, BorderLine::Dashed));
    }

    #[gpui::test]
    fn set_border_color_with_target_repaints(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| c.set_border_color(red, window, cx));
        let (preset, _, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Outer);
        assert_eq!(color, Some(red), "the target repaints in the new pen color");
    }

    #[gpui::test]
    fn pen_carries_across_target_switch(cx: &mut TestAppContext) {
        // Set a non-default pen on Outer, then switch to Top — the carried-over pen paints Top.
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::Dashed, window, cx);
            c.set_border_color(red, window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Top, window, cx)
        });
        let (preset, line, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Top);
        assert_eq!(
            line,
            BorderLine::Dashed,
            "pen line carries across the switch"
        );
        assert_eq!(color, Some(red), "pen color carries across the switch");
    }

    #[gpui::test]
    fn set_border_line_without_target_updates_pen_only(cx: &mut TestAppContext) {
        // No target selected: changing the line updates the pen but changes nothing on the sheet
        // (MVP; P2 restyle-all is deferred — GAPS F2).
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::ThickSolid, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "changing the line with no target selected must not touch the sheet"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_line()),
            BorderLine::ThickSolid,
            "the pen still updates (the next target click paints with it)"
        );
    }

    #[gpui::test]
    fn set_border_color_without_target_updates_pen_only(cx: &mut TestAppContext) {
        // Symmetric to the line path: with no target selected, changing the color updates the pen
        // only — no sheet change (MVP; P2 restyle-all is deferred — GAPS F2).
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| c.set_border_color(red, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "changing the color with no target selected must not touch the sheet"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_color()),
            red,
            "the pen still updates (the next target click paints with it)"
        );
    }

    #[gpui::test]
    fn border_none_clears_and_deselects(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        // Select a real target first so we can see None clear it.
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::None, window, cx)
        });
        let (preset, _, _) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::None, "None dispatches a clear");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "None leaves no target selected"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open()),
            "None clears but does not close the popover (only click-away/Esc closes)"
        );
    }

    #[test]
    fn border_target_icon_mask_matches_border_type_edges() {
        // The 2×2 icon's per-preset dark-edge table is the one piece of new UI logic with no render
        // coverage (the harness doesn't render the chrome popover), so pin it here: a future
        // Top/Bottom (or inner/outer) swap fails loudly. Tuple = (top, bottom, left, right,
        // inner_h, inner_v). Mirrors `functional_spec.md §2.2` / IronCalc's per-`BorderType` edges.
        use BorderPreset::*;
        assert_eq!(
            border_target_icon_mask(All),
            (true, true, true, true, true, true),
            "All darkens every outer edge + the inner cross"
        );
        assert_eq!(
            border_target_icon_mask(Inner),
            (false, false, false, false, true, true),
            "Inner darkens only the inner cross"
        );
        assert_eq!(
            border_target_icon_mask(Outer),
            (true, true, true, true, false, false),
            "Outer darkens only the perimeter"
        );
        assert_eq!(
            border_target_icon_mask(BorderPreset::None),
            (false, false, false, false, false, false),
            "None darkens nothing (all grey)"
        );
        assert_eq!(
            border_target_icon_mask(Top),
            (true, false, false, false, false, false),
            "Top darkens only the top outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Bottom),
            (false, true, false, false, false, false),
            "Bottom darkens only the bottom outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Left),
            (false, false, true, false, false, false),
            "Left darkens only the left outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Right),
            (false, false, false, true, false, false),
            "Right darkens only the right outer edge"
        );
    }

    #[gpui::test]
    fn borders_reopen_resets_target_and_pen(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        // Dirty the transient state: a target + a non-default pen.
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx);
            c.set_border_line(BorderLine::Double, window, cx);
            c.set_border_color(Rgb::from_hex(0xFF0000), window, cx);
        });
        // Close, then reopen.
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "reopen resets the target"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_line()),
            BorderLine::ThinSolid,
            "reopen resets the pen line to the default"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_color().to_hex()),
            0x000000,
            "reopen resets the pen color to black"
        );
    }

    #[gpui::test]
    fn borders_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a target click can no longer dispatch.
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetBorders dispatches while degraded"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "a degraded target click leaves no target selected"
        );
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    #[gpui::test]
    fn font_dropdowns_reflect_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_font_family(SheetId(0), cell(1, 1), "Arial");
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                font_size_q: 48, // 12pt
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.font_family_label().to_string()),
            "Arial"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "12");
    }

    #[gpui::test]
    fn font_size_box_shows_workbook_default_for_default_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // A default cell (no explicit font_size_q) shows the WORKBOOK default (13pt for a new
        // workbook) — not a hardcoded "11" that would mismatch the cell (CR Moderate).
        h.client.set_default_font_size_pt(13.0);
        select_single(&h, cx, 1, 1);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "13");

        // An opened file whose default is 10pt shows "10" for its default cells (and re-picking 10
        // is a no-op in the engine, so no size jump).
        h.client.set_default_font_size_pt(10.0);
        select_single(&h, cx, 2, 2);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "10");
    }

    #[gpui::test]
    fn font_family_pick_and_system_default(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Times New Roman", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetFont { family: Some(f), size_pt: None, .. }] if f == "Times New Roman"
            ),
            "family pick emits SetFont, got {cmds:?}"
        );

        // "Default (Inter)" clears the override (family = Some("")).
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family(SYSTEM_DEFAULT_FAMILY, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()
        ));
    }

    #[gpui::test]
    fn font_size_pick_emits_setfont(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| c.apply_font_size(18.0, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 18.0).abs() < 1e-9
        ));
    }

    #[gpui::test]
    fn font_controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // A pick made while degraded dispatches nothing.
        upd(&h, cx, |c, window, cx| c.apply_font_size(24.0, window, cx));
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Arial", window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetFont dispatches while degraded"
        );
    }

    // ---- Action row / data row: the two 250 ms spinners -----------------------------------

    #[gpui::test]
    fn eval_spinner_hidden_for_short_eval(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalStarted, window, cx)
        });
        tick(cx, 100);
        assert!(!upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalFinished, window, cx)
        });
        tick(cx, 300);
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()),
            "a fast eval never flashes the spinner"
        );
    }

    #[gpui::test]
    fn eval_spinner_shown_for_long_eval(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalStarted, window, cx)
        });
        tick(cx, 300);
        assert!(upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalFinished, window, cx)
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
    }

    #[gpui::test]
    fn formula_field_spinner_only_after_250ms(cx: &mut TestAppContext) {
        // Long fetch: no reply → after 250 ms the field spinner shows, then a reply hides it.
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
        tick(cx, 300);
        assert!(upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "x".into(),
                },
                window,
                cx,
            )
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
    }

    #[gpui::test]
    fn formula_field_spinner_never_flashes_on_fast_reply(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "fast".into(),
                },
                window,
                cx,
            );
        });
        tick(cx, 300);
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
    }

    // ---- Find / replace bar (`functional_spec.md §4`) -------------------------------------

    /// The find-bar's `Find` commands sent since the last drain.
    fn find_cmds(client: &RecordingClient) -> Vec<Command> {
        client
            .take_commands()
            .into_iter()
            .filter(|c| matches!(c, Command::Find { .. }))
            .collect()
    }

    #[gpui::test]
    fn cmd_f_opens_focuses_and_closes_find(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // ⌘F path → toggle_find opens the bar.
        upd(&h, cx, |c, window, cx| c.toggle_find(window, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.find_is_open()));
        // Type retained across close/reopen (`functional_spec.md §4.2`).
        upd(&h, cx, |c, window, cx| {
            c.test_find_type("hello", window, cx)
        });
        // Close returns focus to the grid and keeps the text.
        upd(&h, cx, |c, window, cx| c.toggle_find(window, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.find_is_open()));
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
        upd(&h, cx, |c, window, cx| c.toggle_find(window, cx));
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.find_field_text(cx)),
            "hello",
            "find text is retained for the next open"
        );
    }

    #[gpui::test]
    fn open_find_selects_existing_text(cx: &mut TestAppContext) {
        // §4.2: opening with retained text selects it. `open_find` schedules a `SelectAll` dispatch to
        // the focused field on the next paint (the field must be in the rendered dispatch tree first).
        // The unit-test harness does not auto-draw on notify, so `on_next_frame` can't fire here (it
        // does in the real event loop); this instead drives the exact same dispatch `open_find` uses
        // — `SelectAll` on the focused field — and asserts the whole value is selected, verifying the
        // mechanism the on-open scheduling relies on.
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("foo", window, cx); // caret ends at 3..3
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked(); // paint the open bar so the focused field is in the dispatch tree
        vcx.dispatch_action(gpui_component::input::SelectAll);
        let range = h
            .window
            .update(&mut vcx, |_root, _window, cx| {
                h.chrome.read(cx).find_selection(cx)
            })
            .unwrap();
        assert_eq!(
            range,
            0..3,
            "SelectAll on the focused find field selects the whole value"
        );
    }

    #[gpui::test]
    fn typing_in_find_sends_find_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.toggle_find(window, cx));
        h.client.take_commands(); // drop the open-time (empty-query) no-op
        upd(&h, cx, |c, window, cx| c.test_find_type("abc", window, cx));
        let cmds = find_cmds(&h.client);
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::Find { query, match_case: false, whole_cell: false, .. }] if query == "abc"
            ),
            "got {cmds:?}"
        );
    }

    #[gpui::test]
    fn find_results_set_counter_and_reveal_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("x", window, cx);
        });
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(2, 1)],
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.matches.len()), 2);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(0));
        // The first match (row-major, at/after A1) is selected + revealed.
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::SelectAndReveal(c) if *c == cell(0, 0))));
    }

    #[gpui::test]
    fn next_prev_wrap_around(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("x", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(1, 0)],
                },
                window,
                cx,
            );
        });
        // idx starts at 0; next → 1; next wraps → 0; prev wraps → 1.
        upd(&h, cx, |c, window, cx| c.next_match(window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(1));
        upd(&h, cx, |c, window, cx| c.next_match(window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(0));
        upd(&h, cx, |c, window, cx| c.prev_match(window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(1));
    }

    #[gpui::test]
    fn find_navigate_while_editing_commits_pending_edit(cx: &mut TestAppContext) {
        // Rapid-edit crash regression (`components/grid.md`): navigating find matches while the
        // formula bar is mid-edit must commit the pending edit first (before emitting the deferred
        // `SelectAndReveal`), so its consumer never drives `on_selection_changed` while Editing.
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.toggle_find(window, cx);
            c.test_find_type("x", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(1, 0)],
                },
                window,
                cx,
            );
        });
        // Start a pending edit AFTER results exist, then navigate to the next match.
        upd(&h, cx, |c, window, cx| c.test_type("=1+1", window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| c.next_match(window, cx));
        // The edit committed (not lost) and the field left Editing before the reveal was emitted.
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        let cmds = h.client.take_commands();
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Command::SetCellInput { cell: cc, input, .. } if *cc == cell(0, 0) && input == "=1+1"
            )),
            "find-navigate must commit the pending edit, got {cmds:?}"
        );
        assert!(
            h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::SelectAndReveal(_))),
            "a committed edit still navigates to the match"
        );
    }

    #[gpui::test]
    fn enter_and_shift_enter_step_matches(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("x", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(1, 0)],
                },
                window,
                cx,
            );
        });
        upd(&h, cx, |c, window, cx| {
            c.test_find_press_enter(false, window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(1));
        upd(&h, cx, |c, window, cx| {
            c.test_find_press_enter(true, window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), Some(0));
    }

    #[gpui::test]
    fn toggles_flip_and_resend_find(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("q", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| c.toggle_match_case(window, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.match_case));
        let cmds = find_cmds(&h.client);
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::Find {
                    match_case: true,
                    ..
                }]
            ),
            "toggling case re-sends Find with the new flag, got {cmds:?}"
        );
        upd(&h, cx, |c, window, cx| c.toggle_whole_cell(window, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.whole_cell));
        let cmds = find_cmds(&h.client);
        assert!(matches!(
            cmds.as_slice(),
            [Command::Find {
                whole_cell: true,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn replace_current_sends_replace_one(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("foo", window, cx);
            c.test_replace_type("bar", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(3, 2)],
                },
                window,
                cx,
            );
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| c.replace_current(window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::ReplaceOne { cell: cc, query, replacement, .. }]
                    if *cc == cell(3, 2) && query == "foo" && replacement == "bar"
            ),
            "got {cmds:?}"
        );
    }

    #[gpui::test]
    fn replace_all_sends_command_and_shows_count(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("foo", window, cx);
            c.test_replace_type("bar", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(1, 0)],
                },
                window,
                cx,
            );
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| c.replace_all(window, cx));
        assert!(
            h.client
                .take_commands()
                .iter()
                .any(|c| matches!(c, Command::ReplaceAll { query, replacement, .. } if query == "foo" && replacement == "bar")),
            "Replace All sends a ReplaceAll command"
        );
        // The reply shows "Replaced N".
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::ReplacedCount { n: 5 }, window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.replaced_notice), Some(5));
    }

    #[gpui::test]
    fn no_matches_yields_no_current_match(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("zzz", window, cx);
            c.on_worker_event(WorkerEvent::FindResults { matches: vec![] }, window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), None);
        // Replace / next / prev are inert with no matches.
        upd(&h, cx, |c, window, cx| c.replace_current(window, cx));
        upd(&h, cx, |c, window, cx| c.next_match(window, cx));
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|c| matches!(c, Command::ReplaceOne { .. })));
    }

    #[gpui::test]
    fn sheet_switch_rescopes_open_find(cx: &mut TestAppContext) {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Sheet2"),
            ],
            SheetId(0),
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_find(window, cx);
            c.test_find_type("x", window, cx);
            c.on_worker_event(
                WorkerEvent::FindResults {
                    matches: vec![cell(0, 0), cell(1, 0)],
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.matches.len()), 2);
        h.client.take_commands();
        // Switch sheets — the open bar re-scopes: cursor resets + a fresh Find for the new sheet.
        upd(&h, cx, |c, window, cx| {
            c.select_sheet(SheetId(1), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.match_idx), None);
        let cmds = find_cmds(&h.client);
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::Find {
                    sheet: SheetId(1),
                    ..
                }]
            ),
            "re-scopes Find to the new sheet, got {cmds:?}"
        );
    }

    // ---- Sheet tab bar --------------------------------------------------------------------

    fn two_sheets(cx: &mut TestAppContext) -> Harness {
        build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Sheet2"),
            ],
            SheetId(0),
        )
    }

    #[gpui::test]
    fn add_sheet_sends_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, _cx| c.add_sheet());
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::AddSheet]
        ));
    }

    #[gpui::test]
    fn select_sheet_switches_and_notifies_grid(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.select_sheet(SheetId(1), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.active_sheet()), SheetId(1));
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::SetActiveSheet(SheetId(1)))));
    }

    #[gpui::test]
    fn select_sheet_while_editing_commits_pending_edit(cx: &mut TestAppContext) {
        // Rapid-edit crash regression (`components/grid.md`): switching sheets while the formula bar
        // is mid-edit must commit the pending edit to the CURRENT sheet's cell first — not leave the
        // field Editing (which the deferred `switch_grid_to_sheet` would drive into
        // `on_selection_changed` while Editing, panicking / silently discarding the edit).
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=1+1", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_sheet(SheetId(1), window, cx)
        });
        // The pending edit committed (not lost), and the field is no longer Editing.
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.active_sheet()), SheetId(1));
        let cmds = h.client.take_commands();
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Command::SetCellInput { sheet, cell: cc, input }
                    if *sheet == SheetId(0) && *cc == cell(0, 0) && input == "=1+1"
            )),
            "the edit must commit to the source sheet's cell before the switch, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn select_sheet_blocked_by_cap_rejected_edit(cx: &mut TestAppContext) {
        // A cap-rejected edit blocks the sheet switch: stay on the current sheet, keep editing.
        let h = two_sheets(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type(&huge, window, cx);
        });
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.select_sheet(SheetId(1), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.active_sheet()), SheetId(0));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert!(
            !h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::SetActiveSheet(_))),
            "a cap-rejected edit must not switch sheets"
        );
    }

    #[gpui::test]
    fn rename_valid_sends_command(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("Revenue", window, cx);
            c.commit_rename(window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::RenameSheet { sheet: SheetId(0), name }] if name == "Revenue"),
            "got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.rename_target()), None);
    }

    #[gpui::test]
    fn rename_invalid_stays_editing(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("Sheet2", window, cx); // duplicate (case-insensitive)
            c.commit_rename(window, cx);
        });
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::RenameSheet { .. })));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.rename_target()),
            Some(SheetId(0))
        );
        assert!(upd(&h, cx, |c, _w, _cx| c.rename_error()));
    }

    #[gpui::test]
    fn rename_escape_reverts(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("whatever", window, cx);
            c.cancel_rename(window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.rename_target()), None);
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::RenameSheet { .. })));
    }

    #[gpui::test]
    fn delete_last_sheet_disabled(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.delete_enabled()));
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(0), cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
        assert!(h.client.take_commands().is_empty());
    }

    #[gpui::test]
    fn delete_empty_sheet_no_confirm(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(1), cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::DeleteSheet { sheet: SheetId(1) }]
        ));
    }

    #[gpui::test]
    fn delete_with_content_confirms_then_deletes(cx: &mut TestAppContext) {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Data").with_content(true),
            ],
            SheetId(0),
        );
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(1), cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()),
            Some(SheetId(1))
        );
        assert!(
            h.client.take_commands().is_empty(),
            "no delete before confirm"
        );
        upd(&h, cx, |c, _w, cx| c.confirm_delete(cx));
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::DeleteSheet { sheet: SheetId(1) }]
        ));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
    }

    #[gpui::test]
    fn sheets_changed_event_updates_tabs(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SheetsChanged {
                    sheets: vec![
                        SheetMeta {
                            id: SheetId(0),
                            name: "Sheet1".into(),
                            has_content: false,
                        },
                        SheetMeta {
                            id: SheetId(7),
                            name: "Sheet2".into(),
                            has_content: false,
                        },
                    ],
                },
                window,
                cx,
            )
        });
        let names: Vec<String> = upd(&h, cx, |c, _w, _cx| {
            c.sheets().iter().map(|t| t.name.clone()).collect()
        });
        assert_eq!(names, vec!["Sheet1".to_string(), "Sheet2".to_string()]);
    }

    // ---- Sheet-tab reorder drag (Phase 6b, `functional_spec.md §6`) ------------------------

    /// Three tabs at slots 0/1/2 with 60 px-wide spans pre-loaded (centers 30/90/150), so the pure
    /// insertion geometry can be exercised without a paint pass — the unit harness does not paint,
    /// so the per-tab `canvas` span probes never run in-test.
    fn three_sheets_with_spans(cx: &mut TestAppContext) -> Harness {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "S0"),
                SheetTab::new(SheetId(1), "S1"),
                SheetTab::new(SheetId(2), "S2"),
            ],
            SheetId(0),
        );
        upd(&h, cx, |c, _w, _cx| {
            c.tab_spans = vec![
                TabSpan {
                    sheet: SheetId(0),
                    left: 0.0,
                    right: 60.0,
                },
                TabSpan {
                    sheet: SheetId(1),
                    left: 60.0,
                    right: 120.0,
                },
                TabSpan {
                    sheet: SheetId(2),
                    left: 120.0,
                    right: 180.0,
                },
            ];
        });
        h.client.take_commands(); // drain any setup commands so tests assert only the drag's
        h
    }

    #[test]
    fn tab_insertion_index_maps_cursor_to_gap() {
        let centers = [30.0, 90.0, 150.0];
        assert_eq!(
            tab_insertion_index(10.0, &centers),
            0,
            "before every tab → gap 0"
        );
        assert_eq!(
            tab_insertion_index(30.0, &centers),
            1,
            "at a center counts it"
        );
        assert_eq!(
            tab_insertion_index(60.0, &centers),
            1,
            "between slot 0 and 1 → gap 1"
        );
        assert_eq!(tab_insertion_index(100.0, &centers), 2);
        assert_eq!(
            tab_insertion_index(200.0, &centers),
            3,
            "after every tab → gap n"
        );
    }

    #[test]
    fn move_target_for_gap_handles_noop_and_shift() {
        // Dragging slot 0: the two gaps bracketing it are no-ops; further gaps shift left by one.
        assert_eq!(move_target_for_gap(0, 0), None);
        assert_eq!(move_target_for_gap(1, 0), None);
        assert_eq!(move_target_for_gap(2, 0), Some(1));
        assert_eq!(move_target_for_gap(3, 0), Some(2));
        // Dragging slot 2 leftward.
        assert_eq!(move_target_for_gap(0, 2), Some(0));
        assert_eq!(move_target_for_gap(1, 2), Some(1));
        assert_eq!(move_target_for_gap(2, 2), None);
        assert_eq!(move_target_for_gap(3, 2), None);
    }

    #[gpui::test]
    fn tab_drag_reorders_sends_move(cx: &mut TestAppContext) {
        let h = three_sheets_with_spans(cx);
        upd(&h, cx, |c, _w, cx| {
            c.tab_press(SheetId(0), 30.0);
            // Past the threshold, into the left half of slot 2 (cursor 140 < its center 150), so the
            // drop inserts BEFORE slot 2 → gap 2 → final index 1 (removing S0 shifts the gap left).
            c.tab_drag_move(140.0, cx);
            c.tab_drag_end(140.0, cx);
        });
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::MoveSheet {
                    sheet: SheetId(0),
                    to_index: 1
                }]
            ),
            "a real drop before slot 2 moves S0 to final index 1"
        );
    }

    #[gpui::test]
    fn tab_drag_below_threshold_is_no_command(cx: &mut TestAppContext) {
        let h = three_sheets_with_spans(cx);
        upd(&h, cx, |c, _w, cx| {
            c.tab_press(SheetId(0), 30.0);
            c.tab_drag_move(32.0, cx); // 2 px < threshold → still a click
            c.tab_drag_end(32.0, cx);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a sub-threshold press stays a click, sends no MoveSheet"
        );
    }

    #[gpui::test]
    fn tab_drag_to_origin_sends_nothing(cx: &mut TestAppContext) {
        let h = three_sheets_with_spans(cx);
        upd(&h, cx, |c, _w, cx| {
            c.tab_press(SheetId(0), 30.0);
            c.tab_drag_move(36.0, cx); // crosses the threshold but stays over the origin tab
            c.tab_drag_end(36.0, cx);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "dropping back on the origin slot is a no-op"
        );
    }

    #[gpui::test]
    fn tab_drag_sets_indicator(cx: &mut TestAppContext) {
        let h = three_sheets_with_spans(cx);
        let (active, indicator) = upd(&h, cx, |c, _w, cx| {
            c.tab_press(SheetId(0), 30.0);
            c.tab_drag_move(140.0, cx);
            (c.tab_drag_active(), c.tab_drop_indicator_x())
        });
        assert!(active, "past the threshold the drag is active");
        assert_eq!(
            indicator,
            Some(120.0),
            "the indicator snaps to the gap between slots 1 and 2"
        );
    }

    #[gpui::test]
    fn tab_move_target_skips_without_geometry(cx: &mut TestAppContext) {
        let h = three_sheets_with_spans(cx);
        upd(&h, cx, |c, _w, _cx| c.tab_spans.clear());
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.tab_move_target(SheetId(0), 140.0)),
            None,
            "an unmeasured tab strip never guesses a reorder"
        );
    }

    #[gpui::test]
    fn worker_input_cap_reject_flags_error(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::EditRejected {
                    reason: freecell_engine::EditRejectedReason::InputCap(
                        freecell_core::input_cap::InputRejection::TooLong {
                            len: 9000,
                            max: MAX_INPUT_LEN,
                        },
                    ),
                },
                window,
                cx,
            );
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        // The worker backstop carries the rejection, so the popover message matches too.
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cap_error_message()),
            Some("Formula too long (max 8,192 characters)".to_string())
        );
        // The next keystroke dismisses the backstop popover (`functional_spec.md §4.2`).
        upd(&h, cx, |c, window, cx| c.test_type("=1", window, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.cap_error_message()), None);
    }

    // ---- Editing feel: type-to-replace, in-cell editor, sync, Tab, mirror ----------------

    /// The most recent edit-state push the chrome sent to the grid (mirror / in-cell / cap).
    type EditStatePush = (
        Option<(SheetId, CellRef, gpui::SharedString)>,
        Option<CellRef>,
        Option<gpui::SharedString>,
    );
    fn last_edit_state(reqs: &[ChromeGridRequest]) -> Option<EditStatePush> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState {
                mirror,
                in_cell,
                cap,
                ..
            } => Some((mirror.clone(), *in_cell, cap.clone())),
            _ => None,
        })
    }

    /// The `quick_edit` flag on the most recent edit-state push (`functional_spec.md §5`).
    fn last_edit_state_quick(reqs: &[ChromeGridRequest]) -> Option<bool> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState { quick_edit, .. } => Some(*quick_edit),
            _ => None,
        })
    }

    /// A chrome whose active cell A1 has fetched `content` (single selection, reply applied).
    fn idle_on_a1(cx: &mut TestAppContext, content: &str) -> Harness {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: content.into(),
                },
                window,
                cx,
            );
        });
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
        h
    }

    #[gpui::test]
    fn type_to_replace_starts_edit_with_char(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "old");
        upd(&h, cx, |c, window, cx| c.begin_typed("x", window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::DataRow
        );
        // A live mirror of the typed char was pushed to the grid for the active cell.
        let mirror = last_edit_state(&h.grid_requests.borrow())
            .and_then(|(m, _, _)| m)
            .expect("mirror pushed while editing");
        assert_eq!(mirror.1, cell(0, 0));
        assert_eq!(mirror.2.as_ref(), "x");
    }

    #[gpui::test]
    fn type_to_replace_on_multiselect_targets_active(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(1, 1),
                    active: cell(3, 3),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Disabled);
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("5", window, cx);
            c.test_press_enter(false, window, cx);
        });
        // The commit targets the active cell of the (multi) selection.
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { cell: cc, input, .. }) if *cc == cell(3, 3) && input == "5"),
            "expected SetCellInput at D4 with \"5\", got {cmds:?}"
        );
    }

    #[gpui::test]
    fn f2_opens_in_cell_keeping_content(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "42");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::InCell
        );
        // The grid got the in-cell overlay open on A1.
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, ic, _)| ic),
            Some(cell(0, 0))
        );
    }

    #[gpui::test]
    fn begin_in_cell_focuses_the_in_cell_input(cx: &mut TestAppContext) {
        // BUG D (seam-level): opening the in-cell editor must focus its input so it shows a caret
        // and accepts typing. The grid-side focus-transfer *race* — where the grid re-steals focus
        // after `begin_in_cell` focuses the input — needs a real grid and is covered by the grid
        // harness test `double_click_keeps_focus_on_in_cell_input`.
        let h = idle_on_a1(cx, "42");
        let focused = upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.edit
                .in_cell()
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(focused, "the in-cell input must be focused on open");
    }

    #[gpui::test]
    fn in_cell_and_data_row_stay_in_sync(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            // Typing in the in-cell editor updates the data row.
            c.test_incell_type("=A1", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=A1");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "=A1");
        // Typing in the data row updates the in-cell editor (both directions, no echo loop).
        upd(&h, cx, |c, window, cx| c.test_type("=B2", window, cx));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "=B2");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=B2");
    }

    #[gpui::test]
    fn in_cell_enter_commits_and_moves_down(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("99", window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "99"),
            "expected SetCellInput \"99\", got {cmds:?}"
        );
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Down))
        )));
        // The overlay closed on commit.
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, ic, _)| ic),
            None
        );
    }

    #[gpui::test]
    fn in_cell_tab_commits_and_moves_right(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("7", window, cx);
            c.commit_incell_move(Direction::Right, window, cx);
        });
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
    }

    #[gpui::test]
    fn in_cell_escape_cancels_and_reverts(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "42");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("999", window, cx);
            c.cancel_incell(window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn in_cell_cap_reject_keeps_editing_and_flags(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type(&huge, window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        // No commit, still editing, overlay still open.
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::SetCellInput { .. })));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        // The cap message is pushed for the in-cell popover (origin == InCell).
        let cap = last_edit_state(&h.grid_requests.borrow()).and_then(|(_, _, cap)| cap);
        assert_eq!(
            cap.as_deref(),
            Some("Formula too long (max 8,192 characters)")
        );
    }

    #[gpui::test]
    fn begin_in_cell_mid_edit_keeps_pending_text(cx: &mut TestAppContext) {
        // Type-to-replace in the data row, then F2 → the in-cell editor keeps the pending text.
        let h = idle_on_a1(cx, "old");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("x", window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "x");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::InCell
        );
    }

    #[gpui::test]
    fn data_row_tab_commits_and_moves_right(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
            c.test_data_row_tab(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=1"));
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
    }

    #[gpui::test]
    fn data_row_shift_tab_moves_left(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
            c.test_data_row_tab(true, window, cx);
        });
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Left))
        )));
    }

    // ---- Quick-edit mode (`functional_spec.md §5`) ----------------------------------------

    /// No modifiers held (a plain keystroke).
    fn plain() -> Modifiers {
        Modifiers::default()
    }

    #[gpui::test]
    fn quick_edit_arrow_commits_and_moves(cx: &mut TestAppContext) {
        // Type-to-replace enters quick-edit; an unmodified arrow commits + moves the active cell.
        let h = idle_on_a1(cx, "");
        let consumed = upd(&h, cx, |c, window, cx| {
            c.begin_typed("abcd", window, cx);
            c.handle_data_row_edit_key("right", plain(), window, cx)
        });
        assert!(
            consumed,
            "an unmodified arrow in quick-edit must be consumed (commit + move)"
        );
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "abcd"),
            "expected SetCellInput \"abcd\", got {cmds:?}"
        );
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
        // The edit ended — back to normal navigation.
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
    }

    #[gpui::test]
    fn quick_edit_arrows_move_each_direction(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        for (key, dir) in [
            ("left", Direction::Left),
            ("right", Direction::Right),
            ("up", Direction::Up),
            ("down", Direction::Down),
        ] {
            h.client.take_commands();
            h.grid_requests.borrow_mut().clear();
            let consumed = upd(&h, cx, |c, window, cx| {
                c.begin_typed("v", window, cx);
                c.handle_data_row_edit_key(key, plain(), window, cx)
            });
            assert!(consumed, "arrow {key} must be consumed in quick-edit");
            assert!(
                h.grid_requests.borrow().iter().any(|r| matches!(
                    r,
                    ChromeGridRequest::MoveActive(Motion::Move(d)) if *d == dir
                )),
                "arrow {key} must move the active cell {dir:?}"
            );
        }
    }

    /// Enters quick-edit by focusing the data-row input and typing `text` (the sole quick-edit
    /// entry, `begin_typed`), then asserts the input actually holds focus — otherwise a
    /// subsequent keystroke would not route to it and the reproduction would be vacuous.
    fn enter_quick_edit_focused(h: &Harness, vcx: &mut gpui::VisualTestContext, text: &str) {
        vcx.update(|window, cx| {
            h.chrome.update(cx, |c, cx| c.begin_typed(text, window, cx));
        });
        vcx.run_until_parked();
        let focused = vcx.update(|window, cx| {
            h.chrome
                .read(cx)
                .content_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(focused, "quick-edit must focus the data-row input");
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
    }

    #[gpui::test]
    fn quick_edit_real_keystroke_arrows_commit_and_move(cx: &mut TestAppContext) {
        // Real-keystroke reproduction of the reported bug (the direct `handle_data_row_edit_key`
        // unit tests miss it): with the data-row input focused in quick-edit, an ACTUAL unmodified
        // arrow keystroke must COMMIT the typed text and MOVE the active cell — not move the text
        // caret. gpui-component's single-line `Input` binds Left/Right to caret actions that
        // dispatch *before* any key-down listener and stop propagation, so before the keystroke-
        // interceptor fix a real Left/Right moved the caret and never committed (Up/Down already
        // worked, being unbound in single-line mode). This drives real keystrokes through gpui
        // dispatch, so it fails against the pre-fix routing and passes once the interceptor preempts
        // the input's caret action.
        let h = idle_on_a1(cx, "");
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        for (key, dir) in [
            ("left", Direction::Left),
            ("right", Direction::Right),
            ("up", Direction::Up),
            ("down", Direction::Down),
        ] {
            enter_quick_edit_focused(&h, &mut vcx, "1234");
            vcx.simulate_keystrokes(key);
            vcx.run_until_parked();
            let cmds = h.client.take_commands();
            assert!(
                matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "1234"),
                "a real {key} keystroke in quick-edit must commit \"1234\", got {cmds:?}"
            );
            assert!(
                h.grid_requests.borrow().iter().any(|r| matches!(
                    r,
                    ChromeGridRequest::MoveActive(Motion::Move(d)) if *d == dir
                )),
                "a real {key} keystroke in quick-edit must move the active cell {dir:?}: {:?}",
                h.grid_requests.borrow()
            );
            assert_eq!(
                vcx.update(|_w, cx| h.chrome.read(cx).data_mode()),
                FieldMode::Idle,
                "commit via a real {key} keystroke must end the edit"
            );
        }
    }

    #[gpui::test]
    fn quick_edit_real_keystroke_left_commits_and_moves(cx: &mut TestAppContext) {
        // The primary user repro, isolated: `[focus cell] type "1234" [press Left]`. Before the fix
        // this moved the caret inside the field (the `Input`'s `MoveLeft` action won) and neither
        // committed nor moved the cell. A real Left keystroke must now commit + move left.
        let h = idle_on_a1(cx, "");
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        enter_quick_edit_focused(&h, &mut vcx, "1234");
        vcx.simulate_keystrokes("left");
        vcx.run_until_parked();
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "1234"),
            "a real Left keystroke in quick-edit must commit \"1234\", got {cmds:?}"
        );
        assert!(
            h.grid_requests.borrow().iter().any(|r| matches!(
                r,
                ChromeGridRequest::MoveActive(Motion::Move(Direction::Left))
            )),
            "a real Left keystroke in quick-edit must move the active cell left: {:?}",
            h.grid_requests.borrow()
        );
    }

    #[gpui::test]
    fn quick_edit_real_keystroke_modified_arrow_leaves_without_moving(cx: &mut TestAppContext) {
        // A real Shift+Right in quick-edit is a caret/selection op: it must leave quick-edit and
        // must NOT commit or move the active cell (the `Input`'s own shift-right selection runs).
        let h = idle_on_a1(cx, "");
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        enter_quick_edit_focused(&h, &mut vcx, "1234");
        vcx.simulate_keystrokes("shift-right");
        vcx.run_until_parked();
        assert!(
            !h.client
                .take_commands()
                .iter()
                .any(|c| matches!(c, Command::SetCellInput { .. })),
            "shift+right must not commit"
        );
        assert!(
            !h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))),
            "shift+right must not move the active cell"
        );
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false),
            "a modified arrow leaves quick-edit"
        );
        assert_eq!(
            vcx.update(|_w, cx| h.chrome.read(cx).data_mode()),
            FieldMode::Editing,
            "a modified arrow does not end the edit"
        );
    }

    #[gpui::test]
    fn quick_edit_real_keystroke_home_leaves(cx: &mut TestAppContext) {
        // A real Home in quick-edit is explicit caret positioning: leaves quick-edit, does not move
        // the active cell, and the edit stays open (the `Input` moves the caret to the start).
        let h = idle_on_a1(cx, "");
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        enter_quick_edit_focused(&h, &mut vcx, "1234");
        vcx.simulate_keystrokes("home");
        vcx.run_until_parked();
        assert!(
            !h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))),
            "home must not move the active cell"
        );
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false),
            "home leaves quick-edit"
        );
        assert_eq!(
            vcx.update(|_w, cx| h.chrome.read(cx).data_mode()),
            FieldMode::Editing
        );
    }

    #[gpui::test]
    fn quick_edit_not_entered_by_in_cell(cx: &mut TestAppContext) {
        // Double-click / F2 (in-cell) edits are NOT quick-edit: arrows control the caret.
        let h = idle_on_a1(cx, "");
        let consumed = upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("z", window, cx);
            c.handle_data_row_edit_key("right", plain(), window, cx)
        });
        assert!(
            !consumed,
            "an in-cell edit must not consume the arrow (caret op)"
        );
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::SetCellInput { .. })));
        assert!(!h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn quick_edit_caret_intent_modifier_arrow_leaves_without_moving(cx: &mut TestAppContext) {
        // Each caret-intent modifier (Shift / Ctrl / Alt / Cmd-platform) + arrow is a caret op: it
        // leaves quick-edit and does NOT move the active cell. `function` is deliberately excluded
        // (tested separately) so a plain macOS arrow — which carries `function` — still moves.
        let cases: [(&str, Modifiers); 4] = [
            (
                "shift",
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
            ),
            (
                "control",
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
            ),
            (
                "alt",
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
            ),
            (
                "platform",
                Modifiers {
                    platform: true,
                    ..Modifiers::default()
                },
            ),
        ];
        for (name, mods) in cases {
            let h = idle_on_a1(cx, "");
            let consumed = upd(&h, cx, |c, window, cx| {
                c.begin_typed("v", window, cx);
                c.handle_data_row_edit_key("right", mods, window, cx)
            });
            assert!(!consumed, "{name}+arrow must fall through to the caret");
            assert!(
                !h.client
                    .take_commands()
                    .iter()
                    .any(|cmd| matches!(cmd, Command::SetCellInput { .. })),
                "{name}+arrow must not commit"
            );
            assert!(
                !h.grid_requests
                    .borrow()
                    .iter()
                    .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))),
                "{name}+arrow must not move the active cell"
            );
            assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
            // Quick-edit is now off: even a subsequent unmodified arrow does not move.
            h.grid_requests.borrow_mut().clear();
            let consumed2 = upd(&h, cx, |c, window, cx| {
                c.handle_data_row_edit_key("right", plain(), window, cx)
            });
            assert!(
                !consumed2,
                "after {name}+arrow, arrows are caret ops for the rest of the edit"
            );
            assert!(!h
                .grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))));
        }
    }

    #[gpui::test]
    fn quick_edit_plain_arrow_with_function_flag_still_moves(cx: &mut TestAppContext) {
        // Cross-platform regression: macOS sets `Modifiers::function` on a *plain* arrow keystroke.
        // The caret-intent predicate excludes `function`, so §5.2's commit + move must still fire —
        // otherwise quick-edit's core feature never works on macOS.
        let h = idle_on_a1(cx, "");
        let fn_only = Modifiers {
            function: true,
            ..Modifiers::default()
        };
        let consumed = upd(&h, cx, |c, window, cx| {
            c.begin_typed("abcd", window, cx);
            c.handle_data_row_edit_key("right", fn_only, window, cx)
        });
        assert!(
            consumed,
            "a plain arrow carrying only the function flag (macOS) must still commit + move"
        );
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "abcd"),
            "expected SetCellInput \"abcd\", got {cmds:?}"
        );
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
    }

    #[gpui::test]
    fn quick_edit_home_leaves(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        let consumed = upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.handle_data_row_edit_key("home", plain(), window, cx)
        });
        assert!(!consumed, "Home is caret positioning — not consumed");
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false),
            "Home leaves quick-edit"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn quick_edit_mouse_down_in_field_leaves(cx: &mut TestAppContext) {
        // The data-row field's on_mouse_down calls leave_quick_edit (placing the caret by click).
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.leave_quick_edit(window, cx);
        });
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false)
        );
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
        let consumed = upd(&h, cx, |c, window, cx| {
            c.handle_data_row_edit_key("right", plain(), window, cx)
        });
        assert!(
            !consumed,
            "after a click into the field, arrows are caret ops"
        );
        assert!(!h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))));
    }

    #[gpui::test]
    fn quick_edit_flag_pushed_to_grid_and_cleared(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        // Type-to-replace pushes quick_edit = true.
        upd(&h, cx, |c, window, cx| c.begin_typed("v", window, cx));
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(true),
            "type-to-replace pushes quick_edit=true to the grid"
        );
        // Opening the in-cell editor pushes quick_edit = false.
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx)
        });
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false),
            "the in-cell editor is never quick-edit"
        );
    }

    #[gpui::test]
    fn quick_edit_cleared_in_grid_push_after_commit(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.handle_data_row_edit_key("down", plain(), window, cx);
        });
        // The commit clears the mirror and quick_edit for the grid.
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false)
        );
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(m, _, _)| m),
            None
        );
    }

    #[gpui::test]
    fn quick_edit_preserves_tab_and_enter(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        // Tab still commits + moves right in quick-edit.
        let consumed = upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.handle_data_row_edit_key("tab", plain(), window, cx)
        });
        assert!(consumed);
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
        // Enter still commits + moves down.
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.test_press_enter(false, window, cx);
        });
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Down))
        )));
    }

    #[gpui::test]
    fn quick_edit_escape_resets_flag(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "42");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("v", window, cx);
            c.escape_edit(window, cx);
        });
        // Escape ends the edit; the grid's quick_edit copy is cleared.
        assert_eq!(
            last_edit_state_quick(&h.grid_requests.borrow()),
            Some(false)
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
    }

    #[gpui::test]
    fn mirror_cleared_on_commit(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
        });
        // Mirror present while editing.
        assert!(last_edit_state(&h.grid_requests.borrow())
            .and_then(|(m, _, _)| m)
            .is_some());
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.test_press_enter(false, window, cx)
        });
        // Cleared on commit.
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(m, _, _)| m),
            None
        );
    }

    #[gpui::test]
    fn double_click_reselect_keeps_content(cx: &mut TestAppContext) {
        // Replays the real double-click chrome-level order: the second mousedown re-emits
        // SelectionChanged for the already-selected cell (restarting the fetch + clearing the
        // field) BEFORE OpenInCellEditor. The in-cell editor must still show the cell's real
        // content ("42"), not the just-cleared field (review Critical #1 — data-loss guard).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            // Redundant re-select (the grid also elides this now, but the chrome must be robust).
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn begin_in_cell_ignored_while_other_cell_editing(cx: &mut TestAppContext) {
        // A cap-rejected/deferred-revert click-away leaves the reducer + selection on the OLD cell;
        // opening the in-cell editor on a DIFFERENT cell must no-op (review Moderate #2).
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("x", window, cx); // editing A1 (the active cell)
            c.begin_in_cell(cell(1, 1), window, cx); // a divergent cell
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.incell_open()),
            None,
            "overlay must not relocate onto a cell the edit isn't on"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn in_cell_opens_empty_while_fetch_pending_then_populates(cx: &mut TestAppContext) {
        // F2 before the content reply arrives: the overlay opens empty (no forced empty edit), and
        // the in-flight reply promotes it once it lands (empty-with-spinner intent, review #3).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx); // reply not yet delivered
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.data_mode()),
            FieldMode::Idle,
            "no empty edit forced while the fetch is still in flight"
        );
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "hello".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "hello");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "hello");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
    }

    #[gpui::test]
    fn double_click_cross_cell_pending_fetch_opens_empty(cx: &mut TestAppContext) {
        // Select non-empty A1 (reply lands), then B2 whose fetch is still in flight, then open the
        // in-cell editor on B2. It must NOT seed A1's stale committed "42" (the reducer keeps A1's
        // `committed` across the single→single switch) — it opens empty, and B2's reply populates
        // it (review New Critical — cross-cell data-corruption guard).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx); // B2, no reply
            c.begin_in_cell(cell(1, 1), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed the previous cell's stale content"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(1, 1)));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "world".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "world");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "world");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn multiselect_collapse_open_does_not_seed_stale(cx: &mut TestAppContext) {
        // A1 reply "42" tags the seed. A range multi-select clears `committed` and resets the tag.
        // Collapsing back to A1 (fetch in flight) and opening the in-cell editor must NOT seed the
        // just-cleared empty content — it opens empty, and A1's reply repopulates it (New Critical
        // path #1: multi-select clears committed but the bare tag used to survive).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            // A range selection → multi → the reducer clears `committed`.
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(2, 2),
                },
                window,
                cx,
            );
            // Collapse back to A1 → a fresh fetch (req 2) is in flight, `committed` still "".
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed the just-cleared committed content"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "42".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn sheet_switch_open_does_not_seed_other_sheet(cx: &mut TestAppContext) {
        // Sheet1!A1 reply lands (tag = (Sheet1, A1)). Switch to Sheet2 and open the in-cell editor
        // on Sheet2!A1 (the default landing cell, same CellRef) before its fetch replies — it must
        // NOT seed Sheet1's content across sheets (New Critical path #2: the bare tag ignored the
        // sheet). Opens empty; Sheet2's reply promotes with the right content.
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "sheet1A1".into(),
                },
                window,
                cx,
            );
            // Switch to Sheet2 (window-driven adopt), then select its A1 (fetch req 2 in flight).
            c.adopt_active_sheet(SheetId(1), cx);
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed another sheet's content"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "sheet2A1".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "sheet2A1");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn commit_retags_so_reopen_other_cell_does_not_seed_committed(cx: &mut TestAppContext) {
        // The commit paths overwrite the reducer's `committed` with the EDITED cell's content; the
        // seed tag must move with it (New Critical — commit-path stale seed). Repro: land A1="Zval",
        // type-to-replace B1="x", click-away commit of B1, then reopen A1 before its re-fetch reply.
        // The A1 editor must NOT show B1's "x"; it opens empty and A1's reply repopulates "Zval".
        let h = one_sheet(cx);
        let b1 = cell(0, 1);
        upd(&h, cx, |c, window, cx| {
            // 1. A1 reply lands → tag = (s, A1).
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "Zval".into(),
                },
                window,
                cx,
            );
            // 2. Move to B1, type-to-replace "x"; B1's reply arrives mid-edit and is dropped.
            c.on_selection_changed(SelectionModel::single(b1), window, cx);
            c.begin_typed("x", window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "Bval".into(),
                },
                window,
                cx,
            );
        });
        // 3. Click-away commit of B1 (the tag must move to B1 here — selection.active is still B1).
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.on_edit_commit_requested(window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { cell: cc, input, .. }] if *cc == b1 && input == "x"),
            "B1 must receive the committed \"x\", got {cmds:?}"
        );
        // Select A1 (its re-fetch req 3 is in flight), then reopen the in-cell editor on A1.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_ne!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "x",
            "A1 must not seed B1's just-committed content"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        // 4. A1's real reply (req 3) promotes the overlay with A1's content.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 3,
                    raw: "Zval".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "Zval");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn focus_flip_clears_incell_cap_push(cx: &mut TestAppContext) {
        // After an in-cell cap reject (grid shows the popover), flipping focus to the data row must
        // clear the in-cell cap push so only one popover shows (review Mild #4).
        let h = idle_on_a1(cx, "");
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type(&huge, window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        assert!(last_edit_state(&h.grid_requests.borrow())
            .and_then(|(_, _, cap)| cap)
            .is_some());
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            let handle = c.content_input.clone();
            c.on_content_event(&handle, &InputEvent::Focus, window, cx);
        });
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, _, cap)| cap),
            None,
            "the in-cell cap popover clears when focus flips to the data row"
        );
    }

    // ---- Function autocomplete + signature hints (`gaps_closing_7_15 §1`) ------------------

    /// The autocomplete display on the most recent in-cell edit-state push.
    fn last_edit_state_autocomplete(reqs: &[ChromeGridRequest]) -> Option<AutocompleteDisplay> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState { autocomplete, .. } => Some(autocomplete.clone()),
            _ => None,
        })?
    }

    #[gpui::test]
    fn typing_prefix_opens_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=su", window, cx));
        upd(&h, cx, |c, _w, _cx| {
            let ac = c.autocomplete.as_ref().expect("list open on =su");
            assert!(ac.matches.len() >= 3, "several SU* matches");
            assert!(ac.matches.iter().all(|f| f.name.starts_with("SU")));
            assert!(ac.matches.iter().any(|f| f.name == "SUM"), "SUM present");
            assert_eq!(ac.highlight, 0, "top row highlighted");
            // The leading block is the common (rank-0) set, alphabetical.
            assert_eq!(ac.matches[0].rank, 0);
        });
    }

    #[gpui::test]
    fn non_formula_never_triggers(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("su", window, cx));
        upd(&h, cx, |c, _w, _cx| {
            assert!(c.autocomplete.is_none(), "no `=` → no list");
        });
    }

    #[gpui::test]
    fn nav_moves_highlight_clamped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=su", window, cx));
        upd(&h, cx, |c, _w, cx| {
            c.autocomplete_nav(false, cx);
            assert_eq!(
                c.autocomplete.as_ref().unwrap().highlight,
                0,
                "clamp at top"
            );
            c.autocomplete_nav(true, cx);
            assert_eq!(c.autocomplete.as_ref().unwrap().highlight, 1);
            c.autocomplete_nav(false, cx);
            assert_eq!(c.autocomplete.as_ref().unwrap().highlight, 0);
        });
    }

    #[gpui::test]
    fn accept_inserts_name_paren_and_places_caret(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // "=sum" → SUM is the exact match, highlighted first.
        upd(&h, cx, |c, window, cx| c.begin_typed("=sum", window, cx));
        upd(&h, cx, |c, window, cx| {
            assert_eq!(
                c.autocomplete.as_ref().unwrap().matches[0].name,
                "SUM",
                "exact SUM highlighted"
            );
            c.autocomplete_accept(window, cx);
        });
        upd(&h, cx, |c, _w, cx| {
            assert!(c.autocomplete.is_none(), "list closes on accept");
            assert_eq!(c.content_input.read(cx).value().to_string(), "=SUM(");
            assert_eq!(c.content_input.read(cx).cursor(), 5, "caret just after `(`");
            assert_eq!(c.sig_hint, Some("SUM(number1, [number2], …)"));
            assert_eq!(c.data_mode(), FieldMode::Editing, "edit continues");
        });
    }

    #[gpui::test]
    fn accept_mid_formula_keeps_suffix(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=1+sum", window, cx));
        // Move the caret back before the trailing (none here) — caret is at end (6).
        upd(&h, cx, |c, window, cx| c.autocomplete_accept(window, cx));
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(c.content_input.read(cx).value().to_string(), "=1+SUM(");
            assert_eq!(c.content_input.read(cx).cursor(), 7);
        });
    }

    #[gpui::test]
    fn esc_closes_list_keeps_edit(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=su", window, cx));
        upd(&h, cx, |c, window, cx| c.autocomplete_dismiss(window, cx));
        upd(&h, cx, |c, _w, cx| {
            assert!(c.autocomplete.is_none(), "list dismissed");
            assert_eq!(c.data_mode(), FieldMode::Editing, "edit continues");
            assert_eq!(
                c.content_input.read(cx).value().to_string(),
                "=su",
                "text unchanged"
            );
        });
    }

    #[gpui::test]
    fn sig_hint_shows_when_caret_inside_call(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=SUM(", window, cx));
        upd(&h, cx, |c, _w, _cx| {
            assert!(c.autocomplete.is_none(), "no name token after `(`");
            assert_eq!(c.sig_hint, Some("SUM(number1, [number2], …)"));
        });
    }

    #[gpui::test]
    fn in_cell_path_opens_and_accepts(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // Open the in-cell overlay over the active cell, then type a formula prefix into it.
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx)
        });
        upd(&h, cx, |c, window, cx| {
            c.edit.in_cell().update(cx, |i, cx| {
                i.set_value("=sum", window, cx);
                i.set_cursor_position(Position::new(0, 4), window, cx);
            });
            c.recompute_autocomplete(cx);
            c.refresh_edit_grid_state(window, cx);
        });
        upd(&h, cx, |c, _w, _cx| {
            assert!(c.autocomplete.is_some(), "in-cell list open");
        });
        // The in-cell list is pushed to the grid for rendering.
        assert!(
            last_edit_state_autocomplete(&h.grid_requests.borrow()).is_some(),
            "in-cell autocomplete pushed to grid"
        );
        upd(&h, cx, |c, window, cx| c.autocomplete_accept(window, cx));
        upd(&h, cx, |c, _w, cx| {
            assert!(c.autocomplete.is_none());
            assert_eq!(c.edit.in_cell().read(cx).value().to_string(), "=SUM(");
            assert_eq!(c.edit.in_cell().read(cx).cursor(), 5);
        });
        // The list-cleared state reached the grid.
        assert!(
            last_edit_state_autocomplete(&h.grid_requests.borrow()).is_none(),
            "in-cell list cleared on the grid after accept"
        );
    }

    #[gpui::test]
    fn caret_move_updates_list_and_accept_replaces_whole_token(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=sum", window, cx));
        // Move the caret into the MIDDLE of the token (offset 2 = "=s|um"); the recompute the caret
        // seam schedules is exercised here directly.
        let name = upd(&h, cx, |c, window, cx| {
            c.content_input.update(cx, |i, cx| {
                i.set_cursor_position(Position::new(0, 2), window, cx);
            });
            c.recompute_autocomplete(cx);
            let ac = c
                .autocomplete
                .as_ref()
                .expect("list still open on prefix 's'");
            assert!(ac.matches[0].name.starts_with('S'));
            ac.matches[ac.highlight].name
        });
        upd(&h, cx, |c, window, cx| c.autocomplete_accept(window, cx));
        upd(&h, cx, |c, _w, cx| {
            // The WHOLE token is replaced — not spliced at the caret (which would leave "um").
            assert_eq!(
                c.content_input.read(cx).value().to_string(),
                format!("={name}("),
                "accept after a mid-token caret move replaces the whole token"
            );
            assert_eq!(c.content_input.read(cx).cursor(), 1 + name.len() + 1);
        });
    }
}
