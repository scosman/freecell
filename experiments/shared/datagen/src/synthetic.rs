//! [`SyntheticSheet`]: a deterministic, seedable [`CellSource`] that produces a
//! varied, realistic-looking spreadsheet proxy without any RNG state or globals.
//!
//! Determinism is a hard requirement (functional_spec §5.3, architecture §3):
//! the same `(seed, row, col)` always yields the same [`CellData`], so results are
//! reproducible and generation is trivially thread-safe.

use crate::cell::{CellData, CellFormat, CellSource, CellValue, HAlign, Rgb};

/// A pool of dictionary-ish words used to build varied text of differing lengths.
const WORDS: &[&str] = &[
    "alpha",
    "revenue",
    "Q3",
    "north-east region",
    "widget",
    "forecast",
    "customer lifetime value estimate",
    "n/a",
    "TOTAL",
    "misc",
    "cost of goods sold (adjusted)",
    "ok",
    "pending review by finance",
    "SKU-4471",
    "x",
];

/// A palette of highlight colours applied to the ~10–20% highlighted cells.
const HIGHLIGHTS: &[Rgb] = &[
    Rgb::new(255, 249, 196), // pale yellow
    Rgb::new(200, 230, 201), // pale green
    Rgb::new(255, 205, 210), // pale red
    Rgb::new(187, 222, 251), // pale blue
];

/// A deterministic synthetic sheet. Cloneable and cheap; holds only its seed and
/// logical dimensions.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticSheet {
    seed: u64,
    rows: u32,
    cols: u32,
}

impl SyntheticSheet {
    /// Creates a synthetic sheet of `rows × cols` addressable cells, driven by
    /// `seed`. Dimensions bound [`CellSource::cell`] callers logically; out-of-range
    /// coordinates still return deterministic data (they are simply not part of the
    /// declared sheet).
    pub fn new(seed: u64, rows: u32, cols: u32) -> Self {
        Self { seed, rows, cols }
    }

    /// The declared row count.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The declared column count.
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// The seed used for generation.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The deterministic width (in logical pixels) of a column.
    ///
    /// Most columns are a normal width; a deterministic minority are "very wide"
    /// to exercise variable-width layout and horizontal scrolling (architecture §7).
    pub fn col_width(&self, col: u32) -> f32 {
        let h = mix(self.seed ^ 0x00C0_1DBE_EF00_0001, col as u64);
        // ~1 in 12 columns is very wide; the rest vary around a normal width.
        if h.is_multiple_of(12) {
            260.0 + (h % 140) as f32 // 260..=399 px
        } else {
            72.0 + (h % 56) as f32 // 72..=127 px
        }
    }

    /// The deterministic height (in logical pixels) of a row.
    ///
    /// A deterministic minority of rows are taller (e.g. wrapped/large text).
    pub fn row_height(&self, row: u32) -> f32 {
        let h = mix(self.seed ^ 0x0000_0DDE_ADBE_EF02_u64, row as u64);
        if h.is_multiple_of(20) {
            34.0 + (h % 20) as f32 // 34..=53 px
        } else {
            22.0 + (h % 6) as f32 // 22..=27 px
        }
    }

    /// The per-cell hash that drives all deterministic choices for `(row, col)`.
    fn cell_hash(&self, row: u32, col: u32) -> u64 {
        // Fold row and col into the seed with distinct constants so that
        // transposition (row<->col) does not collide.
        let a = mix(self.seed, (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        mix(a ^ 0xD1B5_4A32_D192_ED03, col as u64)
    }
}

impl CellSource for SyntheticSheet {
    fn cell(&self, row: u32, col: u32) -> CellData {
        let h = self.cell_hash(row, col);

        // Value: mostly numbers, some text, a few empty — a realistic mix.
        let value = match h % 10 {
            0 => CellValue::Empty,
            1..=4 => {
                // A number with varied magnitude and some decimals.
                let magnitude = (h >> 8) % 1_000_000;
                let cents = (h >> 4) % 100;
                CellValue::Number(magnitude as f64 + cents as f64 / 100.0)
            }
            _ => {
                // Text of varied length: 1 to 4 words joined.
                let word_count = 1 + ((h >> 12) % 4) as usize;
                let mut parts = Vec::with_capacity(word_count);
                for i in 0..word_count {
                    let idx = ((h >> (16 + i * 4)) as usize) % WORDS.len();
                    parts.push(WORDS[idx]);
                }
                CellValue::Text(parts.join(" "))
            }
        };

        // Formatting: scattered bold/italic, ~10–20% highlighted, varied alignment.
        let bold = (h >> 32).is_multiple_of(9); // ~11%
        let italic = (h >> 36).is_multiple_of(11); // ~9%
        let highlight = if (h >> 40) % 100 < 15 {
            // ~15% highlighted -> in the 10–20% band.
            Some(HIGHLIGHTS[((h >> 44) as usize) % HIGHLIGHTS.len()])
        } else {
            None
        };
        let h_align = match (h >> 48) % 4 {
            0 => HAlign::Center,
            1 => HAlign::Right,
            _ => HAlign::Left,
        };

        CellData {
            value,
            format: CellFormat {
                bold,
                italic,
                highlight,
                h_align,
            },
        }
    }
}

/// A fast, allocation-free `u64` mixing function (a splitmix64 finalizer applied to
/// `key + state`). Used instead of a stateful RNG so generation stays a pure
/// function of its inputs.
fn mix(state: u64, key: u64) -> u64 {
    let mut z = state.wrapping_add(key).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cells(sheet: &SyntheticSheet, rows: u32, cols: u32) -> Vec<CellData> {
        let mut out = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                out.push(sheet.cell(r, c));
            }
        }
        out
    }

