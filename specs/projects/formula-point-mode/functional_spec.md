---
status: complete
---

# Functional Spec: Formula Point-Mode + Range Highlighting

The second half of formula-entry UX. The first half — function autocomplete + signature
hints — shipped in `gaps_closing_7_15` Phase 1 and runs off the same editor plumbing this
project extends. This document fixes **what the user sees**; mechanism (where tokenization
runs, how a grid click is routed into an active editor, where colors are painted) is
deferred to `architecture.md` and flagged under "Architecture questions" below.

While a user is editing a **formula** (the edit text starts with `=`), two behaviors turn
on:

1. **Range highlighting (grid)** — every distinct cell reference already in the in-progress
   formula is drawn on the grid as a **rich colored highlight — a colored fill + border**
   around the referenced range, each distinct reference a distinct color (Excel's
   colored-refs). "What does this formula point at" becomes legible at a glance.
2. **Point-mode** — clicking, or click-dragging, a cell/range on the grid inserts that
   reference into the formula at the caret, instead of the user typing `A1:B5` by hand. A
   grid click when the caret is **not** in a reference-ready position behaves as today
   (commits the edit and moves the selection).

Point-mode works from **both** editors: the in-cell editor (the overlay the grid draws
over the active cell) and the data-row editor (the formula bar). The two editors already
share one pending edit through the chrome's edit reducer, so the grid highlighting and
point-mode are both driven off the same shared pending text/caret regardless of which
editor has focus. Range highlighting is drawn **on the grid only** — there is no coloring
of the reference tokens inside the editor text in v0.5 (that is deferred; see §6).

## Cross-cutting conventions

- **Formula-only.** Everything here is gated on the existing formula predicate: the edit
  text starts with `=` (the same predicate autocomplete uses, `freecell-core` /
  `input_cap.rs`). A non-formula edit (plain text/number) never highlights and never
  enters point-mode; a grid click during a non-formula edit behaves exactly as today.
- **View-only, never mutating the selection.** Neither behavior changes the actual grid
  `SelectionModel` (`freecell-core/src/selection.rs`). Point-mode changes **only the
  formula text**; the active cell / selected range the grid tracks is untouched. The
  reference highlights (fill + border) are a **separate grid overlay** from the selection
  rectangle and the point-mode drag preview — three visually distinct things can be on
  screen at once.
- **Reference source of truth = the engine lexer.** Which byte spans in the edit text are
  reference tokens (and what cell/range each resolves to) comes from tokenizing the
  in-progress formula with the engine's public `Lexer`/`Parser` (in the IronCalc fork, not
  this repo). The autocomplete feature deliberately used a lexical *heuristic* in
  `freecell-core` because that crate may not depend on IronCalc
  (`tests/dependency_rule.rs`); this feature needs the **real** tokenizer, which is why
  tokenization placement is an architecture question, not a `freecell-core` detail.
- **Undo / commit.** Point-mode edits the pending text like typing does; committing the
  edit is **one** undo step (the existing per-edit commit), exactly as if the reference had
  been typed. Point-mode itself adds no separate undo entries.
- **Pixel suite scope.** Range highlighting draws a rich colored **fill + border on the
  grid**, and the point-mode drag shows a **preview rectangle** on the grid — both move grid
  pixels, so this feature is **in-scope** for the pixel render suite (verify with a
  `render_tests.sh test` subset while iterating; full suite + CI `render` gate once, in a
  dedicated late phase). There is **no in-editor token coloring** in v0.5, so no chrome
  element-tree coloring to validate. Plan render validation as its own late phase per the
  repo convention.

---

## 1. Reference-ready caret position (the predicate that decides a grid click)

The single most important concept. At any moment during a formula edit, the caret is
either **reference-ready** or not. This governs what a grid click does.

### Reference-ready

The caret is reference-ready when the character **immediately before it** (skipping any
run of spaces back to a non-space) is one of:

