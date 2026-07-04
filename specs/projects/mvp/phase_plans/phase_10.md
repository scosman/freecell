---
status: complete
---

# Phase 10: App shell (welcome window, window registry & lifecycle, menus, dialogs, save/quit)

## Overview

Phase 10 builds the application shell around the grid + chrome: the entry point, the
welcome window, the multi-window registry + lifecycle rules (last window closes → app
quits), the macOS menu bar + per-platform key bindings, the file-open/save panels, all
the modal dialogs, and the save + quit flows. The **silent-strip save** (no fidelity
warning, `functional_spec.md §5.2`) is the save behaviour.

The end-to-end grid+chrome+worker composition inside each document window is **Phase 11**.
Phase 10 stands up the `WorkbookWindow` root entity + its lifecycle state (path, dirty,
loading, degraded, modal), owns a real `DocumentClient`, folds the *shell-relevant* worker
events (Loaded / LoadFailed / Saved / SaveFailed / WorkerDegraded), and renders a
placeholder content body Phase 11 replaces with the composed grid + chrome.

Per the manager's split: all lifecycle/registry/save/quit **decision logic is extracted to
pure, gpui-free modules** (`shell::registry`, `shell::lifecycle`) and exhaustively
unit-tested in `cargo test --workspace` (no display needed); GPUI is reserved for the
actual window/menu/dialog plumbing.

## Steps

1. **`shell/registry.rs` (PURE, gpui-free).** `WindowKey(u64)` opaque id (GPUI maps 1:1 to
   `WindowId`). `WindowRegistry { windows: Vec<WindowRecord{key,path,dirty}>, welcome_open,
   next_key }`. API: `register(path) -> WindowKey`, `remove(key)`, `set_path`, `set_dirty`,
   `resolve_open(&Path) -> OpenOutcome::{Activate(key)|OpenNew}` (dedupe by canonical path,
   §5.1), `open_count`/`is_empty` (welcome counts), `set_welcome_open`, `dirty_among(order)
   -> Vec<WindowKey>` (quit prompt order from a GPUI-supplied front-to-back key list).

2. **`shell/lifecycle.rs` (PURE).** `document_name(path)->String` ("Budget.xlsx"/"Untitled");
   `window_title(name, dirty, use_edited_suffix)` (append " — Edited" only when the macOS
   edited-dot is unavailable); `is_dirty(committed_ops, last_saved_ops)`; `SaveTarget::{Path,
   Prompt{suggested_name}}` + `resolve_save_target(current, save_as)`; `with_xlsx_extension`;
   `QuitPlan` queue (front-to-back dirty keys, `next()->QuitStep::{Prompt|QuitNow|Aborted}`,
   `resolved(key)`, `cancel()`). All table-tested.

3. **`shell/mod.rs`.** `actions!(freecell, [NewWorkbook, OpenFile, Save, SaveAs, CloseWindow,
   Undo, Redo, ToggleBold, ToggleItalic, ToggleUnderline, Quit, About])` (single source of
   truth). Module wiring + re-exports.

4. **`shell/menus.rs` (GPUI).** `build_menus()->Vec<Menu>` (macOS: FreeCell / File / Edit per
   §2.4). `bind_keys(cx)` — one action list, two keymaps: `cmd-*` on macOS, `ctrl-*` on Linux
   (`cfg!(target_os="macos")`), over identical action names. `set_menus` only on macOS.

5. **`shell/fonts.rs` (GPUI).** `register_fonts(cx)` hook (bundled Inter via `add_fonts` before
   any window opens). Inter TTFs are not vendored yet → best-effort no-op that logs + records in
   DECISIONS_TO_REVIEW; app falls back to gpui's default UI font (render baselines are on it).

6. **`shell/welcome.rs` (GPUI).** `WelcomeView`: small fixed centered window — app name,
   **New Spreadsheet** (→ `FreeCellApp::new_workbook`), **Open…** (→ open panel). Hosts an
   optional `ErrorInfo` modal (app-level LoadFailed when opened from Welcome). Registers no
   document actions → Save/Undo/etc. are disabled while Welcome is frontmost.

