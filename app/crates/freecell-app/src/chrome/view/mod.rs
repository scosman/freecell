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
//!
//! The implementation is split across this directory by feature domain
//! (`specs/projects/chrome-view-split`). This file owns the [`ChromeView`] struct itself, its
//! constructor, and the constants shared across domains; each child module holds one domain's
//! `impl ChromeView` methods and that domain's tests. Children are *descendants* of this
//! module, so they reach every private field here without any visibility change; items they
//! need from each other are `pub(super)`, which scopes them to this subtree exactly as
//! private-to-`view.rs` did before the split.

mod cf_editor;
mod cf_sidebar;
mod charts;
mod find;
mod formatting;
mod stats;
mod tabs;
#[cfg(test)]
mod test_support;

use cf_editor::CfMenu;
pub use charts::{ChartPanel, ChartPanelSeries};
use formatting::SYSTEM_DEFAULT_FAMILY;
use tabs::{TabDrag, TabSpan};

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
use freecell_core::functions;
use freecell_core::input_cap::InputRejection;
use freecell_core::palette::FILL_PALETTE;
use freecell_core::selection::{Direction, Motion};
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::{
    effective_range, format_stat_count, format_stat_value, limits, region_at, regions_intersecting,
    Align, CellKind, CellRange, CellRef, CfColorStop, CfFormat, CfPeriod, CfPreview, CfRuleSpec,
    CfRuleView, CfTextOp, CfThresholdKind, CfValueOp, RenderStyle, Rgb, SelectionModel,
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

// `pub(crate)` so the shared docked-sidebar container (`chrome::sidebar`) positions the card
// between the data row and the tab bar (the right-docked width is `sidebar::SIDEBAR_W`).
pub(crate) const ACTION_ROW_H: f32 = 36.0;

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

// `DATA_ROW_H` / `TAB_BAR_H` are `pub(crate)` so the shared docked-sidebar container
// (`chrome::sidebar`) can dock the card between the data row and the tab bar.
pub(crate) const DATA_ROW_H: f32 = 32.0;
/// The formula-bar content entry's height: [`DATA_ROW_H`] minus 2 px breathing room above **and**
/// below (BUG C), so the row's `items_center` insets the entry within the bar without changing the
/// bar height. gpui-component's single-line `Input` otherwise renders at its fixed control height
/// (`Size::Medium` → 32 px) and fills the row edge-to-edge, which reads as cramped.
const DATA_ROW_FIELD_H: f32 = DATA_ROW_H - 4.0;
pub(crate) const TAB_BAR_H: f32 = 30.0;
const REF_BOX_W: f32 = 72.0;
/// The content field's left edge inside the data row = padding + ref box + gap + divider +
/// gap (`render_data_row` layout); the cap-error popover anchors here.
const DATA_ROW_CONTENT_LEFT: f32 = 8.0 + REF_BOX_W + 8.0 + 1.0 + 8.0;

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
    /// The color-scale editor's per-stop value inputs (`components/cf_sidebar.md §8`) — a fixed 3
    /// (the max stop count), of which only the active scale's stops are shown. Seeded from
    /// `CfEditorState.scale` on editor open / kind change and synced back into it on edit.
    cf_stop_value_inputs: Vec<Entity<InputState>>,
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
        let cf_stop_value_inputs: Vec<Entity<InputState>> = (0..3)
            .map(|_| cx.new(|cx| InputState::new(window, cx).placeholder("Value")))
            .collect();

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
        let mut subscriptions = vec![
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
        for input in &cf_stop_value_inputs {
            subscriptions.push(cx.subscribe_in(input, window, Self::on_cf_input_event));
        }

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
            cf_stop_value_inputs,
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
            // Symmetric with `escape_edit` / `commit_and_move`: a click-away / adopt-selection /
            // action-button commit ends the edit, so drop all formula-feature derived state
            // (highlights, autocomplete, sig-hint, pending-ref). Not user-visible in Phase 2 (the
            // grid push is gated on `editing`), but keeps the pending-ref span from surviving a
            // commit into Phase 3's point-mode consumption.
            self.edit.clear_formula_state();
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
        self.edit.clear_formula_state();
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
        self.recompute_formula_edit_state(cx);
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
        self.recompute_formula_edit_state(cx);
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
            self.edit.clear_formula_state();
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
        if self.edit.autocomplete().is_some() {
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
                self.recompute_formula_edit_state(cx);
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
            .then(|| self.edit.sig_hint().map(SharedString::from))
            .flatten();
        // Reference highlights + point-mode signals are meaningful only while a formula edit is
        // live; gate on `editing` so the grid's copy auto-clears the instant the edit ends
        // (`functional_spec.md §3` lifecycle). `reference_ready`/`pending_ref` are plumbed now but
        // only consumed by the grid in Phase 3. `ref_highlights` already holds only same-sheet
        // tokens (the color map colors cross-sheet refs for the future in-editor control).
        let reference_ready = editing && self.edit.reference_ready();
        let pending_ref = editing && self.edit.pending_ref().is_some();
        let ref_highlights = if editing {
            self.edit.ref_highlights()
        } else {
            Vec::new()
        };
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
                reference_ready,
                pending_ref,
                ref_highlights,
            },
            window,
            cx,
        );
    }

    /// The active sheet's display name, resolved from the tab list (used to set each reference
    /// token's `same_sheet` flag — `freecell_engine::lex_formula_refs`). Empty if the sheet is not
    /// (yet) in the tab list.
    fn active_sheet_name(&self) -> String {
        self.sheets
            .iter()
            .find(|t| t.id == self.active_sheet)
            .map(|t| t.name.clone())
            .unwrap_or_default()
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

    /// Recomputes **all** formula-feature state — reference highlights + color map, the
    /// reference-ready predicate, the autocomplete list, and the signature hint — from the
    /// **driving** editor's live text + caret in one pass (the consolidation seam,
    /// `architecture.md §6`; generalizes the shipped `recompute_autocomplete`). Delegates the
    /// computation to the `edit`-owned [`EditController::recompute_formula`]; the caller pushes grid
    /// state + notifies. A visible cap error takes precedence — every formula feature is cleared
    /// while it shows.
    fn recompute_formula_edit_state(&mut self, cx: &mut Context<Self>) {
        self.recompute_formula_edit_state_keep_pending(false, cx);
    }

    /// [`recompute_formula_edit_state`](Self::recompute_formula_edit_state) with explicit control
    /// over the pending-ref span: every **user-driven** transition (keystroke / caret move) passes
    /// `keep_pending = false` so the "replace on next point" window is exactly one action wide
    /// (`architecture.md §5` Cleared); only [`insert_reference`](Self::insert_reference) passes
    /// `true`, so the span it just set survives its own recompute.
    fn recompute_formula_edit_state_keep_pending(
        &mut self,
        keep_pending: bool,
        cx: &mut Context<Self>,
    ) {
        if self.cap_error_visible() {
            self.edit.clear_formula_state();
            return;
        }
        let input = self.driving_input().read(cx);
        let text = input.value().to_string();
        let caret = input.cursor();
        let sheet = self.active_sheet_name();
        self.edit
            .recompute_formula(&text, caret, &sheet, keep_pending);
    }

    /// Recompute the formula state and re-push grid state + notify (the full per-keystroke effect,
    /// used by the deferred caret-move recompute below).
    fn recompute_autocomplete_and_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_formula_edit_state(cx);
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
        let ac = self.edit.autocomplete()?;
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
        if let Some(ac) = self.edit.autocomplete_mut() {
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
        if self.edit.take_autocomplete().is_some() {
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
        if let Some(ac) = self.edit.autocomplete_mut() {
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
        let Some(ac) = self.edit.take_autocomplete() else {
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

        // Recompute the whole formula state off the spliced text/caret so the reference highlights,
        // color map, and signature hint stay in lockstep (the caret now sits just after `(`, inside
        // the accepted call — the recompute derives the same `sig.template` signature hint).
        self.recompute_formula_edit_state(cx);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Point-mode splice (`functional_spec.md §2`, `architecture.md §5`): insert the pointed
    /// reference `a1` into the in-progress formula at the caret — the exact analog of
    /// [`accept_autocomplete`](Self::accept_autocomplete). When `replace_pending` and a pending-ref
    /// span is set (a just-pointed reference with nothing typed since), **overwrite** that span
    /// (re-aiming / the live drag); otherwise **append** at the caret. The just-inserted span becomes
    /// the new pending ref, so the next point action replaces it — until a keystroke / caret move
    /// clears it (`§5` lifecycle). Routed here from [`GridEvent::InsertReference`].
    pub fn insert_reference(
        &mut self,
        a1: &str,
        replace_pending: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Point-mode only reaches an open edit (the grid emits only when reference-ready / pending);
        // guard defensively so a stray route can never splice into an idle field.
        if self.data_row.mode() != FieldMode::Editing {
            return;
        }
        let origin = self.edit.origin();
        let text = self.driving_input().read(cx).value().to_string();
        let caret = self.driving_input().read(cx).cursor();
        // Splice region: replace the pending span (re-aim / drag-grow), else insert at the caret.
        let (start, end) = match self.edit.pending_ref() {
            Some(span) if replace_pending && span.start <= text.len() && span.end <= text.len() => {
                (span.start, span.end)
            }
            _ => (caret.min(text.len()), caret.min(text.len())),
        };
        let mut new_text = String::with_capacity(text.len() + a1.len());
        new_text.push_str(&text[..start]);
        new_text.push_str(a1);
        new_text.push_str(&text[end..]);
        let new_caret = start + a1.len();

        // Drive the shared reducer + both editors exactly as the accept path does (keeps
        // cap-validation / mirror / undo identical to typing — one commit = one undo step).
        let effects = self.data_row.reduce(DataRowEvent::Edited {
            text: new_text.clone(),
        });
        let char_col = new_text[..new_caret].chars().count() as u32;
        self.set_driving_text_and_caret(origin, &new_text, char_col, window, cx);
        self.apply_data_effects(effects, window, cx);
        self.mirror_other_editor(origin, &new_text, window, cx);
        // Return focus to the driving editor. The grid took keyboard focus in its own
        // `handle_mouse_down` before emitting this point insert, and `prevent_default` only skips
        // gpui's end-of-dispatch focus transfer — it does not undo that explicit grab. Without this,
        // the next keystroke would miss the editor entirely (data-row → a fresh type-to-replace that
        // wipes the formula; in-cell → swallowed). Re-focusing here (as `begin_typed`/`begin_in_cell`
        // do) survives the built-in transfer because the whole emit chain is synchronous inside the
        // mouse-down dispatch and `prevent_default` runs afterward (`functional_spec.md §2/§4`).
        self.driving_input()
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
        // The just-inserted span becomes pending (replace on the next point action).
        self.edit
            .set_pending_ref(Some(new_caret - a1.len()..new_caret));
        // Recompute the formula state off the spliced text — KEEPING the pending span we just set
        // (the programmatic `set_value` suppressed `Change`, so nothing else clears it here).
        self.recompute_formula_edit_state_keep_pending(true, cx);
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
                self.recompute_formula_edit_state(cx);
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

    /// Whether the worker is degraded (read-only) — all mutating action-bar controls disable.
    pub fn is_degraded(&self) -> bool {
        self.degraded
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
            // Merge / Unmerge toggle — a cell-layout concern grouped after wrap (`ui_design.md §1`).
            // Built directly (not the shared `toggle` closure, which drives character styles): it has
            // its own `on_click`, and a distinct disabled rule (a lone 1×1 not in any merge is
            // disabled — nothing to toggle — as well as the degraded case). One icon both directions;
            // the pressed state + tooltip swap convey mode.
            .child(
                Button::new("merge")
                    .icon(Icon::empty().path("icons/table-cells-merge.svg"))
                    .tooltip(if self.merge_active() {
                        "Unmerge cells"
                    } else {
                        "Merge cells"
                    })
                    .ghost()
                    .small()
                    .disabled(self.merge_disabled())
                    .selected(self.merge_active())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_merge(window, cx);
                    })),
            )
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
            } else if let Some(template) = self.edit.sig_hint() {
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
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use freecell_core::{CellRange, CellRef, SelectionModel};
    use freecell_engine::{Command, WorkerEvent};
    use gpui::{Modifiers, TestAppContext};

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
        // A window comfortably wider than the true controls but still below the old 1152 estimate —
        // the band the removed `min_w = 1152` over-padded (its viewport would fall between the true
        // width and 1152 → false chevrons). Derived as the middle of that band rather than a fixed
        // offset, so it stays valid as the action row grows: the frame's `px_2` (16px total) is the
        // gap between the window width and the scroller's viewport, so the band is
        // `(natural + FRAME_PAD, 1152)`.
        const FRAME_PAD: f32 = 16.0;
        let fits = (natural + FRAME_PAD + 1152.0) / 2.0;
        assert!(
            fits > natural + FRAME_PAD && fits < 1152.0,
            "the fit window {fits} sits in the (controls, 1152) no-overflow band (natural {natural})"
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

    /// The reference highlights on the most recent edit-state push (`None` = no `EditState` pushed).
    fn last_edit_state_ref_highlights(reqs: &[ChromeGridRequest]) -> Option<Vec<(CellRange, u8)>> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState { ref_highlights, .. } => Some(ref_highlights.clone()),
            _ => None,
        })
    }

    /// The `reference_ready` flag on the most recent edit-state push.
    fn last_edit_state_reference_ready(reqs: &[ChromeGridRequest]) -> Option<bool> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState {
                reference_ready, ..
            } => Some(*reference_ready),
            _ => None,
        })
    }

    fn a1(row: u32, col: u32) -> CellRange {
        CellRange::single(CellRef::new(row, col))
    }

    // ---- Reference highlighting (formula-point-mode Phase 2) --------------------------------

    #[gpui::test]
    fn formula_edit_pushes_same_sheet_ref_highlights(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1+B2", window, cx));
        let highlights = last_edit_state_ref_highlights(&h.grid_requests.borrow())
            .expect("an EditState was pushed");
        assert_eq!(
            highlights,
            vec![(a1(0, 0), 0), (a1(1, 1), 1)],
            "A1 and B2 highlight in distinct slots, first-appearance order"
        );
    }

    #[gpui::test]
    fn repeated_ref_shares_highlight_color(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1+A1", window, cx));
        let highlights = last_edit_state_ref_highlights(&h.grid_requests.borrow()).unwrap();
        assert_eq!(
            highlights,
            vec![(a1(0, 0), 0), (a1(0, 0), 0)],
            "the two A1 occurrences share slot 0"
        );
    }

    #[gpui::test]
    fn cross_sheet_ref_absent_from_highlights_but_in_color_map(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("=Sheet2!A1", window, cx)
        });
        // No grid highlight for a cross-sheet ref (the other sheet is not visible)…
        let highlights = last_edit_state_ref_highlights(&h.grid_requests.borrow()).unwrap();
        assert!(
            highlights.is_empty(),
            "a cross-sheet reference draws no grid highlight"
        );
        // …but the color map still colors it (consumed by the future in-editor styling control).
        upd(&h, cx, |c, _w, _cx| {
            assert_eq!(c.edit.ref_tokens().len(), 1, "the token is still lexed");
            assert!(
                !c.edit.ref_tokens()[0].same_sheet,
                "and flagged cross-sheet"
            );
            assert_eq!(c.edit.ref_colors().len(), 1, "and still assigned a color");
        });
    }

    #[gpui::test]
    fn commit_clears_ref_highlights(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1", window, cx));
        assert_eq!(
            last_edit_state_ref_highlights(&h.grid_requests.borrow())
                .unwrap()
                .len(),
            1,
            "the highlight shows while editing"
        );
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.on_edit_commit_requested(window, cx);
        });
        assert_eq!(
            last_edit_state_ref_highlights(&h.grid_requests.borrow()),
            Some(vec![]),
            "committing the edit removes every reference highlight"
        );
    }

    #[gpui::test]
    fn escape_clears_ref_highlights(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1", window, cx));
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| c.escape_edit(window, cx));
        assert_eq!(
            last_edit_state_ref_highlights(&h.grid_requests.borrow()),
            Some(vec![]),
            "cancelling the edit removes every reference highlight"
        );
    }

    #[gpui::test]
    fn reference_ready_pushed_for_open_formula(cx: &mut TestAppContext) {
        // After an operator the caret is reference-ready (point-mode entry, consumed in Phase 3)…
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1+", window, cx));
        assert_eq!(
            last_edit_state_reference_ready(&h.grid_requests.borrow()),
            Some(true),
            "caret after `+` is reference-ready"
        );
        // …but right after a complete reference it is not.
        upd(&h, cx, |c, window, cx| c.begin_typed("=A1", window, cx));
        assert_eq!(
            last_edit_state_reference_ready(&h.grid_requests.borrow()),
            Some(false),
            "caret right after a complete ref is not reference-ready"
        );
    }

    // ---- Point-mode insertion (formula-point-mode Phase 3) ----------------------------------

    /// A point insert on an empty formula appends the reference at the caret and marks its span
    /// pending (so the next point re-aims it).
    #[gpui::test]
    fn insert_reference_appends_at_caret(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("C3", false, window, cx)
        });
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(c.content_input.read(cx).value().to_string(), "=C3");
            assert_eq!(c.content_input.read(cx).cursor(), 3, "caret after the ref");
            assert_eq!(
                c.edit.pending_ref(),
                Some(1..3),
                "the inserted span is pending"
            );
        });
    }

    /// A second point with the ref still pending REPLACES it (Excel re-aiming), not appends.
    #[gpui::test]
    fn insert_reference_replaces_pending(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("A1", false, window, cx)
        });
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("B2", true, window, cx)
        });
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(
                c.content_input.read(cx).value().to_string(),
                "=B2",
                "the pending A1 is replaced, not appended"
            );
            assert_eq!(c.edit.pending_ref(), Some(1..3), "B2 is now pending");
        });
    }

    /// A keystroke after a point FIXES the pending ref (clears the span); the next point then appends
    /// a fresh reference (`functional_spec.md §2` DPM.2).
    #[gpui::test]
    fn keystroke_after_point_appends_next(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("B2", false, window, cx)
        });
        // Simulate typing `+` (the Change path recomputes with keep_pending = false → clears pending).
        upd(&h, cx, |c, window, cx| {
            c.content_input.update(cx, |i, cx| {
                i.set_value("=B2+", window, cx);
                i.set_cursor_position(Position::new(0, 4), window, cx);
            });
            c.recompute_formula_edit_state(cx);
        });
        upd(&h, cx, |c, _w, _cx| {
            assert_eq!(
                c.edit.pending_ref(),
                None,
                "a keystroke fixes the pending ref"
            );
        });
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("C3", false, window, cx)
        });
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(
                c.content_input.read(cx).value().to_string(),
                "=B2+C3",
                "the next point appends a fresh ref"
            );
        });
    }

    /// A caret move (not a point) also clears the pending-ref window.
    #[gpui::test]
    fn pending_cleared_by_caret_move(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("A1", false, window, cx)
        });
        upd(&h, cx, |c, _w, _cx| {
            assert!(c.edit.pending_ref().is_some(), "pending after the point");
        });
        upd(&h, cx, |c, window, cx| {
            c.content_input.update(cx, |i, cx| {
                i.set_cursor_position(Position::new(0, 1), window, cx);
            });
            c.recompute_formula_edit_state(cx);
        });
        upd(&h, cx, |c, _w, _cx| {
            assert_eq!(
                c.edit.pending_ref(),
                None,
                "a caret move clears the pending ref"
            );
        });
    }

    /// Pointing at the cell being edited inserts a self-reference (DPM.5) — not blocked; the engine's
    /// circular-ref handling surfaces it at commit.
    #[gpui::test]
    fn self_reference_allowed(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        upd(&h, cx, |c, window, cx| {
            // A1 is the active cell; pointing it is allowed.
            c.insert_reference("A1", false, window, cx)
        });
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(c.content_input.read(cx).value().to_string(), "=A1");
        });
    }

    /// The autocomplete→point happy path (`functional_spec.md §4`): accepting `SUM(` leaves the caret
    /// reference-ready, so a point click yields `=SUM(C3` with no typing.
    #[gpui::test]
    fn autocomplete_then_point_happy_path(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=sum", window, cx));
        upd(&h, cx, |c, window, cx| c.autocomplete_accept(window, cx));
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(c.content_input.read(cx).value().to_string(), "=SUM(");
            assert!(
                c.edit.reference_ready(),
                "caret right after `(` is reference-ready"
            );
        });
        upd(&h, cx, |c, window, cx| {
            c.insert_reference("C3", false, window, cx)
        });
        upd(&h, cx, |c, _w, cx| {
            assert_eq!(
                c.content_input.read(cx).value().to_string(),
                "=SUM(C3",
                "the pointed ref lands inside the accepted call"
            );
        });
    }

    /// A point insert must RETURN keyboard focus to the driving editor: the grid takes focus in its
    /// own `handle_mouse_down` before emitting the insert, so without an explicit re-focus the next
    /// keystroke would miss the editor entirely (data-row → a fresh type-to-replace that wipes the
    /// formula). We model the grid's focus grab by focusing a throwaway handle, then assert
    /// `insert_reference` restored the data-row input's focus (so the next key appends to the formula
    /// rather than starting a new edit).
    #[gpui::test]
    fn point_insert_returns_focus_to_data_row_editor(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=", window, cx));
        let focused = upd(&h, cx, |c, window, cx| {
            // The grid grabbed focus at mouse-down; model that.
            let grid_focus = cx.focus_handle();
            window.focus(&grid_focus, cx);
            c.insert_reference("C3", false, window, cx);
            c.content_input.read(cx).focus_handle(cx).is_focused(window)
        });
        assert!(
            focused,
            "the data-row editor must regain focus so typing continues the formula"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.content_text(cx)),
            "=C3",
            "the ref was inserted"
        );
    }

    /// The in-cell driving editor half of the same guarantee: after a point insert the in-cell input
    /// (not the grid) holds focus, so its next keystroke is not swallowed.
    #[gpui::test]
    fn point_insert_returns_focus_to_in_cell_editor(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("=", window, cx);
        });
        let focused = upd(&h, cx, |c, window, cx| {
            let grid_focus = cx.focus_handle();
            window.focus(&grid_focus, cx);
            c.insert_reference("C3", false, window, cx);
            c.edit
                .in_cell()
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(
            focused,
            "the in-cell editor must regain focus after a point insert"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "=C3");
    }

    #[gpui::test]
    fn typing_prefix_opens_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=su", window, cx));
        upd(&h, cx, |c, _w, _cx| {
            let ac = c.edit.autocomplete().expect("list open on =su");
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
            assert!(c.edit.autocomplete().is_none(), "no `=` → no list");
        });
    }

    #[gpui::test]
    fn nav_moves_highlight_clamped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| c.begin_typed("=su", window, cx));
        upd(&h, cx, |c, _w, cx| {
            c.autocomplete_nav(false, cx);
            assert_eq!(c.edit.autocomplete().unwrap().highlight, 0, "clamp at top");
            c.autocomplete_nav(true, cx);
            assert_eq!(c.edit.autocomplete().unwrap().highlight, 1);
            c.autocomplete_nav(false, cx);
            assert_eq!(c.edit.autocomplete().unwrap().highlight, 0);
        });
    }

    #[gpui::test]
    fn accept_inserts_name_paren_and_places_caret(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // "=sum" → SUM is the exact match, highlighted first.
        upd(&h, cx, |c, window, cx| c.begin_typed("=sum", window, cx));
        upd(&h, cx, |c, window, cx| {
            assert_eq!(
                c.edit.autocomplete().unwrap().matches[0].name,
                "SUM",
                "exact SUM highlighted"
            );
            c.autocomplete_accept(window, cx);
        });
        upd(&h, cx, |c, _w, cx| {
            assert!(c.edit.autocomplete().is_none(), "list closes on accept");
            assert_eq!(c.content_input.read(cx).value().to_string(), "=SUM(");
            assert_eq!(c.content_input.read(cx).cursor(), 5, "caret just after `(`");
            assert_eq!(c.edit.sig_hint(), Some("SUM(number1, [number2], …)"));
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
            assert!(c.edit.autocomplete().is_none(), "list dismissed");
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
            assert!(c.edit.autocomplete().is_none(), "no name token after `(`");
            assert_eq!(c.edit.sig_hint(), Some("SUM(number1, [number2], …)"));
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
            c.recompute_formula_edit_state(cx);
            c.refresh_edit_grid_state(window, cx);
        });
        upd(&h, cx, |c, _w, _cx| {
            assert!(c.edit.autocomplete().is_some(), "in-cell list open");
        });
        // The in-cell list is pushed to the grid for rendering.
        assert!(
            last_edit_state_autocomplete(&h.grid_requests.borrow()).is_some(),
            "in-cell autocomplete pushed to grid"
        );
        upd(&h, cx, |c, window, cx| c.autocomplete_accept(window, cx));
        upd(&h, cx, |c, _w, cx| {
            assert!(c.edit.autocomplete().is_none());
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
            c.recompute_formula_edit_state(cx);
            let ac = c
                .edit
                .autocomplete()
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
