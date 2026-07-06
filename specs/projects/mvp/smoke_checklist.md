---
status: complete
---

# Manual Smoke Checklist — FreeCell MVP

The smoke items accumulated across phases (Phase 9 controlled-input assumptions, Phase 10
shell flows, Phase 11 composed window), plus the `coverage_matrix.md` `M-N` items. Each is
either **DRIVEN** here under Xvfb + Mesa lavapipe (with the observed result recorded) or
**DOCUMENTED-MANUAL** (a native-OS surface, real-hardware timing, or root-container
limitation that cannot be exercised in this environment) with clear repro steps.

**Environment for the driven items:** the pinned image (Ubuntu 24.04, Rust 1.95.0), Mesa
lavapipe (`llvmpipe (LLVM 20.1.2)`), `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`,
`LIBGL_ALWAYS_SOFTWARE=1`, `xvfb-run -s "-screen 0 1240x840x24"`. The app self-exits via
`--exit-after-ms`; a screenshot is forced with `xrefresh` + `import -window root` (the
Phase-1 capture path). Fixture: a real `.xlsx` written by `freecell_engine::fixtures::styles()`
(A1=`1` bold, B1=`2`, C1=`3` underlined, A2=`4` on a red fill, B2=`5`).

---

## DRIVEN this phase (Xvfb + lavapipe) — observed results

| # | Item (source) | Result |
|---|---|---|
| M-1 | **Launch + open a real `.xlsx` via CLI argv** (`§2.1`, Phase 10) | **PASS.** `freecell smoke.xlsx --exit-after-ms 5000` boots, selects the lavapipe Vulkan adapter, opens the workbook window (not Welcome), runs, exits **0**. No panic/abort/assertion in the log (the only ERROR — "no xinput mouse pointers" — is a benign headless-Xvfb artifact). |
| M-2 | **Composed window renders a real file** (`§3`, Phase 11) | **PASS (screenshot verified).** The captured frame shows the full composed window: **action row** ([B] pressed, I, U, `Fill ▾`), **formula bar** (ref box `A1` + content field `1`), the **custom grid** (headers A–L / 1–28, gridlines) with real engine values + styles — A1=`1` bold + active-cell blue border, C1=`3` underlined, A2=`4` on a red fill, B1=`2`, B2=`5` — and the **sheet tab bar** (`Sheet1` active + `+`). |
| M-2a | **Action-row bold state reflects the active cell** (`§3.5`) | **PASS.** With A1 active (bold in the fixture), the **B** toggle renders pressed. |
| M-2b | **Formula bar shows the active cell's raw content** (`§3.3`) | **PASS.** Ref box `A1`, content field `1` (A1's literal), fetched via the worker's `GetCellContent`. |
| M-2c | **Fill color renders on a cell** (`§3.6`) | **PASS.** A2 renders on a solid red fill from the file's style. |
| M-2d | **Sheet tab bar + `+`** (`§3.7`) | **PASS.** `Sheet1` tab (active/white) + `+` button render at the bottom, below the grid. |
| Boot | **Welcome/first-window boot** (`§2.2`, Phase 10) | **PASS** (also re-confirmed at Phase 10). The app initializes `FreeCellApp` + gpui-component `Root`, opens a window, and exits cleanly with no panic. |

Underlying flows for these are also covered headlessly by the engine round-trip harness
(`roundtrip.rs`, `worker_seam.rs`): open→edit→save→reopen, values/formulas/styles/number
formats/sheets preserved, atomic-save failure injection, typed load errors — so the
open/edit/save/reopen *engine* path is automated even though the *native panels* that
select paths are manual (below).

## DOCUMENTED-MANUAL (native-OS / real-hardware / root-container — not driveable here)

Each has clear repro steps; run on the target platform.

