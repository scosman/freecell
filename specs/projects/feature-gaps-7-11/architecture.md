---
status: complete
---

# Architecture: Feature Gaps 7_11

Technical design for the seven in-scope features. Each is independent (async-codeable after
this pass). Anchors are `file:line` at planning time (2026-07-12) — treat as pointers, not
exact after edits.

## 0. Shared context

**Data flow (recap).** UI mutations go through the single-writer worker:

```
UI (grid / chrome) → GridEvent OR client.send(Command)
  → window.rs::route_grid_event → DocumentClient::send(Command)   [freecell-engine]
  → mpsc → Worker loop (worker/run.rs) → WorkbookDocument (document.rs) → ironcalc UserModel
  → WorkerEvent back to UI (publication / cache / sheet metas / rejections)
```

- `Command` enum: `freecell-engine/src/worker/protocol.rs:198`; dispatch `worker/run.rs:2210+`.
- Cell edit: `Command::SetCellInput { sheet, cell, input }` → `document.rs::set_cell_input:500`
  (`UserModel::set_user_input`).
- Raw content read: `document.rs::cell_content:337` (`get_cell_content`, returns formula text
  for formula cells).
- Used range: `worksheet.dimension()` (1-based inclusive; see `document.rs:842`).
- `GridEvent` → `Command` routing: `shell/window.rs::route_grid_event:1411`.
- Render read-model: `Publication` (non-empty cells only) + `SheetCache` (axes/styles);
  render hot path is `GridView::render:3407` → `resolve_frame:753` → `build_grid_layers:2071`
  → `cell_element:3034`. **Zero engine calls on the render path** — preserve this.

**Test posture.** Per CLAUDE.md, §2 (spill) and §3 (auto-grow) move grid pixels → in-scope
for the pixel render suite (dedicated late render phase). §1/§4/§5/§6/§7 do not touch
baselined surfaces → gpui view tests + Xvfb smoke launch, no pixel run.

---

## 1. Font-warning fix (§1)

**Change:** one line. The default `tracing` `EnvFilter` is built at `shell/main.rs:41-47`
(`.with_env_filter(EnvFilter…"info")`). Append a per-target directive so the
`gpui::svg_renderer` target is raised above WARN in the default filter, e.g. the fallback
string becomes `"info,gpui::svg_renderer=error"`.

- Keeps `RUST_LOG` override intact (an explicit `RUST_LOG` still wins, so the warning can be
  re-enabled for debugging).
- Do **not** serve Zed's fonts and do **not** alias our fonts under `fonts/*`. The font DB
  gpui builds from those paths is unused (our icon SVGs have no `<text>`), so suppression is
  correct, not a mask over a real problem.

**Test:** a small assertion that the constructed default filter directive contains the
target (or a doc-comment + manual smoke — the value is "no warning in the log"). No pixel
impact.

---

## 2. Text spill / overflow (§2)

**Where:** the per-frame cell loop in `build_grid_layers` (`grid/view.rs:2107-2159`) and the
cell builder `cell_element` (`grid/view.rs:3034`). Today each cell is one
`overflow_hidden` + `whitespace_nowrap` div (`view.rs:3062`), so text can't escape.

### 2.1 Eligibility & neighbor scan

For each visible **text** cell with content and **wrap off**, decide spill in the cell loop:

1. **Type gate:** only `CellKind::Text` spills (from `PublishedCell.kind`, `publication.rs:49`).
   Numbers/dates/bool/error skip (clip as today).
2. **Width gate:** the cell spills only if its rendered text is wider than its column. We do
   **not** want a full glyph measure per cell on the hot path; measure lazily — only attempt
   spill when text length plausibly exceeds the column, then let the wider text element clip
   naturally if it actually fits. (Architecture-acceptable: build the spill element whenever
   a text cell is non-empty + wrap-off + has an empty neighbor in the spill direction; the
   clip bounds do the rest. Avoids a measurement dependency.)
