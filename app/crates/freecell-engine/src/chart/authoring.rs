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
use freecell_chart_model::{Anchor, AnchorCell};

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
    num_ref_for(&format!("Data!{range}"), values)
}

/// A `c:numRef` over an **arbitrary** `c:f` formula (e.g. a reference to a sheet that doesn't
/// exist — the P14 corpus's unresolved-`c:f` edge case) plus a `numCache` of `values`.
fn num_ref_for(formula: &str, values: &[f64]) -> String {
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#, fmt_num(*v)))
        .collect();
    format!(
        r#"<c:numRef><c:f>{formula}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{n}"/>{pts}</c:numCache></c:numRef>"#,
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

// ---------------------------------------------------------------------------------------------
// Single line-chart fixture (P7 — the real `.xlsx` the load path parses end-to-end)
// ---------------------------------------------------------------------------------------------

/// The single chart part in the line fixture.
pub const LINE_CHART_PART: &str = "xl/charts/chart1.xml";
/// The line fixture chart's title.
pub const LINE_CHART_TITLE: &str = "Sales Trend";
/// The line fixture chart's `twoCellAnchor` placement — the single source of truth shared by the
/// generated drawing XML ([`write_line_fixture`]) and the load-path assertions. Carries non-zero
/// EMU offsets so the offset fields are exercised, not just the cell indices.
pub const LINE_ANCHOR: Anchor = Anchor::new(
    AnchorCell::with_offsets(1, 12_700, 6, 0),
    AnchorCell::with_offsets(9, 0, 22, 6_350),
);

/// Writes a **single line-chart** `.xlsx` to `path` (creating parent dirs): a straight
/// (non-smooth) two-series line over the shared `Data` grid, anchored at [`LINE_ANCHOR`], whose
/// chart part additionally carries a `_rels` → `colors1.xml` + `style1.xml` aux chain — so the
/// load path's related-part retention (charts/architecture §3.2) is exercised end-to-end. Like
/// [`write_fixture`], it is a valid OPC package IronCalc accepts (asserted in tests).
pub fn write_line_fixture(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let parts: &[(&str, String)] = &[
        ("[Content_Types].xml", line_content_types()),
        ("_rels/.rels", root_rels()),
        ("xl/workbook.xml", workbook()),
        ("xl/_rels/workbook.xml.rels", workbook_rels()),
        ("xl/styles.xml", styles()),
        ("xl/sharedStrings.xml", shared_strings()),
        ("xl/worksheets/sheet1.xml", worksheet()),
        ("xl/worksheets/_rels/sheet1.xml.rels", worksheet_rels()),
        ("xl/drawings/drawing1.xml", line_drawing()),
        ("xl/drawings/_rels/drawing1.xml.rels", line_drawing_rels()),
        ("xl/charts/chart1.xml", line_fixture_chart()),
        ("xl/charts/_rels/chart1.xml.rels", line_chart_rels()),
        ("xl/charts/colors1.xml", chart_colors()),
        ("xl/charts/style1.xml", chart_style()),
    ];
    for (name, body) in parts {
        zw.start_file(*name, opts)
            .with_context(|| format!("starting zip entry {name}"))?;
        zw.write_all(body.as_bytes())
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    zw.finish().context("finishing line fixture zip")?;
    Ok(())
}

/// Content types for the line fixture — the base workbook parts plus the chart, chart-colors,
/// and chart-style part overrides (real Excel declares each; IronCalc ignores the chart parts).
fn line_content_types() -> String {
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
 <Override PartName="/xl/charts/colors1.xml" ContentType="application/vnd.ms-office.chartcolorstyle+xml"/>
 <Override PartName="/xl/charts/style1.xml" ContentType="application/vnd.ms-office.chartstyle+xml"/>
</Types>"#
    )
}

/// The line fixture's drawing: one `twoCellAnchor` graphic frame (at [`LINE_ANCHOR`]) referencing
/// `chart1.xml` via `r:id="rId1"`. Built from [`LINE_ANCHOR`] so the XML and the assertions
/// cannot drift.
fn line_drawing() -> String {
    let (f, t) = (LINE_ANCHOR.from, LINE_ANCHOR.to);
    format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
 <xdr:twoCellAnchor>
  <xdr:from><xdr:col>{fc}</xdr:col><xdr:colOff>{fco}</xdr:colOff><xdr:row>{fr}</xdr:row><xdr:rowOff>{fro}</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>{tc}</xdr:col><xdr:colOff>{tco}</xdr:colOff><xdr:row>{tr}</xdr:row><xdr:rowOff>{tro}</xdr:rowOff></xdr:to>
  <xdr:graphicFrame macro="">
   <xdr:nvGraphicFramePr>
    <xdr:cNvPr id="2" name="Line Chart 1"/>
    <xdr:cNvGraphicFramePr/>
   </xdr:nvGraphicFramePr>
   <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>
   <a:graphic><a:graphicData uri="{NS_CHART}">
     <c:chart xmlns:c="{NS_CHART}" xmlns:r="{NS_REL}" r:id="rId1"/>
   </a:graphicData></a:graphic>
  </xdr:graphicFrame>
  <xdr:clientData/>
 </xdr:twoCellAnchor>
</xdr:wsDr>"#,
        fc = f.col,
        fco = f.col_off_emu,
        fr = f.row,
        fro = f.row_off_emu,
        tc = t.col,
        tco = t.col_off_emu,
        tr = t.row,
        tro = t.row_off_emu,
    )
}

fn line_drawing_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/chart" Target="../charts/chart1.xml"/></Relationships>"#
    )
}

