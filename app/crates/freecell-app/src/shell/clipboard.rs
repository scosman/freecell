//! [`ClipboardCoordinator`] — the UI side of the range clipboard (`components/clipboard.md`,
//! `functional_spec.md §2`). The worker owns the full-fidelity payload; this shell decides
//! internal-vs-external paste and bridges the system clipboard.
//!
//! Copy/cut send [`Command::CopySelection`]; the worker replies [`WorkerEvent::CopyReady`]
//! (folded by the window into [`on_copy_ready`](ClipboardCoordinator::on_copy_ready)) with the
//! tab-separated text this writes to the system clipboard and remembers as `last_copy_text`.
//! Paste reads the system clipboard: if the text is byte-identical to our last copy we route an
//! internal paste (full fidelity); otherwise an external TSV paste.

use gpui::{App, ClipboardItem};

use freecell_core::{CellRef, SelectionModel, SheetId};
use freecell_engine::{Command, DocumentClient};

/// Owns the "is the system clipboard still ours?" decision. Held by the window (behind a
/// `RefCell` in its sink-shared state) so both the grid-sink key events and the worker
/// `CopyReady` reply can reach it.
#[derive(Debug, Default)]
pub struct ClipboardCoordinator {
    /// The TSV of our most recent copy/cut, once the worker reply has written it to the system
    /// clipboard. `None` once a foreign clipboard change is observed (a paste that didn't match).
    last_copy_text: Option<String>,
}

impl ClipboardCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cmd/Ctrl+C (or +X, `cut = true`): copy/cut the selection. The worker clamps to the used
    /// range, so a full-column / select-all copy is cheap. The reply arrives via
    /// [`on_copy_ready`](Self::on_copy_ready).
    pub fn copy(
        &mut self,
        sheet: SheetId,
        selection: SelectionModel,
        cut: bool,
        client: &DocumentClient,
    ) {
        client.send(Command::CopySelection {
            sheet,
            range: selection.range(),
            cut,
        });
    }

    /// Fold a [`WorkerEvent::CopyReady`]: write the TSV to the system clipboard so other apps can
    /// paste values, and remember it so our own next paste routes internally.
    pub fn on_copy_ready(&mut self, tsv: String, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(tsv.clone()));
        self.last_copy_text = Some(tsv);
    }

    /// Cmd/Ctrl+V: paste at `anchor` (the selection's top-left). Reads the system clipboard and
    /// picks the internal (full-fidelity) or external (TSV) path. A no-op if the clipboard has
    /// no text. The window commits any pending edit *before* calling this
    /// (`functional_spec.md §2.2`).
    pub fn paste(
        &mut self,
        sheet: SheetId,
        anchor: CellRef,
        client: &DocumentClient,
        cx: &mut App,
    ) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return; // nothing on the clipboard
        };
        if text.is_empty() {
            return;
        }
        if self.last_copy_text.as_deref() == Some(text.as_str()) {
            // Still the newest thing on the clipboard → paste the full-fidelity worker payload.
            client.send(Command::PasteInternal { sheet, anchor });
        } else {
            // A foreign clipboard change: our slot is no longer the newest, so forget it and
            // paste the plain text as TSV.
            self.last_copy_text = None;
            client.send(Command::PasteTsv {
                sheet,
                anchor,
                text,
            });
        }
    }

    /// Test seam: the decision `paste` would make for a given clipboard `text` + last-copy state,
    /// without a live worker (the detached UI test asserts routing).
    #[cfg(test)]
    pub(crate) fn decide_paste(&self, clipboard_text: Option<&str>) -> PasteRoute {
        match clipboard_text {
            None | Some("") => PasteRoute::None,
            Some(t) if self.last_copy_text.as_deref() == Some(t) => PasteRoute::Internal,
            Some(_) => PasteRoute::Tsv,
        }
    }

    /// Test seam: seed `last_copy_text` as if a `CopyReady` had been folded.
    #[cfg(test)]
    pub(crate) fn set_last_copy_text_for_test(&mut self, tsv: &str) {
        self.last_copy_text = Some(tsv.to_string());
    }
}

/// The route [`ClipboardCoordinator::paste`] would take (test-only).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasteRoute {
    None,
    Internal,
    Tsv,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_prefers_internal_when_text_matches() {
        let mut coord = ClipboardCoordinator::new();
        coord.set_last_copy_text_for_test("1\t2");
        assert_eq!(coord.decide_paste(Some("1\t2")), PasteRoute::Internal);
    }

    #[test]
    fn paste_falls_back_to_tsv_for_foreign_text() {
        let mut coord = ClipboardCoordinator::new();
        coord.set_last_copy_text_for_test("1\t2");
        // A different payload is not ours → external TSV.
        assert_eq!(coord.decide_paste(Some("hello\tworld")), PasteRoute::Tsv);
    }

    #[test]
    fn paste_noop_on_empty_or_absent_clipboard() {
        let coord = ClipboardCoordinator::new();
        assert_eq!(coord.decide_paste(None), PasteRoute::None);
        assert_eq!(coord.decide_paste(Some("")), PasteRoute::None);
        // With no prior copy, any text is foreign → TSV.
        assert_eq!(coord.decide_paste(Some("x")), PasteRoute::Tsv);
    }

    #[gpui::test]
    fn copy_ready_writes_system_clipboard_and_marks_it_ours(cx: &mut gpui::TestAppContext) {
        let mut coord = ClipboardCoordinator::new();
        cx.update(|cx| coord.on_copy_ready("1\t2".to_string(), cx));
        // The TSV is on the system clipboard for other apps…
        let text = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(text.as_deref(), Some("1\t2"));
        // …and a paste of that same text now routes internally (it's still ours).
        assert_eq!(coord.decide_paste(text.as_deref()), PasteRoute::Internal);
    }
}
