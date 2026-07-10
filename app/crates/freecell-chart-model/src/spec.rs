//! The **production chart envelope** — [`ChartSpec`] and its supporting types (charts/
//! architecture §3.2). This is the widening of `chart-model` from the PoC's static render
//! seam ([`Chart`]) to the shape production needs: a [`Chart`] wrapped with its retained
//! **source** XML, its live-binding **source ranges** (`c:f`), its in-grid **anchor**, and
//! its **origin** (loaded from a file vs authored in-app).
//!
//! Everything here is pure data — **gpui-free and ironcalc-free**, like the rest of the
//! crate — so the same value the engine *produces* on load is the value the app *consumes*
//! to place and render a chart, with neither layer reaching across the seam.

use std::sync::Arc;

use crate::{source_fidelity, Chart, Fidelity};

/// One corner of an `xdr:twoCellAnchor` (its `<xdr:from>` / `<xdr:to>`): a 0-based sheet cell
/// plus an intra-cell offset in **EMUs** (English Metric Units, 914 400 per inch) from that
/// cell's top-left. Mapping this to pixels against the grid geometry is the app layer's job
/// (P8); the model retains the raw OOXML shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorCell {
    /// 0-based column index (`<xdr:col>`).
    pub col: u32,
    /// Offset in EMUs within the column (`<xdr:colOff>`).
    pub col_off_emu: i64,
    /// 0-based row index (`<xdr:row>`).
    pub row: u32,
    /// Offset in EMUs within the row (`<xdr:rowOff>`).
    pub row_off_emu: i64,
}

impl AnchorCell {
    /// A cell corner pinned to a cell's top-left (zero EMU offsets).
    pub const fn new(col: u32, row: u32) -> Self {
        Self {
            col,
            col_off_emu: 0,
            row,
            row_off_emu: 0,
        }
    }

    /// A cell corner with explicit intra-cell EMU offsets.
    pub const fn with_offsets(col: u32, col_off_emu: i64, row: u32, row_off_emu: i64) -> Self {
        Self {
            col,
            col_off_emu,
            row,
            row_off_emu,
        }
    }
}

/// A chart's placement in the sheet — an `xdr:twoCellAnchor`'s from/to corners. The chart
/// occupies the rectangle spanning `from`..`to`, so it scrolls and zooms with the sheet
/// (the anchor is in sheet coordinates, not screen coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    /// Top-left corner.
    pub from: AnchorCell,
    /// Bottom-right corner.
    pub to: AnchorCell,
}

impl Anchor {
    pub const fn new(from: AnchorCell, to: AnchorCell) -> Self {
        Self { from, to }
    }
}

/// One `<c:f>` data reference retained from a chart series — the formula text exactly as
/// written in the source (e.g. `Data!$B$2:$B$5`). Live binding (P9) resolves this against the
/// current worksheet to refresh the chart's values; at this phase it is the retained,
/// as-written reference. Structured sheet/range decomposition arrives with live binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfRange {
    pub formula: String,
}

impl CfRange {
    pub fn new(formula: impl Into<String>) -> Self {
        Self {
            formula: formula.into(),
        }
    }

    /// The formula text as written.
    pub fn as_str(&self) -> &str {
        &self.formula
    }
}

/// One package part retained alongside the chart XML — the chart's own relationships or aux
/// parts (`xl/charts/_rels/chartN.xml.rels`, `colorsN.xml`, `styleN.xml`, any embeddings),
/// kept as raw bytes so a save can carry them through **byte-for-byte**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePart {
    /// Package part name, e.g. `xl/charts/_rels/chart1.xml.rels`.
    pub part_name: String,
    /// The part's bytes, verbatim.
    pub bytes: Vec<u8>,
}

impl SourcePart {
    pub fn new(part_name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            part_name: part_name.into(),
            bytes: bytes.into(),
        }
    }
}

/// The retained source of a chart **loaded from a file**: the chart part's XML plus its own
/// related parts. It is the substrate for save byte-preservation, targeted edit-patching
/// (charts/architecture §5), and the derived fidelity accessor (P3). It is deliberately kept
/// as the raw, as-loaded text/bytes — **not** a borrowed DOM: the engine re-parses it on
/// demand and patches it textually, matching the existing `open_fixups` / chart-`save`
/// second-pass style (and side-stepping the borrow-from-source lifetime a stored DOM carries).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceXml {
    /// The `xl/charts/chartN.xml` document text, verbatim as loaded.
    pub chart_xml: String,
    /// The chart part's related parts (its `_rels`, `colorsN`/`styleN`, embeddings), retained
    /// verbatim so a save can carry them through byte-for-byte.
    pub related_parts: Vec<SourcePart>,
}

