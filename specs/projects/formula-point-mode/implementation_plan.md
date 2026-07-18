---
status: draft
---

# Implementation Plan: Formula Point-Mode + Range Highlighting

Six phases. Build order front-loads the **pure, no-pixel** foundation (tokenization seam +
palette/predicate), then the **grid outlines** (ships user-visible value with **no** vendored-widget
dependency), then the **point-mode routing**, then the **vendored `InputState` highlight API** +
editor-token coloring, then **consolidation cleanup** + the v1.0 GAP entry, and finally the single
**render-validation** phase. Each coding phase: build **crate-scoped**, `cargo fmt --all --check`
(whole workspace — a `render-tests`/sibling-crate edit must not slip a fmt violation past a
crate-scoped check), unit/gpui tests + (grid phases) a render **subset**, commit + push. The **full**
render suite + CI `render` gate run **once**, in Phase 6.

Section numbers reference [`architecture.md`](architecture.md) (§1–§9) and
[`components/formula_editor.md`](components/formula_editor.md). Behaviour + DPM.1–8 are locked in
`functional_spec.md`; the six architecture questions are resolved in `architecture.md`. Run cargo
from `app/`.

## Phases

- [ ] **Phase 1 — Tokenization seam + pure foundation (§1, §2).** No pixels, no gpui.
      - `freecell-core`: add `RefToken` (`refs.rs`), `is_reference_ready` (`functions.rs`, beside
        `is_function_position_prev`/`in_string_at`), `RefColor` + `REF_HIGHLIGHT_PALETTE` (7 authored
        light/dark pairs) + `ref_color` + `assign_ref_colors` (`palette.rs`).
      - `freecell-engine`: add `formula_refs.rs` with `pub fn lex_formula_refs(edit_text,
        active_sheet_name) -> Vec<RefToken>` (real IronCalc `Lexer`, Model-free, strip leading `=`
        + `+1` byte-offset mapping, `same_sheet` via active-sheet-name compare); re-export from
        `lib.rs`. **Bind the exact `TokenType::Reference`/`Range` field names against the pinned
        fork** (the fork is not in this repo; the round-3 audit confirms the variants —
        `experiments/round-3/B-api-audit/src/formula_helpers.rs`).
      - Checks: `cargo build -p freecell-core -p freecell-engine`; `cargo test -p freecell-core
        --lib -p freecell-engine --lib`; the §8 unit/integration tables (`is_reference_ready` truth
        table, `assign_ref_colors`, palette len/wrap, `lex_formula_refs` over crafted formulas incl.
        cross-sheet + partial). `cargo fmt --all --check`. Commit + push.

- [ ] **Phase 2 — Shared color map + grid outlines (§2.4, §3.1, §4.1, §6).** Ships value with **no**
      vendored-widget dependency; no editor-token coloring yet.
      - `freecell-app` chrome: promote the formula-feature state onto `EditController`
        (`components/formula_editor.md §1.2-1.4` — relocate `autocomplete`/`sig_hint`, add
        `ref_tokens`/`ref_colors`, `recompute_formula`); generalize `recompute_autocomplete` →
        `recompute_formula_edit_state` wired into both Change handlers + the caret-move paths.
      - Extend `ChromeGridRequest::EditState` + `GridView::set_edit_state` with `reference_ready`,
        `pending_ref`, `ref_outlines` (§3.1); `refresh_edit_grid_state` fills them (`ref_outlines`
        = same-sheet subset). (`reference_ready`/`pending_ref` are pushed now but only *consumed* by
        the grid in Phase 3 — plumb them here so the payload lands once.)
      - Grid: store the three fields; paint outlines in the overlay pass (§4.1, border-only, themed
        slot color, clipped like the selection overlay); clear on `set_active_sheet`.
      - Checks: `cargo build -p freecell-app` (+ `-p freecell-engine`); gpui view tests (outlines
        appear for same-sheet refs, absent for cross-sheet, cleared on commit); **render subset**
        `render_tests.sh test formula_ref` / `… test incell_` while iterating. `cargo fmt --all
        --check`. Commit + push.

- [ ] **Phase 3 — Point-mode routing (§3, §5).** The `InsertReference` path + pending-ref.
      - `GridEvent::InsertReference { a1, replace_pending }` (`grid/mod.rs`); route in `make_grid_sink`
        (`shell/window.rs`) → `ChromeView::insert_reference`.
      - `insert_reference` (chrome; analog of `accept_autocomplete`) + `pending_ref` lifecycle on
        `EditController` (set on insert; cleared on any non-point transition; own-insert doesn't
        clear — §5).
      - Grid: `mouse_down_cell` point branch (`point_ready = reference_ready || pending_ref`, emit
        `InsertReference`, no `SelectionChanged`); `point_drag` state machine (move/preview/up,
        replace-on-grow, dedupe); `resolve_merge_anchor` + `expand_range_for_merges` over
        `cache.merges()` (Q6); auto-scroll + early-return guards extended for `point_drag`.
      - Checks: `cargo build -p freecell-app`; gpui view tests (routing point-vs-commit;
        append/replace/self-ref; pending lifecycle; point-drag range + merges; the autocomplete→point
        happy path). **Render subset** `… test formula_ref` (point-preview vs selection vs outline).
        `cargo fmt --all --check`. Commit + push.