/// The line chart part: a straight (non-smooth) two-series line over the `Data` grid.
fn line_fixture_chart() -> String {
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
    chart_space(LINE_CHART_TITLE, &group, true)
}

/// The line-fixture chart part XML (two cat/val series over the `Data` grid) — exposed for the
/// live-binding (`chart::binding`) and worker-seam tests, which need the exact part
/// [`write_line_fixture`] embeds so their `c:f` role/range assertions can't drift from the file.
#[cfg(test)]
pub(crate) fn line_chart_xml_for_test() -> String {
    line_fixture_chart()
}

/// The chart part's `_rels` — the `chartStyle`/`chartColorStyle` aux parts every modern Excel
/// chart carries (retained byte-for-byte by the load path, never parsed by us).
fn line_chart_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2011/relationships/chartStyle" Target="style1.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2011/relationships/chartColorStyle" Target="colors1.xml"/></Relationships>"#
    )
}

/// A minimal `colorsN.xml` chart-color-style part (its `colorStyle` root is asserted in tests).
fn chart_colors() -> String {
    format!(
        r#"{DECL}<cs:colorStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" xmlns:a="{NS_A}" meth="cycle" id="10"><a:schemeClr val="accent1"/><a:schemeClr val="accent2"/><a:schemeClr val="accent3"/><cs:variation/></cs:colorStyle>"#
    )
}

/// A minimal `styleN.xml` chart-style part (a stub — we never parse it, only byte-preserve it).
fn chart_style() -> String {
    format!(
        r#"{DECL}<cs:chartStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" xmlns:a="{NS_A}" id="201"/>"#
    )
}

// ---------------------------------------------------------------------------------------------
// Line + unsupported-group fixture (P7 — the load layer must never break on one bad chart)
// ---------------------------------------------------------------------------------------------

/// The two chart parts in the mixed fixture, in drawing order: a parseable line, then a
/// `c:surfaceChart` our `parse_chart_xml` does not recognize.
pub const MIXED_CHART_PARTS: [&str; 2] = ["xl/charts/chart1.xml", "xl/charts/chart2.xml"];

/// Writes a single-sheet `.xlsx` with **two** charts — a parseable line (`chart1`) and an
/// **unparseable** `c:surfaceChart` (`chart2`) — to prove the load walk is per-chart non-fatal:
/// `discover_and_parse` skips the surface chart (logging it) and still returns the line chart,
/// rather than aborting the whole load (charts/architecture §6).
pub fn write_line_plus_unsupported_fixture(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let parts: &[(&str, String)] = &[
        ("[Content_Types].xml", mixed_content_types()),
        ("_rels/.rels", root_rels()),
        ("xl/workbook.xml", workbook()),
        ("xl/_rels/workbook.xml.rels", workbook_rels()),
        ("xl/styles.xml", styles()),
        ("xl/sharedStrings.xml", shared_strings()),
        ("xl/worksheets/sheet1.xml", worksheet()),
        ("xl/worksheets/_rels/sheet1.xml.rels", worksheet_rels()),
        ("xl/drawings/drawing1.xml", mixed_drawing()),
        ("xl/drawings/_rels/drawing1.xml.rels", mixed_drawing_rels()),
        ("xl/charts/chart1.xml", line_fixture_chart()),
        ("xl/charts/chart2.xml", unsupported_surface_chart()),
    ];
    for (name, body) in parts {
        zw.start_file(*name, opts)
            .with_context(|| format!("starting zip entry {name}"))?;
        zw.write_all(body.as_bytes())
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    zw.finish().context("finishing mixed fixture zip")?;
    Ok(())
}

fn mixed_content_types() -> String {
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
</Types>"#
    )
}

