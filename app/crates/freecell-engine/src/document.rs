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
use std::io::{self, BufWriter, ErrorKind, Read};
use std::path::{Path, PathBuf};

use freecell_core::format_color::{format_color_rgb, is_date_format};
use freecell_core::{CellKind, CellRange, CellRef, Rgb};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::cell::CellValue;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::formatter::format::format_number;
use ironcalc_base::locale::get_locale;
use ironcalc_base::types::{CellType, Font, Style, Worksheet};
use ironcalc_base::Model;

use crate::UserModel; // the crate's single canonical path to the IronCalc workbook type
use ironcalc_base::BorderArea;
use ironcalc_base::ClipboardData;
use serde::Deserialize;
use tempfile::NamedTempFile;
use thiserror::Error;

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
/// / `font.u`). In-crate: the toggle *policy* (any-lacking → set-all) lives in the worker;
/// this is only the read/write *mechanism* over the pinned IronCalc range-style API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontFlag {
    Bold,
    Italic,
    Underline,
}

impl FontFlag {
    /// The IronCalc `update_range_style` / `get_cell_style` path for this flag.
    fn style_path(self) -> &'static str {
        match self {
            FontFlag::Bold => "font.b",
            FontFlag::Italic => "font.i",
            FontFlag::Underline => "font.u",
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

        match load_from_xlsx(path_str, DEFAULT_LOCALE, DEFAULT_TIMEZONE, DEFAULT_LANGUAGE) {
            Ok(mut model) => {
                // Correct IronCalc's theme-colour, indexed-colour, and built-in number-format
                // import before the model is wrapped and read by the caches (`open_fixups`
                // module docs).
                crate::open_fixups::apply_open_fixups(&mut model, path);
                Ok(Self {
                    model: UserModel::from_model(model),
                })
            }
            // A real read error after the magic check (e.g. the file vanished mid-open).
            Err(ironcalc::error::XlsxError::IO(msg)) => Err(LoadError::Io(msg)),
            // It IS a Zip, so any structural/parse/workbook/feature failure means the
            // workbook itself is damaged or unsupported. Before giving up, try one
            // best-effort **reactive repair** for IronCalc's over-strict styles parser
            // (which rejects a `<cellXfs>` `<xf>` that omits the *optional* `xfId` — as
            // Numbers/LibreOffice-exported files do). `try_repair_and_reload` returns `Some`
            // only for that specific error class and only if the read→patch→reload all
            // succeed; on any failure we fall through to the ORIGINAL typed error so the
            // file's real problem is what surfaces (`open_repair` module docs).
            Err(other) => match crate::open_repair::try_repair_and_reload(path, &other) {
                Some(mut model) => {
                    crate::open_fixups::apply_open_fixups(&mut model, path);
                    Ok(Self {
                        model: UserModel::from_model(model),
                    })
                }
                // The message is preserved for the dialog details line (a `NotImplemented`
                // message names the unsupported feature).
                None => Err(LoadError::Corrupt(other.to_string())),
            },
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
        }
    }

