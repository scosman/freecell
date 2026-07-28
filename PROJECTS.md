# FreeCell — Projects

Forward-looking product/engineering initiatives for FreeCell. This is a lightweight
registry: each entry is a short description plus a pointer to a design note under
[`projects/`](projects/).

> Not to be confused with [`specs/projects/`](specs/projects/), which holds the
> spec-driven **planning + build** artifacts for a phase of work (overview →
> functional spec → architecture → implementation plan). `projects/` here is a
> backlog of *initiatives and design notes* — some future, some speculative.

## Backlog

> **Known gaps** (things missing / partial / deferred, as opposed to the forward-looking
> initiatives below) live in the running log at [`GAPS.md`](GAPS.md). Several entries
> below are the *design notes* for gaps tracked there (type-aware alignment, bundled
> Inter, pre-distribution audit).

- **Architecture-Review Remediation** — *Proposed (2026-07-28) — plan of record.*
  The 30-unit remediation plan closing the findings of the whole-codebase architecture
  review of `app/` at `e4c7afa` (22 critical / 55 moderate / 39 mild). Each unit is
  sized (task vs project), tiered (v0.5 / v1.0 / v2.0) and ordered, so they can be
  picked up independently — 13 of them have no dependencies at all. Includes the
  root-cause finding behind most of the criticals: the project consistently mistakes
  having *reasoned about* an invariant for having *enforced* it. Keystone unit is **C1**
  (a part-inventory round-trip test), which four other units sit behind.
  → [`projects/architecture-review-remediation.md`](projects/architecture-review-remediation.md)

- **All-Styles Resident Cache (grid geometry + styling)** — *Near-MVP.*
  An always-resident cache of the full resolved style for the sheet — **all** row/col
  sizes (geometry) + fills/lines/bold/number-format — **not** viewport-based. Needed to
  render the grid at all (geometry), takes the ~10× style read (SP4) off the scroll path,
  and — since styles/sizes don't change during a recompute and it's frontend-resident —
  lets the grid render **fully-styled during an eval** (only cell values lag).
  → [`projects/style-cache.md`](projects/style-cache.md)

- **`.xlsx` Preservation on Save** — *Future (post-MVP by product call, 2026-07-02).*
  IronCalc's writer silently drops everything it doesn't model (comments, validation,
  hyperlinks, merges, CF — and charts/pivots/drawings/VBA were never examined), so
  "open a colleague's file, fix one cell, save" is destructive. MVP ships this
  behavior with no warning (decided in MVP planning Round 1); this project adds the
  warn-and-strip UX first, then weighs zip-level unknown-part pass-through vs owning
  the writer, plus the real-file-corpus de-risk.
  → [`projects/xlsx-preservation.md`](projects/xlsx-preservation.md)

- **IME / International Text Input** — *Future (post-MVP by product call, 2026-07-02).*
  Full IME (CJK composition), dead keys, layouts, decimal-comma entry for the custom
  raw-gpui cell editor. What GPUI exposes at the pinned rev is unknown — carries a cheap
  probe to run before the editor architecture hardens.
  → [`projects/ime-text-input.md`](projects/ime-text-input.md)

- **Excel Clipboard Interop** — *Future (post-MVP by product call, 2026-07-02).*
  Rich range copy/paste with Excel via TSV + HTML/XML-Spreadsheet clipboard flavors.
  All FreeCell-side work (IronCalc's clipboard isn't externally chainable); plain TSV
  values may ride along with the editor build.
  → [`projects/excel-clipboard.md`](projects/excel-clipboard.md)

- **Merged Cells (render + selection + merge/unmerge UI)** — *Future (deferred from `mvp-gaps` scope-back, 2026-07-04; tiers a+b ready to build).*
  Full merged-cell support. Tiers a+b (render file-loaded merges + selection
  correctness) are investigated, need **zero engine changes**, and are ready to build
  as a focused project; tier c (merge/unmerge UI) is blocked on an IronCalc
  `UserModel` merge API (upstream PR preferred, minimal patch-fork fallback) + the
  structural-edit merge-adjustment landmine. `mvp-gaps` ships only a guard blocking
  insert/delete rows/cols that would displace merges.
  → [`projects/merged-cells.md`](projects/merged-cells.md)

