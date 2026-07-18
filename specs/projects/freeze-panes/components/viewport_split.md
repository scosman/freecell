---
status: draft
---

# Component: Viewport Split (four-quadrant render + frozen-aware geometry)

The field-level design for the grid render change (`architecture.md §3`). The custom grid today
has **one** content rect + **one** scroll pair per sheet (`scroll: HashMap<SheetId,(f64,f64)>`,
`grid/view.rs:273`); `resolve_frame` (`view.rs:997`) computes one visible row/col range and
`build_grid_layers` (`view.rs:2863`) paints one content layer. This component splits that single
body into up to **four quadrants** and makes every scroll/clamp/hit-test/reveal computation
frozen-extent-aware, while keeping `M = K = 0` byte-for-byte identical to today and never adding
per-frame work proportional to sheet size.

`(M, K)` are read per frame off `SheetCache` (`cache.frozen_rows()` / `cache.frozen_cols()`,
`architecture.md §2.4`); no `GridView` field is added.

---

## 1. Geometry primitives — band extents

Two pixel extents drive everything, both cheap prefix-sum reads on the **committed** axes (no
rebuild, hidden-aware — a hidden track contributes 0):

```
frozen_w = col_axis.offset_of(K)   // px width  of the frozen-columns band (cols 0..K)
frozen_h = row_axis.offset_of(M)   // px height of the frozen-rows    band (rows 0..M)
```

With `content_w` / `content_h` the current body area minus headers (as today,
`view.rs:1035/1048`), the four quadrant **destination rects** (grid-local px; origin at the grid
top-left) are:

| Quadrant  | rows    | cols    | dest x0                   | dest y0                    | dest w                    | dest h                    |
|-----------|---------|---------|---------------------------|----------------------------|---------------------------|---------------------------|
| Corner    | `0..M`  | `0..K`  | `row_header_w`            | `COL_HEADER_H`             | `frozen_w`                | `frozen_h`                |
| Top band  | `0..M`  | body cols | `row_header_w + frozen_w`| `COL_HEADER_H`             | `body_w`                  | `frozen_h`                |
| Left band | body rows | `0..K` | `row_header_w`            | `COL_HEADER_H + frozen_h`  | `frozen_w`                | `body_h`                  |
| Body      | body rows | body cols | `row_header_w + frozen_w`| `COL_HEADER_H + frozen_h` | `body_w`                  | `body_h`                  |

where the **body area** is:

```
body_w = (content_w - frozen_w).max(0.0)
body_h = (content_h - frozen_h).max(0.0)
```

A quadrant with an empty row/col range or a zero-size dest is simply not built (`M=0` drops the
corner + top band; `K=0` drops the corner + left band; `M=K=0` leaves only the body = today).

**Each quadrant maps a cell `(r, c)` to its own div-relative pixel** via an effective content
origin `(qorigin_x, qorigin_y)` — the content-space pixel that lands at the dest rect's top-left:

| Quadrant  | qorigin_x            | qorigin_y            |
|-----------|----------------------|----------------------|
| Corner    | `0`                  | `0`                  |
| Top band  | `frozen_w + body_sx` | `0`                  |
| Left band | `0`                  | `frozen_h + body_sy` |
| Body      | `frozen_w + body_sx` | `frozen_h + body_sy` |

`(body_sx, body_sy)` is the **re-based body scroll** (§3). Since each quadrant is painted into its
own `.absolute().left(dest_x0).top(dest_y0).…overflow_hidden()` div, its children use
**quadrant-relative** coords: a cell paints at

```
qx = col_offset(c) - qorigin_x
qy = row_offset(r) - qorigin_y
```

For the **body** quadrant this reduces to `qx = col_offset(c) - frozen_w - body_sx`. Note that
today's body math (`view.rs:4360`) is `col_offset(c) - scroll_x` with the div at
`left(row_header_w)`; the frozen body div sits at `left(row_header_w + frozen_w)` and subtracts
`frozen_w + body_sx` — the two agree exactly when `frozen_w = 0`, so **`M=K=0` is unchanged**.

---

## 2. The `Quadrant` sub-frame + `build_grid_layers` rework

Introduce a small per-quadrant descriptor resolved in `resolve_frame`:

```rust
struct Quadrant {
    rows: Range<u32>,        // visible row indices in this quadrant (clamped)
    cols: Range<u32>,        // visible col indices
    dest: (f32, f32, f32, f32),   // grid-local dest rect (x0, y0, w, h) for the clip div
    qorigin_x: f64,          // content-x that maps to dest.x0
    qorigin_y: f64,          // content-y that maps to dest.y0
}
```

`resolve_frame` returns the existing `Frame` (axes/previews/totals unchanged) **plus** a
`quadrants: [Option<Quadrant>; 4]` (Corner, TopBand, LeftBand, Body). The four visible ranges:

- **Corner/Top-band rows** = `0..M` clamped so their pixel extent doesn't exceed `frozen_h`
  (already true by construction) — i.e. `0..M`.
- **Left-band/Body rows** = `row_axis.visible_range(frozen_h + body_sy, body_h, RENDER_OVERSCAN)`,
  with the range **start floored at `M`** (overscan must never pull a frozen row into the body).
- **Corner/Left-band cols** = `0..K`.
- **Top-band/Body cols** = `col_axis.visible_range(frozen_w + body_sx, body_w, RENDER_OVERSCAN)`,
  start floored at `K`.

`build_grid_layers` changes from "one content div" to "**one content div per present quadrant**".
The current per-cell / border / spill / selection / fill-handle loop (`view.rs:2906-3204`) is
factored into a helper `build_quadrant(frame, quad, ...) -> Vec<AnyElement>` that runs the same
loop over `quad.rows × quad.cols` using `cell_rect(r, c, quad)` (the quadrant-relative form, §1)
instead of the frame-global one. Then each quadrant's children go into their own clipped div at
`quad.dest`. Consequences:

- **`cell_rect` / `span_rect`** (`view.rs:4359/4368`) take a `&Quadrant` (or the pair
  `qorigin_x/qorigin_y`) instead of reading `frame.scroll_x/scroll_y`. Their body is otherwise
  unchanged (`col_offset(c) - qorigin_x`, …).
- **Selection spanning quadrants** (`functional_spec.md §3.3`): the selection overlay / range
  border / active border loop runs **inside each quadrant**, each clipped to its own div, so a
  range crossing the divider paints its portion in each quadrant and they visually join at the
  divider — no gap, no double-draw (each quadrant clips to disjoint rects). `range_overlay_rects`
  (`layout.rs:302`) is unchanged; it is just intersected with each quadrant's visible range
  (exactly as the current clip at `view.rs:3128` intersects with `frame.rows/cols`).
- **The published-cell index** (`self.cell_index`, `view.rs:2881`) is built once over the **union**
  of the four quadrants' ranges (rows `0..M ∪ body_rows`, cols `0..K ∪ body_cols`); the published
  cells are ≤512×256, so the filter stays cheap. Each quadrant looks up its own sub-range. Total
  work = O(published + Σ visible-per-quadrant), still not O(sheet).
- **In-cell editor overlay** (`view.rs:3209`) is placed in the quadrant that contains the editing
  cell (body if it's a body cell; the matching band/corner if the edited cell is frozen), so a
  frozen cell's editor pins with its band.
- **ChartLayer** (`view.rs:3236`) is clipped to the **body** quadrant's dest rect (charts are
  body-anchored and scroll with the body — `functional_spec.md §4`; a chart anchored under a
  frozen band is the known minor limitation). Its geometry still reads `frame` offsets, but its
  clip div moves from `(row_header_w, COL_HEADER_H, content_w, content_h)` to the body dest rect,
  and it uses the body's `qorigin`.
- **`wrap_cells` auto-grow** (`view.rs:2962`, `run_autogrow` `:3550`) collects across all
  quadrants (a wrap-on cell in the top band still grows its frozen row); `run_autogrow`'s visible
  set becomes the union of quadrant ranges.

`FrameTiming.content_cells` (the perf FORCE+ASSERT witness, `view.rs:3214`) sums across quadrants,
so the perf harness still asserts real per-cell work.

---

## 3. Frozen-aware scroll / clamp / reveal (Q3) — the re-basing