- the leading `=` (caret right after `=`, i.e. an empty formula `=|`);
- a binary/unary **operator**: `+`, `-`, `*`, `/`, `^`, `&`, `=`, `<`, `>`, `%`;
- an argument **comma** `,`;
- an **open paren** `(`;
- a **range colon** `:` (the caret sits mid-range, e.g. `=A1:|`, so pointing completes the
  range's second endpoint).

This is the same operator/opener/separator set the shipped autocomplete already treats as
"function position" (`functions.rs::is_function_position_prev`), which is not a
coincidence — a reference and a function name are legal in exactly the same syntactic slots.

### Not reference-ready

The caret is **not** reference-ready when it sits:

- **mid-identifier** — inside or at the end of a name being typed (`=SU|`, `=SUM|`), i.e.
  the char before it is a letter/digit/`.`/`_`. (This is where autocomplete is active
  instead; see §4.)
- **mid-number** — the char before it is a digit that is part of a numeric literal
  (`=12|`, `=3.1|`).
- **right after a complete reference** — the char before it closes a reference token
  (`=A1|`, `=A1:B2|`). A completed ref is a value; the next legal thing is an operator, not
  another ref. (Excel's "pending ref" exception to this is §2's replace behavior — a
  *just-pointed* ref is a special transient state, not ordinary typed text.)
- **after a close paren** `)` (`=SUM(A1)|`) — a value.
- **inside or right after a string literal** (`="foo|`, `="foo"|`).

### Behavior of a grid click in each state

- **Reference-ready → point-mode.** The click (or drag) inserts a reference at the caret
  (§2). The edit stays open; the grid selection does not change.
- **Not reference-ready → today's behavior.** The click **commits** the pending edit and
  moves the selection to the clicked cell (the existing commit-on-click path: the grid
  emits `SelectionChanged`, the window commits the pending data-row edit, then adopts the
  new selection — `shell/window.rs`). Highlights clear because the edit ended.

The predicate is evaluated against the **driving** editor's live caret (whichever of the
two editors is focused), so pointing works identically from the formula bar and the in-cell
editor.

---

## 2. Point-mode insertion (click and click-drag)

### What gets inserted

- **Single click** on cell `C3` inserts the text `C3` at the caret.
- **Click-drag** from `C3` to `E7` inserts the range `C3:E7` (normalized to
  top-left`:`bottom-right regardless of drag direction).
- **Reference style: relative A1**, no `$` (`C3`, `C3:E7`) — DPM.1. An absolute/mixed
  toggle (Excel's F4) is out of scope (§6).
- **Same-sheet only** for v0.5 — no sheet-qualifier is ever inserted. Clicking a different
  sheet's tab mid-formula does **not** insert a cross-sheet reference (§6, and it must be
  logged as a v1.0 GAP).

The caret is placed at the **end** of the inserted reference so the user can immediately
type an operator to continue.

### Replace the pending ref (Excel's pointing model) — DPM.2

Pointing puts the formula into a transient **pending-ref** state:

1. `=|` — user types `=`. Caret reference-ready.
2. User clicks `A1` → text becomes `=A1`, caret after `A1`. `A1` is the **pending ref**
   (highlighted like any ref, §3).
3. User clicks `B2` **without typing anything in between** → the pending `A1` is
   **replaced**, text becomes `=B2` (not `=A1B2`). This is re-aiming; it can repeat
   indefinitely, and a drag replaces just the same.
4. User types any character (e.g. `+`) → the pending ref is **fixed** into the text
   (`=B2+`), the pending state ends. The caret is now reference-ready again (after `+`).
5. User clicks `C3` → a **new** reference is appended: `=B2+C3`.

So: while a pointed ref is pending and nothing has been typed since, a grid click
**replaces** it; the first keystroke after pointing **commits it into the text**, and the
next click then **appends** a fresh ref. This is the only case in which a grid click acts
on a caret that the §1 predicate would call "not reference-ready" (the caret sits right
after a complete ref) — the pending state overrides the predicate for exactly one
follow-up point action.

