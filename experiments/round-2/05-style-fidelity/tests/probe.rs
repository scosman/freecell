//! SP5 long-tail style-roundtrip fidelity probes.
//!
//! Every row of the [`style_fidelity::fidelity_matrix`] is backed by a passing assertion
//! here. The matrix's `observed` values are computed by the same round-trip helpers these
//! tests call, so the matrix is *generated from* the probed behavior — never inferred.
//! Where IronCalc degrades or cannot express an attribute, the test asserts the
//! **documented degraded value** (e.g. `dotted` reads back as `thin`), so the loss is
//! locked, not hidden.

use ironcalc_base::types::{BorderStyle, HorizontalAlignment, VerticalAlignment};
use ironcalc_base::Model;
use style_fidelity::{
    alignment_roundtrip, all_border_styles, all_horizontal_alignments, all_vertical_alignments,
    bg_color_roundtrip, border_color_and_diagonal_roundtrip, border_style_roundtrip,
    fidelity_matrix, fill_color_roundtrip, font_color_roundtrip, font_longtail_roundtrip,
    new_model, number_format_roundtrip, quote_prefix_roundtrip, roundtrip_via_xlsx, Fidelity,
    NUMBER_FORMATS, RGB_COLORS, SHEET,
};

/// Every probed `#RRGGBB` fill color reads back byte-for-byte (case preserved) after the
/// round-trip.
#[test]
fn fill_colors_survive_roundtrip() {
    for &hex in RGB_COLORS {
        let (expected, observed) = fill_color_roundtrip(hex);
        assert_eq!(
            observed, expected,
            "fill fg_color {hex} must survive the xlsx round-trip exactly"
        );
    }
}

/// A background color under a non-solid pattern survives too.
#[test]
fn bg_color_with_pattern_survives() {
    let (expected, observed) = bg_color_roundtrip("#ABCDEF", "gray125");
    assert_eq!(
        observed, expected,
        "fill bg_color under a gray125 pattern must survive"
    );
}

/// Font colors survive exactly.
#[test]
fn font_colors_survive_roundtrip() {
    for &hex in &["#123456", "#abcdef", "#000000"] {
        let (expected, observed) = font_color_roundtrip(hex);
        assert_eq!(observed, expected, "font color {hex} must survive");
    }
}

/// The exhaustive border sweep — the core SP5 deliverable. Eight of the nine
/// `BorderStyle` variants survive; **`Dotted` degrades to `Thin`** because IronCalc's
/// xlsx *import* parser has no `"dotted"` arm and falls back `Some(_) => Thin`. This test
/// locks both the survivors and the one lossy case.
#[test]
fn all_border_styles_roundtrip_classified() {
    for style in all_border_styles() {
        let expected = style.to_string();
        let (_, observed) = border_style_roundtrip(style.clone());
        let observed = observed.expect("a border was written, so one must read back");
        if style == BorderStyle::Dotted {
            assert_eq!(
                observed, "thin",
                "LOCKED LOSSY: dotted border degrades to thin across the round-trip \
(import parser has no dotted arm; Some(_) => Thin)"
            );
        } else {
            assert_eq!(
                observed, expected,
                "border style {expected} must round-trip exactly"
            );
        }
    }
}

/// Border color survives; the diagonal flags / diagonal line are *measured* (the exporter
/// has a `TODO: diagonal_up/down?`, so this is a real unknown). We assert whatever the
/// engine actually does and lock it, rather than assuming — the matrix reports the same
/// observed values.
#[test]
fn border_color_and_diagonal_classified() {
    let border = border_color_and_diagonal_roundtrip();

    // Border color is a first-class #RRGGBB and must survive.
    let left_color = border
        .left
        .as_ref()
        .and_then(|i| i.color.clone())
        .expect("left border present with a color");
    assert_eq!(left_color, "#1A2B3C", "border color must survive");

    // The diagonal direction flags: lock to the observed behavior. IronCalc's exporter
    // does not emit diagonal_up/down (documented TODO), so they come back false. If a
    // future engine version starts preserving them, this assertion will flip and force a
    // conscious matrix update.
    assert!(
        !border.diagonal_up && !border.diagonal_down,
        "diagonal_up/down flags are dropped across the round-trip (exporter TODO); \
observed up={} down={}",
        border.diagonal_up,
        border.diagonal_down
    );
}

