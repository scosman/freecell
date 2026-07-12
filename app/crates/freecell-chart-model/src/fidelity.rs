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
//! Excluded on the same grounds: the `scaling` wrapper (always present), the **major**-gridline
//! toggle (`c:majorGridlines` — the line renderer honors it, P13, so its presence *or absence*
//! renders as authored; **minor** gridlines are a different story — see below), `varyColors` /
//! `firstSliceAng` / `holeSize` / `explosion` (**honored by the P24 pie/doughnut renderer** — varied
//! slice colors, rotation, the doughnut annulus, and exploded slices all render as authored),
//! `gapWidth`/`overlap` (**honored by the P22 bar renderer**, so any value renders as authored), and
//! `schemeClr` (a theme reference we now resolve to a color, P6).
//!
//! **Auto-dropped / scoped as support arrives.** `smooth` (curved lines) **renders faithfully as
//! of P6** (on `lineChart`, the only group that draws it), so it left this set. `c:marker` symbols
//! and `c:dLbls` data labels are now **scoped to the renderer that honors them**: P6's line renderer
//! and **P25's scatter renderer** paint every marker symbol (they share `paint_marker`), and P12's
//! line draws data labels (value / percent / names / legend key), so a marker on a `lineChart` **or**
//! `scatterChart` — and a shown label on a `lineChart` — is Faithful; but the bar/area/pie renderers
//! still ignore markers (and bar/area/scatter still ignore labels), so a non-`circle`/non-`none`
//! marker on **those** groups, or a shown label on a non-line group, still degrades (a wrong chart
//! must keep its badge). A **smoothed** scatter (`c:scatterStyle val="smooth"`/`"smoothMarker"`)
//! renders as **straight** segments → Degraded ([`unsupported_scatter_smooth`], P25), the honest
//! badge for the straight-segment fallback. A **3-D bubble** (`c:bubble3D val="1"`) renders as flat
//! 2-D circles → Degraded ([`unsupported_bubble_3d`], P26); `c:sizeRepresents` (area/width) is
//! honored, so it stays Faithful. A **per-point** `c:dLbl` override degrades on any group (we draw
//! uniform series labels, not per-point ones). `c:numFmt` is now **scoped to codes we approximate**:
//! P6/P12 apply a **bounded** subset to ticks + labels, so a code we render exactly (`General`,
//! percent, thousands, decimals, currency) is Faithful and only a code we fall back on (dates /
//! scientific / fractions / multi-section / conditional — see
//! [`renders_faithfully`](crate::numfmt::renders_faithfully)) still warns. Entries auto-drop /
//! re-scope with no separate bookkeeping (architecture §3.3).
//!
//! **Partially-honored features degrade on their *un*-rendered variants (P13).** The line renderer
//! draws a **plain solid** `a:ln` (width / color / alpha) and **major** gridlines, but not every
//! variant, so the un-rendered ones keep their honest badge rather than silently misleading
//! (functional_spec §5): a non-solid line stroke — a preset/custom dash or a compound line — renders
//! as a solid line ([`unsupported_line_stroke`]), and an authored `c:minorGridlines` renders without
//! its minor lines ([`unsupported_minor_gridlines`]). Both are line-scoped, like the scaling/marker/
//! label features above.

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
/// - `a:gradFill` / `a:pattFill` — non-solid fills we don't render. Detected **anywhere** in
///   the part (series/point fill *or* a themed chart-/plot-area background). This is the
///   deliberate asymmetry with excluding `schemeClr`: a `gradFill`/`pattFill` element is written
///   only where a non-solid fill is *actually used* — a reliable "we can't draw this" signal —
///   whereas `schemeClr` is a pervasive solid-color reference throughout text/chrome that we
///   *do* resolve to a color, so its presence is not by itself a fidelity loss.
///
/// The value-aware / context-scoped features (`numFmt` codes we approximate, axis `scaling`
/// min/max/reversed line-scoped, `dLbls` toggles line-/pie-scoped, the line-scoped marker check, the
/// pie-scoped `c:dPt` check, and per-point `dLbl` overrides) need their value/context inspected and
/// are handled by dedicated detectors below. `c:smooth` left the set entirely in P6 (rendered on
/// line, and line is the only group that draws it); `c:marker` and shown `c:dLbls` are **scoped** —
/// Faithful on a `lineChart` (P6 renders every symbol, P12 draws labels) but still degrading on a
/// non-line group (see [`unsupported_marker`] / [`unsupported_data_labels`]). `c:dPt` per-slice
/// overrides left this presence set in **P24**: the pie/doughnut renderer now honors them (per-slice
/// fill color + explosion), so they are pie-scoped in [`unsupported_data_point`]. `c:min`/`c:max` and
/// the reversed `orientation` left the presence set in **P13**: the line renderer now honors axis
/// scaling, so they are line-scoped in [`unsupported_axis_scaling`].
const RENDER_AFFECTING_PRESENCE_MARKERS: &[&str] = &["gradFill", "pattFill"];

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
        || unsupported_axis_scaling(xml)
        || unsupported_minor_gridlines(xml)
        || unsupported_line_stroke(xml)
        || unsupported_number_format(xml)
        || unsupported_data_labels(xml)
        || unsupported_data_point(xml)
        || unsupported_marker(xml)
        || unsupported_scatter_smooth(xml)
        || unsupported_bubble_3d(xml)
}

