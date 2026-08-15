//! Integration tests for the Phase-3 file-I/O adapter (`components/engine_worker.md §File
//! I/O`, test plan). These drive the **public** `freecell-engine` surface only —
//! `WorkbookDocument` + `fixtures` — as a real consumer (the Phase-4 worker) would, proving
//! open→save→reopen round-trips, atomic-save safety on failure, and the typed open-failure
//! matrix. Style round-trips (which read the raw `ironcalc` `Style`) live in-crate in
//! `document.rs`.

use std::fs;

use freecell_core::{CellLink, CellRef, LinkTarget};
use freecell_engine::{fixtures, DocumentSource, LoadError, SaveError, WorkbookDocument};
use tempfile::tempdir;

/// Saves `doc` under a fresh temp path and reopens it, returning the reopened document. The
/// heart of every round-trip assertion.
fn save_and_reopen(doc: &WorkbookDocument, name: &str) -> (tempfile::TempDir, WorkbookDocument) {
    let dir = tempdir().unwrap();
    let path = dir.path().join(name);
    doc.save(&path).expect("save should succeed");
    let reopened = WorkbookDocument::open(&path).expect("reopen should succeed");
    (dir, reopened)
}

fn fmt(doc: &WorkbookDocument, sheet: u32, row: u32, col: u32) -> String {
    doc.formatted_value(sheet, CellRef::new(row, col))
        .expect("cell in range")
}

fn content(doc: &WorkbookDocument, sheet: u32, row: u32, col: u32) -> String {
    doc.cell_content(sheet, CellRef::new(row, col))
        .expect("cell in range")
}

fn link(doc: &WorkbookDocument, sheet: u32, row: u32, col: u32) -> Option<CellLink> {
    doc.cell_link(sheet, CellRef::new(row, col))
        .expect("cell in range")
}

#[test]
fn new_empty_has_one_sheet() {
    let doc = WorkbookDocument::new_empty().unwrap();
    assert_eq!(doc.sheet_names(), vec!["Sheet1".to_string()]);

    // The `DocumentSource::NewWorkbook` entry point produces the same thing.
    let via_source = WorkbookDocument::from_source(&DocumentSource::NewWorkbook).unwrap();
    assert_eq!(via_source.sheet_names(), vec!["Sheet1".to_string()]);
}

#[test]
fn new_empty_roundtrips() {
    let doc = WorkbookDocument::new_empty().unwrap();
    let (_dir, reopened) = save_and_reopen(&doc, "empty.xlsx");
    assert_eq!(reopened.sheet_names(), vec!["Sheet1".to_string()]);
}

#[test]
fn roundtrip_values_preserved() {
    let (_dir, doc) = save_and_reopen(&fixtures::values(), "values.xlsx");
    assert_eq!(fmt(&doc, 0, 0, 0), "42"); // A1 number
    assert_eq!(fmt(&doc, 0, 0, 1), "hello"); // B1 text
    assert_eq!(fmt(&doc, 0, 1, 0), "3.14"); // A2 decimal
    assert_eq!(fmt(&doc, 0, 1, 1), "-7"); // B2 negative
    assert_eq!(fmt(&doc, 0, 0, 2), "TRUE"); // C1 boolean
                                            // Empty cells come back empty (never a stale value).
    assert_eq!(fmt(&doc, 0, 5, 5), "");
}

#[test]
fn roundtrip_formulas_preserved() {
    let (_dir, doc) = save_and_reopen(&fixtures::formulas(), "formulas.xlsx");
    // Raw formula text survives (what the formula bar shows/edits).
    assert_eq!(content(&doc, 0, 3, 0), "=SUM(A1:A3)");
    assert_eq!(content(&doc, 0, 0, 1), "=A1*2");
    // Cached results survive too (no eval-on-open is needed — SP2).
    assert_eq!(fmt(&doc, 0, 3, 0), "60"); // A4 = SUM(A1:A3)
    assert_eq!(fmt(&doc, 0, 0, 1), "20"); // B1 = A1*2
                                          // A plain literal reports itself as its own content.
    assert_eq!(content(&doc, 0, 0, 0), "10");
}

