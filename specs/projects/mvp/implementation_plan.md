---
status: complete
---

# Implementation Plan: FreeCell MVP

Many small phases (per the overview). Details live in `functional_spec.md`,
`ui_design.md`, `architecture.md`, and `components/*.md` — this is the ordered
checklist.

## Autonomy contract (for implementation agents)

Work **autonomously — never stop to ask a human**. The specs are the source of
truth; where they're silent, decide yourself in the spirit of the specs and keep
building. Record in `DECISIONS_TO_REVIEW.md` (append-only, below its marker line):
any judgment call a human might want to revisit, any spec deviation forced by
reality (API missing at the pinned rev, etc.), and any placeholder you resolved
(pinned SHAs, calibrated thresholds). Do **not** relitigate decisions the specs mark
as decided/ratified. If something is truly blocked (e.g., a dependency won't build
any way you try), implement the specced fallback, record it, and continue.

## Dependency graph & parallelism

```
P1 scaffolding ─► P2 core ─┬─► P3 doc I/O ─► P4 worker ──┐
                           │              └► P5 cache ───┤   (Track A: engine, Linux)
                           ├─► P6 grid render ─► P8 grid input ─┐ (Track B: grid, GPUI)
                           │        └────(P5,P6)─► P7 render suite │
                           └─► P9 chrome ─► P10 app shell ──────┤ (Track C: shell, GPUI)
                                                                 ▼
                                    P11 integration ─► P12 perf ─► P13 hardening
```

- **Tracks A, B, C run in parallel** after Phase 2 (three concurrent agents/worktrees
  is the intended shape). Within tracks: P5 can start alongside P4 (both need P3);
  P7 needs P5+P6; P9 and P10 overlap but P10 finishes after P9.
- P11–P13 are serial integration phases — single agent each.

## Phases

- [x] **Phase 1 — Scaffolding & CI.** `app/` workspace; `freecell-core` /
  `freecell-engine` / `freecell-app` / `render-tests` crate skeletons with the strict
  dependency rule; pinned toolchain, rustfmt, clippy(-D warnings), cargo-deny (incl.
  documented ztracing/GPL exception); GitHub Actions per `architecture.md §9` —
  Linux `checks` job required-green, `macos-verify` manual/cron job defined;
  hello-world GPUI + gpui-component window **builds on Linux and macOS** (this
  **locks the gpui-component SHA** against the pinned gpui rev — record it +
  toolchain in DECISIONS_TO_REVIEW.md). **Includes the load-bearing Linux render
  spike**: run the hello-world under Xvfb + Mesa lavapipe on the CI image and
  capture pixels to PNG (capture-path preference order in
  `components/render_test_harness.md §Mechanism`); record which capture variant
  works. **A failed spike is a decision point, not a stopper** (product call): if no
  Linux capture works, record it in DECISIONS_TO_REVIEW.md, move the render suite to
  the `macos-verify` workflow, and keep building — nothing downstream blocks on the
  answer. `app/README.md` skeleton. (`architecture.md §1, §9`)
- [x] **Phase 2 — Core foundations** (Linux). Axis port + POC tests; A1/CellRange;
  `RenderStyle`; `Publication`/`PublishedCell`; `SheetCaches` read model; input-cap
  validator (incl. round-3 D abort reproducers as rejected cases); sheet-name
  validator; palette; `SelectionModel` + keyboard motions; data-row reducer.
  (`architecture.md §3`; components: grid/style_cache/app_shell test plans' Linux
  halves)

**Track A — engine (Linux-testable):**
- [x] **Phase 3 — Document I/O.** IronCalc adapter: new/open/save (atomic
  temp+rename), typed load/save errors, fixture workbooks, open→save→reopen
  round-trip tests. (`components/engine_worker.md §File I/O`)
- [x] **Phase 4 — Eval worker seam.** Command/event loop, drain-coalescing,
  publish-then-bump generation, viewport publication build, 64 MiB stack, worker-side
  cap re-check, catch_unwind + degraded policy, dirty-op accounting, full seam test
  suite (incl. negative control). (`components/engine_worker.md`)
- [x] **Phase 5 — Style & geometry cache.** Interner, build-on-activation, unit
  conversions, mirror-on-edit, undo/redo touch-set re-read, agreement-contract tests
  + negative control; integrate into worker (StyleCacheUpdated deltas).
  (`components/style_cache.md`)

**Track B — grid (GPUI, cross-platform):**
- [x] **Phase 6 — Grid static rendering.** Headers, gridlines, cells (fills, text
  attrs, alignment, clipping), variable geometry, wheel scroll + clamping, custom
  scrollbars, loading overlay — against hand-built core fixtures.
  (`components/grid.md`, `ui_design.md §3.3`)
- [x] **Phase 7 — Render-test harness + initial suite.** (needs P5, P6) Capture via
  the variant the Phase-1 spike proved (Linux Xvfb+lavapipe primary; macOS fallback)
  + perceptual diff ported from round-3 C; scene builder through the real engine;
  `generate_baselines`; README (human baseline process); initial ~45-case suite
  green in CI with committed baselines. (`components/render_test_harness.md`)
- [x] **Phase 8 — Grid interaction.** Mouse selection (click/drag/shift, edge
  auto-scroll), keyboard motions wired, scroll-into-view, ViewportChanged events,
  selection render snapshots. (`components/grid.md`)

**Track C — shell & chrome (GPUI, cross-platform):**
- [x] **Phase 9 — Chrome.** Action row (toggles + fill popover), data row (ref box +
  content field state machine + cap error + eval spinner), sheet tab bar (switch /
  add / inline rename / context menu / delete confirm) — against a test-double
  client. (`components/app_shell.md`, `ui_design.md §3.1–3.4`)
- [x] **Phase 10 — App shell.** Welcome window, window registry + lifecycle rules
  (last window closes → app quits), menu bar (macOS) + actions + per-platform key
  bindings, file panels, all modals, save flow (no fidelity warning — silent strip
  per `functional_spec.md §5.2`), quit flow. (`components/app_shell.md`,
  `functional_spec.md §2`)

**Integration (serial):**
- [ ] **Phase 11 — Integration.** Real `DocumentClient` wired end-to-end
  (grid+chrome+worker+shell); open/edit/eval/save flows; dirty + title state; sheet
  switching with per-sheet scroll/selection; eval indicator; error paths
  (LoadFailed, SaveFailed, EditRejected, degraded bar); gpui-context integration
  tests + explicit list of anything untestable. (`functional_spec.md` end-to-end)
- [ ] **Phase 12 — Perf harness + CI gates.** POC run-test scenario against the real
  grid + 1M×100 styled fixture; true budgets (frame p99 ≤ 8.33 ms, worst ≤ 16.67 ms,
  cell load p99 < 2 ms, zero engine calls on scroll path) measured on real hardware
  and recorded; **Linux CI gates hard-fail at committed thresholds = 2× the p99
  calibrated on the pinned runner image** (buffer for slow shared runners — product
  call); numbers adversarially reviewed per repo convention.
  (`architecture.md §4, §9`)
- [ ] **Phase 13 — Hardening & completion sweep.** Render suite complete w/
  eyeballed baselines; READMEs complete; DECISIONS_TO_REVIEW.md finalized;
  cargo-deny clean-or-documented; manual smoke checklist executed and recorded;
  verify every `functional_spec.md` behavior has a test or an explicit
  documented-manual entry.
