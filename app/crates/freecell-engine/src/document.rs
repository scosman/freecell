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

use freecell_core::{CellRange, CellRef, Rgb};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::types::{Style, Worksheet};

use crate::UserModel; // the crate's single canonical path to the IronCalc workbook type
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
                // Correct IronCalc's theme-colour and built-in number-format import before the
                // model is wrapped and read by the caches (`open_fixups` module docs).
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
    pub(crate) fn clear_contents(
        &mut self,
        sheet_idx: u32,
        range: CellRange,
    ) -> Result<(), String> {
        crate::instrument::record_engine_call();
        self.model.range_clear_contents(&area_of(sheet_idx, range))
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

/// Converts a 0-based [`CellRef`] to IronCalc's 1-based `(row, column)` `i32` coordinates.
/// The Excel-max bounds (`freecell_core::limits`) fit comfortably in `i32`.
fn to_engine_coords(cell: CellRef) -> (i32, i32) {
    (cell.row as i32 + 1, cell.col as i32 + 1)
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
}
