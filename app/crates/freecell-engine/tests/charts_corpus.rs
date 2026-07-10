//! P14 — **robustness on a real-file + generated corpus** (charts/functional_spec §1/§7,
//! architecture §6). The exit criterion: *the whole corpus opens without breakage; a line chart
//! renders (Faithful); every other type degrades cleanly (Degraded for 3-D, Unsupported placeholder
//! for the rest) and is RETAINED, not dropped; edge cases fall back without crashing.*
//!
//! The corpus is:
//! - the owner's **real** Excel line-chart workbook (`tests/fixtures/charts/…`, 4 line charts on
//!   2 sheets), committed;
//! - a **generated all-types + edge-case** workbook ([`write_corpus_fixture`]);
//! - two **broken-drawing** workbooks for the per-drawing-resilient `discover` walk.
//!
//! Every step asserts the workbook OPENS (IronCalc + our chart walk) and never panics.

use std::path::PathBuf;

use freecell_chart_model::{ChartKind, Fidelity, SeriesData};
use freecell_engine::chart::authoring::{
    write_bad_aux_rels_fixture, write_corpus_fixture, write_dangling_chart_rel_fixture,
    write_missing_drawing_part_fixture, write_missing_drawing_rels_fixture, CorpusExpect,
};
use freecell_engine::chart::discover_and_parse;

fn real_workbook() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/charts/excel_line_chart_workbook.xlsx")
}

/// Assert a workbook OPENS in IronCalc (the base premise — a real chart-bearing file the engine
/// must load) and our chart walk never breaks on it.
fn assert_opens_in_ironcalc(path: &std::path::Path) {
    ironcalc::import::load_from_xlsx(path.to_str().unwrap(), "en", "UTC", "en")
        .unwrap_or_else(|e| panic!("IronCalc must open {}: {e:?}", path.display()));
}

/// The owner's real Excel workbook opens and parses its 4 line charts as Faithful — the line
/// checkpoint on a genuine Excel-authored file (not an agent fixture).
#[test]
fn real_excel_line_workbook_opens_and_line_charts_render_faithfully() {
    let path = real_workbook();
    assert_opens_in_ironcalc(&path);

    let specs = discover_and_parse(&path).expect("the real workbook's charts discover cleanly");
    assert_eq!(specs.len(), 4, "the real workbook has four embedded charts");
    for spec in &specs {
        assert!(
            matches!(spec.chart().map(|c| &c.kind), Some(ChartKind::Line { .. })),
            "every chart in the real workbook is a line chart"
        );
        assert_eq!(
            spec.display_fidelity(),
            Fidelity::Faithful,
            "a real Excel line chart classifies Faithful"
        );
    }
}

/// The whole generated corpus opens, and every type classifies + retains as expected: supported
/// groups parse Faithful, 3-D groups degrade to 2-D, truly-unsupported groups are RETAINED as
/// Unsupported placeholders (source kept, no render picture) — none dropped.
#[test]
fn generated_corpus_opens_and_every_type_classifies_and_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.xlsx");
    let manifest = write_corpus_fixture(&path).unwrap();

    // The workbook opens despite carrying surface/radar/stock/bubble/3-D charts + a garbage part.
    assert_opens_in_ironcalc(&path);

    // Every chart is RETAINED (nothing dropped): one spec per corpus chart, in document order.
    let specs = discover_and_parse(&path).expect("the corpus discovers without breakage");
    assert_eq!(
        specs.len(),
        manifest.len(),
        "every corpus chart is retained (none dropped)"
    );

    for (spec, entry) in specs.iter().zip(manifest.iter()) {
        match entry.expect {
            CorpusExpect::Faithful => {
                assert_eq!(
                    spec.display_fidelity(),
                    Fidelity::Faithful,
                    "{} must be Faithful",
                    entry.label
                );
                assert!(
                    spec.chart().is_some(),
                    "{} must parse into a typed chart",
                    entry.label
                );
            }
            CorpusExpect::Degraded => {
                assert_eq!(
                    spec.display_fidelity(),
                    Fidelity::Degraded,
                    "{} (3-D) must degrade to 2-D",
                    entry.label
                );
                assert!(
                    spec.chart().is_some(),
                    "{} must still parse into a 2-D chart (rendered + badged)",
                    entry.label
                );
            }
            CorpusExpect::Unsupported => {
                assert_eq!(
                    spec.display_fidelity(),
                    Fidelity::Unsupported,
                    "{} must classify Unsupported",
                    entry.label
                );
                assert!(
                    spec.chart().is_none(),
                    "{} has no render picture (placeholder)",
                    entry.label
                );
                // RETAINED, not dropped: its source XML is kept so save byte-preserves it.
                assert!(
                    spec.is_loaded() && spec.source().is_some(),
                    "{} must retain its source (placeholder-able + byte-preservable)",
                    entry.label
                );
            }
        }
    }
}