#[test]
fn roundtrip_number_formats_preserved() {
    let (_dir, doc) = save_and_reopen(&fixtures::number_formats(), "numfmt.xlsx");
    // The `num_fmt` string round-trips AND the engine renders the display text (round-3 B:
    // FreeCell owns no number-format logic).
    assert_eq!(fmt(&doc, 0, 0, 0), "$1,234.50"); // currency
    assert_eq!(fmt(&doc, 0, 0, 1), "100.00%"); // percent
    assert_eq!(fmt(&doc, 0, 0, 2), "2021-01-01"); // date serial → yyyy-mm-dd
}

/// F10.1–F10.3: hyperlinks survive save → reopen — targets, ScreenTips, and internal links
/// alike. The fixture's links were created through FreeCell's own engine facade, so this covers
/// "a link created in FreeCell survives a round-trip" as well as the import direction.
#[test]
fn roundtrip_links_preserved() {
    let (_dir, doc) = save_and_reopen(&fixtures::links(), "links.xlsx");

    assert_eq!(
        link(&doc, 0, 0, 0).unwrap().target,
        LinkTarget::external("https://ironcalc.com")
    );
    // The display text is the cell's own content, not part of the link.
    assert_eq!(fmt(&doc, 0, 0, 0), "IronCalc");

    // A query string survives verbatim, and the ScreenTip with it (F10.2).
    assert_eq!(
        link(&doc, 0, 1, 0).unwrap().target,
        LinkTarget::external("https://example.com/docs?a=1&b=2")
            .with_tooltip(Some("The docs".into()))
    );
    assert_eq!(
        link(&doc, 0, 2, 0).unwrap().target,
        LinkTarget::external("mailto:someone@example.com")
    );
    // An internal link stays internal (it rides the `location` attribute, not a relationship).
    assert_eq!(
        link(&doc, 0, 3, 0).unwrap().target,
        LinkTarget::internal("Sheet1!A30").with_tooltip(Some("Down the sheet".into()))
    );
    // The auto-linked cell round-trips like any other worksheet link.
    assert_eq!(
        link(&doc, 0, 4, 0).unwrap().target,
        LinkTarget::external("https://auto.example.com")
    );
    // Every one of those is a worksheet link, and the unlinked cell stays unlinked.
    assert!((0..=4).all(|r| !link(&doc, 0, r, 0).unwrap().dynamic));
    assert_eq!(link(&doc, 0, 0, 1), None);
}

/// F1.2: a link's **colour** is ordinary cell formatting, so it round-trips with the cell rather
/// than being re-derived on load — that is what lets FreeCell render an imported link blue with no
/// styling of its own. Asserted as a **differential** (the reopened colour equals the live one,
/// which pins preservation exactly rather than "some colour survived") plus a blue check, so a
/// theme-resolution regression that turned links black would fail here.
///
/// The **underline** half of the formatting cannot be reached from an integration test —
/// `published_style` carries colour only, and the resolved `RenderStyle` is crate-private — so it
/// is covered in-crate by `document.rs`'s `a_reopened_link_keeps_its_underline_and_blue`.
#[test]
fn roundtrip_link_formatting_preserved() {
    let live = fixtures::links();
    let live_color = live
        .published_style(0, CellRef::new(0, 0))
        .expect("cell in range")
        .1
        .expect("a freshly created link has an explicit font colour");

    let (_dir, doc) = save_and_reopen(&live, "link-style.xlsx");
    let reopened = doc
        .published_style(0, CellRef::new(0, 0))
        .expect("cell in range")
        .1
        .expect("the reopened link cell keeps an explicit font colour");

    assert_eq!(
        reopened, live_color,
        "the link colour survives save → reopen unchanged"
    );
    assert!(
        reopened.b > 128 && reopened.b > reopened.r + 48 && reopened.b > reopened.g + 48,
        "the reopened link colour is a blue, got {reopened:?}"
    );
    // The unlinked control cell has no explicit colour, so the assertion above is about the link
    // rather than about every cell in the workbook.
    assert_eq!(
        doc.published_style(0, CellRef::new(0, 1))
            .expect("cell in range")
            .1,
        None
    );
}

/// A `HYPERLINK()` link is **not persisted** — nothing about it is written to the file, and the
/// reopened workbook carries no link on that cell until an evaluation re-creates it (the
/// re-creation half is `document.rs`'s `a_reopened_hyperlink_formula_recreates_its_link`, which
/// can reach the crate-private `evaluate`). That asymmetry is exactly why the link cache is
/// mirrored off evaluation rather than off the worksheet's stored links.
#[test]
fn roundtrip_dynamic_link_is_not_persisted() {
    let (_dir, doc) = save_and_reopen(&fixtures::links(), "dynamic-link.xlsx");
    assert_eq!(
        content(&doc, 0, 5, 0),
        "=HYPERLINK(\"https://dynamic.example.com\",\"Dynamic\")",
        "the formula — the only thing that persists — survives"
    );
    assert_eq!(fmt(&doc, 0, 5, 0), "Dynamic"); // F9.1, from the file's cached value
    assert_eq!(
        link(&doc, 0, 5, 0),
        None,
        "opening does not evaluate, so the dynamic link does not exist yet"
    );
}

