//! [`WorkbookWindow`] — the root entity of a document window (`components/app_shell.md
//! §Structure`, `functional_spec.md §2.3, §5, §6`).
//!
//! **Phase-10 scope (shell).** This owns the document window's *lifecycle* — the worker
//! ([`DocumentClient`]), the loading / degraded / dirty state, the window title + macOS
//! edited dot, the modal dialogs, and the save / close flows. The grid + chrome composition
//! and their event routing land in **Phase 11**, which replaces the placeholder content body
//! here; the shell folds only the lifecycle-relevant worker events (Loaded / LoadFailed /
//! Saved / SaveFailed / Published / WorkerDegraded).
//!
//! The lifecycle *decisions* are the pure [`super::lifecycle`] helpers (title, dirty, save
//! target, `.xlsx` enforcement); this module performs them against real windows + panels +
//! dialogs.

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, Context, FocusHandle, Focusable, PathPromptOptions,
    Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::spinner::Spinner;

use freecell_engine::{Command, DocumentClient, DocumentSource, WorkerEvent, WorkerEventReceiver};

use super::lifecycle::{self, SaveTarget};
use super::registry::WindowKey;
use super::{
    CloseWindow, FreeCellApp, Redo, Save, SaveAs, ToggleBold, ToggleItalic, ToggleUnderline, Undo,
};

// --- Look constants (functional-POC greys, matching the chrome / grid) ---------------------
const WINDOW_BG: u32 = 0xFFFFFF;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;
const CARD_BG: u32 = 0xFFFFFF;
const HAIRLINE: u32 = 0xD9D9D9;
const DANGER: u32 = 0xDC2626;
const DEGRADED_BG: u32 = 0xFEF2F2;

/// The one-at-a-time modal a document window can show (`components/app_shell.md §Dialogs`).
#[derive(Debug, Clone)]
enum ActiveModal {
    /// Unsaved-changes confirm: Save / Don't Save / Cancel (`functional_spec.md §2.3`).
    UnsavedChanges,
    /// An error dialog (open / save failure). `close_window_on_dismiss` closes the window
    /// after the user acknowledges a load failure (`functional_spec.md §5.1`), but not a save
    /// failure (`§5.2`: the document stays dirty and open).
    Error {
        title: String,
        detail: String,
        close_window_on_dismiss: bool,
    },
    /// The About dialog.
    About,
}

/// A document window's shell state + lifecycle.
pub struct WorkbookWindow {
    /// The registry key that identifies this window app-side.
    key: WindowKey,
    /// The per-window engine worker handle (`components/engine_worker.md`).
    client: DocumentClient,
    focus_handle: FocusHandle,

    /// The file's canonical path, or `None` for an unsaved (`Untitled`) workbook.
    path: Option<PathBuf>,
    /// `Some(file_name)` renders the "Opening <name>…" loading state (`functional_spec.md
    /// §5.1`); `None` once the document has loaded.
    loading: Option<String>,
    /// A degraded-worker reason (`functional_spec.md §6`): the window keeps serving the last
    /// good state + Save As but refuses edits.
    degraded: Option<String>,

    /// The op index the file on disk currently reflects; the dirty flag is
    /// `committed_ops > last_saved_ops` (`architecture.md §2`).
    last_saved_ops: u64,
    /// The current dirty flag (mirrored into the registry + title).
    dirty: bool,

    /// The active modal, if any (owned here — one at a time).
    modal: Option<ActiveModal>,
    /// After the pending save succeeds, close the window (the Save-then-close / quit path).
    close_after_save: bool,
    /// The path a pending `Save`/`Save As` is writing to (adopted as `self.path` on success).
    pending_save_path: Option<PathBuf>,
    /// The in-flight save's request id (matched against `Saved`/`SaveFailed`).
    pending_save_req: Option<u64>,
    next_req_id: u64,

    /// Keeps the worker→UI event task alive for the window's lifetime.
    _event_task: gpui::Task<()>,
}

