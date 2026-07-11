//! Load stitching: walk the OOXML `worksheet → drawing → chart` relationship chain in an
//! `.xlsx` zip and parse each embedded chart's XML into the [`freecell_chart_model::Chart`] model,
//! reading **cached** `numCache`/`strCache` values (no formula evaluation, no IronCalc).
//!
//! This is the read side of Experiment 1 (functional_spec §5). It is deliberately
//! IronCalc-free and gpui-free: the same `zip` + `roxmltree` second pass `open_fixups.rs`
//! already does, producing `chart-model` values the render crate draws.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};

use freecell_chart_model::{
    normalize_3d_chart_group, Anchor, AnchorCell, Axis, BarDir, BarLayout, Category, CfRange,
    Chart, ChartColor, ChartKind, ChartSpec, Color, DataLabelPosition, DataLabels, Grouping,
    Legend, LegendPosition, LineStroke, Series, SourcePart, SourceXml, ThemeSlot,
};

use super::xlsx::{self, attr};

/// One chart discovered under a worksheet's `<drawing>`: its package part name plus the
/// [`Anchor`] parsed from the `xdr:*Anchor` graphic frame that references it. Produced in
/// document order by [`discover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredChart {
    /// The chart part name, e.g. `xl/charts/chart1.xml`.
    pub part: String,
    /// The chart's in-grid placement (`xdr:twoCellAnchor` from/to). Best-effort: a chart
    /// whose graphic frame carries no resolvable anchor gets a zero anchor at `A1` (P8 maps
    /// this to pixels; a malformed anchor never fails the load).
    pub anchor: Anchor,
}

/// One discovered chart: its `xl/charts/chartN.xml` part paired with its parsed [`ChartSpec`]. The
/// part is the stable key the source-first save re-injects + reflows on (P10).
pub type PartAndSpec = (String, ChartSpec);
/// The charts on one worksheet, in document order (part + spec each).
pub type SheetCharts = Vec<PartAndSpec>;
/// Charts grouped by their owning worksheet **name**, in discovery order — the shape
/// [`discover_and_parse_by_sheet`] returns and the worker binds by `SheetId`.
pub type ChartsBySheet = Vec<(String, SheetCharts)>;

/// The charts carried by one worksheet's single `<drawing>`, plus everything the save
/// re-injection needs to patch that worksheet. Discovered by [`discover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetDrawing {
    /// e.g. `xl/worksheets/sheet1.xml`.
    pub sheet_part: String,
    /// e.g. `xl/drawings/drawing1.xml`.
    pub drawing_part: String,
    /// The `r:id` on the worksheet's `<drawing>` element (worksheet → drawing).
    pub drawing_rel_id: String,
    /// The relationship `Type` URI for that worksheet → drawing relationship.
    pub drawing_rel_type: String,
    /// The charts this drawing's graphic frames reference (part name + anchor), in document
    /// order.
    pub charts: Vec<DiscoveredChart>,
}

/// Walks every worksheet in the package and returns the `<drawing>`-bearing ones with their
/// resolved chart part names. Worksheets without a `<drawing>` (or whose drawing carries no
/// charts) are omitted. Namespace/prefix-agnostic throughout.
///
/// **Per-drawing non-fatal** (charts/functional_spec §1, architecture §6, P14): a **broken drawing
/// relationship** on one worksheet — a missing drawing `_rels` part, a missing drawing part, or an
/// individual `<c:chart r:id>` whose `rId` is absent — drops just *that* drawing/chart (logged) and
/// the walk continues, so the rest of the workbook and its other charts still open. Only a genuinely
/// unreadable **package** (the zip itself won't open) is a hard error — the workbook can't open at
/// all in that case either.
pub fn discover(path: &Path) -> Result<Vec<SheetDrawing>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;

    // Enumerate worksheet parts directly (xl/worksheets/sheetN.xml, not the _rels siblings).
    let sheet_parts: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| {
            n.starts_with("xl/worksheets/") && n.ends_with(".xml") && !n.contains("/_rels/")
        })
        .collect();

    let mut out = Vec::new();
    for sheet_part in sheet_parts {
        match discover_sheet_drawing(&mut archive, &sheet_part) {
            Ok(Some(sheet_drawing)) => out.push(sheet_drawing),
            Ok(None) => {} // no drawing, or a drawing with no resolvable charts
            // A dangling drawing/rel on this worksheet drops just its drawing (never the load).
            Err(err) => tracing::warn!(
                sheet = %sheet_part,
                "skipping a worksheet's broken drawing/chart relationships: {err:#}"
            ),
        }
    }
    // Deterministic order (zip index order is not guaranteed sorted).
    out.sort_by(|a, b| a.sheet_part.cmp(&b.sheet_part));
    Ok(out)
}

/// Resolve one worksheet's `<drawing>` chain into a [`SheetDrawing`] (part + charts), or `None`
/// when the worksheet has no `<drawing>` / its drawing references no resolvable charts. Fallible so
/// [`discover`] can treat a broken drawing relationship as **per-drawing non-fatal** (skip + log)
/// rather than aborting the whole load (charts/architecture §6).
fn discover_sheet_drawing<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    sheet_part: &str,
) -> Result<Option<SheetDrawing>> {
    let sheet_xml = xlsx::read_entry_from(archive, sheet_part)?;
    let Some((rel_id, drawing_part, rel_type)) =
        worksheet_drawing(&sheet_xml, sheet_part, archive)?
    else {
        return Ok(None);
    };
    let drawing_xml = xlsx::read_entry_from(archive, &drawing_part)?;
    let charts = drawing_charts(&drawing_xml, &drawing_part, archive)?;
    if charts.is_empty() {
        return Ok(None);
    }
    Ok(Some(SheetDrawing {
        sheet_part: sheet_part.to_string(),
        drawing_part,
        drawing_rel_id: rel_id,
        drawing_rel_type: rel_type,
        charts,
    }))
}

/// Resolves a worksheet's `<drawing r:id>` to its drawing part via the sheet's `_rels`.
/// Returns `None` when the worksheet has no `<drawing>` element.
fn worksheet_drawing<R: std::io::Read + std::io::Seek>(
    sheet_xml: &str,
    sheet_part: &str,
    archive: &mut zip::ZipArchive<R>,
) -> Result<Option<(String, String, String)>> {
    let doc = Document::parse(sheet_xml).context("parsing worksheet XML")?;
    let Some(drawing) = doc.descendants().find(|n| n.tag_name().name() == "drawing") else {
        return Ok(None);
    };
    let rel_id = attr(&drawing, "id")
        .ok_or_else(|| anyhow!("worksheet <drawing> in {sheet_part} has no r:id"))?
        .to_string();

    let rels_part = xlsx::rels_part_for(sheet_part);
    let rels_xml = xlsx::read_entry_from(archive, &rels_part).with_context(|| {
        format!("worksheet {sheet_part} references a drawing but {rels_part} is missing")
    })?;
    let rels = xlsx::parse_rels(&rels_xml)?;
    let rel = rels
        .get(&rel_id)
        .ok_or_else(|| anyhow!("{rels_part} has no relationship {rel_id}"))?;
    let drawing_part = xlsx::resolve_target(sheet_part, &rel.target);
    Ok(Some((rel_id, drawing_part, rel.rel_type.clone())))
}

/// Collects the charts referenced by a drawing's `<c:chart r:id>` graphic frames, in document
/// order, resolving each `r:id` through the drawing's `_rels` (part name) and reading each
/// frame's enclosing `xdr:*Anchor` (placement).
fn drawing_charts<R: std::io::Read + std::io::Seek>(
    drawing_xml: &str,
    drawing_part: &str,
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<DiscoveredChart>> {
    let doc = Document::parse(drawing_xml).context("parsing drawing XML")?;
    // `<c:chart r:id="...">` frames — one per embedded chart — each paired with the anchor of
    // the graphic frame that holds it.
    let referenced: Vec<(String, Anchor)> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "chart")
        .filter_map(|n| attr(&n, "id").map(|id| (id.to_string(), enclosing_anchor(&n))))
        .collect();
    if referenced.is_empty() {
        return Ok(Vec::new());
    }

    let rels_part = xlsx::rels_part_for(drawing_part);
    let rels_xml = xlsx::read_entry_from(archive, &rels_part).with_context(|| {
        format!("drawing {drawing_part} references charts but {rels_part} is missing")
    })?;
    let rels = xlsx::parse_rels(&rels_xml)?;

    // A `<c:chart r:id>` whose `rId` is absent from the drawing's `_rels` is a dangling reference:
    // skip **that** chart (logged) and keep its siblings — the walk is per-chart resilient (P14,
    // charts/architecture §6), never dropping a whole drawing over one broken relationship.
    let mut charts = Vec::new();
    for (rel_id, anchor) in referenced {
        match rels.get(&rel_id) {
            Some(rel) => charts.push(DiscoveredChart {
                part: xlsx::resolve_target(drawing_part, &rel.target),
                anchor,
            }),
            None => tracing::warn!(
                drawing = %drawing_part,
                rel_id = %rel_id,
                "drawing references a chart relationship absent from its _rels; skipping that chart"
            ),
        }
    }
    Ok(charts)
}

/// The [`Anchor`] of the `xdr:*Anchor` element enclosing a `<c:chart>` graphic frame. Walks up
/// to the first `twoCellAnchor`/`oneCellAnchor`/`absoluteAnchor` ancestor; a chart with no
/// anchor ancestor (unexpected) gets a zero anchor at `A1` so the load never fails on it.
fn enclosing_anchor(chart_node: &Node) -> Anchor {
    chart_node
        .ancestors()
        .find(|n| is_anchor_element(n))
        .map(|el| parse_anchor(&el))
        .unwrap_or_else(|| Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(0, 0)))
}

