---
status: draft
---

# Architecture: FreeCell MVP

This document turns the adopted, validated decisions from the de-risking rounds
(`experiments/round-2/SYNTHESIS.md` §"Adopted baseline decisions",
`experiments/round-3/SYNTHESIS.md` §"Adopted decisions confirmed") into the concrete
technical design for the MVP app. Where this doc and the syntheses appear to disagree,
the syntheses win — flag it, don't improvise.

This is a large project: this doc holds the system-level design; per-component detail
lives in `components/*.md` (grid, engine worker, style cache, render test harness, app
shell).

## 1. Repository & workspace layout

New top-level `app/` folder (per the overview), a self-contained Cargo workspace.
`experiments/` stays untouched and is **not** a dependency — POC code is **ported**
(copied + adapted, with attribution comments) into the app, never path-referenced:
experiments are frozen and the app must not depend on throwaway crates.

```
app/
├── Cargo.toml                 # [workspace], shared deps, lints, profile
├── rust-toolchain.toml        # pinned stable toolchain
├── rustfmt.toml               # defaults (+ max_width = 100)
├── deny.toml                  # cargo-deny: licenses/advisories/bans
├── README.md                  # build/run/test + render-baseline process
├── crates/
│   ├── freecell-core/         # GPUI-free, IronCalc-free foundation
│   ├── freecell-engine/       # IronCalc adapter + eval worker + caches + file I/O
│   └── freecell-app/          # GPUI application (macOS)
├── render-tests/              # cell-render snapshot suite (crate; see components/)
│   ├── baselines/             # committed reference PNGs
│   └── README.md              # human baseline process
└── .github/ → ../.github/workflows/*.yml   # CI lives at repo root (shared repo)
```

### Crate dependency rule (strict, CI-enforced by structure)

```
freecell-core      →  (std only + small utility deps)      # Linux-testable
freecell-engine    →  freecell-core, ironcalc(_base)       # Linux-testable
freecell-app       →  core, engine, gpui, gpui-component   # macOS build
render-tests       →  freecell-app (grid), gpui            # macOS build
```

`freecell-core` and `freecell-engine` never import GPUI; everything CI-testable on
Linux lives there. This mirrors the POC's proven `poc-core` split
(`experiments/04-ui-poc/poc-core`).

### Pinned dependencies (exact — scaffolding phase must use these)

- `ironcalc = "=0.7.1"`, `ironcalc_base = "=0.7.1"` (underscore crate name; the pin all
  experiment numbers are comparable against).
- `gpui`, `gpui_platform` (features `["font-kit"]`): git
  `https://github.com/zed-industries/zed`, rev
  `1d217ee39d381ac101b7cf49d3d22451ac1093fe`.
- `gpui-component`, `gpui-component-assets`: git
  `https://github.com/longbridge/gpui-component`, pinned to a `main` SHA resolved at
  scaffolding time (must build against the gpui rev above — start from the SHA
  gpui-component's own workspace pins that zed rev; record the chosen SHA in
  DECISIONS_TO_REVIEW.md).
- Utility: `thiserror` (lib error enums), `anyhow` (app edges), `serde`/`serde_json`
  (fixtures, perf reports), `image`/`png` (render tests), `smol` only if needed for
  channel/executor glue (gpui already embeds smol).
- Edition 2021 (matches the pinned gpui workspace); toolchain = a recent stable that
  builds the zed rev (resolve at scaffolding; record it).

## 2. System overview — one window, one document, one worker

Per spreadsheet window (no cross-window shared document state):

