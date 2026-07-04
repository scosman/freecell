//! The window registry — the pure, gpui-free bookkeeping behind the app's multi-window
//! lifecycle (`components/app_shell.md §Lifecycle rules`, `functional_spec.md §2`).
//!
//! It tracks the open workbook windows (their canonical path + dirty flag) and whether the
//! welcome window is up, and answers the three lifecycle questions the GPUI layer asks:
//!
//! 1. **Open dedupe** — is a file already open (`resolve_open`)? (`functional_spec.md §5.1`).
//! 2. **Quit-when-empty** — did the last window just close (`is_empty`)? The welcome window
//!    counts toward the open count, so closing it with no workbook windows also quits
//!    (`functional_spec.md §2`).
//! 3. **Quit prompt order** — which open windows are dirty, front-to-back (`dirty_among`)?
//!
//! Windows are identified by an opaque [`WindowKey`] the registry assigns; the GPUI layer
//! keeps the `WindowKey ↔ gpui WindowId` map so the pure logic never names a gpui type and
//! is unit-testable without a windowing system.

use std::path::{Path, PathBuf};

/// An opaque per-window identity assigned by [`WindowRegistry::register`]. The GPUI layer
/// maps this 1:1 onto a gpui `WindowId`. Deliberately *not* a `WindowId` so the registry
/// stays gpui-free and headlessly testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowKey(pub u64);

/// One workbook window as the registry tracks it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowRecord {
    key: WindowKey,
    /// The canonical path of the open file, or `None` for an unsaved (`Untitled`) or
    /// still-loading window. Dedupe (`resolve_open`) only matches `Some` paths.
    path: Option<PathBuf>,
    dirty: bool,
}

/// The outcome of an open request — open dedupes by canonical path (`functional_spec.md
/// §5.1`: opening an already-open file focuses the existing window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A window already has this path open — activate it instead of opening a duplicate.
    Activate(WindowKey),
    /// No open window has this path — create a new one.
    OpenNew,
}

/// The pure window bookkeeping. Owns no gpui state; the GPUI `FreeCellApp` global wraps it.
#[derive(Debug, Default)]
pub struct WindowRegistry {
    windows: Vec<WindowRecord>,
    welcome_open: bool,
    next_key: u64,
}

