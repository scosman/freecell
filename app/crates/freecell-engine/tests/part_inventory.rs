//! **Part-inventory round-trip** over REAL Excel/LibreOffice files (unit C1 —
//! `projects/architecture-review-remediation.md`).
//!
//! Why this file exists: `tests/roundtrip.rs` round-trips only workbooks FreeCell itself
//! authored (`fixtures::*`). That is a closed loop over IronCalc's own serializer — anything
//! IronCalc cannot *represent* is also absent from the input, so the loop closes perfectly
//! while arbitrary content is being dropped. Meanwhile five real third-party `.xlsx` files sit
//! in `tests/fixtures/` used for **open** assertions only.
//!
//! So this suite opens each real fixture, saves it through a **production** save path, and
//! compares the **part-level inventory** of the two OPC zips. It is a *detector*, not a fix:
//! the preservation work is C3 (v1.0) and the user-facing warning is C2. What this file buys
//! is that the loss is now (a) measured, (b) named in-tree, and (c) non-regressing — a newly
//! dropped part fails the build.
//!
//! **The `DROPPED` lists below are a record of a known defect, not an approval of it.**
//! Growing one is a regression. Shrinking one is a fix — update the list and `GAPS.md`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freecell_core::CellRef;
use freecell_engine::{
    Command, DocumentClient, DocumentSource, WorkbookDocument, WorkerEvent, WorkerEventReceiver,
};
use tempfile::tempdir;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Wait for the first event matching `pred` (bounded, so a stuck worker fails instead of hangs).
fn wait_for(
    rx: &WorkerEventReceiver,
    mut pred: impl FnMut(&WorkerEvent) -> bool,
) -> Option<WorkerEvent> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match rx.recv_timeout(remaining) {
            Some(ev) if pred(&ev) => return Some(ev),
            Some(_) => continue,
            None => return None,
        }
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Every part name in an `.xlsx` (an OPC zip), sorted. Directory entries are skipped — some
/// writers emit them and some don't, and an explicit `xl/` entry is not content.
fn part_names(path: &Path) -> BTreeSet<String> {
    let file = std::fs::File::open(path).expect("fixture opens");
    let mut zip = zip::ZipArchive::new(file).expect("fixture is a zip");
    (0..zip.len())
        .map(|i| {
            let e = zip.by_index(i).expect("entry readable");
            (e.name().to_string(), e.is_dir())
        })
        .filter(|(_, is_dir)| !is_dir)
        .map(|(name, _)| name)
        .collect()
}

/// The three save paths, mirroring what the app actually does:
///
/// - [`SavePath::App`] — spawn the real eval worker over the file and send `Command::Save`, i.e.
///   **exactly** what the shipped app does when the user hits ⌘S. This is the authoritative one;
///   it picks its own strategy internally (chart re-injection when there is something to
///   re-inject, IronCalc's writer otherwise) and writes atomically.
/// - [`SavePath::Plain`] — `WorkbookDocument::save`, IronCalc's writer straight to disk. The
///   engine's own file-I/O surface, and the strategy the app falls back to for a workbook with
///   nothing to re-inject. Isolating it shows what the serializer alone preserves.
/// - [`SavePath::PreserveCharts`] — `chart::save_with_charts`, the no-edit form of the
///   byte-preserve path: reload the original, run IronCalc's writer, splice the original zip's
///   chart/drawing parts and content-type overrides back in.
///
/// Running more than one is deliberate: the *difference* between their inventories is a direct
/// measure of what the re-injection machinery buys over the bare serializer.
#[derive(Clone, Copy, Debug)]
enum SavePath {
    App { edit: bool },
    Plain,
    PreserveCharts,
}

impl SavePath {
    fn run(self, original: &Path, out: &Path) {
        match self {
            SavePath::App { edit } => save_through_worker(original, out, edit),
            SavePath::Plain => {
                let doc = WorkbookDocument::open(original).expect("fixture opens");
                doc.save(out).expect("save succeeds");
            }
            SavePath::PreserveCharts => {
                freecell_engine::chart::save_with_charts(original, out)
                    .expect("chart-preserving save succeeds");
            }
        }
    }
}

