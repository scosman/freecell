//! The worker boundary contract: the `Command` / `WorkerEvent` protocol and its supporting
//! types (`architecture.md §2`, `components/engine_worker.md §Public interface`).
//!
//! Every type here is **engine-free**: it names only `freecell-core` types (`SheetId`,
//! `CellRef`, `CellRange`, `Rgb`, `InputRejection`, `SheetNameError`), `std`, and the
//! Phase-3 typed file errors (`LoadError` / `SaveError`, which carry only `String`s). **No
//! IronCalc type crosses this seam** — that is the headless boundary `freecell-engine`
//! exists to hold (`architecture.md §2`).

use std::ops::Range;
use std::path::PathBuf;

use freecell_core::input_cap::InputRejection;
use freecell_core::sheet_name::SheetNameError;
use freecell_core::{CellRange, CellRef, Rgb, SheetId};

use crate::document::{LoadError, SaveError};

/// A character/fill style change (`SetStyleAttr`). Bold / italic / underline are **toggles**
/// resolved worker-side — "any cell in the range lacks it → set the whole range, else clear
/// the whole range" (`components/engine_worker.md §SetStyleAttr`). `Fill` is a direct set
/// (`Some(color)`) or clear (`None`), matching the fill popover's swatches + "No Fill".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleAttr {
    Bold,
    Italic,
    Underline,
    Fill(Option<Rgb>),
}

/// A command the UI hands the worker over the (unbounded, non-blocking) command channel.
/// Undoable edits trigger a coalesced eval + publish; reads/control do not
/// (`components/engine_worker.md §Public interface` — the authoritative semantics table).
///
/// `sheet: SheetId` is the **stable** worksheet id (from IronCalc's `sheet_id`), so per-sheet
/// UI state survives a rename; the worker maps it to the volatile worksheet index before each
/// IronCalc call (`architecture.md §3`).
#[derive(Debug, Clone)]
pub enum Command {
    /// Set a cell's raw input. Pre-validated UI-side against the input cap and **re-validated
    /// here** (the worker is the security boundary for the round-3 D abort class).
    SetCellInput {
        sheet: SheetId,
        cell: CellRef,
        input: String,
    },
    /// Clear a range's contents (keep styles).
    ClearCells { sheet: SheetId, range: CellRange },
    /// Toggle/set a style attribute over a range.
    SetStyleAttr {
        sheet: SheetId,
        range: CellRange,
        attr: StyleAttr,
    },
    /// Append a new sheet.
    AddSheet,
    /// Rename a sheet (re-validated against the other sheet names here).
    RenameSheet { sheet: SheetId, name: String },
    /// Delete a sheet.
    DeleteSheet { sheet: SheetId },
    /// Undo the last committed edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Set the active sheet + overscanned viewport (already overscanned UI-side); triggers an
    /// immediate republish from current model state (no eval).
    SetViewport {
        sheet: SheetId,
        rows: Range<u32>,
        cols: Range<u32>,
    },
    /// Request a cell's raw content (the formula-bar text) — replied via `CellContent`.
    GetCellContent {
        sheet: SheetId,
        cell: CellRef,
        req_id: u64,
    },
    /// Serialize + atomically save to `path` — replied via `Saved` / `SaveFailed`.
    Save { path: PathBuf, req_id: u64 },
    /// Drop the model and exit the loop.
    Shutdown,
    /// Test-only: panic inside the `catch_unwind`-guarded apply, to exercise the recovery +
    /// degraded policy. Never constructed outside the crate's tests.
    #[cfg(test)]
    TestPanic,
}

/// Why an edit was refused (carried by `WorkerEvent::EditRejected`) — typed so the UI shows a
/// precise message instead of a generic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRejectedReason {
    /// The formula exceeded the length / nesting cap (the worker-side re-check).
    InputCap(InputRejection),
    /// A proposed sheet name failed validation.
    InvalidSheetName(SheetNameError),
    /// IronCalc returned a typed error for the edit (message preserved).
    Engine(String),
    /// The apply panicked and was caught (`catch_unwind`); the batch was dropped.
    EnginePanic,
    /// The worker is degraded (a prior unrecoverable panic) and is refusing edits; the UI
    /// offers Save As + reopen.
    Degraded,
}

/// Sheet metadata the worker publishes for the tab bar (`architecture.md §3`). `id` is stable
/// across renames; `name` is the current display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetMeta {
    pub id: SheetId,
    pub name: String,
}

/// An event the worker pushes to the UI over the (unbounded) event channel. The window awaits
/// these on a gpui foreground task and folds each into the entity
/// (`components/engine_worker.md §Threading & channels`).
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    /// The document loaded (new or opened); carries the sheet list. First paint uses the
    /// file's cached values (no eval on open — SP2).
    Loaded { sheets: Vec<SheetMeta> },
    /// The document failed to load (typed reason for the dialog).
    LoadFailed { error: LoadError },
    /// A new generation is available — re-read `DocumentClient::publication` and repaint.
    Published,
    /// A coalesced eval started (drives the "evaluating…" spinner after its no-flash delay).
    EvalStarted,
    /// The coalesced eval finished.
    EvalFinished,
    /// Reply to `GetCellContent`: the cell's raw text.
    CellContent { req_id: u64, raw: String },
    /// Reply to `Save`: success, acking the op-index the file now contains.
    Saved { req_id: u64, ops_seen: u64 },
    /// Reply to `Save`: failure (typed; the original file is untouched — atomic save).
    SaveFailed { req_id: u64, error: SaveError },
    /// An edit was refused (cap re-check, name validation, caught panic, or degraded).
    EditRejected { reason: EditRejectedReason },
    /// The style/geometry cache for `sheet` changed (deltas shipped via the shared cache).
    /// Defined now for the seam; **emitted in Phase 5** when the cache logic lands.
    StyleCacheUpdated { sheet: SheetId },
    /// The sheet list changed (add / rename / delete) — the UI re-syncs its tab bar.
    SheetsChanged { sheets: Vec<SheetMeta> },
    /// The worker hit an unrecoverable panic and is degraded: it keeps serving the last good
    /// publication + reads/save, but refuses edits. The UI shows the error bar + Save As.
    WorkerDegraded { reason: String },
}
