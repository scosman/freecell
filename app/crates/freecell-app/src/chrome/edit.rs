//! [`EditController`] — the in-cell editor + cross-editor sync layered over the data row
//! (`components/edit_controller.md`).
//!
//! **Ownership note (deviation from the component doc — see `DECISIONS_TO_REVIEW.md` Phase 2).**
//! The component doc sketches a `WorkbookWindow`-owned controller owning *both* editor
//! `InputState`s and the whole pending-edit state machine. FreeCell instead keeps the single
//! pending edit inside **one entity** — [`ChromeView`](super::ChromeView), which already owns the
//! data-row `InputState` and the proven [`freecell_core::data_row::DataRow`] reducer
//! (fetch / spinner / disabled / cap / commit / escape, all table-tested). This
//! [`EditController`] holds the **second** editor — the in-cell overlay `InputState` — plus the
//! overlay's open cell, the current [`EditOrigin`], and the `syncing` re-entrancy guard. The two
//! editors therefore sync *within one entity* (no cross-entity `InputState` feedback loop). The
//! canonical pending **text + commit/cap** live in the `DataRow` reducer; this controller adds the
//! in-cell editor, the two-way text sync, and origin tracking on top.

use std::ops::Range;

use gpui::Entity;
use gpui_component::input::InputState;

use freecell_core::functions::{self, FnSig};
use freecell_core::{assign_ref_colors, is_reference_ready, CellRange, CellRef, RefToken};
use freecell_engine::lex_formula_refs;

/// Which editor currently drives the shared pending edit (== has focus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOrigin {
    /// The data-row (formula bar) editor.
    DataRow,
    /// The in-cell overlay editor.
    InCell,
}

/// The function-autocomplete list's live state while the dropdown is open (`gaps_closing_7_15 §1`).
/// Relocated onto the shared [`EditController`] (formula-point-mode consolidation, Q3) so all
/// formula-feature state has one owner. Cleared to `None` when the list dismisses.
pub struct Autocomplete {
    /// The current filtered, ordered match list (from [`functions::complete`]).
    pub matches: Vec<&'static FnSig>,
    /// The highlighted row index into [`matches`](Self::matches).
    pub highlight: usize,
    /// The byte offset in the edit text where the typed prefix begins — the replace point on
    /// accept.
    pub token_start: usize,
}

/// The in-cell editor half of the single pending edit (`components/edit_controller.md`). Owned by
/// [`ChromeView`](super::ChromeView); the data-row half is the chrome's existing `content_input` +
/// `DataRow` reducer.
pub struct EditController {
    /// The reused in-cell overlay input (one instance for every in-cell edit). Created in
    /// `ChromeView::new` (where `&mut Window` is available) and rendered by the **grid** as an
    /// absolute overlay; the chrome subscribes to it for its edit events.
    in_cell: Entity<InputState>,
    /// The cell the in-cell overlay currently covers, or `None` when the overlay is closed. The
    /// data-row editor can be the active `origin` while the overlay is open (focus moved back).
    open: Option<CellRef>,
    /// Which editor currently drives the edit (follows focus).
    origin: EditOrigin,
    /// Re-entrancy guard for the two-way text sync: set around a programmatic `set_value` push so
    /// the resulting (suppressed) echo is ignored belt-and-braces.
    syncing: bool,