7. **`shell/window.rs` (GPUI).** `WorkbookWindow` root entity: `client: DocumentClient`,
   `doc: DocState{key, source, path, loading, degraded, last_saved_ops}`, `modal:
   Option<ActiveModal>`, `save_followup`, req-id counter. Spawns the worker on build; a
   `cx.spawn` event task folds shell events. Registers window-scoped action handlers (Save,
   SaveAs, CloseWindow, Undo, Redo, ToggleBold/Italic/Underline) on its root element →
   naturally enabled here, disabled on Welcome. `on_window_should_close` hook drives the
   dirty prompt (the good API path is present at the pinned rev — no papercut). Renders:
   loading overlay / degraded bar / placeholder content (Phase 11 mounts grid+chrome) /
   modal overlay. Modals: `UnsavedChanges{then}`, `ErrorInfo{title,detail}`, `About`.

8. **`shell/app.rs` (GPUI).** `FreeCellApp` global `{ registry: WindowRegistry,
   welcome: Option<WindowHandle<Root>>, key↔WindowId maps }`. `init(cx)` registers global
   actions (NewWorkbook, OpenFile, About, Quit) + `on_window_closed` (→ quit when registry
   empty) + `on_open_urls`/argv (best-effort). `show_welcome`, `new_workbook`,
   `open_path(path, from_welcome)`, `open_via_panel`, `quit` (front-to-back dirty prompts;
   any Cancel aborts). Open dedupes via `registry.resolve_open`.

9. **`main.rs`.** Replace the Phase-6 demo grid window: init tracing, `application()`,
   `gpui_component::init`, `register_fonts`, `FreeCellApp::init`, `FreeCellApp::show_welcome`,
   argv `.xlsx` open. Keep the `--exit-after-ms` render-spike valve.

10. **`Cargo.toml`.** Add `anyhow`, `tracing` (now used), `tempfile` (dev). `serde` for the
    `#[derive(Action)]` payloads is already transitively available via gpui; actions are unit
    structs so no serde derive needed.

## Tests

Pure (plain `#[test]`, `cargo test --workspace`, no display):
- `registry_resolve_open_dedupes_by_path` / `_opens_new_when_absent`
- `registry_quit_when_last_window_and_welcome_gone` (open_count/is_empty incl. welcome)
- `registry_welcome_counts_toward_open_count`
- `registry_dirty_among_preserves_front_to_back_order`
- `registry_set_path_and_dirty`, `registry_remove`
- `lifecycle_document_name_and_title` (Untitled / file / — Edited suffix vs dot)
- `lifecycle_is_dirty_op_accounting`
- `lifecycle_resolve_save_target` (Untitled→Prompt, Save As→Prompt, titled Save→Path)
- `lifecycle_with_xlsx_extension` (adds / preserves / replaces)
- `quitplan_empty_quits_now`, `quitplan_prompts_in_order`, `quitplan_cancel_aborts`,
  `quitplan_resolves_all_then_quits`

GPUI (`#[gpui::test]`, TestAppContext — drivable subset):
- `welcome_window_opens_on_show` (FreeCellApp::show_welcome adds a window; registry welcome_open)
- `new_workbook_opens_document_window_and_closes_welcome`
- `open_dedupes_same_path_activates_existing` (two opens of one fixture → one window)
- `workbook_close_when_clean_removes_window`
- `close_dirty_prompts_and_cancel_keeps_window` (modal state machine via direct calls)
- `save_untitled_prompts_then_saves_and_sets_title` (real NewWorkbook worker +
  `simulate_new_path_selection`; assert file written + title/dirty cleared)
- `save_failed_shows_error_and_stays_dirty`
- `load_failed_shows_error_dialog` (open a non-xlsx fixture → LoadFailed modal)
- `menu_actions_present_on_workbook_absent_on_welcome` (action handlers registered where expected)

Manual smoke checklist (documented; not a substitute): macOS traffic-light close prompt,
Finder open-file event, native panel `.xlsx` filter, Cmd+Q multi-window quit, menu
enable/disable, degraded bar Save-As.
