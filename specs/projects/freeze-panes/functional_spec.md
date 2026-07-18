---
status: draft
---

# Functional Spec: Freeze Panes

Pin leading row/column bands so header rows and label columns stay visible while the rest
of the sheet scrolls underneath — Excel/Sheets' "Freeze Panes." A v0.5 table-stakes gap:
frozen header rows are near-universal in real sheets, and a file that already contains them
renders today as an ordinary scrolling sheet (the frozen state is silently dropped on the
render side even though the engine round-trips it).

This document fixes **what the user sees and does**. Mechanism (how the single grid
viewport splits into quadrants, where the frozen counts live in the read model, the exact
scroll-clamp math, the worker command shape) is deferred to `architecture.md`; where a
technical seam is named below it is only to ground the behavior, not to prescribe the
"how".

## Scope in one line

Render + interaction + persistence for **one freeze boundary per axis** (Excel's model):
frozen rows and/or frozen columns, set from the header context menu, pinned while the body
scrolls, persisted to `.xlsx`, and undoable. Split panes (movable, freeze-independent
splits) are **v2.0** and out of scope.

## Decisions locked at spec time (owner, 2026-07-18)

- **Entry point: header context menu ONLY.** Right-click a row-number or column-letter
  header → a single **Freeze** / **Unfreeze** item (§1). **No** View-menu presets ("Freeze
  Top Row", "Freeze First Column", "Freeze Panes"), **no** keyboard shortcut, **no**
  toolbar button in v0.5.
- **Header-driven, not "freeze at the active cell."** The boundary is derived from the
  clicked header track, not from where the active cell sits (Excel's alternative variant is
  out of scope).
- **One freeze boundary per axis** — a single leading band of rows and a single leading
  band of columns. No multiple frozen bands / no interior freeze. Freezing rows and
  freezing columns are **independent** (you can have neither, either, or both → up to four
  quadrants, §2).
- **Persistence:** freeze state persists to `.xlsx` on save (IronCalc round-trips `<pane>`)
  and is **undoable** (rides IronCalc's undoable `set_frozen_rows_count` /
  `set_frozen_columns_count`).

---

## 1. The Freeze / Unfreeze interaction

### 1.1 Terminology

- **Frozen-rows count `M`** — the number of leading rows pinned to the top: rows `1..M`
  (0-based indices `0..M−1`). `M = 0` means no row freeze.
- **Frozen-columns count `K`** — the number of leading columns pinned to the left:
  columns `1..K` (0-based `0..K−1`). `K = 0` means no column freeze.
- Each axis is independent; a sheet's state is the pair `(M, K)`.

### 1.2 Where the item lives

The existing header context menu (right-click a row-number header or a column-letter
header — the menu that today carries Insert / Delete / Hide / Unhide) gains **one** new
item at the bottom: **Freeze** or **Unfreeze** (the same slot flips its label per §1.4).

- A **row** header's menu item controls only the **frozen-rows** count `M`.
- A **column** header's menu item controls only the **frozen-columns** count `K`.
- There is no freeze item on the top-left corner (select-all) affordance, and none in the
  cell-area right-click menu — freeze is a header-only action.

### 1.3 What "the clicked track" is (single vs. multi-track selection)

The header menu already normalizes the clicked track against the current header selection:
right-clicking a header **outside** the current selection first collapses the selection to
that single track; right-clicking **inside** a multi-track header selection keeps the whole
selected run. Freeze uses the resulting run's **last (bottom-most row / right-most column)
track** as the boundary track:

- **Single header right-clicked (the overwhelmingly common case):** the boundary track is
  simply the clicked track.
- **Multi-track header run selected, right-clicked inside it:** the boundary is the run's
  last track — i.e. "freeze through the end of the selection." (Assumption default, §
  Assumptions.)

Let `b` be the 0-based index of the boundary track.

### 1.4 Label, and what each click does

For a **row** header with boundary index `b` (so the implied count is `b+1`):

- **If `M == b+1`** (the clicked boundary already equals the current freeze) → the item
  reads **Unfreeze**. Clicking it sets `M = 0` (clears the row freeze; leaves `K`
  untouched).
- **Otherwise** → the item reads **Freeze**. Clicking it sets `M = b+1` — freezing rows
  `1..b+1`, i.e. the boundary track **and everything above it**. This applies whether there
  was no freeze before (`M = 0`) or a freeze at a **different** boundary: freezing at a new
  track simply **moves** the boundary (no separate "move" affordance is needed).

Columns are exactly symmetric with `K`, "left of," and the column-letter header.

Consequences of this single-item model (owner-chosen, faithful to Excel-lite header
freezing):

- **To clear a freeze you right-click the current boundary track** (whose item reads
  Unfreeze). Right-clicking any other track reads Freeze and re-freezes at that track.
- Clearing is **per axis**: Unfreeze on a row header clears only `M`; to remove both bands
  you Unfreeze once on the row axis and once on the column axis.
- The item is always **enabled** (unlike Hide, freeze has no "would leave nothing visible"
  guard — see §5.1 for the degenerate large-band case, which is tolerated at render time,
  not blocked here).

### 1.5 Examples

- Fresh sheet, right-click **row 1** header → "Freeze" → `M = 1`: row 1 pins to the top,
  nothing above it (the "freeze top row" outcome), body scrolls beneath.
- `M = 3`, right-click **row 3** → "Unfreeze" → `M = 0`.
- `M = 3`, right-click **row 6** → "Freeze" → `M = 6` (boundary moves down).
- Right-click **column B** header → "Freeze" → `K = 2`: columns A–B pin to the left.
- With `M = 2` already, right-click **column C** → "Freeze" → `K = 3`: now both bands →
  four quadrants (§2).

---

## 2. The four-quadrant layout

With `M` frozen rows and `K` frozen columns, the sheet body (the region **inside** the
row-number and column-letter headers) is partitioned into up to four regions. The headers
themselves are unchanged in role — they still sit above (column letters) and to the left
(row numbers) of everything, and split to match the bands (frozen row numbers / column
letters pin; body ones scroll).

```
            │  frozen cols (K)   │   scrolling cols
────────────┼────────────────────┼─────────────────────
 frozen     │   CORNER           │   TOP BAND
 rows (M)   │  (pinned both)     │  (pinned vertically,
            │                    │   scrolls horizontally)
────────────┼────────────────────┼─────────────────────
 scrolling  │   LEFT BAND        │   BODY
 rows       │ (pinned horizontally, (scrolls both)
            │  scrolls vertically) │
```

- **Corner** = frozen rows ∩ frozen cols — pinned in both directions; never moves.
- **Top band** = frozen rows × scrolling cols — pinned vertically; scrolls **horizontally**
  in lockstep with the body.
- **Left band** = scrolling rows × frozen cols — pinned horizontally; scrolls **vertically**
  in lockstep with the body.
- **Body** = scrolling rows × scrolling cols — scrolls both directions.

Degenerate cases of the same layout:

- `M > 0, K = 0` → two regions: a pinned top band (frozen rows across the full width) over a
  scrolling body. No corner, no left band.
- `M = 0, K > 0` → a pinned left band beside a scrolling body.
- `M = 0, K = 0` → today's single scrolling viewport (unchanged).

Cells in the frozen bands are **real, fully interactive cells** (selectable, editable,
right-clickable) — they are pinned copies of rows `1..M` / columns `1..K`, not a static
snapshot.

### 2.1 The freeze divider

A visible **freeze divider line** marks each active boundary:

- A horizontal divider along the **bottom edge of the frozen-rows band** (present iff
  `M > 0`), spanning the full body width (across the corner + top band).
- A vertical divider along the **right edge of the frozen-columns band** (present iff
  `K > 0`), spanning the full body height (across the corner + left band).

The divider is visually distinct from an ordinary gridline (heavier / darker, matching the
platform's freeze-line convention) so the pinned region reads as pinned. It is drawn only
for an axis that actually has a freeze; unfreezing removes it.

---

## 3. Rendering & scroll behavior

### 3.1 What pins, what scrolls

- Scrolling the sheet **down/up** moves the body and the **left band** together (their rows
  stay aligned); the **top band** and **corner** do not move vertically.
- Scrolling **right/left** moves the body and the **top band** together (their columns stay
  aligned); the **left band** and **corner** do not move horizontally.
- The **corner** is fixed.
- The frozen bands are always shown at their tracks' natural (offset-0) positions — the
  frozen rows always start immediately below the column-letter header; the frozen columns
  always start immediately right of the row-number header.

### 3.2 Scroll offset & clamping

The per-sheet scroll offset now describes the **scrolling body's** position only (the
frozen bands never carry a scroll offset). Concretely:

- Body vertical scroll `0` shows the **first non-frozen row** (row `M+1`) at the top of the
  body region, immediately under the freeze divider. It cannot show a frozen row in the body
  (frozen rows are only ever in the pinned band).
- Body scroll is clamped so the **last** row/column can reach the bottom/right of the
  **body** region — the reachable extent is computed against the body area (viewport minus
  the frozen band and headers), not the full viewport. You can never scroll the body such
  that content hides **behind** a frozen band or such that you scroll past the end.
- Freezing/unfreezing re-clamps the existing scroll to the new valid range (see §5.3 for
  when this visibly moves the body).

### 3.3 Active cell, selection, and reveal

- **Active cell / selection anywhere.** The active cell may be in any region. If it is in a
  frozen band or the corner, it is **always visible** (pinned) and never triggers a scroll.
  If it is in the body it participates in scrolling normally.
- **Selections spanning regions.** A selection range that straddles the freeze boundary
  (e.g. a range that starts in the frozen rows and continues into the body, or a
  whole-column selection that crosses the row freeze) renders its highlight **continuously
  across the quadrants** — each quadrant paints its portion in that quadrant's coordinate
  space, and they visually join at the divider. There is no gap or double-draw at the
  boundary.
- **Scroll-to-reveal must respect frozen bands.** Revealing a target cell (arrow-key
  navigation, type-to-edit, click-select, "go to") must:
  - **Do nothing on an axis where the target is frozen** — a target in the frozen rows
    and/or columns is already pinned and visible; reveal is a no-op on that axis.
  - **Reveal a body target into the body sub-area**, never behind a frozen band — the reveal
    math aligns the target inside the region below/right of the divider (viewport minus the
    frozen band), so a just-revealed cell is never tucked under the pinned band.

### 3.4 Headers, gridlines, row-height gutter

- The row-number gutter and column-letter header split with the bands: frozen row numbers /
  column letters pin; body ones scroll — labels always match the row/column beneath them in
  every region.
- The dynamic row-number-gutter width (which grows with the deepest visible row number)
  continues to track the deepest **visible** row across all regions.
- Ordinary gridlines, fills, borders, fonts, alignment, in-cell editor, and the loading
  overlay render identically inside every region — a frozen band is the same cell rendering,
  just pinned.

---

## 4. Interaction with existing features

- **Header resize drags.** Resizing a **frozen** row's height or **frozen** column's width
  works exactly as for any track; the frozen band grows/shrinks and the divider moves with
  it. Resizing a body track is unchanged. Resize is per-track and never crosses regions.
  Row-height autofit (double-click a row divider) and column autofit behave the same in a
  frozen band as in the body.
- **Selection drags across the boundary.** A drag-select that crosses the freeze divider
  extends the selection continuously across it (from body into a frozen band or vice
  versa). Edge auto-scroll fires **only** near the scrolling body's live edges — dragging
  **into** a pinned band does **not** auto-scroll (there is nothing to scroll there); a drag
  that leaves the band back into the body near the bottom/right edge resumes auto-scroll.
  The same applies to the **fill-handle** drag.
- **Scrollbars.** Frozen bands are **excluded** from the scrollable extent — the overlay
  scrollbar represents the **scrolling region only** (its thumb size/position are computed
  over "total tracks minus the frozen band" against the body area, matching Excel). When a
  freeze pins enough that the remaining rows/cols fit the body, that axis's scrollbar
  disappears (nothing to scroll), even if the full sheet is huge.
- **Insert / delete rows & columns relative to the boundary.** The freeze boundary tracks
  structural edits the way the data does (Excel behavior): inserting rows entirely above or
  within the frozen band **grows** the band (the frozen count follows), deleting rows within
  it **shrinks** it, and edits entirely below the band leave `M` unchanged (symmetric for
  columns). This rides the engine's structural-edit handling; whether IronCalc adjusts the
  frozen count automatically is an architecture question (§ Architecture questions), and if
  it does not, the count is adjusted alongside the insert/delete so the user-visible result
  matches Excel.
- **Hidden rows / columns inside a frozen band.** The freeze count is by **track index**,
  independent of hidden state. A hidden track inside the band contributes **zero** pixels,
  so freezing `M` rows where some are hidden shows fewer than `M` rows in the pinned band;
  unhiding a track inside the band grows the band. Hiding/unhiding never changes `M`/`K`.
- **Per-sheet freeze.** `(M, K)` is **per worksheet** (IronCalc models it per sheet).
  Switching the active sheet shows that sheet's own freeze and its own scroll position; one
  sheet frozen and another not is normal.
- **Opening a file that already has `<pane>`.** A freshly opened `.xlsx` that carries frozen
  panes shows the frozen bands **immediately on open**, at the stored counts, with the
  divider drawn — no user action required. (This is the concrete "table-stakes" fix: such a
  file renders correctly instead of as a plain scrolling sheet.)
- **Charts / floating objects.** Charts are anchored in body cell-space and continue to
  scroll with the body; freeze does **not** pin charts (a chart whose anchor sits within a
  frozen band is a known minor limitation, not addressed in v0.5). See Out of scope.

---

## 5. Edge cases & errors

### 5.1 Frozen band larger than the viewport (tiny window / large band)

A window can be resized smaller after a freeze, or a user can freeze many/tall rows, so the
frozen band's pixel extent may exceed the available body space. This is **tolerated, not
blocked**:

- The frozen band renders from the top/left and **clips** at the viewport edge; the band
  itself does **not** scroll internally.
- The body (scrolling) region shrinks toward zero; when it is empty there is simply nothing
  to scroll on that axis. No error dialog, no crash.
- This is a degenerate display state the user escapes by enlarging the window or unfreezing;
  it is not a state we prevent at freeze time (the window size is not known to be stable).

### 5.2 Freezing at/near the last row or column ("freeze everything")

Freezing is permitted at any track, including the last row/column. Freezing at or near the
end simply produces a very large band and a tiny (possibly empty) scrolling region — the §
5.1 tolerance applies. There is no "freeze would leave nothing visible" block (that guard
exists for Hide, where it hides data; freeze hides nothing — every cell is still on screen,
just pinned).

### 5.3 Freeze/unfreeze while scrolled

- **Freezing while the body is scrolled down/right:** the newly frozen tracks (`1..M` /
  `1..K`) are pinned at the top/left and the body re-clamps to a valid offset. The frozen
  band always shows the leading tracks regardless of where the body was scrolled — freezing
  does not "capture" whatever rows happened to be near the top.
- **Unfreezing:** the pinned band is reabsorbed into a single scrolling viewport; scroll
  re-clamps to the (now larger) valid range. Content does not jump beyond what the clamp
  requires.
- **Freeze at row 1 / column A:** `M = 1` pins exactly row 1 with nothing above it (the
  minimal, most common freeze); `K = 1` pins column A. `M = 0` / `K = 0` is the no-freeze
  state.

### 5.4 Undo / redo

Freeze and Unfreeze (and moving the boundary) are each **one undo step** (rides IronCalc's
undoable frozen-count setter). Undo restores the prior `(M, K)` on that axis and redraws the
bands/divider accordingly; redo re-applies. Scroll position is view state (not itself on the
undo stack) but is re-clamped so it stays valid after an undo/redo changes the bands.

### 5.5 Degraded / read-only worker

If the engine worker is degraded/read-only (the same condition that disables Hide/other
mutations today), the **Freeze/Unfreeze** item is disabled, consistent with the other
data-mutating header-menu items. Existing frozen state still **renders** (render is a
read-side concern); only changing it is blocked.

---

## 6. Out of scope (v0.5)

- **Split panes** — movable, freeze-independent split bars that divide the window into
  independently scrollable panes (Excel's View ▸ Split). **v2.0**; it will reuse this
  project's viewport-split machinery.
- **"Freeze at the active cell" variant** — deriving the boundary from the active cell's
  position rather than the clicked header. We are header-driven only.
- **View-menu presets** — "Freeze Top Row", "Freeze First Column", "Freeze Panes" menu-bar
  items. No menu-bar or toolbar entry point in v0.5; header context menu only.
- **Keyboard shortcut** for freeze/unfreeze.
- **Multiple frozen bands / interior freeze** on an axis (Excel doesn't do this either).
- **Pinning charts / floating objects** with the freeze (charts continue to scroll with the
  body).
- Any change to which rows/columns are frozen based on a **cell** selection (freeze is set
  only from **header** tracks).

---

## 7. Constraints

- **Engine-boundary discipline.** No IronCalc type crosses the `freecell-engine` boundary;
  the frozen counts flow to the UI through the existing publication / `SheetCache` read
  model, and the freeze mutation flows through the worker command/protocol seam (a
  `SetFrozen`-style command wrapping the engine's undoable setters). No IronCalc **fork**
  change is required — the API exists and `<pane>` round-trips (confirmed by the round-3 API
  audit).
- **Grid is the real work.** This is a grid/cell/sheet rendering change (the custom GPU
  viewport, its single-scroll geometry, hit-testing, and reveal math) — the most
  performance-sensitive, most-tested part of the app. Freeze must not add per-frame work
  proportional to sheet size; each quadrant still renders only its **visible** tracks.
- **Render-test coverage (in scope).** Frozen bands move grid pixels, so per the project's
  render policy this is a pixel-suite change: iterate with the render **subset**
  (`render_tests.sh test <prefix>`), and add new baselines for a frozen-row sheet, a
  frozen-col sheet, both (four quadrants), the divider, and representative scroll-offset
  states. A **dedicated late render-validation phase** runs the full suite (with a watchdog),
  eyeballs/commits the new baselines, and dispatches the CI `render` gate to green — not
  intermingled per coding phase.
- **`.xlsx` fidelity.** Frozen state saves and reopens losslessly via `<pane>` (both
  FreeCell→FreeCell and FreeCell↔Excel), matching the one-boundary-per-axis model exactly.