impl SourceXml {
    /// Retained source with just the chart XML (no related parts yet).
    pub fn new(chart_xml: impl Into<String>) -> Self {
        Self {
            chart_xml: chart_xml.into(),
            related_parts: Vec::new(),
        }
    }

    /// Attach the chart's related parts (builder style).
    pub fn with_related_parts(mut self, related_parts: Vec<SourcePart>) -> Self {
        self.related_parts = related_parts;
        self
    }
}

/// Where a chart came from — which decides whether it carries retained source XML (charts/
/// architecture §3.2). Folding the source into this enum makes the invariant "authored charts
/// have no source, loaded charts do" **unrepresentable-if-violated**, rather than two fields
/// that must be kept in sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Parsed from an opened `.xlsx`; carries its retained [`SourceXml`] so save can
    /// byte-preserve it or targeted-patch it (charts/architecture §5).
    ///
    /// The source is held behind an [`Arc`] so cloning a [`ChartSpec`] — which the worker does on
    /// **every** intersecting edit to build the published snapshot (P9/P11) — bumps a refcount
    /// instead of deep-copying the (potentially large) chart XML + related parts. Only the render
    /// [`Chart`], the value that actually changes on a re-resolve, is cloned. The app shares the
    /// same `Arc` when it reads the snapshot, so an on-grid chart holds no independent copy of its
    /// retained source (charts/architecture §5 challenge 5, "off-screen free").
    Loaded { source: Arc<SourceXml> },
    /// Built in-app via authoring (P22+). Has no retained source — the write path synthesizes
    /// chart XML from a template on save.
    Authored,
}

/// The render content of a [`ChartSpec`] — either a chart we parsed into a typed render
/// [`Chart`], or a chart the load walk **reached but could not parse** into one (an unsupported
/// group — surface / radar / stock / ofPie / bubble / a 3-D type with no 2-D reduction / malformed
/// chart XML, charts/architecture §6).
///
/// Folding "no typed chart" into the spec this way — rather than an `Option<Chart>` plus a
/// separate flag — makes the robustness invariant **unrepresentable-if-violated** (the same
/// rationale as [`Origin`]): an [`Unsupported`](ChartBody::Unsupported) body has no [`Chart`],
/// its [`display_fidelity`](ChartSpec::display_fidelity) is forced to
/// [`Unsupported`](Fidelity::Unsupported) (→ the P8 placeholder, architecture §4.2, which does
/// **not** use the `Chart` content), and live binding (P9) has nothing to re-resolve. The only
/// thing salvaged for the placeholder is the chart's title.
// The `Parsed(Chart)` variant is deliberately inline, not boxed: it is the overwhelmingly-common
// case (this field was a plain inline `chart: Chart` before P14), and the P11 snapshot path clones
// the render `Chart` on every intersecting edit — boxing it would add heap indirection to that hot
// path for a rare `Unsupported` case. The size gap is expected, not an oversight.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum ChartBody {
    /// A chart parsed into a typed render [`Chart`]. Its fidelity is classified from the retained
    /// source (Faithful, or Degraded for a 3-D→2-D reduction or an unrendered feature).
    Parsed(Chart),
    /// A chart the load walk reached but could not parse into a typed [`Chart`]. Retained (its
    /// source, anchor, and ranges live on the enclosing [`ChartSpec`]) so save byte-preserves it
    /// and the grid draws its placeholder, but it carries **no** render picture — only the chart's
    /// title (if any) for the placeholder caption.
    Unsupported { title: Option<String> },
}

/// The production chart envelope (charts/architecture §3.2): the render [`body`](ChartSpec::body)
/// (a typed [`Chart`], or an [`Unsupported`](ChartBody::Unsupported) placeholder marker) wrapped
/// with everything production needs beyond a static picture — its live-binding
/// [`source_ranges`](ChartSpec::source_ranges), its in-grid [`anchor`](ChartSpec::anchor), and
/// its [`origin`](ChartSpec::origin) (which carries the retained source XML for a loaded
/// chart). The engine produces this on load; the app consumes it to place and render a chart.
///
/// **Deferred to later phases** (kept out of this shape until their behavior is built): the
/// `dirty` / `last_values` live-binding bookkeeping (P9 — the file cache already in
/// `chart.series` is the fallback until then).
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSpec {
    /// The render seam — a typed [`Chart`] picture, or an [`Unsupported`](ChartBody::Unsupported)
    /// marker for a chart we retained but cannot draw (→ placeholder).
    pub body: ChartBody,
    /// The `c:f` references the chart's data resolves against (for live binding, P9).
    pub source_ranges: Vec<CfRange>,
    /// The chart's `twoCellAnchor` placement in the sheet.
    pub anchor: Anchor,
    /// Where the chart came from (and, for a loaded chart, its retained source).
    pub origin: Origin,
}

