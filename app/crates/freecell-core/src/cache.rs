//! `SheetCaches` — the engine-free geometry + resolved-style **read model** the grid
//! consumes per frame (`components/style_cache.md`, `architecture.md §6`).
//!
//! This is the read side only: `SheetCaches`, `SheetCache`, `StyleId`, and the resolved
//! `RenderStyle` table. The worker-side `StyleInterner` (which touches IronCalc's `Style`)
//! and all build/mutation logic live in `freecell-engine::cache` (Phase 5). Keeping the
//! read model in `freecell-core` lets the grid and render-test fixtures build against it
//! without the engine — [`SheetCacheBuilder`] is the fixture/engine-facing constructor.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::axis::Axis;
use crate::border::BorderSpec;
use crate::refs::{CellRange, SheetId};
use crate::style::RenderStyle;

/// Default column width in px when the file specifies no override (`ui_design.md §3.3`).
pub const DEFAULT_COL_WIDTH_PX: f32 = 100.0;
/// Default row height in px when the file specifies no override (`ui_design.md §3.3`).
pub const DEFAULT_ROW_HEIGHT_PX: f32 = 24.0;

/// An index into a [`SheetCache`]'s resolved-style table. Interned worker-side so equal
/// styles share one id (`components/style_cache.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(pub u32);

/// The resident cache for one worksheet: variable geometry (default + sparse overrides,
/// fronted by prefix-sum [`Axis`]es) and resolved cell/band styles.
///
/// The worker builds it (`freecell-engine::cache::build_sheet_cache`) then keeps it in
/// agreement with IronCalc by **mirroring each issued edit** — the in-place mutators below
/// (`set_cell_style`, geometry setters, …) let a style edit touch only the changed cells
/// instead of rebuilding the whole sheet (`components/style_cache.md §Lifecycle`). Writes
/// happen on the worker thread under the `RwLock`; the UI only ever reads.
#[derive(Debug)]
pub struct SheetCache {
    row_count: u32,
    col_count: u32,
    row_default_px: f32,
    col_default_px: f32,
    row_overrides: BTreeMap<u32, f32>,
    col_overrides: BTreeMap<u32, f32>,
    row_axis: Arc<Axis>,
    col_axis: Arc<Axis>,
    cell_styles: BTreeMap<(u32, u32), StyleId>,
    row_styles: BTreeMap<u32, StyleId>,
    col_styles: BTreeMap<u32, StyleId>,
    /// `StyleId` → resolved render form (parallel table; `StyleId(i)` indexes `resolved[i]`).
    resolved: Vec<RenderStyle>,
    /// Reverse index for O(1) interning on mutation: equal [`RenderStyle`]s share one
    /// [`StyleId`]. `RenderStyle` is `Eq + Hash`, so — unlike the full IronCalc `Style`, which
    /// needed serialization to be a map key — the render form is a direct key
    /// (`components/style_cache.md`: the MVP read model holds `RenderStyle`, so the dedup is by
    /// render form). Entries are never removed; the resolved table only grows, bounded by the
    /// (small) number of distinct styles a sheet uses.
    style_ids: HashMap<RenderStyle, StyleId>,
    /// Number-format code strings, indexed by [`RenderStyle::num_fmt`]. `[0]` is always
    /// `"general"`. Built worker-side alongside the resolved-style table and read by the action
    /// bar for category display + decimals ± (`components/action_bar.md`). Tiny (a handful of
    /// distinct formats per sheet), so interning is a linear scan; `Arc<str>` keeps `SheetCache`
    /// `Send + Sync` without pulling gpui into this headless crate.
    num_fmts: Vec<Arc<str>>,
    /// Font-family names, indexed by [`RenderStyle::font_family`]. `[0]` is always `""` = the
    /// workbook default (rendered in the grid's default family). Built worker-side alongside the
    /// resolved-style table; the grid resolves a cell's family name from this for `.font_family`,
    /// and the action bar reads it for the family dropdown's active label (`components/style_render.md`,
    /// `components/action_bar.md`). Tiny (a handful of families per sheet) — same linear-scan
    /// interning + `Arc<str>` rationale as `num_fmts`.
    font_families: Vec<Arc<str>>,
    /// Resolved cell borders, indexed by [`RenderStyle::border`]. `[0]` is always
    /// [`BorderSpec::NONE`]. Built worker-side alongside the resolved-style table; the grid resolves
    /// a cell's `BorderSpec` from this to paint its edges (`components/style_render.md §Border
    /// painting`). Tiny (a handful of distinct border combinations per sheet), so interning is a
    /// linear scan — same rationale as `num_fmts` / `font_families`, except `BorderSpec` is
    /// `Copy + Eq + Hash` so it is a direct value (no `Arc<str>` needed to stay `Send + Sync`).
    border_specs: Vec<BorderSpec>,
    /// The **workbook's default font size** in quarter-points (e.g. `52` = 13pt) — the size a cell
    /// with `RenderStyle::font_size_q == 0` actually is. The action bar shows this in the size box
    /// for a default cell (so the label reflects the real default, not a hardcoded value —
    /// `components/action_bar.md`); the grid does not read it (default cells render at the grid's
    /// own `CELL_FONT_PX`). `0` when unknown (a bare fixture cache that never set it).
    default_font_size_q: u16,
    /// The sheet's file-loaded merged ranges (0-based), parsed once at cache build from
    /// `worksheet().merge_cells` (`components/grid_structure.md §5.3`). Consumed by the insert/
    /// delete **merge guard**: the UI disables a menu item whose op would displace a merge. Merged
    /// cells are otherwise unsupported (a deferred project), so the grid does not render them.
    merges: Vec<CellRange>,
}

