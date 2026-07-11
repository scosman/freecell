//! The worker boundary contract: the `Command` / `WorkerEvent` protocol and its supporting
//! types (`architecture.md §2`, `components/engine_worker.md §Public interface`).
//!
//! Every type here is **engine-free**: it names only `freecell-core` types (`SheetId`,
//! `CellRef`, `CellRange`, `Rgb`, `InputRejection`, `SheetNameError`), the pure
//! `freecell-chart-model` types (`Anchor`, `ChartInsertKind` — which already cross this seam via
//! [`ChartSnapshot`](super::charts::ChartSnapshot)/`ChartSpec`), `std`, and the Phase-3 typed file
//! errors (`LoadError` / `SaveError`, which carry only `String`s). **No IronCalc type crosses this
//! seam** — that is the headless boundary `freecell-engine` exists to hold (`architecture.md §2`).

use std::ops::Range;
use std::path::PathBuf;

use freecell_chart_model::{Anchor, ChartId, ChartInsertKind};
use freecell_core::input_cap::InputRejection;
use freecell_core::sheet_name::SheetNameError;
use freecell_core::{CellRange, CellRef, Rgb, SheetId};

use crate::document::{LoadError, SaveError};

/// A character/fill style change (`SetStyleAttr`). Bold / italic / underline / strikethrough /
/// wrap-text are **toggles** resolved worker-side — "any cell in the range lacks it → set the
/// whole range, else clear the whole range" (`components/engine_worker.md §SetStyleAttr`). `Fill`
/// is a direct set (`Some(color)`) or clear (`None`), matching the fill popover's swatches + "No
/// Fill".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleAttr {
    Bold,
    Italic,
    Underline,
    /// Strikethrough (`font.strike`) — toggles like [`Bold`](StyleAttr::Bold).
    Strikethrough,
    /// Wrap text (`alignment.wrap_text`) — toggles like [`Bold`](StyleAttr::Bold): any cell in the
    /// range lacking wrap → set the whole range, else clear it.
    WrapText,
    Fill(Option<Rgb>),
}

/// A direct-set style attribute addressed by IronCalc's `update_range_style` path
/// (`architecture.md §3.1`, `components/action_bar.md`). Typed (instead of a raw path string) so
/// the UI can only ever address the three formatting paths this project owns — the value carried by
/// [`Command::SetStylePath`] is what varies. No IronCalc type crosses the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StylePath {
    /// `font.color` — `#RRGGBB` sets, `""` clears (→ Automatic).
    FontColor,
    /// `alignment.horizontal` — `left|center|right` sets, `general` clears horizontal only
    /// (leaving any file-loaded vertical/wrap alignment intact).
    AlignHorizontal,
    /// `alignment.vertical` — `top|center|bottom` sets the cell's vertical alignment (a plain set,
    /// exactly like [`AlignHorizontal`](StylePath::AlignHorizontal); no toggle). Justify/Distributed
    /// are out of scope.
    AlignVertical,
    /// `num_fmt` — the raw number-format code (one of the dropdown codes, or a decimals-adjusted
    /// derivative).
    NumFmt,
}

impl StylePath {
    /// The IronCalc `update_range_style` path string for this attribute.
    pub fn as_str(self) -> &'static str {
        match self {
            StylePath::FontColor => "font.color",
            StylePath::AlignHorizontal => "alignment.horizontal",
            StylePath::AlignVertical => "alignment.vertical",
            StylePath::NumFmt => "num_fmt",
        }
    }
}

/// A fixed border preset the borders popover applies over the selection (`functional_spec.md §3.6`,
/// `components/action_bar.md`). Each maps 1:1 to an IronCalc `BorderType` and selects **which edges**
/// are written (the worker builds the `BorderArea` `"type"` from
/// [`border_type_tag`](BorderPreset::border_type_tag)); it is orthogonal to the line style + color,
/// which the pen ([`BorderLine`] + `color`) carries. Kept a plain enum (no IronCalc type crosses the
/// seam), mirroring [`StylePath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderPreset {
    /// Every edge of every selected cell.
    All,
    /// Interior edges only (between adjacent selected cells).
    Inner,
    /// The selection's outer perimeter.
    Outer,
    /// The top edge of the selection's top row.
    Top,
    /// The bottom edge of the selection's bottom row.
    Bottom,
    /// The left edge of the selection's left column.
    Left,
    /// The right edge of the selection's right column.
    Right,
    /// Clears all borders in the selection.
    None,
}

