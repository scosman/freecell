//! The worker's **chart** half: the published [`ChartSnapshot`], the authored-chart store, the
//! chart undo timeline, and every chart command the worker applies (charts/architecture §4.1, §5).
//!
//! Charts ride the **same lock-free snapshot path as cells**, not a bespoke channel: the worker
//! stores a `ChartSnapshot` into an [`ArcSwap`](arc_swap::ArcSwap) in
//! [`Shared`](super::client) and signals it with the *existing*
//! [`WorkerEvent::Published`](super::WorkerEvent) — the one that already fires per edit. The UI
//! loads the snapshot wait-free on each `Published` (and on `Loaded`) and installs it only when the
//! [`version`](ChartSnapshot::version) changed, so a scroll-only publish — or an edit that touches
//! no chart — never re-installs.
//!
//! The `impl Worker` block here is the chart machinery **extracted from `run.rs`** (F1): it was
//! ~1,000 production lines of a 3,984-line file whose subject already had this module. Nothing about
//! it changed in the move — same commands, same events, same ordering. `run.rs` keeps the loop,
//! the publication and the save; this file keeps everything chart-shaped, including the pieces
//! `save_workbook` calls into (`ensure_all_charts_discovered`, `authored_write_list`,
//! `sheet_name_of`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use freecell_chart_model::{
    Anchor, CfRange, Chart, ChartColor, ChartId, ChartInsertKind, ChartKind, ChartSpec, Color,
    Legend,
};
use freecell_core::{CellRange, CellRef, SheetId};

use crate::chart::binding::{
    binding_from_refs, binding_is_dirty, build_series_shells, resolve_chart, CellData,
    RemovedChart, SheetResolver,
};
use crate::chart::write::{AuthoredChart, SeriesRefs};

use super::protocol::{ChartAxisKind, ChartChromeEdit, EditRejectedReason, WorkerEvent};
use super::run::{resolve_idx, StagedCommit, UndoEntry, Worker};

/// The published set of live-bound charts, grouped by the sheet they're anchored on. Each
/// [`ChartSpec`]'s `chart` field carries the **current** (live-resolved) values; the rest of the
/// envelope (retained source, anchor, origin) is unchanged from load, so the UI derives fidelity and
/// places the chart exactly as in P8.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartSnapshot {
    /// Bumped **only** when the bound charts change (on load, and on each dirty re-resolve). The UI
    /// installs into the grid iff this differs from what it last installed. The empty initial
    /// snapshot is version `0`; a file with charts publishes version `1` on load.
    ///
    /// Deliberately **not** the generation: making it one would re-install every chart on every
    /// scroll republish and destroy the "off-screen free" property (charts/architecture §5
    /// challenge 5).
    pub version: u64,
    /// The generation this snapshot was committed at (E1, `functional_spec.md F4.3`) — the chart
    /// surface's answer to "what does the UI see at generation N".
    ///
    /// It needs its own stamp because, unlike the style cache and the CF map, this surface is an
    /// [`ArcSwap`](arc_swap::ArcSwap): a reader takes no lock, so there is no release edge to reason
    /// from except the one the payload carries. Written by [`Worker::commit`] before the bump; read
    /// by the ordering tests, not by the UI (which gates on [`version`](Self::version)).
    pub generation: u64,
    /// Charts to paint, keyed by their anchor [`SheetId`]. Each per-sheet list is an
    /// `Arc<[ChartSpec]>` so the UI installs it into `GridView::set_sheet_charts` by **sharing the
    /// same allocation** (a refcount bump, not a deep copy) — the grid holds no independent copy of
    /// its charts' render pictures or retained source (charts/architecture §5 challenge 5,
    /// "off-screen free").
    pub sheets: Vec<(SheetId, Arc<[ChartSpec]>)>,
}

impl ChartSnapshot {
    /// The pre-load / chart-less snapshot (version `0`, no charts).
    pub fn empty() -> Self {
        Self::default()
    }
}

/// One **authored** chart the worker holds (P17, charts/write-path §1 mode 3). Distinct from the
/// loaded [`ChartBindings`]: an authored chart is **snapshot-but-not-live** — it rides the published
/// [`ChartSnapshot`] so the grid renders it, but it carries no `c:f` binding yet (ranges arrive in
/// P19), so it is **never** touched by the dirty-set re-resolve and is saved by the
/// **write-from-model** path ([`write::write_authored_charts`](crate::chart::write::write_authored_charts)),
/// never the loaded re-inject. `spec.origin` is always [`Authored`](freecell_chart_model::Origin::Authored).
#[derive(Clone)]
pub(super) struct AuthoredEntry {
    /// The worksheet the chart is anchored on (keys the published snapshot; resolved to the current
    /// worksheet name at save time, so an in-session rename follows and a deleted host drops it).
    anchor_sheet: SheetId,
    /// The stable manipulation handle (P18) the worker stamps onto the published spec, so the app
    /// can name this authored chart back for move/resize/delete.
    id: ChartId,
    /// The authored render envelope (a `ChartSpec::authored`, no retained source).
    spec: ChartSpec,
    /// The per-series `c:f` references once a **data range** is set (P19) — **empty** for a still
    /// near-empty placeholder. This is the source of truth for a bound authored chart: its live
    /// re-resolve derives a `ChartBinding` from it ([`binding_from_refs`]), and the write path
    /// consumes it directly so the saved chart carries `c:f` + caches (not literals). Setting a range
    /// (or switching type on a bound chart) rebuilds these; the chart becomes LIVE the moment it is
    /// non-empty.
    refs: Vec<SeriesRefs>,
}

/// The stashed inverse (and redo) of one chart op — enough whole worker state to both revert it and
/// re-apply it. Deliberately snapshot-based (clone the affected entry) rather than delta-based:
/// simple + obviously correct over lean. The heavy snapshots ([`AuthoredEntry`] / [`RemovedChart`])
/// are boxed so the enum stays small (an undo stack is cold, so the indirection is free).
pub(super) enum ChartUndo {
    /// Insert of an **authored** chart at list `index` (the born-live `entry` stashed whole, so a
    /// redo re-inserts it with its Batch-3 range binding intact — no re-derivation). Undo removes
    /// index; redo re-inserts `entry` at index.
    InsertAuthored {
        index: usize,
        entry: Box<AuthoredEntry>,
    },
    /// Delete of an **authored** chart. Undo re-inserts `entry` at `index`; redo removes index.
    DeleteAuthored {
        index: usize,
        entry: Box<AuthoredEntry>,
    },
    /// Delete of a **loaded** chart. `removed` is the whole binding (so undo re-binds it exactly);
    /// `chart_part` is the save-drop key the delete added to `loaded_deletes`; `prior_anchor_edit`
    /// is the `loaded_anchor_edits` value the delete evicted (restored on undo). Undo re-binds +
    /// clears the save-set bookkeeping; redo re-runs the delete effects.
    DeleteLoaded {
        removed: Box<RemovedChart>,
        chart_part: String,
        prior_anchor_edit: Option<Anchor>,
    },
    /// Anchor move/resize of an **authored** chart: swap `prior` and `applied` on the chart's model
    /// anchor.
    SetAnchorAuthored {
        id: ChartId,
        prior: Anchor,
        applied: Anchor,
    },
    /// Anchor move/resize of a **loaded** chart: swap the render anchor AND the `loaded_anchor_edits`
    /// entry. `prior_render` is the render anchor before the move; `prior_edit` is the
    /// `loaded_anchor_edits` value the move replaced (restored on undo).
    SetAnchorLoaded {
        id: ChartId,
        chart_part: String,
        prior_render: Anchor,
        prior_edit: Option<Anchor>,
        applied: Anchor,
    },
    /// Range bind of an **authored** chart (P19): the whole pre-bind `prior` entry and post-bind
    /// `applied` entry are stashed, so undo/redo just restore the clone (no re-derivation).
    SetRangeAuthored {
        index: usize,
        prior: Box<AuthoredEntry>,
        applied: Box<AuthoredEntry>,
    },
}

impl Worker {
    /// Re-resolve the charts whose source ranges the edit touched, and store a fresh
    /// [`ChartSnapshot`] iff any changed (P9, charts/architecture §4.1, §5 challenge 2). The dirty
    /// set is the range→chart index intersected with the edit's `refresh` cells (+ any structurally
    /// `rebuilt` data sheet); only those charts read live values. Runs **before** the `Published`
    /// bump so the fresh charts ride the same event that repaints the cells. Cheap when nothing
    /// intersects — a disjoint edit does no reads and leaves the snapshot untouched.
    pub(super) fn reresolve_charts(
        &mut self,
        refresh: &[(SheetId, CellRange)],
        rebuilt: &[SheetId],
    ) {
        // A bound authored chart (P19) rides the SAME dirty-set/re-resolve path as a loaded one, so
        // an edit re-renders it too — even in a workbook that has *only* authored charts (no loaded
        // set). Bail only when there is nothing bound to re-resolve.
        let any_authored_bound = self.authored_charts.iter().any(|e| !e.refs.is_empty());
        if self.charts.is_empty() && !any_authored_bound {
            return;
        }
        // A `c:f` sheet name → stable id against the current model. Owned (`move`), so it never
        // borrows `self` while the chart sets are mutated below.
        let props = self.doc.sheet_properties();
        let resolve_sheet = move |name: &str| -> Option<SheetId> {
            props
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| SheetId(*id))
        };
        let mut changed = false;