3. **Direction** (§2.2) from effective alignment (`style.h_align` else `kind.default_align()`,
   `view.rs:3073`): general/left → right; right → left; center → both.
4. **Neighbor emptiness:** walk outward in the spill direction across cells that are empty,
   stopping at the first cell with **content**. Emptiness = no mirror (`mirror_text_for`) and
   not present in the publication. Two lookups:
   - within the visible frame, use the per-frame `cell_index` (`view.rs:229`);
   - **past the frame edge / coverage**, consult `publication` coverage
     (`Publication::covers`, `publication.rs:94`) — and **stop spilling** when coverage is
     unknown (never treat "beyond covered region" as empty; §2.5 in the func spec).
   Fill/border on an empty neighbor does **not** stop the spill (content-only).

The scan is bounded: rightward spill stops at the first non-empty cell or the right edge of
the visible content + a small overscan; worst case is O(visible cols) for a row of empties,
which is fine.

### 2.2 Rendering approach

Render the spilling text as a **separate positioned text element** spanning the origin cell
through the last empty neighbor (the "spill rect"), rather than removing the origin cell's
clip:

- Keeps the origin cell div's fill/borders/gridlines intact (borders are on that div,
  `view.rs:3054`).
- The spill text element is `overflow_hidden` to the **spill rect** (origin cell start →
  stop boundary), `whitespace_nowrap`, aligned per direction, using the origin cell's
  font/color/vertical-align. For center spill, the spill rect extends both directions to the
  nearest non-empty on each side; text is centered within it.
- Painted **after** the empty neighbor cells so it sits on top; still inside the content clip
  wrapper (`view.rs:2258`) so it never escapes into headers.
- The origin cell itself no longer paints its own (clipped) text when it spills — the spill
  element replaces it — to avoid double-paint. (Or: keep origin clip and add the overflow as
  a sibling; pick the cleaner of the two at implementation — both are viable; spill-element-
  replaces-origin-text is preferred for center.)

### 2.3 Scope cut

Rightward-only (general/left) is the must-have; right-aligned (leftward) and center
(bidirectional) are punt-able if the bidirectional spill-rect math is disproportionate
(func spec §2.2). Recommend implementing all three (the rect math is symmetric) but gate
center/left behind the same code path so it can be trivially disabled.

### 2.4 Tests

Render cases: `spill_` prefix — long text over empties (right), right-aligned (left),
centered (both), stop at non-empty, stop at fill-only cell (does NOT stop → spills over),
wrap-on (no spill), number (no spill), stop at frame/coverage edge. Plus a unit test for the
pure neighbor-scan/direction helper (extract it gpui-free into `grid/layout.rs` so it's
unit-testable without a Window).

---

## 3. Auto-grow rows (§3) — sequenced LAST, independently revertible

The one feature with an architectural wrinkle. Built last; dropping it must not unwind
others. Baseline font-size + explicit-newline auto-grow already ship (`worker/run.rs:988-1018`,
`run.rs:1194-1200`) and stay; this adds **wrap-driven** growth + the **manual-height flag**.

### 3.1 The wrinkle: measurement needs the UI thread

Row height today is computed **worker-side** (formula from point size; or IronCalc's own
newline auto-fit). But **soft-wrapped** height (a wrap-on cell wrapping a long single line at
the column width) depends on glyph metrics + the column width — the worker has **no gpui
text system**. Measurement must happen on the **UI/render thread**, where a `Window` exists
(the app already has `measure_text_width` from `gpui_component::plot::label`, used by charts).

### 3.2 Design: measure on UI thread, persist height via the existing command

Flow (a bounded, debounced feedback loop):

1. **Detect dirty rows.** A row's wrapped height may change when: a wrap-on cell's content
   changes, wrap is toggled, font/size changes, or the **column is resized**. The UI learns
   of content/style changes via the publication/cache refresh and of resizes via the resize
   commit. Maintain a small **dirty-row set** on `GridView` for visible wrap-on cells whose
   inputs changed.