/// The line style the borders control paints — the pen's `style`, mirroring the line-style
/// gallery (`architecture.md §2`, `functional_spec.md §2.3`). Engine-free: it maps to an IronCalc
/// `BorderStyle` serde tag via [`style_tag`](BorderLine::style_tag). The MVP set (thin/medium/thick
/// solid, dashed, double) is fully `.xlsx`-representable; Dotted / dash-dot are deferred (GAPS F3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderLine {
    /// Thin (1px) solid — the default pen (`"thin"`).
    #[default]
    ThinSolid,
    /// Medium (2px) solid (`"medium"`).
    MediumSolid,
    /// Thick (3px) solid (`"thick"`).
    ThickSolid,
    /// Dashed (2px, IronCalc `MediumDashed` — `"mediumdashed"`).
    Dashed,
    /// Double (3px, two thin parallel lines — `"double"`).
    Double,
}

impl BorderLine {
    /// The IronCalc `BorderStyle` serde tag for this line (the `"style"` field of the JSON-built
    /// `BorderArea` item, `architecture.md §4`). All five round-trip through `.xlsx`. Same plain
    /// pattern as [`BorderPreset::border_type_tag`] — no engine type crosses the seam.
    pub fn style_tag(self) -> &'static str {
        match self {
            BorderLine::ThinSolid => "thin",
            BorderLine::MediumSolid => "medium",
            BorderLine::ThickSolid => "thick",
            BorderLine::Dashed => "mediumdashed",
            BorderLine::Double => "double",
        }
    }
}

