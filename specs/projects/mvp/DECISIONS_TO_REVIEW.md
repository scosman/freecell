# DECISIONS_TO_REVIEW — implementation-phase log

**Purpose (part of the autonomy contract — see `implementation_plan.md`):**
implementation agents work autonomously and never stop to ask a human. When you make
a judgment call the specs don't cover, deviate from a spec because reality forced it
(e.g., an API missing at the pinned rev), or resolve a placeholder (pinned SHA,
calibrated threshold), **append an entry here and keep building**. A human reviews
this file at their leisure — it is a log, not a request queue.

Entry format: `- [Phase N] <decision> — <one-line rationale> (<files/spec section
affected>)`.

Known placeholders the build will resolve (append the resolution here):

- gpui-component pinned SHA + Rust toolchain version (Phase 1).
- Linux render-capture variant proven by the Phase-1 spike (or fallback-to-macOS).
- Perf-gate CI thresholds calibrated on the pinned runner image (Phase 12).
- Perceptual-diff thresholds after first real baselines (Phase 7).

---
*Append entries below this line. Do not edit above it.*

- [Phase 1] gpui-component pinned to SHA `a9a7341c35b62f27ff512371c62419342264710c`
  (longbridge/gpui-component `main`) — its workspace `Cargo.toml` at this SHA pins the
  exact target zed rev `1d217ee39d381ac101b7cf49d3d22451ac1093fe`, so it is the
  known-good pair with no rev-pair bisection needed. (`app/Cargo.toml`)
- [Phase 1] Rust toolchain pinned to stable `1.95.0` (`app/rust-toolchain.toml`).
  Resolved empirically: gpui at the pinned rev calls `std::hint::cold_path()` with no
  `#![feature(...)]` gate, so it requires the stable where `cold_path` is stabilized.
  Building on 1.94.1 fails with E0658 (`crates/gpui/src/profiler.rs`). zed's own
  `rust-toolchain.toml` at the pinned rev pins stable `1.95.0`, which is exactly the
  version that stabilized `cold_path` — so FreeCell matches zed's pin. Any future gpui
  rev bump must re-check zed's toolchain pin. (`app/rust-toolchain.toml`)
- [Phase 1] `gpui_platform` features set to `["font-kit", "x11", "wayland",
  "runtime_shaders"]` rather than architecture §1's `["font-kit"]` — the extra features
  are the Linux backends (x11/wayland) + shader path that gpui-component's own workspace
  enables, required for the cross-platform (macOS + Linux) build the phase mandates. The
  §1 list read as the macOS-focused subset. (`app/Cargo.toml`)
- [Phase 1] Workspace crates use edition 2021 (per architecture §1) even though the gpui
  and gpui-component crates themselves are edition 2024 — per-crate editions are
  independent; the pinned 1.95.0 toolchain supports both. (`app/*/Cargo.toml`)
- [Phase 1] `render-tests` is a bare skeleton crate that does NOT yet depend on
  `gpui`/`freecell-app` — those deps + the ported round-3 C perceptual diff + the case
  suite are Phase 7. Keeping it off the gpui edge in P1 means a render-spike failure can
  never block the workspace build/test. (`app/render-tests/`)
- [Phase 1] `deny.toml` is lenient on `[bans]` (multiple-versions/wildcards allowed) and
  `[sources] unknown-git = "allow"` because zed's tree carries many duplicate versions
  and pinned git forks; the load-bearing gate is licenses (the documented GPL `ztracing`
  exception, all three GPL-3.0 SPDX spellings, tracked vs zed#55470). P13 hardening may
  tighten bans/sources. (`app/deny.toml`)
- [Phase 1] `perf-gates.yml` is DEFINED but a placeholder (builds the workspace, prints a
  TODO) — the perf harness + committed buffered thresholds are Phase 12. The Phase-1
  `checks.yml` render step runs the spike as `continue-on-error` (informational); Phase 7
  flips it to a required `cargo test -p render-tests`. (`.github/workflows/`)
- [Phase 1] **LINUX RENDER SPIKE: PASSED** — the primary capture path works, so the
  render suite stays on Linux CI and the macOS offscreen-Metal fallback is NOT needed.
  The hello-world GPUI + gpui-component window renders under Xvfb + Mesa lavapipe
  (software Vulkan: "llvmpipe (LLVM 20.1.2)") and its pixels capture to a non-blank PNG
  (1566 colors — the FreeCell title, subtitle, 3×3 grid, and yellow B2 fill all render;
  text, fills, and borders confirmed). Capture variant = **option 2** from
  `render_test_harness.md §Mechanism`: render to an X window under Xvfb, capture the root
  via ImageMagick `import`. (`app/scripts/linux_render_spike.sh`)
- [Phase 1] **Load-bearing spike detail (MUST carry into Phase 7):** gpui's X11 backend
  only *presents* a rendered frame when it receives an **Expose** event
  (`crates/gpui_linux/.../x11/client.rs`: `require_presentation` is gated on
  `expose_event_received`). Under Xvfb there is no compositor to emit one, so the frame
  renders but never reaches the framebuffer → blank capture. Fix: run **`xrefresh`**
  (x11-xserver-utils) after the window settles — it repaints the root, forcing an Expose
  on every window so gpui presents. Phase 7's capture step must do the same (or drive an
  equivalent redraw). Related: the spike app quits via a real executor timer
  (`App::spawn` + `background_executor().timer`), NOT a render-loop deadline — with no
  compositor `render` runs only once, so a paint-path deadline never fires.
  (`app/scripts/linux_render_spike.sh`, `crates/freecell-app/src/main.rs`)
- [Phase 1] Linux system deps needed beyond architecture §1's list, found while making
  the app link + render: `libxkbcommon-x11-dev` (link fails on `-lxkbcommon-x11` without
  it), `libfreetype-dev` (the `libfreetype6-dev` name is obsolete on Ubuntu 24.04), and
  `x11-xserver-utils` (xrefresh). All added to `checks.yml` / `perf-gates.yml` /
  `app/README.md`. (`.github/workflows/`, `app/README.md`)
- [Phase 1] Verified end-to-end on the container image (Ubuntu 24.04, Rust 1.95.0):
  `cargo build --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` (freecell-core: 2 unit + 5
  integration dependency-rule-guard tests; freecell-engine: 1 unit — 8 total), the render
  spike, AND `cargo deny check` (cargo-deny 0.19.9) all pass. (`app/`)
- [Phase 1] **cargo-deny: needs human review.** Making the required gate pass surfaced,
  beyond the specced GPL `ztracing` license exception, a set of TRANSITIVE issues from
  gpui/zed's pinned tree with **no safe upgrade** (we do not control zed's lockfile),
  all now documented (not silently skipped) in `app/deny.toml`:
  - Licenses: `bzip2-1.0.6` (permissive, via async_zip) added to `allow`; the GPL
    exception widened to the real crate names `zlog` + `ztracing_macro` (+ `ztracing`),
    which are the ones declaring `GPL-3.0-or-later`.
  - Advisories `ignore`d w/ rationale: `RUSTSEC-2025-0052` (async-std discontinued),
    `-2024-0384` (instant), `-2024-0436` (paste), `-2026-0173` (proc-macro-error2),
    `-2026-0192` (ttf-parser) — all *unmaintained*; and **two quick-xml 0.39.4
    vulnerabilities** `-2026-0194`/`-2026-0195` (DoS on *untrusted* XML, via
    wayland-scanner build dep + zbus/atspi accessibility). FreeCell feeds no untrusted
    XML through those paths; both are fixed in quick-xml ≥0.41, which needs a zed bump.
  - **All of the above (plus the GPL exception) must be re-audited on any gpui rev bump
    and resolved before any binary distribution.** Two non-fatal `no-license-field`
    warnings remain (zed-internal `gpui_shared_string`, `gpui_util`); left for P13.
    (`app/deny.toml`)
- [Phase 1] (post-CR) The strict dependency rule (`architecture §1`) is now
  CI-enforced by a guard test rather than only the hand-written graph:
  `freecell-core/tests/dependency_rule.rs` scans the core + engine manifests and fails
  if `freecell-core` gains a `gpui*`/`ironcalc*` runtime dependency or `freecell-engine`
  gains a `gpui*` one (dev-deps exempt; includes a negative-control test). A `deny.toml`
  ban was considered but rejected: cargo-deny can't scope a ban to a subgraph, and gpui
  is legitimately in the tree via `freecell-app`. (`app/crates/freecell-core/tests/`)
- [Phase 1] (post-CR) Trimmed forward-declared, not-yet-used crate deps to keep the
  manifests honest: dropped `anyhow`/`tracing` from `freecell-app` and
  `ironcalc`/`thiserror`/`tracing` from `freecell-engine` (only `ironcalc_base` is used
  in P1). The version pins stay in the workspace dependency table; each crate re-adds the
  line in the phase that first uses it (noted inline in the manifests). MSRV corrected to
  `rust-version = "1.95"` to match the pin. (`app/Cargo.toml`, `app/crates/*/Cargo.toml`)
- [Phase 2] `Axis`'s sizer bound widened from the POC's `Fn(u32) -> f32` to
  `Fn(u32) -> f32 + Send + Sync` (and the box to `Box<dyn ... + Send + Sync>`) — deviation
  from the ported `poc-core/layout.rs`, forced by reality: the geometry cache holds
  `Arc<Axis>` inside `Arc<RwLock<SheetCaches>>` shared between the worker (writes) and UI
  (reads) threads (`architecture.md §2, §6`), so `Axis` must be `Send + Sync`. All POC
  tests still pass (their sizers are plain fns). Added `Axis::from_overrides` for the
  cache's "default + sparse overrides" geometry. (`app/crates/freecell-core/src/axis.rs`)
- [Phase 2] Input-cap validator is **scoped to formulas** (input starting with `=`);
  non-formula text passes uncapped. Rationale: round-3 D established that only formulas run
  through IronCalc's recursive parser (the sole stack-overflow-abort vector); a long plain
  string is stored, not parsed. `functional_spec.md §3.3` says "reject *formulas* with
  length > 8192 or nesting depth > 64", so this matches the spec's wording. The paren-depth
  scan skips double-quoted string literals (`""` escape) so `="((("` isn't miscounted as
  nesting. Both round-3 D abort reproducers are covered: deep parens → depth cap, long flat
  `=1+1+…` chain → length cap. (`app/crates/freecell-core/src/input_cap.rs`)
- [Phase 2] Keyboard-motion model: `apply_motion(sel, Motion, SheetDims)` with a `Motion`
  enum (Move/Extend/JumpEdge/ExtendEdge/Page/ExtendPage/RowStart/ExtendRowStart). Tab/Enter
  and their Shift variants are **not** distinct `Motion`s — they map to
  `Move(Right/Down/Left/Up)` at the window/keymap layer (their only extra behaviour,
  committing a pending data-row edit, is the data-row reducer's job, not the motion's).
  Cmd/Ctrl+arrow jumps to the **sheet** edge (MVP: not edge-of-data, per §3.2).
  (`app/crates/freecell-core/src/selection.rs`)
- [Phase 2] Data-row reducer: on `EditCommitRequested` (grid click-away) with a
  cap-**rejected** pending formula, the reducer emits `ShowCapError` and does **not** commit
  or leave `Editing` — the window must cancel the pending selection change and keep the
  field editing. Judgment call (the spec covers cap-reject on Enter but not on click-away);
  chosen so an invalid formula can never be silently committed or lost.
  (`app/crates/freecell-core/src/data_row.rs`)
- [Phase 2] `SheetCacheBuilder` (the read-model constructor for fixtures + the Phase-5
  engine builder) interns `RenderStyle`s by **value equality** into the `resolved` table.
  This is a fixture-side intern distinct from the engine's worker-side `StyleInterner`
  (which dedups on the serialized IronCalc `Style`, `components/style_cache.md`); what the
  read model guarantees is a consistent `StyleId → RenderStyle` mapping, which either
  interning path satisfies. (`app/crates/freecell-core/src/cache.rs`)
- [Phase 2] `CellRef::from_a1` implemented (not just `to_a1`, which the read-only ref box
  needs) to make A1 conversion round-trippable/testable per `architecture.md §3`
  ("A1-reference conversion"); it accepts and ignores `$` absolute markers and rejects
  out-of-Excel-max refs. (`app/crates/freecell-core/src/refs.rs`)
