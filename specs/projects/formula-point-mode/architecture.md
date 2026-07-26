---
status: complete
---

# Architecture: Formula Point-Mode + Range Highlighting

Technical design for the feature fixed in `functional_spec.md` (behaviour + DPM.1–8 are
**locked**; owner resolved the editor-consolidation question — spec §Q3). This doc resolves
the six architecture questions, specifies the data model + seams + algorithms, and hands the
coding agent a plan with no significant technical decisions left. Component-level detail for
the consolidated editor lives in
**[`components/formula_editor.md`](components/formula_editor.md)**; this doc references it
rather than repeating it. **Scope note (owner, 2026-07-18):** in-editor token coloring is
**out of v0.5** — deferred to the v1.0 FreeCell styled text-input control
([`../../../projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)),
which will consume this project's color map. **There is no gpui-component / vendored-widget
change in v0.5.**

**Crate map (unchanged, `gaps_closing_7_15/architecture.md`):** `freecell-core` (pure — no
gpui, **no IronCalc**, enforced by `tests/dependency_rule.rs`), `freecell-engine` (IronCalc-fork
adapter + worker; the **only** crate that may touch IronCalc types; depends on `freecell-core`),
`freecell-app` (GPUI shell: `grid/`, `chrome/`, `shell/`; depends on both). This split is the
spine of the whole design: reference tokenization needs the **real** IronCalc lexer, so it lives
in `freecell-engine`; the pure predicates/palette/color-assignment live in `freecell-core`; all
GPUI wiring lives in `freecell-app`.

**Standing conventions (`CLAUDE.md`):** crate-scoped build/test per phase; `cargo fmt --all
--check` every phase; render **subset** while iterating the grid phases; **one** full render
suite + CI `render` gate in the final phase; commit + push regularly; run cargo from `app/`.
The feature is FreeCell-only: no engine-fork change and **no gpui-component / vendored-widget
change** (in-editor coloring, which would have needed the widget change, is deferred — see the
scope note above).

---

## 0. Design overview + data flow

The feature threads one new piece of derived state through the existing single-pending-edit
plumbing: **the reference tokens of the in-progress formula**. Everything else is a consumer of
that list.

Per edit transition (keystroke, caret move, point insert), while the edit text starts with `=`:

```
 driving InputState (.value(), .cursor())                    [freecell-app: chrome]
        │  text, caret(byte)
        ▼
 freecell_engine::lex_formula_refs(text, active_sheet_name)  [engine: real IronCalc Lexer, Q1]
        │  Vec<freecell_core::RefToken>  (byte span + 0-based CellRange + sheet + valid)
        ▼
 freecell_core::assign_ref_colors(&tokens)                   [core: DPM.3, pure]
        │  Vec<u8>  (palette slot per token, first-appearance order, mod 7)
        ▼
 EditController formula-feature state  (tokens, colors, pending_ref)   [shared edit layer, Q3/Q5]
        └──► ChromeGridRequest::EditState { reference_ready, pending_ref, ref_highlights, … }
                 │                                            [Q2 routing + Q4 same-sheet highlights]
                 ▼
             GridView.set_edit_state(...)  → paints highlights (§4.1); mouse_down_cell consults
             reference_ready / pending_ref to branch point-insert vs commit (§3.2); emits
             GridEvent::InsertReference { a1, replace_pending }  ──► chrome splices the ref.
```

The token→color map is computed **once** on the shared edit state (`EditController`); the grid
paints the same-sheet **highlights** (rich fill + border, §4.1) from it. The two "editors" — the
data-row `content_input` and the in-cell overlay `edit.in_cell()`, both
`gpui_component::input::InputState` and both owned by the single `ChromeView` entity
(`chrome/edit.rs` ownership note, lines 1–14) — share that one pending edit, so the highlight
reflects the edit regardless of which editor is focused. In-editor coloring of the reference
*tokens inside the formula text* is **deferred** (scope note above); the color map is preserved
because it drives the grid highlights now and the future styled-control will consume it. No
per-editor formula logic exists — see §6 and the component doc.

---

## 1. Q1 — Tokenization seam (source + cadence)

**Decision: a synchronous, Model-free free function in `freecell-engine`, called directly from
the app layer on each edit transition. No worker round-trip.**

### 1.1 Why engine, synchronous, Model-free

- The IronCalc `Lexer` is **public and pure**: `ironcalc_base::expressions::lexer::{Lexer,
  LexerMode}`, `Lexer::new(formula, LexerMode::A1, locale, language)` + `next_token() ->
  TokenType` + `get_position()`, with `TokenType::Reference { .. }` / `TokenType::Range { .. }`
  public. Confirmed by the round-3 API audit
  (`experiments/round-3/B-api-audit/src/formula_helpers.rs:1-60` — it already tokenizes a formula
  and counts reference/range tokens with exactly this call). `locale`/`language` are process-wide
  statics (`get_locale("en")`, `get_language("en")`), so **the lexer needs no `Model`** — the
  token's row/column are the literal 1-based A1 coordinates (`A1` → row 1, col 1; absolute flags
  are separate), so "what does this reference point at" is context-free and needs neither the
  editing cell nor the live document. This makes lexing a **pure function of the string**.
- Because it needs no `Model`, it does **not** have to go through the worker (the worker exists to
  serialize stateful `UserModel` mutations). A worker round-trip would be async — wrong for a
  per-keystroke highlight (latency + reordering under fast typing). A direct synchronous call on
  the main thread is correct and cheap: formula strings are short (≤ the input cap), the lexer is a
  single O(n) pass, and this runs at most once per keystroke/caret-move — the same cadence the
  shipped autocomplete recompute already runs at (`chrome/view.rs:1383 recompute_autocomplete`).
- It must live in `freecell-engine`, not `freecell-core`: core may not depend on IronCalc
  (`tests/dependency_rule.rs`). This is the exact reason the autocomplete feature used a lexical
  *heuristic* in core (`functions.rs`); this feature needs the **real** tokenizer, so it sits one
  crate up. `freecell-app` already depends on `freecell-engine` (it owns `DocumentClient`), so it
  can call the free function directly with no new dependency.

### 1.2 Engine public surface

New module `freecell-engine/src/formula_refs.rs`, re-exported from `lib.rs`:

```rust
/// Tokenize `edit_text` (the full pending edit, **including** the leading `=`) with the real
/// IronCalc lexer and return its reference/range tokens as gpui-free `freecell_core::RefToken`s,
/// with byte spans in `edit_text` coordinates and 0-based resolved targets. Non-`=` text, or a
/// text with no complete references, returns an empty vec. `active_sheet_name` is the visible
/// sheet's name, used to set each token's `same_sheet` flag (Q4).
///
/// Pure + synchronous + Model-free: safe to call on the render/main thread per keystroke.
pub fn lex_formula_refs(edit_text: &str, active_sheet_name: &str) -> Vec<freecell_core::RefToken>;
```

Implementation notes (bind the exact `TokenType` field names against the pinned fork at
implementation — the fork is not checked out here; the round-3 audit confirms the variants exist):

1. Return empty unless `edit_text.starts_with('=')`. Strip the leading `=`; the IronCalc lexer
   takes the formula **body without `=`** (the audit passes `"A2+B3*2"`, not `"=A2+B3*2"`). Lex the
   body; **all byte spans are body-relative, so add `1`** to map them back into `edit_text`.
2. `Lexer::new(body, LexerMode::A1, get_locale("en"), get_language("en"))`. Loop `next_token()`
   until `EOF`; bracket each token's byte span with `get_position()` (position before vs after the
   token) — or the token's own start/len if the fork exposes it (confirm at impl). Keep only
   `TokenType::Reference { .. }` and `TokenType::Range { .. }`; ignore idents, numbers, strings,
   operators, and stop contributing at the first `Illegal`/`EOF` (a partial trailing ref lexes as
   `Illegal` → **not** emitted, satisfying "invalid/partial refs never highlight",
   `functional_spec.md §3`).
3. Build the target `CellRange` (0-based, normalized top-left→bottom-right) from the token's
   1-based `row`/`column` (subtract 1). A `Reference` → `CellRange::single`; a `Range` →
   `CellRange` over both endpoints normalized (drag direction / endpoint order irrelevant).
4. Sheet (Q4): the token carries `sheet: Option<String>` (a qualifier like `Sheet2!A1`; `None`
   for an unqualified ref). Set `same_sheet = sheet.is_none() ||
   sheet.as_deref().eq_ignore_ascii_case(active_sheet_name)` and record `sheet` on the token
   (used as the color key so `Sheet2!A1` ≠ `A1`).

The result is a `Vec<RefToken>` — a plain `freecell_core` data type (no IronCalc types cross the
crate boundary), consumable by pure core code and the app.

---

## 2. Data model — shared edit state additions

### 2.1 `freecell_core::RefToken` (new, in `refs.rs`)

```rust
/// One complete reference token found in an in-progress formula (`architecture.md §1`).
/// Produced by `freecell_engine::lex_formula_refs`; consumed by color assignment + the grid
/// highlight pass (the `span` is also what the future v1.0 styled text-input control will use
/// to color the token in-editor). gpui-free + IronCalc-free (plain data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefToken {
    /// Byte span of the token text within the full edit text (leading `=` included).
    pub span: std::ops::Range<usize>,
    /// The resolved target rectangle, 0-based, normalized (single cell → 1×1 range).
    pub target: CellRange,
    /// The reference's sheet qualifier, if any (`Some("Sheet2")` for `Sheet2!A1`, else `None`).
    /// Part of the color key so a same-target ref on another sheet gets its own color.
    pub sheet: Option<String>,
    /// Whether `target` resolves to the currently visible sheet (Q4): `sheet` is `None` or names
    /// the active sheet. Only `same_sheet` tokens draw a grid highlight; the color map still
    /// assigns every token a color (consumed by the future in-editor styling control).
    pub same_sheet: bool,
}
```

### 2.2 Palette + color assignment (DPM.3, `freecell_core::palette`)

```rust
/// One reference-highlight color, with a light- and dark-theme variant (theme-aware, DPM.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefColor { pub light: Rgb, pub dark: Rgb }

/// The fixed 7-color reference-highlight cycle (DPM.3). Distinct + legible as a grid fill + border
/// (and, for the future in-editor styling control, as editor text), in both themes. Curated from
/// Excel's colored-refs feel; hexes chosen for contrast against the light cell background and the
/// dark cell background respectively.
pub const REF_HIGHLIGHT_PALETTE: [RefColor; 7] = [ /* … 7 authored light/dark pairs … */ ];

/// The palette slot (0..7) for `index`, recycling past 7 (`index % 7`).
pub fn ref_color(index: usize) -> RefColor { REF_HIGHLIGHT_PALETTE[index % REF_HIGHLIGHT_PALETTE.len()] }

/// Assign a palette **slot** to each token by distinct resolved reference, first-appearance
/// order (DPM.3): two tokens with the same `(sheet, target)` share a slot; a new distinct ref
/// takes the next slot; slot = distinct-index % 7. Returns one slot per input token (parallel).
/// Pure — unit-tested headless.
pub fn assign_ref_colors(tokens: &[RefToken]) -> Vec<u8>;
```

`assign_ref_colors` walks tokens left-to-right, keying an insertion-ordered map on
`(sheet.clone(), target)`; the slot is the key's insertion index `% 7`. First-appearance order
makes colors **stable** as the user types a later ref (never recolors earlier ones); removing an
earlier ref may shift later slots by one (cosmetic, accepted, `functional_spec.md §5`).

### 2.3 Reference-ready predicate (§1 of the spec, `freecell_core::functions`)

```rust
/// Whether a grid click at `caret` (byte offset) should point-insert vs commit — the spec's
/// "reference-ready" caret (`functional_spec.md §1`). True iff the text is a formula, the caret
/// is not inside a string literal, and the nearest non-space char before it is the leading `=`
/// (empty formula) or an operator/opener/comma/colon. Reuses the existing `is_function_position_prev`
/// operator set + `in_string_at` (the same syntactic slots autocomplete treats as function
/// position — a ref and a function name are legal in exactly the same places). Pure, headless.
pub fn is_reference_ready(text: &str, caret: usize) -> bool;
```

This lives beside the existing `fn_edit_context`/`is_function_position_prev` in `functions.rs`
(already private helpers there). It is the pure decision core; the chrome evaluates it against the
driving editor's live caret and pushes only the boolean to the grid (§3.1).

### 2.4 Consolidated formula-feature state (on `EditController`)

The formula-feature state moves onto the shared edit layer — the promoted `EditController`
(`chrome/edit.rs`) — rather than staying loose `ChromeView` fields (Q3 consolidation; detail +
signatures in **`components/formula_editor.md`**). New state it owns:

- `ref_tokens: Vec<RefToken>` and `ref_colors: Vec<u8>` (parallel) — the current formula's
  highlight map, recomputed per edit transition.
- `pending_ref: Option<Range<usize>>` — the byte span of the just-pointed reference (Q5, §5).
- the existing autocomplete + sig-hint state (relocated here from `ChromeView`, so all
  formula-feature state has one owner — see the component doc's migration note).

Note `.cursor()` (the caret byte offset) is **not** stored on the shared state and is **not**
pushed to the grid — it is read on demand from whichever `InputState` is driving. Keeping the
caret as the input's single source of truth (rather than mirroring it, as the spec's Q2 sketch
tentatively suggested) avoids a second copy that could drift; the grid only needs the *derived*
booleans (§3.1). This is the one place the architecture refines the spec's recommended mechanism;
behaviour is unchanged. (Pushback note, §10.)

---

## 3. Q2 — Routing a grid click into the active editor

**Decision: extend `ChromeGridRequest::EditState` with `reference_ready` + `pending_ref` booleans
+ a same-sheet `ref_highlights` list; add `GridEvent::InsertReference { a1, replace_pending }`; the
grid consults the pushed booleans in `mouse_down_cell` to branch point-insert vs commit, and runs
a local `point_drag` state machine for the drag/replace behaviour.**

### 3.1 `EditState` extension (chrome → grid)

`ChromeGridRequest::EditState` (`chrome/mod.rs:106-127`) today carries `mirror`, `in_cell`, `cap`,
`quick_edit`, `autocomplete`, `sig_hint` — **no caret, and no signal a data-row edit is active**.
Add three fields:

```rust
EditState {
    // … existing six …
    /// The reference-ready predicate (`freecell_core::is_reference_ready`) evaluated at the
    /// driving editor's live caret. Only ever `true` during a formula edit; `false` otherwise.
    /// The grid reads it at mouse-down to choose point-insert vs commit (Q2).
    reference_ready: bool,
    /// Whether a just-pointed reference is pending (a point action with nothing typed since).
    /// While set, a grid click **replaces** it even when the caret is not reference-ready
    /// (the pending-ref override, `functional_spec.md §2`).
    pending_ref: bool,
    /// The same-sheet reference highlights to paint on the grid: each visible-sheet target range
    /// with its palette slot (Q4), drawn as a rich fill + border. Cross-sheet tokens are omitted
    /// here (the color map still colors them for the future in-editor control). Empty while not
    /// editing / no same-sheet refs.
    ref_highlights: Vec<(CellRange, u8)>,
}
```

`ChromeView::refresh_edit_grid_state` (`chrome/view.rs:1322-1365`) fills these: `reference_ready =
editing_formula && is_reference_ready(text, caret)`; `pending_ref = self.edit.pending_ref().is_some()`;
`ref_highlights` = the `same_sheet` subset of `ref_tokens`, each mapped to `(token.target,
ref_colors[i])`. The window's `EditState` handler (`shell/window.rs:1925-1957`) forwards them to
`GridView::set_edit_state` (add the three params; stored as `self.reference_ready`,
`self.pending_ref`, `self.ref_highlights`; `set_active_sheet` and the idle-clear path zero them,
mirroring the existing `incell_*` fields at `view.rs:896-900`).

### 3.2 The mouse-down branch (`mouse_down_cell`, `grid/view.rs:1461`)

At the **top** of `mouse_down_cell`, before the selection/emit + drag-arm code, add the point-mode
branch:

```
let point_ready = self.reference_ready || self.pending_ref;      // both false unless editing a formula
if point_ready && !event.modifiers.shift {                       // shift-click never points (range-extend semantics stay)
    let cell = self.resolve_merge_anchor(row, col);              // Q6, §3.4
    let a1 = CellRange::single(cell).to_a1();                    // freecell_core::CellRange::to_a1
    self.events.emit(&GridEvent::InsertReference { a1, replace_pending: self.pending_ref }, window, cx);
    self.point_drag = Some(PointDrag { origin: cell, last_range: CellRange::single(cell) });
    window.prevent_default();                                    // keep editor focus (as the dbl-click path does, view.rs:1498)
    cx.notify();
    return;                                                      // NO set_selection_and_emit, NO DragMode::Cell
}
// … existing behaviour (commit-on-click) unchanged below …
```

- **Not point-ready** → the existing path runs untouched: `set_selection_and_emit` →
  `GridEvent::SelectionChanged` → `route_selection_changed` (`shell/window.rs:1466`) commits the
  pending edit (`commit_then_adopt_selection`) and adopts the selection. This is exactly today's
  commit-on-click for a non-reference-ready formula click **and** every non-formula edit — the
  feature adds nothing to that path (the `functional_spec.md §1` "not reference-ready → today's
  behaviour" guarantee).
- **Point-ready** → emit `InsertReference`, **do not** emit `SelectionChanged`, **do not** arm a
  cell drag. The grid selection is untouched (`functional_spec.md §4 Selection`).

`GridEvent::InsertReference { a1: String, replace_pending: bool }` (new variant, `grid/mod.rs`)
routes in `shell/window.rs` (the `make_grid_sink` match, ~:1573) to
`chrome.update(|c, cx| c.insert_reference(&a1, replace_pending, window, cx))`.

### 3.3 Point-drag state machine (drag → range, live preview, replace-on-grow)

New grid field `point_drag: Option<PointDrag>` (mirrors the existing `fill_drag`,
`view.rs:371,642`):

```rust
struct PointDrag {
    origin: CellRef,      // merge-anchor-resolved start cell
    last_range: CellRange // last range emitted (dedupe; also the released single-cell case)
}
```

- **Arm:** in the `mouse_down_cell` point branch above (the first `InsertReference` uses
  `replace_pending: self.pending_ref`; from here on the drag replaces its **own** in-progress ref).
- **Move** (`handle_mouse_move`, extend the `fill_drag` block at `view.rs:1599`): when
  `point_drag` is set, map the pointer to a cell via `layout::hit_test`, build the swept
  `CellRange` `origin..current` normalized, **expand for merges** (Q6, §3.4). If it differs from
  `last_range`, emit `GridEvent::InsertReference { a1: range.to_a1(), replace_pending: true }`
  (always replace during a drag — the grid's own prior insert is the pending ref; this side-steps
  the deferred `EditState` round-trip, so mid-drag correctness never depends on the pushed
  `pending_ref` catching up), update `last_range`, and `cx.notify()`. Kick
  `maybe_start_autoscroll`.
- **Preview:** in the overlay pass (§4.1), when `point_drag` is set draw a preview border over
  `last_range` — **visually distinct** from the selection border and the ref highlights
  (`functional_spec.md §2`): use a dashed/2px accent-variant border (reuse `rect_div(...).border_2()`
  with a distinct color constant, e.g. a "point-preview" accent). The editor text already tracks
  the range because each move emitted an `InsertReference`.
- **Up** (`handle_mouse_up`, beside the `fill_drag` take at `view.rs:1643`): if `point_drag` is
  set, just clear it and bump `autoscroll_epoch` — the last emitted `InsertReference` already left
  the correct text. Releasing on the origin cell means `last_range` stayed `single(origin)` → a
  single-cell ref, no degenerate range (`functional_spec.md §2 Drag details`).

### 3.4 Q6 — Merged-cell resolution (in the grid hit-test, one source of truth)

**Decision: resolve merges in the grid, against the merge list the grid already renders from — no
second source of truth.** The grid reads `cache.merges() -> &[CellRange]`
(`freecell-core/cache.rs:379`; already used at `view.rs:2119` for the header-menu merge guard).
Two pure helpers on `GridView` (or free fns over `&[CellRange]`):

- `resolve_merge_anchor(row, col) -> CellRef`: if `(row,col)` is covered by a merge in
  `cache.merges()`, return that merge's top-left (`range.start`); else `CellRef::new(row,col)`.
  Used for the click target (DPM.6 — click a covered cell → insert the anchor ref).
- `expand_range_for_merges(range) -> CellRange`: union `range` with every merge it intersects,
  iterating to a fixed point (an expansion can newly touch another merge). Used for the drag range
  (DPM.6 — a swept rect touching a merge grows to the whole span, never bisects it).

Because this reuses the same `merges()` the selection/render path uses, point-mode and selection
agree on merge geometry by construction (Q6). The in-progress merged-cells work changes only the
*population* of `merges()`, not these consumers.

### 3.5 Guards (existing gestures keep precedence)

`handle_mouse_down` already returns early when `resize_drag`/`fill_drag` own the pointer
(`view.rs:1272`) and hit-tests the fill handle + charts before cells (`view.rs:1290-1377`). The
point branch is inside `mouse_down_cell`, which runs **after** those, so the fill handle, resize
dividers, and charts keep precedence over a point click exactly as they do over a selection click
(`functional_spec.md §5`). Add `point_drag.is_some()` to the `handle_mouse_down` early-return
guard (`view.rs:1272`) and to the `handle_mouse_move`/`maybe_start_autoscroll`/`autoscroll_tick`
drag-active gates (`view.rs:1599,2476,2513`) alongside `fill_drag`, so an armed point-drag owns the
pointer + auto-scroll loop identically.

---

## 4. Range highlighting (render)

**One** render surface, fed from the one token→color map (§0): the **grid highlight overlay**
(§4.1), a rich fill + border per same-sheet reference. In-editor token coloring is deferred to the
v1.0 styled-control project (scope note above), so there is **no editor render path and no
gpui-component / vendored-widget change in v0.5**. The color map is still computed for every valid
ref (§0, §6) — it drives the grid highlights now and the future control will consume it.

### 4.1 Grid highlights (same-sheet, DPM.4/DPM.7)

In the grid overlay pass, immediately **after** the selection overlay + **before** the fill
handle/in-cell overlay (`view.rs:3124-3204`), iterate `self.ref_highlights`. For each `(range,
slot)`: clip to the visible frame exactly like the selection overlay (`view.rs:3128-3132`),
compute `span_rect(range.rows, range.cols, frame)` (`view.rs:4368`), and push a **rich highlight —
a translucent fill + a border** in the reference's color (DPM.7, no drag handles): a filled
`rect_div(x,y,w,h)` with `.bg(...)` at a low alpha over `ref_slot_rgba(slot, is_dark)`, plus
`.border_2().border_color(rgb(ref_slot_rgb(slot, is_dark)))`. `ref_slot_rgb`/`ref_slot_rgba` resolve
the palette slot to the theme-appropriate color via `freecell_core::palette::ref_color(slot)`,
picking `.light`/`.dark` from the window appearance (the grid already themes `ACCENT`/gridlines, so
it has the appearance).

- Off-screen same-sheet refs clip to nothing → no visible highlight (`functional_spec.md §3`), by
  construction (the clip drops them). The color map still holds their slot for the future control.
- Cross-sheet tokens are **absent from `ref_highlights`** (§3.1 sends only the `same_sheet`
  subset), so they never draw a grid highlight (DPM.4/Q4); the color map still assigns them a color
  for the future in-editor control.
- Three overlays can coexist — selection rectangle, point-drag preview (§3.3), ref highlights —
  each a distinct color/style, satisfying `functional_spec.md §Cross-cutting` "three visually
  distinct things on screen at once." Keep the translucent fill under the selection rectangle so
  the active selection stays legible.

### 4.2 Theme-aware color resolution (shared helper)

One `freecell-app` helper resolves a palette slot to concrete colors for the grid render:
`ref_slot_rgb(slot, is_dark) -> Rgb` (the border) and `ref_slot_rgba(slot, is_dark) -> Hsla` (the
translucent fill), both over `freecell_core::palette::ref_color(slot)`. `is_dark` comes from the
active gpui theme/appearance at the grid call site (the grid already resolves theme colors). The
same helper is what the future styled-control will reuse for editor-text coloring — one palette,
one resolution path.

### 4.3 Deferred: in-editor token coloring (v1.0 styled text-input control)

Coloring the reference *tokens inside the formula text* is **out of v0.5**. gpui-component's
`InputState` exposes no public per-range text-styling hook an external caller can drive
per-keystroke (the pinned rev's `SyntaxHighlighter`/`display_map` accessors are `pub(super)`, and
its `CodeEditor` mode takes only a built-in language name), and the owner will not maintain a second
fork. In-editor coloring is therefore deferred to the **v1.0 FreeCell styled text-input control**
([`../../../projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)),
which replaces `InputState` for FreeCell's formula editors and consumes this project's token→color
map (`RefToken.span` + `ref_colors`). **No gpui-component / vendored-widget change ships in v0.5.**