/// Open `original` through the real worker, optionally type into A1, and save to `out` — the
/// production ⌘S path end to end.
fn save_through_worker(original: &Path, out: &Path, edit: bool) {
    let (client, rx) = DocumentClient::spawn(DocumentSource::OpenFile(original.to_path_buf()));
    let loaded = wait_for(&rx, |e| matches!(e, WorkerEvent::Loaded { .. }))
        .expect("worker emits Loaded for a real fixture");
    let WorkerEvent::Loaded { sheets } = loaded else {
        unreachable!()
    };

    if edit {
        client.send(Command::SetCellInput {
            sheet: sheets[0].id,
            cell: CellRef::new(0, 0),
            input: "c1-probe".to_string(),
        });
    }

    client.send(Command::Save {
        path: out.to_path_buf(),
        req_id: 1,
    });
    wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 1, .. }))
        .expect("worker emits Saved");
    assert!(out.exists(), "the worker save produced no file");
}

/// What an open→save→reopen of one fixture actually did to the package.
struct Inventory {
    dropped: BTreeSet<String>,
    added: BTreeSet<String>,
}

/// Open `fixture`, save it through `path`, reopen the result, and diff the two zips' part names.
fn round_trip(name: &str, path: SavePath) -> Inventory {
    let original = fixture(name);
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("saved.xlsx");

    path.run(&original, &out);

    // Independent of the inventory: the file we wrote must still be a workbook.
    WorkbookDocument::open(&out).unwrap_or_else(|e| {
        panic!("{name}: the file we just saved does not reopen ({path:?}): {e:?}")
    });

    let before = part_names(&original);
    let after = part_names(&out);
    Inventory {
        dropped: before.difference(&after).cloned().collect(),
        added: after.difference(&before).cloned().collect(),
    }
}

/// Assert the measured drop set equals the committed baseline, in BOTH directions.
///
/// - a **new** name in `dropped` is a regression: we started losing something we used to keep;
/// - a name that **disappears** from `dropped` is a fix, and the baseline must be updated so the
///   record stays true (and `GAPS.md` with it).
///
/// The failure output names the parts, because that is the entire value of this suite: a human
/// reading a CI failure has to be able to see *what* is now being lost without re-running
/// anything.
fn assert_dropped(label: &str, inv: &Inventory, expected: &[&str]) {
    let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
    if inv.dropped == expected {
        return;
    }
    let newly: Vec<_> = inv.dropped.difference(&expected).collect();
    let no_longer: Vec<_> = expected.difference(&inv.dropped).collect();
    panic!(
        "{label}: the set of parts dropped by save changed.\n\
         \n  NEWLY DROPPED (a regression — content we used to preserve is now lost):\n{}\
         \n  NO LONGER DROPPED (a fix — update the baseline in this file and GAPS.md):\n{}\
         \n  parts the writer ADDED (context only, not asserted):\n{}",
        fmt_list(newly.into_iter()),
        fmt_list(no_longer.into_iter()),
        fmt_list(inv.added.iter()),
    );
}

fn fmt_list<'a>(it: impl Iterator<Item = &'a String>) -> String {
    let v: Vec<&String> = it.collect();
    if v.is_empty() {
        return "    (none)\n".to_string();
    }
    v.iter().map(|n| format!("    {n}\n")).collect()
}

// ---------------------------------------------------------------------------------------
// Baselines — the measured, committed record of what each fixture loses on save.
//
// These are NOT an allow-list of acceptable loss. Every name here is content that was in the
// user's file and is not in the file we wrote back. The fix is C3 (default-carry / explicit-drop
// preservation model, v1.0); the user-facing warning is C2. See GAPS.md.
// ---------------------------------------------------------------------------------------

