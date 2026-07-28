//! The find/replace bar (`functional_spec.md §4`, `ui_design.md §1`) — the bar itself, which
//! renders below the data row and pushes the grid down, plus match stepping, the two match
//! toggles, and replace/replace-all.
//!
//! The worker reply that drives the match set (`WorkerEvent::FindResults`) is routed by
//! [`super::shell`]'s `on_worker_event`; this module owns the query, the match cursor and the
//! bar itself.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

use crate::chrome::sidebar::close_button;

/// The find/replace bar's two text fields' width (`ui_design.md §1`: ~220 px each).
const FIND_FIELD_W: f32 = 220.0;
/// The match counter's min width so "3 of 12" ↔ "No results" doesn't jitter the trailing group.
const FIND_COUNTER_MIN_W: f32 = 72.0;
/// Muted counter text (`ui_design.md §1`: "No results" in a muted color).
const FIND_COUNTER_MUTED: u32 = 0x777777;

impl ChromeView {
    /// The find/replace bar (`functional_spec.md §4.1`, `ui_design.md §1`) — rendered directly below
    /// the data row while [`find_open`](Self::find_open). Left→right: find field · replace field ·
    /// match-case + match-entire-cell toggles · divider · prev/next · counter · spacer · Replace +
    /// Replace All · dismiss. Escape (on the bar) closes it (mirrors the data row's Escape).
    pub(super) fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
    pub(super) fn recompute_matches(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn rescope_find_if_open(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn first_match_from_selection(&self) -> Option<usize> {
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
    pub(super) fn select_current_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn on_find_input_event(
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
    pub(super) fn on_replace_input_event(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use crate::chrome::RecordingClient;
    use gpui::TestAppContext;

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
}
