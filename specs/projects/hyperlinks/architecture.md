---
status: draft
---

# Architecture: Hyperlinks

Technical design for `functional_spec.md`. Every engine fact cited is established in
`research/ironcalc-links-api.md`; every FreeCell seam in
`research/freecell-integration.md`. Read both before implementing — they carry the
`file:line` map and the landmine list.

## 0. Shape of the work

The engine half already exists upstream. FreeCell's work is: **get the fork current**, then
**expose links through the worker seam**, then **render / click / author** them.

```
┌ fork ─────────────────────────────────────────────────────────────────┐
│ scosman/ironcalc  main ──fast-forward──▶ upstream 91d343c3            │
│                   freecell-fixes ──merge new main──▶ + 1 real fix     │
└───────────────────────────────────────────────────────────────────────┘
                                   │  [patch.crates-io] branch pin
┌ freecell-engine ──────────────────▼───────────────────────────────────┐
│ WorkbookDocument::{cell_link, set_cell_link, remove_cell_link}        │
│ Command::{SetCellLink, RemoveCellLink} ─▶ apply_one ─▶ AppliedKind    │
│ SheetCache.links : sparse map  ◀── build + mirror + undo-touch paths  │
└───────────────────────────────────────────────────────────────────────┘
                                   │  read lock, once per frame
┌ freecell-app ─────────────────────▼───────────────────────────────────┐
│ resolve_frame ▶ visible_links ▶ cell_element (cursor, tooltip, style) │
│ mouse_down/up ▶ GridEvent::OpenLink ▶ window sink ▶ policy ▶ open_url │
│ ChromeView link dialog (⌘K)  ·  Edit menu  ·  context menu  ·  toolbar│
└───────────────────────────────────────────────────────────────────────┘
```

## 1. Fork sync

Fork `main` fast-forwards cleanly to upstream `91d343c3` — verified by
`git merge-base --is-ancestor 60d9db19 upstream/main`. This is an **upstream sync, not a fork
fix**, so it needs no `fix/<slug>` branch. Sequence:

1. `main` ← fast-forward to upstream `91d343c3`.
2. **Delete five `fix/*` branches that are already upstream** (patch-id verified):
   `fix/trim-internal-runs`, `fix/address-empty-sheet`, `fix/xmatch-array-constant`,
   `fix/xlsx-bool-import`, `fix/e2-numfmt`. Also ignore the stale
   `claude/merged-cells-implementation-yv1pr7` (superseded by `b922df5e` on `freecell-fixes`).
3. Merge the new `main` into `freecell-fixes`. Expected: **6 files, 11 hunks**, of which 8 are
   "both sides added a module line or a match arm in the same slot — keep both"
   (`base/src/lib.rs`, `base/src/user_model/mod.rs`, `base/src/test/user_model/mod.rs`,
   `base/src/actions.rs` import line, `base/src/user_model/undo_redo.rs` ×2).
4. Resolve `base/src/user_model/common.rs` (5 hunks) properly — 1 positional (keep both new
   methods), 4 real: upstream restructured `delete_rows`/`delete_columns` to seed `diff_list`
   from `range_link_diffs(&Area{..})`, while our frozen-pane + merged-cells work added
   `old_merge_cells` / `old_frozen_rows` / `old_frozen_columns` to the same `Diff` variant. Take
   upstream's structure, keep our fields.
5. Rebase the three surviving `fix/*` branches onto the new `main`:
   `fix/paste-fill-relative-refs` (clean), `fix/batch-set-inputs` (1 positional conflict + the
   §2 fix), `fix/structural-edits-adjust-frozen-pane` (4 `common.rs` conflicts, same shape as
   step 4).

**Non-issues, confirmed:** the `Worksheet` bitcode layout change is irrelevant — FreeCell never
calls `Model::to_bytes`/`from_bytes`. The `Diff` discriminant shift is irrelevant for the same
reason. `Function::Hyperlink` collides with nothing; our fork adds zero function variants.

## 2. The one real fork fix: batch input must emit link diffs

`freecell-fixes:base/src/user_model/common.rs:565` — our batch `set_user_inputs` calls
`self.model.set_user_input(...)` directly and pushes only `Diff::SetCellValue`. Upstream changed
single `set_user_input` and CSV paste to route through `set_user_input_with_link_diffs`, which
emits the `SetCellLink` (+ `SetCellStyle`) diffs that make auto-linking undoable. Our batch path
missed that treatment because it did not exist upstream.

