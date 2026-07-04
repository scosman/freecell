//! Pure lifecycle helpers — window titles, the dirty computation, save-target resolution,
//! and the quit-flow queue (`components/app_shell.md`, `functional_spec.md §2, §5.2`).
//!
//! All gpui-free and table-tested, so the load-bearing rules (Untitled titling, the
//! op-accounting dirty flag, Save-vs-Save-As targeting, `.xlsx` enforcement, the
//! front-to-back quit-prompt order + cancel-aborts semantics) run in `cargo test
//! --workspace` with no display.

use std::path::{Path, PathBuf};

use super::registry::WindowKey;

/// The document name shown in the window title: the file's base name (`Budget.xlsx`) or
/// `Untitled` for an unsaved workbook (`functional_spec.md §2.3`).
pub const UNTITLED: &str = "Untitled";

/// The default file name a `Save As` on an untitled workbook suggests (`functional_spec.md
/// §5.2`).
pub const UNTITLED_FILE: &str = "Untitled.xlsx";

/// The enforced workbook extension (MVP is `.xlsx`-only, `functional_spec.md §1, §5.2`).
pub const XLSX_EXT: &str = "xlsx";

/// The document name for a window: the path's file name, or `Untitled` when unsaved.
pub fn document_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| UNTITLED.to_string())
}

/// The window title string. `use_edited_suffix` is the fallback for platforms/revs that
/// don't expose the macOS document-edited dot (`functional_spec.md §2.3`: "if GPUI doesn't
/// expose it … suffix the title with `— Edited`"). When the dot is available the caller
/// passes `false` and reflects dirtiness through `Window::set_window_edited` instead.
pub fn window_title(name: &str, dirty: bool, use_edited_suffix: bool) -> String {
    if dirty && use_edited_suffix {
        format!("{name} — Edited")
    } else {
        name.to_string()
    }
}

/// The dirty flag from op accounting (`architecture.md §2`): the document is dirty when the
/// worker has committed more undoable ops than were present at the last successful save.
/// `committed_ops` is monotonic (undo/redo also count, per the Phase-4 accounting), so this
/// is a plain "has anything happened since the save index" test.
pub fn is_dirty(committed_ops: u64, last_saved_ops: u64) -> bool {
    committed_ops > last_saved_ops
}

/// Where a save should write (`components/app_shell.md §Save flow`, `functional_spec.md
/// §5.2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    /// Save directly to a known path (`Save` on a titled document).
    Path(PathBuf),
    /// No destination yet — show the save panel (`Save` on `Untitled`, or `Save As`). Carries
    /// the file name to pre-fill.
    Prompt { suggested_name: String },
}

/// Resolves the destination of a save. `Save As` (or an untitled document) prompts; `Save`
/// on a titled document writes straight to its path (`functional_spec.md §5.2`: "Save on an
/// `Untitled` workbook = Save As").
pub fn resolve_save_target(current: Option<&Path>, save_as: bool) -> SaveTarget {
    match current {
        Some(path) if !save_as => SaveTarget::Path(path.to_path_buf()),
        Some(path) => SaveTarget::Prompt {
            suggested_name: document_name(Some(path)),
        },
        None => SaveTarget::Prompt {
            suggested_name: UNTITLED_FILE.to_string(),
        },
    }
}

/// Enforces the `.xlsx` extension on a save-panel result (`functional_spec.md §5.2`:
/// "enforcing the `.xlsx` extension"). A path with no or a different extension gets `.xlsx`;
/// an existing `.xlsx` (any case) is left unchanged.
pub fn with_xlsx_extension(path: PathBuf) -> PathBuf {
    let already = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case(XLSX_EXT))
        .unwrap_or(false);
    if already {
        path
    } else {
        path.with_extension(XLSX_EXT)
    }
}

/// The next step the quit flow should take (`functional_spec.md §2.3`: "prompts per-window
/// for unsaved changes, any Cancel aborts the quit").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitStep {
    /// Prompt this dirty window's unsaved-changes modal before continuing.
    Prompt(WindowKey),
    /// All dirty windows resolved (saved or discarded) — quit the application.
    QuitNow,
    /// A window's prompt was cancelled — the quit is off.
    Aborted,
}

/// Drives the app-quit flow across windows: a front-to-back queue of the dirty windows to
/// prompt, plus a cancelled flag. Pure so the ordering + cancel-aborts semantics are unit
/// tested; the GPUI layer shows each modal and reports the answer back
/// (`resolved`/`cancel`).
#[derive(Debug, Clone)]
pub struct QuitPlan {
    pending: Vec<WindowKey>,
    aborted: bool,
}

impl QuitPlan {
    /// A plan over the dirty windows, front-to-back (already filtered + ordered by the
    /// registry's `dirty_among`).
    pub fn new(dirty_front_to_back: Vec<WindowKey>) -> Self {
        Self {
            pending: dirty_front_to_back,
            aborted: false,
        }
    }

