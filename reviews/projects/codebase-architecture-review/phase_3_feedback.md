# Phase 3: UI architecture (GPUI)

Scope: `app/crates/freecell-app/src/{grid,chrome,shell}`, `lib.rs`, `main.rs`. Charts reviewed
only where they meet the grid. Read-only review — nothing built or run.

**The real entity graph** (not the one the docs describe):

```
FreeCellApp (Global)  ──owns──► Vec<AppWindow> ──► Entity<WorkbookWindow>
                                                        │
                        ┌───────────────────────────────┴────────────────┐
                        │                                                │
                 Entity<ChromeView> ──renders as body──► Entity<GridView>
                        │  ▲                                   │  ▲
                        │  └──── ChromeGridSink closure ───────┘  │
                        └────────── GridEventSink closure ────────┘
                                    (both resolved through
                                     Rc<OnceCell<WeakEntity<…>>>,
                                     most calls window.defer'd)
                        │                                     │
                        └──► Rc<dyn ChromeClient> ◄───────────┘
                                    (DocumentClient → worker thread)
                        ▲
                 Rc<SinkShared>  (Cell<active_sheet>, Cell<last_selection>,
                                  RefCell<ClipboardCoordinator>, edge seq)
```

It is a **cycle**, not a tree: chrome→grid and grid→chrome, plus a shared mutable side-channel
(`SinkShared`) that exists purely because neither sink may touch the window entity. Ownership
is also inverted from what you'd expect — the *chrome* owns the grid as a child view, while the
*window* owns both and mediates all traffic between them.

---

## What's Good

- **[grid/layout.rs:1]** **The pure-geometry seam is genuinely well drawn.** 677 production
  lines of scroll/clamp/hit-test/reveal/scrollbar/spill math with zero gpui and zero engine
  imports, backed by 31 plain `#[test]`s. `PaneGeometry`, `hit_test`, `scroll_to_reveal`,
  `spill_span`, `edge_autoscroll_delta` are all callable from a headless test. This is the
  right instinct and it's executed properly.

- **[grid/input.rs:52]** **`command_for_key` is a pure keymap.** Keystroke → `GridKeyCommand`
  with the platform modifier already resolved to a `secondary: bool` by the caller. 10 plain
  tests, no gpui. Adding a shortcut is a one-line change in one testable function. Contrast
  this with how mouse modality is handled (see M2) — the same discipline was not applied.

- **[grid/chart_layer.rs:26]** **The chart↔grid boundary is the best-factored seam in the UI.**
  A minimal `GridGeometry` trait (4 methods) that the view implements over its per-frame
  `Frame`; `anchor_rect` / `rect_to_anchor` / culling are pure and unit-tested without gpui;
  the *presentation* decision (faithful / degraded badge / unsupported placeholder) lives in
  `chart/in_grid.rs`, not in the grid. `grid/view.rs:4022` scans only the tiny `ChartPlacement`
  structs and materializes the heavy `Chart` for the on-screen few. This is how the other
  cross-cutting features should have been built.

- **[grid/view.rs:1250]** **Virtualization is real and the render path makes zero engine calls.**
  `resolve_frame` answers from `Axis` prefix sums, never iterates the sheet, and snapshots
  visible styles/borders/fonts under a briefly-held read lock that is released before any
  painting. `AxisPreview` (`grid/view.rs:471`) applies a live resize as an O(1)-per-track delta
  rather than rebuilding an axis — that's the correct trick for Excel-max, and it's documented.
  `build_grid_layers` iterates only `quad.rows × quad.cols`. Nothing walks 1M rows.

- **[worker/client.rs:29]** **Async discipline is sound.** The engine runs on its own 64 MiB
  thread; the UI reads an `ArcSwap<Publication>` wait-free per frame and a `parking_lot::RwLock`
  cache with short critical sections. No `block_on`, no channel `recv()`, no `std::fs` on the
  render or input path (the only `fs` calls are in save/backup and demo staging, both off the
  paint path). I found exactly one write-lock from the UI thread (`grid/view.rs:4482`) and it is
  a render-test-only hook, clearly labelled.

- **[shell/lifecycle.rs, shell/registry.rs]** **Window lifecycle decisions are pure.** Dedupe,
  quit ordering, dirty accounting, save targeting, `.xlsx` enforcement, overscan — all in
  gpui-free modules with 27 plain tests, with `shell/app.rs` as thin plumbing that performs
  them. `shell/app.rs` is 850 production lines for a lot of surface area precisely because the
  decisions live elsewhere. This is the model `chrome/view.rs` should have followed.