/// Two `twoCellAnchor` frames referencing `chart1`/`chart2` (reuses [`anchor`], as the 3-chart
/// fixture drawing does).
fn mixed_drawing() -> String {
    format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
 {a1}
 {a2}
</xdr:wsDr>"#,
        a1 = anchor(1, 1, 5, 10, 14),
        a2 = anchor(2, 12, 5, 21, 14),
    )
}

fn mixed_drawing_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/chart" Target="../charts/chart1.xml"/><Relationship Id="rId2" Type="{NS_REL}/chart" Target="../charts/chart2.xml"/></Relationships>"#
    )
}

/// A `c:surfaceChart` — a group our `parse_chart_xml` does not recognize, so parsing it fails
/// (and the load must skip it, not abort). Structurally a valid `c:chartSpace` otherwise.
fn unsupported_surface_chart() -> String {
    let group = format!(
        r#"<c:surfaceChart>
   <c:ser><c:idx val="0"/><c:order val="0"/><c:val>{val}</c:val></c:ser>
   <c:axId val="111111111"/><c:axId val="222222222"/><c:axId val="333333333"/>
  </c:surfaceChart>"#,
        val = num_ref("$B$2:$B$5", &WIDGETS),
    );
    chart_space("Terrain", &group, false)
}

// ---------------------------------------------------------------------------------------------
// Two-sheet fixture (P10 — the multi-sheet save part map + chart→SheetId grouping)
// ---------------------------------------------------------------------------------------------

/// The two worksheet names in the two-sheet fixture, in workbook order.
pub const TWO_SHEET_NAMES: [&str; 2] = ["Data", "Summary"];

/// The two chart parts in the two-sheet fixture: `chart1` anchored on **Data**, `chart2` anchored
/// on **Summary** (the association the save part map + grouped discovery must get right).
pub const TWO_SHEET_CHART_PARTS: [&str; 2] = ["xl/charts/chart1.xml", "xl/charts/chart2.xml"];

/// Writes a valid, IronCalc-openable **two-worksheet** `.xlsx` to `path`: sheet "Data" carries a
/// column chart, sheet "Summary" a line chart, each on its own `<drawing>`. Exercises the P10
/// multi-sheet save part map (sheet→drawing mapped by name across IronCalc's regenerated parts)
/// and the chart→owning-sheet grouping. Both charts read the `Data` grid's cached values (a chart's
/// data sheet is independent of the worksheet it is *anchored* on — what this fixture varies).
pub fn write_two_sheet_fixture(path: &Path) -> Result<()> {
    write_two_sheet(path, column_chart(), line_chart())
}

/// A two-sheet fixture whose two chart parts are **byte-identical** (both the column chart) — for
/// the save wrong-patch test: two twins bound to different sheets must each be patched with their
/// OWN live values, not the first XML match (charts/architecture §5).
pub fn write_two_sheet_twin_charts_fixture(path: &Path) -> Result<()> {
    write_two_sheet(path, column_chart(), column_chart())
}

/// A two-sheet fixture: a SUPPORTED column chart on "Data" (chart1) + an UNSUPPORTED surface chart
/// alone on "Summary" (chart2, which the loader skips → never bound). For the save no-silent-drop
/// test (architecture §6): editing Data and saving must **byte-preserve** Summary's unsupported
/// chart (best-effort carry, its host sheet survives), not drop it.
pub fn write_two_sheet_supported_plus_unsupported_fixture(path: &Path) -> Result<()> {
    write_two_sheet(path, column_chart(), unsupported_surface_chart())
}

/// Writes a valid two-worksheet workbook ("Data" + "Summary"), each with its own drawing anchoring
/// one chart part (`chart1_body` on Data, `chart2_body` on Summary).
fn write_two_sheet(path: &Path, chart1_body: String, chart2_body: String) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let parts: &[(&str, String)] = &[
        ("[Content_Types].xml", two_sheet_content_types()),
        ("_rels/.rels", root_rels()),
        ("xl/workbook.xml", two_sheet_workbook()),
        ("xl/_rels/workbook.xml.rels", two_sheet_workbook_rels()),
        ("xl/styles.xml", styles()),
        ("xl/sharedStrings.xml", shared_strings()),
        ("xl/worksheets/sheet1.xml", worksheet()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_drawing_rels(1)),
        ("xl/worksheets/sheet2.xml", summary_worksheet()),
        ("xl/worksheets/_rels/sheet2.xml.rels", sheet_drawing_rels(2)),
        ("xl/drawings/drawing1.xml", one_chart_drawing(5, 0, 15, 10)),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            one_chart_drawing_rels(1),
        ),
        ("xl/drawings/drawing2.xml", one_chart_drawing(5, 0, 15, 10)),
        (
            "xl/drawings/_rels/drawing2.xml.rels",
            one_chart_drawing_rels(2),
        ),
        ("xl/charts/chart1.xml", chart1_body),
        ("xl/charts/chart2.xml", chart2_body),
    ];
    for (name, body) in parts {
        zw.start_file(*name, opts)
            .with_context(|| format!("starting zip entry {name}"))?;
        zw.write_all(body.as_bytes())
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    zw.finish().context("finishing two-sheet fixture zip")?;
    Ok(())
}

