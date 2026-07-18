---
status: draft
---

# Component: Formula Editor (consolidated) + `InputState` highlight API

This is the **delta** for the formula-point-mode + range-highlighting feature. It extends the
shared edit layer already documented in
[`specs/projects/mvp-gaps/components/edit_controller.md`](../../mvp-gaps/components/edit_controller.md)
(that project's committed doc — **do not edit it**; this doc specifies the changes and references
it) and the autocomplete stack shipped in `gaps_closing_7_15 §1`. Architecture refs:
[`../architecture.md`](../architecture.md) §2, §3, §4.3, §5, §6.

Two things are specified here:

1. **The consolidated formula editor** — the promoted `EditController` (`chrome/edit.rs`) as the
   single owner of the formula-feature state (autocomplete, sig-hints, token→color map,
   pending-ref/point-mode), with two thin host adapters (grid overlay; chrome data-row bar) that
   carry **zero** formula logic.
2. **The vendored `InputState` per-range highlight API** — a small public surface added to the
   pinned gpui-component copy so both editors color from one code path.

---

## Purpose and scope

**In:** own all in-progress-formula derived state and the operations over it (recompute
tokens/colors/reference-ready, insert a reference, maintain the pending-ref span), and drive the
editor-token highlight on both `InputState`s. Keep the two host adapters dumb.

**Not in:** the pending **text/commit/cap** state machine (stays in `freecell_core::data_row::DataRow`
+ `ChromeView`, unchanged — `chrome/edit.rs` ownership note lines 1–14); selection movement (grid);
worker/engine mutation (there is none — tokenization is a pure read, `../architecture.md §1`);
non-formula `InputState`s (chart title/axes, number-format, rename) — they never call the highlight
API and never enter this stack.

---

## 1. Consolidated `EditController` (the shared owner)

### 1.1 What consolidates, and why here

Today the formula-feature state is loose fields on `ChromeView` (`autocomplete: Option<Autocomplete>`,
`sig_hint: Option<&'static str>`, plus `quick_edit`), while `EditController` owns only the in-cell
`InputState` + `EditOrigin` + `syncing` guard (`chrome/edit.rs:33-46`). Per the owner decision
(functional_spec §Q3), the formula-feature stack attaches to the **shared edit layer** — the
promoted `EditController` — so there is **one** owner/factory for the formula-editor pair and its
features, not per-editor helpers. `EditController` is the right home because it already is the
"second editor + cross-editor sync" object that both editors funnel through.

Pragmatic constraint (respected): FreeCell keeps the single pending edit inside the **one**
`ChromeView` entity (the deliberate deviation in `chrome/edit.rs:1-14`), so `EditController` does
**not** take ownership of the two `InputState`s away from `ChromeView`; it owns the **derived
formula state**, and `ChromeView`'s methods become thin delegators that read the driving
`InputState`, call `EditController` for the pure/stateful decisions, and write results back to the
`InputState`s. This satisfies "single owner for the formula-feature stack" without re-plumbing the
proven pending-edit ownership.

### 1.2 New state on `EditController`

```rust
pub struct EditController {
    // ── existing (chrome/edit.rs:33-46) ──
    in_cell: Entity<InputState>,
    open: Option<CellRef>,
    origin: EditOrigin,
    syncing: bool,

    // ── new: formula-feature state (relocated + added) ──
    /// The current formula's reference tokens (byte spans + resolved targets + sheet), recomputed
    /// per edit transition; empty for a non-formula / no-complete-ref edit. (`RefToken` is
    /// `freecell_core`; produced by `freecell_engine::lex_formula_refs`.)
    ref_tokens: Vec<freecell_core::RefToken>,
    /// Palette slot per token, parallel to `ref_tokens` (`freecell_core::assign_ref_colors`).
    ref_colors: Vec<u8>,
    /// The just-pointed reference's byte span in the edit text (Q5). `Some` only in the transient
    /// pending-ref window; cleared on any non-point edit transition.
    pending_ref: Option<std::ops::Range<usize>>,
    /// Relocated from ChromeView: the function-autocomplete list state + signature hint, so all
    /// formula-feature state has one owner. (Same shapes as the shipped `Autocomplete`/`sig_hint`.)
    autocomplete: Option<Autocomplete>,
    sig_hint: Option<&'static str>,
}
```