/// The canonical default number-format code (IronCalc's `Style::default().num_fmt`, lowercase).
const DEFAULT_NUM_FMT: &str = "general";

/// The `num_fmts` table every fresh cache/builder starts with: `[0] = "general"`.
fn seed_num_fmts() -> Vec<Arc<str>> {
    vec![Arc::from(DEFAULT_NUM_FMT)]
}

/// The `font_families` table every fresh cache/builder starts with: `[0] = ""` (the workbook
/// default font, rendered in the grid's default family).
fn seed_font_families() -> Vec<Arc<str>> {
    vec![Arc::from("")]
}

/// The `border_specs` table every fresh cache/builder starts with: `[0] = BorderSpec::NONE` (so a
/// borderless cell interns to the default `RenderStyle`).
fn seed_border_specs() -> Vec<BorderSpec> {
    vec![BorderSpec::NONE]
}

/// Interns a [`BorderSpec`] into `table`, returning its [`RenderStyle::border`] id. The empty
/// [`BorderSpec::NONE`] always maps to `0`. Shares the `num_fmts` overflow guard: at the
/// (unreachable) `u16::MAX` cap a further distinct spec returns `u16::MAX`, which
/// [`SheetCache::border_spec`] resolves to `NONE` rather than wrapping to `0`.
fn intern_border_spec_into(table: &mut Vec<BorderSpec>, spec: BorderSpec) -> u16 {
    if spec.is_none() {
        return 0;
    }
    if let Some(idx) = table.iter().position(|s| *s == spec) {
        return idx as u16;
    }
    debug_assert!(
        table.len() < u16::MAX as usize,
        "border_specs table overflow (>= u16::MAX distinct borders)"
    );
    if table.len() >= u16::MAX as usize {
        return u16::MAX;
    }
    let id = table.len() as u16;
    table.push(spec);
    id
}

/// Interns a font-family `name` into `table`, returning its [`RenderStyle::font_family`] id. An
/// empty name (the workbook default) always maps to `0`. Shares the `num_fmts` overflow guard:
/// at the (unreachable) `u16::MAX` cap a further distinct name returns `u16::MAX`, which
/// [`SheetCache::font_family_name`] resolves to the default `""` rather than wrapping to `0`.
fn intern_font_family_into(table: &mut Vec<Arc<str>>, name: &str) -> u16 {
    if name.is_empty() {
        return 0;
    }
    if let Some(idx) = table.iter().position(|c| c.as_ref() == name) {
        return idx as u16;
    }
    debug_assert!(
        table.len() < u16::MAX as usize,
        "font_families table overflow (>= u16::MAX distinct families)"
    );
    if table.len() >= u16::MAX as usize {
        return u16::MAX;
    }
    let id = table.len() as u16;
    table.push(Arc::from(name));
    id
}

/// Interns a number-format `code` into `table`, returning its `u16` id. Case-insensitive
/// `"general"` always maps to `0` (so a plain cell interns to the default `RenderStyle`).
///
/// Ids are `u16`, so the table holds at most `u16::MAX` (65535) distinct formats — orders of
/// magnitude beyond the few dozen a real sheet uses. At that (unreachable) cap a further distinct
/// code is **not** interned: it returns `u16::MAX`, which [`SheetCache::num_fmt_code`] resolves to
/// the general fallback (out of range), rather than letting `len() as u16` wrap to `0` and collide
/// with `"general"`.
fn intern_num_fmt_into(table: &mut Vec<Arc<str>>, code: &str) -> u16 {
    if code.eq_ignore_ascii_case(DEFAULT_NUM_FMT) {
        return 0;
    }
    if let Some(idx) = table.iter().position(|c| c.as_ref() == code) {
        return idx as u16;
    }
    debug_assert!(
        table.len() < u16::MAX as usize,
        "num_fmts table overflow (>= u16::MAX distinct formats)"
    );
    if table.len() >= u16::MAX as usize {
        return u16::MAX;
    }
    let id = table.len() as u16;
    table.push(Arc::from(code));
    id
}

impl SheetCache {
    /// The resolved style for `(row, col)`, or `None` when the cell uses the default
    /// (plain) style. Resolution order is **cell > row-band > col-band > default** — the
    /// engine-defined precedence, SP4-verified (`components/style_cache.md`).
    pub fn render_style(&self, row: u32, col: u32) -> Option<&RenderStyle> {
        let id = self
            .cell_styles
            .get(&(row, col))
            .or_else(|| self.row_styles.get(&row))
            .or_else(|| self.col_styles.get(&col))?;
        self.resolved.get(id.0 as usize)
    }

    /// The two prefix-sum axes (row, col). Cheap `Arc` clones so the caller can drop the
    /// `RwLock` guard before doing layout math (`components/grid.md §Render pass`).
    pub fn axes(&self) -> (Arc<Axis>, Arc<Axis>) {
        (Arc::clone(&self.row_axis), Arc::clone(&self.col_axis))
    }

    /// The row axis.
    pub fn row_axis(&self) -> &Arc<Axis> {
        &self.row_axis
    }

    /// The column axis.
    pub fn col_axis(&self) -> &Arc<Axis> {
        &self.col_axis
    }

    /// The height (px) of a single row.
    pub fn row_height(&self, row: u32) -> f32 {
        self.row_axis.size_of(row)
    }

    /// The width (px) of a single column.
    pub fn col_width(&self, col: u32) -> f32 {
        self.col_axis.size_of(col)
    }

    /// The total content height (px) — the vertical scroll extent.
    pub fn total_height(&self) -> f64 {
        self.row_axis.total()
    }

