//! P15 — **external round-trip in CI** (charts/implementation_plan P15, architecture §7 real-file
//! corpus / §8 risk 3). Excel can't run in CI, but **LibreOffice can** (`soffice --headless`), so
//! this proves a FreeCell-saved line-chart `.xlsx` survives being read + re-written by a *different*
//! real spreadsheet application — the external half of "save→reopen (Excel + LibreOffice)".
//!
//! The round-trip:
//! 1. **FreeCell saves** the owner's real Excel line workbook through the engine save path
//!    ([`save_with_charts`] — the byte-preserve path the app's Save rides).
//! 2. **Headless LibreOffice** opens that file and converts it back to `.xlsx`
//!    (`--convert-to xlsx`) under an isolated user profile, which must exit 0 and write the output.
//! 3. The **chart part survives**: LibreOffice's re-written `.xlsx` still contains a chart part
//!    that parses back — via our own [`discover_and_parse`] — as a **line** chart. That proves
//!    LibreOffice both *read* our chart and *re-emitted* it (a genuine external round-trip), and
//!    that our loader reads a LibreOffice-authored chart (real-file variety, architecture §8 #4).
//!
//! **Gate policy** (mirrors the render suite's `FREECELL_RENDER`): with `FREECELL_LIBREOFFICE=1`
//! set (the CI job sets it) a missing `soffice` is a HARD failure — a required external-round-trip
//! gate must not silently skip. Without it (a dev box / `cargo test --workspace` with no
//! LibreOffice) the test self-skips with a note, so the workspace stays green.

use std::path::{Path, PathBuf};
use std::process::Command;

use freecell_chart_model::ChartKind;
use freecell_engine::chart::{discover_and_parse, save_with_charts};

fn real_workbook() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/charts/excel_line_chart_workbook.xlsx")
}

/// The LibreOffice CLI binary (`soffice`, or `libreoffice`), or `None` if neither is installed.
/// Probes with `--version` so a broken install (present but unrunnable) also reads as absent.
fn soffice_bin() -> Option<&'static str> {
    for bin in ["soffice", "libreoffice"] {
        let ok = Command::new(bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(bin);
        }
    }
    None
}

/// True when a file's zip package contains a `xl/charts/chartN.xml` part whose bytes carry a
/// `<c:lineChart>` group — a version-robust "the chart part survives" check independent of our
/// own parser (used as a cross-check alongside `discover_and_parse`).
fn zip_has_line_chart_part(path: &Path) -> bool {
    let file = std::fs::File::open(path).expect("open converted xlsx");
    let mut zip = zip::ZipArchive::new(file).expect("read converted xlsx as zip");
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().to_string();
        if name.starts_with("xl/charts/chart") && name.ends_with(".xml") {
            use std::io::Read;
            let mut buf = String::new();
            if entry.read_to_string(&mut buf).is_ok() && buf.contains("lineChart") {
                return true;
            }
        }
    }
    false
}

/// Whether any spec parsed a **line** chart.
fn has_line_chart(specs: &[freecell_chart_model::ChartSpec]) -> bool {
    specs
        .iter()
        .any(|s| matches!(s.chart().map(|c| &c.kind), Some(ChartKind::Line { .. })))
}

#[test]
fn libreoffice_reopens_freecell_saved_line_chart() {
    let require = std::env::var("FREECELL_LIBREOFFICE").ok().as_deref() == Some("1");
    let Some(soffice) = soffice_bin() else {
        if require {
            panic!(
                "FREECELL_LIBREOFFICE=1 but no `soffice`/`libreoffice` on PATH — the external \
                 round-trip gate must not silently skip; install libreoffice-calc"
            );
        }
        eprintln!(
            "libreoffice round-trip skipped: no soffice/libreoffice on PATH \
             (set FREECELL_LIBREOFFICE=1 to require it)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();

    // 1. FreeCell saves the real Excel line workbook through the engine save path.
    let freecell_saved = dir.path().join("freecell_saved.xlsx");
    let report = save_with_charts(&real_workbook(), &freecell_saved).expect("FreeCell save");
    assert!(
        report.charts_preserved >= 1,
        "the FreeCell save must carry the line charts"
    );
    // Sanity: our own loader reads our own save back as line charts.
    let ours = discover_and_parse(&freecell_saved).expect("reopen FreeCell's own save");
    assert!(
        has_line_chart(&ours),
        "FreeCell's saved workbook must still hold a line chart"
    );

    // 2. Headless LibreOffice opens + converts it back to xlsx. An isolated `UserInstallation`
    //    profile keeps CI from tripping on a shared/locked default profile; `--convert-to` runs a
    //    full load → save cycle in a different real app.
    let profile = dir.path().join("lo_profile");
    let out_dir = dir.path().join("lo_out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let user_install = format!("-env:UserInstallation=file://{}", profile.display());

    let output = Command::new(soffice)
        .args([
            "--headless",
            "--nologo",
            "--nofirststartwizard",
            "--norestore",
        ])
        .arg(&user_install)
        .args(["--convert-to", "xlsx", "--outdir"])
        .arg(&out_dir)
        .arg(&freecell_saved)
        .output()
        .expect("spawn soffice");

    assert!(
        output.status.success(),
        "soffice --convert-to failed (exit {:?}):\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let converted = out_dir.join("freecell_saved.xlsx");
    assert!(
        converted.exists(),
        "LibreOffice must write the converted xlsx to {} — soffice stdout: {}",
        converted.display(),
        String::from_utf8_lossy(&output.stdout),
    );

    // 3. The chart part survived the external round-trip. Cross-check with a raw zip scan (version-
    //    robust) AND our own loader (proves the LibreOffice-authored chart is readable by FreeCell).
    assert!(
        zip_has_line_chart_part(&converted),
        "the converted xlsx must still contain a <c:lineChart> chart part (external round-trip lost the chart)"
    );
    let lo_specs =
        discover_and_parse(&converted).expect("parse the LibreOffice-written xlsx with our loader");
    assert!(
        has_line_chart(&lo_specs),
        "the line chart must survive LibreOffice's read+rewrite and reparse as a line chart in FreeCell"
    );
}