- [Phase 3] The file-I/O adapter is `freecell_engine::WorkbookDocument` (new/open/save +
  typed errors), NOT the `DocumentClient` handle from `components/engine_worker.md` — that
  name is the Phase-4 worker handle. `WorkbookDocument` owns the `UserModel<'static>` and is
  what the Phase-4 worker will hold on its thread. `DocumentSource {NewWorkbook, OpenFile}`
  is defined now as specced. The public read API (`sheet_names`, `formatted_value`,
  `cell_content`) returns only `std`/`freecell-core` types — no `ironcalc` type escapes the
  crate; `user_model_mut`/`cell_style` are `pub(crate)`. A compile-time guard asserts
  `WorkbookDocument: Send` (verified) so the Phase-4 thread move is sound.
  (`app/crates/freecell-engine/src/document.rs`)
- [Phase 3] `load_from_xlsx` at 0.7.1 takes **four** args `(path, locale, tz, language)`, not
  the `(path, locale, tz)` in `components/engine_worker.md §File I/O`. The extra `language`
  selects the formula-language pack; the adapter passes the `'static` literal `"en"`
  (`DEFAULT_LANGUAGE`), which is what makes the returned `Model`/`UserModel` `'static`. Not a
  blocker — an added required arg with an obvious value. (`document.rs`)
- [Phase 3] Timezone default is **`UTC`**, not the "system tz" the component doc names. UTC
  is deterministic (only volatile date/time functions like `NOW()`/`TODAY()` depend on it,
  and those are outside the round-trip test scope); reading the OS timezone would need an
  extra crate (`iana-time-zone`) and add non-determinism. System-tz detection is deferred;
  it is a one-line change at `DEFAULT_TIMEZONE` if a real requirement appears. (`document.rs`)
- [Phase 3] Save uses `ironcalc::export::save_xlsx_to_writer(&Model, W)` into a
  `tempfile::NamedTempFile` in the destination dir, then `sync_all` (fsync) + `persist`
  (atomic rename) — **not** `save_to_xlsx`, which hard-errors if the target path already
  exists (it would make every re-save fail). Durability order is file-fsync-then-rename per
  the spec; a parent-directory fsync (to persist the rename itself across power loss) is
  deliberately NOT added — atomicity/no-half-written-target already holds via temp+rename;
  flagged as optional P13 hardening. Added `tempfile = "3"` to the workspace dep table.
  (`app/Cargo.toml`, `document.rs`)
- [Phase 3] Typed open errors are classified by **leading magic bytes** before the load, because
  IronCalc's flat `XlsxError {IO, Zip, Xml, Workbook, …}` cannot by itself separate
  not-xlsx / corrupt / password. Rule: OLE2/CFB `D0CF11E0…` → `PasswordProtected` (encrypted
  OOXML *and* legacy binary `.xls` share this container — the MVP treats both as
  can't-open/protected); `PK…` → treat as a zip and map any subsequent load failure to
  `Corrupt`; anything else (text, empty file, other binary) → `NotXlsx`; an OS open/read
  failure → `Io`. `XlsxError::NotImplemented` (a valid xlsx with unsupported features) maps
  to `Corrupt` with the underlying message preserved — the spec's `LoadError` enum lists only
  `{NotXlsx, Corrupt, PasswordProtected, Io}`, so no new variant was added. (`document.rs`)
- [Phase 3] `SaveError` is split into `{Io, Serialize}` (the component doc only says
  `SaveFailed{reason}`) so a writer-serialization failure is distinguishable from a
  disk/rename failure in the eventual dialog. Both leave the destination untouched. (`document.rs`)
- [Phase 3] The `save_atomic_on_failure` test uses **root-proof** failure injection because the
  build container runs as **root** (verified: `chmod 0555` dir perms are bypassed, so a
  read-only-directory injection would not fail). Two injections used instead: (a) a target in a
  non-existent directory → `ENOENT` on temp creation → `SaveError::Io`, asserts nothing is
  created; (b) a target that is an existing non-empty **directory** → `EISDIR` on the rename →
  `SaveError::Io`, asserts the directory + its sentinel file are byte-identical and no temp
  file leaks (the real "existing file preserved on failed save" invariant). (`tests/roundtrip.rs`)
- [Phase 3] Round-trip tests assert exact engine-formatted strings that were probe-confirmed
  live: currency `1234.5`→`"$1,234.50"`, percent `1`→`"100.00%"`, serial `44197`→`"2021-01-01"`,
  and formula errors `#DIV/0!` / `#CIRC!` survive save→reopen from cached values (no eval on
  open — SP2). If an IronCalc version bump changes any formatted output, these tests flip and
  force a conscious update. The circular-ref fixture uses `pause_evaluation`/`evaluate` (the
  batch API the Phase-4 worker will use) to build the ring once. (`fixtures.rs`,
  `tests/roundtrip.rs`)
- [Phase 3] (post-CR) The engine's `UserModel` re-export is tightened to `pub(crate) use`
  (was `pub use`) — an IronCalc type must not sit on `freecell-engine`'s public surface
  (`architecture.md §2` headless boundary). Nothing outside the crate referenced it, and the
  Phase-4 worker lives inside the crate; `WorkbookDocument` keeps all IronCalc types
  in-crate. Confirmed the crate + all tests still build. (`crates/freecell-engine/src/lib.rs`)
- [Phase 3] (post-CR) The OLE2/CFB magic (`D0CF11E0…`) can be either an encrypted `.xlsx`
  **or** a plain (unencrypted) legacy binary `.xls`; the two can't be told apart cheaply
  (both share the magic — distinguishing needs a CFB directory parse). Rather than add a CFB
  parser, the spec-named `LoadError::PasswordProtected` variant is **kept** but its
  user-facing message is reworded to name both possibilities accurately ("a legacy Excel
  workbook (.xls) or a password-protected/encrypted .xlsx — re-save as modern .xlsx"), so it
  is not inaccurate for a plain `.xls` (`functional_spec.md §5.1`). A future cheap CFB-stream
  probe could split this into a distinct `UnsupportedLegacyFormat` reason.
  (`crates/freecell-engine/src/document.rs`)
- [Phase 3] (post-CR) `save_xlsx_to_writer` errors are now split accurately: an
  `XlsxError::IO` (temp-file write failure) → `SaveError::Io`; any other (structural) error →
  `SaveError::Serialize` (`map_writer_error`). With a healthy model + working temp file the
  pinned writer only fails on I/O, so `SaveError::Serialize` is a **defensive, not reachably
  triggerable** path (documented in code) — a malformed model would be needed and the edit
  APIs prevent that. The real "existing valid `.xlsx` preserved on a failed save" invariant
  is now covered against a **genuine prior workbook** (test
  `failed_save_leaves_real_existing_xlsx_byte_identical`, root-proof ENOTDIR injection),
  since a save to a writable regular-file target cannot fail root-proof by design.
  (`crates/freecell-engine/src/document.rs`, `tests/roundtrip.rs`)
- [Phase 4] `SheetId` on the worker seam is IronCalc's **stable `sheet_id`** (not the
  volatile worksheet index), resolving core's "stable, positional identifier … index↔id
  map" doc (`refs.rs`). Commands / `Publication` / `SheetMeta` all carry the stable id; the
  worker maps it to the current index before each IronCalc call. The component-doc command
  table writes `idx`, but stable ids keep per-sheet UI state (scroll/selection) correct
  across a sheet **delete** (index shift), not just a rename — the more-correct MVP choice
  at the cost of an O(sheets) resolve per op (trivial; few sheets). No reorder API exists
  (round-3 B), so ordering is unaffected. (`worker/protocol.rs`, `worker/run.rs`)
- [Phase 4] `WorkerEvent` uses `async_channel` (2.x) rather than the spec-named
  `smol::channel`. `smol::channel` **re-exports `async-channel`** — same types — so the
  gpui foreground task still `recv().await`s it; using the crate directly keeps the headless
  `freecell-engine` free of the smol async runtime. (`worker/client.rs`, `Cargo.toml`)
- [Phase 4] `Publication.text_color` is published as `None` in this phase. IronCalc's
  formatted-value path exposes number-format colour (`[Red]`-style) only as a **palette
  index** (`Formatted.color: Option<i32>`; 0–6 named + 1–56 indexed), not an RGB — mapping
  it needs the indexed-colour table that belongs with the Phase-5 style cache (which owns
  the colour domain, cf. the fill palette). Display **text** is fully correct; the colour
  override lands with the style cache. (`worker/run.rs build_publication`)
- [Phase 4] `DocumentClient` adds `committed_ops() -> u64` (a shared `AtomicU64`) beyond the
  component-doc interface sketch, mirroring `generation()`. The architecture frames the
  dirty flag as `latest_committed_op > last_saved_op`; exposing the live committed-op count
  (incremented on every applied undoable op, including undo/redo per `architecture.md §2`)
  gives Phase-11 dirty tracking a lock-free read and makes the accounting testable.
  (`worker/client.rs`, `worker/run.rs`)
- [Phase 4] `Command` carries a `#[cfg(test)] TestPanic` variant (never compiled into the
  public build) to inject a panic inside the `catch_unwind`-guarded apply, exercising the
  recovery + degraded policy deterministically from in-crate unit tests. The degraded policy
  implemented: 1st caught panic + responsive probe → `EditRejected{EnginePanic}` and keep
  serving; a 2nd panic **or** an unresponsive probe → `WorkerDegraded`, refuse further edits
  (reads/save still work — the Save-As escape hatch). (`worker/protocol.rs`, `worker/run.rs`)
- [Phase 4] Style toggles (`SetStyleAttr` Bold/Italic/Underline) resolve "any-lacking →
  set-all" by reading current per-cell state **from the engine** (`get_cell_style`) in this
  phase; Phase 5's resident cache turns this into an O(1)-ish map lookup. Ranges are bounded
  user selections. `Fill(Option<Rgb>)` is a direct set (`fill.fg_color = #RRGGBB`) / clear
  (empty string → IronCalc "no fill"). (`worker/run.rs apply_style`, `document.rs`)
- [Phase 4] `LoadError` / `SaveError` gained `#[derive(Clone)]` so the typed reasons ride
  `WorkerEvent::LoadFailed` / `SaveFailed` (they carry only `String`s). (`document.rs`)
- [Phase 4] The worker-side abort-cap test uses cap-**exceeding** reproducers only (depth
  490 > 64; the canonical round-3 D 11,897-term flat chain ⇒ ~23.8k chars > 8192). The
  component doc's "~2832-term" figure is D's abort *ceiling on a small default stack*, which
  is **under** the 8192-char length cap and therefore (correctly) allowed through — a
  cap-passing chain of that size would reach the engine and overflow the small **test-thread**
  stack (the 64 MiB stack is the spawned worker's, not the unit-test thread's). The cap
  eliminates the class *in combination with* the 64 MiB worker stack.
  (`worker/run.rs` cap test, `input_cap.rs`)
- [Phase 4] (post-CR) `SetStyleAttr` no longer forces a recompute: a new `AppliedKind::StyleOnly`
  applies the style + counts a committed op + publishes (a repaint; P5 ships cache deltas) but
  skips `evaluate()` — styles don't affect values (component-doc command table). Only
  value/undo/redo/clear (`Cell`) and sheet ops (`SheetOp`) recompute.
  (`worker/run.rs apply_one`, `apply_edit_batch`)
- [Phase 4] (post-CR) The published viewport is capped to a bounded overscan window
  (`MAX_PUBLISH_ROWS=512 × MAX_PUBLISH_COLS=256` = 131,072 cells worst case) in addition to the
  sheet-bound clamp, so a pathological full-sheet `SetViewport` can't wedge the worker in a
  billions-of-cells probe (the build-publication loop is the robustness boundary and is not
  inside `catch_unwind`). The bounds are sized to comfortably exceed a ~3× overscan of the
  largest supported display (a 4K screen ⇒ ~300 rows × ~180 cols), so overscan pre-fetch is
  never clipped in practice; a clipped overscan would only render blank and self-heal on the
  next `SetViewport`. **Phase 6/7 must cross-check** `MAX_PUBLISH_ROWS`/`MAX_PUBLISH_COLS` keep
  margin over the real overscan dimensions once the grid exists. (`worker/run.rs clamp_viewport`)
- [Phase 4] (post-CR) The worker now emits the SP1 apply/eval/publish observables as `tracing`
  debug events (`apply_us` / `eval_us` on the coalesced batch; `publish_us` + cell count in
  `publish`), so `tracing` is a live dependency and Phase 12's perf harness can read the
  timings (`architecture.md §8`). (`worker/run.rs`)
- [Phase 4] (post-CR) The caught-panic recovery's `resume_evaluation()` is itself wrapped in
  `catch_unwind` — a poisoned model could panic on that call, and recovery must never unwind
  out of `run()` and kill the thread. (`worker/run.rs apply_edit_batch` Err arm)
- [Phase 4] (post-CR) `process_batch` command routing is an exhaustive match (control arms +
  an or-pattern of the edit variants, no catch-all), so a future `Command` variant must be
  explicitly classified and can't silently fall through to the apply path.
  (`worker/run.rs process_batch`)

- [Phase 5] **IronCalc geometry getters return pixels, not raw units/points** as
  `components/style_cache.md` assumed. `get_row_height`/`get_column_width`
  (`ironcalc_base/src/constants.rs`: "COLUMN_WIDTH and ROW_HEIGHT are pixel values"; defaults
  125 px col / 28 px row) already apply IronCalc's own COLUMN_WIDTH_FACTOR/ROW_HEIGHT_FACTOR.
  FreeCell's chosen grid defaults are 100 px / 24 px (`ui_design.md §3.3`). The single
  conversion (`cache::col_px`/`row_px`) scales an override by `freecell_default /
  ironcalc_default` (0.8 col, 6/7 row), so a track at IronCalc's default maps exactly to the
  FreeCell default and deviations scale proportionally. The exact scale may want calibration
  against real render fidelity in Phase 7. (`freecell-engine/src/cache.rs`)
- [Phase 5] **The style interner dedups by `RenderStyle` (in `freecell-core`), not by a
  serialized IronCalc `Style` (in `freecell-engine`)** as the component doc's data-structure
  sketch phrased it. Rationale: the *MVP read model holds `RenderStyle`*, which is `Eq + Hash`,
  so it is a direct map key (no serialization needed — the doc's serialize-because-not-Hash was
  for a full-`Style` cache). The engine still owns the IronCalc-touching part (the
  `Style → RenderStyle` conversion in `cache::render_style_from`); `freecell-core` owns the
  render-form dedup. Two distinct engine `Style`s that differ only in fields the grid ignores
  (borders/font size/…) collapse to one `RenderStyle`/`StyleId` — correct for rendering and
  keeps the resolved table minimal. (`freecell-core/src/cache.rs`, `freecell-engine/src/cache.rs`)
- [Phase 5] **`SheetCache` is mutable in place**, not "immutable once built" as the doc phrased
  it: in-place mutators (`set_cell_style`/`clear_cell_style`, band + geometry setters) let
  mirror-on-edit touch only the changed cells under the write lock instead of rebuilding the
  whole sheet — the cheap forward path the doc's lifecycle §2 intends. Geometry setters rebuild
  the affected `Axis` (immutable). The interner never GCs unused `StyleId`s (bounded distinct
  styles; matches the builder). (`freecell-core/src/cache.rs`)
- [Phase 5] **Style→RenderStyle mapping choices:** IronCalc's default font colour is pure black
  (`#000000`); black (and absent/unparseable) maps to `RenderStyle.font_color = None` (= the
  grid's default near-black), so plain cells intern to the default style. `num_format_is_default
  = num_fmt.eq_ignore_ascii_case("general")`. Fill = `fill.fg_color` when present (a cleared
  fill leaves `fg_color = None`). Only Left/Center/Right alignment map to `Some`; General and the
  unimplemented variants → `None`. (`freecell-engine/src/cache.rs render_style_from`)
- [Phase 5] **Build-on-activation reproduces IronCalc's cell-vs-band shadowing exactly**: a cell
  present in `sheet_data` uses its *own* style (via `get_cell_style_or_none`), even the default,
  which shadows any band (matching `Model::get_cell_style_index`); an absent cell falls through
  to the row (gated on `custom_format`) then col band. A populated cell with default own style on
  a non-default band gets an explicit default entry to shadow it. Micro-edge not handled: a
  `custom_format` row whose style resolves to default while shadowing a non-default *col* band —
  not reachably produced by the edit APIs (setting a row style to index 0 clears `custom_format`).
  (`freecell-engine/src/cache.rs build_sheet_cache`, `refresh_cell`)
- [Phase 5] **Undo/redo touch-sets are aligned 1:1 with IronCalc's undo history.** The worker
  keeps `undo_touches`/`redo_touches` stacks (`Touch::Cells{sheet,range}` for cell edits,
  `Touch::Sheets` markers for sheet ops) so a pop re-reads exactly the reverted op's cells.
  Sheet-op undo/redo is handled by comparing the sheet-id set before/after and dropping caches
  for absent sheets (a returning sheet rebuilds lazily on activation) rather than replaying the
  marker. A rejected edit creates no engine history entry and pushes no touch, so the stacks stay
  aligned. (`freecell-engine/src/worker/run.rs mirror_applied_ops`)
- [Phase 5] `StyleCacheUpdated{sheet}` is emitted on load, on sheet activation, and after any
  cell-edit batch that mirrored the active sheet (this slightly over-emits on pure value edits —
  a cheap, always-correct "re-read the cache" signal). `SheetsChanged` is now driven by a
  sheet-list value comparison (so undo/redo of an add/rename/delete re-syncs the tab bar too), a
  small enhancement over Phase 4's applied-kind flag. A pathological mirror range (> 100k cells)
  falls back to a full active-sheet rebuild. (`freecell-engine/src/worker/run.rs`)

- [Phase 5] (post-CR) **Band-creating style edits force a full cache rebuild, detected by range
  shape — not the cell-count cap.** A `SetStyleAttr` spanning all columns of a row (or all rows of
  a column) makes IronCalc's `update_range_style` take its full-rows/full-columns branch and set a
  ROW/COLUMN BAND (`ironcalc_base/src/user_model/common.rs`), which the per-cell `refresh_cell`
  mirror structurally cannot create. A single full-row band is only 16,384 cells — below
  `MAX_REFRESH_CELLS` (100k) — so the old cap-only guard let it slip to the per-cell path and rot
  the cache (empty banded cells stayed default). Fix: `is_band_creating(range)` → full
  `build_and_store_cache`. Full columns (1M rows) were already caught by the cap; now covered
  explicitly too. (`worker/run.rs refresh_cache_cells` + `is_band_creating`)
- [Phase 5] (post-CR) **Row-height auto-fit is mirrored on value edits (not worked around by
  stripping newlines).** IronCalc's `set_user_input` grows a row's height when the content is
  taller than the current height (multi-line, or *any* input on a row currently shorter than
  ~21px) — so stripping newlines alone would NOT uphold the agreement contract for all inputs. The
  mirror path re-reads the touched rows' heights (`cache::row_override_px`) and applies them via
  the batched `SheetCache::set_row_heights` (one axis rebuild), so multi-line input and its undo
  stay in agreement with the engine — with no silent alteration of user data. The MVP grid still
  renders a single clipped line (a Phase-6 concern), independent of the cache's correct geometry.
  (`worker/run.rs refresh_cache_cells`, `freecell-core/src/cache.rs set_row_heights`,
  `freecell-engine/src/cache.rs row_override_px`)

- [Phase 5] (post-CR2) **A failed cache rebuild DROPS the sheet's entry (never leaves it stale)**
  and reports failure, so no `StyleCacheUpdated` is announced for it. A full-rebuild replaces a
  pre-edit cache; leaving the old one on a build failure would make the grid re-read a stale cache
  (the divergence this phase prevents). `build_and_store_cache` now returns `bool` and `remove`s
  the entry on `Err`/unresolvable sheet (mirroring how activation already stays absent on
  failure); `refresh_cache_cells` skips the `touched`/emit for a failed rebuild. The build `Err`
  path is effectively unreachable today (getters only fail on an invalid sheet index, resolved
  first), so the added test exercises the reachable unresolvable-sheet proxy; the invariant is
  documented on `build_and_store_cache`. (`worker/run.rs`)

- [Phase 6] **Selection accent is pinned to blue-600 `#2563EB`, not "the gpui-component
  primary token".** `ui_design.md §3.3` says the selection accent = "gpui-component primary
  blue", but the pinned gpui-component default theme's `primary` is **neutral** (`neutral-900`
  ≈ near-black, `default-theme.json`), not blue — a spreadsheet selection drawn in it would be
  black. Pinned the accent to the theme's own blue ramp (`blue-600 = #2563EB`,
  `default-colors.json`) so it reads as a spreadsheet selection and still comes from
  gpui-component's palette. (`app/crates/freecell-app/src/grid/mod.rs` `ACCENT`)
