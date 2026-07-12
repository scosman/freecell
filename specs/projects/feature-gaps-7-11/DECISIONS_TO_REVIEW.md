# Decisions to review — Feature Gaps 7_11

Judgment calls made during coding that a human may want to sanity-check. One bullet per call,
tagged by phase.

> **Phase 8 closeout sweep (2026-07-12).** Every decision below was reviewed at the render-
> validation + closeout phase. All are **accepted as shipped** (per-phase note under each heading),
> and the Phase 8 full pixel suite re-confirmed all 136 committed baselines byte-match (no incidental
> regression). The one item that was OPEN — the Phase 4 "Replace All is N undo steps" decision
> (§Phase 4, first bullet) — is now **RESOLVED in Phase 9**: the standalone `scosman/ironcalc` fork
> fix `UserModel::set_user_inputs` shipped and both FreeCell call sites were swapped, so Replace All
> is a **single undo step** (see §Phase 9 below).

## Phase 2 — Quick-edit mode (§5)

*Sweep (Phase 8): all three calls accepted as shipped.*

- **"Modified arrow" excludes the `function` modifier (cross-platform fix).** The caret-intent
  predicate is `shift || control || alt || platform` (shared helper `grid::caret_intent_modifiers`),
  NOT gpui's `Modifiers::modified()`. macOS sets `Modifiers::function` on the arrow / Home / End
  cluster itself, so `modified()` would flag a *plain* arrow as modified and defeat §5.2's
  commit-and-move on macOS (the in-container Linux gpui backend never sets `function`, so it would
  pass CI silently). This mirrors `command_for_key`, which never consults `function`. Applied to
  both the data-row arm and the (dead) grid in-cell arm; covered by
  `quick_edit_plain_arrow_with_function_flag_still_moves` and
  `quick_edit_caret_intent_modifier_arrow_leaves_without_moving`.

- **Caret-intent "leave quick-edit" via `on_mouse_down`, NOT the `on_content_event` Focus
  handler.** `architecture.md §5.1/§5.3` suggested clearing `quick_edit` "on formula-bar focus
  (`on_content_event` Focus)". Reading the code showed that would break type-to-replace:
  `InputEvent::Focus` is emitted from a *deferred* `on_focus` observer, so `begin_typed`'s
  programmatic `input.focus()` fires Focus **after** `begin_typed` returns — clearing the flag in
  the Focus handler would immediately undo the quick-edit it just set. Instead the "clicked into
  the formula bar" case is handled precisely by an `on_mouse_down` on the data-row field (which is
  exactly the §5.3 mouse case), plus Home/End and modified-arrow in the key handler. The flag
  starts `false`, so a user who clicks straight into the formula bar (no type-to-replace) never has
  it set anyway — nothing to clear. Net behavior matches the spec; only the mechanism differs from
  the pointer. Low risk.