    #[test]
    fn synthetic_is_deterministic() {
        let a = SyntheticSheet::new(42, 1000, 100);
        let b = SyntheticSheet::new(42, 1000, 100);
        for r in [0, 1, 7, 99, 512, 999] {
            for c in [0, 1, 3, 25, 26, 99] {
                assert_eq!(a.cell(r, c), b.cell(r, c), "cell ({r},{c}) not stable");
                // Repeated calls also stable.
                assert_eq!(a.cell(r, c), a.cell(r, c));
            }
        }
    }

    #[test]
    fn synthetic_seed_varies() {
        let a = SyntheticSheet::new(1, 200, 20);
        let b = SyntheticSheet::new(2, 200, 20);
        let sa = sample_cells(&a, 50, 20);
        let sb = sample_cells(&b, 50, 20);
        assert_ne!(sa, sb, "different seeds should differ over a sample");
    }

    #[test]
    fn synthetic_highlight_ratio_in_band() {
        let sheet = SyntheticSheet::new(7, 400, 40);
        let cells = sample_cells(&sheet, 300, 40); // 12,000 cells
        let highlighted = cells
            .iter()
            .filter(|c| c.format.highlight.is_some())
            .count();
        let frac = highlighted as f64 / cells.len() as f64;
        // Loose band around the 10–20% target so the test never flakes.
        assert!(
            (0.05..=0.30).contains(&frac),
            "highlight fraction {frac} outside expected band"
        );
    }

    #[test]
    fn synthetic_has_bold_italic_and_varied_text() {
        let sheet = SyntheticSheet::new(11, 500, 30);
        let cells = sample_cells(&sheet, 400, 30);
        assert!(cells.iter().any(|c| c.format.bold), "expected some bold");
        assert!(
            cells.iter().any(|c| c.format.italic),
            "expected some italic"
        );

        let text_lengths: Vec<usize> = cells
            .iter()
            .filter_map(|c| match &c.value {
                CellValue::Text(t) => Some(t.len()),
                _ => None,
            })
            .collect();
        assert!(!text_lengths.is_empty(), "expected some text cells");
        let min = *text_lengths.iter().min().unwrap();
        let max = *text_lengths.iter().max().unwrap();
        assert!(max > min, "expected varied text lengths");
        assert!(max >= 20, "expected at least one long/wide text value");
    }

    #[test]
    fn synthetic_has_empty_and_numeric_and_text() {
        let sheet = SyntheticSheet::new(3, 500, 30);
        let cells = sample_cells(&sheet, 300, 30);
        assert!(cells.iter().any(|c| matches!(c.value, CellValue::Empty)));
        assert!(
            cells
                .iter()
                .any(|c| matches!(c.value, CellValue::Number(_)))
        );
        assert!(cells.iter().any(|c| matches!(c.value, CellValue::Text(_))));
    }

    #[test]
    fn col_width_has_wide_columns_and_is_deterministic() {
        let sheet = SyntheticSheet::new(5, 100, 500);
        let widths: Vec<f32> = (0..500).map(|c| sheet.col_width(c)).collect();
        // Deterministic.
        for c in [0u32, 11, 12, 200, 499] {
            assert_eq!(sheet.col_width(c), sheet.col_width(c));
        }
        // At least one "very wide" column (> 250 px).
        assert!(
            widths.iter().any(|&w| w > 250.0),
            "expected at least one very wide column"
        );
        // All widths positive and finite.
        assert!(widths.iter().all(|&w| w.is_finite() && w > 0.0));
    }

    #[test]
    fn row_height_varies_and_is_deterministic() {
        let sheet = SyntheticSheet::new(9, 1000, 10);
        let heights: Vec<f32> = (0..1000).map(|r| sheet.row_height(r)).collect();
        for r in [0u32, 1, 20, 500, 999] {
            assert_eq!(sheet.row_height(r), sheet.row_height(r));
        }
        let min = heights.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = heights.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max > min, "expected varied row heights");
        assert!(heights.iter().all(|&h| h.is_finite() && h > 0.0));
    }
}
