---
status: draft
---

# Phase 4: Render links + open them

## Overview

The first phase a user can *see*. Phase 3 made the cache's link map trustworthy; this phase reads
it: a per-frame `visible_links` snapshot taken under `resolve_frame`'s existing single read lock,
link appearance at the three call sites that must agree (F1.3/F1.5), a pointer cursor + tooltip on
a link cell (F2.4/F2.5), plain-click-to-open decided on **mouse-up** by a pure function (F2.1/F2.2),
and — behind a `GridEvent::OpenLink` the worker never sees — the open policy: the pure `decide_open`
allowlist with its hostile-input table (F3), the `file:` confirm, the refusal message, and
internal-link navigation (F4).

Authoring (⌘K, remove, menus, toolbar) is Phase 5; the full pixel suite, baseline generation, and
the CI `render` gate are Phase 7. This phase verifies with the `link_` render subset only (which,
per *Found while building*, still costs a full capture pass — the filter narrows the diffing, not
the rendering).

## Decisions taken here

### D1 — Worksheet links get no FreeCell styling; only dynamic links are synthesized

Settled in Phase 1's verification gate and Phase 3's plan: theme index 10 is the Office `hlink` slot
and resolves to `#0563C1` through the existing `resolve_rgb`, so a worksheet link's blue+underline is
ordinary engine formatting. Arch §3.5's `LINK`-substitution fallback stays dropped. The **only**
render-time link styling is F1.3's synthesis for `HYPERLINK()` cells, applied only where the cell has
no explicit font colour of its own (F1.4).

Consequence for the new `LINK` constant: it is used by the **dynamic** synthesis path (and already by
`shell/about.rs`). Consolidating it into `grid/mod.rs` per arch §4 still happens — it is now a
two-caller constant instead of a three-caller one.

### D2 — The degraded gate lives on the window, not in the grid's click decision

Arch §5 lists `degraded` among the pure click-decision function's inputs. `GridView` has no degraded
state today (only `ChromeView` and `WorkbookWindow` do), so honouring that literally would mean a
third mirror of the flag, pushed on every `WorkerDegraded`. Instead the pure function decides the
*gesture* (`same_cell`, `dragged`, `primary_modifier`, `click_count`) and
`WorkbookWindow::open_link` — which owns `degraded` and is where anything actually opens — refuses in
degraded mode (F2.2). One source of truth, same observable behaviour.

### D3 — "Non-blocking message" = the gpui-component notification layer

F3.3 and F4.3 both want a non-blocking message; the app has none today (`ActiveModal` is blocking and
`dialog_card` is a modal card). The app already builds every window over `gpui_component::Root`, which
owns a `NotificationList`; it just never renders the layer. `WorkbookWindow::render` gains
`.children(Root::render_notification_layer(window, cx))` and refusals/unresolvable targets go through
`window.push_notification(...)`. This is out of the pixel suite's scope (window chrome, no baseline).

### D4 — Phase 3's two-write-lock transient: confirmed, not a flicker

Phase 3 carried this forward for Phase 4 to settle: `refresh_cache_cells` installs the touched cells'
styles under one write lock and `refresh_resident_links` installs the link maps under a second, so a
frame can observe an edit's new styles with its old link map.

**Verdict: it cannot surface as a flicker, and needs no fix in this phase.** The reasoning, recorded
so a future reader does not have to redo it:

- A *flicker* is a channel showing a value that is neither the old nor the new state, or showing the
  new state and then reverting. Neither happens here: both installs move monotonically from the old
  map to the new one, and the `StyleCacheUpdated` → `cx.notify()` → repaint that follows **both**
  installs always paints the settled state. The worst case is a bounded, self-healing ≤1-frame lag of
  one channel behind the other.
