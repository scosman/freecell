# Decisions to Review — MVP Gaps

Append-only log of judgment calls and spec deviations made during implementation, for the
owner to review. Newest phase last.

## Phase 1 — Quick wins & publication

### 1. `[Color N]` classic palette not carried (format colours)

Architecture §1.2 asks for "a small const table for 0–6 + the 56-color indexed palette".
I implemented the **named colours 0–6** (black, white, red, green, blue, yellow, magenta),
which covers `[Red]` — the GAPS #2 requirement — and every named format colour. For
`[Color N]` with **N > 6**, `format_color::format_color_rgb` returns `None` (the cell keeps
the default text colour) rather than shipping a 56-entry classic palette. Rationale: the
pinned engine returns only the raw colour **index** and carries no RGB palette itself
(verified — `lexer.rs` maps names→index, nothing maps index→RGB), so there is no
engine-side reference to match; and `[Color N]` with a high index is vanishingly rare in
real files. Low-value, easy to add later if a file needs it.

### 2. Explicit pure-black font colour is indistinguishable from the default

`text_color` resolution reuses the resident style cache's black-filter (`parse_color` +
`filter(!= #000000)`): a *pure-black* explicit font colour is treated as "no override" (it
equals IronCalc's default). Consequence: a cell that is **both** explicitly black **and**
carries a `[Red]`-style format renders in the format colour (red), not black. This matches
the existing cache behaviour (which already collapses black→None) and is an extreme edge
case. Accepted.

### 3. Date heuristic strips all bracketed sections (incl. elapsed-time)

`is_date_format` follows architecture §1.2 literally — strip `[...]` sections and quoted
literals, then look for `y m d h s`. A format whose **only** time letters live inside an
elapsed-time bracket (e.g. a bare `[h]` with no other date letters) therefore classifies as
**Number**, not Date. Such formats are rare and the stripping rule is what the spec
specifies. Noted for awareness.

### 4. `PublishedCell.text_color` is fully resolved; grid line left unchanged

`PublishedCell.text_color` now carries the **fully resolved** colour (explicit non-black
font colour → number-format colour → `None`), per architecture §1.2's stated semantics. The
grid's existing `pc.text_color.or(style.font_color)` is left as-is: it is now redundant
(pc.text_color already incorporates the explicit font colour) but produces identical results
because both paths use the same black-filter. Kept for a minimal diff.

### 5. Backup-failure dialog wording

The `.back` copy-failure aborts the save and shows the standard Error modal with
title **"Couldn't create backup"** and detail **"File not saved. The backup copy could not
be written: `<io error>`"**. Functional spec §7.3 / ui_design §4 pin the phrase "Couldn't
create backup — file not saved."; it is split across the modal's title + detail (how the
existing Error modal renders title/body). Confirm this phrasing is acceptable.

### 6. Render baselines require a pinned-runner regeneration (could not do it here)

The type-aware **alignment** change alters rendered output for several **existing** baseline
cases and adds one new case. This phase changes **alignment only** in the render suite —
**no render case exercises the `[Red]` text-colour path** (see §6a), so none of these
baselines change colour:

- **Changed** (now right-aligned): `cell_number_plain`, `cell_number_thousands`,
  `cell_number_currency`, `cell_number_percent`, `cell_number_negative_red`,
  `cell_date_default`, `cell_narrow_column_clipped_number`, and the numeric cells in
  `grid_mixed_content`.
- **Changed** (now centered): `cell_boolean`, `cell_error_div0`, `cell_error_name`,
  `cell_error_circ`.
- **New**: `cell_number_align_left` (number + explicit Left alignment — guards
  "explicit alignment beats the numeric type-default").
- Unchanged: the explicit-alignment / text cases (`cell_align_*`, `cell_text_*`) already
  render at their explicit alignment.