Left alone, F7.5 breaks: paste or fill a column of URLs, hit undo, and the values revert while
the auto-created links and their blue-underline styling persist.

This is a genuine fork fix on a capability upstream owns, so per `CLAUDE.md` it belongs on its
own branch — **`fix/batch-set-inputs-link-diffs`**, off the freshly-synced `main`, with
upstream-style tests — and gets prepared as its own single-purpose upstream PR. It must not be
folded into the merge commit or into any UI phase.

> The agent cannot open upstream PRs. It prepares a compare link
> (`.../compare/main...scosman:ironcalc:fix/batch-set-inputs-link-diffs`), title, and description
> for the owner.

## 3. Engine facade + cache

### 3.1 `WorkbookDocument` (`freecell-engine/src/document.rs`)

Three methods, following the CF/style precedent, each opening with
`crate::instrument::record_engine_call();`:

```rust
pub fn cell_link(&self, sheet: u32, cell: CellRef)
    -> Result<Option<LinkTarget>, CellQueryError>;          // effective link, near cell_content
pub(crate) fn set_cell_link(&mut self, sheet: u32, cell: CellRef,
                            target: &LinkTarget, label: Option<&str>) -> Result<(), String>;
pub(crate) fn remove_cell_link(&mut self, sheet: u32, cell: CellRef) -> Result<(), String>;
```

`set_cell_link` passes `label` straight through to upstream's
`UserModel::set_cell_link(sheet, row, column, link, label)` so F5.3's "one undo step" is the
engine's guarantee, not something FreeCell composes.

### 3.2 `LinkTarget` — a `freecell-core` type

The UI must never see `ironcalc_base::Link` (`freecell-core` has a dependency test forbidding
it). Mirror it in `freecell-core/src/style.rs` or a new `link.rs`:

```rust
pub enum LinkTarget {
    External { target: String, tooltip: Option<String> },
    Internal { location: String, tooltip: Option<String> },
}
```

Converted at the `freecell-engine` boundary, same as `CfRuleView` already is.

### 3.3 Reading the effective link

Upstream has **no per-cell call covering both worksheet and dynamic links** —
`get_cell_link` is worksheet-only; `get_links_list(sheet)` is the union. So FreeCell reads the
**union list per sheet** and caches it, exactly as upstream's own web UI does.

`SheetCache` gains a **sparse map**, not a `RenderStyle` field:

```rust
pub links: HashMap<(u32, u32), LinkTarget>,   // shaped like `merges`
```

`RenderStyle` is `Copy + Eq + Hash` and **interned**; adding a link field would give every
distinct link its own `StyleId` — 10 000 links, 10 000 resolved-style rows. The sparse map
leaves interning and the style-agreement contract untouched.

**The agreement contract needs all three paths** (`freecell-engine/src/lib.rs:20-24`) — miss one
and you get the classic bug shape here:

1. **build** — populate `links` in `build_sheet_cache` from `get_links_list`.
2. **mirror** — refresh after each applied edit. Because dynamic links are rebuilt on *every*
   evaluation, any `AppliedKind::Cell` must refresh the sheet's link map, not just the touched
   cells.
3. **undo/redo touch-set** — `SetCellLink`/`RemoveCellLink` map to `AppliedOp::Cells { sheet,
   range }` like `SetStyleAttr` does, so the re-read covers them.

### 3.4 Commands

`Command::{SetCellLink { sheet, cell, target, label }, RemoveCellLink { sheet, cell }}` in
`worker/protocol.rs`, bucketed with the **edits** arm (they are undoable), dispatched in
`apply_one`, returning `AppliedKind::StyleOnly` — a link never changes a value. `HYPERLINK()`
cells are ordinary `SetCellInput` and stay `AppliedKind::Cell`.

### 3.5 Styling decision

**Worksheet links need no FreeCell styling.** The engine applies `font.u = true` +
`Color::Theme(10, 0.0)` as real formatting, and `resolve_rgb` already resolves theme colors. This
is what makes F1.4 (explicit user color wins) fall out for free.

**Phase-1 verification gate:** confirm theme index 10 resolves to a blue in the default theme. If
it does not, `render_style_from` substitutes `LINK` when it sees `Color::Theme(10, _)` — a
three-line change, localized, no other design impact.