        // Loaded charts (P9): intersect the range→chart index, re-resolve only the dirty ones.
        if !self.charts.is_empty() {
            let indices = self.charts.dirty_indices(refresh, rebuilt, &resolve_sheet);
            if !indices.is_empty() {
                // Live cell reader over the doc — a disjoint field borrow from `self.charts` below.
                let doc = &self.doc;
                let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
                    match resolve_idx(doc, sheet) {
                        Ok(idx) => doc.cell_value(idx, cell),
                        Err(_) => CellData::Empty,
                    }
                };
                if self.charts.reresolve(&indices, &resolve_sheet, &read_cell) {
                    changed = true;
                }
            }
        }

        // Authored charts (P19): re-resolve any bound authored chart the edit touched, so a range set
        // in the panel behaves exactly like a loaded chart's live binding.
        if any_authored_bound {
            changed |= self.reresolve_authored(refresh, rebuilt, &resolve_sheet);
        }

        if changed {
            self.chart_version += 1;
            self.store_chart_snapshot();
        }
    }

    /// Capture the **stable** `SheetId → file worksheet part` map at open (P11 CR fix): the file's
    /// `workbook.xml.rels` name→part map joined with the model's **at-open** sheet names (which still
    /// match the file). No chart XML is parsed — this is the tiny, eager half of "lazy parse off the
    /// critical path"; the heavy chart XML still defers to first paint. A read failure yields an
    /// empty map (the workbook opens chart-less rather than failing) and is logged.
    ///
    /// **Join assumption:** the map is built by **exact name-equality** between the file's
    /// `workbook.xml` `<sheet name>` and the model's at-open `sheet_properties()` name — i.e. it
    /// assumes IronCalc loads sheet names byte-identical to the file's `<sheets>` (true at open;
    /// both derive from the same `workbook.xml`). A sheet whose name fails to join is filter-mapped
    /// out, so its charts degrade to **chart-less** (never discovered/saved) rather than
    /// mis-anchored — matching the "workbook open never breaks on charts" invariant.
    pub(super) fn build_chart_sheet_part_map(&self, path: &Path) -> HashMap<SheetId, String> {
        let file_parts = match crate::chart::workbook_sheet_parts(path) {
            Ok(parts) => parts,
            Err(err) => {
                tracing::warn!("chart sheet-part map unreadable; opening chart-less: {err:#}");
                return HashMap::new();
            }
        };
        let props = self.doc.sheet_properties();
        file_parts
            .into_iter()
            .filter_map(|(name, part)| {
                props
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(id, _)| (SheetId(*id), part))
            })
            .collect()
    }

    /// Walk + bind `sheet`'s charts the first time it is painted (P11 lazy discovery,
    /// charts/architecture §5 challenge 5). Runs after the viewport publish, so the parse is off the
    /// first-paint critical path — the cells are already on screen; the charts ride the **next**
    /// `Published`, exactly as a live re-resolve does (P9). Keyed on the sheet's **stable file part**
    /// (via [`chart_sheet_parts`](Self::chart_sheet_parts)), NOT its live name, so a sheet renamed
    /// before it is painted still resolves to its charts (P11 CR fix). A no-op once the sheet has been
    /// walked, once every sheet has been discovered, or for a non-file / in-session-added sheet.
    pub(super) fn ensure_sheet_charts_discovered(&mut self, sheet: SheetId) {
        if self.charts_fully_discovered {
            return;
        }
        let Some(path) = self.chart_source_path.clone() else {
            return; // never opened from a file → nothing to discover
        };
        if !self.discovered_chart_sheets.insert(sheet) {
            return; // already walked this sheet (walk each at most once)
        }
        let Some(part) = self.chart_sheet_parts.get(&sheet).cloned() else {
            return; // not a file worksheet (added in-session) → no file charts
        };
        match crate::chart::discover_and_parse_for_part(&path, &part) {
            Ok(specs) => {
                if self.bind_discovered(sheet, specs) {
                    self.chart_version += 1;
                    self.store_chart_snapshot();
                    self.commit(StagedCommit::default());
                }
            }
            Err(err) => tracing::warn!(%part, "lazy chart discovery failed: {err:#}"),
        }
    }

    /// Bind the charts `specs` discovered on `sheet`, skipping any deleted in-session (P18 — so a
    /// save-time full sweep can't resurrect a deleted loaded chart), and — when new charts were
    /// bound — stamp their stable [`ChartId`]s. Returns whether anything was added.
    pub(super) fn bind_discovered(
        &mut self,
        sheet: SheetId,
        specs: crate::chart::load::SheetCharts,
    ) -> bool {
        let specs: crate::chart::load::SheetCharts = specs
            .into_iter()
            .filter(|(part, _)| !self.loaded_deletes.contains(part))
            .collect();
        if self.charts.add_missing(vec![(sheet, specs)]) {
            self.charts.assign_missing_ids(&mut self.next_chart_id);
            true
        } else {
            false
        }
    }

    /// Discover + bind **every** file worksheet's charts (P11), so a chart-preserving save never
    /// drops a chart whose sheet the user never painted. Runs once at the top of
    /// [`save_workbook`](Self::save_workbook); a no-op after the first full sweep. Iterates the
    /// **stable** `SheetId → file part` map, so each chart binds to its real `SheetId` regardless of
    /// any in-session rename — a renamed host's chart follows the rename, a deleted host's `SheetId`
    /// no longer resolves so `live_sheet_targets` drops it (the P10 delete outcome), and the
    /// active-sheet-fallback mis-anchoring bug is impossible. Merges through
    /// [`add_missing`](ChartBindings::add_missing), so charts already bound lazily (and their
    /// live-resolved values) are kept untouched. A discovery failure is logged (the save then
    /// proceeds with whatever was already bound, rather than aborting the user's save).
    ///
    /// **Re-entrant across a panic** (B1, `functional_spec.md F2.2`). This runs inside the save's
    /// `catch_unwind`, and its per-sheet `discover_and_parse_for_part` is a zip/XML parse of
    /// user-supplied bytes — the most panic-prone call on the save path. A sheet is therefore
    /// recorded in [`discovered_chart_sheets`](Self::discovered_chart_sheets) only **after** its
    /// parse returned; a panic mid-sweep leaves every unprocessed sheet still eligible for lazy
    /// discovery. Marking before the parse (as this did) left sheets flagged "walked" whose charts
    /// were never bound: [`ensure_sheet_charts_discovered`](Self::ensure_sheet_charts_discovered)
    /// then returned early for them forever while `charts_fully_discovered` stayed `false`, so
    /// their charts silently went missing until some later save re-ran the whole sweep.
    pub(super) fn ensure_all_charts_discovered(&mut self) {
        if self.charts_fully_discovered {
            return;
        }
        if self.chart_source_path.is_none() {
            self.charts_fully_discovered = true;
            return; // never opened from a file → nothing to discover
        }
        let path = self.chart_source_path.clone().expect("checked Some above");
        // Snapshot the stable map so we don't borrow `self` while binding into `self.charts`.
        let sheet_parts: Vec<(SheetId, String)> = self
            .chart_sheet_parts
            .iter()
            .map(|(id, part)| (*id, part.clone()))
            .collect();
        let mut added = false;
        for (sheet, part) in sheet_parts {
            // Panic injection for this sweep's re-entrancy test (B1) — a chart source named
            // `PANIC_SENTINEL` panics HERE, standing in for a zip/XML parse that blows up on
            // hostile bytes. Placed at the parse call because that is what the loop has to be
            // re-entrant across; `#[cfg(test)]`, like `document::PANIC_SENTINEL`'s other uses.
            #[cfg(test)]
            if path
                .file_name()
                .is_some_and(|n| n == crate::document::PANIC_SENTINEL)
            {
                panic!("injected test panic (save-time chart discovery sweep)");
            }
            let parsed = crate::chart::discover_and_parse_for_part(&path, &part);
            // Marked only once the parse has RETURNED (either way): a panic inside it must leave
            // this sheet re-discoverable — see the re-entrancy note on this function. A parse that
            // failed cleanly is still marked, so the lazy path does not re-attempt (and re-log) a
            // failure that is a property of the file.
            self.discovered_chart_sheets.insert(sheet);
            match parsed {
                Ok(specs) if !specs.is_empty() => {
                    if self.bind_discovered(sheet, specs) {
                        added = true;
                    }
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(%part, "chart discovery for save failed: {err:#}"),
            }
        }
        self.charts_fully_discovered = true;
        if added {
            self.chart_version += 1;
            self.store_chart_snapshot();
            self.commit(StagedCommit::default());
        }
    }

    /// The current name of the worksheet with stable id `sheet` (against the live model), or `None`
    /// if that sheet no longer exists (deleted in-session). The rename-safe key the chart-preserving
    /// save resolves each chart's host worksheet through.
    pub(super) fn sheet_name_of(&self, sheet: SheetId) -> Option<String> {
        self.doc
            .sheet_properties()
            .into_iter()
            .find(|(id, _)| SheetId(*id) == sheet)
            .map(|(_, name)| name)
    }

    /// The authored charts as [`AuthoredChart`]s for the write-from-model save, resolving each one's
    /// host worksheet name (dropping a chart whose host sheet was deleted in-session, like a loaded
    /// chart) and assigning it a **free** `xl/charts/chartN.xml` part — one that collides with
    /// neither an existing part in `package_bytes` (loaded charts already re-injected) nor another
    /// authored chart. A **ranged** chart (P19) carries its per-series `c:f`
    /// [`refs`](AuthoredEntry::refs), so the serializer emits `numRef`/`strRef` + caches (fully
    /// cell-bound, live-binds like a loaded chart on reopen); a still near-empty placeholder carries
    /// empty `refs`, so the serializer emits schema-valid literals.
    pub(super) fn authored_write_list(&self, package_bytes: &[u8]) -> Vec<AuthoredChart> {
        let mut used = existing_chart_parts(package_bytes);
        let mut out = Vec::new();
        for entry in &self.authored_charts {
            let Some(sheet_name) = self.sheet_name_of(entry.anchor_sheet) else {
                tracing::warn!("dropping an authored chart whose host worksheet was deleted");
                continue;
            };
            let Some(chart) = entry.spec.chart().cloned() else {
                continue; // an authored chart always has a typed Chart; defensive
            };
            out.push(AuthoredChart {
                sheet_name,
                chart_part: next_chart_part(&mut used),
                chart,
                anchor: entry.spec.anchor,
                refs: entry.refs.clone(),
            });
        }
        out
    }

    /// Insert an **authored** chart of `kind` onto `sheet` at `anchor` (P17, charts/ui_design §3.1).
    /// A degraded worker rejects it (like every mutating op). Otherwise it builds the template chart
    /// and holds it as an Authored [`ChartSpec`], marks the document dirty, and republishes the chart
    /// snapshot so the window's `sync_charts` installs it into the grid. When `data` is `Some` (the
    /// action bar captured a real selection — Batch 3 item 8) the chart is **bound at creation** via
    /// [`bind_authored_range_at`](Self::bind_authored_range_at), so it is born LIVE; when `None` it
    /// stays snapshot-but-not-live (no `c:f` binding, so it never enters the dirty-set re-resolve
    /// until a range is set in P19).
    pub(super) fn insert_authored_chart(
        &mut self,
        sheet: SheetId,
        kind: ChartInsertKind,
        anchor: Anchor,
        data: Option<CellRange>,
    ) {
        // A degraded worker refuses edits (consistent with the edit batch / paste / SetFont).
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // The host sheet must exist (the UI only ever sends the active sheet; this is a backstop).
        if self.resolve(sheet).is_none() {
            tracing::warn!(sheet = sheet.0, "InsertChart onto a missing sheet ignored");
            return;
        }
        let id = ChartId(self.next_chart_id);
        self.next_chart_id += 1;
        self.authored_charts.push(AuthoredEntry {
            anchor_sheet: sheet,
            id,
            spec: ChartSpec::authored(kind.near_empty_chart(), anchor),
            // A freshly inserted chart carries placeholder literals — no `c:f` binding until a range
            // is set (P19). Empty `refs` keeps it snapshot-but-not-live, saved as literals.
            refs: Vec::new(),
        });
        // Post-v1 Batch 3, item 8: if the action bar captured a real selection at insert time, bind
        // it right now so the chart is born LIVE (real `c:f` refs + resolved values) — same block→
        // series binding as `SetChartRange`, on the id we just assigned. The data lives on the insert
        // `sheet` (the selection's own — anchor — sheet). A `None` selection stays near-empty.
        let pos = self.authored_charts.len() - 1;
        if let Some(data) = data {
            if let Some(data_sheet_name) = self.sheet_name_of(sheet) {
                self.bind_authored_range_at(pos, sheet, &data_sheet_name, data);
            }
        }
        // **Undo timeline (charts feedback item 4, reversing the earlier P18 "charts off Ctrl+Z"
        // decision):** an insert now pushes onto the unified undo stack. We stash the FINAL entry
        // (after any born-live range bind above) so a redo re-inserts it whole — refs/series intact,
        // no re-derivation. A chart entry never touches IronCalc's own undo stack, so the `Cell`
        // entries stay 1:1 with it (no desync). `ops_seen` still counts the op for the dirty flag.
        let entry = Box::new(self.authored_charts[pos].clone());
        self.push_chart_undo(ChartUndo::InsertAuthored { index: pos, entry });
        // Publish the new chart on the same seam the loaded charts ride, so the window installs it.
        self.commit_chart_op();
    }

    /// Move/resize a chart (P18, `Command::SetChartAnchor`): set the chart named by `id` to `anchor`.
    /// Degraded-guarded like every mutating op. An **authored** chart's model anchor is rewritten
    /// (the write-from-model save re-synthesizes its drawing there); a **loaded** chart's render
    /// anchor is updated AND recorded in `loaded_anchor_edits` so the source-first save patches its
    /// retained `twoCellAnchor`. Republishes the chart snapshot so the grid repaints at the new rect.
    pub(super) fn set_chart_anchor(&mut self, _sheet: SheetId, id: ChartId, anchor: Anchor) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // Authored first (its ids never collide with loaded ones — one shared counter). Each branch
        // stashes the prior placement onto the undo timeline (charts feedback item 4) before the move.
        if let Some(entry) = self.authored_charts.iter_mut().find(|e| e.id == id) {
            let prior = entry.spec.anchor;
            entry.spec.anchor = anchor;
            self.push_chart_undo(ChartUndo::SetAnchorAuthored {
                id,
                prior,
                applied: anchor,
            });
        } else if let Some(prior_render) = self.charts.anchor_by_id(id) {
            let chart_part = self
                .charts
                .set_anchor_by_id(id, anchor)
                .expect("id resolved by anchor_by_id");
            // `insert` returns the value it replaced — the prior `loaded_anchor_edits` state to
            // restore on undo (a chart moved twice in-session has a prior edit; a first move has None).
            let prior_edit = self.loaded_anchor_edits.insert(chart_part.clone(), anchor);
            self.push_chart_undo(ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied: anchor,
            });
        } else {
            tracing::warn!(id = id.0, "SetChartAnchor for an unknown chart id ignored");
            return;
        }
        self.commit_chart_op();
    }

    /// Delete a chart (P18, `Command::DeleteChart`): drop the chart named by `id`. Degraded-guarded.
    /// An **authored** chart is removed from the authored set; a **loaded** chart is unbound and its
    /// `chart_part` recorded in `loaded_deletes` so the source-first save drops it from the package
    /// (its `twoCellAnchor` + part chain) — and the save-time discovery sweep skips it so it can't be
    /// re-bound. Republishes so the grid drops it. Both provenances stash enough state to **undo** the
    /// delete (charts feedback item 4): the removed authored entry, or the whole removed loaded
    /// binding + the save-set bookkeeping it changed.
    pub(super) fn delete_chart(&mut self, _sheet: SheetId, id: ChartId) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        if let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) {
            let entry = Box::new(self.authored_charts.remove(pos));
            self.push_chart_undo(ChartUndo::DeleteAuthored { index: pos, entry });
        } else if let Some(removed) = self.charts.take_by_id(id) {
            let chart_part = removed.chart_part().to_string();
            // `remove` returns the anchor-edit the delete evicts — restored on undo.
            let prior_anchor_edit = self.loaded_anchor_edits.remove(&chart_part);
            self.loaded_deletes.insert(chart_part.clone());
            self.push_chart_undo(ChartUndo::DeleteLoaded {
                removed: Box::new(removed),
                chart_part,
                prior_anchor_edit,
            });
        } else {
            tracing::warn!(id = id.0, "DeleteChart for an unknown chart id ignored");
            return;
        }
        self.commit_chart_op();
    }

    /// Set an **authored** chart's data range (P19, `Command::SetChartRange`): give the chart named by
    /// `id` real `c:f` refs derived from the `data` block on `sheet` (the sheet the data lives on —
    /// not necessarily the chart's host sheet; the chart is found by `id`), rebuild its series in the
    /// kind's data shape, and re-resolve their values from the current cells — so it transitions from
    /// P17's snapshot-but-not-live placeholder to a **LIVE** chart (re-renders on edit,
    /// `reresolve_authored`; saves with `c:f` + caches, `authored_write_list`). Degraded-guarded; a
    /// loaded/unknown id is ignored (loaded re-range is P20's source-patch territory).
    pub(super) fn set_chart_range(&mut self, sheet: SheetId, id: ChartId, data: CellRange) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(data_sheet_name) = self.sheet_name_of(sheet) else {
            tracing::warn!(
                sheet = sheet.0,
                "SetChartRange onto a missing sheet ignored"
            );
            return;
        };
        let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) else {
            tracing::warn!(
                id = id.0,
                "SetChartRange for a non-authored/unknown chart ignored (loaded re-range is P20)"
            );
            return;
        };
        // Stash the whole pre-bind entry (refs/series/source_ranges) and the post-bind entry, so the
        // undo timeline can restore either clone without re-deriving (charts feedback item 4).
        let prior = Box::new(self.authored_charts[pos].clone());
        self.bind_authored_range_at(pos, sheet, &data_sheet_name, data);
        let applied = Box::new(self.authored_charts[pos].clone());
        self.push_chart_undo(ChartUndo::SetRangeAuthored {
            index: pos,
            prior,
            applied,
        });
        self.commit_chart_op();
    }

    /// Bind the authored chart at index `pos` to the `data` block on `data_sheet` (named
    /// `data_sheet_name`): derive its `c:f` refs, rebuild its series shells in the current kind's data
    /// shape, and re-resolve their values from the current cells — turning a near-empty placeholder
    /// into a **LIVE** chart. The shared body of `SetChartRange` (P19) and the range-at-insert path
    /// (Batch 3 item 8). Does **not** publish — the caller commits/publishes once.
    pub(super) fn bind_authored_range_at(
        &mut self,
        pos: usize,
        data_sheet: SheetId,
        data_sheet_name: &str,
        data: CellRange,
    ) {
        let Some(mut template) = self.authored_charts[pos].spec.chart().cloned() else {
            return; // an authored chart always has a typed Chart; defensive
        };
        let refs = crate::chart::series_refs_from_block(data_sheet_name, data);
        let binding = binding_from_refs(&refs);
        // The range keeps the current type, so the series data shape is unchanged — derive the shape
        // from the current kind (xy for scatter, xy+size for bubble, else category/value) the same
        // way `set_chart_type` does (`ChartInsertKind::series_shape`), not with an ad-hoc `matches!`.
        let shape = ChartInsertKind::from_chart_kind(&template.kind)
            .map(|k| k.series_shape())
            .unwrap_or(freecell_chart_model::SeriesShape::CategoryValue);
        template.series = build_series_shells(refs.len(), shape);
        let resolved = self.resolve_authored_chart(data_sheet, &template, &binding);
        let source_ranges = source_ranges_from_refs(&refs);

        let entry = &mut self.authored_charts[pos];
        if let Some(slot) = entry.spec.chart_mut() {
            *slot = resolved;
        }
        entry.spec.source_ranges = source_ranges;
        entry.refs = refs;
    }

    /// Switch an **authored** chart's type (P19, `Command::SetChartType`): rebuild the chart named by
    /// `id` to `kind`, preserving its title and — if it is already **bound** to a data range — its
    /// `c:f` refs (rebuilding the series in the new kind's data shape and re-resolving live). An
    /// unbound (still near-empty) chart is swapped to that kind's placeholder template, keeping the
    /// title. Degraded-guarded; a loaded/unknown id is ignored.
    pub(super) fn set_chart_type(&mut self, sheet: SheetId, id: ChartId, kind: ChartInsertKind) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) else {
            tracing::warn!(
                id = id.0,
                "SetChartType for a non-authored/unknown chart ignored"
            );
            return;
        };
        let title = self.authored_charts[pos]
            .spec
            .chart()
            .and_then(|c| c.title.clone());
        let refs = self.authored_charts[pos].refs.clone();

        if refs.is_empty() {
            // Unbound placeholder: swap to the new kind's near-empty template, keeping the title so a
            // pre-range retype doesn't reset the (only) field the user has set.
            let mut chart = kind.near_empty_chart();
            chart.title = title;
            if let Some(slot) = self.authored_charts[pos].spec.chart_mut() {
                *slot = chart;
            }
        } else {
            // Bound: keep the range refs + title/axes/legend, rebuild the series in the new kind's
            // data shape, and re-resolve their values from the current cells.
            let mut template = self.authored_charts[pos]
                .spec
                .chart()
                .cloned()
                .expect("authored chart has a typed Chart");
            template.kind = kind.chart_kind();
            template.series = build_series_shells(refs.len(), kind.series_shape());
            let binding = binding_from_refs(&refs);
            let resolved = self.resolve_authored_chart(sheet, &template, &binding);
            if let Some(slot) = self.authored_charts[pos].spec.chart_mut() {
                *slot = resolved;
            }
        }
        // Type change stays IMMEDIATE (not itself undoable — charts feedback item 4 covers only
        // insert/delete/anchor/range), but it is a new forward action, so it invalidates the redo
        // stack (a pending redo must not resurrect pre-retype state).
        self.redo_stack.clear();
        self.commit_chart_op();
    }

    /// Edit a chart's **chrome** (P20, `Command::SetChartChrome`): apply one chrome attribute change
    /// — title / legend / axis title / series color / data-label toggles — to the chart named by `id`,
    /// on **either** provenance. An **authored** chart's model is mutated (re-serialized on save); a
    /// **loaded** chart's retained render model is mutated (so it re-renders live) and its retained
    /// `chartN.xml` is source-patched on save (only the changed sub-element, preserving unmodeled
    /// styling — the edit contract). Degraded-guarded; an unknown/Unsupported id is ignored.
    pub(super) fn set_chart_chrome(&mut self, _sheet: SheetId, id: ChartId, edit: ChartChromeEdit) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // Authored first (its ids never collide with loaded ones — one shared counter). Chrome edits
        // stay IMMEDIATE (not themselves undoable — charts feedback item 4 covers only insert/delete/
        // anchor/range), but each is a new forward action, so it invalidates the redo stack.
        if let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) {
            if let Some(chart) = self.authored_charts[pos].spec.chart_mut() {
                apply_chrome_edit(chart, &edit);
                self.redo_stack.clear();
                self.commit_chart_op();
            }
            return;
        }
        // Loaded: mutate the bound chart's render model in place (its binding is untouched, so it
        // still live-re-resolves; the save patches its retained source).
        if self
            .charts
            .edit_chart_by_id(id, |chart| apply_chrome_edit(chart, &edit))
        {
            self.redo_stack.clear();
            self.commit_chart_op();
            return;
        }
        tracing::warn!(
            id = id.0,
            "SetChartChrome for an unknown/unsupported chart id ignored"
        );
    }

    /// Resolve an authored chart's series values from the **current** cells, given a `template` whose
    /// series shells are already in the right data shape and its `binding` (P19). A `&self` mirror of
    /// the [`reresolve_charts`](Self::reresolve_charts) closure setup, used by the range/type handlers
    /// to fill a freshly-rebuilt chart before it is published.
    pub(super) fn resolve_authored_chart(
        &self,
        anchor_sheet: SheetId,
        template: &Chart,
        binding: &crate::chart::ChartBinding,
    ) -> Chart {
        let props = self.doc.sheet_properties();
        let resolve_sheet = move |name: &str| -> Option<SheetId> {
            props
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| SheetId(*id))
        };
        let doc = &self.doc;
        let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
            match resolve_idx(doc, sheet) {
                Ok(idx) => doc.cell_value(idx, cell),
                Err(_) => CellData::Empty,
            }
        };
        resolve_chart(template, binding, anchor_sheet, &resolve_sheet, &read_cell)
    }

    /// Re-resolve every **bound** authored chart the edit touched (P19), in place — returns whether
    /// any authored chart's picture changed. Mirrors [`ChartBindings::reresolve`] for the authored
    /// set: an authored chart's `ChartBinding` is derived from its `refs` on demand (their single
    /// source of truth), so a dirty chart refreshes from the current cells through the shared
    /// [`resolve_chart`].
    pub(super) fn reresolve_authored(
        &mut self,
        refresh: &[(SheetId, CellRange)],
        rebuilt: &[SheetId],
        resolve_sheet: &SheetResolver<'_>,
    ) -> bool {
        let doc = &self.doc;
        let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
            match resolve_idx(doc, sheet) {
                Ok(idx) => doc.cell_value(idx, cell),
                Err(_) => CellData::Empty,
            }
        };
        let mut changed = false;
        for entry in &mut self.authored_charts {
            if entry.refs.is_empty() {
                continue; // a still near-empty placeholder has no binding to re-resolve
            }
            let binding = binding_from_refs(&entry.refs);
            let anchor_sheet = entry.anchor_sheet;
            if !binding_is_dirty(&binding, anchor_sheet, refresh, rebuilt, resolve_sheet) {
                continue;
            }
            let Some(template) = entry.spec.chart() else {
                continue;
            };
            let resolved =
                resolve_chart(template, &binding, anchor_sheet, resolve_sheet, &read_cell);
            if entry.spec.chart() != Some(&resolved) {
                if let Some(slot) = entry.spec.chart_mut() {
                    *slot = resolved;
                }
                changed = true;
            }
        }
        changed
    }

    /// Shared post-mutation bookkeeping for a chart op: count the committed op (dirty + savable),
    /// bump the chart version, re-store the snapshot, and commit. Deliberately does **not** touch
    /// the undo/redo stacks — the caller (a forward op via [`push_chart_undo`](Self::push_chart_undo),
    /// or an undo/redo via [`undo_chart_op`](Self::undo_chart_op)) owns the timeline — so it is reused
    /// verbatim by both the forward chart ops and the chart undo/redo republish.
    ///
    /// It goes through [`commit`](Worker::commit) rather than emitting `Published` directly (E1).
    /// It used to do the latter, which left `Shared::generation` standing still — a `Published` that
    /// announced a generation that never happened. The cost is one `build_publication` over the
    /// current viewport per chart gesture: the same cost class as a single scroll republish, and a
    /// chart op is a discrete user action (`ChartAnchorChanged` fires on mouse-up, not per drag
    /// frame), so it is not close.
    pub(super) fn commit_chart_op(&mut self) {
        self.ops_seen += 1;
        self.shared
            .committed_ops
            .store(self.ops_seen, Ordering::Release);
        self.chart_version += 1;
        self.store_chart_snapshot();
        self.commit(StagedCommit::default());
    }

    /// Push a chart op onto the unified undo timeline (charts feedback item 4) and clear the redo
    /// stack — a new forward action always invalidates redo. Called by the four undoable chart ops
    /// (insert / delete / anchor / range) just before [`commit_chart_op`](Self::commit_chart_op).
    pub(super) fn push_chart_undo(&mut self, cu: ChartUndo) {
        self.undo_stack.push(UndoEntry::Chart(cu));
        self.redo_stack.clear();
    }

    /// Undo a chart op: pop the `Chart` entry (the caller guaranteed one is on top), apply its
    /// inverse worker-side, push the counterpart onto the redo stack, and republish. Degraded-guarded
    /// like every mutating op — a degraded worker refuses (leaving the stacks untouched). Does NOT
    /// call IronCalc's `undo()`, so the `Cell` entries stay 1:1 with IronCalc's stack.
    pub(super) fn undo_chart_op(&mut self) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(UndoEntry::Chart(cu)) = self.undo_stack.pop() else {
            return; // caller guarantees a Chart entry is on top
        };
        let counterpart = self.undo_chart_entry(cu);
        self.redo_stack.push(UndoEntry::Chart(counterpart));
        self.commit_chart_op();
    }

    /// Redo a chart op: the mirror of [`undo_chart_op`](Self::undo_chart_op) over the redo stack.
    pub(super) fn redo_chart_op(&mut self) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(UndoEntry::Chart(cu)) = self.redo_stack.pop() else {
            return;
        };
        let counterpart = self.redo_chart_entry(cu);
        self.undo_stack.push(UndoEntry::Chart(counterpart));
        self.commit_chart_op();
    }

    /// Apply the **inverse** of one chart op (revert it), and return the [`ChartUndo`] to push onto
    /// the redo stack (the same variant — [`redo_chart_entry`](Self::redo_chart_entry) re-applies it
    /// forward). Snapshot-based, so each arm just restores or removes stashed whole state.
    pub(super) fn undo_chart_entry(&mut self, cu: ChartUndo) -> ChartUndo {
        match cu {
            // Undo an insert → remove it (redo re-inserts `entry`).
            ChartUndo::InsertAuthored { index, entry } => {
                if index < self.authored_charts.len() {
                    self.authored_charts.remove(index);
                }
                ChartUndo::InsertAuthored { index, entry }
            }
            // Undo a delete → re-insert `entry` at its old slot (redo removes it again).
            ChartUndo::DeleteAuthored { index, entry } => {
                let at = index.min(self.authored_charts.len());
                self.authored_charts.insert(at, (*entry).clone());
                ChartUndo::DeleteAuthored { index, entry }
            }
            // Undo a loaded delete → re-bind the chart, clear the save-set bookkeeping the delete
            // added, and restore the anchor-edit it evicted.
            ChartUndo::DeleteLoaded {
                removed,
                chart_part,
                prior_anchor_edit,
            } => {
                self.charts.reinsert_removed((*removed).clone());
                self.loaded_deletes.remove(&chart_part);
                match prior_anchor_edit {
                    Some(a) => {
                        self.loaded_anchor_edits.insert(chart_part.clone(), a);
                    }
                    None => {
                        self.loaded_anchor_edits.remove(&chart_part);
                    }
                }
                ChartUndo::DeleteLoaded {
                    removed,
                    chart_part,
                    prior_anchor_edit,
                }
            }
            ChartUndo::SetAnchorAuthored { id, prior, applied } => {
                if let Some(e) = self.authored_charts.iter_mut().find(|e| e.id == id) {
                    e.spec.anchor = prior;
                }
                ChartUndo::SetAnchorAuthored { id, prior, applied }
            }
            ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied,
            } => {
                self.charts.set_anchor_by_id(id, prior_render);
                match prior_edit {
                    Some(a) => {
                        self.loaded_anchor_edits.insert(chart_part.clone(), a);
                    }
                    None => {
                        self.loaded_anchor_edits.remove(&chart_part);
                    }
                }
                ChartUndo::SetAnchorLoaded {
                    id,
                    chart_part,
                    prior_render,
                    prior_edit,
                    applied,
                }
            }
            ChartUndo::SetRangeAuthored {
                index,
                prior,
                applied,
            } => {
                if index < self.authored_charts.len() {
                    self.authored_charts[index] = (*prior).clone();
                }
                ChartUndo::SetRangeAuthored {
                    index,
                    prior,
                    applied,
                }
            }
        }
    }

    /// Apply the **forward** of one chart op (re-apply it), and return the [`ChartUndo`] to push onto
    /// the undo stack — the mirror of [`undo_chart_entry`](Self::undo_chart_entry).
    pub(super) fn redo_chart_entry(&mut self, cu: ChartUndo) -> ChartUndo {
        match cu {
            // Redo an insert → re-insert `entry` at its slot.
            ChartUndo::InsertAuthored { index, entry } => {
                let at = index.min(self.authored_charts.len());
                self.authored_charts.insert(at, (*entry).clone());
                ChartUndo::InsertAuthored { index, entry }
            }
            // Redo a delete → remove it again.
            ChartUndo::DeleteAuthored { index, entry } => {
                if index < self.authored_charts.len() {
                    self.authored_charts.remove(index);
                }
                ChartUndo::DeleteAuthored { index, entry }
            }
            // Redo a loaded delete → re-run the delete effects: take the chart out again (a fresh
            // whole binding for a later undo), re-add its part to `loaded_deletes`, drop the
            // anchor-edit the undo restored.
            ChartUndo::DeleteLoaded {
                removed,
                chart_part,
                prior_anchor_edit,
            } => {
                let fresh = self.charts.take_by_id(removed.id());
                self.loaded_anchor_edits.remove(&chart_part);
                self.loaded_deletes.insert(chart_part.clone());
                // A fresh whole binding for a later undo (identical to `removed`; the chart wasn't
                // touched while re-bound), falling back to the stash if the take somehow missed.
                let removed = fresh.map(Box::new).unwrap_or(removed);
                ChartUndo::DeleteLoaded {
                    removed,
                    chart_part,
                    prior_anchor_edit,
                }
            }
            ChartUndo::SetAnchorAuthored { id, prior, applied } => {
                if let Some(e) = self.authored_charts.iter_mut().find(|e| e.id == id) {
                    e.spec.anchor = applied;
                }
                ChartUndo::SetAnchorAuthored { id, prior, applied }
            }
            ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied,
            } => {
                self.charts.set_anchor_by_id(id, applied);
                self.loaded_anchor_edits.insert(chart_part.clone(), applied);
                ChartUndo::SetAnchorLoaded {
                    id,
                    chart_part,
                    prior_render,
                    prior_edit,
                    applied,
                }
            }
            ChartUndo::SetRangeAuthored {
                index,
                prior,
                applied,
            } => {
                if index < self.authored_charts.len() {
                    self.authored_charts[index] = (*applied).clone();
                }
                ChartUndo::SetRangeAuthored {
                    index,
                    prior,
                    applied,
                }
            }
        }
    }

    /// Store the current bound charts as the published [`ChartSnapshot`] (charts/architecture §4.1),
    /// riding the same wait-free `arc_swap` container as the cell publication. Merges the loaded
    /// (live-bound) charts with the **authored** ones (P17) into per-sheet groups.
    pub(super) fn store_chart_snapshot(&self) {
        let sheets = if self.authored_charts.is_empty() {
            // Fast path (no authored charts): share the worker's `Arc<[ChartSpec]>` allocations
            // directly (P11 "off-screen free" — no per-publish deep copy of the loaded specs).
            self.charts.specs_by_sheet()
        } else {
            self.charts_by_sheet_with_authored()
        };
        // Stamped with the generation currently committed; the `commit` that follows re-stamps it
        // with the new one (E1). Truthful at every instant either way.
        self.shared.chart_snapshot.store(Arc::new(ChartSnapshot {
            version: self.chart_version,
            generation: self.shared.generation.load(Ordering::Acquire),
            sheets,
        }));
    }

    /// Re-store the current snapshot stamped with `generation` — the chart surface's half of the
    /// single commit (E1, `functional_spec.md F4.3`). Called by [`Worker::commit`] immediately
    /// before the generation bump, so a reader that observes generation `N` finds a chart snapshot
    /// at `N` (or later), never behind it.
    ///
    /// Cheap when the charts didn't move: `sheets` is a `Vec<(SheetId, Arc<[ChartSpec]>)>`, so the
    /// clone is a handful of refcount bumps, not a copy of any chart. [`version`](ChartSnapshot::version)
    /// is carried through untouched, so this never triggers a UI re-install.
    pub(super) fn stamp_chart_snapshot(&self, generation: u64) {
        let current = self.shared.chart_snapshot.load();
        if current.generation == generation {
            return;
        }
        self.shared.chart_snapshot.store(Arc::new(ChartSnapshot {
            version: current.version,
            generation,
            sheets: current.sheets.clone(),
        }));
    }

    /// The loaded specs (grouped by sheet) with each authored chart appended to its anchor sheet's
    /// group — the snapshot payload when the workbook carries authored charts. Loaded charts keep
    /// their discovery order; authored charts follow in insert order.
    pub(super) fn charts_by_sheet_with_authored(&self) -> Vec<(SheetId, Arc<[ChartSpec]>)> {
        let mut groups: Vec<(SheetId, Vec<ChartSpec>)> = self
            .charts
            .specs_by_sheet()
            .into_iter()
            .map(|(sheet, specs)| (sheet, specs.to_vec()))
            .collect();
        for entry in &self.authored_charts {
            // Stamp the authored chart's stable id (P18) so the app can manipulate it.
            let spec = entry.spec.clone().with_id(entry.id);
            match groups.iter_mut().find(|(s, _)| *s == entry.anchor_sheet) {
                Some((_, specs)) => specs.push(spec),
                None => groups.push((entry.anchor_sheet, vec![spec])),
            }
        }
        groups
            .into_iter()
            .map(|(sheet, specs)| (sheet, Arc::from(specs)))
            .collect()
    }
}

