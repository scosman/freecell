---
status: complete
---

# Architecture: MVP Gaps — Core Spreadsheet Feel

Extends the MVP architecture (`specs/projects/mvp/architecture.md`): same crates,
same worker seam, same resident style cache, same undo model. This doc specifies only
what changes. **All engine/gpui facts below were verified against the pinned sources
(2026-07-04 audits): ironcalc/ironcalc_base 0.7.1 (crates.io, checksums matched to
`app/Cargo.lock`) and gpui @ zed rev `1d217ee39d…`.** Citations like `base/…` are
ironcalc_base 0.7.1 paths; `gpui/…` are `crates/gpui/…` at the pinned zed rev. The
implementing agent should treat these as ground truth and not re-derive them.

## 0. Principles (unchanged, restated as constraints)

- Every engine mutation goes through the worker (`freecell-engine/src/worker/`) as a
  `Command`; the UI thread never calls a mutating `UserModel` method. All new
  mutations listed here are undoable via the engine history **except** none — every
  API chosen below is history-integrated (verified per-API).
- Scroll path reads only the resident `SheetCache` + published values. New render
  inputs (fonts, borders, value types) must live in cache/publication, never fetched
  per-frame.
- `RenderStyle` stays `Copy + Eq + Hash` (it is interned, `freecell-engine/src/cache.rs:143-151`);
  variable-size data goes in side tables with small ids.

## 1. Cross-cutting data-model changes

### 1.1 `RenderStyle` (freecell-core/src/style.rs)

Add fields (the omission comment at style.rs:20-22 is now partially lifted):

```rust
pub font_size_q: u16,      // font size in quarter-points; 0 = default (engine default 11pt)
pub font_family: u16,      // index into SheetCache.font_families; 0 = default font
pub border: u16,           // index into SheetCache.border_specs; 0 = no borders
```

Side tables on `SheetCache` (freecell-core/src/cache.rs), built worker-side with the
rest of the cache and swapped atomically with it:

```rust
pub font_families: Vec<SharedString>,           // [0] = "" (default)
pub border_specs: Vec<BorderSpec>,              // [0] = BorderSpec::NONE
pub struct BorderSpec { pub top: Option<Edge>, pub right: …, pub bottom: …, pub left: … }
pub struct Edge { pub weight: u8 /*1,2,3 px*/, pub color: Rgb }
```

Weight mapping from IronCalc `BorderStyle` (`base/types.rs:596`): Thin/Dotted/Hair/
Dashed→1, Medium/MediumDashed/MediumDashDot/MediumDashDotDot/SlantDashDot→2,
Thick/Double→3. All drawn solid (SP5 already accepted dotted→thin class fidelity).
Interning: `HashMap<BorderSpec, u16>` during cache build, same pattern as style
interning.

### 1.2 `PublishedCell` (freecell-core/src/publication.rs:18-26)

Add:

```rust
pub kind: CellKind,   // #[repr(u8)] Number, Date, Text, Bool, Error  (default Text)
```

and actually populate the existing `text_color` (today always `None`):

