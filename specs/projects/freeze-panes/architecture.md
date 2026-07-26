---
status: complete
---

# Architecture: Freeze Panes

Render + interaction + persistence for **one freeze boundary per axis** (`functional_spec.md`).
The engine already models frozen panes end-to-end and round-trips them through xlsx `<pane>` —
so this is a **FreeCell-side integration**, no IronCalc **fork** change (one contingency, §7 /
Q4). The work is thin at the seams (a worker command + read-model field) and concentrated in the
**custom grid's viewport**, which today has a single content rect + single scroll pair per sheet
and knows nothing about frozen regions.

**This doc** gives the whole-system design: the data model, the engine/read-model plumbing, the
header-menu entry, structural-edit tracking, error handling, and the test/render plan. **One
component design carries the high-risk, field-level detail:**

- [`components/viewport_split.md`](components/viewport_split.md) — the four-quadrant render + the
  frozen-extent-aware scroll/clamp/hit-test/reveal/auto-scroll geometry (the real complexity).

**1-phase vs 2-phase:** two-phase. The engine/read-model/header-menu wiring is small and fits
here; the viewport split is large enough (four sub-frames, re-based clamp math, every geometry
function becoming frozen-aware, divider drawing) to exceed the single-doc bar and get its own
component doc.

**Crate map (unchanged):** `freecell-core` (pure — `layout.rs` grid geometry, `cache::SheetCache`
read model, `Axis`), `freecell-engine` (IronCalc-fork adapter + worker: `document.rs`,
`worker/{protocol,run}.rs`, `cache.rs`), `freecell-app` (GPUI shell: `grid/`, `shell/`). Only
`freecell-engine` may touch IronCalc types (enforced by `freecell-core`'s `dependency_rule.rs`).

**Standing conventions:** crate-scoped build/test per phase; `cargo fmt --all --check` every
phase; render **subset** while iterating grid phases; **one** full render suite + CI `render` gate
in a dedicated late phase; commit + push regularly.

---

## 1. Data model — the frozen pair `(M, K)`

A sheet's freeze state is the pair `(M, K)`:

- **`M` = frozen-rows count** — leading rows `0..M` (0-based) pinned to the top; `M = 0` = no row
  freeze.
- **`K` = frozen-columns count** — leading columns `0..K` pinned to the left; `K = 0` = no column
  freeze.

`(M, K)` is **per worksheet**, independent per axis, and is the *whole* persisted state (the
scroll offset is view state, not part of it). It lives in three places, each a thin mirror of the
next:

1. **Source of truth — IronCalc.** `Worksheet.frozen_rows` / `Worksheet.frozen_columns`
   (`types.rs:115/116`), set via the undoable `UserModel::set_frozen_rows_count` /
   `set_frozen_columns_count` (`common.rs:1143/1157`), imported/exported through xlsx `<pane>`.
   Confirmed present + round-tripping by the round-3 API audit
   (`experiments/round-3/B-api-audit/findings.md`).
2. **Read model — `freecell_core::cache::SheetCache`.** Two new `u32` fields `frozen_rows`,
   `frozen_cols` (§2), published to the grid alongside axes/styles.
3. **Grid — read per frame** off the resident `SheetCache`; never stored on `GridView` (§3).

Note `(M, K)` is a **track-index** count, independent of hidden state: the frozen band's *pixel*
extent is `axis.offset_of(M)` (which is 0 for hidden tracks), so a frozen band containing hidden
rows shows fewer than `M` rows — exactly `functional_spec.md §4` "hidden rows inside a frozen
band". No special-casing is needed; the axis prefix-sum already yields this.

---

## 2. Engine wiring + read model (Q2, Q5) — mirror `SetRowsHidden`

Freeze is a geometry-only, undoable, whole-sheet-rebuild mutation — **structurally identical to
Hide/Unhide** (`gaps_closing_7_15 §4`), which is the pattern to mirror end to end.

### 2.1 Worker command (Q5)

`worker/protocol.rs` — new variant beside `SetRowsHidden` (`protocol.rs:307`):

```rust
/// Set the frozen-rows and/or frozen-columns count on `sheet` (`freeze-panes`). Each `Some`
/// field rides the engine's undoable setter; the UI sends exactly ONE axis per action (the
/// other is `None`) so it is one undo step. Geometry-only (no evaluation — freeze never changes
/// values); the sheet cache is rebuilt (it re-reads `frozen_rows`/`frozen_columns`).
SetFrozen { sheet: SheetId, rows: Option<u32>, cols: Option<u32> },
```

`Option` per axis maps 1:1 to the fork's two setters and keeps a single command; because the
header menu changes only the axis it was invoked on, exactly one field is `Some` per command →
one IronCalc call → **one undo step** (`functional_spec.md §5.4`).

### 2.2 Document wrappers

`document.rs` — beside `set_rows_hidden` (`document.rs:815`), two thin wrappers (IronCalc types
stay inside the crate):

```rust
pub(crate) fn set_frozen_rows(&mut self, sheet_idx: u32, count: u32) -> Result<(), String>;
pub(crate) fn set_frozen_columns(&mut self, sheet_idx: u32, count: u32) -> Result<(), String>;
```

each recording `instrument::record_engine_call()` then calling
`self.model.set_frozen_{rows,columns}_count(sheet_idx, count)`. No getter is needed on
`WorkbookDocument`: the cache reads `ws.frozen_rows`/`ws.frozen_columns` directly at build time
(§2.4), exactly as it reads `r.hidden`.

### 2.3 Worker dispatch + classification

`worker/run.rs`:

- **Apply** (beside `SetRowsHidden`, `run.rs:3481`): call `doc.set_frozen_rows` /
  `set_frozen_columns` for whichever field is `Some`, return `AppliedKind::GeometryOnly`.
- **Edit-bucket classification** (`run.rs:591`): add `Command::SetFrozen { .. }` to the
  geometry-only edits arm (so it rides the coalesced publish + undo/redo machinery).
- **Refresh classification `op_of`** (`run.rs:3746`): map `Command::SetFrozen { sheet, .. }` to
  `AppliedOp::Rebuild { sheet }`. The whole-sheet rebuild re-reads the frozen counts from the
  worksheet, so **undo/redo needs no special handling** — popping IronCalc's `set_frozen_*` diff
  restores the worksheet count, and the rebuild re-publishes it to the cache (identical to how
  `SetRowsHidden` undo restores the hidden set).

### 2.4 Read into the cache

`freecell-core/cache.rs` — add `frozen_rows: u32`, `frozen_cols: u32` to `SheetCache`
(`cache.rs:48`) and `SheetCacheBuilder` (`:517`), with accessors `frozen_rows()` /
`frozen_cols()` (beside `hidden_rows()`, `:493`) and builder setters
`set_frozen_rows(u32)` / `set_frozen_cols(u32)` (+ fluent `frozen_rows(u32)` /
`frozen_cols(u32)` for fixtures). Unlike hidden, frozen does **not** feed `axis_from` — it has
no geometry effect, so `build()` just copies the two counts through.

`freecell-engine/cache.rs::build_sheet_cache` (`cache.rs:323`) — after the row/col loops, read
the two worksheet fields into the builder:

```rust
builder.set_frozen_rows(ws.frozen_rows as u32);
builder.set_frozen_cols(ws.frozen_columns as u32);
```

This is the "opening a file with `<pane>` shows the bands immediately" path
(`functional_spec.md §4`): the counts land in the cache on first build with no user action.

### 2.5 GridEvent + window routing

- `grid/mod.rs` `GridEvent` (beside `HideRows`, `mod.rs:208`):
  `SetFrozen { rows: Option<u32>, cols: Option<u32> }` (grid-relative; the window supplies the
  active `SheetId`).
- `shell/window.rs` (beside the `HideRows` arm, `window.rs:1795`): forward as
  `Command::SetFrozen { sheet: shared.active_sheet.get(), rows, cols }`.

**No IronCalc type crosses the engine boundary** (`functional_spec.md §7`): the mutation flows as
`SetFrozen`, the counts flow back as two `u32`s on `SheetCache`.

---

## 3. Grid render + geometry (the real work) → `components/viewport_split.md`

The grid reads `(M, K)` off the resident `SheetCache` **per frame** (like axes/styles), so freeze
adds **no `GridView` field** and no cross-frame state — a freeze/unfreeze/undo is a cache
republish the next `render` picks up. Given `(M, K)`, `resolve_frame`
(`grid/view.rs:997`) splits the single body viewport into up to **four quadrants** (corner /
top band / left band / body, `functional_spec.md §2`), and every scroll/clamp/hit-test/reveal
computation becomes frozen-extent-aware. Because this is the render hot path and the most-tested
grid code, the full field-level design — quadrant sub-frames, the re-based clamp math (Q1/Q3),
divider drawing, and each geometry function's frozen-aware form — is in
[`components/viewport_split.md`](components/viewport_split.md).

**Invariants that doc must hold (repeated here as the contract):**

- **No per-frame cost proportional to sheet size** (`functional_spec.md §7`). Each quadrant
  renders only its **visible** tracks; the corner/top/left bands are the *leading* `M`/`K` tracks,
  which are few. `(M, K)` never drives a loop over the sheet.
- **`M = K = 0` degenerates to today's single viewport, byte-for-byte** — the quadrant math with
  zero band extents reduces to the current body-only path, so an un-frozen sheet's pixels and hot
  path are unchanged (a hard requirement for the baseline suite and perf gate).
- **The frozen bands never scroll**; the body's stored scroll is re-based so `scroll = 0` shows
  the first non-frozen track at the divider, and clamps against `viewport − headers − band`.

---

## 4. Header-menu entry (`functional_spec.md §1`)

The Freeze/Unfreeze item joins the existing header context menu (which today carries Insert /
Delete / Hide / Unhide), reusing its machinery:

- **Menu state.** `HeaderMenu` (`grid/view.rs:127`) gains one field: `frozen: u32` — the current
  count on the menu's axis (`M` for a row header, `K` for a column header), read from the cache in
  `handle_right_mouse_down` (`view.rs:2071`) alongside the hidden sets/dims already gathered under
  the one lock (`view.rs:2083`), and stored at `HeaderMenu` construction (`:2203`).
- **Boundary track.** The menu already normalizes the clicked track against the header selection
  into a run `(u32, u32)` (`functional_spec.md §1.3`). The **boundary index `b` = `run.1`** (the
  last/bottom-most track), so the implied count is `b + 1`.
- **Item mapping** (in the pure `header_menu_items`, `view.rs:3734`, one new tuple after
  Hide/Unhide): with `b = menu.run.1`,
  - if `menu.frozen == b + 1` → label **"Unfreeze {row|column}s"**, event sets that axis to `0`;
  - else → label **"Freeze {rows|columns}"**, event sets that axis to `b + 1` (freezing the
    boundary track **and everything above/left** — moving the boundary if a different freeze
    existed).

  A **row** header's item drives `SetFrozen { rows: Some(_), cols: None }`; a **column** header's
  drives `{ rows: None, cols: Some(_) }`. The item is **always enabled** (freeze hides nothing;
  no "would leave nothing visible" guard, `functional_spec.md §1.4`) — a new
  `GridEvent::SetFrozen` per §2.5. `header_menu_items` stays pure → the label/event mapping is
  unit-testable exactly like the Hide/Unhide items.
- **No corner / cell-menu item** (`functional_spec.md §1.2`): freeze is header-only. `Corner` and
  `Cell` right-clicks are untouched (`view.rs:2150-2173`).

---

## 5. Structural edits — boundary tracking (Q4)

`functional_spec.md §4`: inserting rows above/within the frozen band **grows** `M`, deleting
within it **shrinks** `M`, edits below leave `M` unchanged (Excel behavior; symmetric for
columns). Because `InsertRows`/`DeleteRows`/`InsertColumns`/`DeleteColumns` already map to
`AppliedOp::Rebuild` (`run.rs:3748`) and the rebuild re-reads `frozen_rows`, **the only question
is whether IronCalc's structural ops already adjust the pane count.**

**Design decision (safe default, verified by a checkpoint):**

1. **Implementation checkpoint (first thing in the structural-edits phase):** in the fork
   container, probe `insert_rows` / `delete_rows` / `insert_columns` / `delete_columns` against a
   sheet with `frozen_rows = 3` — insert 1 row above (expect `M → 4`), delete 1 within (expect
   `M → 2`), insert below (expect `M` unchanged). IronCalc targets Excel parity, so the **expected
   outcome is that it already adjusts** — in which case FreeCell writes **zero** code: the
   post-op rebuild surfaces the adjusted count, and undo (which pops the *insert's own* diff)
   restores it in one step.
2. **If the probe shows IronCalc does NOT adjust:** the fix belongs **upstream in the fork**, not
   in FreeCell (CLAUDE.md: "fix upstream, don't hack FreeCell"). Adjusting the pane inside
   IronCalc's `insert_rows`/`delete_rows` is (a) the engine-correct Excel behavior and (b) the
   *only* way to keep it **one undo step** — it becomes part of that op's single undo diff. A
   FreeCell-side second `set_frozen_*` call would be a **separate** IronCalc diff (a second
   `ops_seen`/undo entry, `run.rs` history is 1:1 with the IronCalc stack), producing a
   two-step undo that violates `functional_spec.md §5.4`. So a negative probe spawns one
   focused `fix/structural-edits-adjust-frozen-pane` branch (one upstream PR), and FreeCell adds
   no compensating logic. This is the single genuinely-contingent item (see §7 / the return
   summary).

Either branch of the checkpoint leaves FreeCell's grid/read-model code unchanged — the boundary
tracks the data purely through the existing rebuild-re-read path.

---

## 6. Error handling & degraded worker (`functional_spec.md §5.5`)

- **Render is read-side.** Frozen bands are drawn from `SheetCache` counts; a degraded/read-only
  worker still publishes them, so an already-frozen sheet keeps rendering its bands + divider —
  nothing to disable there.
- **Mutation blocked when degraded.** `SetFrozen` rides the identical rejection path as every
  other data mutation: when the worker is degraded it is refused (`EditRejectedReason::Degraded`,
  `window.rs:744` — no state change, no undo step, silently ignored), matching the
  Insert/Delete/Hide items. The grid does not today gray header-menu mutation items on degrade
  (degraded state lives on the window, not the grid); Freeze follows that established behavior, so
  the substance of §5.5 ("existing frozen state renders; changing it is blocked") holds via the
  worker reject. (If the grid later gains a degraded flag to gray items, Freeze participates for
  free — it is just another mutation tuple.)
- **Degenerate band ≥ viewport / freeze-everything** (`functional_spec.md §5.1/§5.2`): no error,
  no block — handled entirely by the render-side clip + shrink-to-zero body in
  `components/viewport_split.md §Degenerate cases`.
- **Bad counts:** `SetFrozen` counts are UI-derived from real track runs (`0..=count`), so they
  are always in range; a defensive clamp to `[0, axis.count()]` sits in the document wrapper.

---

## 7. Testing strategy

Layered, cheapest-first (mirrors `gaps_closing_7_15`):

- **Pure unit — `freecell-core`** (`layout.rs`, no gpui): the frozen-extent-aware clamp/reveal/
  hit-test math (the crux; enumerated in `components/viewport_split.md §Test plan`) — re-based
  body scroll clamps against `total − band` and `viewport − headers − band`; `scroll = 0` shows
  the first non-frozen track; reveal is a no-op on a frozen axis and never tucks a body target
  under the band; hit-test routes each of corner/top/left/body/headers to the right cell/track;
  `M = K = 0` reproduces the pre-freeze results exactly. Plus `header_menu_items` Freeze/Unfreeze
  label + event mapping (pure).
- **Engine — `freecell-engine`:** `SetFrozen` toggles `frozen_rows`/`frozen_columns` and the cache
  reflects it; **one undo step** restores the prior count; open a fixture carrying `<pane>` →
  cache counts populated; save→reopen preserves them (round-trip). The Q4 checkpoint probe (in
  the fork repo if a fix branch is needed).
- **gpui view tests — `freecell-app`:** the header menu shows Freeze on a fresh row header and
  Unfreeze on the current boundary track; the item emits the right `GridEvent::SetFrozen`; a
  frozen sheet resolves four quadrant sub-frames with the expected ranges; a body scroll leaves
  the bands at offset 0 and clamps correctly; scroll-to-reveal into the body doesn't hide behind
  the band. (No IronCalc needed — driven off a `SheetCacheBuilder` with `frozen_rows(m)`.)
- **Render pixel suite (Q6) — dedicated LATE phase, never per coding phase** (repo render
  policy). Frozen bands move grid pixels (in scope: cell/row/column/sheet + the bands + divider).
  While coding, iterate with the **subset** (`render_tests.sh test <prefix>`); defer the full
  suite + CI `render` gate to one late phase. New baseline cases (added to
  `render-tests/src/cases.rs`; the `Scene` builder gains `.frozen_rows(m)` / `.frozen_cols(k)`
  mirroring `.hide_row`, `scene.rs:204`):
  - `freeze_top_row` (`M=1`), `freeze_rows_band` (`M=3`),
  - `freeze_first_col` (`K=1`), `freeze_cols_band` (`K=3`),
  - `freeze_four_quadrant` (`M=2, K=2`) — corner + both bands + body,
  - `freeze_scrolled_body` (`M=2, K=2` + `.reveal(...)` deep) — bands pinned while the body is
    scrolled, proving the re-based offsets + divider position,
  - `freeze_divider` — the divider line(s) against ordinary gridlines.

  The late phase runs the **full** suite under a `timeout` + ~10-min watchdog, eyeballs +
  commits the new baselines, and dispatches the CI `render` gate to green.

---

## 8. Summary of resolved architecture questions

- **Q1 viewport structure** — one committed axis pair per sheet; up to four quadrant sub-frames
  sharing the axes, each drawn into its own clipped rect; O(visible-per-quadrant), no sheet-size
  cost. Detailed in `components/viewport_split.md`.
- **Q2 where `(M, K)` lives** — two `u32`s on `SheetCache`, published like axes/styles; no
  IronCalc type crosses the boundary.
- **Q3 clamp math** — body scroll re-based (0 = first non-frozen track at the divider), clamped
  against `body_area = viewport − headers − band` and total `= axis.total() − band`; every
  geometry fn frozen-aware (`components/viewport_split.md §3`).
- **Q4 structural edits** — checkpoint-verified; expected native IronCalc adjustment (zero
  FreeCell code); a negative probe spawns one upstream fork fix, never a FreeCell hack.
- **Q5 command shape** — `Command::SetFrozen { sheet, rows: Option<u32>, cols: Option<u32> }`
  wrapping the fork's undoable setters, mirroring `SetRowsHidden`/`SetColumnsHidden`.
- **Q6 render baselines** — new frozen/divider/scrolled cases in a dedicated late
  render-validation phase; subsets while iterating.

No functional requirement forces disproportionate complexity — the locked spec maps cleanly onto
the existing seams. **No pushback.** The one open contingency is Q4's probe outcome (§5).

---

## Owner decisions (2026-07-18)

- **Q4 (structural edits vs. the frozen boundary) — RESOLVED.** Probe IronCalc's native
  behavior first (Phase 5). If it does **not** adjust the frozen count on insert/delete of
  rows/cols, the fix goes in the **fork** — a `fix/structural-edits-adjust-frozen-pane`
  branch off `main` → integrated on `freecell-fixes` → a prepared upstream PR in our usual
  one-fix-one-PR format (owner opens upstream). **No FreeCell-side compensating call** (a
  second `set_frozen_*` would be a second undo diff and break the one-undo-step guarantee).
  Per the standing "fix upstream, don't hack FreeCell" policy.
