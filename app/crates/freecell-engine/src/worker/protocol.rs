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

use freecell_chart_model::{Anchor, ChartId, ChartInsertKind, LegendPosition};
use freecell_core::input_cap::InputRejection;
use freecell_core::sheet_name::SheetNameError;
use freecell_core::{
    CellRange, CellRef, CfRuleSpec, Direction, FillAxis, Rgb, SelectionStats, SheetId,
};

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

/// Which axis a [`ChartChromeEdit::AxisTitle`] targets — the category (`c:catAx`, or scatter's X
/// `c:valAx`) or the value (`c:valAx`, or scatter's Y) axis, matching the model's
/// `Chart::cat_axis` / `Chart::val_axis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartAxisKind {
    Category,
    Value,
}

/// The data-label **show** toggles a [`ChartChromeEdit::DataLabels`] applies across every series of a
/// chart (`c:dLbls` `showVal` / `showCatName` / `showPercent`, functional_spec §6.B). Each series'
/// existing label number-format / separator / position (and any legend-key / series-name already on)
/// is preserved; only these three flags are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataLabelToggles {
    pub show_value: bool,
    pub show_category_name: bool,
    pub show_percent: bool,
}

/// One chrome attribute change carried by [`Command::SetChartChrome`] (P20, functional_spec §6.B).
/// Each variant names exactly one field of the render [`Chart`](freecell_chart_model::Chart) — so the
/// loaded-chart source patch can splice **only** that sub-element and leave everything else
/// byte-stable (the edit contract).
#[derive(Debug, Clone, PartialEq)]
pub enum ChartChromeEdit {
    /// Set (or clear, `None`) the chart title.
    Title(Option<String>),
    /// Turn the legend on at a position, or off (`None`).
    Legend(Option<LegendPosition>),
    /// Set (or clear, `None`) an axis title.
    AxisTitle {
        axis: ChartAxisKind,
        title: Option<String>,
    },
    /// Set (or clear back to the palette, `None`) one series' color, by 0-based series index.
    SeriesColor { series: usize, color: Option<Rgb> },
    /// Apply the data-label toggles across every series.
    DataLabels(DataLabelToggles),
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
    /// Fill Down (⌘D): copy `range`'s **top row** down over the rest of `range` — a copy, not a
    /// series (`functional_spec.md §3`). A lone single-cell `range` pulls from the cell above. One
    /// undo step (rides IronCalc's `auto_fill_rows` history).
    FillDown { sheet: SheetId, range: CellRange },
    /// Fill Right (⌘R): copy `range`'s **left column** right over the rest of `range` (the column
    /// analog of [`Command::FillDown`]). A lone single-cell `range` pulls from the cell to the left.
    FillRight { sheet: SheetId, range: CellRange },
    /// Drag-fill (`gaps_closing_7_15 §3`): extend `seed`'s content into `target` (⊇ `seed`) along
    /// the dominant `axis`. Unlike ⌘D/⌘R this seeds `auto_fill_*` with the **full** `seed` block, so
    /// a multi-cell seed extrapolates a series (a single-cell seed copies). One undo step.
    FillDrag {
        sheet: SheetId,
        seed: CellRange,
        target: CellRange,
        axis: FillAxis,
    },
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
    /// A **user** row-resize — so the worker marks the run **manual**, exempting it from wrap-driven
    /// auto-grow (`functional_spec.md §3.3`).
    SetRowHeights {
        sheet: SheetId,
        row_start: u32,
        row_end: u32,
        px: f64,
    },
    /// Wrap-driven row auto-grow (`functional_spec.md §3.2`, `architecture.md §3`): the UI measured
    /// each 0-based `row`'s wrapped height (device px) on the render thread — the worker can't (no
    /// gpui text system). **Distinct** from [`Command::SetRowHeights`] so the worker knows these are
    /// **auto** (never marked manual). Applied as a **cache-only** geometry update — final row height
    /// = `max(base IronCalc height, wrap)`, clamped to the cap — so it never shrinks below a
    /// font/newline auto-fit, **skips manual rows**, does **not** touch IronCalc / `ops_seen` / the
    /// undo stack (rides the causing edit — no separate undo step, §3.4), and republishes only when a
    /// height actually changed. A per-row value `<= default` drops that row's wrap contribution
    /// (shrink on unwrap / clear / column-widen).
    AutoGrowRowHeights {
        sheet: SheetId,
        heights: Vec<(u32, f32)>,
    },
    /// Set (or clear) the **hidden** flag on the inclusive 0-based row run `[start, end]`
    /// (`gaps_closing_7_15 §4`). Geometry-only (no evaluation — hiding never changes values), one
    /// undoable diff-list (`set_rows_hidden`); the active sheet's cache is rebuilt (it re-reads the
    /// hidden flags → zero-size geometry). `hidden: true` = Hide, `false` = Unhide (restore).
    SetRowsHidden {
        sheet: SheetId,
        start: u32,
        end: u32,
        hidden: bool,
    },
    /// Set (or clear) the **hidden** flag on the inclusive 0-based column run `[start, end]` (the
    /// column analog of [`Command::SetRowsHidden`]).
    SetColumnsHidden {
        sheet: SheetId,
        start: u32,
        end: u32,
        hidden: bool,
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
    /// Merge `area` (0-based inclusive) into one region, keeping only the anchor (`area.start`)
    /// value and clearing the covered cells' content (merged-cell-ui `functional_spec.md F2`,
    /// `architecture.md §3`). Undoable + needs a recompute (a covered-content clear can feed
    /// formulas). `confirmed` is the data-loss round-trip flag: an unconfirmed merge that would
    /// discard covered content replies [`WorkerEvent::MergeNeedsConfirm`] and applies nothing; the
    /// UI re-sends with `confirmed: true` to proceed. A merge with no covered content (or a single
    /// value) applies regardless of `confirmed`. Engine-rejected (array/spill collision) → an
    /// `Engine` [`WorkerEvent::EditRejected`]; the toggle structurally avoids overlap rejects.
    MergeCells {
        sheet: SheetId,
        area: CellRange,
        confirmed: bool,
    },
    /// Remove the merged region whose anchor is `anchor` (0-based) — Unmerge (merged-cell-ui
    /// `functional_spec.md F2`). The toggle always names a region's anchor; a cell in no region is
    /// an engine no-op. Undoable + republishes so the resident `MergeMap` and restored region draw
    /// together.
    UnmergeCells { sheet: SheetId, anchor: CellRef },
    /// Append a new sheet.
    AddSheet,
    /// Rename a sheet (re-validated against the other sheet names here).
    RenameSheet { sheet: SheetId, name: String },
    /// Delete a sheet.
    DeleteSheet { sheet: SheetId },
    /// Move the sheet with stable id `sheet` so it lands at 0-based worksheet index `to_index`,
    /// shifting the intervening sheets (`functional_spec.md §6`). Undoable (rides the fork's
    /// history); the new order is preserved on xlsx save. The worker maps `sheet` → its current
    /// worksheet index before calling the fork's index-based reorder API. Republishes
    /// [`WorkerEvent::SheetsChanged`] so the tab bar rebuilds in the new engine order.
    MoveSheet { sheet: SheetId, to_index: u32 },
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
    /// Paste the internal clipboard slot's **computed values** into `target` — the values-only
    /// sibling of [`PasteInternal`] (⌘⇧V, `functional_spec.md §5`): each source cell pastes its
    /// evaluated value as a **literal** (a formula collapses to its result, a leading-`=` string
    /// stays literal text, numbers stay numbers), with **no** formulas and **no** formatting — the
    /// target keeps its own. Sizing/overflow match `PasteInternal` (a single-cell / exact-divisor
    /// source tiles to fill the selection; a block pastes from the anchor; oversized → Overflow).
    /// One undo step. Replies with [`WorkerEvent::Pasted`] or [`WorkerEvent::PasteRejected`].
    PasteValues { sheet: SheetId, target: CellRange },
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
    /// Aggregate statistics for `range ∩ the sheet's populated cells` (the status-bar selection
    /// readout, `functional_spec.md §1`). A **read**: no evaluation, no publish. Computed
    /// worker-side (which owns the model) so a full-column/row selection is correct **past the
    /// published viewport** and a huge sparse sheet is walked in O(populated), never O(cells
    /// selected). `req_id` tags the reply so the chrome drops a stale one for a superseded
    /// selection. Replies with [`WorkerEvent::SelectionStats`].
    SelectionStats {
        sheet: SheetId,
        range: CellRange,
        req_id: u64,
    },
    /// Resolve the **edge-of-data** target for a ⌘/Ctrl+arrow jump from `from` in `dir`
    /// (`functional_spec.md §4`; the `JumpEdge`/`ExtendEdge` motions). A **read**: no evaluation, no
    /// publish. Computed worker-side (which owns the model) so occupancy **past the published
    /// viewport** feeds the exact Excel algorithm — gathering the active line's populated cells
    /// (O(populated cells on the line) for a row jump; O(populated rows) for a column jump), not
    /// O(sheet). The `extend`/anchor semantics stay UI-side (the grid collapses or keeps the anchor on
    /// the reply); `req_id` tags the reply so the grid drops a stale one for a superseded jump. Replies
    /// with [`WorkerEvent::EdgeResolved`].
    ResolveEdge {
        sheet: SheetId,
        from: CellRef,
        dir: Direction,
        req_id: u64,
    },
    /// Scan `sheet`'s used range for cells whose **raw content** (formula text for formula cells)
    /// matches `query` under the case / whole-cell rules (`functional_spec.md §4.3`). A **read**: no
    /// evaluation, no publish. Replies with [`WorkerEvent::FindResults`] (row-major matches). Runs in
    /// the worker (which owns the model) so a huge sheet's scan never blocks the UI (`§4.5`).
    Find {
        sheet: SheetId,
        query: String,
        match_case: bool,
        whole_cell: bool,
    },
    /// Replace the match in a single cell (`functional_spec.md §4.4`): the **worker** re-reads the
    /// cell's raw content and applies the replacement (substring, or whole content if `whole_cell`),
    /// then commits it via the normal edit path — recomputing the replacement worker-side (from fresh
    /// `cell_content`) avoids a stale-content race with the UI. A single-cell edit is inherently one
    /// undo step. Replies [`WorkerEvent::ReplacedCount`] (`n = 1` if it wrote, else `0`).
    ReplaceOne {
        sheet: SheetId,
        cell: CellRef,
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
    },
    /// Replace **every** match in `sheet`'s used range (`functional_spec.md §4.4`), evaluate once,
    /// publish, and reply [`WorkerEvent::ReplacedCount`] with the number of cells changed.
    ///
    /// INTENDED as one undoable batch (`§4.4`). IronCalc's atomic multi-cell undo mechanism
    /// (`History::push`) is `pub(crate)` and the public rectangle pastes are unusable for scattered
    /// matches, so this currently records **one engine undo entry per changed cell** (like the
    /// accepted `SetFont` "K+1 undo steps" precedent) pending a fork `set_user_inputs` batch method —
    /// see `phase_plans/phase_4.md` ROADBLOCK + `DECISIONS_TO_REVIEW.md`.
    ReplaceAll {
        sheet: SheetId,
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
    },
    /// Insert an **authored chart** of `kind` onto `sheet`, placed at `anchor` (P17,
    /// charts/ui_design §3.1). The worker builds the template chart
    /// ([`ChartInsertKind::near_empty_chart`]), holds it as an
    /// [`Authored`](freecell_chart_model::Origin::Authored) `ChartSpec`, and publishes it on the
    /// chart snapshot so the grid renders it. On save it takes the **write-from-model** path
    /// (`write::write_authored_charts`), never the loaded re-inject. Rejected with
    /// [`EditRejectedReason::Degraded`] when the worker is degraded (like every mutating op).
    ///
    /// `data` seeds the chart's initial **data range** (post-v1 Batch 3, item 8): when the action
    /// bar captures a real multi-cell selection at insert time, the worker binds that range
    /// **immediately** — running the same block→series binding as [`SetChartRange`](Command::SetChartRange)
    /// on the freshly-assigned id — so the chart is born **LIVE** (real `c:f` refs + resolved values),
    /// no follow-up "Use selection" click. `None` (a single-cell/empty selection) keeps the P17
    /// snapshot-but-not-live placeholder (no `c:f` binding until a range is set). The range's cells
    /// live on the insert `sheet` (the active sheet the selection came from).
    InsertChart {
        sheet: SheetId,
        kind: ChartInsertKind,
        anchor: Anchor,
        data: Option<CellRange>,
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
    /// **Edit a chart's chrome** (P20, the edit panel): change one chrome attribute — title, legend,
    /// axis title, series color, or data-label toggles — of the chart named by its [`ChartId`]. Unlike
    /// range/type, this applies to **both** provenances: an **authored** chart's model is mutated and
    /// re-serialized on save (write-from-model); a **loaded** chart's retained render model is mutated
    /// (so it re-renders live) and its retained `chartN.xml` is **source-patched** on save — only the
    /// changed sub-element is spliced, so unmodeled OOXML styling is preserved byte-for-byte (the edit
    /// contract, functional_spec §6). `sheet` is advisory (the panel's host sheet); the chart is found
    /// by `id`. Degraded-guarded; an unknown id is ignored.
    SetChartChrome {
        sheet: SheetId,
        id: ChartId,
        edit: ChartChromeEdit,
    },
    /// Add a conditional-formatting rule over `range` (an A1 range / multi-area) on `sheet`
    /// (`architecture.md §4.2`, `components/engine_cf.md §5`). Style-only (no recompute): the CF rule
    /// changes styles, not values. Undoable (the engine records a diff). On success the worker
    /// refreshes the published CF map + emits [`WorkerEvent::CondFmtUpdated`] (plus the usual
    /// `StyleCacheUpdated`); a bad range/formula/operand surfaces as
    /// [`EditRejectedReason::Engine`] on the result channel, nothing partially applied. `spec` is an
    /// engine-free [`CfRuleSpec`] — no IronCalc type crosses the seam.
    AddCondFmt {
        sheet: SheetId,
        range: String,
        spec: CfRuleSpec,
    },
    /// Replace the CF rule at storage `index` with `spec` over `new_range` (the column analog of
    /// [`Command::AddCondFmt`] for an existing rule). For a highlight rule the engine merges the new
    /// [`CfFormat`](freecell_core::CfFormat) onto the rule's existing `Dxf`, preserving unmodeled
    /// differential attributes (`components/engine_cf.md §4`).
    UpdateCondFmt {
        sheet: SheetId,
        index: u32,
        range: String,
        spec: CfRuleSpec,
    },
    /// Delete the CF rule at storage `index` on `sheet`.
    DeleteCondFmt { sheet: SheetId, index: u32 },
    /// Raise the priority of the CF rule at storage `index` (swap with the next-higher rule; a
    /// boundary raise is a no-op that records nothing).
    RaiseCondFmtPriority { sheet: SheetId, index: u32 },
    /// Lower the priority of the CF rule at storage `index` (the mirror of
    /// [`Command::RaiseCondFmtPriority`]).
    LowerCondFmtPriority { sheet: SheetId, index: u32 },
    /// Serialize + atomically save to `path` — replied via `Saved` / `SaveFailed`.
    Save { path: PathBuf, req_id: u64 },
    /// Export `sheet`'s used range to `path` as a `.csv` (`functional_spec.md §2`, D2.2 — raw
    /// stored values). A **pure read**: it never touches the model, `ops_seen`, or the undo stack,
    /// so it can't change the document's dirty flag. Replied via `CsvExported` / `CsvExportFailed`.
    ExportCsv {
        sheet: SheetId,
        path: PathBuf,
        req_id: u64,
    },
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
    /// A **fill** (⌘D / ⌘R / drag-fill) was blocked because its target intersects a merged region
    /// (merged-cell-ui `functional_spec.md F6` documented limitation — fill into a merge isn't a
    /// supported edit target, matching the engine's covered-cell write rejection). Insert/delete
    /// near a merge is **no longer** blocked (the engine now displaces merges), so this reason is
    /// now fill-only. Carries no payload (fixed message).
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
    /// Reply to [`Command::SelectionStats`]: the aggregate over the selection's populated cells.
    /// The chrome keeps it only when `req_id` still matches its latest request (`functional_spec.md
    /// §1`).
    SelectionStats { req_id: u64, stats: SelectionStats },
    /// Reply to [`Command::ResolveEdge`]: the edge-of-data `target` cell (`functional_spec.md §4`).
    /// The grid applies it only when `req_id` still matches its latest jump — collapsing to
    /// `single(target)` for `JumpEdge` or keeping the anchor for `ExtendEdge`.
    EdgeResolved { req_id: u64, target: CellRef },
    /// Reply to [`Command::Find`]: the matching cells in **row-major** order (empty = no matches).
    /// The UI stores these, selects + reveals the current one, and drives the "N of M" counter
    /// (`functional_spec.md §4.3`).
    FindResults { matches: Vec<CellRef> },
    /// Reply to [`Command::ReplaceOne`] / [`Command::ReplaceAll`]: how many cells were changed
    /// (`functional_spec.md §4.4` — "Replaced 7"; `0` when nothing matched).
    ReplacedCount { n: usize },
    /// Reply to `Save`: success, acking the op-index the file now contains.
    Saved { req_id: u64, ops_seen: u64 },
    /// Reply to `Save`: failure (typed; the original file is untouched — atomic save).
    SaveFailed { req_id: u64, error: SaveError },
    /// Reply to [`Command::ExportCsv`]: the `.csv` was written. Carries no state — export is a side
    /// output (the document's dirty flag / path / title are unchanged, `functional_spec.md §2`).
    CsvExported { req_id: u64 },
    /// Reply to [`Command::ExportCsv`]: the export failed (typed; any existing file is untouched —
    /// atomic write). Surfaced by the standard save-error dialog.
    CsvExportFailed { req_id: u64, error: SaveError },
    /// An edit was refused (cap re-check, name validation, caught panic, or degraded).
    EditRejected { reason: EditRejectedReason },
    /// A [`Command::MergeCells`] with `confirmed: false` would discard covered content — the UI
    /// must confirm before it applies (merged-cell-ui `functional_spec.md F3`, `architecture.md
    /// §8`). Nothing was mutated. The window opens the data-loss confirm dialog; **Merge** re-sends
    /// `MergeCells { area, confirmed: true }`, **Cancel** dismisses.
    MergeNeedsConfirm { sheet: SheetId, area: CellRange },
    /// The style/geometry cache for `sheet` changed (deltas shipped via the shared cache).
    /// Defined now for the seam; **emitted in Phase 5** when the cache logic lands.
    StyleCacheUpdated { sheet: SheetId },
    /// The conditional-formatting rule list for `sheet` changed (add / update / delete / reorder, a
    /// structural CF-range shift, or the undo/redo of one). The published CF map has already been
    /// refreshed; the window re-reads [`DocumentClient::cond_fmt_rules`](super::client::DocumentClient::cond_fmt_rules)
    /// and rebuilds the sidebar rows (`architecture.md §4.2`, `components/engine_cf.md §5`).
    CondFmtUpdated { sheet: SheetId },
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