### Ending point-mode

- **Type an operator / comma / paren** — fixes the pending ref and continues the formula
  (caret becomes reference-ready again after the operator).
- **Enter** — commits the whole edit and moves the selection **down** (existing commit +
  move). Highlights clear.
- **Tab** — commits and moves **right** (existing). Highlights clear.
- **Escape** — cancels the whole edit, reverting the cell to its prior content (existing
  Escape). Highlights clear.

### Drag details

- **Auto-scroll.** A drag toward a viewport edge auto-scrolls (reusing the existing
  cell-drag auto-scroll loop, `grid/view.rs`), so a range can be swept beyond the visible
  area; the inserted range reflects the final released rectangle.
- A live point-mode drag shows a **preview rectangle** of the range being swept (visually
  distinct from the selection rectangle and from the reference highlights), and the editor
  text updates to the in-progress range as the drag grows (so `C3`, then `C3:D5`, then
  `C3:E7`), mirroring the pending-ref replace behavior on each frame.
- Releasing on the **same cell** the drag started on inserts a single-cell ref (`C3`), not
  a degenerate range.

---

## 3. Range highlighting (colored refs, grid-only)

### What is highlighted

For every **valid reference token** the engine lexer finds in the in-progress formula, if
the reference resolves to a cell/range **on the currently visible sheet**, a rich colored
**highlight — a fill + border (no drag handles — DPM.7)** — is drawn around that cell/range
on the grid, in the reference's assigned color (DPM.3).

The highlight is drawn on the **grid only**; the reference token inside the editor text is
**not** colored in v0.5 (in-editor coloring is deferred to the future styled text-input
control — §6). The token→color map is still computed for every valid reference (§Color
assignment below) — it drives the grid highlights now and will feed the future in-editor
styling control.

### Color assignment — DPM.3

- A color is assigned **per distinct reference**, keyed by the reference's resolved target
  (so two occurrences of `A1` in one formula share one color; `A1` and `B2` get different
  colors).
- The palette is a fixed cycle of **N distinct colors** (recommend **7**), theme-aware
  (legible in light and dark, as a grid fill + border; the same slots will color editor
  text in the future control). Beyond N distinct references the palette **recycles** (the
  8th distinct ref reuses color 1, etc.) — Excel does the same; collisions past 7 refs are
  acceptable.
- Assignment order is by first appearance left-to-right in the formula text, so colors are
  stable as the user types more of the formula (adding a later ref never recolors earlier
  ones; only removing/merging references can shift the cycle).

### What is NOT highlighted

- **Invalid / partial references** — a bare `=A`, an unterminated range `=A1:`, an
  incomplete sheet qualifier `=Sheet2!`, garbage, or a reference inside a string literal —
  get **no** highlight. Only tokens the lexer classifies as complete references highlight.
  As the user finishes typing a partial ref, it lights up.
- **Off-screen references** — a valid same-sheet ref scrolled out of view has **no visible
  highlight** (there is nothing on screen to highlight). This is fine and expected. (The
  color map still assigns it a color for the future control.)
- **Cross-sheet references** (`Sheet2!A1`) — **no grid highlight** on the current grid,
  because the other sheet is not shown (default per the owner decision: highlight only
  same-sheet refs on the visible grid). The color map still assigns them a color (consumed
  by the future in-editor styling control), but v0.5 draws nothing on the grid for them.
  (This split is an architecture question — DPM.4 / Q4.)

### Lifecycle

Highlights exist **only while a formula edit is open**. They appear as soon as the edit
starts with `=` and a valid ref is present, update live per keystroke and per point action,
and are **fully removed** the instant the edit commits (Enter/Tab/click-away) or cancels
(Escape) — a committed cell shows its computed value with no reference highlights.

---

## 4. Interaction with adjacent features

### Function autocomplete + signature hints (shipped, `gaps_closing_7_15 §1`)

Both are active during the same edit but keyed off **different caret contexts**, so they
never both apply to one caret position:

