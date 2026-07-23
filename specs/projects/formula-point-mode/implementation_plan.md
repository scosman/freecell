---
status: complete
---

# Implementation Plan: Formula Point-Mode + Range Highlighting

Five phases. Build order front-loads the **pure, no-pixel** foundation (tokenization seam +
palette/predicate), then the **shared color map + grid highlights** (rich fill + border), then the
**point-mode routing**, then **consolidation cleanup** + the v1.0 GAP entries, and finally the
single **render-validation** phase. Each coding phase: build **crate-scoped**, `cargo fmt --all
--check` (whole workspace — a `render-tests`/sibling-crate edit must not slip a fmt violation past a
crate-scoped check), unit/gpui tests + (grid phases) a render **subset**, commit + push. The
**full** render suite + CI `render` gate run **once**, in Phase 5. The feature makes **no
gpui-component / vendored-widget change** — in-editor token coloring is deferred to the v1.0 styled
text-input control (`../../../projects/styled-text-input-control.md`).

Section numbers reference [`architecture.md`](architecture.md) (§1–§9) and
[`components/formula_editor.md`](components/formula_editor.md). Behaviour + DPM.1–8 are locked in
`functional_spec.md`; the six architecture questions are resolved in `architecture.md`. Run cargo
from `app/`.

## Phases

- [x] **Phase 1 — Tokenization seam + pure foundation (§1, §2).** No pixels, no gpui.
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

- [x] **Phase 2 — Shared color map + grid highlights (§2.4, §3.1, §4.1, §6).** Ships user-visible
      value with **no** gpui-component / vendored-widget dependency; no in-editor token coloring
      (deferred to v1.0).
      - `freecell-app` chrome: promote the formula-feature state onto `EditController`
        (`components/formula_editor.md §1.2-1.4` — relocate `autocomplete`/`sig_hint`, add
        `ref_tokens`/`ref_colors`, `recompute_formula`); generalize `recompute_autocomplete` →
        `recompute_formula_edit_state` wired into both Change handlers + the caret-move paths.
      - Extend `ChromeGridRequest::EditState` + `GridView::set_edit_state` with `reference_ready`,
        `pending_ref`, `ref_highlights` (§3.1); `refresh_edit_grid_state` fills them (`ref_highlights`
        = same-sheet subset). (`reference_ready`/`pending_ref` are pushed now but only *consumed* by
        the grid in Phase 3 — plumb them here so the payload lands once.)
      - Grid: store the three fields; paint the reference highlights in the overlay pass (§4.1, rich
        **fill + border**, themed slot color, clipped like the selection overlay); clear on
        `set_active_sheet`.
      - Checks: `cargo build -p freecell-app` (+ `-p freecell-engine`); gpui view tests (highlights
        appear for same-sheet refs, absent for cross-sheet, cleared on commit); **render subset**
        `render_tests.sh test formula_ref` while iterating. `cargo fmt --all --check`. Commit + push.

- [x] **Phase 3 — Point-mode routing (§3, §5).** The `InsertReference` path + pending-ref.
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
        happy path). **Render subset** `… test formula_ref` (point-preview vs selection vs highlight).
        `cargo fmt --all --check`. Commit + push.

- [ ] **Phase 4 — Consolidation cleanup + v1.0 GAP entries (§6).**
      - Finish the consolidation: confirm the two host adapters carry **zero** formula logic (grid =
        primitives + paint; data-row bar = render only); remove any now-dead scattered fields; the
        migration-step-1 regression gate (all shipped autocomplete + `data_row` tests green).
      - **Confirm the `GAPS.md` v1.0 entries** the deferrals point at, and add any that is missing:
        (a) the **already-logged** FreeCell styled text-input control
        (`projects/styled-text-input-control.md`) — owns in-editor token coloring + rich in-editor
        formatting, and will consume this project's token→color map; and (b) **cross-sheet
        point-mode insertion** (click another sheet's tab mid-formula → insert `Sheet2!A1`), per
        `functional_spec.md §6`. For (b) note the asymmetry: the color map still colors already-typed
        cross-sheet refs for the future control, but v0.5 draws **no cross-sheet grid highlight** and
        inserts no cross-sheet ref.
      - Checks: `cargo build -p freecell-app`; full crate-scoped test run for the touched crates;
        `cargo fmt --all --check`. Commit + push.

- [ ] **Phase 5 — Render validation (dedicated late phase, §9).** No new behaviour.
      - Regenerate + **eyeball** baselines: add `formula_ref_highlight_same_sheet`,
        `formula_ref_point_preview`; refresh any shifted `selection`/`cell_*`.
      - Run the **full** pixel suite once under `timeout` + ~10-min watchdog
        (`app/render-tests/scripts/render_tests.sh test`); fix/accept each diff; commit refreshed
        baselines with sign-off.
      - Dispatch the CI **`render`** gate on the branch (`gh workflow run render.yml --ref <branch>`),
        poll to green. `cargo fmt --all --check`. Commit + push.

## Notes for the build

- **No worker command, no engine-state mutation, no IronCalc-*fork* change, and no gpui-component /
  vendored-widget change.** The only engine surface is one pure `lex_formula_refs` free function
  (§1); the whole feature is FreeCell-only. In-editor token coloring — which would have needed the
  widget change — is deferred to the v1.0 styled text-input control
  (`../../../projects/styled-text-input-control.md`). The IronCalc fork is untouched.
- **Phase ordering rationale:** the grid highlights (Phase 2) deliberately precede point-mode
  routing (Phase 3) so the highest-value, lowest-risk half ships and validates first; the
  `reference_ready`/`pending_ref` payload is plumbed in Phase 2 but only consumed in Phase 3.
- **Render discipline (`CLAUDE.md`):** in-scope pixels are the grid reference highlights (fill +
  border) + the point-drag preview; subset per grid phase, **full** suite + CI gate once in Phase 5.
  There is **no in-editor token coloring** in v0.5, so no chrome-side coloring to validate (a
  `xvfb-run … cargo run -p freecell-app` smoke launch still sanity-checks the chrome).
- **Ephemeral container:** commit + push after **every** phase (and mid-phase for Phases 2–3).
- **Efficiency:** crate-scoped checks per phase; reserve `--workspace` build/test for one final
  pre-Phase-5 validation; run cargo from `app/` (add `-j 2` for `render-tests` if the `ld` bus error
  recurs).
