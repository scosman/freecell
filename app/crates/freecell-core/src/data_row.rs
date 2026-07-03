//! The data-row (formula-bar) state machine as a pure reducer
//! (`components/app_shell.md §Data row`, `functional_spec.md §3.3`).
//!
//! Extracted to `freecell-core` so the load-bearing behaviours — stale-reply drop,
//! cap-reject-keeps-editing, escape-reverts, commit-on-click-away, multiselect-disables,
//! the 250 ms fetch spinner — are table-tested headless on Linux. The window owns a
//! [`DataRow`], feeds it [`DataRowEvent`]s, and performs the returned [`DataRowEffect`]s
//! (send worker commands, move the grid selection, toggle the spinner, focus the grid).

use crate::input_cap::validate_input;
use crate::selection::{Direction, Motion};

/// The field's high-level mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldMode {
    /// Single-cell selection: showing the engine's content (or awaiting the reply).
    Idle,
    /// The user is editing a pending value.
    Editing,
    /// Multi-cell selection: the field is disabled and empty (`functional_spec.md §3.2`).
    Disabled,
}

/// The data-row state. All fields are observable so the view renders directly from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRow {
    mode: FieldMode,
    /// The text shown / being edited.
    text: String,
    /// The last content fetched from the engine — what Escape reverts to.
    committed: String,
    /// The id of the most recent `GetCellContent` request. Replies with a different id are
    /// stale (the selection moved on) and are dropped.
    latest_req: u64,
    /// Whether an Idle content fetch is still outstanding (drives the delayed spinner).
    awaiting: bool,
    /// Whether the content-fetch spinner is currently shown.
    spinner: bool,
    /// Whether the last commit attempt was input-cap rejected (danger border).
    cap_error: bool,
}

impl Default for DataRow {
    fn default() -> Self {
        Self {
            mode: FieldMode::Idle,
            text: String::new(),
            committed: String::new(),
            latest_req: 0,
            awaiting: false,
            spinner: false,
            cap_error: false,
        }
    }
}

/// Inputs to the reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataRowEvent {
    /// The active selection changed. `single == false` → multi-cell → the field disables.
    SelectionChanged { single: bool },
    /// The worker replied to a `GetCellContent` request.
    ContentFetched { req_id: u64, raw: String },
    /// The user typed / edited the field (enters `Editing`).
    Edited { text: String },
    /// Enter: commit the pending edit and move the active cell down.
    Commit,
    /// The grid asked the field to commit before applying a click-away selection change
    /// (`components/grid.md`: grid emits this before `SelectionChanged`).
    EditCommitRequested,
    /// Escape: revert to the last-fetched content and hand focus back to the grid.
    Escape,
    /// The 250 ms content-fetch timer fired (arm the spinner if still awaiting).
    FetchTimeout { req_id: u64 },
}

/// Side effects the window performs after a reduce step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataRowEffect {
    /// Send `GetCellContent { req_id }` to the worker.
    Fetch { req_id: u64 },
    /// Send `SetCellInput { input }` to the worker.
    Commit { input: String },
    /// Move the active cell (Enter → down).
    MoveActive(Motion),
    /// Return focus to the grid.
    FocusGrid,
    /// Show the input-cap rejection error (danger border + message).
    ShowCapError,
    /// Show or hide the content-fetch spinner.
    SetSpinner(bool),
}

impl DataRow {
    pub fn mode(&self) -> FieldMode {
        self.mode
    }

    /// The current field text (shown / edited).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the content-fetch spinner is shown.
    pub fn spinner(&self) -> bool {
        self.spinner
    }

    /// Whether the field is in the cap-rejected error state.
    pub fn cap_error(&self) -> bool {
        self.cap_error
    }

    /// Clears the spinner and returns the `SetSpinner(false)` effect iff it was showing.
    fn hide_spinner(&mut self, effects: &mut Vec<DataRowEffect>) {
        if self.spinner {
            self.spinner = false;
            effects.push(DataRowEffect::SetSpinner(false));
        }
    }