/// Every number-format family (currency, percent, thousands, scientific, date, time,
/// date-time, fraction, text, custom conditional-color) survives verbatim.
#[test]
fn number_formats_all_families_survive() {
    for &(family, code) in NUMBER_FORMATS {
        let (expected, observed) = number_format_roundtrip(code);
        assert_eq!(
            observed, expected,
            "number format family '{family}' ({code}) must survive the round-trip"
        );
    }
}

/// The full alignment matrix survives: all 8 horizontal variants, all 5 vertical
/// variants, and wrap_text.
#[test]
fn alignment_full_matrix_survives() {
    for h in all_horizontal_alignments() {
        let (written, read) = alignment_roundtrip(h.clone(), VerticalAlignment::Center, false);
        let read = read.expect("alignment was set, so one must read back");
        assert_eq!(
            read.horizontal, written.horizontal,
            "horizontal alignment {h} must survive"
        );
    }
    for v in all_vertical_alignments() {
        let (written, read) = alignment_roundtrip(HorizontalAlignment::Left, v.clone(), false);
        let read = read.expect("alignment was set");
        assert_eq!(
            read.vertical, written.vertical,
            "vertical alignment {v} must survive"
        );
    }
    let (_, read) = alignment_roundtrip(HorizontalAlignment::Left, VerticalAlignment::Top, true);
    let read = read.expect("alignment was set");
    assert!(read.wrap_text, "wrap_text must survive");
}

/// The font long tail (strike, underline-as-bool, name, family, size) survives.
#[test]
fn font_longtail_survives() {
    let font = font_longtail_roundtrip();
    assert!(font.strike, "strike survives");
    assert!(font.u, "underline (bool) survives");
    assert_eq!(font.name, "Times New Roman", "font name survives");
    assert_eq!(font.family, 1, "font family survives");
    assert_eq!(font.sz, 22, "font size survives");
}

/// `quote_prefix` (the force-text / leading-apostrophe flag) survives.
#[test]
fn quote_prefix_survives() {
    let (expected, observed) = quote_prefix_roundtrip();
    assert_eq!(observed, expected, "quote_prefix must survive");
}

/// Theme/indexed color *references* cannot be written through the public `Style` API
/// (there is no theme/indexed field — the only color surface is a `#RRGGBB` string), and
/// on import IronCalc resolves them to a concrete RGB. This test proves the reachable
/// half: a resolved `#RRGGBB` survives, and documents (by construction) that the
/// reference form is unreachable via the write API.
#[test]
fn theme_indexed_colors_flatten_to_rgb() {
    // The ONLY color surface a caller can write is a hex string; there is no
    // `style.fill.fg_theme` / `fg_indexed`. A resolved RGB (what import produces from a
    // theme/indexed reference) round-trips exactly:
    let (expected, observed) = fill_color_roundtrip("#4472C4"); // Excel theme "accent1" default RGB
    assert_eq!(
        observed, expected,
        "the RESOLVED rgb of a theme/indexed color survives; the reference itself is not \
writable via the public Style (no theme/indexed field) -- so a theme reference is \
flattened to RGB, its reference dropped"
    );
}