- [Phase 6] **The grid inherits gpui's default UI font in Phase 6; bundled Inter lands at
  startup (Phase 10).** `ui_design.md §3.3` specs 13 px bundled Inter, registered via
  `add_fonts` at app startup. Bundling the font file + `add_fonts` is an app-shell concern
  (Phase 10); Phase 7's render suite needs it for pixel-stable baselines. Phase 6 sets the cell
  (13 px) / header (11.5 px) sizes + weights but not `font_family`, so text renders on the
  default font with no fallback-resolution risk. `GRID_FONT_FAMILY = "Inter"` is reserved in one
  place for when the bundle lands. (`grid/mod.rs`, `grid/view.rs`)
- [Phase 6] **Default cell alignment is Left (not type-aware).** `components/grid.md` says
  "align per style/type" (text left, numbers/dates right, …), but `Publication`/`PublishedCell`
  carries only a pre-formatted display string, not the cell's value type, and a General-format
  number resolves to `RenderStyle.h_align = None`. So a number is left-aligned unless the file's
  style sets an explicit alignment. Type-based default right-alignment needs the engine to
  publish the value type (or a resolved alignment) in the publication — deferred to Phase 11
  engine wiring. Explicit `h_align` renders correctly today (verified: B4 `1234.5` right-aligned
  via a fixture style). (`grid/view.rs` `cell_element`)
- [Phase 6] **Nested clipped containers instead of the POC's flat draw-order.** The port renders
  the content (cells + selection) inside an `overflow_hidden` container sized to the content area,
  and each header strip inside its own clipped container, rather than the POC's single flat root
  that relied on drawing fixed headers last to cover scrolled content. Cleaner and avoids
  header/content z-fighting or label bleed into the gutter; the virtualization math is identical.
  (`grid/view.rs` `render`)
- [Phase 6] **Visible styles are snapshotted under the read lock, then the lock is released
  before painting.** `components/grid.md §Render pass` says clone the two `Arc<Axis>` so the guard
  drops "immediately". Resolved styles (`render_style`) still need the cache, so the render path
  holds the read lock a moment longer to copy the visible cells' `RenderStyle` (`Copy`, bounded by
  visible-cell count ≈ a few thousand) into a reused buffer, then drops the lock — so **no lock is
  held while painting**, which is the invariant that matters (`architecture.md §4`). One lock per
  frame (no re-entrant read locks, to avoid a parking_lot reader/writer starvation deadlock).
  (`grid/view.rs` `resolve_frame`)