/// A **3-D** bubble the renderer draws flat (P26). The bubble renderer draws flat 2-D circles, so a
/// `c:bubble3D val="1"`/`"true"` renders without its 3-D shading → Degraded (an honest badge, not a
/// silent flatten). Excel writes `<c:bubble3D val="0"/>` for a normal 2-D bubble (group-level and
/// per-series), which is benign; only the truthy form degrades. `c:sizeRepresents` (area/width) is
/// **honored** by the renderer, so it never degrades.
fn unsupported_bubble_3d(xml: &str) -> bool {
    is_bubble_chart(xml) && any_opening_tag(xml, "bubble3D", val_is_true)
}

/// A **smoothed** scatter the renderer draws straight (P25). The scatter renderer approximates
/// `c:scatterStyle val="smooth"`/`"smoothMarker"` with **straight** connecting segments, so a smoothed
/// scatter renders as its straight twin → Degraded (an honest badge, not a silent curve-to-straight).
/// The `marker`/`line`/`lineMarker` styles render exactly → Faithful.
fn unsupported_scatter_smooth(xml: &str) -> bool {
    is_scatter_chart(xml)
        && any_opening_tag(xml, "scatterStyle", |attrs| {
            matches!(
                attr_value(attrs, "val"),
                Some("smooth") | Some("smoothMarker")
            )
        })
}

/// A `c:dPt` per-slice / per-point override the renderer does not honor (P24). The pie/doughnut
/// renderer draws `c:dPt` (per-slice fill color + explosion), so a `dPt` on a pie/doughnut is
/// Faithful; on any other group (bar/line/area/scatter — their per-point styling is a later phase) a
/// `dPt` still renders wrong → Degraded. Scoped exactly like the P6 marker / P12 label checks.
fn unsupported_data_point(xml: &str) -> bool {
    contains_element(xml, "dPt") && !is_pie_chart(xml)
}

/// The `cx:` extended-chart family (sunburst, treemap, waterfall, histogram, box-&-whisker,
/// funnel, region map): a different schema (`.../2014/chartex`) we don't render. The retained
/// part declares that namespace, so its URI fragment is a reliable marker.
fn is_extended_chart(xml: &str) -> bool {
    xml.contains("chartex")
}

/// Axis `c:scaling` features the renderer honors only on a **line** chart (P13): explicit
/// `c:min`/`c:max` bounds and a reversed `c:orientation val="maxMin"`. On a `lineChart` the renderer
/// applies all three, so they are Faithful; on a group whose renderer ignores scaling
/// (bar/area/pie/scatter — their phases are P16+) any of them still renders wrong → Degraded. The
/// default `minMax` orientation and an absent min/max are benign (present on essentially every
/// axis), so only the *active* forms are inspected. Same textual-classifier combo caveat as
/// [`unsupported_marker`]: a combo part holding a `lineChart` treats scaling on the other group as
/// Faithful (revisit with real multi-group parsing).
fn unsupported_axis_scaling(xml: &str) -> bool {
    if is_line_chart(xml) {
        return false;
    }
    axis_reversed(xml) || contains_element(xml, "min") || contains_element(xml, "max")
}