- **[chrome/client.rs:19]** **`ChromeClient` is a properly narrow port.** 8 read methods plus
  `send`, with a `RecordingClient` double. The chrome can be driven end-to-end with no engine.

- **freecell-core carries real reducers**, not just types: `data_row` (509 lines),
  `eval_indicator`, `selection` (1,203), `format_ui` (770), `find`, `merge`, `input_cap`. The
  layering rule (core = no gpui, no ironcalc) is enforced and holds.

---

## Critical (must fix)

- **[chrome/view.rs:330] `ChromeView` is a god object, and it is the single biggest liability
  in the UI.**

  Hard numbers, production code only (test modules begin at line 8249, so the split is 8,248
  production / 7,851 test):

  | metric | value |
  |---|---|
  | production lines | 8,248 |
  | production methods on `ChromeView` | 267 |
  | struct fields | 71 (17 `bool`, 20 `Option`, 15 `Entity<…>`) |
  | `render_*` methods | 35 |
  | `cx.listener` closures | 91 |
  | largest method | `render_action_row`, 407 lines |

  Method mass by feature domain (heuristic classification of the 267 methods):

  | domain | methods | lines |
  |---|---|---|
  | conditional formatting (list + editor + colour scales) | 53 | ~1,510 |
  | formatting / fonts / borders / fills / number formats | 62 | ~1,450 |
  | sheet tabs (select, rename, delete, reorder-drag, menu) | 36 | ~1,350 |
  | edit / formula bar / in-cell / autocomplete / point-mode | 38 | ~990 |
  | charts (insert menu + edit panel) | 29 | ~790 |
  | find / replace | 12 | ~155 |
  | selection stats | 6 | ~110 |
  | shell (`new`, `render`, worker-event fold, overlays, scrollers) | 31 | ~1,700 |

  That is **eight independent products** in one GPUI entity, sharing one 71-field struct and one
  `render`. A new engineer asked "where is the formula bar handled?" has to know that
  `render_data_row` is at 4558, the reducer bridge (`on_content_event`, `apply_data_effects`,
  `sync_input_from_reducer`) is at 1776–1868, the commit path is at 1102, the autocomplete is at
  1421–1578, the cap popover is at 5339, and the worker replies land at 1683 — six disjoint
  regions of one file, none adjacent, with 1,500 lines of conditional-formatting code sitting
  between two of them. Any two of these eight features being edited concurrently is a merge
  conflict in the same struct and the same `render`.

  There is no technical reason for this. Each domain already *has* a natural boundary: CF has
  `chrome/cond_fmt.rs` (state) but its 53 methods and 1,500 lines of UI stayed in `view.rs`;
  charts have `ChartPanel` but the same story; the find bar has 11 fields on `ChromeView` and no
  module at all. **Direction:** each domain becomes its own entity or its own `impl` module with
  its own state struct, communicating with the chrome shell through a small typed interface —
  exactly what `chrome/h_scroller.rs` and `chrome/sidebar.rs` already demonstrate at small
  scale. `ChromeView` should retain layout composition, focus, and routing, and nothing else.
  This is a multi-week refactor and it gets more expensive every phase; it should be scheduled
  now, not "when we get to it."