/// Apply one chrome edit to a render [`Chart`] (P20) — the pure mutation shared by the authored and
/// loaded chrome-edit paths (`set_chart_chrome`). A [`DataLabels`](ChartChromeEdit::DataLabels) edit
/// applies the show toggles across **every** series, preserving each series' existing label
/// number-format / separator / position (and any legend-key / series-name already shown), and clears
/// a series' labels to `None` only when nothing at all would show.
fn apply_chrome_edit(chart: &mut Chart, edit: &ChartChromeEdit) {
    match edit {
        ChartChromeEdit::Title(title) => chart.title = title.clone(),
        ChartChromeEdit::Legend(position) => {
            chart.legend = position.map(|position| Legend { position })
        }
        ChartChromeEdit::AxisTitle { axis, title } => {
            let ax = match axis {
                ChartAxisKind::Category => &mut chart.cat_axis,
                ChartAxisKind::Value => &mut chart.val_axis,
            };
            ax.title = title.clone();
        }
        ChartChromeEdit::SeriesColor { series, color } => {
            // A "series color" edit recolors the WHOLE series. Only a LINE or SCATTER series carries
            // its visible color on the `a:ln` stroke, which the renderer prefers over `color`
            // (line.rs / scatter.rs: `stroke.color.or(color)`). Leaving the stroke on its original
            // color is exactly why a loaded LINE chart kept its old color on screen and through save
            // while an authored one (no stroke) honored the edit — charts feedback item 9. FILLED
            // kinds (bar/column/area/pie/bubble) render from the fill (`color`) and treat `a:ln` as
            // a decorative border, so recoloring their stroke would over-reach — mutating a border
            // the user never touched (charts feedback Batch 5). Gate the stroke recolor accordingly.
            let recolor_stroke = matches!(
                chart.kind,
                ChartKind::Line { .. } | ChartKind::Scatter { .. }
            );
            if let Some(s) = chart.series.get_mut(*series) {
                let new = color.map(|rgb| ChartColor::Rgb(Color::from_hex(rgb.to_hex())));
                s.color = new;
                // Override only the stroke's COLOR (keep its width/alpha); clearing the series color
                // (`None`) reverts the stroke to the palette too.
                if recolor_stroke {
                    if let Some(stroke) = &mut s.stroke {
                        stroke.color = new;
                    }
                }
            }
        }
        ChartChromeEdit::DataLabels(toggles) => {
            for s in &mut chart.series {
                let mut labels = s.data_labels.clone().unwrap_or_default();
                labels.show_value = toggles.show_value;
                labels.show_category_name = toggles.show_category_name;
                labels.show_percent = toggles.show_percent;
                s.data_labels = labels.is_shown().then_some(labels);
            }
        }
    }
}