- **Grid-root in-cell arrow arm is a defensive symmetric mirror (currently unreachable).** The
  task asked to intercept arrows in both the data-row and the grid-root in-cell `capture_key_down`
  and to thread `quick_edit` into the grid via `ChromeGridRequest::EditState`. Both are done. But
  in the current flow type-to-replace edits live in the **data row** and never open the in-cell
  overlay, and `begin_in_cell` clears `quick_edit`, so `incell_open.is_some() && quick_edit` cannot
  co-occur — the grid arm is dead in practice. It is implemented for symmetry / future-proofing (a
  future overlay-hosted quick-edit) and commented as such. No `leave_quick_edit` routing was added
  to the grid arm (it would need a new `GridEvent` + window route for a path that can't execute).
  Flag if a future feature makes the overlay host a quick-edit — the leave-cases would then need
  wiring back to the chrome.

## Phase 3 — Text spill / overflow (§2)

*Sweep (Phase 8): all accepted as shipped. The three legitimately-changed pre-existing baselines
(`cell_text_clipped`, `grid_mixed_content`, `font_size_24_row_grown`) were re-confirmed byte-matching
their committed versions in the Phase 8 full-suite run — no reviewer took the "block the neighbour"
cosmetic option, so they stay as the genuine spills the feature produces.*

- **Width gate is a cheap gpui-free ESTIMATE, not a real glyph measure.** `architecture.md §2.1.2`
  wanted to avoid a per-cell glyph measurement on the render hot path. I gate spill with
  `layout::text_overflows_column`, which uses `estimated_text_width` (≈ `0.5em` average glyph
  advance × char count). This keeps `build_grid_layers` **gpui-free** (no `Window` threaded into the
  hot loop; the perf harness `measure_frame` path is unchanged) and — crucially — lets every
  comfortably-fitting text cell take the **identical existing** `cell_element` path so its pixels are
  byte-unchanged. Trade-off: the estimate ignores font family/weight and exact glyph metrics, so a
  text that overflows its column by only a hair might not spill (a conservative miss, acceptable per
  the architecture). It correctly caught every genuine overflow in the render suite and left the
  snug-fit `cell_text_exact_fit` ("Exactly" @ 62 px) non-spilling. Flag if we later want
  Excel-exact spill on borderline overflows — that needs a real UI-thread measure (as auto-grow,
  Phase 7, will introduce anyway).

- **Spill is bounded to the visible frame columns (`[frame.cols.start, frame.cols.end-1]`), not
  scanned into the publication-covered overscan.** `architecture.md §2.1.4` suggested consulting
  `Publication::covers` *past* the frame edge. Since the spill element is clipped to the content
  viewport, painting past the frame edge is invisible, and the frame already carries a small render
  overscan — so I stop the scan at the frame edge. The coverage guard still runs *within* the frame
  (`neighbor_occupancy` returns `Blocked` when `!publication.covers`), so a transient where the
  publication lags the frame never false-spills, and we never treat "beyond covered" as empty
  (func spec §2.5). Net behaviour matches the spec; the scan is simpler and strictly O(visible cols).

- **Left + center spill were BOTH implemented (not punted).** The scope note (func spec §2.2,
  `architecture.md §2.3`) made rightward the must-have and left/center punt-able. The spill-rect math
  is symmetric (`span_rect` + `SpillDirection`), so all three ride one code path at no extra cost —
  covered by `spill_left_right_aligned` and `spill_center_both`.

- **Three PRE-EXISTING baselines legitimately changed (genuine spills, NOT a leak).** The isolation
  check found exactly three tracked baselines byte-changed by this phase, all because their scenes
  already contained wrap-off TEXT that overflows its column into empty neighbours, so the feature now
  (correctly) spills it: `cell_text_clipped` (long text → spills right across C2/D2), `grid_mixed_content`
  (the B7 "clipped-long-note-text" note → spills ~8 visible glyphs into the previously-blank C7 — a
  clearly visible change, but small enough in area that it stays under the render harness's *tolerant
  diff metric*, which is why the generate tool reported "unchanged" and a `test` run would not have
  failed it against the stale baseline; verified visually that ONLY B7 changed, the rest of the canary
  is byte-identical, and the Phase-8 full-suite eyeball is the confirming check on that), and
  `font_size_24_row_grown` ("Sample" at 24 pt genuinely overflows the
  100 px column — HEAD showed it clipped to "Sampl"; now the "e" spills into C2). All three were
  eyeballed. These changes are unavoidable for any correct spill implementation (the scenes contain
  overflowing text with empty neighbours) and are the feature working as intended — not spill leaking
  into unrelated cells. Every other tracked baseline is byte-identical. If a reviewer prefers those
  three cases to keep demonstrating their original intent (clip / font / canary) without incidental
  spill, block the neighbour (or widen the column) in the scene — a cosmetic test-fixture choice, not
  a correctness issue.

## Phase 4 — Find / replace (§4)

*Sweep (Phase 8): all calls accepted as shipped **except the first bullet** — **Replace All is N undo
steps**, whose single-undo fix was deferred to Phase 9 (a standalone `scosman/ironcalc` fork fix + the
two FreeCell call-site swaps). **Phase 9 is now DONE** — Replace All is a single undo step (see §Phase 9).
The other four Phase-4 calls (search.svg from the bundle, `ReplaceOne` worker-computes, select-on-open
via `on_next_frame`, toggle glyph + non-degraded search button) are accepted as-is.*

- **Replace All is N undo steps for now — its own final phase (Phase 9) delivers single-undo via a
  standalone ironcalc fork fix (verified IronCalc gap). — STILL OPEN (Phase 9 not started).** `functional_spec.md §4.4` requires Replace
  All to be ONE undoable batch. IronCalc **can't** group scattered writes from FreeCell's accessible
  API: paste's single-undo mechanism (`History::push` / `UserModel::push_diff_list`) is `pub(crate)`,
  and the public rectangle pastes (`paste_csv_string` clears+rewrites the whole rectangle;
  `paste_from_clipboard` needs the crate-private `ClipboardCell`) are unusable for scattered
  find/replace matches without a FreeCell-side hack CLAUDE.md forbids. **Interim shipped:**
  `Command::ReplaceAll` works fully (one eval, one publish) but records one engine undo entry per
  changed cell — the accepted `SetFont` "K+1 undo steps" precedent. **Resolution (project owner):
  this is NOT folded into Phase 6 — it is its own new standalone final phase, `implementation_plan.md`
  Phase 9, with its own clean single-feature ironcalc `fix/` branch + upstream PR, independently
  revertible.** Phase 9 adds `UserModel::set_user_inputs(&[(sheet,row,col,String)])` (one `diff_list`,
  no rectangle clear) to the fork, folds it into `freecell-fixes`, re-pins FreeCell + bumps
  `Cargo.lock`, then swaps the **two** isolated FreeCell call sites so ReplaceAll becomes one undo
  step: (1) `document.rs::replace_all_matches` (the per-cell `set_user_input` loop → one batch call),
  and (2) `worker/run.rs::apply_replace_all` (which currently pushes one `Touch::Cells` + one
  `ops_seen` per changed cell and must **collapse to a single undo touch/op** when the batch lands).
  Full write-up: `phase_plans/phase_4.md` §ROADBLOCK.

- **`search.svg` is referenced from the gpui-component bundle, NOT vendored.** The task/`ui_design §2`
  said to add a NEW vendored `search.svg` to `FREECELL_ICONS`. But the bundle **already ships**
  `icons/search.svg` (tintable) AND gpui-component itself renders it via `IconName::Search`. Vendoring
  it would *shadow* a bundle icon (violating `assets.rs`'s documented "FreeCell icons are disjoint
  from the bundle" invariant) and change gpui-component's internal Search rendering. So the action-row
  button uses the bundle's `icons/search.svg` directly — exactly the existing precedent for
  `panel-right/left/bottom` (referenced from the bundle, not vendored). Net effect identical (a
  tintable magnifier); nothing vendored, nothing shadowed. Covered by `find_bar_icons_all_resolve`.

- **`Command::ReplaceOne` is a new command (worker computes), not a reused `SetCellInput`.** The task
  said "reuse `SetCellInput` — prefer the WORKER computing the replacement (avoid a stale-content
  race)." Those pull opposite ways (reuse = UI computes; worker-computes = new command). I honored the
  parenthetical intent: `ReplaceOne` carries `(cell, query, replacement, flags)` and the worker
  re-reads fresh content + applies the shared `replace_in_cell` helper, so there is no stale-content
  race and single-cell replace shares one predicate with Replace All. A single-cell replace is
  inherently one undo step, so this adds no undo concern. Low risk.

- **"Select existing find text on open" IS implemented — no fork, via `on_next_frame` + gpui-component
  `SelectAll`.** `InputState::select_all` is `pub(super)` (no public selection setter), but the
  `SelectAll` **action** is public (`gpui_component::input::SelectAll`) and the field's `Input` element
  registers a handler for it. `open_find` focuses the field, then schedules a dispatch of `SelectAll`
  to the field's focus handle via `window.on_next_frame` (the field must be in the rendered dispatch
  tree first — a `defer` runs before the repaint and would fizzle). No gpui-component fork. The unit
  harness does not auto-draw on notify, so `on_next_frame` can't fire in-test (it does in the real
  event loop); `open_find_selects_existing_text` instead drives the same `SelectAll`-on-the-focused-
  field dispatch and asserts the whole value is selected, verifying the mechanism the on-open
  scheduling relies on.

