//! Editing: the data row (formula bar) and the in-cell editor as one surface
//! (`components/edit_controller.md`, `functional_spec.md §5`). Covers the pending edit — typed
//! entry, quick-edit mode, commit/cancel, Tab, and the cross-editor mirror — plus function
//! autocomplete and signature hints, the reducer/effect plumbing behind the content field, and
//! the data row, cap-error, autocomplete and signature-hint rendering.
//!
//! The reducer itself is [`freecell_core::data_row::DataRow`] and the cross-editor sync is
//! [`crate::chrome::EditController`]; this module is the GPUI plumbing over both. Moved
//! verbatim out of the single-file `chrome/view.rs` (`specs/projects/chrome-view-split`).

use super::*;

impl ChromeView {
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
    pub(super) fn commit_and_move(
        &mut self,
        dir: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn note_commit(&mut self, was_editing: bool) {
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
    pub(super) fn handle_data_row_edit_key(
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
    pub(super) fn on_incell_event(
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
    pub(super) fn mirror_to_in_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn refresh_edit_grid_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn schedule_autocomplete_recompute(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn autocomplete_display(&self) -> Option<AutocompleteDisplay> {
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

    /// Mirrors the reducer's current text into the content widget (suppressing the widget's
    /// change event — `InputState::set_value` sets `emit_events = false`).
    pub(super) fn sync_input_from_reducer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.data_row.text().to_string();
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// The content input emitted an event: typing enters Editing; Enter commits (+ moves the
    /// active cell); Shift+Enter commits + moves up.
    pub(super) fn on_content_event(
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
    pub(super) fn apply_data_effects(
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
    pub(super) fn apply_eval_effects(&mut self, effects: Vec<EvalEffect>, cx: &mut Context<Self>) {
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

    pub(super) fn render_data_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    /// The cap-error popover (`functional_spec.md §4.2`, `ui_design.md §4`): a small dark
    /// tooltip anchored just below the data-row content field's left edge. No backdrop — it
    /// auto-dismisses on the next keystroke (reducer clears its rejection) or focus change.
    pub(super) fn render_cap_error_popover(&self, message: String) -> gpui::AnyElement {
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
    pub(super) fn render_autocomplete_popover(
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
    pub(super) fn render_sig_hint_popover(&self, template: &str) -> gpui::AnyElement {
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
    use super::*;
    use crate::chrome::view::test_support::*;
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use gpui::TestAppContext;

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
