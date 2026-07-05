//! `freecell-engine` — the IronCalc adapter, evaluation worker, caches, and file I/O.
//!
//! The worker owns the `UserModel` (workbook truth) on a dedicated 64 MiB-stack thread
//! and is the only code that touches an IronCalc type; the UI reads published snapshots
//! and the resident style/geometry cache (`architecture.md §2`). This crate is
//! GPUI-free so it builds and tests headless in Linux CI (`architecture.md §9`).
//!
//! Phase 3 adds the **file-I/O adapter** ([`WorkbookDocument`]): new/open/save with atomic
//! temp-file+rename and typed [`LoadError`]/[`SaveError`]s, plus the [`fixtures`] module of
//! deterministic test workbooks.
//!
//! Phase 4 adds the **eval worker seam** ([`worker`]): [`DocumentClient::spawn`] runs the
//! `UserModel` on a dedicated 64 MiB-stack thread and drives the drain-coalesce → apply →
//! publish-then-bump loop, the viewport [`Publication`](freecell_core::Publication) build,
//! the worker-side input-cap re-check, `catch_unwind` + degraded policy, and dirty-op
//! accounting.
//!
//! Phase 5 adds the **style & geometry cache** ([`cache`]): the IronCalc-facing builder/mutator
//! that converts engine geometry + `Style` into the engine-free
//! [`SheetCache`](freecell_core::SheetCache) read model (resolved `RenderStyle`s + px geometry).
//! The worker builds it on sheet activation and mirrors each issued edit into it (re-reading the
//! touched cells), shipping `StyleCacheUpdated` deltas — provably in agreement with a fresh
//! engine re-read (the load-bearing contract).

pub(crate) mod cache;
pub mod document;
pub mod fixtures;
pub mod instrument;
pub(crate) mod open_fixups;
pub(crate) mod open_repair;
pub mod worker;

pub use document::{
    CellQueryError, DocumentSource, LoadError, SaveError, WorkbookDocument, DEFAULT_LANGUAGE,
    DEFAULT_LOCALE, DEFAULT_TIMEZONE, NEW_WORKBOOK_NAME,
};
pub use instrument::{engine_call_count, reset_engine_call_count};
pub use worker::{
    Command, DocumentClient, EditRejectedReason, PasteError, SheetMeta, StyleAttr, StylePath,
    WorkerEvent, WorkerEventReceiver, WORKER_STACK_SIZE,
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