/// `c:minorGridlines` on a **line** chart (P13). The line renderer draws only **major** gridlines,
/// so an authored minor-gridline set renders without its lines → Degraded. Major-gridline on/off
/// stays Faithful (honored). Line-scoped like the other P13 axis features; `c:minorGridlines` is
/// written only when authored (Excel omits it by default), so its presence is a safe active-trigger.
fn unsupported_minor_gridlines(xml: &str) -> bool {
    is_line_chart(xml) && contains_element(xml, "minorGridlines")
}

/// A series line stroke style the renderer does not draw (P13). The line renderer paints a **plain
/// solid** `a:ln` (honoring width / color / alpha), so an *active* non-solid stroke sub-feature
/// renders as a solid line and silently misleads → Degraded (functional_spec §5). Detected:
/// - a preset dash (`a:prstDash` `val` != `solid`) or a custom dash (`a:custDash`) — dashed / dotted
///   lines we draw solid (the misleading forecast/target-line case);
/// - a compound / multi-line stroke (`a:ln@cmpd` != `sng`) — double / thick-thin lines we draw as one.
///
/// **Deliberately *not* degrading:** plain **joins** (`a:round` / `a:bevel` / `a:miter`) and line
/// **caps**. `<a:round/>` is the pervasive default join Excel writes on nearly every series `a:ln`
/// (a benign default like the always-present `scaling` wrapper), and caps are imperceptible on a thin
/// stroke, so treating them as degrading would false-badge real files.
///
/// Line-scoped: only the line renderer claims to draw `a:ln`. Same textual-classifier caveat as
/// [`unsupported_marker`]: a `prstDash`/`cmpd` can't be bound to its owning element, so a dashed
/// *gridline* `a:ln` in a line part also degrades — acceptable (a dashed *series* line is the case
/// that matters, and we'd rather badge than risk a silent miss).
fn unsupported_line_stroke(xml: &str) -> bool {
    if !is_line_chart(xml) {
        return false;
    }
    // A preset dash other than solid (an absent `val` is malformed → treated as the benign solid).
    any_opening_tag(xml, "prstDash", |attrs| {
        !matches!(attr_value(attrs, "val"), None | Some("solid"))
    })
        // A custom dash pattern.
        || contains_element(xml, "custDash")
        // A compound (multi-line) stroke — `cmpd` on the `a:ln` other than single.
        || any_opening_tag(xml, "ln", |attrs| {
            !matches!(attr_value(attrs, "cmpd"), None | Some("sng"))
        })
}

/// `c:orientation val="maxMin"` — a reversed axis. The default `minMax` is benign and present on
/// essentially every axis.
fn axis_reversed(xml: &str) -> bool {
    any_opening_tag(xml, "orientation", |attrs| {
        attr_value(attrs, "val") == Some("maxMin")
    })
}

/// A `c:numFmt` whose `formatCode` the applier does not render **exactly** (see
/// [`renders_faithfully`](crate::numfmt::renders_faithfully)) — a date / scientific / fraction /
/// multi-section / conditional code we approximate with general formatting. As of P12 the applier
/// drives both axis ticks (P6) and data labels, so a code it renders exactly (`General`, percent,
/// thousands, decimals, currency) is Faithful and no longer degrades; `formatCode="General"` (the
/// pervasive default) is benign.
fn unsupported_number_format(xml: &str) -> bool {
    any_opening_tag(xml, "numFmt", |attrs| {
        match attr_value(attrs, "formatCode") {
            Some(code) => !crate::numfmt::renders_faithfully(code),
            None => false,
        }
    })
}

/// Data-label features the renderer does not draw as authored. Cases that degrade:
/// - a **per-point** label override (`<c:dLbl>` — an idx-keyed custom text / position / deletion)
///   on **any** group: we draw uniform series labels, not per-point overrides. Boundary-matched so
///   it does **not** fire on the plural `<c:dLbls>` container.
/// - **shown** series/chart-level labels (`data_labels_shown`) on a group whose renderer ignores
///   them (bar/area/scatter — their label phases are later).
///
/// **Scoped to the renderer that honors each label kind** (like the P6 marker scoping): P12's
/// **line** renderer draws every label kind → any shown label on a `lineChart` is Faithful; P24's
/// **pie/doughnut** renderer draws on-slice **percent** labels → a pie showing *only* percent is
/// Faithful, but a pie showing any *non-percent* kind (value / category-name / series-name /
/// legend-key / bubble-size) still renders wrong → Degraded. Same textual-classifier combo caveat as
/// [`unsupported_marker`]: in a combo part holding a `lineChart`/`pieChart` plus another group, a
/// shown label on the other group is currently treated as the scoped group's (revisit with real
/// multi-group parsing).
fn unsupported_data_labels(xml: &str) -> bool {
    if contains_element(xml, "dLbl") {
        return true;
    }
    if is_line_chart(xml) {
        return false;
    }
    if is_pie_chart(xml) {
        return pie_shows_non_percent_label(xml);
    }
    data_labels_shown(xml)
}

