# Research: FreeCell integration points for hyperlinks

Point-in-time research (2026-08-07). Paths relative to `/home/user/freecell/`. Line numbers
are as-of this date — verify before editing.

## 1. Engine facade (`freecell-engine`)

The worker thread owns the `UserModel` on a 64 MiB stack and is the only code that touches an
IronCalc type (`freecell-engine/src/lib.rs:1-25`, `pub(crate) use ironcalc_base::UserModel;` at
`:50`). The UI reads published snapshots + the resident style cache.

| Layer | File | Role |
|---|---|---|
| `WorkbookDocument` | `freecell-engine/src/document.rs:186` | typed wrapper over `UserModel`; every IronCalc call goes through here |
| `Worker` command loop | `freecell-engine/src/worker/run.rs` | buckets commands, applies, evaluates, publishes, mirrors the cache, records undo touch-sets |
| `DocumentClient` | `freecell-engine/src/worker/client.rs:72` | the `Send` handle the window keeps |

### Read models (both engine-free, in `freecell-core`)

`PublishedCell` — `freecell-core/src/publication.rs:47`:

```rust
pub struct PublishedCell {
    pub row: u32, pub col: u32,
    pub display_text: String,
    pub kind: CellKind,
    pub text_color: Option<Rgb>,
}
```

`RenderStyle` — `freecell-core/src/style.rs:38-86`. Already carries `bold`, `italic`,
**`underline` (`:42`)**, `strikethrough`, `wrap`, `fill`, `font_color`, `h_align`, `v_align`,
plus side-table indices. Interned by `StyleId` in `SheetCache`
(`freecell-core/src/cache.rs:48-117`, interning at `:74-85`). Built by
`freecell-engine/src/cache.rs:323` (`build_sheet_cache`) via `render_style_from` (`:215`, maps
`style.font.u → underline`).

### Mutation flow

UI → `DocumentClient::send(Command::…)` (`client.rs:120`) → bucketed in `run.rs:486-614`
(`Command::SetCellInput` at `:577`) → `apply_edit_batch` (`run.rs:793`) validates → per-edit
`apply_one` (`run.rs:3462`) returns an `AppliedKind` (`run.rs:68-99`:
`Cell`/`StyleOnly`/`SheetOp`/`GeometryOnly`/`Structure`/`CondFmt`/`NoOp`) → each op records an
`AppliedOp` (`run.rs:173`) → `Touch` (`run.rs:105-120`) in a worker-side history aligned 1:1
with IronCalc's undo stack. Publish-then-bump via `Shared` (`client.rs:29-54`:
`publication: ArcSwap<Publication>`, `generation`, `committed_ops`, `caches: RwLock<SheetCaches>`,
`cond_fmt: RwLock<…>`). Dirty = `committed_ops > last_saved_ops` (`shell/window.rs:145`).

### Where link methods go — follow the conditional-formatting precedent

- **Document methods** beside `set_fill`/`update_style_path`/`set_borders`
  (`document.rs:1056-1113`). Every method opens with `crate::instrument::record_engine_call();`
  (`document.rs:387, 396, 418, 445, 458, 477, 487, 1047, 1062, 1083, 1105`). Direct UI reads use
  the typed `CellQueryError` (`document.rs:127`), e.g. beside `cell_content` (`document.rs:444`).
- **Commands** — `worker/protocol.rs:200` `enum Command`. Bucket with the **edits** arm
  (`run.rs:577-610`) since they're undoable; dispatch in `apply_one` (`run.rs:3462`); map to
  `AppliedOp::Cells` like `SetStyleAttr`/`SetStylePath` do (`run.rs:3843-3847`).
- **Read surface** — either fold into `SheetCache` (grid reads it per frame, lock-briefly, like
  `render_style`) or publish a per-sheet map into `Shared` like `cond_fmt` (`client.rs:48-53`,
  read via `DocumentClient::cond_fmt_rules` `client.rs:161`).
- **Events** — `WorkerEvent` (`protocol.rs:670`); `StyleCacheUpdated { sheet }` (`:717`) comes
  free if links live in the `SheetCache`.