#[test]
fn roundtrip_multi_sheet_and_names() {
    let (_dir, doc) = save_and_reopen(&fixtures::multi_sheet(), "multi.xlsx");
    assert_eq!(doc.sheet_names(), vec!["Sheet1", "Sheet2", "Sheet3"]);
    assert_eq!(doc.sheet_count(), 3);
    assert_eq!(fmt(&doc, 0, 0, 0), "10"); // Sheet1!A1
    assert_eq!(fmt(&doc, 1, 0, 0), "20"); // Sheet2!A1 = Sheet1!A1 * 2
    assert_eq!(fmt(&doc, 2, 0, 0), "world"); // Sheet3!A1
}

#[test]
fn roundtrip_after_rename() {
    let (_dir, doc) = save_and_reopen(&fixtures::multi_sheet_renamed(), "renamed.xlsx");
    assert_eq!(doc.sheet_names(), vec!["Sheet1", "Data", "Sheet3"]);
    // The renamed sheet's cross-sheet formula still evaluates.
    assert_eq!(fmt(&doc, 1, 0, 0), "20");
}

#[test]
fn formula_errors_are_values() {
    // `#DIV/0!` from the formulas fixture and `#CIRC!` from a circular ring both come back as
    // display text (never a panic or hang), in memory and after a round-trip.
    let live_div0 = fixtures::formulas();
    assert_eq!(fmt(&live_div0, 0, 0, 2), "#DIV/0!"); // C1 = 1/0, freshly evaluated

    let (_d1, div0) = save_and_reopen(&fixtures::formulas(), "div0.xlsx");
    assert_eq!(fmt(&div0, 0, 0, 2), "#DIV/0!");

    let ring = fixtures::circular_ref(50);
    assert_eq!(fmt(&ring, 0, 0, 0), "#CIRC!"); // every cell in the ring is circular
    assert_eq!(fmt(&ring, 0, 49, 0), "#CIRC!");
    let (_d2, ring) = save_and_reopen(&ring, "circ.xlsx");
    assert_eq!(fmt(&ring, 0, 0, 0), "#CIRC!");
}

#[test]
fn save_overwrites_existing_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("book.xlsx");

    // First save: the values fixture.
    fixtures::values().save(&path).unwrap();
    let v1 = WorkbookDocument::open(&path).unwrap();
    assert_eq!(fmt(&v1, 0, 0, 0), "42");

    // Second save to the SAME path: a different workbook. Atomic rename replaces the file.
    fixtures::formulas().save(&path).unwrap();
    let v2 = WorkbookDocument::open(&path).unwrap();
    assert_eq!(fmt(&v2, 0, 3, 0), "60"); // now the formulas fixture
    assert_eq!(content(&v2, 0, 3, 0), "=SUM(A1:A3)");

    // Exactly one file at the path (no temp-file litter left behind).
    let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn save_failure_missing_directory_creates_nothing() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("does-not-exist").join("book.xlsx");
    let err = fixtures::values().save(&missing).unwrap_err();
    assert!(matches!(err, SaveError::Io(_)), "got {err:?}");
    assert!(
        !missing.exists(),
        "a failed save must not create the target"
    );
}

#[test]
fn save_failure_preserves_destination() {
    // Root-proof failure injection: make the destination path an existing NON-empty directory.
    // The atomic rename (temp file → directory) fails with EISDIR even for root, so the
    // destination and its contents must be left byte-identical and no temp file may leak.
    let dir = tempdir().unwrap();
    let target = dir.path().join("book.xlsx"); // a directory named like the target file
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), b"original-bytes").unwrap();

    let err = fixtures::values().save(&target).unwrap_err();
    assert!(matches!(err, SaveError::Io(_)), "got {err:?}");

    // Destination directory + sentinel untouched.
    assert!(target.is_dir());
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"original-bytes"
    );
    // No leftover temp file beside the target (only `book.xlsx` remains in `dir`).
    let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(
        entries.len(),
        1,
        "the temp file must be cleaned up on failure"
    );
}

