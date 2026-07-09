//! The **production chart envelope** — [`ChartSpec`] and its supporting types (charts/
//! architecture §3.2). This is the widening of `chart-model` from the PoC's static render
//! seam ([`Chart`]) to the shape production needs: a [`Chart`] wrapped with its retained
//! **source** XML, its live-binding **source ranges** (`c:f`), its in-grid **anchor**, and
//! its **origin** (loaded from a file vs authored in-app).
//!
//! Everything here is pure data — **gpui-free and ironcalc-free**, like the rest of the
//! crate — so the same value the engine *produces* on load is the value the app *consumes*
//! to place and render a chart, with neither layer reaching across the seam.

use crate::Chart;

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
    Loaded { source: SourceXml },
    /// Built in-app via authoring (P22+). Has no retained source — the write path synthesizes
    /// chart XML from a template on save.
    Authored,
}

/// The production chart envelope (charts/architecture §3.2): the render seam [`Chart`] wrapped
/// with everything production needs beyond a static picture — its live-binding
/// [`source_ranges`](ChartSpec::source_ranges), its in-grid [`anchor`](ChartSpec::anchor), and
/// its [`origin`](ChartSpec::origin) (which carries the retained source XML for a loaded
/// chart). The engine produces this on load; the app consumes it to place and render a chart.
///
/// **Deferred to later phases** (kept out of this shape until their behavior is built): the
/// `dirty` / `last_values` live-binding bookkeeping (P9 — the file cache already in
/// `chart.series` is the fallback until then) and the derived `display_fidelity()` accessor
/// (P3, computed over `chart` + `source`).
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSpec {
    /// The render seam — the static chart picture.
    pub chart: Chart,
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
            chart,
            source_ranges,
            anchor,
            origin: Origin::Loaded { source },
        }
    }

    /// A chart **authored in-app** — no retained source (synthesized on save). Its source
    /// ranges start empty and are set as the chart is shaped (P25).
    pub fn authored(chart: Chart, anchor: Anchor) -> Self {
        Self {
            chart,
            source_ranges: Vec::new(),
            anchor,
            origin: Origin::Authored,
        }
    }

    /// The retained source XML — `Some` iff this chart was loaded from a file.
    pub fn source(&self) -> Option<&SourceXml> {
        match &self.origin {
            Origin::Loaded { source } => Some(source),
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
        assert_eq!(spec.chart, sample_chart());
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
}