- **[shell/window.rs:47, grid/view.rs:282, chrome/view.rs:339] There is no single source of
  truth for selection, active sheet, or the pending edit — each is mirrored across three or
  four owners and reconciled by hand.**

  - **Selection**: authoritative in `GridView.selection: HashMap<SheetId, SelectionModel>`
    (grid/view.rs:282), mirrored in `ChromeView.selection` (chrome/view.rs:339), mirrored again
    in `SinkShared.last_selection: Cell<SelectionModel>` (shell/window.rs:55) as a rollback
    buffer. Three copies, kept in sync by `route_selection_changed` (shell/window.rs:1559),
    which must decide whether the chrome accepted the change and, if not, `window.defer` a
    write back into the grid.
  - **Active sheet**: `GridView.active_sheet`, `ChromeView.active_sheet`,
    `SinkShared.active_sheet`, plus `WorkbookWindow.sheets`. Four places, synchronised by the
    47-line ritual in `switch_grid_to_sheet` (shell/window.rs:2091) whose ordering is
    load-bearing and documented as such.
  - **Pending edit**: the text exists simultaneously as the `DataRow` reducer's buffer, the
    `content_input: Entity<InputState>` widget buffer, the `EditController`'s in-cell
    `InputState`, and `GridView.mirror` — four representations of one string, kept coherent by a
    `syncing: bool` re-entrancy guard (chrome/edit.rs:59) and a manual push.
  - **Ten `GridView` fields are pure mirrors of chrome state**: `mirror`, `incell_open`,
    `incell_cap`, `incell_autocomplete`, `incell_sig_hint`, `quick_edit`, `reference_ready`,
    `pending_ref`, `ref_highlights`, `incell_input`. All are written by one 9-positional-argument
    setter, `set_edit_state` (grid/view.rs:947).

  This is not a theoretical concern. The codebase already documents a **data-corruption hazard**
  produced by it, at shell/window.rs:2129-2136: after a cap-rejected edit during a worker-driven
  sheet delete, "the field is left Editing the old text with `active_sheet` now the new sheet, so
  a later trim+commit of that same edit would land on the new sheet." The comment argues this is
  acceptable because the alternative was a panic. It is a symptom: when the same fact lives in
  four places, no amount of careful ordering makes every interleaving safe.

  **Direction:** hoist the shared facts (`active_sheet`, `selection`, the pending edit) into one
  observed document-view-model entity that both the grid and the chrome *read*, and have each
  mutation go through it. The mirrors then become derived reads, and `SinkShared`,
  `set_edit_state`, and most of the defer dance disappear with them.

- **[shell/window.rs:246] The grid↔chrome cycle is wired through `Rc<OnceCell<WeakEntity<…>>>`
  slots and made correct by remembering to `window.defer`.**

  `build` constructs the grid, then the chrome, then back-patches two one-shot slots so each
  sink can find its sibling. Every sink handler that reaches the sibling must defer, because the
  handler runs *inside* the sibling's `update` and a synchronous re-entry aborts on gpui's
  `entity_map` re-entrancy check. The code carries five separate "BUG #5" comments explaining
  this (shell/window.rs:1982, 1994, 2002, 2029, and the counter-case at 300). `FocusGrid`,
  `MoveActive`, `SelectAndReveal`, and `SetActiveSheet` are all deferred; `set_edit_state` is
  not. Whether a given call needs a defer is decided per-call-site by reasoning about who might
  be leased — there is no type or structure that enforces it.

  The failure mode is a hard abort at runtime, discoverable only by exercising the exact
  interleaving. Every new cross-entity call is a fresh coin flip, and the pixel/gpui test suite
  will only catch the paths it happens to drive.

  **Direction:** the cycle is avoidable. If both views read a shared model entity (previous
  finding) and emit upward-only events to the window, there is no sibling handle to hold, no
  weak slot to back-patch, and no re-entrancy class to reason about. Failing that, at minimum
  make deferral structural — a single `post_to_sibling(…)` helper that always defers — rather
  than a convention documented in comments.

---

## Moderate (should fix)

- **[grid/view.rs:274] `GridView` is heading the same way as `ChromeView`, and layout is not
  actually separated from painting.** 6,575 production lines, 134 methods, 45 fields. The
  hot path is three very large methods: `build_quadrant` (3419, 502 lines), `build_grid_layers`
  (3921, 425), `render` (6210, 261), plus `resolve_frame` (1250, 246). `grid/layout.rs` holds
  only the *axis* math; the frame-level geometry that decides where anything lands —`Frame`,
  `Quadrant`, `AxisPreview`, `cell_rect`, `span_rect`, `fill_handle_square`,
  `incell_editor_size`, `incell_input_geometry` — lives in `grid/view.rs`, interleaved with the
  element construction that consumes it. So the freeze-pane quadrant split, the merge/spill
  resolution, and the wrap auto-grow signature logic can only be tested through a gpui
  `TestAppContext`. **Direction:** move `Frame`/`Quadrant`/`AxisPreview` and the per-cell rect
  math into `layout.rs` (they have no gpui dependency today except by residence), leaving
  `build_quadrant` to turn a resolved rect list into elements.