- [Phase 6] **`GridEventSink` is a boxed `Fn(&GridEvent, &mut Window, &mut App)`.**
  `components/grid.md` names `GridEventSink` in the constructor but doesn't define it. Chose a
  boxed closure with full `Window`/`App` access (over gpui's `EventEmitter`) so the Phase-11
  window can forward `ViewportChanged` to the worker and drive the data row on `SelectionChanged`
  from one handler. Phase 6 emits only `ViewportChanged` (from the scroll path — it is naturally
  coupled to scroll and debounced on the visible-index range); selection/commit events + the full
  input wiring are Phase 8. (`grid/mod.rs` `GridEventSink`, `grid/view.rs` `handle_scroll`)
- [Phase 6] **`set_active_sheet` takes `cx` only and does not emit on switch (Phase 6).**
  `components/grid.md` has it "emit ViewportChanged so the worker re-publishes"; that re-publish
  wiring is Phase 11 (there is no worker in Phase 6). It swaps the per-sheet scroll/selection maps
  and clears `last_viewport` so the next scroll/publish re-announces the viewport. Generation-driven
  repaint (`WorkerEvent::Published` → `notify`) is also Phase 11. (`grid/view.rs`)
- [Phase 6] **`freecell-app` gained a `[lib]` target** (`freecell_app`, `src/lib.rs` → `pub mod
  grid`) so `render-tests` (Phase 7) and the perf harness (Phase 12) can render the real
  `GridView` over fixtures, per the `architecture.md §1` crate rule (`render-tests → freecell-app
  (grid)`). The `freecell` bin now `use`s the lib. Added `arc-swap` + `parking_lot` (workspace
  pins) to the app manifest for `GridDataSources`. (`app/crates/freecell-app/Cargo.toml`,
  `src/lib.rs`)
- [Phase 6] **`row_header_width` estimates the gutter from a per-digit width (7.5 px), not a glyph
  measurement.** The gutter only needs to comfortably fit the deepest visible row's label; a px of
  over-estimate is harmless and keeps the width a pure, unit-tested function (no text-system
  dependency). Floored at 48 px, widens for 7-digit Excel-max labels. (`grid/layout.rs`)
- [Phase 6] **`scroll_cell_into_view` is implemented now (a `pending_reveal` applied on the next
  render) though the plan lists it under Phase 8.** The pure `scroll_to_reveal` math is trivial and
  unit-tested; wiring the method keeps the public interface complete. Keyboard/mouse *drivers* of it
  (and edge auto-scroll) remain Phase 8. (`grid/view.rs`, `grid/layout.rs`)
- [Phase 6] **Cross-check of the Phase-4 publish bounds (as that note requested):**
  `RENDER_OVERSCAN = 2` (the grid's own tiny overscan) and the worker's `MAX_PUBLISH_ROWS=512 ×
  MAX_PUBLISH_COLS=256` comfortably exceed a real overscanned viewport (a 4K display ≈ 90 visible
  rows × 38 cols; ×3 worker overscan ≈ 270 × 114), so overscan is never clipped in practice. Margin
  confirmed. (`grid/layout.rs`, cf. `worker/run.rs clamp_viewport`)
- [Phase 6] **Visual verification: PASSED.** The Linux render spike (Xvfb + lavapipe + xrefresh)
  captured the real grid over `demo_sources()` to a non-blank PNG (2082 colors): headers with
  selected-row/col tint + accent edge, gridlines, bold/italic/underline/fill/right-aligned cells,
  a clipped long string, the B2:D4 selection with its 10% overlay + white anchor at D4, and the
  wide-B / tall-row-3 variable geometry all render correctly. (`app/scripts/linux_render_spike.sh`)

- [Phase 7] **Capture = Phase-1 spike option 2, refined: per-case Xvfb sized to the viewport +
  capture the window by id.** A load-bearing finding beyond the spike: gpui/lavapipe (backend is
  `gpui_wgpu`, not blade, at this rev) only *presents* a window's frame to the framebuffer when the
  window **nearly fills the screen** — a small window on a large screen captures blank (verified:
  480×160 on 1400×900 = blank both via root and by-id; on 488×168 = 534 colours). So each case
  renders under its **own** `xvfb-run` display sized to `viewport + 8px`, and the harness finds the
  grid window (`xwininfo` by `WxH`) and captures it with `import -window <id>` (clean, no crop). The
  xrefresh-forces-Expose-so-gpui-presents trick from Phase 1 still applies. Subprocess-per-case
  (the `render_scene` bin) = clean gpui lifecycle, no resize API, no stale-pixel races.
  (`render-tests/src/capture.rs`, `render-tests/src/bin/render_scene.rs`)
- [Phase 7] **The scene builder drives the REAL worker** (`DocumentClient(NewWorkbook)` →
  `SetCellInput` / `SetStyleAttr` → `SetViewport` → drain → read `Publication` + `SheetCaches`). Two
  render features have **no MVP worker edit command** and are applied to the real `SheetCache` the
  grid consumes (its public mutators, the same the worker uses), after the worker builds it:
  (a) column/row **geometry** (`cell_tall_row`, `cell_wide_column`, `grid_variable_geometry`);
  (b) explicit **alignment** + **font colour** (`cell_align_*`, `cell_fill_dark_text_contrast`). In
  the product these arrive from an opened file, not an edit — there is no "more real" path to drive
  them at this rev; this mirrors how Phase 6 itself tested alignment/geometry. Values/formats/errors
  and bold/italic/underline/fill are fully engine-driven. (`render-tests/src/scene.rs`)
- [Phase 7] **Number formats are driven by IronCalc input inference, not a `num_fmt` command**
  (the worker has none). Probed against the pinned engine: `set_user_input` infers currency
  (`"$1,234.50"`→`$1,234.50`), percent (`"50%"`→`50%`), thousands (`"1,234,567"`), and date
  (`"2021-01-01"`) from the input string, exactly Excel-like. Guarded by the
  `scene_number_formats_infer` unit test so an engine bump that changes inference flips a test.
  (`render-tests/src/lib.rs`, `render-tests/src/scene.rs`)
- [Phase 7] **`cell_number_negative_red`: the `[Red]` number-format COLOUR path is still deferred.**
  The worker publishes `PublishedCell.text_color = None` (Phase-4 decision — the palette-index→RGB
  mapping is future work), so this baseline shows the negative number correctly *formatted* in the
  default colour, not red. The case stays in the table so the feature is tracked; its baseline
  updates when `text_color` is wired. (`render-tests/src/cases.rs`, README note)
- [Phase 7] **Perceptual-diff thresholds RESOLVED: kept at round-3 C's 12/255 per-channel & 0.5%
  fraction** (the "thresholds after first real baselines" placeholder). With real lavapipe baselines
  in hand the whole suite re-renders and passes deterministically at these defaults (47 tests green,
  ~222 s), so no re-tune was needed. They may be *tightened* later if lavapipe proves bit-exact
  (never loosened). Both constants live in one place, `DiffOptions::default()`.
  (`render-tests/src/diff.rs`, README "Tolerance constants")
- [Phase 7] **The pixel suite is gated on `FREECELL_RENDER=1` (+ capture tooling present), separate
  from `cargo test --workspace`.** Without the env var the render integration test skips (the
  GPUI-free perceptual-diff unit tests still run), so a plain `cargo test --workspace` needs no
  display and the desktop-dev footgun (rendering on a real GPU vs lavapipe baselines) is avoided. CI
  runs the **required** pixel gate as a dedicated step (`render-tests/scripts/render_tests.sh test`)
  that sets the env var; the harness self-manages the per-case Xvfb (no single ambient `xvfb-run`
  wrapper). This replaces the Phase-1 informational render-spike step in `checks.yml`.
  (`render-tests/tests/render_suite.rs`, `render-tests/scripts/render_tests.sh`, `.github/workflows/checks.yml`)
- [Phase 7] **CI system-dep added: `x11-utils` (provides `xwininfo`)** — the capture path finds each
  case's window by id via `xwininfo`, which was NOT in the Phase-1 apt list (only `x11-apps` +
  `x11-xserver-utils`). Added to `checks.yml`. `convert` is no longer used (capture-by-id needs no
  crop); `import` (imagemagick) + `xrefresh` (x11-xserver-utils) + `xvfb-run` (xvfb) are the rest.
  (`.github/workflows/checks.yml`)
- [Phase 7] **`freecell-app` added to the workspace dependency table** so `render-tests` can depend
  on its `[lib]` (`freecell_app`) to render the real `GridView` (`architecture.md §1` crate rule
  `render-tests → freecell-app (grid)`). `render-tests` gained `gpui`/`gpui_platform`/`gpui-component`/
  `freecell-engine`/`image`/`arc-swap`/`parking_lot` deps + two bins (`render_scene`,
  `generate_baselines`); all are already in the tree, so `cargo deny` stays clean.
  (`app/Cargo.toml`, `render-tests/Cargo.toml`)
- [Phase 7] **Initial suite = 45 cases, all committed baselines generated on the pinned image and
  visually spot-checked** (montages): text attrs, fills (incl. a 2×2 fill covering gridlines),
  engine-formatted numbers/currency/percent/date, #DIV/0! / #NAME? / #CIRC! errors, alignment,
  clipping, tall-row/wide-column/variable geometry, deep-scrolled headers (rows 490–501, cols
  Z–AE), single/range/edge-spanning selection, the loading overlay, forced scrollbars, and a busy
  `grid_mixed_content` canary whose formula totals (`=B2*C2` → `$13.50`, …) evaluate through the
  real engine. The full human eyeball-sweep of all baselines is Phase 13. (`render-tests/baselines/`)
- [Phase 7] **macOS pixel capture stays a documented, UNIMPLEMENTED fallback.** The Phase-1 Linux
  spike PASSED, so the Linux Xvfb+lavapipe suite is the primary + only implemented pixel gate; the
  round-3 C macOS offscreen-Metal capture function is not written (it would only be needed if the
  Linux path regressed, and can't be validated in the Linux container). `render-tests` still builds
  on macOS and its GPUI-free perceptual-diff unit tests run there; the pixel cases self-skip
  (no lavapipe/xvfb). The `macos-verify` placeholder step was updated to say so.
  (`.github/workflows/macos-verify.yml`, `render-tests/src/capture.rs`)

- [Phase 7] (post-CR) **The loading spinner is FROZEN to a static loader icon under a render-test
  capture, gated by a `GridView` flag (not an env var/cfg).** The animated `gpui_component`
  `Spinner` rotates by wall-clock elapsed time between first paint and the xrefresh-forced capture
  (~3.5 s ± jitter), so the `grid_loading_overlay` baseline was non-deterministic. Chose a
  `GridView::set_freeze_spinner` flag (mirroring the existing `force_scrollbars` render-test hook)
  over reading `FREECELL_RENDER` inside `view.rs`: it keeps the app crate free of any test-harness
  env-var knowledge (cleaner layering) and is explicit/testable. The render harness sets it on every
  capture (`render.rs`); the normal app leaves it off and keeps the animated spinner. The frozen
  render matches the old animated baseline within tolerance (the icon is tiny vs the 640×320
  viewport, the reviewer's own point), but the baseline was regenerated so it is now deterministic:
  the case passed the full suite and two back-to-back frozen renders both diffed `unchanged`
  (< 0.5 %). Only `grid_loading_overlay.png` was regenerated. (`grid/view.rs`, `render-tests/src/render.rs`,
  `render-tests/baselines/grid_loading_overlay.png`)
- [Phase 7] (post-CR) **A required pixel gate can no longer silently self-skip to green.** When
  `FREECELL_RENDER=1` (operator explicitly wants the pixel suite) but `capture_available()` is false,
  the suite now **FAILS** instead of skipping. Factored the policy into a pure `gate(want_render,
  capture)` fn (Skip / Fail / Render) so it is unit-tested without the process-global env var +
  `OnceLock` (three new tests, run on every `cargo test --workspace`). The `capture_available()`
  skip is kept ONLY for the implicit path (`FREECELL_RENDER` unset — `cargo test --workspace` /
  macOS). Belt-and-suspenders: `render_tests.sh` also asserts the tools (`xvfb-run`, `xrefresh`,
  `xwininfo`, `import`, lavapipe ICD) exist before invoking cargo and exits non-zero otherwise, for a
  clear early operator message even when the script is used directly.
  (`render-tests/tests/render_suite.rs`, `render-tests/scripts/render_tests.sh`)
- [Phase 7] (post-CR) **`scene.rs drain_to_idle` escalates the `DRAIN_CAP` (10 s) path to a hard
  error** instead of returning `Ok(())` and rendering possibly-incomplete data. Keyed "done" off an
  explicit publish signal was considered but not adopted: `WorkerEvent::Published` is a unit variant
  carrying no generation, so precise keying would need publish-counting against coalescing, whereas
  the 200 ms idle-gap drain works reliably (all 45 baselines) and now fails loudly on a genuine
  worker fault rather than silently proceeding. (`render-tests/src/scene.rs`)
- [Phase 7] (post-CR) **`capture.rs unique_colors` comment corrected to match the guard (not the
  guard strengthened).** The guard rejects a capture with `<= 1` distinct colour (a failed present is
  uniform); the stale comment claimed "a real grid has many colours". Fixed the comment rather than
  raising the threshold, because a higher bar risks false-failing legitimately sparse captures and
  the 45-case suite is green at the current bar; the real failure mode this guards (window didn't
  present) is a single uniform colour, which the `>= 2` check catches. (`render-tests/src/capture.rs`)
- [Phase 7] (post-CR) **`diff.rs` module doc reworded from "ported verbatim" to "a faithful
  refactor" of round-3 C** — the metric and both tolerance constants are identical, but the code
  extracts `pixel_delta` and adds `diff_image` (the magenta failure visualization), so "verbatim"
  was inaccurate. Doc-only. (`render-tests/src/diff.rs`)

- [Phase 8] **Keyboard is wired via `on_key_down` + a pure `command_for_key` mapper, NOT GPUI
  `Action`/keymap registration.** `components/grid.md §Public interface` says "the grid registers
  GPUI key bindings"; instead the grid handler reads `keystroke.key` + `modifiers` and calls a
  gpui-free `grid::input::command_for_key(key, shift, secondary, page_rows) -> Option<GridKeyCommand>`
  (unit-tested headless). Rationale: it keeps the key→motion map **pure + unit-tested** (the
  manager's explicit requirement), avoids app-global `bind_keys` registration that the render-test
  harness / demo don't need, and resolves Cmd-on-mac / Ctrl-on-linux uniformly via
  `Modifiers::secondary()` (no per-platform keymap files). The window-level shortcuts (Cmd+B/I/U,
  undo/redo, menu actions) still bind as actions in later phases; those aren't the grid's.
  (`grid/input.rs`, `grid/view.rs handle_key_down`)
- [Phase 8] **Added `Motion::DocumentStart` / `Motion::ExtendDocumentStart` to `freecell-core`.**
  `ui_design.md §6` maps Cmd/Ctrl+Home → cell A1, which no existing `Motion` could express
  (`JumpEdge(Up)` keeps the column). Added the two variants (collapse / keep-anchor to A1) so
  every keyboard selection change still flows through the single `apply_motion` transformer,
  keeping it the pure, testable dispatch point. Cmd/Ctrl+Shift+Home (extend to A1) is wired for
  symmetry with RowStart/ExtendRowStart though §6 lists only Home/Cmd+Home. (`selection.rs`)
- [Phase 8] **Added `GridEvent::ClearCells(CellRange)`.** `components/grid.md §Input` says
  "Delete emits a ClearCells request via the event sink", but the Phase-6 `GridEvent` enum had no
  such variant. Added it carrying the selection range (sheet-less); the window supplies the active
  `SheetId` → `Command::ClearCells` (which already exists in the worker protocol) in Phase 11. The
  grid does not clear anything itself (no engine access). (`grid/mod.rs`, `grid/view.rs`)
- [Phase 8] **Edge auto-scroll is a `cx.spawn_in(window, …)` timer loop polling
  `window.mouse_position()`**, not `request_animation_frame`. The manager flagged that a
  held-at-the-edge drag emits no mouse-move events, so a frame/timer must drive it. `spawn_in`
  gives an `AsyncWindowContext` (→ `update_in` yields `&mut Window`), so each 16 ms tick reads the
  live pointer, applies the fixed `EDGE_AUTOSCROLL_STEP_PX` (20 px, clamped via `clamp_scroll`),
  re-extends the selection to the hovered cell (`cell_at_point`), and emits debounced
  `ViewportChanged`. An epoch guard stops the loop on drag-end; the loop self-stops when the
  pointer returns inside the content (delta 0). `request_animation_frame` was rejected: it doesn't
  present headless under Xvfb and couples auto-scroll to the render cadence. (`grid/view.rs`
  `maybe_start_autoscroll` / `autoscroll_tick`)
- [Phase 8] **Mouse event positions are treated as grid-local (window == grid) with the same
  PHASE 11 caveat as `handle_scroll`/`render`.** The grid is full-window in Phase 8 (demo +
  render harness), so `MouseDownEvent.position` (window coords) is grid-local. Once chrome wraps
  the grid (Phase 11), `event_local` must subtract the grid element's laid-out origin — flagged
  inline, matching the existing `viewport_size()` notes. (`grid/view.rs event_local`)
- [Phase 8] **Keyboard motions reveal the active cell immediately + announce `ViewportChanged`
  (`reveal_and_announce`), rather than deferring to the render-time `pending_reveal`.** The
  Phase-6 `scroll_cell_into_view` sets a pending reveal applied in `resolve_frame` but does NOT
  emit `ViewportChanged` (fine when there was no worker). A keyboard scroll must re-publish, so the
  key handler computes the reveal-scroll eagerly and emits the debounced `ViewportChanged` (mirrors
  `handle_scroll`). `pending_reveal` stays for the public `scroll_cell_into_view` + render-test
  `reveal` hook. (`grid/view.rs reveal_and_announce`)
- [Phase 8] **The three new selection render cases set the post-interaction state directly (via
  the existing `selection`/`reveal` harness hooks), not by injecting synthetic input events.** The
  drag/shift/scroll *logic* is unit-tested (`cell_at_point`, `edge_autoscroll_delta`,
  `command_for_key`, `apply_motion`); the render cases capture the real `GridView` selection layer
  for the resulting states — `grid_selection_shift_extended` (active at the range's top-left),
  `grid_selection_drag_extended` (larger block, active bottom-right), `grid_selection_scrolled`
  (top-left clipped off-screen, the complement of `grid_selection_range_spans_edge`). This mirrors
  Phase 7's choice to set selection directly rather than drive it through a (non-existent) worker
  command. Baselines generated on the pinned image, eyeballed, committed; full suite green
  (53 tests, 235 s). (`render-tests/src/cases.rs`, `render-tests/tests/render_suite.rs`,
  `render-tests/baselines/grid_selection_{shift_extended,drag_extended,scrolled}.png`)

- [Phase 8] (post-CR) **Edge auto-scroll gained an inward HOTZONE inset so it can START at the
  right/bottom edges — the initial version could never trigger there.** gpui delivers
  `on_mouse_move` only while the pointer is inside the (full-window) grid element, but the content's
  right/bottom edges coincide with the window edge, and the first `edge_autoscroll_delta` only
  returned a positive step *strictly past* those edges — exactly where no move event fires. So
  dragging past the right/bottom never auto-scrolled (top/left worked incidentally, over the header
  strips). Fix landed in the PURE, unit-tested `edge_autoscroll_delta`: it now returns the step when
  the pointer is within `EDGE_AUTOSCROLL_HOTZONE_PX` (24 px, ~a cell) INSIDE each edge, so the loop
  launches from a real move event; the running loop still re-reads the unclamped out-of-window
  pointer. New test `edge_autoscroll_delta_starts_inside_hotzone`. Excel-like feel (scroll begins as
  the pointer nears an edge). The capture-based hot-zone was chosen over pointer-capture-on-mousedown
  (simpler + testable). (`grid/layout.rs`, `grid/mod.rs EDGE_AUTOSCROLL_HOTZONE_PX`)
- [Phase 8] (post-CR) **Mild CR items:** keyboard motions now emit `SelectionChanged` + reveal ONLY
  when the motion actually moved the selection (a no-op at a sheet edge changes nothing), matching
  the change-guarded drag/auto-scroll paths; `page_rows` (a caches read) is resolved lazily, only for
  `pageup`/`pagedown`, so every other keystroke stays lock-free; the `handle_mouse_up` auto-scroll
  restart gap (`autoscrolling` stays set until the running loop's next ≤16 ms tick) is documented as
  the deliberate "one loop live" trade-off; and `cell_at_point`'s benign inclusive-edge clamp is
  noted in a comment. (`grid/view.rs`, `grid/layout.rs`)

- [Phase 9] **Chrome ↔ engine seam = a `ChromeClient` trait** (`send(Command)` +
  `render_style(sheet, cell) -> Option<RenderStyle>`), implemented for the real
  `DocumentClient` and by a `RecordingClient` double. `components/app_shell.md` sketched the
  chrome holding a concrete `DocumentClient`; the trait keeps Phase 9 fully headless and lets
  Phase 11 drop the real client in unchanged. Chrome→grid coupling (move active cell / focus
  grid / switch sheet) is a second seam, `ChromeGridSink` (a boxed closure like
  `GridEventSink`) delivering `ChromeGridRequest`. (`chrome/client.rs`, `chrome/mod.rs`)
- [Phase 9] **Added `freecell-core::eval_indicator`** — the action-row evaluating-spinner
  250 ms no-flash timer as a pure `reduce(event) -> Vec<Effect>` reducer (mirrors the
  data-row fetch spinner), table-tested headless. `app_shell.md` places the eval-spinner
  logic in the window; extracting it to core keeps the timing (short-never-flashes,
  long-shows-then-hides, coalesced-stays-shown, stale-timeout-noops) unit-tested without
  gpui, and the GPUI layer just arms a timer on `ArmTimer`. (`freecell-core/src/eval_indicator.rs`)
- [Phase 9] **Fill popover, tab context menu, and delete-confirm modal are lightweight
  `ChromeView`-owned panels, NOT the stock gpui-component `Popover`/`ContextMenu`/`Modal`.**
  Those widgets' content closures run in a *foreign* entity context (`PopoverState` /
  `PopupMenu`) and `PopupMenu` items dispatch app-global gpui `Action`s — both would force
  cross-entity dispatch / an action registry for what `ui_design.md` calls a
  "don't-over-invest" chrome surface. The custom panels are controlled by view state
  (`fill_open` / `context_menu` / `confirm_delete`), so every action is a directly-testable
  method. Stock gpui-component `Button`, `Input`/`InputState`, `ColorPicker` (the fill
  "Custom…" entry), and `Spinner` ARE used as specced. (`chrome/view.rs`)
- [Phase 9] **The chrome keeps its own `SheetTab { id, name, has_content }` mirror; the
  worker's `SheetMeta { id, name }` has no `has_content`.** `app_shell.md` says "the worker
  includes `has_content` in `SheetMeta`" (gates the delete-confirm modal), but Phase 4 built
  `SheetMeta` without it. Rather than change the engine seam from Track C, the chrome carries
  `has_content` on its own view-model (defaults false; `merge_sheet_metas` preserves a known
  value across a `SheetsChanged`). **Phase 11 must source `has_content`** — either add the
  field to `SheetMeta` (worker populates it) or a per-sheet content query — before the
  delete-confirm rule is correct against the real worker. (`chrome/mod.rs`, `chrome/view.rs`)
- [Phase 9] **gpui `test-support` feature enabled as a `freecell-app` dev-dependency** so the
  chrome interaction tests can drive `ChromeView` through the headless `TestAppContext` /
  `VisualTestContext` (additive feature; the release binary is unaffected). Same knob
  gpui-component uses for its own widget tests. (`crates/freecell-app/Cargo.toml`)
- [Phase 9] **Controlled content field:** `InputState` owns the text buffer; the `DataRow`
  reducer is the state machine. Widget `Change` → `Edited`; reducer text → widget via
  `set_value` (verified to suppress the `Change` event, so no feedback loop). A late
  `CellContent` reply is synced to the widget only while the field is Idle, never mid-edit —
  so a stale reply can't reset the caret. Escape is caught by an `on_key_down` on the
  data-row container (the `InputState` propagates Escape). Shift+Enter remaps the reducer's
  `MoveActive(Down)` to `Up` in the GPUI layer (the reducer's Commit hardcodes Down); a
  Tab-commit in the formula bar is deferred (a single-line `Input` emits no commit on Tab —
  minor, revisit with the in-cell editor). (`chrome/view.rs`)
- [Phase 9] **Chrome tests are direct method + handler invocation against `TestAppContext`
  (no synthetic pixel input, no draw).** The heavy state machines are unit-tested in core
  (`data_row`, `eval_indicator`, `sheet_name`); the 27 gpui-context tests drive the real
  `ChromeView` entity (subscriptions' handlers, the 250 ms timers via `advance_clock`, the
  `RecordingClient`/`ChromeGridSink` command+request assertions) but do not render or inject
  keystrokes — matching the Phase-8 precedent (pure logic + real render baselines, no
  gpui-input simulation). Full widget event-dispatch + the rendered look are verified via the
  manual smoke checklist (phase plan) and Phase 11. (`chrome/view.rs` tests)
- [Phase 9] **Toggle pressed state (`bold`/`italic`/`underline`) reads the active cell's
  `RenderStyle` from `ChromeClient::render_style` at selection-change time.** `app_shell.md`
  also refreshes it on `StyleCacheUpdated`; in Phase 9 there is no worker, so only
  `on_selection_changed` refreshes. The `StyleCacheUpdated` refresh is Phase-11 wiring (the
  window will re-read the active cell's style on that event). (`chrome/view.rs`)
- [Phase 9] **Chrome uses flat grey constants, not gpui-component theme tokens.**
  `ui_design.md §3` names "gpui-component's secondary/panel background token"; for the
  functional-POC chrome the rows use plain `#F3F3F3`/`#D9D9D9`-class greys (like the grid's
  own look constants) rather than threading `cx.theme()` through every element. A theme-token
  pass is optional polish. (`chrome/view.rs`)

- [Phase 9] (post-CR) **Input-cap rejection surfaces the danger BORDER only, not the
  message popover** that `app_shell.md §Data row` ("danger border + message popover") and
  `ui_design.md §3.2` ("error text in a tooltip-style popover below") specify. The field
  border switches to the theme danger colour and the field stays Editing (cap-reject-keeps-
  editing works), so the rejection is visible and non-destructive — a POC-acceptable partial.
  The message-popover text is deferred (it needs a positioned tooltip/popover keyed off the
  precise `InputRejection` reason). Recording per the CR (UI approved fine-for-MVP); a future
  pass adds the reason text. (`chrome/view.rs` `render_data_row` cap-error branch,
  `DataRowEffect::ShowCapError` is a no-op in `apply_data_effects`)

- [Phase 10] **Lifecycle/registry/save/quit DECISIONS are extracted to gpui-free modules
  (`shell/registry.rs`, `shell/lifecycle.rs`) and unit-tested headlessly** (20 pure tests in
  `cargo test --workspace`); the GPUI submodules (`shell/app.rs` `FreeCellApp` global,
  `shell/window.rs` `WorkbookWindow`, `shell/welcome.rs`, `shell/menus.rs`) are thin plumbing
  that performs those decisions. `components/app_shell.md`'s test plan only lists pure logic in
  `freecell-core`; the shell decisions live in `freecell-app` (gpui-allowed crate) but the
  modules themselves import no gpui, so they test with plain `#[test]`. The registry keys
  windows on an opaque `WindowKey(u64)` (not gpui `WindowId`) so it stays pure.
  (`crates/freecell-app/src/shell/{registry,lifecycle}.rs`)
- [Phase 10] **Last-window-closes-quits is implemented in the `on_window_closed` observer →
  `WindowRegistry::is_empty()` → `cx.quit()`.** The welcome window counts toward the open
  count (`open_count = workbook windows + welcome`), so closing Welcome with no workbook
  windows also quits (`functional_spec.md §2`). The observer body is wrapped in `cx.defer(…)`
  so it never runs nested inside another `update_global` lease (a `remove_window` issued from
  within the welcome-close flow synchronously fires the observer → a nested `update_global`
  would panic "no global registered"). (`shell/app.rs on_window_closed`)
- [Phase 10] **The good close-interception API EXISTS at the pinned rev — NO data-loss
  papercut.** `components/app_shell.md §Lifecycle rules` flagged that the traffic-light close
  might skip the dirty prompt if `on_should_close` were absent. `Window::on_window_should_close
  (cx, |window, cx| -> bool)` is present and used: a dirty window vetoes the OS close and shows
  the unsaved-changes modal. The fallback (Cmd+W-only interception) was NOT needed.
  (`shell/app.rs open_document`, `shell/window.rs on_titlebar_close`)
- [Phase 10] **Per-platform key bindings = one action list, two keymaps via
  `cfg!(target_os = "macos")`.** Actions (`shell/mod.rs actions!`) bind to `cmd-*` on macOS and
  `ctrl-*` on Linux (`functional_spec.md §2.4`). The macOS menu bar (`cx.set_menus`) is
  installed only on macOS; Linux has no menu bar (every action reachable via its shortcut).
  Window-scoped actions (Save/SaveAs/CloseWindow/Undo/Redo/ToggleBold/Italic/Underline) are
  registered on the `WorkbookWindow` root element (so they are disabled on Welcome = no handler
  in scope); New/Open/About/Quit are global. (`shell/menus.rs`, `shell/window.rs`, `shell/app.rs`)
- [Phase 10] **Silent-strip save (no fidelity warning, `functional_spec.md §5.2`).** `Save`
  resolves a `SaveTarget` (titled `Save` → its path; untitled / `Save As` → native save panel
  with `.xlsx` enforced), then sends `Command::Save{path, req_id}` and folds `Saved`/`SaveFailed`.
  There is deliberately no fidelity dialog — a successful write silently drops anything IronCalc
  can't model (the warn-and-strip UX is `projects/xlsx-preservation.md`).
  (`shell/window.rs save/send_save`, `shell/lifecycle.rs resolve_save_target`)
- [Phase 10] **gpui `PathPromptOptions` has NO extension filter at the pinned rev.** So the
  open panel can't be restricted to `.xlsx` in-dialog; the restriction is enforced *after*
  selection by the loader's typed `LoadError::NotXlsx` (a non-xlsx pick opens a loading window
  that immediately shows the error dialog and closes). Recorded as an API limitation.
  (`shell/window.rs open_panel_options`)
- [Phase 10] **Finder open-file events (`App::on_open_urls`) are deferred (best-effort per
  `components/app_shell.md`).** The pinned-rev callback is `FnMut(Vec<String>)` with **no `cx`**,
  so it can't open a window from inside itself without an app-global channel; skipped for the
  MVP. **CLI `.xlsx` argv open IS wired** in `main.rs` (both platforms) — the Linux/CLI path the
  spec calls best-effort. macOS Finder double-click / xdg association is a follow-up.
  (`main.rs xlsx_arg`, `shell/app.rs` — no `on_open_urls` wiring)
- [Phase 10] **Bundled Inter is NOT vendored yet; `register_fonts` is a best-effort hook.**
  `components/app_shell.md §Structure` / `ui_design.md §3.3` bundle Inter via `add_fonts` before
  any window opens. The four Inter TTFs are not committed (the Phase-6/7 render baselines were
  captured on gpui's default UI font — vendoring + `add_fonts` now would change every baseline).
  `register_fonts` logs + no-ops; the app runs on the default font. **Phase 13 owns the font
  pass** (vendor `assets/fonts/Inter-*.ttf` + flip the hook + regenerate baselines).
  (`shell/fonts.rs`)
- [Phase 10] **macOS edited-dot vs `— Edited` suffix.** `Window::set_window_edited` exists at
  the pinned rev, so on macOS the dirty state shows as the native document-edited dot and the
  title stays clean; on Linux the fallback `— Edited` title suffix is used
  (`title_uses_suffix() = !cfg!(target_os = "macos")`). Both drive from the same op-accounting
  dirty flag. (`shell/window.rs refresh_dirty/title_uses_suffix`, `shell/lifecycle.rs window_title`)
- [Phase 10] **`WorkbookWindow` is the shell scaffold; grid+chrome composition is Phase 11.**
  It owns the worker (`DocumentClient`), loading/degraded/dirty state, the window title + edited
  dot, all modals, and the save/close flows, and folds only the *lifecycle* worker events
  (Loaded/LoadFailed/Saved/SaveFailed/Published/WorkerDegraded). It renders a placeholder body;
  Phase 11 replaces it with the composed grid+chrome and routes the remaining events
  (CellContent/EvalStarted/Finished/StyleCacheUpdated/SheetsChanged/EditRejected). The
  `ToggleBold/Italic/Underline` action handlers are registered as **keyboard-only no-op
  placeholders** (cmd/ctrl-b/i/u) — the real handlers need the grid selection range and land in
  Phase 11; there is **no Format menu** yet, so nothing else references them. `Undo`/`Redo` send
  real `Command`s; `Save`/`SaveAs`/`Close Window` are fully functional. **About is a single
  path**: it is registered *only* as a global action (`app.rs`) that routes to the frontmost
  document window (else the welcome window) — the earlier duplicate per-window `About` handler
  was removed so `do_show_about`'s document-window branch is live, not dead. (`shell/window.rs`,
  `shell/app.rs do_show_about`)
- [Phase 10] **Only worker-*emission* flows (not the folding logic) are undrivable in
  `#[gpui::test]`.** `DocumentClient::spawn` runs the eval worker on a real OS thread; gpui's
  `TestScheduler` panics at `end_test` if a test observes cross-thread activity (via
  `run_until_parked`) — so the *end-to-end* flows that wait for the worker to **emit** an event
  (welcome-closes-on-`Loaded`, the save round-trip's real `Saved`, a real `LoadFailed`) can't be
  driven headlessly here (covered by the `freecell-engine` round-trips `roundtrip.rs` /
  `worker_seam.rs` + the manual smoke checklist below + Phase-11 integration). The event
  **folding** logic, however, **IS** deterministically testable by **injecting** a synthesized
  `WorkerEvent` straight into `on_worker_event` (no emission, no parking): three injection tests
  now cover the highest-risk arms — `saved_adopts_canonical_path_and_closes_after_save` (path
  adoption + close-on-save), `save_failed_keeps_window_and_shows_non_closing_error` (quit-abort +
  non-closing error, stays open), `load_failed_shows_closing_error_and_clears_loading`
  (close-on-dismiss dialog). Together with the pure decision logic (registry/lifecycle/quit,
  21 tests) and the 5 synchronous lifecycle tests (`welcome_window_opens_on_show`,
  `new_workbook_registers_a_document_window`, `open_dedupes_same_path_activates_existing`,
  `close_dirty_prompts_and_cancel_keeps_window`, `clean_close_does_not_prompt`). **Manual smoke
  checklist (Phase 13 executes + records):**
  welcome New/Open buttons; new-workbook window opens + welcome closes; open a real `.xlsx` (CLI
  argv) → document window; Save/Save-As native panel writes `.xlsx`; Save-failed dialog keeps
  dirty; close-dirty Save/Don't-Save/Cancel; Cmd/Ctrl+Q multi-window quit with per-window
  prompts + Cancel aborts; macOS traffic-light close prompt; menu enable/disable on Welcome vs
  document; the degraded bar's real **Save As…** button; Welcome Cmd/Ctrl+W closes it; launching
  with a **bad `.xlsx` path** surfaces an error dialog on the welcome window (no invisible app).
  Runtime-verified this phase: the app boots under Xvfb+lavapipe (`FreeCellApp::init` +
  `show_welcome` open the welcome window, clean exit, no panics). (`shell/app.rs tests`, `main.rs`)
- [Phase 10 · code-review fixes] Behavior hardened after review: **(a)** the quit-prompt order
  (`front_to_back_keys`) is now the **union** of the `window_stack()` order and every registered
  window — a partial/empty stack can never let a dirty window skip its prompt and be force-closed.
  **(b)** A startup/`Open…` path that fails to resolve now reports on the frontmost document
  window if one is active, else **opens the welcome window to host the error dialog** — no more
  windowless, unquittable zombie on a bad CLI path (`report_error`, `functional_spec.md §5.1`).
  **(c)** `Saved` **canonicalizes** the adopted Save-As path before storing/registering, so a
  later open of that file dedupes (`§5.1`). **(d)** A window not in the quit's pending set closing
  mid-quit no longer re-issues the in-flight prompt (`on_window_closed` gates on
  `QuitPlan::is_pending`). (`shell/app.rs`, `shell/window.rs`, `shell/lifecycle.rs`)
- [Phase 10] **`anyhow` not added; `tracing` added.** gpui's `open_window` returns its own
  `anyhow::Result` (handled with `.expect`), so `freecell-app` needs no direct `anyhow` dep yet;
  `tracing` is now a live dep (the `register_fonts` hook logs). `tempfile` added as a dev-dep for
  the save-flow test fixtures. (`crates/freecell-app/Cargo.toml`)

- [Phase 11] **`SheetMeta` gained `has_content: bool`, sourced worker-side** (resolving the
  Phase-9 follow-up). `document.sheet_properties_with_content()` checks `!worksheet.sheet_data.
  is_empty()` per sheet; `sheet_metas()` populates it; the chrome's `merge_sheet_metas` now reads
  `meta.has_content` (dropped the "preserve own guess" fallback). This makes the delete-confirm
  gate correct — deleting a non-empty sheet now actually confirms (data-safety, `functional_spec.md
  §3.7`). Engine + chrome `SheetMeta { id, name }` constructions updated. (`worker/protocol.rs`,
  `document.rs`, `worker/run.rs`, `chrome/view.rs`)
- [Phase 11] **`Shared` publication/generation are now `Arc<ArcSwap<Publication>>` /
  `Arc<AtomicU64>`** (were bare fields inside `Arc<Shared>`) + `DocumentClient::publication_swap()`
  / `generation_counter()` accessors, so the window can hand the exact swap container + counter to
  the grid's `GridDataSources` (which need those `Arc` shapes). The worker's `.store()`/`.fetch_add()`
  still work through the `Arc` via `Deref`. (`worker/client.rs`)
- [Phase 11] **Resolved the Phase-6/8 "window-vs-element bounds" marker via a `gpui::canvas`
  probe.** The grid renders a zero-cost full-size `canvas` whose prepaint captures the grid
  element's real laid-out `Bounds` into the entity; `viewport_wh()` uses `bounds.size` and
  `event_local()` subtracts `bounds.origin`, so virtualization + hit-testing are correct now that
  chrome offsets the grid down (~68 px) and shrinks it. Falls back to `window.viewport_size()`
  before the first capture (grid ≈ full window then → identical for the full-window render-tests /
  demo). The probe's `cx.notify()` (for crisp resize) is suppressed under a render-test capture
  (`freeze_spinner`) so the pixel suite stays a single deterministic frame — verified: all 53
  render baselines still pass. (`grid/view.rs`)
- [Phase 11] **The grid announces its viewport from `render` (debounced on `last_viewport`)** — the
  single viewport-announce covering first paint, sheet switch, and resize; scroll/keyboard still
  emit eagerly, all sharing `last_viewport` so there is no double-emit and a values-only republish
  (same range) never re-announces. Emitting from render is side-effect-only (`client.send` via the
  sink, no entity update), so it is safe on the hot path. (`grid/view.rs render`)
- [Phase 11] **Grid ⇄ chrome routing goes through boxed-closure sinks that capture the *sibling*
  entity handles (not the `WorkbookWindow` entity)**, resolved via `Rc<OnceCell<WeakEntity<..>>>`
  slots after both children are built. This is the load-bearing anti-reentrancy design: a sink
  fires from *inside* a sibling's `update`, so it must never lease the window entity (which owns the
  worker-event task + action handlers). Cyclic follow-ups are broken with `Window::defer`:
  chrome→`MoveActive`/`SetActiveSheet` (the grid re-emits `SelectionChanged`/`ViewportChanged` back
  into the still-leased chrome) and the grid→`SelectionChanged` cap-reject revert. `FocusGrid`,
  `ViewportChanged`, `ClearCells` are direct (no cycle). The active sheet + last-accepted selection
  the sinks read ride a shared `Rc<SinkShared>` (lock-free `Cell`s) the window writes on switch.
  (`shell/window.rs make_grid_sink`, `make_chrome_grid_sink`, `route_selection_changed`)
- [Phase 11] **`ChromeView` hosts the grid as an `AnyView` body slot** (`set_grid_body`), rendering
  action-row → data-row → **grid (flex-fill)** → tab-bar in one entity, so the tab bar sits below
  the grid per `ui_design.md §3` while the window still owns both entities for event routing. The
  window body wrapper is a flex column so the chrome's `flex_1` stretches (found + fixed via the
  Xvfb smoke — without it the grid slot collapsed to zero). Phase-9 standalone chrome (no body)
  renders as before. (`chrome/view.rs`, `shell/window.rs render_body`)
- [Phase 11] **`DocumentClient` is held as `Rc<DocumentClient>`** and shared with the chrome as
  `Rc<dyn ChromeClient>` (respecting the Phase-4 "not `Clone`, one window one worker" spirit — a
  single handle, one worker, shared within the one window). (`shell/window.rs`)
- [Phase 11] **Sheet reconciliation on `Loaded`/`SheetsChanged`:** a newly-added sheet becomes
  active (`+` switches to it), a surviving active sheet stays, a deleted active falls back to the
  first remaining; the switch restores the grid's per-sheet scroll/selection, sends a **bootstrap**
  `SetViewport` (overscanned `INITIAL_VIEWPORT_*`, so an unvisited sheet's cache builds + publishes
  even before the grid lays it out), and re-points the data row. `overscan_range` (3×, clamped) is a
  pure unit-tested helper in `lifecycle`. (`shell/window.rs reconcile_sheets`,
  `switch_grid_to_sheet`, `shell/lifecycle.rs`)
- [Phase 11] **`EditRejected` routing:** `InputCap` → chrome danger border; `EnginePanic`/`Engine`
  → transient "That change couldn't be applied" dialog (only if no modal is up; document intact,
  `functional_spec.md §6`); `InvalidSheetName` (backstop — chrome validates first) + `Degraded` (the
  degraded bar already explains it) → no dialog. `ToggleBold/Italic/Underline` are wired to
  `chrome.toggle_style` (Phase-10 left them no-op placeholders). (`shell/window.rs on_edit_rejected`,
  render actions)
- [Phase 11] **Degraded-worker edit refusal is worker-enforced, not UI-disabled.** The degraded bar
  + Save-As show, and the worker rejects any further edit (`EditRejected{Degraded}`, ignored UI-side
  since the bar is up); the chrome's toggles/data-row are NOT greyed out. Acceptable for the MVP
  (no data change slips through — the worker is the boundary); a future pass can disable the chrome
  inputs while degraded. (`shell/window.rs`)
- [Phase 11] **Untestable boundary (documented, not skipped):** the deferred chrome→grid follow-ups
  (`MoveActive`/`SetActiveSheet`) and every worker-*command* emission (`SetViewport`/`SetCellInput`/
  …) are not deterministically assertable in `#[gpui::test]` — the window holds a live OS-thread
  worker and `run_until_parked` trips gpui's cross-thread `end_test` panic (Phase-10 boundary), and
  `Window::defer` needs a parked flush. The **synchronous** folding (`reconcile_sheets`,
  `on_edit_rejected`, `Published`/`StyleCacheUpdated`, the grid→chrome selection route via the real
  `route_selection_changed`) IS injection-tested (7 new `shell/app.rs` tests). The deferred paths +
  the full composed pixel layout are covered by the Phase-8/9 component tests + the **Xvfb+lavapipe
  smoke launch** (Phase 11 ran it: opened a styled fixture `.xlsx` → the composed window rendered
  the real grid values/styles, the data row's `GetCellContent` reply, the action-row bold state,
  and the bottom tab bar, with correct grid hit-test offset — no panics). (`shell/app.rs` tests)
- [Phase 11 · CR fix] **Flaky gpui gate FIXED — the window tests are now worker-less.** The
  gpui shell/composition tests used to spawn a real OS-thread worker (`DocumentClient::spawn` via
  `FreeCellApp::new_workbook` / `open_path`), which intermittently tripped gpui's `TestScheduler`
  non-determinism `end_test` panic in the full-workspace run. Added a `test-support` cargo feature
  on `freecell-engine` exposing `DocumentClient::detached()` — a worker-less client (dropped
  command receiver → sends are no-ops; closed event channel → the window's event task completes
  immediately; empty initial read-surfaces). `freecell-app`'s **dev-dependencies** enable the
  feature (test build only — the release binary never sees `detached()`), and the tests build
  windows via `WorkbookWindow::new_detached_for_test` / `FreeCellApp::{new_workbook_detached,
  open_path_detached}`. Folding is still injection-tested; the real worker stays covered by the
  `freecell-engine` `worker_seam.rs`/`roundtrip.rs` blocking-`recv` tests + the Xvfb smoke.
  **Verified: `cargo test -p freecell-app --lib` 25/25 clean + 3 full-workspace runs clean.**
  (`freecell-engine/Cargo.toml`, `worker/client.rs`, `freecell-app/Cargo.toml`, `shell/window.rs`,
  `shell/app.rs`)
- [Phase 11 · CR fix] **CRITICAL data-safety bug fixed: adding a sheet routed edits to the WRONG
  sheet.** On a `SheetsChanged` that *added* a sheet, `switch_grid_to_sheet` moved the grid + the
  window's active sheet but never re-pointed `ChromeView::active_sheet` (its `merge_sheet_metas`
  only reassigns on a *delete*), so after clicking `+` the tab highlight, formula bar, and every
  subsequent `SetCellInput`/`SetStyleAttr`/`GetCellContent` targeted the OLD sheet while the user
  looked at the new one. Fix: added `ChromeView::adopt_active_sheet(id)` (sets the active sheet +
  refreshes the toggle style, WITHOUT re-emitting `SetActiveSheet` — it must not re-enter the
  window's defer loop), called in `switch_grid_to_sheet` before the content fetch. Regression:
  `sheets_changed_add_switches_to_new_sheet` now asserts `chrome.active_sheet() == SheetId(9)` (the
  delete-fallback path asserts the chrome too). (`chrome/view.rs adopt_active_sheet`,
  `shell/window.rs switch_grid_to_sheet`, `shell/app.rs` tests)
- [Phase 11 · CR fix] **Dropped the vestigial `GridDataSources.generation`** (the grid never read
  it — repaint is `Published` → `grid.notify()` + the per-frame `ArcSwap` re-read) + the
  `DocumentClient::generation_counter()` accessor, and reverted `Shared.generation` to a plain
  `AtomicU64` (no longer needs `Arc`-wrapping; `publication` stays `Arc<ArcSwap>`). Render baselines
  unaffected (53/53 pass). (`grid/view.rs`, `worker/client.rs`, `grid/fixtures.rs`,
  `render-tests/src/scene.rs`)
- [Phase 11 · CR fix] **Mild:** `LoadFailed` now also clears the grid's "Opening…" overlay
  (`grid.set_loading(None)`, symmetry with `Loaded`); and `on_worker_event` is now an **exhaustive**
  match (no `_ => {}`) — `Saved`/`SaveFailed` match unconditionally and branch on the pending
  `req_id` inside, so a future `WorkerEvent` variant is a compile error forcing a conscious routing
  decision. (`shell/window.rs on_worker_event`)

- [Phase 12] **Perf-gate CI thresholds CALIBRATED + committed** (resolves the placeholder in
  this file's header). Calibrated on this container (proxy for the pinned ubuntu-24.04 runner
  image): **Intel Xeon @ 2.80 GHz, 4 cores, Linux x86_64, rustc 1.95.0, `--release`, Mesa
  lavapipe (software Vulkan)**. Measured over the POC "Run Test" scenario (348 scripted
  frames) against the **real `GridView`** + a **1M×100 styled** engine-backed fixture. This is
  the **single canonical run** cited identically here, in `render-tests/src/perf.rs`, and in
  `render-tests/results/perf-runtest.json`: **frame-build p50 = 1.89 ms, p99 = 5.56 ms,
  max = 5.80 ms; cell-load p50 = 54.6 µs, p99 = 93.6 µs, max = 165 µs.** These meet the §4
  real-hardware budgets (frame p99 ≤ 8.33 ms, worst ≤ 16.67 ms, cell-load p99 < 2 ms) even on
  this slow shared CPU. Committed buffered CI gates (`render-tests/src/perf.rs` `CI_*`), each
  ≥ ~2× the canonical figure it buffers: **frame-p99 = 11.5 ms** (2.07× the 5.56 ms p99),
  **frame-max = 13 ms** (2.24× the 5.80 ms max), **cell-load-p99 = 500 µs**. The cell-load gate
  is a **deviation from strict-2×**: the calibrated 93.6 µs is a micro-measurement dominated by
  scheduler/cache noise, so a ~190 µs (2×) gate would be flaky across runner CPUs; 500 µs
  (~5.3×) still catches a real regression yet stays 4× under the 2 ms product budget.
  Recalibrate only deliberately (a committed change), never to quiet a regression.
  (`render-tests/src/perf.rs`, `.github/workflows/perf-gates.yml`)
- [Phase 12] **What lavapipe does / doesn't represent (and how the harness handles it).**
  This container renders through software Vulkan, so **GPU present + gpui layout/text-shaping**
  (which run *after* `render()` returns) are **not representative** of real hardware and are
  **deliberately NOT measured/gated**. The harness measures the **CPU render-build path** —
  data resolution (axis math + resident-style snapshot + the O(published-cells) publication
  scan) + element construction — exactly the window the POC's `frame_render_ns` timed on
  macOS/Metal, plus the **engine-call counter** (fully representative). The frame-build p99
  (5.56 ms) is directly comparable to the POC's raw-gpui p99 (~2 ms on M-series); ~2.8× is
  plausible slow-shared-CPU scaling, not a measurement artifact. The full painted-frame budget
  (layout + shaping + present) remains a **macos-verify (real-hardware) concern**, recorded but
  not gated in Linux CI. (`render-tests/src/bin/perf_harness.rs`, `grid/view.rs measure_frame`)
- [Phase 12] **"Zero engine calls on the scroll path" is asserted with an instrumented engine
  counter + a negative control.** `freecell-engine::instrument` holds a process-global
  `AtomicU64` bumped at the entry of **every `WorkbookDocument` method that reads or mutates the
  constructed IronCalc model** — all per-cell/geometry reads, sheet-metadata reads, edits, batch
  controls, `save`, and the `user_model_mut` escape hatch; the only exclusions are the
  constructors (they *build* a fresh model) and two `#[cfg(test)]` style-read helpers.
  `WorkbookDocument` is the single IronCalc boundary; the grid render path never holds one. The
  harness snapshots `engine_call_count()` before/after the whole 348-frame scroll sweep and
  asserts a **zero delta** (measured: 0 calls over 284 distinct viewports) — the sweep reads only
  the shared `Publication` + `SheetCaches`, never the worker. **Negative control:** one real edit
  (`SetStyleAttr`) to the still-alive worker moves the counter by **25,615 calls** (apply +
  publish re-probe), proving the gate is discriminating, not vacuous. Both are hard-failed in
  `--gate` mode. (`freecell-engine/src/instrument.rs`, `document.rs`,
  `render-tests/src/bin/perf_harness.rs`)
- [Phase 12] **The perf harness drives the REAL grid via a `measure_frame` hook, not a copy.**
  `GridView::render`'s frame-dependent element build was factored into a shared
  `build_grid_layers` used by BOTH `render()` and the new `GridView::measure_frame` (perf), so
  the measured build can never drift from the real render path. `measure_frame` is a public hook
  alongside the existing render-test/debug hooks (`set_force_scrollbars` / `set_freeze_spinner`);
  it applies a scripted scroll (clamped to the sheet), times the build, and FORCE+ASSERTs
  non-empty content (can't measure a no-op). The scripted scenario + latency stats + gates live
  in `freecell-core::perf` (engine-free, gpui-free, unit-tested — ported from
  `experiments/04-ui-poc/poc-core`; core stays serde-free, so JSON is written by the binary).
  (`grid/view.rs`, `freecell-core/src/perf.rs`)
- [Phase 12] **Fixture design — "1M×100 styled" realized within the publication cap.** The
  sheet is Excel-max deep (1,048,576 rows) with the 100-col region of interest densely styled
  via **col-band styles** (5-way rotating fills/bold/italic/underline/align — so every visible
  cell at any scroll depth builds a fully-styled element, near-worst-case the whole sweep),
  variable column widths (incl. 300 px wide cols for text-shaping stress), and a spread of
  row-height overrides (variable geometry at 1M scale). Real engine **values** populate the top
  256 rows × 100 cols and are published as one window (25,600 cells, under the worker's 512×256
  `MAX_PUBLISH` cap) — deliberately larger than a typical overscan so every frame pays the
  `O(published-cells)` publication scan the grid flags in `grid/view.rs` (measured fine at
  cell-load p99 = 93.6 µs, resolving that concern). Deep-scroll frames beyond the valued band render
  style-only (blank text) — the real beyond-overscan regime — so the heaviest (fully-valued +
  styled) frames dominate p99. Env overrides `FREECELL_PERF_COLS` / `FREECELL_PERF_VALUE_ROWS`
  shrink it for a quick smoke run (never for calibration/gate). (`render-tests/src/perf.rs`)

- [Phase 13] **FONTS DECISION: bundled Inter DEFERRED; MVP ships on the default UI font.**
  `ui_design.md §3.3`/§7 call for bundled Inter (registered via `add_fonts`) for pixel-stable
  baselines. Decided to DEFER, evidence-based: (a) the baseline-stability rationale is already
  delivered for the MVP by pinning the render-suite runner image + Mesa + font packages (the
  48-case suite is green + bit-stable on the default font); (b) only Inter *variable* fonts are
  readily fetchable here, not the 4 static faces the spec names, adding font-kit variable-axis
  resolution risk; (c) the render harness (`render_scene`) opens its own gpui `App` and doesn't
  register fonts, so vendoring means wiring `add_fonts` there too AND regenerating + re-eyeballing
  all 48 baselines against the load-bearing pixel gate at the finish line — disproportionate to a
  robustness upgrade that is not a functional gap. Tracked: `PROJECTS.md` +
  `projects/bundled-inter-font.md`. Fixed the stale/aspirational font claims so nothing falsely
  says fonts are registered: `shell/fonts.rs` (module + fn doc now state it's a no-op),
  `main.rs` (comment), `grid/mod.rs` (`GRID_FONT_FAMILY` doc = reserved-not-applied).
  (`shell/fonts.rs`, `main.rs`, `grid/mod.rs`, `projects/bundled-inter-font.md`)
- [Phase 13] **KNOWN LIMITATION finalized: type-based default cell alignment + `[Red]` number-format
  text color are not rendered.** `functional_spec.md §3.6` says unstyled numbers/dates default
  right, booleans/errors center, and `[Red]` formats color the text. `PublishedCell` carries only a
  display string (+ an always-`None` `text_color`), no value type, so the grid defaults **all** cells
  to left (`grid/view.rs`: `style.h_align.unwrap_or(Align::Left)`) in the default color. Phase 6
  deferred this to "Phase 11 engine wiring", which did not land it; Phase 13 (no new features) tracks
  it rather than change the publication schema + regenerate ~10 baselines at the finish line. Values,
  formats (currency/percent/date/thousands), error text, and **explicit** alignment are all correct
  (verified in the baselines). Home: `projects/type-aware-alignment.md`, `PROJECTS.md`. This
  supersedes the Phase-6/Phase-4 "deferred to Phase 11" notes. (`crates/freecell-core/src/publication.rs`,
  `grid/view.rs`, `worker/run.rs build_publication`)
- [Phase 13] **cargo-deny RE-AUDIT: posture unchanged — still no safe upgrade at the pinned gpui/
  ironcalc revs; documented, not silent.** Re-ran `cargo deny check` (0.19.9, `working-directory:
  app`) → clean. Re-verified every ignored advisory + the GPL `ztracing` license exception (zed#55470)
  still has no upgrade we control. **Corrected a provenance detail** (the Phase-1 note + the task
  framing loosely attributed quick-xml to ironcalc): verified against `Cargo.lock`, the **quick-xml
  0.39.4 DoS pair** (`RUSTSEC-2026-0194/-0195`) enters the tree ONLY via desktop-protocol/build-time
  parsers of **trusted** XML — `wayland-scanner` (build), `xcb` (build, 0.30.0), and `zbus_xml`
  (D-Bus introspection via atspi). It is **NOT** on the `.xlsx` open path: ironcalc `=0.7.1` reads
  workbooks with `roxmltree` + `zip 0.6.6`, not quick-xml — so a hostile `.xlsx` does not exercise
  these advisories. Fixed in quick-xml ≥0.41 (needs a zed/wayland bump). Captured the full posture +
  the pre-ship checklist as a security note
  with a home: `PROJECTS.md` + `projects/pre-distribution-security-audit.md`. FreeCell ships no
  binaries yet, so the documented exception posture is acceptable for MVP. `deny.toml` header + the
  Phase-1 cargo-deny entry above remain accurate; no `deny.toml` change needed. (`app/deny.toml`,
  `projects/pre-distribution-security-audit.md`)
- [Phase 13] **Render baselines EYEBALLED (all 48).** Reviewed every committed baseline PNG (via
  category montages): text attrs (bold/italic/underline + combos), fills (incl. dark-fill contrast,
  fill-covers-gridlines, empty-styled), engine-formatted numbers/currency/percent/date/boolean/text,
  `#DIV/0!`/`#NAME?`/`#CIRC!` errors (incl. the circular-ref pair resolving with no hang), explicit
  alignment (left/right/center + overrides-default), clipping (clipped/exact-fit/narrow-column),
  variable geometry (tall row / wide column), and grid scenes (empty origin, deep-scrolled headers
  Z→AE / rows 490–501, single/range/edge-spanning/shift/drag/scrolled selections with the white
  anchor + blue overlay, visible scrollbars, the loading overlay, the mixed-content canary). All
  render correctly and match their intended scenes. The only cross-spec observations are the two
  known limitations above (default-left alignment; non-red negative), which are visible + tracked.
  The human final-eyeball remains the user's, but this sweep found nothing wrong. (`render-tests/baselines/`)
- [Phase 13] **Coverage matrix + smoke checklist produced as durable artifacts.**
  `specs/projects/mvp/coverage_matrix.md` maps EVERY `functional_spec.md §2–§9` behavior to a named
  automated test or a documented-manual smoke item (`M-1`…`M-16`) — no silent gaps. Smoke items
  driveable under Xvfb+lavapipe were driven this phase (see `smoke_checklist.md`); native-OS /
  real-hardware items are recorded documented-manual with repro steps. (`coverage_matrix.md`,
  `smoke_checklist.md`)
- [Phase 13] **READMEs completed.** Created the repo **root `README.md`** (there was none) describing
  the real project (full spreadsheet, not the "hello-world" the stale `app/README.md` run line
  implied); fixed that stale `app/README.md` line + refreshed its CI/render/perf sections;
  `render-tests/README.md` verified accurate (pinned image, tolerance, human baseline process).
  (`README.md`, `app/README.md`, `app/render-tests/README.md`)

---
## Phase 13 resolution index (curated — disposition of every deferred/flagged item)

A reviewer's one-stop list. "Resolved" = done in-repo; "Known-limitation" = shipped as-is with a
tracked home (nothing silently lost); "MVP-scope" = intentional §8 omission.

**Resolved this phase**
- Fonts placeholder / false `add_fonts` claim → conscious DEFER decision + code-claim fixes (above).
- cargo-deny re-audit → re-run clean; posture documented with a home (above).
- All render baselines → eyeballed (above).
- READMEs (root missing, app "hello-world" line) → written/fixed (above).
- Coverage completeness → `coverage_matrix.md` (every behavior mapped).

**Known-limitations shipped with a home (not silent)**
- Type-based default alignment (§3.6) → `projects/type-aware-alignment.md`.
- `[Red]` number-format text color (§3.6; Phase 4/7) → same project note.
- Input-cap message-popover *text* (§3.3; Phase 9 post-CR) → danger border shown; popover text is
  chrome polish. Cell-unmodified + focus-kept behaviors ARE covered.
- Bundled Inter (§3.3) → `projects/bundled-inter-font.md`.
- macOS Finder open-file `on_open_urls` (§2.1; Phase 10) → CLI argv wired; macOS Finder assoc is a
  documented gap.
- GPL `ztracing` + quick-xml DoS + bans/sources leniency → `projects/pre-distribution-security-audit.md`
  (MANDATORY pre-distribution).
- Save fidelity silent-strip (§5.2) → intentional; warn-and-strip is `projects/xlsx-preservation.md`.

**MVP-scope (intentional §8 omissions, already tracked)**
- Dynamic arrays/spill; in-cell edit; IME; clipboard; structural-edit UI; row/col resize; merges/CF/
  comments/validation/hyperlinks; CSV; recent-files; find/replace; sort/filter; freeze; hide; zoom;
  charts; named-range UI; multi-range; fill handle; session restore; autosave; Windows. Homes in §8 +
  `PROJECTS.md` (`xlsx-preservation`, `ime-text-input`, `excel-clipboard`, `viewport-cache`,
  `style-cache`).

**Un-driveable-here manual smoke (documented in `smoke_checklist.md`, native-OS / real-hardware)**
- Native NSOpen/NSSave panels; macOS menu bar + enable/disable; traffic-light close prompt; edited
  dot; 100 MB open timing; real read-only-perms save failure (container runs as root); real-hardware
  frame budget; scrollbar auto-hide + held-drag edge auto-scroll.