/// Whether a node is a spreadsheet-drawing anchor element (the three `xdr:` anchor kinds).
fn is_anchor_element(node: &Node) -> bool {
    matches!(
        node.tag_name().name(),
        "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor"
    )
}

/// Parses an anchor element's `<xdr:from>`/`<xdr:to>` cell corners into an [`Anchor`]. A
/// `oneCellAnchor`/`absoluteAnchor` (which carries `<xdr:ext>` instead of `<xdr:to>`) has no
/// `to` corner, so it falls back to `to = from` — a degenerate rectangle P8 can still place.
fn parse_anchor(anchor_el: &Node) -> Anchor {
    let from = child(anchor_el, "from")
        .map(|n| anchor_cell(&n))
        .unwrap_or_else(|| AnchorCell::new(0, 0));
    let to = child(anchor_el, "to")
        .map(|n| anchor_cell(&n))
        .unwrap_or(from);
    Anchor::new(from, to)
}

/// Reads an `<xdr:from>`/`<xdr:to>` corner: its `<xdr:col>`/`<xdr:colOff>`/`<xdr:row>`/
/// `<xdr:rowOff>` children (a missing/unparseable child reads as `0`).
fn anchor_cell(cell: &Node) -> AnchorCell {
    AnchorCell::with_offsets(
        child_number(cell, "col"),
        child_number(cell, "colOff"),
        child_number(cell, "row"),
        child_number(cell, "rowOff"),
    )
}

/// The integer text of a named child element (`0` when absent or unparseable).
fn child_number<T: std::str::FromStr + Default>(node: &Node, name: &str) -> T {
    child(node, name)
        .and_then(|n| n.text())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_default()
}

/// Loads every embedded chart in the workbook into [`freecell_chart_model::Chart`], in
/// (worksheet, document) order — the bare render pictures, without the production envelope.
/// [`discover_and_parse`] is the full-envelope path (source, ranges, anchor, origin).
pub fn load_charts_from_xlsx(path: &Path) -> Result<Vec<Chart>> {
    let sheets = discover(path)?;
    let mut charts = Vec::new();
    for sheet in &sheets {
        for dc in &sheet.charts {
            let xml = xlsx::read_entry(path, &dc.part)
                .with_context(|| format!("reading chart part {}", dc.part))?;
            let chart =
                parse_chart_xml(&xml).with_context(|| format!("parsing chart part {}", dc.part))?;
            charts.push(chart);
        }
    }
    Ok(charts)
}

/// Discovers every embedded chart and parses it into a full [`freecell_chart_model::ChartSpec`]
/// — the production envelope (charts/architecture §3.2, §4.1): the render [`Chart`] wrapped with
/// its retained **source** (the `chartN.xml` verbatim + its related parts), its `c:f`
/// **source ranges** (live binding, P9), and its `twoCellAnchor` **anchor** (in-grid placement,
/// P8), with [`Origin::Loaded`](freecell_chart_model::Origin::Loaded). Charts come back in
/// (worksheet, document) order.
///
/// This is the read side the whole chart pipeline hangs off: the engine produces `ChartSpec`s,
/// the app consumes them. It reads **cached** values only (no IronCalc eval); live resolution of
/// the retained `source_ranges` is P9.
///
/// **A bad chart is per-chart non-fatal, never fatal to the load** (charts/architecture §6,
/// functional_spec §1). Two distinct kinds of "bad", handled differently (P14):
/// - A chart the walk **reaches but can't parse into a typed [`Chart`]** — an unsupported group our
///   `parse_chart_xml` doesn't recognize (surface / radar / stock / ofPie / bubble), or malformed
///   chart XML — is **retained as an Unsupported [`ChartSpec`]** ([`ChartSpec::loaded_unsupported`]):
///   it keeps its source XML + anchor + `c:f` ranges, its
///   [`display_fidelity`](ChartSpec::display_fidelity) is [`Unsupported`](freecell_chart_model::Fidelity::Unsupported)
///   so P8 renders its placeholder in-grid and P10 byte-preserves it on save, but it carries no
///   render picture. (A **3-D** group is instead normalized to its 2-D equivalent by
///   [`parse_chart_xml`] and classifies Degraded — it parses, it isn't dropped.)
/// - A chart whose **part can't even be read** (a missing chart part, a malformed chart `_rels`) —
///   there is nothing to retain — is **skipped and logged**, and the walk continues.
///
/// A corrupt **drawing relationship** in the shared [`discover`] walk (a missing drawing `_rels`,
/// an absent `<c:chart r:id>` `rId`, a missing drawing part) is likewise per-drawing non-fatal —
/// [`discover`] drops just that drawing/chart and keeps the rest. Opening a workbook must never
/// break on one broken chart or drawing.
pub fn discover_and_parse(path: &Path) -> Result<Vec<ChartSpec>> {
    let sheets = discover(path)?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;

    let mut specs = Vec::new();
    for sheet in &sheets {
        for dc in &sheet.charts {
            match parse_discovered_chart(&mut archive, dc) {
                // An unparseable-but-readable chart comes back as a retained Unsupported spec (see
                // `parse_discovered_chart`); only a genuinely UNREADABLE part is an Err here.
                Ok(spec) => specs.push(spec),
                Err(err) => {
                    tracing::warn!(chart_part = %dc.part, "skipping unreadable chart part: {err:#}");
                }
            }
        }
    }
    Ok(specs)
}

/// Discovers every embedded chart and parses it into a [`ChartSpec`], **grouped by the name of the
/// worksheet it is anchored on** (charts/architecture §4.1). This is the multi-sheet placement P9
/// deferred to P10: it resolves each chart-bearing worksheet's part → **sheet name** via
/// `xl/_rels/workbook.xml.rels` (the same [`workbook_sheet_parts`](xlsx::workbook_sheet_parts) map
/// the save part-map uses), so the worker can anchor each chart to its own `SheetId` instead of
/// pinning them all to the first sheet.
///
/// Groups come back in worksheet-discovery order; within a group, each chart is paired with its
/// **package part** (`xl/charts/chartN.xml`) in document order — the part the save later keys its
/// re-injection + reflow on, so the worker never has to re-derive the chart↔part association.
/// Per-chart resilience matches [`discover_and_parse`] — an unparseable chart is skipped + logged,
/// never fatal to the load. A worksheet whose name can't be resolved falls back to its part name as
/// the group key (still distinct per worksheet).
pub fn discover_and_parse_by_sheet(path: &Path) -> Result<ChartsBySheet> {
    let sheets = discover(path)?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;

    let part_to_name: std::collections::HashMap<String, String> =
        xlsx::workbook_sheet_parts(&mut archive)?
            .into_iter()
            .map(|(name, part)| (part, name))
            .collect();

    let mut groups: ChartsBySheet = Vec::new();
    for sheet in &sheets {
        let sheet_name = part_to_name
            .get(&sheet.sheet_part)
            .cloned()
            .unwrap_or_else(|| sheet.sheet_part.clone());
        let mut specs = Vec::new();
        for dc in &sheet.charts {
            match parse_discovered_chart(&mut archive, dc) {
                Ok(spec) => specs.push((dc.part.clone(), spec)),
                Err(err) => {
                    tracing::warn!(chart_part = %dc.part, "skipping unreadable chart part: {err:#}");
                }
            }
        }
        if !specs.is_empty() {
            groups.push((sheet_name, specs));
        }
    }
    Ok(groups)
}

/// Discovers + parses **only** the charts anchored on the one worksheet whose package part is
/// `sheet_part` (e.g. `xl/worksheets/sheet2.xml`) — the worker's **per-sheet lazy discovery**
/// primitive (charts/architecture §5 challenge 5, "lazy parse"). Keying on the **stable package
/// part** (never the live sheet name) is what keeps discovery rename-safe: the worker captures each
/// [`SheetId`](freecell_core::SheetId) → part correspondence once at open (before any in-session
/// rename) and drives both this and the save-time sweep off it, so a sheet renamed before it is
/// painted still resolves to its charts. Only that worksheet's chart XML is read — nothing is parsed
/// for sheets the user never visits. Returns an **empty** list when the sheet has no `<drawing>` /
/// no charts, or when `sheet_part` isn't in the package. Per-chart resilience matches
/// [`discover_and_parse`] (an unparseable chart is skipped + logged, never fatal).
pub fn discover_and_parse_for_part(path: &Path, sheet_part: &str) -> Result<SheetCharts> {
    let sheets = discover(path)?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;
    match sheets.iter().find(|s| s.sheet_part == sheet_part) {
        Some(sheet) => Ok(parse_sheet_charts(&mut archive, sheet)),
        None => Ok(Vec::new()),
    }
}

/// Discovers + parses **only** the charts anchored on the one worksheet named `sheet_name`,
/// resolving the name → its package part via the `workbook.xml.rels` map. A name-keyed convenience
/// (used by the perf harness + tests over a fresh fixture, where names match the file); the worker
/// uses the rename-safe part-keyed [`discover_and_parse_for_part`] instead. Returns empty when the
/// name isn't a file worksheet or its sheet carries no charts.
pub fn discover_and_parse_for_sheet(path: &Path, sheet_name: &str) -> Result<SheetCharts> {
    let sheets = discover(path)?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;
    let name_to_part: std::collections::HashMap<String, String> =
        xlsx::workbook_sheet_parts(&mut archive)?
            .into_iter()
            .collect();
    let Some(target_part) = name_to_part.get(sheet_name) else {
        return Ok(Vec::new());
    };
    match sheets.iter().find(|s| &s.sheet_part == target_part) {
        Some(sheet) => Ok(parse_sheet_charts(&mut archive, sheet)),
        None => Ok(Vec::new()),
    }
}

