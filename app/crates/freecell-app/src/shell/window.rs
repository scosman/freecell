//! [`WorkbookWindow`] — the root entity of a document window (`components/app_shell.md
//! §Structure`, `functional_spec.md §2.3, §5, §6`).
//!
//! Owns the document window's *lifecycle* — the worker ([`DocumentClient`]), loading /
//! degraded / dirty state, the window title + macOS edited dot, the modal dialogs, and the
//! save / close flows — and, since **Phase 11**, the composed [`GridView`] + [`ChromeView`]
//! (the chrome hosts the grid as its body). The window routes every [`WorkerEvent`] to the
//! grid / chrome / lifecycle state, forwards [`GridEvent`]s to the worker + chrome, and drives
//! the chrome→grid coupling — all without ever leasing this entity from inside a sibling's
//! `update` (the sinks capture the *sibling* handles; cyclic follow-ups are `Window::defer`red).
//!
//! The lifecycle *decisions* are the pure [`super::lifecycle`] helpers (title, dirty, save
//! target, `.xlsx` enforcement, viewport overscan); this module performs them against real
//! windows + panels + dialogs, and mediates the grid/chrome/worker seams.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, rgb, AnyView, App, ClickEvent, Context, Entity, FocusHandle, Focusable,
    PathPromptOptions, WeakEntity, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};

use freecell_chart_model::{ChartColor, ChartId, ChartInsertKind};
use freecell_core::{limits, Rgb, SelectionModel, SheetId};
use freecell_engine::{
    Command, DataLabelToggles, DocumentClient, DocumentSource, EditRejectedReason, PasteError,
    SheetMeta, StyleAttr, WorkerEvent, WorkerEventReceiver,
};

use crate::chrome::{ChartPanel, ChartPanelSeries, ChromeGridRequest, ChromeGridSink, ChromeView};
use crate::grid::{GridDataSources, GridEvent, GridEventSink, GridView, RowOrCol};

use super::clipboard::ClipboardCoordinator;
use super::lifecycle::{self, SaveTarget};
use super::registry::WindowKey;
use super::titlebar;
use super::{
    CloseWindow, FreeCellApp, Redo, Save, SaveAs, ToggleBold, ToggleItalic, ToggleUnderline, Undo,
};

/// Shared, lock-free state the grid/chrome sinks read on the UI thread (they run from inside a
/// sibling entity's `update`, so they must not touch the `WorkbookWindow` entity). The window
/// writes these on sheet switch / load.
struct SinkShared {
    /// The active sheet — read by the grid's `ViewportChanged` / `ClearCells` routing to scope
    /// the worker command, and set by the window on a switch.
    active_sheet: Cell<SheetId>,
    /// The last *accepted* selection — restored on the grid if a click-away commit is blocked by
    /// a cap-rejected pending edit (`functional_spec.md §3.3`).
    last_selection: Cell<SelectionModel>,
    /// The range clipboard's UI state (`last_copy_text`), shared so the grid-sink copy/paste key
    /// events and the window's `CopyReady` fold both reach it (`components/clipboard.md`).
    clipboard: RefCell<ClipboardCoordinator>,
}

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
}

/// A document window's shell state + lifecycle.
pub struct WorkbookWindow {
    /// The registry key that identifies this window app-side.
    key: WindowKey,
    /// The per-window engine worker handle (`components/engine_worker.md`). Held behind `Rc` so
    /// the composed [`ChromeView`] can send commands + read styles through the same client
    /// (`Rc<dyn ChromeClient>`) the window uses.
    client: Rc<DocumentClient>,
    /// The composed grid + chrome (`ui_design.md §3`). The chrome hosts the grid as its body;
    /// the window renders the chrome and routes worker/grid/chrome events between them.
    grid: Entity<GridView>,
    chrome: Entity<ChromeView>,
    /// Shared state the grid/chrome sinks read (active sheet + last accepted selection).
    sink_shared: Rc<SinkShared>,
    /// The window's mirror of the worker's sheet list — for reconciling adds/deletes and
    /// deciding the active sheet on `SheetsChanged`.
    sheets: Vec<SheetMeta>,
    focus_handle: FocusHandle,

    /// The file's canonical path, or `None` for an unsaved (`Untitled`) workbook.
    path: Option<PathBuf>,
    /// The disk path this document was opened from (`None` for a new workbook), captured at
    /// construction and never changed by Save-As — the `.back` backup gate
    /// (`functional_spec.md §7.3`).
    opened_from: Option<PathBuf>,
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

