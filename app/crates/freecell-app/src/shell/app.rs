//! [`FreeCellApp`] — the app-global that owns the window registry + menu/action wiring and
//! orchestrates the welcome / open / new / quit flows (`components/app_shell.md §Structure,
//! §Lifecycle rules`, `functional_spec.md §2`).
//!
//! It is a gpui [`Global`]. All the *decisions* it makes are the pure [`super::registry`] /
//! [`super::lifecycle`] helpers; this type is the plumbing that performs them against real
//! windows, menus, panels, and dialogs. Public entry points are static fns taking `&mut App`;
//! each takes the global lease exactly once (`update_global`) and drives internal `&mut self`
//! helpers, so the global is never re-entered.

use std::path::{Path, PathBuf};

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Entity, Global,
    TitlebarOptions, WindowBounds, WindowHandle, WindowId, WindowOptions,
};
use gpui_component::Root;

use freecell_core::recent::{RecentList, WELCOME_LIMIT};
use freecell_engine::DocumentSource;

use super::about::AboutView;
use super::lifecycle::{QuitPlan, QuitStep};
use super::registry::{OpenOutcome, WindowKey, WindowRegistry};
use super::welcome::WelcomeView;
use super::window::{open_panel_options, WorkbookWindow};
use super::{menus, recents, About, ClearRecent, NewWorkbook, OpenFile, OpenRecent, Quit};

/// A document window as the app tracks it: its registry key, gpui identity, and the root
/// entity (so the app can drive its modals during the quit flow).
struct AppWindow {
    key: WindowKey,
    window_id: WindowId,
    handle: AnyWindowHandle,
    entity: Entity<WorkbookWindow>,
}

/// The app global.
pub struct FreeCellApp {
    registry: WindowRegistry,
    welcome: Option<Entity<WelcomeView>>,
    welcome_id: Option<WindowId>,
    /// The standalone About window (`functional_spec.md §4`), when open. Single-instance: a second
    /// `About` just activates it. Cleared with `about_id` when the window closes.
    about: Option<Entity<AboutView>>,
    about_id: Option<WindowId>,
    windows: Vec<AppWindow>,
    /// An in-progress app quit (front-to-back dirty-window prompting).
    quit_plan: Option<QuitPlan>,
    /// Set once the app has decided to quit, so a cascade of window closes doesn't re-quit.
    quitting: bool,
    /// The live most-recent-first recent-files list, loaded in [`init`](Self::init) and updated
    /// at every record site (`record_recent`) + `clear_recents`. All the list logic is the pure
    /// [`RecentList`] (`freecell_core::recent`); this owns the app's working copy.
    recents: RecentList,
    /// Where the recent-files list persists (`<data_dir>/FreeCell/recents.json`), or `None` when
    /// no per-user data directory resolves — then the list is in-memory only. Tests point this at
    /// a temp file (or `None`) so they never touch the real user store.
    recents_store: Option<PathBuf>,
}

impl Global for FreeCellApp {}

impl FreeCellApp {
    /// Installs the global, registers the global actions + menu bar + key bindings + the
    /// window-closed / open-file hooks. Call once at startup, before `show_welcome`.
    ///
    /// The recent-files list starts **empty**; the resolved store path is recorded but the disk
    /// read is deferred to [`load_recents`](Self::load_recents) (a startup call from `main.rs`).
    /// Keeping the load out of `init` means gpui tests — which call `init` then reset the store —
    /// never touch the real per-user data dir (`architecture.md §3`).
    pub fn init(cx: &mut App) {
        cx.set_global(FreeCellApp {
            registry: WindowRegistry::new(),
            welcome: None,
            welcome_id: None,
            about: None,
            about_id: None,
            windows: Vec::new(),
            quit_plan: None,
            quitting: false,
            recents: RecentList::default(),
            recents_store: recents::recents_store_path(),
        });

        cx.on_action(|_: &NewWorkbook, cx| FreeCellApp::new_workbook(cx));
        cx.on_action(|_: &OpenFile, cx| FreeCellApp::open_via_panel(cx));
        cx.on_action(|_: &About, cx| FreeCellApp::show_about(cx));
        cx.on_action(|_: &Quit, cx| FreeCellApp::request_quit(cx));
        // Open Recent carries the exact file path (not an install-time snapshot index), so it
        // dispatches straight to `open_path` — which opens that file or shows the vanished-file
        // dialog if it moved (`architecture.md §3.2, §5`).
        cx.on_action(|a: &OpenRecent, cx| FreeCellApp::open_path(&a.path, cx));
        cx.on_action(|_: &ClearRecent, cx| FreeCellApp::clear_recents(cx));

        cx.on_window_closed(|cx, window_id| {
            // Deferred so the handler never runs nested inside another `update_global` lease
            // (e.g. a `remove_window` issued from within the welcome-close flow synchronously
            // firing this observer). Deferring restores the lease before we re-take it.
            cx.defer(move |cx| {
                cx.update_global::<FreeCellApp, _>(|app, cx| app.on_window_closed(window_id, cx));
            });
        })
        .detach();

        menus::bind_keys(cx);
        // Install the base menu bar (empty **Open Recent** until `load_recents` fills it; macOS
        // only, a no-op on Linux).
        menus::install_menus_with(&cx.global::<FreeCellApp>().recents, cx);
    }