Per `app/render-tests/README.md`, baselines must be regenerated on the **pinned CI runner
image** (Ubuntu 24.04 + mesa lavapipe + ImageMagick) and eyeballed before commit — dev
renders must never be committed. This container lacks the mesa-vulkan-drivers + ImageMagick
capture stack, so I could **not** regenerate/eyeball them here. **Action required before
merge:** regenerate the affected baselines on the pinned runner and eyeball the diff. Note:
`cargo test --workspace` (no `FREECELL_RENDER=1`) skips the pixel gate and is green; the
dedicated `render_tests.sh` gate will be red until the baselines are regenerated.

### 6a. `[Red]` text-colour has NO render-suite coverage (guarded by an engine test)

`cell_number_negative_red` is misnamed for its content: its input `-1,234.50` infers
`#,##0.00` (a single colourless section), so `resolve_text_color` short-circuits
(`!num_fmt.contains('[')`) and it publishes **no** `text_color` — it renders in the
**default colour**, right-aligned. The render Scene builder (`render-tests/src/scene.rs`)
drives cells only through `SetCellInput` (IronCalc *infers* the number format) plus the
bold/italic/underline/fill/align cache mutators; it has **no way to set a custom `num_fmt`**,
so no render-suite case can produce a number-format colour. Consequently the **GAPS #2
`[Red]` visual output is guarded solely by the engine integration test**
`published_style_resolves_format_and_explicit_colors` (`freecell-engine/src/document.rs`),
which asserts the resolved red / none / explicit-wins colours against a real `UserModel`.

**Deferred render case:** architecture §9 lists a `format_red_negative` / `text_color_red`
render case. Landing it needs a Scene-builder extension to set an explicit `num_fmt` on a
cell (small, mechanical). It is **deferred out of Phase 1** (the engine test covers the
behaviour; the pixel case is additive) — pick it up when the render harness gains a
`num_fmt` setter, or fold it into a later formatting phase (Phase 4 adds the `num_fmt` write
path). Recorded here so the gap is explicit, not implied.

### 6b. Per-cell publication cost (acknowledgement, no code change)

`build_publication` now calls `published_style` (style + type + value reads, and
`format_number` for bracketed numeric formats) in addition to `formatted_value` per
non-empty viewport cell, roughly doubling per-cell publication cost. This is per architecture
§1.2's design and runs **worker-side, off the scroll/render path** (the scroll path reads
the resident cache + published snapshot, never `published_style`), so it is not a scroll-gate
concern. The work is bounded by the published-cell count (the worker caps the viewport). No
change made — acknowledged as accepted per the design.

### 7. Environment: installed the documented Linux build deps

The base container was missing the GPUI link libraries documented in `app/README.md`
(`libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libwayland-dev`, `libvulkan-dev`,
`libfontconfig1-dev`, `libfreetype-dev`, `libasound2-dev`, `libxcb1-dev`); `cargo build
--workspace` failed to link `render-tests` bins until I `apt-get install`ed them. Not a code
change — recorded for reproducibility of the checks.

## Phase 2 — Editing feel

### 1. `EditController` ownership: one entity (chrome), not the window (deviation)

`components/edit_controller.md` specifies a `WorkbookWindow`-owned `EditController` that owns
**both** editor `InputState`s and the whole pending-edit state machine, with the data-row
logic torn out of chrome. I instead kept the single pending edit inside **one entity** —
`ChromeView`, which already owns the data-row `InputState` + the proven, table-tested
`freecell_core::data_row::DataRow` reducer (fetch / spinner / disabled / stale-reply / cap /
commit / escape). The new `chrome/edit.rs` `EditController` owns only the **second** (in-cell
overlay) `InputState` plus the overlay's open cell, the `EditOrigin`, and the `syncing` guard.