impl BorderPreset {
    /// The IronCalc `BorderType` serde tag for this preset (the `"type"` field of the JSON-built
    /// `BorderArea`, `architecture.md §3.4`). Same pattern as [`StylePath::as_str`]: a plain string,
    /// not an engine type.
    pub fn border_type_tag(self) -> &'static str {
        match self {
            BorderPreset::All => "All",
            BorderPreset::Inner => "Inner",
            BorderPreset::Outer => "Outer",
            BorderPreset::Top => "Top",
            BorderPreset::Bottom => "Bottom",
            BorderPreset::Left => "Left",
            BorderPreset::Right => "Right",
            BorderPreset::None => "None",
        }
    }
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
    /// Set a direct style attribute (text color, horizontal alignment, or number format) over a
    /// range via IronCalc's `update_range_style` path (`architecture.md §3.1`). Style-only: no
    /// evaluation, cache rebuild + publish only. Fire-and-forget (log-only on engine rejection —
    /// the UI only ever sends valid paths/values).
    SetStylePath {
        sheet: SheetId,
        range: CellRange,
        path: StylePath,
        value: String,
    },
    /// Set the font **family** and/or **size** over a range (`architecture.md §3.3`,
    /// `components/action_bar.md`). IronCalc 0.7.1 has no font-name/absolute-size style path, so the
    /// worker applies it via `on_paste_styles` (materialising per-cell styles → full row/col/
    /// select-all clamps to the used range, a documented deviation) and **auto-grows** rows too
    /// small for a larger size. Style-only: no evaluation. `family = Some("")` = System Default
    /// (reset to the workbook default); `Some(name)` sets it; `None` leaves the family. `size_pt =
    /// Some(pt)` sets the size; `None` leaves it. A too-large clamped selection replies
    /// [`WorkerEvent::EditRejected`] with an `Engine` message the window dialogs.
    SetFont {
        sheet: SheetId,
        range: CellRange,
        family: Option<String>,
        size_pt: Option<f64>,
    },
    /// Apply a border preset over a range with a given line style + color
    /// (`architecture.md §2/§4`, `components/action_bar.md`). Style-only (no evaluation): applied
    /// via IronCalc `set_area_with_border` — one undoable diff-list, band-aware for full rows/
    /// columns, with the engine's heavier-wins fix-up on the four adjacent strips. Fire-and-forget
    /// (log-only on engine rejection). The written border item carries `line`'s
    /// [`style_tag`](BorderLine::style_tag) and `color` (`None` ⇒ default black); non-targeted edges
    /// are preserved (the `preset`'s `BorderType` implies which edges are written).
    SetBorders {
        sheet: SheetId,
        range: CellRange,
        preset: BorderPreset,
        line: BorderLine,
        color: Option<Rgb>,
    },
    /// Set the width of an inclusive column run `[col_start, col_end]` (0-based) to `px`
    /// **device px** (`functional_spec.md §5.1`). Geometry-only (no evaluation): applied via
    /// `set_columns_width` (one undoable diff-list), then the active sheet's cache is rebuilt.
    /// Sent over a **bounded** run only (a resize target / selected header run).
    SetColumnWidths {
        sheet: SheetId,
        col_start: u32,
        col_end: u32,
        px: f64,
    },
    /// Set the height of an inclusive row run `[row_start, row_end]` (0-based) to `px` **device
    /// px** (`functional_spec.md §5.1`). Geometry-only (no evaluation); cf. [`Command::SetColumnWidths`].
    SetRowHeights {
        sheet: SheetId,
        row_start: u32,
        row_end: u32,
        px: f64,
    },
    /// Insert `count` blank rows so new rows appear at 0-based `row` (`functional_spec.md §5.3`);
    /// content at/after `row` shifts down and formulas adjust. Undoable; needs evaluation. The
    /// worker **merge-guards** it first (a merge at/after `row` → [`EditRejectedReason::MergedCells`]
    /// dialog); a shift past the sheet edge returns an engine error → dialog.
    InsertRows {
        sheet: SheetId,
        row: u32,
        count: u32,
    },
    /// Insert `count` blank columns at 0-based `col` (column analog of [`Command::InsertRows`]).
    InsertColumns {
        sheet: SheetId,
        col: u32,
        count: u32,
    },
    /// Delete `count` rows starting at 0-based `row` (row analog; merge-guarded + undoable).
    DeleteRows {
        sheet: SheetId,
        row: u32,
        count: u32,
    },
    /// Delete `count` columns starting at 0-based `col` (column analog).
    DeleteColumns {
        sheet: SheetId,
        col: u32,
        count: u32,
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
    /// Copy (or cut) the selection to the engine clipboard slot (`components/clipboard.md`).
    /// Replies with [`WorkerEvent::CopyReady`] carrying the tab-separated text to place on the
    /// system clipboard. `cut` is recorded for the later paste (nothing clears at cut time).
    CopySelection {
        sheet: SheetId,
        range: CellRange,
        cut: bool,
    },
    /// Paste the engine clipboard slot into `target` — the destination selection (full-fidelity:
    /// values + adjusted formulas + styles). The paste anchors at `target.start`; when the copied
    /// source is a single cell (or an exact divisor of the target) and `target` is larger, the
    /// source is **tiled/filled** across the whole selection as one undo step (BUG 4). Values and
    /// styles fill exactly; a **formula** gets the top-left cell's `anchor − source` reference shift
    /// applied uniformly to every filled cell — NOT Excel's per-cell relative fill (accepted
    /// limitation U2 in `GAPS.md`, to keep the fill one undo step). Replies with
    /// [`WorkerEvent::Pasted`] (the pasted range) or [`WorkerEvent::PasteRejected`].
    PasteInternal { sheet: SheetId, target: CellRange },
    /// Paste external tab-separated `text` at `anchor` (each token as user input). Replies with
    /// [`WorkerEvent::Pasted`] or [`WorkerEvent::PasteRejected`].
    PasteTsv {
        sheet: SheetId,
        anchor: CellRef,
        text: String,
    },
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
    /// Insert a **near-empty authored chart** of `kind` onto `sheet`, placed at `anchor` (P17,
    /// charts/ui_design §3.1). The worker builds the template chart
    /// ([`ChartInsertKind::near_empty_chart`]), holds it as an
    /// [`Authored`](freecell_chart_model::Origin::Authored) `ChartSpec` **snapshot-but-not-live**
    /// (it has no `c:f` binding yet — ranges arrive in P19), and publishes it on the chart snapshot
    /// so the grid renders it. On save it takes the **write-from-model** path
    /// (`write::write_authored_charts`), never the loaded re-inject. Rejected with
    /// [`EditRejectedReason::Degraded`] when the worker is degraded (like every mutating op).
    InsertChart {
        sheet: SheetId,
        kind: ChartInsertKind,
        anchor: Anchor,
    },
    /// **Move or resize** a chart on `sheet` (P18, `ui_design §3.2`): set the chart named by its
    /// stable [`ChartId`] to a new [`Anchor`] (both move and resize produce one). For an **authored**
    /// chart the worker rewrites the model anchor; for a **loaded** chart it updates the render
    /// anchor and records a drawing-anchor patch the source-first save applies to the retained
    /// `twoCellAnchor`. Degraded-guarded like every mutating op.
    SetChartAnchor {
        sheet: SheetId,
        id: ChartId,
        anchor: Anchor,
    },
    /// **Delete** a chart on `sheet` (P18, Delete/Backspace or the chart context menu): drop the
    /// chart named by its [`ChartId`]. An **authored** chart is removed from the authored set; a
    /// **loaded** chart is unbound and recorded so the save drops it from the package (its
    /// `twoCellAnchor` + part chain) without corrupting the rest. Degraded-guarded.
    DeleteChart { sheet: SheetId, id: ChartId },
    /// **Set an authored chart's data range** (P19, the edit panel): bind the chart named by its
    /// [`ChartId`] to the rectangular `data` block on `sheet`. The chart is resolved by **`id`** (not
    /// `sheet`), so `sheet` names the worksheet the **data** lives on — the worker qualifies the
    /// emitted `c:f` with it and reads the values there; it may differ from the chart's host/anchor
    /// sheet (valid cross-sheet chart data). The worker interprets the block (first row = series
    /// names, first column = categories/x, each remaining column a series), gives the chart real `c:f`
    /// refs, and re-resolves its values from the current cells — so it transitions from P17's
    /// snapshot-but-not-live placeholder to a **LIVE** chart that re-renders on edit and saves with
    /// `c:f` + caches (write-from-model). Degraded-guarded; a loaded/unknown id is ignored (loaded
    /// re-range is P20).
    SetChartRange {
        sheet: SheetId,
        id: ChartId,
        data: CellRange,
    },
    /// **Switch an authored chart's type** (P19, the edit panel): rebuild the chart named by its
    /// [`ChartId`] to `kind`, preserving its data-range binding + title where sensible, so it renders
    /// as the new kind and round-trips. Degraded-guarded; a loaded/unknown id is ignored.
    SetChartType {
        sheet: SheetId,
        id: ChartId,
        kind: ChartInsertKind,
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
    /// An insert/delete rows/columns was blocked because the sheet has merged cells the op would
    /// displace (`functional_spec.md §5.3`). Merged cells aren't yet supported, so the op is
    /// refused and the UI shows the merge-guard dialog. Carries no payload (fixed message).
    MergedCells,
    /// The apply panicked and was caught (`catch_unwind`); the batch was dropped.
    EnginePanic,
    /// The worker is degraded (a prior unrecoverable panic) and is refusing edits; the UI
    /// offers Save As + reopen.
    Degraded,
}

/// Why a paste was refused (carried by [`WorkerEvent::PasteRejected`]). `Overflow` is
/// user-visible (a dialog — the copied range would spill past the sheet edge, so nothing is
/// pasted); `NothingToPaste` is log-only (an internal paste with no live slot, e.g. the second
/// paste of a cut).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteError {
    /// The paste would extend past the Excel-max sheet edge (`functional_spec.md §2.2`).
    Overflow,
    /// An internal paste ran with no clipboard slot (empty, or a cut already consumed).
    NothingToPaste,
}