- [ ] **Phase 4 — Vendored `InputState` highlight API + editor-token coloring (§4.2-4.4,
      `components/formula_editor.md §2`).**
      - Vendored gpui-component: add `HighlightSpan` + `InputState::set_highlights`/`clear_highlights`
        (additive `highlights` field + run-color override at paint; empty = no-op). Confirm the
        internal offset unit against the input's `cursor()`/`Position`/run conversions. Carry the
        upstream-style unit test. Bump/patch the `app/Cargo.toml` pin; prepare the single-feature
        upstream PR (`components/formula_editor.md §2.3`).
      - `freecell-app`: `ref_slot_rgba`/`ref_slot_rgb` theme helpers; `EditController::highlight_spans`;
        `recompute_formula_edit_state` drives `set_highlights` on `content_input` **and**
        `edit.in_cell()`; `clear_highlights` on non-formula/commit/cancel.
      - Checks: `cargo build -p freecell-app` (rebuilds the vendored crate); gpui view tests
        (`set_highlights` fan-out to both inputs; cross-sheet token colored but not outlined; cleared
        on commit); **render subset** `… test incell_` (in-cell token colors). `cargo fmt --all
        --check`. Commit + push.

- [ ] **Phase 5 — Consolidation cleanup + v1.0 GAP entry (§6).**
      - Finish the consolidation: confirm the two host adapters carry **zero** formula logic (grid =
        primitives + paint; data-row bar = render only); remove any now-dead scattered fields; the
        migration-step-1 regression gate (all shipped autocomplete + `data_row` tests green).
      - **Add the `GAPS.md` v1.0 entry** for **cross-sheet point-mode insertion** (click another
        sheet's tab mid-formula → insert `Sheet2!A1`), per `functional_spec.md §6` (note the
        asymmetry: cross-sheet *highlighting* of already-typed tokens ships in v0.5; only cross-sheet
        *insertion* + cross-sheet *grid outlines* are deferred).
      - Checks: `cargo build -p freecell-app`; full crate-scoped test run for the touched crates;
        `cargo fmt --all --check`. Commit + push.

- [ ] **Phase 6 — Render validation (dedicated late phase, §9).** No new behaviour.
      - Regenerate + **eyeball** baselines: add `formula_ref_outlines_same_sheet`,
        `incell_editor_token_colors`, `formula_ref_point_preview`; refresh any shifted
        `incell_editor_*`/`selection`/`cell_*`.
      - Run the **full** pixel suite once under `timeout` + ~10-min watchdog
        (`app/render-tests/scripts/render_tests.sh test`); fix/accept each diff; commit refreshed
        baselines with sign-off.
      - Dispatch the CI **`render`** gate on the branch (`gh workflow run render.yml --ref <branch>`),
        poll to green. `cargo fmt --all --check`. Commit + push.

## Notes for the build

- **No worker command, no engine-state mutation, no IronCalc-*fork* change.** The only engine
  surface is one pure `lex_formula_refs` free function (§1). The only external-crate change is the
  bounded vendored `InputState` highlight API (Phase 4) — treat its upstreaming like a fork fix (one
  focused change; the agent prepares the PR, does not open upstream). The IronCalc fork is untouched.
- **Phase ordering rationale:** outlines (Phase 2) deliberately precede the vendored change (Phase 4)
  so the highest-value, lowest-risk half ships and validates without waiting on the widget edit; the
  `reference_ready`/`pending_ref` payload is plumbed in Phase 2 but only consumed in Phase 3.
- **Render discipline (`CLAUDE.md`):** in-scope pixels are the in-cell overlay's outlines + token
  colors only; subset per grid phase, **full** suite + CI gate once in Phase 6. The data-row field's
  token coloring is chrome (out of pixel scope) — cover with gpui view tests + a `xvfb-run … cargo
  run -p freecell-app` smoke launch.
- **Ephemeral container:** commit + push after **every** phase (and mid-phase for Phases 2–4).
- **Efficiency:** crate-scoped checks per phase; reserve `--workspace` build/test for one final
  pre-Phase-6 validation; run cargo from `app/` (add `-j 2` for `render-tests` if the `ld` bus error
  recurs).
