//! The bundled **Demo spreadsheet** — the embedded demo workbook the welcome-window link opens.
//!
//! FreeCell ships one committed demo `.xlsx` asset. The welcome window's "Open Demo Spreadsheet"
//! opens a fresh **untitled** copy of it each time (Save → Save-As, like a new sheet) — the app
//! opens the window with `path: None` (`FreeCellApp::open_demo`), which gives the untitled/save-as
//! behavior; this module only produces a file the engine can load.
//!
//! The IronCalc loader is path-based (no bytes/reader API), so the embedded bytes are materialized
//! to an `.xlsx` on disk that the engine opens. That file must **outlive** the window: the demo is
//! a real chart workbook, and FreeCell's chart system re-reads the source file lazily (on first
//! paint of each sheet) and again on save (chart re-inject) — so it is a persisted (non-deleted)
//! path, not a drop-on-close temp handle.
//!
//! It materializes into the **per-user** cache dir (`<cache_dir>/FreeCell`), matching how the app
//! keeps per-user files (`shell::recents` uses `<data_dir>/FreeCell`). This deliberately avoids a
//! predictable world-shared `/tmp/FreeCell/Demo.xlsx` — a fixed path in the shared temp root a
//! co-tenant could pre-create or content-swap out from under us (a TOCTOU / DoS smell on a
//! multi-user host). The per-user cache dir is the user's own (not world-writable), and the demo is
//! a regenerable cache artifact, so the cache dir (not the data dir) is its natural home.
//!
//! **Tuning:** replace `assets/demo/demo.xlsx` and rebuild. This module is the single place that
//! points at the asset.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The embedded demo workbook bytes — the single committed demo asset (`assets/demo/demo.xlsx`).
const DEMO_XLSX: &[u8] = include_bytes!("../../assets/demo/demo.xlsx");

/// The name the demo is materialized under. Its stem is what the loading overlay briefly shows
/// ("Opening Demo.xlsx…") before the window settles on its untitled title.
const DEMO_FILE_NAME: &str = "Demo.xlsx";

/// Materializes the embedded demo workbook to an `.xlsx` under the per-user cache dir and returns
/// its path, for the path-based engine loader to open.
///
/// Published atomically: the bytes are written to a unique sibling temp file, then renamed over the
/// destination. A same-filesystem rename is atomic, so an already-open demo window whose worker
/// re-reads the source file (lazy chart discovery, or a save) never observes a half-written file
/// when a second demo open refreshes it. In the per-user cache dir repeated opens reuse the one
/// destination (the bytes are identical + static); the file is intentionally not deleted.
pub(crate) fn materialize_demo_xlsx() -> std::io::Result<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = demo_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(DEMO_FILE_NAME);

    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let staging = dir.join(format!(".demo-{}-{seq}.xlsx.tmp", std::process::id()));
    std::fs::write(&staging, DEMO_XLSX)?;
    match std::fs::rename(&staging, &dest) {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = std::fs::remove_file(&staging);
            Err(e)
        }
    }
}

/// The directory the demo materializes into: the per-user cache dir `<cache_dir>/FreeCell`
/// (falling back to the per-user data dir), so the demo file lives under the user's own directory
/// rather than a predictable world-shared temp path (see the module docs).
///
/// - Linux: `${XDG_CACHE_HOME:-~/.cache}/FreeCell` (then `${XDG_DATA_HOME:-~/.local/share}/FreeCell`)
/// - macOS: `~/Library/Caches/FreeCell` (then `~/Library/Application Support/FreeCell`)
///
/// Fallback: on a headless host where no per-user directory resolves (no `HOME`), a **randomized,
/// per-process** subdir of the temp root — never the fixed shared `<temp>/FreeCell` — so the path
/// still isn't predictable to a co-tenant.
fn demo_dir() -> PathBuf {
    if let Some(base) = dirs::cache_dir().or_else(dirs::data_dir) {
        return base.join("FreeCell");
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ_DIR.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "FreeCell-demo-{}-{nanos}-{seq}",
        std::process::id()
    ))
}

