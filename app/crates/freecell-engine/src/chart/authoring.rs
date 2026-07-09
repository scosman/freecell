//! Programmatic OOXML fixture authoring (functional_spec §10 #4 — the agent authors the
//! example `.xlsx`). Scales up the `open_fixups.rs::write_crafted_xlsx` pattern to a full,
//! **valid, minimal, single-sheet** workbook carrying THREE embedded charts — a clustered
//! **column**, a multi-series **line**, and a **pie** — with hand-set `numCache`/`strCache`
//! so every loaded value is known and asserted in tests.
//!
//! Hand-crafting (over LibreOffice) is the phase-brief's "fullest control over the cached
//! values … most reliable path for a PoC" option. The workbook is deliberately kept to the
//! parts IronCalc's importer actually reads (`[Content_Types]`, `_rels`, workbook + its rels,
//! one worksheet + its rels, styles, sharedStrings) plus the drawing/chart parts — no theme,
//! no docProps — so `ironcalc::import::load_from_xlsx` accepts it (asserted in tests).

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// The four category labels shared by all three charts (`Data!$A$2:$A$5`).
pub const CATEGORIES: [&str; 4] = ["Q1", "Q2", "Q3", "Q4"];
/// The "Widgets" series values (`Data!$B$2:$B$5`).
pub const WIDGETS: [f64; 4] = [120.0, 150.0, 90.0, 170.0];
/// The "Gadgets" series values (`Data!$C$2:$C$5`).
pub const GADGETS: [f64; 4] = [80.0, 110.0, 140.0, 100.0];
/// The "Total" series values, pie slices (`Data!$D$2:$D$5`).
pub const TOTALS: [f64; 4] = [200.0, 260.0, 230.0, 270.0];

/// The three chart parts authored into the fixture, in drawing order.
pub const CHART_PARTS: [&str; 3] = [
    "xl/charts/chart1.xml",
    "xl/charts/chart2.xml",
    "xl/charts/chart3.xml",
];

/// The titles of the three charts, in drawing order (column, line, pie).
pub const CHART_TITLES: [&str; 3] = [
    "Quarterly Sales by Product",
    "Sales Trend by Product",
    "Quarterly Totals",
];

const WIDGETS_COLOR: &str = "4472C4"; // blue
const GADGETS_COLOR: &str = "ED7D31"; // orange

/// Writes the fixture `.xlsx` to `path` (creating parent dirs). Returns the chart part names.
pub fn write_fixture(path: &Path) -> Result<Vec<String>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let parts: &[(&str, String)] = &[
        ("[Content_Types].xml", content_types()),
        ("_rels/.rels", root_rels()),
        ("xl/workbook.xml", workbook()),
        ("xl/_rels/workbook.xml.rels", workbook_rels()),
        ("xl/styles.xml", styles()),
        ("xl/sharedStrings.xml", shared_strings()),
        ("xl/worksheets/sheet1.xml", worksheet()),
        ("xl/worksheets/_rels/sheet1.xml.rels", worksheet_rels()),
        ("xl/drawings/drawing1.xml", drawing()),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels()),
        ("xl/charts/chart1.xml", column_chart()),
        ("xl/charts/chart2.xml", line_chart()),
        ("xl/charts/chart3.xml", pie_chart()),
    ];
    for (name, body) in parts {
        zw.start_file(*name, opts)
            .with_context(|| format!("starting zip entry {name}"))?;
        zw.write_all(body.as_bytes())
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    zw.finish().context("finishing fixture zip")?;
    Ok(CHART_PARTS.iter().map(|s| s.to_string()).collect())
}

// ---------------------------------------------------------------------------------------------
// Package boilerplate
// ---------------------------------------------------------------------------------------------

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

fn content_types() -> String {
    format!(
        r#"{DECL}
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="xml" ContentType="application/xml"/>
 <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
 <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
 <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
 <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
 <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
 <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
 <Override PartName="/xl/charts/chart2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
 <Override PartName="/xl/charts/chart3.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
</Types>"#
    )
}

// IMPORTANT: relationship parts carry NO inter-element whitespace. IronCalc's `load_sheet_rels`
// iterates the raw children of `<Relationships>` and calls `get_attribute(child, "Type")` on each
// — a whitespace text node between elements would trip "Missing Type". Real Excel writes these on
// one line for the same reason.
fn root_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    )
}