2. **Measure (UI thread, post-layout).** For each dirty row, measure the wrapped line count /
   height of each wrap-on cell at its column width using the gpui text system, take the row
   max, clamp to `[default, MAX_AUTO_ROW_PX]` (define cap, e.g. ~`default * 10` or an N-line
   cap — pick in impl; content beyond the cap clips within the cell).
3. **Apply once, if changed and row is AUTO.** If the measured height differs from the current
   height and the row is **not manual** (§3.3), send `Command::SetRowHeights { row_start=row,
   row_end=row, px }` (the existing geometry command, `protocol.rs:264`). The worker applies
   it (`document.rs::set_row_heights_px`), rebuilds the axis, republishes; the grid re-renders
   at the new height. **Guard against oscillation:** only emit when the delta exceeds a small
   epsilon and the row is dirty; clear the dirty flag after emitting so a measure→apply→
   refresh cycle converges in one step (the re-render measures the same height → no delta →
   no command).
4. **Shrink:** auto rows may shrink back toward default when tall content is removed (measure
   yields a smaller height). Manual rows never change.

This keeps the worker as the source of truth for geometry (heights still live in
`SheetCache`/IronCalc, still undoable, still saved) — the UI only *computes* the wrap height
the worker can't.

### 3.3 The manual-height flag

`custom_height` in IronCalc is set by **any** `set_rows_height` (including our auto-grow), so
it can't distinguish manual from auto. Introduce an explicit **manual-rows set**:

- Storage: per sheet, a `HashSet<u32>` of manually-resized rows. Home it where the height
  override lives — the resident `SheetCache` (`freecell-core/src/cache.rs`) is the natural
  place (it already owns `row_overrides`), or on the worker doc state. A row enters the set
  when a **user** row-resize commits (`GridEvent::ResizeCommitted` for a row →
  `Command::SetRowHeights`); it never auto-clears in this batch.
- The auto-grow path (§3.2 step 3) **skips** rows in the manual set.
- **Session-scoped** (func spec §3.3): not persisted to xlsx; a reloaded file's rows start
  auto. To avoid clobbering a file's intentional custom heights on load, treat any row with a
  **loaded** `custom_height` as manual at load time as well (so we don't shrink a file's set
  heights) — cheap and safe; note in impl.
  - *Distinguishing user-resize from auto-grow at the command layer:* the worker sets manual
    on `SetRowHeights` that originate from a resize, **not** on the auto-grow `SetRowHeights`.
    Since both use the same command, add a lightweight discriminator — either a separate
    `Command::AutoGrowRowHeights { … }` (preferred: keeps intent explicit and avoids a bool
    flag on the hot command) or a `manual: bool` on `SetRowHeights`. **Recommend a distinct
    `AutoGrowRowHeights` command** so the worker knows not to mark those rows manual and can
    treat them as coalescible/non-undo-spamming.

### 3.4 Undo & coalescing

An `AutoGrowRowHeights` height change should ride with the edit that caused it, not add a
separate user-visible undo step. The font-size auto-grow precedent (`run.rs:988-1018`) already
grows rows as part of `SetFont`; mirror that grouping. For content-driven wrap growth the
height change trails the cell edit — acceptable as a follow-on cache update that does not push
a separate undo entry (match how the value-edit auto-fit at `run.rs:1194-1200` behaves today).

### 3.5 Tests

- Unit: manual-set membership (resize marks manual; auto-grow does not; auto skips manual).
- Worker: `AutoGrowRowHeights` applies + rebuilds axis; does not mark manual; does not add an
  undo step the user must double-step.
- Render (`autogrow_` prefix): wrap-on long text grows row; narrowing column grows further;
  widening shrinks; manual row unchanged by content change; cap clip; large-font baseline
  still grows (regression).