- The **autocomplete list** shows when the caret is at the end of an identifier in function
  position (`=SU|`). That caret is **mid-identifier**, hence **not** reference-ready — a
  grid click there commits (§1) and, per the existing rule that "clicking away dismisses the
  list," also closes the popup.
- **Point-mode** applies when the caret is reference-ready (after an operator/paren/comma).
  That caret is never inside an identifier, so the autocomplete list is not open.
- **The happy path that ties them together:** accepting a completion inserts `NAME(` and
  leaves the caret right after `(` — which **is** reference-ready. So `=SUM(` then a grid
  click on `C3:E7` yields `=SUM(C3:E7`, no typing. Call this out; it is the payoff of the
  two features sharing plumbing.
- The **signature hint** (passive, shows the arg template while the caret is inside a
  recognized call) coexists with highlighting; it is unaffected. The grid highlights and the
  hint can be on screen together.

### Merged cells (work in progress) — DPM.6

- A **click on any covered cell of a merge** inserts the **merge's anchor** reference (the
  top-left cell of the merged area), matching how selection resolves a merge.
- A **drag whose swept rectangle touches a merge** expands the inserted range to include
  the **whole merged span**, so the range never bisects a merge.
- Because merged-cell support is itself in progress, the exact resolution seam is an
  architecture question (Q6); the *behavior* above is fixed.

### Selection

Point-mode and highlighting **never** change the grid selection. The selected cell/range
stays put throughout an edit; only the formula text and the grid overlay highlights change. (This
is the delicate part called out in the overview: routing grid mouse input into the active
editor **without** firing the usual selection change / commit-on-click.)

---

## 5. Edge cases

- **Click-drag that auto-scrolls** — the range keeps growing as the view scrolls; the
  inserted text tracks the live range and settles on the released rectangle (§2).
- **Clicking the cell being edited.**
  - *Data-row editor:* the editing cell is fully visible on the grid; clicking it inserts a
    **self-reference** (`=A1` while editing `A1`). This is not blocked in point-mode; the
    engine's existing circular-reference handling reports it at commit/evaluation, exactly
    as if it had been typed (DPM.5).
  - *In-cell editor:* the editor overlay is drawn **over** its own cell and `.occlude()`s
    it, so a click there lands on the **editor** and is ordinary text-caret placement, not a
    point action. (Pointing at the editing cell from the in-cell editor is therefore
    effectively unreachable, which is acceptable.)
- **Clicking a text-caret position within the same editor** (e.g. clicking earlier in the
  formula-bar text) — ordinary caret movement; the pending-ref state ends (a caret move is
  "something happened since the last point"), and the reference-ready predicate is then
  re-evaluated at the new caret.
- **Clicking outside the grid** — the data row, action row, sheet tabs, or other chrome:
  **not a grid click**, so no point insertion. Clicking into the data-row field places the
  text caret (normal). Clicking a **sheet tab** follows existing tab-click behavior (no
  cross-sheet insertion — that is the v1.0 GAP, §6).
- **Escape / Enter / Tab** — as in §2: Escape cancels+reverts, Enter commits+down,
  Tab commits+right; all three clear highlights. A committed edit leaves **no** highlight
  overlays.
- **A drag that starts on the fill handle or a resize divider** is *not* a point-mode drag
  — those existing gestures (fill handle, `gaps_closing_7_15 §3`; track resize) own the
  pointer and take precedence, exactly as they guard against a selection drag today
  (`grid/view.rs` checks `fill_drag`/`resize_drag` before arming a cell drag).
- **Recoloring on edit** — deleting a reference from the formula frees its color; because
  assignment is by first-appearance order, removing an earlier ref may shift later refs'
  colors by one slot. This is cosmetic and acceptable (Excel does the same).

---

## 6. Out of scope (v0.5) — deferred to v1.0

