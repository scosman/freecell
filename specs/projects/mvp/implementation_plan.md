---
status: complete
---

# Implementation Plan: FreeCell MVP

Many small phases (per the overview). Details live in `functional_spec.md`,
`ui_design.md`, `architecture.md`, and `components/*.md` — this is the ordered
checklist. **Autonomy rule:** implementing agents decide open details per the specs,
log judgment calls in `DECISIONS_TO_REVIEW.md`, and do not stop to ask.

## Dependency graph & parallelism

```
P1 scaffolding ─► P2 core ─┬─► P3 doc I/O ─► P4 worker ──┐
                           │              └► P5 cache ───┤   (Track A: engine, Linux)
                           ├─► P6 grid render ─► P8 grid input ─┐ (Track B: grid, macOS)
                           │        └────(P5,P6)─► P7 render suite │
                           └─► P9 chrome ─► P10 app shell ──────┤ (Track C: shell, macOS)
                                                                 ▼
                                    P11 integration ─► P12 perf ─► P13 hardening
```

- **Tracks A, B, C run in parallel** after Phase 2 (three concurrent agents/worktrees
  is the intended shape). Within tracks: P5 can start alongside P4 (both need P3);
  P7 needs P5+P6; P9 and P10 overlap but P10 finishes after P9.
- P11–P13 are serial integration phases — single agent each.

## Phases

- [ ] **Phase 1 — Scaffolding & CI.** `app/` workspace; `freecell-core` /
  `freecell-engine` / `freecell-app` / `render-tests` crate skeletons with the strict
  dependency rule; pinned toolchain, rustfmt, clippy(-D warnings), cargo-deny (incl.
  documented ztracing/GPL exception); GitHub Actions `linux-checks` + `macos-checks`
  green; hello-world GPUI + gpui-component window builds on macOS CI (this **locks
  the gpui-component SHA** against the pinned gpui rev — record it + toolchain in
  DECISIONS_TO_REVIEW.md); `app/README.md` skeleton. (`architecture.md §1, §9-CI`)
- [ ] **Phase 2 — Core foundations** (Linux). Axis port + POC tests; A1/CellRange;
  `RenderStyle`; `Publication`/`PublishedCell`; `SheetCaches` read model; input-cap
  validator (incl. round-3 D abort reproducers as rejected cases); sheet-name
  validator; palette; `SelectionModel` + keyboard motions; data-row reducer.
  (`architecture.md §3`; components: grid/style_cache/app_shell test plans' Linux
  halves)

**Track A — engine (Linux-testable):**
- [ ] **Phase 3 — Document I/O.** IronCalc adapter: new/open/save (atomic
  temp+rename), typed load/save errors, fixture workbooks, open→save→reopen
  round-trip tests. (`components/engine_worker.md §File I/O`)
- [ ] **Phase 4 — Eval worker seam.** Command/event loop, drain-coalescing,
  publish-then-bump generation, viewport publication build, 64 MiB stack, worker-side
  cap re-check, catch_unwind + degraded policy, dirty-op accounting, full seam test
  suite (incl. negative control). (`components/engine_worker.md`)
- [ ] **Phase 5 — Style & geometry cache.** Interner, build-on-activation, unit
  conversions, mirror-on-edit, undo/redo touch-set re-read, agreement-contract tests
  + negative control; integrate into worker (StyleCacheUpdated deltas).
  (`components/style_cache.md`)

**Track B — grid (macOS):**
- [ ] **Phase 6 — Grid static rendering.** Headers, gridlines, cells (fills, text
  attrs, alignment, clipping), variable geometry, wheel scroll + clamping, custom
  scrollbars, loading overlay — against hand-built core fixtures.
  (`components/grid.md`, `ui_design.md §3.3`)
- [ ] **Phase 7 — Render-test harness + initial suite.** (needs P5, P6) Offscreen
  capture + perceptual diff ported from round-3 C; scene builder through the real
  engine; `generate_baselines`; README (human baseline process); initial ~45-case
  suite green on macOS CI with committed baselines.
  (`components/render_test_harness.md`)
- [ ] **Phase 8 — Grid interaction.** Mouse selection (click/drag/shift, edge
  auto-scroll), keyboard motions wired, scroll-into-view, ViewportChanged events,
  selection render snapshots. (`components/grid.md`)

**Track C — shell & chrome (macOS):**
- [ ] **Phase 9 — Chrome.** Action row (toggles + fill popover), data row (ref box +
  content field state machine + cap error + eval spinner), sheet tab bar (switch /
  add / inline rename / context menu / delete confirm) — against a test-double
  client. (`components/app_shell.md`, `ui_design.md §3.1–3.4`)
- [ ] **Phase 10 — App shell.** Welcome window, window registry + lifecycle rules,
  menu bar + actions + key bindings, native panels, all modals, save flow with
  fidelity warning, quit flow. (`components/app_shell.md`, `functional_spec.md §2`)

**Integration (serial):**
- [ ] **Phase 11 — Integration.** Real `DocumentClient` wired end-to-end
  (grid+chrome+worker+shell); open/edit/eval/save flows; dirty + title state; sheet
  switching with per-sheet scroll/selection; eval indicator; error paths
  (LoadFailed, SaveFailed, EditRejected, degraded bar); gpui-context integration
  tests + explicit list of anything untestable. (`functional_spec.md` end-to-end)
- [ ] **Phase 12 — Perf harness + CI gates.** POC run-test scenario against the real
  grid + 1M×100 styled fixture; gates (frame p99 ≤ 8.33 ms, worst ≤ 16.67 ms, cell
  load p99 < 2 ms, zero engine calls on scroll path) enforced in `macos-checks`;
  numbers adversarially reviewed per repo convention. (`architecture.md §4, §9`)
- [ ] **Phase 13 — Hardening & completion sweep.** Render suite complete w/
  eyeballed baselines; READMEs complete; DECISIONS_TO_REVIEW.md finalized;
  cargo-deny clean-or-documented; manual smoke checklist executed and recorded;
  verify every `functional_spec.md` behavior has a test or an explicit
  documented-manual entry.
