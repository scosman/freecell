//! The resident style/geometry cache prototype (architecture §4.1–§4.3).
//!
//! This is a **headless** stand-in for the frontend cache `projects/style-cache.md`
//! describes. It holds, per axis (rows / columns):
//!   - a **default size** + a **sparse override map** (index → size),
//!   - a **default band style** + a **sparse override map** (index → interned `StyleId`),
//! plus a per-cell sparse style override map and a **dense cumulative-size prefix sum**
//! for scroll math (architecture §4.3 candidate **(a)** — the simple option we measure
//! before rejecting).
//!
//! The load-bearing operation is [`Axis::shift`] (insert/delete): re-key the sparse maps
//! for indices `>= at`, splice the dense sizes array, and patch the prefix sum. On a
//! delete it **returns the removed overrides** so undo can restore them exactly
//! (architecture §4.3 "mirror-the-primitive" undo strategy).
//!
//! Indices here are **1-based** to match IronCalc (row/column 1 is the first line), so a
//! sync check reads `cache.rows.size(r)` against `user_model.get_row_height(sheet, r)`
//! with the same `r`.

use std::collections::BTreeMap;

use ironcalc_base::types::Style;

/// An interned style handle. `0` is reserved for the default style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleId(pub u32);

/// Deduplicating style interner. `Style` derives `Eq` but **not `Hash`**, so we key the
/// lookup table on the style's `bitcode` encoding (a stable byte identity) rather than on
/// `Style` directly. Styles are highly repetitive (SP4/SP5), so this collapses a whole
/// sheet to a handful of ids.
#[derive(Default)]
pub struct StyleInterner {
    by_key: std::collections::HashMap<Vec<u8>, StyleId>,
    styles: Vec<Style>,
}

impl StyleInterner {
    pub fn new() -> Self {
        // Id 0 is always the default style, matching IronCalc's style_index 0.
        let mut interner = Self::default();
        let default = Style::default();
        interner.by_key.insert(encode_style(&default), StyleId(0));
        interner.styles.push(default);
        interner
    }

    /// Interns `style`, returning a stable id shared by all equal styles.
    pub fn intern(&mut self, style: &Style) -> StyleId {
        let key = encode_style(style);
        if let Some(id) = self.by_key.get(&key) {
            return *id;
        }
        let id = StyleId(self.styles.len() as u32);
        self.styles.push(style.clone());
        self.by_key.insert(key, id);
        id
    }

    /// Resolves an id back to its style (for agreement checks).
    pub fn resolve(&self, id: StyleId) -> &Style {
        &self.styles[id.0 as usize]
    }

    /// Number of distinct interned styles (default counts as one).
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

/// A stable byte identity for a `Style`. `Style` derives `Eq` but not `Hash`, so we key
/// the interner on its serialized form (styles are highly repetitive, SP4/SP5, so this
/// collapses a whole sheet to a few ids). Serialization is total for `Style`.
fn encode_style(style: &Style) -> Vec<u8> {
    serde_json::to_vec(style).expect("Style serializes")
}

/// Overrides removed by a delete, kept so an undo can restore them.
#[derive(Debug, Default, Clone)]
pub struct RemovedOverrides {
    pub sizes: Vec<(i64, f64)>,
    pub band_styles: Vec<(i64, StyleId)>,
}

/// Per-cell style overrides dropped by a delete (keyed by pre-delete coordinate), returned
/// so an undo can restore them.
pub type RemovedCellStyles = Vec<((i64, i64), StyleId)>;

/// One axis (rows or columns) of the resident cache.
pub struct Axis {
    default_size: f64,
    default_band_style: StyleId,
    /// index → custom size (sparse; only lines that differ from the default).
    size_overrides: BTreeMap<i64, f64>,
    /// index → interned band style (sparse).
    band_style_overrides: BTreeMap<i64, StyleId>,
    /// Dense sizes over `1..=extent` (index 0 unused) + its prefix sum, for scroll math.
    /// This is architecture §4.3(a): O(1) `offset`/`index_at`, O(extent) splice.
    dense_sizes: Vec<f64>,
    prefix: Vec<f64>,
    prefix_dirty: bool,
}

impl Axis {
    /// A fresh axis of `extent` lines, all at `default_size`, band style = default.
    pub fn new(extent: usize, default_size: f64) -> Self {
        // dense_sizes[0] is a padding slot so lines are 1-based.
        let dense_sizes = vec![default_size; extent + 1];
        let mut axis = Self {
            default_size,
            default_band_style: StyleId(0),
            size_overrides: BTreeMap::new(),
            band_style_overrides: BTreeMap::new(),
            dense_sizes,
            prefix: Vec::new(),
            prefix_dirty: true,
        };
        axis.rebuild_prefix();
        axis
    }

