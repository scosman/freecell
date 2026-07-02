---
status: draft
---

# Component: App Shell & Chrome (`freecell-app`)

Everything around the grid: application lifecycle, welcome window, workbook windows,
menus/shortcuts, the action row / data row / sheet tab bar, and dialogs. Built from
stock gpui-component controls per `ui_design.md` — deliberately thin and unfancy.

## Purpose and scope

**Does:** app entry + window registry; welcome window; `WorkbookWindow` composition
(chrome + grid + document client wiring); menu bar + key bindings; dialogs and their
async flows; dirty/close/quit logic.

**Does not:** cell rendering (grid.md), engine anything (engine_worker.md), business
rules living in `freecell-core` (validators, selection math).

## Structure

```rust
// main.rs
gpui_platform::application().run(|cx| {
    gpui_component::init(cx);            // theme (default light), components
    FreeCellApp::init(cx);               // global: window registry, menus, actions
    FreeCellApp::show_welcome(cx);
});

struct FreeCellApp {                     // gpui Global
    windows: Vec<WindowRegistration>,    // workbook windows: handle + path (canonical) + dirty
    welcome: Option<WindowHandle<WelcomeView>>,
}

struct WorkbookWindow {                  // root Entity per document window
    client: DocumentClient,              // engine_worker.md
    grid: Entity<GridView>,
    chrome: ChromeState,                 // action-row toggle states, data-row editor state,
                                         // sheet tabs (mirrored SheetMeta), rename-in-flight
    doc: DocState,                       // path, dirty (op accounting), loading, degraded,
                                         // eval_in_flight, fidelity_warning_pending: bool
    modal: Option<ActiveModal>,          // one modal at a time, owned here
}
```

### Lifecycle rules (functional_spec §2)

- `show_welcome` only when no workbook windows exist. Any workbook window opening
  closes welcome; last workbook window closing (post-prompt) reopens it.
- **Open**: dedupe by canonical path — if already open, activate that window.
  Otherwise create the window immediately in loading state and `DocumentClient::spawn
  (OpenFile)`; `Loaded` → populate tabs + grid; `LoadFailed` → close window (or if it
  was the only pending one, return to welcome) + error dialog.
- **Close (Cmd+W / traffic light)**: if dirty → modal Save / Don't Save / Cancel;
  Save routes through the save flow and closes on `Saved`. GPUI window-close
  interception at the pinned rev: use the `on_should_close`-style hook if present;
  fallback = intercept Cmd+W/menu only and accept that the traffic-light close may
  skip the prompt — **check the API first; if fallback is taken, record it in
  DECISIONS_TO_REVIEW.md** (it's a data-loss papercut).
- **Quit**: iterate dirty windows, prompt each (front-to-back); any Cancel aborts
  quit.
- Finder open-events: wire `App::on_open_urls`/equivalent if the pinned rev exposes
  it; else skip (record).

### Menus & actions (single source of truth)

GPUI actions: `NewWorkbook, OpenFile, Save, SaveAs, CloseWindow, Undo, Redo,
ToggleBold, ToggleItalic, ToggleUnderline, Quit, About`. Menu bar (`cx.set_menus`) and
key bindings both dispatch these; handlers live on `WorkbookWindow` (or the app global
for New/Open/About/Quit). Enable/disable via standard GPUI action availability (no
handler in scope = disabled menu item): Save/Undo/etc. are naturally disabled on the
welcome window because it registers no handlers.

### Action row

- Three toggle `Button`s + fill split-button per `ui_design.md §3.1`. Pressed state =
  active cell's `RenderStyle` read from the style cache at selection-change time
  (cached in `ChromeState`, refreshed on `SelectionChanged` and `StyleCacheUpdated`).
- Click → commit pending data-row edit first (if any) → send
  `SetStyleAttr{selection.range(), attr}`; worker computes multi-cell toggle
  resolution (engine_worker.md).
- Fill popover: gpui-component popover + a 5×2 swatch grid + "No fill" row; swatch
  constants in `freecell-core::palette` (10 named colors — chosen at implementation
  from a standard palette, documented in code).

### Data row (formula bar)

- `RefBox`: read-only 72 px field; text from `SelectionModel::to_a1()` (`B7` /
  `B2:D9`).
- `ContentField`: gpui-component `TextInput`. State machine:
  - **Idle**: shows active cell's `raw_content` (from Publication;
    `GetCellContent` fallback for beyond-overscan cells — show a muted "…" until the
    reply). Multi-cell selection: disabled + empty.
  - **Editing** (user focused/typed): pending text held locally; selection changes
    via *grid click* commit-then-move (grid emits `EditCommitRequested` before
    `SelectionChanged` applies); Enter = validate cap → send `SetCellInput` → move
    down; Shift+Enter/Tab variants per keymap; Escape = revert to raw_content, back
    to Idle, grid regains focus.
  - **Cap-rejected**: danger border + message popover; stays Editing.