    /// The total content width (px) — the horizontal scroll extent.
    pub fn total_width(&self) -> f64 {
        self.col_axis.total()
    }

    /// The sheet's dimensions `(rows, cols)`.
    pub fn dims(&self) -> (u32, u32) {
        (self.row_count, self.col_count)
    }

    /// The default row height (px) — the size of any row without an override.
    pub fn row_default_px(&self) -> f32 {
        self.row_default_px
    }

    /// The default column width (px) — the size of any column without an override.
    pub fn col_default_px(&self) -> f32 {
        self.col_default_px
    }

    /// The per-row height overrides (px). The engine's resize path (Phase 5) reads these
    /// to add/update an entry and rebuild the row axis (`components/style_cache.md`).
    pub fn row_overrides(&self) -> &BTreeMap<u32, f32> {
        &self.row_overrides
    }

    /// The per-column width overrides (px). See [`SheetCache::row_overrides`].
    pub fn col_overrides(&self) -> &BTreeMap<u32, f32> {
        &self.col_overrides
    }

    /// Whether `(row, col)` sits on a styled row or column band. The worker's mirror path uses
    /// this to decide whether a cell reverting to the *default* style must be stored as an
    /// explicit entry to **shadow** the band (reproducing IronCalc's rule that a cell present in
    /// the sheet data uses its own style, even the default, over any band).
    pub fn is_on_band(&self, row: u32, col: u32) -> bool {
        self.row_styles.contains_key(&row) || self.col_styles.contains_key(&col)
    }

    /// Interns a number-format `code` into the side table, returning its [`RenderStyle::num_fmt`]
    /// id. Case-insensitive `"general"` → `0`. The worker's mirror path calls this before storing
    /// a refreshed [`RenderStyle`] so the resident table carries every live format string.
    pub fn intern_num_fmt(&mut self, code: &str) -> u16 {
        intern_num_fmt_into(&mut self.num_fmts, code)
    }

    /// The number-format code string for [`RenderStyle::num_fmt`] id `id`. `0` (or an out-of-range
    /// id, defensively) → the default `"general"`. The action bar reads this for the active cell's
    /// format category + decimals ±.
    pub fn num_fmt_code(&self, id: u16) -> &str {
        self.num_fmts
            .get(id as usize)
            .map(|c| c.as_ref())
            .unwrap_or(DEFAULT_NUM_FMT)
    }

    /// Interns a font-family `name` into the side table, returning its [`RenderStyle::font_family`]
    /// id (empty `name` → `0` = the workbook default). The worker's mirror path calls this before
    /// storing a refreshed [`RenderStyle`] so the resident table carries every live family name.
    pub fn intern_font_family(&mut self, name: &str) -> u16 {
        intern_font_family_into(&mut self.font_families, name)
    }

    /// The font-family name for [`RenderStyle::font_family`] id `id`. `0` (or an out-of-range id,
    /// defensively) → `""` = the workbook default (the grid renders its default family). The grid
    /// resolves a cell's `.font_family` from this; the action bar reads it for the active cell's
    /// family label.
    pub fn font_family_name(&self, id: u16) -> &str {
        self.font_families
            .get(id as usize)
            .map(|c| c.as_ref())
            .unwrap_or("")
    }

    /// The full font-family side table — the grid snapshots this (cheap `Arc` clones) alongside the
    /// visible styles so it can resolve each cell's `.font_family` after releasing the cache lock.
    pub fn font_families(&self) -> &[Arc<str>] {
        &self.font_families
    }

    /// Interns a [`BorderSpec`] into the side table, returning its [`RenderStyle::border`] id
    /// ([`BorderSpec::NONE`] → `0`). The worker's mirror path calls this before storing a refreshed
    /// [`RenderStyle`] so the resident table carries every live border.
    pub fn intern_border_spec(&mut self, spec: BorderSpec) -> u16 {
        intern_border_spec_into(&mut self.border_specs, spec)
    }

    /// The [`BorderSpec`] for [`RenderStyle::border`] id `id`. `0` (or an out-of-range id,
    /// defensively) → [`BorderSpec::NONE`]. The grid resolves a cell's edges from this.
    pub fn border_spec(&self, id: u16) -> BorderSpec {
        self.border_specs
            .get(id as usize)
            .copied()
            .unwrap_or(BorderSpec::NONE)
    }

    /// The full border side table — the grid snapshots this (cheap `Copy`s) alongside the visible
    /// styles so it can resolve each cell's `BorderSpec` after releasing the cache lock.
    pub fn border_specs(&self) -> &[BorderSpec] {
        &self.border_specs
    }

    /// The workbook's default font size in quarter-points (`0` = unknown). The action bar reads this
    /// to label the size box for a default cell (`font_size_q == 0`) with the real workbook default.
    pub fn default_font_size_q(&self) -> u16 {
        self.default_font_size_q
    }

    /// The sheet's file-loaded merged ranges (0-based). The grid reads these to gate the insert/
    /// delete context-menu items behind the merge guard (`components/grid_structure.md §5.3`).
    pub fn merges(&self) -> &[CellRange] {
        &self.merges
    }

    /// Interns `style` into the resolved table (equal styles share a [`StyleId`]).
    fn intern(&mut self, style: RenderStyle) -> StyleId {
        if let Some(&id) = self.style_ids.get(&style) {
            return id;
        }
        let id = StyleId(self.resolved.len() as u32);
        self.resolved.push(style);
        self.style_ids.insert(style, id);
        id
    }

    /// Sets (or replaces) the per-cell style at `(row, col)` — the mirror-on-edit primitive.
    pub fn set_cell_style(&mut self, row: u32, col: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.cell_styles.insert((row, col), id);
    }