- Worker publication (freecell-engine/src/worker/run.rs, viewport build ~:625-658):
  - `kind`: `UserModel::get_cell_type(sheet,row,col)` (`base/user_model/common.rs:488`,
    cheap hash lookup). Map `CellType::{Number, Text, LogicalValue, ErrorValue}`
    (`base/types.rs:153`) → `CellKind`. **Gotchas (verified):** empty cell returns
    `Number` — but empty cells aren't published, so irrelevant; dates return `Number` —
    reclassify as `Date` when the cell's `num_fmt` is date-like (heuristic below).
  - Date heuristic (freecell-core, unit-tested): strip `[...]` sections and
    `"quoted"`/`\`-escaped literals from the format string; if any of `y m d h s`
    remain → Date. (`@`, `#`, `0` formats stay Number.)
  - `text_color` precedence: explicit `style.font.color` if set, else the number
    format's produced color, else None. Format color: gate on `num_fmt.contains('[')`,
    then call `ironcalc::base::formatter::format::format_number(value, num_fmt, locale)
    -> Formatted { color: Option<i32>, .. }` (`base/formatter/format.rs:59,10` — public
    module, `base/lib.rs:33`; locale via `base/locale/get_locale`, `locale/mod.rs:105`).
    Color index map (from `base/formatter/lexer.rs:286-303`): 0 black, 1 white, 2 red,
    3 green, 4 blue, 5 yellow, 6 magenta; `[Color N]` → classic indexed palette N.
    FreeCell carries a small const table for 0–6 + the 56-color indexed palette.

### 1.3 Grid default alignment (freecell-app/src/grid/view.rs, cell_element ~:1231)

When the resolved style has no explicit horizontal alignment: `Number|Date` → right,
`Bool|Error` → center, `Text` → left. Explicit alignment (already rendered) wins.
This closes GAPS #1/#2; update those GAPS.md rows on completion.

## 2. Worker protocol additions (freecell-engine/src/worker/protocol.rs:39)

```rust
Command::SetStylePath   { sheet, area: Area, path: String, value: String }  // §3.1
Command::SetBorders     { sheet, area: Area, preset: BorderPreset }         // §3.4
Command::SetFont        { sheet, area: Area, family: Option<String>, size_pt: Option<f64> } // §3.3
Command::SetColumnWidths{ sheet, col_start, col_end, px: f64 }
Command::SetRowHeights  { sheet, row_start, row_end, px: f64 }
Command::InsertRows     { sheet, row, count }        // + InsertColumns/DeleteRows/DeleteColumns
Command::CopySelection  { sheet, range: CellRange, cut: bool }   // reply: TSV string
Command::PasteInternal  { sheet, anchor: (i32,i32) }
Command::PasteTsv       { sheet, anchor: (i32,i32), text: String }
```

Replies use the existing worker reply channel; commands that can fail user-visibly
(paste, insert/delete, per §6) reply `Result<(), WorkerError>` surfaced as dialogs.
All run inside the existing coalescing/catch_unwind/eval pipeline; style-only ops
(SetStylePath/SetBorders/SetFont, widths/heights) trigger a **cache rebuild publish
but no evaluation** (match existing `SetStyleAttr` behavior).

## 3. Formatting features

### 3.1 Style paths (text color, alignment, number format)

One generic pass-through: `WorkbookDocument::update_style_path(area, path, value)` →
`UserModel::update_range_style` (`base/user_model/common.rs:1253`). Valid paths at
0.7.1 (verified, dispatch at `common.rs:113-194`): `font.color` (`#RRGGBB` or `""` to
clear), `alignment.horizontal` (`general|left|center|right|justify|fill|
centerContinuous|distributed`), `num_fmt` (raw code, **unvalidated by the engine** —
FreeCell sends only its own dropdown codes, so no validation layer needed).

**Band fast path (verified):** `update_range_style` automatically writes a column
band when the Area is exactly full-height (`row==1 && height==1_048_576`) and a row
band when exactly full-width (`column==1 && width==16_384`) (`common.rs:1274-1375`) —
cost O(populated cells in the band), not O(area). Therefore `area_of`
(freecell-engine/src/document.rs:569) must emit exact full extents for header
selections (§5.2). Select-all (full sheet) is NOT a band shape → route select-all
formatting as one full-column-band call per **used** column? No — simpler and correct:
select-all formatting applies the op to the used range only (clamped via
`worksheet.dimension()`), documented deviation matching Excel's practical behavior.

Number-format dropdown codes: General→`general` (clears), Number→`#,##0.00`,
Currency→`$#,##0.00`, Percent→`0.00%`, Date→`m/d/yyyy`, Time→`h:mm AM/PM`, Text→`@`.
Decimals ±: read active cell's `num_fmt` (already resident in cache), regex-adjust
the last `0.0…0` group (add/remove one `0`; min zero decimals), apply to selection
via `num_fmt` path. No-op if current format is General/Text/Date/Time.

### 3.2 Category display for the dropdown

Reverse-map the active cell's `num_fmt` to a category label by exact-match against
the table above; anything else displays "Custom" (no editing of custom codes).

### 3.3 Font family & size

- **No `font.name` or absolute-size path exists at 0.7.1** (only `font.size_delta`,
  `common.rs:154`). Use the verified fallback: `UserModel::on_paste_styles(&[Vec<Style>])`
  (`common.rs:1172`, undoable, tiles over the engine-side selected area). Worker flow
  for `Command::SetFont`:
  1. Clamp area: bounded selections as-is; full-row/col/select-all clamp to
     `dimension()` (documented deviation — no font bands).
  2. `set_selected_cell` + `set_selected_range` (`base/user_model/ui.rs:92,118`) to the
     clamped area. **Anchor must lie on the range edge** (`ui.rs:151-165`).
  3. For each cell: `get_model().get_style_for_cell(..)`, set `font.sz` and/or
     `font.name`, collect row-major `Vec<Vec<Style>>`, one `on_paste_styles` call.
  4. Cap: if clamped area exceeds 100k cells, reply error "Selection too large for
     font changes" (protects against pathological used-ranges; dialog).
- **Row auto-grow** (worker-side, same command): for each affected row, needed_px =
  `ceil(max_font_size_pt_in_op * 96/72 * 1.25) + 4`; if `needed_px >
  get_row_height(row)` (`common.rs:1108`) → collect and apply one
  `set_rows_height` per contiguous run (`common.rs:1081`, undoable — lands in the same
  history window as the style op; acceptable: undo restores height then style, two
  steps — record in DECISIONS if the engine coalesces differently). Never shrinks;
  no auto-grow on file open (files carry authored heights).
- **Enumeration** (UI): `cx.text_system().all_font_names()` (`gpui/src/text_system.rs:88-99`)
  once per window, cached; "System Default" prepended.
- **Rendering**: `cell_element` (grid/view.rs:1231-1282) adds `.font_family(name)` and
  `.text_size(px)` from `RenderStyle` via the side table (gpui `Styled::font_family`
  styled.rs:708, `text_size` :538 — per-element, no renderer change; missing fonts
  fall back via gpui's fallback stack). Cache build resolves family/size from the
  engine `Style` (already read per cell during build).

### 3.4 Borders

- **Write**: `UserModel::set_area_with_border(range, &BorderArea)`
  (`base/user_model/border.rs:346`; undoable; band-aware for full rows/cols; engine
  applies heavier-wins fix-up to the 4 adjacent strips). `BorderArea` has
  `pub(crate)` fields and no constructor (verified) — construct via serde:
  `serde_json::from_value(json!({"item":{"style":"thin","color":"#000000"},
  "type":"All"}))`; `type ∈ All|Inner|Outer|Top|Right|Bottom|Left|None` (PascalCase;
  `BorderStyle` serde is lowercase). Map the 8 UI presets 1:1.
- **Read/render**: cache build reads each populated/styled cell's `Style.border` into
  interned `BorderSpec`s (§1.1). Paint (grid/view.rs cell loop): for each visible
  cell draw its **right** and **bottom** effective edges as absolute 1–3 px solid
  divs over the gridline; effective edge = heavier of (cell.right, right-neighbor's
  left) — neighbor lookup hits the resident cache snapshot already held by the frame
  (no locks). Draw **left** edge only for the first visible column and **top** only
  for the first visible row (same merge rule with the off-screen neighbor). Border
  color from `Edge.color`.
- Perf: border draw adds ≤4 quads per bordered cell, only for cells whose
  `RenderStyle.border != 0`; scroll gates re-verified in CI.

## 4. Editing feel

### 4.1 Single pending-edit controller

Refactor: one `EditController` (new module freecell-app/src/shell/edit.rs) owned by
`WorkbookWindow`, holding `Option<PendingEdit { sheet, cell, text: SharedString,
origin: DataRow | InCell }>`. The chrome data-row `InputState` and a new in-cell
`InputState` (both created in `WorkbookWindow::build`, shell/window.rs:199-217, where
`&mut Window` is available — `InputState::new` requires it, chrome/view.rs:124) are
**views onto the controller**: input events update the controller; the controller
pushes text to whichever editor(s) are visible. Commit/cancel logic moves from
chrome/view.rs into the controller (chrome keeps rendering; existing cap validation +
danger state stay, now shared by both editors). This is the one real refactor in the
project — do it first in its phase, keep the existing data-row tests green, then add
surfaces.

### 4.2 Type-to-replace

`grid/input.rs` (~:75-78, where printable keys currently return `None`): printable,
modifier-free keystroke + single selection → `GridEvent::TypeToEdit(char)`. Window
shell: `EditController::begin(cell, String::from(char), DataRow)`, set data-row input
text, focus it (`InputState::focus`, gpui-component state.rs:1212), caret at end.
Multi-selection: same, targeting the anchor.

### 4.3 Live mirror

Grid render: if `EditController` has a pending edit on a visible cell of the active
sheet, that cell renders `pending.text` (left-aligned, default font/style) instead of
its published value. Controller text changes → `cx.notify` the grid (existing
chrome→window→grid event path). No engine involvement.

### 4.4 In-cell editor

- Trigger: double-click (grid mouse handler — add click-count check to
  `handle_mouse_down`, view.rs:491; gpui MouseDownEvent carries `click_count`) or F2
  (keymap addition) → `EditController::begin(cell, current_raw_content, InCell)` —
  raw content comes from the same fetch the data row already does on selection
  (chrome/view.rs:358-368 path; reuse its cached value, don't refetch).
- Render: in `build_grid_layers` (view.rs:972-982 content layer), when origin==InCell
  push `div().absolute()` at `cell_rect(active)` (view.rs:1212-1218), min-width 80 px,
  2 px accent border, containing `Input::new(&incell_state)` (gpui-component Input is
  `RenderOnce + Styled`, mountable in any div — verified input.rs:34-35,238-244).
  Wrap in `deferred()` (gpui/src/elements/deferred.rs:7) to paint above overlays.
- Focus: on begin, focus the in-cell `InputState`; grid mouse-down currently grabs
  grid focus (view.rs:474-476) — begin() runs after, order matters; on commit/cancel
  return focus to the grid focus handle.
- Editor scrolls with content (its rect is computed per-frame from cell_rect, which
  is scroll-relative); when the cell is fully outside the viewport the overlay is
  clipped by the existing `overflow_hidden` container — no special handling.

### 4.5 Tab-commit

The controller handles Tab/Shift+Tab from both `InputState`s' key events (the known
gpui-component limitation is that the bare Input doesn't emit a commit on Tab —
DECISIONS_TO_REVIEW: intercept the key **before** the input consumes it via a
`.on_key_down` on the wrapping div, mark handled, run commit+move).

## 5. Structure & navigation

### 5.1 Resize

- Hotspots: in the grid element's header strips, a 6 px zone centered on each divider
  (positions from the existing `Axis` prefix sums). Implement as absolute divs in the
  header layer (headers are plain divs — `header_element`, view.rs:1285-1309) with
  `.cursor_col_resize()` / `.cursor_row_resize()` (gpui Styled one-liners, verified
  present with exactly these variants, gpui/src/platform.rs:1887,1891) and
  mouse-down handlers.
- Drag: `GridView.resize_preview: Option<ResizePreview { axis: RowOrCol, index,
  new_px }>`; layout consults it as a post-`Axis` adjustment: coordinates of tracks
  \> index shift by `delta = new_px - axis.size_of(index)`; `size_of(index)` reports
  `new_px`. Touch points: `cell_rect` (view.rs:1212-1218), header layout, overlay
  rects, hit-testing (`grid/layout.rs:126-168,220-249`) — one shared helper, applied
  in `GridLayout` construction so all consumers see it. Guide line + tooltip per
  ui_design §3.
- Release: clamp (col ≥ 8 px, row ≥ 12 px), then `Command::SetColumnWidths` /
  `SetRowHeights` over the selected header run if the dragged index is inside the
  header selection, else just that index. Engine: `set_columns_width`
  (`common.rs:1055`) / `set_rows_height` (`common.rs:1081`) — both undoable, both
  range-native. Worker rebuilds cache geometry (existing `set_row_heights` batch
  path, freecell-core/src/cache.rs:218-229) → publish → grid clears preview on next
  cache generation. **Never** call these with unbounded ranges (each creates
  per-row/col entries; `worksheet.rows` is a linearly-scanned Vec — verified).

### 5.2 Header selection

- `SelectionModel` (freecell-core/src/selection.rs:64-67) is unchanged: full column =
  anchor `(1,c)`, active `(1_048_576,c)`; full row analogous; select-all = `A1` →
  `XFD1048576`. Header hit-tests (currently explicit no-ops, view.rs:501-503) set
  these; drag across headers extends the track range; Shift+click extends.
- `area_of` (document.rs:569) already maps ranges to Areas — add unit tests asserting
  exact `row==1,height==1_048_576` output so the band path (§3.1) engages.
- Reference-box display: `C:C` / `3:7` / `A:XFD` formatting in the existing ref-box
  formatter; data row disabled per MVP multi-select rules.
- **Clamping rule (load-bearing, verified):** `range_clear_contents`
  (`common.rs:644`) has NO band path — a full-column Area iterates 1,048,576 cells.
  Worker clamps ClearCells (and PasteTsv target checks, and font ops §3.3) to
  `dimension()` before calling. Style paths and borders do NOT clamp (they have band
  paths). Centralize as `WorkbookDocument::clamp_to_used(area) -> Area`.
- Cmd/Ctrl+A keymap → select-all.

### 5.3 Insert/delete + merge guard

- Header right-click → gpui-component context menu (pattern: sheet-tab menu,
  chrome/view.rs:1074-1178). Counts from the header selection.
- Engine: `insert_rows`/`insert_columns`/`delete_rows`/`delete_columns`
  (`common.rs:882,907,932,974` — all undoable with full data snapshots; heights/
  widths/band styles shift correctly via `base/actions.rs:331,397,136,214`; bound
  errors returned as `Err(String)` → dialog).
- **Merge guard** (worker, before dispatch): read `doc.worksheet(sheet).merge_cells`
  (`Vec<String>` A1 ranges — public field, `base/types.rs:113`; accessor
  document.rs:301-303). Parse with the existing A1-range parsing used by `area_of`'s
  inverse (or a 20-line local parser; ranges like `"K7:L10"`). Block iff any merge
  intersects-or-follows the affected index: row ops at row r block if any merge's
  `max_row >= r`; column ops likewise. Blocked → typed `WorkerError::MergesInWay` →
  dialog per functional spec §5.3.

## 6. Clipboard

Worker owns the clipboard slot (engine types aren't nameable outside the crate —
verified: `Clipboard`/`ClipboardCell` fields `pub(crate)`, not re-exported; only
`ClipboardData = HashMap<i32, HashMap<i32, ClipboardCell>>` is exported and it's
serde-able, `base/user_model/common.rs:29,40`):

```rust
struct ClipboardSlot { sheet: u32, range: ClipboardTuple, data: JsonValue /*serialized ClipboardData*/, cut: bool }
```

- **Copy/Cut** (`Command::CopySelection`): set engine selection (`set_selected_cell` +
  `set_selected_range`, `base/user_model/ui.rs:92,118` — the ONLY feature routing
  through UserModel's hidden view-selection state; anchor on range edge, `ui.rs:151-165`),
  call `copy_to_clipboard` (`common.rs:1765`; clamps to `dimension()` — full-column
  copy is cheap), `serde_json::to_value` the result, stash in the slot, reply with
  the `csv` field (tab-separated formatted text) → UI writes it to the system
  clipboard (`cx.write_to_clipboard`) and remembers `last_copy_text` + generation.
- **Paste** (UI decides): read system clipboard; if text == `last_copy_text` →
  `Command::PasteInternal` (worker: set selection to anchor, deserialize slot,
  `paste_from_clipboard(sheet, range, &data, cut)` — `common.rs:1811`, undoable,
  Excel ref-adjustment via `extend_copied_value`/`move_cell_value_to_area`
  (`base/model.rs:1179,1053`); on cut success clear the slot). Else →
  `Command::PasteTsv` (worker: compute Area from parsed dims at anchor, reject if
  overflows sheet bounds, `paste_csv_string(&area, &text)` — `common.rs:1926`,
  **tab-delimited** (verified `b'\t'`, :1934), values-as-user-input, undoable).
- Keyboard: Cmd/Ctrl+C/X/V in the grid keymap (grid focused only; the data-row/
  in-cell inputs keep their native text clipboard).
- Empty selection copy = single active cell. Paste with a pending edit commits the
  edit first (existing rule).

## 7. Chrome & safety

### 7.1 Titlebar (macOS)

- `document_window_options()` / `welcome_window_options()`
  (freecell-app/src/shell/app.rs:502-518): on macOS set `titlebar:
  Some(TitlebarOptions { appears_transparent: true, traffic_light_position:
  Some(point(px(12.), px(12.))), title: None })` (gpui/src/platform.rs:1647-1657;
  macOS impl verified: transparent titlebar + hidden system title,
  gpui_macos/src/window.rs:792,952-954; traffic-light repositioning :902-904).
- `WorkbookWindow::render` (shell/window.rs:773-819) prepends a 36 px `CHROME_BG` row:
  centered title text from `lifecycle::window_title` (already the source for
  set_window_title at window.rs:232/469 — keep calling `set_window_title` too; it
  feeds the hidden native title/Exposé), `.window_control_area(WindowControlArea::Drag)`
  (gpui/src/window.rs:594-603; div fluent elements/div.rs:1136). Same for Welcome.
- Linux: `cfg!(target_os = "macos")` guards both the options change and the row.
  `set_window_edited` (window.rs:468) unaffected (dot lives in the traffic light).
- **First task of the phase is a 30-minute on-device smoke** (build, check traffic
  lights/drag/zoom/fullscreen); if broken at this rev, drop the feature (flag off) —
  pre-agreed fallback, no gpui bump.

### 7.2 Cap-error popover

Wire the existing no-op `DataRowEffect::ShowCapError` (chrome/view.rs:352): chrome
state gains `cap_error: Option<CapErrorKind>`; render a tooltip-style anchored div
under the active editor (data row or in-cell — controller knows origin); clear on
next input event or focus change. Strings per functional spec §4.2.

### 7.3 `.back` backup

In the save flow (shell/window.rs:570-618), before the existing atomic
temp-write+rename and only when: document was opened from disk ∧ saving to that same
path ∧ `<path>.back` does not exist → `std::fs::copy(path, path.back)`. On copy
failure: abort the save, dialog "Couldn't create backup — file not saved." The
existence check makes it write-once across sessions; Save-As to a new path never
creates one. Unit-test with tempdirs (first save creates, second save doesn't
overwrite, save-as doesn't create, failure aborts).

## 8. Error handling

- Worker command failures: `WorkerError { kind, message }` on the reply channel.
  Dialog-worthy: paste overflow, insert/delete bounds, MergesInWay, backup failure,
  font-op-too-large. Log-only: style path rejections (impossible via our UI),
  clipboard empty.
- Engine `Err(String)` values are wrapped, never panic; the existing catch_unwind +
  degraded-mode machinery is unchanged and covers all new commands (they run in the
  same apply loop).
- Degraded mode: all new mutating controls check the existing read-only flag.

## 9. Testing

- **Unit (freecell-core / freecell-app logic):** date-format heuristic; decimals
  regex; TSV dims parsing + overflow check; EditController state machine (begin/
  mirror/commit/cancel/origin switching, Tab commit); resize preview adjustment math;
  merge-guard predicate (A1 parsing, intersects-or-follows cases); backup rules;
  `area_of` full-extent exactness; ref-box `C:C`/`3:7` formatting; border weight
  mapping + heavier-edge merge; CellKind mapping incl. date reclassification.
- **Engine integration (freecell-engine, real UserModel):** copy→paste roundtrip
  (values, formula ref-shift `=A1` copied down → `=A2`, styles); cut = move (source
  cleared, refs follow); TSV paste; full-column style write lands as a band
  (`get_model().workbook` inspection) and does NOT materialize 1M cells; borders
  preset → engine border model + heavier-wins; on_paste_styles font set + undo;
  insert/delete rows with formulas + undo restores data; row-height/col-width set +
  undo; merge guard blocks/allows correctly on a fixture with merges; clipboard slot
  cut-clears-once.
- **Render suite (existing PNG harness):** `border_all_thin`, `border_outer_medium`,
  `border_heavier_edge_wins`, `font_family_serif`, `font_size_24_row_grown`,
  `text_color_red`, `format_red_negative`, `align_number_default_right`,
  `align_error_center`, `align_explicit_beats_default`, `incell_editor_open` (editor
  overlay on a cell), `titlebar_row` (macOS-styled chrome row renders in the Linux
  harness too — it's just a div; the *native* integration is the on-device smoke).
- **Perf:** existing CI scroll gates unchanged and must stay green (borders/fonts
  are cache-resident); add a harness assertion that a 500-bordered-cell viewport
  stays within budget.
- **Smoke (macOS, manual, end of project):** titlebar behaviors; Finder-level flows
  unchanged; add to `specs/projects/mvp/smoke_checklist.md` successors.

## 10. Phasing (see implementation_plan.md)

Dependency notes: §1.2/§1.3 (publication) has no dependents — first. EditController
(§4.1) precedes all of §4. Clipboard (§6) is independent. §3.1 paths are independent
of §3.3/§3.4. Resize/headers/insert-delete (§5) independent of §3/§4. Titlebar last
(device-verification gate).