### 1.3 New / changed methods

```rust
impl EditController {
    // ── pending-ref (Q5) ──
    pub fn pending_ref(&self) -> Option<std::ops::Range<usize>>;
    pub fn set_pending_ref(&mut self, span: Option<std::ops::Range<usize>>);

    // ── token/color map (read by the host adapters) ──
    pub fn ref_tokens(&self) -> &[freecell_core::RefToken];
    pub fn ref_colors(&self) -> &[u8];
    /// Same-sheet (target-visible) outlines for the grid: `(target, slot)` per `same_sheet` token
    /// (Q4). Cross-sheet tokens are excluded (colored in the editor only).
    pub fn ref_outlines(&self) -> Vec<(freecell_core::CellRange, u8)>;
    /// Editor-token highlight spans for BOTH inputs: one span per valid ref (same- AND cross-sheet),
    /// byte range + themed color. `is_dark` selects the palette variant.
    pub fn highlight_spans(&self, is_dark: bool) -> Vec<gpui_component::input::HighlightSpan>;

    /// Recompute tokens + colors + reference-ready from the given driving `text` + `caret`, and
    /// (unless `keep_pending`) clear `pending_ref`. Returns whether the caret is reference-ready.
    /// `keep_pending` is set only when re-entered from an insert (so the insert's own pending span
    /// survives). Autocomplete/sig-hint recompute folds in here too (single traversal).
    pub fn recompute_formula(&mut self, text: &str, caret: usize, keep_pending: bool) -> bool;
    pub fn reference_ready(&self) -> bool;   // last recompute_formula result, cached
}
```

`recompute_formula` is the consolidation seam: it calls `freecell_engine::lex_formula_refs(text,
active_sheet_name)` (the active sheet name is passed in by the `ChromeView` delegator, which knows
it) → `freecell_core::assign_ref_colors` → `freecell_core::is_reference_ready`, updates
`ref_tokens`/`ref_colors`/`reference_ready`, and recomputes autocomplete/sig-hint (the existing
`fn_edit_context`/`enclosing_fn_name` logic, now living behind this call). One text/caret read
feeds every formula feature.

### 1.4 `ChromeView` delegators (thin)

`ChromeView` keeps ownership of both `InputState`s and the render/event wiring; its formula methods
become thin:

- `recompute_formula_edit_state(cx)` (generalizes the shipped `recompute_autocomplete`,
  `chrome/view.rs:1383`): read `driving_input().value()`/`.cursor()`; call
  `edit.recompute_formula(text, caret, /*keep_pending=*/ false)`; then drive the editor highlight
  (§1.5) and let `refresh_edit_grid_state` push grid state. Called from **both** Change handlers
  (`on_content_event`, `on_incell_event` — `view.rs:1263-1280`) and the caret-move paths (data-row
  intercept + `AutocompleteCaretMoved`, `view.rs:1494`), replacing the direct
  `recompute_autocomplete` calls.
- `insert_reference(a1, replace_pending, window, cx)`: the point-mode splice — the exact analog of
  `accept_autocomplete` (`view.rs:1516-1569`); see `../architecture.md §5` step-by-step. Sets the
  new pending span via `edit.set_pending_ref(..)`, then calls `edit.recompute_formula(new_text,
  new_caret, /*keep_pending=*/ true)` so the just-set pending span survives its own recompute.
- `refresh_edit_grid_state` (`view.rs:1322`): additionally fills the three new `EditState` fields —
  `reference_ready = editing_formula && edit.reference_ready()`, `pending_ref =
  edit.pending_ref().is_some()`, `ref_outlines = edit.ref_outlines()` (`../architecture.md §3.1`).

### 1.5 Driving the editor highlight (both editors, one path)