    /// Removes the per-cell style at `(row, col)` (it reverts to the band/default resolution).
    pub fn clear_cell_style(&mut self, row: u32, col: u32) {
        self.cell_styles.remove(&(row, col));
    }

    /// Sets (or replaces) a whole-row band style.
    pub fn set_row_band_style(&mut self, row: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.row_styles.insert(row, id);
    }

    /// Removes a whole-row band style.
    pub fn clear_row_band_style(&mut self, row: u32) {
        self.row_styles.remove(&row);
    }

    /// Sets (or replaces) a whole-column band style.
    pub fn set_col_band_style(&mut self, col: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.col_styles.insert(col, id);
    }

    /// Removes a whole-column band style.
    pub fn clear_col_band_style(&mut self, col: u32) {
        self.col_styles.remove(&col);
    }

    /// Sets a per-row height override (px) and rebuilds the row axis. `Axis` is immutable, so a
    /// geometry change rebuilds the affected axis (ms-scale even at 1M rows — measured in the
    /// POC; `components/style_cache.md §Data structures`).
    pub fn set_row_height(&mut self, row: u32, px: f32) {
        self.row_overrides.insert(row, px);
        self.rebuild_row_axis();
    }

    /// Removes a per-row height override (the row reverts to the default) and rebuilds the axis.
    pub fn reset_row_height(&mut self, row: u32) {
        if self.row_overrides.remove(&row).is_some() {
            self.rebuild_row_axis();
        }
    }

    /// Sets a per-column width override (px) and rebuilds the column axis.
    pub fn set_col_width(&mut self, col: u32, px: f32) {
        self.col_overrides.insert(col, px);
        self.rebuild_col_axis();
    }

    /// Removes a per-column width override (the column reverts to the default) and rebuilds it.
    pub fn reset_col_width(&mut self, col: u32) {
        if self.col_overrides.remove(&col).is_some() {
            self.rebuild_col_axis();
        }
    }

    /// Applies a batch of row-height overrides — `Some(px)` sets, `None` resets to the default —
    /// rebuilding the row axis **at most once**. The worker's mirror path uses this to reflect
    /// IronCalc's row-height auto-fit (a value edit can grow a row) across a touched range without
    /// an axis rebuild per row.
    pub fn set_row_heights(&mut self, updates: &[(u32, Option<f32>)]) {
        let mut changed = false;
        for &(row, px) in updates {
            match px {
                Some(v) => changed |= self.row_overrides.insert(row, v) != Some(v),
                None => changed |= self.row_overrides.remove(&row).is_some(),
            }
        }
        if changed {
            self.rebuild_row_axis();
        }
    }

    fn rebuild_row_axis(&mut self) {
        self.row_axis = Arc::new(Axis::from_overrides(
            self.row_count,
            self.row_default_px,
            self.row_overrides.clone(),
        ));
    }

    fn rebuild_col_axis(&mut self) {
        self.col_axis = Arc::new(Axis::from_overrides(
            self.col_count,
            self.col_default_px,
            self.col_overrides.clone(),
        ));
    }
}

/// Builds a [`SheetCache`] from geometry + styles. Used by test/render fixtures and by the
/// engine's Phase-5 cache builder. Style setters **intern** each [`RenderStyle`] into the
/// resolved table (equal styles share a [`StyleId`]), mirroring the worker's interner so
/// the read model's id→style mapping is consistent.
pub struct SheetCacheBuilder {
    row_count: u32,
    col_count: u32,
    row_default_px: f32,
    col_default_px: f32,
    row_overrides: BTreeMap<u32, f32>,
    col_overrides: BTreeMap<u32, f32>,
    cell_styles: BTreeMap<(u32, u32), StyleId>,
    row_styles: BTreeMap<u32, StyleId>,
    col_styles: BTreeMap<u32, StyleId>,
    resolved: Vec<RenderStyle>,
    style_ids: HashMap<RenderStyle, StyleId>,
    num_fmts: Vec<Arc<str>>,
    font_families: Vec<Arc<str>>,
    border_specs: Vec<BorderSpec>,
    default_font_size_q: u16,
    merges: Vec<CellRange>,
}

impl SheetCacheBuilder {
    /// A builder for a `rows`×`cols` sheet with default geometry and no styles.
    pub fn new(rows: u32, cols: u32) -> Self {
        Self {
            row_count: rows,
            col_count: cols,
            row_default_px: DEFAULT_ROW_HEIGHT_PX,
            col_default_px: DEFAULT_COL_WIDTH_PX,
            row_overrides: BTreeMap::new(),
            col_overrides: BTreeMap::new(),
            cell_styles: BTreeMap::new(),
            row_styles: BTreeMap::new(),
            col_styles: BTreeMap::new(),
            resolved: Vec::new(),
            style_ids: HashMap::new(),
            num_fmts: seed_num_fmts(),
            font_families: seed_font_families(),
            border_specs: seed_border_specs(),
            default_font_size_q: 0,
            merges: Vec::new(),
        }
    }

    /// Records the workbook's default font size (quarter-points) — the engine's build loop sets it
    /// so the action bar can label a default cell with the real workbook default size.
    pub fn set_default_font_size_q(&mut self, q: u16) {
        self.default_font_size_q = q;
    }

    /// Records a file-loaded merged range (0-based) — the engine's build loop pushes one per parsed
    /// `worksheet().merge_cells` entry, for the insert/delete merge guard.
    pub fn push_merge(&mut self, range: CellRange) {
        self.merges.push(range);
    }

    /// Records a merged range (consuming/fluent form for hand-built fixtures/tests).
    pub fn merge(mut self, range: CellRange) -> Self {
        self.push_merge(range);
        self
    }