- Convergence: a measure→apply cycle settles in one frame (no oscillation) — assert the
  dirty set empties.

---

## 4. Find / replace (§4)

Greenfield. Split: **bar UI** (chrome) + **search/replace** (worker) + **action/keymap** (shell).

### 4.1 Bar UI (chrome)

- New render method `render_find_bar(cx)` inserted in `ChromeView::render` between the data
  row and the body (`chrome/view.rs:2381`, after `render_data_row`) — the `flex_col` stacks
  it, pushing the grid down (func spec §4.1, ui §1).
- New `ChromeView` state (near the other open-flags, ~`view.rs:290`): `find_open: bool`, two
  `Entity<InputState>` for find/replace fields, `match_case: bool`, `whole_cell: bool`,
  `matches: Vec<CellRef>`, `match_idx: Option<usize>`. Add to the constructor + subscribe to
  the two inputs (mirror the `content_input` subscription at `view.rs:411`).
- Buttons/toggles reuse the action-row `Button` idiom (ghost/small/selected). Icons: bundled
  `chevron-up/down`, a new vendored `search.svg`, `square-x`/`x` for dismiss.

### 4.2 Search + replace (worker)

The scan must own the model (huge-sheet-safe, off the render publication). Add commands to
`protocol.rs`:

- `Command::Find { sheet, query, match_case, whole_cell }` → worker iterates the sheet's
  `dimension()` used range, reads `cell_content` per populated cell, applies the
  case/whole-cell predicate, returns ordered (row-major) matches via a new
  `WorkerEvent::FindResults { matches: Vec<CellRef> }`. Used range is normally small; this is
  a read (no eval, no publish).
- **Replace one:** reuse `Command::SetCellInput { sheet, cell, input }` where `input` is the
  cell's raw content with the match substring replaced (whole-cell ⇒ the replacement is the
  whole input). The UI computes the new string from the raw content it already got (or the
  worker recomputes — prefer the worker computing the replacement from `cell_content` to avoid
  a stale-content race).
- **Replace all:** `Command::ReplaceAll { sheet, query, replacement, match_case, whole_cell }`
  applied as **one undoable batch** (precedent: paste / `set_area_with_border` are "one
  undoable diff-list"). Worker re-scans, applies all replacements atomically, evaluates once,
  publishes, replies with a count (`WorkerEvent::ReplacedCount { n }`). If IronCalc can't
  group multiple `set_user_input` into one undo step, note the fallback (loop with a single
  undo boundary, or a small fork helper) — but atomic multi-cell writes already exist for
  paste, so the mechanism is available.

### 4.3 UI behavior wiring

- On find-field change / toggle change: send `Find`; on `FindResults`, store `matches`, set
  `match_idx` to the first match at/after the current selection, update the counter, select +
  reveal the match (reuse the grid's `set_selection_and_emit` + scroll-to-reveal path via a
  `ChromeGridRequest`).
- Next/Prev: advance/retreat `match_idx` with wraparound; reveal.
- Replace: replace current match → on the post-edit refresh, re-find (or advance) and reveal
  next. Replace All: send `ReplaceAll`, show count.
- Sheet switch while open: re-scope (re-send `Find` for the new active sheet; reset idx).

### 4.4 Action + keymap (shell)

- Add `OpenFind` to `actions!` (`shell/mod.rs:46`); bind `primary()+f` in
  `menus::bind_keys` (`shell/menus.rs:29`); optionally add to the Edit menu (`menus.rs:76`).
- Handler on `WorkbookWindow::render` (`shell/window.rs:1018-1040`, alongside `toggle_style`):
  call a new `ChromeView` method to toggle/focus the bar. Escape-to-close is handled on the
  bar's key handling (mirror the data row's Escape at `chrome/view.rs:2775`).

### 4.5 Tests