In `recompute_formula_edit_state`, after `recompute_formula`, build the spans once and apply to
both owned inputs (the chrome owns both, so this is one code path — `../architecture.md §4.2`):

```rust
let is_dark = /* active gpui appearance */;
let spans = self.edit.highlight_spans(is_dark);
self.content_input.update(cx, |i, cx| i.set_highlights(spans.clone(), cx));
if self.edit.is_open() {
    self.edit.in_cell().update(cx, |i, cx| i.set_highlights(spans, cx));
}
```

Cleared (`clear_highlights`) whenever the edit is non-formula or ends (commit/cancel), alongside
the existing `EditState` clear. The in-cell input is the same entity the grid renders as the
overlay (`grid/view.rs:3209`), so its colored runs reach the grid frame with no grid-side highlight
wiring — the grid only paints **outlines** (§ host adapter B).

### 1.6 Host adapter A — grid overlay (zero formula logic)

The grid consumes only pushed primitives; it computes no formula state:
- Stores `reference_ready: bool`, `pending_ref: bool`, `ref_outlines: Vec<(CellRange, u8)>` from
  the extended `set_edit_state` (`grid/view.rs:795`; add the three params, cleared on
  `set_active_sheet` like the `incell_*` fields at `view.rs:896-900`).
- `mouse_down_cell` (`view.rs:1461`) point branch: `point_ready = reference_ready || pending_ref`
  → emit `GridEvent::InsertReference { a1, replace_pending: pending_ref }`, arm `point_drag`, no
  `SelectionChanged` (`../architecture.md §3.2`).
- `point_drag` state machine + merge helpers (`../architecture.md §3.3-3.4`); outline paint
  (`../architecture.md §4.1`); preview paint distinct from selection + outlines.
- Renders the in-cell `InputState` as today — its colored runs come from the chrome's
  `set_highlights`, not from the grid.

### 1.7 Host adapter B — chrome data-row bar (zero formula logic)

The data-row editor renders the `content_input` (whose colored runs come from `set_highlights`) +
its existing autocomplete/sig-hint popovers. No formula computation lives here — the bar reads
`edit`-owned state via the delegators. Point-mode reaches the data-row edit through the **grid's**
`InsertReference` (a data-row formula edit is still driven by grid clicks); the data-row field
itself is not a grid surface, so clicking **into** it is ordinary caret placement (`functional_spec.md
§5`).

### 1.8 Migration (keep green)

1. **Relocate** `autocomplete`/`sig_hint` from `ChromeView` onto `EditController`; route the shipped
   `recompute_autocomplete`/`accept_autocomplete` through `edit`-owned state. All existing
   `gaps_closing_7_15 §1` view tests + the `data_row` unit tests must pass unchanged. (Pure move.)
2. Add `ref_tokens`/`ref_colors`/`pending_ref` + `recompute_formula` (folding the relocated
   autocomplete recompute in); wire `recompute_formula_edit_state` into the Change + caret-move
   paths. Grid outlines + editor highlight land here (highlight needs §2).
3. Add `insert_reference` + the grid point routing + `point_drag`.

Each step is crate-scoped-buildable and independently testable.

---

## 2. Vendored `gpui_component::input::InputState` highlight API

**A small, public, caller-driven per-range foreground-highlight API added to the pinned
gpui-component copy** (`app/Cargo.toml` git pin `a9a7341c35b62f27ff512371c62419342264710c`).
Independent of the crate's internal `SyntaxHighlighter`/LSP/`display_map` path (whose accessors are
`pub(super)`), so an external caller can drive per-keystroke highlights on a plain single-line
input. Architecture ref: `../architecture.md §4.3`.

### 2.1 Public surface (new)

