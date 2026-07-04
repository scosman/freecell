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
    div, prelude::*, px, rgb, App, ClickEvent, Context, Entity, FocusHandle, Focusable, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, Rgba, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::spinner::Spinner;
use gpui_component::{Disableable as _, Selectable as _, Sizable as _};

use freecell_core::data_row::{DataRow, DataRowEffect, DataRowEvent, FieldMode};
use freecell_core::eval_indicator::{EvalEffect, EvalEvent, EvalIndicator};
use freecell_core::input_cap::InputRejection;
use freecell_core::palette::FILL_PALETTE;
use freecell_core::selection::{Direction, Motion};
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::{RenderStyle, Rgb, SelectionModel, SheetId};

use freecell_engine::{Command, EditRejectedReason, StyleAttr, WorkerEvent};

use super::{ChromeClient, ChromeGridRequest, ChromeGridSink, SheetTab};

/// The 250 ms no-flash delay for both the content-fetch and evaluating spinners
/// (`ui_design.md §3.1/§3.2`, mirrored from the grid's own delayed hooks).
const SPINNER_DELAY: Duration = Duration::from_millis(250);

// --- Chrome look constants (functional POC greys; `ui_design.md §3`) -----------------
const CHROME_BG: u32 = 0xF3F3F3;
const HAIRLINE: u32 = 0xD9D9D9;
const DIVIDER: u32 = 0xC8C8C8;
const ACTIVE_TAB_BG: u32 = 0xFFFFFF;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;
/// Danger border/text for cap-rejected input + invalid rename (theme danger, `#DC2626`).
const DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the cap-error popover (`ui_design.md §4`).
const TOOLTIP_BG: u32 = 0x2B2B2B;
const TOOLTIP_TEXT: u32 = 0xF5F5F5;