    /// Begins a fresh content fetch for the newly-selected single cell.
    fn begin_fetch(&mut self, effects: &mut Vec<DataRowEffect>) {
        self.hide_spinner(effects);
        self.latest_req += 1;
        self.mode = FieldMode::Idle;
        self.text.clear();
        self.awaiting = true;
        self.cap_error = false;
        effects.push(DataRowEffect::Fetch {
            req_id: self.latest_req,
        });
    }

    /// Applies `event`, mutating the state and returning the effects to perform.
    pub fn reduce(&mut self, event: DataRowEvent) -> Vec<DataRowEffect> {
        let mut effects = Vec::new();
        match event {
            DataRowEvent::SelectionChanged { single } => {
                if single {
                    // Protocol invariant (the window must uphold it): a click-away while
                    // editing sends `EditCommitRequested` *before* `SelectionChanged`
                    // (`components/grid.md`), so the field is never `Editing` here. If it
                    // were, `begin_fetch` would silently discard the pending edit — assert
                    // in debug builds so a protocol violation surfaces in tests, not as
                    // quiet data loss.
                    debug_assert_ne!(
                        self.mode,
                        FieldMode::Editing,
                        "SelectionChanged arrived while Editing — the window must send \
                         EditCommitRequested first (grid.md protocol)"
                    );
                    self.begin_fetch(&mut effects);
                } else {
                    // Multi-cell: disable + empty, no fetch.
                    self.hide_spinner(&mut effects);
                    self.mode = FieldMode::Disabled;
                    self.text.clear();
                    self.committed.clear();
                    self.awaiting = false;
                    self.cap_error = false;
                }
            }
            DataRowEvent::ContentFetched { req_id, raw } => {
                // Only the latest request's reply, and only while still Idle (the user
                // hasn't started editing), may populate the field. Everything else is stale.
                if req_id == self.latest_req && self.mode == FieldMode::Idle {
                    self.text = raw.clone();
                    self.committed = raw;
                    self.awaiting = false;
                    self.hide_spinner(&mut effects);
                }
            }
            DataRowEvent::FetchTimeout { req_id } => {
                if req_id == self.latest_req
                    && self.awaiting
                    && self.mode == FieldMode::Idle
                    && !self.spinner
                {
                    self.spinner = true;
                    effects.push(DataRowEffect::SetSpinner(true));
                }
            }
            DataRowEvent::Edited { text } => {
                // Editing supersedes any pending fetch: drop late replies and the spinner.
                self.mode = FieldMode::Editing;
                self.text = text;
                self.awaiting = false;
                self.cap_error = false;
                self.hide_spinner(&mut effects);
            }
            DataRowEvent::Commit => {
                if self.mode == FieldMode::Editing {
                    match validate_input(&self.text) {
                        Ok(()) => {
                            let input = self.text.clone();
                            self.committed = input.clone();
                            self.mode = FieldMode::Idle;
                            self.cap_error = false;
                            effects.push(DataRowEffect::Commit { input });
                            // Enter moves the active cell down; the resulting
                            // SelectionChanged will fetch the new cell's content.
                            effects.push(DataRowEffect::MoveActive(Motion::Move(Direction::Down)));
                            effects.push(DataRowEffect::FocusGrid);
                        }
                        Err(_) => {
                            // Cap-rejected: stay Editing, flag the error, don't commit.
                            self.cap_error = true;
                            effects.push(DataRowEffect::ShowCapError);
                        }
                    }
                }
            }
            DataRowEvent::EditCommitRequested => {
                // Click-away while editing: commit the pending edit (Excel behaviour) so
                // the grid's selection change proceeds. The grid performs the move itself,
                // so no MoveActive here. A cap-rejected edit blocks the commit (the caller
                // keeps the field editing and cancels the pending selection change).
                if self.mode == FieldMode::Editing {
                    match validate_input(&self.text) {
                        Ok(()) => {
                            let input = self.text.clone();
                            self.committed = input.clone();
                            self.mode = FieldMode::Idle;
                            self.cap_error = false;
                            effects.push(DataRowEffect::Commit { input });
                        }
                        Err(_) => {
                            self.cap_error = true;
                            effects.push(DataRowEffect::ShowCapError);
                        }
                    }
                }
            }
            DataRowEvent::Escape => {
                if self.mode == FieldMode::Editing {
                    self.text = self.committed.clone();
                    self.mode = FieldMode::Idle;
                    self.cap_error = false;
                    effects.push(DataRowEffect::FocusGrid);
                }
            }
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DataRow that has fetched `content` for the current single-cell selection.
    fn idle_with(content: &str) -> DataRow {
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        let req = d.latest_req;
        d.reduce(DataRowEvent::ContentFetched {
            req_id: req,
            raw: content.to_string(),
        });
        d
    }

    #[test]
    fn selection_single_fetches() {
        let mut d = DataRow::default();
        let effects = d.reduce(DataRowEvent::SelectionChanged { single: true });
        assert_eq!(effects, vec![DataRowEffect::Fetch { req_id: 1 }]);
        assert_eq!(d.mode(), FieldMode::Idle);
    }

    #[test]
    fn multiselect_disables() {
        let mut d = idle_with("=A1");
        let effects = d.reduce(DataRowEvent::SelectionChanged { single: false });
        assert_eq!(d.mode(), FieldMode::Disabled);
        assert_eq!(d.text(), "");
        // No fetch is issued for a multi-cell selection.
        assert!(!effects
            .iter()
            .any(|e| matches!(e, DataRowEffect::Fetch { .. })));
    }

    #[test]
    fn fresh_content_reply_shown() {
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        d.reduce(DataRowEvent::ContentFetched {
            req_id: 1,
            raw: "=SUM(A1:A5)".into(),
        });
        assert_eq!(d.text(), "=SUM(A1:A5)");
        assert_eq!(d.mode(), FieldMode::Idle);
    }

    #[test]
    fn stale_content_reply_dropped() {
        let mut d = DataRow::default();
        // req 1, then req 2 — the selection moved on before the first reply arrived.
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        // The req-1 reply is now stale and must not populate the field.
        d.reduce(DataRowEvent::ContentFetched {
            req_id: 1,
            raw: "stale".into(),
        });
        assert_eq!(d.text(), "");
        // The current req-2 reply is applied.
        d.reduce(DataRowEvent::ContentFetched {
            req_id: 2,
            raw: "fresh".into(),
        });
        assert_eq!(d.text(), "fresh");
    }

    #[test]
    fn edit_enters_editing() {
        let mut d = idle_with("42");
        d.reduce(DataRowEvent::Edited { text: "99".into() });
        assert_eq!(d.mode(), FieldMode::Editing);
        assert_eq!(d.text(), "99");
    }

    #[test]
    fn escape_reverts_to_committed() {
        let mut d = idle_with("42");
        d.reduce(DataRowEvent::Edited { text: "999".into() });
        let effects = d.reduce(DataRowEvent::Escape);
        assert_eq!(d.text(), "42", "Escape reverts to the last-fetched content");
        assert_eq!(d.mode(), FieldMode::Idle);
        assert_eq!(effects, vec![DataRowEffect::FocusGrid]);
    }

    #[test]
    fn cap_reject_keeps_editing() {
        let mut d = idle_with("");
        // A formula one over the length cap.
        let huge = format!("={}", "1".repeat(crate::input_cap::MAX_INPUT_LEN));
        d.reduce(DataRowEvent::Edited { text: huge });
        let effects = d.reduce(DataRowEvent::Commit);
        assert_eq!(effects, vec![DataRowEffect::ShowCapError]);
        assert_eq!(
            d.mode(),
            FieldMode::Editing,
            "stays editing after cap reject"
        );
        assert!(d.cap_error());
        // No SetCellInput was emitted.
        assert!(!effects
            .iter()
            .any(|e| matches!(e, DataRowEffect::Commit { .. })));
    }

    #[test]
    fn commit_valid_moves_down() {
        let mut d = idle_with("");
        d.reduce(DataRowEvent::Edited {
            text: "=1+1".into(),
        });
        let effects = d.reduce(DataRowEvent::Commit);
        assert_eq!(
            effects,
            vec![
                DataRowEffect::Commit {
                    input: "=1+1".into()
                },
                DataRowEffect::MoveActive(Motion::Move(Direction::Down)),
                DataRowEffect::FocusGrid,
            ]
        );
        assert_eq!(d.mode(), FieldMode::Idle);
    }

    #[test]
    fn edit_commit_on_cell_click() {
        // Editing, then the grid requests a commit before a click-away selection change.
        let mut d = idle_with("old");
        d.reduce(DataRowEvent::Edited { text: "=A1".into() });
        let effects = d.reduce(DataRowEvent::EditCommitRequested);
        assert_eq!(
            effects,
            vec![DataRowEffect::Commit {
                input: "=A1".into()
            }]
        );
        assert_eq!(d.mode(), FieldMode::Idle);
        // No selection move here — the grid performs the click's move itself.
        assert!(!effects
            .iter()
            .any(|e| matches!(e, DataRowEffect::MoveActive(_))));
    }

    #[test]
    fn edit_commit_request_when_not_editing_is_noop() {
        let mut d = idle_with("x");
        let effects = d.reduce(DataRowEvent::EditCommitRequested);
        assert!(effects.is_empty());
    }

    #[test]
    fn fetch_timeout_shows_spinner() {
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true }); // req 1, awaiting
        let effects = d.reduce(DataRowEvent::FetchTimeout { req_id: 1 });
        assert_eq!(effects, vec![DataRowEffect::SetSpinner(true)]);
        assert!(d.spinner());
        // A stale timeout (old req id) does nothing.
        let mut d2 = DataRow::default();
        d2.reduce(DataRowEvent::SelectionChanged { single: true }); // req 1
        d2.reduce(DataRowEvent::SelectionChanged { single: true }); // req 2
        let e = d2.reduce(DataRowEvent::FetchTimeout { req_id: 1 });
        assert!(e.is_empty());
        assert!(!d2.spinner());
    }