/// The `xl/charts/chartN.xml` part names already present in a serialized package (loaded charts
/// re-injected by mode 1/2) — the used set the authored-chart part assignment avoids colliding with.
/// A package that can't be read as a zip yields an empty set (the write path then validates any real
/// collision and fails loudly).
fn existing_chart_parts(package_bytes: &[u8]) -> HashSet<String> {
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(package_bytes)) else {
        return HashSet::new();
    };
    (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("xl/charts/") && n.ends_with(".xml") && !n.contains("/_rels/"))
        .collect()
}

/// The distinct `c:f` formulas of a ranged authored chart's [`SeriesRefs`] as [`CfRange`]s, in
/// first-seen order (name / categories / values across the series), for the published spec's
/// `source_ranges` (P19). Deduped so a shared category column isn't listed once per series — the
/// value the edit panel reads back to show the chart's current data range.
fn source_ranges_from_refs(refs: &[SeriesRefs]) -> Vec<CfRange> {
    let mut out: Vec<CfRange> = Vec::new();
    for formula in refs
        .iter()
        .flat_map(|r| [&r.name, &r.categories, &r.values, &r.sizes])
        .flatten()
    {
        if !out.iter().any(|r| r.as_str() == formula) {
            out.push(CfRange::new(formula.clone()));
        }
    }
    out
}

