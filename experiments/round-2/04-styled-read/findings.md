# SP4 — Styled viewport read at scale + style-API coverage: findings

_IronCalc 0.7.1 (pinned, same as the frozen `round-2/harness`). FreeCell Phase 2
(Round-2), build-out cohort. All numbers are foreground, in-container (4c / ~15 GB, no
GPU); real hardware is faster._

Reproduce everything from this folder (all foreground):

```
cargo test                            # 10 unit tests (read core + style-resolution semantics)
cargo run --release --bin probe       # results/style_api_coverage.{json,md}
timeout 400 cargo run --release --bin bench   # results/styled_read.json + summary.md + env.txt
```

The `bench` binary builds the styled fixture **once** (not timed), then times only the
read; it **force+asserts** the reads are real (non-empty values AND resolved styles)
before recording, and exits non-zero if a window misses the gate (the FAIL is still
recorded — an unmet target is a finding, not a hidden skip).

---

## 1. Question(s)

1. **Styled read GATE.** Phase-1 measured the newly-visible viewport read at **392 µs
   p99 (value-only, 1,800 cells, IronCalc D2)**. Does the per-cell viewport read still
   hold **p99 < 2 ms when it also reads the style** (`get_style_for_cell`) per visible
   cell, over a viewport+overscan window (~10³–10⁴ cells), **at Excel-max positions**?
2. **Style-API coverage.** Does IronCalc's public style API expose what FreeCell needs —
   **per-cell** attributes, **row/column band** styles, and **empty-cell** styling?
   (Excel styles whole empty rows/cols; verify via the public API, don't assume.)

---

## 2. What was done (approach + code pointers)

### 2.1 The value+style read path (`src/lib.rs`)

The frozen harness `IronCalcEngine::read_viewport` reads **values only** (`get_value`
per cell) and its trait exposes no style read, so SP4 adds its own read path here without
touching the frozen crate:

- **`read_styled_viewport(&IronCalcEngine, Viewport) -> Vec<StyledCell>`** — per visible
  cell, one value read (`get_value`) **and** one style resolution
  (`engine.model().get_style_for_cell(sheet, row+1, col+1)`), projected into a compact
  `StyledCell { value, bold, italic, font_size, fill_argb, num_fmt }`. This is exactly
  what the FreeCell grid binding would do; IronCalc has **no native bulk style read**
  (mirrors the value side — no native range read either), so it is a per-cell loop.
- **Build vs read split.** Style *setters* (`set_cell_style` / `set_row_style` /
  `set_column_style`) live only on `&mut Model`; the frozen adapter exposes only
  `&Model`. Rather than edit the frozen crate or use `unsafe`, SP4 **owns the raw
  `Model`** while building the styled fixture (full mutable access), then wraps the
  finished model with `IronCalcEngine::from_model` for the read half (`&self` only) —
  the harness is untouched, and the wrapped engine is read-only exactly like the UI path.

### 2.2 The styled benchmark (`src/bin/bench.rs`)

- **Fixture at Excel-max.** IronCalc storage is a sparse `HashMap`, so a styled band is
  built anchored at the **bottom-right of the grid** (far corner = row 1,048,575 /
  col 16,383, 0-based = IronCalc `LAST_ROW` / `LAST_COLUMN`). The band carries a **mix**
  of a column band on every column, a row band on every row, and scattered per-cell
  styles + values — so the read exercises IronCalc's real cell → row → column → default
  resolution fallthrough at the maximal coordinate, not a trivial origin read.
- **Scroll/jump path.** Reuses the harness `pan_path` (the *same* deterministic scroll
  the Phase-1 scrolling read used) over a region == the band, offset to the Excel-max
  origin. 300 pan steps per window; p50/p99/max via `bench_util::LatencyStats`.
- **Two windows** across the spec's 10³–10⁴ band: **`viewport`** (60×30 = **1,800
  cells**, the Phase-1-comparable shape) and **`overscan`** (100×100 = **10,000 cells**,
  the top of the band).