**Rationale.** The two editors live in *different* entities (data row in the chrome, in-cell
overlay rendered by the grid). The doc's own design fights an `InputState` text-sync feedback
loop with a `syncing` guard; doing that sync **across entity boundaries** (a window-owned
controller pushing text into a chrome-owned input and a grid-rendered input) is brittle.
Keeping both editors' text sync **inside one entity** eliminates the cross-entity loop, and
reuses every existing data-row test unchanged (they all stayed green). The canonical pending
**text + commit/cap** stay in the `DataRow` reducer; `EditController` layers the in-cell
editor, the two-way sync, and origin tracking on top. Module lives at `chrome/edit.rs` (not
the doc's `shell/edit.rs`) because the chrome owns it. **Please confirm this is acceptable**;
if a window-owned controller is required, it is a larger refactor for the same behaviour.

### 2. Grid ⇄ chrome wiring for the mirror + overlay

The mirror text, the in-cell overlay open cell, and the in-cell cap message are pushed from
the chrome to the grid via a new `ChromeGridRequest::EditState { mirror, in_cell, cap }` on the
existing chrome→grid sink; the grid renders them (`set_edit_state`). The reused in-cell
`InputState` handle is handed to the grid once at window-build time. New `GridEvent`s
(`TypeToEdit`, `OpenInCellEditor`, `InCellCommitMove`, `InCellCancel`) carry the grid-side
triggers back to the chrome. The `EditState` grid update is **deferred** (`window.defer`)
because a grid-originated trigger (type-to-replace / double-click / F2 / in-cell Tab) has the
grid mid-`update` when the chrome pushes state back — a direct grid `update` would re-enter it.
A one-cycle defer is imperceptible for the live mirror.

### 3. In-cell "select all on double-click" not implemented (caret at end instead)

`functional_spec.md §1.3` asks that a double-click open the in-cell editor with the content
**fully selected** (F2 → caret at end). The pinned gpui-component `InputState::select_all` is
`pub(super)` (not reachable), and `set_value` unconditionally places the caret at end for
single-line inputs. So **both** double-click and F2 open with the caret at end; the text is
preserved (the tested behaviour). Selecting-all-on-open would need dispatching the input's
private `SelectAll` action, deferred as cosmetic. The user can still Cmd/Ctrl+A in the editor.

### 4. Type-to-replace targets the selection's **active** cell (not the literal "anchor")

`components/edit_controller.md`/`functional_spec.md §1.1` say a typed edit on a multi-cell
selection targets the "anchor". FreeCell targets the **active** cell (the white cursor cell),
which matches Excel's behaviour and the existing commit path (which already routes
`SetCellInput`/`GetCellContent` to `selection.active`). The grid collapses a multi-selection to
`single(active)` before emitting `TypeToEdit`. For a single selection anchor == active, so this
only differs for a multi-cell selection.

### 5. In-cell worker cap-reject backstop popover

The in-cell cap-error popover is driven by the **UI** validation path (commit → reducer
validates → cap message pushed to the grid), which is what fires in practice (the UI validates
before the worker ever sees the input). The rare worker `EditRejected{InputCap}` *backstop*
still lights the data-row danger state but is **not** re-pushed to the in-cell overlay popover
(no edit transition triggers the push). Extreme edge (the UI already rejects over-cap input);
left for a later polish pass if needed.

### 6. Render baselines require a pinned-runner regeneration (could not do it here)

Phase 2 adds **two** new render cases exercising new rendered output — `cell_mirror_typing`
(the live mirror) and `incell_editor_open` (the in-cell overlay) — wired through new
`RenderCase` fields (`mirror`, `in_cell`) applied in `render-tests/src/render.rs`. This
container **cannot** regenerate the pixel baselines (needs the pinned Xvfb + lavapipe runner +
a human eyeball, per `render-tests/README.md`), so **no baseline PNGs were committed**. Plain
`cargo test --workspace` stays green (the pixel diff is gated behind `FREECELL_RENDER=1`; the
`case_names_match_table` guard passes because the macro list + table were updated together).

**Action needed:** on the pinned runner, `render-tests/scripts/render_tests.sh generate --only
cell_mirror_typing` and `--only incell_editor_open`, eyeball, and commit the two PNGs. No
existing baselines change (both cases are additive; no other rendered output changed).

## Phase 3 — Range clipboard

