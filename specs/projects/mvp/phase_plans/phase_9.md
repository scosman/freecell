---
status: complete
---

# Phase 9: Chrome (action row, data row / formula bar, sheet tab bar)

## Overview

Builds the chrome around the grid (`components/app_shell.md §Action row / §Data row /
§Sheet tab bar`, `ui_design.md §3.1–3.4`) as GPUI views assembled from stock
gpui-component controls, driven by the **Phase-2 `data_row` reducer** and the **Phase-2
palette**, and wired to the engine through a **test-double client seam** (a `ChromeClient`
trait). The real `DocumentClient` drops into that seam in Phase 11; Phase 9 exercises the
chrome fully headless against a `RecordingClient`.

Design stance (per the coding role + the "don't over-invest in chrome" spec note): the
heavy state-machine logic lives in **pure, unit-tested reducers** (`freecell-core`), and
the GPUI layer is thin plumbing that (a) translates widget events → reducer events →
effects → client commands and (b) renders from state. Every user action is also reachable
as a plain method on `ChromeView`, so behaviour is testable without simulating pixel
clicks; the gpui test-context tests then cover the widget/timer wiring the pure methods
can't.

## Client seam (for Phase 11)

`freecell-app/src/chrome/client.rs`:

```rust
pub trait ChromeClient {
    /// Send a command to the worker (fire-and-forget, matches DocumentClient::send).
    fn send(&self, cmd: Command);
    /// The resolved style of a single cell — for action-row toggle pressed states,
    /// read at selection-change time. `None` = unknown/empty (default style).
    fn render_style(&self, sheet: SheetId, cell: CellRef) -> Option<RenderStyle>;
}
impl ChromeClient for DocumentClient { … }   // send → self.send; render_style → caches().read()
pub struct RecordingClient { … }              // records Vec<Command> + a style map (test/demo double)
```

Chrome ↔ grid coupling (move active cell / focus grid / switch sheet) is a second seam,
`ChromeGridSink` (a boxed closure like `GridEventSink`), delivering `ChromeGridRequest`
`{ MoveActive(Motion), FocusGrid, SetActiveSheet(SheetId) }`. Phase 11 wires it to the
real `GridView`; Phase 9 uses a recording sink.

## Steps

1. **`freecell-core/src/eval_indicator.rs`** — new pure reducer for the action-row
   evaluating spinner (the 250 ms no-flash timer, §3.1 / §Data row "Evaluating spinner").
   `EvalIndicator { in_flight, spinner, epoch }`; events `Started | Finished | Timeout{epoch}`;
   effects `ArmTimer{epoch} | SetSpinner(bool)`. Re-arm only on not-in-flight→in-flight
   (coalesced back-to-back evals keep it shown); short eval never flashes. Register in
   `lib.rs`. Table-driven tests.
2. **`freecell-app/src/chrome/mod.rs`** — module root: re-exports; `SheetTab { id, name,
   has_content }` (the chrome's own tab view-model — the worker's `SheetMeta` lacks
   `has_content`; Phase 11 maps `SheetMeta`→`SheetTab`, sourcing content-ness — recorded in
   DECISIONS); `ChromeGridRequest` + `ChromeGridSink`.
3. **`freecell-app/src/chrome/client.rs`** — `ChromeClient` trait, `impl for
   DocumentClient`, and the `RecordingClient` double.
4. **`freecell-app/src/chrome/view.rs`** — `ChromeView` entity holding: `client`,
   `grid_sink`, `active_sheet`, `selection`, cached `active_style`, `data_row: DataRow`,
   `content_input: Entity<InputState>`, `eval: EvalIndicator`, `sheets: Vec<SheetTab>`,
   rename state (`rename_target`, `rename_input`, `rename_error`), `context_menu`,
   `confirm_delete`, `cap_error_external`, focus handle. Action methods (used by widget
   handlers **and** tests) + `on_worker_event` folding + `Render` (three grey rows +
   hairlines per §3, action row = B/I/U toggles + Fill popover + right eval spinner; data
   row = ref box + content field + fetch spinner + danger border; tab bar = tabs + `+` +
   inline rename input + custom right-click menu + delete-confirm modal).
5. **`freecell-app/src/lib.rs`** — add `pub mod chrome;`.
6. **`freecell-app/Cargo.toml`** — no new deps (gpui-component + freecell-engine already
   present).
7. Optional: mount `ChromeView` in a small capture harness for a spot-check screenshot.

## Tests

Pure (`freecell-core`, headless):
- `eval_indicator`: `short_eval_never_shows`, `long_eval_shows_then_hides`,
  `coalesced_back_to_back_stays_shown`, `stale_timeout_noops`.
- (Reused, already green: `data_row` state machine, `sheet_name` validate, `to_a1`,
  `palette` constants.)

GPUI test-context (`TestAppContext` + `VisualTestContext`, Linux CI):
- `selection_single_fetches_content`, `content_reply_populates_field`,
  `stale_content_reply_dropped`, `multiselect_disables_field`.
- `edit_change_enters_editing`, `enter_commits_and_moves_down`, `escape_reverts_field`,
  `cap_reject_keeps_editing_and_flags_error`, `commit_on_edit_commit_requested`.
- `toggle_bold_sends_setstyleattr`, `toggles_reflect_active_style`,
  `fill_swatch_sends_fill`, `no_fill_sends_none`, `custom_color_sends_fill`,
  `formatting_commits_pending_edit_first`.
- `add_sheet_sends_command`, `select_sheet_switches`, `rename_valid_sends_command`,
  `rename_invalid_stays_editing`, `rename_escape_reverts`, `delete_last_disabled`,
  `delete_empty_no_confirm`, `delete_with_content_confirms_then_deletes`.
- `eval_spinner_only_after_250ms` (short vs long, `advance_clock`),
  `formula_field_spinner_only_after_250ms`.

Manual smoke (documented, Phase 11 wires the real client): open the fill popover and pick a
swatch; right-click a tab for the menu; the eval spinner appearing on a slow recalc.

Manual smoke — GPUI-wiring assumptions the interaction tests verify by direct handler
invocation (not through the real `InputState` event dispatch), so confirm them once in a live
window (Phase 11 / desktop run):
- **`set_value` suppresses the `Change` event** (no feedback loop): programmatic content-field
  updates on selection change / fetch reply / Escape must NOT re-enter `Editing` or reset the
  caret while typing. (Verified in gpui-component's source — `set_value` sets `emit_events =
  false` — but relied upon by the controlled-input plumbing.)
- **`InputState` propagates Escape** up to the data-row container's `on_key_down`: pressing
  Escape while editing the formula bar reverts to the last-fetched content and returns focus to
  the grid (the field itself does not consume Escape when `clean_on_escape` is off).