- **Value-only control** in the same positions (harness `read_viewport`) so the added
  style cost is a clean delta against the Phase-1 392 µs baseline.
- **Crossover sweep** (8 window sizes, 60 steps each) to pinpoint the largest window that
  still fits the 2 ms budget — so the verdict is precise, not just "1,800 passes / 10,000
  fails".

### 2.3 The style-API coverage probe (`src/bin/probe.rs`)

Each capability is a **runtime assertion against IronCalc's real public API** (a missing
capability would either fail to compile — no such method — or fail its read-back
assertion). Verified against the pinned 0.7.1 source before coding
(`model.rs:1958-1994`, `2307-2361`; `worksheet.rs:97-144`; `types.rs:323`):

- **(a) per-cell** — `set_cell_style` a non-default `Style`; `get_style_for_cell` reads
  bold + num_fmt + fill back.
- **(b) row band / column band** — `set_row_style` / `set_column_style`; assert an
  **untouched** cell far along the row/column resolves the band; `get_row_style` /
  `get_column_style` return it; an adjacent row/column is unaffected.
- **(c) empty-cell** — a **valueless** cell under a band (and one given a direct
  per-cell style with no value) still resolves the style via `get_style_for_cell` while
  its value reads empty.
- **(+) precedence** — a column band, a row band over it, and a per-cell style: each
  cell resolves to the expected winner (**cell > row > column > default**), matching
  IronCalc's documented `get_cell_style_index` order — so one `get_style_for_cell` call
  yields the value the UI paints.

---

## 3. Results / evidence

**Environment (stamped in `results/env.txt` + `results/styled_read.json`):**
linux / x86_64 / 4 cores / Intel(R) Xeon(R) @ 2.80 GHz; IronCalc 0.7.1; Excel-max grid
1,048,576 × 16,384.

### 3.1 Styled read GATE (value + style, at Excel-max) — **PARTIAL PASS**

| window | cells | value+style p50 | value+style p99 | value-only p99 | added-style p99 | GATE p99<2ms |
|--------|-------|-----------------|-----------------|----------------|-----------------|--------------|
| `viewport` | 1,800 | ~0.68–0.77 ms | **~0.85–1.20 ms** | ~0.14 ms | ~0.71–1.06 ms | **PASS** |
| `overscan` | 10,000 | ~3.8 ms | **~4.4–7.2 ms** | ~0.55 ms | ~3.8–6.4 ms | **FAIL** |

(p99 ranges are across repeated foreground runs on the shared 4-core box; the last
recorded run is in `results/styled_read.json`. Every run gave the same verdict per
window.)

**Crossover sweep (largest window still under the 2 ms p99 budget, at Excel-max):**

| cells | value+style p99 | GATE |
|-------|-----------------|------|
| 1,200 | ~0.54 ms | PASS |
| 2,000 | ~0.85 ms | PASS |
| 2,475 | ~1.05 ms | PASS |
| 2,750 | ~1.15 ms | PASS |
| 3,000 | ~1.25 ms | PASS |
| 3,500 | ~1.5–1.9 ms | PASS |
| 4,800 | ~2.2–2.5 ms | FAIL |
| 7,000 | ~3.0–4.9 ms | FAIL |

**Crossover ≈ 3,500 cells PASS, 4,800 cells FAIL** (reproducible across runs).

**The number that explains everything: style resolution costs ~700 ns/cell.**
- value-only read: ~78 ns/cell (135 µs / 1,800 ≈ 75 ns; 558 µs / 10,000 ≈ 56 ns).
- value+style read: ~700 ns/cell (1,196 µs / 1,800 ≈ 664 ns; 6,967 µs / 10,000 ≈ 697 ns).
- So **`get_style_for_cell` is ~9–10× the cost of a value read**, and the styled read
  scales **linearly** with the cell count (p99 ≈ 700 ns × cells). The 2 ms budget is hit
  at ≈ 2 ms / 700 ns ≈ **~2,850 cells** (matching the observed ~3,500-pass / ~4,800-fail
  crossover, allowing for the value half and run-to-run jitter).

