//! `freecell-engine` — the IronCalc adapter, evaluation worker, caches, and file I/O.
//!
//! The worker owns the `UserModel` (workbook truth) on a dedicated 64 MiB-stack thread
//! and is the only code that touches an IronCalc type; the UI reads published snapshots
//! and the resident style/geometry cache (`architecture.md §2`). This crate is
//! GPUI-free so it builds and tests headless in Linux CI (`architecture.md §9`).
//!
//! Phase 3 adds the **file-I/O adapter** ([`WorkbookDocument`]): new/open/save with atomic
//! temp-file+rename and typed [`LoadError`]/[`SaveError`]s, plus the [`fixtures`] module of
//! deterministic test workbooks. The worker seam (Phase 4) and caches (Phase 5) build on it.

pub mod document;
pub mod fixtures;

pub use document::{
    CellQueryError, DocumentSource, LoadError, SaveError, WorkbookDocument, DEFAULT_LANGUAGE,
    DEFAULT_LOCALE, DEFAULT_TIMEZONE, NEW_WORKBOOK_NAME,
};

/// Re-export of the pinned IronCalc workbook type the worker will own. `pub(crate)` — the
/// worker lives inside this crate and `WorkbookDocument` keeps every IronCalc type off the
/// public surface (`architecture.md §2`: `freecell-engine` is the headless boundary; no
/// IronCalc type escapes it). Kept as a single canonical path for in-crate use.
pub(crate) use ironcalc_base::UserModel;

#[cfg(test)]
mod tests {
    use super::UserModel;

    /// Foundation check for the whole engine track: the exact-pinned IronCalc crates
    /// resolve, link, and expose a working `UserModel::new_empty`. If this fails the
    /// dependency pin is wrong and nothing downstream can be built.
    #[test]
    fn ironcalc_links_and_creates_empty_model() {
        let model = UserModel::new_empty("Book", "en", "UTC", "en")
            .expect("IronCalc should create an empty workbook");
        // A fresh workbook has exactly one sheet, the state the app opens on when
        // creating a new document. `get_worksheets_properties` is the `UserModel`
        // enumeration API (round-3 B api-audit: `sheet_ops.rs`).
        let sheets = model.get_worksheets_properties();
        assert_eq!(sheets.len(), 1, "a new workbook has one sheet");
    }
}
