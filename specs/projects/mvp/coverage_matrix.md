---
status: complete
---

# Coverage Matrix — FreeCell MVP (`functional_spec.md`)

Every behavior in `functional_spec.md` (§2–§9) mapped to its **automated test(s)** or an
explicit **documented-manual smoke** entry (`M-N`, see `smoke_checklist.md`). Produced in
Phase 13 as the durable proof that "MVP complete" is trustworthy: no spec behavior is
silently uncovered.

**Legend**
- **Auto** — covered by a named automated test (unit / integration / render / perf), run in
  Linux CI.
- **Manual** — a `smoke_checklist.md` item (`M-N`); either driven under Xvfb+lavapipe this
  phase or a documented-manual repro (native OS surfaces, real-hardware perf, 100 MB files).
- **Known-limitation** — an implemented-partial / conscious deviation, captured with a home
  (a `PROJECTS.md` entry + `DECISIONS_TO_REVIEW.md`), **not** silent.

Test names below are the function names from the crates (see the per-file inventory in
`git`: `crates/*/src/**`, `crates/*/tests/**`, `render-tests/**`). Totals (runnable
`#[test]`/`#[gpui::test]` functions, per `cargo test <crate> -- --list` on Linux CI): **96
core + 78 engine + 92 app** = 266, plus **48** generated render cases (`render_cases!`) + a
render-harness handful (`perceptual_diff` + `render_suite` gate tests) + the perf gates. (A
naive text grep reports 93 for app; the 93rd is the `/// #[gpui::test]` mention in a doc
comment on `shell/window.rs`, not a real test — the compiled count is 92.)

---

## §2 Application lifecycle & windows

| Behavior | Status | Test(s) / entry |
|---|---|---|
| §2.1 Launch with no doc → Welcome | Auto | `shell/app.rs::welcome_window_opens_on_show` |
| §2.1 Open `.xlsx` via CLI argv / xdg (Linux best-effort) | Auto+Manual | `main.rs` `xlsx_arg` wiring; **M-1** (launch with a fixture path) |
| §2.1 macOS Finder open-file (`on_open_urls`) | Known-limitation | Deferred at pinned rev (callback lacks `cx`); CLI argv is the wired path. `DECISIONS` Phase 10; **M-15** documented-manual (macOS) |
| §2.2 New Spreadsheet button → empty workbook window, Welcome closes | Auto+Manual | `shell/app.rs::new_workbook_registers_a_document_window`; **M-2** |
| §2.2 Open… button → native picker, opens on success / stays on cancel | Manual | Native panel — **M-3** (driven), **M-16** (macOS NSOpenPanel doc) |
| §2.2 No recent-files list (MVP) | N/A | Explicit MVP omission (§8) |
| §2.3 One window per workbook, no shared doc state | Auto | `shell/registry.rs::registers_and_assigns_distinct_keys`; architecture (per-window worker) |
| §2.3 Default size 1200×800, resizable, min 640×480 | Manual | Window constants (`shell/window.rs`); **M-4** visual |
| §2.3 Window title = file name / `Untitled` | Auto | `shell/lifecycle.rs::document_name_untitled_and_named` |
| §2.3 macOS edited dot / `— Edited` suffix | Auto+Manual | `shell/lifecycle.rs::window_title_suffix_only_when_no_dot`, `dirty_by_op_accounting`; **M-13** (macOS dot) |
| §2.3 Close with unsaved → Save / Don't Save / Cancel | Auto | `shell/app.rs::close_dirty_prompts_and_cancel_keeps_window`, `clean_close_does_not_prompt` |
| §2.3 Last window closes → app quits | Auto | `shell/registry.rs::empty_only_when_no_windows_and_no_welcome`, `welcome_counts_toward_open_count`; `shell/app.rs` `on_window_closed` |
| §2.3 Quit (Cmd/Ctrl+Q): per-window prompts, any Cancel aborts | Auto | `shell/lifecycle.rs::quitplan_prompts_in_order_then_quits`, `quitplan_cancel_aborts`, `quitplan_empty_quits_now` |
| §2.4 macOS menu bar: FreeCell / File / Edit menus | Auto | `shell/menus.rs::menu_bar_has_the_three_specced_menus` |
| §2.4 Menu items enable/disable by context | Auto+Manual | Window-scoped actions registered on `WorkbookWindow` root (absent on Welcome); structure test above; **M-14** enable/disable visual |
| §2.4 Linux: no menu bar, Ctrl replaces Cmd, shortcuts cover all actions | Auto | `shell/menus.rs::primary_modifier_is_platform_appropriate`; per-platform keymap (`shell/mod.rs`) |

