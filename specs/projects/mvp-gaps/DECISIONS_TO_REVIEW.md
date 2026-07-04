# Decisions to Review — MVP Gaps

Append-only log of judgment calls and spec deviations made during implementation, for the
owner to review. Newest phase last.

## Phase 1 — Quick wins & publication

### 1. `[Color N]` classic palette not carried (format colours)

Architecture §1.2 asks for "a small const table for 0–6 + the 56-color indexed palette".
I implemented the **named colours 0–6** (black, white, red, green, blue, yellow, magenta),
which covers `[Red]` — the GAPS #2 requirement — and every named format colour. For
`[Color N]` with **N > 6**, `format_color::format_color_rgb` returns `None` (the cell keeps
the default text colour) rather than shipping a 56-entry classic palette. Rationale: the
pinned engine returns only the raw colour **index** and carries no RGB palette itself
(verified — `lexer.rs` maps names→index, nothing maps index→RGB), so there is no
engine-side reference to match; and `[Color N]` with a high index is vanishingly rare in
real files. Low-value, easy to add later if a file needs it.

### 2. Explicit pure-black font colour is indistinguishable from the default

`text_color` resolution reuses the resident style cache's black-filter (`parse_color` +
`filter(!= #000000)`): a *pure-black* explicit font colour is treated as "no override" (it
equals IronCalc's default). Consequence: a cell that is **both** explicitly black **and**
carries a `[Red]`-style format renders in the format colour (red), not black. This matches
the existing cache behaviour (which already collapses black→None) and is an extreme edge
case. Accepted.

### 3. Date heuristic strips all bracketed sections (incl. elapsed-time)

`is_date_format` follows architecture §1.2 literally — strip `[...]` sections and quoted
literals, then look for `y m d h s`. A format whose **only** time letters live inside an
elapsed-time bracket (e.g. a bare `[h]` with no other date letters) therefore classifies as
**Number**, not Date. Such formats are rare and the stripping rule is what the spec
specifies. Noted for awareness.

### 4. `PublishedCell.text_color` is fully resolved; grid line left unchanged

`PublishedCell.text_color` now carries the **fully resolved** colour (explicit non-black
font colour → number-format colour → `None`), per architecture §1.2's stated semantics. The
grid's existing `pc.text_color.or(style.font_color)` is left as-is: it is now redundant
(pc.text_color already incorporates the explicit font colour) but produces identical results
because both paths use the same black-filter. Kept for a minimal diff.

### 5. Backup-failure dialog wording

The `.back` copy-failure aborts the save and shows the standard Error modal with
title **"Couldn't create backup"** and detail **"File not saved. The backup copy could not
be written: `<io error>`"**. Functional spec §7.3 / ui_design §4 pin the phrase "Couldn't
create backup — file not saved."; it is split across the modal's title + detail (how the
existing Error modal renders title/body). Confirm this phrasing is acceptable.

### 6. Render baselines require a pinned-runner regeneration (could not do it here)

The type-aware **alignment** change alters rendered output for several **existing** baseline
cases and adds one new case. This phase changes **alignment only** in the render suite —
**no render case exercises the `[Red]` text-colour path** (see §6a), so none of these
baselines change colour:

- **Changed** (now right-aligned): `cell_number_plain`, `cell_number_thousands`,
  `cell_number_currency`, `cell_number_percent`, `cell_number_negative_red`,
  `cell_date_default`, `cell_narrow_column_clipped_number`, and the numeric cells in
  `grid_mixed_content`.
- **Changed** (now centered): `cell_boolean`, `cell_error_div0`, `cell_error_name`,
  `cell_error_circ`.
- **New**: `cell_number_align_left` (number + explicit Left alignment — guards
  "explicit alignment beats the numeric type-default").
- Unchanged: the explicit-alignment / text cases (`cell_align_*`, `cell_text_*`) already
  render at their explicit alignment.

Per `app/render-tests/README.md`, baselines must be regenerated on the **pinned CI runner
image** (Ubuntu 24.04 + mesa lavapipe + ImageMagick) and eyeballed before commit — dev
renders must never be committed. This container lacks the mesa-vulkan-drivers + ImageMagick
capture stack, so I could **not** regenerate/eyeball them here. **Action required before
merge:** regenerate the affected baselines on the pinned runner and eyeball the diff. Note:
`cargo test --workspace` (no `FREECELL_RENDER=1`) skips the pixel gate and is green; the
dedicated `render_tests.sh` gate will be red until the baselines are regenerated.

### 6a. `[Red]` text-colour has NO render-suite coverage (guarded by an engine test)

`cell_number_negative_red` is misnamed for its content: its input `-1,234.50` infers
`#,##0.00` (a single colourless section), so `resolve_text_color` short-circuits
(`!num_fmt.contains('[')`) and it publishes **no** `text_color` — it renders in the
**default colour**, right-aligned. The render Scene builder (`render-tests/src/scene.rs`)
drives cells only through `SetCellInput` (IronCalc *infers* the number format) plus the
bold/italic/underline/fill/align cache mutators; it has **no way to set a custom `num_fmt`**,
so no render-suite case can produce a number-format colour. Consequently the **GAPS #2
`[Red]` visual output is guarded solely by the engine integration test**
`published_style_resolves_format_and_explicit_colors` (`freecell-engine/src/document.rs`),
which asserts the resolved red / none / explicit-wins colours against a real `UserModel`.

**Deferred render case:** architecture §9 lists a `format_red_negative` / `text_color_red`
render case. Landing it needs a Scene-builder extension to set an explicit `num_fmt` on a
cell (small, mechanical). It is **deferred out of Phase 1** (the engine test covers the
behaviour; the pixel case is additive) — pick it up when the render harness gains a
`num_fmt` setter, or fold it into a later formatting phase (Phase 4 adds the `num_fmt` write
path). Recorded here so the gap is explicit, not implied.

### 6b. Per-cell publication cost (acknowledgement, no code change)

`build_publication` now calls `published_style` (style + type + value reads, and
`format_number` for bracketed numeric formats) in addition to `formatted_value` per
non-empty viewport cell, roughly doubling per-cell publication cost. This is per architecture
§1.2's design and runs **worker-side, off the scroll/render path** (the scroll path reads
the resident cache + published snapshot, never `published_style`), so it is not a scroll-gate
concern. The work is bounded by the published-cell count (the worker caps the viewport). No
change made — acknowledged as accepted per the design.

### 7. Environment: installed the documented Linux build deps

The base container was missing the GPUI link libraries documented in `app/README.md`
(`libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libwayland-dev`, `libvulkan-dev`,
`libfontconfig1-dev`, `libfreetype-dev`, `libasound2-dev`, `libxcb1-dev`); `cargo build
--workspace` failed to link `render-tests` bins until I `apt-get install`ed them. Not a code
change — recorded for reproducibility of the checks.