---

## 5. Q5 — Pending-ref state ownership + lifecycle

**Decision: a chrome-owned `pending_ref: Option<Range<usize>>` byte span on the shared edit state
(`EditController`), set on each point insertion, cleared on any non-point edit transition; replace
= overwrite that span, append = insert at caret.** (Spec §Q5, adopted verbatim.)

- **Set:** `insert_reference(a1, replace_pending)` (chrome; the exact analog of the shipped
  `accept_autocomplete`, `chrome/view.rs:1516-1569`):
  1. read driving `text` + `caret` (`.value()`, `.cursor()` — byte offset).
  2. splice region: if `replace_pending` **and** `self.edit.pending_ref()` is `Some(span)` →
     replace `text[span]`, `new_caret = span.start + a1.len()`; else (append) → insert at `caret`,
     `new_caret = caret + a1.len()`.
  3. `new_text = text[..start] + a1 + text[end..]`; drive `data_row.reduce(Edited{new_text})`,
     `set_driving_text_and_caret(origin, &new_text, char_col_of(new_caret), …)`
     (`view.rs:1572`), `apply_data_effects`, `mirror_other_editor` — **exactly** the accept path's
     programmatic-text mechanics (so cap-validation/mirror/undo stay identical to typing; one
     commit = one undo step, `functional_spec.md §Undo/commit`).
  4. `self.edit.set_pending_ref(Some(new_caret - a1.len() .. new_caret))` — the just-inserted span
     becomes pending.
  5. recompute the formula-feature state (§6) + `refresh_edit_grid_state` + `notify`.