## §3 Spreadsheet window layout & behavior

### §3.1 Grid & scrolling

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Renders full Excel-max sheet | Auto | `axis.rs::handles_excel_max_rows_without_oom`, `cache.rs::excel_max_geometry_totals`; render `grid_empty_origin` |
| Fixed column (`A`…`XFD`) + row (`1`…`1048576`) headers | Auto | `refs.rs::column_label_known_values`, `column_label_roundtrip`; render `grid_headers_scrolled_deep` (Z→AE, rows 490–501) |
| Gridlines + cell content | Auto | render `cell_plain`, `cell_fill_covers_gridlines`, `grid_mixed_content` |
| Pixel-precision scroll, both axes | Auto | `layout.rs::hit_test_scrolled_variable_geometry`, `cell_at_point_scrolled_variable_geometry` |
| Scroll clamps at sheet edges | Auto | `layout.rs::max_scroll_never_negative`, `clamp_scroll_bounds` |
| Scrollbars proportional + draggable | Auto | `layout.rs::scrollbar_thumb_proportional_and_positioned`, `scrollbar_thumb_at_extremes`, `scrollbar_thumb_min_length`, `hit_test_zones`; render `grid_scrollbars_visible` |
| Scrollbar auto-hide 2 s after scroll | Manual | `SCROLLBAR_FADE_SECS` (`grid/mod.rs`); **M-5** (timing is wall-clock/animation) |
| Keyboard nav scrolls to keep active cell visible | Auto | `layout.rs::scroll_to_reveal_directions_and_clamp`; render `grid_selection_scrolled` |
| Frame budget p99 < 8.33 ms during recompute; values from published viewport | Auto | perf `perf.rs::all_gates_pass_under_target`; `grid/view.rs::measure_frame_*`; worker staleness (below) |
| Beyond-overscan mid-eval → blank values, never stale/frozen | Auto | `publication.rs::covers_reports_membership`; `worker_seam.rs::edit_reflected_after_publish_and_reads_are_wait_free` |
| Row heights / col widths honor file overrides + defaults | Auto | `cache.rs::geometry_defaults_and_overrides`, `set_row_heights_batches_sets_and_resets`; render `cell_tall_row`, `cell_wide_column`, `grid_variable_geometry` |

### §3.2 Selection

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Click → single active cell, 2px accent border | Auto | `selection.rs::single_selection_is_single`; render `grid_selection_single`, `cell_plain` (A1 border) |
| Drag → rectangular range | Auto | `layout.rs::cell_at_point_inside_and_clamped`; render `grid_selection_drag_extended` |
| Shift+click / Shift+arrows extend from anchor | Auto | `selection.rs::extend_keeps_anchor`, `extend_edge_keeps_anchor`; render `grid_selection_shift_extended` |
| Range overlay + border, distinct anchor (white) | Auto | `layout.rs::range_overlay_rects_row_excludes_active`, `range_overlay_rects_block_tiles_without_active`, `range_overlay_rects_single_is_empty`; render `grid_selection_range` (white anchor) |
| Arrow keys move active cell | Auto | `selection.rs::move_each_direction_collapses`, `move_from_range_collapses_to_stepped_active`, `move_clamps_at_edges`; `grid/input.rs::arrows_map_by_shift_and_secondary` |
| Cmd/Ctrl+arrow → sheet edge | Auto | `selection.rs::jump_edge_goes_to_sheet_bound`; `grid/input.rs::arrows_map_by_shift_and_secondary` |
| Tab/Shift+Tab, Enter/Shift+Enter move (after commit) | Auto | `grid/input.rs::tab_enter_map_to_moves`; `chrome/view.rs::enter_commits_and_moves_down`, `shift_enter_moves_up` |
| Page Up/Down, Home, Cmd/Ctrl+Home | Auto | `grid/input.rs::page_keys_map`, `home_and_cmd_home`; `selection.rs::page_moves_by_rows_clamped`, `row_start_goes_to_col_zero`, `document_start_goes_to_a1`, `extend_document_start_keeps_anchor` |
| Per-sheet selection + scroll restored on switch | Auto+Manual | `grid/view.rs` per-sheet maps + `switch_grid_to_sheet` (`shell/window.rs`); `shell/app.rs::loaded_populates_tabs_and_switches_active_sheet`; **M-6** visual round-trip |
| Single selection → data row shows content; toggles show state | Auto | `chrome/view.rs::selection_single_fetches_content`, `toggles_reflect_active_style` |
| Multi selection → data row disabled/empty; ref box shows range; toggles apply to all | Auto | `chrome/view.rs::multiselect_disables_field`; `refs.rs::range_to_a1_single_vs_rect`; `worker/run.rs::style_toggle_any_lacking_sets_all_then_clears` |

