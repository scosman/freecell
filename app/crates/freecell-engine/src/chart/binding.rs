//! Live binding: `c:f` → structured ranges, the range→chart index, and the re-resolution of a
//! chart's values from the **current** worksheet cells (charts/architecture §4.1, §5 challenge 2;
//! functional_spec §2). This is the engine-side machinery the worker drives on each edit; it is
//! pure (IronCalc-free, gpui-free) and drives its cell reads through **closures**, so the whole
//! index / intersection / re-resolve pipeline unit-tests headless with fakes.
//!
//! The flow the worker wires up (`worker::run`):
//! 1. On load, [`ChartBindings::from_specs`] parses each chart's retained source XML **once** into
//!    a per-series [`SeriesBinding`] (its `c:f` name / category / value refs → structured
//!    [`CfRef`]s). The specs keep their file-cached values as **first paint**.
//! 2. On an edit, [`ChartBindings::dirty_indices`] intersects the **edited-cell set** against those
//!    ranges — the range→chart index — to select only the charts the edit touched (no rescan).
//! 3. [`ChartBindings::reresolve`] rebuilds only those charts' [`Chart`]s from live cell values;
//!    a range that can't resolve (deleted / renamed sheet) falls back to the cached value.
//!
//! The retained raw [`CfRange`](freecell_chart_model::CfRange) refs stay as-loaded on the spec; this
//! module is where they become structured, sheet-resolved ranges.

use std::collections::HashSet;
use std::sync::Arc;

use roxmltree::{Document, Node};

use freecell_chart_model::{Anchor, Category, Chart, ChartId, ChartSpec, Series, SeriesData};
use freecell_core::{CellRange, CellRef, SheetId};

use super::load::is_chart_group;

/// One resolved cell value, read from the current model for live binding — the engine-free bridge
/// [`WorkbookDocument::cell_value`](crate::WorkbookDocument) produces and the resolver consumes (no
/// IronCalc type escapes the crate).
#[derive(Clone, Debug, PartialEq)]
pub enum CellData {
    Number(f64),
    Text(String),
    Bool(bool),
    /// Empty or otherwise value-less cell.
    Empty,
}

/// One rectangular area of a `c:f` reference — its sheet (by **name**, `None` when the reference is
/// unqualified) and its 0-based inclusive rectangle. A multi-area `c:f` (a union) carries several.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfArea {
    /// The sheet name exactly as written in the `c:f` (`Data` in `Data!$B$2:$B$5`), or `None` for
    /// an unqualified reference (resolved against the chart's own sheet).
    pub sheet: Option<String>,
    /// The referenced rectangle, 0-based inclusive (`$` absolute markers dropped).
    pub range: CellRange,
}

/// A parsed `c:f` data reference: one or more [areas](CfArea) (usually one; a comma-union yields
/// several). The structured form of a retained [`CfRange`](freecell_chart_model::CfRange).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfRef {
    pub areas: Vec<CfArea>,
}

/// Parse a `c:f` formula string into a structured [`CfRef`]. Handles absolute `$` markers, a
/// sheet-name prefix (bare `Data!…` or quoted `'My Data'!…`), and a comma-union of areas
/// (optionally wrapped in one layer of parentheses, `(A!…,A!…)`). Returns `None` if no area parses;
/// areas that don't parse are dropped. Never panics on hostile input.
///
/// **Unsupported shapes fall back to cache** (return `None` → the role keeps its cached value):
/// whole-column / whole-row references (`Data!A:A`, `Data!1:1`) — [`CellRange::from_a1`] needs both a
/// column and a row, and Excel/LibreOffice don't emit these bare for chart `c:f`. The paren-stripper
/// is deliberately naive (strips exactly one leading `(` + trailing `)`), which is all a real union
/// ever carries. Both are intentional, documented limitations, not silent bugs.
pub fn parse_cf(formula: &str) -> Option<CfRef> {
    let trimmed = formula.trim();
    // A union is sometimes wrapped in a single layer of parentheses.
    let body = trimmed
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(trimmed);
    let areas: Vec<CfArea> = split_top_level_commas(body)
        .into_iter()
        .filter_map(|part| parse_cf_area(part.trim()))
        .collect();
    (!areas.is_empty()).then_some(CfRef { areas })
}

/// Split on commas that are **not** inside a `'…'` quoted sheet name. Comma and quote are ASCII, so
/// every split index is a char boundary.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'\'' => in_quote = !in_quote,
            b',' if !in_quote => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse one area: an optional `SheetName!` / `'Quoted Name'!` prefix followed by an A1 range.
fn parse_cf_area(part: &str) -> Option<CfArea> {
    let (sheet, range_str) = split_sheet_prefix(part);
    // A `!range` with an **empty** sheet name (e.g. `!A1`) is malformed, not unqualified.
    if matches!(&sheet, Some(name) if name.is_empty()) {
        return None;
    }
    let range = CellRange::from_a1(range_str)?;
    Some(CfArea { sheet, range })
}