- **Cleared** on any non-point edit transition, so the "replace on next point" window is exactly
  one action wide: a real keystroke fires `InputEvent::Change` → the Change handlers
  (`on_content_event`/`on_incell_event`, `view.rs:1263-1280`) clear it; a caret move fires the
  caret-move recompute (data-row intercept + `AutocompleteCaretMoved`, `view.rs:1494`) → clears it;
  a commit/cancel/focus-change clears it. Because `set_value` suppresses `Change`
  (`view.rs:1585` etc.), the programmatic insert in step 3 does **not** re-fire the Change handler
  → does **not** wrongly clear the pending span it just set (the one subtle ordering fact — noted
  so the coding agent keeps the clear in the *user-driven* Change path only).

This is the only case where a grid click acts on a "not reference-ready" caret (right after a
complete ref): `point_ready = reference_ready || pending_ref` (§3.2), and `replace_pending =
pending_ref`, so the pending ref is overwritten rather than a second ref appended
(`functional_spec.md §2`).

---

## 6. Q3 — Editor consolidation (component-doc pointer)

Per the owner decision (spec §Q3), the formula-feature stack — autocomplete, sig-hints, the
token→color map, the pending-ref/point-mode state — attaches to the **shared edit layer**
(`DataRow` reducer + the promoted `EditController`), **not** as per-editor helpers. Both editors
are the one `InputState` control, so point-mode and the shared derived state drive off the same
pending edit regardless of which editor is focused. The consolidation shape, the exact
`EditController` delta (new state + methods: `pending_ref`/`set_pending_ref`, the relocated
autocomplete/sig-hint state, `recompute_formula_edit_state`, `insert_reference`, `ref_highlights`
for the grid), the two thin host adapters (grid overlay render/event; chrome data-row
render/event — each carrying **zero formula logic**), and the migration steps are specified in
**[`components/formula_editor.md`](components/formula_editor.md)**. That doc is the DELTA to the
existing (other-project) `specs/projects/mvp-gaps/components/edit_controller.md`, which is **not**
edited here.

