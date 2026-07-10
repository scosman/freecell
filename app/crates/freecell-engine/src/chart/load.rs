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
    Anchor, AnchorCell, Axis, BarDir, Category, CfRange, Chart, ChartKind, ChartSpec, Grouping,
    Legend, LegendPosition, Series, SourcePart, SourceXml,
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
        let sheet_xml = xlsx::read_entry_from(&mut archive, &sheet_part)?;
        let Some((rel_id, drawing_part, rel_type)) =
            worksheet_drawing(&sheet_xml, &sheet_part, &mut archive)?
        else {
            continue;
        };
        let drawing_xml = xlsx::read_entry_from(&mut archive, &drawing_part)?;
        let charts = drawing_charts(&drawing_xml, &drawing_part, &mut archive)?;
        if charts.is_empty() {
            continue;
        }
        out.push(SheetDrawing {
            sheet_part,
            drawing_part,
            drawing_rel_id: rel_id,
            drawing_rel_type: rel_type,
            charts,
        });
    }
    // Deterministic order (zip index order is not guaranteed sorted).
    out.sort_by(|a, b| a.sheet_part.cmp(&b.sheet_part));
    Ok(out)
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

    let mut charts = Vec::new();
    for (rel_id, anchor) in referenced {
        let rel = rels
            .get(&rel_id)
            .ok_or_else(|| anyhow!("{rels_part} has no relationship {rel_id}"))?;
        charts.push(DiscoveredChart {
            part: xlsx::resolve_target(drawing_part, &rel.target),
            anchor,
        });
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
/// functional_spec §1): a chart whose part can't be read or parsed — an unsupported group our
/// `parse_chart_xml` doesn't recognize (surface / radar / stock / 3-D / bubble), or a malformed
/// part — is **skipped and logged**, and the walk continues, returning the charts that did
/// parse. Opening a workbook must never break on one broken chart.
///
/// NOTE (P8 / P14): this phase **drops** an unparseable chart entirely. The fuller handling —
/// retaining its source so it still byte-preserves on save, and rendering an actual placeholder
/// for it (charts/functional_spec §5 "Unsupported → placeholder") — is deferred to **P8**
/// (placeholder render) and **P14** (cross-type graceful-degrade / real-file corpus), per the
/// plan's risk ordering (line end-to-end first, cross-type robustness at P14). Skip-and-log is
/// sufficient for the line-only slice while honoring the never-breaks invariant.
///
/// This resilience covers a chart the walk *reaches* but can't parse. It does **not** yet cover
/// a corrupt **drawing relationship** in the shared [`discover`] walk — a missing drawing `_rels`
/// part, or a `<c:chart r:id>` whose `rId` is absent from it — which still `?`-aborts the whole
/// load (a package-corruption path, distinct from an unsupported chart). Making that dangling-rel
/// case per-chart-resilient is **P14** robustness work; tracked in `phase_plans/phase_7.md`.
pub fn discover_and_parse(path: &Path) -> Result<Vec<ChartSpec>> {
    let sheets = discover(path)?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;

    let mut specs = Vec::new();
    for sheet in &sheets {
        for dc in &sheet.charts {
            match parse_discovered_chart(&mut archive, dc) {
                Ok(spec) => specs.push(spec),
                // Skip + log; the walk continues. (P8/P14 upgrade skip → retain-source +
                // placeholder — see the fn docs.)
                Err(err) => {
                    tracing::warn!(chart_part = %dc.part, "skipping unparseable chart: {err:#}");
                }
            }
        }
    }
    Ok(specs)
}

/// Parses one discovered chart into a [`ChartSpec`] — read the part, map it to a [`Chart`], and
/// gather its `c:f` ranges + retained source (XML + related parts). Fallible so
/// [`discover_and_parse`] can treat a failure as per-chart non-fatal (skip + log).
fn parse_discovered_chart<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dc: &DiscoveredChart,
) -> Result<ChartSpec> {
    let chart_xml = xlsx::read_entry_from(archive, &dc.part)
        .with_context(|| format!("reading chart part {}", dc.part))?;
    let chart =
        parse_chart_xml(&chart_xml).with_context(|| format!("parsing chart part {}", dc.part))?;
    let source_ranges = parse_cf_ranges(&chart_xml);
    let related_parts = read_related_parts(archive, &dc.part)?;
    let source = SourceXml::new(chart_xml).with_related_parts(related_parts);
    Ok(ChartSpec::loaded(chart, source, source_ranges, dc.anchor))
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
const CHART_GROUP_TAGS: &[&str] = &[
    "barChart",
    "lineChart",
    "areaChart",
    "pieChart",
    "doughnutChart",
    "scatterChart",
];

/// Parses a single `xl/charts/chartN.xml` document (`c:chartSpace`) into a [`Chart`], reading
/// cached values only. Returns an error if no recognized chart-group element is present.
pub fn parse_chart_xml(xml: &str) -> Result<Chart> {
    let doc = Document::parse(xml).context("parsing chart XML")?;
    let root = doc.root_element(); // c:chartSpace
    let chart = child(&root, "chart").ok_or_else(|| anyhow!("no <c:chart> in chartSpace"))?;
    let plot_area = child(&chart, "plotArea").ok_or_else(|| anyhow!("no <c:plotArea> in chart"))?;

    let group = plot_area
        .children()
        .find(|n| n.is_element() && CHART_GROUP_TAGS.contains(&n.tag_name().name()))
        .ok_or_else(|| anyhow!("no recognized chart-group element in <c:plotArea>"))?;

    let kind = parse_kind(&group)?;
    let is_scatter = matches!(kind, ChartKind::Scatter);

    let series = group
        .children()
        .filter(|n| n.tag_name().name() == "ser")
        .map(|ser| parse_series(&ser, is_scatter))
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

/// The [`ChartKind`] for a chart-group element, reading `c:barDir` / `c:grouping` /
/// `c:holeSize` as needed.
fn parse_kind(group: &Node) -> Result<ChartKind> {
    Ok(match group.tag_name().name() {
        "barChart" => {
            let dir = match child_val(group, "barDir").as_deref() {
                Some("bar") => BarDir::Bar,
                _ => BarDir::Col, // default column
            };
            ChartKind::Bar {
                dir,
                grouping: grouping_of(group, Grouping::Clustered),
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
/// `c:cat`/`c:val`. Series name from `c:tx` cache; color from `c:spPr` solid fill.
fn parse_series(ser: &Node, is_scatter: bool) -> Result<Series> {
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
    Ok(series)
}

/// The series color from `c:ser/c:spPr/a:solidFill/a:srgbClr@val`, if present.
fn parse_series_color(ser: &Node) -> Option<freecell_chart_model::Color> {
    let sp_pr = child(ser, "spPr")?;
    let solid = child(&sp_pr, "solidFill")?;
    let srgb = child(&solid, "srgbClr")?;
    let hex = attr(&srgb, "val")?;
    let v = u32::from_str_radix(hex.trim(), 16).ok()?;
    Some(freecell_chart_model::Color::from_hex(v))
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
        let x = val_axes.first().and_then(axis_title);
        let y = val_axes.get(1).and_then(axis_title);
        return (axis(x), axis(y));
    }
    let cat = plot_area
        .children()
        .find(|n| n.tag_name().name() == "catAx")
        .and_then(|n| axis_title(&n));
    let val = plot_area
        .children()
        .find(|n| n.tag_name().name() == "valAx")
        .and_then(|n| axis_title(&n));
    (axis(cat), axis(val))
}

fn axis(title: Option<String>) -> Axis {
    match title {
        Some(t) => Axis::titled(t),
        None => Axis::untitled(),
    }
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
    use freecell_chart_model::{Fidelity, SeriesData};

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
                grouping: Grouping::Clustered
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
        assert!(matches!(
            spec.chart.kind,
            ChartKind::Line { smooth: false, .. }
        ));
        assert_eq!(spec.chart.title.as_deref(), Some(LINE_CHART_TITLE));
        assert_eq!(spec.chart.series.len(), 2);
        assert_eq!(spec.chart.series[0].name.as_deref(), Some("Widgets"));
        assert_eq!(spec.chart.series[1].name.as_deref(), Some("Gadgets"));
        match &spec.chart.series[0].data {
            SeriesData::CategoryValue { categories, values } => {
                let cats: Vec<String> = categories.iter().map(Category::label).collect();
                assert_eq!(cats, CATEGORIES);
                assert_eq!(values, &WIDGETS.to_vec());
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        match &spec.chart.series[1].data {
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
        assert!(matches!(specs[0].chart.kind, ChartKind::Bar { .. }));
        assert!(matches!(specs[1].chart.kind, ChartKind::Line { .. }));
        assert_eq!(
            specs[2].chart.kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        assert_eq!(specs[1].chart.title.as_deref(), Some(CHART_TITLES[1]));

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

    /// One unparseable chart (an unsupported `c:surfaceChart`) must NOT abort the load or drop
    /// the other charts: `discover_and_parse` succeeds, skips the bad chart, and returns the
    /// parseable line chart (charts/architecture §6, functional_spec §1).
    #[test]
    fn discover_and_parse_skips_unparseable_charts_without_failing_the_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_plus_unsupported.xlsx");
        authoring::write_line_plus_unsupported_fixture(&path).unwrap();

        // The load SUCCEEDS (no Err) despite the surface chart being unparseable.
        let specs = discover_and_parse(&path).expect("load succeeds despite an unparseable chart");

        // Only the parseable line chart comes through; the surface chart is skipped, not
        // returned (and did not crash the walk).
        assert_eq!(specs.len(), 1);
        assert!(matches!(specs[0].chart.kind, ChartKind::Line { .. }));
        assert_eq!(specs[0].display_fidelity(), Fidelity::Faithful);
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
