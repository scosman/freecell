//! The menu bar (macOS only) and the per-platform key bindings
//! (`components/app_shell.md §Menus & actions`, `functional_spec.md §2.4`).
//!
//! One action list, two keymaps: the same actions bind to `cmd-*` on macOS and `ctrl-*` on
//! Linux (`functional_spec.md §1, §2.4`: "Ctrl replaces Cmd throughout"). Linux has **no
//! native global menu bar** at the pinned gpui rev, so the menu bar is macOS-only; every menu
//! action stays reachable through its keyboard shortcut.

use gpui::{App, KeyBinding, Menu, MenuItem};

use freecell_core::recent::{RecentList, MENU_LIMIT};

use super::{
    recents, About, ClearRecent, CloseWindow, ExportCsv, NewWorkbook, OpenFile, OpenFind,
    OpenRecent, Quit, Redo, Save, SaveAs, ToggleBold, ToggleItalic, ToggleUnderline, Undo,
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
        KeyBinding::new(&key("f"), OpenFind, None),
        KeyBinding::new(&key("q"), Quit, None),
    ]);
}

/// The macOS menu bar (`functional_spec.md §2.4`). The key equivalents shown next to each
/// item are resolved from the keymap by gpui (`set_menus` receives the app keymap), so they
/// stay in sync with [`bind_keys`]. Menu items enable/disable by whether a handler is in
/// scope: Save / Undo / … are handled on the `WorkbookWindow`, so they grey out when the
/// Welcome window is frontmost.
///
/// The **File** menu carries an **Open Recent** submenu built from `recents` (positioned after
/// "Open…", `functional_spec.md §3`). `now` (injected — the sole wall-clock read lives in
/// [`recents::now_unix_secs`]) only feeds `display_entries`; each item carries its file's
/// `PathBuf` (`architecture.md §5`: no snapshot-index coupling), and the submenu's item labels,
/// order, and count are time-independent, so the exact instant never affects which file an item
/// opens.
pub fn build_menus(recents: &RecentList, now: i64) -> Vec<Menu> {
    vec![
        Menu::new("FreeCell").items([
            MenuItem::action("About FreeCell", About),
            MenuItem::separator(),
            MenuItem::action("Quit FreeCell", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New", NewWorkbook),
            MenuItem::action("Open…", OpenFile),
            MenuItem::submenu(open_recent_submenu(recents, now)),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Save As…", SaveAs),
            MenuItem::action("Export as CSV…", ExportCsv),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
            MenuItem::separator(),
            MenuItem::action("Find…", OpenFind),
        ]),
    ]
}

/// The **Open Recent** submenu (`functional_spec.md §3`): up to [`MENU_LIMIT`] existing files
/// (name-only labels, most-recent-first) each dispatching `OpenRecent { path }` for that file,
/// then a separator and **Clear Recent Files**. When there are no existing recent files the
/// submenu is a single **disabled** `No Recent Files` placeholder (standard macOS behaviour) and
/// no Clear item. Missing/moved files are already pruned by `display_entries`.
fn open_recent_submenu(recents: &RecentList, now: i64) -> Menu {
    let rows = recents.display_entries(now, MENU_LIMIT);
    let items = if rows.is_empty() {
        // No action fires while the item is disabled; `ClearRecent` is just a harmless binding.
        vec![MenuItem::action("No Recent Files", ClearRecent).disabled(true)]
    } else {
        let mut items: Vec<MenuItem> = rows
            .into_iter()
            .map(|row| MenuItem::action(row.name, OpenRecent { path: row.path }))
            .collect();
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Clear Recent Files", ClearRecent));
        items
    };
    Menu::new("Open Recent").items(items)
}

/// Installs the menu bar on macOS; a no-op elsewhere (`functional_spec.md §2.4`: no menu bar
/// on Linux in the MVP). Called from `init` and whenever the recent-files store changes
/// (`FreeCellApp::refresh_recents_ui`), so **Open Recent** always reflects the current list.
pub fn install_menus_with(recents: &RecentList, cx: &App) {
    if cfg!(target_os = "macos") {
        cx.set_menus(build_menus(recents, recents::now_unix_secs()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_core::recent::RecentEntry;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    // The menu labels/order/count are time-independent (relative-time is unused by menu items),
    // so any fixed instant builds a deterministic snapshot.
    const NOW: i64 = 1_800_000_000;

    /// Creates `names` as real files under `dir` and returns a most-recent-first `RecentList`
    /// over them (so `display_entries` finds live files to stat).
    fn recent_list_over(dir: &Path, names: &[&str]) -> RecentList {
        let entries = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let path = dir.join(name);
                std::fs::write(&path, b"x").expect("write temp recent file");
                RecentEntry {
                    path,
                    last_opened: NOW - i as i64,
                }
            })
            .collect();
        RecentList { entries }
    }

    fn menu_named<'a>(menus: &'a [Menu], name: &str) -> &'a Menu {
        menus
            .iter()
            .find(|m| m.name.as_ref() == name)
            .unwrap_or_else(|| panic!("{name} menu present"))
    }

    fn open_recent_submenu(menus: &[Menu]) -> &Menu {
        menu_named(menus, "File")
            .items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(menu) if menu.name.as_ref() == "Open Recent" => Some(menu),
                _ => None,
            })
            .expect("Open Recent submenu present")
    }

    /// The action-item labels of `menu`, in order (skips separators/submenus).
    fn action_labels(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    /// The `OpenRecent` target paths carried by `menu`'s action items, in order — the exact file
    /// each item dispatches to `open_path`.
    fn open_recent_paths(menu: &Menu) -> Vec<PathBuf> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { action, .. } => action
                    .as_any()
                    .downcast_ref::<OpenRecent>()
                    .map(|open| open.path.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn menu_bar_has_the_three_specced_menus() {
        let menus = build_menus(&RecentList::default(), NOW);
        let names: Vec<_> = menus.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(names, vec!["FreeCell", "File", "Edit"]);
    }

    #[test]
    fn file_menu_has_csv_export_but_no_dedicated_import_item() {
        // CSV *import* has no dedicated menu item — the "Open…" panel already accepts a `.csv`
        // (routed to an untitled import by extension), so a separate "Import CSV…" item is
        // redundant. Export keeps its own item (window-scoped, like Save As).
        let menus = build_menus(&RecentList::default(), NOW);
        let labels = action_labels(menu_named(&menus, "File"));
        assert!(
            labels.contains(&"Export as CSV…".to_string()),
            "File menu offers Export as CSV…: {labels:?}"
        );
        assert!(
            !labels.contains(&"Import CSV…".to_string()),
            "File menu should NOT offer a dedicated Import CSV… item (Open handles .csv): {labels:?}"
        );
    }

    #[test]
    fn file_menu_places_open_recent_after_open() {
        let menus = build_menus(&RecentList::default(), NOW);
        let file = menu_named(&menus, "File");
        // "Open Recent" sits at the index immediately after the "Open…" action.
        let open_idx = file
            .items
            .iter()
            .position(
                |item| matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Open…"),
            )
            .expect("Open… item present");
        assert!(
            matches!(&file.items[open_idx + 1], MenuItem::Submenu(menu) if menu.name.as_ref() == "Open Recent"),
            "Open Recent submenu directly follows Open…"
        );
    }

    #[test]
    fn open_recent_lists_existing_files_capped_and_labelled() {
        let dir = TempDir::new().unwrap();
        // 11 files (> MENU_LIMIT) so the cap is exercised.
        let names: Vec<String> = (0..=MENU_LIMIT).map(|i| format!("book{i}.xlsx")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let list = recent_list_over(dir.path(), &name_refs);

        let menus = build_menus(&list, NOW);
        let submenu = open_recent_submenu(&menus);
        let labels = action_labels(submenu);

        // Exactly MENU_LIMIT file items (capped), most-recent-first, then "Clear Recent Files".
        assert_eq!(labels.len(), MENU_LIMIT + 1);
        assert_eq!(labels[..MENU_LIMIT], name_refs[..MENU_LIMIT]);
        assert_eq!(labels.last().unwrap(), "Clear Recent Files");
        // Each item carries its own file path (the capped, most-recent-first prefix).
        let expected_paths: Vec<PathBuf> = name_refs[..MENU_LIMIT]
            .iter()
            .map(|name| dir.path().join(name))
            .collect();
        assert_eq!(open_recent_paths(submenu), expected_paths);
        // A separator precedes the Clear item.
        assert!(
            submenu
                .items
                .iter()
                .any(|item| matches!(item, MenuItem::Separator)),
            "a separator divides the files from Clear Recent Files"
        );
        assert!(
            submenu.items.iter().all(|item| !item.is_disabled()),
            "populated Open Recent items are all enabled"
        );
    }

    #[test]
    fn open_recent_empty_shows_disabled_placeholder() {
        let menus = build_menus(&RecentList::default(), NOW);
        let submenu = open_recent_submenu(&menus);
        assert_eq!(submenu.items.len(), 1, "just the placeholder");
        match &submenu.items[0] {
            MenuItem::Action { name, disabled, .. } => {
                assert_eq!(name.as_ref(), "No Recent Files");
                assert!(disabled, "the empty-state placeholder is disabled");
            }
            _ => panic!("the placeholder is a disabled action item"),
        }
    }

    #[test]
    fn open_recent_omits_missing_files() {
        let dir = TempDir::new().unwrap();
        let list = recent_list_over(dir.path(), &["present.xlsx", "gone.xlsx"]);
        // Delete one recorded file without re-recording — display-time pruning must drop it.
        std::fs::remove_file(dir.path().join("gone.xlsx")).unwrap();

        let menus = build_menus(&list, NOW);
        let labels = action_labels(open_recent_submenu(&menus));
        assert_eq!(
            labels,
            vec!["present.xlsx".to_string(), "Clear Recent Files".to_string()]
        );
    }

    #[test]
    fn open_recent_items_carry_the_exact_file_path() {
        // Each item dispatches `OpenRecent { path }` for its own file (most-recent-first), so a
        // click opens THAT file via `open_path` — never a wrong file re-derived from a shifted
        // index into the self-pruning display list.
        let dir = TempDir::new().unwrap();
        let list = recent_list_over(dir.path(), &["a.xlsx", "b.xlsx"]);
        let menus = build_menus(&list, NOW);
        assert_eq!(
            open_recent_paths(open_recent_submenu(&menus)),
            vec![dir.path().join("a.xlsx"), dir.path().join("b.xlsx")]
        );
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