### §3.3 Data entry (data row)

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Data row shows active cell raw content (formula text / literal / empty) | Auto | `chrome/view.rs::content_reply_populates_field`; `worker_seam.rs::get_cell_content_replies_with_raw_formula`; `document.rs::cell_content` |
| Fetch spinner only after 250 ms, no flash | Auto | `data_row.rs::fetch_timeout_shows_spinner`, `reply_before_timeout_never_flashes_spinner`, `spinner_hidden_when_reply_arrives`; `chrome/view.rs::formula_field_spinner_only_after_250ms`, `formula_field_spinner_never_flashes_on_fast_reply` |
| Typing edits pending value | Auto | `data_row.rs::edit_enters_editing`; `chrome/view.rs` controlled field |
| Enter commits → engine input → eval → move down | Auto | `data_row.rs::commit_valid_moves_down`; `chrome/view.rs::enter_commits_and_moves_down`; `worker_seam.rs::set_viewport_then_edit_publishes_values` |
| Escape reverts + cancels edit | Auto | `data_row.rs::escape_reverts_to_committed`; `chrome/view.rs::escape_reverts_field` |
| Click another cell commits pending first, then moves | Auto | `data_row.rs::edit_commit_on_cell_click`; `chrome/view.rs::edit_commit_requested_commits_without_moving` |
| Delete/Backspace (grid focus) clears selected cells, one undo | Auto | `grid/input.rs::delete_backspace_clear`; `GridEvent::ClearCells` → `worker/run.rs` `ClearCells` |
| Input cap: reject formula > 8192 chars / depth > 64, keep focus + inline error, cell unmodified | Auto | `input_cap.rs::rejects_over_length`, `rejects_over_nesting_depth`, `rejects_round3_d_deep_parens_reproducer`, `rejects_round3_d_flat_chain_reproducer`, `boundary_at_exactly_the_caps`, `paren_in_string_literal_not_counted`; `data_row.rs::cap_reject_keeps_editing`; `worker/run.rs::worker_side_cap_rejects_abort_reproducers_without_touching_engine`; `chrome/view.rs::cap_reject_keeps_editing_and_flags_error`, `worker_input_cap_reject_flags_error` |
| Input-cap error shows inline message-popover text | Known-limitation | Danger **border** only; message-popover text deferred (`DECISIONS` Phase 9 post-CR). Cell unmodified + focus kept are covered. `PROJECTS`: chrome polish |

### §3.4 Formulas

| Behavior | Status | Test(s) / entry |
|---|---|---|
| `=` → engine parses + evaluates; full IronCalc function set | Auto | `roundtrip.rs::roundtrip_formulas_preserved`; `worker_seam.rs::set_viewport_then_edit_publishes_values` |
| References: relative/absolute/ranges/cross-sheet/defined-name | Auto | `refs.rs::a1_roundtrip`, `cell_range_contains`, `cell_range_normalizes_corners`; `roundtrip.rs::roundtrip_formulas_preserved`, `roundtrip_multi_sheet_and_names` (engine-owned resolution) |
| Editing triggers whole-workbook recompute; dependents update on publish | Auto | `worker/run.rs::drain_coalesces_burst_into_one_eval`; `worker_seam.rs::eval_started_and_finished_bracket_an_edit` |
| Error results are values (`#DIV/0!`/`#NAME?`/`#CIRC!`/…) rendered in-cell | Auto | `roundtrip.rs::formula_errors_are_values`; `worker_seam.rs::formula_errors_are_published_as_values`; render `cell_error_div0`, `cell_error_name`, `cell_error_circ` |
| Circular refs → `#CIRC!` in ms, never hang | Auto | `roundtrip.rs::formula_errors_are_values` (built via pause/evaluate ring); render `cell_error_circ` |
| Dynamic arrays / spill absent → engine error surfaced | Known-limitation | Accepted absent for v1 (§8); engine emits the error. `DECISIONS`; `PROJECTS` (out-of-scope) |
| Data row shows formula text, not result | Auto | `worker_seam.rs::get_cell_content_replies_with_raw_formula` |