/// Split a reference into its optional sheet-name prefix and the trailing A1 range. Bare names split
/// at the first `!`; a `'…'`-quoted name honors the `''` escape.
fn split_sheet_prefix(part: &str) -> (Option<String>, &str) {
    if let Some(rest) = part.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if rest[i + 1..].starts_with('\'') {
                    name.push('\''); // an escaped '' → a literal quote
                    chars.next();
                } else {
                    // Closing quote; a well-formed reference has `!range` after it.
                    return match rest[i + 1..].strip_prefix('!') {
                        Some(range) => (Some(name), range),
                        None => (None, part), // malformed → let the range parse fail
                    };
                }
            } else {
                name.push(c);
            }
        }
        (None, part) // no closing quote
    } else if let Some(idx) = part.find('!') {
        (Some(part[..idx].to_string()), &part[idx + 1..])
    } else {
        (None, part)
    }
}

/// The `c:f` refs of one series, by role: its name (`c:tx`), its category / x (`c:cat` / `c:xVal`),
/// and its value / y (`c:val` / `c:yVal`). A role with no cached formula reference is `None` (a
/// literal name, or an absent axis) and keeps its template value on re-resolve.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeriesBinding {
    pub name: Option<CfRef>,
    pub cat: Option<CfRef>,
    pub val: Option<CfRef>,
}

/// A chart's per-series `c:f` bindings, in the **same order** as the parsed [`Chart`]'s series (both
/// walk the first chart-group's `<c:ser>` children), so `series[i]` re-resolves `chart.series[i]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartBinding {
    pub series: Vec<SeriesBinding>,
}

/// Parse a chart part's XML into its per-series [`SeriesBinding`]s (charts/architecture §4.1). Reads
/// only the `<c:f>` references (structure), never the caches — the values come live. A part that
/// won't parse, or has no recognized chart-group, yields an empty binding (the chart simply never
/// live-updates; its cached values still render).
pub fn parse_chart_binding(chart_xml: &str) -> ChartBinding {
    let Ok(doc) = Document::parse(chart_xml) else {
        return ChartBinding::default();
    };
    let root = doc.root_element();
    let Some(group) = child(&root, "chart")
        .and_then(|chart| child(&chart, "plotArea"))
        .and_then(|plot| {
            plot.children()
                .find(|n| n.is_element() && is_chart_group(n.tag_name().name()))
        })
    else {
        return ChartBinding::default();
    };
    let series = group
        .children()
        .filter(|n| n.tag_name().name() == "ser")
        .map(|ser| SeriesBinding {
            name: ser_ref(&ser, &["tx"]),
            cat: ser_ref(&ser, &["cat", "xVal"]),
            val: ser_ref(&ser, &["val", "yVal"]),
        })
        .collect();
    ChartBinding { series }
}

/// The first `<c:f>` under any of `holder_tags` (a series' `c:tx` / `c:cat` / `c:val` …), parsed to
/// a [`CfRef`].
fn ser_ref(ser: &Node, holder_tags: &[&str]) -> Option<CfRef> {
    holder_tags
        .iter()
        .filter_map(|tag| child(ser, tag))
        .find_map(|holder| first_f_text(&holder).and_then(parse_cf))
}

/// The text of the first `<c:f>` element anywhere under `node`.
fn first_f_text<'a>(node: &Node<'a, '_>) -> Option<&'a str> {
    node.descendants()
        .find(|n| n.tag_name().name() == "f")
        .and_then(|n| n.text())
}

/// The first child *element* with this local tag name.
fn child<'a>(node: &Node<'a, '_>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// A closure resolving a `c:f` sheet **name** to its stable [`SheetId`] (against the current model).
pub type SheetResolver<'a> = dyn Fn(&str) -> Option<SheetId> + 'a;
/// A closure reading a cell's current value from the model.
pub type CellReader<'a> = dyn Fn(SheetId, CellRef) -> CellData + 'a;

/// Do two 0-based inclusive rectangles overlap?
pub fn ranges_intersect(a: &CellRange, b: &CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.col <= b.end.col
        && b.start.col <= a.end.col
}