- **[grid/view.rs:1705, chrome/view.rs:330] Modality is a pile of `Option`s and `bool`s where
  the domain is an enum.** `GridView` carries six mutually-exclusive pointer states as
  independent fields — `drag`, `resize_drag`, `resize_preview`, `chart_drag`, `fill_drag`,
  `point_drag` — plus three independent menu `Option`s (`header_menu`, `chart_menu`,
  `cell_menu`) and 7 `bool`s. `handle_mouse_down` (1705) opens with a manual guard chain
  (`if self.resize_drag.is_some() || self.fill_drag.is_some() || self.point_drag.is_some()`),
  and the fill-handle hit test 50 lines later re-derives an overlapping but *different* guard
  (`incell_open.is_none() && drag.is_none() && resize_drag.is_none() && chart_drag.is_none()`) —
  the same invariant expressed twice, inconsistently. `ChromeView` has the mirror problem: eight
  independent `*_open: bool` popover flags plus two dock panels, where the invariant is "at most
  one is open." Nothing enforces it; it holds only because each popover paints a full-size
  occluding backdrop that swallows the click that would open a second. **Direction:**
  `enum PointerMode { Idle, SelectDrag(…), Resize(…), ChartDrag(…), Fill(…), Point(…) }` and
  `Option<OpenPanel>` on the chrome. The guard chains collapse to `match`, and the invariants
  become unrepresentable-if-violated.

- **[grid/view.rs:3419] Per-frame allocation in the per-cell loop.** For every visible cell:
  a `String` (`pc.display_text.clone()` or `String::new()`), an `AnyElement` box, and up to two
  more allocations for spilled/wrapped cells. `resolve_frame` additionally rebuilds
  `visible_font_families` (a `Vec<SharedString>` with a `String` per family, allocated fresh
  every frame at grid/view.rs:1373), `visible_border_specs` (`to_vec()`), and `visible_merges`
  each frame. Worse, `visible_merges` is **linear-scanned three times per cell** inside the
  double loop (3443 for the skip test, then twice via the `same_fill` closure at 3462) — so the
  cell loop is O(visible_cells × visible_merges), not O(visible_cells). On a merge-heavy sheet
  that is the dominant term. **Direction:** index `visible_merges` by row (or precompute a
  per-cell covered bitmap once per frame), and pass `&str`/`SharedString` through to
  `cell_element` instead of an owned `String`.

- **[grid/view.rs:5117, freecell-core/src/perf.rs:113] The perf gate does not measure the render
  strategy the app actually uses.** `FrameSample::frame_render_ns` is honestly documented as
  "excludes gpui layout/shape/present", and `measure_frame` times exactly `resolve_frame` +
  `build_grid_layers`. But the grid paints **one `div` per visible cell** (`cell_element`,
  grid/view.rs:5353) — there is no custom `Element` with manual `request_layout`/`paint` anywhere
  in `grid/` (the only hand-written `paint` impls in the crate are the six chart widgets). So the
  cost of handing ~2,000 absolutely-positioned styled divs to taffy and the painter every frame
  — plausibly the largest single term — is the one thing the 8.33 ms gate never sees. For a
  project whose stated premise is "stupid-fast on huge sheets," measuring only the construction
  half of the frame is a blind spot worth closing before more is built on top. **Direction:**
  add an end-to-end frame-time measurement (window-level, not build-level), even a coarse one, so
  the div-per-cell decision is validated rather than assumed.

- **[chrome/view.rs:3772] The chrome takes a lock and clones an engine-cache `Vec` during
  `render`.** `merge_active()` (3779) and `merge_disabled()` (3787) each call
  `active_sheet_merges()` → `ChromeClient::sheet_merges` → acquire the shared cache read lock and
  `cache.merges().to_vec()`. Both are called from `render_action_row`, so every chrome repaint
  does two lock acquisitions and two full clones of the sheet's merge list on the same `RwLock`
  the grid reads per frame. The doc comment at chrome/client.rs:135 says "the toggle reads at
  render / click time, not per frame" — but render *is* per frame. Every other action-row toggle
  correctly reads a value cached at selection-change time (`active_style`, `active_num_fmt`,
  `active_font_family`); merge is the one that didn't follow the pattern. **Direction:** cache
  the merge-derived booleans alongside `active_style`, refreshed on selection change and
  `StyleCacheUpdated`.

