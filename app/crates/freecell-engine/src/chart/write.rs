//! **Write-from-model** — the third chart save write-mode (charts/components/write-path §2–§3,
//! implementation_plan P16). Where [`load`](super::load) parses chart XML *into* the model and
//! [`save`](super::save) *byte-preserves / targeted-patches* a **loaded** chart's retained source,
//! this module **synthesizes** chart XML (+ its drawing / rels / content-types) *from* an
//! [`Authored`](freecell_chart_model::Origin::Authored) `chart-model` value — the path an in-app
//! authored chart takes on save (it has no retained source to preserve).
//!
//! Three layers:
//! - [`serialize_chart_xml`] — the serializer core: a [`Chart`] → a `c:chartSpace` document that
//!   **round-trips through [`parse_chart_xml`](super::load::parse_chart_xml)** and opens in Excel +
//!   LibreOffice. It is the inverse of the loader's parse.
//! - [`synthesize_drawing_xml`] — an [`Anchor`] (+ chart rel-id) → the `xdr:wsDr` graphic frame that
//!   places the chart on the sheet.
//! - [`write_authored_charts`] — assembles the two into IronCalc's chart-less model bytes: chart
//!   parts + one drawing per host sheet + their rels + `[Content_Types]` overrides + a worksheet
//!   `<drawing>` ref, **failing loudly** on an unknown sheet or one that already carries a drawing
//!   (charts/architecture §6, components/write-path §3).
//!
//! Value caches (`numCache`/`strCache`) are built by the **same** helpers the edited-loaded reflow
//! patcher uses ([`save::rebuild_num_cache`]/[`save::rebuild_str_cache`]), so an authored cache is
//! byte-identical to a reflowed one — the reconciliation invariant (components/write-path §4).

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use anyhow::{anyhow, Context, Result};

use freecell_chart_model::{
    Anchor, Axis, BarDir, Category, Chart, ChartColor, ChartKind, Grouping, LegendPosition, Series,
    SeriesData, ThemePalette,
};

use super::save;
use super::xlsx;

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NS_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const CT_DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";

/// The category / x axis id, cross-referenced by the value / y axis (and vice versa). Fixed values —
/// they only need to be unique + self-consistent within one chart part.
const CAT_AX: &str = "111111111";
const VAL_AX: &str = "222222222";

/// The `c:f` reference formulas for one authored series' data roles (components/write-path §2.3).
/// `categories` is the domain (`c:cat` for a category/value series, `c:xVal` for scatter);
/// `values` is `c:val` / `c:yVal`. A `None` role whose data is non-empty is serialized as a
/// **literal** (`c:numLit`/`c:strLit`/`c:v`) so the XML is always schema-valid even before a range
/// is picked — a literal is not read back by FreeCell's own loader (which reads `numCache`/
/// `strCache`), which is fine: a literal-data chart has no ranges to live-bind. Once the authoring
/// flow sets a range (P19) each role has a real `c:f` and the chart is fully cell-bound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeriesRefs {
    /// `c:tx` series-name reference.
    pub name: Option<String>,
    /// `c:cat` (category/value) or `c:xVal` (scatter/bubble) domain reference.
    pub categories: Option<String>,
    /// `c:val` (category/value) or `c:yVal` (scatter/bubble) value reference.
    pub values: Option<String>,
    /// `c:bubbleSize` reference (bubble only, P26) — the third range. `None` for every other type.
    pub sizes: Option<String>,
}

/// One authored chart to write into a workbook (components/write-path §3): the render [`Chart`] to
/// serialize, its `twoCellAnchor` [`Anchor`], the host worksheet's **name** (resolved to its part
/// through the model's `workbook.xml.rels`), the caller-assigned chart part
/// (`xl/charts/chartN.xml`, unique + non-colliding), and one [`SeriesRefs`] per series.
#[derive(Clone, Debug)]
pub struct AuthoredChart {
    /// Host worksheet name (must exist in the model; must not already carry a `<drawing>`).
    pub sheet_name: String,
    /// The chart's package part, e.g. `xl/charts/chart1.xml` (caller-assigned, unique).
    pub chart_part: String,
    /// The render model to serialize.
    pub chart: Chart,
    /// The chart's in-grid placement.
    pub anchor: Anchor,
    /// One entry per `chart.series`, in order — the `c:f` refs for each series' roles.
    pub refs: Vec<SeriesRefs>,
}

// ---------------------------------------------------------------------------------------------
// Serializer core: Chart -> chartN.xml
// ---------------------------------------------------------------------------------------------