gpui view tests: bar opens/focuses on ⌘F + button; toggles; counter; next/prev wrap;
Escape/X close. Worker unit tests: `Find` predicate (case, whole-cell, substring, formula-text
match), used-range scoping, `ReplaceAll` count + single-undo (`undo` restores all). No pixel
impact (chrome not baselined) — smoke launch to confirm layout.

---

## 5. Quick-edit mode (§5)

State + key interception; no engine change.

### 5.1 The mode flag

- Add `quick_edit: bool` to the edit state. Home it on `ChromeView` alongside the edit
  plumbing, or on `EditController` (`chrome/edit.rs:33`) which already carries cross-editor
  `origin`/`open`. **Recommend `ChromeView`** (the single pending edit already lives there:
  `content_input` + `DataRow`).
- Set `quick_edit = true` in `begin_typed` (`chrome/view.rs:612`, the type-to-replace entry).
  Set/leave `false` in `begin_in_cell` (`view.rs:635`) and on formula-bar focus
  (`on_content_event` Focus, `view.rs:946`).

### 5.2 Arrow interception (commit + move)

Two capture handlers already intercept Tab there; add an arrow branch **only when
`quick_edit`**:

- **Type-to-replace path (data row):** the data-row `capture_key_down`
  (`chrome/view.rs:2783`). When `quick_edit && FieldMode::Editing` and the key is an
  unmodified `left/right/up/down`: `stop_propagation`, `commit_and_move(dir)`
  (`view.rs:703`) — Left/Right = column step, Up/Down = row step. Otherwise fall through
  (caret).
- **In-cell path:** the grid root `capture_key_down` (`grid/view.rs:3546`, which today
  handles Tab/Escape while `incell_open`). Add the same arrow branch guarded by a
  quick-edit signal forwarded to the grid (the grid already receives edit state via
  `set_edit_state`, `grid/view.rs:622` — extend `ChromeGridRequest::EditState`
  (`chrome/mod.rs:77`) with a `quick_edit` bool). Note: quick-edit begins in the **data
  row** (type-to-replace does not open the in-cell overlay), so the in-cell arm mainly
  matters if a quick-edit later mirrors into the overlay; keep both arms symmetric.

`commit_and_move` already retargets direction for Tab/Shift+Enter — reuse it with a
`Direction` argument from the arrow.

### 5.3 Leaving quick-edit (caret intent)

Set `quick_edit = false` on any of:

- **Mouse caret placement** in the field: a mouse-down on the data-row `Input` (or in-cell
  overlay). Add an `on_mouse_down` on the field that clears the flag. (There is no
  caret-move `InputEvent` today — `InputEvent` is only `Change/PressEnter/Focus/Blur`,
  `chrome/view.rs:747/911` — so we detect caret intent from the **input events we do
  control**: mouse-down and the specific keys below.)
- **Home / End** keys: intercept in the same capture handler; clear the flag and let the key
  reach the input (caret moves).
- **Modified arrow** (shift/secondary + arrow): clear the flag; treat as a caret/selection op
  (do not move the active cell).

Once cleared, the edit continues normally (arrows = caret) until commit/cancel. Commit/cancel
also resets `quick_edit` to `false` for the next edit.

### 5.4 Tests

gpui view tests / unit on the reducer + handlers: type→arrow commits & moves (each
direction); double-click→arrow moves caret (no move); formula-bar focus→arrow caret; Home/End
cancels then arrow = caret; mouse-click-in-field cancels; modified-arrow cancels; Tab/Enter/
Escape unchanged. No pixel impact.

---

## 6. Sheet reorder (§6) — includes an IronCalc fork change

### 6.1 IronCalc fork API (do first — FreeCell wiring depends on it)

Per CLAUDE.md, add to `scosman/ironcalc` on a clean `fix/sheet-reorder` branch off `main`,
upstream-style tests, fold into `freecell-fixes`:

- `UserModel::set_worksheet_index(sheet_id_or_index, new_index)` (or `move_sheet`) — moves a
  worksheet within the workbook vector, **undoable** (records a history diff so Undo/Redo
  restore order), and **xlsx-order-preserving** on save. Confirm no dangling references
  (sheet order is a vector position; formulas key off sheet **id/name**, not position, so
  intra-formula refs are unaffected — verify in the fork's tests).
- Open the upstream PR (`ironcalc/IronCalc`) as a single clean fix per the standing process.
- Update FreeCell's `app/Cargo.toml` patch pin (`freecell-fixes`) once merged into the
  integration branch; expect a `Cargo.lock` bump.

### 6.2 FreeCell engine wiring

- `Command::MoveSheet { sheet: SheetId, to_index: u32 }` in `protocol.rs:198` (after the
  Add/Rename/Delete sheet commands); dispatch in `worker/run.rs` (near `:2260`) →
  `document.rs::move_sheet` wrapper over the fork API; republish `WorkerEvent::SheetsChanged`
  so `merge_sheet_metas` (`chrome/view.rs:1847`) rebuilds tabs in the new order.
- Because tab order is derived from engine order, a UI-only reorder would be overwritten — so
  the reorder **must** round-trip the worker (do not locally reorder `self.sheets`).

### 6.3 Tab drag (chrome)

- Add drag state to `ChromeView` (e.g. `tab_drag: Option<TabDrag { sheet: SheetId, start_x,
  cur_x }>`) near the other tab state (`view.rs:351`).
- In `render_tab` (`chrome/view.rs:2853`): add `on_mouse_down(Left)` (record potential drag +
  start x), `on_mouse_move` (past ~4px threshold → enter drag: set `tab_drag`, compute the
  target insertion index from cursor x vs tab boundaries), `on_mouse_up` (if dragged → send
  `MoveSheet { sheet, to_index }`; else the existing click-select fires). Preserve the
  double-click→rename and right-click→menu handlers (guard them against the drag).
- Render the **drop indicator** (2px accent bar) at the computed insertion gap while
  `tab_drag.is_some()`, and lift the dragged tab (elevation/opacity). Model the drag off the
  grid's existing drag structs (`ResizeDrag`/`ChartDrag`, `grid/view.rs:106-140`).
- Active sheet stays active across the move (active follows `SheetId`, not slot). Drop on
  origin slot = no command.

### 6.4 Tests

Fork: upstream-style unit tests for reorder + undo + xlsx round-trip. FreeCell: worker test
`MoveSheet` reorders `sheet_metas` + undo restores; gpui view test for tab drag threshold /
indicator / drop → command (mouseless: exercise the index-computation helper as a pure fn).
No pixel impact (tab bar not baselined) — smoke launch to eyeball the drag.

---

## 7. Verify right-click insert/delete (§7)

No architecture. Smoke via Xvfb launch + confirm `header_menu_elements` (`grid/view.rs:2592`)
shows correct labels/counts and applies; confirm the merge guard. File a bug if a real gap
surfaces; otherwise close as verified.

---

## 8. Cross-cutting: render validation (per CLAUDE.md)

Only §2 (spill) and §3 (auto-grow) move baselined pixels. Plan (implementation_plan §):

- While coding §2/§3, run **subset** cases only (`render_tests.sh test spill_` /
  `… test autogrow_`).
- A **dedicated final render phase** (after all coding): full suite under a ~10-min
  watchdog, regenerate + **eyeball** the intentional spill/auto-grow baseline changes, commit
  them, then dispatch the CI `render` gate and confirm green.
- §1/§4/§5/§6/§7 validated by gpui view/unit tests + a single Xvfb smoke launch — **no** pixel
  run (they don't touch grid/cell/sheet/titlebar baselines).

## 9. Sequencing constraint

Auto-grow (§3) is the **last** coding phase and independently revertible (func spec §3
sequencing note). The render-validation phase (§8) runs after it. All other features (§1,
§4, §5, §6, §7) are mutually independent and can proceed in any order / in parallel.