fn two_sheet_content_types() -> String {
    format!(
        r#"{DECL}
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="xml" ContentType="application/xml"/>
 <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
 <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
 <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
 <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
 <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
 <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
 <Override PartName="/xl/drawings/drawing2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
 <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
 <Override PartName="/xl/charts/chart2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
</Types>"#
    )
}

fn two_sheet_workbook() -> String {
    format!(
        r#"{DECL}
<workbook xmlns="{NS_MAIN}" xmlns:r="{NS_REL}">
 <sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Summary" sheetId="2" r:id="rId2"/></sheets>
</workbook>"#
    )
}

fn two_sheet_workbook_rels() -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{NS_REL}/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="{NS_REL}/styles" Target="styles.xml"/><Relationship Id="rId4" Type="{NS_REL}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
    )
}

/// The "Summary" worksheet: a minimal grid (so IronCalc accepts it) plus a `<drawing r:id="rId1"/>`
/// anchoring its chart.
fn summary_worksheet() -> String {
    format!(
        r#"{DECL}
<worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_REL}"><dimension ref="A1"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData/><drawing r:id="rId1"/></worksheet>"#
    )
}

/// A worksheet `_rels` pointing its `rId1` `<drawing>` at `../drawings/drawing{n}.xml`.
fn sheet_drawing_rels(n: u32) -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/drawing" Target="../drawings/drawing{n}.xml"/></Relationships>"#
    )
}

/// A single-frame drawing whose one `twoCellAnchor` references the drawing's `rId1` chart.
fn one_chart_drawing(from_row: u32, from_col: u32, to_row: u32, to_col: u32) -> String {
    format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
 {frame}
</xdr:wsDr>"#,
        frame = anchor(1, from_row, from_col, to_row, to_col),
    )
}

/// A drawing `_rels` pointing its `rId1` chart at `../charts/chart{n}.xml`.
fn one_chart_drawing_rels(n: u32) -> String {
    format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/chart" Target="../charts/chart{n}.xml"/></Relationships>"#
    )
}

// ---------------------------------------------------------------------------------------------
// P14 robustness corpus — many chart types + edge cases in one openable workbook, plus the
// broken-drawing fixtures for the per-chart-resilient `discover` walk.
// ---------------------------------------------------------------------------------------------

/// The classification the loader should produce for one corpus chart (asserted by the P14 corpus
/// robustness test).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorpusExpect {
    /// Parses into a typed chart, Faithful (a supported group, or an edge case that still parses).
    Faithful,
    /// Parses into a **2-D** chart, Degraded (a 3-D source normalized to its 2-D equivalent).
    Degraded,
    /// Retained as an Unsupported placeholder — no typed chart, source kept, not dropped.
    Unsupported,
}

/// One chart in the corpus fixture: its package part, a human label, and the classification the
/// loader should produce for it.
#[derive(Clone, Debug)]
pub struct CorpusChart {
    pub part: String,
    pub label: &'static str,
    pub expect: CorpusExpect,
}

