//! The application shell (`components/app_shell.md`, `functional_spec.md §2`): app entry +
//! window registry, the welcome window, the per-document [`WorkbookWindow`], the menu bar +
//! key bindings, the dialogs, and the save / close / quit flows.
//!
//! Phase 10 built the *shell* — the window/menu/dialog plumbing and the lifecycle around each
//! document window; **Phase 11** composed the grid + chrome + worker *inside* each
//! [`WorkbookWindow`] (worker-event routing, grid/chrome coupling, selection/viewport wiring),
//! replacing the Phase-10 placeholder body.
//!
//! **Pure vs GPUI.** The lifecycle *decisions* — window dedupe, quit-when-empty, dirty
//! accounting, save targeting, quit-prompt ordering — live in the gpui-free [`registry`] and
//! [`lifecycle`] modules and are unit-tested headlessly. This module's GPUI submodules
//! ([`app`], `welcome`, `window`, [`menus`]) are the thin plumbing that performs those
//! decisions against real windows, menus, panels, and dialogs.

pub mod assets;
pub mod lifecycle;
pub mod menus;
pub mod recents;
pub mod registry;
pub mod titlebar;

mod about;
mod app;
mod clipboard;
mod fonts;
mod welcome;
mod window;

use std::path::PathBuf;

use gpui::actions;

pub use app::FreeCellApp;
pub use assets::AppAssets;
pub use fonts::register_fonts;
pub use welcome::WelcomeView;
pub use window::WorkbookWindow;

// The single source of truth for the app's actions (`components/app_shell.md §Menus &
// actions`). The macOS menu bar and the per-platform key bindings both dispatch these exact
// names; New/Open/About/Quit are handled globally (available from any window, incl. Welcome),
// while Save/SaveAs/CloseWindow/Undo/Redo/ToggleBold/Italic/Underline are handled on the
// `WorkbookWindow` (so they are naturally disabled when Welcome is frontmost — no handler in
// scope = disabled menu item).
actions!(
    freecell,
    [
        /// Create a new empty workbook in a new window.
        NewWorkbook,
        /// Open an `.xlsx` file (native panel).
        OpenFile,
        /// Save the focused workbook (Save As if it has no path).
        Save,
        /// Save the focused workbook to a new path (native panel).
        SaveAs,
        /// Close the focused window (prompts if dirty).
        CloseWindow,
        /// Undo the last edit in the focused workbook.
        Undo,
        /// Redo the last undone edit in the focused workbook.
        Redo,
        /// Toggle bold over the focused workbook's selection.
        ToggleBold,
        /// Toggle italic over the focused workbook's selection.
        ToggleItalic,
        /// Toggle underline over the focused workbook's selection.
        ToggleUnderline,
        /// Quit the application (prompts each dirty window).
        Quit,
        /// Show the About window.
        About,
        /// Clear the recent-files list (Open Recent → Clear Recent Files).
        ClearRecent,
    ]
);

/// Open a specific recent file (`architecture.md §5`). Carries the file's **`PathBuf`** — the
/// exact file to open — not an index into the self-pruning `display_entries` snapshot: its
/// handler dispatches the path straight to [`FreeCellApp::open_path`], which dedupes/opens it or
/// shows the vanished-file dialog (`§6`) if it moved. An index would open the *wrong* file if an
/// earlier recent file were deleted between menu install and click. Carrying a `PathBuf` keeps it
/// `Clone + PartialEq`, so it uses gpui's data-carrying `#[derive(Action)]` with
/// `#[action(no_json)]` (dispatched only programmatically from the menu bar, never built from
/// keymap JSON — so it needs neither `serde::Deserialize` nor `schemars::JsonSchema`).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = freecell, no_json)]
pub struct OpenRecent {
    /// The exact file to open.
    pub path: PathBuf,
}