/// The next free `xl/charts/chartN.xml` part, marking it used (mirrors `write::next_drawing_part`).
fn next_chart_part(used: &mut HashSet<String>) -> String {
    let mut n = 1;
    loop {
        let candidate = format!("xl/charts/chart{n}.xml");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::binding::ChartBindings;
    use crate::worker::protocol::Command;
    use crate::worker::run::tests::{drain_events, quiet_panics, set_input, sheet0, test_worker};
    use freecell_core::Rgb;

    /// A default authored-chart anchor (8 cols × 15 rows from A1), matching the chrome's insert.
    fn test_anchor() -> Anchor {
        Anchor::new(
            freecell_chart_model::AnchorCell::new(0, 0),
            freecell_chart_model::AnchorCell::new(8, 15),
        )
    }

    #[test]
    fn insert_chart_publishes_authored_snapshot() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let before = worker.chart_version;
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);

        // The insert holds one authored chart, bumps the version, and publishes.
        assert_eq!(worker.authored_charts.len(), 1);
        assert!(
            worker.chart_version > before,
            "the insert bumps the chart version"
        );
        assert!(drain_events(&rx)
            .iter()
            .any(|e| matches!(e, WorkerEvent::Published)));

        // The published snapshot carries the authored (snapshot-but-not-live) line chart.
        let snap = worker.shared.chart_snapshot.load_full();
        let (snap_sheet, specs) = &snap.sheets[0];
        assert_eq!(*snap_sheet, sheet);
        assert_eq!(specs.len(), 1);
        assert!(
            specs[0].is_authored(),
            "the inserted chart is Authored (no retained source, no live binding)"
        );
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));
        // Dirty tracking: one committed op is recorded so the chart can be saved.
        assert_eq!(worker.shared.committed_ops.load(Ordering::Acquire), 1);
    }

    /// Batch 3 item 8: inserting a chart with a `data` range binds it **at creation** — the published
    /// chart is born LIVE (real `c:f` refs + values resolved from the current cells), exactly as if a
    /// `SetChartRange` had followed the insert, but in one op. A `data: None` insert (asserted by
    /// `insert_chart_publishes_authored_snapshot`) stays the near-empty placeholder.
    #[test]
    fn insert_chart_with_data_binds_range_at_creation() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet); // A1:B3 = Widgets / Q1,Q2 / 10,20
        let before = worker.chart_version;
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: Some(CellRange::from_a1("A1:B3").unwrap()),
        }]);

        // Exactly one authored chart, bound at creation (non-empty refs) — no follow-up SetChartRange.
        assert_eq!(worker.authored_charts.len(), 1);
        let entry = &worker.authored_charts[0];
        assert!(
            !entry.refs.is_empty(),
            "inserting with a data range binds `c:f` refs at creation"
        );
        assert!(
            entry
                .spec
                .source_ranges
                .iter()
                .any(|r| r.as_str().contains("$B$2:$B$3")),
            "the value range is published on the spec (the chart is LIVE, not near-empty)"
        );
        // The series resolved LIVE from B2:B3 (10, 20), not the placeholder literals.
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        assert_eq!(
            entry.spec.chart().unwrap().series[0].name.as_deref(),
            Some("Widgets"),
            "the series name resolved from B1 at creation"
        );
        assert!(
            worker.chart_version > before,
            "the create-and-bind publishes once"
        );
    }

    /// Criterion #3: a degraded worker MUST reject `InsertChart` (consistent with the edit batch /
    /// paste / SetFont), pushing no authored chart and bumping no version.
    #[test]
    fn insert_chart_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);
        let version_before = worker.chart_version;

        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);

        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects InsertChart"
        );
        assert!(
            worker.authored_charts.is_empty(),
            "no authored chart is held when degraded"
        );
        assert_eq!(
            worker.chart_version, version_before,
            "no publish / version bump when degraded"
        );
    }

    /// P18: `SetChartAnchor` moves/resizes an authored chart's model anchor + bumps the version.
    #[test]
    fn set_chart_anchor_updates_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.authored_charts[0].spec.anchor, moved);
        assert!(worker.chart_version > version_before, "a move republishes");
    }

    /// P18: `DeleteChart` removes the named authored chart (leaving the rest).
    #[test]
    fn delete_chart_removes_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::InsertChart {
                sheet,
                kind: ChartInsertKind::Line,
                anchor: test_anchor(),
                data: None,
            },
            Command::InsertChart {
                sheet,
                kind: ChartInsertKind::Bar,
                anchor: test_anchor(),
                data: None,
            },
        ]);
        assert_eq!(worker.authored_charts.len(), 2);
        let first = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id: first }]);
        assert_eq!(worker.authored_charts.len(), 1);
        assert_ne!(
            worker.authored_charts[0].id, first,
            "the other chart survives"
        );
    }

    /// P18 degraded guard: a degraded worker rejects `SetChartAnchor` + `DeleteChart` (like insert).
    #[test]
    fn set_anchor_and_delete_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Insert a chart BEFORE degrading (so there's an id), then degrade.
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![
            Command::SetChartAnchor {
                sheet,
                id,
                anchor: test_anchor(),
            },
            Command::DeleteChart { sheet, id },
        ]);
        let rejects = drain_events(&rx)
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Degraded
                    }
                )
            })
            .count();
        assert_eq!(rejects, 2, "both chart ops are rejected when degraded");
        // The chart is untouched (still present).
        assert_eq!(worker.authored_charts.len(), 1);
    }

    /// Unified undo timeline (charts feedback item 4): with a chart present but the **cell edit** the
    /// most-recent action, Ctrl+Z targets the cell (not the chart), and the chart is left untouched.
    /// The `Cell` entries stay 1:1 with IronCalc's stack even with a `Chart` entry beneath them, so a
    /// chart's presence never breaks cell undo/redo.
    #[test]
    fn cell_undo_redo_correct_with_authored_chart_present() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let cell00 = |w: &Worker| w.doc.formatted_value(0, CellRef::new(0, 0)).unwrap();
        worker.process_batch(vec![set_input(sheet, 0, 0, "hello")]);
        assert_eq!(cell00(&worker), "hello");
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(cell00(&worker), "", "cell undo works with a chart present");
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(cell00(&worker), "hello", "cell redo works too");
        // The authored chart is untouched by the cell undo/redo.
        assert_eq!(worker.authored_charts.len(), 1);
    }

    /// Build + install a single **loaded** chart on `sheet` with the given `chart_part`, returning
    /// its assigned [`ChartId`]. Mirrors a discovered file-loaded chart so its delete/anchor undo can
    /// be exercised at the worker level (no real xlsx needed).
    fn install_loaded_chart(worker: &mut Worker, sheet: SheetId, chart_part: &str) -> ChartId {
        use freecell_chart_model::{
            Axis, Category, Chart, ChartKind, Grouping, Legend, Series, SourceXml,
        };
        let chart = Chart {
            title: None,
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("Loaded"),
                vec![Category::Text("Q1".into())],
                vec![1.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        let spec = ChartSpec::loaded(
            chart,
            SourceXml::new("<c:chartSpace/>"),
            Vec::new(),
            test_anchor(),
        );
        worker.charts =
            ChartBindings::from_specs_by_sheet(vec![(sheet, vec![(chart_part.to_string(), spec)])]);
        let id = ChartId(worker.next_chart_id);
        worker.charts.assign_missing_ids(&mut worker.next_chart_id);
        id
    }

    /// Delete an authored chart → Undo restores it (same id/kind/anchor) → Redo removes it again.
    #[test]
    fn delete_authored_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let anchor = worker.authored_charts[0].spec.anchor;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(
            worker.authored_charts.is_empty(),
            "delete removes the chart"
        );

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(worker.authored_charts.len(), 1, "undo brings it back");
        assert_eq!(worker.authored_charts[0].id, id, "same id restored");
        assert_eq!(worker.authored_charts[0].spec.anchor, anchor, "same anchor");
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));

        worker.process_batch(vec![Command::Redo]);
        assert!(worker.authored_charts.is_empty(), "redo re-deletes it");
    }

    /// Delete a **loaded** chart → Undo re-binds it AND drops its part from `loaded_deletes` → Redo
    /// re-deletes it AND restores the part to `loaded_deletes` (so a later save writes the right set).
    #[test]
    fn delete_loaded_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let part = "xl/charts/chart1.xml";
        let id = install_loaded_chart(&mut worker, sheet, part);
        assert!(!worker.charts.is_empty());

        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(worker.charts.is_empty(), "delete unbinds the loaded chart");
        assert!(
            worker.loaded_deletes.contains(part),
            "delete records the part in loaded_deletes"
        );

        worker.process_batch(vec![Command::Undo]);
        assert!(!worker.charts.is_empty(), "undo re-binds the loaded chart");
        assert_eq!(
            worker.charts.anchor_by_id(id),
            Some(test_anchor()),
            "the re-bound chart keeps its id + anchor"
        );
        assert!(
            !worker.loaded_deletes.contains(part),
            "undo clears the loaded-delete record"
        );

        worker.process_batch(vec![Command::Redo]);
        assert!(worker.charts.is_empty(), "redo re-deletes it");
        assert!(
            worker.loaded_deletes.contains(part),
            "redo restores the loaded-delete record"
        );
    }

    /// Insert a chart → Undo removes it → Redo re-inserts it.
    #[test]
    fn insert_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Bar,
            anchor: test_anchor(),
            data: None,
        }]);
        assert_eq!(worker.authored_charts.len(), 1);

        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty(), "undo removes the insert");

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(worker.authored_charts.len(), 1, "redo re-inserts it");
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar { .. }
        ));
    }

    /// SetAnchor (authored) → Undo restores the prior anchor → Redo re-applies the new one.
    #[test]
    fn set_anchor_authored_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let prior = worker.authored_charts[0].spec.anchor;
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.authored_charts[0].spec.anchor, moved);

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            worker.authored_charts[0].spec.anchor, prior,
            "undo restores"
        );

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(
            worker.authored_charts[0].spec.anchor, moved,
            "redo re-applies"
        );
    }

    /// SetAnchor (loaded) → Undo restores the prior render anchor AND clears the `loaded_anchor_edits`
    /// entry the move added → Redo re-applies both.
    #[test]
    fn set_anchor_loaded_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let part = "xl/charts/chart1.xml";
        let id = install_loaded_chart(&mut worker, sheet, part);
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.charts.anchor_by_id(id), Some(moved));
        assert_eq!(worker.loaded_anchor_edits.get(part), Some(&moved));

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            worker.charts.anchor_by_id(id),
            Some(test_anchor()),
            "undo restores the render anchor"
        );
        assert!(
            !worker.loaded_anchor_edits.contains_key(part),
            "undo clears the anchor-edit the move added"
        );

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(worker.charts.anchor_by_id(id), Some(moved));
        assert_eq!(worker.loaded_anchor_edits.get(part), Some(&moved));
    }

    /// SetRange (authored, born-live) → Undo restores the pre-bind (near-empty) state → Redo re-binds.
    #[test]
    fn set_range_authored_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet); // A1:B3 = Widgets / Q1,Q2 / 10,20
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        assert!(
            worker.authored_charts[0].refs.is_empty(),
            "inserted near-empty (no range yet)"
        );
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        assert!(
            !worker.authored_charts[0].refs.is_empty(),
            "range bound it live"
        );
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);

        worker.process_batch(vec![Command::Undo]);
        assert!(
            worker.authored_charts[0].refs.is_empty(),
            "undo restores the pre-bind (unbound) state"
        );

        worker.process_batch(vec![Command::Redo]);
        assert!(
            !worker.authored_charts[0].refs.is_empty(),
            "redo re-binds the range"
        );
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
    }

    /// The key correctness test: an INTERLEAVE of cell edit + chart ops undoes/redoes in exact
    /// most-recent-first order across both op families (cellEdit → InsertChart → DeleteChart, then
    /// Undo×3 restores-then-removes-the-chart-then-undoes-the-cell, and Redo×3 replays forward).
    #[test]
    fn interleaved_cell_and_chart_undo_redo_ordering() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let cell00 = |w: &Worker| w.doc.formatted_value(0, CellRef::new(0, 0)).unwrap();

        worker.process_batch(vec![set_input(sheet, 0, 0, "hello")]);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(worker.authored_charts.is_empty());
        assert_eq!(cell00(&worker), "hello");

        // Undo #1 → most recent = DeleteChart → the chart comes back.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(worker.authored_charts.len(), 1, "undo1 restores the chart");
        assert_eq!(cell00(&worker), "hello", "the cell edit is untouched");
        // Undo #2 → InsertChart → the chart goes away (NOT the cell edit).
        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty(), "undo2 removes the chart");
        assert_eq!(cell00(&worker), "hello", "the cell edit is still untouched");
        // Undo #3 → the cell edit.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(cell00(&worker), "", "undo3 undoes the cell edit");

        // Redo replays forward: cell → insert → delete.
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(cell00(&worker), "hello", "redo1 re-applies the cell edit");
        assert!(
            worker.authored_charts.is_empty(),
            "chart still gone after redo1"
        );
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(
            worker.authored_charts.len(),
            1,
            "redo2 re-inserts the chart"
        );
        worker.process_batch(vec![Command::Redo]);
        assert!(
            worker.authored_charts.is_empty(),
            "redo3 re-deletes the chart"
        );
    }

    /// A new action after an undo clears the redo stack: chart op → Undo → cell edit → the pending
    /// chart redo is discarded, so Redo is a no-op (the chart is not resurrected).
    #[test]
    fn new_action_after_undo_clears_chart_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty());
        assert_eq!(worker.redo_stack.len(), 1, "the insert is redoable");

        // A new (cell) action must invalidate the pending chart redo.
        worker.process_batch(vec![set_input(sheet, 0, 0, "x")]);
        assert!(worker.redo_stack.is_empty(), "a new action clears redo");

        worker.process_batch(vec![Command::Redo]);
        assert!(
            worker.authored_charts.is_empty(),
            "redo is a no-op — the chart is not resurrected"
        );
    }

    /// A degraded worker refuses a chart Undo/Redo (like every mutating op), leaving the timeline +
    /// chart state untouched.
    #[test]
    fn chart_undo_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![Command::Undo]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects a chart undo"
        );
        assert!(
            worker.authored_charts.is_empty(),
            "the delete is not undone while degraded"
        );
    }

    /// A small data grid an authored chart can bind to: B1 header, A2:A3 categories, B2:B3 values.
    fn seed_chart_data(worker: &mut Worker, sheet: SheetId) {
        worker.process_batch(vec![
            set_input(sheet, 0, 1, "Widgets"), // B1 (series name)
            set_input(sheet, 1, 0, "Q1"),      // A2 (category)
            set_input(sheet, 2, 0, "Q2"),      // A3
            set_input(sheet, 1, 1, "10"),      // B2 (value)
            set_input(sheet, 2, 1, "20"),      // B3
        ]);
    }

    fn first_series_values(worker: &Worker, chart_idx: usize) -> Vec<f64> {
        match &worker.authored_charts[chart_idx]
            .spec
            .chart()
            .unwrap()
            .series[0]
            .data
        {
            freecell_chart_model::SeriesData::CategoryValue { values, .. } => values.clone(),
            freecell_chart_model::SeriesData::Xy { y, .. } => y.clone(),
        }
    }

    /// P19: setting a data range binds an authored chart to real cells — its published spec gains
    /// `source_ranges` (`c:f`) AND its values re-resolve LIVE from the current cells (not the
    /// placeholder literals).
    #[test]
    fn set_chart_range_binds_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);

        let entry = &worker.authored_charts[0];
        assert!(!entry.refs.is_empty(), "the range binds `c:f` refs");
        assert!(
            entry
                .spec
                .source_ranges
                .iter()
                .any(|r| r.as_str().contains("$B$2:$B$3")),
            "the value range is published on the spec"
        );
        // The first series re-resolved from B2:B3 (10, 20), replacing the (4,6,5,8) placeholder.
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        // And its category + name came from the cells.
        match &entry.spec.chart().unwrap().series[0].data {
            freecell_chart_model::SeriesData::CategoryValue { categories, .. } => {
                assert_eq!(
                    categories[0],
                    freecell_chart_model::Category::Text("Q1".into())
                );
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        assert_eq!(
            worker.authored_charts[0].spec.chart().unwrap().series[0]
                .name
                .as_deref(),
            Some("Widgets"),
        );
        assert!(
            worker.chart_version > version_before,
            "the range republishes"
        );
    }

    /// P19: once ranged, an authored chart re-resolves on a source-cell edit — it rides the SAME
    /// dirty-set/publish path as a loaded chart, even though the workbook has no loaded charts.
    #[test]
    fn edit_reresolves_ranged_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);

        let version_before = worker.chart_version;
        worker.process_batch(vec![set_input(sheet, 1, 1, "999")]); // edit B2
        assert_eq!(
            first_series_values(&worker, 0),
            vec![999.0, 20.0],
            "editing a bound cell re-resolves the authored chart"
        );
        assert!(
            worker.chart_version > version_before,
            "the live re-resolve bumped the chart version"
        );
    }

    /// A disjoint edit (outside every bound authored range) does NOT re-resolve the chart — the
    /// authored dirty-set intersection is honored just like the loaded one.
    #[test]
    fn disjoint_edit_leaves_ranged_authored_chart_untouched() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        let version_before = worker.chart_version;
        worker.process_batch(vec![set_input(sheet, 20, 20, "42")]); // far outside A1:B3
        assert_eq!(
            worker.chart_version, version_before,
            "a disjoint edit re-resolves nothing"
        );
    }

    /// P19: switching an authored chart's type rebuilds it to the new kind, preserving the title.
    #[test]
    fn set_chart_type_switches_kind_and_preserves_title() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        worker.process_batch(vec![Command::SetChartType {
            sheet,
            id,
            kind: ChartInsertKind::Column,
        }]);
        let chart = worker.authored_charts[0].spec.chart().unwrap();
        assert!(
            matches!(
                chart.kind,
                freecell_chart_model::ChartKind::Bar {
                    dir: freecell_chart_model::BarDir::Col,
                    ..
                }
            ),
            "the chart is now a column chart"
        );
        assert_eq!(
            chart.title.as_deref(),
            Some("Chart"),
            "the title is preserved across a retype"
        );
        assert!(
            worker.chart_version > version_before,
            "a retype republishes"
        );
    }

    /// P19: retyping a chart that already has a data range keeps the range binding (its `c:f` refs +
    /// live values) — only the kind changes.
    #[test]
    fn set_chart_type_preserves_the_range_binding() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        worker.process_batch(vec![Command::SetChartType {
            sheet,
            id,
            kind: ChartInsertKind::Column,
        }]);
        // Still bound to the same cells → still the live values, now on a column chart.
        assert!(!worker.authored_charts[0].refs.is_empty(), "refs preserved");
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar { .. }
        ));
    }

    /// P19 degraded guard: a degraded worker rejects `SetChartRange` + `SetChartType` (like every
    /// mutating op), leaving the chart untouched.
    #[test]
    fn set_chart_range_and_type_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![
            Command::SetChartRange {
                sheet,
                id,
                data: CellRange::from_a1("A1:B3").unwrap(),
            },
            Command::SetChartType {
                sheet,
                id,
                kind: ChartInsertKind::Bar,
            },
        ]);
        let rejects = drain_events(&rx)
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Degraded
                    }
                )
            })
            .count();
        assert_eq!(rejects, 2, "both chart edits are rejected when degraded");
        // Untouched: still an unbound line chart.
        assert!(worker.authored_charts[0].refs.is_empty());
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));
    }

    /// The published authored chart's typed [`Chart`] (for chrome assertions).
    fn authored_chart(worker: &Worker, idx: usize) -> Chart {
        worker.authored_charts[idx].spec.chart().unwrap().clone()
    }

    fn chrome(sheet: SheetId, id: ChartId, edit: ChartChromeEdit) -> Command {
        Command::SetChartChrome { sheet, id, edit }
    }

    /// P20: each chrome edit mutates an **authored** chart's model + republishes.
    #[test]
    fn set_chart_chrome_edits_an_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;

        // Title.
        let v0 = worker.chart_version;
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Title(Some("Sales".into())),
        )]);
        assert_eq!(authored_chart(&worker, 0).title.as_deref(), Some("Sales"));
        assert!(worker.chart_version > v0, "a chrome edit republishes");

        // Legend off, then on-at-bottom.
        worker.process_batch(vec![chrome(sheet, id, ChartChromeEdit::Legend(None))]);
        assert_eq!(authored_chart(&worker, 0).legend, None);
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Legend(Some(freecell_chart_model::LegendPosition::Bottom)),
        )]);
        assert_eq!(
            authored_chart(&worker, 0).legend.map(|l| l.position),
            Some(freecell_chart_model::LegendPosition::Bottom)
        );

        // Axis titles.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Category,
                title: Some("Quarter".into()),
            },
        )]);
        assert_eq!(
            authored_chart(&worker, 0).cat_axis.title.as_deref(),
            Some("Quarter")
        );

        // Series color.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: Some(Rgb::from_hex(0x70AD47)),
            },
        )]);
        assert_eq!(
            authored_chart(&worker, 0).series[0].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x70AD47)
            )),
        );

        // Data-label toggles apply to every series; clearing all turns labels off.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::DataLabels(crate::worker::protocol::DataLabelToggles {
                show_value: true,
                show_category_name: false,
                show_percent: true,
            }),
        )]);
        let dl = authored_chart(&worker, 0).series[0]
            .data_labels
            .clone()
            .expect("labels set");
        assert!(dl.show_value && dl.show_percent && !dl.show_category_name);
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::DataLabels(crate::worker::protocol::DataLabelToggles::default()),
        )]);
        assert!(
            authored_chart(&worker, 0).series[0].data_labels.is_none(),
            "clearing every toggle removes the labels"
        );
    }

    /// **Batch 5 gate — a `SeriesColor` edit recolors the `a:ln` STROKE only for LINE / SCATTER.**
    /// Those two kinds paint their visible color on the stroke (renderer prefers `stroke.color` over
    /// `color`), so the edit must override it. FILLED kinds (bar/column/area/pie/bubble) render from
    /// the fill and treat `a:ln` as a decorative border, so recoloring their imported stroke would
    /// over-reach — mutating a border the user never touched. Pins the live-side gate.
    #[test]
    fn series_color_recolors_stroke_only_for_line_and_scatter() {
        use freecell_chart_model::{
            Axis, BarDir, BarLayout, Category, Chart, ChartColor, ChartKind, Color, Grouping,
            LineStroke, ScatterStyle, Series,
        };

        // A series carrying BOTH a fill color (4472C4) and an imported `a:ln` stroke on a DISTINCT
        // color (203040) — the shape that distinguishes a fill recolor from a stroke recolor.
        let stroked = || {
            let mut s =
                Series::category_value(Some("S"), vec![Category::Text("Q1".into())], vec![1.0])
                    .with_color(Color::from_hex(0x4472C4));
            s.stroke = Some(
                LineStroke::new()
                    .with_width_pt(1.0)
                    .with_color(Color::from_hex(0x203040)),
            );
            s
        };
        let chart_of = |kind: ChartKind| Chart {
            title: None,
            kind,
            series: vec![stroked()],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: None,
        };
        let edit = ChartChromeEdit::SeriesColor {
            series: 0,
            color: Some(Rgb::from_hex(0x70AD47)),
        };
        let new_cc = Some(ChartColor::Rgb(Color::from_hex(0x70AD47)));
        let orig_stroke = Some(ChartColor::Rgb(Color::from_hex(0x203040)));

        // FILLED (column): the fill recolors, the imported border stroke is LEFT on its color.
        let mut bar = chart_of(ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        });
        apply_chrome_edit(&mut bar, &edit);
        assert_eq!(bar.series[0].color, new_cc, "the filled fill recolors");
        assert_eq!(
            bar.series[0].stroke.and_then(|s| s.color),
            orig_stroke,
            "a filled series' imported border stroke is left untouched (Batch 5 gate)",
        );

        // LINE: both fill and stroke recolor (the stroke is the visible line — feedback item 9).
        let mut line = chart_of(ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        });
        apply_chrome_edit(&mut line, &edit);
        assert_eq!(line.series[0].color, new_cc, "the line fill recolors");
        assert_eq!(
            line.series[0].stroke.and_then(|s| s.color),
            new_cc,
            "a line's visible stroke recolors to the new color",
        );

        // SCATTER: same as line — its color also lives on the stroke.
        let mut scatter = chart_of(ChartKind::Scatter {
            style: ScatterStyle::LineMarker,
        });
        apply_chrome_edit(&mut scatter, &edit);
        assert_eq!(
            scatter.series[0].stroke.and_then(|s| s.color),
            new_cc,
            "a scatter's visible stroke recolors to the new color",
        );
    }

    /// P20 degraded guard: a degraded worker rejects `SetChartChrome`, leaving the chart untouched.
    #[test]
    fn set_chart_chrome_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let title_before = authored_chart(&worker, 0).title;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Title(Some("nope".into())),
        )]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects SetChartChrome"
        );
        assert_eq!(
            authored_chart(&worker, 0).title,
            title_before,
            "the chart is untouched when degraded"
        );
    }

    /// B1 (`functional_spec.md F2.2`): the save-time sweep runs INSIDE the save's `catch_unwind`
    /// and its per-sheet `discover_and_parse_for_part` parses user-supplied zip/XML — the most
    /// panic-prone call on the save path. A panic there must leave the unswept sheets
    /// **re-discoverable**: before the fix the sheet was marked "walked" *before* its parse, so a
    /// caught panic left a sheet flagged discovered whose charts were never bound —
    /// `ensure_sheet_charts_discovered` then returned early for it forever (while
    /// `charts_fully_discovered` stayed `false`), silently dropping its charts until some later
    /// save re-ran the whole sweep.
    #[test]
    fn a_save_panic_mid_sweep_leaves_the_sheet_re_discoverable() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("two_sheet.xlsx");
        crate::chart::authoring::write_two_sheet_fixture(&good).unwrap();
        // The same fixture under the sentinel name: the sweep panics parsing it (the stand-in for
        // a zip/XML parse that blows up), while the identical bytes under `good` parse normally.
        let poisoned = dir.path().join(crate::document::PANIC_SENTINEL);
        std::fs::copy(&good, &poisoned).unwrap();
        let data_part = crate::chart::workbook_sheet_parts(&good)
            .unwrap()
            .into_iter()
            .find(|(name, _)| name == "Data")
            .expect("the fixture's chart-carrying worksheet")
            .1;

        // A worker that still has charts to discover — the state a freshly opened `.xlsx` is in
        // before any sheet has been painted, and the state `test_worker()` alone never reaches
        // (a `NewWorkbook` is born `charts_fully_discovered`).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.chart_source_path = Some(poisoned);
        worker.charts_fully_discovered = false;
        worker.chart_sheet_parts = HashMap::from([(sheet, data_part)]);

        quiet_panics(|| {
            worker.process_batch(vec![Command::Save {
                path: dir.path().join("out.xlsx"),
                req_id: 1,
            }])
        });

        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::SaveFailed {
                    req_id: 1,
                    error: crate::document::SaveError::EnginePanic
                }
            )),
            "the sweep's panic is caught by the save guard and reported"
        );
        assert!(
            !worker.charts_fully_discovered,
            "the sweep did not finish, so it must not claim it did"
        );
        assert!(
            !worker.discovered_chart_sheets.contains(&sheet),
            "a sheet whose parse panicked must NOT be recorded as walked"
        );
        assert!(
            worker.charts.is_empty(),
            "nothing was bound — the panic was the parse itself"
        );

        // The proof that discovery is not permanently disabled: with the same workbook parsing
        // normally, the lazy per-sheet path still walks this sheet and binds its chart.
        worker.chart_source_path = Some(good);
        worker.ensure_sheet_charts_discovered(sheet);
        assert!(
            !worker.charts.is_empty(),
            "lazy discovery still reaches the sheet after a caught save panic"
        );
        assert!(worker.discovered_chart_sheets.contains(&sheet));
    }
}