/// The workbook's `SheetId`-independent **name → worksheet-part** map, read straight from the file's
/// `workbook.xml.rels` (no chart XML parsed). The worker joins this with the model's at-open sheet
/// names to capture the stable `SheetId → part` correspondence that makes lazy discovery + save
/// rename-safe (P11 CR fix). Returns `(sheet name, worksheet part)` pairs in workbook order.
pub fn workbook_sheet_parts(path: &Path) -> Result<Vec<(String, String)>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;
    xlsx::workbook_sheet_parts(&mut archive)
}

/// Parse every chart anchored on one worksheet's `<drawing>` into `(part, ChartSpec)` pairs, in
/// document order. Per-chart non-fatal (an unparseable chart is skipped + logged). Shared by the
/// part- and name-keyed per-sheet discovery entry points.
fn parse_sheet_charts<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    sheet: &SheetDrawing,
) -> SheetCharts {
    let mut specs = Vec::new();
    for dc in &sheet.charts {
        match parse_discovered_chart(archive, dc) {
            Ok(spec) => specs.push((dc.part.clone(), spec)),
            Err(err) => {
                tracing::warn!(chart_part = %dc.part, "skipping unreadable chart part: {err:#}");
            }
        }
    }
    specs
}

/// Parses one discovered chart into a [`ChartSpec`], **retaining** its source + ranges + anchor
/// regardless of whether we can build a typed [`Chart`] (charts/architecture §6, P14). A chart is
/// **only ever fully dropped when its own chart XML part is unreadable/absent** (a genuinely missing
/// part); everything else is retained:
/// - a part that parses into a [`Chart`] **and** whose aux parts read → [`ChartSpec::loaded`]
///   (Faithful/Degraded per its source);
/// - a **readable but unparseable** part (an unsupported group, malformed chart XML), **or** a
///   parseable part whose aux `_rels` is malformed/unreadable → [`ChartSpec::loaded_unsupported`]
///   with the salvaged title, so it still byte-preserves on save and renders its placeholder
///   in-grid (`display_fidelity() == Unsupported`). A broken aux `_rels` retains the chart with
///   **empty** related parts (its own chart XML + anchor + ranges are still kept) rather than
///   dropping it.
///
/// Fallible **only** when the chart XML part itself can't be read (missing/absent) — there is
/// nothing to retain — so [`discover_and_parse`] can skip + log that narrow case.
fn parse_discovered_chart<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dc: &DiscoveredChart,
) -> Result<ChartSpec> {
    let chart_xml = xlsx::read_entry_from(archive, &dc.part)
        .with_context(|| format!("reading chart part {}", dc.part))?;
    let source_ranges = parse_cf_ranges(&chart_xml);
    let parsed = parse_chart_xml(&chart_xml);

    // Aux parts are **best-effort**: a malformed/unreadable aux `_rels` must NOT drop the chart (its
    // own XML is already in hand). `None` here means "couldn't read the aux parts" → retain the chart
    // as a placeholder with no related parts (a chart whose styling deps we can't read isn't drawn as
    // itself). An absent `_rels` reads as `Some(empty)` (the ordinary no-aux-parts case).
    let related_parts = match read_related_parts(archive, &dc.part) {
        Ok(related) => Some(related),
        Err(err) => {
            tracing::warn!(
                chart_part = %dc.part,
                "chart aux _rels unreadable; retaining as an Unsupported placeholder: {err:#}"
            );
            None
        }
    };

    match (parsed, related_parts) {
        // Parses cleanly AND its aux parts read → the full render envelope.
        (Ok(chart), Some(related)) => {
            let source = SourceXml::new(chart_xml).with_related_parts(related);
            Ok(ChartSpec::loaded(chart, source, source_ranges, dc.anchor))
        }
        // Otherwise retain as a placeholder: unparseable chart XML, or a parseable one whose aux
        // parts failed to read. Salvage the title (from the parsed chart if we have it, else from the
        // source text) and keep whatever source we could (related parts if they read, else none).
        (parsed, related) => {
            if let Err(err) = &parsed {
                tracing::debug!(
                    chart_part = %dc.part,
                    "retaining unparseable chart as an Unsupported placeholder: {err:#}"
                );
            }
            let title = match &parsed {
                Ok(chart) => chart.title.clone(),
                Err(_) => chart_title_from_xml(&chart_xml),
            };
            let source = SourceXml::new(chart_xml).with_related_parts(related.unwrap_or_default());
            Ok(ChartSpec::loaded_unsupported(
                title,
                source,
                source_ranges,
                dc.anchor,
            ))
        }
    }
}

/// Collects every `<c:f>` data-reference formula in a chart part, as written, in document order
/// — a [`CfRange`] per `<c:f>` whose parent is a `*Ref` (`numRef`/`strRef`/`multiLvlStrRef`).
/// These are retained raw for live binding (P9); structured sheet/range decomposition + index
/// resolution is P9's job. Whitespace-only / empty formulas are skipped.
fn parse_cf_ranges(chart_xml: &str) -> Vec<CfRange> {
    let Ok(doc) = Document::parse(chart_xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "f")
        .filter(|n| {
            n.parent()
                .is_some_and(|p| p.tag_name().name().ends_with("Ref"))
        })
        .filter_map(|n| n.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(CfRange::new)
        .collect()
}

/// Retains a chart part's own related parts as raw bytes (charts/architecture §3.2) — its
/// `_rels` (`xl/charts/_rels/chartN.xml.rels`) plus every non-external part that `_rels`
/// references (`colorsN.xml`, `styleN.xml`, embeddings) that exists in the package. A chart
/// with no `_rels` retains nothing. A missing referenced target is skipped (never a hard
/// error); a malformed `_rels` is a hard error (the package is broken).
fn read_related_parts<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    chart_part: &str,
) -> Result<Vec<SourcePart>> {
    let rels_part = xlsx::rels_part_for(chart_part);
    if !xlsx::has_entry(archive, &rels_part) {
        return Ok(Vec::new());
    }
    let rels_bytes = xlsx::read_entry_bytes_from(archive, &rels_part)?;
    let rels = xlsx::parse_rels(
        std::str::from_utf8(&rels_bytes).with_context(|| format!("{rels_part} is not UTF-8"))?,
    )?;

    let mut parts = vec![SourcePart::new(rels_part, rels_bytes)];
    for rel in rels.values() {
        let target = xlsx::resolve_target(chart_part, &rel.target);
        if xlsx::has_entry(archive, &target) {
            let bytes = xlsx::read_entry_bytes_from(archive, &target)?;
            parts.push(SourcePart::new(target, bytes));
        }
    }
    Ok(parts)
}

// ---------------------------------------------------------------------------------------------
// Chart XML → chart-model
// ---------------------------------------------------------------------------------------------

/// The chart-group element tag names this PoC understands, in the order they're checked.
/// `pub(super)` so the live-binding [`binding`](super::binding) parser finds the **same** first
/// chart-group as [`parse_chart_xml`], keeping its per-series role refs aligned 1:1 with the
/// parsed [`Chart`]'s series (combo charts read only the first group — functional_spec §10).
pub(super) const CHART_GROUP_TAGS: &[&str] = &[
    "barChart",
    "lineChart",
    "areaChart",
    "pieChart",
    "doughnutChart",
    "scatterChart",
];

/// Whether `local_name` names a chart-group element we parse: one of the 2-D [`CHART_GROUP_TAGS`],
/// **or** a 3-D group (`bar3DChart` / `line3DChart` / `pie3DChart` / `area3DChart`) that
/// [`parse_chart_xml`] normalizes to its 2-D equivalent (charts/functional_spec §5, "3-D → 2-D + a
/// compatibility flag"). Shared by [`parse_chart_xml`], the live-binding parser
/// ([`binding`](super::binding)), and the save reflow ([`save`](super::save)) so all three agree on
/// which element is the chart group — including the 3-D case, so a Degraded 3-D chart still
/// live-binds + reflows off its (2-D-parsed) series.
pub(super) fn is_chart_group(local_name: &str) -> bool {
    CHART_GROUP_TAGS.contains(&local_name) || normalize_3d_chart_group(local_name).is_some()
}

/// Parses a single `xl/charts/chartN.xml` document (`c:chartSpace`) into a [`Chart`], reading
/// cached values only. Returns an error if no recognized chart-group element is present.
///
/// A **3-D** chart group is normalized to its 2-D equivalent (`bar3DChart` → a `Bar` kind, etc.)
/// via [`normalize_3d_chart_group`], so a 3-D chart parses (and later classifies
/// [`Degraded`](freecell_chart_model::Fidelity::Degraded), since its retained source still names the
/// 3-D element) rather than being dropped (charts/functional_spec §5).
pub fn parse_chart_xml(xml: &str) -> Result<Chart> {
    let doc = Document::parse(xml).context("parsing chart XML")?;
    let root = doc.root_element(); // c:chartSpace
    let chart = child(&root, "chart").ok_or_else(|| anyhow!("no <c:chart> in chartSpace"))?;
    let plot_area = child(&chart, "plotArea").ok_or_else(|| anyhow!("no <c:plotArea> in chart"))?;

    let group = plot_area
        .children()
        .find(|n| n.is_element() && is_chart_group(n.tag_name().name()))
        .ok_or_else(|| anyhow!("no recognized chart-group element in <c:plotArea>"))?;

    // Normalize a 3-D group to its 2-D equivalent name before reading its kind.
    let group_name = group.tag_name().name();
    let kind_name = normalize_3d_chart_group(group_name).unwrap_or(group_name);
    let kind = parse_kind(&group, kind_name)?;
    let is_scatter = matches!(kind, ChartKind::Scatter);

    // The chart-group-level `c:dLbls` (a direct child of the group, after the series) is the
    // default for every series that has no `c:dLbls` of its own (OOXML: a series `c:dLbls`
    // *replaces* the chart-level default for that series).
    let group_labels = parse_data_labels_of(&group);

    let series = group
        .children()
        .filter(|n| n.tag_name().name() == "ser")
        .map(|ser| parse_series(&ser, is_scatter, group_labels.as_ref()))
        .collect::<Result<Vec<_>>>()?;

    let (cat_axis, val_axis) = parse_axes(&plot_area, is_scatter);

    Ok(Chart {
        title: parse_title(&chart),
        kind,
        series,
        cat_axis,
        val_axis,
        legend: parse_legend(&chart),
    })
}