/// Sheet metadata the worker publishes for the tab bar (`architecture.md §3`). `id` is stable
/// across renames; `name` is the current display name; `has_content` gates the delete-confirm
/// modal (`functional_spec.md §3.7`: a non-empty sheet confirms before delete).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetMeta {
    pub id: SheetId,
    pub name: String,
    /// Whether the sheet has any cell content (populated worker-side from `sheet_data`).
    pub has_content: bool,
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
    /// Reply to [`Command::CopySelection`]: the tab-separated text the UI writes to the system
    /// clipboard (and remembers as its last copy, to route a later paste internally).
    CopyReady { tsv: String },
    /// Reply to a paste: it applied and the pasted rectangle (0-based) is now selected — the UI
    /// mirrors it into its `SelectionModel`.
    Pasted { sheet: SheetId, range: CellRange },
    /// Reply to a paste that could not apply (`Overflow` → dialog; `NothingToPaste` → log).
    PasteRejected { reason: PasteError },
    /// The worker hit an unrecoverable panic and is degraded: it keeps serving the last good
    /// publication + reads/save, but refuses edits. The UI shows the error bar + Save As.
    WorkerDegraded { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_line_style_tags_are_stable_ironcalc_serde_tags() {
        // All five map to a lowercase IronCalc `BorderStyle` serde tag; Phase 3's gallery depends on
        // these exact strings (`architecture.md §4`).
        assert_eq!(BorderLine::ThinSolid.style_tag(), "thin");
        assert_eq!(BorderLine::MediumSolid.style_tag(), "medium");
        assert_eq!(BorderLine::ThickSolid.style_tag(), "thick");
        assert_eq!(BorderLine::Dashed.style_tag(), "mediumdashed");
        assert_eq!(BorderLine::Double.style_tag(), "double");
        // The pen defaults to thin solid black.
        assert_eq!(BorderLine::default(), BorderLine::ThinSolid);
    }
}