### §3.5 Formatting actions

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Bold/Italic/Underline multi-cell toggle (any-lacking → set all; all → clear) | Auto | `worker/run.rs::style_toggle_any_lacking_sets_all_then_clears`; `chrome/view.rs::toggle_bold_sends_setstyleattr` |
| One undo step per click | Auto | `worker/run.rs::ops_seen_counts_edits_and_undo`; `worker_seam.rs::undo_redo_through_worker` |
| Fill palette: 10 Office colors + No fill + Custom (ColorPicker) | Auto | `palette.rs::palette_has_ten_office_swatches`, `palette_hexes_match_spec`; `chrome/view.rs::fill_swatch_and_no_fill` (ColorPicker for Custom…) |
| Applies fill to all selected cells | Auto | `chrome/view.rs::fill_swatch_and_no_fill`; `worker/run.rs::apply_style` range |
| Per-cell styles persist to `.xlsx` | Auto | `roundtrip.rs::roundtrip_styles_preserved`; `document.rs::roundtrip_styles_preserved` |
| Button state reflects active cell | Auto | `chrome/view.rs::toggles_reflect_active_style` |
| Formatting commits pending edit first | Auto | `chrome/view.rs::formatting_commits_pending_edit_first` |

### §3.6 Cell rendering

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Text runs bold/italic/underline + combinations | Auto | render `cell_bold`, `cell_italic`, `cell_underline`, `cell_bold_italic`, `cell_bold_underline`, `cell_italic_underline`, `cell_bold_italic_underline` |
| Background fill (solid) | Auto | render `cell_fill_red`, `cell_fill_yellow`, `cell_fill_dark_text_contrast`, `cell_fill_none_explicit`, `cell_bold_fill_yellow`, `cell_bold_italic_underline_fill_blue`, `cell_fill_covers_gridlines`, `cell_empty_styled` |
| Engine-owned display text (number formats, dates, %, currency, thousands) | Auto | render `cell_number_plain`, `cell_number_thousands`, `cell_number_currency`, `cell_number_percent`, `cell_date_default`, `cell_boolean`, `cell_text_plain`; `roundtrip.rs::roundtrip_number_formats_preserved`; `render-tests::scene_number_formats_infer` |
| `[Red]`-style number-format text color applied | Known-limitation | `PublishedCell.text_color` published as `None` (Phase 4/7). `cell_number_negative_red` shows default color. Home: `projects/type-aware-alignment.md`, `DECISIONS` Phase 13 |
| Error values render as engine text | Auto | render `cell_error_div0`, `cell_error_name`, `cell_error_circ` |
| Horizontal alignment: **explicit** style honored | Auto | render `cell_align_left_text`, `cell_align_right_number`, `cell_align_center_explicit`, `cell_align_explicit_overrides_default` |
| Horizontal alignment: **type-based defaults** (numbers/dates right, booleans/errors center) | Known-limitation | Grid defaults all cells to left; `PublishedCell` carries no value type. Phase 6 deferred, not landed. Home: `projects/type-aware-alignment.md`, `DECISIONS` Phase 13. **Visible** in `cell_number_*`/`cell_boolean`/`cell_error_*` baselines |
| Text clips at cell boundary (no overflow/wrap) | Auto | render `cell_text_clipped`, `cell_text_exact_fit`, `cell_narrow_column_clipped_number` |
| Vertical alignment centered | Auto | render (all cell cases render vertically centered) |
| Not-rendered (borders/font family/size/color-overrides/strikethrough/wrap) silently ignored, preserved + saved | Auto | `roundtrip.rs::roundtrip_styles_preserved` (preserve); grid renders none of them (by omission); `DECISIONS`/§8 |