/// The [`ChartKind`] for a chart-group element named `kind_name` (the group node's own name, or its
/// 2-D equivalent when the source group was 3-D — see [`parse_chart_xml`]), reading `c:barDir` /
/// `c:grouping` / `c:holeSize` from `group` as needed.
fn parse_kind(group: &Node, kind_name: &str) -> Result<ChartKind> {
    Ok(match kind_name {
        "barChart" => {
            let dir = match child_val(group, "barDir").as_deref() {
                Some("bar") => BarDir::Bar,
                _ => BarDir::Col, // default column
            };
            ChartKind::Bar {
                dir,
                grouping: grouping_of(group, Grouping::Clustered),
                layout: bar_layout(group),
            }
        }
        "lineChart" => ChartKind::Line {
            grouping: grouping_of(group, Grouping::Standard),
            smooth: child_val(group, "smooth").as_deref() == Some("1"),
        },
        "areaChart" => ChartKind::Area {
            grouping: grouping_of(group, Grouping::Standard),
        },
        "pieChart" => ChartKind::Pie {
            doughnut_hole: None,
        },
        "doughnutChart" => {
            // c:holeSize is a percentage of the outer radius (default 50 when absent).
            let hole = child_val(group, "holeSize")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(50.0)
                / 100.0;
            ChartKind::Pie {
                doughnut_hole: Some(hole),
            }
        }
        "scatterChart" => ChartKind::Scatter,
        other => return Err(anyhow!("unhandled chart-group element <c:{other}>")),
    })
}

/// The bar-slot spacing ([`BarLayout`], P22) from a `c:barChart` group: `c:gapWidth@val` (clamped to
/// the OOXML `ST_GapAmount` 0..=500, default 150) and `c:overlap@val` (clamped to `ST_Overlap`
/// -100..=100, default 0). An absent element takes its default — Excel omits them at the default, so a
/// plain bar chart round-trips.
fn bar_layout(group: &Node) -> BarLayout {
    let default = BarLayout::default();
    let gap_width = child_val(group, "gapWidth")
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map(|g| g.clamp(0, 500) as u16)
        .unwrap_or(default.gap_width);
    let overlap = child_val(group, "overlap")
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map(|o| o.clamp(-100, 100) as i16)
        .unwrap_or(default.overlap);
    BarLayout { gap_width, overlap }
}

/// Maps a group's `c:grouping@val` to [`Grouping`], falling back to `default` when absent.
fn grouping_of(group: &Node, default: Grouping) -> Grouping {
    match child_val(group, "grouping").as_deref() {
        Some("clustered") => Grouping::Clustered,
        Some("stacked") => Grouping::Stacked,
        Some("percentStacked") => Grouping::PercentStacked,
        Some("standard") => Grouping::Standard,
        _ => default,
    }
}

/// Parses one `<c:ser>` into a [`Series`]. For scatter, reads `c:xVal`/`c:yVal`; otherwise
/// `c:cat`/`c:val`. Series name from `c:tx` cache; color from `c:spPr` solid fill; data labels
/// from the series' own `c:dLbls`, falling back to the chart-group-level `default_labels`.
fn parse_series(
    ser: &Node,
    is_scatter: bool,
    default_labels: Option<&DataLabels>,
) -> Result<Series> {
    let name = child(ser, "tx").and_then(|tx| ref_strings(&tx).into_iter().next());
    let color = parse_series_color(ser);

    let mut series = if is_scatter {
        let x = child(ser, "xVal")
            .map(|n| ref_numbers(&n))
            .unwrap_or_default();
        let y = child(ser, "yVal")
            .map(|n| ref_numbers(&n))
            .unwrap_or_default();
        Series::xy(name, x, y)
    } else {
        let categories = child(ser, "cat")
            .map(|n| ref_categories(&n))
            .unwrap_or_default();
        let values = child(ser, "val")
            .map(|n| ref_numbers(&n))
            .unwrap_or_default();
        Series::category_value(name, categories, values)
    };
    if let Some(c) = color {
        series = series.with_color(c);
    }
    if let Some(stroke) = parse_series_stroke(ser) {
        series = series.with_stroke(stroke);
    }
    // A series `c:dLbls` (even all-off, an explicit "no labels") overrides the chart-level default;
    // a series with none inherits the chart-level default.
    if let Some(labels) = parse_data_labels_of(ser).or_else(|| default_labels.cloned()) {
        series = series.with_data_labels(labels);
    }
    Ok(series)
}

/// The series line stroke from `c:ser/c:spPr/a:ln` (P13): its width (`@w`, EMU→pt), its own
/// `a:solidFill` color (sRGB or a `schemeClr` theme reference), and that fill's `a:alpha`. Returns
/// `None` when there is no `a:ln`, or when it carries nothing we model (e.g. only `a:round`), so a
/// plain series leaves the renderer's default weight.
fn parse_series_stroke(ser: &Node) -> Option<LineStroke> {
    let ln = child(ser, "spPr").and_then(|sp| child(&sp, "ln"))?;
    let width_pt = attr(&ln, "w")
        .and_then(|w| w.trim().parse::<i64>().ok())
        .map(LineStroke::width_pt_from_emu);
    let (color, alpha) = match parse_solid_fill(&ln) {
        Some((c, a)) => (Some(c), a),
        None => (None, None),
    };
    if width_pt.is_none() && color.is_none() && alpha.is_none() {
        return None;
    }
    Some(LineStroke {
        width_pt,
        color,
        alpha,
    })
}

/// Reads a `<a:solidFill>` child of `node` into a [`ChartColor`] plus an optional alpha fraction.
/// Handles both an explicit `a:srgbClr` and a `a:schemeClr` theme reference (with `lumMod`/`lumOff`
/// tint), mirroring [`freecell_chart_model::ChartColor`]. The `a:alpha` (per-mille `val`) rides on
/// the color element and is returned as a `0..=1` fraction.
fn parse_solid_fill(node: &Node) -> Option<(ChartColor, Option<f32>)> {
    let solid = child(node, "solidFill")?;
    if let Some(srgb) = child(&solid, "srgbClr") {
        let v = u32::from_str_radix(attr(&srgb, "val")?.trim(), 16).ok()?;
        return Some((ChartColor::Rgb(Color::from_hex(v)), alpha_fraction(&srgb)));
    }
    if let Some(scheme) = child(&solid, "schemeClr") {
        let slot = ThemeSlot::from_ooxml(attr(&scheme, "val")?)?;
        let color = ChartColor::Theme {
            slot,
            lum_mod: per_mille_fraction(&scheme, "lumMod"),
            lum_off: per_mille_fraction(&scheme, "lumOff"),
        };
        return Some((color, alpha_fraction(&scheme)));
    }
    None
}

/// The `a:alpha` fraction (`val` per-mille ÷ 100000, so `50000` → `0.5`) on a color element, if present.
fn alpha_fraction(color_el: &Node) -> Option<f32> {
    per_mille_fraction(color_el, "alpha")
}

/// The `val` per-mille (÷ 100000) of a color element's named child (`lumMod`/`lumOff`/`alpha`), if present.
fn per_mille_fraction(color_el: &Node, name: &str) -> Option<f32> {
    child_val(color_el, name)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|per_mille| per_mille / 100_000.0)
}

/// The [`DataLabels`] of a node's direct-child `c:dLbls`, if present. Returns `None` when the
/// element is absent (so a series with no `c:dLbls` inherits the chart-level default); a
/// present-but-all-off `c:dLbls` returns `Some` (an explicit "no labels" that overrides the
/// default).
fn parse_data_labels_of(parent: &Node) -> Option<DataLabels> {
    child(parent, "dLbls").map(|dlbls| read_data_labels(&dlbls))
}

/// Reads a `<c:dLbls>` element: the five `show*` toggles, the label `c:numFmt` format code, the
/// part `c:separator`, and the `c:dLblPos` position. Namespace/prefix-agnostic (local-name
/// matching), like the rest of the parser.
fn read_data_labels(dlbls: &Node) -> DataLabels {
    // A label `c:numFmt`; `General`/empty is the benign default and reads as "no explicit format".
    let number_format = child(dlbls, "numFmt")
        .and_then(|n| attr(&n, "formatCode"))
        .map(str::trim)
        .filter(|code| !code.is_empty() && !code.eq_ignore_ascii_case("General"))
        .map(str::to_string);
    let separator = child(dlbls, "separator")
        .and_then(|n| n.text())
        .map(str::to_string);
    let position = child_val(dlbls, "dLblPos").and_then(|v| DataLabelPosition::from_ooxml(&v));

    DataLabels {
        show_legend_key: child_bool(dlbls, "showLegendKey"),
        show_value: child_bool(dlbls, "showVal"),
        show_category_name: child_bool(dlbls, "showCatName"),
        show_series_name: child_bool(dlbls, "showSerName"),
        show_percent: child_bool(dlbls, "showPercent"),
        number_format,
        separator,
        position,
    }
}

/// Whether the first child element named `name` carries a truthy OOXML boolean `val` (`1`/`true`).
fn child_bool(node: &Node, name: &str) -> bool {
    matches!(child_val(node, name).as_deref(), Some("1") | Some("true"))
}