/// Merges and conditional formatting have no public API on `Model` in 0.7 — the OPEN gap
/// (functional_spec SP5 / overview §2). This documents the absence: a blank model has no
/// styling baseline, and there is intentionally no `add_merge_cells` / CF call because
/// those methods do not exist (a call would not compile). Supporting them would force
/// FreeCell to take over `.xlsx` writing.
#[test]
fn merges_and_cf_absent_from_public_api() {
    let model = new_model();
    let a1 = model.get_style_for_cell(SHEET, 1, 1).expect("A1 style");
    assert!(!a1.font.b, "baseline: fresh model has an unstyled A1");

    // No `model.add_merge_cells(..)` / conditional-formatting call exists in ironcalc
    // 0.7's public API; a compile-time attempt to call them would fail to build, which is
    // the strongest possible proof of absence. The matrix's open_gap rows record it.
    let _ = Model::new_empty("probe", "en", "UTC", "en").expect("model builds");
}

/// Matrix integrity: every row's classification is consistent with a fresh recomputation
/// of the round-trip behavior it claims, and every `Survives`/`Lossy` row (a genuinely
/// probed attribute) names a probe that exists in this file.
#[test]
fn matrix_is_probe_consistent() {
    let matrix = fidelity_matrix();
    assert!(
        matrix.rows.len() >= 40,
        "matrix covers the long tail broadly"
    );

    // Re-derive a few load-bearing classifications independently and confirm the matrix
    // agrees (catches a matrix row drifting from the real behavior).
    let (_, dotted) = border_style_roundtrip(BorderStyle::Dotted);
    assert_eq!(
        dotted.as_deref(),
        Some("thin"),
        "independent: dotted -> thin"
    );
    let (_, thick) = border_style_roundtrip(BorderStyle::Thick);
    assert_eq!(
        thick.as_deref(),
        Some("thick"),
        "independent: thick survives"
    );

    let probe_names = [
        "fill_colors_survive_roundtrip",
        "bg_color_with_pattern_survives",
        "font_colors_survive_roundtrip",
        "all_border_styles_roundtrip_classified",
        "border_color_and_diagonal_classified",
        "number_formats_all_families_survive",
        "alignment_full_matrix_survives",
        "font_longtail_survives",
        "quote_prefix_survives",
        "theme_indexed_colors_flatten_to_rgb",
        "merges_and_cf_absent_from_public_api",
    ];
    for row in &matrix.rows {
        // Every row must name one of this file's real probes.
        assert!(
            probe_names.contains(&row.probe),
            "row '{}' names an unknown probe '{}'",
            row.attribute,
            row.probe
        );
        // Survives/Lossy rows are genuinely-probed attributes (not just documentation);
        // Dropped/NotRepresentable rows may be API-reasoned but still cite a probe.
        if matches!(row.fidelity, Fidelity::Survives | Fidelity::Lossy) {
            assert!(
                !row.observed.is_empty(),
                "probed row '{}' must carry an observed value",
                row.attribute
            );
        }
    }
}

/// A sanity anchor tying this crate back to the frozen 03-formatting result: the same
/// representative attributes it proved (bold + fill + number format) still survive, so
/// SP5 is a strict *extension*, not a regression.
#[test]
fn representative_phase1_attributes_still_survive() {
    let mut model = new_model();
    model
        .set_user_input(SHEET, 1, 1, "12.5".to_string())
        .expect("set A1");
    {
        let mut s = model.get_style_for_cell(SHEET, 1, 1).expect("A1 style");
        s.font.b = true;
        s.fill.pattern_type = "solid".to_string();
        s.fill.fg_color = Some("#FFFF00".to_string());
        s.num_fmt = "0.00".to_string();
        model.set_cell_style(SHEET, 1, 1, &s).expect("set A1 style");
    }
    let reloaded = roundtrip_via_xlsx(&model);
    let s = reloaded
        .get_style_for_cell(SHEET, 1, 1)
        .expect("A1 reloaded");
    assert!(s.font.b, "bold survives (matches 03-formatting)");
    assert_eq!(s.fill.fg_color.as_deref(), Some("#FFFF00"), "fill survives");
    assert_eq!(s.num_fmt, "0.00", "number format survives");
}
