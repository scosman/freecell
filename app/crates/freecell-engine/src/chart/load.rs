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
    Axis, BarDir, Category, Chart, ChartKind, Grouping, Legend, LegendPosition, Series,
};

use super::xlsx::{self, attr};

/// The chart part names carried by one worksheet's single `<drawing>`, plus everything the
/// save re-injection needs to patch that worksheet. Discovered by [`discover`].
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
    /// The chart part names this drawing's graphic frames reference, in document order —
    /// e.g. `["xl/charts/chart1.xml", "xl/charts/chart2.xml"]`.
    pub chart_parts: Vec<String>,
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
        let chart_parts = drawing_chart_parts(&drawing_xml, &drawing_part, &mut archive)?;
        if chart_parts.is_empty() {
            continue;
        }
        out.push(SheetDrawing {
            sheet_part,
            drawing_part,
            drawing_rel_id: rel_id,
            drawing_rel_type: rel_type,
            chart_parts,
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

/// Collects the chart part names referenced by a drawing's `<c:chart r:id>` graphic frames,
/// in document order, resolving each `r:id` through the drawing's `_rels`.
fn drawing_chart_parts<R: std::io::Read + std::io::Seek>(
    drawing_xml: &str,
    drawing_part: &str,
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<String>> {
    let doc = Document::parse(drawing_xml).context("parsing drawing XML")?;
    // `<c:chart r:id="...">` frames — one per embedded chart.
    let chart_rel_ids: Vec<String> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "chart")
        .filter_map(|n| attr(&n, "id").map(str::to_string))
        .collect();
    if chart_rel_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rels_part = xlsx::rels_part_for(drawing_part);
    let rels_xml = xlsx::read_entry_from(archive, &rels_part).with_context(|| {
        format!("drawing {drawing_part} references charts but {rels_part} is missing")
    })?;
    let rels = xlsx::parse_rels(&rels_xml)?;

    let mut parts = Vec::new();
    for rel_id in chart_rel_ids {
        let rel = rels
            .get(&rel_id)
            .ok_or_else(|| anyhow!("{rels_part} has no relationship {rel_id}"))?;
        parts.push(xlsx::resolve_target(drawing_part, &rel.target));
    }
    Ok(parts)
}

/// Loads every embedded chart in the workbook into [`freecell_chart_model::Chart`], in
/// (worksheet, document) order. The seam this whole experiment exists to prove: file → model.
pub fn load_charts_from_xlsx(path: &Path) -> Result<Vec<Chart>> {
    let sheets = discover(path)?;
    let mut charts = Vec::new();
    for sheet in &sheets {
        for chart_part in &sheet.chart_parts {
            let xml = xlsx::read_entry(path, chart_part)
                .with_context(|| format!("reading chart part {chart_part}"))?;
            let chart = parse_chart_xml(&xml)
                .with_context(|| format!("parsing chart part {chart_part}"))?;
            charts.push(chart);
        }
    }
    Ok(charts)
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
    use freecell_chart_model::SeriesData;

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
            Some(freecell_chart_model::Color::from_hex(0x4472C4))
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
}
