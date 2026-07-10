//! **Display fidelity** — the derived accessor answering "how faithfully can we draw this
//! chart?" as one of [`Fidelity::Faithful`] / [`Fidelity::Degraded`] / [`Fidelity::Unsupported`]
//! (charts/functional_spec §5, architecture §3.3).
//!
//! It is **derived, not stored state**: there is no parse-time flag to keep in sync. The
//! category is computed on demand from the model plus a chart's **retained source XML**
//! ([`ChartSpec::display_fidelity`](crate::ChartSpec::display_fidelity)), so it **auto-clears
//! as renderer support lands** — once a feature becomes rendered it drops out of the curated
//! set below and the `Degraded` warning disappears with no separate bookkeeping.
//!
//! The classifier scans the source **textually** (namespace-prefix-agnostic local-name
//! matching, like the PoC parser), rather than building a DOM: the model crate stays
//! dependency-light, and this matches the "engine re-parses / patches the retained source
//! textually" note on [`SourceXml`](crate::SourceXml).
//!
//! ## What each bucket means (functional_spec §5)
//! Evaluated against the source in this precedence:
//! 1. **Unsupported** — a chart-group type with no faithful 2-D rendering in our set
//!    (`surfaceChart` / `surface3DChart` / `radarChart` / `ofPieChart` / `stockChart`, or the
//!    `cx:` *extended* family). The chart falls back to the placeholder (ui_design §2.3).
//! 2. **Degraded (3-D → 2-D)** — a 3-D chart-group (`bar3DChart` / `line3DChart` /
//!    `pie3DChart` / `area3DChart`) rendered as its 2-D equivalent (see
//!    [`normalize_3d_chart_group`]); the retained source still names the 3-D element.
//! 3. **Degraded (unsupported render-affecting feature)** — the source contains a **curated**
//!    render-affecting feature the renderer does not yet honor (see below).
//! 4. **Faithful** — otherwise.
//!
//! ## The curated "render-affecting unsupported" set
//! Sourced from `experiments/chart-poc/ooxml-coverage-matrix.md`: the renderer's honored
//! baseline (the matrix **OK** rows) is Faithful; a render-affecting feature the matrix marks as
//! not-yet-supported degrades the chart. The set is deliberately curated to fire **only when a
//! feature is actually active** — benign/default forms (a `General` number format, a `minMax`
//! orientation, an all-zero `dLbls`) must **not** raise a false warning (architecture §3.3).
//! Excluded on the same grounds: the `scaling` wrapper (always present), gridline toggles (we
//! draw gridlines anyway), `varyColors` (matches our palette), `gapWidth`/`overlap`/`firstSliceAng`
//! (written at defaults on nearly every bar/pie), and `schemeClr` (a theme reference we now
//! resolve to a color, P6).
//!
//! **Auto-dropped / scoped as support arrives.** `smooth` (curved lines) **renders faithfully as
//! of P6** (on `lineChart`, the only group that draws it), so it left this set. `c:marker` symbols
//! are now **scoped to the renderer that honors them**: P6's line renderer paints every symbol, so
//! a marker on a `lineChart` is Faithful — but the scatter/point renderers still draw a fixed
//! circle and ignore `c:marker` (their marker support is a later phase), so a non-`circle`/non-`none`
//! symbol on a **non-line** group still degrades (a wrong chart must keep its badge). `c:numFmt`
//! remains — P6 applies a **bounded** subset to ticks, so a format code we don't parse must still
//! warn; P12 completes numFmt and shrinks this further. Entries auto-drop / re-scope with no
//! separate bookkeeping (architecture §3.3).

/// How faithfully the renderer can draw a chart, derived from its model + retained source
/// (charts/functional_spec §5, architecture §3.3). Consumed by the render/UI layer (P8):
/// [`Faithful`](Fidelity::Faithful) and [`Degraded`](Fidelity::Degraded) both draw the chart
/// (Degraded adds the corner "⚠ May not display as intended" badge, ui_design §2.2);
/// [`Unsupported`](Fidelity::Unsupported) draws the placeholder (ui_design §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// Renders exactly as authored — everything present is either honored or benign.
    Faithful,
    /// Renders, but not exactly as authored: a 3-D type shown as its 2-D equivalent, or a
    /// render-affecting feature we don't yet honor. Draws the chart **plus** the compatibility
    /// badge.
    Degraded,
    /// No faithful rendering (an unsupported chart-group type, or a part that failed to parse).
    /// Falls back to the placeholder.
    Unsupported,
}