    /// Loads the persisted recent-files list from disk into the app, once, at **startup**
    /// (invoked from `main.rs` right after [`init`](Self::init)), and refreshes the UI. Kept out
    /// of `init` so gpui tests never read the real per-user data dir (`architecture.md §3`);
    /// production behavior is unchanged (load once at launch, missing/corrupt ⇒ empty).
    pub fn load_recents(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| {
            if let Some(path) = app.recents_store.clone() {
                app.recents = RecentList::load(&path);
                app.refresh_recents_ui(cx);
            }
        });
    }

    /// Opens the welcome window (`functional_spec.md §2.1`).
    pub fn show_welcome(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.do_show_welcome(cx));
    }

    /// Creates a new empty workbook in a new window (`functional_spec.md §2.2`).
    pub fn new_workbook(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| {
            app.open_document(DocumentSource::NewWorkbook, None, cx);
        });
    }

    /// Test-only mirror of [`new_workbook`](Self::new_workbook) over a worker-less window.
    #[cfg(test)]
    pub(crate) fn new_workbook_detached(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.open_detached_document(None, cx));
    }

    /// Test-only mirror of [`open_path`](Self::open_path): same canonicalize + dedupe, but the
    /// opened window is worker-less (so the required suite stays deterministic).
    #[cfg(test)]
    pub(crate) fn open_path_detached(path: &Path, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.do_open_path_detached(path, cx));
    }

    /// Opens the native file panel, then opens the chosen `.xlsx` (`functional_spec.md §5.1`).
    pub fn open_via_panel(cx: &mut App) {
        let receiver = cx.prompt_for_paths(open_panel_options());
        cx.spawn(async move |cx| {
            let picked = receiver
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten()
                .and_then(|paths| paths.into_iter().next());
            if let Some(path) = picked {
                cx.update(|cx| FreeCellApp::open_path(&path, cx));
            }
        })
        .detach();
    }

    /// Opens a file by path (Finder/CLI/panel), deduping against already-open windows
    /// (`functional_spec.md §5.1`).
    pub fn open_path(path: &Path, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.do_open_path(path, cx));
    }

    /// Requests an application quit (`functional_spec.md §2.3`, `Cmd/Ctrl+Q`).
    pub fn request_quit(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.do_request_quit(cx));
    }

    /// Opens the standalone About window, or activates it if it is already open (single-instance,
    /// `functional_spec.md §4`).
    pub fn show_about(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.do_show_about(cx));
    }

    /// Clears the recent-files list (**Clear Recent Files**, `functional_spec.md §3`), persists
    /// the now-empty store, and refreshes the UI (menu → empty state).
    fn clear_recents(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| {
            app.recents.clear();
            app.persist_recents();
            app.refresh_recents_ui(cx);
        });
    }

    // ---- WorkbookWindow → app notifications (called from the document window) --------------

    /// A document window finished loading → the welcome window (if still up) closes
    /// (`functional_spec.md §2.2`).
    pub fn note_window_loaded(_key: WindowKey, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| app.close_welcome(cx));
    }

    /// A window adopted a canonical path (after a `Save As`), so a later open dedupes to it —
    /// and it becomes a recent file (`functional_spec.md §1.1`: adopting a path via Save/Save As
    /// records it).
    pub fn note_window_path(key: WindowKey, path: PathBuf, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| {
            app.registry.set_path(key, Some(path.clone()));
            app.record_recent(path, cx);
        });
    }

    /// A window's dirty flag changed.
    pub fn note_window_dirty(key: WindowKey, dirty: bool, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, _| app.registry.set_dirty(key, dirty));
    }

    /// A close/quit prompt (or its save panel) was cancelled → abort an in-progress quit.
    pub fn note_prompt_cancelled(cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, cx| {
            if let Some(plan) = app.quit_plan.as_mut() {
                plan.cancel();
                app.advance_quit(cx);
            }
        });
    }

    // ---- Internal (operate on the leased global) ------------------------------------------

    fn do_show_welcome(&mut self, cx: &mut App) {
        // `welcome_id` and the `welcome` entity are set/cleared together.
        if let Some(id) = self.welcome_id {
            // Already open — just activate it.
            if let Some(w) = cx.windows().into_iter().find(|w| w.window_id() == id) {
                w.update(cx, |_, window, _| window.activate_window()).ok();
            }
            return;
        }
        let mut entity_slot: Option<Entity<WelcomeView>> = None;
        let slot = &mut entity_slot;
        let handle: WindowHandle<Root> = cx
            .open_window(welcome_window_options(cx), |window, cx| {
                let welcome = cx.new(|cx| WelcomeView::new(window, cx));
                *slot = Some(welcome.clone());
                cx.new(|cx| Root::new(welcome, window, cx))
            })
            .expect("open welcome window");
        let any: AnyWindowHandle = handle.into();
        self.welcome = entity_slot;
        self.welcome_id = Some(any.window_id());
        self.registry.set_welcome_open(true);
        // Seed the freshly-opened welcome with the current recent rows (and rebuild the menu);
        // `refresh_recents_ui` then keeps it live while it stays open (`functional_spec.md §2.4`).
        self.refresh_recents_ui(cx);
    }

    fn close_welcome(&mut self, cx: &mut App) {
        let Some(id) = self.welcome_id else { return };
        self.registry.set_welcome_open(false);
        self.welcome = None;
        self.welcome_id = None;
        if let Some(w) = cx.windows().into_iter().find(|w| w.window_id() == id) {
            w.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }

    fn do_open_path(&mut self, path: &Path, cx: &mut App) {
        // Canonicalize so dedupe (and later saves) compare stable paths.
        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.report_error(
                    "Couldn't open the file",
                    &format!("{}: {e}", path.display()),
                    cx,
                );
                return;
            }
        };
        // A successful canonicalize means the file exists → record it as a recent file for BOTH
        // outcomes (activate an already-open window, or open a new one). One record here covers
        // the Open… panel, CLI argv, a welcome-row click, and the Open Recent menu
        // (`functional_spec.md §1.1`, `architecture.md §3.1`).
        self.record_recent(canonical.clone(), cx);
        match self.registry.resolve_open(&canonical) {
            OpenOutcome::Activate(key) => {
                if let Some(w) = self.windows.iter().find(|w| w.key == key) {
                    w.handle
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                }
            }
            OpenOutcome::OpenNew => {
                self.open_document(
                    DocumentSource::OpenFile(canonical.clone()),
                    Some(canonical),
                    cx,
                );
            }
        }
    }

    /// Test-only: [`do_open_path`](Self::do_open_path) with a worker-less window on `OpenNew` (the
    /// dedupe/activate path is identical — it is what the tests exercise).
    #[cfg(test)]
    fn do_open_path_detached(&mut self, path: &Path, cx: &mut App) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        // Mirror the production record site (`do_open_path`) so the recents wiring is exercised
        // under the deterministic test harness. `record` prunes a non-existent path, so a bogus
        // detached open never leaves a phantom entry.
        self.record_recent(canonical.clone(), cx);
        match self.registry.resolve_open(&canonical) {
            OpenOutcome::Activate(key) => {
                if let Some(w) = self.windows.iter().find(|w| w.key == key) {
                    w.handle
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                }
            }
            OpenOutcome::OpenNew => self.open_detached_document(Some(canonical), cx),
        }
    }

    /// Opens a document window over `source`. `canonical` is the file's path (for an open) so
    /// dedupe + the title are correct before the load completes; `None` for a new workbook.
    fn open_document(&mut self, source: DocumentSource, canonical: Option<PathBuf>, cx: &mut App) {
        let key = self.registry.register(canonical.clone());
        let title_path = canonical;
        self.install_document_window(
            key,
            move |window, cx| WorkbookWindow::new(key, source, title_path, window, cx),
            cx,
        );
    }

    /// Opens the document window built by `build` (the real spawn-a-worker constructor in
    /// production, or the worker-less test constructor), registering its handle + close hook.
    fn install_document_window(
        &mut self,
        key: WindowKey,
        build: impl FnOnce(&mut gpui::Window, &mut gpui::Context<WorkbookWindow>) -> WorkbookWindow
            + 'static,
        cx: &mut App,
    ) {
        let mut entity_slot: Option<Entity<WorkbookWindow>> = None;
        let slot = &mut entity_slot;
        let handle: WindowHandle<Root> = cx
            .open_window(document_window_options(cx), move |window, cx| {
                let ww = cx.new(|cx| build(window, cx));
                *slot = Some(ww.clone());
                let close_entity = ww.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    close_entity.update(cx, |w, cx| w.on_titlebar_close(window, cx))
                });
                cx.new(|cx| Root::new(ww, window, cx))
            })
            .expect("open document window");

        let entity = entity_slot.expect("document window built its root entity");
        let any: AnyWindowHandle = handle.into();
        self.windows.push(AppWindow {
            key,
            window_id: any.window_id(),
            handle: any,
            entity,
        });
    }

    /// Test-only: opens a document window over a **worker-less** [`WorkbookWindow`] (no OS thread
    /// under the deterministic `TestScheduler`), used by the gpui window tests.
    #[cfg(test)]
    fn open_detached_document(&mut self, canonical: Option<PathBuf>, cx: &mut App) {
        let key = self.registry.register(canonical.clone());
        let title_path = canonical;
        self.install_document_window(
            key,
            move |window, cx| WorkbookWindow::new_detached_for_test(key, title_path, window, cx),
            cx,
        );
    }

    fn do_request_quit(&mut self, cx: &mut App) {
        let order = self.front_to_back_keys(cx);
        let dirty = self.registry.dirty_among(&order);
        if dirty.is_empty() {
            self.do_quit(cx);
        } else {
            self.quit_plan = Some(QuitPlan::new(dirty));
            self.advance_quit(cx);
        }
    }

    /// Drives the quit flow one step: prompt the next dirty window, or quit, or (on cancel)
    /// stand down.
    fn advance_quit(&mut self, cx: &mut App) {
        let step = match self.quit_plan.as_ref() {
            Some(plan) => plan.next(),
            None => return,
        };
        match step {
            QuitStep::Prompt(key) => match self.windows.iter().find(|w| w.key == key) {
                Some(w) => {
                    w.handle
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                    let entity = w.entity.clone();
                    entity.update(cx, |ww, cx| ww.show_unsaved_modal(cx));
                }
                None => {
                    // The window vanished between planning and prompting — treat as resolved.
                    if let Some(plan) = self.quit_plan.as_mut() {
                        plan.resolved(key);
                    }
                    self.advance_quit(cx);
                }
            },
            QuitStep::QuitNow => {
                self.quit_plan = None;
                self.do_quit(cx);
            }
            QuitStep::Aborted => {
                self.quit_plan = None;
            }
        }
    }

    fn do_quit(&mut self, cx: &mut App) {
        self.quitting = true;
        cx.quit();
    }

    fn on_window_closed(&mut self, window_id: WindowId, cx: &mut App) {
        if self.welcome_id == Some(window_id) {
            self.welcome = None;
            self.welcome_id = None;
            self.registry.set_welcome_open(false);
        } else if self.about_id == Some(window_id) {
            self.about = None;
            self.about_id = None;
            self.registry.set_about_open(false);
        } else if let Some(pos) = self.windows.iter().position(|w| w.window_id == window_id) {
            let key = self.windows[pos].key;
            self.windows.remove(pos);
            self.registry.remove(key);
            // Only advance the quit flow when the window that closed was actually one of the
            // windows we're prompting. A *clean* (or otherwise unrelated) window closing
            // mid-quit must not re-issue the in-flight prompt — the unsaved modals are plain
            // overlays, not app-modal, so re-showing the current window's modal is glitchy.
            let was_pending = self
                .quit_plan
                .as_ref()
                .map(|plan| plan.is_pending(key))
                .unwrap_or(false);
            if was_pending {
                if let Some(plan) = self.quit_plan.as_mut() {
                    plan.resolved(key);
                }
                self.advance_quit(cx);
                return;
            }
        }

        // Last window closed (workbook or welcome) → the app quits (`functional_spec.md §2`).
        if !self.quitting && self.registry.is_empty() {
            self.do_quit(cx);
        }
    }

    /// Opens or activates the standalone About window (`architecture.md §9.2`). Mirrors
    /// [`do_show_welcome`](Self::do_show_welcome): `about` and `about_id` are set/cleared together.
    fn do_show_about(&mut self, cx: &mut App) {
        if let Some(id) = self.about_id {
            // Already open — just activate it (single-instance, `functional_spec.md §4`).
            if let Some(w) = cx.windows().into_iter().find(|w| w.window_id() == id) {
                w.update(cx, |_, window, _| window.activate_window()).ok();
            }
            return;
        }
        let mut entity_slot: Option<Entity<AboutView>> = None;
        let slot = &mut entity_slot;
        let handle: WindowHandle<Root> = cx
            .open_window(about_window_options(cx), |window, cx| {
                let about = cx.new(|cx| AboutView::new(window, cx));
                *slot = Some(about.clone());
                cx.new(|cx| Root::new(about, window, cx))
            })
            .expect("open about window");
        let any: AnyWindowHandle = handle.into();
        self.about = entity_slot;
        self.about_id = Some(any.window_id());
        self.registry.set_about_open(true);
    }

    /// Reports an app-level error (e.g. an `Open…`/CLI path that failed to resolve): on the
    /// frontmost workbook window if one is active, else the welcome window — **opening the
    /// welcome window to host it when neither exists**. Without that fallback a startup open of
    /// a bad path would leave the app running with no window and no menu bar on Linux (an
    /// unquittable zombie); the error must always surface (`functional_spec.md §5.1`: "Never a
    /// crash").
    fn report_error(&mut self, title: &str, detail: &str, cx: &mut App) {
        // Prefer the frontmost document window.
        if let Some(id) = cx.active_window().map(|w| w.window_id()) {
            if let Some(w) = self.windows.iter().find(|w| w.window_id == id) {
                let entity = w.entity.clone();
                entity.update(cx, |ww, cx| ww.show_error_dialog(title, detail, cx));
                return;
            }
        }
        // Otherwise host it on the welcome window, opening it first if it isn't up.
        if self.welcome.is_none() {
            self.do_show_welcome(cx);
        }
        if let Some(welcome) = self.welcome.clone() {
            welcome.update(cx, |w, cx| w.show_error(title, detail, cx));
        }
    }

    // ---- Recent files (`functional_spec.md §1`, `architecture.md §3`) ----------------------

    /// The single record choke point (`architecture.md §3.1`): records `canonical` (already a
    /// canonical, existing path) at the current instant, persists best-effort, and refreshes the
    /// UI. Called from every place a window successfully associates with a file on disk — an open
    /// (`do_open_path`) and a Save/Save-As path adoption (`note_window_path`).
    fn record_recent(&mut self, canonical: PathBuf, cx: &mut App) {
        self.recents.record(canonical, recents::now_unix_secs());
        self.persist_recents();
        self.refresh_recents_ui(cx);
    }

    /// Writes the recent-files store to disk when a store path is configured. Best-effort: a
    /// write failure (e.g. a read-only disk) is logged and swallowed so it can never block an
    /// open/save or raise a dialog (`functional_spec.md §1.5`, `architecture.md §6`).
    fn persist_recents(&self) {
        if let Some(path) = &self.recents_store {
            if let Err(err) = self.recents.save(path) {
                tracing::warn!(
                    error = %err,
                    path = %path.display(),
                    "failed to persist recent files"
                );
            }
        }
    }

    /// Pushes the current recent-files list to the UI surfaces that reflect it: the welcome
    /// window (if open) gets fresh display rows (`functional_spec.md §2.4` live updates), and the
    /// macOS **Open Recent** menu is rebuilt (`architecture.md §3.2`).
    fn refresh_recents_ui(&mut self, cx: &mut App) {
        if let Some(welcome) = self.welcome.clone() {
            let rows = self
                .recents
                .display_entries(recents::now_unix_secs(), WELCOME_LIMIT);
            welcome.update(cx, |w, cx| w.set_recents(rows, cx));
        }
        menus::install_menus_with(&self.recents, cx);
    }

    /// The registered window keys, front-to-back (the quit-prompt order). Uses the platform
    /// window stack for ordering, then **unions in every registered window not present in the
    /// stack** — a partial `window_stack()` (e.g. a minimized window omitted on some platform)
    /// must never let a registered window escape the quit prompt and get force-closed with
    /// unsaved changes. When the stack is unavailable this degrades to registration order.
    fn front_to_back_keys(&self, cx: &App) -> Vec<WindowKey> {
        let mut ordered: Vec<WindowKey> = Vec::with_capacity(self.windows.len());
        if let Some(stack) = cx.window_stack() {
            for handle in stack.iter() {
                if let Some(w) = self
                    .windows
                    .iter()
                    .find(|w| w.window_id == handle.window_id())
                {
                    if !ordered.contains(&w.key) {
                        ordered.push(w.key);
                    }
                }
            }
        }
        // Append any registered window the stack didn't cover (missing stack, or a partial one).
        for key in self.registry.keys() {
            if !ordered.contains(&key) {
                ordered.push(key);
            }
        }
        ordered
    }

    // ---- Test accessors -------------------------------------------------------------------

    /// The number of open document windows (tests).
    #[cfg(test)]
    pub(crate) fn window_count(cx: &App) -> usize {
        cx.global::<FreeCellApp>().windows.len()
    }

    /// Whether the welcome window is registered as open (tests).
    #[cfg(test)]
    pub(crate) fn welcome_open(cx: &App) -> bool {
        cx.global::<FreeCellApp>().registry.welcome_open()
    }

    /// The live welcome view entity, if the welcome window is open (tests).
    #[cfg(test)]
    pub(crate) fn welcome_view(cx: &App) -> Option<Entity<WelcomeView>> {
        cx.global::<FreeCellApp>().welcome.clone()
    }

    /// Whether the About window is registered as open (tests).
    #[cfg(test)]
    pub(crate) fn about_open(cx: &App) -> bool {
        cx.global::<FreeCellApp>().registry.about_open()
    }

    /// The live About view entity, if the About window is open (tests).
    #[cfg(test)]
    pub(crate) fn about_view(cx: &App) -> Option<Entity<AboutView>> {
        cx.global::<FreeCellApp>().about.clone()
    }

    /// The document window entity at `index` (tests).
    #[cfg(test)]
    pub(crate) fn nth_window(cx: &App, index: usize) -> Option<Entity<WorkbookWindow>> {
        cx.global::<FreeCellApp>()
            .windows
            .get(index)
            .map(|w| w.entity.clone())
    }

    /// The document window handle at `index` (tests).
    #[cfg(test)]
    pub(crate) fn nth_window_handle(cx: &App, index: usize) -> Option<AnyWindowHandle> {
        cx.global::<FreeCellApp>()
            .windows
            .get(index)
            .map(|w| w.handle)
    }

    /// The stored recent-files paths, most-recent-first (tests).
    #[cfg(test)]
    pub(crate) fn recents_paths(cx: &App) -> Vec<PathBuf> {
        cx.global::<FreeCellApp>()
            .recents
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Resets the recent-files state for a test: empties the in-memory list and points
    /// persistence at `store` (`None` disables disk writes). `boot()` calls this with `None` so
    /// no gpui test ever reads from / writes to the real per-user store; a test exercising the
    /// persistence wiring passes a temp path.
    #[cfg(test)]
    pub(crate) fn reset_recents_for_test(store: Option<PathBuf>, cx: &mut App) {
        cx.update_global::<FreeCellApp, _>(|app, _| {
            app.recents = RecentList::default();
            app.recents_store = store;
        });
    }
}

