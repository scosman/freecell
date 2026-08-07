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
//! compares two *package-level* inventories of the two OPC zips:
//!
//! 1. the set of **part names** (zip entries), and
//! 2. the set of **`[Content_Types].xml` declarations** (`<Default>` / `<Override>`).
//!
//! (2) is not redundant with (1): a part can survive while its `<Override PartName=…>` does
//! not, and Excel then reports the file as needing repair or silently ignores the part. That is
//! precisely the failure mode `chart::save_with_charts` splices the overrides back to avoid, so
//! it is the one C3 must not regress.
//!
//! **SCOPE — this detector is package-level only.** It answers "which parts and content-type
//! declarations survive", *not* "does the surviving part still say what it said". Content
//! dropped **inside** a surviving part is invisible here, and it is substantial. Measured
//! 2026-08-07 over these same six fixtures, on the same app save path:
//!
//! - **every fixture that has one loses `pageSetup` + `pageMargins`** out of *every*
//!   `xl/worksheets/sheetN.xml` — 5 of 6 (only `dates.xlsx` has none to lose) — usually along
//!   with `headerFooter`, `printOptions`, `sheetPr`/`pageSetUpPr` and `sheetFormatPr`;
//! - `personal_monthly_budget.xlsx` and both LibreOffice-written fixtures lose `xl/workbook.xml`'s
//!   `extLst` and most of that part's bytes (2227 → 448 and 1102 → 560 respectively);
//! - `libreoffice_custom_height_wrap.xlsx` loses ~35% of each sheet part (`sheet1.xml`
//!   10186 → 6665 bytes) while this detector records it as dropping one part;
//! - `personal_monthly_budget.xlsx`'s `sheet2.xml` loses all twelve `<tablePart>` references and
//!   its `ignoredErrors` — the in-part shadow of the twelve dropped table parts;
//! - `xl/styles.xml` loses `indexedColors` / `tableStyles` / `protection`, `docProps/app.xml`
//!   loses `Company` / `Manager` / `Template`, and `xl/theme/theme1.xml` is replaced wholesale
//!   with IronCalc's default theme (`FONTS.xlsx`: 21716 → 8716 bytes).
//!
//! A fixture recorded below as dropping **no parts** is therefore **not** "fully preserved" —
//! see `GAPS.md`. Measuring in-part loss is a bigger job (it needs a semantic XML diff per part
//! type, not a name-set diff) and belongs with C3, not here.
//!
//! It is a *detector*, not a fix: the preservation work is C3 (v1.0) and the user-facing
//! warning is C2. What this file buys is that the part-level loss is now (a) measured, (b) named
//! in-tree, and (c) non-regressing — a newly dropped part fails the build.
//!
//! **The `DROPPED` lists below are a record of a known defect, not an approval of it.**
//! Growing one is a regression. Shrinking one is a fix — update the list and `GAPS.md`.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freecell_core::CellRef;
use freecell_engine::{
    Command, DocumentClient, DocumentSource, WorkbookDocument, WorkerEvent, WorkerEventReceiver,
};
use tempfile::tempdir;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Wait for the first event matching `pred` (bounded, so a stuck worker fails instead of hangs).
///
/// NOTE: `TIMEOUT` + `wait_for` are a verbatim copy of `tests/worker_seam.rs:20-42`. Two
/// integration-test binaries cannot share code without introducing a `tests/common/mod.rs`, and
/// two copies of fourteen lines does not earn that. **A third copy should trigger the
/// extraction.**
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

fn open_zip(path: &Path) -> zip::ZipArchive<std::fs::File> {
    let file = std::fs::File::open(path).expect("fixture opens");
    zip::ZipArchive::new(file).expect("fixture is a zip")
}

/// Every part name in an `.xlsx` (an OPC zip), sorted. Directory entries are skipped — some
/// writers emit them and some don't, and an explicit `xl/` entry is not content.
fn part_names(path: &Path) -> BTreeSet<String> {
    let mut zip = open_zip(path);
    (0..zip.len())
        .map(|i| {
            let e = zip.by_index(i).expect("entry readable");
            (e.name().to_string(), e.is_dir())
        })
        .filter(|(_, is_dir)| !is_dir)
        .map(|(name, _)| name)
        .collect()
}

