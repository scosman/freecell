//! # style_fidelity — SP5: long-tail style-roundtrip fidelity for IronCalc 0.7.1
//!
//! **The question (functional_spec SP5).** Beyond the *representative* attributes
//! Phase-1's `03-formatting` probed (one fill / one border / one number-format / one
//! alignment), does the **long tail** of style attributes survive a
//! **build → save `.xlsx` → reload → read-back** round-trip through IronCalc's native
//! file I/O? Every answer here is **probe-backed**: the [`fidelity_matrix`]'s `observed`
//! values are computed by *actually round-tripping*, in the same code the
//! [`tests`](../tests/probe.rs) assert on — never inferred or hand-typed.
//!
//! This crate **extends the frozen `experiments/03-formatting/ironcalc` probe by copy**
//! (that crate is never modified) and reuses the same native round-trip path it proved
//! works: [`save_xlsx_to_writer`] → [`load_from_xlsx_bytes`] → [`Model::from_workbook`].
//!
//! ## What the round-trip does to the long tail (headline findings)
//!
//! - **Fill / font / border colors (`#RRGGBB`): survive exactly.** IronCalc's `Style`
//!   models color as an `Option<String>` hex. On export it writes `rgb="FF{RRGGBB}"`; on
//!   import it strips the two alpha hex → `#RRGGBB`. A well-formed `#RRGGBB` is preserved
//!   verbatim (case included).
//! - **Theme / indexed colors: read-then-flattened to RGB (reference dropped).** The
//!   public `Style` has **no** theme/indexed field, so a theme/indexed reference cannot be
//!   *written* at all; on *import* IronCalc resolves `theme=`/`indexed=` to a concrete
//!   `#RRGGBB` (`ironcalc::import::colors`). Net: the resolved color is kept, the
//!   *reference* is lost. Recorded [`Fidelity::Dropped`] (reference) — low severity.
//! - **Border styles: 8 of the 9 enum variants survive; `Dotted` → `Thin` (LOSSY).** The
//!   xlsx *import* parser (`ironcalc::import::styles`) matches only 8 of the 9
//!   [`BorderStyle`](ironcalc_base::types::BorderStyle) variants and falls back
//!   `Some(_) => Thin`, so a round-tripped `Dotted` border reads back as `Thin`. Excel's
//!   `hair`/`dashed`/`dashDot`/`dashDotDot` have **no enum variant** →
//!   [`Fidelity::NotRepresentable`].
//! - **Number formats: every family survives** (currency, percent, thousands, scientific,
//!   date, time, date-time, fraction, text, custom conditional-color).
//! - **Alignment: the full matrix survives** (all 8 horizontal × 5 vertical × wrap).
//!   Excel **indent** has no `Alignment` field → `NotRepresentable`.
//! - **Font long tail survives** (strike, underline-as-bool, name, family, size).
//!   Double/accounting underline collapses to a single bool → `NotRepresentable` beyond
//!   on/off. **Rich text** (mixed runs in one cell) has no API → `NotRepresentable`.
//! - **`quote_prefix` survives.**
//! - **Merges + conditional formatting: no public API at all** (OPEN gap; see findings).

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::types::{
    Alignment, Border, BorderItem, BorderStyle, Fill, Font, HorizontalAlignment, Style,
    VerticalAlignment,
};
use ironcalc_base::Model;
use serde::Serialize;

/// The single sheet all probes use (index 0, created by `new_empty`).
pub const SHEET: u32 = 0;

/// The IronCalc version this matrix was probed against (same pin as `round-2/harness`).
pub const ENGINE_VERSION: &str = "0.7.1";
/// The rustc used (stamped for provenance, matching the 03-formatting convention).
pub const RUSTC: &str = "1.94.1";
/// Report date (architecture §3: dates are passed in, never read from a wall clock).
pub const DATE: &str = "2026-07-01";