/// Writes a valid, IronCalc-openable single-sheet workbook to `path` whose one drawing anchors a
/// broad **corpus** of chart types + edge cases — every supported group (line/column/bar/area/pie/
/// doughnut/scatter), every 3-D group (→ degraded 2-D), every truly-unsupported group (surface/
/// radar/stock/ofPie/bubble), and parse edge cases (unresolved `c:f`, empty range, non-numeric
/// cell, a groupless chartSpace, a non-XML "garbage" part). Returns the manifest of expected
/// classifications, in the drawing's document order — the order `discover_and_parse` returns them.
pub fn write_corpus_fixture(path: &Path) -> Result<Vec<CorpusChart>> {
    use CorpusExpect::{Degraded, Faithful, Unsupported};

    // (label, chartSpace XML, expected classification) in drawing/document order.
    let entries: Vec<(&'static str, String, CorpusExpect)> = vec![
        // --- Supported groups → parse Faithful (they render via the P5/PoC widgets). ---
        (
            "line",
            catval_group_chart(
                "Line",
                "<c:lineChart><c:grouping val=\"standard\"/>",
                "</c:lineChart>",
            ),
            Faithful,
        ),
        (
            "column",
            catval_group_chart(
                "Column",
                "<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>",
                "</c:barChart>",
            ),
            Faithful,
        ),
        (
            "bar",
            catval_group_chart(
                "Bar",
                "<c:barChart><c:barDir val=\"bar\"/><c:grouping val=\"clustered\"/>",
                "</c:barChart>",
            ),
            Faithful,
        ),
        (
            "area",
            catval_group_chart(
                "Area",
                "<c:areaChart><c:grouping val=\"standard\"/>",
                "</c:areaChart>",
            ),
            Faithful,
        ),
        (
            "pie",
            catval_group_chart("Pie", "<c:pieChart>", "</c:pieChart>"),
            Faithful,
        ),
        (
            "doughnut",
            catval_group_chart(
                "Doughnut",
                "<c:doughnutChart><c:holeSize val=\"50\"/>",
                "</c:doughnutChart>",
            ),
            Faithful,
        ),
        (
            "scatter",
            xy_group_chart(
                "Scatter",
                "<c:scatterChart><c:scatterStyle val=\"lineMarker\"/>",
                "<c:axId val=\"1\"/><c:axId val=\"2\"/></c:scatterChart>",
            ),
            Faithful,
        ),
        // --- 3-D groups → normalized to 2-D, Degraded (retained, rendered 2-D + badge). ---
        (
            "bar3D",
            catval_group_chart(
                "Bar 3D",
                "<c:bar3DChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>",
                "</c:bar3DChart>",
            ),
            Degraded,
        ),
        (
            "line3D",
            catval_group_chart(
                "Line 3D",
                "<c:line3DChart><c:grouping val=\"standard\"/>",
                "</c:line3DChart>",
            ),
            Degraded,
        ),
        (
            "pie3D",
            catval_group_chart("Pie 3D", "<c:pie3DChart>", "</c:pie3DChart>"),
            Degraded,
        ),
        (
            "area3D",
            catval_group_chart(
                "Area 3D",
                "<c:area3DChart><c:grouping val=\"standard\"/>",
                "</c:area3DChart>",
            ),
            Degraded,
        ),
        // --- Truly-unsupported groups → retained Unsupported placeholder (not dropped). ---
        (
            "surface",
            catval_group_chart("Surface", "<c:surfaceChart>", "</c:surfaceChart>"),
            Unsupported,
        ),
        (
            "radar",
            catval_group_chart("Radar", "<c:radarChart>", "</c:radarChart>"),
            Unsupported,
        ),
        (
            "stock",
            catval_group_chart("Stock", "<c:stockChart>", "</c:stockChart>"),
            Unsupported,
        ),
        (
            "ofPie",
            catval_group_chart("Bar of Pie", "<c:ofPieChart>", "</c:ofPieChart>"),
            Unsupported,
        ),
        (
            "bubble",
            xy_group_chart("Bubble", "<c:bubbleChart>", "</c:bubbleChart>"),
            Unsupported,
        ),
        // --- Parse edge cases (functional_spec §7). ---
        ("unresolved_cf", unresolved_cf_line_chart(), Faithful),
        ("empty_range", empty_range_line_chart(), Faithful),
        ("nonnumeric", nonnumeric_line_chart(), Faithful),
        ("groupless", groupless_chart(), Unsupported),
        ("garbage", garbage_chart(), Unsupported),
    ];

    let xmls: Vec<String> = entries.iter().map(|(_, xml, _)| xml.clone()).collect();
    write_charts_fixture(path, &xmls)?;
    Ok(entries
        .into_iter()
        .enumerate()
        .map(|(i, (label, _, expect))| CorpusChart {
            part: format!("xl/charts/chart{}.xml", i + 1),
            label,
            expect,
        })
        .collect())
}

/// Writes a valid single-sheet workbook whose one `<drawing>` anchors `chart_xmls` (one graphic
/// frame per chart, `chart{i}.xml`). The generic corpus builder [`write_corpus_fixture`] drives it.
pub fn write_charts_fixture(path: &Path, chart_xmls: &[String]) -> Result<()> {
    let n = chart_xmls.len();
    let mut parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), corpus_content_types(n)),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), workbook()),
        ("xl/_rels/workbook.xml.rels".into(), workbook_rels()),
        ("xl/styles.xml".into(), styles()),
        ("xl/sharedStrings.xml".into(), shared_strings()),
        ("xl/worksheets/sheet1.xml".into(), worksheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            worksheet_rels(),
        ),
        ("xl/drawings/drawing1.xml".into(), corpus_drawing(n)),
        (
            "xl/drawings/_rels/drawing1.xml.rels".into(),
            corpus_drawing_rels(n),
        ),
    ];
    for (i, body) in chart_xmls.iter().enumerate() {
        parts.push((format!("xl/charts/chart{}.xml", i + 1), body.clone()));
    }
    write_package(path, &parts)
}

