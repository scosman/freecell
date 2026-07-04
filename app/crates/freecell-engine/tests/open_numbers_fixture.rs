//! Regression test for the reactive missing-`xfId` repair (`src/open_repair.rs`).
//!
//! `numbers_table.xlsx` is a real workbook exported by Apple Numbers. Its `xl/styles.xml`
//! `<cellXfs>` `<xf>` elements omit the *optional* `xfId` attribute, which IronCalc 0.7.1's
//! styles parser wrongly requires — so a plain `load_from_xlsx` fails with
//! `XML Error: Missing "xfId" XML attribute`. `WorkbookDocument::open` must transparently
//! repair-and-retry, opening the file and exposing its cached values.
//!
//! The fixture is the user's own file (committed on purpose), and the test drives only the
//! **public** `freecell-engine` surface — exactly as the Phase-4 worker would.

use std::path::PathBuf;

use freecell_core::CellRef;
use freecell_engine::WorkbookDocument;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/numbers_table.xlsx")
}

fn fmt(doc: &WorkbookDocument, row: u32, col: u32) -> String {
    doc.formatted_value(0, CellRef::new(row, col))
        .expect("cell in range")
}

#[test]
fn numbers_export_missing_xf_id_opens_via_repair() {
    let doc = WorkbookDocument::open(&fixture())
        .expect("a Numbers-exported .xlsx missing the optional xfId must open via repair");

    // The workbook structure survived the repair.
    assert_eq!(doc.sheet_count(), 1);
    assert_eq!(doc.sheet_names(), vec!["Sheet 1".to_string()]);

    // The merged title cell A1 and the bold "ASDF" row label A3 (from sharedStrings).
    assert_eq!(fmt(&doc, 0, 0), "Table 1"); // A1
    assert_eq!(fmt(&doc, 2, 0), "ASDF"); // A3

    // Header row labels (B1 blank, "Test 1"/"C"/"D"/"E" across B2:E2).
    assert_eq!(fmt(&doc, 1, 1), "Test 1"); // B2
    assert_eq!(fmt(&doc, 1, 2), "C"); // C2

    // The "Test 1" column is powers of two down the rows: B3=1 … B17=16384. These are the
    // cached values in the file (open does not evaluate), so they must be present verbatim.
    let powers = [
        (2u32, "1"),
        (3, "2"),
        (4, "4"),
        (5, "8"),
        (6, "16"),
        (16, "16384"),
    ];
    for (row, want) in powers {
        assert_eq!(fmt(&doc, row, 1), want, "B{} should be {want}", row + 1);
    }

    // The "C" column mirrors it descending: C3=16384 … C16=2, C17=2.
    assert_eq!(fmt(&doc, 2, 2), "16384"); // C3
    assert_eq!(fmt(&doc, 15, 2), "2"); // C16
    assert_eq!(fmt(&doc, 16, 2), "2"); // C17

    // The TOTAL row (A18="TOTAL") sums each column: B18=32767 (yellow cell), C18=32768 (red).
    assert_eq!(fmt(&doc, 17, 0), "TOTAL"); // A18
    assert_eq!(fmt(&doc, 17, 1), "32767"); // B18
    assert_eq!(fmt(&doc, 17, 2), "32768"); // C18
}