/// Edge cases (functional_spec §7) fall back without crashing: an unresolved `c:f` keeps its cached
/// values, an empty range yields a zero-length series, and a non-numeric cell drops just that point.
#[test]
fn generated_corpus_edge_cases_fall_back_without_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.xlsx");
    let manifest = write_corpus_fixture(&path).unwrap();
    let specs = discover_and_parse(&path).unwrap();

    let by_label = |label: &str| {
        let idx = manifest.iter().position(|e| e.label == label).unwrap();
        specs[idx]
            .chart()
            .unwrap_or_else(|| panic!("{label} parsed"))
    };

    // Unresolved c:f (a `Ghost!` sheet): the chart still parses from its cache (live binding would
    // fall back to these cached values, `binding::resolve_falls_back_to_cache…`).
    match &by_label("unresolved_cf").series[0].data {
        SeriesData::CategoryValue { values, .. } => {
            assert_eq!(
                values,
                &vec![120.0, 150.0, 90.0, 170.0],
                "cached values kept"
            )
        }
        other => panic!("expected CategoryValue, got {other:?}"),
    }

    // Empty range: a zero-length series, no crash.
    match &by_label("empty_range").series[0].data {
        SeriesData::CategoryValue { values, .. } => assert!(values.is_empty(), "empty series"),
        other => panic!("expected CategoryValue, got {other:?}"),
    }

    // Non-numeric cell: the bad point is dropped, the numeric one survives.
    match &by_label("nonnumeric").series[0].data {
        SeriesData::CategoryValue { values, .. } => {
            assert_eq!(
                values,
                &vec![42.0],
                "the non-numeric point is dropped, 42 survives"
            )
        }
        other => panic!("expected CategoryValue, got {other:?}"),
    }

    // The groupless + garbage parts are retained as Unsupported placeholders (never crash the load).
    for label in ["groupless", "garbage"] {
        let idx = manifest.iter().position(|e| e.label == label).unwrap();
        assert_eq!(
            specs[idx].display_fidelity(),
            Fidelity::Unsupported,
            "{label}"
        );
        assert!(specs[idx].chart().is_none(), "{label} has no chart");
    }
    // The groupless chart still salvages its title for the placeholder caption.
    let groupless = manifest
        .iter()
        .position(|e| e.label == "groupless")
        .unwrap();
    assert_eq!(specs[groupless].title(), Some("Groupless"));
}

/// A **dangling** `<c:chart r:id>` (an `rId` absent from the drawing's `_rels`) drops just that
/// chart; the workbook opens and the sibling line chart still loads (per-chart-resilient walk).
#[test]
fn dangling_chart_relationship_never_breaks_the_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dangling.xlsx");
    write_dangling_chart_rel_fixture(&path).unwrap();
    assert_opens_in_ironcalc(&path);

    let specs = discover_and_parse(&path).expect("a dangling chart rel never fails the load");
    assert_eq!(specs.len(), 1, "only the resolvable chart comes through");
    assert!(matches!(
        specs[0].chart().unwrap().kind,
        ChartKind::Line { .. }
    ));
}

/// A worksheet whose `<drawing>` has NO `_rels` part drops just that drawing; the workbook opens
/// and the OTHER sheet's line chart still loads (per-drawing-resilient walk).
#[test]
fn missing_drawing_rels_never_breaks_the_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing_rels.xlsx");
    write_missing_drawing_rels_fixture(&path).unwrap();
    assert_opens_in_ironcalc(&path);

    let specs = discover_and_parse(&path).expect("a missing drawing _rels never fails the load");
    assert_eq!(specs.len(), 1, "the healthy sheet's chart survives");
    assert!(matches!(
        specs[0].chart().unwrap().kind,
        ChartKind::Line { .. }
    ));
}

/// A chart with valid chart XML but a **malformed aux `_rels`** is RETAINED as an Unsupported
/// placeholder (its chart XML + anchor + ranges kept, empty related parts) — never dropped over a
/// broken secondary aux part. The workbook opens (architecture §6, "never lose a chart").
#[test]
fn bad_aux_rels_retains_chart_as_unsupported_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad_aux_rels.xlsx");
    write_bad_aux_rels_fixture(&path).unwrap();
    assert_opens_in_ironcalc(&path);

    let specs = discover_and_parse(&path).expect("a broken aux _rels never fails the load");
    assert_eq!(specs.len(), 1, "the chart is retained, not dropped");
    let spec = &specs[0];
    assert_eq!(
        spec.display_fidelity(),
        Fidelity::Unsupported,
        "a chart with unreadable aux parts degrades to a placeholder"
    );
    assert!(spec.chart().is_none(), "no render picture");
    // Its own chart XML + title are still retained (byte-preservable + placeholder-able)…
    assert!(spec.is_loaded() && spec.source().is_some());
    assert!(spec.source().unwrap().chart_xml.contains("<c:lineChart"));
    assert_eq!(spec.title(), Some("Broken Aux"));
    // …but the broken aux `_rels` is dropped → no related parts carried.
    assert!(
        spec.source().unwrap().related_parts.is_empty(),
        "the unreadable aux _rels contributes no related parts"
    );
}

/// A worksheet whose `<drawing>` points at a drawing PART that is **absent** drops just that
/// drawing; the workbook opens and the OTHER sheet's line chart still loads (per-drawing-resilient
/// walk — the `discover` docstring's missing-drawing-part claim, now tested).
#[test]
fn missing_drawing_part_never_breaks_the_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing_drawing_part.xlsx");
    write_missing_drawing_part_fixture(&path).unwrap();
    assert_opens_in_ironcalc(&path);

    let specs = discover_and_parse(&path).expect("a missing drawing part never fails the load");
    assert_eq!(specs.len(), 1, "the healthy sheet's chart survives");
    assert!(matches!(
        specs[0].chart().unwrap().kind,
        ChartKind::Line { .. }
    ));
}

/// A cross-check that the corpus builder + `authoring` module are wired for the integration test —
/// the manifest labels are unique (so `by_label` lookups are unambiguous).
#[test]
fn corpus_manifest_labels_are_unique() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.xlsx");
    let manifest = write_corpus_fixture(&path).unwrap();
    let mut labels: Vec<&str> = manifest.iter().map(|e| e.label).collect();
    labels.sort_unstable();
    let before = labels.len();
    labels.dedup();
    assert_eq!(before, labels.len(), "corpus labels must be unique");
    // A sanity floor: the corpus spans the supported + degraded + unsupported + edge families.
    assert!(
        manifest.len() >= 18,
        "corpus should be broad, got {}",
        manifest.len()
    );
}