/// Counter for the headless fallback's randomized subdir name (see [`demo_dir`]).
static SEQ_DIR: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded bytes materialize to a real `.xlsx` on disk that our engine can open with the
    /// demo's actual shape — the same load path the demo window drives. Guards the asset staying
    /// present + loadable and its sheet count, so a future content swap that trips an IronCalc gap
    /// or drops a sheet is caught here (a corrupt/removed file also fails the build's
    /// `include_bytes!` or this `open`).
    #[test]
    fn materialized_demo_loads_in_the_engine() {
        let path = materialize_demo_xlsx().expect("demo materializes to an .xlsx");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(DEMO_FILE_NAME),
            "the demo materializes under its display name"
        );
        let doc = freecell_engine::WorkbookDocument::open(&path).expect("the demo workbook loads");
        assert_eq!(
            doc.sheet_count(),
            4,
            "the demo workbook has its four sheets (Sales Overview / Product Catalog / \
             Regional Sales / Quarterly P&L)"
        );
    }

    /// All five embedded charts parse into typed, renderable specs (fidelity check), and the
    /// Quarterly P&L **area** chart carries its four quarter categories. The category assertion is
    /// the real guard for this fix: the area chart's category axis was rewritten from
    /// `multiLvlStrRef` (which our loader ignored → **empty categories**) to a plain `strRef`, so
    /// this asserts the categories now load in C→F order. Note the pre-fix "Unsupported chart type"
    /// placeholder did **not** come from `display_fidelity()` — that is derived from the source-XML
    /// classification and stayed Faithful even with empty categories (so the fidelity assertion was
    /// already true pre-fix). The placeholder came from the **app layer**: `AreaPlot::from_chart`
    /// returns `None` on empty categories, so `in_grid` drew `placeholder_element` for an
    /// otherwise-Faithful chart. Empty categories are exactly what the category assertion catches.
    #[test]
    fn demo_charts_all_render_and_area_categories_load() {
        use freecell_chart_model::{ChartKind, SeriesData};

        let path = materialize_demo_xlsx().expect("demo materializes to an .xlsx");
        let specs = freecell_engine::chart::discover_and_parse(&path)
            .expect("the demo's charts discover + parse");
        assert_eq!(specs.len(), 5, "the demo embeds five charts");
        assert!(
            specs
                .iter()
                .all(|s| s.display_fidelity().renders_as_chart()),
            "every demo chart renders as a chart (none shows the Unsupported placeholder)"
        );

        let area = specs
            .iter()
            .filter_map(|s| s.chart())
            .find(|c| matches!(c.kind, ChartKind::Area { .. }))
            .expect("the Quarterly P&L area chart parses into a typed Area chart");
        for series in &area.series {
            let SeriesData::CategoryValue { categories, .. } = &series.data else {
                panic!("area series should carry category/value data");
            };
            let labels: Vec<String> = categories.iter().map(|c| c.label()).collect();
            assert_eq!(
                labels,
                ["Q1", "Q2", "Q3", "Q4"],
                "the area chart's categories load from C5:F5 in C→F order"
            );
        }
    }

    /// The sheet-3 "Share of Revenue by Region" **pie** classifies **Faithful** — no "⚠ May not
    /// display as intended" compatibility badge. The demo's pie originally carried unsupported
    /// data-label options (per-point `<c:dLbl>` overrides plus series-level non-percent label
    /// kinds — value / category / series / legend-key) that FreeCell's pie renderer does not draw,
    /// so `fidelity::unsupported_data_labels` classified it **Degraded**. The demo's `chart3.xml`
    /// was content-edited to keep only the on-slice **percent** labels the renderer honors, which
    /// clears the badge without changing what FreeCell draws. This guards against a future content
    /// swap silently reintroducing a degrading label option.
    #[test]
    fn demo_pie_chart_classifies_faithful() {
        use freecell_chart_model::{ChartKind, Fidelity};

        let path = materialize_demo_xlsx().expect("demo materializes to an .xlsx");
        let specs = freecell_engine::chart::discover_and_parse(&path)
            .expect("the demo's charts discover + parse");

        let pie = specs
            .iter()
            .find(|s| {
                s.title() == Some("Share of Revenue by Region")
                    && matches!(s.chart().map(|c| &c.kind), Some(ChartKind::Pie { .. }))
            })
            .expect("the Regional Sales pie parses into a typed Pie chart with its title");

        assert_eq!(
            pie.display_fidelity(),
            Fidelity::Faithful,
            "the demo pie must classify Faithful (no compatibility badge), not Degraded"
        );
        assert!(
            !pie.display_fidelity().shows_compatibility_warning(),
            "a Faithful pie shows no '⚠ May not display as intended' badge"
        );
    }
}