impl Fidelity {
    /// Whether the chart draws as itself (both [`Faithful`](Fidelity::Faithful) and
    /// [`Degraded`](Fidelity::Degraded)) rather than falling back to the placeholder
    /// ([`Unsupported`](Fidelity::Unsupported)).
    pub fn renders_as_chart(self) -> bool {
        !matches!(self, Fidelity::Unsupported)
    }

    /// Whether to show the corner "⚠ May not display as intended" compatibility warning
    /// (ui_design §2.2) — [`Degraded`](Fidelity::Degraded) only.
    pub fn shows_compatibility_warning(self) -> bool {
        matches!(self, Fidelity::Degraded)
    }
}

/// The 3-D chart-group elements that degrade to a 2-D equivalent (see
/// [`normalize_3d_chart_group`]). Kept consistent with the normalizer by a unit test.
const CHART_GROUPS_3D: &[&str] = &["bar3DChart", "line3DChart", "pie3DChart", "area3DChart"];

/// Classic `c:` chart-group elements with **no faithful 2-D rendering** in our supported set
/// → [`Fidelity::Unsupported`] (the chart shows the placeholder). The `cx:` extended family is
/// detected separately by its namespace (see [`is_extended_chart`]).
const UNSUPPORTED_CHART_GROUPS: &[&str] = &[
    "surfaceChart",
    "surface3DChart",
    "radarChart",
    "ofPieChart",
    "stockChart",
];

/// Render-affecting OOXML features the renderer does not yet honor whose **mere presence**
/// (namespace-prefix-agnostic) means the chart won't draw as authored — each is written only
/// when actually used, so presence alone is a safe trigger. Sourced from
/// `experiments/chart-poc/ooxml-coverage-matrix.md` sections C/D:
/// - `c:dPt` — per-point / per-slice color/style override (P1 for pie).
/// - `a:gradFill` / `a:pattFill` — non-solid fills we don't render. Detected **anywhere** in
///   the part (series/point fill *or* a themed chart-/plot-area background). This is the
///   deliberate asymmetry with excluding `schemeClr`: a `gradFill`/`pattFill` element is written
///   only where a non-solid fill is *actually used* — a reliable "we can't draw this" signal —
///   whereas `schemeClr` is a pervasive solid-color reference throughout text/chrome that we
///   *do* resolve to a color, so its presence is not by itself a fidelity loss.
/// - `c:min` / `c:max` — explicit axis-scaling bounds (written only when set).
///
/// The value-aware features still tracked (`numFmt`, `orientation`, `dLbls` toggles, and the
/// **line-scoped** marker check) need their value/context inspected and are handled by dedicated
/// detectors below. `c:smooth` left the set entirely in P6 (rendered on line, and line is the only
/// group that draws it); `c:marker` is now **scoped** — Faithful on a `lineChart` (P6 renders every
/// symbol) but still degrading on a non-line group (see [`unsupported_marker`]).
const RENDER_AFFECTING_PRESENCE_MARKERS: &[&str] = &["dPt", "gradFill", "pattFill", "min", "max"];

/// Map a **3-D** chart-group element local-name to its **2-D** equivalent element local-name
/// (charts/functional_spec §5): `bar3DChart→barChart`, `line3DChart→lineChart`,
/// `pie3DChart→pieChart`, `area3DChart→areaChart`. Returns `None` for anything else.
///
/// The 3-D → 2-D reduction (architecture §3.3): the P7 parser treats a 3-D group as its 2-D
/// name when building the [`ChartKind`](crate::ChartKind), while the retained source still names
/// the 3-D element — so [`source_fidelity`] reports [`Fidelity::Degraded`]. Types with **no**
/// 2-D equivalent in our set (surface / radar / ofPie / stock / `cx:`) are
/// [`Fidelity::Unsupported`], not normalized.
///
/// ```
/// # use freecell_chart_model::normalize_3d_chart_group;
/// assert_eq!(normalize_3d_chart_group("bar3DChart"), Some("barChart"));
/// assert_eq!(normalize_3d_chart_group("surfaceChart"), None);
/// ```
pub fn normalize_3d_chart_group(local_name: &str) -> Option<&'static str> {
    match local_name {
        "bar3DChart" => Some("barChart"),
        "line3DChart" => Some("lineChart"),
        "pie3DChart" => Some("pieChart"),
        "area3DChart" => Some("areaChart"),
        _ => None,
    }
}