The stored `scroll` value's **meaning changes** from "absolute content offset" to "**body-relative
offset**": `body_sx = 0` shows the first non-frozen **column** (`K`) at the body's left edge,
`body_sy = 0` shows the first non-frozen **row** (`M`) at the divider (`functional_spec.md §3.2`).
The stored `HashMap<SheetId,(f64,f64)>` type is unchanged; only its interpretation, applied
uniformly through the functions below. The key trick: **the existing pure `layout.rs` functions
work unchanged if fed frozen-adjusted totals and the body area** — no new clamp algebra, just the
right arguments.

Introduce one pure struct in `layout.rs` that carries the split so callers thread one value, not
six params:

```rust
/// The frozen-pane split for a frame (px extents + body-relative scroll), all frozen geometry
/// routes through this. `frozen_w`/`frozen_h` are `col_axis.offset_of(K)` / `row_axis.offset_of(M)`
/// (0 when that axis is unfrozen); with both 0 every method reduces to the pre-freeze math.
pub struct PaneGeometry {
    pub row_header_w: f32,
    pub frozen_w: f64,
    pub frozen_h: f64,
    pub content_w: f64,   // body area BEFORE removing the frozen band (viewport - headers)
    pub content_h: f64,
    pub body_sx: f64,     // body-relative scroll
    pub body_sy: f64,
}
```

with `body_area(&self) -> (f64, f64)` = `((content_w-frozen_w).max(0), (content_h-frozen_h).max(0))`.

### 3.1 `max_scroll` / `clamp_scroll` (`layout.rs:89/94`) — unchanged signatures

The scrollable extent on an axis is the **non-frozen** tracks against the body area:

```
max_body_sx = max_scroll(col_axis.total() - frozen_w, body_w)
max_body_sy = max_scroll(row_axis.total() - frozen_h, body_h)
```

so callers build a `ContentArea { row_header_w, width: body_w, height: body_h }` and call
`clamp_scroll(body_sx, body_sy, total_w - frozen_w, total_h - frozen_h, body_area)`. At
`body_sy = 0` the body top shows content-y `frozen_h` (row `M`); at `body_sy = max` the body
bottom `frozen_h + body_sy + body_h = total_h` (last row reachable) — exactly
`functional_spec.md §3.2` ("last row can reach the bottom of the body; never scroll content behind
a band or past the end"). The **call sites** to update (each currently builds a `ContentArea` and
clamps against `total_w/total_h`): `handle_scroll` (`view.rs:1161`), `autoscroll_tick`
(`view.rs:2552`), and the reveal path in `resolve_frame` (`view.rs:1072`). Each becomes a
`PaneGeometry` build + the frozen-adjusted clamp — the arithmetic is a parameter change, not new
logic (region-aware **at the call site**, not a new clamp function).

**Re-clamp on freeze/unfreeze + resize** (`functional_spec.md §3.2/§5.3`): freeze/unfreeze
republishes the cache with new `(M,K)`; the very next `resolve_frame` recomputes `frozen_w/frozen_h`
and re-clamps the stored `body_sx/body_sy` to the new `[0, max_body_*]` (the reveal/clamp path
already re-derives + stores on every frame). A window resize likewise shrinks `body_w/body_h` →
re-clamp on the next frame. No explicit "on freeze" hook is needed — the per-frame clamp is the
single chokepoint.

### 3.2 `scroll_to_reveal` (`layout.rs:173`) — no-op on a frozen axis

Add `PaneGeometry::reveal(row, col, row_axis, col_axis) -> (body_sx, body_sy)`:

- **Frozen-axis no-op:** if `row < M` the target row is already pinned in a band → **don't touch
  `body_sy`**; if `col < K` → don't touch `body_sx` (`functional_spec.md §3.3`).
- **Body target:** for a non-frozen row, reveal into the **body sub-area** using the re-based
  math — call the existing `reveal_axis` (`layout.rs:201`) with `content = body_h`,
  `scroll = body_sy`, `max = max_body_sy`, **and the axis offset re-based by `frozen_h`**: reveal
  against `offset_of(row) - frozen_h` (so the target aligns inside the region *below* the divider,
  never tucked under the band). Symmetric for columns with `frozen_w`.

This is the only reveal subtlety; `reveal_axis` itself is reused unchanged with re-based inputs.
Call sites: `resolve_frame`'s pending-reveal (`view.rs:1059`) and `reveal_and_announce`
(`view.rs:2612`).