/// A pie/doughnut showing a label kind the pie renderer does not draw (P24). The renderer draws
/// on-slice **percent** labels (`c:showPercent`), so `showPercent` alone is Faithful; any of the
/// non-percent `show*` toggles set true still renders wrong.
fn pie_shows_non_percent_label(xml: &str) -> bool {
    const NON_PERCENT: &[&str] = &[
        "showVal",
        "showCatName",
        "showSerName",
        "showLegendKey",
        "showBubbleSize",
    ];
    NON_PERCENT
        .iter()
        .any(|toggle| any_opening_tag(xml, toggle, val_is_true))
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

/// A `c:marker` with a `c:symbol` shape the renderer does not draw. P6's line renderer and P25's
/// scatter renderer paint **every** OOXML marker symbol (they share `paint_marker`), so a marker on a
/// `lineChart` **or** a `scatterChart` is Faithful; but any other chart-group (bar/area/pie) still
/// ignores `c:marker`, so a non-`circle`/non-`none` symbol on those still renders wrong → Degraded.
/// `circle`/`none` are exactly what the fixed renderers draw (or nothing), so they are Faithful
/// anywhere. (A `line3DChart` is *not* a `lineChart` — it degrades as a 3-D group first, which takes
/// precedence in [`source_fidelity`], so the scoping here only ever sees the 2-D line.)
///
/// **Caveat (combo parts):** this classifier is textual, not a DOM, so it can't bind a `<c:marker>`
/// to its *enclosing* chart-group. In a **combo** part that holds a marker-honoring group
/// (`<c:lineChart>`/`<c:scatterChart>`) plus another, a non-circle marker on the *other* group is
/// currently classified Faithful (its advisory badge is dropped) — revisit when real multi-group
/// parsing can associate a marker with its group.
fn unsupported_marker(xml: &str) -> bool {
    if is_line_chart(xml) || is_scatter_chart(xml) {
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

/// Whether the source contains a 2-D `c:pieChart` or `c:doughnutChart` group — the groups whose
/// renderer honors `c:dPt` per-slice overrides + on-slice percent labels (P24). Boundary-aware, so
/// it does **not** match `pie3DChart` (which degrades as a 3-D group first) or `ofPieChart` (an
/// Unsupported group, detected earlier).
fn is_pie_chart(xml: &str) -> bool {
    contains_element(xml, "pieChart") || contains_element(xml, "doughnutChart")
}

/// Whether the source contains a 2-D `c:scatterChart` group — the group whose renderer honors the
/// full `c:marker` symbol set (P25, sharing the line renderer's `paint_marker`) and the
/// `c:scatterStyle` marker/line combination. Boundary-aware. Same textual-classifier caveat as
/// [`unsupported_marker`] (a combo part with a scatter plus another group returns true for the whole
/// part).
fn is_scatter_chart(xml: &str) -> bool {
    contains_element(xml, "scatterChart")
}

/// Whether the source contains a 2-D `c:bubbleChart` group — the group whose renderer draws sized
/// circles over two numeric axes (P26). Boundary-aware. Used to scope the `c:bubble3D` degrade
/// (`unsupported_bubble_3d`) to bubble charts.
fn is_bubble_chart(xml: &str) -> bool {
    contains_element(xml, "bubbleChart")
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
            "<c:bubbleChart><c:sizeRepresents val=\"area\"/></c:bubbleChart>",
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
    fn p22_bar_gap_overlap_and_theme_fill_stay_faithful() {
        // The P22 bar renderer honors `c:gapWidth` / `c:overlap` and resolves a `schemeClr` series
        // fill, so a bar chart carrying non-default spacing + a theme fill must NOT be badged.
        let xml = r#"
            <c:barChart>
              <c:barDir val="bar"/><c:grouping val="clustered"/>
              <c:ser>
                <c:spPr><a:solidFill><a:schemeClr val="accent2"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></c:spPr>
              </c:ser>
              <c:gapWidth val="40"/><c:overlap val="50"/>
              <c:axId val="1"/><c:axId val="2"/>
            </c:barChart>
        "#;
        assert_eq!(source_fidelity(xml), Fidelity::Faithful);
    }

    #[test]
    fn p23_area_stays_faithful_but_line_scoped_features_degrade() {
        // A plain area (even carrying a resolved `schemeClr` series fill) is Faithful — the P23 area
        // renderer draws all three groupings + theme fills.
        let faithful = r#"
            <c:areaChart>
              <c:grouping val="stacked"/>
              <c:ser>
                <c:spPr><a:solidFill><a:schemeClr val="accent3"><a:lumOff val="40000"/></a:schemeClr></a:solidFill></c:spPr>
              </c:ser>
              <c:axId val="1"/><c:axId val="2"/>
            </c:areaChart>
        "#;
        assert_eq!(source_fidelity(faithful), Fidelity::Faithful);

        // But the area renderer does NOT draw data labels or honor axis scaling (those stay
        // line-scoped, exactly like bar), so an area carrying either keeps its honest Degraded badge.
        let shown_labels = "<c:areaChart><c:dLbls><c:showVal val=\"1\"/></c:dLbls></c:areaChart>";
        assert_eq!(
            source_fidelity(shown_labels),
            Fidelity::Degraded,
            "shown data labels on area → Degraded"
        );
        let scaled =
            "<c:areaChart/><c:valAx><c:scaling><c:min val=\"0\"/><c:max val=\"100\"/></c:scaling></c:valAx>";
        assert_eq!(
            source_fidelity(scaled),
            Fidelity::Degraded,
            "axis min/max on area → Degraded"
        );
    }

    #[test]
    fn p24_pie_dpt_and_percent_labels_faithful_but_other_labels_degrade() {
        // The P24 pie/doughnut renderer honors c:dPt (per-slice color + explosion), varyColors,
        // firstSliceAng, holeSize, and on-slice PERCENT labels — so a pie carrying them is Faithful.
        for faithful in [
            "<c:pieChart><c:varyColors val=\"1\"/><c:ser><c:dPt><c:idx val=\"1\"/><c:explosion val=\"25\"/></c:dPt></c:ser><c:firstSliceAng val=\"90\"/></c:pieChart>",
            "<c:doughnutChart><c:ser/><c:holeSize val=\"50\"/></c:doughnutChart>",
            "<c:pieChart><c:ser><c:dLbls><c:showPercent val=\"1\"/></c:dLbls></c:ser></c:pieChart>",
        ] {
            assert_eq!(
                source_fidelity(faithful),
                Fidelity::Faithful,
                "expected Faithful for {faithful}"
            );
        }

        // But the pie renderer does NOT draw the OTHER label kinds (value / category / series name /
        // legend key), so a pie showing any of those keeps its honest Degraded badge.
        for degraded in [
            "<c:pieChart><c:ser><c:dLbls><c:showVal val=\"1\"/></c:dLbls></c:ser></c:pieChart>",
            "<c:pieChart><c:ser><c:dLbls><c:showCatName val=\"1\"/></c:dLbls></c:ser></c:pieChart>",
            "<c:doughnutChart><c:dLbls><c:showSerName val=\"1\"/></c:dLbls></c:doughnutChart>",
        ] {
            assert_eq!(
                source_fidelity(degraded),
                Fidelity::Degraded,
                "expected Degraded for {degraded}"
            );
        }

        // c:dPt is pie-SCOPED: a dPt on a non-pie group still renders wrong → Degraded.
        for group in ["barChart", "lineChart", "areaChart", "scatterChart"] {
            let xml =
                format!("<c:{group}><c:ser><c:dPt><c:idx val=\"0\"/></c:dPt></c:ser></c:{group}>");
            assert_eq!(
                source_fidelity(&xml),
                Fidelity::Degraded,
                "a dPt on {group} → Degraded"
            );
        }
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
                // A conditional format whose value contains an unescaped `>` — a format we only
                // approximate (the condition is dropped), and also exercises the quote-aware tag
                // scan (a naive `find('>')` would truncate the attribute and miss the code).
                "conditional numFmt with '>' in value",
                "<c:valAx><c:numFmt formatCode=\"[Red][>1000]#,##0\" sourceLinked=\"0\"/></c:valAx>",
            ),
            (
                // A per-point label override (idx-keyed) — degrades even on a line chart, since we
                // draw uniform series labels, not per-point ones.
                "per-point dLbl override",
                "<c:ser><c:dLbls><c:dLbl><c:idx val=\"0\"/><c:showVal val=\"1\"/></c:dLbl></c:dLbls></c:ser>",
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
    fn shown_data_labels_are_line_scoped() {
        // P12's LINE renderer draws data labels, so a shown `c:dLbls` on a `lineChart` is Faithful
        // (the accessor auto-drops the feature for the group that honors it). The same shown label
        // on a group whose renderer ignores labels (bar/area/pie/scatter, their phases are P16+)
        // still renders wrong → Degraded.
        let shown = "<c:dLbls><c:showVal val=\"1\"/><c:showPercent val=\"1\"/></c:dLbls>";
        assert_eq!(
            source_fidelity(&format!("<c:lineChart>{shown}</c:lineChart>")),
            Fidelity::Faithful,
            "line renders data labels → Faithful"
        );
        for group in ["barChart", "areaChart", "pieChart", "scatterChart"] {
            assert_eq!(
                source_fidelity(&format!("<c:{group}>{shown}</c:{group}>")),
                Fidelity::Degraded,
                "{group} ignores data labels → Degraded"
            );
        }
    }

    #[test]
    fn per_point_label_override_degrades_everywhere_but_dlbls_container_does_not() {
        // A singular `<c:dLbl>` (per-point) degrades on ANY group, including line.
        let per_point = "<c:dLbls><c:dLbl><c:idx val=\"1\"/></c:dLbl></c:dLbls>";
        for group in ["lineChart", "barChart"] {
            assert_eq!(
                source_fidelity(&format!("<c:{group}>{per_point}</c:{group}>")),
                Fidelity::Degraded,
                "{group} with a per-point dLbl → Degraded"
            );
        }
        // The plural `<c:dLbls>` container (all-off) must NOT be mistaken for a per-point `<c:dLbl>`
        // — boundary matching keeps them distinct.
        let all_off = "<c:dLbls><c:showVal val=\"0\"/></c:dLbls>";
        assert_eq!(
            source_fidelity(&format!("<c:lineChart>{all_off}</c:lineChart>")),
            Fidelity::Faithful,
            "an all-off dLbls container is benign, not a per-point override"
        );
    }

    #[test]
    fn number_format_degrades_only_codes_we_approximate() {
        // A code the applier renders exactly is Faithful (P6/P12 apply it to ticks + labels).
        for code in ["General", "0%", "$#,##0", "#,##0.00"] {
            let xml = format!(
                "<c:lineChart><c:valAx><c:numFmt formatCode=\"{code}\" sourceLinked=\"0\"/></c:valAx></c:lineChart>"
            );
            assert_eq!(
                source_fidelity(&xml),
                Fidelity::Faithful,
                "supported numFmt {code:?} → Faithful"
            );
        }
        // A code we only approximate still warns — including a `#,##0,` scaling comma (÷1000),
        // which parses but mis-renders (the Critical false-Faithful this guards against).
        for code in [
            "yyyy-mm-dd",
            "0.00E+00",
            "#,##0;(#,##0)",
            "#,##0,",
            "0.00_)",
        ] {
            let xml = format!(
                "<c:lineChart><c:valAx><c:numFmt formatCode=\"{code}\" sourceLinked=\"0\"/></c:valAx></c:lineChart>"
            );
            assert_eq!(
                source_fidelity(&xml),
                Fidelity::Degraded,
                "approximated numFmt {code:?} → Degraded"
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
    fn markers_are_scoped_to_the_line_and_scatter_renderers() {
        // The marker fidelity is SCOPED: the line AND scatter renderers paint the full symbol set
        // (P6/P25, sharing `paint_marker`), so a non-circle marker on either is Faithful. The
        // bar/area/pie renderers still ignore `c:marker`, so a non-circle marker on those STILL
        // degrades (no false-Faithful — the wrong chart keeps its badge).
        let diamond = "<c:ser><c:marker><c:symbol val=\"diamond\"/></c:marker></c:ser>";

        for group in ["lineChart", "scatterChart"] {
            assert_eq!(
                source_fidelity(&format!("<c:{group}>{diamond}</c:{group}>")),
                Fidelity::Faithful,
                "{group} renders the diamond marker → Faithful"
            );
        }
        for group in ["barChart", "areaChart", "pieChart"] {
            assert_eq!(
                source_fidelity(&format!("<c:{group}>{diamond}</c:{group}>")),
                Fidelity::Degraded,
                "{group} ignores the marker (draws its default) → Degraded"
            );
        }

        // circle / none are what the fixed renderer draws (or nothing) → Faithful anywhere.
        for symbol in ["circle", "none"] {
            let marker =
                format!("<c:ser><c:marker><c:symbol val=\"{symbol}\"/></c:marker></c:ser>");
            assert_eq!(
                source_fidelity(&format!("<c:barChart>{marker}</c:barChart>")),
                Fidelity::Faithful,
                "bar + {symbol} marker → Faithful"
            );
        }
    }

    #[test]
    fn p25_scatter_markers_faithful_but_smooth_degrades() {
        // The P25 scatter renderer draws every c:marker symbol and the marker/line/lineMarker styles
        // exactly → Faithful (with any marker).
        for faithful in [
            "<c:scatterChart><c:scatterStyle val=\"marker\"/><c:ser><c:marker><c:symbol val=\"diamond\"/></c:marker></c:ser></c:scatterChart>",
            "<c:scatterChart><c:scatterStyle val=\"lineMarker\"/><c:ser><c:marker><c:symbol val=\"square\"/></c:marker></c:ser></c:scatterChart>",
            "<c:scatterChart><c:scatterStyle val=\"line\"/></c:scatterChart>",
        ] {
            assert_eq!(
                source_fidelity(faithful),
                Fidelity::Faithful,
                "expected Faithful for {faithful}"
            );
        }
        // But a SMOOTH scatter is drawn straight → Degraded (honest badge for the fallback).
        for degraded in [
            "<c:scatterChart><c:scatterStyle val=\"smooth\"/></c:scatterChart>",
            "<c:scatterChart><c:scatterStyle val=\"smoothMarker\"/><c:ser><c:marker><c:symbol val=\"circle\"/></c:marker></c:ser></c:scatterChart>",
        ] {
            assert_eq!(
                source_fidelity(degraded),
                Fidelity::Degraded,
                "expected Degraded for {degraded}"
            );
        }
    }

    #[test]
    fn p26_bubble_faithful_but_3d_degrades() {
        // The P26 bubble renderer honors two numeric axes, √-area/width size encoding, and
        // `c:sizeRepresents` (area/width) → Faithful (with either representation, and a benign 2-D
        // `c:bubble3D val="0"`).
        for faithful in [
            "<c:bubbleChart><c:ser><c:bubbleSize><c:numRef/></c:bubbleSize></c:ser><c:sizeRepresents val=\"area\"/></c:bubbleChart>",
            "<c:bubbleChart><c:sizeRepresents val=\"w\"/></c:bubbleChart>",
            "<c:bubbleChart><c:ser><c:bubble3D val=\"0\"/></c:ser></c:bubbleChart>",
        ] {
            assert_eq!(
                source_fidelity(faithful),
                Fidelity::Faithful,
                "expected Faithful for {faithful}"
            );
        }
        // But a 3-D bubble is drawn flat → Degraded (honest badge for the flatten).
        for degraded in [
            "<c:bubbleChart><c:bubble3D val=\"1\"/></c:bubbleChart>",
            "<c:bubbleChart><c:ser><c:bubble3D val=\"true\"/></c:ser></c:bubbleChart>",
        ] {
            assert_eq!(
                source_fidelity(degraded),
                Fidelity::Degraded,
                "expected Degraded for {degraded}"
            );
        }
        // `c:bubble3D` is bubble-scoped: it never appears on other groups, but the scoping guard
        // means a stray bubble3D on a non-bubble group is ignored (no false badge from this check).
        assert_eq!(
            source_fidelity("<c:lineChart><c:bubble3D val=\"1\"/></c:lineChart>"),
            Fidelity::Faithful,
            "bubble3D is scoped to bubbleChart"
        );
    }

    #[test]
    fn axis_scaling_is_line_scoped() {
        // P13's LINE renderer honors explicit min/max bounds and a reversed orientation, so on a
        // `lineChart` they are Faithful (the accessor auto-drops the feature for the group that
        // honors it). The same scaling on a group whose renderer ignores it (bar/area/pie/scatter,
        // their phases are P16+) still renders wrong → Degraded.
        for scaling in [
            "<c:valAx><c:scaling><c:min val=\"0\"/><c:max val=\"100\"/></c:scaling></c:valAx>",
            "<c:catAx><c:scaling><c:orientation val=\"maxMin\"/></c:scaling></c:catAx>",
        ] {
            assert_eq!(
                source_fidelity(&format!("<c:lineChart>{scaling}</c:lineChart>")),
                Fidelity::Faithful,
                "line honors scaling → Faithful: {scaling}"
            );
            for group in ["barChart", "areaChart", "pieChart", "scatterChart"] {
                assert_eq!(
                    source_fidelity(&format!("<c:{group}>{scaling}</c:{group}>")),
                    Fidelity::Degraded,
                    "{group} ignores scaling → Degraded: {scaling}"
                );
            }
        }
    }

    #[test]
    fn major_gridline_toggle_is_faithful_but_minor_degrades() {
        // The line renderer honors the MAJOR gridline toggle (P13), so `c:majorGridlines` present or
        // absent both render as authored — never a badge.
        for xml in [
            "<c:lineChart/><c:valAx><c:majorGridlines/></c:valAx>",
            "<c:lineChart/><c:valAx></c:valAx>",
        ] {
            assert_eq!(
                source_fidelity(xml),
                Fidelity::Faithful,
                "major gridline toggle is honored → Faithful: {xml}"
            );
        }
        // MINOR gridlines are NOT drawn (the renderer draws only major), so an authored
        // `c:minorGridlines` renders without them → Degraded (honest badge, not a silent drop).
        assert_eq!(
            source_fidelity("<c:lineChart/><c:valAx><c:minorGridlines/></c:valAx>"),
            Fidelity::Degraded,
            "minor gridlines are not rendered → Degraded"
        );
    }

    #[test]
    fn non_solid_line_stroke_degrades_but_plain_solid_is_faithful() {
        // The line renderer draws a PLAIN SOLID a:ln (width/color/alpha), so a non-solid stroke
        // sub-feature renders as solid and must degrade (a dashed forecast line drawn solid would
        // silently mislead — functional_spec §5).
        for (label, ln) in [
            (
                "preset dash",
                "<c:spPr><a:ln w=\"28440\"><a:solidFill><a:srgbClr val=\"4a7ebb\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></c:spPr>",
            ),
            (
                "custom dash",
                "<c:spPr><a:ln w=\"28440\"><a:custDash><a:ds d=\"400000\" sp=\"200000\"/></a:custDash></a:ln></c:spPr>",
            ),
            (
                "compound (double) line",
                "<c:spPr><a:ln w=\"38100\" cmpd=\"dbl\"><a:solidFill><a:srgbClr val=\"4a7ebb\"/></a:solidFill></a:ln></c:spPr>",
            ),
        ] {
            assert_eq!(
                source_fidelity(&format!("<c:lineChart><c:ser>{ln}</c:ser></c:lineChart>")),
                Fidelity::Degraded,
                "non-solid line stroke ({label}) → Degraded"
            );
        }

        // A PLAIN solid a:ln (width + color + alpha, plus the pervasive default round join and an
        // explicit `prstDash val="solid"`) is exactly what we render → Faithful, no false badge.
        let solid = "<c:spPr><a:ln w=\"28440\" cmpd=\"sng\"><a:solidFill><a:srgbClr val=\"4a7ebb\"><a:alpha val=\"60000\"/></a:srgbClr></a:solidFill><a:prstDash val=\"solid\"/><a:round/></a:ln></c:spPr>";
        assert_eq!(
            source_fidelity(&format!(
                "<c:lineChart><c:ser>{solid}</c:ser></c:lineChart>"
            )),
            Fidelity::Faithful,
            "a plain solid width/color/alpha a:ln with the default round join → Faithful"
        );

        // The default a:ln Excel emits on every series (a solid fill + `<a:round/>` join, no dash)
        // must NOT degrade — the exact shape from the reference workbook's chart1.xml.
        let excel_default = "<c:spPr><a:solidFill><a:srgbClr val=\"4a7ebb\"/></a:solidFill><a:ln w=\"28440\"><a:solidFill><a:srgbClr val=\"4a7ebb\"/></a:solidFill><a:round/></a:ln></c:spPr>";
        assert_eq!(
            source_fidelity(&format!(
                "<c:lineChart><c:ser>{excel_default}</c:ser></c:lineChart>"
            )),
            Fidelity::Faithful,
            "Excel's default solid a:ln with a round join → Faithful"
        );
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
