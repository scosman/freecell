//! E3 regression: a workbook whose cells reference the **built-in** date/time `numFmtId`s
//! (14–22) *without* defining them must render as dates/times, not raw serials. This exercises
//! the corrected `DEFAULT_NUM_FMTS` table (fork `fix/e2-numfmt`) end-to-end through the public
//! `freecell-engine` open + formatted-value surface. `dates.xlsx` is a hand-crafted fixture
//! (`tests/fixtures/dates.xlsx`); serials 44197 = 2021-01-01, 0.5 = 12:00, 44562.5 = 2022-01-01 12:00.

use std::path::PathBuf;

use freecell_core::CellRef;
use freecell_engine::WorkbookDocument;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dates.xlsx")
}

#[test]
fn builtin_date_formats_render_as_dates_not_serials() {
    let doc = WorkbookDocument::open(&fixture()).expect("dates.xlsx should open");
    let f = |r: u32| {
        doc.formatted_value(0, CellRef::new(r, 0))
            .expect("cell in range")
    };
    for r in 0..8u32 {
        let s = f(r);
        eprintln!("A{} => {s:?}", r + 1);
        assert!(
            !s.contains("44197") && !s.contains("44562"),
            "row {r} still shows the raw serial: {s:?}"
        );
        assert!(!s.contains('#'), "row {r} shows an error value: {s:?}");
        assert!(!s.trim().is_empty(), "row {r} rendered empty: {s:?}");
    }
    // Concrete spot-checks: a month name for a date id, a clock time for a time id.
    assert!(f(1).contains("Jan"), "id 15 (d-mmm-yy) should name the month: {:?}", f(1));
    assert!(f(5).contains("12:00"), "id 20 (h:mm) should show a time: {:?}", f(5));
}