```
┌─ UI thread (GPUI) ────────────────────────────────────────────────┐
│ WorkbookWindow (gpui Entity)                                      │
│  ├─ chrome: action row / data row / tab bar (gpui-component)      │
│  ├─ GridView (custom): draws from ▼ read-only, never touches      │
│  │   the engine                                                   │
│  ├─ SelectionModel, per-sheet scroll state (freecell-core)        │
│  └─ DocumentClient ── commands ──► mpsc ──► EvalWorker            │
│        ▲ events (smol channel → gpui task → entity.update)        │
├─ shared read surfaces (written by worker, read by UI) ────────────┤
│  • Arc<Publication>  (viewport values snapshot + generation)      │
│  • Arc<RwLock<SheetCaches>> (style + geometry resident cache)     │
│  • AtomicU64 generation counter                                   │
├─ EvalWorker thread (64 MiB stack) ────────────────────────────────┤
│  owns UserModel<'static> (IronCalc)                               │
│  loop: drain commands → apply batch → evaluate() → re-pull        │
│        viewport → publish → bump generation → notify UI           │
│  also: open/save file I/O, style-cache builds & deltas,           │
│        cell-content reads, catch_unwind around apply+eval         │
└───────────────────────────────────────────────────────────────────┘
```

This is the SP1 seam, verbatim, carrying `UserModel` (validated in round-3 A):

- **Worker owns the model.** The UI thread never holds a reference to any IronCalc
  type. All reads the UI needs are either (a) published snapshots, (b) the resident
  style/geometry cache, or (c) explicit request/response messages.
- **Coalescing:** the worker drains its command queue completely before each
  `evaluate()`; N rapid edits → 1 eval.
- **Publish-then-bump:** the viewport snapshot `Arc` is swapped **before** the
  generation counter increments (SP1's ordering fix); the UI treats a generation bump
  as "re-read the publication and repaint".
- **~3× overscan** on the published viewport; scrolling inside it during an eval stays
  fully live; beyond it, value cells render blank until the next publish (styles and
  geometry always render — resident cache).
- **UI notification:** the worker pushes `WorkerEvent`s over a smol channel; a gpui
  foreground task per window awaits it and calls `entity.update(cx, …)` → `cx.notify()`.
  No polling, no frame-time engine work.

### Command / event protocol (the boundary contract)

`freecell-engine` defines (names indicative; component doc is authoritative):

```rust
enum Command {
    // edits (undoable, trigger eval + publish)
    SetCellInput { sheet, row, col, input: String },       // pre-validated (input cap)
    ClearCells { sheet, range: CellRange },
    SetStyleAttr { sheet, range: CellRange, attr: StyleAttr }, // Bold(bool)|Italic|Underline|Fill(Option<Rgb>)
    AddSheet, RenameSheet { idx, name }, DeleteSheet { idx },
    Undo, Redo,
    // reads / control (not undoable)
    SetViewport { sheet, rows: Range<u32>, cols: Range<u32> }, // already overscanned
    GetCellContent { sheet, row, col, req_id },            // formula-bar raw text
    Save { path: PathBuf, req_id }, 
    Shutdown,
}

enum WorkerEvent {
    Loaded { sheets: Vec<SheetMeta>, .. } | LoadFailed { reason },
    Published,                       // new generation available
    EvalStarted | EvalFinished,      // drives the evaluating spinner (UI shows after 250 ms)
    CellContent { req_id, raw: String },
    Saved { req_id, ops_seen: u64 } | SaveFailed { req_id, reason },
    EditRejected { reason },         // catch_unwind recovery path
    StyleCacheUpdated { sheet },
    SheetsChanged { sheets: Vec<SheetMeta> },
}
```

`Publication` (per sheet, active sheet only): a flat `Vec<PublishedCell>` for the
overscanned viewport — `{ row, col, display_text, text_color: Option<Rgb>,
raw_content: String }` — plus the covered ranges and generation. Including
`raw_content` makes the formula bar instant and never blocked on an in-flight eval;
`GetCellContent` is the fallback for cells outside the published window. Display text
and its optional color come from the engine's formatted-value API — **no number-format
logic in FreeCell** (round-3 B).

### Dirty tracking

The worker counts committed ops (`ops_seen`). `Saved{ops_seen}` acks the op-index the
file contains; the UI's dirty flag = `latest_committed_op > last_saved_op`. Undo can
un-dirty only if op indices match exactly — MVP keeps it simple: undo also increments
the op counter (undoing to the saved state still shows dirty; acceptable, Excel-like
enough).

## 3. Data model (per window)

| State | Owner | Read by | Notes |
|---|---|---|---|
| `UserModel` (workbook truth) | worker | worker only | undo/redo, styles, sheets all inside |
| Viewport `Publication` | worker (writes) | UI | `Arc` swap + generation |
| `SheetCaches`: per-sheet `StyleCache` + `GeometryCache` | worker (writes) | UI (RwLock read) | built on sheet activation; see components/style_cache.md |
| `SelectionModel` (active cell, anchor, range) per sheet | UI | UI | in-session only |
| Scroll offsets per sheet | UI | UI | pixel-space, clamped by geometry cache totals |
| Sheet list/order/active (`SheetMeta`) | worker publishes | UI | UI mirrors for tab bar |
| Dirty flag, file path, pending edit text | UI | UI | dirty per §2 |
| App-global: window registry, welcome-window handle, menu state | app shell | app shell | the only cross-window state |

`freecell-core` owns the pure logic types: `Axis` (two-level prefix-sum virtualization,
BLOCK=512 — ported from `poc-core/src/layout.rs`), `CellRange`/A1-reference conversion,
`SelectionModel` + keyboard-motion rules, formula **input-cap validator** (length
> 8192 or nesting depth > 64 → reject; depth counted by parenthesis nesting scan),
sheet-name validator, the fill palette constants, `RenderStyle` (engine-free resolved
style: bold/italic/underline/fill/align/font-color), and the engine-free **read
models** the grid consumes: `Publication`/`PublishedCell` and the `SheetCaches` cache
read model (`components/style_cache.md`) — so the UI track builds against core
fixtures without waiting on the engine track.

## 4. The grid (custom component — summary)

Full design in `components/grid.md`; the load-bearing rules:

- Rendering follows the **raw-gpui POC** (`experiments/04-ui-poc/raw-gpui/src/grid.rs`):
  per frame, compute visible index ranges via `Axis::visible_range(scroll, extent,
  overscan)`, and emit **absolutely-positioned divs** only for visible cells + headers.
  gpui-component's table primitives are structurally disqualified (uniform heights,
  per-column materialization at 16k cols) — decided in the POC, do not revisit.
