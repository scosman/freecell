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
/// the fill dropdown), and every *other* slot must appear in `EXPECTED`.
///
/// The classification is an **exhaustive `match`** rather than a length comparison on purpose: the
/// previous version asserted `FILL_PALETTE.len() == EXPECTED.len()`, and both sides are the
/// compile-time constant 10, so it was a tautology that could never fail. Adding a variant to
/// `ThemeSlot` now fails to **compile** here until someone decides, explicitly, whether it is a
/// fill swatch or not.
///
/// The slot list it is swept over is `ThemeSlot::ALL`, **owned by the type's own crate**. It was a
/// hand-written array in this file, which made the sweep exhaustive only over what someone
/// remembered to type here — a new variant could be classified by the `match` above and then never
/// swept, because the local list had not grown.
#[test]
fn every_theme_slot_is_either_a_fill_swatch_or_a_declared_non_swatch() {
    /// Exhaustive over `ThemeSlot` — no wildcard arm, so a new variant is a compile error.
    fn is_ui_fill_swatch(slot: ThemeSlot) -> bool {
        match slot {
            ThemeSlot::Light1
            | ThemeSlot::Dark1
            | ThemeSlot::Light2
            | ThemeSlot::Dark2
            | ThemeSlot::Accent1
            | ThemeSlot::Accent2
            | ThemeSlot::Accent3
            | ThemeSlot::Accent4
            | ThemeSlot::Accent5
            | ThemeSlot::Accent6 => true,
            // Theme colours with no fill swatch: Excel does not offer them in the fill dropdown.
            ThemeSlot::Hyperlink | ThemeSlot::FollowedHyperlink => false,
        }
    }

    // Every slot the match calls a swatch is mapped by EXPECTED, and vice versa.
    for (_, slot) in EXPECTED {
        assert!(
            is_ui_fill_swatch(slot),
            "{slot:?} is mapped to a fill swatch by EXPECTED but the exhaustive classification \
             says it has none",
        );
    }
    // `ThemeSlot::ALL` must itself be complete for this sweep to mean anything: no repeats, and one
    // entry per slot the classification knows about.
    for (i, slot) in ThemeSlot::ALL.iter().enumerate() {
        assert!(
            !ThemeSlot::ALL[..i].contains(slot),
            "{slot:?} appears twice in ThemeSlot::ALL — the sweep below would then skip whichever \
             slot was displaced",
        );
    }
    for slot in ThemeSlot::ALL {
        assert_eq!(
            is_ui_fill_swatch(slot),
            EXPECTED.iter().any(|(_, mapped)| *mapped == slot),
            "{slot:?} is classified as a fill swatch = {} but EXPECTED disagrees — a theme slot \
             gained or lost its swatch and the two crates' lists are now different lengths",
            is_ui_fill_swatch(slot),
        );
    }
    assert_eq!(
        FILL_PALETTE.len(),
        EXPECTED.len(),
        "FILL_PALETTE and the slot mapping have different lengths",
    );
}