impl ChartSpec {
    /// A chart **loaded from a file** — retains its source XML so save can byte-preserve or
    /// targeted-patch it.
    pub fn loaded(
        chart: Chart,
        source: SourceXml,
        source_ranges: Vec<CfRange>,
        anchor: Anchor,
    ) -> Self {
        Self {
            body: ChartBody::Parsed(chart),
            source_ranges,
            anchor,
            origin: Origin::Loaded {
                source: Arc::new(source),
            },
        }
    }

    /// A chart **loaded from a file that we could not parse** into a typed [`Chart`] (an
    /// unsupported group / malformed part, charts/architecture §6). It retains its source XML +
    /// ranges + anchor — so save byte-preserves it and the grid draws its placeholder — but has no
    /// render picture; `title` is salvaged for the placeholder caption. Its
    /// [`display_fidelity`](Self::display_fidelity) is always [`Unsupported`](Fidelity::Unsupported).
    pub fn loaded_unsupported(
        title: Option<String>,
        source: SourceXml,
        source_ranges: Vec<CfRange>,
        anchor: Anchor,
    ) -> Self {
        Self {
            body: ChartBody::Unsupported { title },
            source_ranges,
            anchor,
            origin: Origin::Loaded {
                source: Arc::new(source),
            },
        }
    }

    /// A chart **authored in-app** — no retained source (synthesized on save). Its source
    /// ranges start empty and are set as the chart is shaped (P25).
    pub fn authored(chart: Chart, anchor: Anchor) -> Self {
        Self {
            body: ChartBody::Parsed(chart),
            source_ranges: Vec::new(),
            anchor,
            origin: Origin::Authored,
        }
    }

    /// The typed render [`Chart`], or `None` for an [`Unsupported`](ChartBody::Unsupported) chart
    /// that has no render picture (→ placeholder). The render + live-binding paths branch on this.
    pub fn chart(&self) -> Option<&Chart> {
        match &self.body {
            ChartBody::Parsed(chart) => Some(chart),
            ChartBody::Unsupported { .. } => None,
        }
    }

    /// Mutable access to the typed render [`Chart`], or `None` for an
    /// [`Unsupported`](ChartBody::Unsupported) chart — the live-binding re-resolve writes fresh
    /// values through this (an Unsupported chart is static, so it is never touched).
    pub fn chart_mut(&mut self) -> Option<&mut Chart> {
        match &mut self.body {
            ChartBody::Parsed(chart) => Some(chart),
            ChartBody::Unsupported { .. } => None,
        }
    }

    /// The chart's title — from the parsed [`Chart`], or the title salvaged for an
    /// [`Unsupported`](ChartBody::Unsupported) chart's placeholder caption.
    pub fn title(&self) -> Option<&str> {
        match &self.body {
            ChartBody::Parsed(chart) => chart.title.as_deref(),
            ChartBody::Unsupported { title } => title.as_deref(),
        }
    }

    /// The retained source XML — `Some` iff this chart was loaded from a file.
    pub fn source(&self) -> Option<&SourceXml> {
        match &self.origin {
            Origin::Loaded { source } => Some(source.as_ref()),
            Origin::Authored => None,
        }
    }

    /// Whether this chart was loaded from a file (and so carries retained source).
    pub fn is_loaded(&self) -> bool {
        matches!(self.origin, Origin::Loaded { .. })
    }

    /// Whether this chart was authored in-app (and so has no retained source).
    pub fn is_authored(&self) -> bool {
        matches!(self.origin, Origin::Authored)
    }