- **Match-entire-cell toggle glyph is `▢` (U+25A2); the search button is NOT degraded-disabled.**
  The match-case/whole-cell toggles use text labels ("Aa" / "▢") per the `ui_design §1` mock (tooltips
  carry the meaning). And the action-row search button stays enabled in degraded/read-only mode
  because **find is a read** — only the bar's Replace / Replace All are gated on `degraded` (and on
  having a current match / any matches). Consistent with "every *mutating* control disables".

## Phase 6b — Sheet reorder wiring + tab drag (§6)

*Sweep (Phase 8): all five calls accepted as shipped. (Phase 6a's fork `set_worksheet_index` — folded
into `freecell-fixes`, no upstream PR yet by owner's choice — is recorded in `phase_plans/phase_6a.md`.)*

- **Drag `on_mouse_move` / `on_mouse_up` live on the tab-bar CONTAINER, not per-tab (as the task
  literally suggested).** gpui gates `on_mouse_move` on `hitbox.is_hovered` — a per-tab move handler
  only fires while *that* tab is under the pointer, so it goes dead the instant the drag crosses onto
  a neighbor (and gaps between tabs fire nothing). The full-width tab-bar container reliably tracks
  the drag across tabs and the release. `on_mouse_down(Left)` stays per-tab (it must know which sheet
  was pressed). This is the `ResizeDrag` pattern (manual state + move/up on a wide element), not
  gpui's built-in drag-and-drop. Net behavior matches §6; only the handler placement differs from the
  prompt's per-tab wording.

- **Click-select is left to fire on a drop-back-on-origin (no extra suppression guard).** gpui forms
  `on_click` only when the pointer releases over the SAME tab it pressed. A real drag to a different
  slot releases over a different tab, so the origin tab's `on_click` never fires and no other tab's
  does either (no pending mouse-down there) — the container's `on_mouse_up` alone sends `MoveSheet`.
  The only overlap is a past-threshold drag that returns to and releases on the origin tab: that's a
  no-op move (no command) AND `on_click` fires `select_sheet(origin)` — harmless (it just reselects
  the sheet the user was dragging). So double-click→rename (needs `click_count ≥ 2`) and
  right-click→menu (`on_mouse_down(Right)`) are naturally guarded against a left-drag without a
  dedicated flag. Low risk; flag if a reviewer wants a drop-on-origin to be a strict no-op with zero
  select.

- **A same-index (`to == from`) MoveSheet is never SENT, not just ignored — deliberate, to keep the
  fork-history and FreeCell `undo_touches` stacks 1:1.** The fork's `set_worksheet_index` no-ops a
  same-index move WITHOUT recording history, but the worker's `apply_one` would still count a
  `SheetOp` and push a FreeCell `Touch::Sheets`, desyncing the parallel undo-bookkeeping stack (same
  latent characteristic the existing same-name `RenameSheet` has). `tab_move_target` returns `None`
  for a drop on the origin slot, so the UI sends nothing — the correct layer to enforce it (mirrors
  §6.4 "dropping back on its original position is a no-op, no engine command, no undo step"). No
  worker-side no-op guard was added (out of scope; would need a non-counting `AppliedKind`).

- **Tab geometry is captured by per-tab `canvas` bounds probes (window x), read back via the pure
  `tab_insertion_index` helper — no glyph measurement.** Tabs are content-width (variable), so the
  insertion math needs real laid-out bounds, not a computed width. Each tab embeds a zero-cost
  `canvas` probe (the `anchored_trigger` idiom) that upserts its window-space span into `tab_spans`
  with NO `notify` (the value is consumed on the next mouse event, so it can't render-loop). The
  reorder computation is fully unit-tested via the pure `tab_insertion_index` + `move_target_for_gap`
  fns; the gpui view tests set `tab_spans` directly because the unit harness does not paint (so the
  probes never run in-test — same constraint the Phase 4 `on_next_frame` note called out).

- **Release OUTSIDE the tab strip leaves the drag pending until the next tab press.** The move/up
  handlers only cover the tab bar; a release over the grid won't clear `tab_drag`. The lift +
  indicator are driven by drag STATE (`tab_drag_active()`), not by the pointer being in the strip —
  so an out-of-strip release leaves `tab_drag` set and, with it, the lift/indicator visible until the
  next tab `on_mouse_down` resets the state (it self-heals on the next press). Matches the grid's own
  resize-drag scoping (handlers on the grid area). Acceptable for a tab-strip interaction; flag if a
  stuck indicator is ever observed in practice.

## Phase 7 — Auto-grow rows (§3)

*Sweep (Phase 8): all six calls accepted as shipped. The six net-new `autogrow_` baselines they produced
were re-confirmed byte-matching in the Phase 8 full-suite run, and the isolation finding (zero
pre-existing baselines moved) holds.*

- **Wrap auto-grow is a CACHE-ONLY geometry update — it never touches IronCalc, `ops_seen`, or the
  undo stacks — so it adds NO user-visible undo step (§3.4).** `architecture.md §3.2` said heights
  "still live in `SheetCache`/IronCalc, still undoable, still saved," but §3.4 also required "no
  separate undo entry." These pull opposite ways: `set_rows_height` records IronCalc history (an undo
  step) and FreeCell mirrors it 1:1 with a `Touch`, so routing wrap growth through it would add the
  very undo step §3.4 forbids. I resolved it in favour of §3.4: `Command::AutoGrowRowHeights` mutates
  only the resident `SheetCache` row axis (`max(base_ironcalc, wrap)`), bumps nothing, pushes no
  `Touch`. Consequence (accepted, matches the func-spec "session-scoped" framing for the manual flag):
  wrap-grown heights are **not persisted to xlsx** and are recomputed on next open — the same
  session-scoped posture §3.3 already takes. The pre-existing font-size (`SetFont`) and explicit-
  newline (IronCalc) auto-grow paths are **unchanged** and still undoable/saved.

- **Wrap heights are persisted in WORKER state (`wrap_heights: HashMap<SheetId, BTreeMap<u32,f32>>`)
  and re-projected on every full cache rebuild.** Because the cache-only height would otherwise be
  wiped whenever the sheet cache is rebuilt from IronCalc (any resize / insert / delete / band edit),
  the worker keeps the wrap contribution and re-applies `max(base, wrap)` in `build_and_store_cache`.
  This is what makes a grown row survive an unrelated rebuild (covered by
  `auto_grow_survives_rebuild_and_shrinks_back`). `build_and_store_cache` became `&mut self` (cascade:
  `refresh_cache_cells`, `ensure_active_cache_built`) — all callers already held `&mut self`.

- **The UI (`run_autogrow`) is MANUAL-AGNOSTIC; the WORKER enforces "auto rows only".** The task's
  step 3 says emit "AND the row is not manual." Threading the manual set into the read-model cache so
  the render thread could pre-filter would add coupling and hurt the phase's revertibility, and buys
  little: the grid emits at most **one** `AutoGrowRowHeights` per genuine wrap-input change (the
  signature guard), and the worker skips manual rows outright, so a manual row is never grown and
  never re-emitted in a loop. Net behaviour matches §3.3; the manual check lives solely in the worker
  (`apply_auto_grow` + `mark_rows_manual`, covered by `user_resize_marks_manual_and_auto_grow_skips_it`).

- **Convergence rests on a per-row wrap-INPUT signature (content/font/column-width), NOT the row
  height.** A dirty row is one whose signature changed; a settled row's height-only republish leaves
  the signature unchanged → not dirty → no re-emit → the loop converges in one frame (the "dirty set
  empties" assertion, `autogrow_measures_wrapped_height_and_emits_once_then_converges`). The worker
  additionally republishes `StyleCacheUpdated` only when a height actually moved (a double guard).

- **Line-height factor = gpui's real default `phi` (`1.618034`), not a made-up 1.25.** The first
  baseline generation clipped the top wrapped line: gpui's `Style::line_height` default is `phi()`
  (golden ratio), so a grown row must reserve `round(1.618 * font_px)` per line or it under-grows.
  Using the true factor makes the grown row fit exactly the lines gpui paints (verified by eyeballing
  the regenerated baselines).

- **Cap `MAX_AUTO_ROW_HEIGHT_PX = default * 10 = 240 px` (~10 lines).** Homed in `freecell-core::cache`
  next to `DEFAULT_ROW_HEIGHT_PX` and shared by the UI clamp + the worker's defensive re-clamp; content
  beyond it clips within the cell (`autogrow_cap_clip`).

- **Render cases run the REAL measurement via an OPT-IN harness hook (`autogrow_measure_now`); NO
  pre-existing baseline moved.** The pixel harness renders a single static frame over a shut-down
  worker, so the live measure→worker→republish loop can't round-trip in-capture (the same limitation
  `font_size_24_row_grown` calls out with "simulated by the injected height"). Rather than hand-inject
  heights, an **opt-in** (`RenderCase::auto_grow`) hook runs the product's real wrap measurement once,
  pre-first-paint, and applies the measured heights straight to the shared cache (skipping rows that
  already carry a non-default override = manual). Because it is opt-in, only the six new `autogrow_`
  cases grow — the **isolation check found ZERO pre-existing baselines changed** (all six are net-new
  files; every other `.png` is byte-identical). Trade-off: pre-existing default-height wrap scenes
  (e.g. `spill_wrap_on_no_spill`) are **not** re-grown in the harness, so they keep their committed
  (clipped) look — a deliberate containment choice (keeps the change to `autogrow_` only, per the phase
  brief) rather than a correctness gap; a follow-up could refresh them if desired.

## Phase 9 — Replace All single-undo (ironcalc fork `set_user_inputs` + FreeCell swap)

- **The fork `set_user_inputs` OMITS the per-cell row-height auto-grow that single-cell
  `set_user_input` does — it follows the `paste_csv_string` precedent instead.** Single-cell
  `set_user_input` appends a `SetRowHeight` diff when multi-line content needs a taller row; the
  multi-cell writer `paste_csv_string` does **not**. Since the batch method is the scattered-cell
  analogue of paste (its whole reason to exist is grouping many writes into one diff_list), I mirrored
  paste: one `SetCellValue` diff per cell, no row auto-grow. Keeps the batch semantics predictable and
  the diff_list minimal; row auto-grow for a find/replace is out of scope (and FreeCell's own wrap
  auto-grow, Phase 7, is a separate cache-only path anyway). Flag if a batch write is ever expected to
  auto-grow rows for newline-bearing replacements.

- **The batch validates ALL coordinates up front (all-or-nothing) and records a spill cell's old
  value as `None`.** Up-front validation (sheet exists + `is_valid_row`/`is_valid_column_number` for
  every entry, before any mutation) means an out-of-range entry mid-list can't leave a partial,
  history-less write — the method either applies the whole batch or rejects it cleanly. The spill-cell
  `old_value == None` handling copies single-cell `set_user_input` exactly (undo re-spills from the
  anchor), which `paste_csv_string` happens to skip. Both are conservative correctness choices for a
  general-purpose public method. Covered by `out_of_range_batch_is_rejected_without_mutating`.

- **The worker records ONE `Touch::Ranges` for the whole Replace All (not one `Touch::Cells` per
  cell) — the `commit_paste` pattern.** The fork's batched `set_user_inputs` is a single engine undo
  entry, so `apply_replace_all` now increments `ops_seen` by 1 and pushes a single `Touch::Ranges`
  covering every changed cell, keeping the FreeCell undo/touch stack 1:1 with the engine history (so a
  single Undo pops exactly one touch and reverts the entire replace). This replaces the interim's
  per-cell `Touch::Cells` + `ops_seen += n`. Verified by `replace_all_is_a_single_undo_step`
  (scattered cells, one `Command::Undo` restores all) and the touch-count assertion (`== 1`).

- **The single-undo fix is its OWN clean fork branch `fix/batch-set-inputs` off `main`, folded into
  `freecell-fixes` alongside sheet-reorder — NOT bundled with Phase 6's branch.** Per CLAUDE.md
  (one fix = one branch = one focused upstream PR). `freecell-fixes` now carries both
  `set_worksheet_index` (Phase 6a) and `set_user_inputs`. **No upstream PR opened** — owner offers
  upstream later. FreeCell re-pinned `Cargo.lock` `a49cfd60 → 7d4b215e` (branch pin unchanged); aside
  from the two `ironcalc`/`ironcalc_base` rev bumps the lock also moved one benign transitive edge —
  `iana-time-zone`'s `windows-core 0.57.0 → 0.58.0`, `cfg(windows)`-only so never compiled on
  FreeCell's macOS/Linux targets (harmless churn, same class as the Phase-6a note; left as resolved).
  The swap is a clean, self-contained, independently-revertible change (reverting it returns to the
  multi-undo interim without touching Find or ReplaceOne).

- **Observation (not a decision): the fork repo was renamed `scosman/ironcalc` → `scosman/IronCalc`,
  which left this container's `origin/*` remote-tracking refs stale.** A `git fetch` through the
  redirect did not refresh them, so `git log origin/freecell-fixes` reported an old tip and
  `git branch -r` was missing `fix/sheet-reorder`. The pushes themselves succeeded, and `git ls-remote
  https://github.com/scosman/IronCalc.git` confirmed the authoritative state:
  `fix/batch-set-inputs=a51cf46c`, `freecell-fixes=7d4b215e`, `fix/sheet-reorder=21cde336`,
  `main=cedba4ea`. So Phase 6a's pushes had landed all along (the stale local refs, not a missing
  push, caused the earlier confusion). Consider updating the fork remote URL to the new casing to
  avoid future stale-ref surprises.