/// Every declaration in `[Content_Types].xml`, rendered one per line as
/// `Default .<ext> -> <content-type>` / `Override <part-name> -> <content-type>`.
///
/// This is the half of the package [`part_names`] cannot see. OPC requires every part to be
/// typed, either by its extension (`Default`) or by name (`Override`). A save that carries a
/// part across but loses its `Override` produces a file Excel offers to "repair" — the part is
/// present and unreachable. `chart::save_with_charts` splices the chart overrides back
/// deliberately for exactly this reason, so without this check
/// `chart_workbook_part_inventory_preserved` would stay green on a regression that dropped them.
///
/// The content type is included, not just the part name, so a *retyped* part is caught too.
fn content_type_decls(path: &Path) -> BTreeSet<String> {
    let mut zip = open_zip(path);
    let mut xml = String::new();
    zip.by_name("[Content_Types].xml")
        .expect("an OPC package has [Content_Types].xml")
        .read_to_string(&mut xml)
        .expect("[Content_Types].xml is readable text");
    let doc = roxmltree::Document::parse(&xml).expect("[Content_Types].xml parses");
    doc.root_element()
        .children()
        .filter(|n| n.is_element())
        .filter_map(|n| match n.tag_name().name() {
            "Default" => Some(format!(
                "Default .{} -> {}",
                n.attribute("Extension").unwrap_or("?"),
                n.attribute("ContentType").unwrap_or("?"),
            )),
            "Override" => Some(format!(
                "Override {} -> {}",
                n.attribute("PartName").unwrap_or("?"),
                n.attribute("ContentType").unwrap_or("?"),
            )),
            _ => None,
        })
        .collect()
}

/// Does any part of the package contain `needle`? Used to prove the edited variant's edit
/// actually reached the file.
fn package_contains(path: &Path, needle: &str) -> bool {
    let mut zip = open_zip(path);
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).expect("entry readable");
        if e.is_dir() {
            continue;
        }
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).expect("entry body readable");
        if buf.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            return true;
        }
    }
    false
}

/// The three save paths, mirroring what the app actually does:
///
/// - [`SavePath::App`] — spawn the real eval worker over the file and send `Command::Save`, i.e.
///   the same engine path the shipped app takes when the user hits ⌘S
///   (`shell/window.rs:991` → `worker/run.rs:559` → `save_workbook`). This is the authoritative
///   one; it picks its own strategy internally (chart re-injection when there is something to
///   re-inject, IronCalc's writer otherwise) and writes atomically.
///
///   Two things the *app* does that this does not, neither of which can move the inventory: it
///   writes a one-time `.back` copy of the original first (`shell/window.rs:981`), and a plain
///   ⌘S normally targets the file already open rather than a fresh path. Saving to a fresh path
///   is what lets this test diff the two packages.
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

/// The probe string the edited variant types into a cell. Distinctive enough to grep the saved
/// package for.
const EDIT_PROBE: &str = "c1-probe";

/// Open `original` through the real worker, optionally type into a cell, and save to `out` — the
/// production ⌘S path end to end.
///
/// When `edit` is set the edit is **forced and asserted**, per CLAUDE.md's benchmark/measurement
/// rule: the worker must report exactly one committed op, and the probe string must be findable
/// in the package we wrote. Without that, a silently-rejected edit (a guard, a stale sheet id, a
/// refused input) would make `an_edit_does_not_change_what_is_dropped` pass *more* easily and its
/// claim vacuous.
fn save_through_worker(original: &Path, out: &Path, edit: bool) {
    let (client, rx) = DocumentClient::spawn(DocumentSource::OpenFile(original.to_path_buf()));
    let loaded = wait_for(&rx, |e| matches!(e, WorkerEvent::Loaded { .. }))
        .expect("worker emits Loaded for a real fixture");
    let WorkerEvent::Loaded { sheets } = loaded else {
        unreachable!()
    };

    if edit {
        // Deliberately NOT sheet 0: on `personal_monthly_budget.xlsx` (the only fixture that
        // takes this branch) sheet 0 is the "START" cover sheet and every one of the twelve
        // tables lives on sheet 1. Editing the table-bearing sheet probes the interesting case.
        let sheet = sheets.get(1).unwrap_or_else(|| {
            panic!("the edited fixture must have a second sheet; got {sheets:?}")
        });
        client.send(Command::SetCellInput {
            sheet: sheet.id,
            cell: CellRef::new(0, 0),
            input: EDIT_PROBE.to_string(),
        });
    }

    client.send(Command::Save {
        path: out.to_path_buf(),
        req_id: 1,
    });
    let saved = wait_for(&rx, |e| {
        matches!(
            e,
            WorkerEvent::Saved { req_id: 1, .. } | WorkerEvent::SaveFailed { req_id: 1, .. }
        )
    })
    // Matching SaveFailed too matters: otherwise a failed save burns the whole 30s TIMEOUT and
    // then blames the worker for not emitting `Saved`, which points the reader at the wrong bug.
    .expect("worker emits neither Saved nor SaveFailed within the timeout");
    match saved {
        WorkerEvent::Saved { ops_seen, .. } => {
            let expected = u64::from(edit);
            assert_eq!(
                ops_seen,
                expected,
                "the worker committed {ops_seen} ops, expected {expected} — the {} save did not \
                 do what this test assumes",
                if edit { "edited" } else { "clean" },
            );
        }
        WorkerEvent::SaveFailed { error, .. } => panic!("the worker save failed: {error:?}"),
        _ => unreachable!(),
    }

    if edit {
        assert!(
            package_contains(out, EDIT_PROBE),
            "the edit never reached the saved package: {EDIT_PROBE:?} is in none of its parts",
        );
    }
}

