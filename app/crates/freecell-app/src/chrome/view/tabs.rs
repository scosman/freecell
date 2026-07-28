//! The sheet tab bar (`components/app_shell.md §Sheet tab bar`, `ui_design.md §3.4`): the tab
//! strip and its scroller, tab selection and add, the reorder drag and its drop indicator,
//! inline rename with validation, and the right-click context menu plus delete confirmation.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

/// A tab press that moves less than this (device px) is a click (select / rename), not a drag —
/// only past it does the lift + drop indicator appear (`ui_design.md §3`).
const TAB_DRAG_THRESHOLD_PX: f32 = 4.0;
/// The reorder drop indicator + dragged-tab outline accent (Office Accent 1, matching the
/// borders popover's selected-swatch ring). `ui_design.md §3`: a 2 px accent vertical bar.
const TAB_DROP_ACCENT: u32 = 0x4472C4;
/// Half the inter-tab gap (`gap_1` = 4 px), used to place the drop indicator in the gap when it
/// lands before the first / after the last tab.
const TAB_GAP_HALF: f32 = 2.0;

/// A potential or in-flight sheet-tab reorder drag (`functional_spec.md §6.1`, `ui_design.md §3`).
/// Recorded on a tab mouse-down as a *potential* drag; `dragging` flips true only once the pointer
/// crosses [`TAB_DRAG_THRESHOLD_PX`] from `start_x`, at which point the lift + drop indicator
/// appear. Modeled off the grid's `ResizeDrag`. All coordinates are window-space device px.
#[derive(Debug, Clone, Copy)]
pub(super) struct TabDrag {
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
pub(super) struct TabSpan {
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

impl ChromeView {
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
    pub(super) fn merge_sheet_metas(&mut self, metas: &[freecell_engine::SheetMeta]) {
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

    pub(super) fn on_rename_event(
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

    pub(super) fn render_tab_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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

    pub(super) fn render_context_menu(
        &self,
        id: SheetId,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
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

    pub(super) fn render_delete_confirm(
        &self,
        id: SheetId,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
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
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use freecell_engine::SheetMeta;
    use gpui::TestAppContext;

    // ---- Sheet tab bar --------------------------------------------------------------------

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
}