## Follow-up bug fix — Quick-edit Left/Right didn't commit+move (real-dispatch routing)

*User-reported after Phase 2 shipped: in quick-edit (type-to-replace), Up/Down committed + moved
the active cell but Left/Right instead moved the text caret inside the data-row field and neither
committed nor moved. This was a real gpui key-dispatch routing bug the Phase-2 unit tests missed
because they call `handle_data_row_edit_key` directly instead of routing an actual keystroke.*

- **Root cause: in this gpui build, key-binding *actions* dispatch BEFORE `capture_key_down` /
  `on_key_down` listeners, and the gpui-component single-line `Input` binds Left/Right to caret
  actions — so our ancestor capture listener could never preempt them.** `Window::dispatch_key_event`
  runs matched action bindings first (its `for binding in match_result.bindings { dispatch_action_on_node(…) }`
  loop) and only calls `finish_dispatch_key_event` (which fires the key-down listeners) *afterwards*,
  and only if propagation is still alive. gpui-component's `Input` registers `on_action(InputState::left/right)`
  unconditionally (`crates/ui/src/input/input.rs`), bound to `MoveLeft`/`MoveRight` in its `Input`
  key-context. So a real Right: the input's `MoveRight` action fires, moves the caret, and (bubble-phase
  actions stop propagation by default) sets `propagate_event = false` — so `finish_dispatch_key_event`
  never runs and the data-row `capture_key_down` + its `cx.stop_propagation()` never fire. Up/Down
  "worked" only by accident: the single-line `Input` registers its `up`/`down` `on_action`s **only in
  multi-line mode** (`.when(state.mode.is_multi_line(), …)`), so `MoveUp`/`MoveDown` had no handler,
  propagation survived, `finish_dispatch_key_event` ran, and our capture listener did the commit+move.
  A capture listener is therefore structurally incapable of beating the input's Left/Right; the
  original Phase-2 direct-call tests couldn't see this because they never went through gpui dispatch.