    pub fn extent(&self) -> usize {
        self.dense_sizes.len().saturating_sub(1)
    }

    /// The size of line `index` (1-based): an override if present, else the default.
    pub fn size(&self, index: i64) -> f64 {
        *self.size_overrides.get(&index).unwrap_or(&self.default_size)
    }

    /// The band style of line `index` (1-based): an override if present, else default.
    pub fn band_style(&self, index: i64) -> StyleId {
        *self
            .band_style_overrides
            .get(&index)
            .unwrap_or(&self.default_band_style)
    }

    /// Records a custom size (also patched into the dense array + prefix).
    pub fn set_size(&mut self, index: i64, size: f64) {
        self.size_overrides.insert(index, size);
        if let Some(slot) = self.dense_sizes.get_mut(index as usize) {
            *slot = size;
            self.prefix_dirty = true;
        }
    }

    /// Records a band style override.
    pub fn set_band_style(&mut self, index: i64, style: StyleId) {
        if style == self.default_band_style {
            self.band_style_overrides.remove(&index);
        } else {
            self.band_style_overrides.insert(index, style);
        }
    }

    /// Cumulative pixels **before** line `index` (1-based). `offset(1) == 0`.
    /// This is the renderer's pixel-from-index scroll lookup. With
    /// `prefix[i] = sum(sizes 1..=i)` and `prefix[0] = 0`, `offset(index) = prefix[index-1]`.
    pub fn offset(&mut self, index: i64) -> f64 {
        self.ensure_prefix();
        let i = (index.max(1) as usize).min(self.prefix.len());
        self.prefix[i - 1]
    }

    /// The 1-based line index whose span contains pixel `y` (renderer's index-from-pixel
    /// lookup): the largest `k` with `offset(k) <= y`, i.e. `prefix[k-1] <= y`.
    pub fn index_at(&mut self, y: f64) -> i64 {
        self.ensure_prefix();
        let extent = self.extent();
        if extent == 0 {
            return 1;
        }
        let mut lo = 1usize;
        let mut hi = extent;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            // offset(mid) = prefix[mid-1].
            if self.prefix[mid - 1] <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo as i64
    }

    fn ensure_prefix(&mut self) {
        if self.prefix_dirty {
            self.rebuild_prefix();
        }
    }

    fn rebuild_prefix(&mut self) {
        // `prefix[i]` = sum of `dense_sizes[1..=i]` (with `prefix[0] = 0`), so
        // `offset(index)` (pixels before line `index`) is `prefix[index - 1]`.
        let n = self.dense_sizes.len();
        let mut prefix = vec![0.0; n];
        let mut sum = 0.0;
        for i in 1..n {
            sum += self.dense_sizes[i];
            prefix[i] = sum;
        }
        self.prefix = prefix;
        self.prefix_dirty = false;
    }