fn workbook() -> String {
    format!(
        r#"{DECL}
<workbook xmlns="{NS_MAIN}" xmlns:r="{NS_REL}">
 <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#
    )
}

fn workbook_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{NS_REL}/styles" Target="styles.xml"/><Relationship Id="rId3" Type="{NS_REL}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
    )
}

fn styles() -> String {
    // Minimal but complete; the single cellXf carries xfId="0" so IronCalc's strict styles
    // parser accepts it (the `<xf>`-without-xfId rejection open_repair.rs works around).
    format!(
        r#"{DECL}
<styleSheet xmlns="{NS_MAIN}">
 <fonts count="1"><font><sz val="11"/><color rgb="FF000000"/><name val="Calibri"/><family val="2"/></font></fonts>
 <fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
 <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
 <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
 <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
 <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#
    )
}

fn shared_strings() -> String {
    format!(
        r#"{DECL}
<sst xmlns="{NS_MAIN}" count="7" uniqueCount="7">
 <si><t>Widgets</t></si><si><t>Gadgets</t></si><si><t>Total</t></si>
 <si><t>Q1</t></si><si><t>Q2</t></si><si><t>Q3</t></si><si><t>Q4</t></si>
</sst>"#
    )
}

fn worksheet() -> String {
    // A1:D5 grid: row 1 = series headers (shared strings 0..2), col A = categories (3..6),
    // cols B/C/D = the widgets/gadgets/total numbers. The `<drawing r:id="rId1"/>` anchors
    // the three charts (this is exactly the element IronCalc drops on save).
    //
    // The `<sheetData>` has NO inter-element whitespace: IronCalc iterates the raw row/cell
    // children and reads `r` on each, so a whitespace text node would trip "Missing r".
    let n = |v: f64| fmt_num(v);
    // Rows 2..5: category label in col A (shared strings 3..6) + three numbers.
    let mut data_rows = String::new();
    for i in 0..4 {
        let row = i + 2; // sheet rows 2..5
        let sst = i + 3; // shared-string indices 3..6 for Q1..Q4
        data_rows.push_str(&format!(
            r#"<row r="{row}"><c r="A{row}" t="s"><v>{sst}</v></c><c r="B{row}"><v>{}</v></c><c r="C{row}"><v>{}</v></c><c r="D{row}"><v>{}</v></c></row>"#,
            n(WIDGETS[i]), n(GADGETS[i]), n(TOTALS[i]),
        ));
    }
    format!(
        r#"{DECL}
<worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_REL}"><dimension ref="A1:D5"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData><row r="1"><c r="B1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c><c r="D1" t="s"><v>2</v></c></row>{data_rows}</sheetData><drawing r:id="rId1"/></worksheet>"#
    )
}

fn worksheet_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#
    )
}

// ---------------------------------------------------------------------------------------------
// Drawing (anchors) — three graphic frames, each pointing at a chart part
// ---------------------------------------------------------------------------------------------

fn drawing() -> String {
    format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
 {a1}
 {a2}
 {a3}
</xdr:wsDr>"#,
        a1 = anchor(1, 5, 0, 15, 10),
        a2 = anchor(2, 5, 11, 15, 21),
        a3 = anchor(3, 17, 0, 27, 10),
    )
}

/// One `twoCellAnchor` graphic frame at a cell rectangle, referencing chart part `n` via
/// `r:id="rId{n}"` (matched in [`drawing_rels`]).
fn anchor(n: u32, from_row: u32, from_col: u32, to_row: u32, to_col: u32) -> String {
    format!(
        r#"<xdr:twoCellAnchor>
  <xdr:from><xdr:col>{from_col}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{from_row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>{to_col}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{to_row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  <xdr:graphicFrame macro="">
   <xdr:nvGraphicFramePr>
    <xdr:cNvPr id="{gid}" name="Chart {n}"/>
    <xdr:cNvGraphicFramePr/>
   </xdr:nvGraphicFramePr>
   <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>
   <a:graphic><a:graphicData uri="{NS_CHART}">
     <c:chart xmlns:c="{NS_CHART}" xmlns:r="{NS_REL}" r:id="rId{n}"/>
   </a:graphicData></a:graphic>
  </xdr:graphicFrame>
  <xdr:clientData/>
 </xdr:twoCellAnchor>"#,
        gid = n + 1,
    )
}

fn drawing_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/chart" Target="../charts/chart1.xml"/><Relationship Id="rId2" Type="{NS_REL}/chart" Target="../charts/chart2.xml"/><Relationship Id="rId3" Type="{NS_REL}/chart" Target="../charts/chart3.xml"/></Relationships>"#
    )
}