- **Tests** — inline `#[cfg(test)] mod tests` per module; integration in
  `freecell-engine/tests/` (xlsx link round-trip → `tests/roundtrip.rs`).

## 2. Cell render pipeline

1. **`resolve_frame`** (`grid/view.rs:1250`) takes the caches read lock **once**, snapshots
   visible styles into `self.visible_styles: HashMap<(u32,u32), RenderStyle>` (`:1367-1374`)
   plus font-family (`:1377`), border (`:1384`), merge (`:1400`) side tables, then **drops the
   lock** (`:1411`).
2. **`build_quadrant`** per-cell loop (`:3436-3599`): resolves `style` (`:3450`), `fill`
   (`:3452`), then `(text, text_color, kind, attr_style)` from mirror → `cell_index` → default
   (`:3474-3494`), emits `cell_element` (spill branch `:3560`, normal `:3584`).
3. **`CellPaint`** (`:731-739`), built by **`resolve_cell_paint`** (`:1031-1067`) — today used
   only for merged-region boxes (`:3625`).
4. **`cell_element`** (`:5353-5471`) is where style is applied: alignment `:5402-5409`, bold
   `:5411`, italic `:5414`, **underline `:5417-5419` (`el = el.underline()`)**, strikethrough
   `:5422`; `text_color` applied at `:5384`.
5. **`SpillPlan`** (`:714-726`) + `spill_element` (`:5500`) **duplicate** the styling
   (`:5540-5543`).

**Underline already exists end-to-end** — `RenderStyle::underline` (`core/style.rs:42`),
engine-mapped (`engine/cache.rs:219`), painted (`view.rs:5417`, `:5540`), mirrored into the
in-cell editor (`view.rs:6045`), and baselined (`baselines/cell_underline.png` + 5 combination
baselines). Nothing new needed for the primitive.

**Theme colors already resolve** — `resolve_rgb(color, theme)` at `freecell-engine/src/cache.rs:181`
handles theme-indexed colors via `Color::to_rgb`, so the engine's `Color::Theme(10, 0.0)` link
color resolves without new code. *Verify what index 10 actually renders as.*

### Colors

`grid/mod.rs:34-63` — `CELL_TEXT = 0x1F1F1F` (`:38`), `ACCENT = 0x2563EB` (`:51`).
`shell/about.rs:33` defines `LINK: u32 = 0x2563EB` with a note that neither the design system
nor gpui-component's theme exposes a link token. **Reuse `0x2563EB`**; add a `LINK` constant to
`grid/mod.rs` next to `ACCENT` rather than a third copy.

### Style-cache interaction

`RenderStyle` is `Copy + Eq + Hash` and **interned** (`core/cache.rs:74-85`). Adding a field
changes interning identity — on a sheet with 10 000 distinct links that's 10 000 distinct
`StyleId`s. **Prefer a separate sparse link map** on `SheetCache` (shaped like `merges: Vec<CellRange>`,
`cache.rs:116`), which leaves interning and the agreement contract untouched; snapshot it in
`resolve_frame` next to `visible_merges` (`view.rs:1400`) into a `visible_links` field beside
`visible_styles` (`view.rs:317`).

The load-bearing invariant (`engine/lib.rs:20-24`): the mirrored cache must provably agree with
a fresh engine re-read. Every new cached field needs **all three** paths — build
(`engine/cache.rs:323`), mirror (per-edit re-read via `document.rs:486` `cell_own_style` /
`:503` `resolved_cell_style`), and the undo/redo touch-set re-read (`run.rs:105-120`). Getting
only one or two is the classic bug shape here.

## 3. Mouse hit-testing & cursor

Handlers registered at `grid/view.rs:6347-6372`: scroll `:6347`, left down → `handle_mouse_down`
(`:1705`), right down → `handle_right_mouse_down` (`:2547`), move → `handle_mouse_move`
(`:2043`), left up → `handle_mouse_up` (`:2106`).