/// The series fill color from `c:ser/c:spPr/a:solidFill` (P22): an explicit `a:srgbClr` **or** a theme
/// `a:schemeClr` (+ `lumMod`/`lumOff` tint), via the shared [`parse_solid_fill`] — the same reader the
/// line stroke uses, so a bar/area/pie series fill honors theme colors (`ooxml-coverage-matrix.md` §C),
/// not only sRGB. The fill's `a:alpha` is dropped (the [`Series`] color model carries none). `None`
/// when the series has no `c:spPr` solid fill (the renderer then cycles the palette).
fn parse_series_color(ser: &Node) -> Option<ChartColor> {
    let sp_pr = child(ser, "spPr")?;
    parse_solid_fill(&sp_pr).map(|(color, _alpha)| color)
}

/// Reads the cached string values under a data-reference holder (`c:tx`, `c:cat`, …): the
/// `<c:pt><c:v>` children of a `strCache`, ordered by `idx`.
fn ref_strings(holder: &Node) -> Vec<String> {
    cache_points(holder, "strCache")
        .into_iter()
        .map(|(_, v)| v)
        .collect()
}

/// Reads the cached numeric values under a data-reference holder (`c:val`, `c:xVal`, …): the
/// `<c:pt><c:v>` children of a `numCache`, ordered by `idx`, parsed as `f64` (unparseable →
/// skipped).
fn ref_numbers(holder: &Node) -> Vec<f64> {
    cache_points(holder, "numCache")
        .into_iter()
        .filter_map(|(_, v)| v.trim().parse::<f64>().ok())
        .collect()
}

/// Reads a `c:cat` into [`Category`] values: a `strCache` yields text categories, a `numCache`
/// yields numeric categories. Prefers whichever cache is present (Excel writes exactly one).
fn ref_categories(holder: &Node) -> Vec<Category> {
    let strs = cache_points(holder, "strCache");
    if !strs.is_empty() {
        return strs.into_iter().map(|(_, v)| Category::Text(v)).collect();
    }
    cache_points(holder, "numCache")
        .into_iter()
        .filter_map(|(_, v)| v.trim().parse::<f64>().ok())
        .map(Category::Number)
        .collect()
}

/// Finds the named cache (`numCache`/`strCache`) anywhere under a data-reference holder and
/// returns its `(idx, value)` points sorted by `idx`. Sparse points (idx gaps, for blanks)
/// are returned as-is; the PoC fixtures are dense.
fn cache_points(holder: &Node, cache_tag: &str) -> Vec<(usize, String)> {
    let Some(cache) = holder
        .descendants()
        .find(|n| n.tag_name().name() == cache_tag)
    else {
        return Vec::new();
    };
    let mut pts: Vec<(usize, String)> = cache
        .children()
        .filter(|n| n.tag_name().name() == "pt")
        .map(|pt| {
            let idx = attr(&pt, "idx")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let v = child(&pt, "v")
                .and_then(|v| v.text())
                .unwrap_or("")
                .to_string();
            (idx, v)
        })
        .collect();
    pts.sort_by_key(|(idx, _)| *idx);
    pts
}

/// The chart title salvaged straight from a chart part's XML text (`c:chart/c:title`), for the
/// placeholder caption of a chart we could **not** parse into a [`Chart`] (an Unsupported spec,
/// P14). Returns `None` when the part won't parse as XML at all, or carries no title.
fn chart_title_from_xml(chart_xml: &str) -> Option<String> {
    let doc = Document::parse(chart_xml).ok()?;
    let chart = child(&doc.root_element(), "chart")?;
    parse_title(&chart)
}