**Dynamic links get synthesized appearance** at scene-build time (F1.3), applied only where the
cell has no explicit font color (F1.4). This is the only render-time link styling.

## 4. Rendering

Add `LINK: u32 = 0x2563EB` to `grid/mod.rs` beside `ACCENT` — the third copy of this literal
otherwise (it already exists at `shell/about.rs:33`); consolidate rather than duplicate.

`resolve_frame` snapshots `visible_links` next to `visible_merges`, under the **same single read
lock**, into a field beside `visible_styles`. `CellPaint` gains `link: Option<LinkTarget>` so
merged-region anchors and 1×1 cells resolve identically.

Three call sites must agree, or F1.5 fails:

| Site | Why |
|---|---|
| the per-cell loop (`build_quadrant`) | ordinary cells |
| `resolve_cell_paint` | merged-region anchors |
| `SpillPlan` / `spill_element` | a long URL is *the* spill case |

At `cell_element`: `.cursor_pointer()` when the cell has a link (F2.5) — declarative, so no hover
state and no per-move repaint — and `.tooltip(...)` showing target + ScreenTip (F2.4), reusing
the same declarative mechanism the toolbar buttons already use.

**The in-cell editor must not inherit link appearance** (F1.7). `IncellFont` has no color field
today, and the mirror path already renders pending edits with `attr_style: None` — so the
requirement is to *not* add link styling there, and to add a test pinning it.

## 5. Click to open

`mouse_down_cell` records a pending link click: `(cell, link)` — but only past the point-mode
branch, and never during a formula edit (F2.2).

`handle_mouse_up` fires it when the release lands on the same cell, no drag occurred, the primary
modifier is not held, and `click_count == 1`. Anything else clears the pending click and the
interaction is an ordinary selection. Putting the decision on **mouse-up** is what makes
drag-to-select from a link cell work.

The decision itself is a **pure, gpui-free function** — inputs (pending cell, release cell,
dragged flag, modifiers, click count, degraded) → open-or-not — unit-tested directly, per house
style. `handle_mouse_up` stays thin.

Emission: `GridEvent::OpenLink { target }`, routed in `make_grid_sink` (`shell/window.rs`) like
`GridEvent::ChartSelected` — a UI-side event that never reaches the worker.

## 6. Open policy (the security seam)

A pure function in `freecell-core`, unit-tested against a table of hostile inputs:

```rust
pub enum OpenDecision { Open(String), ConfirmFile(String), Refuse { scheme: String } }
pub fn decide_open(target: &LinkTarget) -> OpenDecision;
```

- `http`, `https`, `mailto`, `ftp`, `ftps` → `Open` → `cx.open_url`.
- `file` → `ConfirmFile` → the existing `ActiveModal::Confirm` pattern, Cancel default, then
  `cx.open_url` on accept.
- everything else → `Refuse` → non-blocking message naming the scheme. Nothing launches.
- `LinkTarget::Internal` never reaches `decide_open` (F4.4).

This runs at **open** time only. Import, render, round-trip, and the ⌘K dialog all show refused
links unchanged (F3.4) — FreeCell does not rewrite a user's file. The one exception is F5.7:
authoring through the dialog rejects a refused scheme inline, because FreeCell should not create
what it will not open.

Hostile-input test table: `javascript:alert(1)`, `data:text/html,…`, `vbscript:`, `smb://host/s`,
`JaVaScRiPt:` (case), `  javascript:` (leading whitespace), `http://…` with embedded newline,
scheme-relative `//evil.com`, and a bare path.

## 7. Internal-link navigation

`decide_open` does not apply. Resolution goes through the engine (sheet name → index, `A1` →
`CellRef`, defined name → range), then a selection move + scroll-into-view, switching the active
sheet when needed. Unresolvable → non-blocking message, no navigation (F4.3).

Reference **parsing** is pure and unit-tested; only the sheet-switch and scroll are gpui-side.

## 8. The ⌘K dialog

Lives in **`ChromeView`**, not `WorkbookWindow`: it is a *form* with focus management, and
`Input`/`InputState` already live there. `window.rs`'s `dialog_card` takes only a `&str` body, so
it cannot host fields.

- **Lifecycle** modelled on `render_find_bar` / `open_find` / `close_find` — including the
  `window.on_next_frame(...)` idiom for focus-and-select-on-open (F5.4).