`handle_mouse_down` precedence (`:1711-1830`): active-drag guard → chart hit-test (`:1748`) →
fill-handle (`:1766`) → `pane.hit_test` (`:1785`) → `mouse_down_cell`/header/corner
(`:1816-1830`). `mouse_down_cell` (`:1899-1984`) is where a link click belongs — but
**point-mode intercepts a plain click first** (`:1912-1936`) when a formula edit is
reference-ready. `event.modifiers.shift` is the only modifier read (`:1913`, `:1940`);
`click_count >= 2` opens the in-cell editor (`:1949`, `:1958`).

Pixel → cell: `grid/layout.rs:126` `hit_test -> GridHit` (`GridHit` at `:57-66`); freeze-aware
`PaneGeometry::hit_test` (`:438`) and `cell_at_point` (`:474`); `PaneGeometry` at `:313`, built
by `GridView::input_pane_geometry` (called `view.rs:1741`, `:2567`).

**There is no per-cell hover state and no hover cursor on cells today.** Existing cursor uses:
`.cursor_col_resize()` (`view.rs:4560`, hotspot `:4553`), `.cursor_row_resize()` (`:4591`,
hotspot `:4578`), `.cursor_pointer()` (`:4750`, `:4825`, `:5021` — context-menu items only);
elsewhere `chrome/view.rs:5117` (tab drag), `shell/about.rs:235`. `handle_mouse_move`
(`:2043-2102`) is purely drag-dispatch and **returns early at `:2087-2089` when no drag is
active** — a bare hover does nothing.

Cheapest cursor approach: a per-cell `.cursor_pointer()` on the `cell_element` div when the cell
has a link (pairs with the `visible_links` change). A `handle_mouse_move`-driven
`hovered_link` + `cx.notify()` adds a per-move repaint — avoid.

**No modifier convention is taken.** `grid/input.rs:74-95` binds only `secondary+shift+v` and
`secondary` + `c/x/v/a/d/r`. `caret_intent_modifiers` (`view.rs:81-90`) treats
shift/control/alt/platform as caret-intent for quick-edit. Excel/Sheets open on **plain click**;
Numbers uses ⌘-click.

Emission: add to `enum GridEvent` (`grid/mod.rs`) and emit via `self.events.emit(&GridEvent::…,
window, cx)` (`view.rs:1966`, `:1916`); route in `make_grid_sink` (`shell/window.rs:1666-1955`
— `GridEvent::ChartSelected(id)` at `:1940` is the template for a UI-side, non-worker event).

## 4. Opening a URL

**`cx.open_url(url)`** — sole existing use, `shell/about.rs:229-238`:

```rust
fn link(id: &'static str, label: &'static str, url: &'static str) -> impl IntoElement {
    div().id(id).font_family(LINK_FAMILY).text_size(px(13.0))
        .text_color(rgb(LINK)).cursor_pointer()
        .on_click(move |_: &ClickEvent, _window, cx| cx.open_url(url))
        .child(label)
}
```

Not to be confused with `Application::on_open_urls` (`main.rs:96`, `shell/open_files.rs`) — the
inbound macOS Finder Apple-Event bridge, opposite direction. `shell/open_files.rs` has
`file_url_to_path` (exported at `shell/mod.rs:47`), useful for `file://` links.

**`cx.open_url` will open anything.** A workbook is untrusted input; `javascript:`, `file:///…`,
`smb://` are all reachable from an opened `.xlsx`. A scheme allowlist is required (see
`projects/pre-distribution-security-audit.md`).

## 5. Dialogs / modals

There is **no dialog framework** (`shell/window.rs:1401-1402` says so), and stock
gpui-component `Popover`/`ContextMenu`/`Modal` are deliberately unused (`chrome/view.rs:11-16` —
their content closures run in a foreign entity context).

**Pattern A — window-owned modal:** `enum ActiveModal` (`shell/window.rs:94-111`), field
`modal: Option<ActiveModal>` (`:151`), `render_modal` (`:1403-1470`, dim backdrop
`bg(rgb(0x000000).opacity(0.3))` + centered card), `dialog_card(title, body, buttons)` helper
(`:1490-1521` — white card, `w(px(360.0))`, hairline border, `rounded_lg`, `shadow_lg`,
right-aligned ghost buttons then a `.primary()` right-most). Open sites `:481, 579, 616, 639,
689, 741, 756, 844, 855, 865, 878, 1000`; dismiss `:901`; test accessors `:1081-1091`.
`dialog_card` takes only `Vec<Button>` + a `&str` body, so a dialog with a text field needs a
sibling function.

