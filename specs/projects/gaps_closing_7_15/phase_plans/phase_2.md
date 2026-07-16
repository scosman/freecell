---
status: complete
---

# Phase 2: CSV import + export

## Overview

Add the ability to (a) **open a `.csv`** file as a new untitled workbook and (b) **export
the active sheet** to a `.csv`. Import parses RFC-4180 comma-delimited data and applies each
field as user input into a fresh single-sheet workbook opened with `path: None` (so Save →
Save-As-to-`.xlsx`, no `.back` backup). Export walks the active sheet's used range, renders
each cell as its **raw stored value** (the `value_token`/paste-values computed value — plain
numbers, date serials, `TRUE`/`FALSE`, error strings, text verbatim — **not** the formatted
display string and **not** the formula source, D2.2), serializes with CRLF line endings, and
writes atomically. Export never changes the document's dirty flag / path / title.

All CSV parse→cells and used-range→CSV logic lives **inside `freecell-engine`** (IronCalc
types are `pub(crate)`); `freecell-app` only wires argv/menus/actions/panels. No fork work,
no pixel suite (IO/chrome only).

## Steps

### freecell-engine

1. **`Cargo.toml`** — add `csv.workspace = true` to `[dependencies]`.

2. **`document.rs`**
   - `use std::fs;` already present via `std::fs::File`; add `use csv;` usage inline.
   - Add variant `DocumentSource::ImportCsv(PathBuf)`.
   - Add `LoadError::BadCsv(String)` — `#[error("This CSV can't be imported: {0}")]`,
     surfaced by the existing "Couldn't open the workbook" dialog path.
   - `from_source`: `DocumentSource::ImportCsv(path) => Self::import_csv(path)`.
   - New `pub fn import_csv(path: &Path) -> Result<Self, LoadError>`:
     - Read bytes (`fs::read` → `LoadError::Io`); strip a leading UTF-8 BOM
       (`EF BB BF`); `String::from_utf8` → invalid → `LoadError::BadCsv(...)` (D2.5).
     - Build a **raw** `Model::new_empty(NEW_WORKBOOK_NAME, DEFAULT_LOCALE,
       DEFAULT_TIMEZONE, DEFAULT_LANGUAGE)` so wrapping it with `UserModel::from_model`
       starts the undo history **empty** (import is not undoable per cross-cutting §Undo).
     - `csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(text.as_bytes())`
       (comma is the default delimiter — same crate/config family as `freecell-core/tsv.rs`).
     - Stream records: overflow-guard `r >= MAX_ROWS` / `c >= MAX_COLS` (0-based index ≥ the
       count means the 1-based coord exceeds the max) → `LoadError::BadCsv("larger than the
       maximum sheet size")`. For each non-empty field apply `model.set_user_input(0, r+1,
       c+1, field)` (numbers/bools/`=formula`/text auto-typed by IronCalc). A record parse
       error → `LoadError::BadCsv`.
     - `model.evaluate()` (fresh import has no cached values, so compute for first paint).
     - Return `Self { model: UserModel::from_model(model) }`.
   - New `pub(crate) fn export_csv(&self, sheet_idx: u32, path: &Path) -> Result<(),
     SaveError>`:
     - `ws = self.worksheet(sheet_idx)` (→ `SaveError::Serialize` on an unresolvable sheet).
     - Empty sheet (`ws.sheet_data.is_empty()`) → create an empty temp file, persist, `Ok`
       (0-byte file — edge case).
     - `dim = ws.dimension()` (1-based inclusive). `decimal_sep =
       self.workbook_decimal_separator()`.
     - `csv::WriterBuilder::new().terminator(Terminator::CRLF).flexible(true)` over a
       `BufWriter<&File>` on `new_temp_beside(path)`. `flexible(true)` is required — trailing-
       empty trimming yields ragged records.
     - For each `row in min_row..=max_row`: build fields `min_col..=max_col` via a new
       `export_cell_value(sheet_idx, row, col, decimal_sep)`; pop trailing empty fields;
       `write_record`. The csv writer handles RFC-4180 quoting (comma/quote/newline).
     - Flush, drop the writer, `persist_atomically(temp, path)`.
   - New `fn export_cell_value(&self, sheet_idx, row, col, decimal_sep) -> String` mirroring
     `value_token` but **without** the text quote-prefix (CSV writes text verbatim; the csv
     writer quotes as needed): `Number → number_token`, `Boolean → TRUE/FALSE`, `String(s) →
     s` (covers both genuine text **and** the error string, both verbatim), `None/Err → ""`.

3. **`worker/protocol.rs`**
   - `Command::ExportCsv { sheet: SheetId, path: PathBuf, req_id: u64 }` (uses `SheetId` for
     consistency with every other command — the worker resolves it; deviates from the spec's
     `usize` note, which is ambiguous index-vs-id).
   - `WorkerEvent::CsvExported { req_id: u64 }` and `WorkerEvent::CsvExportFailed { req_id:
     u64, error: SaveError }`.