- Frame inputs are **only**: geometry cache (sizes/offsets), style cache (fills,
  attrs), current `Publication` (display strings), selection, scroll. **Zero engine
  calls, zero locks held across paint beyond the brief RwLock read, zero allocation
  proportional to sheet size.**
- Draw order per cell: fill → gridlines-adjacent edges → text (clipped); selection
  overlay + active-cell border on top; fixed headers last.
- Perf gates (CI): frame p99 ≤ 8.33 ms, worst ≤ 16.67 ms; newly-visible cell load p99
  < 2 ms — the POC run-test scenario ported as a perf harness binary.

## 5. Engine adapter & file I/O (summary)

Full design in `components/engine_worker.md`:

- **Open:** worker thread starts with `Load(path)` → `load_from_xlsx` → wrap in
  `UserModel` → build active-sheet caches → initial publish → `Loaded`. First paint
  uses the file's cached values (no evaluate on open — SP2 behavior). Parse failures →
  `LoadFailed{reason}` (typed: not-xlsx / corrupt / password / io).
- **Save:** `get_model()` → IronCalc xlsx writer → **temp file + atomic rename**;
  fsync before rename. Save runs on the worker (serializes with evals — acceptable;
  UI shows the indicator).
- **Style edits** go through `UserModel`'s range-style API (attr paths like `font.b`,
  `fill.fg_color`) so they're undoable and round-trip. Exact method names come from
  the round-3 B matrix (`experiments/round-3/B-api-audit/findings.md`); if a needed
  setter exists only on `Model`, route it there and record the undo limitation in
  DECISIONS_TO_REVIEW.md.