/// **Dropped by every save, on every fixture measured.** `docProps/custom.xml` holds the
/// workbook's user-defined document properties (the "Custom" tab of Excel's Info pane, and where
/// SharePoint/DMS systems stash document metadata). IronCalc's writer emits `docProps/app.xml` +
/// `docProps/core.xml` and has no model for custom properties, so they are silently discarded.
const CUSTOM_PROPS: &str = "docProps/custom.xml";

/// `personal_monthly_budget.xlsx` — a real Excel template. **27 parts lost**, and the most
/// alarming result in this file:
///
/// - **`xl/tables/table1..12.xml` — all twelve table (ListObject) definitions.** These are the
///   workbook's actual structure: banded formatting, header/total rows, filter buttons, and the
///   structured references formulas use by name. Open this budget template in FreeCell, save, and
///   reopen it in Excel and every table has become a plain range.
/// - **`customXml/*` (3 items + their rels + itemProps)** — custom XML parts. In an Excel
///   *template* these commonly carry the data bindings and content-type metadata that make the
///   template a template.
/// - **`xl/printerSettings/*.bin`** — page setup / printer configuration.
/// - **`xl/worksheets/_rels/sheet{1,2}.xml.rels`** — the relationship parts binding each sheet to
///   its tables and printer settings. They go because their targets do.
/// - `xl/calcChain.xml` — **benign**: a formula-dependency cache Excel rebuilds on open. Listed
///   because this file records what changed, not what matters; it is the one entry here that
///   costs the user nothing.
const PMB_DROPPED: &[&str] = &[
    "customXml/_rels/item1.xml.rels",
    "customXml/_rels/item2.xml.rels",
    "customXml/_rels/item3.xml.rels",
    "customXml/item1.xml",
    "customXml/item2.xml",
    "customXml/item3.xml",
    "customXml/itemProps1.xml",
    "customXml/itemProps2.xml",
    "customXml/itemProps3.xml",
    CUSTOM_PROPS,
    "xl/calcChain.xml",
    "xl/printerSettings/printerSettings1.bin",
    "xl/printerSettings/printerSettings2.bin",
    "xl/tables/table1.xml",
    "xl/tables/table10.xml",
    "xl/tables/table11.xml",
    "xl/tables/table12.xml",
    "xl/tables/table2.xml",
    "xl/tables/table3.xml",
    "xl/tables/table4.xml",
    "xl/tables/table5.xml",
    "xl/tables/table6.xml",
    "xl/tables/table7.xml",
    "xl/tables/table8.xml",
    "xl/tables/table9.xml",
    "xl/worksheets/_rels/sheet1.xml.rels",
    "xl/worksheets/_rels/sheet2.xml.rels",
];

/// `dates.xlsx` — nothing dropped. A small, plain workbook is fully covered by IronCalc's model.
const DATES_DROPPED: &[&str] = &[];

/// `numbers_table.xlsx` — nothing dropped.
const NUMBERS_DROPPED: &[&str] = &[];

/// `FONTS.xlsx` — nothing dropped.
const FONTS_DROPPED: &[&str] = &[];

/// `libreoffice_custom_height_wrap.xlsx` — written by LibreOffice, not Excel. Only the custom
/// document properties are lost.
const LIBREOFFICE_DROPPED: &[&str] = &[CUSTOM_PROPS];

/// `charts/excel_line_chart_workbook.xlsx` through the real app save path. Four charts, their
/// drawing, and the sheet→drawing relationship all survive — the re-injection machinery works.
/// Compare against [`CHART_PLAIN_DROPPED`].
const CHART_APP_DROPPED: &[&str] = &[CUSTOM_PROPS];

/// The same file through the PLAIN serializer, with re-injection NOT applied — **all four charts,
/// the drawing, and its relationships are gone.** The delta against [`CHART_APP_DROPPED`] is
/// exactly what the chart-preserving path buys, and it is the strongest evidence in this file that
/// "IronCalc's writer plus targeted re-injection" is the right shape for the C3 fix: the same
/// machinery generalises from charts to tables.
const CHART_PLAIN_DROPPED: &[&str] = &[
    CUSTOM_PROPS,
    "xl/charts/chart1.xml",
    "xl/charts/chart2.xml",
    "xl/charts/chart3.xml",
    "xl/charts/chart4.xml",
    "xl/drawings/_rels/drawing1.xml.rels",
    "xl/drawings/drawing1.xml",
    "xl/worksheets/_rels/sheet2.xml.rels",
];

