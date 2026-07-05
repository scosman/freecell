---
status: complete
---

# Implementation Plan: MVP Gaps — Core Spreadsheet Feel

Ordered checklist; details live in `functional_spec.md`, `ui_design.md`,
`architecture.md` (§ references below), and `components/*.md` (full designs — where a
component doc covers a phase, it is the primary spec: Phase 2 →
`components/edit_controller.md`; Phase 3 → `components/clipboard.md`; Phases 4–6 →
`components/action_bar.md` + `components/style_render.md`; Phase 7 →
`components/grid_structure.md`). Phases 3–7 are mutually independent — if one
stalls, skip and return. The MVP autonomy contract applies (work autonomously, record
judgment calls in a `DECISIONS_TO_REVIEW.md` here).

## Phases

- [x] **Phase 1 — Quick wins & publication**: `.back` backup (§7.3); cap-error
      popover, data-row only for now (§7.2); `PublishedCell.kind` + populated
      `text_color` + type-aware default alignment + `[Red]` color (§1.2–1.3);
      update GAPS.md rows on completion.
- [x] **Phase 2 — Editing feel**: EditController refactor (§4.1); type-to-replace
      (§4.2); live mirror (§4.3); in-cell editor (§4.4); Tab-commit (§4.5); cap
      popover on the in-cell editor.
- [ ] **Phase 3 — Range clipboard**: worker commands + slot, copy/cut/paste internal,
      TSV out/in, keymap (§6).
- [ ] **Phase 4 — Formatting controls**: `SetStylePath` command; text color,
      alignment, number-format dropdown + decimals ± ; action-bar layout rework
      (§3.1–3.2, ui_design §2).
- [ ] **Phase 5 — Fonts**: `RenderStyle`/cache font fields + rendering; `SetFont` via
      on_paste_styles + clamps + row auto-grow; family/size dropdowns (§3.3).
- [ ] **Phase 6 — Borders**: cache `BorderSpec` interning + edge rendering; presets
      menu via `set_area_with_border` (§3.4).
- [ ] **Phase 7 — Structure**: resize hotspots/cursors/preview/commit (§5.1); header
      selection + select-all + clamping rule (§5.2); insert/delete menu + merge
      guard (§5.3).
- [ ] **Phase 8 — Titlebar (macOS) + closeout**: on-device smoke first, then
      implement or flag-off per §7.1; full render-baseline regen + smoke checklist
      pass; GAPS.md sweep.