**Pattern B — chrome-owned overlay panel (has `InputState`):** `render_overlays`
(`chrome/view.rs:5294-5353`, pushed from `Render` at `:4112`), `backdrop(on_dismiss, cx)`
(`:5355+`, uses `BlockMouse` so clicks/scroll don't leak to the grid — the comment at `:5365`
notes it fixed two real bugs), `render_delete_confirm` (`:7547-7607`) for card chrome, and
**`render_find_bar` (`:4650-4760`)** for a real form: state `find_open` (`:505`), `open_find`
(`:4843-4855` — note `window.on_next_frame(Self::select_find_query(field))` at `:4851` for
focus+select), `close_find` (`:4874`), Escape wired at `:4702-4706`. `anchored_trigger`
(`:4123-4149`) if the panel should anchor under a toolbar button.

**Recommendation:** ⌘K is a *form* (URL + optional display text, pre-filled, Enter = OK,
Escape = cancel, focus+select on open) — that's the find bar's problem, not the confirm dialog's.
Model the lifecycle on `render_find_bar`/`open_find`/`close_find` and the card chrome on
`render_delete_confirm`. That puts it in `ChromeView`, where `Input`/`InputState` already live
(`chrome/view.rs:29`).

## 6. Keybindings + menus

- **Actions** — `shell/mod.rs:57-101`, single `actions!(freecell, [...])`. Data-carrying actions
  use `#[derive(gpui::Action)] #[action(namespace = freecell, no_json)]` (`OpenRecent` at
  `:104-116`).
- **Keymap** — `shell/menus.rs:30-52` `bind_keys(cx)`; `primary()` = cmd/ctrl (`:20-26`).
  Existing: `n o s shift-s w z shift-z b i u ctrl-m f q`. **`k` is free**; ⌘K does not collide
  with a system shortcut (contrast the ⌃⌘M precedent at `:44-48`).
- **Menu bar (macOS only)** — `menus.rs:66-95`; Edit menu `:84-93` (Undo, Redo, sep, Find…,
  Merge Cells). Items grey out by whether a handler is in scope (`:56-58`); key equivalents come
  from the keymap. Installed via `install_menus_with` (`:122-126`). **No Linux menu bar**
  (`:4-7`) — every action must be keyboard- or context-menu-reachable. Menu tests inline at
  `:128-348`; `edit_menu_has_merge_cells_after_find` (`:322-337`) is the placement-test template.
- **Action handlers** — `shell/window.rs:1326-1362`, `.on_action(cx.listener(…))` on the
  `WorkbookWindow` root. `ToggleUnderline` (`:1347-1349`) → `toggle_style` (`:1396-1399`) →
  `chrome.update(…)` is the shape to copy (window-scoped, so it greys out on the Welcome window).
- **Cell context menu exists** — `struct CellMenu` (`grid/view.rs:163-168`), opened from
  `handle_right_mouse_down` → `open_cell_menu` (`:2622-2626`, fn `:2698`), items from the **pure**
  `cell_menu_items` (`:4880-4990+`) returning `Vec<Option<(String, bool, GridEvent)>>` (`None` =
  separator), unit-tested at `:9138+`. Sibling `header_menu_items` (`:4614`). Adding link items
  is a ~10-line pure change + new `GridEvent` variants + a `window.rs:1666` sink arm.

## 7. Action row / toolbar

`ChromeView::render_action_row` (`chrome/view.rs:4151`, rendered from `Render` at `:4101`).
Button pattern (`:4158-4174`):

```rust
Button::new(id).icon(Icon::empty().path(icon_path)).tooltip(tooltip)
    .ghost().small().disabled(disabled).selected(pressed)
    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| { … }))
```

`disabled = self.degraded` (`:4153`). Buttons live in an `h_scroller` and must be
`flex_shrink_0` (`:4219-4225`) — "scroll, don't squish". Divider helper `action_divider()`
(used `:4733`). Dialog-opening precedent: the Find trigger (`:4531-4535`, `icons/search.svg`,
`.selected(self.find_open)`).