    /// Insert `count` default lines at `at`, or delete `count` lines starting at `at`.
    /// `count > 0` inserts, `count < 0` deletes `|count|`. Returns any overrides removed
    /// by a delete so undo can restore them (mirror-the-primitive).
    pub fn shift(&mut self, at: i64, count: i64) -> RemovedOverrides {
        let mut removed = RemovedOverrides::default();
        if count > 0 {
            self.reinsert_after_insert(at, count);
            self.splice_insert(at, count);
        } else if count < 0 {
            let delete_count = -count;
            removed = self.reinsert_after_delete(at, delete_count);
            self.splice_delete(at, delete_count);
        }
        removed
    }

    // --- helpers -----------------------------------------------------------------

    /// Re-key both sparse maps for an insert: every override at index `>= at` moves up by
    /// `count`. O(overrides shifted), not O(extent) (architecture §4.2 target).
    fn reinsert_after_insert(&mut self, at: i64, count: i64) {
        self.size_overrides = shift_keys_up(std::mem::take(&mut self.size_overrides), at, count);
        self.band_style_overrides =
            shift_keys_up(std::mem::take(&mut self.band_style_overrides), at, count);
    }

    /// Re-key both sparse maps for a delete: drop overrides in `[at, at+count)`, shift the
    /// rest down. Returns the dropped overrides so undo can restore them.
    fn reinsert_after_delete(&mut self, at: i64, count: i64) -> RemovedOverrides {
        let mut removed = RemovedOverrides::default();
        let (sizes, removed_sizes) =
            shift_keys_down(std::mem::take(&mut self.size_overrides), at, count);
        let (bands, removed_bands) =
            shift_keys_down(std::mem::take(&mut self.band_style_overrides), at, count);
        self.size_overrides = sizes;
        self.band_style_overrides = bands;
        removed.sizes = removed_sizes;
        removed.band_styles = removed_bands;
        removed
    }

    fn splice_insert(&mut self, at: i64, count: i64) {
        let at = (at as usize).min(self.dense_sizes.len());
        let inserted = vec![self.default_size; count as usize];
        self.dense_sizes.splice(at..at, inserted);
        self.prefix_dirty = true;
    }

    fn splice_delete(&mut self, at: i64, count: i64) {
        let start = (at as usize).min(self.dense_sizes.len());
        let end = (start + count as usize).min(self.dense_sizes.len());
        self.dense_sizes.drain(start..end);
        self.prefix_dirty = true;
    }

    /// Restore overrides removed by a delete (undo of a delete).
    pub fn restore_removed(&mut self, removed: &RemovedOverrides) {
        for (k, v) in &removed.sizes {
            self.size_overrides.insert(*k, *v);
            if let Some(slot) = self.dense_sizes.get_mut(*k as usize) {
                *slot = *v;
            }
        }
        for (k, v) in &removed.band_styles {
            self.band_style_overrides.insert(*k, *v);
        }
        self.prefix_dirty = true;
    }

    /// Count of size + band-style overrides (a proxy for cache footprint).
    pub fn override_count(&self) -> usize {
        self.size_overrides.len() + self.band_style_overrides.len()
    }
}

/// Shift every key `>= at` up by `count` (used on insert).
fn shift_keys_up<V: Copy>(map: BTreeMap<i64, V>, at: i64, count: i64) -> BTreeMap<i64, V> {
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if k >= at {
            out.insert(k + count, v);
        } else {
            out.insert(k, v);
        }
    }
    out
}

/// Drop keys in `[at, at+count)`, shift keys `>= at+count` down by `count` (used on
/// delete). Returns the surviving map and the dropped `(key, value)` pairs (keyed by
/// their **pre-delete** index, so an undo restores them at the right place).
fn shift_keys_down<V: Copy>(
    map: BTreeMap<i64, V>,
    at: i64,
    count: i64,
) -> (BTreeMap<i64, V>, Vec<(i64, V)>) {
    let mut out = BTreeMap::new();
    let mut removed = Vec::new();
    for (k, v) in map {
        if k >= at && k < at + count {
            removed.push((k, v));
        } else if k >= at + count {
            out.insert(k - count, v);
        } else {
            out.insert(k, v);
        }
    }
    (out, removed)
}