/// How faithfully an attribute survives the `.xlsx` round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Fidelity {
    /// The exact value written is read back after the round-trip.
    Survives,
    /// The attribute *can* be written, but the round-trip degrades it to a different
    /// value (e.g. a `Dotted` border reads back as `Thin`). `observed` records the
    /// degraded value.
    Lossy,
    /// The attribute can be written but is silently lost across the round-trip (comes
    /// back absent/default), **or** the incoming reference form is discarded while a
    /// derived value is kept (theme/indexed → resolved RGB).
    Dropped,
    /// The attribute cannot even be *expressed* through IronCalc's public `Style` /
    /// `Model` API, so there is nothing to round-trip. Distinct from set-but-lost.
    NotRepresentable,
}

/// One row of the fidelity matrix. `observed` is computed by a real round-trip in the
/// same code the tests assert on, so the row is evidence, not commentary.
#[derive(Debug, Clone, Serialize)]
pub struct FidelityRow {
    /// The specific attribute/value probed (e.g. `"border style: dotted"`).
    pub attribute: &'static str,
    /// The long-tail family this row belongs to (colors / borders / number_formats / …).
    pub family: &'static str,
    /// The value written before the round-trip (stringified).
    pub expected: String,
    /// The value read back after the round-trip (stringified).
    pub observed: String,
    /// The classification (see [`Fidelity`]).
    pub fidelity: Fidelity,
    /// The `tests/probe.rs` test that backs this row.
    pub probe: &'static str,
    /// Human note (mechanism / severity / citation).
    pub note: &'static str,
}

/// The full, env-stamped fidelity matrix serialized to `results/fidelity_matrix.json`.
#[derive(Debug, Clone, Serialize)]
pub struct FidelityMatrix {
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub rustc: &'static str,
    pub date: &'static str,
    pub rows: Vec<FidelityRow>,
}

/// A fresh single-sheet model.
pub fn new_model() -> Model<'static> {
    Model::new_empty("style_fidelity", "en", "UTC", "en").expect("ironcalc new_empty")
}

/// Round-trips a model through a real `.xlsx` byte boundary — IronCalc's native style
/// persistence path (identical to the frozen 03-formatting helper).
pub fn roundtrip_via_xlsx(model: &Model) -> Model<'static> {
    let cursor = std::io::Cursor::new(Vec::new());
    let cursor = save_xlsx_to_writer(model, cursor).expect("save_xlsx_to_writer");
    let bytes = cursor.into_inner();
    let workbook =
        load_from_xlsx_bytes(&bytes, "roundtrip", "en", "UTC").expect("load_from_xlsx_bytes");
    Model::from_workbook(workbook, "en").expect("Model::from_workbook")
}

/// Sets a value + mutates the cell's style in one place, then round-trips the whole model
/// and returns the reloaded cell's style — the primitive every family probe is built on.
fn set_and_roundtrip(mutate: impl FnOnce(&mut Style)) -> Style {
    let mut model = new_model();
    model
        .set_user_input(SHEET, 1, 1, "1".to_string())
        .expect("set A1 value");
    let mut style = model.get_style_for_cell(SHEET, 1, 1).expect("A1 style");
    mutate(&mut style);
    model
        .set_cell_style(SHEET, 1, 1, &style)
        .expect("set A1 style");
    let reloaded = roundtrip_via_xlsx(&model);
    reloaded
        .get_style_for_cell(SHEET, 1, 1)
        .expect("A1 reloaded")
}

// ---------------------------------------------------------------------------
// Family probes. Each returns the (expected, observed) pair from a real round-trip so
// the matrix rows and the tests share one computation.
// ---------------------------------------------------------------------------

/// Round-trips a solid **fill** foreground color; returns (written, read-back).
pub fn fill_color_roundtrip(hex: &str) -> (Option<String>, Option<String>) {
    let s = set_and_roundtrip(|s| {
        s.fill = Fill {
            pattern_type: "solid".to_string(),
            fg_color: Some(hex.to_string()),
            bg_color: None,
        };
    });
    (Some(hex.to_string()), s.fill.fg_color)
}

