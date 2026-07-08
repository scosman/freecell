---
status: complete
---

# Implementation Plan: Recent Files + Welcome Screen

Three phases, dependency-ordered: pure core → app/menu wiring → welcome redesign. Each is a
coherent CR unit. Details live in `functional_spec.md`, `ui_design.md`, `architecture.md`.

## Phases

- [x] **Phase 1 — Recent-files core (`freecell-core::recent`).** Data model
  (`RecentEntry`/`RecentList`), JSON (de)serialize, `record` (front-insert + dedupe-by-path +
  cap + prune-missing), `clear`, `display_entries` (stat for size, drop missing, build
  `DisplayEntry`), and pure formatters (`format_size`, `parent_folder_label`,
  `format_relative_time` + the dependency-free civil-date helper). Add `serde`/`serde_json`
  to the core manifest. Full unit-test coverage (`architecture.md §7`). No GPUI, no
  wall-clock reads.

- [x] **Phase 2 — App integration + Open Recent menu (`freecell-app` shell).**
  `shell/recents.rs` (`recents_store_path` via `dirs`); `FreeCellApp` owns `RecentList`,
  loads it in `init`, records on `do_open_path` (post-canonicalize) and `note_window_path`,
  and refreshes UI; `OpenRecent { index }` + `ClearRecent` actions + handlers; macOS
  **File → Open Recent** submenu (`build_menus(&RecentList)` + `install_menus_with`) with the
  disabled empty state; rebuild-on-change. Add `dirs` to the app manifest. gpui + menu tests
  (`architecture.md §7`). *No welcome-view changes yet* (the welcome still renders its current
  body; it just gets seeded/updated harmlessly or is left untouched until Phase 3).

- [x] **Phase 3 — Welcome screen redesign (`freecell-app::shell::welcome`).** Two-pane layout
  (720×480 window), new tagline "The open spreadsheet", RECENT list of up to 5 rows
  (glyph/name/subtitle/relative-time, hover, click→`open_path`), empty state, and the
  `set_recents` update seam wired from `FreeCellApp::refresh_recents_ui` +
  seeded on `do_show_welcome`. Reuse the existing app palette (`ui_design.md §0`) — no new
  hexes. gpui view tests (row count, empty-state predicate, click routes to open) + `#[cfg]`
  test accessors. Confirm the existing render suite still passes and the Xvfb smoke launch
  opens the redesigned welcome window.

- [ ] **Phase 4 — About window (`freecell-app` shell).** Replace the About modal with a
  standalone, single-instance About **window** (`functional_spec.md §4`, `ui_design.md §6`,
  `architecture.md §9`): new `shell/about.rs` (`AboutView` — wordmark, tagline, version from
  `CARGO_PKG_VERSION`, hairline, Homepage / Built-with link rows opening
  github.com/scosman/freecell, ironcalc.com, gpui.rs via `cx.open_url`); `FreeCellApp`
  open/activate/track/close the About window; `registry` `about_open` accounting; rewire the
  `About` action away from the modal; remove the About modal from `welcome.rs` + `window.rs`.
  One `LINK` color token (build on our design system, mockup is directional). gpui tests
  (opens window, single-instance, quit-accounting, link URLs/version) + registry test +
  Xvfb smoke (About FreeCell opens the window).

## Notes

- Phases 2 and 3 both consume Phase 1's `RecentList`. Phase 3 depends on Phase 2's
  `FreeCellApp.recents` + `refresh_recents_ui` seam.
- The one design-time risk (data-carrying `OpenRecent` menu action at the pinned gpui rev) is
  resolved in Phase 2 with a bounded fallback (`architecture.md §5`).
- **Phase 4 is independent of the recents feature** but is sequenced last because it edits
  `welcome.rs` (removing the About modal) and is cleaner on top of Phase 3's welcome rewrite.
  Its own risk (`cx.open_url` availability at the pinned gpui rev) has a bounded fallback
  (`architecture.md §9.1`).