/// Serializes a [`Chart`] into an `xl/charts/chartN.xml` (`c:chartSpace`) document
/// (components/write-path §2). The output **round-trips** through
/// [`parse_chart_xml`](super::load::parse_chart_xml): `parse_chart_xml(serialize_chart_xml(c, r))`
/// reconstructs `c` for every field the model carries (title, kind, per-series name/data/color, the
/// two axes' title/gridlines/scaling/number-format, legend). `refs[i]` supplies the `c:f` formulas
/// for `chart.series[i]`; a shorter `refs` (or a `None` role) falls back to a literal (§2.3).
///
/// **Two round-trip caveats, both inherited from the loader (not introduced here):**
/// - A **non-finite** value (`NaN`, a blanked point) is omitted as a sparse gap while `ptCount` keeps
///   the full length — the same shape the edited-loaded reflow patcher writes ([`save::rebuild_num_cache`]),
///   so an authored + reflowed chart agree. The loader then reads the surviving points into a *dense*
///   vector, so a series carrying blanks does not round-trip position-for-position. Authored models
///   built from real cells carry finite values, so this is not hit in practice.
/// - Leading/trailing **whitespace** in a title / axis title is emitted verbatim but the loader
///   *trims* it ([`parse_title`](super::load)), so `" Sales "` reads back `"Sales"`.
pub fn serialize_chart_xml(chart: &Chart, refs: &[SeriesRefs]) -> String {
    let series_xml: String = chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| series_element(i, s, refs.get(i)))
        .collect();
    let group = group_element(chart, &series_xml);
    let axes = axes_xml(chart);

    let (title, auto_title_deleted) = match &chart.title {
        Some(t) => (title_block(t), r#"<c:autoTitleDeleted val="0"/>"#),
        None => (String::new(), r#"<c:autoTitleDeleted val="1"/>"#),
    };
    let legend = chart
        .legend
        .map(|l| {
            format!(
                r#"<c:legend><c:legendPos val="{}"/><c:overlay val="0"/></c:legend>"#,
                legend_pos(l.position)
            )
        })
        .unwrap_or_default();

    format!(
        r#"{DECL}
<c:chartSpace xmlns:c="{NS_CHART}" xmlns:a="{NS_A}" xmlns:r="{NS_REL}"><c:chart>{title}{auto_title_deleted}<c:plotArea><c:layout/>{group}{axes}</c:plotArea>{legend}<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart></c:chartSpace>"#
    )
}

/// The `c:<type>Chart` group element for `chart.kind`, wrapping the already-serialized series. Child
/// order follows the `CT_*Chart` schema sequence so strict readers (Excel) accept it.
fn group_element(chart: &Chart, series_xml: &str) -> String {
    match &chart.kind {
        ChartKind::Bar {
            dir,
            grouping,
            layout,
        } => format!(
            // `c:gapWidth` / `c:overlap` sit after the series and before the `c:axId` pair — the
            // `CT_BarChart` child order — so the output round-trips through `parse_chart_xml` (P22).
            r#"<c:barChart><c:barDir val="{}"/><c:grouping val="{}"/><c:varyColors val="0"/>{series_xml}<c:gapWidth val="{}"/><c:overlap val="{}"/><c:axId val="{CAT_AX}"/><c:axId val="{VAL_AX}"/></c:barChart>"#,
            bar_dir(*dir),
            grouping_val(*grouping),
            layout.gap_width,
            layout.overlap,
        ),
        ChartKind::Line { grouping, smooth } => {
            let smooth_el = if *smooth {
                r#"<c:smooth val="1"/>"#
            } else {
                ""
            };
            format!(
                r#"<c:lineChart><c:grouping val="{}"/><c:varyColors val="0"/>{series_xml}<c:marker val="1"/>{smooth_el}<c:axId val="{CAT_AX}"/><c:axId val="{VAL_AX}"/></c:lineChart>"#,
                grouping_val(*grouping),
            )
        }
        ChartKind::Area { grouping } => format!(
            r#"<c:areaChart><c:grouping val="{}"/><c:varyColors val="0"/>{series_xml}<c:axId val="{CAT_AX}"/><c:axId val="{VAL_AX}"/></c:areaChart>"#,
            grouping_val(*grouping),
        ),
        ChartKind::Pie {
            doughnut_hole: None,
            first_slice_ang,
            vary_colors,
        } => format!(
            // CT_PieChart child order: varyColors?, ser*, dLbls?, firstSliceAng?.
            r#"<c:pieChart><c:varyColors val="{}"/>{series_xml}<c:firstSliceAng val="{first_slice_ang}"/></c:pieChart>"#,
            bool_val(*vary_colors),
        ),
        ChartKind::Pie {
            doughnut_hole: Some(hole),
            first_slice_ang,
            vary_colors,
        } => format!(
            // CT_DoughnutChart child order: varyColors?, ser*, dLbls?, firstSliceAng?, holeSize?.
            r#"<c:doughnutChart><c:varyColors val="{}"/>{series_xml}<c:firstSliceAng val="{first_slice_ang}"/><c:holeSize val="{}"/></c:doughnutChart>"#,
            bool_val(*vary_colors),
            (hole * 100.0).round() as i64,
        ),
        ChartKind::Scatter { style } => format!(
            // CT_ScatterChart child order: scatterStyle, varyColors?, ser*, axId, axId.
            r#"<c:scatterChart><c:scatterStyle val="{}"/>{series_xml}<c:axId val="{CAT_AX}"/><c:axId val="{VAL_AX}"/></c:scatterChart>"#,
            style.as_ooxml(),
        ),
        ChartKind::Bubble {
            size_representation,
        } => format!(
            // CT_BubbleChart child order: varyColors?, ser*, dLbls?, bubble3D?, bubbleScale?,
            // showNegBubbles?, sizeRepresents?, axId, axId. We emit varyColors, the series, then
            // sizeRepresents (from the model), then the axId pair — a valid subsequence.
            r#"<c:bubbleChart><c:varyColors val="0"/>{series_xml}<c:sizeRepresents val="{}"/><c:axId val="{CAT_AX}"/><c:axId val="{VAL_AX}"/></c:bubbleChart>"#,
            size_representation.as_ooxml(),
        ),
    }
}

/// One `<c:ser>` element (`CT_*Ser` child order: idx, order, tx?, spPr?, then the data roles).
fn series_element(idx: usize, series: &Series, refs: Option<&SeriesRefs>) -> String {
    let name_f = refs.and_then(|r| r.name.as_deref());
    let cat_f = refs.and_then(|r| r.categories.as_deref());
    let val_f = refs.and_then(|r| r.values.as_deref());
    let size_f = refs.and_then(|r| r.sizes.as_deref());

    let tx = series
        .name
        .as_deref()
        .map(|n| tx_element(n, name_f))
        .unwrap_or_default();
    let sp = sppr_element(series.color.as_ref());
    // `c:dPt` per-slice overrides (pie/doughnut, P24) sit after `spPr` and before `dLbls` — the
    // `CT_*Ser` slot valid across every type (empty for non-pie, so only a pie ever emits them).
    let dpts: String = series.data_points.iter().map(dpt_element).collect();
    // Data labels (`c:dLbls`, P20) sit after `spPr`/`dPt` and before the data roles — schema-valid in
    // every `CT_*Ser`. Shared with the loaded chrome patch via `chrome::dlbls_element`.
    let dlbls = series
        .data_labels
        .as_ref()
        .map(|l| super::chrome::dlbls_element("c:", l))
        .unwrap_or_default();
    let data = match &series.data {
        SeriesData::CategoryValue { categories, values } => {
            format!(
                "{}{}",
                cat_role(cat_f, categories),
                num_role("val", val_f, values)
            )
        }
        SeriesData::Xy { x, y, size } => {
            // CT_ScatterSer/CT_BubbleSer order: xVal, yVal, then (bubble only) bubbleSize.
            let mut data = format!(
                "{}{}",
                num_role("xVal", cat_f, x),
                num_role("yVal", val_f, y)
            );
            if let Some(size) = size {
                data.push_str(&num_role("bubbleSize", size_f, size));
            }
            data
        }
    };
    format!(
        r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/>{tx}{sp}{dpts}{dlbls}{data}</c:ser>"#
    )
}

/// One `<c:dPt>` per-slice override (P24) in `CT_DPt` child order: idx, then `c:explosion`?, then
/// `c:spPr`? (a solid fill). A theme dPt color resolves to its office-default sRGB — authored charts
/// use concrete sRGB, matching [`sppr_element`].
fn dpt_element(dp: &freecell_chart_model::DataPoint) -> String {
    let explosion = dp
        .explosion
        .map(|e| format!(r#"<c:explosion val="{e}"/>"#))
        .unwrap_or_default();
    let sp = dp
        .color
        .as_ref()
        .map(|c| {
            format!(
                r#"<c:spPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill></c:spPr>"#,
                srgb_hex(c)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<c:dPt><c:idx val="{}"/>{explosion}{sp}</c:dPt>"#,
        dp.index
    )
}

/// A `<c:tx>` series-name element — a `strRef` (with cache) when a name ref is given, else a plain
/// `<c:v>` string literal.
fn tx_element(name: &str, f: Option<&str>) -> String {
    match f {
        Some(f) => format!(
            r#"<c:tx><c:strRef><c:f>{}</c:f>{}</c:strRef></c:tx>"#,
            save::escape_xml(f),
            save::rebuild_str_cache("c:", &[name.to_string()]),
        ),
        None => format!(r#"<c:tx><c:v>{}</c:v></c:tx>"#, save::escape_xml(name)),
    }
}

/// A `<c:spPr>` solid-fill element for a series color, or empty when the series has no color. An
/// authored color is a concrete sRGB (functional_spec §6.A); a theme reference resolves to its
/// office-default RGB so the fill is always a round-trippable `a:srgbClr`.
fn sppr_element(color: Option<&ChartColor>) -> String {
    match color {
        Some(c) => format!(
            r#"<c:spPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill></c:spPr>"#,
            srgb_hex(c)
        ),
        None => String::new(),
    }
}

/// The `RRGGBB` hex for a [`ChartColor`] — the color itself for an sRGB value, or the office-default
/// palette color for a theme slot (authored charts use concrete sRGB, so the theme path is a
/// best-effort resolution rather than a fidelity target).
fn srgb_hex(c: &ChartColor) -> String {
    let color = match c {
        ChartColor::Rgb(col) => *col,
        ChartColor::Theme { slot, .. } => ThemePalette::office_default().color(*slot),
    };
    format!("{:06X}", color.to_hex())
}

/// A numeric data role (`c:val` / `c:xVal` / `c:yVal`) — a `numRef` (with the shared value cache)
/// when a ref is given, else a `numLit` (§2.3). The cache is built by [`save::rebuild_num_cache`]
/// so it matches a reflow byte-for-byte.
fn num_role(tag: &str, f: Option<&str>, values: &[f64]) -> String {
    let inner = match f {
        Some(f) => format!(
            r#"<c:numRef><c:f>{}</c:f>{}</c:numRef>"#,
            save::escape_xml(f),
            save::rebuild_num_cache("c:", Some("General"), values),
        ),
        None => format!("<c:numLit>{}</c:numLit>", num_lit_body(values)),
    };
    format!("<c:{tag}>{inner}</c:{tag}>")
}

/// A `<c:cat>` role: all-numeric categories → a numeric ref/lit; any text category → a string
/// ref/lit (numeric labels stringified), mirroring the loader's str-then-num preference.
fn cat_role(f: Option<&str>, categories: &[Category]) -> String {
    let all_numeric =
        !categories.is_empty() && categories.iter().all(|c| matches!(c, Category::Number(_)));
    let inner = if all_numeric {
        let nums: Vec<f64> = categories
            .iter()
            .map(|c| match c {
                Category::Number(n) => *n,
                Category::Text(_) => f64::NAN,
            })
            .collect();
        match f {
            Some(f) => format!(
                r#"<c:numRef><c:f>{}</c:f>{}</c:numRef>"#,
                save::escape_xml(f),
                save::rebuild_num_cache("c:", Some("General"), &nums),
            ),
            None => format!("<c:numLit>{}</c:numLit>", num_lit_body(&nums)),
        }
    } else {
        let labels: Vec<String> = categories.iter().map(Category::label).collect();
        match f {
            Some(f) => format!(
                r#"<c:strRef><c:f>{}</c:f>{}</c:strRef>"#,
                save::escape_xml(f),
                save::rebuild_str_cache("c:", &labels),
            ),
            None => format!("<c:strLit>{}</c:strLit>", str_lit_body(&labels)),
        }
    };
    format!("<c:cat>{inner}</c:cat>")
}

/// The body of a `c:numLit` (`ptCount` + finite points) — the literal twin of a `numCache` without
/// the `c:formatCode`, using the same value formatting as the shared cache builder.
fn num_lit_body(values: &[f64]) -> String {
    let mut s = format!(r#"<c:ptCount val="{}"/>"#, values.len());
    for (idx, v) in values.iter().enumerate() {
        if v.is_finite() {
            s.push_str(&format!(
                r#"<c:pt idx="{idx}"><c:v>{}</c:v></c:pt>"#,
                save::fmt_cache_num(*v)
            ));
        }
    }
    s
}

/// The body of a `c:strLit` (`ptCount` + points).
fn str_lit_body(values: &[String]) -> String {
    let mut s = format!(r#"<c:ptCount val="{}"/>"#, values.len());
    for (idx, v) in values.iter().enumerate() {
        s.push_str(&format!(
            r#"<c:pt idx="{idx}"><c:v>{}</c:v></c:pt>"#,
            save::escape_xml(v)
        ));
    }
    s
}

/// The `c:catAx` + `c:valAx` (or, for scatter, two `c:valAx`) block, or empty for pie/doughnut
/// (which have no axes — their model axes round-trip as `Axis::default()`, what the loader returns
/// for an absent axis).
fn axes_xml(chart: &Chart) -> String {
    match &chart.kind {
        ChartKind::Pie { .. } => String::new(),
        // Scatter AND bubble carry two value axes (both numeric).
        ChartKind::Scatter { .. } | ChartKind::Bubble { .. } => format!(
            "{}{}",
            axis_element("valAx", &chart.cat_axis, CAT_AX, VAL_AX, "b"),
            axis_element("valAx", &chart.val_axis, VAL_AX, CAT_AX, "l"),
        ),
        _ => format!(
            "{}{}",
            axis_element("catAx", &chart.cat_axis, CAT_AX, VAL_AX, "b"),
            axis_element("valAx", &chart.val_axis, VAL_AX, CAT_AX, "l"),
        ),
    }
}

/// One axis element (`c:catAx` / `c:valAx`), in `CT_*Ax` child order: axId, scaling, delete, axPos,
/// majorGridlines?, minorGridlines?, title?, numFmt?, crossAx.
fn axis_element(tag: &str, axis: &Axis, ax_id: &str, cross_ax: &str, ax_pos: &str) -> String {
    let orientation = if axis.reversed { "maxMin" } else { "minMax" };
    let max = axis
        .max
        .map(|m| format!(r#"<c:max val="{}"/>"#, save::fmt_cache_num(m)))
        .unwrap_or_default();
    let min = axis
        .min
        .map(|m| format!(r#"<c:min val="{}"/>"#, save::fmt_cache_num(m)))
        .unwrap_or_default();
    let major = if axis.major_gridlines {
        "<c:majorGridlines/>"
    } else {
        ""
    };
    let minor = if axis.minor_gridlines {
        "<c:minorGridlines/>"
    } else {
        ""
    };
    let title = axis.title.as_deref().map(title_block).unwrap_or_default();
    let numfmt = axis
        .number_format
        .as_deref()
        .map(|code| {
            format!(
                r#"<c:numFmt formatCode="{}" sourceLinked="0"/>"#,
                attr_escape(code)
            )
        })
        .unwrap_or_default();

    format!(
        r#"<c:{tag}><c:axId val="{ax_id}"/><c:scaling><c:orientation val="{orientation}"/>{max}{min}</c:scaling><c:delete val="0"/><c:axPos val="{ax_pos}"/>{major}{minor}{title}{numfmt}<c:crossAx val="{cross_ax}"/></c:{tag}>"#
    )
}

/// A `c:title` block with a single rich-text run — the shape [`parse_title`](super::load) reads.
fn title_block(text: &str) -> String {
    format!(
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>"#,
        save::escape_xml(text),
    )
}

fn bar_dir(d: BarDir) -> &'static str {
    match d {
        BarDir::Col => "col",
        BarDir::Bar => "bar",
    }
}

/// An OOXML boolean `val` string (`"1"`/`"0"`) — the shape `c:varyColors`/toggles emit.
fn bool_val(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

fn grouping_val(g: Grouping) -> &'static str {
    match g {
        Grouping::Standard => "standard",
        Grouping::Clustered => "clustered",
        Grouping::Stacked => "stacked",
        Grouping::PercentStacked => "percentStacked",
    }
}

fn legend_pos(p: LegendPosition) -> &'static str {
    match p {
        LegendPosition::Right => "r",
        LegendPosition::Bottom => "b",
        LegendPosition::Left => "l",
        LegendPosition::Top => "t",
        LegendPosition::TopRight => "tr",
    }
}

/// XML attribute-value escaping (adds `"` on top of the element-text set).
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------------------------
// Drawing synthesis: Anchor -> drawingN.xml
// ---------------------------------------------------------------------------------------------

/// Synthesizes a worksheet drawing part (`xdr:wsDr`) with one `xdr:twoCellAnchor` graphic frame per
/// `(anchor, chart_rel_id)` — the placement layer (components/write-path §3). Each frame's
/// `<c:chart r:id=…>` points at the chart part through the drawing's `_rels`; the anchor's
/// `<xdr:from>`/`<xdr:to>` carry the cell corners + EMU offsets, so the load path's
/// [`parse_anchor`](super::load) reconstructs the [`Anchor`].
pub fn synthesize_drawing_xml(anchors: &[(Anchor, &str)]) -> String {
    let frames: String = anchors
        .iter()
        .enumerate()
        .map(|(i, (anchor, rel_id))| {
            let f = anchor.from;
            let t = anchor.to;
            let shape_id = i + 2; // Excel numbers drawing shapes from 2
            format!(
                r#"<xdr:twoCellAnchor><xdr:from><xdr:col>{fc}</xdr:col><xdr:colOff>{fco}</xdr:colOff><xdr:row>{fr}</xdr:row><xdr:rowOff>{fro}</xdr:rowOff></xdr:from><xdr:to><xdr:col>{tc}</xdr:col><xdr:colOff>{tco}</xdr:colOff><xdr:row>{tr}</xdr:row><xdr:rowOff>{tro}</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{shape_id}" name="Chart {shape_id}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="{NS_CHART}"><c:chart xmlns:c="{NS_CHART}" xmlns:r="{NS_REL}" r:id="{rel_id}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"#,
                fc = f.col,
                fco = f.col_off_emu,
                fr = f.row,
                fro = f.row_off_emu,
                tc = t.col,
                tco = t.col_off_emu,
                tr = t.row,
                tro = t.row_off_emu,
            )
        })
        .collect();
    format!(r#"{DECL}<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">{frames}</xdr:wsDr>"#)
}

// ---------------------------------------------------------------------------------------------
// Package assembly: inject synthesized charts into IronCalc's model bytes
// ---------------------------------------------------------------------------------------------

/// The outcome of a [`write_authored_charts`] call, reported with **authored-path provenance**.
///
/// Deliberately a **distinct** shape from [`SaveReport`](super::save::SaveReport) (which reports the
/// *loaded* byte-preserve / edit-patch paths): its fields say "authored" and "synthesized", not
/// "preserved" and "carried", so a combined save that runs both
/// [`reinject_live_charts`](super::save::reinject_live_charts) and [`write_authored_charts`] over one
/// workbook (P17) never conflates written-from-scratch charts with charts carried through from a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthoredWriteReport {
    /// Number of charts **serialized from the model** — written from scratch, not preserved.
    pub charts_authored: usize,
    /// Worksheet parts that got a `<drawing>` reference injected.
    pub patched_sheets: Vec<String>,
    /// The package parts **synthesized** into the output, in write order: each host sheet's drawing
    /// part + its `_rels`, then that drawing's chart parts.
    pub synthesized_parts: Vec<String>,
}

/// One host worksheet's synthesized drawing + charts, ready to write into the output zip.
struct DrawingPlan {
    sheet_part: String,
    sheet_rels_part: String,
    /// The worksheet → drawing relationship id (`<drawing r:id>`), non-colliding with IronCalc's.
    ws_rel_id: String,
    drawing_part: String,
    drawing_rels_part: String,
    drawing_xml: String,
    drawing_rels_xml: String,
    /// `(chart part, chart XML)` pairs, in anchor order within this drawing.
    charts: Vec<(String, String)>,
}

/// Writes the [`AuthoredChart`]s into `model_bytes` (IronCalc's chart-less serialization of the
/// current model) and returns the final `.xlsx` bytes (components/write-path §3). Authored charts
/// are **grouped by host worksheet** (a worksheet has at most one `<drawing>`), so each sheet gets
/// one synthesized drawing carrying all its authored anchors; each chart part, drawing part, drawing
/// `_rels`, and content-type override is injected, and each target worksheet is patched with its
/// `<drawing>` ref.
///
/// **Fail-loud preconditions** (charts/architecture §6 — never silently corrupt): an
/// [`AuthoredChart::sheet_name`] with no worksheet in the model, a target worksheet that **already**
/// carries a `<drawing>` (merging authored charts onto a sheet that already has charts is not yet
/// supported — P17), or a [`chart_part`](AuthoredChart::chart_part) that collides with an existing
/// part or another authored chart, is a hard error.
///
/// This entry point operates on already-serialized `model_bytes` and only **adds** parts, so it
/// **composes** with the loaded-chart save ([`reinject_live_charts`](super::save::reinject_live_charts)):
/// a workbook with both loaded and authored charts runs the loaded re-inject first, then this — the
/// orchestration that does so is the app's insert-flow concern (P17).
pub fn write_authored_charts(
    model_bytes: &[u8],
    authored: &[AuthoredChart],
) -> Result<(Vec<u8>, AuthoredWriteReport)> {
    if authored.is_empty() {
        return Ok((model_bytes.to_vec(), AuthoredWriteReport::default()));
    }

    let name_to_part = save::name_to_part_map(model_bytes)?;

    let mut ic = zip::ZipArchive::new(Cursor::new(model_bytes)).context("reading model zip")?;
    let ic_names: Vec<String> = (0..ic.len())
        .filter_map(|i| ic.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();
    let existing: HashSet<&str> = ic_names.iter().map(String::as_str).collect();

    // Group authored charts by host worksheet part, in first-seen order; validate chart parts.
    let mut order: Vec<String> = Vec::new();
    let mut by_sheet: HashMap<String, Vec<&AuthoredChart>> = HashMap::new();
    let mut authored_parts: HashSet<&str> = HashSet::new();
    for a in authored {
        let sheet_part = name_to_part.get(&a.sheet_name).cloned().ok_or_else(|| {
            anyhow!(
                "no worksheet named {:?} in the workbook to author a chart onto",
                a.sheet_name
            )
        })?;
        if existing.contains(a.chart_part.as_str()) {
            return Err(anyhow!(
                "authored chart part {} already exists in the workbook (would overwrite it)",
                a.chart_part
            ));
        }
        if !authored_parts.insert(a.chart_part.as_str()) {
            return Err(anyhow!(
                "two authored charts share the part {} (parts must be unique)",
                a.chart_part
            ));
        }
        if !by_sheet.contains_key(&sheet_part) {
            order.push(sheet_part.clone());
        }
        by_sheet.entry(sheet_part).or_default().push(a);
    }

    // Free drawing-part indices (existing drawings + the ones we assign as we go).
    let mut used_drawings: HashSet<String> = ic_names
        .iter()
        .filter(|n| n.starts_with("xl/drawings/") && n.ends_with(".xml") && !n.contains("/_rels/"))
        .cloned()
        .collect();

    let mut plans: Vec<DrawingPlan> = Vec::new();
    for sheet_part in &order {
        // A worksheet can hold only one <drawing>; refuse to author onto one that already has charts.
        let ws_xml = save::read_named_string(&mut ic, sheet_part)?;
        if ws_xml.contains("<drawing ")
            || ws_xml.contains("<drawing>")
            || ws_xml.contains("<drawing/>")
        {
            return Err(anyhow!(
                "worksheet {sheet_part} already carries a <drawing>; authoring a chart onto a sheet \
                 that already has charts (merging into its drawing) is not yet supported"
            ));
        }

        let drawing_part = next_drawing_part(&mut used_drawings);
        let drawing_rels_part = xlsx::rels_part_for(&drawing_part);

        let charts = &by_sheet[sheet_part];
        let mut anchors: Vec<(Anchor, String)> = Vec::new();
        let mut rels_body = String::new();
        let mut chart_parts: Vec<(String, String)> = Vec::new();
        for (i, a) in charts.iter().enumerate() {
            let rel_id = format!("rId{}", i + 1);
            let target = save::relative_part(&drawing_part, &a.chart_part);
            rels_body.push_str(&format!(
                r#"<Relationship Id="{rel_id}" Type="{NS_REL}/chart" Target="{target}"/>"#
            ));
            anchors.push((a.anchor, rel_id));
            chart_parts.push((a.chart_part.clone(), serialize_chart_xml(&a.chart, &a.refs)));
        }
        let anchor_refs: Vec<(Anchor, &str)> =
            anchors.iter().map(|(an, id)| (*an, id.as_str())).collect();
        let drawing_xml = synthesize_drawing_xml(&anchor_refs);
        let drawing_rels_xml =
            format!(r#"{DECL}<Relationships xmlns="{NS_PKG_REL}">{rels_body}</Relationships>"#);

        plans.push(DrawingPlan {
            sheet_part: sheet_part.clone(),
            sheet_rels_part: xlsx::rels_part_for(sheet_part),
            ws_rel_id: format!("rIdAuthDraw{}", plans.len() + 1),
            drawing_part,
            drawing_rels_part,
            drawing_xml,
            drawing_rels_xml,
            charts: chart_parts,
        });
    }

    // Content-type overrides for every new drawing + chart part (package-absolute PartNames).
    let mut ct_overrides: Vec<(String, &str)> = Vec::new();
    for p in &plans {
        ct_overrides.push((format!("/{}", p.drawing_part), CT_DRAWING));
        for (cp, _) in &p.charts {
            ct_overrides.push((format!("/{cp}"), CT_CHART));
        }
    }

    // Rewrite the zip: patch content-types + target worksheets, carry everything else, then append
    // the synthesized drawing + chart parts. Reuses the single `ic` archive already open above
    // (`ZipArchive::by_name` re-seeks), so the package is parsed once for reads, not per phase.
    let patched_sheets: HashSet<&str> = plans.iter().map(|p| p.sheet_part.as_str()).collect();
    let patched_rels: HashSet<&str> = plans.iter().map(|p| p.sheet_rels_part.as_str()).collect();
    let rel_id_by_sheet: HashMap<&str, &str> = plans
        .iter()
        .map(|p| (p.sheet_part.as_str(), p.ws_rel_id.as_str()))
        .collect();

    let out = Cursor::new(Vec::<u8>::new());
    let mut zw = zip::ZipWriter::new(out);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut existing_sheet_rels: HashMap<String, String> = HashMap::new();
    for name in &ic_names {
        if name == "[Content_Types].xml" {
            let ct = save::read_named_string(&mut ic, name)?;
            let merged = add_content_type_overrides(&ct, &ct_overrides)?;
            save::write_part(&mut zw, opts, name, merged.as_bytes())?;
        } else if patched_sheets.contains(name.as_str()) {
            let ws = save::read_named_string(&mut ic, name)?;
            let rel_id = rel_id_by_sheet[name.as_str()];
            let patched = save::patch_worksheet(&ws, rel_id)?;
            save::write_part(&mut zw, opts, name, patched.as_bytes())?;
        } else if patched_rels.contains(name.as_str()) {
            // Defer: merge with our drawing relationship after the copy loop (avoid a double write).
            existing_sheet_rels.insert(name.clone(), save::read_named_string(&mut ic, name)?);
        } else {
            let bytes = save::read_named_bytes(&mut ic, name)?;
            save::write_part(&mut zw, opts, name, &bytes)?;
        }
    }

    // Merged worksheet _rels (IronCalc's own, if any, plus our drawing relationship).
    for p in &plans {
        let existing = existing_sheet_rels
            .get(&p.sheet_rels_part)
            .map(String::as_str);
        let drawing_target = save::relative_part(&p.sheet_part, &p.drawing_part);
        let rels = save::build_sheet_rels(existing, &p.ws_rel_id, &drawing_target, "")?;
        save::write_part(&mut zw, opts, &p.sheet_rels_part, rels.as_bytes())?;
    }

    // The synthesized drawing + chart parts.
    let mut synthesized_parts: Vec<String> = Vec::new();
    for p in &plans {
        save::write_part(&mut zw, opts, &p.drawing_part, p.drawing_xml.as_bytes())?;
        save::write_part(
            &mut zw,
            opts,
            &p.drawing_rels_part,
            p.drawing_rels_xml.as_bytes(),
        )?;
        synthesized_parts.push(p.drawing_part.clone());
        synthesized_parts.push(p.drawing_rels_part.clone());
        for (cp, cx) in &p.charts {
            save::write_part(&mut zw, opts, cp, cx.as_bytes())?;
            synthesized_parts.push(cp.clone());
        }
    }

    let cursor = zw.finish().context("finishing authored-chart zip")?;
    let report = AuthoredWriteReport {
        charts_authored: authored.len(),
        patched_sheets: plans.iter().map(|p| p.sheet_part.clone()).collect(),
        synthesized_parts,
    };
    Ok((cursor.into_inner(), report))
}

/// The next free `xl/drawings/drawingN.xml` part name, marking it used so the next call skips it.
fn next_drawing_part(used: &mut HashSet<String>) -> String {
    let mut n = 1;
    loop {
        let candidate = format!("xl/drawings/drawing{n}.xml");
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

/// Appends `<Override>`s for the synthesized chart/drawing parts to IronCalc's `[Content_Types].xml`.
fn add_content_type_overrides(ic_ct: &str, overrides: &[(String, &str)]) -> Result<String> {
    let mut additions = String::new();
    for (part, ct) in overrides {
        additions.push_str(&format!(
            r#"<Override PartName="{part}" ContentType="{ct}"/>"#
        ));
    }
    let close = ic_ct
        .rfind("</Types>")
        .ok_or_else(|| anyhow!("IronCalc [Content_Types].xml has no </Types>"))?;
    Ok(format!(
        "{}{}{}",
        &ic_ct[..close],
        additions,
        &ic_ct[close..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::load::{discover_and_parse, parse_chart_xml};
    use crate::chart::save::patch_chart_source;
    use crate::document::WorkbookDocument;
    use freecell_chart_model::{
        Axis, BarLayout, Category, Chart, ChartKind, Color, Legend, ScatterStyle, SeriesData,
        SizeRepresentation,
    };
    use freecell_core::CellRef;

    const CATS: [&str; 4] = ["Q1", "Q2", "Q3", "Q4"];
    const VALUES: [f64; 4] = [120.0, 150.0, 90.0, 170.0];

    fn text_categories() -> Vec<Category> {
        CATS.iter().map(|c| Category::Text((*c).into())).collect()
    }

    /// One category/value series with a color, over the `CATS`/`VALUES` grid.
    fn sales_series() -> Series {
        Series::category_value(Some("Sales"), text_categories(), VALUES.to_vec())
            .with_color(Color::from_hex(0x4472C4))
    }

    /// The `c:f` refs a `Sheet1` layout (cats A1:A4, values B1:B4, name C1) would carry.
    fn sheet1_refs() -> Vec<SeriesRefs> {
        vec![SeriesRefs {
            name: Some("Sheet1!$C$1".into()),
            categories: Some("Sheet1!$A$1:$A$4".into()),
            values: Some("Sheet1!$B$1:$B$4".into()),
            sizes: None,
        }]
    }

    fn line_chart() -> Chart {
        Chart {
            title: Some("Authored Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![sales_series()],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("Units"),
            legend: Some(Legend::default()),
        }
    }

    /// A `WorkbookDocument` (IronCalc) with the `CATS`/`VALUES` grid on `Sheet1`, serialized to the
    /// chart-less model bytes a save would produce.
    fn data_model_bytes() -> Vec<u8> {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        for (i, cat) in CATS.iter().enumerate() {
            doc.set_cell_input(0, CellRef::new(i as u32, 0), cat)
                .unwrap();
        }
        for (i, v) in VALUES.iter().enumerate() {
            doc.set_cell_input(0, CellRef::new(i as u32, 1), &format!("{v}"))
                .unwrap();
        }
        doc.set_cell_input(0, CellRef::new(0, 2), "Sales").unwrap();
        doc.evaluate();
        doc.to_xlsx_bytes().unwrap()
    }

    fn authored(chart: Chart, chart_part: &str, refs: Vec<SeriesRefs>) -> AuthoredChart {
        AuthoredChart {
            sheet_name: "Sheet1".into(),
            chart_part: chart_part.into(),
            chart,
            anchor: Anchor::new(
                freecell_chart_model::AnchorCell::new(4, 1),
                freecell_chart_model::AnchorCell::new(12, 15),
            ),
            refs,
        }
    }

    fn first_value(chart: &Chart) -> f64 {
        match &chart.series[0].data {
            SeriesData::CategoryValue { values, .. } => values[0],
            other => panic!("expected CategoryValue, got {other:?}"),
        }
    }

    // --- serializer round-trip through our own parser ---------------------------------------

    fn assert_roundtrip(chart: Chart, refs: &[SeriesRefs]) {
        let xml = serialize_chart_xml(&chart, refs);
        assert!(
            roxmltree::Document::parse(&xml).is_ok(),
            "serialized chart XML is well-formed:\n{xml}"
        );
        let parsed =
            parse_chart_xml(&xml).unwrap_or_else(|e| panic!("re-parse failed: {e:#}\n{xml}"));
        assert_eq!(parsed, chart, "serialize→parse must reconstruct the model");
    }

    #[test]
    fn serialize_roundtrips_line() {
        assert_roundtrip(line_chart(), &sheet1_refs());
    }

    #[test]
    fn serialize_roundtrips_bar_both_orientations() {
        // Both orientations × both a default and a NON-default gap/overlap layout — the P22
        // `c:gapWidth`/`c:overlap` must round-trip serialize→parse, not just the default.
        for dir in [BarDir::Col, BarDir::Bar] {
            for layout in [BarLayout::default(), BarLayout::new(60, 40)] {
                let chart = Chart {
                    title: Some("Bars".into()),
                    kind: ChartKind::Bar {
                        dir,
                        grouping: Grouping::Clustered,
                        layout,
                    },
                    series: vec![sales_series()],
                    cat_axis: Axis::titled("Quarter"),
                    val_axis: Axis::default(),
                    legend: Some(Legend::default()),
                };
                assert_roundtrip(chart, &sheet1_refs());
            }
        }
    }

    /// The serialized `c:barChart` carries the `c:gapWidth` / `c:overlap` (in schema order, before the
    /// `c:axId` pair) — the P22 layout is emitted, not just modeled.
    #[test]
    fn serialize_emits_gap_width_and_overlap() {
        let chart = Chart {
            title: Some("Bars".into()),
            kind: ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered,
                layout: BarLayout::new(75, -20),
            },
            series: vec![sales_series()],
            cat_axis: Axis::default(),
            val_axis: Axis::default(),
            legend: None,
        };
        let xml = serialize_chart_xml(&chart, &sheet1_refs());
        assert!(
            xml.contains(r#"<c:gapWidth val="75"/>"#),
            "gapWidth emitted:\n{xml}"
        );
        assert!(
            xml.contains(r#"<c:overlap val="-20"/>"#),
            "overlap emitted:\n{xml}"
        );
        // Schema order: both spacing knobs precede the axId pair.
        let gap = xml.find("<c:gapWidth").unwrap();
        let ax = xml.find("<c:axId").unwrap();
        assert!(gap < ax, "gapWidth/overlap must precede axId");
    }

    #[test]
    fn serialize_roundtrips_area_with_scaling_and_numfmt() {
        let chart = Chart {
            title: None, // exercise the auto-title-deleted path
            kind: ChartKind::Area {
                grouping: Grouping::Stacked,
            },
            series: vec![sales_series()],
            cat_axis: Axis::titled("Quarter").without_major_gridlines(),
            val_axis: Axis::titled("Units")
                .with_bounds(Some(0.0), Some(200.0))
                .with_number_format("#,##0"),
            legend: None,
        };
        assert_roundtrip(chart, &sheet1_refs());
    }

    /// P23: an authored area serializes + round-trips through the loader in **all three groupings**
    /// (standard / stacked / percentStacked), so each grouping survives the write→parse cycle.
    #[test]
    fn serialize_roundtrips_area_all_groupings() {
        for grouping in [
            Grouping::Standard,
            Grouping::Stacked,
            Grouping::PercentStacked,
        ] {
            let chart = Chart {
                title: Some("Traffic".into()),
                kind: ChartKind::Area { grouping },
                series: vec![sales_series()],
                cat_axis: Axis::titled("Quarter"),
                val_axis: Axis::titled("Visits"),
                legend: Some(Legend::default()),
            };
            assert_roundtrip(chart, &sheet1_refs());
        }
    }

    #[test]
    fn serialize_roundtrips_pie_and_doughnut() {
        for hole in [None, Some(0.5)] {
            let chart = Chart {
                title: Some("Share".into()),
                kind: ChartKind::Pie {
                    doughnut_hole: hole,
                    first_slice_ang: 0,
                    vary_colors: true,
                },
                series: vec![sales_series()],
                cat_axis: Axis::default(),
                val_axis: Axis::default(),
                legend: Some(Legend::default()),
            };
            assert_roundtrip(chart, &sheet1_refs());
        }
    }

    /// P24: a doughnut carrying the full pie feature set — a **rotation** (`firstSliceAng`),
    /// `varyColors` OFF, a `holeSize`, and a `c:dPt` per-slice override (sRGB color + explosion) —
    /// round-trips serialize→parse, so every P24 field survives the write path.
    #[test]
    fn serialize_roundtrips_pie_with_dpt_rotation_and_hole() {
        use freecell_chart_model::DataPoint;
        let chart = Chart {
            title: Some("Segments".into()),
            kind: ChartKind::Pie {
                doughnut_hole: Some(0.4),
                first_slice_ang: 90,
                vary_colors: false,
            },
            series: vec![sales_series().with_data_points(vec![DataPoint {
                index: 1,
                color: Some(ChartColor::Rgb(Color::from_hex(0xE15759))),
                explosion: Some(20),
            }])],
            cat_axis: Axis::default(),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let xml = serialize_chart_xml(&chart, &sheet1_refs());
        assert!(
            xml.contains(r#"<c:firstSliceAng val="90"/>"#),
            "rotation:\n{xml}"
        );
        assert!(
            xml.contains(r#"<c:varyColors val="0"/>"#),
            "varyColors off:\n{xml}"
        );
        assert!(
            xml.contains(r#"<c:explosion val="20"/>"#),
            "explosion:\n{xml}"
        );
        assert!(xml.contains(r#"<c:dPt>"#), "dPt emitted:\n{xml}");
        assert_roundtrip(chart, &sheet1_refs());
    }

    /// P20: an authored series carrying **data labels** serializes a `c:dLbls` (between `spPr` and
    /// the data roles) that round-trips through the loader — the authored twin of the loaded chrome
    /// patch.
    #[test]
    fn serialize_roundtrips_series_data_labels() {
        use freecell_chart_model::{DataLabelPosition, DataLabels};
        let labels = DataLabels::new()
            .value()
            .category_name()
            .with_number_format("#,##0")
            .at(DataLabelPosition::Above);
        let chart = Chart {
            title: Some("Labeled".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![sales_series().with_data_labels(labels.clone())],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let xml = serialize_chart_xml(&chart, &sheet1_refs());
        assert!(xml.contains("<c:dLbls>"), "data labels emitted");
        assert_roundtrip(chart, &sheet1_refs());
    }

    /// A one-series scatter with the given style + `Sheet1` refs — the shared shape for the scatter
    /// round-trip tests.
    fn scatter_chart(style: ScatterStyle) -> Chart {
        Chart {
            title: Some("XY".into()),
            kind: ChartKind::Scatter { style },
            series: vec![
                Series::xy(Some("Pts"), vec![1.0, 2.0, 3.0], vec![10.0, 20.0, 30.0])
                    .with_color(Color::from_hex(0xED7D31)),
            ],
            cat_axis: Axis::titled("X"),
            val_axis: Axis::titled("Y"),
            legend: None,
        }
    }

    fn scatter_refs() -> Vec<SeriesRefs> {
        vec![SeriesRefs {
            name: Some("Sheet1!$A$1".into()),
            categories: Some("Sheet1!$B$1:$B$3".into()),
            values: Some("Sheet1!$C$1:$C$3".into()),
            sizes: None,
        }]
    }

    #[test]
    fn serialize_roundtrips_scatter() {
        assert_roundtrip(scatter_chart(ScatterStyle::LineMarker), &scatter_refs());
    }

    #[test]
    fn serialize_roundtrips_scatter_styles() {
        // The c:scatterStyle is emitted from the model and survives serialize→parse for every style.
        for style in [
            ScatterStyle::Marker,
            ScatterStyle::Line,
            ScatterStyle::LineMarker,
            ScatterStyle::Smooth,
            ScatterStyle::SmoothMarker,
        ] {
            let xml = serialize_chart_xml(&scatter_chart(style), &scatter_refs());
            assert!(
                xml.contains(&format!("<c:scatterStyle val=\"{}\"/>", style.as_ooxml())),
                "scatterStyle {style:?} emitted from the model:\n{xml}"
            );
            assert_roundtrip(scatter_chart(style), &scatter_refs());
        }
    }

    /// A one-series bubble with the given size representation + refs — the shared shape for the
    /// bubble round-trip tests. x from the A column, y from the B column, size from the C column.
    fn bubble_chart(representation: SizeRepresentation) -> Chart {
        Chart {
            title: Some("XYZ".into()),
            kind: ChartKind::Bubble {
                size_representation: representation,
            },
            series: vec![Series::bubble(
                Some("Pts"),
                vec![1.0, 2.0, 3.0, 4.0],
                VALUES.to_vec(),
                vec![4.0, 16.0, 9.0, 25.0],
            )
            .with_color(Color::from_hex(0x4472C4))],
            cat_axis: Axis::titled("X"),
            val_axis: Axis::titled("Y"),
            legend: Some(Legend::default()),
        }
    }

    fn bubble_refs() -> Vec<SeriesRefs> {
        vec![SeriesRefs {
            name: Some("Sheet1!$D$1".into()),
            categories: Some("Sheet1!$A$1:$A$4".into()),
            values: Some("Sheet1!$B$1:$B$4".into()),
            sizes: Some("Sheet1!$C$1:$C$4".into()),
        }]
    }

    #[test]
    fn serialize_roundtrips_bubble() {
        // A full serialize→parse == template for a bubble (kind + XY + bubbleSize + axes + legend).
        assert_roundtrip(bubble_chart(SizeRepresentation::Area), &bubble_refs());
    }

    #[test]
    fn serialize_roundtrips_bubble_size_representation() {
        // `c:sizeRepresents` is emitted FROM the model and survives serialize→parse for both
        // representations; and the `c:bubbleSize` element is present (the third range).
        for rep in [SizeRepresentation::Area, SizeRepresentation::Width] {
            let xml = serialize_chart_xml(&bubble_chart(rep), &bubble_refs());
            assert!(
                xml.contains(&format!("<c:sizeRepresents val=\"{}\"/>", rep.as_ooxml())),
                "sizeRepresents {rep:?} emitted from the model:\n{xml}"
            );
            assert!(
                xml.contains("<c:bubbleSize>"),
                "bubbleSize (the third range) emitted:\n{xml}"
            );
            // CT_BubbleChart order: sizeRepresents precedes the axId pair.
            let sr = xml.find("<c:sizeRepresents").unwrap();
            let ax = xml.find("<c:axId").unwrap();
            assert!(sr < ax, "sizeRepresents must precede axId");
            assert_roundtrip(bubble_chart(rep), &bubble_refs());
        }
    }

    #[test]
    fn write_authored_bubble_reopens_as_bubble_with_size() {
        let model = data_model_bytes();
        let bubble = bubble_chart(SizeRepresentation::Area);
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(bubble, "xl/charts/chart1.xml", bubble_refs())],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored_bubble.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored bubble reopens in IronCalc");

        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        let chart = specs[0].chart().unwrap();
        assert_eq!(
            chart.kind,
            ChartKind::Bubble {
                size_representation: SizeRepresentation::Area
            },
            "the reopened chart is an area-represented bubble"
        );
        match &chart.series[0].data {
            SeriesData::Xy {
                size: Some(size), ..
            } => {
                assert_eq!(
                    size,
                    &vec![4.0, 16.0, 9.0, 25.0],
                    "the bubbleSize survived the write"
                );
            }
            other => panic!("expected a bubble Xy with size, got {other:?}"),
        }
    }

    #[test]
    fn serialize_roundtrips_numeric_categories() {
        let chart = Chart {
            title: Some("Numeric cats".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: true,
            },
            series: vec![Series::category_value(
                Some("s"),
                vec![
                    Category::Number(2021.0),
                    Category::Number(2022.0),
                    Category::Number(2023.0),
                ],
                vec![5.0, 6.0, 7.0],
            )],
            cat_axis: Axis::default(),
            val_axis: Axis::default(),
            legend: None,
        };
        assert_roundtrip(chart, &sheet1_refs());
    }

    #[test]
    fn serialize_emits_cf_ranges_in_order() {
        let xml = serialize_chart_xml(&line_chart(), &sheet1_refs());
        assert!(xml.contains("<c:f>Sheet1!$C$1</c:f>"), "name c:f");
        assert!(xml.contains("<c:f>Sheet1!$A$1:$A$4</c:f>"), "cat c:f");
        assert!(xml.contains("<c:f>Sheet1!$B$1:$B$4</c:f>"), "val c:f");
    }

    #[test]
    fn serialize_no_ref_role_emits_literal_not_broken_cf() {
        // No refs → data still serializes, as literals, never a broken empty <c:f>.
        let xml = serialize_chart_xml(&line_chart(), &[]);
        assert!(roxmltree::Document::parse(&xml).is_ok());
        assert!(xml.contains("<c:numLit>"), "values fall back to numLit");
        assert!(xml.contains("<c:strLit>"), "categories fall back to strLit");
        assert!(!xml.contains("<c:f>"), "no c:f at all when no refs");
        assert!(!xml.contains("<c:f/>") && !xml.contains("<c:f></c:f>"));
    }

    /// P17 round-trip guard: **every** [`ChartInsertKind`]'s near-empty insert template must survive
    /// the write path (serialize → parse), not just Line. P22–P26 build breadth directly on these
    /// templates, so a template that emits something the serializer/loader can't round-trip must fail
    /// here rather than in a later phase.
    ///
    /// Two shapes, per kind:
    /// - **with refs** — a full `serialize→parse == template` (the refs make the placeholder DATA
    ///   round-trip through `numRef`/`strRef` caches, so the whole template is checked, not just its
    ///   chrome);
    /// - **ref-less** — the *actual* insert/save shape (`&[]` → literals, §2.3) must still be
    ///   well-formed XML whose structure re-parses (a ref-less template legitimately loses its literal
    ///   data on reload — the loader reads caches, not literals — so this half asserts structure only).
    #[test]
    fn near_empty_insert_templates_round_trip_through_the_write_path() {
        use freecell_chart_model::ChartInsertKind;

        let refs = vec![SeriesRefs {
            name: Some("Sheet1!$A$1".into()),
            categories: Some("Sheet1!$A$2:$A$5".into()),
            values: Some("Sheet1!$B$2:$B$5".into()),
            // A size ref so the Bubble template's `c:bubbleSize` round-trips through a numRef cache
            // (harmless for the non-bubble kinds — their series emit no bubbleSize).
            sizes: Some("Sheet1!$C$2:$C$5".into()),
        }];
        for kind in [
            ChartInsertKind::Line,
            ChartInsertKind::Column,
            ChartInsertKind::Bar,
            ChartInsertKind::Area,
            ChartInsertKind::Pie,
            ChartInsertKind::Doughnut,
            ChartInsertKind::Scatter,
            ChartInsertKind::Bubble,
        ] {
            let template = kind.near_empty_chart();
            // (a) Full structural + data round-trip with refs.
            assert_roundtrip(template.clone(), &refs);

            // (b) The real ref-less insert/save shape (literals) is still well-formed, and its
            // structure (kind / title / axes / legend / series name) re-parses.
            let literal_xml = serialize_chart_xml(&template, &[]);
            assert!(
                roxmltree::Document::parse(&literal_xml).is_ok(),
                "{kind:?} ref-less near-empty template must be well-formed XML:\n{literal_xml}"
            );
            let reparsed = parse_chart_xml(&literal_xml).unwrap_or_else(|e| {
                panic!("{kind:?} ref-less re-parse failed: {e:#}\n{literal_xml}")
            });
            assert_eq!(reparsed.kind, template.kind, "{kind:?} kind round-trips");
            assert_eq!(reparsed.title, template.title, "{kind:?} title round-trips");
            assert_eq!(
                reparsed.legend, template.legend,
                "{kind:?} legend round-trips"
            );
        }
    }

    // --- drawing synthesis -------------------------------------------------------------------

    #[test]
    fn synthesize_drawing_roundtrips_the_anchor() {
        // The synthesized drawing's anchor must re-discover to the same Anchor via the load path.
        let model = data_model_bytes();
        let anchor = Anchor::new(
            freecell_chart_model::AnchorCell::with_offsets(2, 12_700, 5, 6_350),
            freecell_chart_model::AnchorCell::with_offsets(9, 0, 20, 0),
        );
        let mut a = authored(line_chart(), "xl/charts/chart1.xml", sheet1_refs());
        a.anchor = anchor;
        let (bytes, _) = write_authored_charts(&model, &[a]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("anchored.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0].anchor, anchor,
            "anchor round-trips through the drawing"
        );
    }

    // --- package assembly: write -> reopen ---------------------------------------------------

    #[test]
    fn write_authored_chart_reopens_and_reparses() {
        let model = data_model_bytes();
        let (bytes, report) = write_authored_charts(
            &model,
            &[authored(
                line_chart(),
                "xl/charts/chart1.xml",
                sheet1_refs(),
            )],
        )
        .unwrap();
        assert_eq!(report.charts_authored, 1);
        assert_eq!(report.patched_sheets, vec!["xl/worksheets/sheet1.xml"]);
        // The report names exactly the parts synthesized from scratch (honest authored provenance).
        assert_eq!(
            report.synthesized_parts,
            vec![
                "xl/drawings/drawing1.xml".to_string(),
                "xl/drawings/_rels/drawing1.xml.rels".to_string(),
                "xl/charts/chart1.xml".to_string(),
            ]
        );

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // (a) IronCalc reopens the package (valid OPC).
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored workbook reopens in IronCalc");

        // (b) Our loader re-reads it as a Loaded line chart with the authored values + ranges.
        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert!(
            spec.is_loaded(),
            "reopened authored chart is now a Loaded spec"
        );
        let chart = spec.chart().unwrap();
        assert!(matches!(chart.kind, ChartKind::Line { .. }));
        assert_eq!(first_value(chart), 120.0);
        assert_eq!(chart.title.as_deref(), Some("Authored Sales"));
        // The c:f ranges re-parse (live-binding ready).
        let ranges: Vec<&str> = spec.source_ranges.iter().map(|r| r.as_str()).collect();
        assert!(ranges.contains(&"Sheet1!$A$1:$A$4"));
        assert!(ranges.contains(&"Sheet1!$B$1:$B$4"));

        // (c) The worksheet carries a <drawing> and content-types declare the new parts.
        assert!(xlsx::read_entry(&out, "xl/worksheets/sheet1.xml")
            .unwrap()
            .contains("<drawing "));
        let ct = xlsx::read_entry(&out, "[Content_Types].xml").unwrap();
        assert!(ct.contains("/xl/charts/chart1.xml"));
        assert!(ct.contains("/xl/drawings/drawing1.xml"));
    }

    /// P22 end-to-end: an authored **horizontal bar** with a non-default `gapWidth`/`overlap` reopens
    /// through the full write→discover path as a `ChartKind::Bar { dir: Bar }` carrying its layout.
    #[test]
    fn write_authored_bar_reopens_as_horizontal_bar_with_layout() {
        let model = data_model_bytes();
        let bar = Chart {
            title: Some("Authored Bars".into()),
            kind: ChartKind::Bar {
                dir: BarDir::Bar,
                grouping: Grouping::Clustered,
                layout: BarLayout::new(75, 25),
            },
            series: vec![sales_series()],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(bar, "xl/charts/chart1.xml", sheet1_refs())],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored_bar.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored bar reopens in IronCalc");

        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0].chart().unwrap().kind,
            ChartKind::Bar {
                dir: BarDir::Bar,
                grouping: Grouping::Clustered,
                layout: BarLayout::new(75, 25),
            },
            "the reopened chart is a horizontal bar with its authored gap/overlap"
        );
    }

    /// P23: an **authored** stacked area written via the write path reopens through
    /// `discover_and_parse` as `ChartKind::Area { grouping: Stacked }` — the area twin of the authored
    /// bar reopen, proving the full write→(IronCalc load)→discover round-trip preserves the area kind +
    /// grouping.
    #[test]
    fn write_authored_area_reopens_as_area_with_grouping() {
        let model = data_model_bytes();
        let area = Chart {
            title: Some("Authored Area".into()),
            kind: ChartKind::Area {
                grouping: Grouping::Stacked,
            },
            series: vec![sales_series()],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(area, "xl/charts/chart1.xml", sheet1_refs())],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored_area.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored area reopens in IronCalc");

        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0].chart().unwrap().kind,
            ChartKind::Area {
                grouping: Grouping::Stacked,
            },
            "the reopened chart is a stacked area"
        );
    }

    /// P24: an **authored** doughnut (holeSize + rotation) written via the write path reopens through
    /// `discover_and_parse` as `ChartKind::Pie { doughnut_hole: Some(..) }` with the rotation
    /// preserved — the pie twin of the authored bar/area reopen, proving the full
    /// write→(IronCalc load)→discover round-trip keeps the pie kind + hole + firstSliceAng.
    #[test]
    fn write_authored_pie_reopens_as_pie_with_hole() {
        let model = data_model_bytes();
        let doughnut = Chart {
            title: Some("Authored Doughnut".into()),
            kind: ChartKind::Pie {
                doughnut_hole: Some(0.5),
                first_slice_ang: 45,
                vary_colors: true,
            },
            series: vec![sales_series()],
            cat_axis: Axis::default(),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(doughnut, "xl/charts/chart1.xml", sheet1_refs())],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored_doughnut.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored doughnut reopens in IronCalc");

        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        match &specs[0].chart().unwrap().kind {
            ChartKind::Pie {
                doughnut_hole: Some(h),
                first_slice_ang,
                ..
            } => {
                assert!((h - 0.5).abs() < 1e-6, "the doughnut hole round-trips");
                assert_eq!(*first_slice_ang, 45, "the rotation round-trips");
            }
            other => panic!("expected a doughnut, got {other:?}"),
        }
    }

    /// P25: an **authored** scatter (marker style) written via the write path reopens through
    /// `discover_and_parse` as `ChartKind::Scatter { style: Marker }` with an `Xy` series — the scatter
    /// twin of the authored bar/area/pie reopen, proving the full write→(IronCalc load)→discover
    /// round-trip keeps the scatter kind + style + the XY data shape.
    #[test]
    fn write_authored_scatter_reopens_as_scatter_with_style() {
        let model = data_model_bytes();
        let scatter = Chart {
            title: Some("Authored Scatter".into()),
            kind: ChartKind::Scatter {
                style: ScatterStyle::Marker,
            },
            // An xy series: x from the A column (cats ref → xVal), y from the B column (values ref → yVal).
            series: vec![
                Series::xy(Some("Pts"), vec![1.0, 2.0, 3.0, 4.0], VALUES.to_vec())
                    .with_color(Color::from_hex(0x4472C4)),
            ],
            cat_axis: Axis::titled("X"),
            val_axis: Axis::titled("Y"),
            legend: Some(Legend::default()),
        };
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(scatter, "xl/charts/chart1.xml", sheet1_refs())],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("authored_scatter.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en")
            .expect("authored scatter reopens in IronCalc");

        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 1);
        let chart = specs[0].chart().unwrap();
        assert_eq!(
            chart.kind,
            ChartKind::Scatter {
                style: ScatterStyle::Marker
            },
            "the reopened chart is a marker scatter"
        );
        assert!(
            matches!(chart.series[0].data, SeriesData::Xy { .. }),
            "the reopened scatter series is xy"
        );
    }

    #[test]
    fn write_two_authored_charts_on_one_sheet_share_a_drawing() {
        let model = data_model_bytes();
        let bar = Chart {
            title: Some("Bars".into()),
            kind: ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered,
                layout: BarLayout::default(),
            },
            series: vec![sales_series()],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::default(),
            legend: Some(Legend::default()),
        };
        let charts = [
            authored(line_chart(), "xl/charts/chart1.xml", sheet1_refs()),
            authored(bar, "xl/charts/chart2.xml", sheet1_refs()),
        ];
        let (bytes, report) = write_authored_charts(&model, &charts).unwrap();
        assert_eq!(report.charts_authored, 2);
        // Both charts share the one synthesized drawing (its rels + both chart parts).
        assert_eq!(
            report.synthesized_parts,
            vec![
                "xl/drawings/drawing1.xml".to_string(),
                "xl/drawings/_rels/drawing1.xml.rels".to_string(),
                "xl/charts/chart1.xml".to_string(),
                "xl/charts/chart2.xml".to_string(),
            ]
        );

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("two.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();

        // Both charts discovered; they share ONE drawing (drawing2 was never created).
        let specs = discover_and_parse(&out).unwrap();
        assert_eq!(specs.len(), 2);
        assert!(xlsx::read_entry(&out, "xl/drawings/drawing1.xml").is_ok());
        assert!(xlsx::read_entry(&out, "xl/drawings/drawing2.xml").is_err());
    }

    #[test]
    fn write_fails_loudly_on_unknown_sheet() {
        let model = data_model_bytes();
        let mut a = authored(line_chart(), "xl/charts/chart1.xml", sheet1_refs());
        a.sheet_name = "Ghost".into();
        let err = write_authored_charts(&model, &[a]).unwrap_err();
        assert!(
            format!("{err:#}").contains("Ghost"),
            "error must name the missing sheet: {err:#}"
        );
    }

    #[test]
    fn write_fails_loudly_on_sheet_with_existing_drawing() {
        let model = data_model_bytes();
        // First authored chart succeeds and gives Sheet1 a <drawing>.
        let (bytes, _) = write_authored_charts(
            &model,
            &[authored(
                line_chart(),
                "xl/charts/chart1.xml",
                sheet1_refs(),
            )],
        )
        .unwrap();
        // A second authored chart onto the SAME sheet (now already drawn) must fail loudly.
        let err = write_authored_charts(
            &bytes,
            &[authored(
                line_chart(),
                "xl/charts/chart2.xml",
                sheet1_refs(),
            )],
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("already carries a <drawing>"),
            "error must explain the existing-drawing precondition: {err:#}"
        );
    }

    #[test]
    fn write_fails_loudly_on_duplicate_chart_part() {
        let model = data_model_bytes();
        let charts = [
            authored(line_chart(), "xl/charts/chart1.xml", sheet1_refs()),
            authored(line_chart(), "xl/charts/chart1.xml", sheet1_refs()),
        ];
        let err = write_authored_charts(&model, &charts).unwrap_err();
        assert!(format!("{err:#}").contains("unique"), "{err:#}");
    }

    // --- reconciliation with the source-patch path -------------------------------------------

    #[test]
    fn authored_caches_match_a_reflow_byte_for_byte() {
        // An authored chart's value/category/name caches are in the exact canonical shape the
        // edited-loaded reflow patcher produces, so a no-op reflow of a serialized authored chart is
        // byte-identical — the write-path/source-patch reconciliation invariant (write-path §4).
        let xml = serialize_chart_xml(&line_chart(), &sheet1_refs());
        let parsed = parse_chart_xml(&xml).unwrap();
        let repatched = patch_chart_source(&xml, &parsed).unwrap();
        assert_eq!(
            repatched, xml,
            "a no-op reflow of a serialized authored chart must not change a byte"
        );
    }
}