    /// Interns a number-format `code` into the side table, returning its [`RenderStyle::num_fmt`]
    /// id (case-insensitive `"general"` → `0`). The engine's build loop calls this per cell/band
    /// before pushing the resolved style.
    pub fn intern_num_fmt(&mut self, code: &str) -> u16 {
        intern_num_fmt_into(&mut self.num_fmts, code)
    }

    /// Interns a font-family `name` into the side table, returning its [`RenderStyle::font_family`]
    /// id (empty `name` → `0` = the workbook default). The engine's build loop calls this per
    /// cell/band before pushing the resolved style.
    pub fn intern_font_family(&mut self, name: &str) -> u16 {
        intern_font_family_into(&mut self.font_families, name)
    }

    /// Interns a [`BorderSpec`] into the side table, returning its [`RenderStyle::border`] id
    /// ([`BorderSpec::NONE`] → `0`). The engine's build loop calls this per cell/band before pushing
    /// the resolved style.
    pub fn intern_border_spec(&mut self, spec: BorderSpec) -> u16 {
        intern_border_spec_into(&mut self.border_specs, spec)
    }

    /// Overrides the default row height / column width (px).
    pub fn defaults(mut self, row_height_px: f32, col_width_px: f32) -> Self {
        self.row_default_px = row_height_px;
        self.col_default_px = col_width_px;
        self
    }

    /// Interns `style` into the resolved table, keyed on the render form (`Eq + Hash`), so
    /// equal styles share a [`StyleId`]. The built cache inherits this map, so builder-built
    /// and mutation-built caches dedup identically.
    fn intern(&mut self, style: RenderStyle) -> StyleId {
        if let Some(&id) = self.style_ids.get(&style) {
            return id;
        }
        let id = StyleId(self.resolved.len() as u32);
        self.resolved.push(style);
        self.style_ids.insert(style, id);
        id
    }

    // --- Non-consuming setters (the engine's build-on-activation loop drives these) ---

    /// Sets a per-row height override (px).
    pub fn push_row_height(&mut self, row: u32, px: f32) {
        self.row_overrides.insert(row, px);
    }

    /// Sets a per-column width override (px).
    pub fn push_col_width(&mut self, col: u32, px: f32) {
        self.col_overrides.insert(col, px);
    }

    /// Interns + sets the style of a single cell.
    pub fn push_cell_style(&mut self, row: u32, col: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.cell_styles.insert((row, col), id);
    }

    /// Interns + sets a whole-row band style.
    pub fn push_row_style(&mut self, row: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.row_styles.insert(row, id);
    }

    /// Interns + sets a whole-column band style.
    pub fn push_col_style(&mut self, col: u32, style: RenderStyle) {
        let id = self.intern(style);
        self.col_styles.insert(col, id);
    }

    // --- Consuming setters (fluent, for hand-built fixtures/tests) ---

    /// Sets a per-row height override (px).
    pub fn row_height(mut self, row: u32, px: f32) -> Self {
        self.push_row_height(row, px);
        self
    }

    /// Sets a per-column width override (px).
    pub fn col_width(mut self, col: u32, px: f32) -> Self {
        self.push_col_width(col, px);
        self
    }

    /// Sets the style of a single cell.
    pub fn cell_style(mut self, row: u32, col: u32, style: RenderStyle) -> Self {
        self.push_cell_style(row, col, style);
        self
    }

    /// Sets a whole-row band style.
    pub fn row_style(mut self, row: u32, style: RenderStyle) -> Self {
        self.push_row_style(row, style);
        self
    }

    /// Sets a whole-column band style.
    pub fn col_style(mut self, col: u32, style: RenderStyle) -> Self {
        self.push_col_style(col, style);
        self
    }

    /// Builds the immutable-shell cache, constructing the prefix-sum axes from the geometry.
    pub fn build(self) -> SheetCache {
        let row_axis = Arc::new(Axis::from_overrides(
            self.row_count,
            self.row_default_px,
            self.row_overrides.clone(),
        ));
        let col_axis = Arc::new(Axis::from_overrides(
            self.col_count,
            self.col_default_px,
            self.col_overrides.clone(),
        ));
        SheetCache {
            row_count: self.row_count,
            col_count: self.col_count,
            row_default_px: self.row_default_px,
            col_default_px: self.col_default_px,
            row_overrides: self.row_overrides,
            col_overrides: self.col_overrides,
            row_axis,
            col_axis,
            cell_styles: self.cell_styles,
            row_styles: self.row_styles,
            col_styles: self.col_styles,
            resolved: self.resolved,
            style_ids: self.style_ids,
            num_fmts: self.num_fmts,
            font_families: self.font_families,
            border_specs: self.border_specs,
            default_font_size_q: self.default_font_size_q,
            merges: self.merges,
        }
    }
}

/// The set of resident per-sheet caches (active + visited sheets). Exposed to the UI as
/// `Arc<RwLock<SheetCaches>>`; the worker writes, the UI only reads.
#[derive(Debug, Default)]
pub struct SheetCaches {
    sheets: HashMap<SheetId, SheetCache>,
}

impl SheetCaches {
    /// An empty set (no sheets built yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// The cache for `sheet`, if it has been built.
    pub fn get(&self, sheet: SheetId) -> Option<&SheetCache> {
        self.sheets.get(&sheet)
    }

    /// Mutable access to the cache for `sheet` — the worker's mirror-on-edit path takes the
    /// write lock and updates the touched cells in place (`components/style_cache.md`).
    pub fn get_mut(&mut self, sheet: SheetId) -> Option<&mut SheetCache> {
        self.sheets.get_mut(&sheet)
    }

