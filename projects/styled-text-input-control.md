# FreeCell Styled Text-Input Control

**Status:** Future (v1.0) — split out of the `formula-point-mode` project (owner decision,
2026-07-18).

## Goal

A FreeCell-owned text-input control that supports **rich per-range in-editor formatting** —
coloring reference *tokens* inside a formula, cell/text backgrounds, and the broader
Excel/Numbers/Sheets-like in-editor styling we want to grow into — for the formula editors
(the in-cell overlay + the data-row/formula bar).

## Why it's its own project (and not a gpui-component fork)

Both formula editors are currently `gpui_component::input::InputState`. That widget is
**closed to external per-range styling**: it exposes no public styling API, its
`code_editor(language)` mode accepts only a built-in *language name* (syntax colors only, no
backgrounds, not our formula tokens), and its text layout is private — so an external crate
cannot render styled text over it. The two ways to get rich in-editor formatting are:

1. **Fork gpui-component** — rejected. It's a second maintained fork (IronCalc is a justified
   core-dependency fork; a gpui-component fork is extra maintenance), and the formatting we
   want is FreeCell-specific and would not be upstreamed/generalized into GPUI anyway.
2. **Build our own control over gpui's text primitives** — this project. We own the text
   rendering + caret + selection + mouse + IME for a styled field (effectively *replacing*
   `InputState` for the formula editors), which is what unlocks arbitrary per-range styling.

## Scope / why it's substantial

- Owning a text input means re-homing the shipped features that currently ride `InputState`:
  **autocomplete + signature hints, the cap-error popover, quick-edit, and the two-editor
  sync** (all built on `InputState` today).
- The in-cell overlay is rendered by the grid, so this is **pixel-suite in-scope** — it needs
  render baselines and IME/selection edge-case coverage.
- Higher-risk, reusable infrastructure → a de-risking project in its own right, in the spirit
  of FreeCell's staged rounds.

## What's already done for it

The `formula-point-mode` (v0.5) project computes a **token→color map** on the shared edit
state (keyed per distinct reference, first-appearance order, a fixed theme-aware palette) to
drive the **grid** range highlighting. This control **consumes that same map** to color the
in-editor tokens — the data model is already in place; this project supplies the render
surface. It builds on the **consolidated `EditController`** that v0.5 promotes into the single
owner/factory for the formula-editor pair.

## Related

- `specs/projects/formula-point-mode/` — the v0.5 project that ships grid highlighting +
  point-mode and computes the color map.
- Existing editor plumbing: `chrome/edit.rs` (`EditController`), `freecell_core::data_row`
  (the canonical pending-edit reducer).