- **In-editor token coloring / rich in-editor text formatting** — coloring the reference
  *tokens inside the formula text* (and richer in-editor formatting: backgrounds,
  Excel/Numbers/Sheets-like styling). **Deferred to a separate future project — the
  FreeCell styled text-input control** (v1.0 GAP,
  [`projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)) —
  because gpui-component's `InputState` exposes no external per-range styling and the owner
  will not fork it. The token→color map this project computes is exactly what that future
  control will consume. **There is no gpui-component / vendored-widget change in v0.5.**
- **Arrow-key point-mode** (arrow around the grid to build a reference without the mouse).
  **Explicit reason:** while a formula editor is focused, the arrow keys are **text-cursor
  movement inside the editor**; overloading them to also move a point-mode marker on the
  grid would conflict with in-editor caret navigation and make it ambiguous whether an arrow
  edits text or builds a reference. Point-mode is therefore **click / click-drag only** for
  v0.5. (v1.0 resolves the conflict, e.g. a mode gate.)
- **Cross-sheet point-mode *insertion*** — clicking another sheet's tab mid-formula, then a
  cell on that sheet, to insert `Sheet2!A1`. **Out for v0.5, and a GAPS.md entry should be
  added** for it (v1.0 tier). The color map still assigns a color to already-typed
  cross-sheet references (consumed by the future in-editor styling control), but v0.5 draws
  **no grid highlight** for them and inserts no cross-sheet reference — only same-sheet
  *insertion* and same-sheet *grid highlights* ship.
- **Editing an existing reference by dragging its highlight's handles** (Excel's blue
  resize box on a highlighted range) — v1.0.
- **Absolute/mixed reference toggle** (F4 cycling `A1` → `$A$1` → `A$1` → `$A1`) while
  pointing — v1.0; v0.5 always inserts relative.
- **Multi-area references** (⌘-click union while pointing, inserting `A1,C3`) — v1.0,
  consistent with multi-area selection being out of scope elsewhere.

---

## 7. Decisions to confirm (recommended defaults)

- **DPM.1 — Inserted reference style.** *Recommended:* **relative A1** (`C3`, `C3:E7`), no
  `$`. Matches Excel's default when pointing. Alt: absolute (rejected — surprising, and F4
  cycling is v1.0).
- **DPM.2 — Replace pending ref vs append.** *Recommended:* **Excel pending-ref** — a fresh
  point action with nothing typed since the last one **replaces** the pending ref; the first
  keystroke fixes it into the text; the next point then **appends**. Alt: always append
  (rejected — makes re-aiming impossible without manual deletion).
- **DPM.3 — Color assignment + palette.** *Recommended:* one color **per distinct resolved
  reference** (repeats share), a fixed palette of **7** theme-aware colors that **recycle**,
  assigned by first-appearance order. Alt: per-token (recolors repeats differently — noisier)
  or a larger palette (diminishing returns; 7 matches Excel's feel).
- **DPM.4 — Cross-sheet highlighting.** *Recommended:* draw a **grid highlight only for
  references resolving to the visible sheet**; the color map still assigns a color to every
  valid reference (incl. cross-sheet) for the future in-editor styling control. Honors
  "highlight only same-sheet on the visible grid." (Also an architecture seam — Q4.)
- **DPM.5 — Clicking the edited cell.** *Recommended:* **insert the self-reference**; let the
  engine's circular-ref handling surface it at commit (data-row editor). The in-cell overlay
  occludes its own cell, so that path is naturally text-editing.
- **DPM.6 — Merged cell resolution.** *Recommended:* a click on a covered cell inserts the
  **merge anchor** ref; a drag touching a merge expands to the **whole merged span**.
- **DPM.7 — Grid highlight style.** *Recommended:* a **rich colored fill + border** (no drag
  handles) for v0.5. Handles are v1.0 (they enable drag-to-edit, which is out).
- **DPM.8 — Highlight/point gating.** *Recommended:* both behaviors are **strictly gated on a
  formula edit** (leading `=`); a non-formula edit never highlights or enters point-mode.

---

## Architecture questions for the next round

These are the deep technical decisions `architecture.md` must resolve. Each carries a
recommended default.

1. **Tokenization source + when it runs.** The engine `Lexer`/`Parser` lives in IronCalc,
   and `freecell-core` may not depend on IronCalc (`tests/dependency_rule.rs`) — the
   autocomplete heuristic lived in core precisely to avoid that. Where does real formula
   tokenization run (a `freecell-engine`/worker call, or a synchronous app-layer call), and
   at what cadence (it must feed live highlighting on **every** keystroke and every point
   action)? *Recommended:* a synchronous lexer call at the engine/app boundary (not core) on
   each edit transition — formula strings are short, so a per-keystroke lex is cheap —
   returning reference tokens with their **byte spans** and **resolved targets** (cell/range +
   sheet).

2. **Routing a grid click into the active editor without breaking commit-on-click.** Today a
   grid click emits `SelectionChanged`, which the window uses to commit the pending edit and
   adopt the selection. Point-mode needs the grid, at mouse-down, to know (a) a formula edit
   is active and (b) whether the caret is reference-ready (or a ref is pending), then branch
   to an **insert** instead of a commit — for edits driven by **either** editor.
   *Recommended:* extend `ChromeGridRequest::EditState` (which today carries `in_cell`, the
   mirror, cap, quick-edit, autocomplete, sig-hint — but **no caret and no signal that a
   data-row edit is active**) with the caret offset + a `reference_ready` / `pending_ref`
   signal, and add a `GridEvent::InsertReference { a1, replace_pending }` the chrome consumes
   against the shared reducer. The grid consults the pushed signal in `mouse_down_cell` to
   choose point vs. commit.

3. **Where highlight colors are computed, and the editor consolidation. — RESOLVED (owner,
   2026-07-18).** **Decision:** compute the token→color map **once** on the shared edit
   state; the grid paints the same-sheet **highlights** (rich fill + border) from it.
   In-editor coloring of the reference *tokens inside the formula text* is **out of v0.5** —
   it needs external per-range text styling that gpui-component's `InputState` does not
   expose and the owner will not fork; that work is **deferred to the v1.0 FreeCell styled
   text-input control**
   ([`projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md)),
   which will consume this project's color map. **There is no gpui-component / vendored-widget
   change in v0.5.** The consolidation still happens: fold the formula-feature stack
   (autocomplete, sig-hints, color map, pending-ref/point-mode state) onto the existing
   shared layer — the `DataRow` reducer + `EditController` promoted into the single
   owner/factory for the formula-editor pair — rather than per-editor helpers (point-mode
   needs the shared state, and it sets up the future control). Detail belongs in
   `architecture.md` / `components/formula_editor.md`.

4. **Cross-sheet highlight handling (grid-only).** With the DPM.4 default, a cross-sheet
   reference draws **no grid highlight**, yet the color map still assigns it a color (for the
   future in-editor styling control) — resolve how the color map handles references that
   never draw a grid highlight, and whether a cross-sheet ref *to the current sheet*
   (`Sheet1!A1` on Sheet1) should highlight. *Recommended:* highlight any reference whose
   resolved **target sheet == the visible sheet** (qualified or not); the color map still
   colors all valid refs regardless (the grid draws only the same-sheet subset).

5. **Pending-ref state ownership + lifecycle.** Where the "just-pointed, replace-on-next-point"
   span lives, and how it is invalidated. *Recommended:* a chrome-owned pending-ref byte span
   on the edit state, set on each point insertion, **cleared on any non-point edit
   transition** (keystroke, caret move, focus change); replace = overwrite that span, append =
   insert at caret when no span is pending.

6. **Merged-cell resolution timing.** Merged-cell support is in progress; where does
   click→anchor and drag→span expansion resolve? *Recommended:* resolve in the **grid
   hit-test** against the merge map the grid already renders from, so point-mode reuses the
   same merge geometry the selection/rendering path uses (no second source of truth).
