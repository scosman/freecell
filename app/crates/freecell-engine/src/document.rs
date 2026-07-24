//! `WorkbookDocument` — the IronCalc file-I/O adapter (`components/engine_worker.md §File
//! I/O`, `architecture.md §5`).
//!
//! This is the single place that opens, creates, and saves `.xlsx` workbooks. It owns the
//! `UserModel` (workbook truth); the Phase-4 worker will own one of these on its dedicated
//! thread and drive edits/evals through it. Everything IronCalc stays behind this crate —
//! the public surface here returns only `freecell-core` / `std` types, never an `ironcalc`
//! type.
//!
//! Scope note: this phase is *only* the I/O adapter. It deliberately does **not** evaluate
//! on open — first paint uses the file's cached values (SP2 / `functional_spec.md §5.1`);
//! the command/event loop, caches, and `Publication` build land in Phases 4–5.

use std::fs::File;
use std::io::{self, BufWriter, Cursor, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use freecell_core::format_color::{format_color_rgb, is_date_format};
use freecell_core::{
    CellKind, CellRange, CellRef, Direction, FillAxis, Rgb, SelectionStats, SheetDims,
};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::cell::CellValue;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::formatter::format::format_number;
use ironcalc_base::locale::get_locale;
use ironcalc_base::types::{CellType, Font, Style, Worksheet};
use ironcalc_base::Model;

use crate::chart::binding::CellData; // engine-free resolved value the live-binding read produces
use crate::UserModel; // the crate's single canonical path to the IronCalc workbook type
use ironcalc_base::BorderArea;
use ironcalc_base::ClipboardData;
use serde::Deserialize;
use tempfile::NamedTempFile;
use thiserror::Error;

/// The conditional-formatting `WorkbookDocument` methods (add/update/delete/reorder/list/
/// `has_cond_fmt`/`extended_render_style`). A child module of `document` so it can reach the
/// private `model` field while keeping the CF surface in its own file
/// (`components/engine_cf.md §4`).
mod cond_fmt;

/// Locale used for a new/opened workbook (`en` — `components/engine_worker.md §File I/O`).
pub const DEFAULT_LOCALE: &str = "en";
/// Timezone used for a new/opened workbook. The component doc says "system tz"; the adapter
/// uses `UTC` — a deterministic default (only volatile date/time functions like `NOW()`
/// depend on it, and those are out of the round-trip test scope). System-tz detection would
/// need an extra crate (`iana-time-zone`) and is deferred (DECISIONS_TO_REVIEW).
pub const DEFAULT_TIMEZONE: &str = "UTC";
/// Language pack for formula parsing / function names (`en`). A `'static` literal so the
/// resulting `Model<'static>` / `UserModel<'static>` outlives the call.
pub const DEFAULT_LANGUAGE: &str = "en";
/// Workbook name for a freshly created document (`functional_spec.md §2.3` — the window
/// titles this `Untitled` until first save).
pub const NEW_WORKBOOK_NAME: &str = "Untitled";

/// Where a document's initial state comes from (`components/engine_worker.md` —
/// `DocumentClient::spawn` consumes this in Phase 4).
#[derive(Debug, Clone)]
pub enum DocumentSource {
    /// A new empty workbook (one sheet, "Sheet1").
    NewWorkbook,
    /// An existing `.xlsx` file on disk.
    OpenFile(PathBuf),
    /// A `.csv` file on disk, imported into a fresh untitled single-sheet workbook
    /// (`functional_spec.md §2`, D2.1): parsed comma-delimited, each field applied as user
    /// input, opened with `path: None` so Save → Save-As-to-`.xlsx` and no `.back` backup.
    ImportCsv(PathBuf),
    /// The bundled **demo** workbook, materialized to a temp `.xlsx` at `path` (the app writes the
    /// embedded demo bytes there — the IronCalc loader is path-based). Loads exactly like
    /// [`OpenFile`](Self::OpenFile) — a real `.xlsx` with charts we want to render + preserve, so
    /// the worker treats its chart discovery like an open (not like a fresh/CSV workbook). It is
    /// **the app** that opens the window untitled (`path: None`) so Save → Save-As-to-`.xlsx`, no
    /// `.back` backup, and no dedupe — each demo open is a fresh untitled copy of the same static
    /// file.
    OpenDemo(PathBuf),
}

/// A typed open failure. Each variant maps to a human-readable dialog sentence; the
/// underlying engine/OS message is preserved for the details line (`architecture.md §5`,
/// `functional_spec.md §5.1`). `Clone` so it can ride the worker's `WorkerEvent::LoadFailed`.
#[derive(Debug, Clone, Error)]
pub enum LoadError {
    /// The file isn't an `.xlsx` workbook at all (its bytes aren't a Zip/OOXML container).
    #[error("This file isn't an .xlsx workbook: {0}")]
    NotXlsx(String),
    /// The file looks like an `.xlsx` (Zip container) but is damaged or missing parts.
    #[error("This workbook appears to be damaged and can't be opened: {0}")]
    Corrupt(String),
    /// The file is an OLE2/CFB container — a legacy binary `.xls` **or** an
    /// encrypted/password-protected `.xlsx`. FreeCell can't tell the two apart cheaply (both
    /// share the same magic; distinguishing them needs a CFB directory parse), so the
    /// message names both possibilities accurately (`functional_spec.md §5.1`). The
    /// spec-named `PasswordProtected` variant is kept.
    #[error(
        "FreeCell can't open this file — it looks like a legacy Excel workbook (.xls) or a \
         password-protected/encrypted .xlsx. Re-save it as a modern .xlsx and try again."
    )]
    PasswordProtected,
    /// The file couldn't be read from disk (missing, no permission, …).
    #[error("The file couldn't be read: {0}")]
    Io(String),
    /// A `.csv` import failed: not valid UTF-8, larger than the maximum sheet size, or a
    /// malformed record (`functional_spec.md §2`, D2.5). The message is the dialog detail.
    #[error("This CSV can't be imported: {0}")]
    BadCsv(String),
}

/// A typed save failure. Because saves are atomic (temp file + rename), any failure leaves
/// the original destination file untouched (`functional_spec.md §5.2`). `Clone` so it can ride
/// the worker's `WorkerEvent::SaveFailed`.
#[derive(Debug, Clone, Error)]
pub enum SaveError {
    /// The workbook couldn't be written to / renamed onto disk.
    #[error("The workbook couldn't be saved: {0}")]
    Io(String),
    /// IronCalc's xlsx writer failed to serialize the model.
    #[error("The workbook couldn't be written: {0}")]
    Serialize(String),
}

/// A cell read (formatted value / raw content / style) hit an invalid sheet or coordinate.
#[derive(Debug, Error)]
#[error("cell query failed: {0}")]
pub struct CellQueryError(String);

/// A character-format boolean the worker toggles (the engine style paths `font.b` / `font.i`
/// / `font.u` / `font.strike`). In-crate: the toggle *policy* (any-lacking → set-all) lives in the
/// worker; this is only the read/write *mechanism* over the pinned IronCalc range-style API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontFlag {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl FontFlag {
    /// The IronCalc `update_range_style` / `get_cell_style` path for this flag.
    fn style_path(self) -> &'static str {
        match self {
            FontFlag::Bold => "font.b",
            FontFlag::Italic => "font.i",
            FontFlag::Underline => "font.u",
            FontFlag::Strike => "font.strike",
        }
    }
}

/// The result of copying a range to the engine clipboard (`components/clipboard.md §Copy /
/// Cut`). Engine-free so it can cross back to the worker: `tsv` goes on the system clipboard,
/// `data` is stashed (serialized — the concrete `ClipboardCell` type is private to
/// ironcalc_base) for a later internal paste, and `range` is the engine's **effective**
/// (dimension-clamped) source rectangle in 1-based inclusive `(row0, col0, row1, col1)` coords.
#[derive(Debug, Clone)]
pub(crate) struct CopiedRange {
    pub tsv: String,
    pub data: serde_json::Value,
    pub range: (i32, i32, i32, i32),
    /// The copied block's **computed values** rendered as literal paste tokens, row-major over the
    /// effective (clamped) source rectangle — the source of a later Paste Values (⌘⇧V,
    /// `functional_spec.md §5`). Captured at copy time (a snapshot, like `data`/`tsv`) so
    /// paste-values is values-only and never re-derives formulas. See
    /// [`WorkbookDocument::value_token`] for the per-cell rendering.
    pub values: Vec<Vec<String>>,
}

/// The magic-byte family of a file, used to classify open failures into typed [`LoadError`]s
/// before the engine collapses them into generic Zip/XML errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// A Zip container (`PK…`) — every `.xlsx` starts this way. Load failures → `Corrupt`.
    Zip,
    /// An OLE2 / Compound File Binary container (`D0CF11E0…`) — encrypted OOXML and legacy
    /// binary `.xls` both use it; treated as password-protected/unsupported.
    Ole,
    /// Anything else (plain text, an empty file, another binary) — not an `.xlsx`.
    Other,
}

/// The owned IronCalc workbook plus its file-I/O operations. Cheap to move; the Phase-4
/// worker owns exactly one on its thread.
#[derive(Debug)]
pub struct WorkbookDocument {
    model: UserModel<'static>,
}

// The Phase-4 worker constructs the document on one thread and moves it into the eval-worker
// thread (`architecture.md §2`), so it MUST be `Send`. Assert it here so a future field that
// breaks the bound fails at this line rather than at the worker spawn site.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<WorkbookDocument>();
};

impl WorkbookDocument {
    /// Creates a new empty workbook (one sheet, "Sheet1"), the state the app opens on when
    /// creating a new document.
    pub fn new_empty() -> Result<Self, LoadError> {
        let model = UserModel::new_empty(
            NEW_WORKBOOK_NAME,
            DEFAULT_LOCALE,
            DEFAULT_TIMEZONE,
            DEFAULT_LANGUAGE,
        )
        .map_err(LoadError::Corrupt)?;
        Ok(Self { model })
    }

    /// Opens an `.xlsx` file into a workbook. Does **not** evaluate — first paint uses the
    /// file's cached values (SP2). Failures are typed by inspecting the file's magic bytes
    /// (which distinguishes not-xlsx / password from a damaged zip that the engine would
    /// otherwise report as a generic Zip/XML error).
    pub fn open(path: &Path) -> Result<Self, LoadError> {
        match classify_magic(path) {
            Err(e) => return Err(LoadError::Io(e.to_string())),
            Ok(FileKind::Ole) => return Err(LoadError::PasswordProtected),
            Ok(FileKind::Other) => {
                return Err(LoadError::NotXlsx(
                    "the file's contents are not a Zip/OOXML workbook".to_string(),
                ))
            }
            Ok(FileKind::Zip) => {}
        }

        let path_str = path
            .to_str()
            .ok_or_else(|| LoadError::Io(format!("path is not valid UTF-8: {}", path.display())))?;

        // Theme/indexed-colour/number-format import fidelity and the optional-`xfId` accept are
        // now handled by the IronCalc build we pin (the fork's `freecell-fixes`), so the former
        // `open_fixups`/`open_repair` post/pre-parse workarounds are gone — the engine loads and
        // resolves these itself. See specs/projects/ironcalc-upstreaming.
        match load_from_xlsx(path_str, DEFAULT_LOCALE, DEFAULT_TIMEZONE, DEFAULT_LANGUAGE) {
            Ok(model) => Ok(Self {
                model: UserModel::from_model(model),
            }),
            // A real read error after the magic check (e.g. the file vanished mid-open).
            Err(ironcalc::error::XlsxError::IO(msg)) => Err(LoadError::Io(msg)),
            // It IS a Zip, so any structural/parse/workbook/feature failure means the workbook
            // itself is damaged or unsupported. The message is preserved for the dialog details
            // line (a `NotImplemented` message names the unsupported feature).
            Err(other) => Err(LoadError::Corrupt(other.to_string())),
        }
    }

    /// Wraps a pre-built IronCalc `Model` (test-only): lets a test author a workbook whose
    /// **default** style differs from `new_empty`'s (e.g. an opened file with a non-Calibri default
    /// font) by mutating the public `workbook.styles` before wrapping.
    #[cfg(test)]
    pub(crate) fn from_test_model(model: Model<'static>) -> Self {
        Self {
            model: UserModel::from_model(model),
        }
    }

    /// Builds a document from a [`DocumentSource`] (Phase-4 `spawn` entry point).
    pub fn from_source(source: &DocumentSource) -> Result<Self, LoadError> {
        match source {
            DocumentSource::NewWorkbook => Self::new_empty(),
            DocumentSource::OpenFile(path) => Self::open(path),
            DocumentSource::ImportCsv(path) => Self::import_csv(path),
            // The demo is a real `.xlsx` (materialized from bundled bytes) — load it exactly like
            // an open so its cells, styles, and charts come through. The untitled/save-as behavior
            // is applied by the app opening the window with `path: None`, not here.
            DocumentSource::OpenDemo(path) => Self::open(path),
        }
    }

    /// Imports a `.csv` file into a fresh untitled single-sheet workbook (`functional_spec.md
    /// §2`, D2.1/D2.4/D2.5). Comma-delimited RFC-4180 parse (quoted fields may contain commas,
    /// embedded newlines, and doubled `""`); each **non-empty** field is applied as user input at
    /// its `A1`-origin cell — so a number becomes a number, `TRUE`/`FALSE` a boolean, a leading
    /// `=` a formula, everything else text (the same "apply as user input" typing as TSV paste).
    /// Empty fields are skipped (the fresh sheet is already blank). Ragged rows are accepted.
    ///
    /// The document is built on a **raw** [`Model`] and wrapped with [`UserModel::from_model`], so
    /// its undo history starts empty (the import is not an undoable edit — cross-cutting §Undo).
    ///
    /// Errors ([`LoadError::BadCsv`]): non-UTF-8 bytes (BOM tolerated), a CSV exceeding the
    /// Excel-max grid (1,048,576 rows / 16,384 cols — rejected, never truncated), or a malformed
    /// record.
    pub fn import_csv(path: &Path) -> Result<Self, LoadError> {
        crate::instrument::record_engine_call();
        let mut bytes = std::fs::read(path).map_err(|e| LoadError::Io(e.to_string()))?;
        // Tolerate a leading UTF-8 BOM (D2.5): strip it before decoding so A1 isn't polluted.
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bytes.drain(0..3);
        }
        // Validate UTF-8 up front so invalid bytes surface a readable dialog rather than mojibake
        // (D2.5) — the csv reader below then yields guaranteed-valid `StringRecord`s.
        let text = String::from_utf8(bytes).map_err(|_| {
            LoadError::BadCsv(
                "the file isn't valid UTF-8 text and can't be read as CSV".to_string(),
            )
        })?;

        let mut model = Model::new_empty(
            NEW_WORKBOOK_NAME,
            DEFAULT_LOCALE,
            DEFAULT_TIMEZONE,
            DEFAULT_LANGUAGE,
        )
        .map_err(LoadError::Corrupt)?;