    #[test]
    fn spinner_hidden_when_reply_arrives() {
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        d.reduce(DataRowEvent::FetchTimeout { req_id: 1 });
        assert!(d.spinner());
        let effects = d.reduce(DataRowEvent::ContentFetched {
            req_id: 1,
            raw: "v".into(),
        });
        assert!(!d.spinner());
        assert!(effects.contains(&DataRowEffect::SetSpinner(false)));
    }

    #[test]
    fn reply_before_timeout_never_flashes_spinner() {
        // The §3.3 no-flash guarantee: the normal instant-reply case must never show the
        // spinner. Reply arrives first (clearing `awaiting`), then the 250 ms timer fires
        // late — the timeout must no-op because `awaiting` is already false.
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true }); // req 1, awaiting
        d.reduce(DataRowEvent::ContentFetched {
            req_id: 1,
            raw: "42".into(),
        });
        assert!(!d.spinner());
        let effects = d.reduce(DataRowEvent::FetchTimeout { req_id: 1 });
        assert!(effects.is_empty(), "a timeout after the reply must no-op");
        assert!(
            !d.spinner(),
            "spinner must never flash on the instant-reply path"
        );
    }

    #[test]
    fn editing_drops_late_reply_and_spinner() {
        // Spinner up, user starts editing, then the late reply arrives — it must not clobber
        // the pending edit, and the spinner must be gone.
        let mut d = DataRow::default();
        d.reduce(DataRowEvent::SelectionChanged { single: true });
        d.reduce(DataRowEvent::FetchTimeout { req_id: 1 });
        d.reduce(DataRowEvent::Edited {
            text: "typing".into(),
        });
        assert!(!d.spinner());
        d.reduce(DataRowEvent::ContentFetched {
            req_id: 1,
            raw: "late".into(),
        });
        assert_eq!(
            d.text(),
            "typing",
            "a late reply must not overwrite the edit"
        );
        assert_eq!(d.mode(), FieldMode::Editing);
    }
}