/// What an open→save→reopen of one fixture actually did to the package.
struct Inventory {
    dropped: BTreeSet<String>,
    added: BTreeSet<String>,
    ct_dropped: BTreeSet<String>,
    ct_added: BTreeSet<String>,
}

/// Open `fixture`, save it through `path`, reopen the result, and diff the two zips' part names
/// and content-type declarations.
fn round_trip(name: &str, path: SavePath) -> Inventory {
    let original = fixture(name);
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("saved.xlsx");

    path.run(&original, &out);

    // Independent of the inventory: the file we wrote must still be a workbook. (This also
    // subsumes an existence check on `out` — a missing file fails here, harder and louder.)
    WorkbookDocument::open(&out).unwrap_or_else(|e| {
        panic!("{name}: the file we just saved does not reopen ({path:?}): {e:?}")
    });

    let before = part_names(&original);
    let after = part_names(&out);
    let ct_before = content_type_decls(&original);
    let ct_after = content_type_decls(&out);
    Inventory {
        dropped: before.difference(&after).cloned().collect(),
        added: after.difference(&before).cloned().collect(),
        ct_dropped: ct_before.difference(&ct_after).cloned().collect(),
        ct_added: ct_after.difference(&ct_before).cloned().collect(),
    }
}

/// Assert the measured drop set equals the committed baseline, in BOTH directions.
///
/// - a **new** name in the drop set is a regression: we started losing something we used to keep;
/// - a name that **disappears** from it is a fix, and the baseline must be updated so the record
///   stays true (and `GAPS.md` with it).
///
/// The failure output names the entries, because that is the entire value of this suite: a human
/// reading a CI failure has to be able to see *what* is now being lost without re-running
/// anything.
fn assert_drop_set(
    label: &str,
    what: &str,
    dropped: &BTreeSet<String>,
    expected: &[&str],
    added: &BTreeSet<String>,
) {
    let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
    if *dropped == expected {
        return;
    }
    let newly: Vec<_> = dropped.difference(&expected).collect();
    let no_longer: Vec<_> = expected.difference(dropped).collect();
    panic!(
        "{label}: the set of {what} dropped by save changed.\n\
         \n  NEWLY DROPPED (a regression — content we used to preserve is now lost):\n{}\
         \n  NO LONGER DROPPED (a fix — update the baseline in this file and GAPS.md):\n{}\
         \n  entries the writer ADDED (context only, not asserted):\n{}",
        fmt_list(newly.into_iter()),
        fmt_list(no_longer.into_iter()),
        fmt_list(added.iter()),
    );
}

fn assert_dropped(label: &str, inv: &Inventory, expected: &[&str]) {
    assert_drop_set(label, "parts", &inv.dropped, expected, &inv.added);
}

