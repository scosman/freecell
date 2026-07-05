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
