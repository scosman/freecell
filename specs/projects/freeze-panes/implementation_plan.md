---
status: complete
---

# Implementation Plan: Freeze Panes

Six phases. Front-load the **engine wiring + read-model plumbing** (thin, no fork, no pixel), then
the **header-menu** entry, then the **quadrant render + clamp rework** (the real work), then the
**cross-boundary interactions**, then the **structural-edit checkpoint**, and finally a
**dedicated late render-validation phase** (full suite + CI `render` gate). Each coding phase:
build **crate-scoped**, `cargo fmt --all --check` (whole workspace), unit/gpui tests + (the two
grid phases) a render **subset**, commit + push. The **full** pixel suite + CI `render` gate run
**once**, in Phase 6.

Section refs point at `architecture.md` (§1–§8) and `components/viewport_split.md` (§1–§7). Cargo
runs from `app/`. Q4 is the one contingent item (Phase 5).

## Phases

- [x] **Phase 1 — Engine wiring + read model (`architecture.md §2`).** No fork, no pixel.
  - `freecell-core/cache.rs`: add `frozen_rows`/`frozen_cols` `u32` to `SheetCache` +
    `SheetCacheBuilder` (accessors + setters + fluent, beside `hidden_rows`); **not** fed to
    `axis_from` (no geometry effect).
  - `freecell-engine`: `Command::SetFrozen { sheet, rows: Option<u32>, cols: Option<u32> }`
    (`protocol.rs`); `document.rs` `set_frozen_rows`/`set_frozen_columns` wrappers (clamp to
    `[0, count]`); `run.rs` apply (`GeometryOnly`) + edit-bucket + `op_of` → `Rebuild`;
    `build_sheet_cache` reads `ws.frozen_rows`/`ws.frozen_columns` into the builder.
  - `freecell-app`: `GridEvent::SetFrozen { rows, cols }` + `shell/window.rs` routing to
    `Command::SetFrozen` (mirror the `HideRows` arm).
  - Tests: engine `SetFrozen` toggles counts + **one undo step** restores prior; open a
    `<pane>` fixture → cache counts populated; save→reopen round-trips. Core builder test.
    Checks: `-p freecell-core -p freecell-engine -p freecell-app`, `fmt --all --check`.

- [x] **Phase 2 — Header-menu Freeze/Unfreeze (`architecture.md §4`, `functional_spec.md §1`).**
  No fork, no pixel (menu overlay is not a baseline surface).
  - `HeaderMenu` (`grid/view.rs`) gains `frozen: u32`; read `M`/`K` in `handle_right_mouse_down`
    under the existing lock and store at construction.
  - `header_menu_items` (pure): one Freeze/Unfreeze tuple using boundary `b = run.1` — count
    `b+1`; Unfreeze when `frozen == b+1` (→ set axis 0) else Freeze (→ set axis `b+1`); always
    enabled; emit `GridEvent::SetFrozen` on the menu's axis.
  - Tests: gpui — Freeze on a fresh row header, Unfreeze on the current boundary, moves boundary
    at a different track, symmetric for columns; pure `header_menu_items` label/event mapping.
    Checks: `-p freecell-app`, `fmt --all --check`.

- [x] **Phase 3 — Quadrant render + clamp rework (`components/viewport_split.md §1–§4`).** The
  core change. Moves grid pixels → iterate with the render **subset** only.
  - `layout.rs`: `PaneGeometry` (band extents, body area) + the re-based `clamp`/`reveal`/
    `hit_test`/`cell_at_point`/`edge_delta`/scrollbar helpers (§3); `M=K=0` reduces to today.
  - `grid/view.rs`: `Quadrant` sub-frames in `resolve_frame`; `build_grid_layers` → per-quadrant
    clipped content divs (factor `build_quadrant`); `cell_rect`/`span_rect` take a `Quadrant`;
    body-relative scroll wired into `handle_scroll` clamp + scrollbar; the freeze divider(s) +
    `FREEZE_DIVIDER` const (`grid/mod.rs`); ChartLayer clipped to the body quadrant.
  - Tests: pure `layout.rs` clamp/reveal/hit-test + `M=K=0` equivalence (`§7`); gpui —
    `resolve_frame` quadrant ranges, bands pinned while body scrolls, divider present iff frozen.
    Render **subset**: `render_tests.sh test freeze_` / `cell_` / `grid_` while iterating.
    Checks: `-p freecell-core -p freecell-app`, `fmt --all --check`.