/// The transparent macOS titlebar options (`architecture.md §7.1`, `ui_design.md §1`): a
/// hidden system title + hidden native titlebar (so the window draws its own 36 px row —
/// [`super::titlebar`]) with the traffic lights repositioned to vertically center in that row.
/// `None` on Linux (server decorations, unchanged) and whenever the §7.1 fallback flips
/// [`super::titlebar::MACOS_TITLEBAR`] off. Verified present at the pinned gpui rev; the native
/// behavior itself is the on-device smoke gate.
fn titlebar_options() -> Option<TitlebarOptions> {
    if super::titlebar::MACOS_TITLEBAR {
        Some(TitlebarOptions {
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.0), px(12.0))),
            title: None,
        })
    } else {
        None
    }
}

/// The document window options: ~1200×800, centered, resizable, macOS custom titlebar (§7.1)
/// or standard traffic lights on Linux (`functional_spec.md §2.3`).
fn document_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1200.0), px(800.0)), cx)),
        titlebar: titlebar_options(),
        ..Default::default()
    }
}

/// The welcome window options: fixed **720×480** two-pane launch surface (`ui_design.md §1`),
/// non-resizable, non-minimizable, centered, macOS custom titlebar (§7.1) or standard traffic
/// lights on Linux (`functional_spec.md §2.2`).
fn welcome_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(720.0), px(480.0)), cx)),
        titlebar: titlebar_options(),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    }
}

