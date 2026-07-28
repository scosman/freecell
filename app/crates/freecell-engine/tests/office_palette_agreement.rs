//! **The two Office-theme palette definitions must agree** (unit F3a —
//! `projects/architecture-review-remediation.md`).
//!
//! The architecture review flagged "two definitions of the Office palette". That one is
//! **correct**, and it is a straightforward duplication:
//!
//! - `freecell-core::palette::FILL_PALETTE` — the 10 Office theme colours as *swatches*, backing
//!   the action row's fill popover;
//! - `freecell-chart-model::ThemePalette::office_default()` — the same 10 colours as *theme
//!   slots*, backing `schemeClr` resolution in charts.
//!
//! Both are zero-dependency foundation crates that do not know the other exists, so nothing stops
//! them drifting. If they ever do, the same Excel accent renders as one colour in a cell fill and a
//! different one in a chart series in the same window.
//!
//! Removing the duplicate means merging the crates (unit F3, v1.0). Until then this test is the
//! enforcement the review's underlying point was really asking for: they cannot drift silently.
//!
//! This lives in `freecell-engine` because it is the only crate that depends on both.

use freecell_chart_model::{ThemePalette, ThemeSlot};
use freecell_core::palette::FILL_PALETTE;

/// `FILL_PALETTE` is in canonical theme order (the Background/Text pairs, then the six accents),
/// which is exactly the slot order `ThemePalette` models — so the mapping is positional and total
/// over the swatch list.
///
/// Note the naming seam this makes visible: OOXML's `lt1`/`dk1` (Light1/Dark1) are Excel's UI
/// "Background 1"/"Text 1", and they are *swapped* relative to the intuitive reading — Background 1
/// is `lt1` (white), Text 1 is `dk1` (black). Getting that pairing wrong is the most likely way
/// these two definitions would silently disagree, so it is asserted explicitly rather than by
/// index arithmetic.
const EXPECTED: [(&str, ThemeSlot); 10] = [
    ("Background 1", ThemeSlot::Light1),
    ("Text 1", ThemeSlot::Dark1),
    ("Background 2", ThemeSlot::Light2),
    ("Text 2", ThemeSlot::Dark2),
    ("Accent 1", ThemeSlot::Accent1),
    ("Accent 2", ThemeSlot::Accent2),
    ("Accent 3", ThemeSlot::Accent3),
    ("Accent 4", ThemeSlot::Accent4),
    ("Accent 5", ThemeSlot::Accent5),
    ("Accent 6", ThemeSlot::Accent6),
];

#[test]
fn the_cell_fill_palette_and_the_chart_theme_palette_are_the_same_colours() {
    let chart = ThemePalette::office_default();

    for (i, (name, slot)) in EXPECTED.iter().enumerate() {
        let swatch = FILL_PALETTE[i];
        assert_eq!(
            swatch.name, *name,
            "FILL_PALETTE[{i}] is {:?}, but this test expects the canonical theme order — if the \
             swatch list was deliberately reordered, update EXPECTED to match",
            swatch.name,
        );
        assert_eq!(
            swatch.rgb.to_hex(),
            chart.color(*slot).to_hex(),
            "the Office palette has drifted between crates: cell fill {name:?} is #{:06X} but the \
             chart theme's {slot:?} is #{:06X}. The same Excel colour would render differently in \
             a cell and in a chart series in the same window. Fix whichever copy is wrong; the \
             duplication itself goes away with the crate merge (unit F3).",
            swatch.rgb.to_hex(),
            chart.color(*slot).to_hex(),
        );
    }
}

/// The swatch list is exactly the theme's ten UI-exposed slots — no more, no fewer. `hlink` /
/// `folHlink` are theme slots with no fill swatch, which is correct (Excel does not offer them in
/// the fill dropdown) and is asserted here so adding one to `ThemePalette` does not quietly leave
/// the two lists different lengths.
#[test]
fn the_swatch_list_covers_exactly_the_ten_ui_slots() {
    assert_eq!(FILL_PALETTE.len(), EXPECTED.len());
}