    /// The chart's [`Fidelity`] — how faithfully the renderer can draw it (charts/
    /// functional_spec §5, architecture §3.3).
    ///
    /// This is a **derived accessor, not stored state**: there is no parse-time flag to keep in
    /// sync.
    /// - An [`Unsupported`](ChartBody::Unsupported) body has **no** render picture, so it is always
    ///   [`Fidelity::Unsupported`] (→ placeholder), regardless of what the source classifies as —
    ///   drawing the placeholder is the only honest outcome (architecture §6).
    /// - A [`Parsed`](ChartBody::Parsed) body is computed on demand — for a [loaded](Origin::Loaded)
    ///   chart, by classifying its retained [`SourceXml`] (see [`source_fidelity`] for the buckets
    ///   and the curated render-affecting set); an [authored](Origin::Authored) chart has no source
    ///   and is [`Fidelity::Faithful`] by construction (built from our own model using only
    ///   features we render). Because it reads the source live, it **auto-clears as renderer
    ///   support lands** — a feature that starts rendering drops out of the degrading set with no
    ///   separate bookkeeping.
    pub fn display_fidelity(&self) -> Fidelity {
        if matches!(self.body, ChartBody::Unsupported { .. }) {
            return Fidelity::Unsupported;
        }
        match &self.origin {
            Origin::Loaded { source } => source_fidelity(&source.chart_xml),
            Origin::Authored => Fidelity::Faithful,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, Category, ChartKind, Grouping, Legend, Series};

    /// A minimal but complete line chart, used across the envelope tests.
    fn sample_chart() -> Chart {
        Chart {
            title: Some("Revenue".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("2024"),
                vec![
                    Category::Text("Q1".into()),
                    Category::Text("Q2".into()),
                    Category::Text("Q3".into()),
                ],
                vec![10.0, 20.0, 30.0],
            )],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("USD"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn anchor_spans_from_and_to_cells() {
        let from = AnchorCell::new(1, 5);
        let to = AnchorCell::with_offsets(4, 12_700, 10, 25_400);
        let anchor = Anchor::new(from, to);

        assert_eq!(anchor.from, from);
        assert_eq!(anchor.to, to);
        // `new` pins to the cell's top-left (zero offsets); `with_offsets` keeps the EMUs.
        assert_eq!(anchor.from.col_off_emu, 0);
        assert_eq!(anchor.from.row_off_emu, 0);
        assert_eq!(anchor.to.col_off_emu, 12_700);
        assert_eq!(anchor.to.row_off_emu, 25_400);
    }

    #[test]
    fn cf_range_retains_formula_text() {
        let r = CfRange::new("Data!$B$2:$B$5");
        assert_eq!(r.as_str(), "Data!$B$2:$B$5");
        assert_eq!(r, CfRange::new(String::from("Data!$B$2:$B$5")));
    }

    #[test]
    fn source_xml_holds_chart_xml_and_related_parts() {
        let source = SourceXml::new("<c:chartSpace/>").with_related_parts(vec![
            SourcePart::new(
                "xl/charts/_rels/chart1.xml.rels",
                b"<Relationships/>".to_vec(),
            ),
            SourcePart::new("xl/charts/colors1.xml", b"<clrMapOvr/>".to_vec()),
        ]);

        assert_eq!(source.chart_xml, "<c:chartSpace/>");
        assert_eq!(source.related_parts.len(), 2);
        assert_eq!(
            source.related_parts[0].part_name,
            "xl/charts/_rels/chart1.xml.rels"
        );
        assert_eq!(source.related_parts[1].bytes, b"<clrMapOvr/>".to_vec());
    }

    #[test]
    fn loaded_spec_carries_source_ranges_and_anchor() {
        let anchor = Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(6, 12));
        let ranges = vec![
            CfRange::new("Data!$B$2:$B$4"),
            CfRange::new("Data!$A$2:$A$4"),
        ];
        let spec = ChartSpec::loaded(
            sample_chart(),
            SourceXml::new("<c:chartSpace>line</c:chartSpace>"),
            ranges.clone(),
            anchor,
        );

        assert!(spec.is_loaded());
        assert!(!spec.is_authored());
        assert_eq!(
            spec.source().map(|s| s.chart_xml.as_str()),
            Some("<c:chartSpace>line</c:chartSpace>")
        );
        assert_eq!(spec.source_ranges, ranges);
        assert_eq!(spec.anchor, anchor);
        assert_eq!(spec.chart(), Some(&sample_chart()));
        assert_eq!(spec.title(), Some("Revenue"));
    }

    #[test]
    fn authored_spec_has_no_source() {
        let anchor = Anchor::new(AnchorCell::new(2, 2), AnchorCell::new(8, 14));
        let spec = ChartSpec::authored(sample_chart(), anchor);

        assert!(spec.is_authored());
        assert!(!spec.is_loaded());
        assert!(spec.source().is_none());
        assert!(spec.source_ranges.is_empty());
        assert_eq!(spec.origin, Origin::Authored);
    }

    #[test]
    fn spec_clone_and_partial_eq() {
        let anchor = Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(5, 10));
        let spec = ChartSpec::loaded(
            sample_chart(),
            SourceXml::new("<c:chartSpace/>"),
            vec![CfRange::new("Data!$B$2:$B$4")],
            anchor,
        );

        // The worker publish path clones + compares specs, so both must round-trip.
        assert_eq!(spec.clone(), spec);

        let mut moved = spec.clone();
        moved.anchor.to.row = 20;
        assert_ne!(moved, spec, "a different anchor makes specs unequal");
    }

    #[test]
    fn loaded_spec_display_fidelity_reads_source() {
        let anchor = Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(6, 12));
        let spec = |chart_xml: &str| {
            ChartSpec::loaded(
                sample_chart(),
                SourceXml::new(chart_xml),
                Vec::new(),
                anchor,
            )
        };

        // A plain supported group renders as authored.
        assert_eq!(
            spec("<c:lineChart/>").display_fidelity(),
            Fidelity::Faithful
        );
        // A 3-D group is degraded to its 2-D equivalent.
        assert_eq!(
            spec("<c:bar3DChart/>").display_fidelity(),
            Fidelity::Degraded
        );
        // A type with no 2-D equivalent falls back to the placeholder.
        assert_eq!(
            spec("<c:surfaceChart/>").display_fidelity(),
            Fidelity::Unsupported
        );
    }