- **IronCalc — move to a released pin** — *Future (follow-up tail of `ironcalc-upstreaming`).*
  That project upgrades FreeCell onto the fork's git-`main` + our E2/E5 fixes (migrating to the
  new `Color`-enum API and deleting `open_fixups`/`open_repair` + `roxmltree`/`zip`). This tail
  swaps the temporary `[patch.crates-io]` git pin for a **released** IronCalc version once one
  ships with all five fixes, and re-validates. → [`projects/ironcalc-upgrade.md`](projects/ironcalc-upgrade.md)

- **Viewport Value Cache** — *Future, optional scroll-perf push.*
  Delta-load only newly-exposed cells' *values* on scroll (styles/geometry come from the
  resident style cache above); invalidate on recompute. Optional — SP4 showed uncached
  value reads are cheap. → [`projects/viewport-cache.md`](projects/viewport-cache.md)

- **Bundled Inter Font** — *Implemented (2026-07-06).*
  Inter (SIL OFL) is now vendored (`crates/freecell-app/assets/fonts/inter/`) and registered
  at startup via GPUI `add_fonts` (`shell/fonts.rs`); the grid + chrome render Inter on every
  platform. This closed a real bug — the deferral assumed baseline stability was "already
  delivered" by pinning the runner, but on Linux the default font silently rendered bold/italic
  as regular, so baselines were untrustworthy. Bundling fixed bold/italic and made macOS, Linux,
  and CI render the same font. → [`projects/bundled-inter-font.md`](projects/bundled-inter-font.md)

- **Type-Aware Default Cell Alignment (+ number-format text color)** — *Future (deferred
  from MVP Phase 13).* Render Excel's type-based default alignment (numbers/dates right,
  booleans/errors center) and `[Red]`-style number-format text color. The MVP publishes
  only a display string per cell (`PublishedCell`), so every cell defaults to left and
  text is the default color; values/formats/errors are correct, and *explicit* alignment
  renders correctly. Needs the worker to publish per-cell value type + resolved color.
  → [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md)