In-editor coloring of the reference *tokens inside the formula text*, and the eventual
`InputState` replacement it needs, are **out of v0.5** — deferred to the v1.0 FreeCell styled
text-input control
([`../../../projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)).
The consolidation here still computes the token→color map: it drives the grid highlights now, and
the future control will consume it.

The one consolidation seam this doc pins: the existing per-keystroke recompute
`recompute_autocomplete` (`chrome/view.rs:1383`), called from both Change handlers + the
caret-move path, is generalized to `recompute_formula_edit_state`, which additionally (a) calls
`freecell_engine::lex_formula_refs`, (b) `assign_ref_colors`, (c) computes `is_reference_ready`,
and (d) clears `pending_ref` (except when re-entered from `insert_reference`). Autocomplete and
grid highlighting thus recompute in lockstep off one text/caret read — no second traversal, no
separate cadence.

---

## 7. Error handling

- **Tokenization never fails.** `lex_formula_refs` returns `Vec<RefToken>` (empty on non-formula /
  no complete refs); an unparseable tail simply yields fewer tokens (the `Illegal`/`EOF` stop). No
  `Result`, no dialog. A malformed formula still highlights whatever complete refs precede the
  error and enters/leaves point-mode by the caret predicate — exactly the "as you finish typing a
  partial ref, it lights up" behaviour (`functional_spec.md §3`).
- **Point insertion is a normal edit.** `insert_reference` goes through the same `DataRow` reducer
  as typing, so the input cap applies identically (an over-cap paste-like insert is impossible —
  an A1 string is tiny; the cap check still runs for consistency). A self-reference (clicking the
  edited cell in the data-row editor, DPM.5) inserts normally and surfaces as a circular-ref at
  commit via the engine's existing handling — no special-casing.
- **No new async, no new worker command, no fork(-engine) change, no vendored-widget change** —
  the whole feature is a synchronous read (lex) + existing edit/commit plumbing. The only "engine"
  surface is one pure free function (§1.2).

---

## 8. Testing strategy

- **Unit — `freecell-core` (headless, no gpui/IronCalc):**
  - `is_reference_ready` truth table: `=|`→true; `=A1+|`→true; `=SUM(|`→true; `=A1:|`→true;
    `= |` (trailing space)→true; `=A1|`→false (after complete ref); `=SUM(A1)|`→false (after `)`);
    `=SU|`→false (mid-identifier); `=12|`→false (mid-number); `="A1+|`→false (in string); `A1`
    (no `=`)→false.
  - `assign_ref_colors`: repeats share a slot (`=A1+A1` → both slot 0); distinct refs step
    (`=A1+B2` → 0,1); >7 distinct recycle (8th → slot 0); first-appearance stability (adding a
    trailing ref never changes earlier slots); `Sheet2!A1` vs `A1` get distinct slots.
  - `ref_color`/palette: `REF_HIGHLIGHT_PALETTE` len == 7; `ref_color(i)` wraps at 7; light≠dark
    per slot (theme-awareness sanity).
- **Unit/integration — `freecell-engine`:** `lex_formula_refs` over crafted formulas: `=A1+B2` →
  2 tokens, spans point at `A1`/`B2` (byte offsets incl. the `+1` for `=`), targets `A1`/`B2`
  0-based, `same_sheet` true; `=SUM(C3:E7)` → 1 Range token, target normalized `C3:E7`;
  `=Sheet2!A1` on "Sheet1" → `same_sheet=false`, `sheet=Some("Sheet2")`; `=Sheet1!A1` on "Sheet1"
  → `same_sheet=true` (Q4 self-qualified); `=A1:` (partial) / `=Sheet2!` / `="A1"` → no tokens;
  drag-direction independence (`=B2:A1`-style input normalizes). (Bind exact `TokenType` fields
  against the fork at impl.)
- **gpui view tests — `freecell-app` (both editors):**
  - point routing: with a reference-ready data-row formula edit active, a simulated
    `mouse_down_cell` emits `GridEvent::InsertReference` and does **not** emit `SelectionChanged`
    nor change the selection; with a non-reference-ready caret it emits `SelectionChanged` (commit
    path) as today.
  - `insert_reference`: append at caret (`=|` + click C3 → `=C3`, caret after); replace pending
    (`=A1` pending + click B2 → `=B2`); append after a keystroke (`=B2+` + click C3 → `=B2+C3`);
    self-ref allowed.
  - pending-ref lifecycle: pending set after a point; cleared by a keystroke / caret move / commit;
    a programmatic insert does not clear its own pending span.
  - point-drag: a synthetic drag origin→target emits `InsertReference` with the merge-expanded
    `to_a1()` range and updates only on cell change; release on origin → single-cell ref.
  - merges: click a covered cell → anchor ref; drag touching a merge → whole-span range.
  - grid highlights: after a recompute, `ref_highlights` holds one entry per **same-sheet** valid
    ref (`(target, slot)`); a cross-sheet ref is **absent** from `ref_highlights` but still present
    in the color map (`ref_tokens`/`ref_colors`); commit/cancel pushes an empty `ref_highlights`.
  - the autocomplete→point happy path (`functional_spec.md §4`): accept `SUM(` then a point click
    → `=SUM(C3` with the caret reference-ready in between (guards the two features' shared caret
    contexts).
- **Render (pixel) — in-scope surfaces (`functional_spec.md §Pixel suite scope`):** the grid
  highlight overlay (rich fill + border) and the point-drag preview land in the grid frame →
  in-scope. New baseline cases (added in the §9 phase): `formula_ref_highlight_same_sheet` (a
  formula editing a cell with `A1`/`C3:E7` refs on the visible sheet → rich fill + border
  highlights), `formula_ref_point_preview` (a live point-drag preview vs selection vs highlight,
  three distinct styles). There is **no in-editor token coloring** in v0.5, so no editor-coloring
  baseline. Subset prefixes while iterating: `render_tests.sh test formula_ref`, `… test selection`.

---

## 9. Render validation (final, dedicated phase)

Per `CLAUDE.md` (in-scope rendering change → its own late phase, never intermixed). After all
coding phases are committed: regenerate + **eyeball** the new/affected baselines (the two new
cases above + any `selection`/`cell_*` that shift), run the **full** pixel suite once under a
`timeout` + ~10-min watchdog (`render_tests.sh test`), commit refreshed baselines with sign-off,
then dispatch the CI **`render`** gate on the branch (`gh workflow run render.yml --ref <branch>`)
and confirm green. Earlier coding phases use only the relevant **subset**.

---

## 10. Pushback / open items

The functional spec is **locked**; nothing here changes locked behaviour. Two notes:

- **Refinement (not a behaviour change): caret stays chrome-side.** The spec's Q2 sketch tentatively
  put "the caret offset" into `EditState`. The grid never needs the raw caret — it needs only the
  *derived* `reference_ready` / `pending_ref` booleans (§3.1), and the chrome does the actual splice
  against the input's own caret. Pushing the caret too would duplicate a source of truth that can
  drift under the deferred `EditState` round-trip. So `EditState` carries the two booleans (+
  `ref_highlights`), **not** the caret. Behaviour is identical; the seam is cleaner. Flagged for
  visibility; no owner decision needed.
- **Mid-drag correctness owned by the grid, not the round-trip (§3.3).** Because `EditState` is
  pushed to the grid **deferred** (`shell/window.rs:1942`), the grid must not depend on the pushed
  `pending_ref` catching up within a single fast drag. The `point_drag` machine makes every insert
  after the first a `replace_pending: true` locally, which is correct by construction (the drag's
  own prior insert is the pending ref). This is a design constraint the coding agent must honour,
  not a spec question.

---

## 11. 1-phase vs 2-phase

**Two-phase** (this `architecture.md` + one component doc). The consolidated editor — the promoted
`EditController` as the single owner of the formula-feature state, with two thin host adapters — is
a genuinely complex, cross-cutting sub-component (it touches the shared edit layer and both host
adapters) that warrants its own `components/formula_editor.md`; the rest (tokenization seam,
palette/predicate, grid routing + highlights) fits here. The engine tokenization function, the core
palette/predicate, and the grid routing are simple enough to specify inline above and don't need
their own docs.