| # | Item (source) | Repro / expected |
|---|---|---|
| M-3 | Welcome **Open…** → native picker (`§2.2`) | Launch with no args → Welcome; click **Open…**; a native NSOpenPanel (macOS) / GPUI paths-prompt (Linux) appears; pick a `.xlsx` → opens in a new window, Welcome closes; Cancel → Welcome stays. (Non-xlsx pick opens a loading window that immediately shows the error dialog — `PathPromptOptions` has no in-dialog filter at the pinned rev.) |
| M-4 | Window default size / min-size (`§2.3`) | New workbook window opens ~1200×800; resize down clamps at 640×480. |
| M-5 | Scrollbar auto-hide (`§3.1`) | Scroll → overlay scrollbars appear; stop → they fade ~2 s later; hover/drag keeps them visible. (Wall-clock animation; the `SCROLLBAR_FADE_SECS` constant + fade path are code-reviewed.) |
| M-6 | Per-sheet scroll + selection restore (`§3.2`/`§3.7`) | Scroll + select on Sheet1; switch to Sheet2; switch back → Sheet1's scroll position + selection are restored (in-session). |
| M-7 | `+` appends `SheetN` + switches (`§3.7`) | Click `+` → a new sheet is appended and becomes active; the tab highlight, formula bar, and edits all target it (Phase-11 CR fixed the "edits went to the old sheet" bug — regression-tested `sheets_changed_add_switches_to_new_sheet`). |
| M-8 | 100 MB styled open (`§5.1`/`§7`) | Open a ~100 MB `.xlsx`; the window opens immediately with the "Opening <name>…" loading overlay; parse runs off-thread (UI stays responsive; window closable to cancel); loading clears within ~parse time + 2 s. |
| M-9 | Save / Save As native panel (`§5.2`) | Save on an Untitled workbook → native save panel defaulting to `Untitled.xlsx`, `.xlsx` enforced; Save on a titled workbook writes in place (atomic temp+rename). |
| M-10 | Degraded-worker error bar + Save As (`§6`) | Force a second engine panic (unreachable in normal use) → a non-dismissable error bar with a **Save As…** escape hatch appears; edits are refused (worker-enforced), reads/save still work. |
| M-11 | Read-only location save failure (`§6`) | On a real OS (non-root), Save to a read-only dir → a clear error dialog, document stays dirty, existing file intact; Save-As elsewhere works. (Not driveable here — the build container runs as **root**, which bypasses `chmod` perms; the atomic-save failure path is instead covered by root-proof injection tests `save_failure_*`, `failed_save_leaves_real_existing_xlsx_byte_identical`.) |
| M-12 | Real-hardware frame budget + edit→ack (`§7`) | On macOS/Metal real hardware, scroll a 1M×100 styled sheet: frame p99 ≤ 8.33 ms, worst ≤ 16.67 ms; an edit shows its pending state < 1 frame. (Linux CI enforces buffered gates; the true budget is the `macos-verify` workflow's job. The CPU-side frame-build path is gated in Linux CI.) |
| M-13 | macOS edited-dot indicator (`§2.3`) | On macOS, an unsaved edit shows the document-edited dot in the close button (title stays clean); Save clears it. (Linux uses the `— Edited` title suffix — `title_uses_suffix()`.) |
| M-14 | Menu enable/disable by context (`§2.4`) | On macOS, with Welcome frontmost, Save/Undo/Redo/Close are disabled; with a document frontmost they enable and Undo/Redo track history availability. |
| M-15 | macOS Finder open-file (`§2.1`) | **Known gap:** double-click / drag-onto-Dock does not open the file (the pinned-rev `on_open_urls` callback lacks `cx`); CLI argv is the wired path. Tracked in `DECISIONS` Phase 10. |
| M-16 | macOS traffic-light close prompt (`§2.3`) | On macOS, closing a dirty window via the red traffic light vetoes the OS close and shows the Save / Don't Save / Cancel modal (`on_window_should_close` is present at the pinned rev). |

## mvp-gaps successors (Phases 1–8) — new smoke items

Added by the `specs/projects/mvp-gaps` project (core-spreadsheet-feel gap closing). Each is
marked **HEADLESS-VERIFIABLE** (driven under Xvfb+lavapipe, or covered by an automated
unit/integration/pixel/perf gate) or **macOS / ON-DEVICE** (native surface, real hardware, or
the §7.1 titlebar — must run on a Mac). The interactive items (drag, right-click menu, live
mirror) need a real pointer the static capture can't drive, so they are **DOCUMENTED-MANUAL**
even on Linux — noted per item.

| # | Item (source) | Kind | Repro / expected |
|---|---|---|---|
| MG-1 | **Type-to-replace + live mirror + in-cell editor** (`mvp-gaps §1`) | macOS / on-device (interactive) | Select a cell, type a char → the data row starts a replaced edit and the cell mirrors the raw text live; double-click / F2 opens the in-cell overlay (2 px accent border) editing the same text; Enter/Tab/Shift+Tab commit + move, Escape cancels. Headless proxy: `EditController`/`DataRow` reducer tests + the `cell_mirror_typing` / `incell_editor_open` pixel cases. |
| MG-2 | **Cap-error popover** (`mvp-gaps §4.2`, GAPS #3) | HEADLESS-VERIFIABLE (interactive to see live) | Paste/type an over-cap formula (> 8,192 chars or > 64 deep) → the edit is rejected, a danger border shows, and a tooltip popover names the reason. Covered by `edit_rejected_input_cap_flags_chrome_data_row` + `input_cap` unit tests. |
| MG-3 | **Range clipboard copy/cut/paste** (`mvp-gaps §2`) | HEADLESS-VERIFIABLE | Cmd/Ctrl+C/X/V over a range: internal paste keeps values/formulas (ref-adjusted)/styles; cut moves + clears source on paste; TSV round-trips to/from other apps; overflow is rejected with a message. Covered by the engine clipboard integration tests (`roundtrip`/`worker_seam`). The **system-clipboard** hop (write/read TSV to the OS) is macOS / on-device. |
| MG-4 | **Formatting controls — text color / alignment / number format / decimals** (`mvp-gaps §3.1, §3.3, §3.4`) | HEADLESS-VERIFIABLE | Action-row: text-color palette, 3-way alignment toggles (pressing the active one clears), number-format dropdown (General/Number/Currency/Percent/Date/Time/Text) + decimals ±. State reflects the active cell (single selection); commands apply to the full selection. Covered by `action_bar`/`format_ui` unit tests + the `text_color_red` / alignment pixel cases. |
| MG-5 | **Type-aware default alignment + `[Red]` format color** (`mvp-gaps §3.5`, GAPS #1/#2) | HEADLESS-VERIFIABLE | Numbers/dates default right, booleans/errors center, text left (explicit wins); `[Red]` negatives render red. Covered by `published_style_resolves_format_and_explicit_colors` + the (to-regen) alignment pixel baselines. |
| MG-6 | **Fonts — family / size + row auto-grow** (`mvp-gaps §3.2`) | HEADLESS-VERIFIABLE | Family + size dropdowns set per-cell fonts (missing family → fallback, style preserved); a larger size auto-grows the row (never shrinks; no grow on file open). Covered by `SetFont` engine tests + the `font_family_serif` / `font_size_24_row_grown` / `font_missing_family_fallback` pixel cases. |
| MG-7 | **Borders — render + presets menu** (`mvp-gaps §3.6`) | HEADLESS-VERIFIABLE | File-loaded borders render (heavier shared edge wins, drawn once over the gridline); the presets menu (All/Inner/Outer/None/Top/Bottom/Left/Right, thin black) applies band-aware + undoable. Covered by the border engine tests + the six `border_*` pixel cases + the §9 500-bordered-cell perf gate. |
| MG-8 | **macOS custom titlebar** (`mvp-gaps §4.1`, arch §7.1) | **macOS / ON-DEVICE — §7.1 30-min smoke (OUTSTANDING GATE)** | On macOS: the window draws its own 36 px action-bar-grey titlebar with the centered document title (`Name` / `Name — Edited`); traffic lights are repositioned to vertically center; the whole row drags the window and double-click zooms; fullscreen works; the edited dot still shows. Welcome gets the same with title "FreeCell". **Drag is wired via `start_window_move` on the row's left mouse-down (AppKit `performWindowDragWithEvent:`) — `window_control_area(Drag)` alone is inert on macOS at this rev.** Explicitly verify on-device: (a) press-drag on the row moves the window; (b) a plain single click on the row does NOT move the window and does not leave the app in a stuck/pressed state (AppKit's tracking loop may swallow that click's mouse-up — confirm nothing downstream depends on it); (c) double-click zooms (handled implicitly by `performWindowDragWithEvent:`, no explicit handler). **If traffic-light / fullscreen glitches appear at the pinned rev, flip `shell::titlebar::MACOS_TITLEBAR` to `false` (pre-agreed fallback, no gpui bump).** Linux is unaffected (server decorations; `MACOS_TITLEBAR == false`, verified). Headless proxy: the `titlebar_row` pixel case renders the row div in the Linux harness — the div's look only, NOT the native integration. |
| MG-9 | **Row/col resize** (`mvp-gaps §5.1`) | macOS / on-device (interactive drag) | Hover a header divider → resize cursor; drag → live guide line + size tooltip + live reflow; release commits (one undo step; min col 8 px / row 12 px; a selected header run all take the size). Headless proxy: the resize-preview math unit tests + the `col_resized_narrow_clips_text` / `row_resized_tall` geometry pixel cases (the cursors/guide/tooltip need a live pointer). |
| MG-10 | **Header selection + select-all** (`mvp-gaps §5.2`) | HEADLESS-VERIFIABLE (drag interactive) | Click a col/row header selects the whole track; Shift/drag extend; corner + Cmd/Ctrl+A select all; ref box shows `C:C` / `3:7` / `A:XFD`; band styles apply fast; Delete/copy clamp to the used range. Covered by selection/`area_of`/`format_selection_ref` unit tests + the `header_full_column_selected` / `header_full_row_selected` pixel cases. Header **drag-extend** needs a live pointer (manual). |
| MG-11 | **Insert/delete rows/cols + merge guard** (`mvp-gaps §5.3`) | HEADLESS-VERIFIABLE (right-click interactive) | Right-click a header → insert/delete N (N = selection size); engine-native, undoable, formulas adjust; a file with merged cells at/after the edit is blocked with the merge-guard dialog. Covered by the insert/delete + merge-guard engine tests (`merged_fixture`). The **right-click menu open** is a live-pointer surface (manual). |

## Summary

- **Driven here (PASS, no panics):** launch, CLI-argv open of a real `.xlsx`, and the full
  composed-window render (grid values + styles + selection, action-row bold state, formula
  bar content, fill color, sheet tab bar). Screenshot-verified.
- **Documented-manual (M-3…M-16):** native file panels, macOS menu bar / edited-dot /
  traffic-light / Finder-open, scrollbar fade animation, 100 MB open timing, degraded bar,
  real read-only-perms failure, and real-hardware frame budget — each with repro steps and,
  where possible, an automated proxy (engine round-trips, injection tests, buffered CI perf
  gates).
- **mvp-gaps successors (MG-1…MG-11):** formatting / fonts / borders / clipboard / editing
  feel / resize / header selection / insert-delete are HEADLESS-VERIFIABLE (unit + engine +
  pixel + perf gates; interactive drag/right-click surfaces flagged manual). The **macOS
  custom titlebar (MG-8) is an OUTSTANDING on-device gate** — the §7.1 30-minute Mac smoke
  with the pre-agreed `MACOS_TITLEBAR` flag-off fallback.
- **Nothing silently skipped.** Every item is either driven, documented-manual, or a tracked
  known gap.
