//! The menu bar (macOS only) and the per-platform key bindings
//! (`components/app_shell.md §Menus & actions`, `functional_spec.md §2.4`).
//!
//! One action list, two keymaps: the same actions bind to `cmd-*` on macOS and `ctrl-*` on
//! Linux (`functional_spec.md §1, §2.4`: "Ctrl replaces Cmd throughout"). Linux has **no
//! native global menu bar** at the pinned gpui rev, so the menu bar is macOS-only; every menu
//! action stays reachable through its keyboard shortcut.

use gpui::{App, KeyBinding, Menu, MenuItem};

use super::{
    About, CloseWindow, NewWorkbook, OpenFile, Quit, Redo, Save, SaveAs, ToggleBold, ToggleItalic,
    ToggleUnderline, Undo,
};

/// The primary modifier for the current platform: `cmd` on macOS, `ctrl` elsewhere.
fn primary() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}

/// Registers the app's key bindings against the current platform's primary modifier
/// (`functional_spec.md §2.4`). Called once at startup, before any window opens.
pub fn bind_keys(cx: &mut App) {
    let m = primary();
    let key = |suffix: &str| format!("{m}-{suffix}");
    cx.bind_keys([
        KeyBinding::new(&key("n"), NewWorkbook, None),
        KeyBinding::new(&key("o"), OpenFile, None),
        KeyBinding::new(&key("s"), Save, None),
        KeyBinding::new(&key("shift-s"), SaveAs, None),
        KeyBinding::new(&key("w"), CloseWindow, None),
        KeyBinding::new(&key("z"), Undo, None),
        KeyBinding::new(&key("shift-z"), Redo, None),
        KeyBinding::new(&key("b"), ToggleBold, None),
        KeyBinding::new(&key("i"), ToggleItalic, None),
        KeyBinding::new(&key("u"), ToggleUnderline, None),
        KeyBinding::new(&key("q"), Quit, None),
    ]);
}

/// The macOS menu bar (`functional_spec.md §2.4`). The key equivalents shown next to each
/// item are resolved from the keymap by gpui (`set_menus` receives the app keymap), so they
/// stay in sync with [`bind_keys`]. Menu items enable/disable by whether a handler is in
/// scope: Save / Undo / … are handled on the `WorkbookWindow`, so they grey out when the
/// Welcome window is frontmost.
pub fn build_menus() -> Vec<Menu> {
    vec![
        Menu::new("FreeCell").items([
            MenuItem::action("About FreeCell", About),
            MenuItem::separator(),
            MenuItem::action("Quit FreeCell", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New", NewWorkbook),
            MenuItem::action("Open…", OpenFile),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Save As…", SaveAs),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
        ]),
    ]
}

/// Installs the menu bar on macOS; a no-op elsewhere (`functional_spec.md §2.4`: no menu bar
/// on Linux in the MVP).
pub fn install_menus(cx: &App) {
    if cfg!(target_os = "macos") {
        cx.set_menus(build_menus());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_has_the_three_specced_menus() {
        let menus = build_menus();
        let names: Vec<_> = menus.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(names, vec!["FreeCell", "File", "Edit"]);
    }

    #[test]
    fn primary_modifier_is_platform_appropriate() {
        let expected = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(primary(), expected);
    }
}
