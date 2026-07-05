---
status: complete
---

# Component: Range Clipboard

## Purpose and scope

Cut/Copy/Paste of cell ranges: engine-native internal payload (values + formulas with
reference adjustment + styles, undoable) and plain-text TSV interop with other apps.
NOT responsible for: rich Excel flavors, paste-special, tiling, marching-ants
(`projects/excel-clipboard.md`); text clipboard inside the input fields (native).

Split: a UI-side `ClipboardCoordinator` (shell) + worker-side handling of three
commands. Architecture refs: §2, §6 (all engine facts verified there).

## Public interface

UI side (`freecell-app/src/shell/clipboard.rs`):

```rust
pub struct ClipboardCoordinator { last_copy_text: Option<String> }

impl ClipboardCoordinator {
    pub fn copy(&mut self, sel: &SelectionModel, cut: bool, worker, cx);  // Cmd+C / Cmd+X
    pub fn paste(&mut self, sel: &SelectionModel, worker, cx);            // Cmd+V
}
```

Worker protocol (freecell-engine/src/worker/protocol.rs):

```rust
Command::CopySelection { sheet: u32, range: CellRange, cut: bool }
    // reply: Result<String /*TSV*/, WorkerError>
Command::PasteInternal { sheet: u32, anchor: (i32, i32) }
    // reply: Result<(), WorkerError>   Err kinds: NothingToPaste (log), Overflow (dialog)
Command::PasteTsv { sheet: u32, anchor: (i32, i32), text: String }
    // reply: Result<(), WorkerError>   Err kinds: Overflow (dialog)
```

Worker-held state:

```rust
struct ClipboardSlot { sheet: u32, range: (i32,i32,i32,i32) /*ClipboardTuple*/,
                       data: serde_json::Value /*ClipboardData*/, cut: bool }
// worker run-loop field: clipboard: Option<ClipboardSlot>
```

## Internal design

### Copy / Cut (worker)

1. Set the engine's view selection (the ONLY hidden-state dance in the project,
   architecture §6): `set_selected_cell(range.anchor)` then `set_selected_range(range)`
   — anchor must lie on the range edge (`ui.rs:151-165`); use the range's top-left as
   anchor always (top-left is on the edge by construction).
2. `copy_to_clipboard()` — engine clamps to `dimension()`, so full-column/select-all
   copies are cheap and the *effective* copied range is the engine-returned `range`
   field, not the request.
3. `serde_json::to_value(&clipboard)` → stash `ClipboardSlot { sheet, range: clip.range,
   data: value["data"], cut }` (fields reached via the serialized value — the concrete
   types are `pub(crate)`, verified). Reply with the `csv` field (tab-separated
   formatted text, `\n` rows).
4. UI on reply: `cx.write_to_clipboard(ClipboardItem::new_string(tsv))`;
   `last_copy_text = Some(tsv)`.

Cut is the same with `cut: true` recorded; **nothing clears at cut time** (functional
spec §2.1). A later copy/cut simply replaces the slot.

### Paste (UI decision → worker)

UI reads the system clipboard text:
- `None`/empty → no-op.
- `Some(text) == last_copy_text` → `PasteInternal { anchor }`. (Known, accepted edge:
  another app copying byte-identical text would false-match; consequence is pasting
  the full-fidelity payload instead of plain text — near-identical outcome. Requires
  the slot to exist worker-side anyway.)
- else → `PasteTsv { anchor, text }`, and `last_copy_text = None` (our copy is no
  longer newest).

Pending edit commits first (existing click-elsewhere rule via EditController).

Worker `PasteInternal`:
1. Slot missing → `NothingToPaste` (UI logs, no dialog).
2. Overflow pre-check: `anchor + (range.height, range.width) - 1` must fit in
   1,048,576 × 16,384 → else `Overflow` (dialog, functional spec §2.2).
3. Select anchor (`set_selected_cell` + single-cell `set_selected_range`), deserialize
   `ClipboardData` from the slot's JSON, call
   `paste_from_clipboard(slot.sheet, slot.range, &data, slot.cut)` — one undoable diff
   list; engine does relative-ref adjustment (copy) or move semantics (cut) and
   re-selects the pasted area; mirror that new selection back to FreeCell's
   `SelectionModel` (reply carries the pasted range → shell updates selection).