- [x] **Phase 4 — Cross-boundary interactions (`components/viewport_split.md §3.2–§3.4, §5`).**
  Render **subset** only.
  - Wire the frozen-aware `hit_test`/`cell_at_point` into the mouse handlers
    (`handle_mouse_down`, `handle_right_mouse_down`, `update_fill_drag`, `autoscroll_tick`,
    `current_edge_delta`); reveal (`resolve_frame` pending-reveal + `reveal_and_announce`) no-op
    on a frozen axis + into the body sub-area; edge auto-scroll over the body sub-rect;
    `resize_hotspots` per-region (frozen vs scrolling dividers); selection/fill drags extend
    continuously across the divider (overlay per-quadrant).
  - **Publish the frozen bands in the viewport announce (from Phase 3).** Phase 3 renders band
    cells (`cell_index` is built over the quadrant union), but the `ViewportChanged` announce still
    reports only the **body** range — so a leading-band cell falls outside the published window once
    the body is scrolled deep past it, and the band then shows its fills/borders but **no VALUES**.
    Fix here: make the announce (`resolve_frame` + `handle_scroll` `ViewportChanged`) cover the
    union `(0..M ∪ body_rows) × (0..K ∪ body_cols)` — or always publish the leading `0..M` / `0..K`
    bands — so band cells show their values regardless of body scroll. Keep it O(visible): the bands
    are the few leading tracks, never a sheet-size loop. (Phase-3 code comment at the `cell_index`
    union filter in `grid/view.rs::build_grid_layers` flags this pointer.) **This MUST land before
    the Phase 6 `freeze_scrolled_body` baseline is generated** — otherwise that baseline would
    enshrine the blank-band bug as golden.
  - Tests: gpui — click/drag select across the boundary, reveal into body not under a band,
    auto-scroll only at body edges, resize a frozen track grows the band, and the announced
    viewport covers the frozen bands when the body is scrolled deep. Render **subset** for the
    cross-divider selection overlay.
    Checks: `-p freecell-app`, `fmt --all --check`.

- [ ] **Phase 5 — Structural-edit boundary tracking (`architecture.md §5`, Q4).** Engine, no
  pixel. **Checkpoint first:** in the fork container probe whether `insert_rows`/`delete_rows`/
  `insert_columns`/`delete_columns` already adjust `frozen_rows`/`frozen_columns` (Excel: insert
  above/within grows, delete within shrinks, below unchanged).
  - **If native (expected):** no code — add an engine regression test asserting the boundary
    tracks an insert/delete (one undo step) and close the phase.
  - **If not:** one focused `fix/structural-edits-adjust-frozen-pane` fork branch (upstream-style
    tests, one PR) that adjusts the pane inside IronCalc's structural ops — **no FreeCell-side
    compensating code** (keeps it one undo step; CLAUDE.md fix-upstream). Re-pin `freecell-fixes`.
    Checks: engine test green; `fmt --all --check`.

- [ ] **Phase 6 — Render validation (`architecture.md §7`, dedicated late phase).** No new
  behavior.
  - `render-tests`: `Scene` builder `.frozen_rows(m)`/`.frozen_cols(k)` (mirror `.hide_row`); new
    cases `freeze_top_row`, `freeze_rows_band`, `freeze_first_col`, `freeze_cols_band`,
    `freeze_four_quadrant`, `freeze_scrolled_body`, `freeze_divider`. **Prerequisite:** the Phase-4
    band-publishing fix must be in before `freeze_scrolled_body` is generated (else the scrolled
    band would baseline with blank values — see Phase 4).
  - Regenerate + **eyeball** baselines; run the **full** pixel suite under `timeout` + ~10-min
    watchdog; commit refreshed baselines; dispatch the CI `render` gate on the branch, poll green.

## Notes for the build

- **Fork policy:** only Phase 5 *may* touch the fork, and only if the probe is negative — then
  one fix = one branch = one upstream PR (never combined). Engine API (`set_frozen_*_count` +
  `<pane>` round-trip) already exists; Phases 1–4 are FreeCell-only.
- **Ephemeral container:** commit + push after every phase (and mid-Phase-3).
- **Efficiency:** crate-scoped checks per phase; reserve `--workspace` for a final pre-Phase-6
  validation; render **subset** while iterating (never a full run per coding phase).