**Why `get_style_for_cell` is ~10× a value read (root cause, from the 0.7.1 source):**
1. `get_cell_style_index` (`model.rs:1959`) does a `HashMap` lookup for the cell; **on a
   band/empty cell it then linearly scans the worksheet's `rows` vector and `cols`
   vector** to find a covering band (`model.rs:1966-1982`). With row+column bands present
   near the viewport this is O(bands) per cell.
2. `styles.get_style(index)` (`styles.rs:192`) **reconstructs and clones a whole
   `Style`** — `fill.clone()`, `font.clone()` (with `name: String`, `color:
   Option<String>`), `border.clone()` (up to 5 `BorderItem`s), the `num_fmt` string, and
   `alignment.clone()` — roughly half a dozen heap allocations **per cell**, discarded
   immediately by our projection.

Neither is a bug; it is simply how IronCalc's read-a-full-style API is shaped. The cost
is **honest and unavoidable through the public `get_style_for_cell` path** as-is.

### 3.2 Style-API coverage — **ALL SUPPORTED** (`results/style_api_coverage.{json,md}`)

| capability | supported | public API | proof |
|------------|-----------|------------|-------|
| per-cell styles | **YES** | `set_cell_style` / `get_style_for_cell` | bold + num_fmt + fill read back |
| **row band** | **YES** | `set_row_style` / `get_row_style` | untouched cell far along the row resolves the band; adjacent row does not |
| **column band** | **YES** | `set_column_style` / `get_column_style` | untouched cell far down the column resolves the band; adjacent column does not |
| **empty-cell styling** | **YES** | `get_style_for_cell` over a valueless cell (under a band OR with a direct per-cell style) | style resolves while `get_cell_value` is empty |
| precedence cell>row>column>default | **YES** | `get_cell_style_index` order | each cell resolves to the expected winner |

All five are backed by executed assertions (also encoded as the 8 `cargo test` cases:
`per_cell_style_roundtrips`, `row_band_applies_to_untouched_cell`,
`column_band_applies_to_untouched_cell`, `empty_cell_styling_resolves`,
`style_precedence_cell_over_row_over_column`, plus `excel_max_read_is_addressable`).

**One IronCalc quirk to know (not a blocker):** `set_row_style` only marks a row
resolvable through `get_style_for_cell` when the style index is **non-default**
(`worksheet.rs:125-131`, flagged `// FIXME: This is a HACK`). Setting a *default* style as
a row band is a silent no-op through the resolution path. FreeCell's bands are always
non-default, so this is a footnote, not a limitation — but the binding layer should be
aware it can't clear a row band by writing the default style (use `delete_row_style`).

---

## 4. Conclusion — a direct answer

**Q1 (styled read GATE): PARTIAL PASS — passes at viewport scale, fails at 10⁴.** At the
Phase-1-comparable **1,800-cell viewport, value+style reads at p99 ≈ 0.85–1.2 ms —
comfortably inside the 2 ms budget, at Excel-max.** But because `get_style_for_cell`
costs ~700 ns/cell (~10× a value read) and the read is linear, the budget is crossed at
**≈ 3,500–4,800 cells**, and the **10,000-cell overscan fails at p99 ≈ 4.4–7.2 ms.**

Whether SP4 "passes its gate" depends on how large the *live, per-frame* read window
really is:
- A real FreeCell viewport is ~1,800 visible cells (Phase-1). A modest overscan margin
  (say 1.5×, ~2,700 cells) **still passes.** So **for a value+style read of the visible
  viewport plus a small overscan, IronCalc holds the 2 ms budget at Excel-max.**
- A **10³–10⁴ single styled read every frame does NOT hold** — a full 10⁴ styled pull is
  ~5–7 ms, well over a 60 fps frame. The spec's own bound is "~10³–10⁴"; the honest
  result is **pass at the 10³ end, fail at the 10⁴ end.**

