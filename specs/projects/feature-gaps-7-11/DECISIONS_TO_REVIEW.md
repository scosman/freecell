# Decisions to review — Feature Gaps 7_11

Judgment calls made during coding that a human may want to sanity-check. One bullet per call,
tagged by phase.

## Phase 2 — Quick-edit mode (§5)

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