    /// The [`ChartSnapshot`](freecell_engine::ChartSnapshot) version last installed into the grid
    /// (P9). The worker bumps the snapshot version on load and on each dirty re-resolve; the window
    /// re-installs only when this differs, so a scroll-only publish never rebuilds the charts.
    installed_chart_version: u64,
    /// The sheets that currently have charts installed — so a snapshot that drops a sheet's charts
    /// can clear them.
    installed_chart_sheets: Vec<SheetId>,
    /// The **authored** chart ids seen in the installed snapshot (P19). A newly-appeared authored id
    /// means the user just inserted a chart, so its edit panel auto-opens (the insert→shape flow,
    /// `ui_design §3.1`); loaded charts are never tracked here, so opening a file never auto-opens a
    /// panel.
    known_authored_charts: HashSet<ChartId>,

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
        Self::build(key, Rc::new(client), receiver, loading, path, window, cx)
    }

    /// Test-only constructor over a **worker-less** [`DocumentClient::detached`] client, so a
    /// `#[gpui::test]` can compose the real grid + chrome without spawning an OS-thread worker
    /// (which races gpui's deterministic `TestScheduler`). Behaviour is otherwise identical; tests
    /// drive folding by injecting `WorkerEvent`s.
    #[cfg(test)]
    pub(crate) fn new_detached_for_test(
        key: WindowKey,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (client, receiver) = DocumentClient::detached();
        Self::build(key, Rc::new(client), receiver, None, path, window, cx)
    }

    /// Shared construction for [`new`](Self::new) and the detached test constructor: wires the
    /// event task, builds + cross-links the grid + chrome, sets loading/title.
    fn build(
        key: WindowKey,
        client: Rc<DocumentClient>,
        receiver: WorkerEventReceiver,
        loading: Option<String>,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let event_task = Self::spawn_event_loop(receiver, window, cx);
        let focus_handle = cx.focus_handle();

        // Shared state the sinks read (they fire from inside a sibling entity's `update`, so they
        // never touch the `WorkbookWindow` entity — the active sheet + last selection ride here).
        let sink_shared = Rc::new(SinkShared {
            active_sheet: Cell::new(SheetId(0)),
            last_selection: Cell::new(SelectionModel::default()),
            clipboard: RefCell::new(ClipboardCoordinator::new()),
        });

        // The grid needs the chrome handle and vice-versa; resolve both after construction via
        // one-shot slots the sinks read.
        let grid_slot: Rc<OnceCell<WeakEntity<GridView>>> = Rc::new(OnceCell::new());
        let chrome_slot: Rc<OnceCell<WeakEntity<ChromeView>>> = Rc::new(OnceCell::new());

        // The grid renders from the client's shared read-surfaces (zero engine calls per frame).
        let sources = GridDataSources {
            publication: client.publication_swap(),
            caches: client.caches(),
        };
        let grid_sink = make_grid_sink(
            chrome_slot.clone(),
            grid_slot.clone(),
            client.clone(),
            sink_shared.clone(),
        );
        let grid = cx.new(|cx| GridView::new(sources, grid_sink, cx));

        let chrome_grid_sink = make_chrome_grid_sink(
            grid_slot.clone(),
            chrome_slot.clone(),
            client.clone(),
            sink_shared.clone(),
        );
        let client_dyn: Rc<dyn crate::chrome::ChromeClient> = client.clone();
        let chrome = cx.new(|cx| {
            ChromeView::new(
                client_dyn,
                chrome_grid_sink,
                SheetId(0),
                Vec::new(),
                window,
                cx,
            )
        });

        // Resolve the cross-references and host the grid inside the chrome (so the layout is
        // action-row → data-row → grid → tab-bar).
        let _ = grid_slot.set(grid.downgrade());
        let _ = chrome_slot.set(chrome.downgrade());
        let grid_view: AnyView = grid.clone().into();
        chrome.update(cx, |c, cx| c.set_grid_body(grid_view, cx));

        // Hand the grid the reused in-cell editor input the chrome owns, so it can render the
        // in-cell overlay (`components/edit_controller.md §4.4`).
        let in_cell_input = chrome.read(cx).in_cell_input();
        grid.update(cx, |g, cx| g.set_incell_input(in_cell_input, cx));

        // An `OpenFile` window shows the "Opening name…" overlay over the grid until `Loaded`.
        if let Some(name) = loading.clone() {
            grid.update(cx, |g, cx| g.set_loading(Some(name), cx));
        }

        focus_handle.focus(window, cx);
        window.set_window_title(&lifecycle::window_title(
            &lifecycle::document_name(path.as_deref()),
            false,
            title_uses_suffix(),
        ));

        Self {
            key,
            client,
            grid,
            chrome,
            sink_shared,
            sheets: Vec::new(),
            focus_handle,
            // The disk path this document was opened from (captured before any Save-As can
            // mutate `path`) — gates the one-time `.back` backup (`functional_spec.md §7.3`).
            opened_from: path.clone(),
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
            installed_chart_version: 0,
            installed_chart_sheets: Vec::new(),
            known_authored_charts: HashSet::new(),
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

    /// Installs the worker's live-bound charts into the grid's **ChartLayer** from the publication
    /// seam (P9, `charts/functional_spec.md §2`, `architecture.md §4.1`). The worker owns chart
    /// discovery + live binding; the window just reads the wait-free
    /// [`ChartSnapshot`](freecell_engine::ChartSnapshot) on `Loaded` / `Published` and installs it
    /// when its version changed — so an edit that re-resolves a chart repaints it, while a
    /// scroll-only publish (or an edit touching no chart) is a no-op. A chart-less / unsaved workbook
    /// publishes the empty (version 0) snapshot, so this never installs anything for it.
    ///
    /// The snapshot is grouped by anchor worksheet (multi-sheet, P10); charts are discovered lazily
    /// on a sheet's first paint (P11), so a version bump can carry newly-parsed charts as well as
    /// live re-resolves. Each per-sheet list is a **shared** `Arc<[ChartSpec]>` — installing it into
    /// the grid bumps a refcount, never copies the charts (P11 "off-screen free").
    fn sync_charts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.client.chart_snapshot();
        if snapshot.version == self.installed_chart_version {
            return; // charts unchanged since the last install
        }
        // Clear any sheet that no longer carries charts, then (re)install the current set.
        let present: Vec<SheetId> = snapshot.sheets.iter().map(|(sheet, _)| *sheet).collect();
        let dropped: Vec<SheetId> = self
            .installed_chart_sheets
            .iter()
            .filter(|s| !present.contains(s))
            .copied()
            .collect();
        self.grid.update(cx, |g, cx| {
            for sheet in dropped {
                g.set_sheet_charts(sheet, std::sync::Arc::from(Vec::new()), cx);
            }
            for (sheet, specs) in &snapshot.sheets {
                // The per-sheet `Arc<[ChartSpec]>` is **shared** with the worker's published snapshot
                // — this `clone` bumps a refcount, it does not copy the charts (P11 "off-screen free").
                g.set_sheet_charts(*sheet, specs.clone(), cx);
            }
        });
        self.installed_chart_sheets = present;
        self.installed_chart_version = snapshot.version;

        // Drive the chart edit panel off the fresh snapshot (P19/P20): auto-open a just-inserted
        // authored chart, or reconcile an already-open panel's shown state (or close it if its chart
        // is gone).
        self.refresh_chart_panel(window, cx);
    }

    /// Reconcile the right-docked chart **edit panel** with the current snapshot (P19 skeleton + P20
    /// chrome): a newly-appeared **authored** chart (the user just inserted one) auto-opens its panel
    /// — the insert→shape flow (`ui_design §3.1`); otherwise an already-open panel (authored **or**
    /// loaded — a loaded chart's panel opens on click, `ChartSelected`) is refreshed to the chart's
    /// current state (or closed if the chart was deleted). Loaded charts never auto-open (they aren't
    /// tracked), so opening a file never pops the panel. A same-chart reconcile does **not** re-seed
    /// the text inputs (only an id change does), so a live republish can't clobber an in-progress edit.
    fn refresh_chart_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.client.chart_snapshot();
        let authored_now: HashSet<ChartId> = snapshot
            .sheets
            .iter()
            .flat_map(|(_, specs)| specs.iter())
            .filter(|s| s.is_authored())
            .map(|s| s.id)
            .collect();
        let newly_inserted = authored_now
            .iter()
            .find(|id| !self.known_authored_charts.contains(id))
            .copied();
        self.known_authored_charts = authored_now;

        if let Some(id) = newly_inserted {
            self.open_chart_panel_for(id, window, cx);
            return;
        }
        // Reconcile an already-open panel.
        let Some(target) = self.chrome.read(cx).chart_panel_target() else {
            return;
        };
        match chart_panel_info(&self.client, target) {
            Some(panel) => self
                .chrome
                .update(cx, |c, cx| c.open_chart_panel(panel, window, cx)),
            None => self.chrome.update(cx, |c, cx| c.close_chart_panel(cx)),
        }
    }

    /// Open the edit panel for chart `id` + outline it in the grid (P19). Used on insert-auto-open (a
    /// user click already outlined the chart, so the grid select there is a harmless re-set).
    fn open_chart_panel_for(&mut self, id: ChartId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = chart_panel_info(&self.client, id) else {
            return;
        };
        self.grid
            .update(cx, |g, cx| g.set_selected_chart(Some(id), cx));
        self.chrome
            .update(cx, |c, cx| c.open_chart_panel(panel, window, cx));
    }

    /// Folds a worker event (`components/engine_worker.md`), routing each to the window's
    /// lifecycle state, the grid (repaint), and/or the chrome (data row, tabs, spinner).
    fn on_worker_event(&mut self, event: WorkerEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            WorkerEvent::Loaded { sheets } => {
                self.loading = None;
                self.grid.update(cx, |g, cx| g.set_loading(None, cx));
                self.reconcile_sheets(sheets, window, cx);
                self.refresh_dirty(window, cx);
                // Install this file's live-bound charts from the worker's publication seam (P9,
                // `charts/architecture.md §4.1`). The worker discovered + bound them before the
                // first publish; per-chart non-fatal, so a chart-less/broken-chart file loads as
                // before. Live re-resolves arrive on later `Published` events.
                self.sync_charts(window, cx);
                // The document finished loading → the welcome window (if still up) can close.
                FreeCellApp::note_window_loaded(self.key, cx);
                cx.notify();
            }
            WorkerEvent::LoadFailed { error } => {
                self.loading = None;
                // Clear the grid's "Opening…" overlay too (symmetry with the Loaded arm) so it is
                // not left spinning behind the error dialog's backdrop.
                self.grid.update(cx, |g, cx| g.set_loading(None, cx));
                self.modal = Some(ActiveModal::Error {
                    title: "Couldn't open the workbook".into(),
                    detail: error.to_string(),
                    close_window_on_dismiss: true,
                });
                cx.notify();
            }
            WorkerEvent::Published => {
                // A fresh generation is available — repaint the grid from the new publication
                // (the grid re-reads the `ArcSwap` each frame; `notify` schedules that frame).
                self.grid.update(cx, |_g, cx| cx.notify());
                // Install any live-bound chart changes that rode this publish (P9). Version-gated,
                // so a scroll-only publish is a no-op.
                self.sync_charts(window, cx);
                self.refresh_dirty(window, cx);
            }
            WorkerEvent::StyleCacheUpdated { sheet } => {
                // Styles/geometry changed — repaint the grid and refresh the action-row toggles
                // for the active sheet (`components/app_shell.md §Action row`). A resize's rebuild
                // lands here, so clear the grid's frozen resize preview (the committed geometry now
                // comes from the resident cache — `components/grid_structure.md §5.1`).
                self.grid.update(cx, |g, cx| {
                    g.clear_resize_preview(cx);
                    cx.notify();
                });
                if sheet == self.sink_shared.active_sheet.get() {
                    self.chrome.update(cx, |c, cx| c.refresh_active_style(cx));
                }
            }
            WorkerEvent::SheetsChanged { sheets } => {
                self.reconcile_sheets(sheets, window, cx);
                self.refresh_dirty(window, cx);
            }
            WorkerEvent::CellContent { .. }
            | WorkerEvent::EvalStarted
            | WorkerEvent::EvalFinished => {
                // Data-row content reply + evaluating-spinner drive live on the chrome.
                self.chrome
                    .update(cx, |c, cx| c.on_worker_event(event, window, cx));
            }
            WorkerEvent::EditRejected { reason } => self.on_edit_rejected(reason, window, cx),
            // Saved / SaveFailed match unconditionally then branch on the pending-save `req_id`
            // (a stale ack from a superseded save is ignored) — so the match stays exhaustive with
            // no catch-all, and a new `WorkerEvent` variant is a compile error that forces a
            // conscious routing decision (mirroring the worker's exhaustive command routing).
            WorkerEvent::Saved { req_id, ops_seen } => {
                if self.pending_save_req == Some(req_id) {
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
            }
            WorkerEvent::SaveFailed { req_id, error } => {
                if self.pending_save_req == Some(req_id) {
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
            }
            WorkerEvent::WorkerDegraded { reason } => {
                self.degraded = Some(reason);
                // Disable the action-bar's mutating controls (`functional_spec.md §6`).
                self.chrome.update(cx, |c, cx| c.set_degraded(true, cx));
                cx.notify();
            }
            WorkerEvent::CopyReady { tsv } => {
                // Write the copied TSV to the system clipboard + remember it (so our next paste
                // routes internally). `cx` derefs to `&mut App` for the clipboard write.
                self.sink_shared
                    .clipboard
                    .borrow_mut()
                    .on_copy_ready(tsv, cx);
            }
            WorkerEvent::Pasted { sheet, range } => {
                // Mirror the pasted rectangle into the selection (the pasted area becomes the new
                // selection, `functional_spec.md §2.2`) — only if it landed on the active sheet.
                if sheet == self.sink_shared.active_sheet.get() {
                    let sel = SelectionModel {
                        anchor: range.start,
                        active: range.end,
                    };
                    self.sink_shared.last_selection.set(sel);
                    self.grid.update(cx, |g, cx| g.set_selection(sel, cx));
                    self.chrome
                        .update(cx, |c, cx| c.on_selection_changed(sel, window, cx));
                }
            }
            WorkerEvent::PasteRejected { reason } => match reason {
                // Overflow is user-visible: a brief dialog, nothing pasted (`functional_spec.md §2.2`).
                PasteError::Overflow => {
                    if self.modal.is_none() {
                        self.modal = Some(ActiveModal::Error {
                            title: "Paste doesn't fit".into(),
                            detail: "The copied range would extend past the edge of the sheet. \
                                     Nothing was pasted."
                                .into(),
                            close_window_on_dismiss: false,
                        });
                        cx.notify();
                    }
                }
                // A missing/consumed slot (e.g. the second paste of a cut) is log-only.
                PasteError::NothingToPaste => {
                    tracing::debug!("paste: nothing to paste (empty or consumed clipboard slot)");
                }
            },
        }
    }

    /// Routes an `EditRejected` (`components/engine_worker.md`): a cap rejection surfaces on the
    /// data row (danger border); a caught engine panic / typed engine error surfaces a transient
    /// "couldn't be applied" dialog (the document is intact, `functional_spec.md §6`); an invalid
    /// sheet name (backstop — the chrome validates first) and a degraded refusal (the degraded
    /// bar already explains it) need no dialog.
    fn on_edit_rejected(
        &mut self,
        reason: EditRejectedReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &reason {
            EditRejectedReason::InputCap(_) => {
                self.chrome.update(cx, |c, cx| {
                    c.on_worker_event(WorkerEvent::EditRejected { reason }, window, cx)
                });
            }
            EditRejectedReason::EnginePanic | EditRejectedReason::Engine(_) => {
                if self.modal.is_none() {
                    let detail = match &reason {
                        EditRejectedReason::Engine(msg) => msg.clone(),
                        _ => "The workbook is unchanged. Please try again.".to_string(),
                    };
                    self.modal = Some(ActiveModal::Error {
                        title: "That change couldn't be applied".into(),
                        detail,
                        close_window_on_dismiss: false,
                    });
                    cx.notify();
                }
            }
            // The insert/delete merge guard (`functional_spec.md §5.3`): an OK-only dialog, nothing
            // changed.
            EditRejectedReason::MergedCells => {
                if self.modal.is_none() {
                    self.modal = Some(ActiveModal::Error {
                        title: "Merged cells not supported".into(),
                        detail: "This sheet contains merged cells (not yet supported); \
                                 inserting or deleting here would corrupt them."
                            .into(),
                        close_window_on_dismiss: false,
                    });
                    cx.notify();
                }
            }
            EditRejectedReason::InvalidSheetName(_) | EditRejectedReason::Degraded => {}
        }
    }

    /// Reconciles a worker sheet list (`Loaded` / `SheetsChanged`) into the tab bar and the
    /// active sheet: a newly-added sheet becomes active (`functional_spec.md §3.7`: `+` switches
    /// to it), a surviving active sheet stays, and a deleted active sheet falls back to the first
    /// remaining. The switch restores the grid's per-sheet scroll/selection and re-points the
    /// data row.
    fn reconcile_sheets(
        &mut self,
        new_sheets: Vec<SheetMeta>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prev_ids: std::collections::HashSet<SheetId> =
            self.sheets.iter().map(|s| s.id).collect();
        let current_active = self.sink_shared.active_sheet.get();
        let added = new_sheets
            .iter()
            .map(|s| s.id)
            .find(|id| !prev_ids.contains(id));
        let survives = new_sheets.iter().any(|s| s.id == current_active);
        let new_active = added
            .or_else(|| survives.then_some(current_active))
            .or_else(|| new_sheets.first().map(|s| s.id));

        self.sheets = new_sheets.clone();
        self.chrome.update(cx, |c, cx| {
            c.on_worker_event(
                WorkerEvent::SheetsChanged { sheets: new_sheets },
                window,
                cx,
            )
        });

        if let Some(new_active) = new_active {
            if new_active != current_active || added.is_some() {
                let chrome_weak = self.chrome.downgrade();
                switch_grid_to_sheet(
                    &self.grid,
                    Some(&chrome_weak),
                    &self.client,
                    &self.sink_shared,
                    new_active,
                    window,
                    cx,
                );
            }
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

    /// Unsaved-changes → **Cancel** (or Error dismiss): keep the window; abort a quit if
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
    /// write); `Saved`/`SaveFailed` drive the rest. Before the write, a disk-opened file gets
    /// a one-time `.back` backup of its original bytes (`§7.3`); a backup failure aborts the
    /// save with a dialog — data safety wins over convenience.
    fn send_save(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(backup) = lifecycle::backup_target(self.opened_from.as_deref(), &path) {
            if let Err(err) = std::fs::copy(&path, &backup) {
                self.abort_save_with_backup_error(err, cx);
                return;
            }
        }
        let req_id = self.next_req_id;
        self.next_req_id += 1;
        self.pending_save_req = Some(req_id);
        self.pending_save_path = Some(path.clone());
        self.client.send(Command::Save { path, req_id });
        cx.notify();
    }

    /// Aborts a save whose pre-write `.back` backup couldn't be created (`§7.3`): the write is
    /// never dispatched, any close/quit follow-up is cancelled, and a dialog explains it.
    fn abort_save_with_backup_error(&mut self, err: std::io::Error, cx: &mut Context<Self>) {
        self.close_after_save = false;
        FreeCellApp::note_prompt_cancelled(cx);
        self.modal = Some(ActiveModal::Error {
            title: "Couldn't create backup".into(),
            detail: format!("File not saved. The backup copy could not be written: {err}"),
            close_window_on_dismiss: false,
        });
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

    /// The text drawn in the macOS custom titlebar row (§7.1 / `ui_design.md §1`): the document
    /// name, **always** with the `— Edited` suffix when dirty. Unlike the native window title
    /// (which drops the suffix on macOS in favor of the traffic-light edited dot — see
    /// [`title_uses_suffix`]), the custom row shows the edited state textually, so the user sees
    /// it in the row we draw. `set_window_edited` still lights the dot too.
    fn titlebar_title(&self) -> String {
        titlebar_title_text(&lifecycle::document_name(self.path.as_deref()), self.dirty)
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

    /// Test seam: the window's active sheet (mirrored to the grid + chrome).
    #[cfg(test)]
    pub(crate) fn active_sheet_for_test(&self) -> SheetId {
        self.sink_shared.active_sheet.get()
    }

    /// Test seam: the composed grid entity.
    #[cfg(test)]
    pub(crate) fn grid_for_test(&self) -> Entity<GridView> {
        self.grid.clone()
    }

    /// Test seam: the window's worker client, so a test can publish a `ChartSnapshot` into the seam
    /// (via `DocumentClient::set_chart_snapshot`) and then drive `sync_charts` through an injected
    /// `Published` event.
    #[cfg(test)]
    pub(crate) fn client_for_test(&self) -> std::rc::Rc<DocumentClient> {
        self.client.clone()
    }

    /// Test seam: the composed chrome entity.
    #[cfg(test)]
    pub(crate) fn chrome_for_test(&self) -> Entity<ChromeView> {
        self.chrome.clone()
    }

    /// Test seam: drive a grid `SelectionChanged` through the window's *real* routing (the same
    /// [`route_selection_changed`] the grid's sink calls), so the commit-first + adopt/revert
    /// logic is exercised without a synthetic re-implementation.
    #[cfg(test)]
    pub(crate) fn route_selection_changed_for_test(
        &mut self,
        sel: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        route_selection_changed(
            &self.chrome,
            self.grid.downgrade(),
            &self.sink_shared,
            sel,
            window,
            cx,
        );
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
            // Bold/Italic/Underline (cmd/ctrl-b/i/u) toggle the character style over the grid
            // selection through the chrome — the same path as the action-row buttons (commit any
            // pending data-row edit first, then `SetStyleAttr`).
            .on_action(cx.listener(|this, _: &ToggleBold, window, cx| {
                this.toggle_style(StyleAttr::Bold, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleItalic, window, cx| {
                this.toggle_style(StyleAttr::Italic, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleUnderline, window, cx| {
                this.toggle_style(StyleAttr::Underline, window, cx)
            }))
            // macOS custom titlebar (§7.1): the very top row, drawn only when the master switch
            // is on (Linux omits it → server decorations, unaffected). Its native integration is
            // the on-device smoke gate.
            .children(
                titlebar::MACOS_TITLEBAR.then(|| titlebar::titlebar_row(self.titlebar_title())),
            )
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
    /// The window body: the composed chrome (action row → data row → grid → tab bar). The grid
    /// shows the "Opening name…" overlay itself while `loading`; the chrome hosts the grid as its
    /// flex-fill body. This wrapper is a flex column so the chrome (a `flex_1` child) stretches to
    /// the full body height — otherwise the chrome sizes to its content and the grid slot
    /// collapses to zero.
    fn render_body(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(self.chrome.clone())
    }

    /// Toggles a character style over the selection through the chrome (shared by the Cmd/Ctrl
    /// shortcuts and — via the chrome's own buttons — the action row).
    fn toggle_style(&mut self, attr: StyleAttr, window: &mut Window, cx: &mut Context<Self>) {
        self.chrome
            .update(cx, |c, cx| c.toggle_style(attr, window, cx));
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

/// The macOS custom-titlebar text (§7.1 / `ui_design.md §1`): the document `name` with the
/// `— Edited` suffix **always** applied when `dirty` — deliberately independent of
/// [`title_uses_suffix`]. The native window title drops the suffix on macOS (the traffic-light
/// dot carries dirtiness), but the row we draw shows the edited state textually. Pure so the
/// "always suffix" contract is directly unit-tested.
fn titlebar_title_text(name: &str, dirty: bool) -> String {
    lifecycle::window_title(name, dirty, /* use_edited_suffix = */ true)
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

/// Routes a grid `SelectionChanged` to the chrome: commit any pending data-row edit first
/// (Excel click-away), then adopt the new selection — unless a cap-rejected edit blocks the
/// commit, in which case the field stays editing and the grid is reverted to the last accepted
/// cell (`functional_spec.md §3.3`). The revert is deferred because the grid is mid-`update` as
/// the emitter. `grid` is the weak handle used only for the (rare) revert.
fn route_selection_changed(
    chrome: &Entity<ChromeView>,
    grid: WeakEntity<GridView>,
    shared: &SinkShared,
    sel: SelectionModel,
    window: &mut Window,
    cx: &mut App,
) {
    let committed = chrome.update(cx, |c, cx| {
        let ok = c.on_edit_commit_requested(window, cx);
        if ok {
            c.on_selection_changed(sel, window, cx);
        }
        ok
    });
    if committed {
        shared.last_selection.set(sel);
    } else {
        let last = shared.last_selection.get();
        window.defer(cx, move |_window, cx| {
            if let Some(grid) = grid.upgrade() {
                grid.update(cx, |g, cx| g.set_selection(last, cx));
            }
        });
    }
}

/// Builds the grid's [`GridEventSink`] — routes grid events to the sibling chrome + the worker
/// **without touching the `WorkbookWindow` entity** (the sink fires from inside the grid's own
/// `update`). Cyclic follow-ups (the cap-reject selection revert) are deferred.
/// Resolve the full chart **edit-panel** state for the chart with [`ChartId`] `id` from the worker's
/// current snapshot (P19 skeleton + P20 chrome), for **either** provenance. `None` if no such chart
/// is published, or it is an [`Unsupported`](freecell_chart_model::ChartBody::Unsupported) chart (no
/// render picture → nothing to edit). The window builds the panel from this on select / insert / a
/// republish reconcile.
fn chart_panel_info(client: &DocumentClient, id: ChartId) -> Option<ChartPanel> {
    let snapshot = client.chart_snapshot();
    for (sheet, specs) in &snapshot.sheets {
        for spec in specs.iter() {
            if spec.id != id {
                continue;
            }
            let chart = spec.chart()?;
            let kind = ChartInsertKind::from_chart_kind(&chart.kind)?;
            let ranges = (!spec.source_ranges.is_empty()).then(|| {
                spec.source_ranges
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            // The chart-wide label toggles read from its first series (chrome edits apply to all).
            let labels = chart
                .series
                .first()
                .and_then(|s| s.data_labels.as_ref())
                .map(|l| DataLabelToggles {
                    show_value: l.show_value,
                    show_category_name: l.show_category_name,
                    show_percent: l.show_percent,
                })
                .unwrap_or_default();
            let series = chart
                .series
                .iter()
                .enumerate()
                .map(|(i, s)| ChartPanelSeries {
                    name: s
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Series {}", i + 1)),
                    color: s.color.as_ref().and_then(chart_color_rgb),
                })
                .collect();
            return Some(ChartPanel {
                sheet: *sheet,
                id,
                is_authored: spec.is_authored(),
                kind,
                ranges,
                title: chart.title.clone(),
                legend: chart.legend.map(|l| l.position),
                cat_axis_title: chart.cat_axis.title.clone(),
                val_axis_title: chart.val_axis.title.clone(),
                series,
                labels,
            });
        }
    }
    None
}

/// The panel swatch color for a series' [`ChartColor`] — a concrete sRGB for an explicit color, or
/// its office-default RGB for a theme reference (so a themed series still shows a highlighted swatch).
fn chart_color_rgb(c: &ChartColor) -> Option<Rgb> {
    let color = match c {
        ChartColor::Rgb(color) => *color,
        ChartColor::Theme { slot, .. } => {
            freecell_chart_model::ThemePalette::office_default().color(*slot)
        }
    };
    Some(Rgb::from_hex(color.to_hex()))
}

fn make_grid_sink(
    chrome_slot: Rc<OnceCell<WeakEntity<ChromeView>>>,
    grid_slot: Rc<OnceCell<WeakEntity<GridView>>>,
    client: Rc<DocumentClient>,
    shared: Rc<SinkShared>,
) -> GridEventSink {
    GridEventSink::new(move |event, window, cx| match event {
        GridEvent::SelectionChanged(sel) => {
            let Some(grid) = grid_slot.get().cloned() else {
                return;
            };
            let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) else {
                return;
            };
            route_selection_changed(&chrome, grid, &shared, *sel, window, cx);
        }
        GridEvent::ViewportChanged { rows, cols } => {
            client.send(Command::SetViewport {
                sheet: shared.active_sheet.get(),
                rows: lifecycle::overscan_range(rows.clone(), limits::MAX_ROWS),
                cols: lifecycle::overscan_range(cols.clone(), limits::MAX_COLS),
            });
        }
        GridEvent::ClearCells(range) => {
            client.send(Command::ClearCells {
                sheet: shared.active_sheet.get(),
                range: *range,
            });
        }
        // The grid commits click-aways via the `SelectionChanged` path above, so it never emits
        // this variant; kept exhaustive so a future emit is a conscious wiring change.
        GridEvent::EditCommitRequested => {}
        // Type-to-replace / in-cell-editor triggers are routed to the chrome (the single
        // pending-edit owner, `components/edit_controller.md`).
        GridEvent::TypeToEdit(text) => {
            if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                let text = text.clone();
                chrome.update(cx, |c, cx| c.begin_typed(&text, window, cx));
            }
        }
        GridEvent::OpenInCellEditor(cell) => {
            if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                let cell = *cell;
                chrome.update(cx, |c, cx| c.begin_in_cell(cell, window, cx));
            }
        }
        GridEvent::InCellCommitMove(dir) => {
            if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                let dir = *dir;
                chrome.update(cx, |c, cx| c.commit_incell_move(dir, window, cx));
            }
        }
        GridEvent::InCellCancel => {
            if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                chrome.update(cx, |c, cx| c.cancel_incell(window, cx));
            }
        }
        GridEvent::Copy { cut } => {
            // Copy/cut operate on the current (last-accepted) selection.
            shared.clipboard.borrow_mut().copy(
                shared.active_sheet.get(),
                shared.last_selection.get(),
                *cut,
                &client,
            );
        }
        GridEvent::Paste => {
            // Commit any pending edit first (Excel click-away rule); a cap-rejected edit blocks
            // the paste and stays editing.
            let committed = match chrome_slot.get().and_then(|w| w.upgrade()) {
                Some(chrome) => chrome.update(cx, |c, cx| c.on_edit_commit_requested(window, cx)),
                None => true,
            };
            if committed {
                let target = shared.last_selection.get().range();
                shared
                    .clipboard
                    .borrow_mut()
                    .paste(shared.active_sheet.get(), target, &client, cx);
            }
        }
        // Structure ops (`functional_spec.md §5`): resize + insert/delete route straight to the
        // worker (the worker merge-guards insert/delete authoritatively).
        GridEvent::ResizeCommitted {
            axis,
            start,
            end,
            px,
        } => {
            let sheet = shared.active_sheet.get();
            let cmd = match axis {
                RowOrCol::Col => Command::SetColumnWidths {
                    sheet,
                    col_start: *start,
                    col_end: *end,
                    px: *px as f64,
                },
                RowOrCol::Row => Command::SetRowHeights {
                    sheet,
                    row_start: *start,
                    row_end: *end,
                    px: *px as f64,
                },
            };
            client.send(cmd);
        }
        GridEvent::InsertRows { at, count } => client.send(Command::InsertRows {
            sheet: shared.active_sheet.get(),
            row: *at,
            count: *count,
        }),
        GridEvent::InsertColumns { at, count } => client.send(Command::InsertColumns {
            sheet: shared.active_sheet.get(),
            col: *at,
            count: *count,
        }),
        GridEvent::DeleteRows { at, count } => client.send(Command::DeleteRows {
            sheet: shared.active_sheet.get(),
            row: *at,
            count: *count,
        }),
        GridEvent::DeleteColumns { at, count } => client.send(Command::DeleteColumns {
            sheet: shared.active_sheet.get(),
            col: *at,
            count: *count,
        }),
        // Chart manipulation (P18): move/resize (a new anchor) + delete route straight to the worker,
        // like the other grid-initiated structure ops. The worker resolves the `ChartId` to the
        // authored set or a loaded binding and republishes the chart snapshot.
        GridEvent::ChartAnchorChanged { id, anchor } => client.send(Command::SetChartAnchor {
            sheet: shared.active_sheet.get(),
            id: *id,
            anchor: *anchor,
        }),
        GridEvent::ChartDeleted { id } => {
            client.send(Command::DeleteChart {
                sheet: shared.active_sheet.get(),
                id: *id,
            });
            // Close the edit panel eagerly if it was shaping the just-deleted chart (the next sync
            // would close it anyway once the chart drops from the snapshot).
            if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                let id = *id;
                chrome.update(cx, |c, cx| {
                    if c.chart_panel_target() == Some(id) {
                        c.close_chart_panel(cx);
                    }
                });
            }
        }
        // A chart was clicked (P19/P20): open the right-docked edit panel for it — authored OR loaded
        // (the grid already outlined it in `begin_chart_interaction`). A loaded chart's panel shows
        // only the chrome controls (no Type/Data-range).
        GridEvent::ChartSelected(id) => {
            if let Some(panel) = chart_panel_info(&client, *id) {
                if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
                    chrome.update(cx, |c, cx| c.open_chart_panel(panel, window, cx));
                }
            }
        }
    })
}

/// Builds the chrome→grid [`ChromeGridSink`]. Every request that touches the grid is deferred:
/// applying it re-emits into or re-focuses the grid, which may be mid-`update` as the emitter.
/// `FocusGrid` in particular is reachable from a focused-in-cell key command (Tab/Escape), which
/// commits/cancels from *inside* the grid's own `capture_key_down` listener — i.e. while the grid
/// entity is already leased — so focusing it synchronously would re-enter that update and abort
/// (`entity_map` re-entrant-update panic, BUG #5). `MoveActive` / `SetActiveSheet` are deferred for
/// the same reason (they re-emit into the chrome, mid-`update` as the emitter).
fn make_chrome_grid_sink(
    grid_slot: Rc<OnceCell<WeakEntity<GridView>>>,
    chrome_slot: Rc<OnceCell<WeakEntity<ChromeView>>>,
    client: Rc<DocumentClient>,
    shared: Rc<SinkShared>,
) -> ChromeGridSink {
    ChromeGridSink::new(move |request, window, cx| {
        let Some(grid) = grid_slot.get().cloned() else {
            return;
        };
        match request {
            ChromeGridRequest::FocusGrid => {
                // Deferred (BUG #5): a focused in-cell key command (Tab/Escape) reaches this from
                // inside the grid's `capture_key_down` listener — the grid entity is already leased
                // (`cx.listener` runs the callback inside `grid.update`). Focusing it synchronously
                // here would re-enter that update and hit the `entity_map` re-entrant-update abort.
                // One deferred cycle lands the focus after the grid's update completes.
                window.defer(cx, move |window, cx| {
                    if let Some(grid) = grid.upgrade() {
                        grid.update(cx, |g, cx| g.focus_self(window, cx));
                    }
                });
            }
            ChromeGridRequest::MoveActive(motion) => {
                let motion = *motion;
                window.defer(cx, move |window, cx| {
                    if let Some(grid) = grid.upgrade() {
                        grid.update(cx, |g, cx| g.move_active(motion, window, cx));
                    }
                });
            }
            ChromeGridRequest::SetActiveSheet(id) => {
                let id = *id;
                let chrome = chrome_slot.get().cloned();
                let client = client.clone();
                let shared = shared.clone();
                window.defer(cx, move |window, cx| {
                    let Some(grid) = grid.upgrade() else {
                        return;
                    };
                    switch_grid_to_sheet(&grid, chrome.as_ref(), &client, &shared, id, window, cx);
                });
            }
            ChromeGridRequest::EditState {
                mirror,
                in_cell,
                cap,
            } => {
                // Deferred: the chrome may be emitting this from inside the grid's own `update`
                // (a grid-originated type-to-replace / in-cell trigger), so touching the grid now
                // would re-enter it. A one-cycle defer is imperceptible for the live mirror.
                let mirror = mirror.clone();
                let in_cell = *in_cell;
                let cap = cap.clone();
                window.defer(cx, move |_window, cx| {
                    if let Some(grid) = grid.upgrade() {
                        grid.update(cx, |g, cx| g.set_edit_state(mirror, in_cell, cap, cx));
                    }
                });
            }
        }
    })
}

/// Switches the grid + chrome to `sheet`: restores the grid's per-sheet scroll/selection,
/// bootstraps the worker's viewport for the new sheet, and points the chrome (ref box + content
/// fetch) at the restored active cell. Shared by the tab-click path and the window's own
/// sheet-reconciliation (add/delete).
fn switch_grid_to_sheet(
    grid: &Entity<GridView>,
    chrome: Option<&WeakEntity<ChromeView>>,
    client: &DocumentClient,
    shared: &SinkShared,
    sheet: SheetId,
    window: &mut Window,
    cx: &mut App,
) {
    shared.active_sheet.set(sheet);
    let sel = grid.update(cx, |g, cx| {
        g.set_active_sheet(sheet, cx);
        *g.selection()
    });
    shared.last_selection.set(sel);
    // Bootstrap the new sheet's cache + values (an unvisited sheet has no cache yet); the grid's
    // own `ViewportChanged` refines the range once it renders the sheet.
    client.send(Command::SetViewport {
        sheet,
        rows: lifecycle::overscan_range(0..lifecycle::INITIAL_VIEWPORT_ROWS, limits::MAX_ROWS),
        cols: lifecycle::overscan_range(0..lifecycle::INITIAL_VIEWPORT_COLS, limits::MAX_COLS),
    });
    if let Some(chrome) = chrome.and_then(|w| w.upgrade()) {
        chrome.update(cx, |c, cx| {
            // Re-point the chrome at the new sheet BEFORE fetching content — otherwise every
            // subsequent edit/style/fetch (and the tab highlight) would target the OLD sheet after
            // an add (`functional_spec.md §3.7`). `adopt_active_sheet` no-ops on the tab-click path
            // (chrome already switched itself) and does not re-emit `SetActiveSheet`.
            c.adopt_active_sheet(sheet, cx);
            c.on_selection_changed(sel, window, cx);
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The macOS custom titlebar (§7.1 / `ui_design.md §1`) shows the edited state **textually**
    /// and **always** — its distinguishing contract vs the native window title, which drops the
    /// `— Edited` suffix on macOS (`title_uses_suffix()` is `false` there, the traffic-light dot
    /// carrying dirtiness). This locks that in: a regression making `titlebar_title_text` defer to
    /// `title_uses_suffix()` would, on macOS, drop the suffix and fail the `assert_ne!` below.
    #[test]
    fn titlebar_title_always_suffixes_when_dirty() {
        // Dirty → suffix; clean → bare name.
        assert_eq!(
            titlebar_title_text("Budget.xlsx", true),
            "Budget.xlsx — Edited"
        );
        assert_eq!(titlebar_title_text("Budget.xlsx", false), "Budget.xlsx");

        // It must equal the ALWAYS-suffix form (use_edited_suffix = true) and must NOT match the
        // native-title rule where the suffix is suppressed (use_edited_suffix = false) — so a
        // future change routing it through `title_uses_suffix()` (false on macOS) is caught.
        assert_eq!(
            titlebar_title_text("Budget.xlsx", true),
            lifecycle::window_title("Budget.xlsx", true, true),
        );
        assert_ne!(
            titlebar_title_text("Budget.xlsx", true),
            lifecycle::window_title("Budget.xlsx", true, false),
        );
    }
}