/// The same file through `save_with_charts` (the no-edit byte-preserve entry point) — agrees with
/// the app path, as it should.
const CHART_PRESERVED_DROPPED: &[&str] = &[CUSTOM_PROPS];

#[test]
fn personal_monthly_budget_part_inventory() {
    let inv = round_trip(
        "personal_monthly_budget.xlsx",
        SavePath::App { edit: false },
    );
    assert_dropped("personal_monthly_budget.xlsx [app]", &inv, PMB_DROPPED);
}

#[test]
fn dates_part_inventory() {
    let inv = round_trip("dates.xlsx", SavePath::App { edit: false });
    assert_dropped("dates.xlsx [app]", &inv, DATES_DROPPED);
}

#[test]
fn numbers_table_part_inventory() {
    let inv = round_trip("numbers_table.xlsx", SavePath::App { edit: false });
    assert_dropped("numbers_table.xlsx [app]", &inv, NUMBERS_DROPPED);
}

#[test]
fn fonts_part_inventory() {
    let inv = round_trip("FONTS.xlsx", SavePath::App { edit: false });
    assert_dropped("FONTS.xlsx [app]", &inv, FONTS_DROPPED);
}

#[test]
fn libreoffice_part_inventory() {
    let inv = round_trip(
        "libreoffice_custom_height_wrap.xlsx",
        SavePath::App { edit: false },
    );
    assert_dropped(
        "libreoffice_custom_height_wrap.xlsx [app]",
        &inv,
        LIBREOFFICE_DROPPED,
    );
}

#[test]
fn chart_workbook_part_inventory_app() {
    let inv = round_trip(
        "charts/excel_line_chart_workbook.xlsx",
        SavePath::App { edit: false },
    );
    assert_dropped(
        "charts/excel_line_chart_workbook.xlsx [app]",
        &inv,
        CHART_APP_DROPPED,
    );
}

/// The bare serializer on the same chart workbook, with no re-injection. The delta against
/// [`CHART_APP_DROPPED`] is exactly what the chart-preserving save path buys.
#[test]
fn chart_workbook_part_inventory_plain() {
    let inv = round_trip("charts/excel_line_chart_workbook.xlsx", SavePath::Plain);
    assert_dropped(
        "charts/excel_line_chart_workbook.xlsx [plain — no re-injection]",
        &inv,
        CHART_PLAIN_DROPPED,
    );
}

/// `save_with_charts`, the no-edit byte-preserve entry point. Should agree with the app path on
/// an unedited workbook; if it ever diverges, one of the two is not doing what it claims.
#[test]
fn chart_workbook_part_inventory_preserved() {
    let inv = round_trip(
        "charts/excel_line_chart_workbook.xlsx",
        SavePath::PreserveCharts,
    );
    assert_dropped(
        "charts/excel_line_chart_workbook.xlsx [save_with_charts]",
        &inv,
        CHART_PRESERVED_DROPPED,
    );
}

/// The loss is a property of the **writer**, not of whether the model was touched — so one
/// fixture is saved both clean and after an edit to prove the inventory is identical either way.
/// Running every fixture twice would double a slow suite for no extra information.
#[test]
fn an_edit_does_not_change_what_is_dropped() {
    let clean = round_trip(
        "personal_monthly_budget.xlsx",
        SavePath::App { edit: false },
    );
    let edited = round_trip("personal_monthly_budget.xlsx", SavePath::App { edit: true });
    assert_eq!(
        clean.dropped, edited.dropped,
        "editing a cell changed which parts survive the save — the loss is supposed to be a \
         property of the serializer, so this means something path-dependent is going on",
    );
}