#[test]
fn failed_save_leaves_real_existing_xlsx_byte_identical() {
    // The invariant `functional_spec.md §5.2` really cares about: a genuine, valid `.xlsx`
    // on disk is byte-for-byte untouched when a save fails. By design a save to a writable
    // regular-file target can't fail root-proof (same-dir temp + rename is exactly the safety
    // property), so the failure is injected by making the *destination's parent* be that real
    // `.xlsx` file: `NamedTempFile::new_in(<a regular file>)` fails with ENOTDIR (root-proof)
    // before anything is written, and the real workbook beside it is never touched.
    let dir = tempdir().unwrap();
    let existing = dir.path().join("book.xlsx");
    fixtures::values().save(&existing).unwrap(); // a genuine IronCalc workbook
    let original_bytes = fs::read(&existing).unwrap();
    assert_eq!(
        fmt(&WorkbookDocument::open(&existing).unwrap(), 0, 0, 0),
        "42"
    );

    // A save whose destination directory *is* the existing file → ENOTDIR on temp creation.
    let target = existing.join("child.xlsx");
    let err = fixtures::formulas().save(&target).unwrap_err();
    assert!(matches!(err, SaveError::Io(_)), "got {err:?}");

    // The real workbook is byte-identical and still opens correctly.
    assert_eq!(fs::read(&existing).unwrap(), original_bytes);
    assert_eq!(
        fmt(&WorkbookDocument::open(&existing).unwrap(), 0, 0, 0),
        "42"
    );
    // Only the one real workbook remains in the directory (no temp litter).
    let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn open_missing_file_is_io_error() {
    let dir = tempdir().unwrap();
    let err = WorkbookDocument::open(&dir.path().join("nope.xlsx")).unwrap_err();
    assert!(matches!(err, LoadError::Io(_)), "got {err:?}");
}

#[test]
fn open_empty_file_is_not_xlsx() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("empty.xlsx");
    fs::write(&path, b"").unwrap();
    let err = WorkbookDocument::open(&path).unwrap_err();
    assert!(matches!(err, LoadError::NotXlsx(_)), "got {err:?}");
}

#[test]
fn open_text_file_is_not_xlsx() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("notes.xlsx"); // .xlsx extension but text content
    fs::write(&path, b"this is just some text, not a spreadsheet").unwrap();
    let err = WorkbookDocument::open(&path).unwrap_err();
    assert!(matches!(err, LoadError::NotXlsx(_)), "got {err:?}");
}

#[test]
fn open_truncated_zip_is_corrupt() {
    // Starts with the Zip magic (so it's classified as an xlsx candidate) but is a bogus,
    // truncated archive → the loader fails → typed `Corrupt`, never a panic.
    let dir = tempdir().unwrap();
    let path = dir.path().join("broken.xlsx");
    fs::write(&path, b"PK\x03\x04\x00\x00garbage-not-a-real-zip").unwrap();
    let err = WorkbookDocument::open(&path).unwrap_err();
    assert!(matches!(err, LoadError::Corrupt(_)), "got {err:?}");
}

#[test]
fn open_ole_file_is_password_protected() {
    // The OLE2/CFB magic marks an encrypted OOXML (or a legacy binary .xls) container.
    let dir = tempdir().unwrap();
    let path = dir.path().join("locked.xlsx");
    fs::write(
        &path,
        [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0],
    )
    .unwrap();
    let err = WorkbookDocument::open(&path).unwrap_err();
    assert!(matches!(err, LoadError::PasswordProtected), "got {err:?}");
}

/// A real IronCalc-written file round-trips cleanly through corruption classification: it is
/// a valid Zip and opens, i.e. the magic pre-check never rejects a legitimate workbook.
#[test]
fn genuine_saved_file_opens() {
    let (_dir, doc) = save_and_reopen(&fixtures::values(), "genuine.xlsx");
    assert_eq!(doc.sheet_names(), vec!["Sheet1".to_string()]);
}

/// `DocumentSource::OpenFile` reaches a saved workbook (the other `from_source` branch).
#[test]
fn from_source_opens_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("src.xlsx");
    fixtures::values().save(&path).unwrap();

    let doc = WorkbookDocument::from_source(&DocumentSource::OpenFile(path)).unwrap();
    assert_eq!(fmt(&doc, 0, 0, 0), "42");
}