/// The chart title text from `c:chart/c:title` (concatenated `a:t` runs), or `None` when
/// there is no title or it's explicitly deleted (`c:autoTitleDeleted val="1"` with no title).
fn parse_title(chart: &Node) -> Option<String> {
    let title = child(chart, "title")?;
    let text: String = title
        .descendants()
        .filter(|n| n.tag_name().name() == "t")
        .filter_map(|n| n.text())
        .collect();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Parses category + value axis titles. For scatter (two `c:valAx`, no `c:catAx`), the first
/// value axis (X) maps to `cat_axis` and the second (Y) to `val_axis` — the convention the
/// scatter renderer expects (phase_3 chrome: X title in `cat_axis`, Y title in `val_axis`).
fn parse_axes(plot_area: &Node, is_scatter: bool) -> (Axis, Axis) {
    if is_scatter {
        let val_axes: Vec<Node> = plot_area
            .children()
            .filter(|n| n.tag_name().name() == "valAx")
            .collect();
        let x = val_axes.first().map(axis_from_node).unwrap_or_default();
        let y = val_axes.get(1).map(axis_from_node).unwrap_or_default();
        return (x, y);
    }
    let cat = plot_area
        .children()
        .find(|n| n.tag_name().name() == "catAx")
        .map(|n| axis_from_node(&n))
        .unwrap_or_default();
    let val = plot_area
        .children()
        .find(|n| n.tag_name().name() == "valAx")
        .map(|n| axis_from_node(&n))
        .unwrap_or_default();
    (cat, val)
}

/// Build an [`Axis`] from a `c:catAx`/`c:valAx` node: its title, tick `c:numFmt` (P6), scaling
/// bounds/orientation (`c:scaling`, P13), and gridline toggles (`c:majorGridlines` /
/// `c:minorGridlines`, P13). An absent axis falls back to [`Axis::default`] (auto scale, major
/// gridlines on — Excel's value-axis default).
fn axis_from_node(ax: &Node) -> Axis {
    let (min, max, reversed) = axis_scaling(ax);
    Axis {
        title: axis_title(ax),
        number_format: axis_number_format(ax),
        min,
        max,
        reversed,
        major_gridlines: child(ax, "majorGridlines").is_some(),
        minor_gridlines: child(ax, "minorGridlines").is_some(),
    }
}

/// The axis tick `c:numFmt/@formatCode`, if it names a non-`General` format. The pervasive
/// `General`/empty default reads as "no explicit format" (the renderer's general formatting).
fn axis_number_format(ax: &Node) -> Option<String> {
    child(ax, "numFmt")
        .and_then(|n| attr(&n, "formatCode"))
        .map(str::trim)
        .filter(|code| !code.is_empty() && !code.eq_ignore_ascii_case("General"))
        .map(str::to_string)
}

/// The axis `c:scaling` — explicit `c:min`/`c:max` bounds (parsed as `f64`) and whether
/// `c:orientation` is `maxMin` (reversed). Absent scaling / bounds leave `None` / `false`.
fn axis_scaling(ax: &Node) -> (Option<f64>, Option<f64>, bool) {
    let Some(scaling) = child(ax, "scaling") else {
        return (None, None, false);
    };
    let bound = |name| child_val(&scaling, name).and_then(|v| v.trim().parse::<f64>().ok());
    let reversed = child_val(&scaling, "orientation").as_deref() == Some("maxMin");
    (bound("min"), bound("max"), reversed)
}

/// The title text of a `c:catAx`/`c:valAx` (concatenated `a:t` runs under its `c:title`).
fn axis_title(ax: &Node) -> Option<String> {
    let title = child(ax, "title")?;
    let text: String = title
        .descendants()
        .filter(|n| n.tag_name().name() == "t")
        .filter_map(|n| n.text())
        .collect();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A [`Legend`] iff the chart carries a `c:legend`, mapping `c:legendPos@val` to position.
fn parse_legend(chart: &Node) -> Option<Legend> {
    let legend = child(chart, "legend")?;
    let position = match child_val(&legend, "legendPos").as_deref() {
        Some("b") => LegendPosition::Bottom,
        Some("l") => LegendPosition::Left,
        Some("t") => LegendPosition::Top,
        Some("tr") => LegendPosition::TopRight,
        _ => LegendPosition::Right,
    };
    Some(Legend { position })
}

// ---------------------------------------------------------------------------------------------
// Small roxmltree helpers (local-name matching)
// ---------------------------------------------------------------------------------------------

/// The first child *element* with this local tag name.
fn child<'a>(node: &Node<'a, '_>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// The `val` attribute of the first child element with this local tag name.
fn child_val(node: &Node, name: &str) -> Option<String> {
    child(node, name).and_then(|n| attr(&n, "val").map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::authoring;
    use freecell_chart_model::{source_fidelity, Fidelity, SeriesData};

    const COLUMN_CHART: &str = r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
              xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
 <c:chart>
  <c:title><c:tx><c:rich><a:p><a:r><a:t>Quarterly Sales</a:t></a:r></a:p></c:rich></c:tx></c:title>
  <c:autoTitleDeleted val="0"/>
  <c:plotArea>
   <c:layout/>
   <c:barChart>
    <c:barDir val="col"/>
    <c:grouping val="clustered"/>
    <c:ser>
     <c:idx val="0"/><c:order val="0"/>
     <c:tx><c:strRef><c:f>Data!$B$1</c:f><c:strCache><c:ptCount val="1"/>
       <c:pt idx="0"><c:v>Widgets</c:v></c:pt></c:strCache></c:strRef></c:tx>
     <c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>
     <c:cat><c:strRef><c:f>Data!$A$2:$A$3</c:f><c:strCache><c:ptCount val="2"/>
       <c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>
     <c:val><c:numRef><c:f>Data!$B$2:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode>
       <c:ptCount val="2"/><c:pt idx="0"><c:v>120</c:v></c:pt>
       <c:pt idx="1"><c:v>150</c:v></c:pt></c:numCache></c:numRef></c:val>
    </c:ser>
    <c:axId val="1"/><c:axId val="2"/>
   </c:barChart>
   <c:catAx><c:axId val="1"/><c:title><c:tx><c:rich><a:p><a:r><a:t>Quarter</a:t></a:r></a:p></c:rich></c:tx></c:title></c:catAx>
   <c:valAx><c:axId val="2"/><c:title><c:tx><c:rich><a:p><a:r><a:t>Units</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx>
  </c:plotArea>
  <c:legend><c:legendPos val="r"/></c:legend>
 </c:chart>
</c:chartSpace>"#;

    #[test]
    fn parses_column_chart_kind_values_and_chrome() {
        let chart = parse_chart_xml(COLUMN_CHART).unwrap();
        assert_eq!(chart.title.as_deref(), Some("Quarterly Sales"));
        assert_eq!(
            chart.kind,
            ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered,
                // COLUMN_CHART omits c:gapWidth/c:overlap, so they take the OOXML defaults.
                layout: BarLayout::default(),
            }
        );
        assert_eq!(chart.cat_axis.title.as_deref(), Some("Quarter"));
        assert_eq!(chart.val_axis.title.as_deref(), Some("Units"));
        assert_eq!(
            chart.legend.map(|l| l.position),
            Some(LegendPosition::Right)
        );

        assert_eq!(chart.series.len(), 1);
        let s = &chart.series[0];
        assert_eq!(s.name.as_deref(), Some("Widgets"));
        assert_eq!(
            s.color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x4472C4)
            ))
        );
        match &s.data {
            SeriesData::CategoryValue { categories, values } => {
                assert_eq!(
                    categories,
                    &vec![Category::Text("Q1".into()), Category::Text("Q2".into())]
                );
                assert_eq!(values, &vec![120.0, 150.0]);
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
    }

    /// P22: a `c:barChart` parses its `c:gapWidth` / `c:overlap` into [`BarLayout`], and a series
    /// `a:schemeClr` fill parses to a theme [`ChartColor`] (not just `a:srgbClr`).
    #[test]
    fn parses_bar_gap_overlap_and_theme_fill() {
        let xml = r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
 <c:chart><c:plotArea>
  <c:barChart>
   <c:barDir val="bar"/>
   <c:grouping val="clustered"/>
   <c:ser>
    <c:idx val="0"/>
    <c:spPr><a:solidFill><a:schemeClr val="accent2"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></c:spPr>
    <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt></c:numCache></c:numRef></c:val>
   </c:ser>
   <c:gapWidth val="75"/>
   <c:overlap val="-20"/>
   <c:axId val="1"/><c:axId val="2"/>
  </c:barChart>
 </c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        assert_eq!(
            chart.kind,
            ChartKind::Bar {
                dir: BarDir::Bar,
                grouping: Grouping::Clustered,
                layout: freecell_chart_model::BarLayout::new(75, -20),
            }
        );
        // The series fill is a theme reference (accent2 + lumMod), not an sRGB.
        assert_eq!(
            chart.series[0].color,
            Some(ChartColor::Theme {
                slot: ThemeSlot::Accent2,
                lum_mod: Some(0.75),
                lum_off: None,
            })
        );
    }

    /// A `c:barChart` that omits `c:gapWidth` / `c:overlap` takes the OOXML defaults (150 / 0), and
    /// an out-of-range value is clamped to its `ST_GapAmount` / `ST_Overlap` bounds.
    #[test]
    fn bar_layout_defaults_and_clamps() {
        let no_layout = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
 <c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
   <c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
 </c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        match parse_chart_xml(no_layout).unwrap().kind {
            ChartKind::Bar { layout, .. } => {
                assert_eq!(layout, freecell_chart_model::BarLayout::default());
            }
            other => panic!("expected Bar, got {other:?}"),
        }

        let out_of_range = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
 <c:chart><c:plotArea><c:barChart><c:barDir val="col"/>
   <c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
   <c:gapWidth val="900"/><c:overlap val="250"/>
 </c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        match parse_chart_xml(out_of_range).unwrap().kind {
            ChartKind::Bar { layout, .. } => {
                assert_eq!(layout.gap_width, 500, "gapWidth clamps to 500");
                assert_eq!(layout.overlap, 100, "overlap clamps to 100");
            }
            other => panic!("expected Bar, got {other:?}"),
        }
    }

    #[test]
    fn scatter_maps_two_value_axes_and_xy_series() {
        let xml = r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
 <c:chart><c:plotArea><c:scatterChart><c:scatterStyle val="lineMarker"/>
   <c:ser><c:idx val="0"/>
     <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Points</c:v></c:pt></c:strCache></c:strRef></c:tx>
     <c:xVal><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:xVal>
     <c:yVal><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:yVal>
   </c:ser></c:scatterChart>
   <c:valAx><c:title><c:tx><c:rich><a:p><a:r><a:t>Ad spend</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx>
   <c:valAx><c:title><c:tx><c:rich><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx>
 </c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        assert_eq!(chart.kind, ChartKind::Scatter);
        // X-axis title in cat_axis, Y-axis title in val_axis (scatter renderer convention).
        assert_eq!(chart.cat_axis.title.as_deref(), Some("Ad spend"));
        assert_eq!(chart.val_axis.title.as_deref(), Some("Revenue"));
        match &chart.series[0].data {
            SeriesData::Xy { x, y } => {
                assert_eq!(x, &vec![1.0, 2.0]);
                assert_eq!(y, &vec![10.0, 20.0]);
            }
            other => panic!("expected Xy, got {other:?}"),
        }
    }

    #[test]
    fn doughnut_reads_hole_size_and_pie_has_none() {
        let pie = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:pieChart><c:ser/></c:pieChart></c:plotArea></c:chart></c:chartSpace>"#;
        assert_eq!(
            parse_chart_xml(pie).unwrap().kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        let dough = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:doughnutChart><c:holeSize val="55"/><c:ser/></c:doughnutChart></c:plotArea></c:chart></c:chartSpace>"#;
        match parse_chart_xml(dough).unwrap().kind {
            ChartKind::Pie {
                doughnut_hole: Some(h),
            } => assert!((h - 0.55).abs() < 1e-6),
            other => panic!("expected doughnut, got {other:?}"),
        }
    }

    #[test]
    fn missing_chart_group_is_an_error() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea/></c:chart></c:chartSpace>"#;
        assert!(parse_chart_xml(xml).is_err());
    }

    // --- P13: axis scaling / gridlines / numFmt + a:ln line stroke -------------------------

    #[test]
    fn parses_axis_scaling_gridlines_and_numfmt() {
        // A reversed category axis (no gridlines), and a value axis with explicit bounds, a
        // majorGridlines child, and a currency tick numFmt — the P13 axis breadth.
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea>
            <c:lineChart><c:ser/></c:lineChart>
            <c:catAx>
              <c:scaling><c:orientation val="maxMin"/></c:scaling>
            </c:catAx>
            <c:valAx>
              <c:scaling><c:orientation val="minMax"/><c:min val="0"/><c:max val="2000"/></c:scaling>
              <c:majorGridlines/>
              <c:numFmt formatCode="$#,##0" sourceLinked="0"/>
            </c:valAx>
          </c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        // Category axis: reversed, no gridlines parsed (absent → false).
        assert!(
            chart.cat_axis.reversed,
            "catAx orientation maxMin → reversed"
        );
        assert!(
            !chart.cat_axis.major_gridlines,
            "catAx has no majorGridlines"
        );
        // Value axis: explicit bounds, major gridlines on, currency numFmt, not reversed.
        assert_eq!(chart.val_axis.min, Some(0.0));
        assert_eq!(chart.val_axis.max, Some(2000.0));
        assert!(!chart.val_axis.reversed);
        assert!(chart.val_axis.major_gridlines, "valAx has majorGridlines");
        assert_eq!(chart.val_axis.number_format.as_deref(), Some("$#,##0"));
    }

    #[test]
    fn absent_scaling_leaves_axis_defaults() {
        // A value axis with no scaling/gridlines: auto bounds, not reversed; the absent
        // majorGridlines parses false (Excel: absence = off).
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea>
            <c:lineChart><c:ser/></c:lineChart>
            <c:catAx/>
            <c:valAx/>
          </c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        assert_eq!((chart.val_axis.min, chart.val_axis.max), (None, None));
        assert!(!chart.val_axis.reversed);
        assert!(!chart.val_axis.major_gridlines);
        assert!(!chart.val_axis.minor_gridlines);
        assert_eq!(chart.val_axis.number_format, None);
    }

    #[test]
    fn parses_series_line_stroke() {
        // Excel's default line series: `a:ln w="28440"` with its own solid fill carrying an alpha.
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
                                   xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser>
              <c:spPr>
                <a:solidFill><a:srgbClr val="4a7ebb"/></a:solidFill>
                <a:ln w="28440">
                  <a:solidFill><a:srgbClr val="4a7ebb"><a:alpha val="60000"/></a:srgbClr></a:solidFill>
                  <a:round/>
                </a:ln>
              </c:spPr>
            </c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        let stroke = chart.series[0].stroke.expect("a:ln parsed into a stroke");
        assert!(
            (stroke.width_pt.unwrap() - 2.24).abs() < 0.01,
            "w=28440 → 2.24pt"
        );
        assert_eq!(
            stroke.color,
            Some(ChartColor::Rgb(Color::from_hex(0x4A7EBB)))
        );
        assert!(
            (stroke.alpha.unwrap() - 0.6).abs() < 1e-4,
            "alpha 60000 → 0.6"
        );
    }

    #[test]
    fn scheme_colored_stroke_resolves_theme_reference() {
        // A themed line stroke (`a:schemeClr` + tint) parses to a ChartColor::Theme.
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
                                   xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser><c:spPr><a:ln w="19050">
              <a:solidFill><a:schemeClr val="accent2"><a:lumMod val="75000"/></a:schemeClr></a:solidFill>
            </a:ln></c:spPr></c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let stroke = parse_chart_xml(xml).unwrap().series[0].stroke.unwrap();
        assert_eq!(
            stroke.color,
            Some(ChartColor::Theme {
                slot: ThemeSlot::Accent2,
                lum_mod: Some(0.75),
                lum_off: None,
            })
        );
        assert!(
            (stroke.width_pt.unwrap() - 1.5).abs() < 0.01,
            "w=19050 → 1.5pt"
        );
    }

    #[test]
    fn plain_series_has_no_stroke() {
        // A series whose spPr line carries nothing we model (only a:round) → no LineStroke.
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
                                   xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser><c:spPr><a:ln><a:round/></a:ln></c:spPr></c:ser>
            <c:ser/>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        assert_eq!(
            chart.series[0].stroke, None,
            "a:ln with only a:round → None"
        );
        assert_eq!(chart.series[1].stroke, None, "no spPr → None");
    }

    // --- P12: data labels (c:dLbls) --------------------------------------------------------

    #[test]
    fn parses_series_level_data_labels_with_numfmt_separator_and_position() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser>
              <c:dLbls>
                <c:numFmt formatCode="$#,##0" sourceLinked="0"/>
                <c:dLblPos val="t"/>
                <c:showLegendKey val="1"/>
                <c:showVal val="1"/>
                <c:showCatName val="0"/>
                <c:showSerName val="1"/>
                <c:showPercent val="0"/>
                <c:separator> </c:separator>
              </c:dLbls>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1200</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        let dl = chart.series[0]
            .data_labels
            .as_ref()
            .expect("series carries its dLbls");
        assert!(dl.show_legend_key && dl.show_value && dl.show_series_name);
        assert!(!dl.show_category_name && !dl.show_percent);
        assert_eq!(dl.number_format.as_deref(), Some("$#,##0"));
        assert_eq!(dl.separator.as_deref(), Some(" "));
        assert_eq!(dl.position, Some(DataLabelPosition::Above));
        assert!(dl.is_shown());
    }

    #[test]
    fn chart_level_data_labels_default_applies_to_series_without_their_own() {
        // A chart-group-level `c:dLbls` (direct child of `c:lineChart`, after the series) is the
        // default: the first series (no dLbls of its own) inherits it; the second series' own
        // dLbls overrides it (even though it is all-off → an explicit "no labels").
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
            <c:ser>
              <c:dLbls><c:showVal val="0"/></c:dLbls>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
            <c:dLbls><c:showVal val="1"/></c:dLbls>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        // Series 0 inherits the chart-level default → value labels shown.
        let s0 = chart.series[0]
            .data_labels
            .as_ref()
            .expect("inherited default");
        assert!(s0.show_value && s0.is_shown());
        // Series 1 has its own (all-off) dLbls → overrides the default → not shown.
        let s1 = chart.series[1].data_labels.as_ref().expect("own dLbls");
        assert!(!s1.is_shown());
    }

    #[test]
    fn absent_data_labels_leave_series_none() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let chart = parse_chart_xml(xml).unwrap();
        assert!(chart.series[0].data_labels.is_none());
    }

    #[test]
    fn shown_data_labels_keep_a_line_chart_faithful() {
        // The exit-criterion reconciliation: a line chart with shown value labels + a supported
        // numFmt classifies Faithful (P12 renders both), not Degraded.
        let xml = r##"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser>
              <c:dLbls><c:numFmt formatCode="#,##0" sourceLinked="0"/><c:showVal val="1"/></c:dLbls>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1200</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"##;
        assert_eq!(source_fidelity(xml), Fidelity::Faithful);
    }

    // --- P7: discover_and_parse → ChartSpec envelope --------------------------------------

    /// The exit-criterion test: a **real** line-chart `.xlsx` (a real OPC zip with the full
    /// worksheet→drawing→chart chain) parses end-to-end into a `ChartSpec` — chart model,
    /// anchor, `c:f` source ranges, retained source XML + related parts, and Faithful fidelity.
    #[test]
    fn discover_and_parse_reads_line_fixture_end_to_end() {
        use authoring::{
            CATEGORIES, GADGETS, LINE_ANCHOR, LINE_CHART_PART, LINE_CHART_TITLE, WIDGETS,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_chart.xlsx");
        authoring::write_line_fixture(&path).unwrap();

        let specs = discover_and_parse(&path).unwrap();
        assert_eq!(specs.len(), 1, "one embedded line chart");
        let spec = &specs[0];

        // Chart model (the render seam): a straight two-series line with cached values.
        let chart = spec.chart().expect("line chart parsed into a typed Chart");
        assert!(matches!(chart.kind, ChartKind::Line { smooth: false, .. }));
        assert_eq!(chart.title.as_deref(), Some(LINE_CHART_TITLE));
        assert_eq!(chart.series.len(), 2);
        assert_eq!(chart.series[0].name.as_deref(), Some("Widgets"));
        assert_eq!(chart.series[1].name.as_deref(), Some("Gadgets"));
        match &chart.series[0].data {
            SeriesData::CategoryValue { categories, values } => {
                let cats: Vec<String> = categories.iter().map(Category::label).collect();
                assert_eq!(cats, CATEGORIES);
                assert_eq!(values, &WIDGETS.to_vec());
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        match &chart.series[1].data {
            SeriesData::CategoryValue { values, .. } => assert_eq!(values, &GADGETS.to_vec()),
            other => panic!("expected CategoryValue, got {other:?}"),
        }

        // Anchor: the twoCellAnchor from/to cells, including EMU offsets.
        assert_eq!(spec.anchor, LINE_ANCHOR);

        // source_ranges: every c:f (tx/cat/val per series) retained as-written, in doc order.
        let ranges: Vec<&str> = spec.source_ranges.iter().map(CfRange::as_str).collect();
        assert_eq!(
            ranges,
            vec![
                "Data!$B$1",
                "Data!$A$2:$A$5",
                "Data!$B$2:$B$5",
                "Data!$C$1",
                "Data!$A$2:$A$5",
                "Data!$C$2:$C$5",
            ]
        );

        // Retained source: chart XML byte-identical to the part, plus its related parts.
        assert!(spec.is_loaded());
        let source = spec.source().expect("loaded chart retains source");
        let part_xml = xlsx::read_entry(&path, LINE_CHART_PART).unwrap();
        assert_eq!(source.chart_xml, part_xml, "chart XML retained verbatim");
        assert!(source.chart_xml.contains("<c:lineChart"));

        let related: Vec<&str> = source
            .related_parts
            .iter()
            .map(|p| p.part_name.as_str())
            .collect();
        assert!(related.contains(&"xl/charts/_rels/chart1.xml.rels"));
        assert!(related.contains(&"xl/charts/colors1.xml"));
        assert!(related.contains(&"xl/charts/style1.xml"));
        let colors = source
            .related_parts
            .iter()
            .find(|p| p.part_name == "xl/charts/colors1.xml")
            .expect("colors part retained");
        assert!(std::str::from_utf8(&colors.bytes)
            .unwrap()
            .contains("colorStyle"));

        // Fidelity: a plain supported line renders faithfully (no badge).
        assert_eq!(spec.display_fidelity(), Fidelity::Faithful);
    }

    /// The walk visits every chart in a multi-chart workbook in document order and associates
    /// each with its *own* anchor (not just the first). Charts without a `_rels` retain no
    /// related parts.
    #[test]
    fn discover_and_parse_walks_multiple_charts_in_document_order() {
        use authoring::CHART_TITLES;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("charts_basic.xlsx");
        authoring::write_fixture(&path).unwrap();

        let specs = discover_and_parse(&path).unwrap();
        assert_eq!(specs.len(), 3);

        // Kinds in drawing order: column, line, pie.
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            ChartKind::Bar { .. }
        ));
        assert!(matches!(
            specs[1].chart().unwrap().kind,
            ChartKind::Line { .. }
        ));
        assert_eq!(
            specs[2].chart().unwrap().kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        assert_eq!(
            specs[1].chart().unwrap().title.as_deref(),
            Some(CHART_TITLES[1])
        );

        // The LINE chart carries the drawing's SECOND anchor — per-chart association, not the
        // first. The fixture's second frame is from(col=11,row=5) to(col=21,row=15).
        assert_eq!(specs[1].anchor.from, AnchorCell::new(11, 5));
        assert_eq!(specs[1].anchor.to, AnchorCell::new(21, 15));

        // Every spec is Loaded, retains its chart XML, and (no chart _rels) has no related parts.
        for spec in &specs {
            assert!(spec.is_loaded());
            let source = spec.source().unwrap();
            assert!(source.chart_xml.contains("<c:chartSpace"));
            assert!(source.related_parts.is_empty());
        }

        // The line chart's c:f refs are retained (tx/cat/val × 2 series = 6).
        assert_eq!(specs[1].source_ranges.len(), 6);
    }

    /// One unparseable chart (an unsupported `c:surfaceChart`) must NOT abort the load — and (P14)
    /// must be **RETAINED** as an Unsupported spec, not dropped: `discover_and_parse` succeeds and
    /// returns BOTH the Faithful line chart and the retained surface chart (source kept, no render
    /// picture, `display_fidelity() == Unsupported` → placeholder). (charts/architecture §6.)
    #[test]
    fn discover_and_parse_retains_unparseable_charts_as_unsupported_specs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_plus_unsupported.xlsx");
        authoring::write_line_plus_unsupported_fixture(&path).unwrap();

        // The load SUCCEEDS (no Err) and returns BOTH charts (the surface chart is no longer
        // dropped — it comes back as a retained Unsupported placeholder spec).
        let specs = discover_and_parse(&path).expect("load succeeds despite an unparseable chart");
        assert_eq!(
            specs.len(),
            2,
            "both charts retained (line + unsupported surface)"
        );

        // Chart 0: the parseable line chart, Faithful, with a render picture.
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            ChartKind::Line { .. }
        ));
        assert_eq!(specs[0].display_fidelity(), Fidelity::Faithful);

        // Chart 1: the surface chart, RETAINED as Unsupported — no render picture, but its source
        // XML + anchor are kept (so P8 placeholders it and P10 byte-preserves it on save).
        let surface = &specs[1];
        assert_eq!(surface.display_fidelity(), Fidelity::Unsupported);
        assert!(
            surface.chart().is_none(),
            "an unsupported chart carries no typed Chart"
        );
        assert!(
            surface.is_loaded(),
            "the surface chart still retains its source"
        );
        assert!(surface
            .source()
            .unwrap()
            .chart_xml
            .contains("<c:surfaceChart"));
        // Its title is salvaged for the placeholder caption.
        assert_eq!(surface.title(), Some("Terrain"));
    }

    /// P14: a **3-D** chart group normalizes to its 2-D `ChartKind` (so it parses, not drops) and
    /// classifies **Degraded** (its retained source still names the 3-D element).
    #[test]
    fn three_d_chart_group_parses_as_2d_and_is_degraded() {
        for (group, expect_bar) in [("bar3DChart", true), ("area3DChart", false)] {
            let xml = format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
                  <c:chart><c:plotArea>
                    <c:{group}><c:barDir val="col"/><c:grouping val="clustered"/>
                      <c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
                    </c:{group}>
                  </c:plotArea></c:chart></c:chartSpace>"#
            );
            let chart =
                parse_chart_xml(&xml).unwrap_or_else(|e| panic!("{group} must parse: {e:#}"));
            if expect_bar {
                assert!(matches!(chart.kind, ChartKind::Bar { .. }), "{group} → Bar");
            } else {
                assert!(
                    matches!(chart.kind, ChartKind::Area { .. }),
                    "{group} → Area"
                );
            }
            // The source still names the 3-D element → Degraded (renders 2-D + badge).
            assert_eq!(
                source_fidelity(&xml),
                Fidelity::Degraded,
                "{group} → Degraded"
            );
        }
    }

    /// P14 (per-chart-resilient walk): a `<c:chart r:id>` whose `rId` is absent from the drawing's
    /// `_rels` drops just THAT chart; its sibling (a valid rId) still comes through, and the load
    /// never errors.
    #[test]
    fn discover_skips_a_chart_with_a_dangling_relationship() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dangling_rel.xlsx");
        authoring::write_dangling_chart_rel_fixture(&path).unwrap();

        let specs = discover_and_parse(&path).expect("a dangling chart rel never fails the load");
        // Only the resolvable chart (rId1, the line) comes through; the dangling rId2 is skipped.
        assert_eq!(specs.len(), 1);
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            ChartKind::Line { .. }
        ));
    }

    /// P14 (per-drawing-resilient walk): a worksheet whose `<drawing>` has NO `_rels` part drops
    /// just that drawing; the OTHER sheet's chart still opens, and the load never errors.
    #[test]
    fn discover_skips_a_drawing_with_missing_rels_but_keeps_other_sheets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing_drawing_rels.xlsx");
        authoring::write_missing_drawing_rels_fixture(&path).unwrap();

        let specs =
            discover_and_parse(&path).expect("a missing drawing _rels never fails the load");
        // The healthy sheet's line chart survives; the broken drawing contributes nothing.
        assert_eq!(specs.len(), 1);
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            ChartKind::Line { .. }
        ));
    }

    /// P10: `discover_and_parse_by_sheet` associates each chart with the **name** of the worksheet
    /// it is anchored on (via the `workbook.xml.rels` part map), grouping the two-sheet fixture's
    /// column chart under "Data" and its line chart under "Summary".
    #[test]
    fn discover_and_parse_by_sheet_groups_charts_by_owning_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two_sheet.xlsx");
        authoring::write_two_sheet_fixture(&path).unwrap();

        let groups = discover_and_parse_by_sheet(&path).unwrap();
        assert_eq!(groups.len(), 2);
        let (data_name, data_specs) = &groups[0];
        let (summary_name, summary_specs) = &groups[1];
        assert_eq!(data_name, "Data");
        assert_eq!(summary_name, "Summary");
        assert_eq!(data_specs.len(), 1);
        assert_eq!(summary_specs.len(), 1);
        // Each spec is paired with its own chart part, in discovery order.
        assert_eq!(data_specs[0].0, "xl/charts/chart1.xml");
        assert_eq!(summary_specs[0].0, "xl/charts/chart2.xml");
        assert!(matches!(
            data_specs[0].1.chart().unwrap().kind,
            ChartKind::Bar { .. }
        ));
        assert!(matches!(
            summary_specs[0].1.chart().unwrap().kind,
            ChartKind::Line { .. }
        ));
    }

    /// P11 lazy discovery: `discover_and_parse_for_sheet` parses **only** the named worksheet's
    /// charts — the two-sheet fixture's column chart under "Data", its line chart under "Summary",
    /// and nothing for an unknown sheet name.
    #[test]
    fn discover_and_parse_for_sheet_parses_only_the_named_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two_sheet.xlsx");
        authoring::write_two_sheet_fixture(&path).unwrap();

        let data = discover_and_parse_for_sheet(&path, "Data").unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].0, "xl/charts/chart1.xml");
        assert!(matches!(
            data[0].1.chart().unwrap().kind,
            ChartKind::Bar { .. }
        ));

        let summary = discover_and_parse_for_sheet(&path, "Summary").unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].0, "xl/charts/chart2.xml");
        assert!(matches!(
            summary[0].1.chart().unwrap().kind,
            ChartKind::Line { .. }
        ));

        // An unknown sheet name (an in-session add, or a rename before first paint) → nothing.
        assert!(discover_and_parse_for_sheet(&path, "Nope")
            .unwrap()
            .is_empty());
    }

    /// P11 CR: the rename-safe part-keyed discovery + the stable name→part map the worker joins at
    /// open. `discover_and_parse_for_part` finds each sheet's charts by its STABLE worksheet part
    /// (never the mutable name), and `workbook_sheet_parts` exposes the name→part correspondence.
    #[test]
    fn discover_and_parse_for_part_keys_on_the_stable_worksheet_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two_sheet.xlsx");
        authoring::write_two_sheet_fixture(&path).unwrap();

        let parts = workbook_sheet_parts(&path).unwrap();
        let data_part = parts.iter().find(|(n, _)| n == "Data").unwrap().1.clone();
        let summary_part = parts
            .iter()
            .find(|(n, _)| n == "Summary")
            .unwrap()
            .1
            .clone();

        let data = discover_and_parse_for_part(&path, &data_part).unwrap();
        assert_eq!(data.len(), 1);
        assert!(matches!(
            data[0].1.chart().unwrap().kind,
            ChartKind::Bar { .. }
        ));

        let summary = discover_and_parse_for_part(&path, &summary_part).unwrap();
        assert_eq!(summary.len(), 1);
        assert!(matches!(
            summary[0].1.chart().unwrap().kind,
            ChartKind::Line { .. }
        ));

        // An unknown part (an in-session-added worksheet) → nothing.
        assert!(discover_and_parse_for_part(&path, "xl/worksheets/nope.xml")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parse_cf_ranges_collects_ref_formulas_in_document_order() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:plotArea><c:lineChart>
            <c:ser>
              <c:tx><c:strRef><c:f>Sheet1!$B$1</c:f></c:strRef></c:tx>
              <c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f></c:strRef></c:cat>
              <c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f></c:numRef></c:val>
            </c:ser>
          </c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let ranges = parse_cf_ranges(xml);
        let formulas: Vec<&str> = ranges.iter().map(CfRange::as_str).collect();
        assert_eq!(
            formulas,
            vec!["Sheet1!$B$1", "Sheet1!$A$2:$A$4", "Sheet1!$B$2:$B$4"]
        );

        // A bare <c:f> not under a *Ref is ignored; a whitespace-only ref is skipped.
        let stray = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:f>Bare!$A$1</c:f><c:numRef><c:f>  </c:f></c:numRef><c:strRef><c:f>Kept!$A$1</c:f></c:strRef></c:chartSpace>"#;
        let kept: Vec<String> = parse_cf_ranges(stray)
            .into_iter()
            .map(|r| r.formula)
            .collect();
        assert_eq!(kept, vec!["Kept!$A$1"]);

        // multiLvlStrRef (hierarchical categories) is a `*Ref` too → its c:f is collected.
        let multi = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:cat><c:multiLvlStrRef><c:f>Data!$A$2:$B$5</c:f></c:multiLvlStrRef></c:cat></c:chartSpace>"#;
        let hier: Vec<String> = parse_cf_ranges(multi)
            .into_iter()
            .map(|r| r.formula)
            .collect();
        assert_eq!(hier, vec!["Data!$A$2:$B$5"]);
    }

    #[test]
    fn anchor_parsing_reads_two_cell_anchor() {
        let two = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing">
          <xdr:twoCellAnchor>
            <xdr:from><xdr:col>2</xdr:col><xdr:colOff>19050</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>9525</xdr:rowOff></xdr:from>
            <xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>18</xdr:row><xdr:rowOff>4762</xdr:rowOff></xdr:to>
          </xdr:twoCellAnchor>
        </xdr:wsDr>"#;
        let doc = Document::parse(two).unwrap();
        let el = doc
            .descendants()
            .find(|n| n.tag_name().name() == "twoCellAnchor")
            .unwrap();
        let anchor = parse_anchor(&el);
        assert_eq!(anchor.from, AnchorCell::with_offsets(2, 19050, 3, 9525));
        assert_eq!(anchor.to, AnchorCell::with_offsets(8, 0, 18, 4762));

        // A oneCellAnchor (no <xdr:to>, carries <xdr:ext>) falls back to `to = from`.
        let one = r#"<xdr:wsDr xmlns:xdr="http://x">
          <xdr:oneCellAnchor>
            <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
            <xdr:ext cx="100" cy="200"/>
          </xdr:oneCellAnchor>
        </xdr:wsDr>"#;
        let doc = Document::parse(one).unwrap();
        let el = doc
            .descendants()
            .find(|n| n.tag_name().name() == "oneCellAnchor")
            .unwrap();
        let anchor = parse_anchor(&el);
        assert_eq!(anchor.from, AnchorCell::new(1, 1));
        assert_eq!(anchor.to, anchor.from);
    }
}