// ---------------------------------------------------------------------------------------------
// Chart parts
// ---------------------------------------------------------------------------------------------

fn column_chart() -> String {
    let group = format!(
        r#"<c:barChart>
   <c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>
   {w}{g}
   <c:axId val="111111111"/><c:axId val="222222222"/>
  </c:barChart>
  {cat_ax}
  {val_ax}"#,
        w = catval_series(0, "Widgets", WIDGETS_COLOR, "$B$1", "$B$2:$B$5", &WIDGETS),
        g = catval_series(1, "Gadgets", GADGETS_COLOR, "$C$1", "$C$2:$C$5", &GADGETS),
        cat_ax = cat_axis("Quarter"),
        val_ax = val_axis("Units (thousands)"),
    );
    chart_space(CHART_TITLES[0], &group, true)
}

fn line_chart() -> String {
    let group = format!(
        r#"<c:lineChart>
   <c:grouping val="standard"/><c:varyColors val="0"/>
   {w}{g}
   <c:marker val="1"/>
   <c:axId val="111111111"/><c:axId val="222222222"/>
  </c:lineChart>
  {cat_ax}
  {val_ax}"#,
        w = catval_series(0, "Widgets", WIDGETS_COLOR, "$B$1", "$B$2:$B$5", &WIDGETS),
        g = catval_series(1, "Gadgets", GADGETS_COLOR, "$C$1", "$C$2:$C$5", &GADGETS),
        cat_ax = cat_axis("Quarter"),
        val_ax = val_axis("Units (thousands)"),
    );
    chart_space(CHART_TITLES[1], &group, true)
}

fn pie_chart() -> String {
    // Single series over the Total column; no axes. Legend maps each quarter to its slice.
    let group = format!(
        r#"<c:pieChart>
   <c:varyColors val="1"/>
   <c:ser><c:idx val="0"/><c:order val="0"/>
    <c:tx>{tx}</c:tx>
    <c:cat>{cat}</c:cat>
    <c:val>{val}</c:val>
   </c:ser>
   <c:firstSliceAng val="0"/>
  </c:pieChart>"#,
        tx = str_ref("$D$1", &["Total"]),
        cat = str_ref("$A$2:$A$5", &CATEGORIES),
        val = num_ref("$D$2:$D$5", &TOTALS),
    );
    chart_space(CHART_TITLES[2], &group, true)
}

/// Wraps a chart-group (+ axes) block in the `c:chartSpace`/`c:chart`/`c:plotArea` skeleton,
/// with a title and (optionally) a right-hand legend.
fn chart_space(title: &str, plot_body: &str, legend: bool) -> String {
    let legend_xml = if legend {
        r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend>"#
    } else {
        ""
    };
    format!(
        r#"{DECL}
<c:chartSpace xmlns:c="{NS_CHART}" xmlns:a="{NS_A}" xmlns:r="{NS_REL}">
 <c:chart>
  {title}
  <c:autoTitleDeleted val="0"/>
  <c:plotArea>
   <c:layout/>
   {plot_body}
  </c:plotArea>
  {legend_xml}
  <c:plotVisOnly val="1"/>
  <c:dispBlanksAs val="gap"/>
 </c:chart>
</c:chartSpace>"#,
        title = title_block(title),
    )
}

/// A `<c:cat>/<c:val>` series with a name, an explicit solid-fill color, and cached values.
fn catval_series(
    idx: u32,
    name: &str,
    color_hex: &str,
    name_ref: &str,
    val_ref: &str,
    values: &[f64],
) -> String {
    format!(
        r#"<c:ser>
    <c:idx val="{idx}"/><c:order val="{idx}"/>
    <c:tx>{tx}</c:tx>
    <c:spPr><a:solidFill><a:srgbClr val="{color_hex}"/></a:solidFill></c:spPr>
    <c:cat>{cat}</c:cat>
    <c:val>{val}</c:val>
   </c:ser>"#,
        tx = str_ref(name_ref, &[name]),
        cat = str_ref("$A$2:$A$5", &CATEGORIES),
        val = num_ref(val_ref, values),
    )
}