```rust
// gpui_component::input
/// A caller-supplied foreground-color span over the input's current value, in BYTE offsets
/// (consistent with `InputState::cursor()` and FreeCell's lexer byte spans). Out-of-range or
/// non-char-boundary spans are clamped; overlapping spans resolve last-wins.
#[derive(Debug, Clone, PartialEq)]
pub struct HighlightSpan {
    pub range: std::ops::Range<usize>,
    pub color: gpui::Hsla,
}

impl InputState {
    /// Replace ALL caller-driven highlights (pass empty to clear). Overlaid as foreground-color
    /// runs on top of the base text style at paint time. Notifies + repaints. Independent of the
    /// CodeEditor/SyntaxHighlighter path — usable on any InputState.
    pub fn set_highlights(&mut self, spans: Vec<HighlightSpan>, cx: &mut Context<Self>);
    /// Clears all caller-driven highlights.
    pub fn clear_highlights(&mut self, cx: &mut Context<Self>);
}
```

### 2.2 Implementation (bounded, upstreamable)

- Add a field `highlights: Vec<HighlightSpan>` to `InputState` (default empty).
- In the run-building path that produces the value's `TextRun`s for paint, after the base run(s)
  are constructed, **split runs at each highlight boundary and override the foreground color** on
  the covered runs. Map each `HighlightSpan.range` (byte) to the run builder's internal index unit
  (byte / char / utf-16 — **confirm against the input's existing `cursor()`↔`Position`↔run
  conversions at impl**; the input already does these conversions), clamping to char boundaries.
- Empty `highlights` ⇒ produce the base runs unchanged ⇒ **no behavioural change** for every input
  that never calls the API (the non-formula inputs, the demo/tests). This keeps the vendored edit
  strictly additive and safe.
- Interplay with the existing `SyntaxHighlighter`: FreeCell uses only the plain single-line input
  (no CodeEditor mode), so the two never combine in FreeCell. Spec the caller highlights to apply
  **after** (on top of) any syntax runs if both are present, so the API is well-defined upstream
  even in CodeEditor mode.

### 2.3 Versioning / upstreaming

This edits the vendored gpui-component at its pinned rev. Carry it the same disciplined way as the
IronCalc fork fixes (one focused change): bump/patch the `app/Cargo.toml` pin to the branch
carrying this addition, and prepare a single-feature upstream PR ("external per-range text
highlights on `InputState`") for gpui-component. The **grid-outline** value (`../architecture.md
§4.1`) ships independently of this change, so a coding phase lands + validates outlines before this
edit is merged (implementation plan phase order).

### 2.4 Test

Upstream-style unit test committed with the change: a single-line `InputState` seeded with
`"=A1+B2"` and two `HighlightSpan`s → the built runs carry the two overridden foreground colors at
the right offsets; `set_highlights(vec![])` / `clear_highlights` restore the base runs. FreeCell's
pixel effect is covered by the `incell_editor_token_colors` baseline (`../architecture.md §8`).

---

## Dependencies

Depends on: `freecell_core::{RefToken, CellRange, palette::REF_HIGHLIGHT_PALETTE, assign_ref_colors,
is_reference_ready}`; `freecell_engine::lex_formula_refs`; the vendored
`gpui_component::input::{InputState, HighlightSpan}`; the existing `DataRow` reducer + the shipped
autocomplete stack. Depended on by: `ChromeView` (delegators + render), `GridView` (point routing +
outline paint via the extended `EditState`/`set_edit_state`), `WorkbookWindow` (the `InsertReference`
route + the `EditState` forward).

## Test plan (component-level)

- Relocation regression: all shipped `gaps_closing_7_15 §1` autocomplete view tests + `data_row`
  unit tests pass after the state moves onto `EditController` (migration step 1 gate).
- `recompute_formula`: given text+caret, populates `ref_tokens`/`ref_colors`, returns the correct
  `reference_ready`, clears `pending_ref` unless `keep_pending`, and keeps autocomplete/sig-hint in
  lockstep (one recompute drives all three).
- `ref_outlines` excludes cross-sheet tokens; `highlight_spans` includes them.
- `insert_reference` + pending-ref lifecycle (append/replace/self-ref; clear on keystroke/caret/commit;
  own-insert doesn't clear) — the `../architecture.md §8` gpui view tests.
- Vendored API unit test (§2.4).