    // ── formula-feature state (formula-point-mode consolidation, Q3 — `architecture.md §2.4`) ──
    /// The in-progress formula's reference tokens (byte spans + resolved targets + sheet),
    /// recomputed per edit transition; empty for a non-formula / no-complete-ref edit. Produced by
    /// [`freecell_engine::lex_formula_refs`].
    ref_tokens: Vec<RefToken>,
    /// Palette slot per token, parallel to [`ref_tokens`](Self::ref_tokens)
    /// ([`freecell_core::assign_ref_colors`]).
    ref_colors: Vec<u8>,
    /// The just-pointed reference's byte span in the edit text (Q5). `Some` only in the transient
    /// pending-ref window; cleared on any non-point edit transition. Set by Phase 3's
    /// `insert_reference`; plumbed here so the state has one owner.
    pending_ref: Option<Range<usize>>,
    /// Whether the driving editor's live caret is reference-ready (the last
    /// [`recompute_formula`](Self::recompute_formula) result, cached). Pushed to the grid; consumed
    /// by the grid's point-vs-commit branch in Phase 3.
    reference_ready: bool,
    /// The function-autocomplete dropdown's live state while open, else `None` (relocated from
    /// `ChromeView`, `gaps_closing_7_15 §1`).
    autocomplete: Option<Autocomplete>,
    /// The active passive signature-hint template (the whole `NAME(args…)` line), or `None` when
    /// the caret is not inside a recognized call (relocated from `ChromeView`).
    sig_hint: Option<&'static str>,
}

impl EditController {
    /// Builds the controller over the reused in-cell input, overlay closed, driving from the data
    /// row.
    pub fn new(in_cell: Entity<InputState>) -> Self {
        Self {
            in_cell,
            open: None,
            origin: EditOrigin::DataRow,
            syncing: false,
            ref_tokens: Vec::new(),
            ref_colors: Vec::new(),
            pending_ref: None,
            reference_ready: false,
            autocomplete: None,
            sig_hint: None,
        }
    }

    /// The reused in-cell input handle (the window hands a clone to the grid to render the
    /// overlay).
    pub fn in_cell_input(&self) -> Entity<InputState> {
        self.in_cell.clone()
    }

    /// The in-cell input handle by reference (for `cx` updates without cloning).
    pub fn in_cell(&self) -> &Entity<InputState> {
        &self.in_cell
    }

    /// The cell the in-cell overlay covers, if it is open.
    pub fn open_cell(&self) -> Option<CellRef> {
        self.open
    }

    /// Whether the in-cell overlay is currently open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Which editor currently drives the edit.
    pub fn origin(&self) -> EditOrigin {
        self.origin
    }

    /// Opens the overlay on `cell`, driving from the in-cell editor.
    pub fn open_on(&mut self, cell: CellRef) {
        self.open = Some(cell);
        self.origin = EditOrigin::InCell;
    }

    /// Closes the overlay and returns the driver to the data row.
    pub fn close(&mut self) {
        self.open = None;
        self.origin = EditOrigin::DataRow;
    }

    /// Sets which editor is driving (focus moved between the two editors — no text moves).
    pub fn set_origin(&mut self, origin: EditOrigin) {
        self.origin = origin;
    }

    /// Raises/lowers the sync guard around a programmatic `set_value` push into the *other*
    /// editor, so the resulting (already event-suppressed) echo is ignored belt-and-braces.
    pub fn set_syncing(&mut self, syncing: bool) {
        self.syncing = syncing;
    }

    /// Whether a sync push is currently in progress (an incoming editor event should be ignored).
    pub fn is_syncing(&self) -> bool {
        self.syncing
    }

    // ── formula-feature state (formula-point-mode, `architecture.md §2.4`, component §1.3) ──

    /// The just-pointed reference's byte span, if a point action is pending (Q5).
    pub fn pending_ref(&self) -> Option<Range<usize>> {
        self.pending_ref.clone()
    }

    /// Sets (or clears) the pending-ref span — set by Phase 3's `insert_reference`.
    pub fn set_pending_ref(&mut self, span: Option<Range<usize>>) {
        self.pending_ref = span;
    }

    /// The current formula's reference tokens (byte span + resolved target + sheet). Includes
    /// cross-sheet tokens (the color map colors every valid ref for the future in-editor control).
    pub fn ref_tokens(&self) -> &[RefToken] {
        &self.ref_tokens
    }

    /// The palette slot per token, parallel to [`ref_tokens`](Self::ref_tokens).
    pub fn ref_colors(&self) -> &[u8] {
        &self.ref_colors
    }

