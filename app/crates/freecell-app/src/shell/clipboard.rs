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

use freecell_core::{CellRange, SelectionModel, SheetId};
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

    /// Cmd/Ctrl+V: paste into `target` (the destination selection). Reads the system clipboard and
    /// picks the internal (full-fidelity) or external (TSV) path. The internal paste carries the
    /// whole `target` so a single-cell source fills the selection (BUG 4); the TSV paste anchors at
    /// `target.start` (its extent comes from the text). A no-op if the clipboard has no text. The
    /// window commits any pending edit *before* calling this (`functional_spec.md §2.2`).
    pub fn paste(
        &mut self,
        sheet: SheetId,
        target: CellRange,
        client: &DocumentClient,
        cx: &mut App,
    ) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        if let Some(cmd) = self.paste_command(sheet, target, text, false) {
            client.send(cmd);
        }
    }

    /// Cmd/Ctrl+Shift+V: **Paste Values** into `target` (`functional_spec.md §5`). Same
    /// internal-vs-external decision as [`paste`](Self::paste), but the *internal* branch pastes the
    /// worker slot's **evaluated values only** (`Command::PasteValues` — no formulas, no
    /// formatting). A foreign clipboard has no internal payload to strip, so it falls back to the
    /// same external `PasteTsv` path (external TSV is already values). A no-op if the clipboard has
    /// no text. The window commits any pending edit *before* calling this.
    pub fn paste_values(
        &mut self,
        sheet: SheetId,
        target: CellRange,
        client: &DocumentClient,
        cx: &mut App,
    ) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        if let Some(cmd) = self.paste_command(sheet, target, text, true) {
            client.send(cmd);
        }
    }

    /// The [`Command`] a paste of the system-clipboard `text` into `target` should send — the
    /// internal-vs-external routing shared by [`paste`](Self::paste) and
    /// [`paste_values`](Self::paste_values), so the decision has one source of truth (and is
    /// unit-testable without a live worker). `values` selects the *internal* command: our slot is
    /// still the newest thing on the clipboard, so paste the worker payload as **values only**
    /// ([`Command::PasteValues`]) when `values`, else full fidelity ([`Command::PasteInternal`]);
    /// both carry the whole `target` so a single-cell source fills the selection. A foreign
    /// clipboard change forgets our slot and routes an external TSV paste (already values) from the
    /// anchor. `None` (a no-op) when the clipboard is empty/absent.
    fn paste_command(
        &mut self,
        sheet: SheetId,
        target: CellRange,
        text: Option<String>,
        values: bool,
    ) -> Option<Command> {
        let text = text?;
        if text.is_empty() {
            return None;
        }
        if self.last_copy_text.as_deref() == Some(text.as_str()) {
            Some(if values {
                Command::PasteValues { sheet, target }
            } else {
                Command::PasteInternal { sheet, target }
            })
        } else {
            // A foreign clipboard change: our slot is no longer the newest, so forget it and paste
            // the plain text as TSV (at the selection's top-left).
            self.last_copy_text = None;
            Some(Command::PasteTsv {
                sheet,
                anchor: target.start,
                text,
            })
        }
    }

    /// Test seam: seed `last_copy_text` as if a `CopyReady` had been folded.
    #[cfg(test)]
    pub(crate) fn set_last_copy_text_for_test(&mut self, tsv: &str) {
        self.last_copy_text = Some(tsv.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_core::CellRef;

    /// A dummy 1×1 target (the routing under test ignores the geometry).
    fn a1() -> CellRange {
        CellRange::single(CellRef::new(0, 0))
    }

    #[test]
    fn paste_prefers_internal_when_text_matches() {
        let mut coord = ClipboardCoordinator::new();
        coord.set_last_copy_text_for_test("1\t2");
        // Our own payload is still newest → the full-fidelity internal paste.
        assert!(matches!(
            coord.paste_command(SheetId(0), a1(), Some("1\t2".into()), false),
            Some(Command::PasteInternal { .. })
        ));
    }

    #[test]
    fn paste_values_routes_internal_to_paste_values_command() {
        // The load-bearing distinction from `paste`: the internal branch must emit `PasteValues`,
        // NOT `PasteInternal` — a copy/paste slip between the two would otherwise pass silently.
        let mut coord = ClipboardCoordinator::new();
        coord.set_last_copy_text_for_test("1\t2");
        assert!(matches!(
            coord.paste_command(SheetId(0), a1(), Some("1\t2".into()), true),
            Some(Command::PasteValues { .. })
        ));
    }

    #[test]
    fn paste_and_paste_values_both_fall_back_to_tsv_for_foreign_text() {
        // A foreign payload has no internal slot to strip → external TSV in BOTH modes.
        for values in [false, true] {
            let mut coord = ClipboardCoordinator::new();
            coord.set_last_copy_text_for_test("1\t2");
            assert!(matches!(
                coord.paste_command(SheetId(0), a1(), Some("hello\tworld".into()), values),
                Some(Command::PasteTsv { .. })
            ));
            // …and our stale slot is forgotten.
            assert!(coord.last_copy_text.is_none());
        }
    }

    #[test]
    fn paste_noop_on_empty_or_absent_clipboard() {
        let mut coord = ClipboardCoordinator::new();
        for values in [false, true] {
            assert!(coord
                .paste_command(SheetId(0), a1(), None, values)
                .is_none());
            assert!(coord
                .paste_command(SheetId(0), a1(), Some(String::new()), values)
                .is_none());
        }
        // With no prior copy, any text is foreign → TSV.
        assert!(matches!(
            coord.paste_command(SheetId(0), a1(), Some("x".into()), false),
            Some(Command::PasteTsv { .. })
        ));
    }

    #[gpui::test]
    fn copy_ready_writes_system_clipboard_and_marks_it_ours(cx: &mut gpui::TestAppContext) {
        let mut coord = ClipboardCoordinator::new();
        cx.update(|cx| coord.on_copy_ready("1\t2".to_string(), cx));
        // The TSV is on the system clipboard for other apps…
        let text = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(text.as_deref(), Some("1\t2"));
        // …and a paste of that same text now routes internally (it's still ours).
        assert!(matches!(
            coord.paste_command(SheetId(0), a1(), text, false),
            Some(Command::PasteInternal { .. })
        ));
    }
}
