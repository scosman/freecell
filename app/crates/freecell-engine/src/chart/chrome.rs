//! **Chrome serializers** — the prefix-aware element builders for the five editable chrome
//! attributes (charts/functional_spec §6.B, implementation_plan P20): chart **title**, **legend**,
//! **axis title**, series **color**, and series **data labels**.
//!
//! These are the single source of truth shared by the two write modes that emit chrome:
//! - the **loaded** source-patch ([`super::save::patch_chart_source`]) splices these into a
//!   retained `chartN.xml`, passing the **file's** own namespace prefixes so the patch keeps the
//!   file's exact `c:` / `a:` spelling (and the rest of the chart stays byte-stable);
//! - the **authored** write-from-model serializer ([`super::write`]) emits `c:dLbls` through
//!   [`dlbls_element`] (its title/legend/axis/color builders predate P20 and stay put).
//!
//! Every element here mirrors a loader read shape ([`super::load`]) so it round-trips: a title's
//! rich run reads back through `parse_title`, a `dLbls`' toggles through `read_data_labels`, a
//! series `solidFill` through `parse_series_color`, etc. All functions are **pure** — model + string
//! only, no I/O — so they unit-test headless.

use freecell_chart_model::{Color, DataLabels, LegendPosition};

use super::save::escape_xml;

/// XML attribute-value escaping (adds `"` on top of the element-text set).
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// The `c:legendPos@val` token for a [`LegendPosition`].
pub(super) fn legend_pos_val(p: LegendPosition) -> &'static str {
    match p {
        LegendPosition::Right => "r",
        LegendPosition::Bottom => "b",
        LegendPosition::Left => "l",
        LegendPosition::Top => "t",
        LegendPosition::TopRight => "tr",
    }
}

/// The `<{c}tx>…</{c}tx>` text holder for a title — a single rich run (the shape
/// [`parse_title`](super::load) reads). Replacing **only** this holder inside an existing
/// `<c:title>` keeps that title's own `c:spPr`/`c:txPr`/`c:overlay`/`c:layout` styling intact.
pub(super) fn title_tx(c: &str, a: &str, text: &str) -> String {
    format!(
        "<{c}tx><{c}rich><{a}bodyPr/><{a}lstStyle/><{a}p><{a}r><{a}t>{}</{a}t></{a}r></{a}p></{c}rich></{c}tx>",
        escape_xml(text),
    )
}

/// A full `<{c}title>…</{c}title>` element (chart title **or** axis title — same shape) with a
/// single rich run and a non-overlaid box.
pub(super) fn title_element(c: &str, a: &str, text: &str) -> String {
    format!(
        "<{c}title>{}<{c}overlay val=\"0\"/></{c}title>",
        title_tx(c, a, text),
    )
}

/// A `<{c}legend>` element at `position` (`c:legendPos` + a non-overlaid box).
pub(super) fn legend_element(c: &str, position: LegendPosition) -> String {
    format!(
        "<{c}legend><{c}legendPos val=\"{}\"/><{c}overlay val=\"0\"/></{c}legend>",
        legend_pos_val(position),
    )
}

/// A `<{a}solidFill>` element for an sRGB color — the fill element the loader's
/// `parse_series_color` reads out of a series `c:spPr`.
pub(super) fn sppr_solid_fill(a: &str, color: Color) -> String {
    format!(
        "<{a}solidFill><{a}srgbClr val=\"{:06X}\"/></{a}solidFill>",
        color.to_hex(),
    )
}

/// A full `<{c}spPr>` element carrying just a solid fill — the whole shape-properties element for a
/// series that had none. When the series already has a `spPr`, the patcher upserts only the fill
/// **inside** it (via [`sppr_solid_fill`]) so a co-located `a:ln` stroke survives.
pub(super) fn series_sppr_element(c: &str, a: &str, color: Color) -> String {
    format!("<{c}spPr>{}</{c}spPr>", sppr_solid_fill(a, color))
}

/// A `<{c}dLbls>` element for `labels`, in `CT_DLbls` child order (`numFmt`, `dLblPos`, then the
/// `show*` flags, then `separator`) so strict readers (Excel) accept it. Round-trips through the
/// loader's `read_data_labels` (which is order-agnostic). Emitted by both the authored serializer
/// (`c:` prefix) and the loaded chrome patch (the file's prefix).
pub(super) fn dlbls_element(c: &str, labels: &DataLabels) -> String {
    let mut s = format!("<{c}dLbls>");
    if let Some(code) = &labels.number_format {
        s.push_str(&format!(
            "<{c}numFmt formatCode=\"{}\" sourceLinked=\"0\"/>",
            attr_escape(code),
        ));
    }
    if let Some(pos) = labels.position {
        s.push_str(&format!("<{c}dLblPos val=\"{}\"/>", dlbl_pos_val(pos)));
    }
    let flag = |name: &str, on: bool| format!("<{c}{name} val=\"{}\"/>", if on { 1 } else { 0 });
    s.push_str(&flag("showLegendKey", labels.show_legend_key));
    s.push_str(&flag("showVal", labels.show_value));
    s.push_str(&flag("showCatName", labels.show_category_name));
    s.push_str(&flag("showSerName", labels.show_series_name));
    s.push_str(&flag("showPercent", labels.show_percent));
    if let Some(sep) = &labels.separator {
        s.push_str(&format!("<{c}separator>{}</{c}separator>", escape_xml(sep)));
    }
    s.push_str(&format!("</{c}dLbls>"));
    s
}

