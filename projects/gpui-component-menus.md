---
status: Future
---

# Adopt gpui-component menus app-wide (native flyout submenus)

**Status:** Future. Deferred from `gaps_closing_7_12` Phase 10.4 (2026-07-13).

## Why

FreeCell's chrome hand-rolls **seven** toolbar/popover menus as custom `div` cards:
`div().absolute().top(ACTION_ROW_H).left(anchor_x).occlude()` over a `backdrop()` dismiss
layer, each driven by its own `anchored_trigger`/`anchor_x` state (fill, text-color, borders,
font-family, font-size, chart, number-format — all in `chrome/view.rs`). They work and look
consistent *with each other*, but they:

- have **no flyout/submenu** support (Phase 10.1's number-format "More ▸" had to ship as a
  **drill-in** — swap the card body + a "◂ Back" row — because a flyout would need a second
  card anchored to the dynamically-positioned "More ▸" row);
- re-implement anchoring, occlude, dismiss, and keyboard handling by hand, seven times.

gpui-component (already vendored) ships a full menu system in `crates/ui/src/menu/`:
`PopupMenu` with **native flyout submenus** (`PopupMenu::submenu(label, window, cx, f)` →
`PopupMenuItem::Submenu { menu: Entity<PopupMenu>, submenu_anchor, parent_menu, … }`),
`Popover`, `DropdownMenu`, `ContextMenu`, keyboard nav, and consistent anchoring/dismiss. Items
accept plain closures (`PopupMenuItem::new(label).on_click(...)`) as well as `Action`s, so the
existing apply paths (e.g. `apply_num_fmt(code)`) drop in.

## Scope

Migrate the seven hand-rolled popovers (and the sheet-tab context menu) to gpui-component
`PopupMenu`/`Popover` so they share one anchoring/dismiss/submenu/keyboard model, then use a
**flyout** for the number-format "More ▸" (replacing the Phase 10.1 drill-in).

## Why it wasn't done in Phase 10

- **`scrollable` ⊗ submenu are mutually exclusive** in `PopupMenu` (`popup_menu.rs`: "If this
  is true, the sub-menus … cannot be support[ed]"), and the number-format card is deliberately
  `max_h(320) + overflow_y_scroll` for the long grouped inventory. A flyout there means moving
  the scroll onto the submenu entity — workable, but exactly the constraint the drill-in avoided.
- Converting **one** popover makes it the app's **only** gpui-component menu, so it would
  anchor/dismiss/animate differently from the six identical siblings sitting next to it in the
  same toolbar. The value is in converting **all** of them at once — an app-wide change, not a
  one-popover tweak.

## Payoff

Native flyout submenus everywhere (num-fmt More, and future nested menus), consistent
anchoring/dismiss/keyboard, and ~seven hand-rolled cards deleted in favor of one library
paradigm.