/// Writes a single-sheet workbook whose drawing references TWO charts (`rId1` line, `rId2` column)
/// but whose drawing `_rels` maps ONLY `rId1` — a **dangling** `<c:chart r:id="rId2">`. `discover`
/// must skip just that chart and still return the line chart (P14 per-chart-resilient walk).
pub fn write_dangling_chart_rel_fixture(path: &Path) -> Result<()> {
    let drawing = format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
 {a1}
 {a2}
</xdr:wsDr>"#,
        a1 = anchor(1, 1, 0, 10, 10),
        a2 = anchor(2, 12, 0, 21, 10),
    );
    // Only rId1 is present → rId2 (the column chart) is a dangling reference.
    let drawing_rels = format!(
        r#"{DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/chart" Target="../charts/chart1.xml"/></Relationships>"#
    );
    let parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), corpus_content_types(2)),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), workbook()),
        ("xl/_rels/workbook.xml.rels".into(), workbook_rels()),
        ("xl/styles.xml".into(), styles()),
        ("xl/sharedStrings.xml".into(), shared_strings()),
        ("xl/worksheets/sheet1.xml".into(), worksheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            worksheet_rels(),
        ),
        ("xl/drawings/drawing1.xml".into(), drawing),
        ("xl/drawings/_rels/drawing1.xml.rels".into(), drawing_rels),
        (
            "xl/charts/chart1.xml".into(),
            catval_group_chart(
                "Line",
                "<c:lineChart><c:grouping val=\"standard\"/>",
                "</c:lineChart>",
            ),
        ),
        (
            "xl/charts/chart2.xml".into(),
            catval_group_chart(
                "Column",
                "<c:barChart><c:barDir val=\"col\"/>",
                "</c:barChart>",
            ),
        ),
    ];
    write_package(path, &parts)
}

/// Writes a **two-sheet** workbook: sheet "Data" has a healthy line chart; sheet "Summary" has a
/// `<drawing>` whose `_rels` part is **entirely missing**. `discover` must drop the Summary drawing
/// (logged) and still return Data's line chart (P14 per-drawing-resilient walk).
pub fn write_missing_drawing_rels_fixture(path: &Path) -> Result<()> {
    let parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), two_sheet_content_types()),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), two_sheet_workbook()),
        (
            "xl/_rels/workbook.xml.rels".into(),
            two_sheet_workbook_rels(),
        ),
        ("xl/styles.xml".into(), styles()),
        ("xl/sharedStrings.xml".into(), shared_strings()),
        ("xl/worksheets/sheet1.xml".into(), worksheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            sheet_drawing_rels(1),
        ),
        ("xl/worksheets/sheet2.xml".into(), summary_worksheet()),
        (
            "xl/worksheets/_rels/sheet2.xml.rels".into(),
            sheet_drawing_rels(2),
        ),
        (
            "xl/drawings/drawing1.xml".into(),
            one_chart_drawing(5, 0, 15, 10),
        ),
        (
            "xl/drawings/_rels/drawing1.xml.rels".into(),
            one_chart_drawing_rels(1),
        ),
        // drawing2 exists but its `_rels` part is DELIBERATELY OMITTED.
        (
            "xl/drawings/drawing2.xml".into(),
            one_chart_drawing(5, 0, 15, 10),
        ),
        ("xl/charts/chart1.xml".into(), line_chart()),
        ("xl/charts/chart2.xml".into(), column_chart()),
    ];
    write_package(path, &parts)
}

/// Writes a single-sheet workbook with one line chart whose own chart XML is **valid** but whose
/// aux `_rels` (`xl/charts/_rels/chart1.xml.rels`) is **malformed** (not valid XML). The chart must
/// be RETAINED as an Unsupported placeholder (its chart XML + anchor + ranges kept, empty related
/// parts) rather than dropped — a broken secondary aux part never loses a chart (architecture §6).
pub fn write_bad_aux_rels_fixture(path: &Path) -> Result<()> {
    let line = chart_space(
        "Broken Aux",
        &format!(
            "<c:lineChart><c:grouping val=\"standard\"/>{ser}</c:lineChart>",
            ser = catval_series(0, "Series", WIDGETS_COLOR, "$B$1", "$B$2:$B$5", &WIDGETS),
        ),
        false,
    );
    let parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), corpus_content_types(1)),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), workbook()),
        ("xl/_rels/workbook.xml.rels".into(), workbook_rels()),
        ("xl/styles.xml".into(), styles()),
        ("xl/sharedStrings.xml".into(), shared_strings()),
        ("xl/worksheets/sheet1.xml".into(), worksheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            worksheet_rels(),
        ),
        ("xl/drawings/drawing1.xml".into(), corpus_drawing(1)),
        (
            "xl/drawings/_rels/drawing1.xml.rels".into(),
            corpus_drawing_rels(1),
        ),
        ("xl/charts/chart1.xml".into(), line),
        // A DELIBERATELY malformed chart aux `_rels` — not well-formed XML, so `parse_rels` fails.
        (
            "xl/charts/_rels/chart1.xml.rels".into(),
            "this is not valid rels XML <<<".into(),
        ),
    ];
    write_package(path, &parts)
}