### §3.7 Sheets & tab bar

| Behavior | Status | Test(s) / entry |
|---|---|---|
| One tab per sheet, active distinct, click to switch | Auto | `chrome/view.rs::sheets_changed_event_updates_tabs`, `select_sheet_switches_and_notifies_grid` |
| Per-sheet scroll + selection in-session | Auto+Manual | see §3.2 per-sheet row; **M-6** |
| `+` appends `SheetN` (smallest N), switches to it | Auto | `chrome/view.rs::add_sheet_sends_command`; `worker_seam.rs::sheet_add_rename_delete_emit_sheets_changed`; naming = engine `new_sheet` (collision-avoiding). **M-7** visual |
| Double-click → inline rename; commit Enter/blur, Esc cancels; validation | Auto | `chrome/view.rs::rename_valid_sends_command`, `rename_invalid_stays_editing`, `rename_escape_reverts`; `sheet_name.rs::accepts_valid`, `rejects_empty`, `rejects_blank`, `rejects_too_long`, `rejects_each_illegal_char`, `rejects_edge_apostrophe`, `rejects_case_insensitive_duplicate`, `allows_rename_to_same_name` |
| Right-click → context menu Rename/Delete | Auto | `chrome/view.rs::delete_last_sheet_disabled`, `delete_empty_sheet_no_confirm`, `delete_with_content_confirms_then_deletes` |
| Delete requires >1 sheet + confirm when content | Auto | `chrome/view.rs::delete_last_sheet_disabled`, `delete_with_content_confirms_then_deletes`; `document.rs::sheet_properties_report_has_content` |
| Sheet ops through engine, undoable | Auto | `worker_seam.rs::sheet_add_rename_delete_emit_sheets_changed`, `undo_redo_through_worker`; `worker/run.rs::delete_sheet_reconciles_cache_map` |

## §4 The evaluate loop

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Commits go to per-window worker; UI never evaluates | Auto | architecture (worker owns model); `worker_seam.rs` full suite |
| Rapid edits coalesce (N → 1 eval) | Auto | `worker/run.rs::drain_coalesces_burst_into_one_eval`, `negative_control_eval_counter_detects_no_coalesce` |
| Publish fresh viewport + generation; grid re-pulls | Auto | `worker/run.rs::publish_then_bump_generation_ordering`; `worker_seam.rs::publish_before_bump_never_shows_a_stale_generation` |
| Staleness ≤ 1 evaluation; styles/geometry/selection/scroll never stale | Auto | `worker_seam.rs::edit_reflected_after_publish_and_reads_are_wait_free`; `cache.rs::perf_smoke_viewport_lookup_is_not_o_sheet_size` |
| Eval spinner only after 250 ms in-flight | Auto | `eval_indicator.rs::short_eval_never_shows`, `long_eval_shows_then_hides`, `started_while_in_flight_does_not_rearm`, `coalesced_back_to_back_stays_shown`, `stale_timeout_after_new_arm_noops`; `chrome/view.rs::eval_spinner_hidden_for_short_eval`, `eval_spinner_shown_for_long_eval` |
| Edits during eval accepted optimistically | Auto | `worker/run.rs` coalescing; `worker_seam.rs::edit_reflected_after_publish_and_reads_are_wait_free` |

## §5 File operations