impl WorkbookWindow {
    /// Builds a document window over a freshly spawned worker for `source`. The window opens
    /// immediately in the loading state; the worker emits `Loaded` / `LoadFailed` as its first
    /// event. `path` is the canonical path for an open (so dedupe + the title are correct
    /// before load completes); `None` for a new workbook.
    pub fn new(
        key: WindowKey,
        source: DocumentSource,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let loading = match &source {
            DocumentSource::NewWorkbook => None,
            DocumentSource::OpenFile(p) => Some(lifecycle::document_name(Some(p))),
        };
        let (client, receiver) = DocumentClient::spawn(source);
        let event_task = Self::spawn_event_loop(receiver, window, cx);
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        window.set_window_title(&lifecycle::window_title(
            &lifecycle::document_name(path.as_deref()),
            false,
            title_uses_suffix(),
        ));

        Self {
            key,
            client,
            focus_handle,
            path,
            loading,
            degraded: None,
            last_saved_ops: 0,
            dirty: false,
            modal: None,
            close_after_save: false,
            pending_save_path: None,
            pending_save_req: None,
            next_req_id: 0,
            _event_task: event_task,
        }
    }

    /// The worker→UI event loop: awaits each [`WorkerEvent`] and folds it into the window.
    fn spawn_event_loop(
        receiver: WorkerEventReceiver,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        this.on_worker_event(event, window, cx)
                    })
                    .is_ok();
                if !alive {
                    break; // the window is gone
                }
            }
        })
    }

    /// Folds a lifecycle-relevant worker event (`components/engine_worker.md`). Grid/chrome
    /// events (Published viewport paint, CellContent, EvalStarted/Finished, StyleCacheUpdated,
    /// SheetsChanged, EditRejected) are Phase-11 concerns and ignored here.
    fn on_worker_event(&mut self, event: WorkerEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            WorkerEvent::Loaded { .. } => {
                self.loading = None;
                self.refresh_dirty(window, cx);
                // The document finished loading → the welcome window (if still up) can close.
                FreeCellApp::note_window_loaded(self.key, cx);
                cx.notify();
            }
            WorkerEvent::LoadFailed { error } => {
                self.loading = None;
                self.modal = Some(ActiveModal::Error {
                    title: "Couldn't open the workbook".into(),
                    detail: error.to_string(),
                    close_window_on_dismiss: true,
                });
                cx.notify();
            }
            WorkerEvent::Published => {
                self.refresh_dirty(window, cx);
            }
            WorkerEvent::Saved { req_id, ops_seen } if self.pending_save_req == Some(req_id) => {
                self.pending_save_req = None;
                self.last_saved_ops = ops_seen;
                if let Some(path) = self.pending_save_path.take() {
                    // Canonicalize the adopted path (best-effort — the file exists, we just
                    // wrote it) so a later open of the same file dedupes against it: `Open…`
                    // canonicalizes before `resolve_open`, so a raw/relative Save-As path here
                    // would miss the dedupe and let two windows edit one file
                    // (`functional_spec.md §5.1`).
                    let path = path.canonicalize().unwrap_or(path);
                    self.path = Some(path.clone());
                    FreeCellApp::note_window_path(self.key, path, cx);
                }
                self.refresh_dirty(window, cx);
                if self.close_after_save {
                    self.close_after_save = false;
                    window.remove_window();
                }
                cx.notify();
            }
            WorkerEvent::SaveFailed { req_id, error } if self.pending_save_req == Some(req_id) => {
                self.pending_save_req = None;
                self.pending_save_path = None;
                // A failed save aborts any close/quit follow-up (§5.2: stay dirty + open).
                self.close_after_save = false;
                FreeCellApp::note_prompt_cancelled(cx);
                self.modal = Some(ActiveModal::Error {
                    title: "Couldn't save the workbook".into(),
                    detail: error.to_string(),
                    close_window_on_dismiss: false,
                });
                cx.notify();
            }
            WorkerEvent::WorkerDegraded { reason } => {
                self.degraded = Some(reason);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Recomputes the dirty flag from op accounting and reflects it in the registry, the
    /// window title, and the macOS edited dot.
    fn refresh_dirty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = lifecycle::is_dirty(self.client.committed_ops(), self.last_saved_ops);
        self.dirty = dirty;
        FreeCellApp::note_window_dirty(self.key, dirty, cx);
        window.set_window_edited(dirty);
        window.set_window_title(&lifecycle::window_title(
            &lifecycle::document_name(self.path.as_deref()),
            dirty,
            title_uses_suffix(),
        ));
    }

    // ---- Close / quit prompts -------------------------------------------------------------

    /// The traffic-light / OS close was requested (`Window::on_window_should_close`). Returns
    /// whether the close may proceed: clean → `true`; dirty → `false` after showing the
    /// unsaved-changes modal (`functional_spec.md §2.3`). Uses the good close-interception API
    /// present at the pinned rev (no data-loss papercut).
    pub fn on_titlebar_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.dirty {
            self.modal = Some(ActiveModal::UnsavedChanges);
            cx.notify();
            false
        } else {
            true
        }
    }

    /// The `Close Window` action / `Cmd+W`: prompt if dirty, else close.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            self.modal = Some(ActiveModal::UnsavedChanges);
            cx.notify();
        } else {
            window.remove_window();
        }
    }

    /// App-driven: show the unsaved-changes modal (used by the quit flow front-to-back
    /// prompting).
    pub fn show_unsaved_modal(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(ActiveModal::UnsavedChanges);
        cx.notify();
    }

    /// Shows the About dialog on this window.
    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(ActiveModal::About);
        cx.notify();
    }

    /// Shows an app-level error dialog on this window (e.g. an `Open…` that failed to resolve
    /// its path, reported on the frontmost document window). Dismiss keeps the window — this is
    /// not *this* document's load failure, so `close_window_on_dismiss` is false.
    pub fn show_error_dialog(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.modal = Some(ActiveModal::Error {
            title: title.into(),
            detail: detail.into(),
            close_window_on_dismiss: false,
        });
        cx.notify();
    }

    /// Unsaved-changes → **Save**: run the save flow, then close on success.
    fn modal_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        self.close_after_save = true;
        self.save(false, window, cx);
    }

    /// Unsaved-changes → **Don't Save**: discard and close.
    fn modal_dont_save(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.modal = None;
        window.remove_window();
    }

    /// Unsaved-changes → **Cancel** (or Error/About dismiss): keep the window; abort a quit if
    /// one is in progress.
    fn dismiss_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let modal = self.modal.take();
        cx.notify();
        if let Some(ActiveModal::Error {
            close_window_on_dismiss: true,
            ..
        }) = modal
        {
            window.remove_window();
            return;
        }
        if matches!(modal, Some(ActiveModal::UnsavedChanges)) {
            // A cancel during the quit flow aborts the quit (`functional_spec.md §2.3`).
            FreeCellApp::note_prompt_cancelled(cx);
        }
    }

    // ---- Save flow (silent strip — no fidelity warning, functional_spec §5.2) --------------

    /// Runs the save flow (`components/app_shell.md §Save flow`). `Save` on an untitled
    /// document, or `Save As`, opens the native save panel; a titled `Save` writes straight
    /// to its path. There is **no fidelity warning** — a successful write silently drops
    /// anything IronCalc can't model (`functional_spec.md §5.2`).
    pub fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        match lifecycle::resolve_save_target(self.path.as_deref(), save_as) {
            SaveTarget::Path(path) => self.send_save(path, cx),
            SaveTarget::Prompt { suggested_name } => {
                self.prompt_then_save(suggested_name, window, cx)
            }
        }
    }

    /// Opens the native save panel, then saves to the chosen path (with `.xlsx` enforced). A
    /// cancelled panel aborts the follow-up close/quit.
    fn prompt_then_save(
        &mut self,
        suggested_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self
            .path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            let picked = receiver.await.ok().and_then(|r| r.ok()).flatten();
            this.update_in(cx, |this, _window, cx| match picked {
                Some(path) => this.send_save(lifecycle::with_xlsx_extension(path), cx),
                None => {
                    // Panel cancelled → abort a pending close/quit follow-up.
                    this.close_after_save = false;
                    FreeCellApp::note_prompt_cancelled(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sends the atomic `Save` command (`functional_spec.md §5.2` — the Phase-3 temp+rename
    /// write); `Saved`/`SaveFailed` drive the rest.
    fn send_save(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let req_id = self.next_req_id;
        self.next_req_id += 1;
        self.pending_save_req = Some(req_id);
        self.pending_save_path = Some(path.clone());
        self.client.send(Command::Save { path, req_id });
        cx.notify();
    }

    // ---- Read accessors (tests) -----------------------------------------------------------

    /// The window's registry key.
    pub fn key(&self) -> WindowKey {
        self.key
    }
    /// Whether the document is dirty.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    /// The document's path, if saved.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
    /// Whether the window is still loading a file.
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }
    /// Whether an unsaved-changes modal is showing.
    pub fn has_unsaved_modal(&self) -> bool {
        matches!(self.modal, Some(ActiveModal::UnsavedChanges))
    }
    /// Whether an error modal is showing.
    pub fn has_error_modal(&self) -> bool {
        matches!(self.modal, Some(ActiveModal::Error { .. }))
    }
    /// The window title as it would be set now.
    pub fn title(&self) -> String {
        lifecycle::window_title(
            &lifecycle::document_name(self.path.as_deref()),
            self.dirty,
            title_uses_suffix(),
        )
    }

    /// Test seam: force the dirty flag (mirrors the registry) without a worker round-trip.
    #[cfg(test)]
    pub(crate) fn set_dirty_for_test(&mut self, dirty: bool, cx: &mut Context<Self>) {
        self.dirty = dirty;
        FreeCellApp::note_window_dirty(self.key, dirty, cx);
    }

    /// Test seam: dismiss the active modal (the Cancel / OK path).
    #[cfg(test)]
    pub(crate) fn dismiss_modal_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
    }

    /// Test seam: fold a synthesized [`WorkerEvent`] directly, with no live-worker
    /// `run_until_parked`. The lifecycle *folding* logic (path adoption + close-on-save on
    /// `Saved`, the quit-abort + error dialog on `SaveFailed`, the close-on-dismiss dialog on
    /// `LoadFailed`) IS deterministically testable this way — only the end-to-end flows that
    /// depend on the real worker *emitting* an event aren't (DECISIONS_TO_REVIEW Phase 10).
    #[cfg(test)]
    pub(crate) fn inject_worker_event_for_test(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_worker_event(event, window, cx);
    }

    /// Test seam: arm a pending save (as if [`send_save`] had run) without touching the worker,
    /// so a synthesized `Saved`/`SaveFailed` with `req_id` matches.
    ///
    /// [`send_save`]: Self::send_save
    #[cfg(test)]
    pub(crate) fn arm_pending_save_for_test(
        &mut self,
        path: PathBuf,
        req_id: u64,
        close_after: bool,
    ) {
        self.pending_save_req = Some(req_id);
        self.pending_save_path = Some(path);
        self.close_after_save = close_after;
        self.next_req_id = self.next_req_id.max(req_id + 1);
    }

    /// Test seam: whether a successful save is pending a follow-up window close.
    #[cfg(test)]
    pub(crate) fn will_close_after_save(&self) -> bool {
        self.close_after_save
    }

    /// Test seam: force the loading state (a `NewWorkbook` window constructs with `loading =
    /// None`, so this lets a test put it in the "Opening …" state an `OpenFile` window starts
    /// in, to prove `LoadFailed` actually clears it).
    #[cfg(test)]
    pub(crate) fn set_loading_for_test(&mut self, name: Option<String>) {
        self.loading = name;
    }

    /// Test seam: for an active error modal, whether dismissing it closes the window
    /// (`true` for a load failure, `false` for a save failure / app-level error); `None` if no
    /// error modal is showing.
    #[cfg(test)]
    pub(crate) fn error_modal_closes_window_on_dismiss(&self) -> Option<bool> {
        match &self.modal {
            Some(ActiveModal::Error {
                close_window_on_dismiss,
                ..
            }) => Some(*close_window_on_dismiss),
            _ => None,
        }
    }
}

impl Focusable for WorkbookWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkbookWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("workbook-window")
            .track_focus(&self.focus_handle)
            .key_context("WorkbookWindow")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(WINDOW_BG))
            // Window-scoped action handlers. Their presence here (and absence on the Welcome
            // window) is what enables/disables the corresponding File/Edit menu items on macOS
            // (Save / Save As / Close Window / Undo / Redo). About is handled globally
            // (`app.rs`), so it is deliberately *not* registered here.
            .on_action(cx.listener(|this, _: &Save, window, cx| this.save(false, window, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save(true, window, cx)))
            .on_action(
                cx.listener(|this, _: &CloseWindow, window, cx| this.request_close(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Undo, _window, _cx| {
                this.client.send(Command::Undo);
            }))
            .on_action(cx.listener(|this, _: &Redo, _window, _cx| {
                this.client.send(Command::Redo);
            }))
            // Bold/Italic/Underline are keyboard-only no-op placeholders (cmd/ctrl-b/i/u): the
            // real handlers need the grid selection and land in Phase 11. There is no Format
            // menu yet, so nothing else references them.
            .on_action(cx.listener(|_this, _: &ToggleBold, _window, _cx| {}))
            .on_action(cx.listener(|_this, _: &ToggleItalic, _window, _cx| {}))
            .on_action(cx.listener(|_this, _: &ToggleUnderline, _window, _cx| {}))
            .children(
                self.degraded
                    .clone()
                    .map(|reason| self.render_degraded_bar(&reason, cx)),
            )
            .child(self.render_body(cx))
            .children(self.render_modal(cx))
    }
}

impl WorkbookWindow {
    /// The window body: the loading overlay while opening, else the placeholder content Phase
    /// 11 replaces with the composed grid + chrome.
    fn render_body(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let center = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .w_full();
        match &self.loading {
            Some(name) => center.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Spinner::new())
                    .child(
                        div()
                            .text_color(rgb(MUTED_TEXT))
                            .child(format!("Opening {name}…")),
                    ),
            ),
            None => center.child(
                div()
                    .text_color(rgb(MUTED_TEXT))
                    .child("Document window — grid + chrome are wired in Phase 11."),
            ),
        }
    }

    /// Renders the active modal over a dim backdrop (`components/app_shell.md §Dialogs` — a
    /// small enum + handler, no dialog framework).
    fn render_modal(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let modal = self.modal.as_ref()?;
        let card = match modal {
            ActiveModal::UnsavedChanges => dialog_card(
                "Unsaved changes",
                "Do you want to save the changes to this workbook?",
                vec![
                    Button::new("dont-save")
                        .label("Don't Save")
                        .ghost()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.modal_dont_save(window, cx)
                        })),
                    Button::new("cancel")
                        .label("Cancel")
                        .ghost()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.dismiss_modal(window, cx)
                        })),
                    Button::new("save").label("Save").primary().on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.modal_save(window, cx)),
                    ),
                ],
            ),
            ActiveModal::Error { title, detail, .. } => dialog_card(
                title,
                detail,
                vec![Button::new("ok").label("OK").primary().on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.dismiss_modal(window, cx)),
                )],
            ),
            ActiveModal::About => dialog_card(
                "FreeCell",
                "A GPU-rendered, Excel-compatible spreadsheet.\nMVP proof of concept.",
                vec![Button::new("ok").label("OK").primary().on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.dismiss_modal(window, cx)),
                )],
            ),
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x000000).opacity(0.3))
                .child(card)
                .into_any_element(),
        )
    }
}