- **Fix: route the data-row edit keys through `App::intercept_keystrokes` instead of an ancestor
  `capture_key_down`.** A keystroke interceptor is the one phase gpui runs *before* action-binding
  dispatch, and its docs state `cx.stop_propagation()` there prevents action dispatch. `ChromeView::new`
  now registers one interceptor (stored in `_subscriptions`, so it lives/dies with the view), guarded to
  fire only when *this* view's `content_input` holds focus (never the in-cell overlay's own input or an
  unrelated field). It delegates to the same `handle_data_row_edit_key` the direct-call unit tests use;
  when that returns "consumed" it calls `stop_propagation`, which preempts the input's `MoveLeft`/
  `MoveRight` so Left/Right now commit+move exactly like Up/Down/Tab. Home/End and modified arrows return
  "not consumed" → no `stop_propagation` → they still reach the input's caret/selection actions (and
  `handle_data_row_edit_key` leaves quick-edit for them). The redundant data-row `capture_key_down` was
  **removed** — keeping it risked a double commit on a cap-rejected edit, since after the interceptor
  stops propagation `finish_dispatch_key_event` still fires the first capture listener once.

  Alternatives rejected: binding our own action in the data-row's key-context can't win — gpui resolves
  competing bindings by context *depth* (deepest wins), and the focused `Input`'s context is always
  deeper than any ancestor's; a `Marker > Input` predicate only *ties* at max depth and then depends on
  the fragile keymap registration-order tiebreak. The interceptor wins deterministically regardless of
  depth or registration order.

