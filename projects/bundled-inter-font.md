# Bundled Inter Font

**Status: Future (deferred from MVP Phase 13, 2026-07-04).**

## Goal

Ship **Inter** (SIL OFL) in the app bundle and register it via GPUI's `add_fonts` at
startup so the grid + chrome render in one predictable family, as `ui_design.md §3.3`
/ §7 and `functional_spec.md §3.6` call for. The stated rationale is **pixel-stable
render-test baselines across machines / OS font-package updates** (font-version drift
was round-3 C's top flakiness risk) plus a clean, tabular-figures face.

## Why deferred (MVP decision)

The MVP ships on GPUI's **default UI font**. `register_fonts` (`shell/fonts.rs`) is a
logging no-op; the grid does not set `font_family` (the `GRID_FONT_FAMILY = "Inter"`
constant is reserved but unused). This was a conscious Phase-13 call, not an oversight:

- **The baseline-stability rationale is already delivered for the MVP** by a *different*
  mechanism: the render suite pins the exact runner image + Mesa/lavapipe version +
  system font packages, and `render-tests/README.md` states baselines are only valid on
  that runner class. On that pinned, deterministic software-rasterized image the default
  font is bit-stable — the whole 48-case suite is green and reproducible. Inter would
  make baselines *portable across differing font environments*, which the harness does
  not attempt anyway.
- **The readily-fetchable Inter distribution here is the *variable* font** (roman +
  italic `Inter[opsz,wght].ttf`), not the 4 static faces the spec names. Registering a
  variable font and resolving `.font_weight(BOLD)` / italic through GPUI's font-kit stack
  adds variable-axis-resolution uncertainty on top of the change.
- **The render harness (`render_scene`) opens its own GPUI `App` and does not call
  `register_fonts`.** Vendoring would require wiring `add_fonts` in the harness too, then
  regenerating **and re-eyeballing all 48 baselines** — changing the render font touches
  the load-bearing pixel gate at the very end of the project, disproportionate to a
  robustness/polish improvement (not a functional gap).

So Inter is a **portability/robustness upgrade**, appropriate post-MVP, not a
functional MVP requirement.

## Work when picked up

1. Vendor the 4 static Inter faces (`Regular`/`Bold`/`Italic`/`BoldItalic`, SIL OFL) under
   `app/crates/freecell-app/assets/fonts/` + the `OFL.txt` license (prefer static faces
   over the variable font for deterministic weight/italic resolution). Fonts are assets,
   not crates — no `cargo-deny` impact, but add the OFL license file.
2. Flip `shell/fonts.rs::register_fonts` to `cx.text_system().add_fonts(...)` the bundled
   bytes; keep the graceful "bundle absent → default font" fallback.
3. Register the same fonts in the render harness (`render-tests/src/render.rs`
   `run_render_scene`) before opening the window.
4. Apply `.font_family(GRID_FONT_FAMILY)` at the grid text sites (`grid/view.rs`
   cell + header text) and set the chrome to Inter if desired (spec keeps chrome on the
   gpui-component theme font).
5. Regenerate **all** baselines on the pinned runner image
   (`render_tests.sh generate`), **eyeball every changed PNG**, and commit them with the
   code change. Confirm the foreground render suite stays green. Text-metric-sensitive
   cases (`cell_text_clipped`, `cell_text_exact_fit`, `cell_narrow_column_clipped_number`)
   will shift and must be re-inspected.
6. Update `render-tests/README.md` (remove the "Phase-10 bundled Inter" caveat) and
   `DECISIONS_TO_REVIEW.md`.