- **Robustness (round-3 D, mandatory):** input cap enforced UI-side *and* re-checked
  worker-side; worker spawned with `stack_size(64 * 1024 * 1024)`;
  `catch_unwind(AssertUnwindSafe(apply_batch_and_eval))` — on catch, drop the batch,
  emit `EditRejected`, keep serving from the last good publication. Worker-thread
  death (join error) → UI error bar with Save As escape hatch.
- **Sheet ops** map to engine sheet APIs (add/rename/delete validated present in B).

## 6. Style & geometry cache (summary)

Full design in `components/style_cache.md`. Locked by round-3 A; MVP subset:

- Per sheet: dense default sizes + `BTreeMap` overrides for row heights / col widths,
  fronted by `freecell-core::Axis` for prefix-sum offset math; a `StyleInterner`
  (dedup on serialized `Style`, since `Style: Eq` but not `Hash`) + `BTreeMap<(row,
  col), StyleId>` for cell styles + row/col band styles; resolution order cell > row >
  col > default (engine-defined, SP4-verified).
- Built **on the worker** when a sheet first activates (open = active sheet only);
  updated by **mirroring the ops FreeCell issued** (it originates every edit). After
  `Undo`/`Redo` the worker re-reads styles for the cell set recorded against the
  history entry and ships deltas — simplest correct MVP path; A's validated
  inverse-op mirror (and the sub-ms structural shift design) slots in when
  insert/delete rows/cols UI arrives (P2, not built now).
- Exposed to the UI as `Arc<RwLock<SheetCaches>>`; writes are rare (style edits,
  sheet switch), reads are per-frame and cheap; values in the cache are resolved
  `RenderStyle`s, so the render path does zero engine-type work.

## 7. App shell, windows, menus (summary)

Full design in `components/app_shell.md`:

- `gpui_platform::application().run(...)`; a `FreeCellApp` global holds the window
  registry. Welcome window at launch; opening/creating a document closes it; when the
  last window closes, the app quits. Quit prompts per dirty window.
- macOS menu bar via GPUI's menu/action API; menu actions dispatch to the focused
  window's entity. Keyboard shortcuts are GPUI key bindings bound to the same actions
  (single source of truth).
- Native NSOpen/NSSave panels via GPUI's paths-prompt APIs at the pinned rev;
  gpui-component dialogs as fallback if those bindings prove unusable (record if so).
- Finder "open with" events: wire GPUI's open-files/urls handler if present at the
  pinned rev; otherwise document as a known gap (best-effort feature,
  DECISIONS_TO_REVIEW.md).
- All dialogs (unsaved changes, errors, delete-sheet confirm) are
  gpui-component modals owned by the window entity; async flows (close-with-prompt →
  save → close) are small state machines on the entity, no blocking.

## 8. Error handling strategy

- **Library crates** (`core`, `engine`): `thiserror` enums (`EngineError::{Load, Save,
  EditRejected, …}`), no panics on user input paths (validated: engine returns typed
  errors for malformed input; the abort-class input is excluded by the cap).
- **App crate**: `anyhow` at the edges; every user-visible failure maps to a dialog
  with a human-readable sentence + the file name; never a silent failure, never a
  crash for document-data reasons.
- **Formula errors are values**, rendered in-cell (`#DIV/0!`, `#CIRC!`, …) — never
  dialogs.
- **Logging:** `tracing` + `tracing-subscriber` (env-filter). Worker logs
  apply/eval/publish timings at `debug` (these are the SP1 observables — keep them
  measurable). No telemetry.

## 9. Testing strategy

Per the overview: each phase ships tested well enough to need no human review.

