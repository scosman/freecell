# Research: function autocomplete + signature hints seams (2026-07-16)

Codebase findings feeding the functional spec + architecture. Paths are `app/crates/…`
unless noted. Feature is scoped + unstarted (GAPS.md:364): no completion logic, no
function list, no lexer usage in app crates today.

## Editing surfaces — two widgets, one pending edit

- Two `gpui_component::input::InputState` widgets: data row `content_input`
  (`chrome/view.rs:325,505`) and in-cell `EditController.in_cell`
  (`chrome/edit.rs:37`, created `chrome/view.rs:506`). Both owned by **one**
  `ChromeView` entity; grid renders the in-cell overlay but doesn't own it
  (`grid/view.rs:306,731`).
- **Text + commit semantics** live in the pure reducer `freecell_core::data_row::DataRow`
  (text/committed/mode only — **no cursor/selection**; those live in the opaque
  `InputState`). Reducer events/effects `data_row.rs:58-93`, `reduce()` :156.
- The two widgets are mirrored byte-identical (`mirror_to_in_cell`
  `chrome/view.rs:1160-1172`; `syncing` guard `edit.rs:105-112`). `EditOrigin`
  (`edit.rs:22-28`) tracks which editor drives; popovers gate on it.
- **Per-keystroke hook:** `InputEvent::Change` in `on_content_event` (:1315) /
  `on_incell_event` (:1121) — the natural place to recompute candidates.
- In-repo `InputState` usage is only `.value()/.set_value()/.focus()`; `set_value`
  replaces the whole text and suppresses `Change` (`:1298,1969-1972`). **Open question:
  does the pinned gpui-component rev (`app/Cargo.toml:57`, SHA a9a7341c) expose
  caret-offset / insert-at-caret APIs?** Must be confirmed at architecture time;
  fallback = compute whole string + reposition caret (or accept caret-at-end).

## Popover machinery to reuse

- **(A) In-cell cap-error popover** — `grid/view.rs:4564-4574` inside
  `in_cell_overlay_elements` (:4459): absolute div anchored under the measured in-cell
  editor rect, `deferred()`. Best anchor template for the in-cell completion list.
- **(B) Data-row cap-error popover** — `render_cap_error_popover`
  `chrome/view.rs:4170-4185`, fixed anchor below the data row, gated on
  `EditOrigin::DataRow`. Best template for the formula-bar list.
- **(C) Number-format popover** — `render_num_fmt_popover` :4383 with `anchored_trigger`
  canvas probe :3003, full-screen `backdrop()` :4143, `.occlude()`, scrollable card.
  Richest pattern; likely overkill (a completion list shouldn't need a backdrop).
- Context menus (`cell_menu_elements` etc.) are mouse-only — **no existing popover has
  keyboard list navigation**; highlighted-index state is net-new (analogous to
  `num_fmt_open` flags on `ChromeView`).

## Function list — must be FreeCell-static

- IronCalc's 345-variant `Function` enum is **private** and non-enumerable from outside
  (`experiments/round-3/B-api-audit/src/formula_helpers.rs:19-25`; GAPS.md:364).
- Seed data committed: `experiments/round-2/03-function-parity/data/ironcalc_functions.csv`
  (345 registered names, count pinned by test) + `excel_functions_canonical.csv`
  (506 names with category/importance). **No argument-signature data exists** — arg
  templates must be authored (source from ECMA-376/Excel docs) for at least the common
  set.
- Only `freecell-engine` deps ironcalc; `freecell-core` is IronCalc-free by rule
  (`tests/dependency_rule.rs`). **Natural home for the static list + prefix matcher:
  `freecell-core`** (pure, headless-testable, beside `input_cap`).
- Formula detection = `input.starts_with('=')` (`input_cap.rs:80`). Lexer/Parser are
  public in ironcalc_base but only reachable via freecell-engine — NOT needed for
  name-prefix autocomplete; only for caret-in-which-arg tracking (richer hint).

## Keyboard interception (the hard seam — already solved twice)

gpui-component's Input binds arrows (and Enter/Tab) as actions that dispatch **before**
`on_key_down` listeners; the codebase preempts them:

- **Data row:** `cx.intercept_keystrokes` (`chrome/view.rs:562-588`) →
  `handle_data_row_edit_key` (:1062-1109) — handles Tab/quick-edit arrows, calls
  `cx.stop_propagation()` when consumed. **Extend here:** list open ⇒ Up/Down move
  highlight, Tab/Enter accept, Esc closes list (edit continues).
- **In-cell:** grid-root `capture_key_down` (`grid/view.rs:4748-4790`) intercepts
  Tab/Esc/quick-edit arrows before the input. **Add completion arms here** (guarded on
  list-open) with `stop_propagation`.
- Enter currently → `PressEnter` → `commit_and_move` (:1138-1148, 1330-1341); Esc →
  revert. When the list is open these must be intercepted first (accept / close-list).
- Grid→chrome routing for new events follows `GridEvent` (`grid/mod.rs:102-124`) →
  `shell/window.rs:1487-1509` → `ChromeView` pub fns.

## Open decisions for the spec

1. Caret-API availability in pinned gpui-component (template insertion UX depends on it).
2. Signature-hint depth: static arg-template line (cheap) vs caret-tracked current-arg
   bolding (needs tokenizer via engine) — recommend static template for this round.
3. Which names to list: the 345 engine-registered names (everything else would error
   anyway) — recommended; importance-ranked ordering from the canonical CSV.