/// Writes a **two-sheet** workbook: sheet "Data" has a healthy line chart; sheet "Summary" has a
/// `<drawing>` whose target drawing **part is entirely missing** (`drawing2.xml` is absent, though
/// the sheet `_rels` references it). `discover` must drop the Summary drawing (logged) and still
/// return Data's line chart (P14 per-drawing-resilient walk — the `discover` docstring's
/// missing-drawing-part claim). The content types declare only the parts that exist.
pub fn write_missing_drawing_part_fixture(path: &Path) -> Result<()> {
    let content_types = format!(
        r#"{DECL}
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="xml" ContentType="application/xml"/>
 <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
 <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
 <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
 <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
 <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
 <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
 <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
</Types>"#
    );
    let parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), content_types),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), two_sheet_workbook()),
        (
            "xl/_rels/workbook.xml.rels".into(),
            two_sheet_workbook_rels(),
        ),
        ("xl/styles.xml".into(), styles()),
        ("xl/sharedStrings.xml".into(), shared_strings()),
        ("xl/worksheets/sheet1.xml".into(), worksheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            sheet_drawing_rels(1),
        ),
        ("xl/worksheets/sheet2.xml".into(), summary_worksheet()),
        // Summary's `_rels` points at drawing2, but that drawing PART is DELIBERATELY OMITTED.
        (
            "xl/worksheets/_rels/sheet2.xml.rels".into(),
            sheet_drawing_rels(2),
        ),
        (
            "xl/drawings/drawing1.xml".into(),
            one_chart_drawing(5, 0, 15, 10),
        ),
        (
            "xl/drawings/_rels/drawing1.xml.rels".into(),
            one_chart_drawing_rels(1),
        ),
        ("xl/charts/chart1.xml".into(), line_chart()),
    ];
    write_package(path, &parts)
}

/// A `<c:barChart>`/`<c:lineChart>`/… chartSpace with one category/value series over the `Data`
/// grid — the corpus body for every category/value group. `group_open`/`group_close` frame the
/// chart-group element (with any group-level children like `barDir`).
fn catval_group_chart(title: &str, group_open: &str, group_close: &str) -> String {
    let group = format!(
        "{group_open}{ser}{group_close}",
        ser = catval_series(0, "Series", WIDGETS_COLOR, "$B$1", "$B$2:$B$5", &WIDGETS),
    );
    chart_space(title, &group, false)
}

/// A chartSpace with one xy series over the `Data` grid — the corpus body for scatter/bubble.
fn xy_group_chart(title: &str, group_open: &str, group_close: &str) -> String {
    let ser = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:xVal>{x}</c:xVal><c:yVal>{y}</c:yVal></c:ser>"#,
        x = num_ref("$B$2:$B$5", &WIDGETS),
        y = num_ref("$C$2:$C$5", &GADGETS),
    );
    chart_space(title, &format!("{group_open}{ser}{group_close}"), false)
}

/// A line chart whose value ref points at a sheet that does **not** exist (`Ghost!…`) but carries a
/// cache — it parses Faithful and renders/falls back to the cache (live binding can't resolve it).
fn unresolved_cf_line_chart() -> String {
    let ser = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:val>{val}</c:val></c:ser>"#,
        val = num_ref_for("Ghost!$B$2:$B$5", &WIDGETS),
    );
    chart_space(
        "Unresolved ref",
        &format!("<c:lineChart><c:grouping val=\"standard\"/>{ser}</c:lineChart>"),
        false,
    )
}

/// A line chart whose value cache is **empty** (`ptCount 0`, no points) — it must parse without a
/// crash (a zero-length series), functional_spec §7.
fn empty_range_line_chart() -> String {
    let ser = r#"<c:ser><c:idx val="0"/><c:order val="0"/>
       <c:val><c:numRef><c:f>Data!$B$2:$B$2</c:f><c:numCache><c:ptCount val="0"/></c:numCache></c:numRef></c:val></c:ser>"#;
    chart_space(
        "Empty range",
        &format!("<c:lineChart><c:grouping val=\"standard\"/>{ser}</c:lineChart>"),
        false,
    )
}