    /// Drops every resident cache whose sheet no longer satisfies `keep` — the worker calls this
    /// after a sheet add/delete (or its undo/redo) so caches for removed sheets don't linger.
    pub fn retain(&mut self, keep: impl Fn(SheetId) -> bool) {
        self.sheets.retain(|id, _| keep(*id));
    }

    /// Installs (or replaces) the cache for `sheet`.
    pub fn insert(&mut self, sheet: SheetId, cache: SheetCache) {
        self.sheets.insert(sheet, cache);
    }

    /// Drops the cache for `sheet` (e.g. sheet deleted).
    pub fn remove(&mut self, sheet: SheetId) -> Option<SheetCache> {
        self.sheets.remove(&sheet)
    }

    /// Whether `sheet` has a built cache.
    pub fn contains(&self, sheet: SheetId) -> bool {
        self.sheets.contains_key(&sheet)
    }

    /// The number of resident sheet caches.
    pub fn len(&self) -> usize {
        self.sheets.len()
    }

    /// Whether no sheet caches are resident.
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }
}

/// Compile-time guard for the phase's headline invariant: the resident cache lives behind
/// `Arc<RwLock<SheetCaches>>` shared between the worker (writes) and UI (reads) threads
/// (`architecture.md §2, §6`), so `SheetCache` and `SheetCaches` MUST stay `Send + Sync`.
/// If a future field breaks that (e.g. an `Rc`, or an `Axis` whose bound was dropped),
/// this fails to compile here rather than at the Phase-6 wiring site.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SheetCache>();
    assert_send_sync::<SheetCaches>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgb;
    use crate::limits;
    use crate::style::Align;

    fn bold() -> RenderStyle {
        RenderStyle {
            bold: true,
            ..RenderStyle::default()
        }
    }

    fn red_fill() -> RenderStyle {
        RenderStyle {
            fill: Some(Rgb::from_hex(0xFF0000)),
            ..RenderStyle::default()
        }
    }

    #[test]
    fn render_style_resolution_order() {
        // A cell style, a row band, and a col band all touch (5, 5); cell must win.
        let cache = SheetCacheBuilder::new(100, 100)
            .cell_style(5, 5, bold())
            .row_style(5, red_fill())
            .col_style(
                5,
                RenderStyle {
                    italic: true,
                    ..RenderStyle::default()
                },
            )
            .build();

        // cell > row > col
        assert_eq!(cache.render_style(5, 5), Some(&bold()));
        // only a row band applies → row band
        assert_eq!(cache.render_style(5, 9), Some(&red_fill()));
        // only a col band applies → col band
        assert_eq!(
            cache.render_style(9, 5),
            Some(&RenderStyle {
                italic: true,
                ..RenderStyle::default()
            })
        );
        // nothing applies → default (None)
        assert_eq!(cache.render_style(9, 9), None);
    }

    #[test]
    fn builder_interns_dedups() {
        // Two cells sharing a style → one entry in `resolved`; a distinct style adds one.
        let cache = SheetCacheBuilder::new(10, 10)
            .cell_style(0, 0, bold())
            .cell_style(1, 1, bold())
            .cell_style(2, 2, red_fill())
            .build();
        assert_eq!(cache.resolved.len(), 2, "equal styles must share a StyleId");
        assert_eq!(cache.cell_styles[&(0, 0)], cache.cell_styles[&(1, 1)]);
        assert_ne!(cache.cell_styles[&(0, 0)], cache.cell_styles[&(2, 2)]);
        // The alignment field participates in dedup identity.
        let c2 = SheetCacheBuilder::new(2, 2)
            .cell_style(
                0,
                0,
                RenderStyle {
                    h_align: Some(Align::Right),
                    ..RenderStyle::default()
                },
            )
            .cell_style(0, 1, RenderStyle::default())
            .build();
        assert_eq!(c2.resolved.len(), 2);
    }

    #[test]
    fn num_fmt_interning_dedups_and_general_is_zero() {
        let mut cache = SheetCacheBuilder::new(4, 4).build();
        // A fresh cache resolves index 0 to "general".
        assert_eq!(cache.num_fmt_code(0), "general");
        // Case-insensitive "general" always maps to 0 (so plain cells stay the default style).
        assert_eq!(cache.intern_num_fmt("general"), 0);
        assert_eq!(cache.intern_num_fmt("General"), 0);
        // Distinct codes get distinct, stable ids; equal codes dedup.
        let cur = cache.intern_num_fmt("$#,##0.00");
        let pct = cache.intern_num_fmt("0.00%");
        assert_ne!(cur, 0);
        assert_ne!(cur, pct);
        assert_eq!(cache.intern_num_fmt("$#,##0.00"), cur, "equal codes dedup");
        // Round-trips back to the string.
        assert_eq!(cache.num_fmt_code(cur), "$#,##0.00");
        assert_eq!(cache.num_fmt_code(pct), "0.00%");
        // An out-of-range id falls back to "general" (defensive, never panics).
        assert_eq!(cache.num_fmt_code(9999), "general");

        // The builder interns identically, and the built cache carries the table.
        let mut b = SheetCacheBuilder::new(2, 2);
        let id = b.intern_num_fmt("m/d/yyyy");
        let built = b
            .cell_style(
                0,
                0,
                RenderStyle {
                    num_fmt: id,
                    ..RenderStyle::default()
                },
            )
            .build();
        assert_eq!(built.num_fmt_code(id), "m/d/yyyy");
        assert_eq!(built.render_style(0, 0).unwrap().num_fmt, id);
    }

    #[test]
    fn font_family_interning_dedups_and_default_is_zero() {
        let mut cache = SheetCacheBuilder::new(4, 4).build();
        // A fresh cache resolves index 0 to "" (the workbook default family).
        assert_eq!(cache.font_family_name(0), "");
        // An empty name always maps to 0.
        assert_eq!(cache.intern_font_family(""), 0);
        // Distinct names get distinct, stable ids; equal names dedup.
        let serif = cache.intern_font_family("Times New Roman");
        let mono = cache.intern_font_family("Courier New");
        assert_ne!(serif, 0);
        assert_ne!(serif, mono);
        assert_eq!(
            cache.intern_font_family("Times New Roman"),
            serif,
            "equal names dedup"
        );
        // Round-trips back to the name.
        assert_eq!(cache.font_family_name(serif), "Times New Roman");
        assert_eq!(cache.font_family_name(mono), "Courier New");
        // An out-of-range id falls back to "" (defensive, never panics).
        assert_eq!(cache.font_family_name(9999), "");

        // The builder interns identically, and the built cache carries the table.
        let mut b = SheetCacheBuilder::new(2, 2);
        let id = b.intern_font_family("Arial");
        let built = b
            .cell_style(
                0,
                0,
                RenderStyle {
                    font_family: id,
                    ..RenderStyle::default()
                },
            )
            .build();
        assert_eq!(built.font_family_name(id), "Arial");
        assert_eq!(built.render_style(0, 0).unwrap().font_family, id);
        // The whole side table is exposed for the grid snapshot.
        assert_eq!(built.font_families().len(), 2); // [""], ["Arial"]
    }

    #[test]
    fn border_spec_interning_dedups_and_none_is_zero() {
        use crate::border::{BorderSpec, Edge};
        let thin = |w| Some(Edge::new(w, Rgb::new(0, 0, 0)));
        let all_thin = BorderSpec {
            top: thin(1),
            right: thin(1),
            bottom: thin(1),
            left: thin(1),
        };
        let thick_bottom = BorderSpec {
            bottom: thin(3),
            ..BorderSpec::NONE
        };

        let mut cache = SheetCacheBuilder::new(4, 4).build();
        // A fresh cache resolves index 0 to NONE.
        assert_eq!(cache.border_spec(0), BorderSpec::NONE);
        // NONE always maps to 0.
        assert_eq!(cache.intern_border_spec(BorderSpec::NONE), 0);
        // Distinct specs get distinct, stable ids; equal specs dedup.
        let a = cache.intern_border_spec(all_thin);
        let b = cache.intern_border_spec(thick_bottom);
        assert_ne!(a, 0);
        assert_ne!(a, b);
        assert_eq!(cache.intern_border_spec(all_thin), a, "equal specs dedup");
        // Round-trips back to the spec.
        assert_eq!(cache.border_spec(a), all_thin);
        assert_eq!(cache.border_spec(b), thick_bottom);
        // An out-of-range id falls back to NONE (defensive, never panics).
        assert_eq!(cache.border_spec(9999), BorderSpec::NONE);

        // The builder interns identically, and the built cache carries the table; `border`
        // participates in StyleId identity (two cells differing only by border → distinct ids).
        let mut bld = SheetCacheBuilder::new(2, 2);
        let id = bld.intern_border_spec(all_thin);
        let built = bld
            .cell_style(
                0,
                0,
                RenderStyle {
                    border: id,
                    ..RenderStyle::default()
                },
            )
            .cell_style(0, 1, RenderStyle::default())
            .build();
        assert_eq!(built.border_spec(id), all_thin);
        assert_eq!(built.render_style(0, 0).unwrap().border, id);
        assert_eq!(built.resolved.len(), 2, "border distinguishes StyleIds");
        assert_eq!(built.border_specs().len(), 2); // [NONE, all_thin]
    }

    #[test]
    fn merges_round_trip_through_builder() {
        use crate::refs::{CellRange, CellRef};
        // A fresh cache carries no merges.
        assert!(SheetCacheBuilder::new(4, 4).build().merges().is_empty());
        // Pushed merges survive the build in order.
        let a = CellRange::new(CellRef::new(6, 10), CellRef::new(9, 11)); // K7:L10
        let b = CellRange::single(CellRef::new(0, 0)); // A1
        let cache = SheetCacheBuilder::new(20, 20).merge(a).merge(b).build();
        assert_eq!(cache.merges(), &[a, b]);
    }

    #[test]
    fn geometry_defaults_and_overrides() {
        let cache = SheetCacheBuilder::new(10, 10)
            .row_height(3, 60.0)
            .col_width(2, 250.0)
            .build();
        assert_eq!(cache.row_height(0), DEFAULT_ROW_HEIGHT_PX);
        assert_eq!(cache.row_height(3), 60.0);
        assert_eq!(cache.col_width(0), DEFAULT_COL_WIDTH_PX);
        assert_eq!(cache.col_width(2), 250.0);
        assert_eq!(cache.dims(), (10, 10));

        // The defaults + sparse overrides are exposed for the engine's rebuild path.
        assert_eq!(cache.row_default_px(), DEFAULT_ROW_HEIGHT_PX);
        assert_eq!(cache.col_default_px(), DEFAULT_COL_WIDTH_PX);
        assert_eq!(cache.row_overrides().get(&3), Some(&60.0));
        assert_eq!(cache.col_overrides().get(&2), Some(&250.0));
        assert!(cache.row_overrides().get(&0).is_none());
    }

    #[test]
    fn axes_total_matches_geometry() {
        let cache = SheetCacheBuilder::new(4, 3)
            .defaults(20.0, 100.0)
            .row_height(1, 50.0)
            .col_width(0, 200.0)
            .build();
        // rows: 20 + 50 + 20 + 20 = 110
        assert!((cache.total_height() - 110.0).abs() < 1e-6);
        // cols: 200 + 100 + 100 = 400
        assert!((cache.total_width() - 400.0).abs() < 1e-6);
    }

    #[test]
    fn excel_max_geometry_totals() {
        // 1M rows at the 24px default — the total is exact and cheap (no per-row array).
        let cache = SheetCacheBuilder::new(limits::MAX_ROWS, limits::MAX_COLS).build();
        assert!(
            (cache.total_height() - limits::MAX_ROWS as f64 * DEFAULT_ROW_HEIGHT_PX as f64).abs()
                < 1.0
        );
        assert!(
            (cache.total_width() - limits::MAX_COLS as f64 * DEFAULT_COL_WIDTH_PX as f64).abs()
                < 1.0
        );
    }

    #[test]
    fn mutators_intern_and_resolve() {
        // Start from a built cache, then drive the mirror-on-edit mutators.
        let mut cache = SheetCacheBuilder::new(10, 10).build();
        cache.set_cell_style(1, 1, bold());
        cache.set_cell_style(2, 2, bold()); // shares the bold StyleId
        cache.set_cell_style(3, 3, red_fill()); // distinct
        assert_eq!(cache.render_style(1, 1), Some(&bold()));
        assert_eq!(cache.render_style(3, 3), Some(&red_fill()));
        assert_eq!(cache.cell_styles[&(1, 1)], cache.cell_styles[&(2, 2)]);
        assert_ne!(cache.cell_styles[&(1, 1)], cache.cell_styles[&(3, 3)]);
        assert_eq!(cache.resolved.len(), 2, "equal styles share one StyleId");

        // Clearing reverts to default resolution.
        cache.clear_cell_style(1, 1);
        assert_eq!(cache.render_style(1, 1), None);

        // Band set/clear + resolution order (cell > row > col).
        cache.set_row_band_style(5, red_fill());
        cache.set_col_band_style(6, bold());
        assert_eq!(cache.render_style(5, 9), Some(&red_fill())); // row band
        assert_eq!(cache.render_style(9, 6), Some(&bold())); // col band
        cache.set_cell_style(5, 6, RenderStyle::default());
        assert_eq!(
            cache.render_style(5, 6),
            Some(&RenderStyle::default()),
            "a cell entry (even default) shadows the bands"
        );
        cache.clear_row_band_style(5);
        assert_eq!(cache.render_style(5, 9), None);
        cache.clear_col_band_style(6);
        assert_eq!(cache.render_style(9, 6), None);
    }

    #[test]
    fn is_on_band_detects_band_membership() {
        let mut cache = SheetCacheBuilder::new(10, 10).build();
        assert!(!cache.is_on_band(3, 4));
        cache.set_row_band_style(3, bold());
        assert!(cache.is_on_band(3, 0) && cache.is_on_band(3, 4));
        assert!(!cache.is_on_band(4, 0));
        cache.set_col_band_style(4, red_fill());
        assert!(cache.is_on_band(9, 4));
    }

    #[test]
    fn geometry_mutation_rebuilds_axis() {
        let mut cache = SheetCacheBuilder::new(4, 3).defaults(20.0, 100.0).build();
        assert!((cache.total_height() - 80.0).abs() < 1e-6);
        cache.set_row_height(1, 50.0);
        assert_eq!(cache.row_height(1), 50.0);
        // 20 + 50 + 20 + 20 = 110; the axis (offsets) reflects the change.
        assert!((cache.total_height() - 110.0).abs() < 1e-6);
        assert!((cache.row_axis().offset_of(2) - 70.0).abs() < 1e-6);
        cache.reset_row_height(1);
        assert_eq!(cache.row_height(1), 20.0);
        assert!((cache.total_height() - 80.0).abs() < 1e-6);

        cache.set_col_width(0, 250.0);
        assert_eq!(cache.col_width(0), 250.0);
        assert!((cache.total_width() - (250.0 + 100.0 + 100.0)).abs() < 1e-6);
        cache.reset_col_width(0);
        assert!((cache.total_width() - 300.0).abs() < 1e-6);
    }

    #[test]
    fn set_row_heights_batches_sets_and_resets() {
        let mut cache = SheetCacheBuilder::new(5, 2).defaults(20.0, 100.0).build();
        cache.set_row_heights(&[(0, Some(40.0)), (1, Some(60.0)), (2, None)]);
        assert_eq!(cache.row_height(0), 40.0);
        assert_eq!(cache.row_height(1), 60.0);
        assert_eq!(cache.row_height(2), 20.0); // reset/absent → default
                                               // total = 40 + 60 + 20 + 20 + 20 = 160, axis rebuilt.
        assert!((cache.total_height() - 160.0).abs() < 1e-6);
        assert!((cache.row_axis().offset_of(2) - 100.0).abs() < 1e-6);
        // A reset removes the override.
        cache.set_row_heights(&[(0, None)]);
        assert_eq!(cache.row_height(0), 20.0);
    }

    #[test]
    fn sheet_caches_insert_get_remove() {
        let mut caches = SheetCaches::new();
        assert!(caches.is_empty());
        caches.insert(SheetId(0), SheetCacheBuilder::new(5, 5).build());
        assert!(caches.contains(SheetId(0)));
        assert_eq!(caches.len(), 1);
        assert!(caches.get(SheetId(0)).is_some());
        assert!(caches.get(SheetId(1)).is_none());
        assert!(caches.remove(SheetId(0)).is_some());
        assert!(caches.is_empty());
    }
}