- **New real-keystroke tests drive actual gpui dispatch (they fail pre-fix, pass post-fix).**
  `quick_edit_real_keystroke_arrows_commit_and_move` (all four directions), `…_left_commits_and_moves`
  (the isolated user repro), plus `…_modified_arrow_leaves_without_moving` and `…_home_leaves` for the
  preserved caret-intent paths — all via `VisualTestContext::simulate_keystrokes` through the focused
  data-row input. The Left/Right cases fail against the pre-fix code (caret moved, no commit/move),
  which is exactly the gap the direct-call tests couldn't catch. The original direct-call
  `handle_data_row_edit_key` tests are kept unchanged.

## Follow-up bug fix — LibreOffice xlsx: custom row heights + wrap dropped on load (ironcalc fork `fix/xlsx-bool-import`)

- **Root cause was an IronCalc importer bug, not FreeCell.** A user's LibreOffice-authored `.xlsx`
  loaded with wrong row heights (tall title rows rendered at the default height until edited) and its
  wrap-on cells were not wrap-on after import. The IronCalc xlsx importer parsed `xsd:boolean`
  attributes with helpers (`get_bool` / `get_bool_false` in `xlsx/src/import/util.rs`) that only
  recognised the Excel lexical form `"1"`/`"0"` and silently mishandled the equally-valid ECMA-376 /
  ISO 29500 form `"true"`/`"false"`. LibreOffice emits `customHeight="true"` and `wrapText="true"`, so
  the custom-height flag and wrap flag were dropped on import (the real `ht` value survived, but with
  `custom_height=false` FreeCell never seeds the override; wrap was lost outright). The row importer in
  `xlsx/src/import/worksheets.rs` also parsed `customHeight`/`customWidth`/`customFormat`/`hidden` with
  inline `matches!(…, Some("1"))` checks that had the same blind spot.