    /// The same-sheet (target-visible) highlights for the grid: `(target, slot)` per `same_sheet`
    /// token (Q4), drawn as a rich fill + border. Cross-sheet tokens are excluded here (the color
    /// map still colors them for the future in-editor control — `architecture.md §4.1`).
    pub fn ref_highlights(&self) -> Vec<(CellRange, u8)> {
        self.ref_tokens
            .iter()
            .zip(self.ref_colors.iter())
            .filter(|(token, _)| token.same_sheet)
            .map(|(token, &slot)| (token.target, slot))
            .collect()
    }

    /// The autocomplete list's live state, if open.
    pub fn autocomplete(&self) -> Option<&Autocomplete> {
        self.autocomplete.as_ref()
    }

    /// The autocomplete list's live state mutably, if open (nav / highlight adjust).
    pub fn autocomplete_mut(&mut self) -> Option<&mut Autocomplete> {
        self.autocomplete.as_mut()
    }

    /// Takes (clears + returns) the autocomplete list state (accept / dismiss).
    pub fn take_autocomplete(&mut self) -> Option<Autocomplete> {
        self.autocomplete.take()
    }

    /// The active signature-hint template, if any.
    pub fn sig_hint(&self) -> Option<&'static str> {
        self.sig_hint
    }

    /// Sets the signature-hint template (used by the autocomplete-accept path).
    pub fn set_sig_hint(&mut self, hint: Option<&'static str>) {
        self.sig_hint = hint;
    }

    /// Whether the driving caret is reference-ready (cached last recompute result).
    pub fn reference_ready(&self) -> bool {
        self.reference_ready
    }

    /// Zeroes all formula-feature derived state — highlights, autocomplete, sig-hint, reference-ready,
    /// and the pending-ref span. Called on commit / cancel / cap-error (highlights must be fully
    /// removed the instant an edit ends — `functional_spec.md §3` lifecycle).
    pub fn clear_formula_state(&mut self) {
        self.ref_tokens.clear();
        self.ref_colors.clear();
        self.reference_ready = false;
        self.autocomplete = None;
        self.sig_hint = None;
        self.pending_ref = None;
    }

    /// Recomputes every formula-feature datum from the driving editor's `text` + `caret` in one
    /// pass (the consolidation seam — `architecture.md §6`, component §1.3): reference tokens
    /// ([`lex_formula_refs`]) + their palette slots ([`assign_ref_colors`]), the reference-ready
    /// predicate ([`is_reference_ready`]), and the function-autocomplete list + signature hint.
    /// Clears the pending-ref span unless `keep_pending` (set only when re-entered from an insert,
    /// so the insert's own span survives its recompute). Returns the reference-ready result (also
    /// cached in [`reference_ready`](Self::reference_ready)).
    pub fn recompute_formula(
        &mut self,
        text: &str,
        caret: usize,
        active_sheet_name: &str,
        keep_pending: bool,
    ) -> bool {
        if !keep_pending {
            self.pending_ref = None;
        }
        // Reference highlights + color map (grid highlights now; future in-editor control).
        self.ref_tokens = lex_formula_refs(text, active_sheet_name);
        self.ref_colors = assign_ref_colors(&self.ref_tokens);
        // Point-mode predicate (consumed by the grid in Phase 3).
        self.reference_ready = is_reference_ready(text, caret);
        // Function autocomplete list — carry the highlight across a same-token refresh (typing more
        // of the same name); reset to the top when the token moved or the list shrank past it.
        self.autocomplete = match functions::fn_edit_context(text, caret) {
            Some(ctx) => {
                let matches = functions::complete(&ctx.prefix);
                if matches.is_empty() {
                    None
                } else {
                    let highlight = match &self.autocomplete {
                        Some(prev) if prev.token_start == ctx.token_start => {
                            prev.highlight.min(matches.len() - 1)
                        }
                        _ => 0,
                    };
                    Some(Autocomplete {
                        matches,
                        highlight,
                        token_start: ctx.token_start,
                    })
                }
            }
            None => None,
        };
        // Passive signature hint while the caret sits inside a recognized call.
        self.sig_hint = functions::enclosing_fn_name(text, caret)
            .and_then(functions::signature)
            .map(|s| s.template);
        self.reference_ready
    }
}