- The **tooltip** channel cannot observe it at all: a tooltip needs a hover dwell (gpui's show delay)
  orders of magnitude longer than the ~µs–ms window between the two locks.
- The **cursor** channel needs the pointer to already be resting on the very cell whose link changed
  *and* a repaint to land inside that window. The lag is one frame and corrects itself.
- The **styling** channel is not affected for worksheet links at all — their blue+underline rides the
  *style* half (D1), so styles and links move in the same install for the only case where a
  disagreement would be visible as pixels. Dynamic-link synthesis does read both, but it tears against
  the **publication** (a separate `ArcSwap`, no cross-consistency with the caches by design), not
  against the style map — so the two-write-lock window is not the mechanism there.
- Cross-lock tearing is anyway pre-existing and tolerated: the style loop itself takes one write lock
  **per `(sheet, range)` entry**, so a multi-entry refresh already tears between sheets.

Arch §4's single-read-lock snapshot is the part that matters and is implemented: `visible_links` is
snapshotted in the same `resolve_frame` guard as `visible_styles` and `visible_merges`, so *within a
frame* the three are mutually consistent. If the lag ever does show up, the fix Phase 3 named (fold
the link install into the style write lock) stays available; it is not worth engine surgery in a UI
phase for a self-healing one-frame affordance lag. A `// see phase_4.md D4` comment sits at the
snapshot site.

### D5 — Defined-name internal links do not resolve

F4.1 allows an internal link to name a defined name. FreeCell models no defined names anywhere (no
engine facade, no worker command, no read model), so `parse_internal_location` classifies a
non-A1 location as a defined name and navigation reports it unresolvable (F4.3's non-blocking
message) rather than pretending. This is a genuine hole → a `GAPS.md` row, carried to Phase 6.

### D6 — The two link blues are near, not equal — and that is as close as a constant gets

Arch §4 and F1.1 name `LINK = 0x2563EB` as *the* link colour; the Phase-1 gate then established that
a **worksheet** link's colour comes from the engine's theme slot 10, which in the default theme is
`#0563C1`. So the two kinds of link paint slightly different blues: worksheet `#0563C1` (engine),
dynamic `#2563EB` (FreeCell's token).

Kept as specified, on two grounds. First, F1.3's requirement is *visually indistinguishable*, and the
`link_plain` / `link_dynamic` captures are: both read as the same link blue at cell size (eyeballed
side by side). Second — the decisive one — **no constant can make them equal anyway**: the engine's
colour is resolved from the *workbook's* theme, so a file with a custom theme moves the worksheet
blue and any FreeCell-side constant drifts with it. Closing this properly means the grid reading the
theme's resolved hyperlink colour (a build-time scalar on `SheetCache`, like `default_font_size_q`
already is) rather than picking a different constant — engine + core surface, for a difference no
user can currently see. Recorded here so the choice is visible; flagged to Phase 6 as a candidate
row rather than done on spec.

## Steps

### 1. `freecell-core::link` — the pure layer (arch §5, §6, §7)

Extend `freecell-core/src/link.rs` (it already owns `LinkTarget`/`CellLink`/`LinkMap`):

```rust
pub enum OpenDecision { Open(String), ConfirmFile(String), Refuse { scheme: String } }
pub fn decide_open(target: &LinkTarget) -> OpenDecision;

pub struct LinkClick { pub same_cell: bool, pub dragged: bool,
                       pub primary_modifier: bool, pub click_count: usize }
pub fn link_click_opens(click: LinkClick) -> bool;

pub enum InternalTarget { Cell { sheet: Option<String>, cell: CellRef }, DefinedName(String) }
pub fn parse_internal_location(location: &str) -> Option<InternalTarget>;

pub fn link_tooltip_text(target: &LinkTarget) -> String;
```

- **`decide_open`** — scheme = the text before the first `:`, ASCII-lowercased, and **only** when it
  is a well-formed RFC-3986 scheme (letter, then letters/digits/`+`/`-`/`.`) reached before any `/`,
  `?`, `#`, or whitespace. `http|https|mailto|ftp|ftps` → `Open`; `file` → `ConfirmFile`; any other
  scheme → `Refuse`. A target with **no** scheme (a bare path, `//evil.com`, `C:\x` — a Windows drive
  letter is one character, not a scheme) is `Refuse { scheme: "" }` rather than being guessed at, and
  the message says so. Leading/trailing whitespace is trimmed before the scheme is read (so
  `"  javascript:"` cannot slip past), and a target containing a control character (`\n`, `\r`, `\0`,
  …) is refused outright — `cx.open_url` must never receive a smuggled second line. `Open`/`ConfirmFile`
  carry the trimmed target, so what is inspected is what is launched.
- **`link_click_opens`** — `same_cell && !dragged && !primary_modifier && click_count == 1`.
- **`parse_internal_location`** — splits on the **last** `!`, unquotes a `'…'` sheet name (with `''`
  → `'`), parses the remainder with `CellRef::from_a1`, and falls back to a range's top-left via
  `CellRange::from_a1`. No `!` and a parseable A1 ref → same-sheet `Cell { sheet: None, .. }`.
  Anything else → `DefinedName` (D5). A leading `#` is stripped by the caller when it builds the
  `LinkTarget::Internal`, so it is not re-handled here.
- **`link_tooltip_text`** — the ScreenTip on its own first line when present, then the destination.

### 2. `LINK` in `grid/mod.rs`; `shell/about.rs` reuses it (arch §4)

`pub const LINK: u32 = 0x2563EB;` beside `ACCENT`, with the note about the design system having no
link token moved onto it. `shell/about.rs` drops its private copy and imports
`crate::grid::LINK`.

### 3. `GridView`: the per-frame link snapshot (arch §4)

- New field `visible_links: HashMap<(u32, u32), CellLink>`, documented beside `visible_merges`.
- Populated in `resolve_frame` **inside the existing caches read guard**, right after
  `visible_styles`, over the same quadrant-union `(0..m).chain(rows) × (0..k).chain(cols)`
  cross-product, and short-circuited when `cache.links().is_empty()` (the overwhelmingly common
  case pays one `is_empty()`). The merged-region loop tops up each region **anchor**, so a region
  whose anchor scrolled off-screen still paints and clicks as a link.
- Carries the D4 comment.

### 4. Link appearance at the three call sites (F1.3–F1.5)

A private helper keeps the three honest:

```rust
/// The paint adjustments a cell's effective link makes: F1.3's synthesized appearance for a
/// **dynamic** link, applied only where the cell has no explicit font colour (F1.4). A worksheet
/// link needs none — the engine already styled it (F1.2).
fn link_appearance(link: Option<&CellLink>, style: Option<RenderStyle>,
                   own_color: Option<Rgb>) -> (Option<RenderStyle>, Rgba);
```

The synthesized `RenderStyle` is a **paint-time local copy**, never interned into the cache — that
is what keeps the interning contract (arch §3.3) intact, and what makes F1.7 fall out: the in-cell
editor resolves from `visible_styles`, which the synthesis never touches.

Call sites, all of which resolve `link` from `visible_links` and **skip it entirely when the cell is
mirrored** (a pending edit renders in the default style — F1.7 and landmine #4):

| Site | What changes |
|---|---|
| `build_quadrant`'s per-cell loop | `attr_style` / `text_color` before the spill decision, so the spill plan inherits them |
| `resolve_cell_paint` | same, so a merged-region anchor paints identically |
| `SpillPlan` → `spill_element` | nothing new — it already carries the adjusted `style` + `text_color` from the loop |

`CellPaint` gains `link: Option<CellLink>` so the merged-region box can decorate (cursor/tooltip)
from the same resolution the 1×1 path uses.

### 5. Cursor + tooltip on a link cell (F2.4, F2.5)

`cell_element` gains a `link: Option<LinkDecoration>` parameter:

```rust
struct LinkDecoration { id: ElementId, tooltip: SharedString }
```

- `.cursor_pointer()` — declarative; `Interactivity::should_insert_hitbox` already makes a
  cursor-only div hittable, so there is no hover state and no per-move repaint.
- `.id(id).tooltip(...)` — gpui's `tooltip` is a `StatefulInteractiveElement` method, so the div
  needs an id, and it must be unique per painted element. The id is
  `("cell-link", quad_tag << 40 | row << 16 | col)`: the quadrant tag is load-bearing because a
  merged region straddling a freeze divider is deliberately painted **once per quadrant**, and two
  identically-id'd elements would share tooltip state. Only link cells get an id, so ordinary cells
  keep today's id-free element.
- The spill element is deliberately **not** decorated: it is painted over the *neighbour* cells,
  and the click decision is geometric (hit-test → cell), so a pointer cursor there would promise an
  open that the neighbour cell will not perform.

### 6. Click to open (arch §5, F2.1–F2.2)

- `mouse_down_cell` records `pending_link: Option<PendingLinkClick { cell, target, press: (f32, f32) }>`
  — **after** the point-mode branch returns, so a reference-ready formula edit never arms one, and
  keyed on the merge-snapped `cell` so a click anywhere in a merged region uses the anchor's link.
  **Shift** is excluded here too, structurally: a shift-click means "extend the range to here",
  which is not a click on that cell at all, so there is no gesture for the release to weigh. (The
  primary modifier is different — it IS weighed at release, because F2.2 makes it the explicit
  select-without-following affordance on an otherwise ordinary click.)
  Every other `handle_mouse_down` arm (chart, fill handle, header, corner) and every mouse-down that
  reaches `mouse_down_cell` with no link clears it.
- `handle_mouse_up` takes the event (it was `_event`), resolves the release cell with the same
  freeze-aware hit-test path as mouse-down, and asks `link_click_opens`:
  `same_cell` = release cell (merge-snapped) equals the pressed cell; `dragged` = the release
  position is more than `LINK_CLICK_SLOP_PX` (3.0) from the press position; `primary_modifier` =
  `event.modifiers.secondary()`; `click_count` from the event. It runs **after** the chart / resize /
  fill / point-drag arms and takes the pending link before the selection-drag arm, so a
  drag-to-select that started on a link cell still ends normally.
- On yes: `GridEvent::OpenLink { target }`.

### 7. The window sink + open policy (arch §6, §7, F3, F4)

- `GridEvent::OpenLink { target: LinkTarget }` in `grid/mod.rs`.
- `make_grid_sink` gains a `window_slot: Rc<OnceCell<WeakEntity<WorkbookWindow>>>`, set from
  `cx.weak_entity()` at the top of `WorkbookWindow::new` (the sink is built before `Self`, and the
  entity handle is already valid inside `cx.new`). The arm forwards to
  `WorkbookWindow::open_link(&target, window, cx)`.
- `WorkbookWindow::open_link`:
  - degraded → return (D2);
  - `LinkTarget::Internal` → `navigate_internal` (never `decide_open` — F4.4);
  - else `decide_open`: `Open(url)` → `cx.open_url`; `ConfirmFile(url)` → `ActiveModal::ConfirmFileLink { url }`
    (Cancel ghost + default, "Open File" primary → `cx.open_url`); `Refuse { scheme }` →
    `window.push_notification(...)` naming the scheme (or "no scheme" for the schemeless case).
- `navigate_internal` parses with `parse_internal_location`, resolves the sheet name against
  `self.sheets` (case-insensitive, as Excel's sheet names are), then **defers** the grid work
  (`window.defer`) because the sink fires from inside the grid's own `update`: switch sheets via the
  existing `switch_grid_to_sheet` when needed, then `select_and_reveal` + mirror the selection into
  `sink_shared` / the chrome, exactly as `ChromeGridRequest::SelectAndReveal` does. An unknown sheet,
  an unparseable ref, or a defined name → notification, no navigation (F4.3, D5).
- `WorkbookWindow::render` renders the notification layer (D3).

### 8. Render-test scene support (arch §10)

- `Scene::link(row, col, target)` beside `underline`, applied as a real `Command::SetCellLink`
  (never `Inject`), so the scene exercises the engine's own link styling.
- New `link_*` cases: `link_plain`, `link_dynamic`, `link_spill`, `link_in_merged_region`,
  `link_over_fill`, `link_with_explicit_font_color`, `link_removed`. Two of them are deliberately
  **`HYPERLINK()`** cells rather than worksheet links, because a worksheet link's appearance rides
  its ordinary `RenderStyle` — plumbing the spill and colour-precedence paths already carried before
  links existed — so it is the FreeCell-**synthesized** half that can regress there:
  - `link_spill` — a long URL from `=HYPERLINK(...)`;
  - `link_with_explicit_font_color` — F1.4's precedence is only FreeCell's business for a dynamic
    link. For a worksheet link the link colour *is* the cell's font colour, so a later user colour
    simply overwrites it, which is the engine's behaviour and is covered by the Phase-3 engine tests.
    (There is also no way to express it in a scene: `Scene::font_color` is an `Inject` applied to the
    cache, while the engine's own link colour arrives in the *published* cell, which wins.)
- The names must be added to `render_suite.rs`'s `render_cases!` list as well as `cases::all()` —
  `case_names_match_table` guards the drift, but a name-filtered subset run filters that guard out
  too, so a missing row shows up as "0 tests ran", not as a failure.
- Baselines are **not** generated here — Phase 7 owns generation, eyeballing, and the CI gate. This
  phase runs `render_tests.sh test link_` only to confirm the cases build and render, and records
  what it saw.

## Tests

**`freecell-core` (`link.rs`)**

- `decide_open_allows_the_five_safe_schemes` — `http`, `https`, `mailto`, `ftp`, `ftps`, including
  uppercase (`HTTPS://`) and surrounding whitespace, each returning the trimmed target.
- `decide_open_confirms_file_urls` — `file:///etc/passwd` → `ConfirmFile`, never `Open`.
- `decide_open_refuses_hostile_targets` — the arch §6 table: `javascript:alert(1)`,
  `JaVaScRiPt:alert(1)`, `  javascript:alert(1)`, `data:text/html,…`, `vbscript:msgbox`,
  `smb://host/s`, scheme-relative `//evil.com`, a bare path, and `C:\Windows\notepad.exe`. Each
  refuses and names the scheme it saw.
- `decide_open_refuses_a_target_with_a_control_character` — `http://ok\njavascript:alert(1)`
  refuses rather than handing a two-line string to `open_url`.
- `decide_open_is_never_reached_for_internal_targets` — an `Internal` target refuses (the belt to
  F4.4's braces: the caller routes it away, and the policy still will not launch it).
- `link_click_opens_only_on_a_plain_single_click` — the truth table: the yes case, plus a different
  cell, a drag, the primary modifier, and `click_count == 2`.
- `parse_internal_location_reads_the_shapes_excel_writes` — `Sheet1!A30`, `'My Sheet'!B2`,
  `'It''s'!A1`, a bare `A30`, a range `Sheet1!A1:C3` → its top-left, and `$A$1`.
- `parse_internal_location_classifies_a_non_reference_as_a_defined_name` — `MyRange`, `Sheet1!zzz`.
- `link_tooltip_text_shows_the_screentip_above_the_target` — with and without a ScreenTip.

**`freecell-app` (`grid/view.rs`, gpui)** — over a `link_sources()` fixture carrying a worksheet
link (with the engine's real blue+underline baked in), two dynamic links (one plain, one with an
explicit red font colour), a merged region whose anchor is linked, a linked *empty* cell, and a
plain control.

- `visible_links_snapshots_the_viewport_and_merge_anchors` — an in-viewport link is snapshotted with
  its `dynamic` flag; a merged region's anchor is too; an unlinked cell is not; and scrolling far
  away empties the snapshot (it is viewport-scoped, not a clone of the sheet map).
- `a_dynamic_link_synthesizes_blue_underline_and_an_explicit_colour_wins` — F1.3 + F1.4 through
  `resolve_cell_paint`: a dynamic link with no colour resolves to `LINK` + underline; with an
  explicit colour it keeps that colour and stays underlined; a **worksheet** link resolves to the
  *engine's* theme blue and explicitly `assert_ne!`s FreeCell's `LINK`, which is what proves no
  substitution happened.
- `a_pending_edit_suppresses_link_appearance` — F1.7 / landmine #4: with the mirror set, the paint is
  default text, default style, and no link affordance.
- `the_in_cell_editor_does_not_inherit_link_appearance` — the discriminating pair: the same cell
  paints underlined while `visible_styles` holds nothing for it, so `resolve_incell_font` cannot see
  the synthesis.
- `a_plain_click_on_a_link_cell_emits_open_link` — the full mouse-down → mouse-up path, with the
  cell-centre helper pinned against `cell_at_local` so the click really lands where it claims.
- `the_gestures_that_do_not_open_a_link` — F2.2's table in one test, each arm also asserting what
  *did* happen instead: a drag (selection), a same-cell sweep past the slop, ⌘/Ctrl-click,
  shift-click (never even armed), a double click (in-cell editor), and a click on an unlinked cell.
- `point_mode_click_on_a_link_cell_inserts_a_reference_and_does_not_open_it` — landmine 10.
- `a_click_on_a_merged_region_opens_the_anchors_link` — F1.5's click half, pressed on a **covered**
  cell.
- `a_pending_link_never_survives_the_press_that_armed_it` — an unrelated second press clears it, so a
  later release cannot pick up a stale link.
- `link_decorations_are_unique_and_carry_the_tooltip` — the tooltip text, and id uniqueness across
  quadrants, rows, and columns (including the packing edge where a max column must not collide with
  the next row).

**`freecell-app` (`shell/app.rs`, gpui)** — the window integration tests live beside the other
worker-less window tests, which is where `loaded_window` + the injectable seams are.

- `an_http_link_opens_and_a_javascript_link_does_not` — asserts the real `cx.opened_url()`: the
  allowed link reaches the platform opener and pushes no message; the refused one launches nothing,
  pushes exactly one notification, and puts up no dialog.
- `a_file_link_asks_before_opening` — the confirm names the path and launches nothing; Cancel
  launches nothing; **Open File** launches it.
- `a_degraded_window_opens_nothing` — D2.
- `an_internal_link_moves_the_selection_and_switches_sheets` — same-sheet, then a quoted +
  case-insensitively named other sheet; nothing is ever launched.
- `an_unresolvable_internal_link_navigates_nowhere` — a resolvable control moves first, then an
  unknown sheet / a defined name / a malformed ref each leave the selection put and say so.

**`render-tests`**

- The seven `link_*` cases build and render through the real worker (`render_tests.sh test link_`).
  Baseline generation is Phase 7's.

### What the subset run actually showed

`render_tests.sh test link_` (11 min; it captures every grid case — see *Found while building*) ended
`0 passed; 7 failed`, each failure being **"no committed baseline"** and nothing else. That is the
expected state for this phase: Phase 7 owns generation, eyeballing, and the commit. No pre-existing
case moved (the non-link cases were captured but not diffed, and the change is behaviour-preserving
for an unlinked cell — `cell_element` gains no id and no cursor without a `LinkDecoration`).

The seven captures in `target/render-actual/` were eyeballed here and each shows what its case is for:

| case | what the capture shows |
|---|---|
| `link_plain` | "Sample" blue + underlined — the **engine's** formatting, no FreeCell styling |
| `link_dynamic` | the same look on a `HYPERLINK()` cell — F1.3's synthesis, indistinguishable from the row above at cell size (D6) |
| `link_spill` | the long URL painted blue + underlined **across** C2/D2, not stopping at B2's edge (F1.5) |
| `link_in_merged_region` | the B3:D4 box painted once, blue + underlined from the anchor's link (F1.5) |
| `link_over_fill` | the yellow fill intact under blue + underlined text (link colour is text-only) |
| `link_with_explicit_font_color` | **red** and still underlined — the explicit colour beat the synthesized blue (F1.4) |
| `link_removed` | still blue + underlined after the link was removed (F1.6) |

## Found while building

- **A name-filtered render run is NOT a cheap subset for grid cases.** `render_suite.rs` renders
  every grid case once behind a `OnceLock` (`render_all(bin, &out, None)` — an unconditional
  `None` filter) and the `#[test]` name filter narrows only which captures get *diffed*. So
  `render_tests.sh test link_` still pays the full ~170-case capture; only `chart_` is genuinely
  cheap, because the chart scenes sit behind their own `CHART_RENDERED` lock. `CLAUDE.md`'s
  "subset while iterating is fast" is therefore true for charts and wrong for grid cases. It is
  fixable (~5 lines: the wrapper exports its filter, `rendered()` forwards it to `render_all`'s
  `only` parameter — a missing capture already fails loudly, so it cannot mask a case), but that is
  render-harness work, not hyperlinks. **Carried to Phase 6** as a `PROJECTS.md` entry, and worth
  knowing before Phase 7 budgets its time.
- **`render_cases!` and `cases::all()` drift is invisible under a filter.** `case_names_match_table`
  guards them, but a name-filtered run filters the guard out too — a case added to the table without
  a `render_cases!` row silently reports "0 tests ran" rather than failing. (Hit exactly this on the
  first `test link_` run.)

## Carried forward

- **Defined-name internal links do not resolve** (D5). `parse_internal_location` classifies them and
  navigation reports them unresolvable, because FreeCell models no defined names anywhere. A genuine
  divergence from Excel → a `GAPS.md` row in **Phase 6**.
- **The two-write-lock transient is settled, not fixed** (D4). No action needed; the analysis and the
  available fix are recorded at the `visible_links` snapshot site and in D4 above, so a future reader
  does not redo it.
- **The `render_all` filter passthrough** (see *Found while building*) — a `PROJECTS.md` entry for
  Phase 6.
- **The two link blues** (D6) — a candidate `GAPS.md`/`PROJECTS.md` row for Phase 6: matching the
  engine's theme-resolved hyperlink colour needs a build-time scalar on `SheetCache`.