- **Fixed upstream-first in the fork, no FreeCell workaround.** Per CLAUDE.md (fix the fork, don't hack
  FreeCell), the parse is corrected on a new single-feature branch `fix/xlsx-bool-import` (off `main`;
  folded into `freecell-fixes`): `xsd:boolean`'s lexical space is exactly `{true, false, 1, 0}` (W3C XML
  Schema Part 2), so the helpers now accept all four (`"1"`/`"true"` → true, `"0"`/`"false"`/absent →
  false, case-insensitive + whitespace-tolerant) and the row importer's inline checks route through the
  shared helper. It deliberately does **not** over-accept — tokens outside the `xsd:boolean` lexical
  space (`"yes"`, `"on"`, garbage) fall back to the schema default, never read as true — so it stays
  spec-compliant and Excel `"1"`/`"0"` files are unaffected. Upstream-style unit tests cover all four
  forms + the non-over-accept cases. **No upstream PR yet** (owner opens it).

- **FreeCell honors it via existing code.** With the flags no longer dropped, FreeCell's existing
  `build_sheet_cache` seeds the custom row-height override (its `r.custom_height` gate) and carries
  `wrap_text` into `RenderStyle::wrap` — **no compensating workaround was added** (in particular the
  previously-reverted cache.rs `>default` row-height seeding was **not** re-added; the importer fix makes
  it unnecessary). `app/Cargo.lock` was re-pinned to the new `freecell-fixes` head. A new regression test
  (`cache::tests::libreoffice_true_form_booleans_honored_on_load`, fixture
  `tests/fixtures/libreoffice_custom_height_wrap.xlsx`) loads the real LibreOffice workbook and asserts
  the title/subtitle/header rows load at their stored tall heights on load with no edit
  (~50.6 / 27.0 / 41.6 px) and that the previously flag-dropped wrap cells (B5:H5) import wrap-on.