- **Card chrome** modelled on `render_delete_confirm`.
- **Backdrop** via the existing `backdrop(on_dismiss, cx)` helper, which uses `BlockMouse` so
  clicks and scroll do not leak to the grid — that helper exists because two real bugs were fixed
  by it.
- Two `InputState` fields (Target, Display text), pre-filled per F5.2; Enter confirms, Escape
  cancels; Remove button present and enabled only for a worksheet link (F6.1, F6.3).
- Confirm → one `Command::SetCellLink { target, label }`; empty target → `RemoveCellLink` (F5.5).

**Normalization** (F5.8) is a pure function mirroring the engine's `detect_link_target` rules —
`www.x.y` → `https://`, bare email → `mailto:`, leading `#` → internal. Unit-tested against the
engine's own accept/reject table so the dialog and auto-linking cannot drift.

## 9. Actions, menus, toolbar

- Actions `InsertLink`, `RemoveLink`, `OpenLink` in `shell/mod.rs`'s single `actions!` list.
- `KeyBinding::new(&key("k"), InsertLink, None)` — **`k` is free** and ⌘K collides with no system
  shortcut.
- Edit menu (macOS): the three items after Merge Cells, behind a separator. Placement asserted by
  a test modelled on `edit_menu_has_merge_cells_after_find`.
- **There is no Linux menu bar**, so the context menu is load-bearing, not a convenience:
  Open link / Insert link… / Edit link / Remove link go in `cell_menu_items` — a **pure**
  function returning `Vec<Option<(String, bool, GridEvent)>>`, so enablement is unit-testable
  without pixels.
- Action row: one `link` button, `.ghost().small()`, `.disabled(self.degraded)`,
  `flex_shrink_0`, modelled on the Find trigger.
- Handlers on the `WorkbookWindow` root so they grey out on the Welcome window (F5.9); ⌘K
  commits an in-flight edit first (F5.10), mirroring `toggle_style`.

**Icon:** `link.svg` is not vendored. Whether the pinned gpui-component Lucide bundle ships it
could not be verified offline. Check with the established idiom —
`assert!(matches!(AppAssets.load("icons/link.svg"), Ok(Some(_))))` — and only if it errs, vendor
Lucide `link` in `stroke="currentColor"` form, register it in `FREECELL_ICONS`, and add the
resolution test. The existing tintability test then enforces the form.

## 10. Render tests

New `link_*` group in `cases.rs`, plus `Scene::link(row, col, target)` next to `underline`,
routed through the real `Command::SetCellLink` (not `Inject`) so the scene stays honest.

Cases: `link_plain`, `link_dynamic` (`HYPERLINK()`), `link_spill` (long URL — pins F1.5),
`link_in_merged_region`, `link_over_fill`, `link_with_explicit_font_color` (pins F1.4
precedence), `link_removed` (pins F1.6 — formatting persists).

Per repo policy: iterate with `render_tests.sh test link_`; the **full** suite, baseline
regeneration + eyeballing, and the CI `render` dispatch happen once, in the dedicated final
phase.

## 11. Test plan beyond render

| Layer | Coverage |
|---|---|
| pure (`freecell-core`) | `decide_open` hostile table; click-decision; target normalization; internal-ref parsing |
| `freecell-engine` unit | `cell_link` / `set_cell_link` / `remove_cell_link`; cache build + mirror + undo-touch agreement; dynamic-link refresh on re-evaluate |
| `freecell-engine` integration (`tests/roundtrip.rs`) | xlsx import → render; FreeCell-created link → save → reopen; tooltip + internal-link preservation |
| fork (upstream-style) | batch `set_user_inputs` auto-link undo (§2); row/column move carrying both a merged region and a link; clear-range vs merged-cells interaction |
| `freecell-app` gpui | click opens / drag does not / ⌘-click does not / point-mode does not; ⌘K pre-fill, confirm, cancel, remove; menu placement; context-menu enablement; degraded-mode gating; in-cell editor does not inherit link styling |

## 12. Docs to update on landing

- **Delete** the `GAPS.md` **Hyperlinks** row, `HYPERLINK()` clause included (repo policy: a
  fixed gap is removed, not annotated). Leave the distinct external-workbook-links row alone.
- Correct `projects/xlsx-preservation.md`'s "hyperlinks dropped" bullet.
- Add `GAPS.md` rows for the genuine remaining holes: applying a link across a range, and
  `file:` links if F3.2 ships as a block rather than a confirm.