/// Classify a chart's retained source XML into a [`Fidelity`] (charts/functional_spec §5,
/// architecture §3.3). Pure over the chart part's text; see the module docs for the buckets,
/// precedence, and the curated render-affecting set.
///
/// The classifier keys off the **source**, not the parsed
/// [`ChartKind`](crate::ChartKind) — [`Fidelity::Unsupported`] types (surface / radar / …) have
/// no `ChartKind` variant to key off, and 3-D degradation must be detected even though the
/// model already holds the normalized 2-D kind.
pub fn source_fidelity(chart_xml: &str) -> Fidelity {
    if is_unsupported_chart(chart_xml) {
        return Fidelity::Unsupported;
    }
    if has_3d_chart_group(chart_xml) {
        return Fidelity::Degraded;
    }
    if has_render_affecting_unsupported_feature(chart_xml) {
        return Fidelity::Degraded;
    }
    Fidelity::Faithful
}

fn is_unsupported_chart(xml: &str) -> bool {
    is_extended_chart(xml)
        || UNSUPPORTED_CHART_GROUPS
            .iter()
            .any(|group| contains_element(xml, group))
}

fn has_3d_chart_group(xml: &str) -> bool {
    CHART_GROUPS_3D
        .iter()
        .any(|group| contains_element(xml, group))
}

fn has_render_affecting_unsupported_feature(xml: &str) -> bool {
    RENDER_AFFECTING_PRESENCE_MARKERS
        .iter()
        .any(|marker| contains_element(xml, marker))
        || axis_reversed(xml)
        || custom_number_format(xml)
        || data_labels_shown(xml)
        || unsupported_marker(xml)
}

/// The `cx:` extended-chart family (sunburst, treemap, waterfall, histogram, box-&-whisker,
/// funnel, region map): a different schema (`.../2014/chartex`) we don't render. The retained
/// part declares that namespace, so its URI fragment is a reliable marker.
fn is_extended_chart(xml: &str) -> bool {
    xml.contains("chartex")
}

/// `c:orientation val="maxMin"` — a reversed axis we don't honor. The default `minMax` is
/// benign and present on essentially every axis.
fn axis_reversed(xml: &str) -> bool {
    any_opening_tag(xml, "orientation", |attrs| {
        attr_value(attrs, "val") == Some("maxMin")
    })
}

/// A `c:numFmt` with a `formatCode` that is neither empty nor `General` — a number format we
/// don't yet apply (P12). `formatCode="General"` (the pervasive default) is benign.
fn custom_number_format(xml: &str) -> bool {
    any_opening_tag(xml, "numFmt", |attrs| {
        match attr_value(attrs, "formatCode") {
            Some(code) => !code.is_empty() && !code.eq_ignore_ascii_case("General"),
            None => false,
        }
    })
}

/// A `c:dLbls` that actually shows something — any `show*` toggle set true. An all-zero
/// `dLbls` (labels present in XML but all off) is benign.
fn data_labels_shown(xml: &str) -> bool {
    const SHOW_TOGGLES: &[&str] = &[
        "showVal",
        "showPercent",
        "showCatName",
        "showSerName",
        "showBubbleSize",
        "showLegendKey",
    ];
    SHOW_TOGGLES
        .iter()
        .any(|toggle| any_opening_tag(xml, toggle, val_is_true))
}