### 3.3 `hit_test` / `cell_at_point` (`layout.rs:126/220`) — region routing

Both become `PaneGeometry` methods that **first classify the region**, then map to a track/cell
with `index_at` on the region's effective scroll:

- **Region classification** of grid-local `(x, y)`:
  - `y < COL_HEADER_H` → column-letter header. Split by x: `x` in
    `[row_header_w, row_header_w+frozen_w)` → frozen letter, content-x `= x - row_header_w`;
    `x ≥ row_header_w+frozen_w` → scrolling letter, content-x `= frozen_w + body_sx + (x -
    row_header_w - frozen_w)`.
  - `x < row_header_w` → row-number gutter, symmetric split on y with `frozen_h`/`body_sy`.
  - else content → sub-classify into Corner / TopBand / LeftBand / Body by comparing `x` to
    `row_header_w+frozen_w` and `y` to `COL_HEADER_H+frozen_h`, then map with that quadrant's
    `(qorigin_x, qorigin_y)`: `col = col_axis.index_at(qorigin_x + (x - dest_x0))`, etc.
- **`hit_test`** returns `GridHit` (Corner/ColHeader/RowHeader/Cell) exactly as today — the frozen
  bands are still cells/headers, just resolved through the right region. **`cell_at_point`** (used
  by drag-extend) resolves the actual cell under the pointer **in whatever region it lands**
  (frozen band cells are interactive, `functional_spec.md §2`), folding a header hit into the
  adjacent cell; it clamps the point into the **whole content rect** (not just the body) so a drag
  into a band selects that band's cell (`functional_spec.md §4` selection drags across the
  boundary).

With `frozen_w = frozen_h = body_sx = body_sy = 0` both methods reduce to the current
single-region math — so the existing `hit_test_*` / `cell_at_point_*` unit tests still pass
unchanged, and the no-freeze mouse path is untouched. Call sites: `handle_mouse_down`
(`view.rs:1339`), `handle_right_mouse_down` (`view.rs:2107`), `update_fill_drag` (`view.rs:2341`),
`autoscroll_tick` (`view.rs:2554`), `current_edge_delta` (`view.rs:2460`).

### 3.4 `edge_autoscroll_delta` (`layout.rs:264`) — fire only at the body's live edges

Auto-scroll must fire **only** near the scrolling body's edges, never the frozen bands
(`functional_spec.md §4` — a drag into a band does not auto-scroll). Reuse the pure
`edge_autoscroll_delta` unchanged, but pass the **body sub-rect** origin/size: `row_header_w +
frozen_w` as the left, `body_w` as the content width, `COL_HEADER_H + frozen_h` as the top,
`body_h` as the content height. Then the returned `(dx, dy)` is applied to `body_sx/body_sy` and
clamped via §3.1. A drag that leaves a band back into the body near its bottom/right resumes
auto-scroll (the delta fires again once the pointer re-enters the body hotzone). This is a
`PaneGeometry::edge_delta(...)` wrapper over the existing function (a hotzone against the body
rect, not the whole content) — no change to the pure function itself.

### 3.5 `scrollbar_thumb` (`layout.rs:108`) — over the scrolling region only

`functional_spec.md §4`: the overlay scrollbar represents the scrolling region only. Pass the
frozen-adjusted totals + body area: vertical thumb `= scrollbar_thumb(total_h - frozen_h, body_h,
body_sy, body_h as f32)` (drawn along the body sub-rect, offset by `frozen_h`); horizontal
symmetric. When `total_* - frozen_* ≤ body_*` (the non-frozen tracks fit) the thumb is `None` →
that axis's bar disappears even on a huge sheet (matches Excel). Call site: the scrollbar block in
`build_grid_layers` (`view.rs:3401`), whose track x/y also shift into the body sub-rect.

---

## 4. The freeze divider (`functional_spec.md §2.1`)

Drawn at the **root** level (fixed, over cells + headers, like the scrollbars), so it never
scrolls:

- **Horizontal divider** — iff `M > 0`: a `rect_div` at grid-y `= COL_HEADER_H + frozen_h`,
  spanning x `[row_header_w, row_header_w + content_w]` (full body width, across corner + top
  band), height = divider weight.