4. `slot.cut` → clear the slot (second paste of a cut is a no-op → `NothingToPaste`).

Worker `PasteTsv`:
1. Parse dims FreeCell-side (`freecell-core::tsv::tsv_dims`) with the **same `csv` crate + reader
   config the engine's `paste_csv_string` uses** (`ReaderBuilder`, delimiter `\t`, no headers,
   default `Terminator::CRLF` — `\r`, `\n`, `\r\n` each terminate — default `"` quoting with `""`
   escapes) rather than a hand-rolled scan, so the bound can never diverge from the engine's own
   parse: a quoted field containing tabs/newlines is ONE field on ONE record, blank records are
   skipped, mixed terminators honoured. `height` = record count, `width` = max field count. Read
   with `flexible(true)` (so ragged records parse and are counted) — a **provable upper bound** on
   BOTH dims: the engine (`flexible = false`) drops blank + ragged records (rows `<= height`) and
   writes only the first record's width (columns `<= width`). `paste_fits` therefore never lets a
   spill-over past the sheet edge reach the engine — a partial write would land outside the single
   undo entry (§2.2 "no partial paste"). *(CR follow-ups: an earlier `\n`-only split undercounted
   bare-`\r`; a physical-line scan undercounted quoted-newline widths — both bypassed the guard;
   the shared-`csv`-parser approach fixes the whole divergence class.)*
2. Overflow pre-check as above (dialog).
3. Build `Area { sheet, row: anchor.0, column: anchor.1, width, height }` and call
   `paste_csv_string(&area, &text)` (tab-delimited, values-as-user-input, undoable — verified).
   The engine (`flexible = false`) has two accepted behaviours (DECISIONS_TO_REVIEW §Phase 3 #5):
   - **Empty tokens** within an equal-width row are **skipped** — the underlying cell is left
     untouched (Excel *clears* it): accepted deviation.
   - **Ragged rows** (a differing field count) error and are **dropped without advancing the
     row**, so any following rows **compact up** into the gap: accepted engine behaviour, not
     "pad with empty".
4. Selection becomes the pasted area (shell, from the reply).

### Keymap & eval

Cmd/Ctrl+C/X/V bind in the grid keymap only (grid focused); the data-row / in-cell
inputs keep native text clipboard behavior. Paste commands run through the normal
apply→evaluate pipeline (they change values); Copy does not evaluate.

## Dependencies

Depends on: worker command/reply plumbing, EditController (commit-first), selection
model, gpui clipboard (`write_to_clipboard`/`read_from_clipboard`), serde_json (already
a workspace dep via open_fixups). Depended on by: header-selection UX (copy of full
columns relies on engine clamping), future fill/paste-special projects.

## Test plan

Engine integration (freecell-engine, real UserModel — no UI):
- `copy_paste_values_and_styles_roundtrip` — values, bold+fill styles arrive.
- `copy_paste_adjusts_relative_refs` — `=A1` copied 1 down pastes as `=A2`; `$A$1` holds.
- `cut_paste_moves_and_source_cleared` — refs pointing into the cut area follow it.
- `cut_slot_single_use` — second PasteInternal after a cut → NothingToPaste.
- `paste_internal_overflow_rejected` — anchor near sheet edge, no partial write.
- `full_column_copy_clamps_to_used_range` — 1M-row selection copies dimension() only.
- `paste_undo_single_step` — one undo restores pre-paste state (incl. cut source).
- `cross_sheet_internal_paste` — copy on Sheet1, paste on Sheet2.
- `paste_tsv_dims_and_types` — `"1\t2\n=A1\ttrue\n"` → number/formula/bool cells.
- `paste_tsv_crlf_and_ragged_rows` — `\r\n` + short row → padded skip, no panic.
- `paste_tsv_overflow_rejected`.
Unit (freecell-core::tsv): `dims_simple`, `dims_trailing_newline`, `dims_ragged`,
`dims_single_token`, `overflow_predicate`.
UI-level: `paste_prefers_internal_when_text_matches`, `paste_falls_back_to_tsv`,
`copy_writes_system_clipboard`.