| Behavior | Status | Test(s) / entry |
|---|---|---|
| §5.1 Native picker, `.xlsx` only | Manual+Known-limitation | Native panel; gpui `PathPromptOptions` has no in-dialog filter → enforced post-select via `LoadError::NotXlsx` (`DECISIONS` Phase 10). **M-3**/**M-16** |
| §5.1 Opening an already-open file focuses the existing window | Auto | `shell/app.rs::open_dedupes_same_path_activates_existing`; `shell/registry.rs::resolve_open_dedupes_by_path`, `set_path_then_dedupes` |
| §5.1 Large file: window opens with loading state, parse off-thread, closable to cancel | Auto+Manual | render `grid_loading_overlay`; worker async open; **M-8** (100 MB timing is manual) |
| §5.1 First paint from cached values (no recompute on open) | Auto | `worker/run.rs::load_builds_active_sheet_cache`; `roundtrip.rs::formula_errors_are_values` (cached errors survive) |
| §5.1 Failure (corrupt/not-xlsx/password/io) → dialog with name + reason, never crash | Auto | `document.rs::classify_magic_recognizes_containers`; `roundtrip.rs::open_missing_file_is_io_error`, `open_empty_file_is_not_xlsx`, `open_text_file_is_not_xlsx`, `open_truncated_zip_is_corrupt`, `open_ole_file_is_password_protected`; `worker_seam.rs::spawn_open_bad_files_emit_typed_load_failed`; `shell/app.rs::load_failed_shows_closing_error_and_clears_loading` |
| §5.2 Save writer; Save on Untitled = Save As | Auto | `worker_seam.rs::save_through_worker_roundtrips`; `shell/lifecycle.rs::save_target_untitled_prompts_with_default_name`, `save_target_titled_save_uses_path`, `save_target_save_as_prompts_with_current_name` |
| §5.2 Save As native panel, `.xlsx` enforced | Auto+Manual | `shell/lifecycle.rs::xlsx_extension_added_kept_and_replaced`; **M-9** native panel |
| §5.2 Atomic write (temp + rename); failed write never destroys existing | Auto | `roundtrip.rs::save_overwrites_existing_file`, `save_failure_missing_directory_creates_nothing`, `save_failure_preserves_destination`, `failed_save_leaves_real_existing_xlsx_byte_identical`; `worker_seam.rs::save_atomic_on_failure_leaves_destination_untouched` |
| §5.2 Save errors surface dialog; document stays dirty | Auto | `shell/app.rs::save_failed_keeps_window_and_shows_non_closing_error` |
| §5.2 Silent fidelity strip, **no** warning dialog | Auto+Known-limitation | Intentional (§5.2); `roundtrip.rs` shows what IronCalc preserves; warn-and-strip is `projects/xlsx-preservation.md` |
| §5.2 Save clears dirty; any edit sets it | Auto | `shell/lifecycle.rs::dirty_by_op_accounting`; `shell/app.rs::saved_adopts_canonical_path_and_closes_after_save` |

## §6 Errors, robustness & edge cases

| Behavior | Status | Test(s) / entry |
|---|---|---|
| Formula errors are data, never dialogs | Auto | `roundtrip.rs::formula_errors_are_values`; render `cell_error_*` |
| Circular refs → `#CIRC!`, no hang / no special UI | Auto | `roundtrip.rs::formula_errors_are_values`; render `cell_error_circ` |
| Input cap eliminates parser stack-overflow at source | Auto | `input_cap.rs` suite; `worker/run.rs::worker_side_cap_rejects_abort_reproducers_without_touching_engine` |
| Worker 64 MiB stack | Auto | `WORKER_STACK_SIZE = 64 << 20` + `.stack_size(...)` (`worker/client.rs`); exercised by `worker_seam.rs` |
| catch_unwind: caught panic drops edit, restores state, "couldn't be applied" | Auto | `worker/run.rs::catch_unwind_recovery_keeps_worker_alive`, `second_panic_degrades_and_refuses_edits`; `shell/app.rs::edit_rejected_engine_panic_shows_transient_dialog`, `edit_rejected_input_cap_flags_chrome_data_row` |
| Worker death → non-dismissable error bar + Save As, no silent loss | Auto+Manual | `shell/window.rs` `WorkerDegraded` → `degraded` bar + Save As; `worker/run.rs::second_panic_degrades_and_refuses_edits`; **M-10** visual bar |
| Sheet-name validation | Auto | `sheet_name.rs` suite; `worker_seam.rs::invalid_sheet_rename_is_rejected` |
| Formula-bar junk input → engine text/typed error, never panic | Auto | `input_cap.rs::accepts_non_formula_text`; engine handles (round-3 D) |
| Read-only location: open works, Save fails with dialog → Save As | Auto+Manual | `roundtrip.rs::save_failure_preserves_destination`; `shell/app.rs::save_failed_keeps_window_and_shows_non_closing_error`; **M-11** (real perms; container runs as root) |
| Unsaved-changes prompts on close + quit | Auto | `shell/app.rs::close_dirty_prompts_and_cancel_keeps_window`; `shell/lifecycle.rs::quitplan_*` |

## §7 Performance requirements (gates)

| Metric | Status | Test(s) / entry |
|---|---|---|
| Scroll frame p99 ≤ 8.33 ms / worst ≤ 16.67 ms (real HW); CI buffered gate | Auto | perf `perf.rs::all_gates_pass_under_target`, `frame_gate_fails_over_120fps_budget_but_within_60fps`; `render-tests/src/perf.rs` `CI_*` gates; `perf_harness --gate`. Real-HW budget = **M-12** (macos-verify) |
| Viewport cell load p99 < 2 ms | Auto | perf `perf.rs::cell_load_gate_fails_over_2ms`; harness cell-load stats |
| Zero engine calls on scroll path | Auto | `instrument.rs` counter; `document.rs::engine_call_counter_registers_real_model_work`; perf harness zero-delta assert + `SetStyleAttr` negative control |
| Edit → UI ack < 1 frame (optimistic pending) | Auto+Manual | `data_row.rs::edit_enters_editing` (pending state); **M-12** frame-timed on real HW |
| Staleness ≤ 1 eval; UI interactive throughout | Auto | `worker_seam.rs::edit_reflected_after_publish_and_reads_are_wait_free` |
| 100 MB styled open: responsive, loading ≤ parse + 2 s | Manual | **M-8** (documented-manual: large-file timing) |
| Memory: cache O(styled cells + overrides), not O(area) | Auto | `cache.rs::perf_smoke_viewport_lookup_is_not_o_sheet_size`, `excel_max_geometry_totals_match_engine` (no per-cell alloc for empty expanse) |

## §8 Explicitly out of scope

All items are intentional MVP omissions (in-cell edit, IME, clipboard, structural-edit UI,
row/col resize, dynamic arrays, merges/CF/comments/validation/hyperlinks, CSV, recent files,
printing, find/replace, sort/filter, freeze, hide, zoom, charts, named-range UI, multi-range,
fill handle, session restore, autosave, Windows). Each has a home in §8 and/or `PROJECTS.md`
(`xlsx-preservation`, `ime-text-input`, `excel-clipboard`, `viewport-cache`, `style-cache`).
No test required; absence is the behavior. Where a file contains an unsupported feature it
**opens** (round-trip tests) and the feature is silently stripped on save (`projects/
xlsx-preservation.md`).

## §9 Testing & quality bar

Meta-requirements, satisfied structurally: per-phase tested-well-enough (this matrix); the
first-class render suite (48 cases, `render-tests/`); file round-trips (`roundtrip.rs`);
worker-seam tests (`worker_seam.rs`, `worker/run.rs`); Linux CI as the gating target
(`checks.yml` + `perf-gates.yml`); `macos-verify.yml` non-required.

---

## Uncovered? — final answer

**No spec behavior is silently uncovered.** Every §2–§9 behavior is either automated,
a recorded manual-smoke item (`smoke_checklist.md` M-1…M-16), or a **known-limitation with a
home**. The known-limitations (not silent):

1. **Type-based default cell alignment** (§3.6) — grid defaults left; numbers/dates should
   default right, booleans/errors center. → `projects/type-aware-alignment.md`, `DECISIONS`
   Phase 13.
2. **`[Red]` number-format text color** (§3.6) — `text_color` published as `None`. → same
   project note, `DECISIONS`.
3. **Input-cap message-popover text** (§3.3) — danger border shown, popover text deferred. →
   chrome-polish, `DECISIONS` Phase 9.
4. **macOS Finder open-file** (§2.1) — CLI argv wired; `on_open_urls` deferred at rev. →
   `DECISIONS` Phase 10.
5. **Bundled Inter font** (§3.3/§3.6) — deferred; default font, baselines pinned to the
   runner image. → `projects/bundled-inter-font.md`, `DECISIONS` Phase 13.
6. **Save fidelity strip** (§5.2) — intentional; warn-and-strip is `projects/
   xlsx-preservation.md`.
7. **Dynamic arrays / spill** (§3.4/§8) — accepted absent for v1.

Items 1–2 are the only ones that change what a user *sees* rendered vs. the spec; both are
visible in the committed baselines and tracked. Everything else is native-OS-surface manual,
real-hardware perf manual, or an intentional MVP scope cut.