**Icon** — assets composed at `shell/assets.rs:244-267`: FreeCell-vendored (`FREECELL_ICONS`,
`:37-180`) first, then `gpui_component_assets::Assets` (`:253`); namespaces disjoint. 32 SVGs
vendored today; **`link.svg` is not among them**. Whether the pinned gpui-component bundle
(`rev = a9a7341c…`, "a curated ~99 icon Lucide subset", `assets.rs:6-8`) ships `icons/link.svg`
**was not verifiable offline**. Verify with the established idiom (`assets.rs:359-365`):

```rust
assert!(matches!(AppAssets.load("icons/link.svg"), Ok(Some(_))));
```

If it errs, vendor Lucide `link` (or `link-2`) in `stroke="currentColor"` form, register it in
`FREECELL_ICONS`, and add a resolution test — the tintability test (`assets.rs:277-295`) then
enforces the form.

## 8. Clipboard

**Links ride along iff IronCalc's `ClipboardData` carries them — there is no FreeCell-side cell
payload to extend.** UI side `shell/clipboard.rs`: `ClipboardCoordinator` holds only
`last_copy_text` (`:21-24`); its job is internal-vs-foreign routing (`paste_command`, `:100-125`)
— byte-identical to our last copy → `Command::PasteInternal`, else `Command::PasteTsv`. Worker
side: `ClipboardSlot` (`run.rs:127-136`) holds the serialized IronCalc `ClipboardData` as
`serde_json::Value`; `apply_copy` (`:1252`, `copy_to_clipboard` `:1278`), `apply_paste_internal`
(`:1288`, `doc.paste_clipboard` `:1380` → `document.rs:1767`).

External TSV paste (`document.rs:1789`) will never carry links — same limitation already logged
as `projects/excel-clipboard.md` / the "Excel clipboard interop" v1.0 gap.

Open question: `Command::PasteValues` (`document.rs:1820`) strips formatting — **Excel's Paste
Values drops hyperlinks**. Decide and test.

## 9. Render tests

- Entry: `app/render-tests/tests/render_suite.rs` — one `#[test]` per case via `render_cases!`
  over `cases::all()`; `Gate` policy `:49-70` (needs `FREECELL_RENDER=1` + capture tools).
  Runner `app/render-tests/scripts/render_tests.sh` (subset filter documented `:19-24`).
- Case table: `app/render-tests/src/cases.rs` — `RenderCase` `:86-136`, `all()` `:570`. Header
  comment (`:1-4`): *"one row per rendering feature or meaningful permutation… `name` is
  snake_case and IS the baseline filename."*
- Scene builder: `app/render-tests/src/scene.rs` — `Scene` `:54`, fluent methods `:93-268`
  (`input`, `bold`, `italic`, `underline` `:133`, `strikethrough`, `wrap`, `fill`, `align`,
  `font_color`, `border`, `cond_fmt` `:252`, `merge` `:261`). `build_sources` `:285` drives the
  **real** worker. `enum Inject` (`:35-50`) is the escape hatch for styles with no worker command.
- Baselines: `app/render-tests/baselines/*.png` (165 files).
- Simplest template (`cases.rs:576-580`):

```rust
let cell = |name, scene| RenderCase::new(name, scene, CELL_VP);
let at = |scene: Scene| scene.input(1, 1, SAMPLE);   // B2, so A1's active outline never overlaps
cell("cell_underline", at(Scene::new()).underline(1, 1)),
```

Add `Scene::link(row, col, url)` next to `underline` (`scene.rs:~135`), routed through a real
`Command::SetCellLink` (preferred — keeps the scene "real") rather than `Inject`.

## 10. Tests generally

| Where | Convention |
|---|---|
| `freecell-engine` unit | inline `#[cfg(test)] mod tests` — `document.rs:2076`, `worker/run.rs:3998`, `cache.rs:520`/`:614`, `worker/protocol.rs:739`, `worker/client.rs:226` |
| `freecell-engine` integration | `freecell-engine/tests/` — xlsx link round-trip → `roundtrip.rs` |
| `freecell-app` gpui view tests | inline `#[gpui::test] fn name(cx: &mut TestAppContext)` — `chrome/view.rs` (248), `grid/view.rs` (77), `shell/app.rs` (47) |
| `freecell-core` | inline `#[cfg(test)]` + `tests/dependency_rule.rs` (no gpui/ironcalc in core) |