/// A `c:marker` with a `c:symbol` shape only the **line** renderer draws. P6's line renderer paints
/// every OOXML marker symbol, so a marker on a `lineChart` is Faithful; but the scatter/point
/// renderers still draw a fixed `circle` and ignore `c:marker` (their marker support is a later
/// phase), so a non-line chart-group carrying a non-`circle`/non-`none` symbol still renders wrong →
/// Degraded. `circle`/`none` are exactly what the fixed renderer draws (or nothing), so they are
/// Faithful anywhere. (A `line3DChart` is *not* a `lineChart` — it degrades as a 3-D group first,
/// which takes precedence in [`source_fidelity`], so the scoping here only ever sees the 2-D line.)
///
/// **Caveat (combo parts):** this classifier is textual, not a DOM, so it can't bind a `<c:marker>`
/// to its *enclosing* chart-group. In a **combo** part that holds both a `<c:lineChart>` and another
/// group (e.g. `<c:scatterChart>`), [`is_line_chart`] is true, so a non-circle marker on the
/// *non-line* series is currently classified Faithful (its advisory badge is dropped) — revisit when
/// P7 lands real multi-group parsing that can associate a marker with its group.
fn unsupported_marker(xml: &str) -> bool {
    if is_line_chart(xml) {
        return false;
    }
    any_opening_tag(
        xml,
        "symbol",
        |attrs| matches!(attr_value(attrs, "val"), Some(val) if val != "none" && val != "circle"),
    )
}

/// Whether the source contains a 2-D `c:lineChart` group — the one group whose renderer honors the
/// full `c:marker` symbol set (and `c:smooth`). Boundary-aware, so it does **not** match
/// `line3DChart`.
///
/// Same textual-classifier caveat as [`unsupported_marker`]: in a combo part with a `lineChart`
/// plus another group this returns true for the whole part. `c:smooth` is likewise unscoped (no
/// per-group binding), but that is harmless today — the non-line renderers draw no connecting line
/// for `smooth` to curve, so an unhonored `smooth` on a non-line series changes nothing.
fn is_line_chart(xml: &str) -> bool {
    contains_element(xml, "lineChart")
}

/// A truthy OOXML boolean `val` attribute (`1` or `true`).
fn val_is_true(attrs: &str) -> bool {
    matches!(attr_value(attrs, "val"), Some("1") | Some("true"))
}

/// Whether `xml` contains an **opening** tag whose namespace-prefix-agnostic local name is
/// `local_name` (e.g. `local_name = "surfaceChart"` matches `<c:surfaceChart …>`,
/// `<surfaceChart/>`, or `<x:surfaceChart>`, but not the closing `</c:surfaceChart>` nor a
/// longer name like `surface3DChart`).
fn contains_element(xml: &str, local_name: &str) -> bool {
    any_opening_tag(xml, local_name, |_| true)
}

/// Core scanner: for every **opening** tag in `xml` whose prefix-agnostic local name is
/// `local_name`, call `pred` with that tag's attribute text (everything between the local name
/// and the tag-closing `>` / `/>`); return true on the first `pred` that returns true.
///
/// Tag-boundary-aware so short/benign names don't false-match: the char **after** the name must
/// end a tag name (whitespace / `>` / `/`), and the text **before** it must open an element
/// (`<` or `<prefix:`) — which also excludes closing tags (`</…>`) and names embedded in a
/// longer name or an attribute value.
fn any_opening_tag<F: Fn(&str) -> bool>(xml: &str, local_name: &str, pred: F) -> bool {
    let mut from = 0;
    while let Some(rel) = xml[from..].find(local_name) {
        let start = from + rel;
        let end = start + local_name.len();
        from = end;

        let after_ends_name = matches!(
            xml[end..].chars().next(),
            Some(c) if c == '>' || c == '/' || c.is_whitespace()
        );
        if !after_ends_name || !opens_tag_name(&xml[..start]) {
            continue;
        }

        let attrs_end = end + tag_close_offset(&xml[end..]);
        if pred(&xml[end..attrs_end]) {
            return true;
        }
    }
    false
}

/// Byte offset of the `>` that closes the current opening tag, **quote-aware**: a `>` inside a
/// quoted attribute value does not end the tag. `>` is legal unescaped in an attribute value and
/// occurs in conditional number formats (e.g. `formatCode="[Red][>1000]#,##0"`), so a naive
/// `find('>')` would truncate the attribute text and miss it. Returns `s.len()` if no closing
/// `>` is found (malformed / truncated source).
fn tag_close_offset(s: &str) -> usize {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return i,
                _ => {}
            },
        }
    }
    s.len()
}