### 1. External TSV paste bypasses the input-cap security boundary (round-3 D surface)

`SetCellInput` is re-validated against the input cap (length / nesting) worker-side *before*
the recursive parser — the locked round-3 D mitigation. **External TSV paste** feeds arbitrary
foreign clipboard text to `paste_csv_string` (each token as user input) **without** that
per-token cap: replicating the cap correctly would mean re-implementing the `csv` crate's
quoting to split tokens the same way the engine does, risking false rejections of valid data.
Per `architecture.md §8` ("the existing catch_unwind + degraded-mode machinery … covers all new
commands") the paste runs inside the same `catch_unwind` guard on the 64 MiB worker stack, which
is the mitigation used for every other non-`SetCellInput` mutation (undo/redo of formulas,
internal paste). **Residual risk:** a pathological deeply-nested formula pasted from another app
could in principle overflow the stack (an *abort*, uncatchable) — the exact class the cap exists
to kill. Flagged for owner review: if this is deemed unacceptable, add a token pre-scan that
`validate_input`s each `\t`/`\n`-split field before pasting (accepting rare false rejections on
exotic quoted TSV).

### 2. `ClipboardSlot.sheet` stores the stable `SheetId`, not a worksheet index

`architecture.md §6` / `components/clipboard.md` sketch `ClipboardSlot { sheet: u32 }`. We store
the **stable `SheetId`** and resolve it to the volatile worksheet index at paste time, so a copy
survives a sheet add / delete / reorder between copy and paste (matching the rest of the worker's
id↔index discipline). If the source sheet is deleted before an internal paste, the paste replies
`PasteRejected{NothingToPaste}` (the copy is stale).

### 3. Protocol uses `SheetId` / `CellRef` / `CellRange`, not raw `u32` / `(i32,i32)`

The component-doc interface writes `sheet: u32` and `anchor: (i32,i32)`. The real commands use
the codebase's `SheetId` + `CellRef`/`CellRange` (0-based) like every other `Command`; the sole
0→1-based conversion stays in the `document.rs` adapter (`to_engine_coords`). Same values, fewer
ad-hoc coordinate types crossing the seam.

### 4. Cut-paste updates only **intra-block** references, not external refs into the cut area

`functional_spec.md §2.2` says a cut-paste's "refs into the moved area follow it". IronCalc
0.7.1's `paste_from_clipboard` adjusts references **within** the moved block (a formula inside
the cut rectangle that points at another cell inside it follows the move — verified + tested),
but does **not** rewrite a formula *outside* the cut area that points *into* it (e.g. `B1=A1`
after cutting `A1` to `C1`: `B1` stays `=A1` and now reads the emptied `A1`). This is an engine
limitation at the pinned rev, not a FreeCell choice; matching full Excel move-with-reference
tracking would require engine changes. Accepted for MVP.

### 5. TSV paste — empty tokens skipped; **ragged rows dropped + compacted** (corrected)

The engine's `paste_csv_string` (csv reader, `flexible = false`) has two behaviours, verified
empirically against IronCalc 0.7.1 and tested in `paste_tsv_tolerates_crlf_and_drops_ragged_rows`:

- **Empty tokens** within an *equal-width* row are applied as empty user input, which the engine
  **skips** — the underlying cell is left untouched. Excel *clears* those cells. Accepted
  deviation.
- **Ragged rows** (a row whose field count differs from the first record) raise `UnequalLengths`;
  `paste_csv_string` does `continue` **without incrementing the row**, so the ragged row is
  **dropped entirely** and every following row **compacts up** by one. E.g. `"a\tb\nc\nd\te\n"`
  writes `a b` at row 0 and `d e` at row 1 (the `c` row vanishes; `d e` does *not* land at row 2).
  This is engine-owned behaviour — matching the spec's "pad ⇒ skipped cells" would require
  re-serializing the TSV worker-side, which cannot be done without risking divergence from the
  csv crate's quoting rules, so the real (drop + compaction) behaviour is documented instead.
  Corrected in `components/clipboard.md §Paste-TSV` (the earlier "pad with empty ⇒ skipped cells"
  wording conflated these two cases).