| Layer | Kind | Runs on | What |
|---|---|---|---|
| `freecell-core` | unit | Linux CI | Axis math (port POC's 20 tests), selection/keyboard rules, A1 conversion, input-cap (incl. the D abort reproducers as *rejected* cases), name validation |
| `freecell-engine` | unit + integration | Linux CI | worker seam: coalescing (N edits→1 eval), publish-then-bump ordering, staleness bound, catch_unwind recovery, dirty-op accounting; style cache: build + mirror + undo re-read agreement vs engine re-read (port round-3 A's agreement-contract tests + negative control); file: open→edit→save→reopen round-trips (values, formulas, styles, number formats, sheets), atomic-save failure injection, corrupt-file fixtures |
| grid & chrome | render snapshot | macOS CI | the cell-render suite (below) + a few whole-grid scenes (headers, selection, range) |
| perf | gated harness | macOS CI | POC run-test scenario against the real grid + engine-backed provider; asserts the §4 gates; **foreground with `timeout`, forced + asserted work, p50/p99 reported** (repo convention) |
| app flows | integration (gpui test context) | macOS CI | open→edit→save happy path, unsaved-close prompt state machine, welcome lifecycle — as far as the gpui test APIs allow; anything untestable is listed explicitly in the phase plan, not silently skipped |

### Cell-render snapshot suite (first-class deliverable)

Full design in `components/render_test_harness.md`. Mechanism = round-3 C, verbatim:
offscreen Metal capture (`show:false`) on a macOS runner → PNG → perceptual diff
(per-channel tolerance 12/255, fail fraction 0.5%, tuned after first real baselines).
Each case renders **the real grid component** over a tiny fixture sheet. Naming:
`cell_bold`, `cell_bold_italic`, `cell_bold_italic_underline`, `cell_fill_red`,
`cell_bold_fill_yellow`, `cell_number_currency`, `cell_date_default`,
`cell_error_div0`, `cell_text_clipped`, `cell_align_right_number`, … — one axis or
meaningful permutation per test, snake_case, so a red CI line reads as the exact
feature broken. `generate_baselines` (script/binary flag) rewrites `baselines/`;
README documents: run it on the CI runner class, eyeball every changed PNG, commit.
Baselines are committed; capture and validation must use the same runner class
(Metal AA/font variance — C's risk note).

### CI (GitHub Actions, repo root)

1. **linux-checks** (every push/PR): `cargo fmt --check`; `cargo clippy --workspace
   --exclude freecell-app --exclude render-tests -- -D warnings`; `cargo test -p
   freecell-core -p freecell-engine`; `cargo-deny check` (licenses/advisories — with a
   documented temporary exception for the GPL `ztracing` transitive dep, tracked
   against zed #55470; must be resolved before any binary distribution).
2. **macos-checks** (every push/PR): full workspace build + clippy `-D warnings` +
   all tests + render suite; perf harness as a separate always-run step whose gates
   fail the job.
3. Caching (`Swatinem/rust-cache`) to keep the gpui build tolerable.

## 10. Technical risks & mitigations (build-time)

- **gpui-component SHA compatibility** with the pinned gpui rev — resolved in the
  scaffolding phase (build a hello-world using both before anything else depends on
  it). Fallback: take gpui-component's own pinned gpui rev pair.
- **GPUI API drift** vs POC code — expected small (one known `use gpui::AppContext as
  _;` class of fix); the grid port phase budgets for it.
- **Text shaping cost** (bold/italic runs, wide columns) is the most likely frame-time
  risk (C's note) — the perf harness scene includes the POC's wide/styled columns so
  it's measured, not felt.
- **Render-suite baseline flakiness** across runner generations — pin the runner image
  (e.g. `macos-14`), record it in the render-tests README, and re-baseline only
  deliberately. Cell text uses the **bundled Inter** font (UI round decision), which
  removes font-version drift — the main variance source C flagged — from cell pixels;
  remaining variance is Metal AA only.
- **IronCalc unit conversions** (column-width units / row-height points → px) —
  resolve against SP4/POC constants in the geometry cache; one place only.

## 11. What this architecture deliberately defers

Structural-edit cache shifting (validated, not built), FreeCell dirty-tracking /
viewport value-delta cache (round-2 agenda #1 — MVP repaints the viewport per publish,
which SP1 showed is fine), snapshot-on-demand for beyond-overscan scroll mid-eval
(blank cells are acceptable for MVP), CSV, clipboard, in-cell editor, IME, merges/CF
pass-through, dynamic arrays (accepted absent for v1), multi-window shared style
interning. All are architected *around* (nothing here blocks them) but not built.