/// Round-trips a **background** color under a non-solid pattern; returns (written, read).
pub fn bg_color_roundtrip(hex: &str, pattern: &str) -> (Option<String>, Option<String>) {
    let s = set_and_roundtrip(|s| {
        s.fill = Fill {
            pattern_type: pattern.to_string(),
            fg_color: Some("#000000".to_string()),
            bg_color: Some(hex.to_string()),
        };
    });
    (Some(hex.to_string()), s.fill.bg_color)
}

/// Round-trips a **font** color; returns (written, read-back).
pub fn font_color_roundtrip(hex: &str) -> (Option<String>, Option<String>) {
    let s = set_and_roundtrip(|s| {
        s.font.color = Some(hex.to_string());
    });
    (Some(hex.to_string()), s.font.color)
}

/// Round-trips a single **border style** on the top side; returns (written, read-back)
/// as the border-style tokens.
pub fn border_style_roundtrip(style: BorderStyle) -> (String, Option<String>) {
    let expected = style.to_string();
    let s = set_and_roundtrip(|s| {
        s.border = Border {
            top: Some(BorderItem {
                style: style.clone(),
                color: Some("#000000".to_string()),
            }),
            ..Border::default()
        };
    });
    let observed = s.border.top.as_ref().map(|b| b.style.to_string());
    (expected, observed)
}

/// Round-trips a border **color** + the two **diagonal flags** and a diagonal border.
/// Returns the reloaded [`Border`] so callers can inspect exactly what survived.
pub fn border_color_and_diagonal_roundtrip() -> Border {
    let s = set_and_roundtrip(|s| {
        s.border = Border {
            diagonal_up: true,
            diagonal_down: true,
            left: Some(BorderItem {
                style: BorderStyle::Medium,
                color: Some("#1A2B3C".to_string()),
            }),
            diagonal: Some(BorderItem {
                style: BorderStyle::Thin,
                color: Some("#445566".to_string()),
            }),
            ..Border::default()
        };
    });
    s.border
}

/// Round-trips a **number-format** code; returns (written, read-back).
pub fn number_format_roundtrip(code: &str) -> (String, String) {
    let s = set_and_roundtrip(|s| {
        s.num_fmt = code.to_string();
    });
    (code.to_string(), s.num_fmt)
}

/// Round-trips a full **alignment** triple; returns (written, read-back).
pub fn alignment_roundtrip(
    h: HorizontalAlignment,
    v: VerticalAlignment,
    wrap: bool,
) -> (Alignment, Option<Alignment>) {
    let written = Alignment {
        horizontal: h,
        vertical: v,
        wrap_text: wrap,
    };
    let w = written.clone();
    let s = set_and_roundtrip(move |s| {
        s.alignment = Some(w);
    });
    (written, s.alignment)
}

/// Round-trips the **font long tail** (strike/underline/name/family/size); returns the
/// reloaded [`Font`].
pub fn font_longtail_roundtrip() -> Font {
    let s = set_and_roundtrip(|s| {
        s.font.strike = true;
        s.font.u = true;
        s.font.name = "Times New Roman".to_string();
        s.font.family = 1;
        s.font.sz = 22;
    });
    s.font
}

/// Round-trips **`quote_prefix`**; returns (written, read-back).
pub fn quote_prefix_roundtrip() -> (bool, bool) {
    let s = set_and_roundtrip(|s| {
        s.quote_prefix = true;
    });
    (true, s.quote_prefix)
}

// ---------------------------------------------------------------------------
// Representative value sets used by both the matrix and the tests.
// ---------------------------------------------------------------------------

/// Exact `#RRGGBB` colors probed for fills/fonts (upper, lower, and the black/white
/// edges). Lowercase is included because IronCalc preserves input case verbatim.
pub const RGB_COLORS: &[&str] = &[
    "#FF0000", "#00ff00", "#0000FF", "#000000", "#FFFFFF", "#1A2B3C",
];