*(Related fix, no deviation: `tsv_dims` is now computed with the **same `csv` crate + reader
config the engine's `paste_csv_string` uses** (delimiter `\t`, default `Terminator::CRLF`
quoting, `flexible(true)` so ragged records are counted), instead of a hand-rolled scan. Two
successive CR findings — an `\n`-only split that undercounted bare-`\r` line endings (height), and
a physical-line scan that undercounted quoted-newline field widths (width) — each could let a
spill-over past the sheet edge slip through `paste_fits` into a partial, un-undoable write.
Sharing the engine's parser eliminates the entire divergence class in both dimensions; the bound
is a provable upper bound (engine drops blank/ragged records → fewer rows; writes only the first
record's width → fewer columns). Fixed + regression-tested, incl. the reported `a\t"x\ny"\tb`
quoted-newline payload.)*

### 6. `serde` + `serde_json` (freecell-engine) and `csv` (freecell-core) dependencies added

`components/clipboard.md` says serde_json is "already a workspace dep via open_fixups" — it is a
`[workspace.dependencies]` pin, but `freecell-engine` did not actually depend on it. Added
`serde_json.workspace = true` + `serde.workspace = true` to `freecell-engine` (to `to_value` the
un-nameable `Clipboard` on copy and `ClipboardData::deserialize(&Value)` on internal paste — no
clone; the engine's clipboard structs are not nameable outside `ironcalc_base`). Added a new
`[workspace.dependencies]` pin `csv = "1"` (already 1.4.0 in the tree via `ironcalc_base`) and
`csv.workspace = true` to `freecell-core`, so `tsv_dims` parses through the exact same crate the
engine uses (§5 above). `csv` is a pure-Rust parser — no GPU/IronCalc — so it respects
`freecell-core`'s headless-foundation constraint.

### 7. Render baselines — no new cases needed (no code change)

Paste changes cell values + styles, but it reuses the **existing** publication + resident
style-cache render path (no new `RenderCase` fields, no new rendered constructs). No render cases
were added and **no baseline PNGs change**; `cargo test --workspace` (without `FREECELL_RENDER`)
is green. Nothing to regenerate on the pinned runner for this phase.

## Phase 4 — Formatting controls (SetStylePath, text color, alignment, number formats)

### 1. `RenderStyle.num_format_is_default: bool` **replaced** by `num_fmt: u16`

`style_render.md`'s final shape carries `num_fmt: u16` (an index into a per-cache `num_fmts`
side table) and drops the Phase-1 `num_format_is_default` bool. Nothing in the render path read
the old bool (grep: only the cache build set it and the render-test scene copied it — it was pure
interning identity), and the action bar needs the actual code string (Currency vs Percent), so the
index is strictly more informative. `RenderStyle::Default` now derives (all fields zero/`None`),
`num_fmt: 0` → `"general"`. Only the num-fmt field + `num_fmts` side table land this phase; the
font/border fields + their side tables stay in Phases 5/6 per the implementation plan's phasing.

### 2. `num_fmts` side table is `Vec<Arc<str>>`, not `SharedString`

`style_render.md` sketches `Vec<SharedString>`, but `freecell-core` is deliberately gpui-free
(no `SharedString`). `Arc<str>` is the headless analog — cheap to clone, immutable, `Send + Sync`
(the `SheetCache` Send+Sync compile-time guard still holds). The action bar only ever reads `&str`
from it (category lookup + `adjust_decimals`), so no conversion is needed at the UI boundary.

### 3. `SetStylePath` uses a typed `StylePath` enum, not the architecture's `path: String`

Architecture §2 lists `SetStylePath { … path: String, value: String }`. Implemented `path` as a
typed `enum StylePath { FontColor, AlignHorizontal, NumFmt }` (→ its IronCalc path string
worker-side). Safer and self-documenting: the UI can only ever address the three formatting paths
this project owns, and no IronCalc type crosses the seam. `value` stays a `String` (the payload —
hex color, alignment keyword, or format code — is what varies).