4. **`worker/run.rs`** — classify `Command::ExportCsv` into a new `exports` bucket in the
   exhaustive `process_batch` routing; after the `saves` loop, for each export resolve the
   sheet and call `self.doc.export_csv(idx, &path)` → emit `CsvExported` / `CsvExportFailed`.
   Pure read: never touches `committed_ops`/dirty.

### freecell-app

5. **`main.rs`** — widen the CLI file-arg filter (`xlsx_arg` → `open_arg`) to accept `.csv`
   **or** `.xlsx`; update the call site + doc comment.

6. **`shell/mod.rs`** — add `ImportCsv` (app-level) and `ExportCsv` (window-scoped) to the
   `actions!` list with doc comments.

7. **`shell/app.rs`**
   - Register `ImportCsv` → `FreeCellApp::import_via_panel` (opens the files panel with an
     "Import" prompt, routes the pick to `open_path`).
   - `do_open_path`: after `record_recent`, branch — a `.csv` (case-insensitive) path →
     `open_document(DocumentSource::ImportCsv(canonical), None, cx)` (untitled; no dedupe —
     imports carry no path), `return`. Non-csv unchanged.
   - `do_open_path_detached` (test): same `.csv` branch → `open_detached_document(None, cx)`
     so the shell routing test can assert an untitled window.

8. **`shell/window.rs`**
   - Add field `pending_export_req: Option<u64>` (init `None`).
   - Register `.on_action(|_: &ExportCsv|)` → `export_csv(window, cx)` on the render div.
   - `export_csv`: suggested name `<active-sheet-name>.csv` (from `self.sheets` by active
     `SheetId`, fallback `Untitled.csv`); `prompt_for_new_path` (directory = current path's
     parent or cwd); on pick force `.csv` extension and send `Command::ExportCsv { sheet:
     active, path, req_id }`, record `pending_export_req`.
   - `on_worker_event`: add `CsvExported { req_id }` (clear `pending_export_req` if it
     matches; nothing else — dirty/title untouched) and `CsvExportFailed { req_id, error }`
     (if it matches, show the standard save-error dialog, `close_window_on_dismiss: false`).
   - Add `import_panel_options()` (files-only, prompt "Import") + a `with_csv_extension` helper
     (mirrors `lifecycle::with_xlsx_extension`, in lifecycle.rs).

9. **`shell/lifecycle.rs`** — add `CSV_EXT` + `with_csv_extension(PathBuf) -> PathBuf`
   (enforce `.csv` on the export panel result), unit-tested like `with_xlsx_extension`.

10. **`shell/menus.rs`** — import `ImportCsv`, `ExportCsv`; add File-menu items "Import CSV…"
    (after Open Recent) and "Export as CSV…" (after Save As…).

## Tests

### freecell-engine (unit, in `document.rs` tests)

- `import_csv_applies_fields_as_user_input`: numbers, `TRUE`/`FALSE`, a `=formula`, and text
  land as the right kinds/values; a quoted field with an embedded comma + a `""` escape + an
  embedded newline parse as one field; ragged rows leave trailing cells empty; the doc is
  untitled (a follow-on: assert cell values via `formatted_value`/`cell_value`).
- `import_csv_strips_bom`: a leading UTF-8 BOM is stripped (A1 is not `\u{FEFF}...`).
- `import_csv_empty_file_yields_one_sheet`: 0-byte input → a valid empty workbook.
- `import_csv_rejects_oversize`: a record beyond `MAX_ROWS` (or a field beyond `MAX_COLS`) →
  `LoadError::BadCsv`.
- `import_csv_rejects_invalid_utf8`: non-UTF-8 bytes → `LoadError::BadCsv`.
- `export_csv_writes_raw_values`: a fixture workbook (number `0.5`, a date serial, a bool, a
  formula computing a value, text, a cell containing a comma) → byte-compare: `0.5` not
  `50%`, serial not formatted date, `TRUE`/`FALSE`, the formula's computed value, quoting of
  the comma cell, CRLF terminators, trailing-empty trimming.
- `export_csv_empty_sheet_writes_empty_file`: empty sheet → 0-byte file.
- `export_csv_import_csv_round_trips_values`: `import_csv(export_csv(x))` reproduces a
  values-only sheet.

### freecell-engine (integration, `tests/worker_seam.rs`)

- `import_csv_source_loads_values`: `DocumentSource::ImportCsv(path)` → `Loaded` → viewport →
  published cell values match the CSV.
- `export_csv_command_writes_file_and_keeps_clean`: NewWorkbook, set a cell, `Command::ExportCsv`
  → `CsvExported`; the file exists with the expected content and `committed_ops` (dirty) is
  unchanged by the export.

### freecell-app

- `lifecycle`: `with_csv_extension` add/keep/replace (unit).
- gpui: opening a `.csv` path routes to import → an **untitled** (`path: None`) window (via the
  detached branch). Menu: `menus.rs` File menu contains "Import CSV…" and "Export as CSV…".