        let max_rows = freecell_core::limits::MAX_ROWS as usize;
        let max_cols = freecell_core::limits::MAX_COLS as usize;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        for (r, record) in reader.records().enumerate() {
            // A 0-based record index that reaches the row count means its 1-based row exceeds the
            // Excel-max — reject (never truncate). Checked as we stream (no full materialization).
            if r >= max_rows {
                return Err(LoadError::BadCsv(
                    "this CSV is larger than the maximum sheet size".to_string(),
                ));
            }
            let record = record.map_err(|e| LoadError::BadCsv(e.to_string()))?;
            for (c, field) in record.iter().enumerate() {
                if c >= max_cols {
                    return Err(LoadError::BadCsv(
                        "this CSV is larger than the maximum sheet size".to_string(),
                    ));
                }
                if !field.is_empty() {
                    model
                        .set_user_input(0, (r + 1) as i32, (c + 1) as i32, field.to_string())
                        .map_err(LoadError::Corrupt)?;
                }
            }
        }
        // A fresh import carries no cached values (unlike an opened `.xlsx`), so evaluate once so
        // formula cells paint their computed value on first frame.
        model.evaluate();
        Ok(Self {
            model: UserModel::from_model(model),
        })
    }

    /// Saves the workbook to `path` **atomically**: serialize into a temp file in the
    /// destination directory, fsync it, then rename it over the target. On any failure the
    /// original file at `path` is left untouched (`functional_spec.md §5.2`,
    /// `components/engine_worker.md §File I/O`).
    pub fn save(&self, path: &Path) -> Result<(), SaveError> {
        crate::instrument::record_engine_call();
        let temp = new_temp_beside(path)?;

        // Serialize the model into the temp file (buffered — the zip writer does many small
        // writes). `save_xlsx_to_writer` takes the writer by value and hands it back.
        let writer = BufWriter::new(temp.as_file());
        let writer =
            save_xlsx_to_writer(self.model.get_model(), writer).map_err(map_writer_error)?;
        writer
            .into_inner()
            .map_err(|e| SaveError::Io(e.to_string()))?;

        persist_atomically(temp, path)
    }

    /// Serializes the current model to an in-memory `.xlsx` zip (IronCalc's chart-less writer
    /// output) — the reinject base the chart-preserving save path (`worker::run`) splices the
    /// original file's charts back into (charts/architecture §4.1, §5). Unlike [`save`](Self::save)
    /// this does no disk I/O; the worker writes the reinjected bytes atomically.
    pub(crate) fn to_xlsx_bytes(&self) -> Result<Vec<u8>, SaveError> {
        crate::instrument::record_engine_call();
        let cursor = save_xlsx_to_writer(self.model.get_model(), Cursor::new(Vec::new()))
            .map_err(map_writer_error)?;
        Ok(cursor.into_inner())
    }

    /// The workbook's sheet names, in workbook order.
    pub fn sheet_names(&self) -> Vec<String> {
        crate::instrument::record_engine_call();
        self.model
            .get_worksheets_properties()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    /// The number of sheets in the workbook.
    pub fn sheet_count(&self) -> usize {
        crate::instrument::record_engine_call();
        self.model.get_worksheets_properties().len()
    }

    /// The engine-formatted display text of a cell (number formats / dates / currency /
    /// error values already rendered to a string; empty cells return `""`). This is the
    /// exact per-cell call the Phase-4 `Publication` build uses — display formatting is
    /// engine-owned (round-3 B; FreeCell adds none).
    pub fn formatted_value(&self, sheet: u32, cell: CellRef) -> Result<String, CellQueryError> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        self.model
            .get_formatted_cell_value(sheet, row, col)
            .map_err(CellQueryError)
    }

    /// A published cell's evaluated [`CellKind`] and fully-resolved text colour
    /// (`architecture.md §1.2`) — the two presentation attributes the worker adds to each
    /// [`PublishedCell`](freecell_core::PublishedCell).
    ///
    /// - **kind**: the engine cell type (`get_cell_type`) mapped to `CellKind`, with a
    ///   `Number` reclassified to `Date` when its number format is date/time-like (the
    ///   engine stores dates as serial numbers, so it has no distinct date type).
    /// - **text colour** (precedence per `§1.2`): the cell's explicit non-black font colour
    ///   if set; else the number format's produced colour (e.g. `[Red]` negatives) when the
    ///   format specifies one and the value is numeric; else `None` (the grid's default).
    pub fn published_style(
        &self,
        sheet: u32,
        cell: CellRef,
    ) -> Result<(CellKind, Option<Rgb>), CellQueryError> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        let cell_type = self
            .model
            .get_cell_type(sheet, row, col)
            .map_err(CellQueryError)?;
        let model = self.model.get_model();
        let style = model
            .get_style_for_cell(sheet, row, col)
            .map_err(CellQueryError)?;

        let kind = match cell_type {
            CellType::Number if is_date_format(&style.num_fmt) => CellKind::Date,
            CellType::Number => CellKind::Number,
            CellType::LogicalValue => CellKind::Bool,
            CellType::ErrorValue => CellKind::Error,
            // Text, and the rare Array / CompoundData results, default to text alignment.
            _ => CellKind::Text,
        };

        let text_color = resolve_text_color(model, sheet, row, col, &style);
        Ok((kind, text_color))
    }

    /// The raw content of a cell: the `=formula` text for formula cells, the literal for
    /// value cells, `""` for empty cells (what the formula bar shows/edits).
    pub fn cell_content(&self, sheet: u32, cell: CellRef) -> Result<String, CellQueryError> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        self.model
            .get_cell_content(sheet, row, col)
            .map_err(CellQueryError)
    }

    /// The **evaluated** value of a cell as an engine-free [`CellData`] — the read chart live
    /// binding resolves its `c:f` source ranges through (charts/architecture §4.1). Unlike
    /// [`formatted_value`](Self::formatted_value) it yields the raw `f64` (not a display string), so
    /// a numeric series binds without a lossy string round-trip. A read error / out-of-range cell is
    /// [`CellData::Empty`]. No IronCalc type escapes: the inner [`CellValue`] is mapped here.
    pub(crate) fn cell_value(&self, sheet: u32, cell: CellRef) -> CellData {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        match self
            .model
            .get_model()
            .get_cell_value_by_index(sheet, row, col)
        {
            Ok(CellValue::Number(n)) => CellData::Number(n),
            Ok(CellValue::String(s)) => CellData::Text(s),
            Ok(CellValue::Boolean(b)) => CellData::Bool(b),
            Ok(CellValue::None) | Err(_) => CellData::Empty,
        }
    }

    /// The raw IronCalc worksheet at `sheet_idx` — the enumeration source the Phase-5 cache
    /// builder scans (`sheet_data` for populated/styled cells, `rows`/`cols` for band styles +
    /// custom sizes). `pub(crate)`: the `Worksheet` is an IronCalc type and must not leave the
    /// crate (the cache module lives in this crate and does the conversion to engine-free forms).
    pub(crate) fn worksheet(&self, sheet_idx: u32) -> Result<&Worksheet, String> {
        crate::instrument::record_engine_call();
        self.model.get_model().workbook.worksheet(sheet_idx)
    }

    /// The cell's **own** style (the style stored on the cell itself), or `None` when the cell is
    /// absent from the sheet data. Mirrors IronCalc's `get_cell_style_index` rule: a cell present
    /// in the sheet data resolves to its own style — even the default — shadowing any band, while
    /// an absent cell falls through to the row/column band. The Phase-5 mirror path reads this to
    /// keep the cache's `render_style` in agreement with `get_style_for_cell`.
    pub(crate) fn cell_own_style(
        &self,
        sheet_idx: u32,
        cell: CellRef,
    ) -> Result<Option<Style>, String> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        self.model
            .get_model()
            .get_cell_style_or_none(sheet_idx, row, col)
    }

    /// The cell's fully-resolved style (cell > row-band > col-band > default) — the engine's
    /// authoritative `get_style_for_cell`, used by the agreement contract as the "fresh re-read"
    /// the cache must match. Test-only: the production mirror path reads the cell's *own* style
    /// ([`cell_own_style`](Self::cell_own_style)), never the resolved one.
    #[cfg(test)]
    pub(crate) fn resolved_cell_style(
        &self,
        sheet_idx: u32,
        cell: CellRef,
    ) -> Result<Style, String> {
        let (row, col) = to_engine_coords(cell);
        self.model
            .get_model()
            .get_style_for_cell(sheet_idx, row, col)
    }

    /// The row band style at `row` (0-based), if the row carries one.
    pub(crate) fn row_band_style(&self, sheet_idx: u32, row: u32) -> Result<Option<Style>, String> {
        crate::instrument::record_engine_call();
        self.model
            .get_model()
            .get_row_style(sheet_idx, row as i32 + 1)
    }

    /// The column band style at `col` (0-based), if the column carries one.
    pub(crate) fn col_band_style(&self, sheet_idx: u32, col: u32) -> Result<Option<Style>, String> {
        crate::instrument::record_engine_call();
        self.model
            .get_model()
            .get_column_style(sheet_idx, col as i32 + 1)
    }

    /// The row height at `row` (0-based) in **IronCalc pixels** (the cache converts to FreeCell
    /// device px). IronCalc's getter already returns px (`ironcalc_base/src/constants.rs`).
    pub(crate) fn row_height_px(&self, sheet_idx: u32, row: u32) -> Result<f64, String> {
        crate::instrument::record_engine_call();
        self.model
            .get_model()
            .get_row_height(sheet_idx, row as i32 + 1)
    }

    /// The column width at `col` (0-based) in **IronCalc pixels** (see [`row_height_px`]).
    ///
    /// [`row_height_px`]: Self::row_height_px
    pub(crate) fn col_width_px(&self, sheet_idx: u32, col: u32) -> Result<f64, String> {
        crate::instrument::record_engine_call();
        self.model
            .get_model()
            .get_column_width(sheet_idx, col as i32 + 1)
    }

    /// Each sheet's `(stable sheet_id, name)` in workbook order — the source the worker uses
    /// to build `SheetMeta` and to map a stable [`SheetId`](freecell_core::SheetId) onto the
    /// volatile worksheet index IronCalc's per-cell/sheet APIs take
    /// (`architecture.md §3` index↔id map).
    pub(crate) fn sheet_properties(&self) -> Vec<(u32, String)> {
        crate::instrument::record_engine_call();
        self.model
            .get_worksheets_properties()
            .into_iter()
            .map(|p| (p.sheet_id, p.name))
            .collect()
    }

    /// Each sheet's `(stable sheet_id, name, has_content)` in workbook order. `has_content` is
    /// `true` iff the worksheet has any populated cell (`sheet_data` non-empty) — the
    /// delete-confirm gate (`functional_spec.md §3.7`). Property position `i` is worksheet
    /// index `i`, so the content probe reads `worksheet(i).sheet_data`.
    pub(crate) fn sheet_properties_with_content(&self) -> Vec<(u32, String, bool)> {
        crate::instrument::record_engine_call();
        self.model
            .get_worksheets_properties()
            .into_iter()
            .enumerate()
            .map(|(idx, p)| {
                let has_content = self
                    .worksheet(idx as u32)
                    .map(|ws| !ws.sheet_data.is_empty())
                    .unwrap_or(false);
                (p.sheet_id, p.name, has_content)
            })
            .collect()
    }

    /// Pauses IronCalc's auto-evaluate so a coalesced batch of edits can be applied and then
    /// evaluated **once** (round-3 A: the `pause`/`resume` batch API is the seam's natural
    /// coalescing fit). Pair with [`resume_evaluation`](Self::resume_evaluation) +
    /// [`evaluate`](Self::evaluate).
    pub(crate) fn pause_evaluation(&mut self) {
        crate::instrument::record_engine_call();
        self.model.pause_evaluation();
    }

    /// Resumes IronCalc's auto-evaluate (mechanically the pause flag; the worker still calls
    /// [`evaluate`](Self::evaluate) explicitly to run the single coalesced recompute).
    pub(crate) fn resume_evaluation(&mut self) {
        crate::instrument::record_engine_call();
        self.model.resume_evaluation();
    }

    /// Runs one full-workbook `evaluate()` (the coalesced recompute after a drained batch).
    pub(crate) fn evaluate(&mut self) {
        crate::instrument::record_engine_call();
        self.model.evaluate();
    }

    /// Sets a cell's raw input (`SetCellInput`). Maps to `set_user_input`; the worker
    /// re-checks the input cap *before* calling this (the security boundary — the abort-class
    /// input must never reach the recursive parser). Auto-evaluates unless paused.
    pub(crate) fn set_cell_input(
        &mut self,
        sheet_idx: u32,
        cell: CellRef,
        input: &str,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        self.model.set_user_input(sheet_idx, row, col, input)
    }

    /// Clears a range's **contents only** (keeps styles) — `ClearCells`. One undoable engine
    /// op over the rectangle. Auto-evaluates unless paused.
    ///
    /// `range_clear_contents` has **no band fast path** — a full-column Area would iterate
    /// 1,048,576 cells (`architecture.md §5.2` clamping rule). So a full-row/col/select-all range
    /// (a header-selection Delete) is clamped to the used rectangle first; a bounded selection is
    /// unchanged. An empty intersection (nothing used) is a no-op.
    pub(crate) fn clear_contents(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let clamped = match self.clamp_to_used(sheet_idx, range)? {
            Some(r) => r,
            None => return Ok(()),
        };
        self.model
            .range_clear_contents(&area_of(sheet_idx, clamped))
    }

    /// Fill Down (⌘D) — copy the selection's **top row** down over the rest of the selection
    /// (`functional_spec.md §3`). A **copy**, not a series: seeding `auto_fill_rows` from a single
    /// row (`height == 1`) leaves the fork's `detect_progression` nothing to extrapolate (it needs
    /// ≥2 seed values), so it falls through to `extend_to` — a value/format copy with relative
    /// formula adjustment. One `auto_fill_rows` call ⇒ one undo step. Returns whether anything was
    /// written (`false` = a no-op, so the caller skips recompute/republish/undo).
    ///
    /// A lone single-cell selection **pulls from the cell above** (Excel behavior, D3.1); at row 0
    /// (no neighbor) it is a no-op. A single row with >1 column has nothing below to fill → no-op.
    ///
    /// **Full-column / select-all policy:** a header ⌘D (selection spanning all 1,048,576 rows) is
    /// **clamped to the used rectangle** first (via [`clamp_to_used`](Self::clamp_to_used), exactly
    /// as `clear_contents` does) — the practical Excel behavior is "fill down alongside the existing
    /// data," so it fills only to the used-range extent, never an unbounded ~1M-cell write with a
    /// 1M-entry undo diff-list. Bounded/explicit-range selections are passed through unchanged.
    pub(crate) fn fill_down(&mut self, sheet_idx: u32, range: CellRange) -> Result<bool, String> {
        crate::instrument::record_engine_call();
        // Clamp full-line targets to the used range (no-op for bounded selections); an empty
        // intersection (full-line fill on unused rows/cols) writes nothing.
        let range = match self.clamp_to_used(sheet_idx, range)? {
            Some(r) => r,
            None => return Ok(false),
        };
        let (top, bottom) = (range.start.row, range.end.row);
        let (left, right) = (range.start.col, range.end.col);

        // Seed row (0-based) and the last row to fill down to.
        let (seed_row, to_row) = if bottom > top {
            // Multi-row selection: seed = its top row, fill down to its bottom row.
            (top, bottom)
        } else if left == right {
            // Single cell: pull from the cell directly above; no neighbor at row 0 → no-op.
            if top == 0 {
                return Ok(false);
            }
            (top - 1, top)
        } else {
            // A single row wider than one cell has nothing below the seed line → no-op.
            return Ok(false);
        };

        let source = Area {
            sheet: sheet_idx,
            row: seed_row as i32 + 1,
            column: left as i32 + 1,
            width: (right - left) as i32 + 1,
            height: 1,
        };
        self.model.auto_fill_rows(&source, to_row as i32 + 1)?;
        Ok(true)
    }

    /// Fill Right (⌘R) — copy the selection's **left column** right over the rest of the selection
    /// (the column analog of [`fill_down`](Self::fill_down); same copy-not-series + one-undo-step +
    /// full-line-clamp properties, mirrored on columns). A lone single-cell selection **pulls from
    /// the cell to the left**; at column 0 it is a no-op. A single column taller than one cell →
    /// no-op. Returns whether anything was written (`false` = a no-op).
    pub(crate) fn fill_right(&mut self, sheet_idx: u32, range: CellRange) -> Result<bool, String> {
        crate::instrument::record_engine_call();
        // Clamp full-line (full-row / select-all) targets to the used range; see `fill_down`.
        let range = match self.clamp_to_used(sheet_idx, range)? {
            Some(r) => r,
            None => return Ok(false),
        };
        let (top, bottom) = (range.start.row, range.end.row);
        let (left, right) = (range.start.col, range.end.col);

        // Seed column (0-based) and the last column to fill right to.
        let (seed_col, to_col) = if right > left {
            // Multi-column selection: seed = its left column, fill right to its right column.
            (left, right)
        } else if top == bottom {
            // Single cell: pull from the cell directly to the left; no neighbor at col 0 → no-op.
            if left == 0 {
                return Ok(false);
            }
            (left - 1, left)
        } else {
            // A single column taller than one cell has nothing right of the seed line → no-op.
            return Ok(false);
        };

        let source = Area {
            sheet: sheet_idx,
            row: top as i32 + 1,
            column: seed_col as i32 + 1,
            width: 1,
            height: (bottom - top) as i32 + 1,
        };
        self.model.auto_fill_columns(&source, to_col as i32 + 1)?;
        Ok(true)
    }

    /// Drag-fill (`gaps_closing_7_15 §3`) — the general fill-handle path. Unlike ⌘D/⌘R
    /// ([`fill_down`](Self::fill_down)/[`fill_right`](Self::fill_right)), the source `Area` is the
    /// **full `seed` block** (its real width×height, NOT clamped to a 1-tall/1-wide line), so a
    /// multi-cell seed gives the fork's `detect_progression` a ≥2-value series to extrapolate
    /// (`1,2 → 3,4,5`; `Jan,Feb → Mar…`). A single-cell seed (`width == height == 1`) has no
    /// progression → the fork falls through to `extend_to`, i.e. a value/format **copy** with
    /// relative-formula adjustment (same as ⌘D/⌘R).
    ///
    /// `target` is the previewed fill region (⊇ `seed`); `axis` is the dominant drag axis. The far
    /// edge of `target` along `axis` is passed to `auto_fill_rows`/`auto_fill_columns` as the `to`
    /// bound — the fork natively supports **both** directions: a `to` past the seed fills
    /// down/right, a `to` before the seed fills up/left (it reverses the seed values and re-runs
    /// `detect_progression`, so an up/left series counts the sequence down — no reversal needed
    /// here). One `auto_fill_*` call ⇒ one undo step. Returns whether anything was written
    /// (`false` = `target` doesn't extend past `seed` → a no-op the caller skips).
    pub(crate) fn fill_drag(
        &mut self,
        sheet_idx: u32,
        seed: CellRange,
        target: CellRange,
        axis: FillAxis,
    ) -> Result<bool, String> {
        crate::instrument::record_engine_call();
        let source = Area {
            sheet: sheet_idx,
            row: seed.start.row as i32 + 1,
            column: seed.start.col as i32 + 1,
            width: seed.width() as i32,
            height: seed.height() as i32,
        };
        match axis {
            FillAxis::Vertical => {
                // Far row edge of the target along the fill direction: below the seed (down) or
                // above it (up). No extension past the seed → nothing to fill.
                let to_row = if target.end.row > seed.end.row {
                    target.end.row
                } else if target.start.row < seed.start.row {
                    target.start.row
                } else {
                    return Ok(false);
                };
                self.model.auto_fill_rows(&source, to_row as i32 + 1)?;
            }
            FillAxis::Horizontal => {
                let to_col = if target.end.col > seed.end.col {
                    target.end.col
                } else if target.start.col < seed.start.col {
                    target.start.col
                } else {
                    return Ok(false);
                };
                self.model.auto_fill_columns(&source, to_col as i32 + 1)?;
            }
        }
        Ok(true)
    }

    /// Sets the width (device px) of the inclusive column run `[col_start, col_end]` (0-based) —
    /// `SetColumnWidths`. One undoable diff-list (`set_columns_width`, `common.rs:1055`). Device px
    /// are converted to IronCalc px at this boundary (the grid speaks device px). Called only over
    /// a bounded run (a resize target / selected header run), never an unbounded range.
    pub(crate) fn set_column_widths(
        &mut self,
        sheet_idx: u32,
        col_start: u32,
        col_end: u32,
        device_px: f64,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let px = crate::cache::col_ironcalc_px(device_px);
        self.model
            .set_columns_width(sheet_idx, col_start as i32 + 1, col_end as i32 + 1, px)
    }

    /// Sets the height (device px) of the inclusive row run `[row_start, row_end]` (0-based) —
    /// `SetRowHeights`. One undoable diff-list (`set_rows_height`, `common.rs:1081`). Device px are
    /// converted to IronCalc px here (cf. [`set_column_widths`](Self::set_column_widths)).
    pub(crate) fn set_row_heights_px(
        &mut self,
        sheet_idx: u32,
        row_start: u32,
        row_end: u32,
        device_px: f64,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let px = crate::cache::row_ironcalc_px(device_px);
        self.model
            .set_rows_height(sheet_idx, row_start as i32 + 1, row_end as i32 + 1, px)
    }

    /// Sets (or clears) the **hidden** flag on the inclusive 0-based row run `[row_start, row_end]`
    /// (`gaps_closing_7_15 §4` Hide/Unhide). One undoable diff-list (the fork's `set_rows_hidden`,
    /// `common.rs:1408`); a hidden row keeps its height (unhide restores it). The fork also moves its
    /// internal view selection to the next visible track when hiding — harmless here, since
    /// FreeCell's grid owns its own selection.
    pub(crate) fn set_rows_hidden(
        &mut self,
        sheet_idx: u32,
        row_start: u32,
        row_end: u32,
        hidden: bool,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .set_rows_hidden(sheet_idx, row_start as i32 + 1, row_end as i32 + 1, hidden)
    }

    /// Sets (or clears) the **hidden** flag on the inclusive 0-based column run `[col_start, col_end]`
    /// (the column analog of [`set_rows_hidden`](Self::set_rows_hidden); the fork's
    /// `set_columns_hidden`, `common.rs:1340`).
    pub(crate) fn set_columns_hidden(
        &mut self,
        sheet_idx: u32,
        col_start: u32,
        col_end: u32,
        hidden: bool,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .set_columns_hidden(sheet_idx, col_start as i32 + 1, col_end as i32 + 1, hidden)
    }

    /// Inserts `count` blank rows so new rows appear at 0-based `row` (`InsertRows`); everything at/
    /// after `row` shifts down and formulas adjust (`insert_rows`, `common.rs:882`; undoable). A
    /// shift that would push used cells past the last row returns `Err(String)` (→ dialog).
    pub(crate) fn insert_rows(
        &mut self,
        sheet_idx: u32,
        row: u32,
        count: u32,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .insert_rows(sheet_idx, row as i32 + 1, count as i32)
    }

    /// Inserts `count` blank columns at 0-based `col` (`InsertColumns`, `common.rs:907`; undoable).
    pub(crate) fn insert_columns(
        &mut self,
        sheet_idx: u32,
        col: u32,
        count: u32,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .insert_columns(sheet_idx, col as i32 + 1, count as i32)
    }

    /// Deletes `count` rows starting at 0-based `row` (`DeleteRows`, `common.rs:932`; undoable —
    /// the removed data + heights + band styles are snapshotted for undo; formulas adjust).
    pub(crate) fn delete_rows(
        &mut self,
        sheet_idx: u32,
        row: u32,
        count: u32,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .delete_rows(sheet_idx, row as i32 + 1, count as i32)
    }

    /// Deletes `count` columns starting at 0-based `col` (`DeleteColumns`, `common.rs:974`;
    /// undoable).
    pub(crate) fn delete_columns(
        &mut self,
        sheet_idx: u32,
        col: u32,
        count: u32,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .delete_columns(sheet_idx, col as i32 + 1, count as i32)
    }

    /// The sheet's file-loaded merged ranges (0-based), parsed from `worksheet.merge_cells`
    /// (`Vec<String>` A1 ranges). Unparseable entries are skipped (defensive). The worker's merge
    /// guard reads this before an insert/delete (`components/grid_structure.md §5.3`).
    pub(crate) fn merge_ranges(&self, sheet_idx: u32) -> Result<Vec<CellRange>, String> {
        let ws = self.worksheet(sheet_idx)?;
        Ok(ws
            .merge_cells
            .iter()
            .filter_map(|m| CellRange::from_a1(m))
            .collect())
    }

    /// Whether `cell` currently has the given character-format flag set (the per-cell read the
    /// worker samples for toggle resolution).
    pub(crate) fn font_flag(
        &self,
        sheet_idx: u32,
        cell: CellRef,
        flag: FontFlag,
    ) -> Result<bool, String> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        let style = self.model.get_cell_style(sheet_idx, row, col)?;
        Ok(match flag {
            FontFlag::Bold => style.font.b,
            FontFlag::Italic => style.font.i,
            FontFlag::Underline => style.font.u,
            FontFlag::Strike => style.font.strike,
        })
    }

    /// Whether `cell` currently has wrap-text set (`alignment.wrap_text`) — the per-cell read the
    /// worker samples for the wrap toggle's decision. A cell with no alignment record reads
    /// `false` (mirrors the [`font_flag`](Self::font_flag) per-cell reader).
    pub(crate) fn wrap_flag(&self, sheet_idx: u32, cell: CellRef) -> Result<bool, String> {
        crate::instrument::record_engine_call();
        let (row, col) = to_engine_coords(cell);
        let style = self.model.get_cell_style(sheet_idx, row, col)?;
        Ok(style.alignment.map(|a| a.wrap_text).unwrap_or(false))
    }

    /// Sets a character-format flag across a range to `value` (one undoable range-style op).
    pub(crate) fn set_font_flag(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        flag: FontFlag,
        value: bool,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let v = if value { "true" } else { "false" };
        self.model
            .update_range_style(&area_of(sheet_idx, range), flag.style_path(), v)
    }

    /// Sets (or clears) a solid background fill across a range. `Some(rgb)` sets
    /// `fill.fg_color` to `#RRGGBB`; `None` passes the empty string, which IronCalc's style
    /// updater reads as "no fill".
    pub(crate) fn set_fill(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        fill: Option<Rgb>,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let value = match fill {
            Some(rgb) => format!("#{:06X}", rgb.to_hex()),
            None => String::new(),
        };
        self.model
            .update_range_style(&area_of(sheet_idx, range), "fill.fg_color", &value)
    }

    /// Sets a direct style attribute over a range via IronCalc's `update_range_style` path — the
    /// generic pass-through behind `SetStylePath` (text color / horizontal alignment / number
    /// format, `architecture.md §3.1`). One undoable range-style op; the band fast path engages
    /// automatically when `range` is a full row/column (`common.rs:1274`). `path` is one of the
    /// three typed [`StylePath`](crate::StylePath) strings, `value` its already-formatted payload.
    pub(crate) fn update_style_path(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        path: &str,
        value: &str,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .update_range_style(&area_of(sheet_idx, range), path, value)
    }

    /// Applies a border over a range: the `border_type` (an IronCalc `BorderType` serde tag) selects
    /// which edges are written; `style_tag` (a `BorderStyle` serde tag, e.g. `"thin"`/`"mediumdashed"`
    /// /`"double"`) and `color_hex` (`#RRGGBB`) are the written border item. Applied via
    /// `set_area_with_border` (`architecture.md §3.4/§4`, `border.rs:346`). One undoable diff-list;
    /// band-aware for full rows/columns; the engine applies its heavier-wins fix-up to the four
    /// adjacent strips and overwrites **only** the edges `border_type` implies (non-targeted edges,
    /// incl. interior borders, are preserved). `BorderArea` has `pub(crate)` fields and no constructor
    /// at 0.7.1 but derives `Deserialize`, so it is built from JSON. For `type: "None"` the engine
    /// ignores `item` and clears the edges.
    pub(crate) fn set_borders(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        border_type: &str,
        style_tag: &str,
        color_hex: &str,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let border_area: BorderArea = serde_json::from_value(serde_json::json!({
            "item": { "style": style_tag, "color": color_hex },
            "type": border_type,
        }))
        .map_err(|e| format!("failed to build BorderArea for {border_type:?}: {e}"))?;
        self.model
            .set_area_with_border(&area_of(sheet_idx, range), &border_area)
    }

    /// The workbook's **default font** `(size_pt, family_name)` — the font a truly-unstyled cell
    /// resolves to (style index 0). Read from the public workbook styles
    /// (`cell_xfs[0].font_id` → `fonts[id]`); a hostile/empty styles table falls back to
    /// IronCalc's `Font::default()` (13pt Calibri) rather than panicking. The cache resolves each
    /// cell's `font_size_q`/`font_family` **relative to this** (so a default-font cell interns to
    /// the default style, exactly as `font.color` is resolved relative to black — `architecture.md
    /// §1.1`, `components/style_render.md`). Cheap (a couple of `Vec` index reads).
    pub(crate) fn default_font(&self) -> (i32, String) {
        crate::instrument::record_engine_call();
        let styles = &self.model.get_model().workbook.styles;
        let font = styles
            .cell_xfs
            .first()
            .map(|xf| xf.font_id as usize)
            .and_then(|id| styles.fonts.get(id));
        match font {
            Some(f) => (f.sz, f.name.clone()),
            None => {
                let d = Font::default();
                (d.sz, d.name)
            }
        }
    }

    /// The workbook's colour theme, used to resolve theme-indexed cell colours
    /// (`Color::Theme`) to concrete `#RRGGBB` — see [`crate::cache::resolve_rgb`]. Read-only;
    /// the theme is a workbook-global property loaded on open (or the default for a new book).
    pub(crate) fn workbook_theme(&self) -> &ironcalc_base::types::Theme {
        &self.model.get_model().workbook.theme
    }

    /// Sets the font **family** and/or **size** over a range (`SetFont`, `architecture.md §3.3`).
    /// IronCalc 0.7.1 has no `font.name`/absolute-size `update_range_style` path, so this uses
    /// `on_paste_styles`: it points the engine's view selection at `range`, builds a row-major
    /// grid of each cell's **resolved** style (`get_style_for_cell`) with the font overridden, and
    /// pastes it back — one undoable diff-list. `family = Some("")` is "System Default" (reset to
    /// `default_name`); `Some(name)` sets it; `None` leaves the family. `size_pt = Some(pt)` sets
    /// `font.sz` (rounded to whole points — IronCalc stores an `i32`); `None` leaves the size.
    /// Because it materialises the resolved style per cell, a whole column/row is clamped to the
    /// used range by the caller first (no font bands — documented deviation).
    pub(crate) fn set_font(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        family: Option<&str>,
        size_pt: Option<f64>,
        default_name: &str,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        // on_paste_styles pastes into the engine's view selection; the top-left anchor is on
        // every range's edge, so this passes IronCalc's anchor-on-edge check.
        self.set_view_selection(sheet_idx, range)?;
        let model = self.model.get_model();
        let mut styles: Vec<Vec<Style>> = Vec::with_capacity(range.rows().count());
        for row in range.rows() {
            let mut row_styles: Vec<Style> = Vec::with_capacity(range.cols().count());
            for col in range.cols() {
                let (r, c) = to_engine_coords(CellRef::new(row, col));
                let mut style = model.get_style_for_cell(sheet_idx, r, c)?;
                if let Some(pt) = size_pt {
                    style.font.sz = pt.round() as i32;
                }
                if let Some(name) = family {
                    style.font.name = if name.is_empty() {
                        default_name.to_string()
                    } else {
                        name.to_string()
                    };
                }
                row_styles.push(style);
            }
            styles.push(row_styles);
        }
        self.model.on_paste_styles(&styles)
    }

    /// Sets a contiguous run of rows `[row_start, row_end]` (0-based) to `px` **IronCalc pixels**
    /// (one undoable diff-list) — the row auto-grow primitive behind `SetFont` (`architecture.md
    /// §3.3`). Never called with unbounded ranges (the worker coalesces only rows that need
    /// growing, within a clamped selection).
    pub(crate) fn set_row_heights_run(
        &mut self,
        sheet_idx: u32,
        row_start: u32,
        row_end: u32,
        px: f64,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model
            .set_rows_height(sheet_idx, row_start as i32 + 1, row_end as i32 + 1, px)
    }

    /// Clamps a **full-row / full-column / select-all** `range` to the sheet's used rectangle
    /// (`worksheet.dimension()`), returning `None` when the intersection is empty (nothing to do).
    /// A **bounded** selection is returned unchanged — it applies exactly as selected, even over
    /// empty cells (`architecture.md §5.2`). Centralised here so `SetFont` (and Phase-7 clears)
    /// never iterate a 1M-cell band.
    pub(crate) fn clamp_to_used(
        &self,
        sheet_idx: u32,
        range: CellRange,
    ) -> Result<Option<CellRange>, String> {
        use freecell_core::limits;
        let spans_all_rows = range.start.row == 0 && range.end.row == limits::MAX_ROWS - 1;
        let spans_all_cols = range.start.col == 0 && range.end.col == limits::MAX_COLS - 1;
        if !(spans_all_rows || spans_all_cols) {
            return Ok(Some(range));
        }
        let dim = self.worksheet(sheet_idx)?.dimension();
        // dimension() is 1-based inclusive; convert to 0-based and intersect with the request.
        let used_r0 = (dim.min_row.max(1) - 1) as u32;
        let used_r1 = (dim.max_row.max(1) - 1) as u32;
        let used_c0 = (dim.min_column.max(1) - 1) as u32;
        let used_c1 = (dim.max_column.max(1) - 1) as u32;
        let r0 = range.start.row.max(used_r0);
        let r1 = range.end.row.min(used_r1);
        let c0 = range.start.col.max(used_c0);
        let c1 = range.end.col.min(used_c1);
        if r1 < r0 || c1 < c0 {
            return Ok(None); // the selected band lies entirely outside the used range
        }
        Ok(Some(CellRange::new(
            CellRef::new(r0, c0),
            CellRef::new(r1, c1),
        )))
    }

    /// Scan `sheet_idx`'s **populated** cells for those whose raw content matches `query`
    /// (`functional_spec.md §4.3`; `Command::Find`). Iterates `sheet_data` (only populated cells,
    /// not the whole used rectangle — O(populated), huge-sheet safe) and returns the matches sorted
    /// **row-major** (`sheet_data` is a `HashMap`, so its iteration order is arbitrary and must be
    /// sorted). Reads the raw content per cell (formula text for formula cells), so a match against a
    /// formula is a match against its text. No mutation, no eval.
    pub(crate) fn find_matches(
        &self,
        sheet_idx: u32,
        query: &str,
        match_case: bool,
        whole_cell: bool,
    ) -> Result<Vec<CellRef>, String> {
        crate::instrument::record_engine_call();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let ws = self.worksheet(sheet_idx)?;
        let mut matches = Vec::new();
        for (row_1, cols) in &ws.sheet_data {
            let row0 = (*row_1 - 1).max(0) as u32;
            for col_1 in cols.keys() {
                let col0 = (*col_1 - 1).max(0) as u32;
                let cell = CellRef::new(row0, col0);
                // `cell_content` returns the formula text for formula cells (Excel "Look in:
                // Formulas") — the same raw string `replace_in_cell` edits (`§4.3`).
                let content = self.cell_content(sheet_idx, cell).unwrap_or_default();
                if freecell_core::find::cell_matches(&content, query, match_case, whole_cell) {
                    matches.push(cell);
                }
            }
        }
        matches.sort_unstable_by_key(|c| (c.row, c.col));
        Ok(matches)
    }

    /// Aggregate statistics for `range ∩ the sheet's populated cells` (`Command::SelectionStats`,
    /// `functional_spec.md §1`; the status-bar readout).
    ///
    /// Walks **populated** cells only (`sheet_data`, exactly like [`find_matches`](Self::find_matches))
    /// and keeps those inside `range` — so a full-column/row selection over a mostly-empty sheet is
    /// O(populated), never O(cells selected), and is correct **past the published viewport** (the
    /// correctness-beyond-the-viewport guarantee, `§1` "Correctness / performance"). No
    /// `dimension()` clamp is needed: iterating `sheet_data` already restricts to the used range, and
    /// an empty (blank) cell contributes to neither `count` nor the math.
    ///
    /// Excel semantics: `count` = every non-empty cell (text + numbers + booleans + errors); the
    /// sum / min / max / average population is the **numeric** cells only. Errors arrive from
    /// [`cell_value`](Self::cell_value) as [`CellData::Text`] (their `#DIV/0!`-style string), so —
    /// D1.1 — they count but are excluded from the math, identically to text. No mutation, no eval.
    pub(crate) fn selection_stats(&self, sheet_idx: u32, range: CellRange) -> SelectionStats {
        crate::instrument::record_engine_call();
        let ws = match self.worksheet(sheet_idx) {
            Ok(ws) => ws,
            Err(_) => return SelectionStats::EMPTY,
        };
        let mut stats = SelectionStats::EMPTY;
        for (row_1, cols) in &ws.sheet_data {
            let row0 = (*row_1 - 1).max(0) as u32;
            if row0 < range.start.row || row0 > range.end.row {
                continue;
            }
            for col_1 in cols.keys() {
                let col0 = (*col_1 - 1).max(0) as u32;
                if col0 < range.start.col || col0 > range.end.col {
                    continue;
                }
                match self.cell_value(sheet_idx, CellRef::new(row0, col0)) {
                    CellData::Number(n) => stats.push_number(n),
                    // Text, booleans, and errors are non-empty but non-numeric: they count, but
                    // are excluded from Sum/Average/Min/Max (D1.1 treats errors like text).
                    CellData::Text(_) | CellData::Bool(_) => stats.push_non_numeric(),
                    // A blank cell present in `sheet_data` (e.g. carries only a style) is not
                    // counted — "blanks don't" (`§1`).
                    CellData::Empty => {}
                }
            }
        }
        stats
    }

    /// Resolve the **edge-of-data** target for a ⌘/Ctrl+arrow jump from `from` in `dir`
    /// (`Command::ResolveEdge`, `functional_spec.md §4`; the status quo `JumpEdge`/`ExtendEdge`).
    ///
    /// Gathers the **populated** indices on the active cell's line (rows in `from`'s column for a
    /// vertical motion, cols in `from`'s row for a horizontal one) from `sheet_data` — populated
    /// cells only, like [`find_matches`](Self::find_matches)/[`selection_stats`](Self::selection_stats)
    /// — **sorts them ascending**, and feeds the slice to the pure Excel algorithm
    /// ([`freecell_core::resolve_edge`]) over the full sheet dims. A cell is "populated" iff its raw
    /// content is non-empty (a style-only `sheet_data` entry is empty content → not occupied, matching
    /// the selection-stats "blanks don't" rule).
    ///
    /// Cost to collect + sort the line's occupancy: O(populated cells on the line) for a row jump;
    /// O(populated rows) for a column jump (row-major `sheet_data` is scanned per row for the column's
    /// key). The resolve itself is then **O(log populated)** (binary searches — no per-cell walk across
    /// empty space, so a jump through an empty 1M-cell column is a couple of lookups). Never O(cells
    /// selected). No mutation, no eval; correct **past the published viewport** (occupancy lives here in
    /// the model).
    pub(crate) fn resolve_edge(&self, sheet_idx: u32, from: CellRef, dir: Direction) -> CellRef {
        crate::instrument::record_engine_call();
        let dims = SheetDims::new(
            freecell_core::limits::MAX_ROWS,
            freecell_core::limits::MAX_COLS,
        );
        let ws = match self.worksheet(sheet_idx) {
            Ok(ws) => ws,
            // An unresolvable sheet has no occupancy — no move.
            Err(_) => return from,
        };
        let vertical = matches!(dir, Direction::Up | Direction::Down);
        // The candidate indices present in `sheet_data` on the active line (1-based engine keys →
        // 0-based). Collected up front so the `&ws` borrow ends before the per-cell content probe.
        let candidates: Vec<u32> = if vertical {
            let col_1 = from.col as i32 + 1;
            ws.sheet_data
                .iter()
                .filter(|(_row_1, cols)| cols.contains_key(&col_1))
                .map(|(row_1, _cols)| (*row_1 - 1).max(0) as u32)
                .collect()
        } else {
            match ws.sheet_data.get(&(from.row as i32 + 1)) {
                Some(cols) => cols
                    .keys()
                    .map(|col_1| (*col_1 - 1).max(0) as u32)
                    .collect(),
                None => Vec::new(),
            }
        };
        // Keep only genuinely populated cells (a `sheet_data` entry can be style-only → empty content),
        // sorted ascending — the shape `resolve_edge`'s binary searches require.
        let mut occupied: Vec<u32> = candidates
            .into_iter()
            .filter(|&idx| {
                let cell = if vertical {
                    CellRef::new(idx, from.col)
                } else {
                    CellRef::new(from.row, idx)
                };
                self.cell_content(sheet_idx, cell)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            })
            .collect();
        occupied.sort_unstable();
        freecell_core::resolve_edge(from, dir, dims, &occupied)
    }

    /// Replace `query` with `replacement` in a single cell (`Command::ReplaceOne`,
    /// `functional_spec.md §4.4`): re-read the cell's raw content **here** (avoiding a stale-content
    /// race with the UI) and, when it matches, write the replaced content via `set_user_input`.
    /// Returns whether it wrote — a match whose replacement yields the **same** text is a no-op
    /// (skipped, returns `false`), so it does not spend an undo entry or count toward "Replaced N".
    /// Auto-evaluates unless paused; one undo step (single cell).
    pub(crate) fn replace_one(
        &mut self,
        sheet_idx: u32,
        cell: CellRef,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
    ) -> Result<bool, String> {
        let content = self.cell_content(sheet_idx, cell).unwrap_or_default();
        match freecell_core::find::replace_in_cell(
            &content,
            query,
            replacement,
            match_case,
            whole_cell,
        ) {
            // A replacement that yields identical text is a no-op — don't spend an undo entry on it.
            Some(new_input) if new_input != content => {
                self.set_cell_input(sheet_idx, cell, &new_input)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Replace `query` with `replacement` in **every** matching cell of `sheet_idx`'s used range
    /// (`Command::ReplaceAll`, `functional_spec.md §4.4`) and return the cells that changed (in
    /// row-major order) — the caller counts them and records a single undo touch.
    ///
    /// The matches are collected first (over `sheet_data`, releasing the borrow), then written — a
    /// cell whose replaced content equals its old content is skipped (no write). All the writes go
    /// through **one** `set_user_inputs` batch call, so the whole replace is a **single** engine undo
    /// entry (one `doc.undo()` reverts every replaced cell — the fork fix from `phase_plans/phase_9.md`,
    /// replacing the former per-cell `set_user_input` loop). Called under the worker's paused-eval
    /// guard (the batch's single `evaluate()` follows).
    pub(crate) fn replace_all_matches(
        &mut self,
        sheet_idx: u32,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
    ) -> Result<Vec<CellRef>, String> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        // Collect (cell, new_content) up front so we are not mutating `sheet_data` mid-iteration.
        let mut edits: Vec<(CellRef, String)> = Vec::new();
        {
            let ws = self.worksheet(sheet_idx)?;
            for (row_1, cols) in &ws.sheet_data {
                let row0 = (*row_1 - 1).max(0) as u32;
                for col_1 in cols.keys() {
                    let col0 = (*col_1 - 1).max(0) as u32;
                    let cell = CellRef::new(row0, col0);
                    let content = self.cell_content(sheet_idx, cell).unwrap_or_default();
                    if let Some(new_input) = freecell_core::find::replace_in_cell(
                        &content,
                        query,
                        replacement,
                        match_case,
                        whole_cell,
                    ) {
                        if new_input != content {
                            edits.push((cell, new_input));
                        }
                    }
                }
            }
        }
        // `sheet_data` is a `HashMap` (arbitrary order); write row-major so the returned + touched
        // cells are deterministic.
        edits.sort_by_key(|(cell, _)| (cell.row, cell.col));
        if edits.is_empty() {
            return Ok(Vec::new());
        }
        // Apply every replacement as ONE batched write → a single engine undo entry (so a single
        // Undo reverts the whole Replace All). `set_user_inputs` takes 1-based (sheet, row, col)
        // engine coords.
        crate::instrument::record_engine_call();
        let mut batch: Vec<(u32, i32, i32, String)> = Vec::with_capacity(edits.len());
        let mut changed: Vec<CellRef> = Vec::with_capacity(edits.len());
        for (cell, new_input) in edits {
            let (row, col) = to_engine_coords(cell);
            batch.push((sheet_idx, row, col, new_input));
            changed.push(cell);
        }
        self.model.set_user_inputs(&batch)?;
        Ok(changed)
    }

    /// Appends a new sheet (`AddSheet`); IronCalc names + numbers it. Undoable.
    pub(crate) fn add_sheet(&mut self) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.new_sheet()
    }

    /// Renames the sheet at `sheet_idx` (`RenameSheet`). The worker re-validates the name
    /// against the other sheets first; IronCalc enforces its own rules too. Undoable.
    pub(crate) fn rename_sheet(&mut self, sheet_idx: u32, name: &str) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.rename_sheet(sheet_idx, name)
    }

    /// Deletes the sheet at `sheet_idx` (`DeleteSheet`). Can affect formulas → the worker
    /// re-evaluates. Undoable.
    pub(crate) fn delete_sheet(&mut self, sheet_idx: u32) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.delete_sheet(sheet_idx)
    }

    /// Moves the sheet at `sheet_idx` to `to_index` (`MoveSheet`), shifting the intervening
    /// sheets so the moved sheet lands at exactly `to_index`. Undoable (rides the fork's
    /// history); the new order is preserved on xlsx save; cross-sheet references stay valid
    /// (sheet order is a vector position, not an identity). Wraps the fork's index-based
    /// `UserModel::set_worksheet_index` (Phase 6a). A same-index move is a fork-level no-op.
    pub(crate) fn move_sheet(&mut self, sheet_idx: u32, to_index: u32) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.set_worksheet_index(sheet_idx, to_index)
    }

    /// Undoes the last committed edit (engine history). Auto-evaluates unless paused.
    pub(crate) fn undo(&mut self) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.undo()
    }

    /// Redoes the last undone edit (engine history). Auto-evaluates unless paused.
    pub(crate) fn redo(&mut self) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.redo()
    }

    // ---- Range clipboard (`components/clipboard.md`, `architecture.md §6`) -----------------
    //
    // These are the ONLY feature routing through IronCalc's hidden view-selection state:
    // `copy_to_clipboard` / `paste_from_clipboard` / `paste_csv_string` all read the engine's
    // *selected view* (not their arguments) for the sheet + anchor, so each op first sets the
    // selection, then calls the engine API. The selection is view-only (not undoable, no eval).

    /// Point the engine's view selection at `range` (top-left as the anchor — always on the
    /// range edge, so IronCalc's edge check passes even for full row/column/select-all ranges).
    fn set_view_selection(&mut self, sheet_idx: u32, range: CellRange) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.set_selected_sheet(sheet_idx)?;
        let (r0, c0) = to_engine_coords(range.start);
        let (r1, c1) = to_engine_coords(range.end);
        // The selected cell must be set before the range (IronCalc validates the range against
        // it); the top-left corner is on every range's edge.
        self.model.set_selected_cell(r0, c0)?;
        self.model.set_selected_range(r0, c0, r1, c1)?;
        Ok(())
    }

    /// Copy `range` on `sheet_idx` to the engine clipboard (`copy_to_clipboard`, `common.rs:1765`).
    /// The engine clamps the copied rectangle to `worksheet.dimension()`, so a full-column /
    /// select-all copy is cheap; the returned `range` is that clamped extent. The `Clipboard`
    /// struct isn't nameable outside ironcalc_base (private fields, not re-exported), so its
    /// `csv` / `data` / `range` are read out of its `Serialize` form.
    pub(crate) fn copy_range(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
    ) -> Result<CopiedRange, String> {
        self.set_view_selection(sheet_idx, range)?;
        crate::instrument::record_engine_call();
        let clipboard = self.model.copy_to_clipboard()?;
        let value = serde_json::to_value(&clipboard)
            .map_err(|e| format!("failed to serialize clipboard: {e}"))?;
        let tsv = value
            .get("csv")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let data = value
            .get("data")
            .cloned()
            .ok_or_else(|| "clipboard payload missing `data`".to_string())?;
        let range = value
            .get("range")
            .cloned()
            .ok_or_else(|| "clipboard payload missing `range`".to_string())
            .and_then(|v| {
                serde_json::from_value::<(i32, i32, i32, i32)>(v)
                    .map_err(|e| format!("bad clipboard range: {e}"))
            })?;
        // Snapshot the block's computed values as literal paste tokens for a later Paste Values
        // (⌘⇧V) — captured here so paste-values pastes values, never re-derived formulas.
        let values = self.copied_value_tokens(sheet_idx, range);
        Ok(CopiedRange {
            tsv,
            data,
            range,
            values,
        })
    }

    /// The computed values of the effective (1-based, inclusive) copied `range` as literal paste
    /// tokens, row-major — the Paste Values (⌘⇧V, `functional_spec.md §5`) snapshot stored on the
    /// clipboard slot. Each token is what [`value_token`](Self::value_token) renders; pasting them
    /// via [`paste_values`](Self::paste_values) reproduces the *values only* (no formulas, no
    /// formatting) at the target.
    fn copied_value_tokens(&self, sheet_idx: u32, range: (i32, i32, i32, i32)) -> Vec<Vec<String>> {
        let (r0, c0, r1, c1) = range;
        if r1 < r0 || c1 < c0 {
            return Vec::new(); // degenerate (inverted) clamp — nothing to copy
        }
        // The workbook's decimal separator, resolved once, so a fractional Number token re-parses
        // as a number under this workbook's own locale (the write path parses with it) — not only
        // under an `en`/`.`-decimal workbook.
        let decimal_sep = self.workbook_decimal_separator();
        (r0..=r1)
            .map(|row| {
                (c0..=c1)
                    .map(|col| self.value_token(sheet_idx, row, col, decimal_sep))
                    .collect()
            })
            .collect()
    }

    /// The workbook locale's decimal separator (default `.`). Paste Values re-parses a Number
    /// token through the write path (`set_user_input` → `parse_formatted_number`), which uses the
    /// **workbook** locale's separators — so a fractional token must carry this separator to land
    /// as a number rather than falling through to text under a non-`.`-decimal workbook (e.g. a
    /// German-locale xlsx: decimal `,`, group `.`).
    fn workbook_decimal_separator(&self) -> char {
        crate::instrument::record_engine_call();
        let model = self.model.get_model();
        get_locale(&model.workbook.settings.locale)
            .ok()
            .and_then(|loc| loc.numbers.symbols.decimal.chars().next())
            .unwrap_or('.')
    }

    /// One cell's evaluated value rendered as a **literal** Paste Values token (1-based coords):
    /// re-expressed so writing it back with [`set_user_input`](UserModel::set_user_input)
    /// reproduces the same *value* under the target's own formatting, never re-interpreting it as a
    /// formula (`functional_spec.md §5`). `decimal_sep` is the workbook locale's decimal separator
    /// (from [`workbook_decimal_separator`](Self::workbook_decimal_separator)).
    ///
    /// - `Number(n)` → the plain unformatted decimal (`f64` `Display`, never grouped or scientific,
    ///   with the decimal point rendered as `decimal_sep`), which re-parses to the same number with
    ///   **no** number format applied (a bare, ungrouped number infers none) under this workbook's
    ///   locale — at full precision, not the rounded display string, so the target keeps its format.
    /// - `Boolean` → `TRUE`/`FALSE` (re-parses to the boolean).
    /// - An **error** cell (its value surfaces as the error string) → that string as-is, so it
    ///   re-parses back to the same error value.
    /// - A non-empty **string** → apostrophe-quoted so it is forced to literal text: this keeps a
    ///   value that only *looks* like a formula (`=x`, `+x`, `-x`, `@x`) from being parsed as one
    ///   and preserves text-vs-number typing (a text `"12"` stays text, not the number 12). The
    ///   `'` toggles only the quote_prefix marker; number format / font / fill / borders are
    ///   untouched.
    /// - Empty — a blank cell **or** an empty-string value (`=""`) → `""`, which **clears** the
    ///   target (a true blank, not a counted empty-text cell), matching a paste of a blank source.
    fn value_token(&self, sheet_idx: u32, row: i32, col: i32, decimal_sep: char) -> String {
        crate::instrument::record_engine_call();
        match self
            .model
            .get_model()
            .get_cell_value_by_index(sheet_idx, row, col)
        {
            Ok(CellValue::Number(n)) => number_token(n, decimal_sep),
            Ok(CellValue::Boolean(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
            // An empty-string value is written as a clear (a true blank), not a quoted empty text.
            Ok(CellValue::String(s)) if s.is_empty() => String::new(),
            Ok(CellValue::String(s)) => {
                // An error cell's value is its error string — let it round-trip as the error
                // (unquoted). Genuine text is force-quoted so it is written as a literal.
                if matches!(
                    self.model.get_cell_type(sheet_idx, row, col),
                    Ok(CellType::ErrorValue)
                ) {
                    s
                } else {
                    format!("'{s}")
                }
            }
            Ok(CellValue::None) | Err(_) => String::new(),
        }
    }

    /// One cell's **raw stored value** rendered for CSV export (`functional_spec.md §2`, D2.2 —
    /// computed underlying values, *not* the formatted display string and *not* the formula
    /// source). Like [`value_token`](Self::value_token) but text is written **verbatim** (no
    /// leading-apostrophe quote-prefix — the csv writer handles RFC-4180 quoting), so a cell whose
    /// display would be `50%` writes `0.5`, a date serial writes its serial number, a boolean
    /// writes `TRUE`/`FALSE`, an error writes its error string, a formula writes its computed
    /// value, and an empty cell writes `""`.
    fn export_cell_value(&self, sheet_idx: u32, row: i32, col: i32, decimal_sep: char) -> String {
        crate::instrument::record_engine_call();
        match self
            .model
            .get_model()
            .get_cell_value_by_index(sheet_idx, row, col)
        {
            // FOLLOW-ON (tracked in GAPS.md, localization): `number_token` renders the decimal point
            // as the **workbook locale's** separator. For a comma-decimal locale (e.g. `de`) that
            // collides with the comma CSV delimiter — `0.5 → "0,5"`, which the csv writer then quotes
            // and a re-import reads as text. Latent only today: the app is en-locale-only
            // (`DEFAULT_LOCALE = "en"` → `.`), and D2.4 keeps this round comma-only. The localization
            // pass should switch the delimiter to `;` for comma-decimal locales.
            Ok(CellValue::Number(n)) => number_token(n, decimal_sep),
            Ok(CellValue::Boolean(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
            // Both genuine text and an error cell surface as `String(s)`; write either verbatim
            // (the error string is its value; text is its value).
            Ok(CellValue::String(s)) => s,
            Ok(CellValue::None) | Err(_) => String::new(),
        }
    }

    /// Exports the active sheet's **used range** to `path` as a `.csv` (`functional_spec.md §2`,
    /// D2.2). Each cell renders its raw stored value via [`export_cell_value`](Self::export_cell_value);
    /// trailing empty fields in a row are trimmed (no trailing commas past the used range); the
    /// `csv` writer serializes RFC-4180 with `CRLF` line endings, quoting any field containing a
    /// comma, double-quote, or newline. Written atomically (temp-file + fsync + rename), so a
    /// failure leaves any existing file intact. An empty sheet writes a 0-byte file. `pub(crate)`:
    /// the walk reads the IronCalc `Worksheet`, which never leaves this crate.
    pub(crate) fn export_csv(&self, sheet_idx: u32, path: &Path) -> Result<(), SaveError> {
        crate::instrument::record_engine_call();
        let ws = self.worksheet(sheet_idx).map_err(SaveError::Serialize)?;

        // A sheet with no populated cells → a clean empty file (edge case: no error).
        if ws.sheet_data.is_empty() {
            let temp = new_temp_beside(path)?;
            return persist_atomically(temp, path);
        }

        let dim = ws.dimension();
        let (min_row, max_row) = (dim.min_row, dim.max_row);
        let (min_col, max_col) = (dim.min_column, dim.max_column);
        let decimal_sep = self.workbook_decimal_separator();

        let temp = new_temp_beside(path)?;
        {
            // `flexible(true)` is required: trailing-empty trimming yields ragged records, which a
            // strict writer would reject with an unequal-lengths error.
            let mut writer = csv::WriterBuilder::new()
                .terminator(csv::Terminator::CRLF)
                .flexible(true)
                .from_writer(BufWriter::new(temp.as_file()));
            for row in min_row..=max_row {
                let mut fields: Vec<String> = (min_col..=max_col)
                    .map(|col| self.export_cell_value(sheet_idx, row, col, decimal_sep))
                    .collect();
                while fields.last().is_some_and(|f| f.is_empty()) {
                    fields.pop();
                }
                writer
                    .write_record(&fields)
                    .map_err(|e| SaveError::Serialize(e.to_string()))?;
            }
            writer.flush().map_err(|e| SaveError::Io(e.to_string()))?;
        }
        persist_atomically(temp, path)
    }

    /// Paste a previously-copied engine payload at `anchor` on `dest_idx` (`paste_from_clipboard`,
    /// `common.rs:1811`): Excel relative-reference adjustment on copy, move semantics + source
    /// clear on cut, one undoable diff list, then the pasted area is re-selected. `source_idx` /
    /// `source_range` are the copy-time sheet index + effective rectangle (the source cleared on
    /// cut). The caller pauses evaluation around this (the batch's single recompute follows).
    pub(crate) fn paste_clipboard(
        &mut self,
        dest_idx: u32,
        source_idx: u32,
        source_range: (i32, i32, i32, i32),
        data_json: &serde_json::Value,
        cut: bool,
        target: CellRange,
    ) -> Result<(), String> {
        // The paste anchors at the target's top-left (the destination selection's anchor).
        let anchor = target.start;
        // A single-cell (or exact-divisor) COPY into a larger selection tiles/fills the source
        // across the whole `target` as ONE diff-list — the engine only ever pastes the source
        // rectangle once at the anchor, so we synthesize a target-sized payload here (BUG 4). Values
        // and styles fill exactly; because the whole synthesized block is pasted in one call it
        // takes a single uniform `anchor − source` reference shift, so a **formula** is filled with
        // the top-left cell's shift on every cell, NOT Excel's per-cell relative fill (accepted
        // limitation U2 in `GAPS.md`; per-cell fill would need N×M pastes = N×M undo entries). A cut
        // is a move with a single destination, so it never fills.
        let fill = (!cut)
            .then(|| fill_target_dims(source_range, target))
            .flatten();
        // Set the destination selection to the single anchor cell (the paste anchors from it).
        self.set_view_selection(dest_idx, CellRange::single(anchor))?;
        crate::instrument::record_engine_call();
        match fill {
            Some((tw, th)) => {
                let tiled_json = tile_clipboard_json(data_json, source_range, tw, th)?;
                let data = ClipboardData::deserialize(&tiled_json)
                    .map_err(|e| format!("failed to deserialize tiled clipboard data: {e}"))?;
                let (sr0, sc0, _, _) = source_range;
                let tiled_range = (sr0, sc0, sr0 + th as i32 - 1, sc0 + tw as i32 - 1);
                self.model
                    .paste_from_clipboard(source_idx, tiled_range, &data, cut)
            }
            None => {
                // Deserialize directly from the borrowed `Value` (no clone — `&Value` is a
                // Deserializer).
                let data = ClipboardData::deserialize(data_json)
                    .map_err(|e| format!("failed to deserialize clipboard data: {e}"))?;
                self.model
                    .paste_from_clipboard(source_idx, source_range, &data, cut)
            }
        }
    }

    /// Paste a tab-delimited TSV at `anchor` on `dest_idx` (`paste_csv_string`, `common.rs:1926`):
    /// each field is applied as user input (numbers / booleans / `=formulas` / text), one
    /// undoable diff list, then the pasted area is re-selected. Only `area.{sheet,row,column}`
    /// are used by the engine (width/height are ignored — the reader derives them from the text).
    pub(crate) fn paste_tsv(
        &mut self,
        dest_idx: u32,
        anchor: CellRef,
        text: &str,
    ) -> Result<(), String> {
        // Set the destination selection so the engine's end-of-paste re-select has a valid anchor.
        self.set_view_selection(dest_idx, CellRange::single(anchor))?;
        let (row, column) = to_engine_coords(anchor);
        let area = Area {
            sheet: dest_idx,
            row,
            column,
            width: 1,
            height: 1,
        };
        crate::instrument::record_engine_call();
        self.model.paste_csv_string(&area, text)
    }

    /// Paste the copied **computed values** (Paste Values, ⌘⇧V — `functional_spec.md §5`) at
    /// `anchor` on `dest_idx`: values only, no formulas, no formatting — the target keeps its own.
    /// `values` is the source block's literal tokens (row-major, `src_h × src_w`, from
    /// [`copied_value_tokens`](Self::copied_value_tokens)); it is **tiled** to fill a
    /// `paste_w × paste_h` block (a single-cell / exact-divisor source fills the larger selection,
    /// matching the internal-paste fill rule — `paste_w`/`paste_h` come from
    /// [`fill_target_dims`], or equal the source dims for a straight block paste). Every target
    /// cell is written through **one** `set_user_inputs` batch → a single undo entry (an empty
    /// token clears its cell, so a blank source cell clears the target). The pasted rectangle is
    /// re-selected so the caller (`run_guarded_paste`) reads it back. The caller pauses evaluation
    /// around this (its single coalesced recompute follows).
    pub(crate) fn paste_values(
        &mut self,
        dest_idx: u32,
        anchor: CellRef,
        values: &[Vec<String>],
        paste_w: u32,
        paste_h: u32,
    ) -> Result<(), String> {
        let src_h = values.len();
        let src_w = values.first().map(Vec::len).unwrap_or(0);
        if src_h == 0 || src_w == 0 {
            return Ok(()); // nothing to paste
        }
        let (anchor_row, anchor_col) = to_engine_coords(anchor);
        let mut batch: Vec<(u32, i32, i32, String)> =
            Vec::with_capacity(paste_w as usize * paste_h as usize);
        for dr in 0..paste_h as i32 {
            let src_row = &values[dr as usize % src_h];
            for dc in 0..paste_w as i32 {
                let token = src_row[dc as usize % src_w].clone();
                batch.push((dest_idx, anchor_row + dr, anchor_col + dc, token));
            }
        }
        crate::instrument::record_engine_call();
        self.model.set_user_inputs(&batch)?;
        // Re-select the pasted rectangle so `selected_range_0based` mirrors it into the UI
        // selection (both other paste APIs re-select their pasted area too).
        let end = CellRef::new(anchor.row + paste_h - 1, anchor.col + paste_w - 1);
        self.set_view_selection(dest_idx, CellRange::new(anchor, end))
    }

    /// The engine's current view selection as a 0-based [`CellRange`] — read back right after a
    /// paste (both paste APIs re-select the pasted rectangle) to mirror it into FreeCell's
    /// `SelectionModel`.
    pub(crate) fn selected_range_0based(&self) -> CellRange {
        crate::instrument::record_engine_call();
        let [r0, c0, r1, c1] = self.model.get_selected_view().range;
        // Engine coords are 1-based inclusive; clamp the `- 1` at 0 defensively.
        let cell = |r: i32, c: i32| CellRef::new(r.max(1) as u32 - 1, c.max(1) as u32 - 1);
        CellRange::new(cell(r0, c0), cell(r1, c1))
    }

    /// Mutable reference to the owned model — the write seam used by the [`fixtures`] module
    /// to populate cells. In-crate only; the model is an `ironcalc` type and never leaves this
    /// crate. (The Phase-4 worker drives edits through the typed methods above, not this.)
    ///
    /// Handing out this raw handle is itself counted as an engine access: the ops performed
    /// *through* it are not individually instrumented, so bumping the counter here keeps the
    /// "any engine model access bumps the counter" invariant airtight for this escape hatch.
    ///
    /// [`fixtures`]: crate::fixtures
    pub(crate) fn user_model_mut(&mut self) -> &mut UserModel<'static> {
        crate::instrument::record_engine_call();
        &mut self.model
    }

    /// Shared reference to the owned model — the read seam the CF wrapper
    /// ([`document::cond_fmt`](crate::document::cond_fmt)) uses to reach the `UserModel` CF query
    /// API (`get_conditional_formatting_list` / `get_dxf_for_conditional_formatting` /
    /// `get_extended_cell_style`). In-crate only; the `UserModel` is an `ironcalc` type and never
    /// leaves this crate. Bumps the engine-call counter to keep the "any engine model access is
    /// counted" invariant airtight (the reads performed *through* it are not individually
    /// instrumented).
    // P1 seam: the CF read methods that consume this land their non-test callers in P2/P3.
    #[allow(dead_code)]
    pub(crate) fn user_model(&self) -> &UserModel<'static> {
        crate::instrument::record_engine_call();
        &self.model
    }

    /// The resolved `ironcalc` style of a cell — a test-only helper for style round-trip
    /// assertions (Phase 5's style cache is the real style-read path).
    #[cfg(test)]
    pub(crate) fn cell_style(&self, sheet: u32, cell: CellRef) -> Result<Style, CellQueryError> {
        let (row, col) = to_engine_coords(cell);
        self.model
            .get_cell_style(sheet, row, col)
            .map_err(CellQueryError)
    }
}

/// The fully-resolved text colour for a cell (`architecture.md §1.2`, precedence: explicit
/// non-black font colour → number-format `[Red]`-style colour → `None`). Shares the cache's
/// [`parse_color`](crate::cache::parse_color) + black-filter so it agrees with the resident
/// style cache the grid also reads.
fn resolve_text_color(model: &Model, sheet: u32, row: i32, col: i32, style: &Style) -> Option<Rgb> {
    // 1. An explicit non-black font colour always wins (a pure-black colour is
    //    indistinguishable from IronCalc's default, so it falls through — matching the cache).
    if let Some(rgb) = crate::cache::resolve_rgb(&style.font.color, &model.workbook.theme)
        .filter(|c| *c != Rgb::new(0, 0, 0))
    {
        return Some(rgb);
    }
    // 2. A number-format-produced colour (e.g. `[Red]` negatives). Only formats carrying a
    //    `[...]` section can produce one, and only numeric values are formatted — gate on
    //    both to avoid formatting text/blank cells.
    if !style.num_fmt.contains('[') {
        return None;
    }
    let value = match model.get_cell_value_by_index(sheet, row, col) {
        Ok(CellValue::Number(v)) => v,
        _ => return None,
    };
    let locale = get_locale(&model.workbook.settings.locale).ok()?;
    format_color_rgb(format_number(value, &style.num_fmt, locale).color?)
}

/// Converts a 0-based [`CellRef`] to IronCalc's 1-based `(row, column)` `i32` coordinates.
/// The Excel-max bounds (`freecell_core::limits`) fit comfortably in `i32`.
fn to_engine_coords(cell: CellRef) -> (i32, i32) {
    (cell.row as i32 + 1, cell.col as i32 + 1)
}

/// Renders a numeric value as a Paste Values token: Rust's `f64` `Display` (full round-trip
/// precision, never a group separator, never scientific for finite magnitudes) with its single
/// `.` decimal point rewritten to the workbook locale's `decimal_sep`. This is the ungrouped,
/// locale-correct form `parse_formatted_number` re-parses to the same number with **no** format
/// inferred — so a fractional number pastes as a number (not text) under any workbook locale.
fn number_token(n: f64, decimal_sep: char) -> String {
    let s = n.to_string();
    if decimal_sep == '.' {
        s
    } else {
        s.replace('.', &decimal_sep.to_string())
    }
}

/// The tiled destination dims `(width, height)` in cells when a copied `source_range` fills a
/// larger `target` selection by whole-multiple replication (single-cell / exact-divisor block
/// fill): `Some((tw, th))` iff `target` is an exact multiple of the source in BOTH axes AND
/// strictly larger; else `None` (paste the source once at the anchor). Shared by the worker (fill
/// cap) and [`WorkbookDocument::paste_clipboard`] (the synthesis) so the two can never disagree.
/// `source_range` is the engine's (1-based, inclusive) copied rectangle; only its dims are read,
/// so the coordinate base is immaterial. (The fill copies values + styles exactly but shifts a
/// formula's refs uniformly, not per-cell — accepted limitation U2 in `GAPS.md`.)
pub(crate) fn fill_target_dims(
    source_range: (i32, i32, i32, i32),
    target: CellRange,
) -> Option<(u32, u32)> {
    let (sr0, sc0, sr1, sc1) = source_range;
    if sr1 < sr0 || sc1 < sc0 {
        return None; // degenerate source
    }
    let sw = (sc1 - sc0 + 1) as u32;
    let sh = (sr1 - sr0 + 1) as u32;
    let tw = target.width();
    let th = target.height();
    let exact = sw != 0 && sh != 0 && tw.is_multiple_of(sw) && th.is_multiple_of(sh);
    (exact && (tw > sw || th > sh)).then_some((tw, th))
}

/// Replicates a clipboard payload's cells to fill a `tw`×`th` block by tiling the `source_range`
/// rectangle across it (BUG 4 fill). `data_json` is the serialized [`ClipboardData`] —
/// `{ "<row>": { "<col>": { text, style } } }` with engine 1-based integer-as-string keys — and
/// the result has the same shape, every tile's cells cloned into place, ready to
/// `ClipboardData::deserialize`. `tw`/`th` are assumed whole multiples of the source dims (the
/// caller gates on [`fill_target_dims`]). Errors only on a malformed payload (non-object shape /
/// non-integer keys) — a defensive guard, since the payload always comes from the engine's own
/// `copy_to_clipboard`.
fn tile_clipboard_json(
    data_json: &serde_json::Value,
    source_range: (i32, i32, i32, i32),
    tw: u32,
    th: u32,
) -> Result<serde_json::Value, String> {
    let (sr0, sc0, sr1, sc1) = source_range;
    let sw = sc1 - sc0 + 1;
    let sh = sr1 - sr0 + 1;
    let src = data_json
        .as_object()
        .ok_or_else(|| "clipboard data is not an object".to_string())?;
    let reps_r = th as i32 / sh;
    let reps_c = tw as i32 / sw;
    let mut out = serde_json::Map::new();
    for a in 0..reps_r {
        for b in 0..reps_c {
            for (row_key, row_val) in src {
                let src_row: i32 = row_key
                    .parse()
                    .map_err(|_| format!("bad clipboard row key: {row_key}"))?;
                let row_obj = row_val
                    .as_object()
                    .ok_or_else(|| "clipboard row is not an object".to_string())?;
                let out_row = out
                    .entry((src_row + a * sh).to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                let out_row_obj = out_row.as_object_mut().expect("just inserted an object");
                for (col_key, cell_val) in row_obj {
                    let src_col: i32 = col_key
                        .parse()
                        .map_err(|_| format!("bad clipboard col key: {col_key}"))?;
                    out_row_obj.insert((src_col + b * sw).to_string(), cell_val.clone());
                }
            }
        }
    }
    Ok(serde_json::Value::Object(out))
}

/// Converts a 0-based [`CellRange`] on `sheet_idx` to IronCalc's 1-based inclusive
/// `Area { row, column, width, height }` (the shape its range-style / clear APIs take).
fn area_of(sheet_idx: u32, range: CellRange) -> Area {
    let (row, column) = to_engine_coords(range.start);
    Area {
        sheet: sheet_idx,
        row,
        column,
        width: (range.end.col - range.start.col) as i32 + 1,
        height: (range.end.row - range.start.row) as i32 + 1,
    }
}

/// Maps an `save_xlsx_to_writer` failure to a [`SaveError`]: an I/O failure writing the temp
/// file is a [`SaveError::Io`]; any other (structural) failure is a [`SaveError::Serialize`].
///
/// In practice, with a healthy model and a working temp file the pinned IronCalc writer only
/// fails on I/O to the temp file (already routed to `Io`). [`SaveError::Serialize`] is a
/// **defensive** path a malformed model would need — but the edit APIs (`set_user_input`,
/// `update_range_style`, …) prevent that state, so it is not reachably triggerable in a test
/// with the pinned API. It is kept so a future engine that surfaces a real serialization
/// error reports it distinctly rather than as a disk failure.
fn map_writer_error(err: ironcalc::error::XlsxError) -> SaveError {
    match err {
        ironcalc::error::XlsxError::IO(msg) => SaveError::Io(msg),
        other => SaveError::Serialize(other.to_string()),
    }
}

/// The directory a save's temp file is created in — the destination's parent, or the current
/// directory when `path` is a bare filename. Keeping the temp file beside the target
/// guarantees the final rename stays on one filesystem (atomic).
fn destination_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// A fresh temp file in `path`'s destination directory — the staging file for an atomic save
/// (same filesystem as the target, so the final rename is atomic).
fn new_temp_beside(path: &Path) -> Result<NamedTempFile, SaveError> {
    let dir = destination_dir(path);
    NamedTempFile::new_in(&dir).map_err(|e| {
        SaveError::Io(format!(
            "couldn't create a temporary file in {}: {e}",
            dir.display()
        ))
    })
}

/// Flushes `temp` to disk, then atomically renames it over `path`. On any failure `path` is left
/// untouched (`functional_spec.md §5.2`): the temp file (returned inside the error) drops and is
/// cleaned up. Shared by [`WorkbookDocument::save`] and [`write_xlsx_bytes_atomic`] so both save
/// paths keep the identical durability contract.
fn persist_atomically(temp: NamedTempFile, path: &Path) -> Result<(), SaveError> {
    // Flush data + metadata to disk BEFORE the rename makes the file visible at `path`.
    temp.as_file()
        .sync_all()
        .map_err(|e| SaveError::Io(e.to_string()))?;
    temp.persist(path)
        .map_err(|e| SaveError::Io(e.error.to_string()))?;
    Ok(())
}

/// Atomically writes pre-serialized `.xlsx` `bytes` to `path` (temp file beside the target +
/// fsync + rename) — the same durability contract as [`WorkbookDocument::save`], but for bytes
/// the chart-preserving save path has already assembled (IronCalc body + re-injected charts).
/// On any failure `path` is untouched.
pub(crate) fn write_xlsx_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let mut temp = new_temp_beside(path)?;
    temp.write_all(bytes)
        .map_err(|e| SaveError::Io(e.to_string()))?;
    temp.flush().map_err(|e| SaveError::Io(e.to_string()))?;
    persist_atomically(temp, path)
}

/// Classifies a file by its leading magic bytes so open failures can be typed precisely
/// (`XlsxError` alone can't distinguish not-xlsx / corrupt / password).
fn classify_magic(path: &Path) -> io::Result<FileKind> {
    const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

    let mut file = File::open(path)?;
    let mut head = [0u8; 8];
    let mut filled = 0;
    // A single `read` may return short even mid-file; fill up to 8 bytes robustly.
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    let head = &head[..filled];

    if head.starts_with(&OLE_MAGIC) {
        Ok(FileKind::Ole)
    } else if head.starts_with(b"PK") {
        // `PK` covers every Zip local/central/spanned header; an `.xlsx` is always `PK\x03\x04`.
        Ok(FileKind::Zip)
    } else {
        Ok(FileKind::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use std::fs;
    use tempfile::tempdir;

    /// End-to-end proof that FreeCell's IronCalc pin (`app/Cargo.toml`'s `[patch.crates-io]` →
    /// the fork's `freecell-fixes` branch) actually carries the scalar-functions batch: each
    /// function is set as a formula in A1 and its **computed, formatted** value is asserted. A
    /// name the pinned engine doesn't know would return `#NAME?` (and a broken impl a wrong value
    /// or `#VALUE!`/`#N/A`), so a literal-value match here is the regression guard that the batch
    /// is present and correct through the real FreeCell engine seam — no FreeCell-side code beyond
    /// the pin bump. Split into "presence" (the 9 functions verified already-present upstream) and
    /// "fixes" (the 4 fork correctness fixes this batch actually landed:
    /// `fix/trim-internal-runs`, `fix/dollar-negative-zero`, `fix/address-empty-sheet`,
    /// `fix/xmatch-array-constant` — see `specs/projects/scalar-functions-batch/fork-fixes/`).
    #[test]
    fn scalar_functions_batch_computes_through_pinned_engine() {
        // Set `formula` (a leading-`=` expression) into A1, evaluate, and return its formatted
        // display value — the exact per-cell text FreeCell would paint.
        fn eval(formula: &str) -> String {
            let mut doc = WorkbookDocument::new_empty().unwrap();
            doc.set_cell_input(0, CellRef::new(0, 0), formula).unwrap();
            doc.evaluate();
            doc.formatted_value(0, CellRef::new(0, 0)).unwrap()
        }

        // Presence: each function computes (not `#NAME?`) and returns its Excel value.
        let presence = [
            ("=SUMPRODUCT({1,2,3},{4,5,6})", "32"),
            ("=PROPER(\"john smith\")", "John Smith"),
            ("=REPLACE(\"abcdefg\",3,2,\"XY\")", "abXYefg"),
            ("=CHAR(65)", "A"),
            ("=CODE(\"A\")", "65"),
            ("=CLEAN(\"Hello\"&CHAR(7)&\"World\")", "HelloWorld"),
            ("=PERCENTILE.INC({1,2,3,4},0.5)", "2.5"),
            ("=QUARTILE.INC({1,2,4,7,8,9,10,12},2)", "7.5"),
            ("=XMATCH(30,{10,20,30,40,50})", "3"),
        ];
        // The four fork correctness fixes — prove the pin carries each landed branch.
        let fixes = [
            ("=TRIM(\"a    b\")", "a b"),           // fix/trim-internal-runs
            ("=DOLLAR(-0.001,2)", "$0.00"),         // fix/dollar-negative-zero
            ("=ADDRESS(1,1,1,TRUE,\"\")", "!$A$1"), // fix/address-empty-sheet
            ("=XMATCH(\"ban*\",{\"apple\",\"banana\",\"cherry\"},2)", "2"), // fix/xmatch-array-constant
        ];

        for (formula, expected) in presence.iter().chain(fixes.iter()) {
            assert_eq!(
                eval(formula),
                *expected,
                "pinned engine mis-evaluated {formula} (expected {expected:?})"
            );
        }
    }

    /// Every number-format preset code the dropdown can send must be renderable by the IronCalc
    /// formatter — a code the lexer can't parse renders `#VALUE!` in every cell (as bare `£`/`¥`
    /// did before they were switched to `[$£]…`/`[$¥]…`, and as the dropped `# ?/?` fraction did).
    /// This guards the whole `NUM_FMT_GROUPS` inventory — and any future preset — against that class
    /// of engine-incompatibility, which is otherwise invisible from FreeCell-side tests.
    #[test]
    fn every_num_fmt_preset_code_renders_without_parse_error() {
        use freecell_core::format_ui::NUM_FMT_GROUPS;
        let locale = get_locale("en").expect("en locale available");
        // Two universally-valid positive inputs (integer part ≥ 1 so date/time serials are real, and
        // fine for number/currency/percent/scientific). A *parse* error is value-independent — it
        // fires before any section is chosen — so a positive value reaches it just as a negative one
        // would, without the value-domain noise of a negative "date" (which the engine legitimately
        // rejects). The negative section of the multi-section numeric presets is exercised below.
        let sample_values = [1234.567_f64, 45283.75];
        for group in NUM_FMT_GROUPS {
            for preset in group.presets {
                for &v in &sample_values {
                    let out = format_number(v, preset.code, locale);
                    assert!(
                        out.error.is_none() && out.text != "#VALUE!",
                        "preset {:?} ({}) failed to render {v}: text={:?} error={:?}",
                        preset.label,
                        preset.code,
                        out.text,
                        out.error
                    );
                }
            }
        }
        // The negative section of the two multi-section *numeric* presets must also render (negatives
        // are valid for numbers, unlike dates): red-negative Number + parens-negative Accounting.
        for code in ["#,##0.00;[Red]-#,##0.00", "$#,##0.00;($#,##0.00)"] {
            let out = format_number(-1234.567, code, locale);
            assert!(
                out.error.is_none() && out.text != "#VALUE!",
                "negative render of {code}: text={:?} error={:?}",
                out.text,
                out.error
            );
        }
        // Spot-check the fix's intent: the bracketed `£`/`¥` presets actually emit their symbol
        // (not merely a non-error), and a plain currency still groups + shows two decimals.
        assert!(format_number(1234.5, "[$£]#,##0.00", locale)
            .text
            .contains('£'));
        assert!(format_number(1234.5, "[$¥]#,##0.00", locale)
            .text
            .contains('¥'));
        assert_eq!(format_number(1234.5, "$#,##0.00", locale).text, "$1,234.50");
    }

    #[test]
    fn destination_dir_uses_parent_or_cwd() {
        assert_eq!(
            destination_dir(Path::new("/tmp/books/a.xlsx")),
            PathBuf::from("/tmp/books")
        );
        // A bare filename has an empty parent → save beside it in the current directory.
        assert_eq!(destination_dir(Path::new("a.xlsx")), PathBuf::from("."));
    }

    /// NEGATIVE CONTROL for Phase 12's "zero engine calls on the scroll path" gate: a real
    /// model read/edit MUST bump `engine_call_count()`. If this ever stopped incrementing, the
    /// harness's zero-delta assertion across a scroll sweep would pass vacuously — this proves
    /// the counter can register engine work.
    #[test]
    fn engine_call_counter_registers_real_model_work() {
        let mut doc = WorkbookDocument::new_empty().unwrap();

        let before = crate::instrument::engine_call_count();
        // A pure read (formatted_value) is one engine call.
        let _ = doc.formatted_value(0, CellRef::new(0, 0)).unwrap();
        let after_read = crate::instrument::engine_call_count();
        assert!(
            after_read > before,
            "a formatted_value read must bump the engine-call counter"
        );

        // An edit + evaluate is more engine calls.
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.evaluate();
        let after_edit = crate::instrument::engine_call_count();
        assert!(
            after_edit > after_read,
            "a set_cell_input + evaluate must bump the counter further"
        );
    }

    #[test]
    fn to_engine_coords_is_one_based() {
        assert_eq!(to_engine_coords(CellRef::new(0, 0)), (1, 1)); // A1
        assert_eq!(to_engine_coords(CellRef::new(6, 1)), (7, 2)); // B7
    }

    // ---- CSV import / export (`functional_spec.md §2`, D2.2) -------------------------------

    /// Writes `content` to a fresh `.csv` in a temp dir and imports it, returning the doc and the
    /// dir (kept alive so the file isn't cleaned up mid-test).
    fn import_csv_str(content: &[u8]) -> (WorkbookDocument, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("in.csv");
        fs::write(&path, content).unwrap();
        let doc = WorkbookDocument::import_csv(&path).expect("import should succeed");
        (doc, dir)
    }

    fn cell_val(doc: &WorkbookDocument, row: u32, col: u32) -> CellData {
        doc.cell_value(0, CellRef::new(row, col))
    }

    #[test]
    fn import_csv_applies_fields_as_user_input() {
        // Numbers, booleans, formulas, and text each auto-type via `set_user_input`; a quoted field
        // carries an embedded comma and newline as ONE field; a ragged short row leaves trailing
        // cells empty.
        let csv = b"42,hello,TRUE\r\n=1+2,\"a, b\",\"line1\nline2\"\r\nx\r\n";
        let (doc, _dir) = import_csv_str(csv);

        assert_eq!(cell_val(&doc, 0, 0), CellData::Number(42.0));
        assert_eq!(cell_val(&doc, 0, 1), CellData::Text("hello".into()));
        assert_eq!(cell_val(&doc, 0, 2), CellData::Bool(true));
        // A leading `=` is a formula: its source is preserved and it computes.
        assert_eq!(doc.cell_content(0, CellRef::new(1, 0)).unwrap(), "=1+2");
        assert_eq!(cell_val(&doc, 1, 0), CellData::Number(3.0));
        assert_eq!(cell_val(&doc, 1, 1), CellData::Text("a, b".into()));
        assert_eq!(cell_val(&doc, 1, 2), CellData::Text("line1\nline2".into()));
        // Ragged row: A3 populated, B3/C3 empty.
        assert_eq!(cell_val(&doc, 2, 0), CellData::Text("x".into()));
        assert_eq!(cell_val(&doc, 2, 1), CellData::Empty);
        assert_eq!(doc.sheet_count(), 1);
    }

    #[test]
    fn import_csv_leaves_undo_history_empty() {
        // The import builds a raw `Model` wrapped with `UserModel::from_model`, so nothing lands on
        // the undo stack — the imported document is fresh (cross-cutting §Undo; guards against a
        // regression to applying inputs through `UserModel::set_user_input`, which would be undoable).
        let (doc, _dir) = import_csv_str(b"1,2\r\n=A1+B1,text\r\n");
        assert!(
            !doc.model.can_undo(),
            "an imported document's undo history starts empty (not dirty)"
        );
    }

    #[test]
    fn import_csv_strips_leading_bom() {
        // A UTF-8 BOM must not pollute A1.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hi,42\r\n");
        let (doc, _dir) = import_csv_str(&bytes);
        assert_eq!(cell_val(&doc, 0, 0), CellData::Text("hi".into()));
        assert_eq!(cell_val(&doc, 0, 1), CellData::Number(42.0));
    }

    #[test]
    fn import_csv_empty_file_yields_one_empty_sheet() {
        let (doc, _dir) = import_csv_str(b"");
        assert_eq!(doc.sheet_count(), 1);
        assert_eq!(cell_val(&doc, 0, 0), CellData::Empty);
    }

    #[test]
    fn import_csv_rejects_more_columns_than_max() {
        // One row with MAX_COLS + 1 fields (16,384 commas ⇒ 16,385 fields) → the col guard fires.
        let dir = tempdir().unwrap();
        let path = dir.path().join("wide.csv");
        let line = ",".repeat(freecell_core::limits::MAX_COLS as usize);
        fs::write(&path, line.as_bytes()).unwrap();
        assert!(matches!(
            WorkbookDocument::import_csv(&path),
            Err(LoadError::BadCsv(_))
        ));
    }

    #[test]
    fn import_csv_rejects_invalid_utf8() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.csv");
        // `0xFF` is never valid UTF-8.
        fs::write(&path, [0x41, 0xFF, 0x42]).unwrap();
        assert!(matches!(
            WorkbookDocument::import_csv(&path),
            Err(LoadError::BadCsv(_))
        ));
    }

    /// Exports sheet 0 of `doc` to a temp `.csv` and returns its raw bytes.
    fn export_csv_bytes(doc: &WorkbookDocument) -> Vec<u8> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.csv");
        doc.export_csv(0, &path).expect("export should succeed");
        fs::read(&path).unwrap()
    }

    #[test]
    fn export_csv_writes_raw_stored_values() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "0.5").unwrap(); // number (not 50%)
        doc.set_cell_input(0, CellRef::new(1, 0), "=1+2").unwrap(); // formula → computed value 3
        doc.set_cell_input(0, CellRef::new(2, 0), "TRUE").unwrap(); // boolean
        doc.set_cell_input(0, CellRef::new(3, 0), "hello, world")
            .unwrap(); // text with a comma → quoted
        doc.set_cell_input(0, CellRef::new(4, 0), "=1/0").unwrap(); // error string
        doc.set_cell_input(0, CellRef::new(5, 0), "2024-01-15")
            .unwrap(); // a date → serial number, not the formatted date
        doc.evaluate();

        let bytes = export_csv_bytes(&doc);
        let text = String::from_utf8(bytes).unwrap();
        // CRLF line endings (RFC 4180).
        assert!(text.contains("\r\n"), "CRLF terminators: {text:?}");
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines[0], "0.5", "raw number, not the 50% display");
        assert_eq!(lines[1], "3", "formula writes its computed value");
        assert_eq!(lines[2], "TRUE");
        assert_eq!(lines[3], "\"hello, world\"", "comma cell is quoted");
        assert_eq!(lines[4], "#DIV/0!", "error string verbatim");
        // The date exports as its serial number (a plain decimal), NOT the formatted date.
        assert_ne!(lines[5], "2024-01-15");
        assert!(
            lines[5].parse::<f64>().is_ok(),
            "date serial is a plain number, got {:?}",
            lines[5]
        );
    }

    #[test]
    fn export_csv_writes_text_that_looks_like_formula_verbatim() {
        // A genuine TEXT cell whose content looks like a formula/number (`'=1+1` — the apostrophe
        // forces literal text) exports as `=1+1` with NO leading apostrophe and NO re-interpretation.
        // Locks in the `String(s) => s` export path (unlike the paste-values `value_token`, which
        // apostrophe-quotes text).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "'=1+1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "'0123").unwrap(); // text that looks like a number
        doc.evaluate();

        let text = String::from_utf8(export_csv_bytes(&doc)).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(
            lines[0], "=1+1",
            "text-as-formula exports verbatim, no apostrophe"
        );
        assert_eq!(
            lines[1], "0123",
            "text-as-number exports verbatim, no apostrophe"
        );
    }

    #[test]
    fn export_csv_trims_trailing_empty_fields() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A1 and C1 populated (B1 empty); the used range is A1:C1, but B1 stays as an empty field
        // and no trailing comma follows C1.
        doc.set_cell_input(0, CellRef::new(0, 0), "a").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 2), "c").unwrap();
        doc.evaluate();
        let text = String::from_utf8(export_csv_bytes(&doc)).unwrap();
        assert_eq!(text, "a,,c\r\n");
    }

    #[test]
    fn export_csv_empty_sheet_writes_empty_file() {
        let doc = WorkbookDocument::new_empty().unwrap();
        assert!(
            export_csv_bytes(&doc).is_empty(),
            "an empty sheet exports a 0-byte file"
        );
    }

    #[test]
    fn export_then_import_round_trips_values() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2.5").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 0), "TRUE").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 1), "hello").unwrap();
        doc.evaluate();

        let dir = tempdir().unwrap();
        let path = dir.path().join("round.csv");
        doc.export_csv(0, &path).unwrap();
        let reimported = WorkbookDocument::import_csv(&path).unwrap();

        assert_eq!(cell_val(&reimported, 0, 0), CellData::Number(1.0));
        assert_eq!(cell_val(&reimported, 1, 0), CellData::Number(2.5));
        assert_eq!(cell_val(&reimported, 2, 0), CellData::Bool(true));
        assert_eq!(cell_val(&reimported, 0, 1), CellData::Text("hello".into()));
    }

    /// A cell's evaluated display string (`""` for empty), for the fill assertions below.
    fn value(doc: &WorkbookDocument, row: u32, col: u32) -> String {
        doc.formatted_value(0, CellRef::new(row, col)).unwrap()
    }

    #[test]
    fn fill_down_copies_top_row_not_series() {
        // ⌘D over A1:A5 with A1=1 must COPY (1,1,1,1,1) — NOT extrapolate a series (2,3,4,5).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.fill_down(0, CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0)))
            .unwrap();
        for r in 0..5 {
            assert_eq!(
                value(&doc, r, 0),
                "1",
                "A{} should be a copy of the seed",
                r + 1
            );
        }
    }

    #[test]
    fn fill_down_adjusts_relative_formula() {
        // A1="=B1"; B1..B5 = 10..50. ⌘D over A1:A5 copies the formula down with RELATIVE ref
        // adjustment → A2="=B2" … A5="=B5", evaluating to 20 … 50.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "=B1").unwrap();
        for (i, v) in [10, 20, 30, 40, 50].iter().enumerate() {
            doc.set_cell_input(0, CellRef::new(i as u32, 1), &v.to_string())
                .unwrap();
        }
        doc.fill_down(0, CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0)))
            .unwrap();
        assert_eq!(doc.cell_content(0, CellRef::new(1, 0)).unwrap(), "=B2");
        assert_eq!(doc.cell_content(0, CellRef::new(4, 0)).unwrap(), "=B5");
        assert_eq!(value(&doc, 1, 0), "20");
        assert_eq!(value(&doc, 4, 0), "50");
    }

    #[test]
    fn fill_down_multi_col_block_copies_each_column() {
        // A1=1, B1=2; ⌘D over A1:B3 fills each column from its OWN top cell.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 1), "2").unwrap();
        doc.fill_down(0, CellRange::new(CellRef::new(0, 0), CellRef::new(2, 1)))
            .unwrap();
        for r in 0..3 {
            assert_eq!(value(&doc, r, 0), "1");
            assert_eq!(value(&doc, r, 1), "2");
        }
    }

    #[test]
    fn fill_right_copies_left_column_not_series() {
        // ⌘R over A1:E1 with A1=7 copies 7 across (not a series).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "7").unwrap();
        doc.fill_right(0, CellRange::new(CellRef::new(0, 0), CellRef::new(0, 4)))
            .unwrap();
        for c in 0..5 {
            assert_eq!(value(&doc, 0, c), "7");
        }
    }

    #[test]
    fn fill_down_single_cell_pulls_from_above() {
        // ⌘D on a lone A2 copies the cell directly above (A1) into it (Excel D3.1).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "9").unwrap();
        doc.fill_down(0, CellRange::single(CellRef::new(1, 0)))
            .unwrap();
        assert_eq!(value(&doc, 1, 0), "9");
    }

    #[test]
    fn fill_right_single_cell_pulls_from_left() {
        // ⌘R on a lone B1 copies the cell directly to the left (A1) into it.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "9").unwrap();
        doc.fill_right(0, CellRange::single(CellRef::new(0, 1)))
            .unwrap();
        assert_eq!(value(&doc, 0, 1), "9");
    }

    #[test]
    fn fill_right_multi_row_block_copies_each_row() {
        // A1=1, A2=2; ⌘R over A1:C2 fills each row from its OWN left cell (column-path parity with
        // `fill_down_multi_col_block_copies_each_column`).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        assert!(doc
            .fill_right(0, CellRange::new(CellRef::new(0, 0), CellRef::new(1, 2)))
            .unwrap());
        for c in 0..3 {
            assert_eq!(value(&doc, 0, c), "1");
            assert_eq!(value(&doc, 1, c), "2");
        }
    }

    #[test]
    fn fill_single_cell_at_edge_is_noop() {
        // A lone cell with no neighbor in the fill direction (row 0 for ⌘D, col 0 for ⌘R) is a
        // clean no-op — returns `false` (skips eval/publish), no engine error, no u32 underflow
        // reading a -1 neighbor.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "5").unwrap();
        assert!(!doc
            .fill_down(0, CellRange::single(CellRef::new(0, 0)))
            .unwrap());
        assert!(!doc
            .fill_right(0, CellRange::single(CellRef::new(0, 0)))
            .unwrap());
        assert_eq!(value(&doc, 0, 0), "5");
    }

    #[test]
    fn fill_down_single_row_multi_col_is_noop() {
        // A single row wider than one cell has nothing below the seed line → ⌘D is a no-op.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 1), "2").unwrap();
        assert!(!doc
            .fill_down(0, CellRange::new(CellRef::new(0, 0), CellRef::new(0, 1)))
            .unwrap());
        assert_eq!(value(&doc, 0, 0), "1");
        assert_eq!(value(&doc, 0, 1), "2");
        assert_eq!(value(&doc, 1, 0), ""); // nothing filled below
        assert_eq!(value(&doc, 1, 1), "");
    }

    #[test]
    fn fill_right_single_col_multi_row_is_noop() {
        // A single column taller than one cell has nothing right of the seed line → ⌘R is a no-op
        // (column-path parity with `fill_down_single_row_multi_col_is_noop`).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        assert!(!doc
            .fill_right(0, CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)))
            .unwrap());
        assert_eq!(value(&doc, 0, 0), "1");
        assert_eq!(value(&doc, 1, 0), "2");
        assert_eq!(value(&doc, 0, 1), ""); // nothing filled to the right
        assert_eq!(value(&doc, 1, 1), "");
    }

    #[test]
    fn full_column_fill_down_clamps_to_used_range() {
        use freecell_core::limits;
        // A1=1 with data only in A1:A3; a header ⌘D over the WHOLE column (all 1,048,576 rows)
        // must clamp to the used-range extent (fill A2:A3), never write ~1M rows.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 0), "x").unwrap(); // used range extends to row 3
        let full_col = CellRange::new(CellRef::new(0, 0), CellRef::new(limits::MAX_ROWS - 1, 0));
        assert!(doc.fill_down(0, full_col).unwrap());
        // Filled down to the used-range bottom (A2, A3 copy the seed)…
        assert_eq!(value(&doc, 1, 0), "1");
        assert_eq!(value(&doc, 2, 0), "1");
        // …and NOT beyond it: the first row past the used range stays empty (no ~1M-cell write).
        assert_eq!(value(&doc, 3, 0), "");
        assert_eq!(value(&doc, 100, 0), "");
    }

    #[test]
    fn full_column_fill_down_on_unused_column_is_noop() {
        use freecell_core::limits;
        // A header ⌘D on a column entirely outside the used range clamps to an empty intersection →
        // a clean no-op (returns `false`), writing nothing.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap(); // used range = A1 only
        let far_col = CellRange::new(CellRef::new(0, 9), CellRef::new(limits::MAX_ROWS - 1, 9));
        assert!(!doc.fill_down(0, far_col).unwrap());
        assert_eq!(value(&doc, 0, 9), "");
    }

    #[test]
    fn fill_down_is_one_undo_step() {
        // One ⌘D pushes exactly ONE history entry: a single undo reverts the WHOLE fill.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.fill_down(0, CellRange::new(CellRef::new(0, 0), CellRef::new(2, 0)))
            .unwrap();
        assert_eq!(value(&doc, 2, 0), "1"); // A3 filled
        doc.undo().unwrap();
        doc.evaluate();
        assert_eq!(value(&doc, 1, 0), ""); // A2 reverted
        assert_eq!(value(&doc, 2, 0), ""); // A3 reverted by the SAME undo
        assert_eq!(value(&doc, 0, 0), "1"); // seed A1 intact
    }

    // ---- Drag-fill (`gaps_closing_7_15 §3`) — the full-seed series/copy path ----------------

    #[test]
    fn fill_drag_two_cell_numeric_seed_extrapolates_series_down() {
        // A1=1, A2=2. Drag the A1:A2 seed down to A5 → the fork's `detect_progression` extends
        // the arithmetic series (3, 4, 5) — the multi-cell-seed behavior ⌘D can't reach.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0));
        assert!(doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap());
        assert_eq!(value(&doc, 2, 0), "3");
        assert_eq!(value(&doc, 3, 0), "4");
        assert_eq!(value(&doc, 4, 0), "5");
    }

    #[test]
    fn fill_drag_single_cell_seed_copies_down() {
        // A single-cell seed has no progression → the fork copies (not a 7,8,9… series).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "7").unwrap();
        let seed = CellRange::single(CellRef::new(0, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(3, 0));
        assert!(doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap());
        for r in 0..4 {
            assert_eq!(value(&doc, r, 0), "7");
        }
    }

    #[test]
    fn fill_drag_single_cell_copies_relative_formula() {
        // A1="=B1"; B1..B4 = 10,20,30,40. A single-cell drag-fill copies the formula down with
        // RELATIVE adjustment → A2="=B2" … A4="=B4" (consistent with ⌘D).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "=B1").unwrap();
        for (i, v) in [10, 20, 30, 40].iter().enumerate() {
            doc.set_cell_input(0, CellRef::new(i as u32, 1), &v.to_string())
                .unwrap();
        }
        let seed = CellRange::single(CellRef::new(0, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(3, 0));
        assert!(doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap());
        assert_eq!(doc.cell_content(0, CellRef::new(1, 0)).unwrap(), "=B2");
        assert_eq!(doc.cell_content(0, CellRef::new(3, 0)).unwrap(), "=B4");
        assert_eq!(value(&doc, 3, 0), "40");
    }

    #[test]
    fn fill_drag_month_seed_extrapolates() {
        // A1="Jan", A2="Feb". Dragging the seed down extends the month sequence (Mar, Apr) via the
        // fork's text-sequence detection.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "Jan").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "Feb").unwrap();
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(3, 0));
        assert!(doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap());
        assert_eq!(value(&doc, 2, 0), "Mar");
        assert_eq!(value(&doc, 3, 0), "Apr");
    }

    #[test]
    fn fill_drag_up_reverses_series() {
        // Seed A4=3, A5=4. Dragging the A4:A5 seed UP to A1 counts the series DOWN (native fork
        // up-fill: A3=2, A2=1, A1=0) — the document method passes the target's top edge as `to_row`
        // (< the seed's first row) and the fork reverses.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(3, 0), "3").unwrap();
        doc.set_cell_input(0, CellRef::new(4, 0), "4").unwrap();
        let seed = CellRange::new(CellRef::new(3, 0), CellRef::new(4, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0));
        assert!(doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap());
        assert_eq!(value(&doc, 2, 0), "2");
        assert_eq!(value(&doc, 1, 0), "1");
        assert_eq!(value(&doc, 0, 0), "0");
    }

    #[test]
    fn fill_drag_horizontal_series_right() {
        // A1=1, B1=2. Dragging the A1:B1 seed right to E1 extends the series (3, 4, 5) along the
        // Horizontal axis (`auto_fill_columns`).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 1), "2").unwrap();
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 1));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 4));
        assert!(doc
            .fill_drag(0, seed, target, FillAxis::Horizontal)
            .unwrap());
        assert_eq!(value(&doc, 0, 2), "3");
        assert_eq!(value(&doc, 0, 3), "4");
        assert_eq!(value(&doc, 0, 4), "5");
    }

    #[test]
    fn fill_drag_is_one_undo_step() {
        // A series drag-fill pushes exactly ONE history entry: one undo reverts the whole fill.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0));
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0));
        doc.fill_drag(0, seed, target, FillAxis::Vertical).unwrap();
        assert_eq!(value(&doc, 4, 0), "5"); // A5 filled
        doc.undo().unwrap();
        doc.evaluate();
        assert_eq!(value(&doc, 2, 0), ""); // A3 reverted
        assert_eq!(value(&doc, 4, 0), ""); // A5 reverted by the SAME undo
        assert_eq!(value(&doc, 0, 0), "1"); // seed intact
        assert_eq!(value(&doc, 1, 0), "2");
    }

    #[test]
    fn fill_drag_no_extension_is_noop() {
        // A target that doesn't extend past the seed writes nothing → `false` (the caller skips
        // eval/publish/undo), matching a release back onto the seed.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "2").unwrap();
        let seed = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0));
        assert!(!doc.fill_drag(0, seed, seed, FillAxis::Vertical).unwrap());
    }

    #[test]
    fn published_style_maps_cell_kinds() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "42").unwrap(); // number
        doc.set_cell_input(0, CellRef::new(1, 0), "hello").unwrap(); // text
        doc.set_cell_input(0, CellRef::new(2, 0), "TRUE").unwrap(); // bool
        doc.set_cell_input(0, CellRef::new(3, 0), "=1/0").unwrap(); // error
        doc.set_cell_input(0, CellRef::new(4, 0), "2021-01-01")
            .unwrap(); // date (inferred fmt)
        doc.evaluate();

        let kind = |r| doc.published_style(0, CellRef::new(r, 0)).unwrap().0;
        assert_eq!(kind(0), CellKind::Number);
        assert_eq!(kind(1), CellKind::Text);
        assert_eq!(kind(2), CellKind::Bool);
        assert_eq!(kind(3), CellKind::Error);
        assert_eq!(kind(4), CellKind::Date, "a date-formatted number is Date");
    }

    #[test]
    fn published_style_resolves_format_and_explicit_colors() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = CellRef::new(0, 0);
        let range = CellRange::single(a1);
        // A currency format whose negative section is `[Red]`.
        doc.set_cell_input(0, a1, "-5").unwrap();
        doc.user_model_mut()
            .update_range_style(&area_of(0, range), "num_fmt", "$#,##0.00;[Red]$#,##0.00")
            .unwrap();
        doc.evaluate();
        assert_eq!(
            doc.published_style(0, a1).unwrap().1,
            Some(Rgb::new(0xFF, 0, 0)),
            "a negative value under a [Red] format publishes red text"
        );

        // A positive value uses the (colourless) positive section → no override.
        doc.set_cell_input(0, a1, "5").unwrap();
        doc.evaluate();
        assert_eq!(
            doc.published_style(0, a1).unwrap().1,
            None,
            "the positive section has no colour"
        );

        // An explicit non-black font colour wins over the format colour.
        doc.set_cell_input(0, a1, "-5").unwrap();
        doc.user_model_mut()
            .update_range_style(&area_of(0, range), "font.color", "#00AA00")
            .unwrap();
        doc.evaluate();
        assert_eq!(
            doc.published_style(0, a1).unwrap().1,
            Some(Rgb::new(0x00, 0xAA, 0x00)),
            "explicit font colour beats the number-format colour"
        );
    }

    #[test]
    fn sheet_properties_report_has_content() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A fresh workbook's only sheet is empty → has_content = false (delete-confirm gate off).
        let before = doc.sheet_properties_with_content();
        assert_eq!(before.len(), 1);
        assert!(!before[0].2, "an empty sheet reports has_content = false");

        // Writing a value populates `sheet_data` → has_content = true.
        doc.set_cell_input(0, CellRef::new(0, 0), "hello").unwrap();
        let after = doc.sheet_properties_with_content();
        assert!(
            after[0].2,
            "a sheet with a populated cell reports has_content = true"
        );
    }

    #[test]
    fn classify_magic_recognizes_containers() {
        let dir = tempdir().unwrap();

        let zipish = dir.path().join("z");
        fs::write(&zipish, b"PK\x03\x04rest").unwrap();
        assert_eq!(classify_magic(&zipish).unwrap(), FileKind::Zip);

        let ole = dir.path().join("o");
        fs::write(
            &ole,
            [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00],
        )
        .unwrap();
        assert_eq!(classify_magic(&ole).unwrap(), FileKind::Ole);

        let text = dir.path().join("t");
        fs::write(&text, b"hello, not a spreadsheet").unwrap();
        assert_eq!(classify_magic(&text).unwrap(), FileKind::Other);

        let empty = dir.path().join("e");
        fs::write(&empty, b"").unwrap();
        assert_eq!(classify_magic(&empty).unwrap(), FileKind::Other);
    }

    /// Bold / italic / underline / strikethrough / fill / font-color / wrap / vertical-align
    /// survive a save→reopen round-trip. In-crate (not an integration test) because it reads back
    /// the raw `ironcalc` `Style`.
    #[test]
    fn roundtrip_styles_preserved() {
        use ironcalc_base::types::VerticalAlignment;
        let dir = tempdir().unwrap();
        let path = dir.path().join("styles.xlsx");

        let doc = fixtures::styles();
        doc.save(&path).unwrap();
        let reopened = WorkbookDocument::open(&path).unwrap();

        // A1 bold, B1 italic, C1 underline (fixtures::styles layout).
        assert!(reopened.cell_style(0, CellRef::new(0, 0)).unwrap().font.b);
        assert!(reopened.cell_style(0, CellRef::new(0, 1)).unwrap().font.i);
        assert!(reopened.cell_style(0, CellRef::new(0, 2)).unwrap().font.u);
        // D1 strikethrough.
        assert!(
            reopened
                .cell_style(0, CellRef::new(0, 3))
                .unwrap()
                .font
                .strike
        );

        // A2 red fill, B2 blue font color.
        let fill = reopened.cell_style(0, CellRef::new(1, 0)).unwrap().fill;
        assert_eq!(
            fill.color,
            ironcalc_base::types::Color::Rgb("#FF0000".to_string())
        );
        let font = reopened.cell_style(0, CellRef::new(1, 1)).unwrap().font;
        assert_eq!(
            font.color,
            ironcalc_base::types::Color::Rgb("#0000FF".to_string())
        );

        // D2 wrap-text, D3 vertical alignment = top.
        let d2 = reopened.cell_style(0, CellRef::new(1, 3)).unwrap();
        assert!(
            d2.alignment.map(|a| a.wrap_text).unwrap_or(false),
            "wrap_text survives the round-trip"
        );
        let d3 = reopened.cell_style(0, CellRef::new(2, 3)).unwrap();
        assert_eq!(
            d3.alignment.map(|a| a.vertical),
            Some(VerticalAlignment::Top),
            "vertical alignment survives the round-trip"
        );
    }

    #[test]
    fn default_font_reads_workbook_default() {
        // A fresh workbook's default is IronCalc's `Font::default()` — now 12pt Inter (our fork
        // updated it from 13pt Calibri). This value only feeds the cache's "is this the default?"
        // detection; default cells render in bundled Inter (`GRID_FONT_FAMILY`) regardless.
        let doc = WorkbookDocument::new_empty().unwrap();
        let (sz, name) = doc.default_font();
        assert_eq!(sz, 12);
        assert_eq!(name, "Inter");
    }

    #[test]
    fn set_borders_applies_all_and_none_clears() {
        use ironcalc_base::types::BorderStyle;
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = CellRef::new(0, 0);
        doc.set_cell_input(0, a1, "x").unwrap();

        // "All" with a dashed-red pen sets all four edges with the requested style + colour.
        doc.set_borders(0, CellRange::single(a1), "All", "mediumdashed", "#FF0000")
            .unwrap();
        let b = doc.cell_style(0, a1).unwrap().border;
        assert!(
            b.top.is_some() && b.right.is_some() && b.bottom.is_some() && b.left.is_some(),
            "All applies every edge"
        );
        let top = b.top.as_ref().unwrap();
        assert_eq!(top.style, BorderStyle::MediumDashed, "pen style is written");
        assert_eq!(
            top.color.to_rgb(doc.workbook_theme()),
            "#FF0000",
            "pen colour is written"
        );

        // "None" clears them again.
        doc.set_borders(0, CellRange::single(a1), "None", "thin", "#000000")
            .unwrap();
        let b = doc.cell_style(0, a1).unwrap().border;
        assert!(
            b.top.is_none() && b.right.is_none() && b.bottom.is_none() && b.left.is_none(),
            "None clears every edge"
        );

        // A bogus tag is a clean error (never panics).
        assert!(doc
            .set_borders(0, CellRange::single(a1), "Bogus", "thin", "#000000")
            .is_err());
    }

    #[test]
    fn set_borders_outer_preserves_existing_interior_edges() {
        use ironcalc_base::types::BorderStyle;
        // Non-destructive per-type application (`architecture.md §0`): painting "Outer" over a cell
        // that already carries an inner edge must leave that inner edge intact.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A 2×2 block B2:C3 so "Inner" writes real interior edges.
        let block = CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2));
        doc.set_borders(0, block, "Inner", "thick", "#0000FF")
            .unwrap();
        // B2's right edge is an interior edge of the block.
        let b2 = CellRef::new(1, 1);
        let inner_right = doc.cell_style(0, b2).unwrap().border.right;
        assert!(
            inner_right.is_some(),
            "Inner wrote B2's interior right edge"
        );

        // Now paint the block's Outer perimeter with a different (thin) pen.
        doc.set_borders(0, block, "Outer", "thin", "#000000")
            .unwrap();
        let b = doc.cell_style(0, b2).unwrap().border;
        // The interior right edge is untouched (still thick blue) …
        let right = b.right.as_ref().expect("interior edge preserved");
        assert_eq!(
            right.style,
            BorderStyle::Thick,
            "interior edge survives Outer"
        );
        assert_eq!(right.color.to_rgb(doc.workbook_theme()), "#0000FF");
        // … while B2's top+left (its share of the perimeter) are the new thin pen.
        assert_eq!(b.top.as_ref().unwrap().style, BorderStyle::Thin);
        assert_eq!(b.left.as_ref().unwrap().style, BorderStyle::Thin);
    }

    #[test]
    fn set_font_applies_family_and_size_and_system_default_clears() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = CellRef::new(0, 0);
        let b2 = CellRef::new(1, 1);
        let range = CellRange::new(a1, b2);
        doc.set_cell_input(0, a1, "x").unwrap();
        doc.set_cell_input(0, b2, "y").unwrap();
        let (_, default_name) = doc.default_font();

        // Set Arial 20pt over A1:B2 (one on_paste_styles undo entry).
        doc.set_font(0, range, Some("Arial"), Some(20.0), &default_name)
            .unwrap();
        for cell in [a1, b2] {
            let style = doc.cell_style(0, cell).unwrap();
            assert_eq!(style.font.name, "Arial");
            assert_eq!(style.font.sz, 20);
        }

        // A size-only change leaves the family.
        doc.set_font(0, CellRange::single(a1), None, Some(9.0), &default_name)
            .unwrap();
        let style = doc.cell_style(0, a1).unwrap();
        assert_eq!(
            style.font.name, "Arial",
            "family untouched by a size-only set"
        );
        assert_eq!(style.font.sz, 9);

        // System Default (family = Some("")) resets the family to the workbook default.
        doc.set_font(0, CellRange::single(a1), Some(""), None, &default_name)
            .unwrap();
        assert_eq!(doc.cell_style(0, a1).unwrap().font.name, default_name);

        // One undo reverts the last font op (on_paste_styles is a single diff-list).
        doc.undo().unwrap();
        assert_eq!(doc.cell_style(0, a1).unwrap().font.name, "Arial");
    }

    #[test]
    fn clamp_to_used_clamps_bands_not_bounded() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "a").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 1), "b").unwrap(); // used range A1:B3
                                                                 // A full column clamps to the used rows.
        let full_col = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 0),
        );
        assert_eq!(
            doc.clamp_to_used(0, full_col).unwrap(),
            Some(CellRange::new(CellRef::new(0, 0), CellRef::new(2, 0)))
        );
        // A bounded selection is returned unchanged (applies even over empty cells).
        let bounded = CellRange::new(CellRef::new(5, 5), CellRef::new(7, 7));
        assert_eq!(doc.clamp_to_used(0, bounded).unwrap(), Some(bounded));
        // A full column beyond the used columns → empty intersection.
        let far_col = CellRange::new(
            CellRef::new(0, 9),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 9),
        );
        assert_eq!(doc.clamp_to_used(0, far_col).unwrap(), None);
    }

    // ---- Find / replace (`functional_spec.md §4`) -----------------------------------------

    #[test]
    fn find_matches_scans_populated_cells_row_major() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "apple").unwrap(); // A1
        doc.set_cell_input(0, CellRef::new(0, 2), "grape").unwrap(); // C1 (no "app")
        doc.set_cell_input(0, CellRef::new(1, 1), "Application")
            .unwrap(); // B2
        doc.set_cell_input(0, CellRef::new(2, 0), "pineapple")
            .unwrap(); // A3

        // Case-insensitive substring: A1, B2, A3 — returned in row-major (sheet_data) order.
        assert_eq!(
            doc.find_matches(0, "app", false, false).unwrap(),
            vec![CellRef::new(0, 0), CellRef::new(1, 1), CellRef::new(2, 0)]
        );
        // Case-sensitive "App" only hits "Application".
        assert_eq!(
            doc.find_matches(0, "App", true, false).unwrap(),
            vec![CellRef::new(1, 1)]
        );
        // Whole-cell "apple" (case-insensitive) hits only the exact A1.
        assert_eq!(
            doc.find_matches(0, "apple", false, true).unwrap(),
            vec![CellRef::new(0, 0)]
        );
        // An empty query never matches.
        assert!(doc.find_matches(0, "", false, false).unwrap().is_empty());
    }

    #[test]
    fn find_matches_targets_formula_text() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap(); // A1
        doc.set_cell_input(0, CellRef::new(1, 0), "=A1+1").unwrap(); // A2 (formula)
                                                                     // "A1" matches the formula TEXT of A2 (Excel "Look in: Formulas").
        assert_eq!(
            doc.find_matches(0, "A1", true, false).unwrap(),
            vec![CellRef::new(1, 0)]
        );
    }

    // ---- Selection stats (`functional_spec.md §1`) ----------------------------------------

    #[test]
    fn selection_stats_aggregates_a_mixed_selection() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A1:A5 — number, text, boolean, error, blank-with-content-then-cleared.
        doc.set_cell_input(0, CellRef::new(0, 0), "10").unwrap(); // number
        doc.set_cell_input(0, CellRef::new(1, 0), "hello").unwrap(); // text
        doc.set_cell_input(0, CellRef::new(2, 0), "TRUE").unwrap(); // boolean
        doc.set_cell_input(0, CellRef::new(3, 0), "=1/0").unwrap(); // error
        doc.set_cell_input(0, CellRef::new(4, 0), "20").unwrap(); // number
        doc.evaluate();

        let range = CellRange::new(CellRef::new(0, 0), CellRef::new(4, 0));
        let stats = doc.selection_stats(0, range);
        // Count = every non-empty cell (number, text, bool, error, number) = 5.
        assert_eq!(stats.count, 5, "count is every non-empty cell");
        // Only the two numbers are numeric — text/bool/error excluded from the math (D1.1).
        assert_eq!(stats.numeric_count, 2);
        assert_eq!(stats.sum, 30.0);
        assert_eq!(stats.min, Some(10.0));
        assert_eq!(stats.max, Some(20.0));
        assert_eq!(stats.average(), Some(15.0));
        assert!(stats.has_numeric());
    }

    #[test]
    fn selection_stats_full_column_walks_only_populated_cells() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Two numbers far apart in column A; the rest of the 1M-row column is empty.
        doc.set_cell_input(0, CellRef::new(0, 0), "5").unwrap(); // A1
        doc.set_cell_input(0, CellRef::new(500_000, 0), "7")
            .unwrap(); // A500001
        doc.set_cell_input(0, CellRef::new(0, 1), "999").unwrap(); // B1 — outside the column
        doc.evaluate();

        // A full-column selection of column A (every row) must aggregate only the two populated A
        // cells, not touch column B, and not iterate the empty rows.
        let full_col_a = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 0),
        );
        let stats = doc.selection_stats(0, full_col_a);
        assert_eq!(stats.count, 2);
        assert_eq!(stats.numeric_count, 2);
        assert_eq!(stats.sum, 12.0);
        assert_eq!(stats.min, Some(5.0));
        assert_eq!(stats.max, Some(7.0));
    }

    #[test]
    fn selection_stats_all_text_has_no_numeric() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "a").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "b").unwrap();
        doc.evaluate();
        let stats = doc.selection_stats(0, CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)));
        assert_eq!(stats.count, 2);
        assert!(
            !stats.has_numeric(),
            "an all-text selection shows no numbers"
        );
        assert_eq!(stats.sum, 0.0);
        assert_eq!(stats.min, None);
    }

    #[test]
    fn selection_stats_empty_range_is_empty() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "1").unwrap();
        doc.evaluate();
        // A selection over cells that are all blank aggregates to nothing.
        let stats = doc.selection_stats(0, CellRange::new(CellRef::new(5, 5), CellRef::new(9, 9)));
        assert_eq!(stats, SelectionStats::EMPTY);
    }

    #[test]
    fn resolve_edge_walks_populated_cells_in_every_direction() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Column A: a run A1:A3, a gap A4:A5, then A6; the rest of the column empty.
        for row in [0, 1, 2, 5] {
            doc.set_cell_input(0, CellRef::new(row, 0), "x").unwrap();
        }
        // Row 10 (index 9): a run at C..D (cols 2,3), then a gap, then F (col 5).
        for col in [2, 3, 5] {
            doc.set_cell_input(0, CellRef::new(9, col), "y").unwrap();
        }
        doc.evaluate();

        // Down from A1 (in the run) → last of the run A3 (row 2).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(0, 0), Direction::Down),
            CellRef::new(2, 0)
        );
        // Down from A3 (run's last, gap below) → cross the gap to A6 (row 5).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(2, 0), Direction::Down),
            CellRef::new(5, 0)
        );
        // Down from A6 (nothing below) → the sheet's last row.
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(5, 0), Direction::Down),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 0)
        );
        // Up from A6 → across the gap to the top run's last cell A3 (row 2).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(5, 0), Direction::Up),
            CellRef::new(2, 0)
        );
        // Horizontal: Right from C10 (col 2, in the run) → D10 (col 3, run's last before the gap).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(9, 2), Direction::Right),
            CellRef::new(9, 3)
        );
        // Right from D10 → cross the gap to F10 (col 5).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(9, 3), Direction::Right),
            CellRef::new(9, 5)
        );
    }

    #[test]
    fn resolve_edge_empty_sheet_goes_to_sheet_edge() {
        let doc = WorkbookDocument::new_empty().unwrap();
        // No data anywhere: ⌘+Down from a middle cell lands on the last row (Excel sheet-edge fallback).
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(5, 5), Direction::Down),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 5)
        );
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(5, 5), Direction::Right),
            CellRef::new(5, freecell_core::limits::MAX_COLS - 1)
        );
    }

    #[test]
    fn resolve_edge_from_empty_cell_jumps_to_next_data() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // A single populated cell far down column A; jumping down from the empty top lands on it.
        doc.set_cell_input(0, CellRef::new(200, 0), "here").unwrap();
        doc.evaluate();
        assert_eq!(
            doc.resolve_edge(0, CellRef::new(0, 0), Direction::Down),
            CellRef::new(200, 0)
        );
    }

    #[test]
    fn replace_one_rewrites_only_a_matching_cell() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "foobar").unwrap();
        doc.set_cell_input(0, CellRef::new(0, 1), "baz").unwrap();

        assert!(doc
            .replace_one(0, CellRef::new(0, 0), "foo", "qux", true, false)
            .unwrap());
        assert_eq!(doc.cell_content(0, CellRef::new(0, 0)).unwrap(), "quxbar");
        // A non-matching cell is untouched (no write, returns false).
        assert!(!doc
            .replace_one(0, CellRef::new(0, 1), "foo", "qux", true, false)
            .unwrap());
        assert_eq!(doc.cell_content(0, CellRef::new(0, 1)).unwrap(), "baz");
    }

    #[test]
    fn replace_all_replaces_every_match_and_single_undo_target() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "cat").unwrap(); // A1
        doc.set_cell_input(0, CellRef::new(0, 1), "cats").unwrap(); // B1
        doc.set_cell_input(0, CellRef::new(1, 0), "dog").unwrap(); // A2 (no match)

        let changed = doc
            .replace_all_matches(0, "cat", "dog", true, false)
            .unwrap();
        assert_eq!(changed, vec![CellRef::new(0, 0), CellRef::new(0, 1)]);
        assert_eq!(doc.cell_content(0, CellRef::new(0, 0)).unwrap(), "dog");
        assert_eq!(doc.cell_content(0, CellRef::new(0, 1)).unwrap(), "dogs");
        assert_eq!(doc.cell_content(0, CellRef::new(1, 0)).unwrap(), "dog");

        // The whole Replace All is a SINGLE engine undo entry (the fork's batched `set_user_inputs`
        // — `phase_plans/phase_9.md`), so ONE `doc.undo()` restores every replaced cell.
        doc.undo().unwrap();
        assert_eq!(doc.cell_content(0, CellRef::new(0, 0)).unwrap(), "cat");
        assert_eq!(doc.cell_content(0, CellRef::new(0, 1)).unwrap(), "cats");
        // The unmatched cell was never touched.
        assert_eq!(doc.cell_content(0, CellRef::new(1, 0)).unwrap(), "dog");
    }

    // ---- Range clipboard (`components/clipboard.md`) --------------------------------------

    fn cell(row: u32, col: u32) -> CellRef {
        CellRef::new(row, col)
    }

    /// Copy `range`, then paste its payload at `anchor` on `dest_idx` (copy semantics unless
    /// `cut`). Returns the pasted 0-based range the engine re-selected.
    fn copy_then_paste(
        doc: &mut WorkbookDocument,
        src_idx: u32,
        range: CellRange,
        dest_idx: u32,
        anchor: CellRef,
        cut: bool,
    ) -> CellRange {
        let copied = doc.copy_range(src_idx, range).unwrap();
        // Paste once at the anchor (no fill): the target is the single anchor cell.
        doc.paste_clipboard(
            dest_idx,
            src_idx,
            copied.range,
            &copied.data,
            cut,
            CellRange::single(anchor),
        )
        .unwrap();
        doc.evaluate();
        doc.selected_range_0based()
    }

    #[test]
    fn copy_paste_roundtrips_values_and_styles() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = cell(0, 0);
        doc.set_cell_input(0, a1, "42").unwrap();
        doc.set_font_flag(0, CellRange::single(a1), FontFlag::Bold, true)
            .unwrap();
        doc.set_fill(0, CellRange::single(a1), Some(Rgb::new(0xFF, 0, 0)))
            .unwrap();
        doc.evaluate();

        let pasted = copy_then_paste(&mut doc, 0, CellRange::single(a1), 0, cell(0, 2), false);
        assert_eq!(pasted, CellRange::single(cell(0, 2)));
        // Value + both styles arrive at C1.
        assert_eq!(doc.formatted_value(0, cell(0, 2)).unwrap(), "42");
        let style = doc.cell_style(0, cell(0, 2)).unwrap();
        assert!(style.font.b, "bold copied");
        assert_eq!(
            style.fill.color,
            ironcalc_base::types::Color::Rgb("#FF0000".to_string()),
            "fill copied"
        );
    }

    #[test]
    fn single_cell_copy_fills_a_larger_target_with_value_and_style() {
        // BUG 4: a 1×1 copy pasted into a 3×3 target fills all nine cells (value + style).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = cell(0, 0);
        doc.set_cell_input(0, a1, "7").unwrap();
        doc.set_font_flag(0, CellRange::single(a1), FontFlag::Bold, true)
            .unwrap();
        doc.set_fill(0, CellRange::single(a1), Some(Rgb::new(0xFF, 0, 0)))
            .unwrap();
        doc.evaluate();

        let copied = doc.copy_range(0, CellRange::single(a1)).unwrap();
        // Target C1:E3 (rows 0..=2, cols 2..=4) — a 3×3 block, anchor at C1.
        let target = CellRange::new(cell(0, 2), cell(2, 4));
        doc.paste_clipboard(0, 0, copied.range, &copied.data, false, target)
            .unwrap();
        doc.evaluate();

        for r in 0..3 {
            for c in 2..5 {
                assert_eq!(
                    doc.formatted_value(0, cell(r, c)).unwrap(),
                    "7",
                    "cell ({r},{c}) should be filled with the copied value"
                );
                let style = doc.cell_style(0, cell(r, c)).unwrap();
                assert!(style.font.b, "cell ({r},{c}) should be bold");
                assert_eq!(
                    style.fill.color,
                    ironcalc_base::types::Color::Rgb("#FF0000".to_string()),
                    "cell ({r},{c}) should carry the copied fill"
                );
            }
        }
        // The engine re-selected the whole filled block.
        assert_eq!(doc.selected_range_0based(), target);
    }

    #[test]
    fn copy_paste_adjusts_relative_but_not_absolute_refs() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "10").unwrap(); // A1
        doc.set_cell_input(0, cell(1, 0), "=A1").unwrap(); // A2 (relative)
        doc.set_cell_input(0, cell(1, 1), "=$A$1").unwrap(); // B2 (absolute)
        doc.evaluate();

        // Copy A2 one row down → A3 should reference A2 (relative shift).
        copy_then_paste(
            &mut doc,
            0,
            CellRange::single(cell(1, 0)),
            0,
            cell(2, 0),
            false,
        );
        assert_eq!(doc.cell_content(0, cell(2, 0)).unwrap(), "=A2");
        // Copy B2 one row down → B3 keeps the absolute reference.
        copy_then_paste(
            &mut doc,
            0,
            CellRange::single(cell(1, 1)),
            0,
            cell(2, 1),
            false,
        );
        assert_eq!(doc.cell_content(0, cell(2, 1)).unwrap(), "=$A$1");
    }

    // ---- Paste Values (⌘⇧V, `functional_spec.md §5`) --------------------------------------

    /// Give the single cell `c` on sheet 0 the number format `code`.
    fn set_num_fmt(doc: &mut WorkbookDocument, c: CellRef, code: &str) {
        doc.user_model_mut()
            .update_range_style(&area_of(0, CellRange::single(c)), "num_fmt", code)
            .unwrap();
    }

    #[test]
    fn paste_values_writes_the_formula_result_not_the_formula_and_keeps_target_format() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = cell(0, 0);
        doc.set_cell_input(0, a1, "=1+2").unwrap(); // evaluates to 3
        doc.evaluate();
        let copied = doc.copy_range(0, CellRange::single(a1)).unwrap();
        assert_eq!(
            copied.values,
            vec![vec!["3".to_string()]],
            "computed value snapshot"
        );

        // The target C1 carries its own 2-decimal format, which paste-values must preserve.
        let c1 = cell(0, 2);
        set_num_fmt(&mut doc, c1, "0.00");
        doc.paste_values(0, c1, &copied.values, 1, 1).unwrap();
        doc.evaluate();

        // The literal value 3 landed (not the formula) and the target kept its own format.
        assert_eq!(
            doc.cell_content(0, c1).unwrap(),
            "3",
            "the value, not `=1+2`"
        );
        assert_eq!(doc.published_style(0, c1).unwrap().0, CellKind::Number);
        assert_eq!(
            doc.formatted_value(0, c1).unwrap(),
            "3.00",
            "the target's own 0.00 format is kept — paste-values applies no formatting"
        );
        // The source formula is untouched.
        assert_eq!(doc.cell_content(0, a1).unwrap(), "=1+2");
    }

    #[test]
    fn value_tokens_force_every_formula_looking_leading_char_literal() {
        // Every leading char that a spreadsheet may treat as a formula (`=`, `+`, `-`, `@`) must be
        // apostrophe-forced when it is a *string* value, while a real number/boolean stays bare.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        for (col, text) in ["=x", "+x", "-x", "@x"].iter().enumerate() {
            doc.set_cell_input(0, cell(0, col as u32), &format!("'{text}"))
                .unwrap(); // a literal text value like the string "=x"
        }
        doc.set_cell_input(0, cell(0, 4), "-5").unwrap(); // a real negative number
        doc.set_cell_input(0, cell(0, 5), "=TRUE").unwrap(); // a boolean
        doc.evaluate();

        let copied = doc
            .copy_range(0, CellRange::new(cell(0, 0), cell(0, 5)))
            .unwrap();
        assert_eq!(
            copied.values,
            vec![vec![
                "'=x".to_string(),
                "'+x".to_string(),
                "'-x".to_string(),
                "'@x".to_string(),
                "-5".to_string(), // a real number is a bare token (no quote)
                "TRUE".to_string(),
            ]],
        );
    }

    #[test]
    fn paste_values_forces_a_formula_looking_string_to_a_literal() {
        // A computed *string* value of `=x` must paste as literal text, never re-parsed as a
        // formula (which would evaluate `x` to an error). The load-bearing edge case.
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = cell(0, 0);
        doc.set_cell_input(0, a1, "=\"=x\"").unwrap(); // a formula whose *value* is the string "=x"
        doc.evaluate();
        assert_eq!(doc.formatted_value(0, a1).unwrap(), "=x");
        let copied = doc.copy_range(0, CellRange::single(a1)).unwrap();
        assert_eq!(
            copied.values,
            vec![vec!["'=x".to_string()]],
            "the string token is apostrophe-forced to a literal"
        );

        let c1 = cell(0, 2);
        doc.paste_values(0, c1, &copied.values, 1, 1).unwrap();
        doc.evaluate();
        assert_eq!(
            doc.formatted_value(0, c1).unwrap(),
            "=x",
            "the literal text `=x` landed — NOT a re-interpreted formula"
        );
        assert_eq!(
            doc.published_style(0, c1).unwrap().0,
            CellKind::Text,
            "the pasted cell is text, not a formula/error"
        );
    }

    #[test]
    fn paste_values_preserves_number_vs_text_typing() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let num = cell(0, 0); // a real number
        let txt = cell(0, 1); // text that merely looks numeric
        doc.set_cell_input(0, num, "42").unwrap();
        doc.set_cell_input(0, txt, "'12").unwrap(); // leading apostrophe → the text "12"
        doc.evaluate();

        let copied = doc.copy_range(0, CellRange::new(num, txt)).unwrap();
        assert_eq!(
            copied.values,
            vec![vec!["42".to_string(), "'12".to_string()]],
            "a number stays a bare number token; text is force-quoted"
        );

        let dest = cell(2, 0);
        doc.paste_values(0, dest, &copied.values, 2, 1).unwrap();
        doc.evaluate();
        // The numeric value pastes as a number…
        assert_eq!(doc.published_style(0, dest).unwrap().0, CellKind::Number);
        assert_eq!(doc.formatted_value(0, dest).unwrap(), "42");
        // …and the numeric-looking text stays text.
        assert_eq!(
            doc.published_style(0, cell(2, 1)).unwrap().0,
            CellKind::Text
        );
        assert_eq!(doc.formatted_value(0, cell(2, 1)).unwrap(), "12");
    }

    #[test]
    fn paste_values_round_trips_booleans_and_errors() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let b = cell(0, 0);
        let e = cell(0, 1);
        doc.set_cell_input(0, b, "=TRUE").unwrap();
        doc.set_cell_input(0, e, "=1/0").unwrap(); // #DIV/0!
        doc.evaluate();
        let copied = doc.copy_range(0, CellRange::new(b, e)).unwrap();
        assert_eq!(
            copied.values,
            vec![vec!["TRUE".to_string(), "#DIV/0!".to_string()]],
        );

        let dest = cell(2, 0);
        doc.paste_values(0, dest, &copied.values, 2, 1).unwrap();
        doc.evaluate();
        assert_eq!(doc.published_style(0, dest).unwrap().0, CellKind::Bool);
        assert_eq!(doc.formatted_value(0, dest).unwrap(), "TRUE");
        assert_eq!(
            doc.published_style(0, cell(2, 1)).unwrap().0,
            CellKind::Error
        );
        assert_eq!(doc.formatted_value(0, cell(2, 1)).unwrap(), "#DIV/0!");
    }

    #[test]
    fn paste_values_tiles_a_single_cell_over_a_larger_block_and_clears_blanks() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "5").unwrap();
        doc.evaluate();
        let copied = doc.copy_range(0, CellRange::single(cell(0, 0))).unwrap();

        // Pre-seed a target cell so we can prove a blank source token would clear it (here the
        // single-cell fill writes 5 everywhere, so all four land).
        let target = CellRange::new(cell(0, 2), cell(1, 3)); // C1:D2, a 2×2 block
        doc.paste_values(0, target.start, &copied.values, 2, 2)
            .unwrap();
        doc.evaluate();
        for r in 0..2 {
            for c in 2..4 {
                assert_eq!(doc.formatted_value(0, cell(r, c)).unwrap(), "5");
            }
        }
        assert_eq!(
            doc.selected_range_0based(),
            target,
            "the filled block is re-selected"
        );
    }

    #[test]
    fn number_token_uses_the_locale_decimal_separator_without_grouping() {
        // `.` locale: Rust's own repr, full precision. (`1.25` avoids clippy's `approx_constant`
        // deny that a `3.14…`-shaped literal trips — any exact non-integer decimal exercises the
        // separator swap identically.)
        assert_eq!(number_token(1.25, '.'), "1.25");
        assert_eq!(number_token(1234567.5, '.'), "1234567.5"); // never grouped
        assert_eq!(number_token(42.0, '.'), "42"); // an integer has no decimal point
                                                   // `,` locale (German): the decimal point becomes `,`; still no grouping.
        assert_eq!(number_token(1.25, ','), "1,25");
        assert_eq!(number_token(1234567.5, ','), "1234567,5");
        assert_eq!(number_token(42.0, ','), "42"); // integer unaffected by the separator
    }

    #[test]
    fn paste_values_number_survives_a_non_dot_decimal_locale() {
        // Moderate CR fix: under a German-locale workbook (decimal `,`, group `.`) a fractional
        // computed number must still paste as a NUMBER — the token carries the workbook locale's
        // decimal separator, so the write path parses it as a number instead of falling to text.
        let model = ironcalc_base::Model::new_empty("de-book", "de", "UTC", "en").unwrap();
        let mut doc = WorkbookDocument::from_test_model(model);
        let a1 = cell(0, 0);
        doc.set_cell_input(0, a1, "=1/4").unwrap(); // 0.25
        doc.evaluate();
        let copied = doc.copy_range(0, CellRange::single(a1)).unwrap();
        assert_eq!(
            copied.values,
            vec![vec!["0,25".to_string()]],
            "the number token uses the workbook locale's decimal separator"
        );

        let c1 = cell(0, 2);
        doc.paste_values(0, c1, &copied.values, 1, 1).unwrap();
        doc.evaluate();
        assert_eq!(
            doc.published_style(0, c1).unwrap().0,
            CellKind::Number,
            "the fractional value pastes as a number, not text, under a non-`.`-decimal locale"
        );
    }

    #[test]
    fn paste_values_empty_string_value_clears_the_target_not_a_counted_blank() {
        // Mild CR fix: an empty-string computed value (`=""`) must CLEAR the target (a true blank
        // that the selection-stats Count ignores), not write a quoted empty text cell (which would
        // count as non-empty).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "=\"\"").unwrap(); // a formula whose value is the string ""
        doc.evaluate();
        let copied = doc.copy_range(0, CellRange::single(cell(0, 0))).unwrap();
        assert_eq!(
            copied.values,
            vec![vec![String::new()]],
            "an empty-string value renders as a clearing token"
        );

        // Pre-seed the target so we can prove the paste CLEARS it.
        let c1 = cell(0, 2);
        doc.set_cell_input(0, c1, "old").unwrap();
        doc.evaluate();
        doc.paste_values(0, c1, &copied.values, 1, 1).unwrap();
        doc.evaluate();
        assert_eq!(doc.formatted_value(0, c1).unwrap(), "");
        // A cleared cell is not counted; a quoted empty-text cell would count as 1.
        assert_eq!(
            doc.selection_stats(0, CellRange::single(c1)).count,
            0,
            "the target is a true blank, not a counted empty-text cell"
        );
    }

    #[test]
    fn cut_paste_moves_value_and_clears_source() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "5").unwrap(); // A1
        doc.set_cell_input(0, cell(1, 0), "=A1+1").unwrap(); // A2, a formula that moves with A1
        doc.evaluate();

        // Cut A1:A2 to C1 (so the internal reference A2→A1 stays within the moved block).
        copy_then_paste(
            &mut doc,
            0,
            CellRange::new(cell(0, 0), cell(1, 0)),
            0,
            cell(0, 2),
            true,
        );
        // The block moved: C1 = 5, C2 = "=C1+1" (the intra-block reference followed the move).
        assert_eq!(doc.formatted_value(0, cell(0, 2)).unwrap(), "5");
        assert_eq!(doc.cell_content(0, cell(1, 2)).unwrap(), "=C1+1");
        assert_eq!(doc.formatted_value(0, cell(1, 2)).unwrap(), "6");
        // The source cells are cleared.
        assert_eq!(
            doc.formatted_value(0, cell(0, 0)).unwrap(),
            "",
            "A1 cleared"
        );
        assert_eq!(
            doc.formatted_value(0, cell(1, 0)).unwrap(),
            "",
            "A2 cleared"
        );
    }

    #[test]
    fn full_column_copy_clamps_to_used_range() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "1").unwrap();
        doc.set_cell_input(0, cell(1, 0), "2").unwrap();
        doc.evaluate();
        // Copy the entire column A (Excel-max rows) — the engine clamps to `dimension()`.
        let full_col = CellRange::new(cell(0, 0), cell(freecell_core::limits::MAX_ROWS - 1, 0));
        let copied = doc.copy_range(0, full_col).unwrap();
        // The effective source range is 1-based rows 1..=2, NOT the whole column.
        assert_eq!(copied.range, (1, 1, 2, 1), "copy clamped to the used range");
    }

    #[test]
    fn cross_sheet_internal_paste() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, cell(0, 0), "9").unwrap();
        doc.add_sheet().unwrap(); // sheet index 1
        doc.evaluate();

        let copied = doc.copy_range(0, CellRange::single(cell(0, 0))).unwrap();
        doc.paste_clipboard(
            1,
            0,
            copied.range,
            &copied.data,
            false,
            CellRange::single(cell(3, 3)),
        )
        .unwrap();
        doc.evaluate();
        assert_eq!(doc.formatted_value(1, cell(3, 3)).unwrap(), "9");
        // The source sheet is untouched (copy, not cut).
        assert_eq!(doc.formatted_value(0, cell(0, 0)).unwrap(), "9");
    }

    #[test]
    fn paste_tsv_writes_dims_and_types() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.paste_tsv(0, cell(0, 0), "1\t2\n=A1\ttrue\n").unwrap();
        doc.evaluate();
        assert_eq!(doc.formatted_value(0, cell(0, 0)).unwrap(), "1");
        assert_eq!(doc.formatted_value(0, cell(0, 1)).unwrap(), "2");
        // A2 got the literal "=A1" (paste is user-input; no ref adjustment) → evaluates to A1.
        assert_eq!(doc.cell_content(0, cell(1, 0)).unwrap(), "=A1");
        assert_eq!(doc.formatted_value(0, cell(1, 0)).unwrap(), "1");
        assert_eq!(doc.formatted_value(0, cell(1, 1)).unwrap(), "TRUE");
    }

    #[test]
    fn paste_tsv_tolerates_crlf_and_drops_ragged_rows() {
        // CRLF-terminated, equal-width rows all land (each `\r\n` is one record terminator).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.paste_tsv(0, cell(0, 0), "1\t2\r\n3\t4\r\n").unwrap();
        doc.evaluate();
        assert_eq!(doc.formatted_value(0, cell(0, 0)).unwrap(), "1");
        assert_eq!(doc.formatted_value(0, cell(0, 1)).unwrap(), "2");
        assert_eq!(doc.formatted_value(0, cell(1, 0)).unwrap(), "3");
        assert_eq!(doc.formatted_value(0, cell(1, 1)).unwrap(), "4");

        // A ragged (narrower) middle row is DROPPED and later rows COMPACT up: with
        // `flexible = false` the csv reader errors on the odd-width record and `paste_csv_string`
        // skips it *without advancing the row*, so the wide row after it lands one row early.
        // (DECISION #5 — accepted engine behaviour; NOT the "pad ⇒ skipped cells" of empty
        // tokens within an equal-width row.)
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.paste_tsv(0, cell(0, 0), "a\tb\nc\nd\te\n").unwrap();
        doc.evaluate();
        assert_eq!(doc.formatted_value(0, cell(0, 0)).unwrap(), "a");
        assert_eq!(doc.formatted_value(0, cell(0, 1)).unwrap(), "b");
        // The ragged "c" row vanished — its cell is empty, not written…
        assert_eq!(
            doc.formatted_value(0, cell(1, 0)).unwrap(),
            "d",
            "the wide row compacts up into the dropped ragged row's slot"
        );
        assert_eq!(doc.formatted_value(0, cell(1, 1)).unwrap(), "e");
        // …and nothing landed at what would have been row 3 (no `c` anywhere).
        assert_eq!(doc.formatted_value(0, cell(2, 0)).unwrap(), "");
    }
}
