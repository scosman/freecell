---
status: complete
---

# Architecture: gaps_closing_7_15

Technical design for the five features in `functional_spec.md`, one section per feature =
one implementation phase, plus the final render-validation phase. Each section gives the
data model, the component/seam breakdown (with the exact files/lines from the research
notes under `research/`), the algorithm for anything non-trivial, and the test plan. The
coding agent executes this; no design decisions are left open (owner decisions D1.1/D2.2/
D4.1 are resolved in the functional spec; the remaining Dx are set to their recommended
defaults here).

**Crate map (unchanged):** `freecell-core` (pure, no gpui/IronCalc — validators, geometry,
selection, tsv), `freecell-engine` (IronCalc-fork adapter + worker: `document.rs`,
`worker/{protocol,run}.rs`, `cache.rs`), `freecell-app` (GPUI shell: `grid/`, `chrome/`,
`shell/`). Only `freecell-engine` may touch IronCalc types.

**Standing conventions:** crate-scoped build/test per phase; `cargo fmt --all --check`
every phase; render **subset** while iterating grid phases; **one** full render suite + CI
`render` gate in the final phase; commit+push regularly; fork = one fix = one `fix/` branch
= one upstream PR, FreeCell pins `freecell-fixes`.

---

## 1. Function autocomplete + signature hints

**Where it lives:** the static function catalog is new data in **`freecell-core`** (pure,
IronCalc-free — the engine's `Function` enum is private and non-enumerable, confirmed in
`research/function-autocomplete.md`). All UI state + rendering lives in **`ChromeView`**
(`freecell-app/src/chrome/view.rs`), which owns both editors and their single pending edit.
**No worker/engine changes, no fork work.**

### 1.1 Data model

New module `freecell-core/src/functions.rs`:

```rust
pub struct FnSig {
    pub name: &'static str,          // "SUMIF"
    pub template: &'static str,      // "SUMIF(range, criteria, [sum_range])"
    pub rank: u16,                   // importance: lower = more common (drives ordering)
}

/// The static catalog (const array). Seeded from
/// experiments/round-2/03-function-parity/data/{ironcalc_functions.csv,
/// excel_functions_canonical.csv}: the 345 engine-registered names (D1.3), each with an
/// authored argument template and an importance rank from the canonical CSV's
/// name/category/importance columns (common < uncommon < rare; unranked → after ranked).
pub const FUNCTIONS: &[FnSig] = &[ /* … generated/curated … */ ];

/// Case-insensitive prefix query. Returns matches ordered:
/// exact-name-first, then by rank, then alphabetical. Caller caps the display count.
pub fn complete(prefix: &str) -> Vec<&'static FnSig>;

/// Exact (case-insensitive) name lookup for the signature hint.
pub fn signature(name: &str) -> Option<&'static FnSig>;
```