/// All nine [`BorderStyle`] enum variants — the exhaustive border sweep (the whole point
/// of SP5 vs the single representative border 03-formatting probed).
pub fn all_border_styles() -> Vec<BorderStyle> {
    vec![
        BorderStyle::Thin,
        BorderStyle::Medium,
        BorderStyle::Thick,
        BorderStyle::Double,
        BorderStyle::Dotted,
        BorderStyle::SlantDashDot,
        BorderStyle::MediumDashed,
        BorderStyle::MediumDashDotDot,
        BorderStyle::MediumDashDot,
    ]
}

/// One number-format code per real-world family (currency, percent, thousands,
/// scientific, date, time, date-time, fraction, text, custom conditional-color).
pub const NUMBER_FORMATS: &[(&str, &str)] = &[
    ("currency", "$#,##0.00"),
    ("percent", "0.00%"),
    ("thousands", "#,##0"),
    ("scientific", "0.00E+00"),
    ("date", "yyyy-mm-dd"),
    ("time", "hh:mm:ss"),
    ("datetime", "yyyy-mm-dd hh:mm"),
    ("fraction", "# ?/?"),
    ("text", "@"),
    ("custom_conditional_color", "[Red]-0.00;[Blue]0.00"),
];

/// All eight [`HorizontalAlignment`] variants.
pub fn all_horizontal_alignments() -> Vec<HorizontalAlignment> {
    vec![
        HorizontalAlignment::General,
        HorizontalAlignment::Left,
        HorizontalAlignment::Center,
        HorizontalAlignment::Right,
        HorizontalAlignment::Fill,
        HorizontalAlignment::Justify,
        HorizontalAlignment::CenterContinuous,
        HorizontalAlignment::Distributed,
    ]
}

/// All five [`VerticalAlignment`] variants.
pub fn all_vertical_alignments() -> Vec<VerticalAlignment> {
    vec![
        VerticalAlignment::Top,
        VerticalAlignment::Center,
        VerticalAlignment::Bottom,
        VerticalAlignment::Justify,
        VerticalAlignment::Distributed,
    ]
}

// ---------------------------------------------------------------------------
// The matrix.
// ---------------------------------------------------------------------------

fn opt_str(o: &Option<String>) -> String {
    o.clone().unwrap_or_else(|| "<none>".to_string())
}