/// Read a reference's cells in row-major order across its areas, or `None` if **any** area's sheet
/// can't resolve (→ the caller keeps the cached value). An unqualified area reads `default_sheet`.
fn read_ref_cells(
    cf: &CfRef,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Option<Vec<CellData>> {
    let mut out = Vec::new();
    for area in &cf.areas {
        let sheet = match &area.sheet {
            Some(name) => resolve_sheet(name)?,
            None => default_sheet,
        };
        for row in area.range.rows() {
            for col in area.range.cols() {
                out.push(read_cell(sheet, CellRef::new(row, col)));
            }
        }
    }
    Some(out)
}

/// Live numeric values for a value/x/y reference. A non-numeric or empty cell becomes `NaN` so
/// positions stay aligned with the categories and the renderer blanks it (functional_spec §7; the
/// P5 renderer already drops non-finite points).
fn resolve_numbers(
    cf: &CfRef,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Option<Vec<f64>> {
    read_ref_cells(cf, default_sheet, resolve_sheet, read_cell).map(|cells| {
        cells
            .into_iter()
            .map(|d| match d {
                CellData::Number(n) => n,
                _ => f64::NAN,
            })
            .collect()
    })
}

/// Live category labels for a `c:cat` reference (text or numeric).
fn resolve_categories(
    cf: &CfRef,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Option<Vec<Category>> {
    read_ref_cells(cf, default_sheet, resolve_sheet, read_cell).map(|cells| {
        cells
            .into_iter()
            .map(|d| match d {
                CellData::Number(n) => Category::Number(n),
                CellData::Text(s) => Category::Text(s),
                CellData::Bool(b) => Category::Text(b.to_string()),
                CellData::Empty => Category::Text(String::new()),
            })
            .collect()
    })
}

/// A live series name from a `c:tx` reference: the first text/number cell it resolves to. `None`
/// (unresolvable, or blank) keeps the template name.
fn resolve_name(
    cf: &CfRef,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Option<String> {
    read_ref_cells(cf, default_sheet, resolve_sheet, read_cell)?
        .into_iter()
        .find_map(|d| match d {
            CellData::Text(s) if !s.is_empty() => Some(s),
            CellData::Number(n) => Some(Category::Number(n).label()),
            _ => None,
        })
}

/// Rebuild one series from live cells, **preserving** the template's color, marker, and
/// [`SeriesData`] variant. A role whose reference is absent or unresolvable keeps the template value
/// (the cache fallback).
fn resolve_series(
    template: &Series,
    binding: &SeriesBinding,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Series {
    let mut series = template.clone();
    if let Some(cf) = &binding.name {
        if let Some(name) = resolve_name(cf, default_sheet, resolve_sheet, read_cell) {
            series.name = Some(name);
        }
    }
    match &mut series.data {
        SeriesData::CategoryValue { categories, values } => {
            if let Some(cf) = &binding.cat {
                if let Some(cats) = resolve_categories(cf, default_sheet, resolve_sheet, read_cell)
                {
                    *categories = cats;
                }
            }
            if let Some(cf) = &binding.val {
                if let Some(vals) = resolve_numbers(cf, default_sheet, resolve_sheet, read_cell) {
                    *values = vals;
                }
            }
        }
        SeriesData::Xy { x, y } => {
            if let Some(cf) = &binding.cat {
                if let Some(xs) = resolve_numbers(cf, default_sheet, resolve_sheet, read_cell) {
                    *x = xs;
                }
            }
            if let Some(cf) = &binding.val {
                if let Some(ys) = resolve_numbers(cf, default_sheet, resolve_sheet, read_cell) {
                    *y = ys;
                }
            }
        }
    }
    series
}

/// Rebuild a whole chart from live cells, keeping its kind / axes / legend / title and each series'
/// styling. Series with no binding entry (shouldn't happen for a well-formed part) are left as-is.
///
/// `template` is the chart's **last-resolved** picture — on first paint that is the file's
/// `numCache`/`strCache`, and thereafter the previous live values. A role whose range can't resolve
/// keeps the template value, so the chart never regresses below its last good state. Reverting a
/// range that *stops* resolving (e.g. its sheet is deleted) all the way back to the **file's**
/// numeric cache — rather than the last live values — is P10 (source-first save) territory.
pub fn resolve_chart(
    template: &Chart,
    binding: &ChartBinding,
    default_sheet: SheetId,
    resolve_sheet: &SheetResolver<'_>,
    read_cell: &CellReader<'_>,
) -> Chart {
    let mut chart = template.clone();
    for (series, sb) in chart.series.iter_mut().zip(binding.series.iter()) {
        *series = resolve_series(series, sb, default_sheet, resolve_sheet, read_cell);
    }
    chart
}

/// One chart the worker owns for live binding: its render/save envelope (whose `chart` field holds
/// the current, live-resolved values), its structured per-series binding, and the sheet it is
/// anchored on (which keys the published snapshot). Data sheets are resolved by name per reference,
/// so they are independent of `anchor_sheet`.
#[derive(Clone, Debug)]
struct BoundChart {
    anchor_sheet: SheetId,
    /// The chart's `xl/charts/chartN.xml` part — the stable key the source-first save re-injects +
    /// reflows on (P10). Set at discovery; empty only for the single-sheet `from_specs` convenience
    /// (never used by the save path).
    chart_part: String,
    /// The stable manipulation handle the worker stamps (P18) — assigned by
    /// [`ChartBindings::assign_missing_ids`] and stamped onto the published spec so the app can name
    /// this chart back for move/resize/delete. [`ChartId::NONE`] until assigned.
    id: ChartId,
    spec: ChartSpec,
    binding: ChartBinding,
}

impl BoundChart {
    /// Is this chart touched by the edit? True if any of its references overlaps an edited range on
    /// its (resolved) sheet, or resolves to a sheet that was structurally rebuilt (insert/delete;
    /// its `c:f` isn't reflowed until save — P10 — so a rebuilt data sheet re-resolves best-effort).
    fn is_dirty(
        &self,
        edited: &[(SheetId, CellRange)],
        rebuilt_sheets: &[SheetId],
        resolve_sheet: &SheetResolver<'_>,
    ) -> bool {
        for sb in &self.binding.series {
            for cf in [&sb.name, &sb.cat, &sb.val].into_iter().flatten() {
                for area in &cf.areas {
                    let sheet = match &area.sheet {
                        Some(name) => match resolve_sheet(name) {
                            Some(s) => s,
                            None => continue,
                        },
                        None => self.anchor_sheet,
                    };
                    if rebuilt_sheets.contains(&sheet) {
                        return true;
                    }
                    if edited
                        .iter()
                        .any(|(es, er)| *es == sheet && ranges_intersect(&area.range, er))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// The worker's set of live-bound charts — the range→chart index (challenge 2). Built once on load
/// from the discovered [`ChartSpec`]s; queried by [`dirty_indices`](Self::dirty_indices) and mutated
/// by [`reresolve`](Self::reresolve) on each edit; snapshotted by
/// [`specs_by_sheet`](Self::specs_by_sheet) for the publication seam.
#[derive(Clone, Debug, Default)]
pub struct ChartBindings {
    charts: Vec<BoundChart>,
}

impl ChartBindings {
    /// Bind the discovered charts, anchoring them all to `anchor_sheet` (single-sheet convenience;
    /// used by tests). Chart parts are unknown here (empty) — this path is never the save path.
    pub fn from_specs(specs: Vec<ChartSpec>, anchor_sheet: SheetId) -> Self {
        Self::from_specs_by_sheet(vec![(
            anchor_sheet,
            specs.into_iter().map(|s| (String::new(), s)).collect(),
        )])
    }

    /// Bind discovered charts anchored to **their own** worksheet's [`SheetId`] (multi-sheet
    /// placement, P10 — the item P8/P9 deferred for lack of the `workbook.xml.rels` part map).
    /// Each group pairs an anchor sheet with `(chart_part, spec)` pairs discovered on it
    /// ([`discover_and_parse_by_sheet`](super::load::discover_and_parse_by_sheet), mapped
    /// name→`SheetId` by the caller). Each chart carries its part so the save can re-inject +
    /// reflow it without re-deriving the association. Within a group, each chart's binding is parsed
    /// from its retained source XML (an authored chart gets an empty binding); the specs keep their
    /// file-cached values as first paint — no model read here.
    pub fn from_specs_by_sheet(groups: Vec<(SheetId, super::load::SheetCharts)>) -> Self {
        let charts = groups
            .into_iter()
            .flat_map(|(anchor_sheet, specs)| {
                specs.into_iter().map(move |(chart_part, spec)| {
                    let binding = spec
                        .source()
                        .map(|s| parse_chart_binding(&s.chart_xml))
                        .unwrap_or_default();
                    BoundChart {
                        anchor_sheet,
                        chart_part,
                        id: ChartId::NONE,
                        spec,
                        binding,
                    }
                })
            })
            .collect();
        Self { charts }
    }

    pub fn is_empty(&self) -> bool {
        self.charts.is_empty()
    }

    /// Assign a stable [`ChartId`] to every bound chart that doesn't have one yet (P18), drawing
    /// from the worker's monotonic `next` counter (shared with authored charts so ids are globally
    /// unique). Called after each [`add_missing`](Self::add_missing) so a lazily-discovered chart
    /// gets an id the moment it is bound.
    pub fn assign_missing_ids(&mut self, next: &mut u64) {
        for bc in &mut self.charts {
            if bc.id == ChartId::NONE {
                bc.id = ChartId(*next);
                *next += 1;
            }
        }
    }

    /// Move/resize a bound chart (P18): set the chart with stable id `id` to `anchor` (updating its
    /// render spec so the republished snapshot places it there) and return its `chart_part` so the
    /// caller can record the drawing-anchor patch the save applies. `None` if no bound chart has
    /// that id (it was authored, or already deleted).
    pub fn set_anchor_by_id(&mut self, id: ChartId, anchor: Anchor) -> Option<String> {
        let bc = self.charts.iter_mut().find(|bc| bc.id == id)?;
        bc.spec.anchor = anchor;
        Some(bc.chart_part.clone())
    }

    /// Delete a bound chart (P18): remove the chart with stable id `id` and return its `chart_part`
    /// so the caller can record it for the save to drop from the package. `None` if no bound chart
    /// has that id.
    pub fn remove_by_id(&mut self, id: ChartId) -> Option<String> {
        let pos = self.charts.iter().position(|bc| bc.id == id)?;
        Some(self.charts.remove(pos).chart_part)
    }

    /// Append the charts in `groups` that aren't **already bound**, anchoring each to its group's
    /// sheet, and return whether anything was added (charts/architecture §5 challenge 5, lazy
    /// discovery). Dedup is by `chart_part` — the stable package key — so it is robust to a group
    /// that a name-fallback anchored onto an already-loaded sheet, and it never re-binds (or
    /// clobbers the live-resolved values of) a chart the worker already owns. Both the **per-sheet
    /// lazy** path (one group) and the **save-time full sweep** (all groups) funnel through here, so
    /// a chart is bound exactly once regardless of which path reaches it first. Each new chart's
    /// binding is parsed from its retained source XML; the spec keeps its file-cached values as
    /// first paint.
    pub fn add_missing(&mut self, groups: Vec<(SheetId, super::load::SheetCharts)>) -> bool {
        let mut existing: HashSet<String> =
            self.charts.iter().map(|bc| bc.chart_part.clone()).collect();
        let mut added = false;
        for (anchor_sheet, specs) in groups {
            for (chart_part, spec) in specs {
                if !existing.insert(chart_part.clone()) {
                    continue; // already bound (visited earlier, or a name-fallback duplicate)
                }
                let binding = spec
                    .source()
                    .map(|s| parse_chart_binding(&s.chart_xml))
                    .unwrap_or_default();
                self.charts.push(BoundChart {
                    anchor_sheet,
                    chart_part,
                    id: ChartId::NONE,
                    spec,
                    binding,
                });
                added = true;
            }
        }
        added
    }

    /// Each bound chart as a [`LiveChart`](super::save::LiveChart) the source-first save consumes
    /// (`worker::run`, charts/architecture §5), in discovery order: its own chart part, its current
    /// (live-resolved) values, and the **current** name of its anchor worksheet — resolved through
    /// `resolve_name` (`SheetId` → current name; `None` when that sheet was deleted, so the save
    /// drops the chart rather than mis-placing it). Resolving here (not by original name) is what
    /// makes save survive an in-session sheet rename.
    pub fn live_charts(
        &self,
        resolve_name: impl Fn(SheetId) -> Option<String>,
    ) -> Vec<super::save::LiveChart> {
        self.charts
            .iter()
            .map(|bc| super::save::LiveChart {
                sheet_name: resolve_name(bc.anchor_sheet),
                chart_part: bc.chart_part.clone(),
                // An Unsupported spec has no typed chart to reflow → `None` (byte-preserved on save).
                chart: bc.spec.chart().cloned(),
            })
            .collect()
    }

    /// The indices of the charts the edit touched (the dirty set) — the range index intersected with
    /// the edited-cell set. No cell reads: cheap enough to run on every edit.
    pub fn dirty_indices(
        &self,
        edited: &[(SheetId, CellRange)],
        rebuilt_sheets: &[SheetId],
        resolve_sheet: &SheetResolver<'_>,
    ) -> Vec<usize> {
        self.charts
            .iter()
            .enumerate()
            .filter(|(_, bc)| bc.is_dirty(edited, rebuilt_sheets, resolve_sheet))
            .map(|(i, _)| i)
            .collect()
    }

    /// Re-resolve the given charts' values from live cells, in place. Returns whether any chart's
    /// picture actually changed (so the caller can skip a needless snapshot + repaint).
    pub fn reresolve(
        &mut self,
        indices: &[usize],
        resolve_sheet: &SheetResolver<'_>,
        read_cell: &CellReader<'_>,
    ) -> bool {
        let mut changed = false;
        for &i in indices {
            let bc = &mut self.charts[i];
            // An Unsupported spec has no typed chart to re-resolve — it's static (its cached
            // placeholder never changes on an edit), so skip it (it never enters the dirty set
            // anyway, since its binding is empty).
            let Some(template) = bc.spec.chart() else {
                continue;
            };
            let rebuilt = resolve_chart(
                template,
                &bc.binding,
                bc.anchor_sheet,
                resolve_sheet,
                read_cell,
            );
            // NOTE: a chart carrying a blanked `NaN` value is never `==` itself (`NaN != NaN`), so
            // `changed` is always true for such a chart. That is acceptable here — `reresolve` only
            // runs on charts already selected as dirty (their ranges intersected the edit), which
            // repaint anyway; the equality check only spares a redundant snapshot for the ordinary
            // finite-valued case. Do NOT "fix" this into NaN-aware equality: it would add complexity
            // for no observable win (the chart is dirty either way).
            if bc.spec.chart() != Some(&rebuilt) {
                if let Some(slot) = bc.spec.chart_mut() {
                    *slot = rebuilt;
                }
                changed = true;
            }
        }
        changed
    }

    /// The current specs grouped by the sheet they're anchored on, each group behind an
    /// [`Arc`] — the payload for a [`ChartSnapshot`](crate::ChartSnapshot). Preserves discovery
    /// order within each sheet.
    ///
    /// The per-sheet list is an `Arc<[ChartSpec]>` so the app shares the **same allocation** the
    /// worker publishes (zero app-side duplicate — charts/architecture §5 challenge 5, "off-screen
    /// free"). Building it clones each bound spec once; since P11 put the heavy retained source
    /// behind an `Arc` (`Origin::Loaded`), that clone bumps a refcount rather than deep-copying the
    /// chart XML — only the render `Chart` (the value a re-resolve actually changed) is copied. So
    /// an intersecting edit rebuilds the snapshot in O(chart values), not O(source bytes).
    pub fn specs_by_sheet(&self) -> Vec<(SheetId, Arc<[ChartSpec]>)> {
        let mut out: Vec<(SheetId, Vec<ChartSpec>)> = Vec::new();
        for bc in &self.charts {
            // Stamp the stable manipulation id onto the published spec (P18) so the app can name
            // this exact chart back for move/resize/delete.
            let spec = bc.spec.clone().with_id(bc.id);
            match out.iter_mut().find(|(s, _)| *s == bc.anchor_sheet) {
                Some((_, specs)) => specs.push(spec),
                None => out.push((bc.anchor_sheet, vec![spec])),
            }
        }
        out.into_iter()
            .map(|(sheet, specs)| (sheet, Arc::from(specs)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::authoring;
    use freecell_chart_model::{Anchor, AnchorCell, Axis, ChartKind, Grouping, Legend, SourceXml};
    use std::collections::HashMap;

    fn cell(row: u32, col: u32) -> CellRef {
        CellRef::new(row, col)
    }

    fn range(a1: &str) -> CellRange {
        CellRange::from_a1(a1).unwrap()
    }

    // --- parse_cf ---------------------------------------------------------------------------

    #[test]
    fn parse_cf_single_absolute_ref() {
        let cf = parse_cf("Data!$B$2:$B$5").unwrap();
        assert_eq!(cf.areas.len(), 1);
        assert_eq!(cf.areas[0].sheet.as_deref(), Some("Data"));
        // B2:B5 → rows 1..=4, col 1.
        assert_eq!(cf.areas[0].range, range("B2:B5"));
    }

    #[test]
    fn parse_cf_multi_area_union() {
        for formula in [
            "(Data!$B$2:$B$5,Data!$D$2:$D$5)",
            "Data!$B$2:$B$5,Data!$D$2:$D$5",
        ] {
            let cf = parse_cf(formula).unwrap();
            assert_eq!(cf.areas.len(), 2, "{formula}");
            assert_eq!(cf.areas[0].range, range("B2:B5"));
            assert_eq!(cf.areas[1].range, range("D2:D5"));
            assert!(cf.areas.iter().all(|a| a.sheet.as_deref() == Some("Data")));
        }
    }

    #[test]
    fn parse_cf_unqualified_quoted_and_single_cell() {
        // Unqualified → no sheet.
        let un = parse_cf("$A$1:$A$3").unwrap();
        assert_eq!(un.areas[0].sheet, None);
        assert_eq!(un.areas[0].range, range("A1:A3"));
        // Quoted sheet name with a space.
        let q = parse_cf("'My Data'!$A$1:$A$3").unwrap();
        assert_eq!(q.areas[0].sheet.as_deref(), Some("My Data"));
        assert_eq!(q.areas[0].range, range("A1:A3"));
        // Single cell → 1×1 range.
        let one = parse_cf("Data!$B$1").unwrap();
        assert_eq!(one.areas[0].range, CellRange::single(cell(0, 1)));
    }

    #[test]
    fn parse_cf_rejects_junk() {
        for bad in ["", "   ", "Data!", "!A1", "Data!ZZZ0"] {
            assert_eq!(parse_cf(bad), None, "{bad:?} should not parse");
        }
    }

    // --- parse_chart_binding ----------------------------------------------------------------

    #[test]
    fn parse_chart_binding_maps_roles_in_series_order() {
        // The line fixture's chart part: two cat/val series over the `Data` grid.
        let xml = authoring::line_chart_xml_for_test();
        let binding = parse_chart_binding(&xml);
        assert_eq!(binding.series.len(), 2);

        let s0 = &binding.series[0];
        assert_eq!(s0.name.as_ref().unwrap().areas[0].range, range("B1:B1"));
        assert_eq!(s0.cat.as_ref().unwrap().areas[0].range, range("A2:A5"));
        assert_eq!(s0.val.as_ref().unwrap().areas[0].range, range("B2:B5"));

        let s1 = &binding.series[1];
        assert_eq!(s1.name.as_ref().unwrap().areas[0].range, range("C1:C1"));
        assert_eq!(s1.val.as_ref().unwrap().areas[0].range, range("C2:C5"));
        // Every ref points at the `Data` sheet.
        assert_eq!(
            s0.val.as_ref().unwrap().areas[0].sheet.as_deref(),
            Some("Data")
        );
    }

    // --- resolution -------------------------------------------------------------------------

    /// A fake model: `(SheetId, CellRef) -> CellData`, plus a name→id table.
    struct FakeModel {
        sheets: HashMap<String, SheetId>,
        cells: HashMap<(SheetId, CellRef), CellData>,
    }

    impl FakeModel {
        fn one_sheet(name: &str) -> Self {
            let mut sheets = HashMap::new();
            sheets.insert(name.to_string(), SheetId(0));
            Self {
                sheets,
                cells: HashMap::new(),
            }
        }
        fn set(&mut self, cell: CellRef, data: CellData) {
            self.cells.insert((SheetId(0), cell), data);
        }
        fn resolver(&self) -> impl Fn(&str) -> Option<SheetId> + '_ {
            move |name| self.sheets.get(name).copied()
        }
        fn reader(&self) -> impl Fn(SheetId, CellRef) -> CellData + '_ {
            move |sheet, cell| {
                self.cells
                    .get(&(sheet, cell))
                    .cloned()
                    .unwrap_or(CellData::Empty)
            }
        }
    }

    fn line_template() -> Chart {
        Chart {
            title: None,
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("Widgets"),
                vec![Category::Text("Q1".into()), Category::Text("Q2".into())],
                vec![1.0, 2.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn resolve_chart_reflects_live_values() {
        let binding = ChartBinding {
            series: vec![SeriesBinding {
                name: None,
                cat: parse_cf("Data!$A$2:$A$3"),
                val: parse_cf("Data!$B$2:$B$3"),
            }],
        };
        let mut model = FakeModel::one_sheet("Data");
        model.set(cell(1, 0), CellData::Text("Q1".into())); // A2
        model.set(cell(2, 0), CellData::Text("Q9".into())); // A3 edited
        model.set(cell(1, 1), CellData::Number(42.0)); // B2 edited
                                                       // B3 left empty → non-numeric → NaN (blanked), positions still aligned.

        let resolver = model.resolver();
        let reader = model.reader();
        let chart = resolve_chart(&line_template(), &binding, SheetId(0), &resolver, &reader);

        match &chart.series[0].data {
            SeriesData::CategoryValue { categories, values } => {
                assert_eq!(categories[1], Category::Text("Q9".into()));
                assert_eq!(values[0], 42.0);
                assert!(values[1].is_nan(), "empty value cell blanks to NaN");
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        // Styling / name preserved from the template.
        assert_eq!(chart.series[0].name.as_deref(), Some("Widgets"));
    }

    #[test]
    fn resolve_falls_back_to_cache_when_sheet_unresolvable() {
        let binding = ChartBinding {
            series: vec![SeriesBinding {
                name: None,
                cat: parse_cf("Ghost!$A$2:$A$3"),
                val: parse_cf("Ghost!$B$2:$B$3"),
            }],
        };
        let model = FakeModel::one_sheet("Data"); // no "Ghost" sheet
        let resolver = model.resolver();
        let reader = model.reader();
        let template = line_template();
        let chart = resolve_chart(&template, &binding, SheetId(0), &resolver, &reader);
        // Unresolvable → the template's cached values are kept unchanged.
        assert_eq!(chart, template);
    }

    // --- dirty index ------------------------------------------------------------------------

    fn spec_for(chart_xml: &str) -> ChartSpec {
        ChartSpec::loaded(
            line_template(),
            SourceXml::new(chart_xml),
            Vec::new(),
            Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(4, 8)),
        )
    }

    #[test]
    fn dirty_indices_selects_only_intersecting_charts() {
        // Chart 0 binds Data!B2:B5; chart 1 binds Data!D2:D5 (disjoint columns).
        let bindings = ChartBindings {
            charts: vec![
                BoundChart {
                    anchor_sheet: SheetId(0),
                    chart_part: "xl/charts/chart1.xml".into(),
                    id: ChartId::NONE,
                    spec: spec_for("<c:lineChart/>"),
                    binding: ChartBinding {
                        series: vec![SeriesBinding {
                            name: None,
                            cat: parse_cf("Data!$A$2:$A$5"),
                            val: parse_cf("Data!$B$2:$B$5"),
                        }],
                    },
                },
                BoundChart {
                    anchor_sheet: SheetId(0),
                    chart_part: "xl/charts/chart2.xml".into(),
                    id: ChartId::NONE,
                    spec: spec_for("<c:lineChart/>"),
                    binding: ChartBinding {
                        series: vec![SeriesBinding {
                            name: None,
                            cat: parse_cf("Data!$A$2:$A$5"),
                            val: parse_cf("Data!$D$2:$D$5"),
                        }],
                    },
                },
            ],
        };
        let model = FakeModel::one_sheet("Data");
        let resolver = model.resolver();

        // Edit B3 (inside chart 0's value range) → only chart 0.
        let hit = bindings.dirty_indices(&[(SheetId(0), range("B3"))], &[], &resolver);
        assert_eq!(hit, vec![0]);

        // Edit Z9, disjoint from both → nothing recomputes.
        let miss = bindings.dirty_indices(&[(SheetId(0), range("Z9"))], &[], &resolver);
        assert!(miss.is_empty());

        // A structural rebuild of the Data sheet touches every chart bound to it.
        let structural = bindings.dirty_indices(&[], &[SheetId(0)], &resolver);
        assert_eq!(structural, vec![0, 1]);
    }

    #[test]
    fn from_specs_by_sheet_anchors_each_group_to_its_sheet() {
        // Two charts on two different worksheets (multi-sheet placement, P10).
        let bindings = ChartBindings::from_specs_by_sheet(vec![
            (
                SheetId(0),
                vec![("xl/charts/chart1.xml".into(), spec_for("<c:lineChart/>"))],
            ),
            (
                SheetId(3),
                vec![
                    ("xl/charts/chart2.xml".into(), spec_for("<c:lineChart/>")),
                    ("xl/charts/chart3.xml".into(), spec_for("<c:barChart/>")),
                ],
            ),
        ]);
        let by_sheet = bindings.specs_by_sheet();
        assert_eq!(by_sheet.len(), 2);
        assert_eq!(by_sheet[0].0, SheetId(0));
        assert_eq!(by_sheet[0].1.len(), 1);
        assert_eq!(by_sheet[1].0, SheetId(3));
        assert_eq!(by_sheet[1].1.len(), 2);

        // Each chart's live descriptor carries its own part + the resolved (current) sheet name.
        let names = |id: SheetId| (id == SheetId(0)).then(|| "Data".to_string());
        let live = bindings.live_charts(names);
        assert_eq!(live.len(), 3);
        assert_eq!(live[0].chart_part, "xl/charts/chart1.xml");
        assert_eq!(live[0].sheet_name.as_deref(), Some("Data"));
        // Sheet 3 doesn't resolve here → its charts report a deleted host (None).
        assert_eq!(live[1].chart_part, "xl/charts/chart2.xml");
        assert_eq!(live[1].sheet_name, None);
    }

    /// P14: an **Unsupported** spec (a retained surface chart) has no typed chart — it binds static:
    /// its live descriptor carries `chart: None` (byte-preserved on save, never patched), it never
    /// enters the dirty set (empty binding), and a forced re-resolve is a no-op that never panics on
    /// the missing `Chart`.
    #[test]
    fn unsupported_spec_binds_static_and_reresolves_to_a_noop() {
        let unsupported = ChartSpec::loaded_unsupported(
            Some("Terrain".into()),
            SourceXml::new("<c:surfaceChart/>"),
            Vec::new(),
            Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(4, 8)),
        );
        let mut bindings = ChartBindings::from_specs_by_sheet(vec![(
            SheetId(0),
            vec![("xl/charts/chart1.xml".into(), unsupported)],
        )]);

        // Its live descriptor carries NO chart to reflow.
        let live = bindings.live_charts(|id| (id == SheetId(0)).then(|| "Data".to_string()));
        assert_eq!(live.len(), 1);
        assert!(
            live[0].chart.is_none(),
            "an unsupported chart has no values to reflow"
        );

        // Not dirty on a structural rebuild (empty binding), and a forced re-resolve is a no-op.
        let model = FakeModel::one_sheet("Data");
        let resolver = model.resolver();
        let reader = model.reader();
        assert!(bindings
            .dirty_indices(&[], &[SheetId(0)], &resolver)
            .is_empty());
        assert!(
            !bindings.reresolve(&[0], &resolver, &reader),
            "reresolve is a no-op on an unsupported spec"
        );
    }

    // --- P11: lazy-discovery merge + coalesced recompute ------------------------------------

    #[test]
    fn add_missing_dedupes_by_chart_part_and_appends_new() {
        // Start with chart1 bound (a lazily-painted sheet).
        let mut bindings = ChartBindings::from_specs_by_sheet(vec![(
            SheetId(0),
            vec![("xl/charts/chart1.xml".into(), spec_for("<c:lineChart/>"))],
        )]);

        // Re-adding chart1 (e.g. the save-time full sweep re-reaching an already-painted sheet) is a
        // NO-OP — never re-binds it or clobbers its live values.
        let re_added = bindings.add_missing(vec![(
            SheetId(0),
            vec![("xl/charts/chart1.xml".into(), spec_for("<c:barChart/>"))],
        )]);
        assert!(!re_added, "an already-bound chart part is not re-added");
        assert_eq!(bindings.specs_by_sheet()[0].1.len(), 1);

        // A not-yet-seen part IS appended (the save-time sweep picking up a never-painted sheet).
        let added = bindings.add_missing(vec![(
            SheetId(3),
            vec![("xl/charts/chart2.xml".into(), spec_for("<c:barChart/>"))],
        )]);
        assert!(added, "a new chart part is appended");
        let by_sheet = bindings.specs_by_sheet();
        assert_eq!(by_sheet.len(), 2);
        assert_eq!(by_sheet[1].0, SheetId(3));
        assert_eq!(by_sheet[1].1.len(), 1);
    }

    #[test]
    fn coalesced_multi_edit_recompute_is_one_pass() {
        // Two charts on the same sheet: chart 0 reads B2:B3, chart 1 reads D2:D3 (disjoint columns),
        // both categorised on A2:A3. This is the shape the worker coalesces: a single drained edit
        // batch that touches BOTH charts' ranges must re-resolve them in ONE dirty-set pass, not two.
        let bind = |val: &str| ChartBinding {
            series: vec![SeriesBinding {
                name: None,
                cat: parse_cf("Data!$A$2:$A$3"),
                val: parse_cf(val),
            }],
        };
        let mut bindings = ChartBindings {
            charts: vec![
                BoundChart {
                    anchor_sheet: SheetId(0),
                    chart_part: "xl/charts/chart1.xml".into(),
                    id: ChartId::NONE,
                    spec: spec_for("<c:lineChart/>"),
                    binding: bind("Data!$B$2:$B$3"),
                },
                BoundChart {
                    anchor_sheet: SheetId(0),
                    chart_part: "xl/charts/chart2.xml".into(),
                    id: ChartId::NONE,
                    spec: spec_for("<c:lineChart/>"),
                    binding: bind("Data!$D$2:$D$3"),
                },
            ],
        };

        let mut model = FakeModel::one_sheet("Data");
        model.set(cell(1, 1), CellData::Number(10.0)); // B2
        model.set(cell(2, 1), CellData::Number(20.0)); // B3
        model.set(cell(1, 3), CellData::Number(30.0)); // D2
        model.set(cell(2, 3), CellData::Number(40.0)); // D3
        let resolver = model.resolver();
        let reader = model.reader();

        // ONE coalesced edit set covering cells in BOTH charts' value ranges → BOTH charts dirty.
        let edited = &[(SheetId(0), range("B2")), (SheetId(0), range("D2"))];
        let dirty = bindings.dirty_indices(edited, &[], &resolver);
        assert_eq!(
            dirty,
            vec![0, 1],
            "both charts are selected in one dirty pass"
        );

        // ONE re-resolve pass updates both (the coalesced recompute), reporting a real change.
        let changed = bindings.reresolve(&dirty, &resolver, &reader);
        assert!(changed, "the coalesced re-resolve changed the charts");

        // ONE snapshot then carries both charts' fresh values — the single publish the worker emits.
        let snap = bindings.specs_by_sheet();
        assert_eq!(snap.len(), 1);
        let value0 = match &snap[0].1[0].chart().unwrap().series[0].data {
            SeriesData::CategoryValue { values, .. } => values.clone(),
            other => panic!("expected CategoryValue, got {other:?}"),
        };
        let value1 = match &snap[0].1[1].chart().unwrap().series[0].data {
            SeriesData::CategoryValue { values, .. } => values.clone(),
            other => panic!("expected CategoryValue, got {other:?}"),
        };
        assert_eq!(
            value0,
            vec![10.0, 20.0],
            "chart 0 reflects its edited B-column"
        );
        assert_eq!(
            value1,
            vec![30.0, 40.0],
            "chart 1 reflects its edited D-column"
        );
    }
}