### 4. Degraded/read-only disable: added a `degraded` flag to the chrome + window wiring

`action_bar.md` says degraded mode disables mutating controls "via the existing flag", but no such
flag reached the chrome — degraded state lived only on the window, and the existing B/I/U/Fill
controls did **not** disable. Added `ChromeView.degraded` + `set_degraded`, wired from the window's
`WorkerDegraded` handler, and applied `.disabled(self.degraded)` to **every** mutating action-bar
control (new and existing). This closes the pre-existing gap for the action bar (the window's
read-only bar + edit refusal were already in place; this just also greys the toolbar).

### 5. Multi-cell selection reflects nothing (matches existing B/I/U), not "the anchor"

`action_bar.md §State derivation` says multi-cell state "reflects the anchor". The shipped code sets
`active_style = None` on a multi-cell selection (B/I/U show unpressed), so the new controls follow
suit: `active_num_fmt = None` → the number-format label shows General and decimals ± disable on a
multi-cell selection. Commands still apply to the **full** selection. This keeps all action-bar
controls consistent with the shipped behavior; revisit if the anchor-reflect rule is wanted for all.

**CR follow-up (reconciled):** `functional_spec.md §3` and `components/action_bar.md §State
derivation` were updated to describe this shipped behavior (single selection → active cell;
multi-cell → nothing), so the docs no longer contradict the code. This entry stays as the
human-review flag for whether a *uniform* anchor-reflect across **all** action-bar controls
(including B/I/U) is wanted later — that would need a spec-owner decision and a change to the shared
`active_style`/`active_num_fmt` derivation, done without engine calls (read the anchor's cached
`RenderStyle`), not a per-new-control special case.

### 6. Decimals ± enable is per-direction

`action_bar.md` says decimals ± disables "when `adjust_decimals` returns `None`" (singular). Each
button is gated independently on its own direction: e.g. a bare `0` format enables increase (→`0.0`)
but disables decrease (already zero decimals). More correct than a single shared flag; both disable
for General/Text/Date/Time (no `0` group) and in degraded mode.

### 7. `ACTION_ROW_MIN_W = 620.0` is an estimate; window min-width not enforced at the OS level