- **Vertical divider** — iff `K > 0`: at grid-x `= row_header_w + frozen_w`, spanning y
  `[COL_HEADER_H, COL_HEADER_H + content_h]`.

Visually distinct from a gridline (`GRIDLINE = 0xE2E2E2`): a new `FREEZE_DIVIDER` color const in
`grid/mod.rs` (a heavier mid-gray, e.g. `0x9E9E9E`) at ~1.5–2 px, matching the platform freeze-line
convention. Drawn only for an axis that is actually frozen; unfreezing (`M`/`K` → 0) drops it. This
is a new tinted rect at the root children after the header/scrollbar layers.

---

## 5. Resize hotspots (`functional_spec.md §4`, `view.rs:3651`)

Resize is per-track and never crosses regions, and a **frozen** track resizes exactly like a body
one (growing/shrinking the band; the divider follows because `frozen_w/frozen_h = offset_of(K/M)`
recomputes from the resized axis next frame). `resize_hotspots` iterates the **union** of visible
columns (`0..K ∪ body_cols`) and rows (`0..M ∪ body_rows`), placing each divider at its
**region-correct** x/y:

- a frozen-column divider at x `= row_header_w + col_offset(c) + col_size(c)` (no scroll);
- a body-column divider at x `= row_header_w + frozen_w + (col_offset(c) + col_size(c) - frozen_w -
  body_sx)` (scrolls with the body);
- symmetric for rows.

The `begin_resize` / `autofit_*` handlers (`view.rs:3679/3713`) are unchanged — they take a track
index, which is region-independent. The double-click autofit and select-all-resize guards are
untouched.

---

## 6. Degenerate cases (`functional_spec.md §5.1/§5.2`)

- **Band ≥ viewport:** `body_w`/`body_h` are floored at `0.0` (§1). When zero, that quadrant's
  visible range is empty (nothing to scroll on that axis), and the band quadrant **clips at the
  viewport edge** via its `overflow_hidden` div — the band does not scroll internally. `max_body_*
  = max_scroll(total - band, 0) = total - band` but with `body_* = 0` no body rows/cols are
  visible; no crash, no dialog. This is the tolerated display state the user escapes by enlarging
  the window or unfreezing.
- **Freeze at the last track ("freeze everything"):** just a large band + a zero/tiny body — the
  same §5.1 clip path; no block (freeze hides nothing).
- **Freeze at row 1 / col A** (`M=1`/`K=1`): a one-track band; nothing above/left of it. The
  common case; no special path.

---

## 7. Test plan (this component)

Pure `layout.rs` unit tests (headless, no gpui) are the backbone — the clamp/reveal/hit-test math
is where the risk is:

- **Re-based clamp:** with `M=2` (band = 2 rows), `body_sy=0` → body top = row 2; clamp keeps
  `body_sy ∈ [0, total_h - frozen_h - body_h]`; the last row is reachable at `body_sy = max`; you
  cannot scroll a frozen row into the body.
- **`M=K=0` equivalence:** every `PaneGeometry` method reproduces the current
  `hit_test`/`cell_at_point`/`clamp_scroll`/`scroll_to_reveal` results bit-for-bit (parametrize
  the existing `layout.rs` tests through `PaneGeometry` with zero band).
- **Reveal:** a target in the frozen rows/cols is a no-op on that axis; a body target aligns inside
  the body sub-area (its end never past `frozen_* + body_*`), never under the band.
- **Hit-test routing:** points in each of corner / top band / left band / body / frozen-vs-body
  column-letter header / row-number gutter resolve to the right cell/track, including scrolled
  bodies and variable geometry (widen the existing `hit_test_scrolled_variable_geometry` case with
  a band).
- **Auto-scroll:** delta is 0 inside a band, non-zero only near the body's live edges; a drag from
  the body into a band stops auto-scrolling, and back into the body resumes it.
- **Scrollbar:** the thumb spans the non-frozen extent and vanishes when the non-frozen tracks fit.

gpui view tests (`freecell-app`) cover the integration: `resolve_frame` yields the expected four
quadrant ranges for `(M,K)`; a frozen sheet renders bands at offset 0 while the body scrolls; the
divider is present iff the axis is frozen. The pixel baselines are `architecture.md §7 (Q6)` — a
dedicated late phase, never per coding phase.