    /// The next step: [`QuitStep::Aborted`] if a prompt was cancelled, else the next dirty
    /// window to prompt, else [`QuitStep::QuitNow`] when none remain.
    pub fn next(&self) -> QuitStep {
        if self.aborted {
            QuitStep::Aborted
        } else if let Some(key) = self.pending.first() {
            QuitStep::Prompt(*key)
        } else {
            QuitStep::QuitNow
        }
    }

    /// Records that `key`'s prompt resolved without cancelling (the user saved or discarded);
    /// it drops out of the pending queue.
    pub fn resolved(&mut self, key: WindowKey) {
        self.pending.retain(|k| *k != key);
    }

    /// Whether `key` is still in the pending prompt set. The GPUI layer checks this before
    /// advancing on a window close: a window *not* in the pending set closing mid-quit must
    /// not disturb the window currently being prompted (`app.rs on_window_closed`).
    pub fn is_pending(&self, key: WindowKey) -> bool {
        self.pending.contains(&key)
    }

    /// Records that a prompt was cancelled — aborts the whole quit
    /// (`functional_spec.md §2.3`).
    pub fn cancel(&mut self) {
        self.aborted = true;
    }

    /// Whether the quit was cancelled.
    pub fn aborted(&self) -> bool {
        self.aborted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn document_name_untitled_and_named() {
        assert_eq!(document_name(None), "Untitled");
        assert_eq!(document_name(Some(&path("/a/Budget.xlsx"))), "Budget.xlsx");
    }

    #[test]
    fn window_title_suffix_only_when_no_dot() {
        assert_eq!(window_title("Budget.xlsx", false, true), "Budget.xlsx");
        // Dirty + no native dot → suffixed.
        assert_eq!(
            window_title("Budget.xlsx", true, true),
            "Budget.xlsx — Edited"
        );
        // Dirty but the native edited-dot is used → title stays clean.
        assert_eq!(window_title("Budget.xlsx", true, false), "Budget.xlsx");
    }

    #[test]
    fn dirty_by_op_accounting() {
        assert!(!is_dirty(0, 0));
        assert!(!is_dirty(5, 5));
        assert!(is_dirty(6, 5));
    }

    #[test]
    fn save_target_untitled_prompts_with_default_name() {
        assert_eq!(
            resolve_save_target(None, false),
            SaveTarget::Prompt {
                suggested_name: "Untitled.xlsx".into()
            }
        );
    }

    #[test]
    fn save_target_titled_save_uses_path() {
        assert_eq!(
            resolve_save_target(Some(&path("/b/Budget.xlsx")), false),
            SaveTarget::Path(path("/b/Budget.xlsx"))
        );
    }

    #[test]
    fn save_target_save_as_prompts_with_current_name() {
        assert_eq!(
            resolve_save_target(Some(&path("/b/Budget.xlsx")), true),
            SaveTarget::Prompt {
                suggested_name: "Budget.xlsx".into()
            }
        );
    }

    #[test]
    fn xlsx_extension_added_kept_and_replaced() {
        assert_eq!(
            with_xlsx_extension(path("/a/book")),
            path("/a/book.xlsx"),
            "no extension → add"
        );
        assert_eq!(
            with_xlsx_extension(path("/a/book.xlsx")),
            path("/a/book.xlsx"),
            "already xlsx → keep"
        );
        assert_eq!(
            with_xlsx_extension(path("/a/book.XLSX")),
            path("/a/book.XLSX"),
            "case-insensitive xlsx → keep as typed"
        );
        assert_eq!(
            with_xlsx_extension(path("/a/book.csv")),
            path("/a/book.xlsx"),
            "wrong extension → replace"
        );
    }

    #[test]
    fn quitplan_empty_quits_now() {
        let plan = QuitPlan::new(vec![]);
        assert_eq!(plan.next(), QuitStep::QuitNow);
    }

    #[test]
    fn quitplan_prompts_in_order_then_quits() {
        let (a, b) = (WindowKey(0), WindowKey(1));
        let mut plan = QuitPlan::new(vec![a, b]);
        assert_eq!(plan.next(), QuitStep::Prompt(a));
        plan.resolved(a);
        assert_eq!(plan.next(), QuitStep::Prompt(b));
        plan.resolved(b);
        assert_eq!(plan.next(), QuitStep::QuitNow);
    }

    #[test]
    fn quitplan_cancel_aborts() {
        let (a, b) = (WindowKey(0), WindowKey(1));
        let mut plan = QuitPlan::new(vec![a, b]);
        assert_eq!(plan.next(), QuitStep::Prompt(a));
        plan.cancel();
        assert!(plan.aborted());
        assert_eq!(plan.next(), QuitStep::Aborted);
    }

    #[test]
    fn quitplan_is_pending_tracks_membership() {
        let (a, b, c) = (WindowKey(0), WindowKey(1), WindowKey(2));
        let mut plan = QuitPlan::new(vec![a, b]);
        assert!(plan.is_pending(a) && plan.is_pending(b));
        // A window that was never dirty (not in the plan) is not pending — closing it mid-quit
        // must not advance the flow.
        assert!(!plan.is_pending(c));
        plan.resolved(a);
        assert!(!plan.is_pending(a), "resolved windows drop out of pending");
        assert!(plan.is_pending(b));
    }
}
