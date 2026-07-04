---
status: complete
---

# Phase 3: Document I/O (IronCalc adapter)

## Overview

Track A, first engine phase. Builds the **file-I/O adapter** in `freecell-engine` that the
Phase-4 worker will own: create a new empty workbook, open an `.xlsx`, and save one with an
**atomic temp-file + rename**, all behind typed `LoadError`/`SaveError` enums. Adds a public
`fixtures` module of programmatically-built workbooks (values / formulas / styles / number
formats / multi-sheet / formula-errors) and a full **open→save→reopen round-trip** test
suite plus atomic-failure and corrupt/not-xlsx/password open-failure tests.

This is *only* the I/O adapter and the workbook handle. The command/event worker loop
(Phase 4), the style/geometry cache build (Phase 5), and the `Publication` build are **not**
in scope — the adapter deliberately does **not** evaluate on open (SP2 first-paint uses the
file's cached values, `functional_spec.md §5.1`).

Everything IronCalc stays behind this crate (`architecture.md §2`); the adapter's public API
returns `freecell-core` / `std` types only (no `ironcalc` type escapes the public surface).

### Key API facts (verified against `ironcalc*-0.7.1` source, not assumed)

- **Open:** `ironcalc::import::load_from_xlsx(path, locale, tz, language) -> Result<Model, XlsxError>`
  (4 args — the spec's `(path, locale, tz)` omits `language`; recorded). Returns a `Model<'a>`
  tied to `language`; passing the `'static` literal `"en"` yields `Model<'static>` →
  `UserModel::from_model` → `UserModel<'static>`.
- **New:** `UserModel::new_empty(name, locale, tz, language) -> Result<UserModel, String>`
  (one sheet "Sheet1").
- **Save:** `ironcalc::export::save_xlsx_to_writer(&Model, W: Write+Seek) -> Result<W, XlsxError>`
  via `UserModel::get_model()`. (Not `save_to_xlsx`, which refuses to overwrite an existing
  file — the temp+rename path is exactly what lets a re-save replace the target.)
- **Typed errors:** `XlsxError` is a flat `{IO, Zip, Xml, Workbook, Evaluation, Comparison,
  NotImplemented}`, so it cannot by itself distinguish not-xlsx / corrupt / password. The
  adapter **classifies by magic bytes first** (OLE `D0CF11E0…` → `PasswordProtected`; `PK…`
  → treat as a zip and map any load failure to `Corrupt`; anything else → `NotXlsx`), and
  OS open/read failures → `Io`.
- **Reads (for round-trip compare + Phase-4 reuse):** `get_formatted_cell_value`,
  `get_cell_content`, `get_worksheets_properties`, `get_cell_style`.
- Fixtures write via `set_user_input` (auto-evaluates) and `update_range_style(area, path,
  value)` with paths `font.b|font.i|font.u|font.color|fill.fg_color|num_fmt`; colors are
  `#RRGGBB`. Large fixtures use `pause_evaluation` … `evaluate()` to build once.

## Steps

1. **`app/Cargo.toml`** — add `tempfile = "3"` to `[workspace.dependencies]` (new pin;
   already present in the tree via zed, MIT/Apache).
2. **`app/crates/freecell-engine/Cargo.toml`** — add `ironcalc.workspace = true`,
   `thiserror.workspace = true`, `tempfile.workspace = true` (drop the inline "added later"
   comment for these three). Keep `ironcalc_base`. Add `dev-dependencies: tempfile` is not
   needed (already a normal dep).
3. **`app/crates/freecell-engine/src/document.rs`** (new) — the adapter:
   - Constants: `DEFAULT_LOCALE="en"`, `DEFAULT_TIMEZONE="UTC"`, `DEFAULT_LANGUAGE="en"`,
     `NEW_WORKBOOK_NAME="Untitled"`. (System-tz deferred → DECISIONS: UTC is deterministic;
     only volatile date/time fns differ, out of round-trip scope.)
   - `pub enum DocumentSource { NewWorkbook, OpenFile(PathBuf) }`.
   - `pub enum LoadError { NotXlsx(String), Corrupt(String), PasswordProtected, Io(String) }`
     (`thiserror`; human sentences, underlying message preserved).
   - `pub enum SaveError { Io(String), Serialize(String) }`.
   - `pub struct CellQueryError(String)` (`thiserror`) for the read helpers.
   - `pub struct WorkbookDocument { model: UserModel<'static> }` with:
     - `new_empty() -> Result<Self, LoadError>`
     - `open(path: &Path) -> Result<Self, LoadError>` (magic-classify → load → map errors)
     - `from_source(&DocumentSource) -> Result<Self, LoadError>`
     - `save(&self, path: &Path) -> Result<(), SaveError>` — `NamedTempFile::new_in(dir)` →
       `BufWriter` over the temp file → `save_xlsx_to_writer` → flush → `sync_all` (fsync) →
       `persist(path)` (atomic rename). `dir` = `path.parent()` (or `.`).
     - `sheet_names() -> Vec<String>`, `sheet_count() -> usize`
     - `formatted_value(sheet: u32, cell: CellRef) -> Result<String, CellQueryError>`
     - `cell_content(sheet: u32, cell: CellRef) -> Result<String, CellQueryError>`
     - `pub(crate) fn user_model(&self)/user_model_mut(&mut self)` (Phase-4 worker + fixtures)
     - `pub(crate) fn cell_style(sheet, cell) -> Result<Style, CellQueryError>` (in-crate
       style round-trip test).
   - private `classify_magic(path) -> io::Result<FileKind>` (robust 8-byte fill),
     `destination_dir(path) -> PathBuf`, and 0-based `CellRef` → 1-based `(i32,i32)` helper.
   - Compile-time `assert_send::<WorkbookDocument>()` guard (the worker moves it to its
     thread in Phase 4). If `UserModel` is not `Send`, record it as a Phase-4 constraint.
4. **`app/crates/freecell-engine/src/fixtures.rs`** (new, `pub mod fixtures`) — builders
   returning a populated `WorkbookDocument` for round-trip + downstream (`render-tests`) use:
   `values()`, `formulas()` (incl. `=1/0` → `#DIV/0!`), `styles()`, `number_formats()`,
   `multi_sheet()`, `circular_ref(ring: u32)` (pause/eval-once; `#CIRC!`). Shared helpers
   `set(doc, sheet, row, col, input)` and `style(doc, sheet, row, col, path, value)`.
5. **`app/crates/freecell-engine/src/lib.rs`** — declare `pub mod document; pub mod
   fixtures;`; re-export `DocumentSource, WorkbookDocument, LoadError, SaveError,
   CellQueryError`. Keep the existing `UserModel` re-export + linking test.

## Tests

Integration (`tests/roundtrip.rs`, public API only):
- `roundtrip_values_preserved` — numbers/text/decimal/negative survive save→reopen (formatted
  values equal; no eval-on-open reliance beyond cached values).
- `roundtrip_formulas_preserved` — `=SUM`, `=A1*2` raw content (`cell_content`) **and**
  cached formatted results survive.
- `roundtrip_number_formats_preserved` — currency/percent/date fixtures reopen with the
  engine-formatted strings (`"$1,234.50"`, `"100.00%"`, `"2021-01-01"`) — proves the
  `num_fmt` string round-trips *and* the engine owns display formatting.
- `roundtrip_multi_sheet_and_names` — 3 sheets, order + names preserved; cross-sheet formula
  result preserved.
- `roundtrip_after_rename` — rename a sheet, save, reopen → new name present, formulas intact.
- `formula_errors_are_values` — `#DIV/0!` and `#CIRC!` come back as display text after
  reopen (never a panic/hang).
- `new_empty_has_one_sheet` — `new_empty()`/`from_source(NewWorkbook)` → one "Sheet1".
- `save_overwrites_existing_file` — save V1, edit, save V2 over the same path → reopen shows
  V2 and the file is a single valid workbook.
- `save_failure_missing_directory` — save to a non-existent dir → `Err(SaveError::Io)`, no
  file created (root-proof ENOENT).
- `save_failure_preserves_destination` — save to a path that is an existing **non-empty
  directory** → `Err(SaveError::Io)` (root-proof EISDIR on the rename); the directory's
  sentinel file is byte-identical and no temp file leaks (proves a failed save never touches
  the destination).
- `open_missing_file_is_io_error`, `open_empty_file_is_not_xlsx`,
  `open_text_file_is_not_xlsx`, `open_truncated_zip_is_corrupt`,
  `open_ole_file_is_password_protected` — the typed open-failure matrix, no panics.

In-crate (`#[cfg(test)]` in `document.rs`, needs `pub(crate)` / ironcalc `Style`):
- `roundtrip_styles_preserved` — bold / italic / underline / fill `#FF0000` / font color
  survive (`cell_style` fields equal after reopen).
- `destination_dir_defaults_to_cwd`, `classify_magic_*` unit tests.
- `workbook_document_is_send` — covered by the compile-time guard (documented).