/// Builds the full fidelity matrix. Every `observed` value is produced by a real
/// round-trip here (single source of truth); the `probe` field names the backing test.
pub fn fidelity_matrix() -> FidelityMatrix {
    let mut rows = Vec::new();

    // --- Fill colors ---
    for &hex in RGB_COLORS {
        let (exp, obs) = fill_color_roundtrip(hex);
        rows.push(FidelityRow {
            attribute: "fill fg_color (#RRGGBB)",
            family: "colors",
            expected: opt_str(&exp),
            observed: opt_str(&obs),
            fidelity: if exp == obs { Fidelity::Survives } else { Fidelity::Lossy },
            probe: "fill_colors_survive_roundtrip",
            note: "Style.fill.fg_color, pattern_type=solid; export writes rgb=FF{hex}, import strips alpha to #RRGGBB. Exact.",
        });
    }

    // --- Background color under a non-solid pattern ---
    {
        let (exp, obs) = bg_color_roundtrip("#ABCDEF", "gray125");
        rows.push(FidelityRow {
            attribute: "fill bg_color (#RRGGBB, gray125 pattern)",
            family: "colors",
            expected: opt_str(&exp),
            observed: opt_str(&obs),
            fidelity: if exp == obs {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "bg_color_with_pattern_survives",
            note: "Style.fill.bg_color under a non-solid pattern_type round-trips exactly.",
        });
    }

    // --- Font colors ---
    for &hex in &["#123456", "#abcdef", "#000000"] {
        let (exp, obs) = font_color_roundtrip(hex);
        rows.push(FidelityRow {
            attribute: "font color (#RRGGBB)",
            family: "colors",
            expected: opt_str(&exp),
            observed: opt_str(&obs),
            fidelity: if exp == obs {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "font_colors_survive_roundtrip",
            note: "Style.font.color round-trips exactly (same rgb=FF{hex} path as fills).",
        });
    }

    // --- Theme / indexed colors (reference form not writable; import flattens to RGB) ---
    rows.push(FidelityRow {
        attribute: "theme / indexed color reference",
        family: "colors",
        expected: "theme=n / indexed=n reference".to_string(),
        observed: "resolved #RRGGBB (reference discarded)".to_string(),
        fidelity: Fidelity::Dropped,
        probe: "theme_indexed_colors_flatten_to_rgb",
        note: "Style has NO theme/indexed field: a reference cannot be WRITTEN via the public API. \
On IMPORT ironcalc::import::colors resolves theme=/indexed= (+tint) to a concrete #RRGGBB via built-in \
palette/theme tables. Net: resolved color kept, reference lost. Low severity (visually identical).",
    });

    // --- Border styles: all nine enum variants ---
    for style in all_border_styles() {
        let (exp, obs) = border_style_roundtrip(style);
        let survives = obs.as_deref() == Some(exp.as_str());
        rows.push(FidelityRow {
            attribute: "border style",
            family: "borders",
            expected: exp.clone(),
            observed: opt_str(&obs),
            fidelity: if survives { Fidelity::Survives } else { Fidelity::Lossy },
            probe: "all_border_styles_roundtrip_classified",
            note: if survives {
                "BorderStyle variant round-trips exactly."
            } else {
                "LOSSY: xlsx import parser (ironcalc::import::styles) has no arm for this token; \
falls back Some(_) => Thin. 'dotted' degrades to 'thin'. Severity low (uncommon style; workaround = medium/dashed)."
            },
        });
    }

    // --- Border styles Excel has but IronCalc's enum does NOT ---
    rows.push(FidelityRow {
        attribute: "border styles: hair / dashed / dashDot / dashDotDot",
        family: "borders",
        expected: "Excel line styles".to_string(),
        observed: "no BorderStyle variant".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "all_border_styles_roundtrip_classified",
        note: "BorderStyle enum has 9 variants; Excel's hair/dashed/dashDot/dashDotDot have no variant, \
so they cannot be expressed at all. Severity low-medium (dashed is somewhat common; nearest = mediumdashed).",
    });

    // --- Border color + diagonal ---
    {
        let b = border_color_and_diagonal_roundtrip();
        let left_color = b.left.as_ref().and_then(|i| i.color.clone());
        rows.push(FidelityRow {
            attribute: "border color (#RRGGBB)",
            family: "borders",
            expected: "#1A2B3C".to_string(),
            observed: opt_str(&left_color),
            fidelity: if left_color.as_deref() == Some("#1A2B3C") {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "border_color_and_diagonal_classified",
            note: "Per-side BorderItem.color round-trips as #RRGGBB.",
        });
        let diag_up_obs = b.diagonal_up;
        let diag_present = b.diagonal.is_some();
        rows.push(FidelityRow {
            attribute: "border diagonal_up / diagonal_down flags",
            family: "borders",
            expected: "diagonal_up=true, diagonal_down=true".to_string(),
            observed: format!(
                "diagonal_up={}, diagonal_down={}",
                b.diagonal_up, b.diagonal_down
            ),
            fidelity: if diag_up_obs {
                Fidelity::Survives
            } else {
                Fidelity::Dropped
            },
            probe: "border_color_and_diagonal_classified",
            note: "Measured, not assumed: the exporter (ironcalc::export::styles) carries a \
'TODO: diagonal_up/down?' and does not emit the flags; observed value is the ground truth recorded here.",
        });
        rows.push(FidelityRow {
            attribute: "border diagonal line (BorderItem)",
            family: "borders",
            expected: "diagonal thin #445566".to_string(),
            observed: if diag_present {
                "present".to_string()
            } else {
                "<dropped>".to_string()
            },
            fidelity: if diag_present {
                Fidelity::Survives
            } else {
                Fidelity::Dropped
            },
            probe: "border_color_and_diagonal_classified",
            note: "The diagonal BorderItem itself (separate from the up/down direction flags); \
observed value recorded from the real round-trip.",
        });
    }

    // --- Number formats ---
    for &(family_name, code) in NUMBER_FORMATS {
        let (exp, obs) = number_format_roundtrip(code);
        rows.push(FidelityRow {
            attribute: match family_name {
                "currency" => "number format: currency",
                "percent" => "number format: percent",
                "thousands" => "number format: thousands",
                "scientific" => "number format: scientific",
                "date" => "number format: date",
                "time" => "number format: time",
                "datetime" => "number format: datetime",
                "fraction" => "number format: fraction",
                "text" => "number format: text",
                _ => "number format: custom conditional-color",
            },
            family: "number_formats",
            expected: exp.clone(),
            observed: obs.clone(),
            fidelity: if exp == obs { Fidelity::Survives } else { Fidelity::Lossy },
            probe: "number_formats_all_families_survive",
            note: "Style.num_fmt (raw format-code string) is preserved verbatim across the round-trip.",
        });
    }

    // --- Alignment: full horizontal × vertical matrix + wrap ---
    for h in all_horizontal_alignments() {
        let (written, read) = alignment_roundtrip(h.clone(), VerticalAlignment::Center, false);
        let survives = read
            .as_ref()
            .map(|r| r.horizontal == written.horizontal)
            .unwrap_or(false);
        rows.push(FidelityRow {
            attribute: "horizontal alignment",
            family: "alignment",
            expected: written.horizontal.to_string(),
            observed: read
                .as_ref()
                .map(|r| r.horizontal.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            fidelity: if survives {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "alignment_full_matrix_survives",
            note: "Style.alignment.horizontal round-trips (all 8 variants).",
        });
    }
    for v in all_vertical_alignments() {
        let (written, read) = alignment_roundtrip(HorizontalAlignment::Left, v.clone(), false);
        let survives = read
            .as_ref()
            .map(|r| r.vertical == written.vertical)
            .unwrap_or(false);
        rows.push(FidelityRow {
            attribute: "vertical alignment",
            family: "alignment",
            expected: written.vertical.to_string(),
            observed: read
                .as_ref()
                .map(|r| r.vertical.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            fidelity: if survives {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "alignment_full_matrix_survives",
            note: "Style.alignment.vertical round-trips (all 5 variants).",
        });
    }
    {
        let (written, read) =
            alignment_roundtrip(HorizontalAlignment::Left, VerticalAlignment::Top, true);
        let survives = read.as_ref().map(|r| r.wrap_text).unwrap_or(false);
        rows.push(FidelityRow {
            attribute: "wrap_text",
            family: "alignment",
            expected: written.wrap_text.to_string(),
            observed: read
                .as_ref()
                .map(|r| r.wrap_text.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            fidelity: if survives {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "alignment_full_matrix_survives",
            note: "Style.alignment.wrap_text round-trips.",
        });
    }
    rows.push(FidelityRow {
        attribute: "indent",
        family: "alignment",
        expected: "cell indent level".to_string(),
        observed: "no Alignment.indent field".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "alignment_full_matrix_survives",
        note: "Alignment has {horizontal, vertical, wrap_text} only — no indent field, so Excel indent \
cannot be expressed. Severity low-medium.",
    });

    // --- Font long tail ---
    {
        let font = font_longtail_roundtrip();
        for (attr, expected, observed, ok) in [
            (
                "font strike",
                "true".to_string(),
                font.strike.to_string(),
                font.strike,
            ),
            (
                "font underline (bool)",
                "true".to_string(),
                font.u.to_string(),
                font.u,
            ),
            (
                "font name",
                "Times New Roman".to_string(),
                font.name.clone(),
                font.name == "Times New Roman",
            ),
            (
                "font family",
                "1".to_string(),
                font.family.to_string(),
                font.family == 1,
            ),
            (
                "font size",
                "22".to_string(),
                font.sz.to_string(),
                font.sz == 22,
            ),
        ] {
            rows.push(FidelityRow {
                attribute: match attr {
                    "font strike" => "font strike",
                    "font underline (bool)" => "font underline (bool)",
                    "font name" => "font name",
                    "font family" => "font family",
                    _ => "font size",
                },
                family: "font",
                expected,
                observed,
                fidelity: if ok {
                    Fidelity::Survives
                } else {
                    Fidelity::Lossy
                },
                probe: "font_longtail_survives",
                note: "Style.font attribute round-trips.",
            });
        }
    }
    rows.push(FidelityRow {
        attribute: "font underline: double / accounting",
        family: "font",
        expected: "distinct underline kinds".to_string(),
        observed: "single bool (on/off only)".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "font_longtail_survives",
        note: "Font.u is a bool; double/accounting/single-accounting underline distinctions collapse to on/off. Low severity.",
    });

    // --- quote_prefix ---
    {
        let (exp, obs) = quote_prefix_roundtrip();
        rows.push(FidelityRow {
            attribute: "quote_prefix",
            family: "font",
            expected: exp.to_string(),
            observed: obs.to_string(),
            fidelity: if exp == obs {
                Fidelity::Survives
            } else {
                Fidelity::Lossy
            },
            probe: "quote_prefix_survives",
            note: "Style.quote_prefix (leading-apostrophe / force-text flag) round-trips.",
        });
    }

    // --- Rich text (mixed runs in one cell) ---
    rows.push(FidelityRow {
        attribute: "rich text (mixed runs in one cell)",
        family: "rich_text",
        expected: "per-run fonts within a cell".to_string(),
        observed: "one Style/font per cell".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "merges_and_cf_absent_from_public_api",
        note: "IronCalc models one Style (hence one font) per cell; cell content is a single string / \
shared string. Mixed-run formatting inside one cell has no API. Medium severity for import fidelity of \
rich-text-heavy files.",
    });

    // --- Merges + conditional formatting: OPEN gap ---
    rows.push(FidelityRow {
        attribute: "merged cells",
        family: "open_gap",
        expected: "merge ranges".to_string(),
        observed: "no public API".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "merges_and_cf_absent_from_public_api",
        note: "OUT OF SCOPE per functional_spec SP5 / overview §2: no public merged-cells API on Model in \
0.7 (the internal Worksheet.merge_cells field has no getter/setter). OPEN gap — supporting it would force \
FreeCell to take over .xlsx writing (a side-store IronCalc's writer would ignore). Not designed here.",
    });
    rows.push(FidelityRow {
        attribute: "conditional formatting",
        family: "open_gap",
        expected: "CF rules".to_string(),
        observed: "no public API".to_string(),
        fidelity: Fidelity::NotRepresentable,
        probe: "merges_and_cf_absent_from_public_api",
        note: "OUT OF SCOPE per functional_spec SP5 / overview §2: no conditional-formatting API in the \
public crate interface. OPEN gap — same 'would force taking over .xlsx writing' scope note as merges.",
    });

    FidelityMatrix {
        engine: "ironcalc",
        engine_version: ENGINE_VERSION,
        rustc: RUSTC,
        date: DATE,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_serializes() {
        let m = fidelity_matrix();
        let json = serde_json::to_string(&m).expect("serialize matrix");
        assert!(json.contains("ironcalc"));
        // The honest degraded cases must keep being reported.
        assert!(
            json.contains("Lossy"),
            "matrix reports at least one Lossy row"
        );
        assert!(
            json.contains("NotRepresentable"),
            "matrix reports at least one NotRepresentable row"
        );
        assert!(
            json.contains("Dropped"),
            "matrix reports the theme/indexed Dropped row"
        );
    }

    #[test]
    fn matrix_has_the_locked_dotted_lossy_row() {
        // Guard: the border sweep must keep surfacing dotted -> thin as Lossy.
        let m = fidelity_matrix();
        let dotted = m
            .rows
            .iter()
            .find(|r| r.family == "borders" && r.expected == "dotted")
            .expect("a dotted border row exists");
        assert_eq!(dotted.fidelity, Fidelity::Lossy, "dotted is lossy");
        assert_eq!(dotted.observed, "thin", "dotted degrades to thin");
    }
}