impl WindowRegistry {
    /// An empty registry (no windows, welcome not yet shown).
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new workbook window with an initial `path` (`None` for `Untitled` or a
    /// window still loading a file whose canonical path is set later via [`set_path`]).
    /// Returns the assigned [`WindowKey`].
    ///
    /// [`set_path`]: Self::set_path
    pub fn register(&mut self, path: Option<PathBuf>) -> WindowKey {
        let key = WindowKey(self.next_key);
        self.next_key += 1;
        self.windows.push(WindowRecord {
            key,
            path,
            dirty: false,
        });
        key
    }

    /// Removes a workbook window (it closed). No-op if the key is unknown.
    pub fn remove(&mut self, key: WindowKey) {
        self.windows.retain(|w| w.key != key);
    }

    /// Whether the registry currently tracks `key`.
    pub fn contains(&self, key: WindowKey) -> bool {
        self.windows.iter().any(|w| w.key == key)
    }

    /// Sets (or clears) a window's canonical path — used once a `Save As` picks a
    /// destination or a load resolves its real path (so a later open dedupes against it).
    pub fn set_path(&mut self, key: WindowKey, path: Option<PathBuf>) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.key == key) {
            w.path = path;
        }
    }

    /// This window's canonical path, if it has one.
    pub fn path(&self, key: WindowKey) -> Option<&Path> {
        self.windows
            .iter()
            .find(|w| w.key == key)
            .and_then(|w| w.path.as_deref())
    }

    /// Sets a window's dirty flag (drives the quit / close prompt).
    pub fn set_dirty(&mut self, key: WindowKey, dirty: bool) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.key == key) {
            w.dirty = dirty;
        }
    }

    /// Whether a window has unsaved changes.
    pub fn is_dirty(&self, key: WindowKey) -> bool {
        self.windows
            .iter()
            .find(|w| w.key == key)
            .map(|w| w.dirty)
            .unwrap_or(false)
    }

    /// Resolves an open request against the already-open windows. `path` must be canonical
    /// (the caller canonicalizes; the registry compares stored canonical paths).
    pub fn resolve_open(&self, path: &Path) -> OpenOutcome {
        self.windows
            .iter()
            .find(|w| w.path.as_deref() == Some(path))
            .map(|w| OpenOutcome::Activate(w.key))
            .unwrap_or(OpenOutcome::OpenNew)
    }

    /// Marks the welcome window as open/closed. It counts toward [`open_count`] so that
    /// closing it with no workbook windows quits the app (`functional_spec.md §2`).
    ///
    /// [`open_count`]: Self::open_count
    pub fn set_welcome_open(&mut self, open: bool) {
        self.welcome_open = open;
    }

    /// Whether the welcome window is currently open.
    pub fn welcome_open(&self) -> bool {
        self.welcome_open
    }

    /// The number of workbook windows currently registered.
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// The total open-window count = workbook windows + the welcome window (if up). The app
    /// quits when this reaches zero.
    pub fn open_count(&self) -> usize {
        self.windows.len() + usize::from(self.welcome_open)
    }

    /// Whether no windows remain open at all — the app should quit
    /// (`components/app_shell.md`: "the registry quits the app when its window count reaches
    /// zero").
    pub fn is_empty(&self) -> bool {
        self.open_count() == 0
    }

    /// The dirty windows among `order`, preserving that order. The GPUI layer supplies the
    /// front-to-back window order (from `window_stack`) so quit prompts appear front-to-back
    /// (`components/app_shell.md §Lifecycle rules`, `functional_spec.md §2.3`).
    pub fn dirty_among(&self, order: &[WindowKey]) -> Vec<WindowKey> {
        order
            .iter()
            .copied()
            .filter(|key| self.is_dirty(*key))
            .collect()
    }

    /// Every registered window key (unordered) — the fallback quit order when the platform
    /// can't report a window stack.
    pub fn keys(&self) -> Vec<WindowKey> {
        self.windows.iter().map(|w| w.key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn registers_and_assigns_distinct_keys() {
        let mut reg = WindowRegistry::new();
        let a = reg.register(None);
        let b = reg.register(Some(p("/x/a.xlsx")));
        assert_ne!(a, b);
        assert_eq!(reg.window_count(), 2);
        assert!(reg.contains(a));
        assert!(reg.contains(b));
    }

    #[test]
    fn resolve_open_dedupes_by_path() {
        let mut reg = WindowRegistry::new();
        let key = reg.register(Some(p("/books/budget.xlsx")));
        assert_eq!(
            reg.resolve_open(&p("/books/budget.xlsx")),
            OpenOutcome::Activate(key)
        );
    }

    #[test]
    fn resolve_open_opens_new_when_absent() {
        let mut reg = WindowRegistry::new();
        reg.register(Some(p("/books/a.xlsx")));
        assert_eq!(reg.resolve_open(&p("/books/b.xlsx")), OpenOutcome::OpenNew);
        // An untitled window never dedupes (no path).
        reg.register(None);
        assert_eq!(reg.resolve_open(&p("/books/b.xlsx")), OpenOutcome::OpenNew);
    }

    #[test]
    fn set_path_then_dedupes() {
        let mut reg = WindowRegistry::new();
        let key = reg.register(None); // still loading / untitled
        assert_eq!(reg.resolve_open(&p("/loaded.xlsx")), OpenOutcome::OpenNew);
        reg.set_path(key, Some(p("/loaded.xlsx")));
        assert_eq!(
            reg.resolve_open(&p("/loaded.xlsx")),
            OpenOutcome::Activate(key)
        );
        assert_eq!(reg.path(key), Some(p("/loaded.xlsx").as_path()));
    }

    #[test]
    fn welcome_counts_toward_open_count() {
        let mut reg = WindowRegistry::new();
        assert!(reg.is_empty());
        reg.set_welcome_open(true);
        assert_eq!(reg.open_count(), 1);
        assert!(!reg.is_empty());
        reg.set_welcome_open(false);
        assert!(reg.is_empty());
    }

    #[test]
    fn empty_only_when_no_windows_and_no_welcome() {
        let mut reg = WindowRegistry::new();
        reg.set_welcome_open(true);
        let w = reg.register(Some(p("/a.xlsx")));
        reg.set_welcome_open(false); // welcome closes when the workbook opens
        assert!(!reg.is_empty());
        reg.remove(w);
        assert!(reg.is_empty(), "last window closing → app quits");
    }

    #[test]
    fn dirty_tracking_and_prompt_order() {
        let mut reg = WindowRegistry::new();
        let a = reg.register(Some(p("/a.xlsx")));
        let b = reg.register(Some(p("/b.xlsx")));
        let c = reg.register(Some(p("/c.xlsx")));
        reg.set_dirty(a, true);
        reg.set_dirty(c, true);
        assert!(reg.is_dirty(a) && !reg.is_dirty(b) && reg.is_dirty(c));
        // Front-to-back order c, b, a → dirty subset preserves it as c, a.
        assert_eq!(reg.dirty_among(&[c, b, a]), vec![c, a]);
        // Clearing dirty drops it from the prompt set.
        reg.set_dirty(a, false);
        assert_eq!(reg.dirty_among(&[c, b, a]), vec![c]);
    }

    #[test]
    fn remove_is_idempotent_and_clears_dedupe() {
        let mut reg = WindowRegistry::new();
        let a = reg.register(Some(p("/a.xlsx")));
        reg.remove(a);
        reg.remove(a); // no panic
        assert!(!reg.contains(a));
        assert_eq!(reg.resolve_open(&p("/a.xlsx")), OpenOutcome::OpenNew);
    }
}