const ACTION_ROW_H: f32 = 36.0;
const DATA_ROW_H: f32 = 32.0;
const TAB_BAR_H: f32 = 30.0;
const REF_BOX_W: f32 = 72.0;
/// The content field's left edge inside the data row = padding + ref box + gap + divider +
/// gap (`render_data_row` layout); the cap-error popover anchors here.
const DATA_ROW_CONTENT_LEFT: f32 = 8.0 + REF_BOX_W + 8.0 + 1.0 + 8.0;

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

    /// The formula-bar state machine (`freecell-core`).
    data_row: DataRow,
    /// The content field's text buffer (stock gpui-component input).
    content_input: Entity<InputState>,
    /// A worker `EditRejected{InputCap}` backstop (the UI validates first, so this is rare);
    /// carries the rejection so the popover shows the same message as a local cap reject.
    cap_error_external: Option<InputRejection>,

    /// The evaluating-spinner state machine (`freecell-core`).
    eval: EvalIndicator,

    /// The fill popover's open state (a `ChromeView`-owned panel).
    fill_open: bool,
    /// The stock color picker for the fill popover's "Custom…" entry.
    color_picker: Entity<ColorPickerState>,

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
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&content_input, window, Self::on_content_event),
            cx.subscribe_in(&rename_input, window, Self::on_rename_event),
            cx.subscribe_in(&color_picker, window, Self::on_color_picker_event),
        ];

        Self {
            client,
            grid,
            focus_handle: cx.focus_handle(),
            active_sheet,
            selection: SelectionModel::default(),
            active_style: None,
            data_row: DataRow::default(),
            content_input,
            cap_error_external: None,
            eval: EvalIndicator::default(),
            fill_open: false,
            color_picker,
            sheets,
            rename_target: None,
            rename_input,
            rename_error: false,
            context_menu: None,
            confirm_delete: None,
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
        self.active_style = if self.selection.is_single() {
            self.client
                .render_style(self.active_sheet, self.selection.active)
        } else {
            None
        };
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
        self.active_style = if selection.is_single() {
            self.client
                .render_style(self.active_sheet, selection.active)
        } else {
            None
        };
        let effects = self.data_row.reduce(DataRowEvent::SelectionChanged {
            single: selection.is_single(),
        });
        // begin_fetch / disable cleared the field; mirror the reducer's text into the widget.
        self.sync_input_from_reducer(window, cx);
        self.apply_data_effects(effects, window, cx);
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
        let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
        self.apply_data_effects(effects, window, cx);
        let committed = self.data_row.mode() != FieldMode::Editing;
        cx.notify();
        committed
    }

    /// Escape while editing: revert the field to the last-fetched content and hand focus back
    /// to the grid.
    pub fn escape_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_row.mode() != FieldMode::Editing {
            return;
        }
        let effects = self.data_row.reduce(DataRowEvent::Escape);
        self.sync_input_from_reducer(window, cx);
        self.apply_data_effects(effects, window, cx);
        cx.notify();
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
                self.data_row
                    .reduce(DataRowEvent::ContentFetched { req_id, raw });
                // Sync the widget only when the reducer populated the field (fresh reply,
                // still Idle) — never mid-edit, so a late reply can't reset the caret.
                if self.data_row.mode() == FieldMode::Idle {
                    self.sync_input_from_reducer(window, cx);
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
                // A keystroke dismisses the cap-error popover (`functional_spec.md §4.2`): the
                // reducer clears its own rejection in `Edited`; the worker backstop is cleared
                // here so both sources dismiss on the next keystroke.
                self.cap_error_external = None;
                let text = self.content_input.read(cx).value().to_string();
                let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                self.apply_data_effects(effects, window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                let mut effects = self.data_row.reduce(DataRowEvent::Commit);
                if *shift {
                    // Shift+Enter commits and moves up (the reducer's Commit hardcodes Down).
                    for effect in &mut effects {
                        if matches!(
                            effect,
                            DataRowEffect::MoveActive(Motion::Move(Direction::Down))
                        ) {
                            *effect = DataRowEffect::MoveActive(Motion::Move(Direction::Up));
                        }
                    }
                }
                self.apply_data_effects(effects, window, cx);
                cx.notify();
            }
            InputEvent::Blur => {
                // Focus leaving the field dismisses the cap-error popover
                // (`functional_spec.md §4.2`). The reducer clears its own rejection on the
                // next edit/escape; the worker backstop is cleared here.
                if self.cap_error_external.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::Focus => {}
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

    // ---- Sheet tab bar --------------------------------------------------------------------

    /// Replaces the tab list + active sheet (fixtures / Phase-11 init).
    pub fn set_sheets(&mut self, sheets: Vec<SheetTab>, active: SheetId, cx: &mut Context<Self>) {
        self.sheets = sheets;
        self.active_sheet = active;
        cx.notify();
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
        self.context_menu = None;
        self.refresh_active_style(cx);
    }

    /// Switches the active sheet (tab click) and asks the grid to follow.
    pub fn select_sheet(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.active_sheet {
            return;
        }
        self.active_sheet = id;
        self.context_menu = None;
        self.grid
            .emit(ChromeGridRequest::SetActiveSheet(id), window, cx);
        cx.notify();
    }

    /// Adds a sheet (the worker names it and republishes; the UI switches on `SheetsChanged`).
    pub fn add_sheet(&self) {
        self.client.send(Command::AddSheet);
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

    /// The ref box text (`B7` / `B2:D9`).
    pub fn ref_box_text(&self) -> String {
        self.selection.to_a1()
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

impl Focusable for ChromeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChromeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("freecell-chrome")
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            // Fill the available height when hosting the grid, so the grid slot can flex.
            .when(self.body.is_some(), |d| d.flex_1().min_h_0())
            .child(self.render_action_row(cx))
            .child(self.render_data_row(cx))
            // The grid body fills the space between the data row and the tab bar
            // (`ui_design.md §3`: action → data → grid → tabs).
            .when_some(self.body.clone(), |d, body| {
                d.child(div().flex_1().min_h_0().w_full().child(body))
            })
            .child(self.render_tab_bar(cx))
            .children(self.render_overlays(cx))
    }
}

impl ChromeView {
    fn render_action_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = |id: &'static str,
                      label: &'static str,
                      tooltip: &'static str,
                      pressed: bool,
                      attr: StyleAttr,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .label(label)
                .tooltip(tooltip)
                .ghost()
                .small()
                .selected(pressed)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.toggle_style(attr, window, cx);
                }))
        };

        div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(ACTION_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            .child(toggle(
                "bold",
                "B",
                "Bold ⌘B",
                self.bold_active(),
                StyleAttr::Bold,
                cx,
            ))
            .child(toggle(
                "italic",
                "I",
                "Italic ⌘I",
                self.italic_active(),
                StyleAttr::Italic,
                cx,
            ))
            .child(toggle(
                "underline",
                "U",
                "Underline ⌘U",
                self.underline_active(),
                StyleAttr::Underline,
                cx,
            ))
            .child(div().w(px(1.0)).h(px(20.0)).mx_1().bg(rgb(DIVIDER)))
            .child(
                Button::new("fill")
                    .label("Fill ▾")
                    .tooltip("Fill color")
                    .ghost()
                    .small()
                    .selected(self.fill_open)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_fill_popover(cx);
                    })),
            )
            // Right-aligned evaluating spinner (`ui_design.md §3.1`).
            .child(div().flex_1())
            .when(self.eval.spinner(), |row| row.child(Spinner::new().small()))
    }

    fn render_data_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.data_row.mode() == FieldMode::Disabled;
        let cap_error = self.cap_error_visible();

        let mut content = Input::new(&self.content_input).disabled(disabled).w_full();
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
            // Content field (danger border on cap reject).
            .child(
                div()
                    .flex_1()
                    .when(cap_error, |d| {
                        d.border_1().border_color(rgb(DANGER)).rounded_md()
                    })
                    .child(content),
            )
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(TAB_BAR_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_t_1()
            .border_color(rgb(HAIRLINE));

        for tab in &self.sheets {
            row = row.child(self.render_tab(tab, cx));
        }

        row.child(
            Button::new("add-sheet")
                .label("+")
                .tooltip("New sheet")
                .ghost()
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.add_sheet();
                    cx.notify();
                })),
        )
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

        div()
            .id(gpui::ElementId::Name(format!("tab-{}", id.0).into()))
            .px_3()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded_t_md()
            .bg(rgb(if is_active { ACTIVE_TAB_BG } else { CHROME_BG }))
            .text_size(px(13.0))
            .text_color(rgb(if is_active { TEXT } else { MUTED_TEXT }))
            .when(is_active, |d| {
                d.border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_color(rgb(HAIRLINE))
            })
            .child(tab.name.clone())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.rename_start(id, window, cx);
                } else {
                    this.select_sheet(id, window, cx);
                }
            }))
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

        if let Some(message) = self.cap_error_message() {
            overlays.push(self.render_cap_error_popover(message));
        }
        if self.fill_open {
            overlays.push(self.render_fill_popover(cx));
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
        div().absolute().top_0().left_0().size_full().on_mouse_down(
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
                self.backdrop(|this, _w, _cx| this.fill_open = false, cx)
                    .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(120.0))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::{ChromeClient, RecordingClient};
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use freecell_core::{CellRef, SelectionModel};
    use freecell_engine::{Command, SheetMeta, StyleAttr, WorkerEvent};
    use gpui::{px, size, TestAppContext};
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
        let client = Rc::new(RecordingClient::new());
        let grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>> = Rc::new(RefCell::new(Vec::new()));

        cx.update(gpui_component::init);

        let client_for_window = client.clone();
        let reqs_for_window = grid_requests.clone();
        let mut chrome_out: Option<Entity<ChromeView>> = None;
        let chrome_slot = &mut chrome_out;

        let window = cx.open_window(size(px(900.0), px(200.0)), |window, cx| {
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
}