    #[test]
    fn authored_spec_is_faithful() {
        let anchor = Anchor::new(AnchorCell::new(2, 2), AnchorCell::new(8, 14));
        let spec = ChartSpec::authored(sample_chart(), anchor);
        assert_eq!(spec.display_fidelity(), Fidelity::Faithful);
    }

    #[test]
    fn loaded_unsupported_retains_envelope_but_has_no_chart() {
        let anchor = Anchor::new(AnchorCell::new(1, 1), AnchorCell::new(6, 12));
        let ranges = vec![CfRange::new("Data!$B$2:$B$5")];
        let spec = ChartSpec::loaded_unsupported(
            Some("Terrain".into()),
            SourceXml::new("<c:surfaceChart/>"),
            ranges.clone(),
            anchor,
        );

        // The envelope is retained — source, ranges, anchor — so save byte-preserves it.
        assert!(spec.is_loaded());
        assert_eq!(
            spec.source().map(|s| s.chart_xml.as_str()),
            Some("<c:surfaceChart/>")
        );
        assert_eq!(spec.source_ranges, ranges);
        assert_eq!(spec.anchor, anchor);

        // But there is NO render picture — only the salvaged placeholder title.
        assert_eq!(spec.chart(), None);
        assert_eq!(spec.title(), Some("Terrain"));
    }

    #[test]
    fn unsupported_body_is_always_unsupported_fidelity() {
        let anchor = Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(6, 12));
        // Even a source that WOULD classify Faithful (a plain, unrecognized-by-us group like
        // bubbleChart) or Degraded must report Unsupported when the body could not be parsed —
        // there is no picture to draw, so the placeholder is the only honest outcome.
        for source in [
            "<c:bubbleChart/>",
            "<c:surfaceChart/>",
            "<c:bar3DChart/>",
            "not xml",
        ] {
            let spec =
                ChartSpec::loaded_unsupported(None, SourceXml::new(source), Vec::new(), anchor);
            assert_eq!(
                spec.display_fidelity(),
                Fidelity::Unsupported,
                "an Unsupported body classifies Unsupported regardless of source {source:?}"
            );
        }
    }

    #[test]
    fn chart_mut_round_trips_a_parsed_body_but_not_an_unsupported_one() {
        let anchor = Anchor::new(AnchorCell::new(0, 0), AnchorCell::new(5, 10));
        let mut parsed = ChartSpec::loaded(
            sample_chart(),
            SourceXml::new("<c:lineChart/>"),
            Vec::new(),
            anchor,
        );
        parsed.chart_mut().expect("parsed body").title = Some("Edited".into());
        assert_eq!(parsed.title(), Some("Edited"));

        let mut unsupported = ChartSpec::loaded_unsupported(
            None,
            SourceXml::new("<c:radarChart/>"),
            Vec::new(),
            anchor,
        );
        assert!(
            unsupported.chart_mut().is_none(),
            "an Unsupported body exposes no Chart to mutate"
        );
    }
}
