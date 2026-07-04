# MVP Deferred Behaviors — tracked gaps

Status: **Future (deferred from the MVP build, 2026-07-04).**

Single source of truth for the `functional_spec.md` behaviors that ship **deferred**
in the MVP. The MVP is a functional proof of concept (§1: "not design-polished"), and
none of these are calculation gaps — values, number formats, and error results all
render correctly. These are presentation / entry-point behaviors consciously deferred,
captured here so the information is not lost. Each row also appears in
`specs/projects/mvp/coverage_matrix.md` (the per-behavior map) and
`specs/projects/mvp/DECISIONS_TO_REVIEW.md`.

## Deferred functional-spec behaviors

| # | Behavior | Spec | Severity | Current MVP behavior | Root cause | Detailed home |
|---|----------|------|----------|----------------------|------------|---------------|
| 1 | **Type-based default cell alignment** — numbers/dates right, booleans/errors center | §3.6 | Moderate | Grid defaults **every** cell to left; *explicit* alignment still renders correctly | `PublishedCell` carries only a display string, no value type | [`type-aware-alignment.md`](type-aware-alignment.md) |
| 2 | **`[Red]` number-format text color** | §3.6 | Mild | `PublishedCell.text_color` always published `None`; negatives render default color | Worker doesn't publish resolved per-cell color | [`type-aware-alignment.md`](type-aware-alignment.md) |
| 3 | **Input-cap rejection message text** — "Formula too long / too deeply nested" popover | §3.3 | Mild | Danger **border** only; the *safety* behavior (reject, keep focus, cell unmodified) is implemented and tested — just no human-readable reason shown | `DataRowEffect::ShowCapError` is a no-op in the chrome; message-popover not built | *this file* (below) |
| 4 | **macOS Finder open-file** — double-click / `open -a` / drag-onto-Dock | §2.1 | Moderate | Only the **CLI-argv** open path is wired; the primary-platform "double-click a file" flow does not open it | Pinned gpui rev's `on_open_urls` callback lacks a context (`cx`) arg | *this file* (below) |
| 5 | **Bundled Inter font** — ship Inter via `add_fonts` at startup | §3.3/§3.6 | Nicety (not a functional gap) | App renders on the platform default font; render baselines pinned to the CI runner image | Fonts not vendored; `register_fonts` is a documented no-op | [`bundled-inter-font.md`](bundled-inter-font.md) |

### Detail for the two without a dedicated note

**#3 — Input-cap rejection message text (§3.3).**
Today an over-cap edit (formula length > 8192 chars or paren-depth > 64) is rejected at
both the `freecell-core::input_cap` validator and the worker-side re-check; the data row
shows a red danger border and keeps the user in edit mode with the cell unmodified
(tested: `chrome/view.rs::cap_reject_keeps_editing_and_flags_error`, plus
`input_cap.rs` unit tests). What's missing is the specced inline message-popover text
telling the user *why* the input bounced. Work when picked up: wire
`DataRowEffect::ShowCapError` to render a tooltip-style popover below the content field
with the reason string (length vs depth). Small, chrome-local change.

**#4 — macOS Finder open-file (§2.1).**
`main.rs` wires only `xlsx_arg` (CLI argv). Opening a `.xlsx` from Finder
(double-click, drag onto the Dock icon, `open -a FreeCell book.xlsx`) does nothing on
macOS — the primary design target. The pinned gpui rev's `on_open_urls` callback
signature lacks the `cx` needed to route the open through `FreeCellApp`. Work when
picked up: this is likely a **spike** first — check whether a newer gpui rev gives
`on_open_urls` a context arg (or an alternative hook), or bridge via an app-global +
deferred dispatch; then map the incoming URLs through the existing `do_open_path`
(canonicalize → dedupe → open) that CLI-argv already uses. Verify on real macOS
(smoke item **M-15**).

## Intentional scope exclusions (NOT gaps — deliberate, listed for completeness)

- **Silent `.xlsx` fidelity strip on save, no warning** (§5.2) — intentional MVP
  decision; the warn-and-strip UX is [`xlsx-preservation.md`](xlsx-preservation.md).
- **Dynamic arrays / spill absent** (§8) — accepted absent for v1; the engine surfaces
  an error. Out of MVP scope by product call.

## When picking these up

Items #1 and #2 share a root cause (the publication carries no per-cell type/color) and
should be done together — see [`type-aware-alignment.md`](type-aware-alignment.md) for
the publication → grid threading plan. #3 is a small chrome-local change. #4 needs a
gpui-capability spike before estimating. None are blocked by the others.