**Catalog authoring:** the 345 names come from the committed CSV; templates are authored.
To keep the phase bounded, author real templates for the **common** set (the canonical
CSV's `common` importance rows, ~150) and a **generic fallback** template (`NAME(…)`) for
the long tail — every name still completes; only the arg hint degrades to `NAME(…)` for
rare functions. A unit test asserts every `FUNCTIONS` name is unique and non-empty and that
the common-set names all have a non-fallback template.

**Completion-context detection** (pure, in `functions.rs` or beside `input_cap`): given the
full edit `text` and the caret **byte offset**, compute the *active function-name token*:

```rust
pub struct FnEditContext {
    pub token_start: usize,   // byte offset where the identifier prefix begins
    pub prefix: String,       // the identifier chars [token_start, caret)
}
/// Returns Some(ctx) when the caret is at the end of an identifier in *function position*
/// inside a formula, else None. Rules (D1.4 threshold = ≥1 char):
///  - text starts with '=' (reuse the input_cap formula predicate)
///  - walk left from caret over [A-Za-z0-9_.] to token_start; the char before token_start
///    is start-of-formula or one of ( , + - * / ^ & < > = % (operator/opener) or whitespace
///    → function position; if it's a digit/')'/cell-ref char, NOT a function name.
///  - not inside a string literal (odd count of unescaped '"' before caret → bail)
///  - prefix len ≥ 1
pub fn fn_edit_context(text: &str, caret: usize) -> Option<FnEditContext>;
```

This is deliberately a **lexical heuristic**, not a real parse — sufficient for name
completion and fully unit-testable without IronCalc (matches the D1.1 "static hint, no
tokenizer" decision). The signature-hint "caret inside a call" detection uses a sibling
helper `enclosing_fn_name(text, caret) -> Option<&str>` that scans left for the nearest
unmatched `(` and reads the identifier before it.

### 1.2 UI state on `ChromeView`

Add fields (mirroring the existing `*_open` popover flags):

```rust
autocomplete: Option<Autocomplete>,   // None = list closed
struct Autocomplete {
    matches: Vec<&'static FnSig>,      // current filtered list (capped ~10 shown)
    highlight: usize,                  // selected row index
    token_start: usize,                // byte offset to replace on accept
    origin: EditOrigin,               // which editor the list is anchored under
}
sig_hint: Option<&'static str>,        // active signature template line, or None
```

### 1.3 Keystroke → recompute (the per-keystroke seam)

Both editors already surface `InputEvent::Change` in `on_content_event`
(`chrome/view.rs:1315`) and `on_incell_event` (`:1121`). After each change, call a new
`recompute_autocomplete(origin, window, cx)`:

1. Read the driving editor's `value()` and `cursor()` (byte offset — the pinned
   `InputState` exposes `pub fn cursor(&self) -> usize`, confirmed on disk).
2. `functions::fn_edit_context(text, caret)`:
   - `Some(ctx)` → `matches = functions::complete(&ctx.prefix)`; if non-empty, set/refresh
     `self.autocomplete` (preserve `highlight` if the same token, else reset to 0); else
     clear it.
   - `None` → clear `self.autocomplete`.
3. Independently refresh `self.sig_hint` from `enclosing_fn_name(text, caret)` →
   `functions::signature(name).map(|s| s.template)`.
4. `cx.notify()`.

### 1.4 Keyboard interception (accept / navigate / dismiss)

The two editors preempt the gpui-component Input's own key actions via two existing hooks
(both documented in `research/function-autocomplete.md`); extend **both**, guarded on
`self.autocomplete.is_some()`:

- **Data row:** `cx.intercept_keystrokes` → `handle_data_row_edit_key`
  (`chrome/view.rs:1062-1109`). Before the existing Tab/quick-edit logic, if the list is
  open: `Down`→`highlight = min(highlight+1, len-1)`; `Up`→`highlight = highlight.saturating_sub(1)`;
  `Tab`/`Enter`→`accept_autocomplete(...)`; `Esc`→close the list only (clear
  `self.autocomplete`, do **not** revert). Each consumed key returns/`stop_propagation`s so
  the Input's caret/commit action is suppressed.
- **In-cell:** grid-root `capture_key_down` (`grid/view.rs:4748-4790`). The grid does not
  own the list state (ChromeView does), so add a lightweight query path: the grid asks the
  window/chrome "is autocomplete open?" and, if so, emits new `GridEvent`s
  (`AutocompleteNav(Up/Down)`, `AutocompleteAccept`, `AutocompleteDismiss`) with
  `stop_propagation`, routed via `shell/window.rs` (~:1487-1509) to `ChromeView` methods.
  Rationale: keeps the single source of truth in ChromeView and reuses the established
  grid→window→chrome event path rather than duplicating list state into the grid.

Enter/Tab when the list is **closed** fall through to today's commit-and-move; the guard
ensures the list never changes those keys' meaning when hidden.

### 1.5 Accept algorithm (D1.2 — insert `NAME(`, caret after the paren)

The pinned `InputState` supports targeted editing (`insert`, `cursor`, `selected_range`,
`set_cursor_position`, `set_value`), so we implement the good UX, not the append-only
fallback. `accept_autocomplete`:

1. `ac = self.autocomplete.take()`; `sig = ac.matches[ac.highlight]`.
2. Read the driving editor `text` + `caret`. `insertion = format!("{}(", sig.name)`.
3. `new_text = text[..ac.token_start] + &insertion + &text[caret..]`.
4. Drive the pending edit with `new_text` **exactly as the existing programmatic-text
   path does**: push through the `DataRow` reducer (so cap-validation/mirror stay
   consistent) and `set_value(new_text)` on the driving editor (which suppresses the echo
   `Change`), then `mirror_to_in_cell`/`content_input` sync — reuse the existing
   `set_editor_text`-style helper the code already uses for programmatic updates.
5. Caret: `set_cursor_position` to the offset `ac.token_start + insertion.len()` (i.e. just
   after `(`). Single-line editor ⇒ `Position { line: 0, character: <col> }`.
6. Set `self.sig_hint = Some(sig.template)`; `cx.notify()`.

No trailing `)` is inserted (D1.2); the engine accepts the closing paren at commit (formulas
are parsed on commit; an unbalanced-paren formula that the user completes normally is the
common flow and already handled by the commit path today).

### 1.6 Rendering

- **List popover:** a new `render_autocomplete_popover` modeled on the **cap-error popover**
  anchoring (passive, no backdrop): data-row origin → fixed anchor below the data-row field
  (`render_cap_error_popover` pattern, `chrome/view.rs:4170`); in-cell origin → the measured
  `(x, y+h+2)` anchor used by the in-cell cap popover (`grid/view.rs:4564-4574`). The card is
  a `deferred()` column of rows; the highlighted row uses the existing selected-row styling;
  each row `on_mouse_down` → set `highlight` + `accept_autocomplete`. `max_h` + scroll for
  >~10 matches (reuse the num-fmt popover's `max_h(px(320)).overflow_y_scroll()`).
- **Signature hint:** a one-line passive strip rendered just below the active editor
  (same anchor family), shown when `sig_hint.is_some()` and the list is **not** covering the
  same spot (list takes precedence at the shared anchor).
- Both gated so they never co-render with the cap-error popover at the same anchor (cap
  error wins — it means the edit can't commit).

### 1.7 Tests

- **Unit (`freecell-core`):** `complete()` ordering (exact-first, rank, alpha) and
  case-insensitivity; `fn_edit_context` truth table — `=su`→ctx, `=SUM(A1,su`→ctx at the
  2nd arg, `=A1`→None (cell ref), `="su`→None (string), `su` (no `=`)→None, `=1+su`→ctx,
  caret-in-middle cases; `enclosing_fn_name` for nested calls; every catalog name
  unique/non-empty/common-set-has-template.
- **gpui view tests (`freecell-app`):** typing `=su` opens the list with SUM first;
  Down moves highlight; Enter inserts `SUM(` and places caret after `(`; Esc closes list
  and keeps the edit; the same via the in-cell editor path; sig-hint appears when caret is
  inside `SUM(`. No pixel suite (chrome-only, not a baseline surface).

---

## 2. CSV import + export

**Where it lives:** import wiring in **`freecell-app/src/shell`** (new action + open branch),
CSV parse→cells and used-range→CSV in **`freecell-engine`** (must be engine-side: it builds/
reads `WorkbookDocument` and the IronCalc `Worksheet` is `pub(crate)`). The `csv` crate
(v1.4.0) is already a workspace dep; **add `csv.workspace = true` to
`freecell-engine/Cargo.toml`**. **No fork work.**

### 2.1 Import — open a `.csv` as a new untitled workbook (D2.1)

**Entry + routing.** `do_open_path` (`shell/app.rs:266`) is the funnel. Branch on the
canonicalized path's extension:
- `.csv` (case-insensitive) → **import path** (below), opened as untitled.
- else → today's `open_document(DocumentSource::OpenFile(...))`.

Also: (a) `main.rs:33` `xlsx_arg` is widened to accept `.csv` too (so CLI/Finder args of a
csv route in); (b) a new **`ImportCsv`** app action (`shell/mod.rs`, menu item in
`shell/menus.rs` File menu, handler in `shell/app.rs` opens the existing files-only panel
then feeds the chosen path to the same import path). The welcome window's Open… already
routes through the panel→`open_path`, so it picks up csv for free.

**New `DocumentSource` variant** in `freecell-engine/document.rs`:
`DocumentSource::ImportCsv(PathBuf)`. `from_source` dispatches it to a new
`WorkbookDocument::import_csv(path) -> Result<Self, LoadError>`:

1. Read bytes; decode UTF-8, tolerating a leading BOM (D2.5). On invalid UTF-8 →
   `LoadError::NotXlsx`-sibling: add `LoadError::BadCsv(String)` with a readable message
   → surfaced by the existing "Couldn't open the workbook" dialog path.
2. `new_empty()` (one sheet "Sheet1", name "Untitled").
3. Parse with `csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(bytes)`
   (delimiter `,` — the default; the same crate/config family as `freecell-core/tsv.rs`).
   For each record (row `r`) and field (col `c`): if the field is non-empty, apply it as
   **user input** at (r, c) via the same set-input path `paste_tsv`/`set_cell_input` uses
   (numbers/booleans/`=formula`/text auto-typed by IronCalc). Empty fields → skip (cell is
   already empty in a fresh sheet, so the semantics match "empty clears").
4. **Overflow guard:** before applying, if a record index ≥ 1,048,576 or a field index ≥
   16,384 is reached → abort with `LoadError::BadCsv("larger than the maximum sheet size")`.
   (Check as you stream; don't materialize the whole file first.)
5. Return the document. The window opens it with **`path: None`** (untitled) so Save →
   Save-As-to-`.xlsx` (existing `resolve_save_target(None,…)` → `Untitled.xlsx` prompt), and
   `opened_from: None` ⇒ no `.back` backup. The imported csv path is recorded in Recents
   (it's a file the user opened) via the normal `do_open_path` recents call.

**Untitled/dirty:** treat like a freshly-opened doc — not dirty until edited (the import is
not a user edit; `committed_ops == last_saved_ops` at open).

### 2.2 Export — active sheet used range → `.csv` (D2.2 = raw stored values)

**Entry.** New **`ExportCsv`** window-scoped action (`shell/mod.rs` + `shell/menus.rs` File
menu "Export as CSV…" + handler on the render div in `shell/window.rs:1148`, like `SaveAs`).
It opens a native save panel (`cx.prompt_for_new_path`) proposing `<sheetname>.csv`, then
sends a new worker command.

**Worker command.** `protocol.rs`: `Command::ExportCsv { sheet: usize, path: PathBuf,
req_id }` → `WorkerEvent::CsvExported { req_id } | ExportFailed { req_id, msg }`. Routed in
`run.rs` to `document.export_csv(sheet, &path)`.

**`WorkbookDocument::export_csv`** (engine):

1. `ws = self.worksheet(sheet)`; `dim = ws.dimension()` (1-based inclusive min/max row/col;
   `document.rs:375`). Empty sheet (no populated cells) → write an empty file, return Ok.
2. `csv::WriterBuilder::new().terminator(Terminator::CRLF).from_writer(temp)` where `temp`
   is `new_temp_beside(path)` (`document.rs:1707`).
3. For `row` in `min_row..=max_row`: build a `Vec<String>` of length `max_col-min_col+1`
   where each cell renders its **raw stored value via the `value_token` path**
   (`document.rs:1368` / `copied_value_tokens` — plain number, date **serial** number (no
   date format), `TRUE`/`FALSE`, error string, text verbatim; empty → ""). Trim trailing
   empty fields in the row before `write_record` (no trailing commas past the used range).
   The `csv` writer handles RFC-4180 quoting (comma/quote/newline) automatically.
4. Flush, fsync, `persist_atomically(temp, path)` (`document.rs:1721`). Any IO error →
   `ExportFailed` → standard save-error dialog. **Does not** touch the document's dirty
   flag/path/title (export is a side output).

`value_token` is `pub(crate)`; expose a thin `export_csv` method on `WorkbookDocument` so
the IronCalc types never leave the engine crate.

### 2.3 Tests

- **Engine unit/integration:** `import_csv` on a crafted csv (quoted fields with embedded
  comma/newline/`""`, ragged rows, numbers/bools/formulas/text, BOM, empty file) → assert
  cell values + that it's untitled/`path:None`; oversize csv → `BadCsv`. `export_csv` on a
  fixture workbook → byte-compare the csv (raw values: `0.5` not `50%`, date serial not
  formatted date, formula's computed value, quoting of a cell containing a comma); empty
  sheet → empty file; round-trip `import_csv(export_csv(x))` value stability for a
  values-only sheet.
- **gpui/shell:** the `.csv` open branch selects import; Save on an imported doc prompts
  Save-As `.xlsx`; Export action writes a file and leaves dirty/title unchanged. No pixel
  suite (IO/chrome).

---

## 3. Drag fill handle + series autofill

**Where it lives:** entirely **`freecell-app/src/grid/`** (handle render + a new drag state
machine) plus a **new document method** in `freecell-engine` to seed `auto_fill_*` with a
**multi-cell** source for series detection. **No fork work** — the fork's
`auto_fill_rows/auto_fill_columns` already do progression detection; today's wrapper just
never feeds them a ≥2 seed (`research/grid-fill-hide-autofit.md`).

### 3.1 Handle rendering

In the selection-overlay pass (`grid/view.rs:2721-2758`), after the range/active borders,
draw one **fill-handle square** at the bottom-right corner of `span_rect(selection_range)`,
reusing the chart-handle drawing pattern (`view.rs:2831-2850`, `chart_layer.rs` `HANDLE_PX`,
`rect_div(...).bg(CELL_BG).border_1().border_color(ACCENT)`). Suppress it while editing,
while any other drag is active, and (D3.4 clamp) when the corner is off-viewport for a
whole-row/column selection.

### 3.2 Drag state machine

New field on `GridView`: `fill_drag: Option<FillDrag>` (mirrors `chart_drag`):

```rust
struct FillDrag {
    seed: CellRange,            // the selection at drag start
    target: CellRange,          // current previewed fill region (⊇ seed)
    axis: Option<FillAxis>,     // Vertical | Horizontal, decided by dominant movement
}
```

- **Hit-test:** in `handle_mouse_down` (`view.rs:1183`), before the cell/header arms and
  after the chart arm, test the pointer against the fill-handle square (a small hit rect
  like `HANDLE_HIT_HALF`). Hit → `self.fill_drag = Some(FillDrag{ seed: selection, target:
  selection, axis: None })`, `stop_propagation`, and (like resize) guard the root handler
  with `if self.fill_drag.is_some() { return }` on the next `mouse_down`.
- **Move:** in `handle_mouse_move` (`view.rs:1457`), when `fill_drag` is set: map the pointer
  to a cell via `layout::hit_test`; decide `axis` from the dominant delta (|Δrow| vs |Δcol|
  in cells) — sticky once set unless the pointer returns inside the seed; compute `target` =
  seed extended along `axis` to the pointer cell (supports down/right and up/left). Kick
  `maybe_start_autoscroll` (`view.rs:2080`) — extend its current `DragMode::Cell` gate to
  also fire for `fill_drag`. `cx.notify()` to redraw the preview.
- **Preview:** in the overlay pass, when `fill_drag` is set, draw a 2px accent border rect
  over `span_rect(target)` (reuse the range-border `rect_div` at `view.rs:2737-2749`).
- **Up:** in `handle_mouse_up` (`view.rs:1508`), if `fill_drag` set: if `target == seed` →
  no-op (drop the drag, no event). Dragging **inward** (target ⊂ seed) → no-op (D3.3).
  Else emit a new `GridEvent::FillDrag { seed, target, axis }`; expand the selection to
  `target ∪ seed`; clear `fill_drag`; bump `autoscroll_epoch`.

### 3.3 Fill semantics (series vs copy)

`GridEvent::FillDrag` → `shell/window.rs` → new `Command::FillDrag { sheet, seed, target,
axis }` → `run.rs` → new **`document.fill_drag(sheet, seed, target, axis)`**:

- Build the IronCalc **source `Area` = the seed** (full height×width, **not** clamped to
  1×1 — this is the key change vs `fill_down`/`fill_right`). Determine the fill extent along
  `axis` beyond the seed.
- Vertical → `self.model.auto_fill_rows(&seed_area, to_row)`; Horizontal →
  `auto_fill_columns(&seed_area, to_col)`, where `to_row/to_col` is the far edge of
  `target`. A multi-cell seed lets the fork's `detect_progression` extrapolate (1,2→3,4…;
  Jan,Feb→Mar…); a 1-cell seed naturally falls through to copy (same as today's ⌘D/⌘R). For
  **up/left** fills, pass the reversed target edge so the progression counts down (confirm
  the fork's `auto_fill_*` supports a `to` before the source; if it only fills forward, the
  document method reverses: seed the opposite edge and fill toward the origin — bind the
  exact behavior against the fork at implementation, as gaps_closing_7_12 Phase 2 did).
- One `auto_fill_*` call ⇒ **one undo step**. Overflow: reuse the existing large-op guard
  (reject > the same cell-count cap paste/fill uses today).

**Reuse note:** the existing `fill_down`/`fill_right` (⌘D/⌘R, 1×N seed = copy) stay as-is;
`fill_drag` is the general multi-cell path. Factor the shared `Area`-building + eval/refresh
classification (`run.rs` needs-eval + refresh lists, ~:2000/:3405) so `FillDrag` is
classified like `FillDown`.

### 3.4 Tests

- **Engine:** `fill_drag` with a 2-cell numeric seed down → series (1,2→3,4,5); single-cell
  seed → copy (value+format+relative formula); month seed → Jan,Feb→Mar; up-fill reverses;
  one undo entry; overflow rejected. (Bind exact `auto_fill_*` arg shape against the fork.)
- **gpui view tests:** handle appears at the selection corner; a synthetic drag sets the
  preview target and emits `FillDrag`; selection expands post-fill; inward drag is a no-op.
- **Render subset** while iterating (handle + preview are new pixels): `render_tests.sh test
  <selection/fill prefix>`; new baseline case(s) for the handle and a drag preview deferred
  to the §6 full run.

---

## 4. Hide / unhide rows & columns

**Where it lives:** header-menu items + zero-size geometry in **`freecell-app/src/grid/`**;
hidden-flag read on open + a new hide/unhide command in **`freecell-engine`**; and **two
fork branches** (D4.1 round-trip). Heaviest phase.

### 4.1 Fork work (two independent `fix/` branches → two upstream PRs)

Per CLAUDE.md, check out the fork (`add_repo scosman/ironcalc`, work at
`/workspace/ironcalc`), one focused branch each, upstream-style tests, then integrate on
`freecell-fixes` and re-pin:

- **`fix/row-hidden-setter`** — IronCalc parses `Row.hidden` on import but has no setter.
  Add an **undoable `UserModel` method** `set_rows_hidden(sheet, row_start, row_end, hidden:
  bool)` (name to match IronCalc conventions; mirror the existing `set_row_height`
  diff-list/undo shape). Export already emits `Row@hidden` if the model carries it — verify
  and add if missing.
- **`fix/column-hidden`** — `Col` has **no** hidden field at all. Add `hidden: bool` to the
  `Col` model, **parse** it on import (`<col hidden="1">`), **emit** it on export, and add an
  undoable `set_columns_hidden(sheet, col_start, col_end, hidden)`. Larger of the two.

Keep them separate (never one combined branch — upstream wants single-feature PRs).

### 4.2 Engine adapter

- **Read on open:** `build_sheet_cache` (`freecell-engine/cache.rs:293-347`) currently reads
  `custom_width/custom_height` only. Add: when `r.hidden` / `col.hidden`, record the track in
  a new **hidden set** on the published `SheetCache` (e.g. `hidden_rows: BTreeSet<u32>`,
  `hidden_cols: BTreeSet<u32>`), **and** remember its non-hidden size for unhide (the file
  still carries `custom_*`/default width; unhide restores that). A hidden track's rendered
  size is **0**, but its stored size is preserved for restore.
- **New commands:** `Command::SetRowsHidden { sheet, start, end, hidden }` and
  `SetColumnsHidden { … }` (`protocol.rs`) → `run.rs` → `document.set_rows_hidden(...)` /
  `set_columns_hidden(...)` (thin wrappers over the fork setters; merge-guard not needed —
  hiding doesn't displace merges). Each is **one undo step**. After the op, the worker
  republishes the sheet cache with the updated hidden set + geometry.

### 4.3 Geometry: rendering a hidden track as zero-size

The `Axis` model already tolerates a `0.0` override safely (`freecell-core/axis.rs`:
`index_at` never lands on a zero-size track; offsets/scroll sum correctly;
`research/grid-fill-hide-autofit.md`). Implementation: when building the render `Axis`/frame
from the cache, treat a hidden index as size `0.0` (overriding any stored width/height) so:
no cell, no header, no gridline draws for it; neighbors abut; you cannot click into it; a
range **spanning** it still includes it (selection math is index-based, unaffected).

Keep the hidden set **distinct** from a 0px manual resize (D4.3): hidden is its own state so
Unhide restores the pre-hide size and a genuine min-clamped resize is never mistaken for
hidden.

### 4.4 UI — header context menu

Extend `header_menu_elements` (`grid/view.rs:3279-3411`) item list (the `(label, disabled,
GridEvent)` tuples):

- **Hide** — always enabled for a header selection, **except** disabled if it would hide
  every remaining visible track on that axis (compute from count − hidden − run; if 0 would
  remain, disable). Emits `GridEvent::HideRows { at, count } | HideColumns { at, count }`
  using the existing `run = resize_run_for(axis, index)`.
- **Unhide** — enabled only when the selected header run **contains** a hidden track (the
  menu-open handler at `view.rs:1913-1947` checks the cache hidden set over the run). Emits
  `UnhideRows { at, count } | UnhideColumns { at, count }` (restores each hidden index in the
  run to its stored size).

`GridEvent` → `shell/window.rs` (~:1623-1642 alongside insert/delete) → the new
`SetRowsHidden/SetColumnsHidden` commands (hidden=true for Hide, false for Unhide over the
run).

**Unhide discoverability (D4.2):** reached via a **spanning selection** (select C:E with D
hidden → Unhide) and **Select-All → Unhide** to reveal everything. **No** thick-divider/gap
marker between non-adjacent headers this round (tracked follow-on).

### 4.5 Tests

- **Fork (upstream-style, in the fork repo):** row-hidden setter round-trips through
  save→load; column `hidden` parses/emits and the setter is undoable. Each branch's tests
  live with its PR.
- **Engine:** open a fixture with hidden rows/cols → cache hidden set populated, sizes
  preserved; `set_rows_hidden`/`set_columns_hidden` toggle + one undo entry; save→reopen
  keeps hidden (integration, against `freecell-fixes`).
- **gpui view tests:** Hide item hides the run (published cache reflects it); Hide disabled
  when it would hide all; Unhide enabled only over a span containing a hidden track and
  restores prior size; a hidden track renders zero-size (frame axis size 0) and can't be
  clicked into. **Render subset** while iterating; new hidden-track baseline case deferred to
  §6.

---

## 5. Autofit row height (double-click a row divider)

**Where it lives:** entirely **`freecell-app/src/grid/view.rs`** — mirror the shipped
autofit-**column** pattern onto rows and reuse the wrap-height measurement. **No engine/fork
work** (reuses the existing `SetRowHeights` command).

### 5.1 Hook — add the double-click branch to the row hotspot

The row-resize hotspot exists (`view.rs:3248-3272`) but its `on_mouse_down` calls
`begin_resize` unconditionally. Change it to match the **column** hotspot
(`view.rs:3237-3242`): `match event.click_count { 1 => begin_resize(Row, r, …), 2 =>
autofit_row(r, window, cx), _ => {} }`. The existing `commit_resize` no-op guard
(`view.rs:1642-1645`) prevents the first click of the double-click from adding a spurious
undo/resize.

### 5.2 `autofit_row` + measurement

New `autofit_row(&mut self, index, window, cx)` mirroring `autofit_column`
(`view.rs:1674-1695`): resolve the row run via `resize_run_for` (multi-row selection → each
row; whole-sheet guard → only the divider's row). For each target row compute the fitted
height via a new `autofit_height_for_row(row, window) -> f32`:

- Snapshot every **populated** cell in the row (from the publication + cache, like
  `autofit_width_for_column` snapshots at `view.rs:1714-1742`): `(text, col_w, font_px,
  bold, italic, family, wrap_on)`. Drop the caches lock before measuring.
- Fold `measure_wrap_height`-style math (`view.rs:3050-3084`) over the cells: for each cell,
  measure at its **own** column width — wrap-on cells soft-wrap (line_wrapper), wrap-off
  cells count only explicit `\n` segments (one line each) — `cell_h = lines * line_px(font_px)
  + vpad`. Take the **max** across the row.
- Clamp to `[DEFAULT_ROW_HEIGHT_PX (24), MAX_AUTO_ROW_HEIGHT_PX (240)]` (the wrap-auto-grow
  clamp, `freecell-core/cache.rs`). Empty row → `DEFAULT_ROW_HEIGHT_PX`.

Emit `GridEvent::ResizeCommitted { axis: Row, start: row, end: row, px }` per target row —
reusing the existing `SetRowHeights` command (no new worker command). **One undo step per
row.**

### 5.3 Manual vs auto (D5.1)

Autofit routes through `SetRowHeights`, which **marks the row manual** (`run.rs:758-764`),
so the autofit'd row is thereafter **exempt from live wrap auto-grow** — consistent with the
column autofit and the manual-resize model. This is the resolved D5.1 default (a slight
departure from Excel's "double-click returns to auto-track"; accepted for consistency and
simplicity, recorded in GAPS follow-ons).

### 5.4 Tests

- **Unit:** `autofit_height_for_row` — single-line row → ~default; a cell with two `\n`
  lines → ~2×line; a wrap-on cell narrower than its text → wrapped line count; clamp at 240;
  empty → 24. (Width-calc-style unit test like column autofit's.)
- **gpui view test:** double-click the row hotspot emits `ResizeCommitted{Row,…}` with the
  fitted px; single-click still begins a manual resize; multi-row selection autofits each.
- **Render subset** while iterating (row geometry changes); no new baseline expected beyond
  what the §6 full run covers.

---

## 6. Render validation (final phase)

No new behavior. After phases 1–5 are committed:

1. Regenerate baselines for the intentional grid changes and **eyeball** them: the
   **fill handle** square + drag-preview (§3), **hidden-track** zero-size rendering (§4),
   and any **row-height** shifts (§5). Add new baseline cases for the fill handle and a
   hidden-row/col sheet. (§1/§2 are chrome/IO — not baseline surfaces.)
2. Run the **full** pixel suite under a `timeout` + ~10-min watchdog
   (`app/render-tests/scripts/render_tests.sh test`); fix or accept each diff; commit
   refreshed baselines **with** eyeball sign-off.
3. Dispatch the CI **`render`** gate on the branch (GitHub Actions / `gh workflow run
   render.yml --ref <branch>`), poll to green.

---

## Cross-cutting: error handling, logging, testing posture

- **Errors:** CSV import/export failures surface through the existing dialog paths
  (`LoadError::BadCsv` → "Couldn't open the workbook"; export IO error → save-error dialog).
  Grid features are in-memory ops that can't fail beyond the existing overflow/large-op
  guards (rejected with the standard dialog). Fork setters return `Result` like their
  siblings; the worker maps failures to the existing `WorkerEvent` error arms.
- **Undo:** every data mutation (fill, hide/unhide, autofit, csv import cells) is one engine
  undo step (or, for import, the whole document is fresh so undo history starts empty).
- **Testing:** pure logic (function catalog/context, csv parse/serialize, autofit
  measurement, fill Area-building) is unit-tested in `freecell-core`/`freecell-engine`
  without gpui; UI wiring via gpui view tests; the three grid phases add render **subset**
  checks while iterating and defer the **full** suite + CI gate to §6. Fork changes carry
  upstream-style tests in the fork repo.
- **Build discipline:** each phase builds crate-scoped (`-p freecell-app` / `-p
  freecell-engine` / `-p freecell-core`), `cargo fmt --all --check` every phase, from `app/`.

## 1-phase vs 2-phase note

Single `architecture.md` (this doc). The five features are independent and each maps 1:1 to
a phase; there are no shared sub-components complex enough to warrant separate
`components/*.md`. The phases themselves are the decomposition.