/// The primary modifier: the macOS edited dot is available at the pinned rev
/// (`Window::set_window_edited`), so the title `— Edited` suffix is only used on non-macOS
/// (`functional_spec.md §2.3`).
fn title_uses_suffix() -> bool {
    !cfg!(target_os = "macos")
}

/// A small modal card: title, body text, and a right-aligned button row.
fn dialog_card(title: &str, body: &str, buttons: Vec<Button>) -> gpui::AnyElement {
    let mut row = div().flex().justify_end().gap_2();
    for button in buttons {
        row = row.child(button);
    }
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .w(px(360.0))
        .bg(rgb(CARD_BG))
        .border_1()
        .border_color(rgb(HAIRLINE))
        .rounded_lg()
        .shadow_lg()
        .child(
            div()
                .text_size(px(15.0))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(MUTED_TEXT))
                .child(body.to_string()),
        )
        .child(row)
        .into_any_element()
}

impl WorkbookWindow {
    /// The non-dismissable degraded-worker bar (`functional_spec.md §6`): an explanatory line
    /// plus a real **Save As** button that writes the last good state (edits are refused).
    fn render_degraded_bar(&self, reason: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .bg(rgb(DEGRADED_BG))
            .border_b_1()
            .border_color(rgb(DANGER))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .text_color(rgb(DANGER))
                    .child(format!(
                        "This workbook hit an internal error and is read-only. \
                         Save As to keep your work. ({reason})"
                    )),
            )
            .child(Button::new("degraded-save-as").label("Save As…").on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.save(true, window, cx)),
            ))
            .into_any_element()
    }
}

/// The open panel's path-prompt options — files only (`.xlsx`). Note: gpui's
/// `PathPromptOptions` has no extension filter at the pinned rev, so the `.xlsx` restriction
/// is enforced after selection by the loader's typed `LoadError::NotXlsx` (DECISIONS_TO_REVIEW).
pub(super) fn open_panel_options() -> PathPromptOptions {
    PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Open".into()),
    }
}