/// A line chart whose value cache holds a **non-numeric** cell — the unparseable point is dropped,
/// the rest parse, no crash (functional_spec §7).
fn nonnumeric_line_chart() -> String {
    let ser = r#"<c:ser><c:idx val="0"/><c:order val="0"/>
       <c:val><c:numRef><c:f>Data!$B$2:$B$3</c:f><c:numCache><c:ptCount val="2"/>
         <c:pt idx="0"><c:v>notanumber</c:v></c:pt><c:pt idx="1"><c:v>42</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#;
    chart_space(
        "Non-numeric",
        &format!("<c:lineChart><c:grouping val=\"standard\"/>{ser}</c:lineChart>"),
        false,
    )
}

/// A structurally-valid chartSpace with a title but **no chart-group element** — `parse_chart_xml`
/// finds no group → retained as an Unsupported placeholder (its title is salvaged).
fn groupless_chart() -> String {
    chart_space("Groupless", "", false)
}

/// A chart part that is **not valid XML at all** — `parse_chart_xml` fails to even parse the
/// document → retained as an Unsupported placeholder (no title salvaged). Its host workbook still
/// opens (IronCalc never reads chart parts).
fn garbage_chart() -> String {
    "This is deliberately not valid chart XML.".to_string()
}

/// The corpus `[Content_Types].xml`: the base workbook parts + one chart override per chart.
fn corpus_content_types(n_charts: usize) -> String {
    let mut chart_overrides = String::new();
    for i in 1..=n_charts {
        chart_overrides.push_str(&format!(
            r#"<Override PartName="/xl/charts/chart{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#
        ));
    }
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
 {chart_overrides}
</Types>"#
    )
}

/// The corpus drawing: `n_charts` `twoCellAnchor` graphic frames, staggered down the sheet, each
/// referencing `chart{i}.xml` via `rId{i}`.
fn corpus_drawing(n_charts: usize) -> String {
    let mut frames = String::new();
    for i in 1..=n_charts as u32 {
        let from_row = (i - 1) * 16;
        frames.push_str(&anchor(i, from_row + 1, 0, from_row + 15, 10));
        frames.push('\n');
    }
    format!(
        r#"{DECL}
<xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">
{frames}</xdr:wsDr>"#
    )
}

/// The corpus drawing `_rels`: one chart relationship (`rId{i}` → `chart{i}.xml`) per chart.
fn corpus_drawing_rels(n_charts: usize) -> String {
    let mut rels = String::new();
    for i in 1..=n_charts {
        rels.push_str(&format!(
            r#"<Relationship Id="rId{i}" Type="{NS_REL}/chart" Target="../charts/chart{i}.xml"/>"#
        ));
    }
    format!(r#"{DECL}<Relationships xmlns="{NS_PKG_REL}">{rels}</Relationships>"#)
}

/// Writes a package: each `(part name, body)` becomes a deflated zip entry. The shared zip-writing
/// tail for the corpus + broken-drawing fixtures.
fn write_package(path: &Path, parts: &[(String, String)]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in parts {
        zw.start_file(name.as_str(), opts)
            .with_context(|| format!("starting zip entry {name}"))?;
        zw.write_all(body.as_bytes())
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    zw.finish().context("finishing package zip")?;
    Ok(())
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

    /// The two-sheet fixture is a valid workbook IronCalc opens, with both named worksheets and a
    /// chart on each (the multi-sheet save/anchor coverage, P10).
    #[test]
    fn two_sheet_fixture_loads_in_ironcalc_with_both_charts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two_sheet.xlsx");
        write_two_sheet_fixture(&path).unwrap();

        let model =
            ironcalc::import::load_from_xlsx(path.to_str().unwrap(), "en", "UTC", "en").unwrap();
        let names: Vec<&str> = model
            .workbook
            .worksheets
            .iter()
            .map(|w| w.name.as_str())
            .collect();
        assert_eq!(names, TWO_SHEET_NAMES);

        // Both drawings/charts are discoverable (one per sheet).
        let charts = load_charts_from_xlsx(&path).unwrap();
        assert_eq!(charts.len(), 2);
        assert!(matches!(charts[0].kind, ChartKind::Bar { .. }));
        assert!(matches!(charts[1].kind, ChartKind::Line { .. }));
    }

    /// The line fixture (with its chart `_rels` + colors/style aux parts) is also a valid
    /// workbook IronCalc opens — the extra chart machinery does not break the package.
    #[test]
    fn line_fixture_loads_in_ironcalc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_chart.xlsx");
        write_line_fixture(&path).unwrap();
        let path_str = path.to_str().unwrap();
        let model = ironcalc::import::load_from_xlsx(path_str, "en", "UTC", "en")
            .expect("IronCalc opens the line fixture");
        assert_eq!(model.workbook.worksheets.len(), 1);
        assert_eq!(model.workbook.worksheets[0].name, "Data");
    }
}
