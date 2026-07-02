---
status: draft
---

# Component: Style & Geometry Cache (`freecell-engine::cache`)

The always-resident, frontend-readable cache of everything the grid needs *except*
cell values: row/col geometry and resolved cell styling. Adopted in round-2
(`projects/style-cache.md`), sync design locked & validated in round-3 A
(`experiments/round-3/A-cache-sync/findings.md`). It exists so the render path makes
**zero engine calls** (the ~10× style-read cost never lands on a frame) and so the
grid renders fully styled during a multi-second eval — only values lag.

## Purpose and scope

**Does:** hold per-sheet geometry (all row heights / col widths + prefix-sum axes) and
styling (per-cell + row/col band, interned, pre-resolved to `RenderStyle`); answer
per-frame lookups; stay provably in agreement with IronCalc across every mutation MVP
can make.

**Does not:** hold values or display strings (Publication's job); implement
structural-edit shifting (validated design, deferred with the P2 insert/delete UI);
talk to GPUI.

## Data structures (locked by round-3 A, MVP subset)

Split for build parallelism: the **read model** (`SheetCaches`, `SheetCache`,
`StyleId`, `RenderStyle`, axes — everything below except `StyleInterner`) is
engine-free and lives in **`freecell-core`**; `freecell-engine::cache` owns the
`StyleInterner` (which touches ironcalc `Style`) and all build/mutation logic. The
grid and render-test fixtures therefore depend only on `freecell-core`.

```rust
pub struct SheetCaches { sheets: HashMap<SheetId, SheetCache>, }   // active + visited sheets

pub struct SheetCache {
    // geometry
    row_default_px: f32, col_default_px: f32,
    row_overrides: BTreeMap<u32, f32>,        // px, converted from engine units once, here
    col_overrides: BTreeMap<u32, f32>,
    row_axis: Arc<Axis>, col_axis: Arc<Axis>, // freecell-core prefix-sum axes (BLOCK=512)
    // styling
    interner: StyleInterner,                  // dedup on serialized ironcalc Style (Eq, not Hash)
    cell_styles: BTreeMap<(u32, u32), StyleId>,
    row_styles: BTreeMap<u32, StyleId>,       // band styles
    col_styles: BTreeMap<u32, StyleId>,
    resolved: Vec<RenderStyle>,               // StyleId → engine-free render form, same indices
}

impl SheetCache {
    pub fn render_style(&self, row: u32, col: u32) -> Option<&RenderStyle>;
        // resolution: cell > row-band > col-band > None(default) — engine order, SP4-verified
    pub fn axes(&self) -> (Arc<Axis>, Arc<Axis>);
}
```

- `RenderStyle` (in `freecell-core`): `{ bold, italic, underline: bool, fill:
  Option<Rgb>, font_color: Option<Rgb>, h_align: Option<Align>, num_format_is_default:
  bool }` — everything the MVP grid draws; extend per rendering feature later.
- `StyleInterner`: `HashMap<Vec<u8>, StyleId>` keyed on the bitcode/serde serialization
  of the engine `Style` (A's technique: `Style` is `Serialize + Eq` but not `Hash`) +
  the parallel `resolved` table. Interning happens worker-side only.
- **Unit conversion happens here, once**: IronCalc column-width units and row-height
  points → px, using the constants established in SP4/the POC; nothing else in the app
  ever sees engine units.
- `Axis` is immutable; geometry changes (file load; P2 resize) **rebuild** the affected
  axis (1M-entry build is ms-scale, measured in the POC) — no incremental axis
  mutation in MVP.

## Lifecycle & mutation protocol

All writes happen on the EvalWorker thread; the UI only ever `read()`s the `RwLock`.

1. **Build on sheet activation** (open builds the active sheet only): scan the
   worksheet's populated-cell iterator + row/col collections from `get_model()`;
   intern styles; fill overrides; build axes. Cost is bounded by populated cells
   (SP2-scale sheets: seconds, on the worker, behind the loading state — measured in
   the perf phase). Non-active sheets build on first activation, then stay resident.
2. **Mirror-the-issued-op** (forward path): FreeCell originates every mutation, so the
   worker updates the cache from the op it just applied — `SetStyleAttr{range, attr}`
   → for each cell: re-intern (fetch the cell's new engine style once, or compute the
   attr change onto the cached style — MVP: **re-read the touched cells' styles from
   the engine** right after applying; ranges are user-sized (≤ full viewport
   selections), so this is cheap and unconditionally correct).
3. **Undo/Redo**: the worker records each undoable command's touch-set (cells/rows
   affected) in a parallel history list; on Undo/Redo it re-reads styles for that
   touch-set and updates the cache. (A's inverse-op mirror with saved overrides is the
   optimization for when structural edits land; the agreement contract, not the
   mechanism, is what's load-bearing.)
4. **Sheet ops**: AddSheet → empty cache entry lazily; DeleteSheet → drop entry;
   Rename → key-stable (SheetId is positional index + rename-safe mapping — worker
   assigns stable `SheetId`s and maintains the index↔id map published in `SheetMeta`).

## The agreement contract (the tests that matter)

Round-3 A's discipline, ported: after **every** mutation kind, the cache must equal a
fresh engine re-read. Test helper `assert_cache_agrees(model, cache, probe_set)`
compares `render_style` + geometry against direct `get_style_for_cell` /
size getters over (a) all touched cells, (b) a fixed probe grid, (c) random cells
(seeded). Include a **negative control** (deliberately skip one cache update; assert
the helper FAILS) so the contract provably discriminates — repo convention.

## Dependencies

Read model: `freecell-core` (std only). Builder/mutator: `freecell-engine` — depends
on `freecell-core`, `ironcalc` types, `serde`/bitcode-style serialization for
interning. Depended on by: engine worker (writes), grid (reads, via core types),
render-tests fixtures.

## Test plan (Linux CI)

- `build_matches_engine_*`: styled fixture workbook (SP5-style long-tail file) →
  build → full agreement sweep; empty sheet; band-styles-only sheet; 1M-row geometry
  totals vs engine.
- `mirror_set_style_*`: each attr (bold/italic/underline/fill/no-fill), single cell +
  multi-range + overlapping band styles → agreement.
- `undo_redo_agreement`: scripted edit/undo/redo walks (incl. interleaved) →
  agreement after every step; negative control test.
- `interner_dedups`: N cells sharing a style → one StyleId; distinct → distinct.
- `unit_conversion_goldens`: known engine width/height values → expected px
  (constants documented in-code with source reference).
- `resolution_order`: cell-over-row-over-col precedence fixtures.
- Perf assertion (Linux, coarse): `render_style` lookup + axis offset for a 2k-cell
  viewport completes ≪ 1 ms (guards accidental O(n) lookups; the real gate is the
  macOS perf harness).