/// The full two-axis resident cache.
pub struct ResidentCache {
    pub rows: Axis,
    pub cols: Axis,
    pub cell_styles: BTreeMap<(i64, i64), StyleId>,
    pub interner: StyleInterner,
}

impl ResidentCache {
    pub fn new(row_extent: usize, col_extent: usize, default_row: f64, default_col: f64) -> Self {
        Self {
            rows: Axis::new(row_extent, default_row),
            cols: Axis::new(col_extent, default_col),
            cell_styles: BTreeMap::new(),
            interner: StyleInterner::new(),
        }
    }

    pub fn cell_style(&self, row: i64, col: i64) -> StyleId {
        *self.cell_styles.get(&(row, col)).unwrap_or(&StyleId(0))
    }

    pub fn set_cell_style(&mut self, row: i64, col: i64, id: StyleId) {
        if id == StyleId(0) {
            self.cell_styles.remove(&(row, col));
        } else {
            self.cell_styles.insert((row, col), id);
        }
    }

    /// Shift rows (insert `count>0` / delete `count<0` at `at`), re-keying cell styles on
    /// the row axis too. Returns removed data for undo.
    pub fn shift_rows(&mut self, at: i64, count: i64) -> (RemovedOverrides, RemovedCellStyles) {
        let removed = self.rows.shift(at, count);
        let removed_cells = shift_cell_styles_row(&mut self.cell_styles, at, count);
        (removed, removed_cells)
    }

    /// Shift columns, re-keying cell styles on the column axis.
    pub fn shift_cols(&mut self, at: i64, count: i64) -> (RemovedOverrides, RemovedCellStyles) {
        let removed = self.cols.shift(at, count);
        let removed_cells = shift_cell_styles_col(&mut self.cell_styles, at, count);
        (removed, removed_cells)
    }

    pub fn restore_row_shift(
        &mut self,
        removed: &RemovedOverrides,
        removed_cells: &[((i64, i64), StyleId)],
    ) {
        self.rows.restore_removed(removed);
        for ((r, c), id) in removed_cells {
            self.cell_styles.insert((*r, *c), *id);
        }
    }

    pub fn restore_col_shift(
        &mut self,
        removed: &RemovedOverrides,
        removed_cells: &[((i64, i64), StyleId)],
    ) {
        self.cols.restore_removed(removed);
        for ((r, c), id) in removed_cells {
            self.cell_styles.insert((*r, *c), *id);
        }
    }
}

fn shift_cell_styles_row(
    map: &mut BTreeMap<(i64, i64), StyleId>,
    at: i64,
    count: i64,
) -> RemovedCellStyles {
    let old = std::mem::take(map);
    let mut removed = Vec::new();
    if count > 0 {
        for ((r, c), id) in old {
            if r >= at {
                map.insert((r + count, c), id);
            } else {
                map.insert((r, c), id);
            }
        }
    } else {
        let del = -count;
        for ((r, c), id) in old {
            if r >= at && r < at + del {
                removed.push(((r, c), id));
            } else if r >= at + del {
                map.insert((r - del, c), id);
            } else {
                map.insert((r, c), id);
            }
        }
    }
    removed
}

fn shift_cell_styles_col(
    map: &mut BTreeMap<(i64, i64), StyleId>,
    at: i64,
    count: i64,
) -> RemovedCellStyles {
    let old = std::mem::take(map);
    let mut removed = Vec::new();
    if count > 0 {
        for ((r, c), id) in old {
            if c >= at {
                map.insert((r, c + count), id);
            } else {
                map.insert((r, c), id);
            }
        }
    } else {
        let del = -count;
        for ((r, c), id) in old {
            if c >= at && c < at + del {
                removed.push(((r, c), id));
            } else if c >= at + del {
                map.insert((r, c - del), id);
            } else {
                map.insert((r, c), id);
            }
        }
    }
    removed
}