- **[chrome/view.rs:5251] The chrome has grown a private, unabstracted popover/menu/modal
  framework.** gpui-component is used only for atomic controls (`Button`, `Input`, `Spinner`,
  `ColorPicker`, `Checkbox` — 5 imports total across the crate). Every compositional surface is
  hand-rolled: 35 `render_*` methods; z-order determined by push order into a `Vec<AnyElement>`
  (with a comment at 5251 explaining that the chart panel must be pushed first or the chart menu
  falls behind it); anchoring via seven `canvas` bounds probes writing into a fixed
  `anchor_x: [f32; 7]` array (chrome/view.rs:4123); a hand-written `backdrop()` per popover;
  `render_borders_popover` alone is 229 lines. The module doc justifies this as "this is chrome —
  don't over-invest," but the investment happened anyway — it just went into eleven one-off
  implementations instead of one reusable `popover(anchor, content)` helper. `chrome/sidebar.rs`
  shows the extraction is straightforward when someone does it. **Direction:** extract one
  anchored-overlay primitive (anchor probe + backdrop + z-slot) and migrate the eleven call
  sites; the `anchor_x` array and the manual ordering comment both disappear.

- **[chrome/view.rs:7573-8248] ~670 lines of pure, gpui-free logic live in a view file.**
  `cf_validate`, `cf_build_spec`, `cf_state_from_spec`, `cf_stops_from_colors`,
  `cf_row_controls`, `cf_rule_intersects_selection`, and a dozen label mappers are free functions
  in `chrome/view.rs`. They have no gpui dependency and belong in `freecell-core::cond_fmt`
  (which exists, 339 lines) — where the existing CF types already live. The tell: the only 9
  plain `#[test]`s in a 16k-line file are the ones testing these functions. Meanwhile the file
  carries **245 `#[gpui::test]`s** to test logic that is fused into view code. **Direction:** move
  them; the ratio of plain-to-gpui tests is the metric to watch.

- **[grid/view.rs:947, chrome/mod.rs:106] The chrome→grid edit seam is a state dump, not a
  protocol.** `set_edit_state` takes **nine positional parameters** including three adjacent
  `Option`s and two adjacent `bool`s — call sites read
  `set_edit_state(None, Some(cell), None, true, None, None, false, false, vec![])`, which is
  unreviewable and one argument-order slip away from a silent bug. The matching
  `ChromeGridRequest::EditState` variant has nine fields and is rebuilt and pushed wholesale on
  *every* edit transition (including a `Vec<(CellRange, u8)>` allocation per keystroke). Every
  formula feature added so far (autocomplete, signature hints, point-mode, quick-edit) widened
  this one message and added a mirror field to `GridView`. **Direction:** pass a single
  `EditOverlayState` struct (the variant's payload already is one — just hand it through), or
  better, have the grid read the state from a shared model rather than being pushed it.

- **[shell/window.rs:87, chrome/view.rs:80, shell/welcome.rs:28, shell/about.rs:26,
  shell/titlebar.rs:36] There is no theme layer; colours are hardcoded `u32` literals duplicated
  across files.** `HAIRLINE = 0xD9D9D9` is defined independently in five files; `MUTED_TEXT =
  0x555555` in five; `TEXT = 0x1F1F1F` in four; `DANGER`, `CARD_BG`, accent blues similarly.
  gpui-component's theme is deliberately bypassed (`chart/style.rs:4` documents this for chart
  captures, and `grid/mod.rs:48` explains the selection blue is pinned because the theme's
  `primary` is neutral). The one place a light/dark seam exists — `ref_slot_border(slot,
  is_dark)` at grid/view.rs:5653 — is called only with `false` and is annotated as a future
  hook. Dark mode, high-contrast, or any brand change is currently an N-file literal hunt.
  **Direction:** one token module (even a plain `struct Palette` of `u32`s threaded from the
  shell) before the surface area grows further.

- **[grid/mod.rs:106] `GridEvent` has 35 variants and is becoming an untyped command bus.**
  It now carries selection, viewport, clipboard, fill, structural row/col ops, freeze, chart
  anchor/delete/select, autocomplete navigation, and formula reference insertion. `make_grid_sink`
  (shell/window.rs:1660) is a 300-line `match` where roughly half the arms are one-line
  `client.send(Command::X)` translations — the grid is effectively naming worker commands through
  an intermediate enum. **Direction:** split into `GridEvent` (things the *view* did, which the
  owner interprets) and a direct pass-through for the arms that are pure command translation, or
  let the grid hold the client for the fire-and-forget structural commands.

---

## Mild (consider fixing)