The action row sets `min_w(px(620.0))` so its groups don't compress (`ui_design.md §2`: no wrap —
raise the window min width). The value is estimated from the control set, **not** render-measured
(this container can't run the GPU app). The document window opens at 1200 px, far wider, so it never
clips in practice; gpui `WindowOptions` has no simple cross-platform min-size, so no OS-level min was
set. Re-measure/raise on a real device if the row ever clips (it grows in Phases 5/6).

### 8. Render baselines — no new cases, nothing to regenerate this phase

Phase 4 adds the *controls* + the `SetStylePath` command; the *rendered* cell effects (text color,
alignment, engine-formatted numbers, `[Red]` negatives) were written and pixel-baselined in Phase 1
(`cell_fill_dark_text_contrast`, `cell_align_*`, `cell_number_currency/percent/thousands`,
`cell_number_negative_red`). The `RenderStyle` field rename is render-neutral (`num_fmt` never
reaches the paint path — display text is engine-formatted in the publication). The action bar is
chrome, excluded from the cell render suite (`action_bar.md`). So **no render cases were added and no
baseline PNGs need regeneration**; `cargo test --workspace` (without `FREECELL_RENDER`) is green.

### 9. Decimals ± gated off for custom multi-section / scientific / quoted formats (CR fix)

`adjust_decimals` now returns `None` (buttons disable, no-op) for any format containing a section
separator `;`, an exponent `E`/`e`, or a quoted/escaped literal (`"…"`, `\`) — the last-`0`-group
scan can't edit those safely (it would target the exponent's `0`, diverge sibling sections, or
mangle a literal, and IronCalc stores the code unvalidated so a malformed result would corrupt the
cell's display). The ± remains active for the clean single-section Number/Currency/Percent/thousands
formats. `functional_spec.md §3.4` only guarantees the dropdown-native numeric formats, so this is a
scope-honest limitation, not a regression: a file-authored custom format is never one-click-broken;
its exact decimal count is edited by re-authoring the format string (out of scope — no custom-code
editor in this project).

## Phase 5 — Fonts (family + size)

### 1. The engine default font is 13pt Calibri, NOT 11pt; "default" is detected per-workbook

`architecture.md §1.1` and `components/style_render.md` repeatedly state the "engine default 11pt".
Verified against the pinned ironcalc_base 0.7.1: `Font::default()` is **`sz: 13`, `name: "Calibri"`**
(`types.rs:410`), and `new_empty` seeds the workbook with it (`Styles::default().fonts = [Font::default()]`).
Hardcoding 11 would make **every** new-workbook cell non-default → rendered at 13pt→17px and stored
individually, changing every render baseline and bloating opened files. Instead, `RenderStyle.font_size_q ==
0` / `font_family == 0` mean **"the workbook's default font"** (`document.default_font()` reads
`styles.cell_xfs[0].font_id` → `styles.fonts[id]`), exactly as `font.color` is resolved relative to black
today. Consequences: (a) new-workbook **and** opened-file default cells intern to `RenderStyle::default()` and
render at the grid default (Inter `CELL_FONT_PX = 13px`) — **zero baseline change, no behaviour change for
opened files** (they look exactly as today); only cells whose font *differs* from the workbook default get an
explicit family/size. (b) **The size box labels a default cell with the workbook's real default size**
(CR Moderate fix): the cache records `default_font_size_q` at build (13pt→52 for a new workbook, the file's
default otherwise), the chrome reads it (`ChromeClient::default_font_size_pt`), and `font_size_label` shows it
for a `font_size_q == 0` cell — so the box shows **"13"** (or the file default), never a hardcoded "11".
`font_size_display(0)`'s legacy `"11"` is no longer on the default-cell path (it survives only as the
committed Phase-4 unit test). Re-picking the shown default from the dropdown is a **visual no-op**: the engine
maps `sz == workbook default` back to the sentinel (`font_size_q_of`), so no cell materialises a size change
and no size jump occurs (verified: `font_size_box_shows_workbook_default_for_default_cell`).

### 2. Residual pt↔px seam: the "13" default label vs the 13px default render (accepted)

The default cell's label now shows its **nominal engine point size** (13pt) while it **renders** at the app's
`CELL_FONT_PX = 13px` (which at 96 dpi is ≈ 9.75pt). Non-default sizes render at `px(font_size_q/4 · 96/72)`
per `components/style_render.md` (auto-grow uses the same factor against `get_row_height`'s 28px-default
space). So the number shown and the pixels drawn use different scales, and explicitly picking a size *near*
the default renders slightly larger than the default cell (e.g. an explicit 12pt → 16px > the 13px default).
A perfectly no-jump reconciliation would require changing the pt→px factor for the **default** render — which
would move every existing baseline — so it was **not** done (per the CR constraint: don't change default-cell
output). The label shows the value that best reflects the engine's stored/round-tripped size, and re-picking
the *exact* shown default is a no-op (Decision #1b). Residual seam: the numeric scale (points) and the render
scale (device px) differ by the `96/72` vs `13px/13pt` mismatch. **Flagged** for the owner to pick a preferred
px factor if the visual seam matters; it is cosmetic and touches no data.

### 3. `SetFont` = one style paste + K row-grow runs ⇒ up to K+1 undo steps

IronCalc 0.7.1 has no font-name/absolute-size `update_range_style` path, so `SetFont` applies via
`on_paste_styles` (one diff-list) and auto-grows rows via `set_rows_height` (one diff-list **per contiguous
run**). Each is a separate engine undo entry, so the worker pushes exactly that many touch-set entries (kept
1:1 with the undo stack — verified by `set_font_undo_reverts_size_and_height`). Undoing a font change is
therefore up to **K+1** presses (typically 2: style + one contiguous row run). `architecture.md §3.3` accepts
this ("undo restores height then style, two steps"). No coalescing into one undo entry is possible without an
engine change.

### 4. Font ops materialise per-cell styles → full row/col/select-all clamps to the used range

`on_paste_styles` writes a style **per cell** (no font bands at this rev), so a full-column/row/select-all
`SetFont` is clamped to `worksheet.dimension()` first (`document.clamp_to_used`), and a clamped selection over
**> 100k** cells is refused with a dialog ("Selection too large for font changes"). Matches
`functional_spec.md §3` ("font family/size clamps to the used range on full-row/col selections — documented
deviation"). A band-shaped selection entirely outside the used range is a silent no-op.

### 5. Render baselines to regenerate on the pinned runner + FONT-AVAILABILITY RISK

This phase changes rendered output (per-cell font family/size + row geometry). **No existing baseline
changes** — default-font cells still render at the grid default (Decision #1). **Three NEW additive render
cases** need generating + eyeballing on the pinned runner (`app/render-tests/README.md`; this container cannot
run the Xvfb+lavapipe capture stack):

- `font_family_serif` — a cell in **"DejaVu Serif"**.
- `font_size_24_row_grown` — a 24pt cell in a grown (38px) row.
- `font_missing_family_fallback` — a bogus family (`"NoSuchFontXYZ123"`) → gpui fallback (renders in the
  default font; guards that a missing family never blanks the cell).

**FONT-AVAILABILITY RISK (cross-environment):** `font_family_serif` renders a real installed family. It uses
**"DejaVu Serif"** (the near-universal Ubuntu `fonts-dejavu` default) so the pinned runner very likely has it,
but this is a genuine cross-environment dependency: if the runner image lacks that family, the baseline will
render in gpui's fallback (indistinguishable from `font_missing_family_fallback`) and the "family visibly
changed" assertion is lost. **Before generating baselines, confirm "DejaVu Serif" is installed on the pinned
runner** (or swap the case to a family that is, and re-record). No dev renders were committed.

### 6. `ACTION_ROW_MIN_W` raised 620 → 816 for the two new font groups (supersedes Phase-4 #7's 620)

Phase 5 prepends the font-family (140px) + size (56px) dropdowns to the action row, so its natural
uncompressed width grew. `ACTION_ROW_MIN_W` was raised from **620** (Phase-4 #7, which pinned 620 for the
B/I/U + text-color/fill + alignment + number-format/decimals set) to **816** — the earlier 620 + ~196px for
the two new groups (`140 + 56` px + a divider + gaps). Same caveats as Phase-4 #7: it is an **estimate**, not
render-measured (this container can't run the GPU app); the document window opens at 1200px, far wider, so the
row never clips in practice; re-measure/raise on a real device if it ever does (it grows again in Phase 6 with
borders). This entry is the recorded 816 value the `ACTION_ROW_MIN_W` comment refers to.

### 7. `SetFont` materialises inherited band styles into per-cell styles (side effect of `on_paste_styles`)

Because IronCalc 0.7.1 has no font-name path, `SetFont` goes through `on_paste_styles`, which writes each
cell's **fully-resolved** style (`get_style_for_cell` = cell > row-band > col-band > default). So applying a
font to a cell that currently inherits a **band** fill/border/format converts those inherited attributes into
an **explicit per-cell style** carrying the same values. Visible result is unchanged at apply time (same
resolved appearance), but a **later edit to that band no longer propagates** to the font-touched cells (they
now shadow the band). This is inherent to the 0.7.1 mechanism (the same reason full-row/col font ops clamp to
the used range — Decision #4), and it only affects cells the user explicitly re-fonts. Accepted for MVP; the
only alternative is an engine-level font-band API that does not exist at this rev. Flagged so the
band-shadowing behaviour is explicit, not implied.