- **Pre-Distribution Security & License Audit** — *Future, MANDATORY before shipping any
  binary.* Re-audit `cargo-deny`: the GPL `ztracing`/`zlog`/`ztracing_macro` blocker
  (zed#55470) is **resolved** — replaced by permissively-licensed no-op stubs via `[patch]`
  (`app/vendor/`), so no GPL code is compiled or linked. Still open: the quick-xml ≥0.41 DoS
  advisories (transitive via ironcalc's xlsx reader + zed's wayland/atspi stack), tighten
  bans/sources. No safe upgrade for those exists at the pinned gpui/ironcalc revs today;
  FreeCell ships no binaries yet, so the documented posture is acceptable for now.
  → [`projects/pre-distribution-security-audit.md`](projects/pre-distribution-security-audit.md)

- **Windows Port** — *Largely done (2026-07-24): Windows compiles, packages, and gates CI;
  `.xlsx`/`.csv` file associations wired.* The build compiles + runs, `scripts/package.ps1`
  produces the NSIS installer, and the Windows `release` CI job is now **required**
  (`continue-on-error` removed); optional Authenticode signing is wired. Residual: a Windows
  hardware smoke.
  → [`projects/windows-port.md`](projects/windows-port.md)

- **Release Signing & Distribution** — *Signing done; distribution is the open half, gated on
  the security audit.* macOS `.app`/`.dmg` are Developer-ID-signed + notarized via
  `app/scripts/sign_macos.sh` (local/manual); Windows `.exe`s are Authenticode-signed via Azure
  Trusted Signing in the CI `release` job (`app/scripts/sign-windows.ps1`, run green). Linux is
  unsigned by design. What remains is **distribution** — switching the workflow from CI run
  artifacts to attaching assets to a GitHub Release (plus optional macOS-in-CI automation) —
  hard-gated on the pre-distribution security audit above.
  → [`projects/release-signing-and-distribution.md`](projects/release-signing-and-distribution.md)
- **Rewrite chart `c:f` on sheet rename** — *Future (deferred from charts P10, 2026-07-10).*
  Chart save is source-first (charts/architecture §5): it byte-preserves / targeted-patches the
  retained chart XML, keeping each data reference's `c:f` **verbatim**. So after an in-session
  data-sheet rename (`Data`→`Data2`), a preserved/patched chart's internal `c:f` still reads
  `Data!…` — the **cached values are correct** (the chart shows the right data on reopen), but the
  live data link **dangles** in Excel/LibreOffice (they can't refresh it). Charts are invisible to
  IronCalc, so its rename doesn't touch them. This project reflows the `c:f` sheet-name prefixes on
  a rename (engine-side, keyed off the rename event) so the link stays live.
  → [`projects/chart-cf-rewrite-on-rename.md`](projects/chart-cf-rewrite-on-rename.md)

- **Chart edit-panel range picking under click-away-close** — *Future (deferred from charts
  post-v1 Batch 2, item 12, 2026-07-11).* Batch 2 made the chart edit panel close on click-away, so
  the old "open panel → drag a range → Use selection" flow no longer works (the drag closes the
  panel first). A workable order survives — select the range **first**, then click the chart (which
  preserves the selection and opens the panel with "Use selection" ready) — the rough edge is a
  freshly-inserted chart whose panel auto-opens. Options: a "pick range" mode that temporarily
  suspends click-away, or an in-panel select-first hint. Batch 3's item 8 (default range = selection
  at creation) already mitigates the most common freshly-inserted case.
  → [`projects/chart-panel-range-pick.md`](projects/chart-panel-range-pick.md)

- **Non-Adjacent (Disjoint) Cell Selection — Cmd/Ctrl+Click** — *Future (deferred from
  `feature-gaps-7-11`, 2026-07-12).* Cmd/Ctrl+click to add scattered cells/ranges as separate
  areas so one op (bold, clear, delete) hits all of them (Excel multi-area selection). Deferred
  because it's a **core refactor**, not a UI tweak: FreeCell's selection is a single contiguous
  `SelectionModel { anchor, active }` (`freecell-core/src/selection.rs:64`) and adding areas
  ripples into render, keyboard motion, clipboard (copy must guard non-contiguous), and every
  formatting/clear op. No IronCalc change. → [`projects/disjoint-selection.md`](projects/disjoint-selection.md)

- **Freeze Panes** — *Future (deferred from `feature-gaps-7-11`, 2026-07-12).* Right-click a
  row/col header → Freeze this-and-above/left (item flips to Unfreeze at the current boundary).
  **Engine side is ready — no IronCalc fork change**: `UserModel::set_frozen_rows_count/set_frozen_columns_count`
  exist, are undoable, and round-trip through xlsx `<pane>` (round-3 API audit). Deferred because
  the render side is the hard part: splitting the custom grid's single viewport/scroll into
  frozen + scrolling quadrants with per-quadrant geometry and scroll clamping (`GAPS.md:141`).
  → [`projects/freeze-panes.md`](projects/freeze-panes.md)

- **Fraction number format (`# ?/?`)** — *Future (deferred from `gaps_closing_7_12` Phase 6,
  2026-07-13).* A Fraction preset (`# ?/?`, `# ??/??`) for the number-format dropdown so a value
  like `1.5` displays as `1 1/2`. Deferred because it needs an **IronCalc fork implementation**:
  the engine's `?/?` fraction formatting is effectively unimplemented (`format_number(1.5, "# ?/?")`
  → `"  /2"`, garbled for every input — not even a `#VALUE!`). Phase 6 was FreeCell-side / no-fork,
  so the Fraction preset + `Category::Fraction` were dropped rather than shipped broken; the
  engine-render guard test (`freecell-engine` `every_num_fmt_preset_code_renders_without_parse_error`)
  now covers the whole inventory so a re-add can't regress. → [`projects/fraction-number-format.md`](projects/fraction-number-format.md)