- **[grid/view.rs:287-292, 1184-1240, 4457]** Render-test hooks are production state:
  `force_scrollbars` and `freeze_spinner` are `GridView` fields that branch `render`, and
  `arm_fill_drag_preview` / `arm_point_drag_preview` / `autogrow_measure_now` are `pub` methods
  that write internal drag state directly. They are clearly labelled, but they mean the shipped
  render path contains capture-harness branches. Consider a `#[cfg(feature = "render-hooks")]`
  gate or a separate wrapper view.

- **[chrome/view.rs:1683, shell/window.rs:460]** Worker events are folded in two places with a
  hand-maintained partition: the window handles lifecycle/modal/dirty events and *forwards* six
  variants to the chrome (shell/window.rs:527), while `ChromeView::on_worker_event` matches those
  plus a couple more and swallows the rest with `_ => {}`. A new `WorkerEvent` that the chrome
  needs requires edits in both, and forgetting the forward fails silently. A single typed
  fan-out (or an exhaustive match on both sides) would make the partition checkable.

- **[chrome/view.rs:560]** `ChromeView::new` is 187 lines and takes six parameters, constructing
  15 `Entity<InputState>`/`ColorPickerState` children and their subscriptions inline. It will
  keep growing with each feature; it is the constructor equivalent of the god-object problem.

- **[grid/view.rs:5892, chrome/view.rs:4116]** Both view files split their `impl` blocks around
  the `Render` impl (logic block, free functions, `Render`, then a *second* logic block). The
  second block is not a meaningful grouping — `grid/view.rs`'s post-`Render` block holds
  `resize_hotspots`, menu builders, and `autogrow_measure_now`; `chrome/view.rs`'s holds the
  find bar's *logic* interleaved with its rendering. It makes navigation worse for no benefit.

- **[chrome/view.rs:4151]** `render_action_row` is 407 lines of a single hardcoded `.child()`
  chain with three inline builder closures. Adding one toolbar control touches: a field on the
  71-field struct, an `Anchor` enum variant + `ANCHOR_COUNT`, a `.child()` insertion in the
  chain, a `toggle_*` method, a `render_*_popover` method, an entry in `render_overlays`, and a
  `_active`/`_enabled` accessor. That fan-out is accidental, not inherent — a declarative
  descriptor list (`&[ToolbarItem]`) would collapse most of it.

- **IME**: no explicit IME handling anywhere in the app layer. Text entry rides gpui-component's
  `Input`, which presumably handles it, but the grid's `capture_key_down` (grid/view.rs:6360)
  intercepts keys ahead of the input by raw `keystroke.key` string match ("tab", "escape",
  arrows) with no composition check. Worth verifying that an active IME composition can't have
  its arrow/Enter keys stolen by the quick-edit branch.

---

## Phase Summary

**Counts:** Critical 3 · Moderate 10 · Mild 6.

The good news first, because it is real: the *hard* parts of this UI are done well. Grid
virtualization is genuine and never touches the engine on the render path; the pure-geometry
(`grid/layout.rs`), pure-keymap (`grid/input.rs`), and chart-anchor (`grid/chart_layer.rs`) seams
are correctly drawn and headlessly tested; the worker threading is clean. Someone understood the
performance-critical architecture and built it properly.

The problem is everything wrapped around it. **`chrome/view.rs` is 16,099 lines — 8,248 of them
production — holding 267 methods, 71 fields, and eight unrelated feature domains in one entity.**
It is not a file that is merely large; it is a file where conditional formatting, sheet tabs,
find/replace, charts, fonts, borders, and the formula bar all share one struct and one `render`,
and where 670 lines of gpui-free logic sit because there was nowhere obvious to put them. The
same pattern is 60% of the way through `grid/view.rs`.

The finding I'd act on first, though, is the **absence of a single source of truth**. Selection
lives in three places, the active sheet in four, the pending edit text in four, and ten `GridView`
fields are hand-pushed mirrors of chrome state through a nine-positional-argument setter. This
forced the cyclic `OnceCell<WeakEntity>` wiring and the `window.defer` discipline that five
"BUG #5" comments document, and it has already produced a **self-documented wrong-sheet-write
hazard** at `shell/window.rs:2129`. That is not a code smell; that is a correctness architecture
that only holds because nobody has hit the wrong interleaving yet. Introducing one shared
document-view-model that both views read would dissolve the mirrors, the `SinkShared`
side-channel, the fat `EditState` message, and most of the re-entrancy class — and it gets
strictly harder every phase that adds another mirrored field.