fn assert_ct_dropped(label: &str, inv: &Inventory, expected: &[&str]) {
    assert_drop_set(
        label,
        "[Content_Types].xml declarations",
        &inv.ct_dropped,
        expected,
        &inv.ct_added,
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
//
// Two baselines per fixture: `*_DROPPED` (part names) and `*_CT_DROPPED`
// ([`Content_Types`] declarations).
// ---------------------------------------------------------------------------------------

/// Dropped by every save on every fixture **that has one** — 3 of the 6 measured
/// (`personal_monthly_budget`, `libreoffice_custom_height_wrap`,
/// `charts/excel_line_chart_workbook`). `dates`, `numbers_table` and `FONTS` contain no
/// `docProps/custom.xml` to lose.
///
/// It holds the workbook's user-defined document properties (the "Custom" tab of Excel's Info
/// pane, and where SharePoint/DMS systems stash document metadata). IronCalc's writer emits
/// `docProps/app.xml` + `docProps/core.xml` and has no model for custom properties, so they are
/// silently discarded.
const CUSTOM_PROPS: &str = "docProps/custom.xml";

/// The `[Content_Types].xml` counterpart of [`CUSTOM_PROPS`] — the part and its declaration go
/// together, as they should.
const CUSTOM_PROPS_CT: &str = "Override /docProps/custom.xml -> \
     application/vnd.openxmlformats-officedocument.custom-properties+xml";

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
///
/// 27 is a **floor**, not the total loss: on top of these parts, `sheet2.xml` loses all twelve
/// `<tablePart>` references and its `ignoredErrors`, `sheet1.xml` goes 1869 → 1274 bytes, and
/// `xl/workbook.xml` goes 2227 → 448 with its `extLst` gone. See the SCOPE note at the top.
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

/// The declarations that go with [`PMB_DROPPED`]. They track the dropped parts one-for-one — the
/// twelve tables, the three `customXml` itemProps, `calcChain`, `custom.xml`, and the `.bin`
/// `Default` that typed the printer settings. Nothing is orphaned in either direction here, which
/// is the *good* case: the writer drops parts and their typing together.
const PMB_CT_DROPPED: &[&str] = &[
    "Default .bin -> application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings",
    "Override /customXml/itemProps1.xml -> \
     application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
    "Override /customXml/itemProps2.xml -> \
     application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
    "Override /customXml/itemProps3.xml -> \
     application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
    CUSTOM_PROPS_CT,
    "Override /xl/calcChain.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml",
    "Override /xl/tables/table1.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table10.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table11.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table12.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table2.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table3.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table4.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table5.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table6.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table7.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table8.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    "Override /xl/tables/table9.xml -> \
     application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
];

/// `dates.xlsx` — **no whole parts dropped**, and no content-type declarations lost either.
///
/// This is NOT "fully preserved": see the SCOPE note at the top of this file. Even here the
/// worksheet's inline strings (`<is>`/`<t>`) are rewritten into `xl/sharedStrings.xml` — a
/// representation change, not a loss, but a reminder that part-level parity says nothing about
/// part contents.
const DATES_DROPPED: &[&str] = &[];
const DATES_CT_DROPPED: &[&str] = &[];

/// The nine `<Default>` media-type declarations Apple Numbers' exporter writes for extensions the
/// package does not actually contain (there is no `.mov` or `.pdf` part in either fixture). Losing
/// them costs nothing today — but they are recorded, because "the writer rebuilds
/// `[Content_Types].xml` from its own model rather than carrying the original's" is exactly the
/// behaviour that also loses the declarations that *do* matter (see `CHART_PLAIN_CT_DROPPED`).
const NUMBERS_MEDIA_DEFAULTS: &[&str] = &[
    "Default .bmp -> image/bmp",
    "Default .gif -> image/gif",
    "Default .jpeg -> image/jpg",
    "Default .mov -> application/movie",
    "Default .pdf -> application/pdf",
    "Default .png -> image/png",
    "Default .tif -> image/tif",
    "Default .vml -> application/vnd.openxmlformats-officedocument.vmlDrawing",
    "Default .xlsx -> application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
];

/// `numbers_table.xlsx` — **no whole parts dropped** (see the SCOPE note: its `sheet1.xml` still
/// loses `pageSetup`, `pageMargins`, `headerFooter`, `pane` and `sheetFormatPr`, and its 21 KB
/// theme is replaced by IronCalc's 8 KB default).
const NUMBERS_DROPPED: &[&str] = &[];

/// `FONTS.xlsx` — **no whole parts dropped** (same content-level caveat as `numbers_table`: its
/// `sheet1.xml` goes 5364 → 5350 bytes with `pageSetup` / `pageMargins` / `headerFooter` / `pane` /
/// `sheetFormatPr` gone).
const FONTS_DROPPED: &[&str] = &[];

/// `libreoffice_custom_height_wrap.xlsx` — written by LibreOffice, not Excel. Only the custom
/// document properties are lost **as a part**. (It also carries five charts and four drawings,
/// which the app path's re-injection preserves — the same machinery `CHART_APP_DROPPED` measures.)
///
/// Do not read this as "LibreOffice files survive": at the content level this is the worst
/// fixture measured. Each of its four `xl/worksheets/sheetN.xml` parts loses ~35% of its bytes
/// (`sheet1.xml` 10186 → 6665: `pageSetup`, `printOptions`, `pageMargins`, `headerFooter`,
/// `sheetPr`), and `xl/workbook.xml` goes 1102 → 560 bytes with its `extLst` gone. None of that is
/// visible to a part-level detector.
const LIBREOFFICE_DROPPED: &[&str] = &[CUSTOM_PROPS];

/// The two LibreOffice-written fixtures declare `image/jpeg` + `image/png` `Default`s for media
/// they do not contain; IronCalc's writer does not reproduce them. Harmless, recorded for the same
/// reason as [`NUMBERS_MEDIA_DEFAULTS`].
const LO_MEDIA_DEFAULTS: &[&str] = &["Default .jpeg -> image/jpeg", "Default .png -> image/png"];

/// `libreoffice_custom_height_wrap.xlsx`'s declaration loss.
///
/// The six `Override … -> …relationships+xml` entries are **benign, and the one category in these
/// baselines that genuinely costs nothing**: LibreOffice types every `.rels` part twice, once by
/// the `Default Extension="rels"` rule and again by an explicit `Override`. IronCalc keeps the
/// `Default` and drops the redundant `Override`, so every `.rels` part stays correctly typed. They
/// are listed anyway because this file records what *changed*, not what mattered.
const LIBREOFFICE_CT_DROPPED: &[&str] = &[
    LO_MEDIA_DEFAULTS[0],
    LO_MEDIA_DEFAULTS[1],
    CUSTOM_PROPS_CT,
    "Override /_rels/.rels -> application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/_rels/workbook.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/worksheets/_rels/sheet1.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/worksheets/_rels/sheet2.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/worksheets/_rels/sheet3.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/worksheets/_rels/sheet4.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
];

/// `charts/excel_line_chart_workbook.xlsx` through the real app save path. Four charts, their
/// drawing, and the sheet→drawing relationship all survive — the re-injection machinery works.
/// Compare against [`CHART_PLAIN_DROPPED`].
const CHART_APP_DROPPED: &[&str] = &[CUSTOM_PROPS];

/// The declarations lost on the app path: the media `Default`s and the redundant `.rels`
/// `Override`s this LibreOffice-written fixture shares with `libreoffice_custom_height_wrap`, plus
/// `custom.xml`. **No chart or drawing override is here** — that is the point.
const CHART_APP_CT_DROPPED: &[&str] = &[
    LO_MEDIA_DEFAULTS[0],
    LO_MEDIA_DEFAULTS[1],
    CUSTOM_PROPS_CT,
    "Override /_rels/.rels -> application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/_rels/workbook.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/worksheets/_rels/sheet2.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
];

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

/// The plain path's declaration loss: everything [`CHART_APP_CT_DROPPED`] loses, **plus the four
/// chart overrides and the drawing's**. The re-injection code splices those back by hand
/// (`chart/save.rs`), so this pair is the direct measurement of that splice working.
const CHART_PLAIN_CT_DROPPED: &[&str] = &[
    LO_MEDIA_DEFAULTS[0],
    LO_MEDIA_DEFAULTS[1],
    CUSTOM_PROPS_CT,
    "Override /_rels/.rels -> application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/_rels/workbook.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/charts/chart1.xml -> \
     application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    "Override /xl/charts/chart2.xml -> \
     application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    "Override /xl/charts/chart3.xml -> \
     application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    "Override /xl/charts/chart4.xml -> \
     application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    "Override /xl/drawings/_rels/drawing1.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
    "Override /xl/drawings/drawing1.xml -> \
     application/vnd.openxmlformats-officedocument.drawing+xml",
    "Override /xl/worksheets/_rels/sheet2.xml.rels -> \
     application/vnd.openxmlformats-package.relationships+xml",
];

/// The same file through `save_with_charts` (the no-edit byte-preserve entry point) — agrees with
/// the app path, as it should.
const CHART_PRESERVED_DROPPED: &[&str] = &[CUSTOM_PROPS];
const CHART_PRESERVED_CT_DROPPED: &[&str] = CHART_APP_CT_DROPPED;

#[test]
fn personal_monthly_budget_part_inventory() {
    let inv = round_trip(
        "personal_monthly_budget.xlsx",
        SavePath::App { edit: false },
    );
    assert_dropped("personal_monthly_budget.xlsx [app]", &inv, PMB_DROPPED);
    assert_ct_dropped("personal_monthly_budget.xlsx [app]", &inv, PMB_CT_DROPPED);
}

#[test]
fn dates_part_inventory() {
    let inv = round_trip("dates.xlsx", SavePath::App { edit: false });
    assert_dropped("dates.xlsx [app]", &inv, DATES_DROPPED);
    assert_ct_dropped("dates.xlsx [app]", &inv, DATES_CT_DROPPED);
}

#[test]
fn numbers_table_part_inventory() {
    let inv = round_trip("numbers_table.xlsx", SavePath::App { edit: false });
    assert_dropped("numbers_table.xlsx [app]", &inv, NUMBERS_DROPPED);
    assert_ct_dropped("numbers_table.xlsx [app]", &inv, NUMBERS_MEDIA_DEFAULTS);
}

#[test]
fn fonts_part_inventory() {
    let inv = round_trip("FONTS.xlsx", SavePath::App { edit: false });
    assert_dropped("FONTS.xlsx [app]", &inv, FONTS_DROPPED);
    assert_ct_dropped("FONTS.xlsx [app]", &inv, NUMBERS_MEDIA_DEFAULTS);
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
    assert_ct_dropped(
        "libreoffice_custom_height_wrap.xlsx [app]",
        &inv,
        LIBREOFFICE_CT_DROPPED,
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
    assert_ct_dropped(
        "charts/excel_line_chart_workbook.xlsx [app]",
        &inv,
        CHART_APP_CT_DROPPED,
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
    assert_ct_dropped(
        "charts/excel_line_chart_workbook.xlsx [plain — no re-injection]",
        &inv,
        CHART_PLAIN_CT_DROPPED,
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
    assert_ct_dropped(
        "charts/excel_line_chart_workbook.xlsx [save_with_charts]",
        &inv,
        CHART_PRESERVED_CT_DROPPED,
    );
}

/// The headline finding of this file, asserted **as a relation** rather than left implicit across
/// three independently-maintained constants.
///
/// Each of `CHART_APP_*` / `CHART_PLAIN_*` is pinned to a real measurement by its own test above,
/// so this adds no new measurement — what it adds is that a careless simultaneous edit of both
/// baselines cannot quietly erase the result while staying green. If re-injection ever stops
/// buying anything, this fails and says so in one line.
#[test]
fn chart_reinjection_strictly_reduces_the_loss() {
    let set = |v: &[&str]| -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() };

    for (what, app, plain, must_include) in [
        (
            "parts",
            set(CHART_APP_DROPPED),
            set(CHART_PLAIN_DROPPED),
            vec![
                "xl/charts/chart1.xml",
                "xl/drawings/drawing1.xml",
                "xl/drawings/_rels/drawing1.xml.rels",
            ],
        ),
        (
            "[Content_Types].xml declarations",
            set(CHART_APP_CT_DROPPED),
            set(CHART_PLAIN_CT_DROPPED),
            vec![
                "Override /xl/charts/chart1.xml -> \
                 application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
                "Override /xl/drawings/drawing1.xml -> \
                 application/vnd.openxmlformats-officedocument.drawing+xml",
            ],
        ),
    ] {
        assert!(
            plain.is_superset(&app),
            "{what}: the app (re-injecting) path is supposed to lose a strict SUBSET of what the \
             bare serializer loses, but it loses things the plain path keeps: {:?}",
            app.difference(&plain).collect::<Vec<_>>(),
        );
        let saved: BTreeSet<&String> = plain.difference(&app).collect();
        for name in must_include {
            assert!(
                saved.iter().any(|s| s.as_str() == name),
                "{what}: {name:?} is supposed to be preserved by chart re-injection and lost \
                 without it — the C3 evidence in this file has been erased. Saved by \
                 re-injection: {saved:?}",
            );
        }
    }
}

/// The loss is a property of the **writer**, not of whether the model was touched — so one
/// fixture is saved both clean and after an edit to prove the inventory is identical either way.
/// Running every fixture twice would double a slow suite for no extra information.
///
/// The edit itself is forced and asserted inside [`save_through_worker`] (one committed op, and
/// the probe string present in the saved package); without that this test's claim would be
/// vacuously true whenever the edit silently failed to land.
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
    assert_eq!(
        clean.ct_dropped, edited.ct_dropped,
        "editing a cell changed which [Content_Types].xml declarations survive the save",
    );
}
