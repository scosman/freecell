//! Engine-neutral cell model: addresses, values, formatting, and the
//! [`CellSource`] provider trait.
//!
//! This is a deliberately small proxy model for "a big, difficult spreadsheet".
//! It is not tied to any spreadsheet engine — later sub-projects map it onto
//! whatever engine the Sub-project A gate selects.

/// Excel's maximum row count (`1,048,576`). See functional_spec §5.4.
pub const EXCEL_MAX_ROWS: u32 = 1_048_576;

/// Excel's maximum column count (`16,384`, i.e. column `XFD`). See functional_spec §5.4.
pub const EXCEL_MAX_COLS: u32 = 16_384;

/// A zero-based `(row, col)` cell coordinate.
///
/// Rows and columns are zero-based internally; [`CellAddress::a1`] converts to the
/// one-based, letter-column Excel "A1" notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellAddress {
    /// Zero-based row index (`0` == spreadsheet row `1`).
    pub row: u32,
    /// Zero-based column index (`0` == column `A`).
    pub col: u32,
}

impl CellAddress {
    /// Creates a new zero-based cell address.
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// Renders this address in Excel "A1" notation, e.g. `(0, 0) -> "A1"`,
    /// `(0, 26) -> "AA1"`, `(0, 16383) -> "XFD1"`.
    pub fn a1(&self) -> String {
        let mut label = column_label(self.col);
        // Excel rows are one-based.
        label.push_str(&(self.row + 1).to_string());
        label
    }
}

/// Converts a zero-based column index to its Excel letter label
/// (`0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`, `16383 -> "XFD"`).
///
/// This is bijective base-26 ("bijective hexavigesimal"): there is no zero digit,
/// so `26` maps to `AA` rather than `BA`.
pub fn column_label(col: u32) -> String {
    let mut n = col as u64 + 1; // shift to one-based for bijective base-26
    let mut bytes = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        bytes.push(b'A' + rem);
        n = (n - 1) / 26;
    }
    bytes.reverse();
    // Bytes are all ASCII 'A'..='Z', so this is always valid UTF-8.
    String::from_utf8(bytes).expect("column label is ASCII")
}

/// A concrete cell value. `Empty` is a distinct state from `Text("")`.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// No value.
    Empty,
    /// A numeric value.
    Number(f64),
    /// A text value.
    Text(String),
}

/// Horizontal text alignment for a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// A 24-bit RGB colour used for cell fills / highlights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// The formatting attributes a PoC needs to exercise a realistic render path:
/// bold, italic, an optional fill/highlight, and horizontal alignment
/// (architecture §7).
#[derive(Debug, Clone, PartialEq)]
pub struct CellFormat {
    pub bold: bool,
    pub italic: bool,
    /// Fill / highlight colour, if any. Around 10–20% of synthetic cells are
    /// highlighted (functional_spec §5.4 look, architecture §7).
    pub highlight: Option<Rgb>,
    pub h_align: HAlign,
}

impl Default for CellFormat {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            highlight: None,
            h_align: HAlign::Left,
        }
    }
}

/// A cell's value plus its formatting — everything the UI PoC needs to draw it.
#[derive(Debug, Clone, PartialEq)]
pub struct CellData {
    pub value: CellValue,
    pub format: CellFormat,
}

impl CellData {
    /// A convenience constructor for an unformatted value.
    pub fn plain(value: CellValue) -> Self {
        Self {
            value,
            format: CellFormat::default(),
        }
    }
}

/// A read-only provider of per-cell data — the "static datamodel provider" the UI
/// proof-of-concept renders against (architecture §7). Implementations must be
/// cheap and side-effect-free so the render loop can call [`CellSource::cell`]
/// freely as the viewport scrolls.
pub trait CellSource {
    /// Returns the [`CellData`] at a zero-based `(row, col)`.
    fn cell(&self, row: u32, col: u32) -> CellData;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_address_a1_mapping() {
        assert_eq!(CellAddress::new(0, 0).a1(), "A1");
        assert_eq!(CellAddress::new(0, 25).a1(), "Z1");
        assert_eq!(CellAddress::new(0, 26).a1(), "AA1");
        assert_eq!(CellAddress::new(0, 16_383).a1(), "XFD1");
        // Row offset is one-based.
        assert_eq!(CellAddress::new(1, 0).a1(), "A2");
        assert_eq!(CellAddress::new(9, 27).a1(), "AB10");
    }

    #[test]
    fn column_label_boundaries() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(51), "AZ");
        assert_eq!(column_label(52), "BA");
        assert_eq!(column_label(701), "ZZ");
        assert_eq!(column_label(702), "AAA");
    }

    #[test]
    fn excel_max_constants() {
        assert_eq!(EXCEL_MAX_ROWS, 1_048_576);
        assert_eq!(EXCEL_MAX_COLS, 16_384);
        // XFD is the last valid Excel column.
        assert_eq!(column_label(EXCEL_MAX_COLS - 1), "XFD");
    }
}