/// A `c:strRef` (formula + `strCache`) over the `Data` sheet.
fn str_ref(range: &str, values: &[&str]) -> String {
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#, escape(v)))
        .collect();
    format!(
        r#"<c:strRef><c:f>Data!{range}</c:f><c:strCache><c:ptCount val="{n}"/>{pts}</c:strCache></c:strRef>"#,
        n = values.len(),
    )
}

/// A `c:numRef` (formula + `numCache`) over the `Data` sheet.
fn num_ref(range: &str, values: &[f64]) -> String {
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#, fmt_num(*v)))
        .collect();
    format!(
        r#"<c:numRef><c:f>Data!{range}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{n}"/>{pts}</c:numCache></c:numRef>"#,
        n = values.len(),
    )
}

fn cat_axis(title: &str) -> String {
    format!(
        r#"<c:catAx><c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/>{}<c:crossAx val="222222222"/></c:catAx>"#,
        title_block(title),
    )
}

fn val_axis(title: &str) -> String {
    format!(
        r#"<c:valAx><c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/>{}<c:crossAx val="111111111"/></c:valAx>"#,
        title_block(title),
    )
}

/// A `c:title` block with rich text (`a:t`) — the shape [`super::load::parse_title`] reads.
fn title_block(text: &str) -> String {
    format!(
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>"#,
        escape(text),
    )
}

/// Prints a value the way a `numCache` `<c:v>` should: whole numbers without a decimal point.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Minimal XML text escaping for the literal strings we embed.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::load::{load_charts_from_xlsx, parse_chart_xml};
    use freecell_chart_model::{BarDir, ChartKind, Grouping, SeriesData};

    #[test]
    fn each_authored_chart_part_parses_to_expected_kind() {
        for (part, body, expected_title) in [
            ("chart1", column_chart(), CHART_TITLES[0]),
            ("chart2", line_chart(), CHART_TITLES[1]),
            ("chart3", pie_chart(), CHART_TITLES[2]),
        ] {
            let chart =
                parse_chart_xml(&body).unwrap_or_else(|e| panic!("{part} failed to parse: {e:#}"));
            assert_eq!(chart.title.as_deref(), Some(expected_title), "{part} title");
        }
        assert!(matches!(
            parse_chart_xml(&column_chart()).unwrap().kind,
            ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered
            }
        ));
        assert!(matches!(
            parse_chart_xml(&line_chart()).unwrap().kind,
            ChartKind::Line { .. }
        ));
        assert_eq!(
            parse_chart_xml(&pie_chart()).unwrap().kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
    }

    #[test]
    fn written_fixture_loads_three_charts_with_cached_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("charts_basic.xlsx");
        let parts = write_fixture(&path).unwrap();
        assert_eq!(parts.len(), 3);

        let charts = load_charts_from_xlsx(&path).unwrap();
        assert_eq!(charts.len(), 3, "three embedded charts");

        // Column chart: two series, cached values as authored.
        let col = &charts[0];
        assert_eq!(col.title.as_deref(), Some(CHART_TITLES[0]));
        assert_eq!(col.series.len(), 2);
        assert_eq!(col.series[0].name.as_deref(), Some("Widgets"));
        match &col.series[0].data {
            SeriesData::CategoryValue { categories, values } => {
                let cats: Vec<String> = categories.iter().map(|c| c.label()).collect();
                assert_eq!(cats, CATEGORIES);
                assert_eq!(values, &WIDGETS.to_vec());
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }

        // Pie chart: single series over the totals.
        let pie = &charts[2];
        assert_eq!(
            pie.kind,
            ChartKind::Pie {
                doughnut_hole: None
            }
        );
        assert_eq!(pie.series.len(), 1);
        match &pie.series[0].data {
            SeriesData::CategoryValue { values, .. } => assert_eq!(values, &TOTALS.to_vec()),
            other => panic!("expected CategoryValue, got {other:?}"),
        }
    }

    /// The premise of the whole save path: IronCalc must be able to open the authored file.
    #[test]
    fn fixture_loads_in_ironcalc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("charts_basic.xlsx");
        write_fixture(&path).unwrap();
        let path_str = path.to_str().unwrap();
        let model = ironcalc::import::load_from_xlsx(path_str, "en", "UTC", "en")
            .expect("IronCalc opens the authored fixture");
        // Sanity: the single "Data" worksheet came through (chart parts are, as expected,
        // invisible to IronCalc — it has no chart model).
        assert_eq!(model.workbook.worksheets.len(), 1);
        assert_eq!(model.workbook.worksheets[0].name, "Data");
    }
}