This is **not a dealbreaker** and needs no engine change — it is a **binding-layer
constraint the real build must respect**: don't pull a full 10⁴-cell styled window
synchronously on the render path. See §5.

**Q2 (style-API coverage): FULLY SUPPORTED — the overview §2 decision STANDS.** IronCalc's
public API natively exposes **per-cell, row-band, column-band, and empty-cell styling**
with a deterministic **cell > row > column > default** resolution surfaced by a single
`get_style_for_cell` call. **SP4 does NOT reopen the overview §2 formatting decision** and
does **not** force a style side-store for these attributes. (The pre-existing overview §2
gaps — no public **merged-cells** API, no **conditional-formatting** API — are unchanged
and still need a scoped side-store for *those two features only*; that is carried, not an
SP4 finding.)

*We could not determine* an exhaustive worst-case for the band linear-scan cost (a sheet
with thousands of distinct row/col bands near the viewport would raise per-cell cost
above the ~700 ns measured here); the fixture uses band counts on the order of the window
(hundreds), which is realistic. Flagged as an open question (§6).

---

## 5. Recommended design + next-best alternative

**Recommended (binding-layer):** treat the **styled read like any other engine read that
can exceed a frame — keep it off the synchronous render path for large windows.**
Concretely, matching SP1's engine↔render seam:
- Read **value + style for the visible viewport + a small overscan (target ≤ ~2,500–3,000
  cells)** synchronously — that is < 2 ms at Excel-max (measured), so a scroll paints
  immediately.
- For anything larger (big overscan, pre-render buffers), pull value+style in a **cached
  window** refreshed off the render loop (the harness `BindingCache`/D3 pattern already
  models this), or in a background pull, so no single frame pays a 10⁴-cell styled read.
- **Cache resolved styles by style index, not by cell.** `get_cell_style_index` is cheap
  relative to `get_style`'s full-`Style` clone; the binding can call the (crate-public)
  index path once per distinct style and reuse the projected `StyledCell` style fields
  across all cells sharing it — collapsing the ~700 ns/cell to roughly the value-read cost
  for the common case where a viewport uses few distinct styles. (Not implemented here —
  it needs `get_cell_style_index`, which is public, plus a small style-index→projection
  cache in the binding; it is a real, low-risk win the build should take.)

**Next-best alternative:** contribute a **bulk / projection style read** upstream to
IronCalc — a `get_style_projection_for_range` that resolves indices once and returns only
the fields a renderer needs (no per-cell full-`Style` clone). This would make even a 10⁴
synchronous styled read frame-safe. Lower priority than the binding-layer cache, since the
cache gets most of the win without an engine change.

---

## 6. Risks / open questions carried forward

- **10⁴ styled read is not frame-safe (the SP4 headline).** A full ~10,000-cell
  value+style pull is ~5–7 ms at Excel-max. The build MUST cap the synchronous styled
  window (~≤ 3k cells) or move the large read off the render path (§5). Not an engine
  pivot; a binding constraint — but it must be designed in, not discovered later.
- **Band linear-scan cost is workload-dependent.** `get_cell_style_index` linearly scans
  the `rows`/`cols` band vectors for band/empty cells. A pathological sheet with thousands
  of distinct bands near the viewport would push per-cell cost above ~700 ns. Worth a
  targeted follow-up if band-heavy sheets are common; the measured ~700 ns assumes
  band counts on the order of the window (hundreds).
- **`set_row_style(default)` is a silent no-op** (IronCalc `custom_format` HACK). The
  binding must clear a row band via `delete_row_style`, not by writing a default style.
- **Style-index caching is unimplemented.** The recommended ~10× win (§5) is designed but
  not built here (SP4 measures; it doesn't build the real binding). Carry to the build.
- **Carried, not SP4:** no public merged-cells / conditional-formatting API (overview §2)
  — a scoped side-store is still needed for those two features only.