    /// Saves the workbook to `path` **atomically**: serialize into a temp file in the
    /// destination directory, fsync it, then rename it over the target. On any failure the
    /// original file at `path` is left untouched (`functional_spec.md §5.2`,
    /// `components/engine_worker.md §File I/O`).
    pub fn save(&self, path: &Path) -> Result<(), SaveError> {
        crate::instrument::record_engine_call();
        let dir = destination_dir(path);

        // Same-directory temp file → the final rename is a same-filesystem atomic op.
        let temp = NamedTempFile::new_in(&dir).map_err(|e| {
            SaveError::Io(format!(
                "couldn't create a temporary file in {}: {e}",
                dir.display()
            ))
        })?;

        // Serialize the model into the temp file (buffered — the zip writer does many small
        // writes). `save_xlsx_to_writer` takes the writer by value and hands it back.
        let writer = BufWriter::new(temp.as_file());
        let writer =
            save_xlsx_to_writer(self.model.get_model(), writer).map_err(map_writer_error)?;
        writer
            .into_inner()
            .map_err(|e| SaveError::Io(e.to_string()))?;

        // Flush data + metadata to disk BEFORE the rename makes the file visible at `path`.
        temp.as_file()
            .sync_all()
            .map_err(|e| SaveError::Io(e.to_string()))?;

        // Atomic rename over the target. On failure the temp file (returned inside the
        // error) drops and is cleaned up, and `path` is never touched.
        temp.persist(path)
            .map_err(|e| SaveError::Io(e.error.to_string()))?;

        Ok(())
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
        })
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

    /// Applies a border `preset` (an IronCalc `BorderType` serde tag) over a range via
    /// `set_area_with_border` (`architecture.md §3.4`, `border.rs:346`). One undoable diff-list;
    /// band-aware for full rows/columns; the engine applies its heavier-wins fix-up to the four
    /// adjacent strips. `BorderArea` has `pub(crate)` fields and no constructor at 0.7.1 but derives
    /// `Deserialize`, so it is built from JSON — thin black only (the only style the borders popover
    /// applies; `functional_spec.md §3.6`). For `type: "None"` the engine ignores `item` and clears
    /// the edges.
    pub(crate) fn set_borders(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
        border_type: &str,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        let border_area: BorderArea = serde_json::from_value(serde_json::json!({
            "item": { "style": "thin", "color": "#000000" },
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
        Ok(CopiedRange { tsv, data, range })
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
    if let Some(rgb) = style
        .font
        .color
        .as_deref()
        .and_then(crate::cache::parse_color)
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

    /// Bold / italic / underline / fill / font-color survive a save→reopen round-trip.
    /// In-crate (not an integration test) because it reads back the raw `ironcalc` `Style`.
    #[test]
    fn roundtrip_styles_preserved() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("styles.xlsx");

        let doc = fixtures::styles();
        doc.save(&path).unwrap();
        let reopened = WorkbookDocument::open(&path).unwrap();

        // A1 bold, B1 italic, C1 underline (fixtures::styles layout).
        assert!(reopened.cell_style(0, CellRef::new(0, 0)).unwrap().font.b);
        assert!(reopened.cell_style(0, CellRef::new(0, 1)).unwrap().font.i);
        assert!(reopened.cell_style(0, CellRef::new(0, 2)).unwrap().font.u);

        // A2 red fill, B2 blue font color.
        let fill = reopened.cell_style(0, CellRef::new(1, 0)).unwrap().fill;
        assert_eq!(fill.fg_color.as_deref(), Some("#FF0000"));
        let font = reopened.cell_style(0, CellRef::new(1, 1)).unwrap().font;
        assert_eq!(font.color.as_deref(), Some("#0000FF"));
    }

    #[test]
    fn default_font_reads_workbook_default() {
        // A fresh workbook's default is IronCalc's `Font::default()` — 13pt Calibri (NOT the
        // 11pt the specs state; the cache detects "default" relative to this value).
        let doc = WorkbookDocument::new_empty().unwrap();
        let (sz, name) = doc.default_font();
        assert_eq!(sz, 13);
        assert_eq!(name, "Calibri");
    }

    #[test]
    fn set_borders_applies_all_and_none_clears() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let a1 = CellRef::new(0, 0);
        doc.set_cell_input(0, a1, "x").unwrap();

        // "All" sets all four thin edges.
        doc.set_borders(0, CellRange::single(a1), "All").unwrap();
        let b = doc.cell_style(0, a1).unwrap().border;
        assert!(
            b.top.is_some() && b.right.is_some() && b.bottom.is_some() && b.left.is_some(),
            "All applies every edge"
        );
        assert_eq!(
            b.top.as_ref().unwrap().style,
            ironcalc_base::types::BorderStyle::Thin
        );

        // "None" clears them again.
        doc.set_borders(0, CellRange::single(a1), "None").unwrap();
        let b = doc.cell_style(0, a1).unwrap().border;
        assert!(
            b.top.is_none() && b.right.is_none() && b.bottom.is_none() && b.left.is_none(),
            "None clears every edge"
        );

        // A bogus tag is a clean error (never panics).
        assert!(doc.set_borders(0, CellRange::single(a1), "Bogus").is_err());
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
            style.fill.fg_color.as_deref(),
            Some("#FF0000"),
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
                    style.fill.fg_color.as_deref(),
                    Some("#FF0000"),
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