- **Shared Remote Compile Cache (sccache + Cloudflare R2)** — *Implemented (2026-07-24),
  dev containers only.* Fresh dev containers no longer recompile the pinned dep tree
  (gpui/zed, gpui-component, ironcalc fork) from scratch: sccache (`RUSTC_WRAPPER`)
  backed by the shared `freecell-dev` R2 bucket serves rustc outputs across sessions,
  auto-activated per session by a `SessionStart` hook and degrading gracefully to
  uncached builds when the R2 secrets are absent. **CI is deliberately untouched**
  (keeps Swatinem/rust-cache). Design, limits, and R2 token rotation:
  → [`projects/build-cache.md`](projects/build-cache.md)

- **Adopt gpui-component menus app-wide (native flyout submenus)** — *Future (deferred from
  `gaps_closing_7_12` Phase 10.4, 2026-07-13).* The whole chrome uses **seven** hand-rolled
  `div().absolute()…occlude()` popover cards over a `backdrop()` (fill, text-color, borders,
  font-family, font-size, chart, number-format), each with its own anchoring/dismiss. This
  project migrates them to gpui-component's `PopupMenu`/`Popover` (`crates/ui/src/menu/`), which
  ships real **flyout submenus** (`PopupMenu::submenu`) + consistent anchoring/dismiss/keyboard.
  The trigger: Phase 10.1's number-format "More ▸" wants a flyout, but gpui-component's
  `scrollable` and submenu modes are mutually exclusive and the num-fmt card must scroll — and
  doing it for one popover would make it the app's only gpui-component menu, diverging from its
  six siblings. So the flyout is only worth it as an app-wide unification (Phase 10 shipped a
  drill-in instead). → [`projects/gpui-component-menus.md`](projects/gpui-component-menus.md)

- **Unary-minus boolean/text coercion (SUMPRODUCT `--` count idiom)** — *Future (deferred from
  `scalar-functions-batch`, 2026-07-22).* The engine's unary-minus operator doesn't coerce a
  boolean/text operand to a number before negating, so `=--TRUE` yields `TRUE` instead of `1` and
  the classic `SUMPRODUCT(--(cond))` boolean-counting idiom returns `0`. SUMPRODUCT itself is
  correct; the bug is operator-level, with a broad blast radius across every formula that negates a
  non-number — out of the scalar-functions batch's function-local scope. Workarounds
  `SUMPRODUCT(1*(cond))` and `SUMPRODUCT((A=x)*(B))` both work today. → [`projects/unary-minus-boolean-coercion.md`](projects/unary-minus-boolean-coercion.md)

- **Frozen-pane boundary overflow on structural edits (fork fix)** — *Future (found in
  `engine-worker-hardening` Phase 1 review, 2026-07-28).* IronCalc grows a sheet's frozen boundary
  inside an insert's diff (`base/src/actions.rs:1051`, and the column twin at `:725`) with no upper
  bound, and `insert_rows` only range-checks the **populated** dimension — which empty inserted rows
  don't grow. So on an empty sheet, freeze 1 row + three `InsertRows { count: 1_000_000 }` leaves
  `frozen_rows = 3,000,001` on a 1,048,576-row sheet, and it survives save/reopen as an invalid
  `<pane ySplit="3000001">`. Reachable by repeated gesture (not one — Select-All → Insert is
  rejected), because the insert count comes from the selected header run — the same
  untrusted-selection pattern that caused B2. Belongs in the fork on its own
  `fix/<slug>` branch as one upstream PR, **not** as a FreeCell workaround; FreeCell itself stays
  bounded because the sheet-cache clamp holds. → [`projects/frozen-pane-boundary-overflow.md`](projects/frozen-pane-boundary-overflow.md)