/// The About window options: small fixed **400×262** surface (`ui_design.md §6`), non-resizable,
/// non-minimizable, centered, macOS custom titlebar (§7.1) or standard traffic lights on Linux
/// (`functional_spec.md §4`). The height is tuned to the top-packed identity/links body
/// (`about.rs`) so the bottom whitespace matches the top margin (no large empty band); verified by
/// a headless capture during development.
fn about_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(400.0), px(262.0)), cx)),
        titlebar: titlebar_options(),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    //! GPUI-context tests for the shell lifecycle + Phase-11 composition.
    //!
    //! **Determinism.** In production a [`WorkbookWindow`] spawns the real eval worker on a
    //! dedicated OS thread ([`DocumentClient::spawn`]); gpui's test scheduler is strictly
    //! deterministic and *panics at `end_test`* if it observes activity on any thread other than
    //! its own. So these tests compose windows over a **worker-less** client
    //! ([`DocumentClient::detached`], via the `test-support` feature +
    //! `new_workbook_detached` / `open_path_detached`) — no OS thread, no flake. The *end-to-end*
    //! flows that need the worker to actually **emit** an event (welcome-closes-on-`Loaded`, a
    //! real `Saved`, real published values) remain covered by the `freecell-engine` round-trips
    //! (`tests/roundtrip.rs`, `tests/worker_seam.rs`, which drive the real worker via blocking
    //! `recv`) + the Xvfb smoke launch. The event **folding** logic is tested here by **injecting**
    //! synthesized [`WorkerEvent`]s straight into `on_worker_event` (no emission, no parking).

    use super::*;
    use freecell_core::recent::DisplayEntry;
    use freecell_engine::{LoadError, SaveError, WorkerEvent};
    use gpui::TestAppContext;
    use tempfile::tempdir;

    /// Boots gpui-component + the app global in a fresh test app.
    fn boot(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            FreeCellApp::init(cx);
            // Isolate recents from the real per-user data dir: start empty and disable disk
            // persistence unless a test opts into a temp store. Keeps every gpui test hermetic.
            FreeCellApp::reset_recents_for_test(None, cx);
        });
    }

    fn window_count(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| FreeCellApp::window_count(cx))
    }

    /// Saves a fresh empty workbook to `path` so it is a real, openable `.xlsx`.
    fn write_xlsx(path: &std::path::Path) {
        freecell_engine::WorkbookDocument::new_empty()
            .unwrap()
            .save(path)
            .unwrap();
    }

    #[gpui::test]
    fn welcome_window_opens_on_show(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::show_welcome);
        assert!(
            cx.update(|cx| FreeCellApp::welcome_open(cx)),
            "welcome registered open"
        );
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    }

    #[gpui::test]
    fn new_workbook_registers_a_document_window(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::show_welcome);
        cx.update(FreeCellApp::new_workbook_detached);
        // The document window is registered synchronously on open (the welcome-closes-on-Loaded
        // step needs a worker event — see the module note).
        assert_eq!(window_count(cx), 1, "a document window opened");
    }

    #[gpui::test]
    fn open_dedupes_same_path_activates_existing(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("book.xlsx");
        write_xlsx(&path);

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(window_count(cx), 1);
        // A second open of the same canonical path focuses the existing window — no duplicate.
        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(window_count(cx), 1, "same path deduped to one window");
    }

    // ---- Recent files (`architecture.md §3`, §7) -------------------------------------------

    #[gpui::test]
    fn open_records_the_canonical_path_in_recents(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Recorded.xlsx");
        write_xlsx(&path);

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)),
            vec![path.canonicalize().unwrap()],
            "opening a file records its canonical path"
        );
    }

    #[gpui::test]
    fn save_as_path_adoption_records_in_recents(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Adopted.xlsx");
        write_xlsx(&path); // the file must exist for the Saved handler's canonicalize() to resolve

        // Drive the same Saved → note_window_path seam as
        // `saved_adopts_canonical_path_and_closes_after_save`.
        let (handle, entity) = new_injectable_window(cx);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.set_dirty_for_test(true, ctx);
                    w.arm_pending_save_for_test(path.clone(), 7, true);
                    w.inject_worker_event_for_test(
                        WorkerEvent::Saved {
                            req_id: 7,
                            ops_seen: 0,
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();

        assert_eq!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)),
            vec![path.canonicalize().unwrap()],
            "adopting a path via Save records it as recent"
        );
    }

    #[gpui::test]
    fn reopening_a_file_dedupes_to_one_recent_entry(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Twice.xlsx");
        write_xlsx(&path);

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)),
            vec![path.canonicalize().unwrap()],
            "re-opening the same path keeps a single deduped entry"
        );
    }

    #[gpui::test]
    fn clear_recents_empties_the_store(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("ToClear.xlsx");
        write_xlsx(&path);

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(cx.update(|cx| FreeCellApp::recents_paths(cx)).len(), 1);
        cx.update(FreeCellApp::clear_recents);
        assert!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)).is_empty(),
            "Clear Recent Files empties the list"
        );
    }

    #[gpui::test]
    fn recording_persists_to_the_configured_store_file(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        // A nested store path exercises `save`'s parent-dir creation.
        let store = dir.path().join("state").join("recents.json");
        cx.update(|cx| FreeCellApp::reset_recents_for_test(Some(store.clone()), cx));

        let path = dir.path().join("Persisted.xlsx");
        write_xlsx(&path);
        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));

        assert!(store.exists(), "recording persisted the store to disk");
        let reloaded = RecentList::load(&store);
        assert_eq!(
            reloaded
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![path.canonicalize().unwrap()],
            "the persisted store round-trips the canonical path"
        );
    }

    #[gpui::test]
    fn load_recents_reads_the_configured_store(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Seeded.xlsx");
        write_xlsx(&path);
        // Pre-seed a store file on disk, then point the app at it via the test seam and load.
        let store = dir.path().join("recents.json");
        let mut seeded = RecentList::default();
        seeded.record(path.canonicalize().unwrap(), 1_000);
        seeded.save(&store).unwrap();

        cx.update(|cx| FreeCellApp::reset_recents_for_test(Some(store.clone()), cx));
        assert!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)).is_empty(),
            "the app starts empty before the startup load"
        );
        cx.update(FreeCellApp::load_recents);
        assert_eq!(
            cx.update(|cx| FreeCellApp::recents_paths(cx)),
            vec![path.canonicalize().unwrap()],
            "load_recents reads the persisted store into the app"
        );
    }

    #[gpui::test]
    fn showing_welcome_seeds_current_recents(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Seed.xlsx");
        write_xlsx(&path);
        // Record a recent *before* the welcome opens, then open it: `do_show_welcome` must seed
        // the fresh view with the current rows (`architecture.md §3.2`).
        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        cx.update(FreeCellApp::show_welcome);
        let welcome = cx.update(|cx| FreeCellApp::welcome_view(cx).expect("welcome open"));
        assert_eq!(
            cx.update(|cx| welcome.read(cx).recent_row_count()),
            1,
            "the freshly-opened welcome is seeded with the current recent rows"
        );
    }

    #[gpui::test]
    fn recording_updates_the_open_welcome(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::show_welcome);
        let welcome = cx.update(|cx| FreeCellApp::welcome_view(cx).expect("welcome open"));
        assert!(
            cx.update(|cx| welcome.read(cx).is_empty_state()),
            "the welcome starts on the empty state with no recents"
        );
        // Recording a recent while the welcome is open pushes the new row into it live
        // (`functional_spec.md §2.4`, the `refresh_recents_ui` welcome branch).
        let dir = tempdir().unwrap();
        let path = dir.path().join("Live.xlsx");
        write_xlsx(&path);
        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        assert_eq!(
            cx.update(|cx| welcome.read(cx).recent_row_count()),
            1,
            "recording a recent live-updates the open welcome"
        );
    }

    #[gpui::test]
    fn clicking_a_recent_row_routes_to_open_path(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Clicked.xlsx");
        write_xlsx(&path);
        let canonical = path.canonicalize().unwrap();

        cx.update(FreeCellApp::show_welcome);
        let welcome = cx.update(|cx| FreeCellApp::welcome_view(cx).expect("welcome open"));
        // Seed a row for the real file (the view renders from its own display rows).
        cx.update(|cx| {
            welcome.update(cx, |w, cx| {
                w.set_recents(
                    vec![DisplayEntry {
                        path: canonical.clone(),
                        name: "Clicked.xlsx".to_string(),
                        subtitle: "0 B · tmp".to_string(),
                        relative_time: "Just now".to_string(),
                    }],
                    cx,
                )
            })
        });
        assert_eq!(cx.update(|cx| welcome.read(cx).recent_row_count()), 1);

        // The file vanishes between render and click; clicking the row must route to `open_path`,
        // whose canonicalize-failure surfaces the "Couldn't open the file" dialog on the welcome
        // (`functional_spec.md §2.2`). That modal is the observable proof the click reached
        // `open_path` — with no real worker window spawned.
        std::fs::remove_file(&canonical).unwrap();
        cx.update(|cx| welcome.update(cx, |w, cx| w.open_recent(0, cx)));
        cx.run_until_parked();
        assert!(
            cx.update(|cx| welcome.read(cx).has_modal()),
            "clicking a recent row routes to open_path (its vanished-file error lands on the welcome)"
        );
    }

    // ---- About window (`functional_spec.md §4`, `architecture.md §9`) ----------------------

    #[gpui::test]
    fn about_action_opens_a_single_about_window(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::show_about);
        assert!(
            cx.update(|cx| FreeCellApp::about_open(cx)),
            "the About action opens (and registers) the About window"
        );
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
        assert!(cx.update(|cx| FreeCellApp::about_view(cx).is_some()));

        // A second About activates the existing window rather than opening a duplicate
        // (single-instance, `functional_spec.md §4`).
        cx.update(FreeCellApp::show_about);
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            1,
            "About is single-instance — a second trigger activates, not duplicates"
        );
    }

    #[gpui::test]
    fn closing_the_last_about_window_quits(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::show_about);
        assert!(cx.update(|cx| FreeCellApp::about_open(cx)));

        // Closing the About window when it is the only window clears its accounting and empties the
        // registry → the app quits (`functional_spec.md §4`; the `about_id` branch of
        // `on_window_closed` falls through to the quit-when-empty check).
        let handle = cx.update(|cx| cx.windows().into_iter().next().expect("about window open"));
        handle
            .update(cx, |_root, window, _appcx| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert!(
            !cx.update(|cx| FreeCellApp::about_open(cx)),
            "closing the About window clears its open accounting"
        );
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            0,
            "no windows remain after the last (About) window closes"
        );
    }

    #[gpui::test]
    fn close_dirty_prompts_and_cancel_keeps_window(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::new_workbook_detached);
        let handle = cx.update(|cx| FreeCellApp::nth_window_handle(cx, 0).unwrap());
        let entity = cx.update(|cx| FreeCellApp::nth_window(cx, 0).unwrap());

        // Make it dirty, then request a close → the unsaved-changes modal appears
        // (all synchronous — request_close sets the modal without a worker round-trip).
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.set_dirty_for_test(true, ctx);
                    w.request_close(window, ctx);
                });
            })
            .unwrap();
        assert!(cx.update(|cx| entity.read(cx).has_unsaved_modal()));
        assert_eq!(
            window_count(cx),
            1,
            "the window is not closed while prompting"
        );

        // Cancel dismisses the modal and keeps the window.
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| w.dismiss_modal_for_test(window, ctx));
            })
            .unwrap();
        assert!(!cx.update(|cx| entity.read(cx).has_unsaved_modal()));
        assert_eq!(window_count(cx), 1, "cancel keeps the window open");
    }

    #[gpui::test]
    fn clean_close_does_not_prompt(cx: &mut TestAppContext) {
        boot(cx);
        cx.update(FreeCellApp::new_workbook_detached);
        let handle = cx.update(|cx| FreeCellApp::nth_window_handle(cx, 0).unwrap());
        let entity = cx.update(|cx| FreeCellApp::nth_window(cx, 0).unwrap());
        // A clean window closing shows no modal (it proceeds straight to close).
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    assert!(w.on_titlebar_close(window, ctx), "clean close is allowed");
                });
            })
            .unwrap();
        assert!(!cx.update(|cx| entity.read(cx).has_unsaved_modal()));
    }

    /// A fresh **worker-less** document window (no OS thread under the deterministic scheduler),
    /// for direct `WorkerEvent` injection. A welcome window is kept open so that a save-then-close
    /// in these tests doesn't leave the registry empty and trigger `cx.quit()`.
    fn new_injectable_window(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<WorkbookWindow>) {
        cx.update(FreeCellApp::show_welcome);
        cx.update(FreeCellApp::new_workbook_detached);
        let handle = cx.update(|cx| FreeCellApp::nth_window_handle(cx, 0).unwrap());
        let entity = cx.update(|cx| FreeCellApp::nth_window(cx, 0).unwrap());
        (handle, entity)
    }

    #[gpui::test]
    fn saved_adopts_canonical_path_and_closes_after_save(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Saved.xlsx");
        write_xlsx(&path); // the file must exist for the handler's canonicalize() to resolve

        let (handle, entity) = new_injectable_window(cx);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    // Pretend the unsaved-changes "Save" path armed a close-on-save.
                    w.set_dirty_for_test(true, ctx);
                    w.arm_pending_save_for_test(path.clone(), 7, true);
                    w.inject_worker_event_for_test(
                        WorkerEvent::Saved {
                            req_id: 7,
                            ops_seen: 0,
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();

        let stored = cx.update(|cx| entity.read(cx).path().map(|p| p.to_path_buf()));
        assert_eq!(
            stored,
            Some(path.canonicalize().unwrap()),
            "Saved adopts the canonical path (so a later open dedupes)"
        );
        assert!(
            !cx.update(|cx| entity.read(cx).is_dirty()),
            "a successful save clears the dirty flag"
        );
        assert!(
            !cx.update(|cx| entity.read(cx).will_close_after_save()),
            "the close-on-save latch is consumed (the window was told to close)"
        );
    }

    #[gpui::test]
    fn first_save_of_opened_file_writes_back_backup_once(cx: &mut TestAppContext) {
        use crate::shell::lifecycle::backup_path;
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Budget.xlsx");
        write_xlsx(&path);
        let original = std::fs::read(&path).unwrap();

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        let entity = cx.update(|cx| FreeCellApp::nth_window(cx, 0).unwrap());
        let handle = cx.update(|cx| FreeCellApp::nth_window_handle(cx, 0).unwrap());
        let canonical = cx
            .update(|cx| entity.read(cx).path().map(|p| p.to_path_buf()))
            .unwrap();
        let backup = backup_path(&canonical);

        // First save of a disk-opened document backs up the original bytes.
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| w.save(false, window, ctx));
            })
            .unwrap();
        assert!(
            backup.exists(),
            "the first save-in-place creates <name>.back"
        );
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            original,
            "the backup holds the original bytes"
        );
        assert!(
            !cx.update(|cx| entity.read(cx).has_error_modal()),
            "a successful backup does not raise a dialog"
        );

        // Corrupt the backup, then save again — it must NOT be overwritten (write-once).
        std::fs::write(&backup, b"sentinel").unwrap();
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| w.save(false, window, ctx));
            })
            .unwrap();
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            b"sentinel",
            "a later save must not overwrite the existing backup"
        );
    }

    #[gpui::test]
    fn backup_failure_aborts_the_save_with_a_dialog(cx: &mut TestAppContext) {
        boot(cx);
        let dir = tempdir().unwrap();
        let path = dir.path().join("Budget.xlsx");
        write_xlsx(&path);

        cx.update(|cx| FreeCellApp::open_path_detached(&path, cx));
        let entity = cx.update(|cx| FreeCellApp::nth_window(cx, 0).unwrap());
        let handle = cx.update(|cx| FreeCellApp::nth_window_handle(cx, 0).unwrap());
        let canonical = cx
            .update(|cx| entity.read(cx).path().map(|p| p.to_path_buf()))
            .unwrap();

        // Remove the source so the pre-save `fs::copy` fails → the save aborts with a dialog.
        std::fs::remove_file(&canonical).unwrap();
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| w.save(false, window, ctx));
            })
            .unwrap();
        assert!(
            cx.update(|cx| entity.read(cx).has_error_modal()),
            "a backup failure surfaces the 'file not saved' dialog"
        );
        assert!(
            !crate::shell::lifecycle::backup_path(&canonical).exists(),
            "no backup is left behind when the copy fails"
        );
    }

    #[gpui::test]
    fn save_failed_keeps_window_and_shows_non_closing_error(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = new_injectable_window(cx);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.arm_pending_save_for_test(std::path::PathBuf::from("/x/Book.xlsx"), 3, true);
                    w.inject_worker_event_for_test(
                        WorkerEvent::SaveFailed {
                            req_id: 3,
                            error: SaveError::Io("disk full".into()),
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();

        assert!(
            cx.update(|cx| entity.read(cx).has_error_modal()),
            "a save failure shows the error dialog"
        );
        assert_eq!(
            cx.update(|cx| entity.read(cx).error_modal_closes_window_on_dismiss()),
            Some(false),
            "dismissing a save-failure dialog keeps the window (stays dirty + open, §5.2)"
        );
        assert!(
            !cx.update(|cx| entity.read(cx).will_close_after_save()),
            "a failed save aborts the pending close/quit follow-up"
        );
        assert_eq!(
            window_count(cx),
            1,
            "the window is not closed by a failed save"
        );
    }

    #[gpui::test]
    fn load_failed_shows_closing_error_and_clears_loading(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = new_injectable_window(cx);
        // Put the window in the "Opening …" state an OpenFile window starts in, so the
        // loading-clear assertion below actually exercises the LoadFailed arm (a NewWorkbook
        // window constructs with loading = None, which would make it vacuous).
        handle
            .update(cx, |_root, _window, appcx| {
                entity.update(appcx, |w, _ctx| {
                    w.set_loading_for_test(Some("Budget.xlsx".into()));
                });
            })
            .unwrap();
        assert!(
            cx.update(|cx| entity.read(cx).is_loading()),
            "precondition: the window is loading before the failure"
        );

        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(
                        WorkerEvent::LoadFailed {
                            error: LoadError::NotXlsx("not a zip container".into()),
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();

        assert!(
            cx.update(|cx| entity.read(cx).has_error_modal()),
            "a load failure shows the error dialog"
        );
        assert_eq!(
            cx.update(|cx| entity.read(cx).error_modal_closes_window_on_dismiss()),
            Some(true),
            "dismissing a load-failure dialog closes the window (§5.1)"
        );
        assert!(
            !cx.update(|cx| entity.read(cx).is_loading()),
            "the loading state is cleared on load failure"
        );
    }

    // ---- Phase 11: composed grid + chrome + worker-event routing ---------------------------
    //
    // These drive the window's *folding* logic by **injecting** worker events (fully synchronous —
    // `reconcile_sheets` / `on_edit_rejected` / `Published` / the grid→chrome selection route run
    // against the sibling entities without deferral or worker observation). Windows are worker-less
    // (`new_injectable_window` → detached client), so the required suite is deterministic. The
    // deferred chrome→grid follow-ups (`MoveActive`/`SetActiveSheet`) + worker-command emission are
    // the documented untestable boundary (Phase-8/9 component tests + the Xvfb smoke).

    use freecell_core::data_row::FieldMode;
    use freecell_core::input_cap::InputRejection;
    use freecell_core::{CellRef, SelectionModel, SheetId};
    use freecell_engine::{EditRejectedReason, SheetMeta};

    fn sheet_meta(id: u32, name: &str, has_content: bool) -> SheetMeta {
        SheetMeta {
            id: SheetId(id),
            name: name.into(),
            has_content,
        }
    }

    /// A worker-less document window with a synthesized `Loaded` already folded in.
    fn loaded_window(
        cx: &mut TestAppContext,
        sheets: Vec<SheetMeta>,
    ) -> (AnyWindowHandle, Entity<WorkbookWindow>) {
        let (handle, entity) = new_injectable_window(cx);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(WorkerEvent::Loaded { sheets }, window, ctx);
                });
            })
            .unwrap();
        (handle, entity)
    }

    #[gpui::test]
    fn loaded_populates_tabs_and_switches_active_sheet(cx: &mut TestAppContext) {
        boot(cx);
        let (_h, entity) = loaded_window(
            cx,
            vec![sheet_meta(3, "Data", false), sheet_meta(5, "Notes", false)],
        );
        // Window, grid, and chrome all adopt the first loaded sheet.
        assert_eq!(
            cx.update(|cx| entity.read(cx).active_sheet_for_test()),
            SheetId(3)
        );
        let grid = cx.update(|cx| entity.read(cx).grid_for_test());
        assert_eq!(cx.update(|cx| grid.read(cx).active_sheet()), SheetId(3));
        let chrome = cx.update(|cx| entity.read(cx).chrome_for_test());
        assert_eq!(cx.update(|cx| chrome.read(cx).active_sheet()), SheetId(3));
        let names: Vec<String> = cx.update(|cx| {
            chrome
                .read(cx)
                .sheets()
                .iter()
                .map(|t| t.name.clone())
                .collect()
        });
        assert_eq!(names, vec!["Data".to_string(), "Notes".to_string()]);
        assert!(!cx.update(|cx| entity.read(cx).is_loading()));
    }

    #[gpui::test]
    fn sheets_changed_add_switches_to_new_sheet(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = loaded_window(cx, vec![sheet_meta(3, "Data", false)]);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(
                        WorkerEvent::SheetsChanged {
                            sheets: vec![
                                sheet_meta(3, "Data", false),
                                sheet_meta(9, "Sheet2", false),
                            ],
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        // The newly-added sheet becomes active (`functional_spec.md §3.7`) — on the window AND on
        // the chrome. The chrome's active sheet is load-bearing: it scopes every command/fetch, so
        // if it lagged (the CRITICAL bug) edits would route to the OLD sheet.
        assert_eq!(
            cx.update(|cx| entity.read(cx).active_sheet_for_test()),
            SheetId(9)
        );
        let chrome = cx.update(|cx| entity.read(cx).chrome_for_test());
        assert_eq!(
            cx.update(|cx| chrome.read(cx).active_sheet()),
            SheetId(9),
            "the chrome must adopt the added sheet, else edits route to the old sheet"
        );
    }

    #[gpui::test]
    fn sheets_changed_delete_active_falls_back_to_first(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = loaded_window(
            cx,
            vec![sheet_meta(3, "Data", false), sheet_meta(5, "Notes", false)],
        );
        // The active sheet (3) is deleted → fall back to the first remaining (5), window + chrome.
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(
                        WorkerEvent::SheetsChanged {
                            sheets: vec![sheet_meta(5, "Notes", false)],
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        assert_eq!(
            cx.update(|cx| entity.read(cx).active_sheet_for_test()),
            SheetId(5)
        );
        let chrome = cx.update(|cx| entity.read(cx).chrome_for_test());
        assert_eq!(cx.update(|cx| chrome.read(cx).active_sheet()), SheetId(5));
    }

    #[gpui::test]
    fn edit_rejected_engine_panic_shows_transient_dialog(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = new_injectable_window(cx);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(
                        WorkerEvent::EditRejected {
                            reason: EditRejectedReason::EnginePanic,
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        assert!(
            cx.update(|cx| entity.read(cx).has_error_modal()),
            "a caught engine panic surfaces the transient error dialog"
        );
        assert_eq!(
            cx.update(|cx| entity.read(cx).error_modal_closes_window_on_dismiss()),
            Some(false),
            "the document is intact — dismissing keeps the window (§6)"
        );
    }

    #[gpui::test]
    fn edit_rejected_input_cap_flags_chrome_data_row(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = loaded_window(cx, vec![sheet_meta(3, "Data", false)]);
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(
                        WorkerEvent::EditRejected {
                            reason: EditRejectedReason::InputCap(InputRejection::TooLong {
                                len: 9000,
                                max: 8192,
                            }),
                        },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        let chrome = cx.update(|cx| entity.read(cx).chrome_for_test());
        assert!(
            cx.update(|cx| chrome.read(cx).cap_error_visible()),
            "a worker cap rejection lights the data-row danger state"
        );
    }

    #[gpui::test]
    fn published_and_style_cache_updated_are_folded(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = loaded_window(cx, vec![sheet_meta(3, "Data", false)]);
        // Both repaint-class events fold without panicking (grid notify + chrome style refresh).
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.inject_worker_event_for_test(WorkerEvent::Published, window, ctx);
                    w.inject_worker_event_for_test(
                        WorkerEvent::StyleCacheUpdated { sheet: SheetId(3) },
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        assert_eq!(
            cx.update(|cx| entity.read(cx).active_sheet_for_test()),
            SheetId(3)
        );
    }

    #[gpui::test]
    fn grid_selection_routes_to_chrome_ref_box(cx: &mut TestAppContext) {
        boot(cx);
        let (handle, entity) = loaded_window(cx, vec![sheet_meta(3, "Data", false)]);
        // A grid selection (as its sink delivers it) drives the chrome ref box + a content fetch
        // (single cell → the field goes Idle and awaits the reply).
        handle
            .update(cx, |_root, window, appcx| {
                entity.update(appcx, |w, ctx| {
                    w.route_selection_changed_for_test(
                        SelectionModel::single(CellRef::new(6, 1)),
                        window,
                        ctx,
                    );
                });
            })
            .unwrap();
        let chrome = cx.update(|cx| entity.read(cx).chrome_for_test());
        assert_eq!(cx.update(|cx| chrome.read(cx).ref_box_text()), "B7");
        assert_eq!(cx.update(|cx| chrome.read(cx).data_mode()), FieldMode::Idle);
    }

    /// Opens the in-cell editor over `cell` in the Editing state with `text` pending: `begin_typed`
    /// forces Editing on the active cell, then `begin_in_cell` promotes that pending text into the
    /// focused overlay (keeping it) — the deterministic way to reach "focused in-cell editor mid-
    /// edit" without relying on headless IME text insertion.
    fn open_incell_editing(
        window_entity: &Entity<WorkbookWindow>,
        cx: &mut gpui::VisualTestContext,
        text: &str,
        cell: CellRef,
    ) {
        let chrome = cx.update(|_window, cx| window_entity.read(cx).chrome_for_test());
        cx.update(|window, cx| {
            chrome.update(cx, |c, ctx| {
                // `begin_typed` forces Editing on the active cell; `begin_in_cell` promotes that
                // pending text into the focused overlay — the deterministic way to reach "focused
                // in-cell editor mid-edit" without relying on headless IME text insertion.
                c.begin_typed(text, window, ctx);
                c.begin_in_cell(cell, window, ctx);
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn in_cell_key_commands_do_not_reenter_grid_update(cx: &mut TestAppContext) {
        use freecell_core::selection::Direction;
        use gpui::Focusable as _;

        // BUG #5 (re-entrant `GridView` update abort). An in-cell Tab/Escape is emitted by the grid
        // root's `capture_key_down` listener — i.e. from *inside* the grid's own `update` lease
        // (`cx.listener` == `grid.update`). It routes to the chrome, which commits/cancels and asks
        // the grid to take focus back. If that `FocusGrid` re-focused the grid synchronously it would
        // re-enter the in-flight grid update and abort ("cannot update GridView while it is already
        // being updated"); the fix DEFERS it. Here we drive the real chrome↔grid sinks and reproduce
        // the exact lease condition by emitting the same `GridEvent`s the capture handler emits, from
        // within `grid.update`. (A real keystroke can't be used: the headless key-dispatch path does
        // not route through the nested + `deferred()` overlay input to this grid ancestor — on macOS
        // the key arrives via `do_command_by_selector`, which is the path the user's crash came from.)
        boot(cx);
        let (handle, entity) = loaded_window(cx, vec![sheet_meta(3, "Data", false)]);
        let grid = cx.update(|cx| entity.read(cx).grid_for_test());
        let mut vcx = gpui::VisualTestContext::from_window(handle, cx);
        vcx.run_until_parked();

        // --- Tab: commit + move, emitted while the grid is leased. ---
        open_incell_editing(&entity, &mut vcx, "7", CellRef::new(0, 0));
        assert_eq!(
            vcx.update(|_w, cx| grid.read(cx).incell_open_for_test()),
            Some(CellRef::new(0, 0)),
            "the in-cell overlay is open before Tab"
        );
        vcx.update(|window, cx| {
            grid.update(cx, |g, cx| {
                g.emit_incell_commit_move_for_test(Direction::Right, window, cx)
            });
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_w, cx| grid.read(cx).incell_open_for_test()),
            None,
            "Tab commits + closes the overlay; the commit's deferred FocusGrid did not re-enter the \
             grid update"
        );

        // --- Escape: cancel, emitted while the grid is leased (active cell moved to B1 after Tab). ---
        let active = vcx.update(|_w, cx| grid.read(cx).selection().active);
        open_incell_editing(&entity, &mut vcx, "8", active);
        assert_eq!(
            vcx.update(|_w, cx| grid.read(cx).incell_open_for_test()),
            Some(active),
            "the in-cell overlay is open before Escape"
        );
        vcx.update(|window, cx| {
            grid.update(cx, |g, cx| g.emit_incell_cancel_for_test(window, cx));
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_w, cx| grid.read(cx).incell_open_for_test()),
            None,
            "Escape cancels + closes the overlay; the cancel's deferred FocusGrid did not re-enter \
             the grid update"
        );

        // The deferred FocusGrid landed focus back on the grid.
        let grid_focused =
            vcx.update(|window, cx| grid.read(cx).focus_handle(cx).is_focused(window));
        assert!(
            grid_focused,
            "focus returns to the grid after an in-cell key command (deferred FocusGrid landed)"
        );
    }
}
