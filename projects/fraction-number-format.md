# Fraction Number Format (`# ?/?`)

**Status: Future** (deferred from `gaps_closing_7_12` Phase 6, 2026-07-13).

## Goal

Offer a **Fraction** category in the number-format dropdown so numbers display as fractions,
matching Excel/Sheets:

- `# ?/?` — up to one-digit denominator (`1.5` → `1 1/2`, `2.25` → `2 1/4`).
- `# ??/??` — up to two-digit denominator.
- (Optionally the fixed-denominator variants: halves `?/2`, quarters `?/4`, eighths `?/8`, ….)

This was in the Phase 6 (`functional_spec.md §6`, D6.1) proposed inventory and is the one preset
from that list that could not ship.

## Why it's deferred (needs an IronCalc fork change)

Phase 6 was explicitly **FreeCell-side / no-fork** (grouped preset breadth is UI-only — the engine
already renders arbitrary format codes). Fraction formatting is the exception: IronCalc's `?/?`
handling is **effectively unimplemented**. `format_number` doesn't error on it — it renders garbage
for every input:

| value | `# ?/?` renders |
|-------|-----------------|
| `1.5` | `"  /2"` |
| `0.5` | `"  /1"` |
| `2.25`| `"  /2"` |

Because it produces a plausible-looking (non-`#VALUE!`) string, the breakage was invisible until a
reviewer rendered the presets through the real engine. Shipping it would have put visibly-wrong
output behind a one-click preset, so the preset **and** the `Category::Fraction` enum variant were
dropped from `NUM_FMT_GROUPS` / `format_ui.rs` for the batch.

## What implementing it requires

1. **Fork work** in `scosman/ironcalc` (per `CLAUDE.md`: fix upstream, one fix = one `fix/<slug>`
   branch = one clean upstream PR): implement `?`/`??` fraction denominators in
   `base/src/formatter/format.rs` (the numerator/denominator search — nearest fraction with a
   denominator that fits the placeholder width, plus the integer + space + `num/den` layout and the
   fixed-denominator forms). Add upstream-style formatter tests.
2. **FreeCell-side, once the fork renders it correctly:** re-add the Fraction group to
   `NUM_FMT_GROUPS` and `Category::Fraction` (+ `label()`), and add the D6.1 line back to
   `functional_spec.md §6`. The existing engine-render guard
   (`freecell-engine::document::tests::every_num_fmt_preset_code_renders_without_parse_error`) will
   then automatically cover it — extend it with a sane-string assertion (`1.5` → `1 1/2`).

## Notes

- **Scientific** (`0.00E+00`), the sibling "More" preset, renders correctly and **did** ship in
  Phase 6 — only Fraction was engine-blocked.
- No FreeCell-side workaround is appropriate (per `CLAUDE.md`: fix the engine, don't hack FreeCell).