House style is **pure-logic-first**: extract the decision into a gpui-free function, unit-test
it, keep the gpui layer thin (`grid/input.rs`, `cell_menu_items`, `header_menu_items`). Do that
for link URL validation/normalization.

gpui test idiom: `cx.update(gpui_component::init)` → `cx.open_window(...)` → `Root::new(...)` →
`VisualTestContext::from_window` → `vcx.run_until_parked()` → `vcx.debug_bounds("selector")`
(`shell/about.rs:246-292`); selectors set with `.debug_selector(|| "…".into())`
(`chrome/view.rs:4699` `"find-bar"`, `grid/view.rs:6049` `"in-cell-editor"`). Mouse helpers:
`mouse_ev(MouseButton::Right, x, y)` and the left-click helper at `grid/view.rs:7510-7536`;
`right_click_cell_outside_selection_moves_and_opens_menu` (`:9497`) is the click-test template.

## 11. Landmines

1. **`GAPS.md:439` is stale** — its "checked:" note is no longer true upstream. Per
   `CLAUDE.md:52-56` the row is **deleted** when this ships, not annotated. It explicitly bundles
   `HYPERLINK()`. `GAPS.md:486` (external workbook links) is a **distinct, later** row — don't
   conflate.
2. **`projects/xlsx-preservation.md:26-27` lists hyperlinks as "confirmed dropped / unmodeled"**
   — needs updating once links are modeled; opening + saving a file with links now round-trips
   instead of silently stripping. Test in `tests/roundtrip.rs`.
3. **In-cell editor.** `resolve_incell_font` (`view.rs:5872-5891`) mirrors bold/italic/underline
   into the hosted `InputState` (`:6006-6047`). If link styling folds into `RenderStyle`,
   **editing a link cell renders the edit text blue+underlined** — probably wrong.
   `incell_font_resolves_cell_style_including_underline` (`:10203-10228`) is the test that will
   fail. `IncellFont` (`:5857-5870`) has no color field today.
4. **The mirror path drops style entirely** — `view.rs:3475` and `:1037`: a cell with a pending
   edit renders `(raw, rgb(CELL_TEXT), CellKind::Text, None)`. Link styling must not resurrect
   through the mirror, and must not flicker.
5. **Text spill duplicates style resolution** — `SpillPlan` (`:714-726`) + `spill_element`
   (`:5500-5560`, underline `:5540`). A long URL is exactly the case that spills.
6. **Merged regions paint through `resolve_cell_paint`** (`:1031`, called `:3625`) — a link on a
   merge anchor must resolve there, and `region_or_cell_rect` (`:1073`) defines the clickable rect.
7. **Style-cache agreement contract** — see §2; needs build + mirror + undo-touch-set paths.
8. **`RenderStyle` interning identity** — prefer a sparse link map over a `RenderStyle` field.
9. **Type-aware alignment is live, not deferred.** `projects/type-aware-alignment.md` says
   "Future" but it **shipped** — `CellKind` (`core/publication.rs:22-29`), `default_align`
   (`:35-43`), consumed at `view.rs:5402-5409`. A `HYPERLINK()` formula returns text →
   left-aligned; a link on a *number* cell stays right-aligned. Don't "fix" alignment for links.
10. **Point-mode steals plain clicks** (`view.rs:1912-1936`) — a plain-click-opens design must
    sit *after* that branch and must not fire during formula editing.
11. **⌘K while an editor is focused.** Every binding uses context `None` (`menus.rs:33-51`), so
    `InsertLink` fires globally — including mid-edit in the in-cell editor or the find field.
    `capture_key_down` (`view.rs:6376`) intercepts only Tab/Esc/nav. Decide: commit-then-open
    (like `toggle_style`, `window.rs:1396`) or suppress.