/// The `c:dLblPos@val` token for a [`DataLabelPosition`](freecell_chart_model::DataLabelPosition).
fn dlbl_pos_val(p: freecell_chart_model::DataLabelPosition) -> &'static str {
    use freecell_chart_model::DataLabelPosition::*;
    match p {
        Center => "ctr",
        Left => "l",
        Right => "r",
        Above => "t",
        Below => "b",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::load::parse_chart_xml;
    use freecell_chart_model::{
        Axis, Category, Chart, ChartColor, ChartKind, DataLabelPosition, Grouping, Legend, Series,
    };

    /// Wrap a chart-group + chrome fragment in a minimal `c:chartSpace` so the loader can parse it,
    /// exercising a serializer's output inside a real (line) chart.
    fn chart_with(title: &str, legend: &str, series_extra: &str, dlbls: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart>{title}<c:plotArea><c:layout/><c:lineChart><c:grouping val="standard"/><c:ser><c:idx val="0"/><c:order val="0"/>{series_extra}<c:cat><c:strRef><c:f>Sheet1!$A$1:$A$2</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$1:$B$2</c:f><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val>{dlbls}</c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:crossAx val="1"/></c:valAx></c:plotArea>{legend}</c:chart></c:chartSpace>"#
        )
    }

    #[test]
    fn title_and_legend_round_trip() {
        let xml = chart_with(
            &title_element("c:", "a:", "My & Chart"),
            &legend_element("c:", LegendPosition::Bottom),
            "",
            "",
        );
        let chart = parse_chart_xml(&xml).expect("well-formed");
        assert_eq!(chart.title.as_deref(), Some("My & Chart"));
        assert_eq!(
            chart.legend,
            Some(Legend {
                position: LegendPosition::Bottom
            })
        );
    }

    #[test]
    fn series_color_and_data_labels_round_trip() {
        let color = Color::from_hex(0x4472C4);
        let labels = DataLabels::new()
            .value()
            .percent()
            .with_number_format("0.0%")
            .at(DataLabelPosition::Above);
        let xml = chart_with(
            "",
            "",
            &series_sppr_element("c:", "a:", color),
            &dlbls_element("c:", &labels),
        );
        let chart = parse_chart_xml(&xml).expect("well-formed");
        assert_eq!(
            chart.series[0].color,
            Some(ChartColor::Rgb(color)),
            "series solidFill round-trips",
        );
        let dl = chart.series[0]
            .data_labels
            .clone()
            .expect("data labels present");
        assert!(dl.show_value && dl.show_percent);
        assert!(!dl.show_category_name);
        assert_eq!(dl.number_format.as_deref(), Some("0.0%"));
        assert_eq!(dl.position, Some(DataLabelPosition::Above));
    }

    /// The serializers keep whatever `c:`/`a:` prefixes they are handed — a loaded patch passes the
    /// file's prefixes, so a file that spells the chart namespace `x:` and drawingml `d:` still gets
    /// consistent, re-parseable output.
    #[test]
    fn prefixes_are_honored() {
        let doc = format!(
            r#"<?xml version="1.0"?><x:chartSpace xmlns:x="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main"><x:chart>{title}<x:plotArea><x:layout/><x:lineChart><x:grouping val="standard"/><x:ser><x:idx val="0"/><x:order val="0"/><x:cat><x:strRef><x:f>S!$A$1</x:f><x:strCache><x:ptCount val="1"/><x:pt idx="0"><x:v>Q</x:v></x:pt></x:strCache></x:strRef></x:cat><x:val><x:numRef><x:f>S!$B$1</x:f><x:numCache><x:ptCount val="1"/><x:pt idx="0"><x:v>1</x:v></x:pt></x:numCache></x:numRef></x:val></x:ser><x:axId val="1"/><x:axId val="2"/></x:lineChart><x:catAx><x:axId val="1"/><x:crossAx val="2"/></x:catAx><x:valAx><x:axId val="2"/><x:crossAx val="1"/></x:valAx></x:plotArea></x:chart></x:chartSpace>"#,
            title = title_element("x:", "d:", "Prefixed"),
        );
        let chart = parse_chart_xml(&doc).expect("prefixed doc parses");
        assert_eq!(chart.title.as_deref(), Some("Prefixed"));
    }

    /// A quick guard that the shared builders match what a hand-built model expects (used by the
    /// worker's `apply_chrome_edit`).
    #[test]
    fn built_chart_matches_model() {
        let chart = Chart {
            title: Some("t".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("s"),
                vec![Category::Text("Q".into())],
                vec![1.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        assert_eq!(chart.title.as_deref(), Some("t"));
    }
}