/// Whether the text immediately preceding an element local name opens a start tag: either a
/// bare `<`, or a `<prefix:` where `prefix` is a valid NCName. (A `</` closing tag returns
/// false — the prefix run stops at the `/`, which is not a `<`.)
fn opens_tag_name(before: &str) -> bool {
    if before.ends_with('<') {
        return true;
    }
    let Some(rest) = before.strip_suffix(':') else {
        return false;
    };
    // The trailing run of NCName chars is the prefix. Those chars are ASCII (1 byte each), so
    // the char count is also the byte length of the run — a valid slice boundary.
    let prefix_len = rest
        .chars()
        .rev()
        .take_while(|&c| is_ncname_char(c))
        .count();
    prefix_len > 0 && rest[..rest.len() - prefix_len].ends_with('<')
}

/// The characters allowed in an XML NCName prefix (a pragmatic ASCII subset — enough to accept
/// real namespace prefixes like `c`, `a`, `cx`, `mc`).
fn is_ncname_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
}

/// The value of `attr="…"` (or `attr='…'`) within an element's attribute text, if present.
/// Matches `attr` only as a whole attribute name (bounded by whitespace / start on the left and
/// optional whitespace then `=` on the right), so it is not fooled by a name embedded in
/// another (`val` is not matched inside `interval`).
fn attr_value<'a>(attrs: &'a str, attr: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(rel) = attrs[from..].find(attr) {
        let start = from + rel;
        let end = start + attr.len();
        from = end;

        let preceded_ok = start == 0
            || attrs[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if !preceded_ok {
            continue;
        }

        let rest = attrs[end..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => continue,
        };
        let value = &rest[quote.len_utf8()..];
        return value.find(quote).map(|end| &value[..end]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Chart-group classification --------------------------------------------------------

    #[test]
    fn supported_group_sources_are_faithful() {
        // Each of the six honored 2-D classic groups, alone, is Faithful.
        for group in [
            "<c:barChart><c:barDir val=\"col\"/></c:barChart>",
            "<c:lineChart/>",
            "<c:areaChart/>",
            "<c:pieChart/>",
            "<c:doughnutChart><c:holeSize val=\"50\"/></c:doughnutChart>",
            "<c:scatterChart/>",
        ] {
            assert_eq!(
                source_fidelity(group),
                Fidelity::Faithful,
                "expected Faithful for {group}"
            );
        }
    }

    #[test]
    fn benign_fields_do_not_degrade() {
        // A realistic supported line chart carrying only honored + benign/default fields must
        // stay Faithful — the false-positive guard from architecture §3.3.
        let xml = r#"
            <c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
              <c:chart>
                <c:plotArea>
                  <c:lineChart>
                    <c:varyColors val="0"/>
                    <c:ser>
                      <c:idx val="0"/><c:order val="0"/>
                      <c:marker><c:symbol val="none"/></c:marker>
                      <c:smooth val="0"/>
                      <c:dLbls>
                        <c:showLegendKey val="0"/><c:showVal val="0"/>
                        <c:showCatName val="0"/><c:showSerName val="0"/>
                        <c:showPercent val="0"/><c:showBubbleSize val="0"/>
                      </c:dLbls>
                    </c:ser>
                  </c:lineChart>
                  <c:catAx>
                    <c:scaling><c:orientation val="minMax"/></c:scaling>
                    <c:majorGridlines/>
                    <c:numFmt formatCode="General" sourceLinked="1"/>
                    <c:tickLblPos val="nextTo"/>
                  </c:catAx>
                  <c:valAx>
                    <c:scaling><c:orientation val="minMax"/></c:scaling>
                  </c:valAx>
                </c:plotArea>
                <c:legend><c:legendPos val="r"/></c:legend>
              </c:chart>
            </c:chartSpace>
        "#;
        assert_eq!(source_fidelity(xml), Fidelity::Faithful);
    }

    #[test]
    fn unsupported_groups_are_unsupported() {
        for group in [
            "<c:surfaceChart/>",
            "<c:surface3DChart/>",
            "<c:radarChart/>",
            "<c:ofPieChart/>",
            "<c:stockChart/>",
        ] {
            assert_eq!(
                source_fidelity(group),
                Fidelity::Unsupported,
                "expected Unsupported for {group}"
            );
        }
    }

    #[test]
    fn extended_cx_family_is_unsupported() {
        let xml = r#"<cx:chartSpace
            xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex">
            <cx:chart><cx:plotArea><cx:series layoutId="waterfall"/></cx:plotArea></cx:chart>
        </cx:chartSpace>"#;
        assert_eq!(source_fidelity(xml), Fidelity::Unsupported);
    }

    #[test]
    fn three_d_groups_are_degraded() {
        for group in ["bar3DChart", "line3DChart", "pie3DChart", "area3DChart"] {
            let xml = format!("<c:{group}/>");
            assert_eq!(
                source_fidelity(&xml),
                Fidelity::Degraded,
                "expected Degraded for {group}"
            );
        }
    }

    #[test]
    fn unsupported_precedes_degraded() {
        // A surface chart that also carries a degrading feature is still Unsupported
        // (placeholder wins over the badge).
        let xml = "<c:surfaceChart><c:ser><c:dPt/></c:ser></c:surfaceChart>";
        assert_eq!(source_fidelity(xml), Fidelity::Unsupported);
    }

    // --- 3-D → 2-D normalization -----------------------------------------------------------

    #[test]
    fn normalize_3d_chart_group_maps_each_3d_to_2d() {
        assert_eq!(normalize_3d_chart_group("bar3DChart"), Some("barChart"));
        assert_eq!(normalize_3d_chart_group("line3DChart"), Some("lineChart"));
        assert_eq!(normalize_3d_chart_group("pie3DChart"), Some("pieChart"));
        assert_eq!(normalize_3d_chart_group("area3DChart"), Some("areaChart"));
    }

    #[test]
    fn normalize_3d_returns_none_for_non_3d() {
        for name in ["barChart", "lineChart", "surfaceChart", "radarChart", ""] {
            assert_eq!(normalize_3d_chart_group(name), None, "for {name}");
        }
    }

    #[test]
    fn chart_groups_3d_const_matches_normalizer() {
        // Every constant used for 3-D detection normalizes to a 2-D name, and each 2-D target
        // is one of our supported groups — the detection set and the reduction table agree.
        for group in CHART_GROUPS_3D {
            let two_d = normalize_3d_chart_group(group)
                .unwrap_or_else(|| panic!("{group} in CHART_GROUPS_3D but has no 2-D mapping"));
            assert!(
                ["barChart", "lineChart", "pieChart", "areaChart"].contains(&two_d),
                "{group} normalized to unexpected {two_d}"
            );
        }
    }

    // --- Curated render-affecting features --------------------------------------------------

    #[test]
    fn active_render_affecting_features_degrade() {
        let cases = [
            (
                "dPt (per-point color)",
                "<c:ser><c:dPt><c:idx val=\"0\"/></c:dPt></c:ser>",
            ),
            (
                "gradient fill",
                "<c:spPr><a:gradFill><a:gsLst/></a:gradFill></c:spPr>",
            ),
            (
                "pattern fill",
                "<c:spPr><a:pattFill prst=\"pct5\"/></c:spPr>",
            ),
            (
                "explicit max",
                "<c:valAx><c:scaling><c:max val=\"100\"/></c:scaling></c:valAx>",
            ),
            (
                "explicit min",
                "<c:valAx><c:scaling><c:min val=\"0\"/></c:scaling></c:valAx>",
            ),
            (
                "reversed axis",
                "<c:catAx><c:scaling><c:orientation val=\"maxMin\"/></c:scaling></c:catAx>",
            ),
            (
                "custom numFmt",
                "<c:valAx><c:numFmt formatCode=\"0.00%\" sourceLinked=\"0\"/></c:valAx>",
            ),
            (
                // A conditional format whose value contains an unescaped `>` — exercises the
                // quote-aware tag scan (a naive `find('>')` would truncate the attribute and
                // miss the custom format, falsely reporting Faithful).
                "conditional numFmt with '>' in value",
                "<c:valAx><c:numFmt formatCode=\"[Red][>1000]#,##0\" sourceLinked=\"0\"/></c:valAx>",
            ),
            (
                "shown data label",
                "<c:dLbls><c:showVal val=\"1\"/></c:dLbls>",
            ),
        ];
        for (label, xml) in cases {
            let framed = format!("<c:lineChart>{xml}</c:lineChart>");
            assert_eq!(
                source_fidelity(&framed),
                Fidelity::Degraded,
                "expected Degraded for {label}"
            );
        }
    }

    #[test]
    fn benign_feature_forms_do_not_degrade() {
        // The default/off form of each still-tracked value-aware feature must NOT degrade.
        let cases = [
            (
                "general numFmt",
                "<c:valAx><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/></c:valAx>",
            ),
            (
                "minMax orientation",
                "<c:catAx><c:scaling><c:orientation val=\"minMax\"/></c:scaling></c:catAx>",
            ),
            (
                "all-off dLbls",
                "<c:dLbls><c:showVal val=\"0\"/><c:showPercent val=\"0\"/></c:dLbls>",
            ),
        ];
        for (label, xml) in cases {
            let framed = format!("<c:lineChart>{xml}</c:lineChart>");
            assert_eq!(
                source_fidelity(&framed),
                Fidelity::Faithful,
                "expected Faithful for {label}"
            );
        }
    }

    #[test]
    fn now_rendered_features_are_faithful() {
        // P6's LINE renderer draws `c:smooth` (curved line) and every `c:marker` symbol, so on a
        // `lineChart` their presence no longer degrades — the accessor auto-drops a feature once the
        // renderer honors it (architecture §3.3). `val="0"`/`none` were always Faithful; the point
        // here is that the *active* forms on a line chart are now Faithful too.
        for (label, xml) in [
            ("smooth on", "<c:ser><c:smooth val=\"1\"/></c:ser>"),
            (
                "square marker",
                "<c:ser><c:marker><c:symbol val=\"square\"/></c:marker></c:ser>",
            ),
            (
                "diamond marker",
                "<c:ser><c:marker><c:symbol val=\"diamond\"/></c:marker></c:ser>",
            ),
            (
                "star marker",
                "<c:ser><c:marker><c:symbol val=\"star\"/></c:marker></c:ser>",
            ),
        ] {
            let framed = format!("<c:lineChart>{xml}</c:lineChart>");
            assert_eq!(
                source_fidelity(&framed),
                Fidelity::Faithful,
                "expected Faithful for {label}"
            );
        }
    }

    #[test]
    fn markers_are_scoped_to_the_line_renderer() {
        // The marker fidelity is SCOPED: only the line renderer paints the full symbol set. The
        // scatter/point renderers still draw a fixed circle and ignore `c:marker`, so a non-circle
        // marker on a non-line group must STILL degrade (no false-Faithful — the wrong chart keeps
        // its badge), while the same marker on a line chart is Faithful.
        let diamond = "<c:ser><c:marker><c:symbol val=\"diamond\"/></c:marker></c:ser>";

        assert_eq!(
            source_fidelity(&format!("<c:lineChart>{diamond}</c:lineChart>")),
            Fidelity::Faithful,
            "line chart renders the diamond marker → Faithful"
        );
        assert_eq!(
            source_fidelity(&format!("<c:scatterChart>{diamond}</c:scatterChart>")),
            Fidelity::Degraded,
            "scatter ignores the marker (draws a circle) → Degraded"
        );
        assert_eq!(
            source_fidelity(&format!("<c:barChart>{diamond}</c:barChart>")),
            Fidelity::Degraded,
            "a non-line group with a non-circle marker → Degraded"
        );

        // circle / none are what the fixed renderer draws (or nothing) → Faithful anywhere.
        for symbol in ["circle", "none"] {
            let marker =
                format!("<c:ser><c:marker><c:symbol val=\"{symbol}\"/></c:marker></c:ser>");
            assert_eq!(
                source_fidelity(&format!("<c:scatterChart>{marker}</c:scatterChart>")),
                Fidelity::Faithful,
                "scatter + {symbol} marker → Faithful"
            );
        }
    }

    #[test]
    fn realistic_scatter_with_circle_markers_is_faithful() {
        // Scatter is a first-class SUPPORTED type and real scatter series carry a round-dot
        // marker (`<c:symbol val="circle"/>`); it must NOT be badged (architecture §3.3).
        let xml = r#"
            <c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
              <c:chart><c:plotArea>
                <c:scatterChart>
                  <c:scatterStyle val="lineMarker"/>
                  <c:ser>
                    <c:idx val="0"/><c:order val="0"/>
                    <c:marker><c:symbol val="circle"/><c:size val="5"/></c:marker>
                    <c:xVal><c:numRef><c:f>Data!$A$2:$A$5</c:f></c:numRef></c:xVal>
                    <c:yVal><c:numRef><c:f>Data!$B$2:$B$5</c:f></c:numRef></c:yVal>
                  </c:ser>
                  <c:axId val="1"/><c:axId val="2"/>
                </c:scatterChart>
                <c:valAx><c:scaling><c:orientation val="minMax"/></c:scaling></c:valAx>
                <c:valAx><c:scaling><c:orientation val="minMax"/></c:scaling></c:valAx>
              </c:plotArea></c:chart>
            </c:chartSpace>
        "#;
        assert_eq!(source_fidelity(xml), Fidelity::Faithful);
    }

    // --- Textual helper edge cases ---------------------------------------------------------

    #[test]
    fn contains_element_is_prefix_agnostic() {
        assert!(contains_element("<c:barChart/>", "barChart"));
        assert!(contains_element("<barChart/>", "barChart"));
        assert!(contains_element("<foo:barChart>", "barChart"));
        assert!(contains_element("<c:barChart attr=\"x\">", "barChart"));
    }

    #[test]
    fn contains_element_respects_tag_boundaries() {
        // `min` must not match inside `minorGridlines`; `surfaceChart` must not match inside
        // `surface3DChart`; a name inside an attribute value must not match.
        assert!(!contains_element("<c:minorGridlines/>", "min"));
        assert!(!contains_element("<c:surface3DChart/>", "surfaceChart"));
        assert!(!contains_element("<c:orientation val=\"maxMin\"/>", "max"));
        assert!(contains_element("<c:min val=\"0\"/>", "min"));
        assert!(contains_element("<c:max val=\"9\"/>", "max"));
    }

    #[test]
    fn contains_element_ignores_closing_tags() {
        assert!(!contains_element("</c:barChart>", "barChart"));
        // An open+close pair is still detected (via the opening tag).
        assert!(contains_element("<c:barChart></c:barChart>", "barChart"));
    }

    #[test]
    fn attr_value_reads_whole_attribute_only() {
        assert_eq!(attr_value(" val=\"1\"", "val"), Some("1"));
        assert_eq!(attr_value(" val='circle'", "val"), Some("circle"));
        assert_eq!(
            attr_value(" formatCode=\"0.00\" sourceLinked=\"0\"", "formatCode"),
            Some("0.00")
        );
        // `val` must not be read out of `interval`.
        assert_eq!(attr_value(" interval=\"2\"", "val"), None);
        assert_eq!(attr_value("majorTickMark=\"out\"", "val"), None);
    }

    #[test]
    fn tag_close_offset_is_quote_aware() {
        // A `>` inside a quoted attribute value does not close the tag; the first *unquoted*
        // `>` does. So the whole conditional-format value is preserved for `attr_value`.
        let attrs = " formatCode=\"[Red][>1000]#,##0\" sourceLinked=\"0\"/>rest";
        let end = tag_close_offset(attrs);
        assert_eq!(&attrs[end..], ">rest");
        assert_eq!(
            attr_value(&attrs[..end], "formatCode"),
            Some("[Red][>1000]#,##0")
        );
        // No closing `>` at all → whole string is the tag body.
        assert_eq!(tag_close_offset(" val=\"1\""), " val=\"1\"".len());
    }

    // --- Fidelity predicates ---------------------------------------------------------------

    #[test]
    fn fidelity_predicates_match_the_ui_contract() {
        assert!(Fidelity::Faithful.renders_as_chart());
        assert!(Fidelity::Degraded.renders_as_chart());
        assert!(!Fidelity::Unsupported.renders_as_chart());

        assert!(!Fidelity::Faithful.shows_compatibility_warning());
        assert!(Fidelity::Degraded.shows_compatibility_warning());
        assert!(!Fidelity::Unsupported.shows_compatibility_warning());
    }
}