- Eval spinner at the row's right end while `eval_in_flight` (from
  `EvalStarted/Finished` events).

### Sheet tab bar

- Mirrors `SheetMeta` list from the worker. Tab click → `set_active_sheet` on grid +
  chrome refresh. `+` → `AddSheet` (worker assigns `SheetN` name and publishes
  `SheetsChanged`; UI switches on arrival).
- Rename: double-click swaps label → `TextInput` (same width); Enter/blur → validate
  (`freecell-core::sheet_name::validate` — the same rules the worker re-checks) →
  `RenameSheet`; invalid → danger border, stay editing; Escape reverts.
- Context menu (gpui-component): Rename, Delete (disabled if last sheet). Delete on
  non-empty sheet (worker includes `has_content` in `SheetMeta`) → confirm modal →
  `DeleteSheet`.

### Dialogs

All gpui-component modals rendered by `WorkbookWindow` (or a bare dialog window for
app-level errors), one at a time via `modal: Option<ActiveModal>`:
`UnsavedChanges{then: CloseWindow|Quit}`, `FidelityWarning{then_save_to: PathBuf}`,
`ErrorInfo{title, detail}`, `ConfirmDeleteSheet{idx}`, `About`. Each is a small enum
variant + handler — no dialog framework. Native `NSOpenPanel`/`NSSavePanel` via
GPUI's path-prompt API (confirmed pattern in zed at the rev; gpui-component fallback
if broken).

### Save flow (ties together §5.2 of the functional spec)

`Save` action → if no path → SavePanel → path. If document was opened from disk and
`fidelity_warning_pending` (set true on Loaded, cleared after first accepted warning)
→ FidelityWarning modal → on Save Anyway proceed. Send `Command::Save{path}`;
`Saved{ops_seen}` clears dirty (op accounting per architecture §2) and updates
title/path (Save As); `SaveFailed` → ErrorInfo, stay dirty.

## Dependencies

Depends on: everything (`core`, `engine`, grid, gpui, gpui-component). Depended on
by: `main.rs` only.

## Test plan

Linux (pure logic extracted to `freecell-core`): `sheet_name_validate_*` (xlsx rule
matrix), `to_a1_*`, palette constants sanity, data-row state machine as a pure
`reduce(state, event) -> (state, Vec<Effect>)` function with table-driven tests
(`edit_commit_on_cell_click`, `escape_reverts`, `cap_reject_keeps_editing`,
`multiselect_disables`, …).

macOS (gpui test context, as far as its APIs allow — anything not drivable is listed
in the phase plan explicitly): `welcome_to_workbook_lifecycle`,
`open_dedupes_same_path`, `close_dirty_prompts_and_cancel_keeps_window`,
`save_as_sets_title`, `fidelity_warning_once_per_document`,
`menu_actions_disabled_on_welcome`. Manual smoke checklist (documented in the phase
plan, not a substitute for the above): traffic-light close prompt, Finder open, panel
filters.