12. **IME / international text** — `projects/ime-text-input.md` (Future). The URL field is a
    `gpui_component::input::InputState` and inherits its behavior; non-ASCII/IDN URLs untested.
13. **`InputState` cannot be range-styled** — `projects/styled-text-input-control.md:14-21`. You
    cannot render a link as styled text *inside* the formula bar or in-cell editor; the link is a
    whole-cell property only.
14. **`cx.open_url` on untrusted workbook content** — scheme allowlist required (§4).
15. **Degraded mode** — `chrome/view.rs:4153` gates every mutating control; `shell/window.rs:141`
    holds the reason. A new toolbar button and ⌘K must both respect it.
16. **Render-test policy is a real gate** (`CLAUDE.md:150-160`) — dedicated final render phase,
    no interleaved full runs.
17. **Fork branch hygiene** (`CLAUDE.md:77-81`) — if this needs two unrelated fork capabilities,
    that's two `fix/<slug>` branches and two upstream PRs, never one bundled branch.

### Resolved by spot-check (2026-08-07)

- **Theme colors resolve.** `resolve_rgb` (`freecell-engine/src/cache.rs:181`) handles
  theme-indexed colors, so the engine's `Color::Theme(10, 0.0)` needs no new plumbing.
- **FreeCell never calls `Model::to_bytes`/`from_bytes`** (grep across `app/crates`), so the
  upstream `Worksheet` bitcode-layout change is a **non-issue** for FreeCell.

## Quick file index

| Concern | File:line |
|---|---|
| Engine facade type | `freecell-engine/src/document.rs:186` |
| Style write methods (add link setters) | `document.rs:1056-1113` |
| Cell reads (add link getter) | `document.rs:395-470` |
| `Command` / `WorkerEvent` enums | `worker/protocol.rs:200` / `:670` |
| Command bucketing (edits arm) | `worker/run.rs:577-610` |
| `apply_one` dispatch | `worker/run.rs:3462` |
| `AppliedKind` / `Touch` / `AppliedOp` | `worker/run.rs:68` / `:105` / `:173` |
| `DocumentClient` / `Shared` | `worker/client.rs:72` / `:29` |
| `RenderStyle` (has `underline`) | `freecell-core/src/style.rs:38-86` |
| `SheetCache` | `freecell-core/src/cache.rs:48-117` |
| `render_style_from` / `resolve_rgb` | `freecell-engine/src/cache.rs:215` / `:181` |
| `build_sheet_cache` (CF gate precedent) | `freecell-engine/src/cache.rs:314-327` |
| `CellPaint` / `resolve_cell_paint` | `grid/view.rs:731` / `:1031` |
| `resolve_frame` (style snapshot) | `grid/view.rs:1250`, snapshot `:1367` |
| **`cell_element`** | `grid/view.rs:5353`, underline `:5417` |
| `SpillPlan` / `spill_element` | `grid/view.rs:714` / `:5500` |
| in-cell editor font resolve | `grid/view.rs:5872`, overlay `:5978-6047` |
| `handle_mouse_down` / `mouse_down_cell` | `grid/view.rs:1705` / `:1899` |
| `handle_mouse_move` (no hover state) | `grid/view.rs:2043` |
| **`cell_menu_items`** | `grid/view.rs:4880` |
| pixel→cell hit-test | `grid/layout.rs:126`, `:438`, `:474` |
| GridEvent routing | `shell/window.rs:1661-1955` |
| Actions / keymap / Edit menu | `shell/mod.rs:57` / `menus.rs:30` / `menus.rs:84` |
| `ActiveModal` / `render_modal` / `dialog_card` | `shell/window.rs:94` / `:1403` / `:1490` |
| `render_find_bar` / `open_find` (⌘K template) | `chrome/view.rs:4650` / `:4843` |
| `render_action_row` / button pattern | `chrome/view.rs:4151` / `:4158` |
| Icon asset source | `shell/assets.rs:37`, `:244` |
| `cx.open_url` | `shell/about.rs:229-238` |
| Render case table / scene builder | `render-tests/src/cases.rs:86`, `all()` `:570` / `scene.rs:54` |
| GAPS hyperlink row | `GAPS.md:439` |
