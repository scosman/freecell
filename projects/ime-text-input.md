# Project: IME / International Text Input for the Cell Editor

**Status:** **Future (post-MVP).** Product call (2026-07-02): full IME/international
input is **not MVP-level**. Tracked so the decision and the known unknowns aren't
lost. Origin: 2026-07-02 pre-build spec review
(`specs/pre-build-spec-review-2026-07-02.md` §2, blind spot 4).

## What

A production-grade international text-input path for FreeCell's cell editor and
formula bar, on the from-scratch raw-gpui grid:

- **IME composition** (CJK: staged composition, candidate window positioning,
  commit/cancel semantics),
- dead keys (é, ü) and international keyboard layouts,
- decimal-comma locales for formula/number entry (ties into IronCalc's locale
  plumbing, which exists in its API).

## Why it's tracked (even though post-MVP)

The grid is a custom raw-gpui widget, so FreeCell inherits no text-input stack.
Zed solved IME for its own editor, but **whether that machinery is reachable
library API at our pinned rev — or entangled with Zed's editor internals — is
unknown** (zero corpus coverage). If it's the latter, the editor is a bigger build
than planned.

MVP posture: basic ASCII/Latin editing must work regardless (that's just the cell
editor); this project is the *full* IME/i18n input story.

## Cheap probe (when picked up — or opportunistically during editor build)

1–2 days: build a minimal GPUI text-overlay at the pinned rev; drive it with a
CJK IME (composition, candidates, cancel) and dead keys; record what GPUI exposes
(`InputHandler`-style surface?) and what FreeCell must own. Do this before the
cell-editor architecture hardens enough to make retrofitting expensive.
