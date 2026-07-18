---
status: complete
---

# Conditional Formatting

Add **conditional formatting** to FreeCell: rules that restyle cells based on their computed
values (highlight cells, color scales, data bars, icon sets), authored through a new sidebar.

## Source capability

The engine work is **already done upstream**. IronCalc's conditional-formatting feature
([ironcalc/IronCalc#951](https://github.com/ironcalc/IronCalc/pull/951)) is merged and is
**already present in our fork's `freecell-fixes` branch** — the branch FreeCell's `app/Cargo.toml`
pins. Verified present: `base/src/cf_types.rs`, `base/src/conditional_formatting.rs`, and
`base/src/user_model/conditional_formatting.rs`. **No fork/upstreaming work is needed** for this
project; it is a pure FreeCell-side integration (engine wrapper → worker protocol → sidebar UI →
grid rendering).

## UX guidance (from the requester)

- **New action-bar button** for conditional formatting (lucide **`split`** icon). Opens a sidebar.
- **New "Conditional Formatting" sidebar**, like the existing chart edit panel. **Extract the
  sidebar container** (docked card + title + close button) from the chart panel into a **reusable
  component**, then build the CF sidebar on top of it.
- **Sidebar content:** a first pass, to be reviewed with feedback (iterative).

## What the engine gives us (already available)

`UserModel` (wrapped by `freecell-engine`) exposes the full CF API:

- **Mutate:** `add_conditional_formatting(sheet, range, CfRuleInput)`,
  `update_conditional_formatting(...)`, `delete_conditional_formatting(...)`,
  `raise_/lower_conditional_formatting_priority(...)` — all undo/redo-aware.
- **Read rules:** `get_conditional_formatting_list(sheet) -> Vec<ConditionalFormattingView>`,
  `get_dxf_for_conditional_formatting(...)`.
- **Read effective style for rendering:** `get_extended_cell_style(sheet, row, col) ->
  ExtendedStyle { style, icon, data_bar, rating }` — the base style with the winning CF overlay
  already folded into `style` (fill/font/border/color-scale), plus separate decoration fields for
  data bars, icon sets, and ratings.

The ~18 rule types fall into **4 visual families**:

1. **Highlight (classic dxf) rules** — CellIs (>, <, between, =, …), Text (contains/begins/ends),
   TimePeriod, Duplicate/Unique, Blanks/Errors, Above/BelowAverage, Top/Bottom-10. Each carries a
   differential format (`Dxf`: fill/font/border). Folds into `ExtendedStyle.style` → rides
   FreeCell's **existing** fill/font/border render path.
2. **Color scales** — 2/3-stop interpolated fills. Also folds into `ExtendedStyle.style.fill` →
   existing render path.
3. **Data bars** — in-cell horizontal bars (`ExtendedStyle.data_bar`) → **new** grid-draw
   primitive.
4. **Icon sets & ratings** — in-cell glyphs (`ExtendedStyle.icon` / `.rating`) → **new** grid-draw
   primitive.

## Notable integration facts

- **Value-dependent rendering.** FreeCell's render path is currently static (style resolved from a
  cell's stored `Style` only; values arrive on a separate channel). CF must be recomputed when
  values change, so the effective (extended) style is computed **worker-side** and folded into the
  engine-free render cache, invalidated on value publications, not only on style edits.
- **Sidebar must stay open while picking ranges.** The chart panel auto-closes on any grid
  selection change; the CF sidebar must **not** — the user needs to keep selecting cells/ranges
  while it is open.
- **Persistence** rides IronCalc's native xlsx writer (CF is part of the worksheet model), unlike
  charts which need special save handling.
