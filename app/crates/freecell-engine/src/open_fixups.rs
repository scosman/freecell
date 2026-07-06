//! `open_fixups` — corrections applied to an IronCalc [`Model`] right after `load_from_xlsx`,
//! before it is wrapped in a `UserModel`. IronCalc 0.7.1 imports three things wrong for real
//! Excel/Numbers files, and all three are correctable from the original `.xlsx` bytes:
//!
//! 1. **Theme colours resolved against the wrong palette.** IronCalc resolves every
//!    theme-indexed colour (`<fgColor theme="3" tint="…"/>`, `<color theme="1"/>`) against a
//!    *hardcoded default Office palette* (`ironcalc::import::colors::get_themed_color`) and
//!    discards the theme index + tint, storing only the (wrong) resolved RGB. It never parses
//!    the workbook's own `xl/theme/theme1.xml`. A file with a custom theme (e.g. one whose
//!    `dk1`/`lt1` are swapped, or whose `dk2` is purple rather than the default navy) renders
//!    with entirely wrong fills and font colours. We re-read `theme1.xml` + `styles.xml`,
//!    recompute each theme-indexed fill/font colour against the *file's* palette (applying the
//!    OOXML §18.8.3 tint), and overwrite the resolved RGB in the style tables.
//!
//! 2. **Custom indexed-colour palette ignored.** IronCalc resolves every `indexed="n"`
//!    colour (`<fgColor indexed="9"/>`, `<color indexed="14"/>`) against a *hardcoded legacy
//!    indexed palette* (`ironcalc::import::colors::get_indexed_color`) and never reads the
//!    workbook's own `<colors><indexedColors>` override (OOXML §18.8.27). A file that
//!    redefines palette entries (as Numbers exports do) renders those `indexed=` fills/fonts
//!    with Excel's *default* colour n instead of the file's colour n. We re-read
//!    `<indexedColors>` from `styles.xml` and, **only when the file supplies that override**,
//!    overwrite each `indexed=`-derived fill/font RGB with the file's palette entry. A file
//!    with no override is left entirely to IronCalc, so standard-palette files never change.
//!
//! 3. **Built-in `numFmtId`s mapped to garbage codes.** IronCalc's `DEFAULT_NUM_FMTS` table
//!    (`ironcalc_base::number_format`) is wrong for many standard built-in ids — e.g. id 39
//!    (Excel's `#,##0.00_);(#,##0.00)`) maps to `"t0.00"`, which its own formatter then can't
//!    parse and returns `#VALUE!` for. The cell's *value* is correct; only the display format
//!    is broken. IronCalc's formatter handles the correct code fine, so we inject the correct
//!    standard built-in code (only for ids the workbook references but doesn't define itself),
//!    which `get_num_fmt` picks up ahead of its broken default table.
//!
//! All corrections are **best-effort**: any parse/read failure leaves the model as IronCalc
//! imported it (never fails the open), and only entries that actually used a theme / an indexed
//! colour with a file override / a broken built-in id are touched — explicit `rgb=` colours,
//! `auto=` (system) colours, and file-defined formats are left exactly as IronCalc parsed them.

use std::path::Path;

use freecell_core::Rgb;
use ironcalc_base::types::NumFmt;
use ironcalc_base::Model;

/// Applies the open-time OOXML fix-ups to a freshly loaded model. See the module docs.
pub(crate) fn apply_open_fixups(model: &mut Model, path: &Path) {
    // Number-format correction needs only the model (the referenced ids live in the style
    // tables), so it always runs. Colour corrections need the original `.xlsx` bytes. Theme
    // and indexed colours are disjoint (a colour node uses exactly one of `theme=`/`indexed=`/
    // `rgb=`/`auto=`), so the order between the two colour passes does not matter.
    inject_builtin_num_fmts(model);
    correct_theme_colors(model, path);
    correct_indexed_colors(model, path);
}

// ---------------------------------------------------------------------------------------------
// Theme colour correction
// ---------------------------------------------------------------------------------------------

/// The number of colours in an OOXML `clrScheme` we index by the `theme="…"` attribute.
const THEME_SLOTS: usize = 12;

/// Re-reads the file's theme + styles and overwrites every theme-indexed fill/font colour with
/// the correctly resolved RGB. Best-effort: returns silently on any read/parse failure.
fn correct_theme_colors(model: &mut Model, path: &Path) {
    let Some(palette) = read_theme_palette(path) else {
        return;
    };
    let Some(styles_xml) = read_zip_entry(path, "xl/styles.xml") else {
        return;
    };
    let Ok(doc) = roxmltree::Document::parse(&styles_xml) else {
        return;
    };
    let root = doc.root_element();

    // Fills: the i-th `<fill>` in `<fills>` is `styles.fills[i]` (IronCalc pushes one entry per
    // child, in document order — we enumerate identically so the indices line up). Only a
    // solid fill's `<fgColor>`/`<bgColor>` that used `theme=` is rewritten.
    if let Some(fills) = root.children().find(|n| n.has_tag_name("fills")) {
        for (i, fill) in fills.children().enumerate() {
            if i >= model.workbook.styles.fills.len() {
                break;
            }
            let Some(pattern) = fill.children().find(|n| n.has_tag_name("patternFill")) else {
                continue;
            };
            for color in pattern.children() {
                let is_fg = color.has_tag_name("fgColor");
                let is_bg = color.has_tag_name("bgColor");
                if !(is_fg || is_bg) {
                    continue;
                }
                if let Some(rgb) = themed_rgb(&color, &palette) {
                    let hex = to_hex(rgb);
                    if is_fg {
                        model.workbook.styles.fills[i].fg_color = Some(hex);
                    } else {
                        model.workbook.styles.fills[i].bg_color = Some(hex);
                    }
                }
            }
        }
    }

    // Fonts: the i-th `<font>` in `<fonts>` is `styles.fonts[i]`. Only a `<color theme="…"/>`
    // is rewritten.
    if let Some(fonts) = root.children().find(|n| n.has_tag_name("fonts")) {
        for (i, font) in fonts.children().enumerate() {
            if i >= model.workbook.styles.fonts.len() {
                break;
            }
            if let Some(color) = font.children().find(|n| n.has_tag_name("color")) {
                if let Some(rgb) = themed_rgb(&color, &palette) {
                    model.workbook.styles.fonts[i].color = Some(to_hex(rgb));
                }
            }
        }
    }

    // Borders: each edge's `<color theme="…"/>` is rewritten the same way, via the shared walk.
    // FONTS.xlsx's borders are *indexed*, not themed, but themed borders exist in other files —
    // keeping the two passes symmetric (both fix fills, fonts, *and* borders) mirrors exactly how
    // they already treat fills and fonts. `root` borrows the local `doc`, separate from `model`.
    correct_border_colors(model, root, |n| themed_rgb(n, &palette));
}

/// Resolves a colour node against the file palette **iff** it used `theme=`; returns `None` for
/// `rgb=`/`indexed=`/`auto` (which IronCalc already parsed correctly and we must not disturb).
fn themed_rgb(node: &roxmltree::Node, palette: &[Option<Rgb>; THEME_SLOTS]) -> Option<Rgb> {
    let theme_idx: usize = node.attribute("theme")?.parse().ok()?;
    let base = *palette.get(theme_idx)?;
    let base = base?;
    // Reject a non-finite tint (`NaN`/`inf` from hostile bytes) — fall back to no tint (the base
    // colour) rather than let it flow into the HSL maths. Best-effort, never fails the open.
    let tint = node
        .attribute("tint")
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|t| t.is_finite())
        .unwrap_or(0.0);
    Some(apply_tint(base, tint))
}

/// Reads `xl/theme/theme1.xml` and returns the palette indexed by the OOXML `theme="…"`
/// attribute. The `clrScheme` lists colours as `dk1, lt1, dk2, lt2, accent1..6, hlink,
/// folHlink`, but the style index applies the well-known dark/light swap for the first two
/// pairs, so index 0 → `lt1`, 1 → `dk1`, 2 → `lt2`, 3 → `dk2`, then accents/links in order.
fn read_theme_palette(path: &Path) -> Option<[Option<Rgb>; THEME_SLOTS]> {
    let xml = read_zip_entry(path, "xl/theme/theme1.xml")?;
    let doc = roxmltree::Document::parse(&xml).ok()?;
    let scheme = doc.descendants().find(|n| n.has_tag_name("clrScheme"))?;

    let slot = |name: &str| -> Option<Rgb> {
        let node = scheme.children().find(|n| n.has_tag_name(name))?;
        let color = node.children().find(|n| n.is_element())?;
        if color.has_tag_name("srgbClr") {
            color.attribute("val").and_then(parse_hex6)
        } else if color.has_tag_name("sysClr") {
            // A system colour (e.g. windowText) carries the concrete RGB in `lastClr`.
            color.attribute("lastClr").and_then(parse_hex6)
        } else {
            None
        }
    };

    Some([
        slot("lt1"), // theme index 0  (dark/light swap: index 0 is lt1, not dk1)
        slot("dk1"), // theme index 1
        slot("lt2"), // theme index 2
        slot("dk2"), // theme index 3
        slot("accent1"),
        slot("accent2"),
        slot("accent3"),
        slot("accent4"),
        slot("accent5"),
        slot("accent6"),
        slot("hlink"),
        slot("folHlink"),
    ])
}

// ---------------------------------------------------------------------------------------------
// Indexed-colour correction (OOXML §18.8.27 `<colors><indexedColors>`)
// ---------------------------------------------------------------------------------------------

/// Re-reads the file's `<colors><indexedColors>` override and overwrites every `indexed=`-derived
/// fill/font colour with the file's palette entry. **Only runs when the file actually supplies an
/// override** — a standard-palette file (no `<indexedColors>`) is left entirely to IronCalc, so it
/// can never regress. Best-effort: returns silently on any read/parse failure.
fn correct_indexed_colors(model: &mut Model, path: &Path) {
    let Some(styles_xml) = read_zip_entry(path, "xl/styles.xml") else {
        return;
    };
    let Ok(doc) = roxmltree::Document::parse(&styles_xml) else {
        return;
    };
    let root = doc.root_element();

    // The key safety gate: without a `<colors><indexedColors>` override there is nothing to
    // correct, and touching nothing guarantees standard-palette files are byte-for-byte
    // unaffected. Bail before walking any style entry.
    let Some(palette) = read_indexed_palette(root) else {
        return;
    };

    // Fills: the i-th `<fill>` in `<fills>` is `styles.fills[i]` (IronCalc pushes one entry per
    // child, in document order — we enumerate identically so the indices line up, exactly as the
    // theme pass does). Only a solid fill's `<fgColor>`/`<bgColor>` that used `indexed=` (and not
    // an explicit `rgb=`) is rewritten.
    if let Some(fills) = root.children().find(|n| n.has_tag_name("fills")) {
        for (i, fill) in fills.children().enumerate() {
            if i >= model.workbook.styles.fills.len() {
                break;
            }
            let Some(pattern) = fill.children().find(|n| n.has_tag_name("patternFill")) else {
                continue;
            };
            for color in pattern.children() {
                let is_fg = color.has_tag_name("fgColor");
                let is_bg = color.has_tag_name("bgColor");
                if !(is_fg || is_bg) {
                    continue;
                }
                if let Some(rgb) = indexed_rgb(&color, &palette) {
                    let hex = to_hex(rgb);
                    if is_fg {
                        model.workbook.styles.fills[i].fg_color = Some(hex);
                    } else {
                        model.workbook.styles.fills[i].bg_color = Some(hex);
                    }
                }
            }
        }
    }

    // Fonts: the i-th `<font>` in `<fonts>` is `styles.fonts[i]`. Only a `<color indexed="…"/>`
    // is rewritten.
    if let Some(fonts) = root.children().find(|n| n.has_tag_name("fonts")) {
        for (i, font) in fonts.children().enumerate() {
            if i >= model.workbook.styles.fonts.len() {
                break;
            }
            if let Some(color) = font.children().find(|n| n.has_tag_name("color")) {
                if let Some(rgb) = indexed_rgb(&color, &palette) {
                    model.workbook.styles.fonts[i].color = Some(to_hex(rgb));
                }
            }
        }
    }

    // Borders: each edge's `<color indexed="…"/>` is rewritten the same way, via the shared walk.
    // This is the FONTS.xlsx bug — a border edge referencing `indexed="10"`/`"11"` was rendering
    // as IronCalc's legacy default (red/green) instead of the file's grey override. `read_indexed_palette`
    // already gated the whole pass on a `<colors><indexedColors>` override, so this only runs when
    // the file supplies one. `root` borrows the local `doc`, separate from `model`.
    correct_border_colors(model, root, |n| indexed_rgb(n, &palette));
}

/// Resolves a colour node against the file's indexed palette **iff** it used `indexed=` (and not an
/// explicit `rgb=`, which takes precedence in IronCalc's `get_color`). Returns `None` — leaving
/// IronCalc's value in place — for `rgb=`/`theme=`/`auto=`, for the conventional system indices
/// 64/65 (auto foreground/background, not a palette lookup), for an index past the end of the
/// override list, and for a malformed override entry.
fn indexed_rgb(node: &roxmltree::Node, palette: &[Option<Rgb>]) -> Option<Rgb> {
    // `rgb=` wins over `indexed=` in IronCalc's `get_color`, so an explicit-RGB node was never
    // resolved from the palette — don't disturb it.
    if node.has_attribute("rgb") {
        return None;
    }
    let idx: usize = node.attribute("indexed")?.parse().ok()?;
    // Indices 64/65 are Excel's "system foreground/background" (automatic), not palette entries;
    // IronCalc/Excel resolve them from the system colours, so never map them to an override slot.
    if idx >= 64 {
        return None;
    }
    // Out of range → `None` (leave IronCalc's value); a malformed override entry is stored as
    // `None` and likewise leaves IronCalc's value.
    *palette.get(idx)?
}

/// Reads `xl/styles.xml`'s `<colors><indexedColors>` override into a palette indexed by position
/// (the OOXML `indexed="…"` value). Each entry is `<rgbColor rgb="AARRGGBB"/>`; the leading alpha
/// byte is dropped. Returns `None` when there is no override (or it is empty) — the signal to the
/// caller that nothing should be touched. A malformed `rgb=` becomes a `None` slot (that index is
/// left to IronCalc), never a panic.
fn read_indexed_palette(root: roxmltree::Node) -> Option<Vec<Option<Rgb>>> {
    let colors = root.children().find(|n| n.has_tag_name("colors"))?;
    let indexed = colors
        .children()
        .find(|n| n.has_tag_name("indexedColors"))?;
    let palette: Vec<Option<Rgb>> = indexed
        .children()
        .filter(|n| n.has_tag_name("rgbColor"))
        .map(|n| n.attribute("rgb").and_then(parse_indexed_rgb))
        .collect();
    if palette.is_empty() {
        return None;
    }
    Some(palette)
}

/// Parses an `<rgbColor rgb="…"/>` value: `AARRGGBB` (drop the alpha byte) or a bare `RRGGBB`.
/// Any other shape / non-hex → `None`.
fn parse_indexed_rgb(s: &str) -> Option<Rgb> {
    // Check all-hex first so the `[2..8]` slice below always lands on a char boundary (a
    // multibyte-garbage string can never panic).
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match s.len() {
        8 => parse_hex6(&s[2..8]), // AARRGGBB → keep RRGGBB
        6 => parse_hex6(s),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Border colour correction (shared by the theme + indexed passes)
// ---------------------------------------------------------------------------------------------

/// The five border edges, in the `<border>` child order the shared walk visits them.
const BORDER_SIDES: [&str; 5] = ["left", "right", "top", "bottom", "diagonal"];

/// Walks `<borders>` and rewrites each edge's stored colour via `resolve` — the same per-node
/// resolver the fill/font passes use (`themed_rgb` / `indexed_rgb`), so a node the resolver
/// declines (`rgb=`/`auto=`/out-of-range/wrong colour-kind) is left untouched. The i-th
/// `<border>` child is `styles.borders[i]` (IronCalc pushes one entry per child in document
/// order — identical to the fills/fonts alignment the other passes rely on). An edge whose model
/// `BorderItem` is `None` (no border on that side) is skipped — we never fabricate an edge.
///
/// Kept generic over `resolve` so both passes reuse one walk: `resolve` closes over the pass's
/// palette (separate from `model`), and `root` borrows the caller's local `doc` (also separate),
/// so there is no aliasing with the `&mut model` the writes go through.
fn correct_border_colors(
    model: &mut Model,
    root: roxmltree::Node,
    resolve: impl Fn(&roxmltree::Node) -> Option<Rgb>,
) {
    let Some(borders) = root.children().find(|n| n.has_tag_name("borders")) else {
        return;
    };
    for (i, border) in borders.children().enumerate() {
        if i >= model.workbook.styles.borders.len() {
            break;
        }
        for side in BORDER_SIDES {
            // The side node (`<left>`/`<top>`/…) and its `<color>` child; a side with no element
            // (an unset edge, e.g. `<diagonal/>`) or no `<color>` is left for IronCalc.
            let Some(side_node) = border.children().find(|n| n.has_tag_name(side)) else {
                continue;
            };
            let Some(color) = side_node.children().find(|n| n.has_tag_name("color")) else {
                continue;
            };
            let Some(rgb) = resolve(&color) else {
                continue;
            };
            // Only overwrite an edge IronCalc actually imported — a `None` edge (no border on that
            // side) stays `None`, never fabricated from the XML.
            let edge = match side {
                "left" => &mut model.workbook.styles.borders[i].left,
                "right" => &mut model.workbook.styles.borders[i].right,
                "top" => &mut model.workbook.styles.borders[i].top,
                "bottom" => &mut model.workbook.styles.borders[i].bottom,
                _ => &mut model.workbook.styles.borders[i].diagonal,
            };
            if let Some(item) = edge {
                item.color = Some(to_hex(rgb));
            }
        }
    }
}

/// Reads one entry from the `.xlsx` (a Zip archive) into a string. `None` on any I/O / missing
/// entry / non-UTF-8 error — the caller then leaves the model untouched.
fn read_zip_entry(path: &Path, name: &str) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(name).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Parses a 6-hex-digit `RRGGBB` (as theme `srgbClr@val` / `sysClr@lastClr` store it).
fn parse_hex6(s: &str) -> Option<Rgb> {
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Rgb::from_hex(v))
}

/// Formats an `Rgb` as the `#RRGGBB` string IronCalc stores in its style tables.
fn to_hex(rgb: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b)
}

// ---------------------------------------------------------------------------------------------
// OOXML §18.8.3 tint algorithm (applied on HSL luminance)
//
// Ported to match Excel's behaviour (and IronCalc's own `hex_with_tint_to_rgb`, which is
// verified against Excel's outputs). HSL luminance is on a 0..=100 integer scale; a negative
// tint darkens (`L' = L·(1+tint)`), a positive tint lightens (`L' = L + (100−L)·tint`).
// ---------------------------------------------------------------------------------------------

/// Applies an OOXML tint to a base colour on its HSL luminance and returns the result.
fn apply_tint(rgb: Rgb, tint: f64) -> Rgb {
    if tint == 0.0 {
        return rgb;
    }
    let [h, s, mut l] = rgb_to_hsl([rgb.r as i32, rgb.g as i32, rgb.b as i32]);
    let lf = l as f64;
    l = if tint < 0.0 {
        (lf * (1.0 + tint)).round() as i32
    } else {
        (lf + (100.0 - lf) * tint).round() as i32
    }
    .clamp(0, 100);
    let [r, g, b] = hsl_to_rgb([h, s, l]);
    Rgb::new(r as u8, g as u8, b as u8)
}

/// RGB (0..=255) → HSL with `h` in 0..=360 and `s`, `l` in 0..=100 (integer-rounded, matching
/// Excel's normalisation as ported by IronCalc).
fn rgb_to_hsl(rgb: [i32; 3]) -> [i32; 3] {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let (red, green, blue) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let chroma = (max_c - min_c) as f64 / 255.0;
    if chroma == 0.0 {
        return [0, 0, (red * 100.0).round() as i32];
    }
    let luminosity = (max_c + min_c) as f64 / (255.0 * 2.0);
    let saturation = if luminosity > 0.5 {
        0.5 * chroma / (1.0 - luminosity)
    } else {
        0.5 * chroma / luminosity
    };
    let hue = if max_c == r {
        if green >= blue {
            60.0 * (green - blue) / chroma
        } else {
            ((green - blue) / chroma + 6.0) * 60.0
        }
    } else if max_c == g {
        ((blue - red) / chroma + 2.0) * 60.0
    } else {
        ((red - green) / chroma + 4.0) * 60.0
    };
    [
        hue.round() as i32,
        (saturation * 100.0).round() as i32,
        (luminosity * 100.0).round() as i32,
    ]
}

/// HSL (`h` 0..=360, `s`/`l` 0..=100) → RGB (0..=255).
fn hsl_to_rgb(hsl: [i32; 3]) -> [i32; 3] {
    let hue = hsl[0] as f64 / 360.0;
    let saturation = hsl[1] as f64 / 100.0;
    let luminosity = hsl[2] as f64 / 100.0;
    if saturation == 0.0 {
        let v = (luminosity * 255.0).round() as i32;
        return [v, v, v];
    }
    let q = if luminosity < 0.5 {
        luminosity * (1.0 + saturation)
    } else {
        luminosity + saturation - luminosity * saturation
    };
    let p = 2.0 * luminosity - q;
    let ch = |t: f64| (255.0 * hue_to_rgb(p, q, t)).round().clamp(0.0, 255.0) as i32;
    [ch(hue + 1.0 / 3.0), ch(hue), ch(hue - 1.0 / 3.0)]
}

/// The standard HSL→RGB channel helper: normalise the hue offset into `[0, 1)`, then use that
/// normalised value throughout. (IronCalc's port keeps the un-normalised offset in the return
/// expressions, which overflows for very light tints on saturated hues; the correct form
/// reproduces the same Excel-verified goldens without that artefact.)
fn hue_to_rgb(p: f64, q: f64, t: f64) -> f64 {
    let mut t = t;
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

// ---------------------------------------------------------------------------------------------
// Built-in number-format correction
// ---------------------------------------------------------------------------------------------

/// The standard Excel built-in number formats IronCalc's `DEFAULT_NUM_FMTS` table maps to
/// garbage (locale-independent numeric / currency / accounting / misc ids). These codes are the
/// ECMA-376 built-ins; `get_num_fmt` prefers a workbook-defined `NumFmt` over its default table,
/// so injecting the correct code for a referenced-but-undefined id fixes the formatter.
const STANDARD_BUILTIN_NUM_FMTS: &[(i32, &str)] = &[
    (5, "$#,##0_);($#,##0)"),
    (6, "$#,##0_);[Red]($#,##0)"),
    (7, "$#,##0.00_);($#,##0.00)"),
    (8, "$#,##0.00_);[Red]($#,##0.00)"),
    (37, "#,##0_);(#,##0)"),
    (38, "#,##0_);[Red](#,##0)"),
    (39, "#,##0.00_);(#,##0.00)"),
    (40, "#,##0.00_);[Red](#,##0.00)"),
    (41, "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)"),
    (
        42,
        "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)",
    ),
    (43, "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)"),
    (
        44,
        "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
    ),
    (45, "mm:ss"),
    (46, "[h]:mm:ss"),
    (47, "mmss.0"),
    (48, "##0.0E+0"),
    (49, "@"),
];

/// Injects correct built-in format codes for the ids the workbook references but does not
/// define itself, so IronCalc's formatter no longer falls through to its broken default table.
fn inject_builtin_num_fmts(model: &mut Model) {
    let styles = &mut model.workbook.styles;

    let mut referenced = std::collections::HashSet::new();
    for xf in &styles.cell_xfs {
        referenced.insert(xf.num_fmt_id);
    }
    for xf in &styles.cell_style_xfs {
        referenced.insert(xf.num_fmt_id);
    }
    let defined: std::collections::HashSet<i32> =
        styles.num_fmts.iter().map(|n| n.num_fmt_id).collect();

    for &(id, code) in STANDARD_BUILTIN_NUM_FMTS {
        if referenced.contains(&id) && !defined.contains(&id) {
            styles.num_fmts.push(NumFmt {
                num_fmt_id: id,
                format_code: code.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tint algorithm reproduces Excel's own (rounding-verified) outputs — the same three
    /// goldens IronCalc's `colors.rs` asserts, computed here against the base colours of the
    /// *default* Office palette so they can be checked without a file.
    #[test]
    fn tint_matches_excel_goldens() {
        // get_themed_color(0, -0.05) == "#F2F2F2": base lt1 = white.
        assert_eq!(
            apply_tint(Rgb::from_hex(0xFFFFFF), -0.05),
            Rgb::from_hex(0xF2F2F2)
        );
        // get_themed_color(5, -0.25) == "#C55911": base accent2 = #ED7D31.
        assert_eq!(
            apply_tint(Rgb::from_hex(0xED7D31), -0.25),
            Rgb::from_hex(0xC55911)
        );
        // get_themed_color(4, 0.6) == "#B5C8E8": base accent1 = #4472C4.
        assert_eq!(
            apply_tint(Rgb::from_hex(0x4472C4), 0.6),
            Rgb::from_hex(0xB5C8E8)
        );
    }

    #[test]
    fn tint_zero_is_identity_and_extremes_saturate() {
        let purple = Rgb::from_hex(0x7A6FB9);
        assert_eq!(apply_tint(purple, 0.0), purple);
        // Fully negative tint → black; fully positive → white (any hue).
        assert_eq!(apply_tint(purple, -1.0), Rgb::from_hex(0x000000));
        assert_eq!(apply_tint(purple, 1.0), Rgb::from_hex(0xFFFFFF));
    }

    /// The dark/light swap + tint resolves this file's actual header/label colours: theme 1
    /// (this workbook's `dk1` = white) → white label fill; theme 3 (`dk2` = purple) tinted for
    /// the header band and the light lavender band.
    #[test]
    fn resolves_swapped_custom_theme() {
        // Palette in theme-index order for the mortgage workbook's "Custom 8" scheme.
        let palette: [Option<Rgb>; THEME_SLOTS] = [
            Some(Rgb::from_hex(0x000000)), // 0 lt1 (black)
            Some(Rgb::from_hex(0xFFFFFF)), // 1 dk1 (white)
            Some(Rgb::from_hex(0xE7E6E6)), // 2 lt2
            Some(Rgb::from_hex(0x7A6FB9)), // 3 dk2 (purple)
            Some(Rgb::from_hex(0x4B7CDD)),
            Some(Rgb::from_hex(0x226C8A)),
            Some(Rgb::from_hex(0x8C2858)),
            Some(Rgb::from_hex(0xBB4545)),
            Some(Rgb::from_hex(0xF6A176)),
            Some(Rgb::from_hex(0x70AD47)),
            Some(Rgb::from_hex(0xAFA8D4)),
            Some(Rgb::from_hex(0x7A6FB9)),
        ];

        // Label cell fill: <fgColor theme="1"/> → white (was black under IronCalc's default).
        let doc = roxmltree::Document::parse(r#"<fgColor theme="1"/>"#).unwrap();
        assert_eq!(
            themed_rgb(&doc.root_element(), &palette),
            Some(Rgb::from_hex(0xFFFFFF))
        );

        // Header band fill: <fgColor theme="3" tint="-0.2499…"/> → a darker purple.
        let doc = roxmltree::Document::parse(r#"<fgColor theme="3" tint="-0.249977111117893"/>"#)
            .unwrap();
        let header = themed_rgb(&doc.root_element(), &palette).unwrap();
        assert_eq!(
            header,
            apply_tint(Rgb::from_hex(0x7A6FB9), -0.249977111117893)
        );
        // It stays purple (blue-dominant) and darker than the base.
        assert!(header.b > header.r && header.b > header.g, "still purple");
        assert!(header.b < 0xB9, "darker than the base purple");

        // Light band fill: <fgColor theme="3" tint="0.7999…"/> → very light lavender.
        let doc = roxmltree::Document::parse(r#"<fgColor theme="3" tint="0.79998168889431442"/>"#)
            .unwrap();
        let band = themed_rgb(&doc.root_element(), &palette).unwrap();
        assert!(
            band.r > 0xD0 && band.g > 0xD0 && band.b > 0xD0,
            "light lavender"
        );
        assert!(band.b >= band.r && band.b >= band.g, "lavender leans blue");

        // A non-theme colour (explicit rgb) is left for IronCalc — themed_rgb declines it.
        let doc = roxmltree::Document::parse(r#"<fgColor rgb="FFAABBCC"/>"#).unwrap();
        assert_eq!(themed_rgb(&doc.root_element(), &palette), None);
    }

    #[test]
    fn out_of_range_theme_index_is_ignored() {
        let palette: [Option<Rgb>; THEME_SLOTS] = [None; THEME_SLOTS];
        let doc = roxmltree::Document::parse(r#"<fgColor theme="99"/>"#).unwrap();
        assert_eq!(themed_rgb(&doc.root_element(), &palette), None);
    }

    #[test]
    fn non_finite_tint_falls_back_to_base_color() {
        let mut palette: [Option<Rgb>; THEME_SLOTS] = [None; THEME_SLOTS];
        palette[3] = Some(Rgb::from_hex(0x7A6FB9));
        // A hostile `tint="NaN"`/`"inf"` must be ignored (→ base colour), never reach the maths.
        for bad in ["NaN", "inf", "-inf"] {
            let xml = format!(r#"<fgColor theme="3" tint="{bad}"/>"#);
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert_eq!(
                themed_rgb(&doc.root_element(), &palette),
                Some(Rgb::from_hex(0x7A6FB9)),
                "tint={bad:?} should fall back to the untinted base"
            );
        }
    }

    #[test]
    fn parse_hex6_validates() {
        assert_eq!(parse_hex6("7A6FB9"), Some(Rgb::from_hex(0x7A6FB9)));
        assert_eq!(parse_hex6("FFFFFF"), Some(Rgb::from_hex(0xFFFFFF)));
        assert_eq!(parse_hex6("FFF"), None); // wrong length
        assert_eq!(parse_hex6("GGGGGG"), None); // non-hex
    }

    // ----------------------------------------------------------------------------------------
    // Built-in number-format injection
    // ----------------------------------------------------------------------------------------

    #[test]
    fn injects_correct_builtin_num_fmt_for_referenced_id() {
        use ironcalc_base::number_format::get_num_fmt;
        use ironcalc_base::types::CellXfs;

        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        // A cell format index that references built-in id 39 — the id the mortgage workbook's
        // currency cells use, which IronCalc's default table maps to the broken "t0.00".
        model.workbook.styles.cell_xfs.push(CellXfs {
            num_fmt_id: 39,
            ..Default::default()
        });
        // Before: IronCalc resolves id 39 to a code its own formatter chokes on (→ #VALUE!).
        assert_eq!(get_num_fmt(39, &model.workbook.styles.num_fmts), "t0.00");

        inject_builtin_num_fmts(&mut model);

        // After: the correct ECMA-376 built-in code wins (formats "175000" as "175,000.00 ").
        assert_eq!(
            get_num_fmt(39, &model.workbook.styles.num_fmts),
            "#,##0.00_);(#,##0.00)"
        );
        // An unreferenced broken id (e.g. 41) is NOT injected — the table stays lean.
        assert!(!model
            .workbook
            .styles
            .num_fmts
            .iter()
            .any(|n| n.num_fmt_id == 41));
    }

    #[test]
    fn does_not_override_file_defined_num_fmt() {
        use ironcalc_base::number_format::get_num_fmt;
        use ironcalc_base::types::{CellXfs, NumFmt};

        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.cell_xfs.push(CellXfs {
            num_fmt_id: 44,
            ..Default::default()
        });
        // The file defines id 44 itself (a common, correct accounting code) — must be kept.
        model.workbook.styles.num_fmts.push(NumFmt {
            num_fmt_id: 44,
            format_code: "FILE-OWN".to_string(),
        });

        inject_builtin_num_fmts(&mut model);

        let count = model
            .workbook
            .styles
            .num_fmts
            .iter()
            .filter(|n| n.num_fmt_id == 44)
            .count();
        assert_eq!(count, 1, "must not duplicate a file-defined id");
        assert_eq!(get_num_fmt(44, &model.workbook.styles.num_fmts), "FILE-OWN");
    }

    // ----------------------------------------------------------------------------------------
    // End-to-end theme correction over a crafted (synthetic) `.xlsx` zip
    // ----------------------------------------------------------------------------------------

    /// A minified theme with the mortgage workbook's swapped "Custom 8" palette — authored here
    /// inline (no copyrighted fixture). `dk1`/`lt1` are inverted vs the default Office theme.
    const CRAFTED_THEME: &str = concat!(
        r#"<?xml version="1.0"?><a:theme xmlns:a="ns"><a:themeElements><a:clrScheme name="c">"#,
        r#"<a:dk1><a:srgbClr val="FFFFFF"/></a:dk1><a:lt1><a:srgbClr val="000000"/></a:lt1>"#,
        r#"<a:dk2><a:srgbClr val="7A6FB9"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>"#,
        r#"<a:accent1><a:srgbClr val="4B7CDD"/></a:accent1><a:accent2><a:srgbClr val="226C8A"/></a:accent2>"#,
        r#"<a:accent3><a:srgbClr val="8C2858"/></a:accent3><a:accent4><a:srgbClr val="BB4545"/></a:accent4>"#,
        r#"<a:accent5><a:srgbClr val="F6A176"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6>"#,
        r#"<a:hlink><a:srgbClr val="AFA8D4"/></a:hlink><a:folHlink><a:srgbClr val="7A6FB9"/></a:folHlink>"#,
        r#"</a:clrScheme></a:themeElements></a:theme>"#,
    );

    /// Minified styles.xml (no inter-element whitespace, as Excel writes it, so `<fills>` /
    /// `<fonts>` / `<borders>` child indices line up with the style tables). Fill 2 uses `theme=1`,
    /// fill 3 uses `theme=3` with a darkening tint, font 1 uses `theme=0`. Border 0 is the empty
    /// default; border 1 exercises the edge branches: `left` themed (`theme=1`), `right` explicit
    /// `rgb=`, `top` a non-theme `indexed=64`, `bottom` themed but with no matching model edge.
    const CRAFTED_STYLES: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="ns"><fills>"#,
        r#"<fill><patternFill patternType="none"/></fill>"#,
        r#"<fill><patternFill patternType="gray125"/></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor theme="1"/><bgColor indexed="64"/></patternFill></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor theme="3" tint="-0.249977111117893"/></patternFill></fill>"#,
        r#"</fills><fonts>"#,
        r#"<font><sz val="10"/></font>"#,
        r#"<font><color theme="0"/><sz val="10"/></font>"#,
        r#"</fonts><borders>"#,
        r#"<border><left/><right/><top/><bottom/><diagonal/></border>"#,
        r#"<border><left style="thin"><color theme="1"/></left>"#,
        r#"<right style="thin"><color rgb="FFAABBCC"/></right>"#,
        r#"<top style="thin"><color indexed="64"/></top>"#,
        r#"<bottom style="thin"><color theme="1"/></bottom><diagonal/></border>"#,
        r#"</borders></styleSheet>"#,
    );

    fn write_crafted_xlsx(dir: &std::path::Path) -> std::path::PathBuf {
        use std::io::Write;
        let path = dir.join("crafted.xlsx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::default();
        zw.start_file("xl/theme/theme1.xml", opts).unwrap();
        zw.write_all(CRAFTED_THEME.as_bytes()).unwrap();
        zw.start_file("xl/styles.xml", opts).unwrap();
        zw.write_all(CRAFTED_STYLES.as_bytes()).unwrap();
        zw.finish().unwrap();
        path
    }

    #[test]
    fn read_theme_palette_applies_dark_light_swap() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_crafted_xlsx(dir.path());
        let palette = read_theme_palette(&path).unwrap();
        // The style `theme=` index applies the swap: 0→lt1 (black), 1→dk1 (white), 3→dk2 (purple).
        assert_eq!(palette[0], Some(Rgb::from_hex(0x000000)));
        assert_eq!(palette[1], Some(Rgb::from_hex(0xFFFFFF)));
        assert_eq!(palette[3], Some(Rgb::from_hex(0x7A6FB9)));
        assert_eq!(palette[9], Some(Rgb::from_hex(0x70AD47))); // accent6
    }

    #[test]
    fn correct_theme_colors_rewrites_only_themed_entries() {
        use ironcalc_base::types::{Fill, Font};

        let dir = tempfile::tempdir().unwrap();
        let path = write_crafted_xlsx(dir.path());

        // A model whose style tables mirror the crafted styles.xml the way IronCalc would import
        // them — theme colours resolved WRONGLY against the default Office palette.
        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.fills = vec![
            Fill {
                pattern_type: "none".into(),
                fg_color: None,
                bg_color: None,
            },
            Fill {
                pattern_type: "gray125".into(),
                fg_color: None,
                bg_color: None,
            },
            Fill {
                pattern_type: "solid".into(),
                fg_color: Some("#000000".into()), // theme=1 wrongly resolved to black
                bg_color: Some("#000000".into()),
            },
            Fill {
                pattern_type: "solid".into(),
                fg_color: Some("#33404F".into()), // theme=3 wrongly resolved to a dark navy
                bg_color: None,
            },
        ];
        model.workbook.styles.fonts = vec![
            Font::default(),
            Font {
                color: Some("#FFFFFF".into()), // theme=0 wrongly resolved to white
                ..Font::default()
            },
        ];

        correct_theme_colors(&mut model, &path);

        // Fill 2 (theme=1 → this file's dk1) is now white; its non-themed bgColor is untouched.
        assert_eq!(
            model.workbook.styles.fills[2].fg_color.as_deref(),
            Some("#FFFFFF")
        );
        assert_eq!(
            model.workbook.styles.fills[2].bg_color.as_deref(),
            Some("#000000"),
            "an indexed bgColor is not themed and must be left as IronCalc parsed it"
        );
        // Fill 3 (theme=3 → purple, darkening tint) is a darker purple (blue-dominant).
        let f3 = model.workbook.styles.fills[3].fg_color.clone().unwrap();
        let rgb = parse_hex6(f3.trim_start_matches('#')).unwrap();
        assert!(rgb.b > rgb.r && rgb.b > rgb.g, "still purple, got {f3}");
        assert!(rgb.b < 0xB9, "darker than the base purple, got {f3}");
        // Font 1 (theme=0 → this file's lt1) is now black, not white.
        assert_eq!(
            model.workbook.styles.fonts[1].color.as_deref(),
            Some("#000000")
        );
    }

    #[test]
    fn correct_theme_colors_rewrites_border_edges() {
        use ironcalc_base::types::{Border, BorderItem, BorderStyle};

        let dir = tempfile::tempdir().unwrap();
        let path = write_crafted_xlsx(dir.path());

        let thin = |c: &str| BorderItem {
            style: BorderStyle::Thin,
            color: Some(c.to_string()),
        };

        // Model borders mirror how IronCalc imports the crafted `<borders>` — each themed edge
        // resolved WRONGLY against the default Office palette. Border 1 has NO `bottom` edge in the
        // model, so even though the XML carries a `bottom` themed colour it must stay `None`.
        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.borders = vec![
            Border::default(), // border 0: the empty default
            Border {
                left: Some(thin("#000000")),  // theme=1 wrongly resolved to black
                right: Some(thin("#AABBCC")), // explicit rgb= (must be left alone)
                top: Some(thin("#000000")),   // indexed=64 (not themed → left alone)
                bottom: None,                 // no model edge → must NOT be fabricated
                ..Border::default()
            },
        ];

        correct_theme_colors(&mut model, &path);

        let b = &model.workbook.styles.borders[1];
        // left (theme=1) → this file's dk1 white.
        assert_eq!(b.left.as_ref().unwrap().color.as_deref(), Some("#FFFFFF"));
        // right (explicit rgb=) is left exactly as IronCalc parsed it.
        assert_eq!(b.right.as_ref().unwrap().color.as_deref(), Some("#AABBCC"));
        // top (indexed=64) is not a theme colour → untouched.
        assert_eq!(b.top.as_ref().unwrap().color.as_deref(), Some("#000000"));
        // bottom had no model edge → the themed XML colour must not fabricate one.
        assert!(
            b.bottom.is_none(),
            "a missing model edge is never fabricated"
        );
    }

    #[test]
    fn correct_theme_colors_is_noop_without_theme_file() {
        use ironcalc_base::types::Fill;
        // A zip with styles but no theme1.xml → theme correction bails, model untouched.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_theme.xlsx");
        {
            use std::io::Write;
            let file = std::fs::File::create(&path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            zw.start_file("xl/styles.xml", zip::write::FileOptions::default())
                .unwrap();
            zw.write_all(CRAFTED_STYLES.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.fills = vec![Fill {
            pattern_type: "solid".into(),
            fg_color: Some("#000000".into()),
            bg_color: None,
        }];
        correct_theme_colors(&mut model, &path);
        assert_eq!(
            model.workbook.styles.fills[0].fg_color.as_deref(),
            Some("#000000"),
            "no theme file → nothing is rewritten"
        );
    }

    // ----------------------------------------------------------------------------------------
    // Indexed-colour correction (`<colors><indexedColors>` override)
    // ----------------------------------------------------------------------------------------

    /// A minified styles.xml carrying a `<colors><indexedColors>` override (the first 8 standard
    /// entries + this file's custom 9/12/13/14, mirroring the Numbers fixture) and fills/fonts/borders
    /// exercising every branch: an in-range `indexed=` fill (2), another in-range indexed fill
    /// (3), an out-of-range index (4), the system index 64 (5), and an explicit `rgb=` (6); font 1
    /// uses an in-range `indexed=` colour; and a `<borders>` section (border 1's edges cover the
    /// in-range / rgb= / system-64 / out-of-range / no-model-edge cases — see the const body).
    const CRAFTED_INDEXED_STYLES: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="ns"><fills>"#,
        r#"<fill><patternFill patternType="none"/></fill>"#,
        r#"<fill><patternFill patternType="gray125"/></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor indexed="9"/><bgColor auto="1"/></patternFill></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor indexed="13"/></patternFill></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor indexed="99"/></patternFill></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor indexed="64"/></patternFill></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor rgb="FFAABBCC"/></patternFill></fill>"#,
        r#"</fills><fonts>"#,
        r#"<font><sz val="10"/></font>"#,
        r#"<font><color indexed="14"/><sz val="10"/></font>"#,
        r#"</fonts>"#,
        // Border 0 is the empty default; border 1 exercises the edge branches: `left` an in-range
        // `indexed=13` (→ the file's #FFD931), `right` an explicit `rgb=` (left alone), `top` the
        // system `indexed=64` (left alone), `bottom` an out-of-range `indexed=99` (left alone),
        // and `diagonal` an in-range `indexed=9` whose model edge is `None` (never fabricated).
        r#"<borders>"#,
        r#"<border><left/><right/><top/><bottom/><diagonal/></border>"#,
        r#"<border><left style="thin"><color indexed="13"/></left>"#,
        r#"<right style="thin"><color rgb="FFAABBCC"/></right>"#,
        r#"<top style="thin"><color indexed="64"/></top>"#,
        r#"<bottom style="thin"><color indexed="99"/></bottom>"#,
        r#"<diagonal style="thin"><color indexed="9"/></diagonal></border>"#,
        r#"</borders>"#,
        r#"<colors><indexedColors>"#,
        r#"<rgbColor rgb="ff000000"/><rgbColor rgb="ffffffff"/><rgbColor rgb="ffff0000"/>"#,
        r#"<rgbColor rgb="ff00ff00"/><rgbColor rgb="ff0000ff"/><rgbColor rgb="ffffff00"/>"#,
        r#"<rgbColor rgb="ffff00ff"/><rgbColor rgb="ff00ffff"/><rgbColor rgb="ff000000"/>"#,
        r#"<rgbColor rgb="ffbdc0bf"/><rgbColor rgb="ffa5a5a5"/><rgbColor rgb="ff3f3f3f"/>"#,
        r#"<rgbColor rgb="ffdbdbdb"/><rgbColor rgb="ffffd931"/><rgbColor rgb="fffe634d"/>"#,
        r#"</indexedColors></colors></styleSheet>"#,
    );

    /// The same fills as above but with **no** `<colors><indexedColors>` override — a
    /// standard-palette file. `correct_indexed_colors` must touch nothing.
    const CRAFTED_INDEXED_NO_OVERRIDE: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="ns"><fills>"#,
        r#"<fill><patternFill patternType="none"/></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor indexed="13"/></patternFill></fill>"#,
        r#"</fills><fonts><font><color indexed="14"/><sz val="10"/></font></fonts></styleSheet>"#,
    );

    /// Zips a single `xl/styles.xml` into a `.xlsx` (no theme) — the input `correct_indexed_colors`
    /// reads. Named distinctly from the theme helper so the two colour passes stay independent.
    fn write_styles_only_xlsx(dir: &std::path::Path, styles: &str) -> std::path::PathBuf {
        use std::io::Write;
        let path = dir.join("indexed.xlsx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        zw.start_file("xl/styles.xml", zip::write::FileOptions::default())
            .unwrap();
        zw.write_all(styles.as_bytes()).unwrap();
        zw.finish().unwrap();
        path
    }

    #[test]
    fn parse_indexed_rgb_strips_alpha_and_validates() {
        // AARRGGBB (the form Excel/Numbers write) drops the leading alpha byte.
        assert_eq!(parse_indexed_rgb("ffbdc0bf"), Some(Rgb::from_hex(0xBDC0BF)));
        assert_eq!(parse_indexed_rgb("00FFD931"), Some(Rgb::from_hex(0xFFD931)));
        // A bare RRGGBB is accepted too.
        assert_eq!(parse_indexed_rgb("fe634d"), Some(Rgb::from_hex(0xFE634D)));
        // Malformed → None (skipped, never a panic).
        assert_eq!(parse_indexed_rgb("xyz"), None); // wrong length
        assert_eq!(parse_indexed_rgb("gggggggg"), None); // non-hex
        assert_eq!(parse_indexed_rgb(""), None);
    }

    #[test]
    fn read_indexed_palette_parses_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_styles_only_xlsx(dir.path(), CRAFTED_INDEXED_STYLES);
        let xml = read_zip_entry(&path, "xl/styles.xml").unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let palette = read_indexed_palette(doc.root_element()).unwrap();
        assert_eq!(palette.len(), 15);
        assert_eq!(palette[9], Some(Rgb::from_hex(0xBDC0BF)));
        assert_eq!(palette[12], Some(Rgb::from_hex(0xDBDBDB)));
        assert_eq!(palette[13], Some(Rgb::from_hex(0xFFD931)));
        assert_eq!(palette[14], Some(Rgb::from_hex(0xFE634D)));

        // A styles.xml with no `<colors><indexedColors>` yields `None` — the "leave it alone"
        // signal that keeps standard-palette files untouched.
        let path2 = write_styles_only_xlsx(dir.path(), CRAFTED_INDEXED_NO_OVERRIDE);
        let xml2 = read_zip_entry(&path2, "xl/styles.xml").unwrap();
        let doc2 = roxmltree::Document::parse(&xml2).unwrap();
        assert!(read_indexed_palette(doc2.root_element()).is_none());
    }

    #[test]
    fn indexed_rgb_honours_precedence_and_guards() {
        let palette = vec![
            Some(Rgb::from_hex(0x000000)),
            Some(Rgb::from_hex(0xFFFFFF)),
            None, // a malformed override entry
        ];
        let resolve = |xml: &str| {
            let doc = roxmltree::Document::parse(xml).unwrap();
            indexed_rgb(&doc.root_element(), &palette)
        };
        // In-range index → the override slot.
        assert_eq!(
            resolve(r#"<fgColor indexed="1"/>"#),
            Some(Rgb::from_hex(0xFFFFFF))
        );
        // Explicit rgb= takes precedence in IronCalc, so we decline (leave its value).
        assert_eq!(resolve(r#"<fgColor rgb="FFAABBCC" indexed="1"/>"#), None);
        // System index 64 (auto) is not a palette lookup → declined.
        assert_eq!(resolve(r#"<fgColor indexed="64"/>"#), None);
        // Out of range → declined (leave IronCalc's value).
        assert_eq!(resolve(r#"<fgColor indexed="9"/>"#), None);
        // A malformed override slot (stored None) → declined.
        assert_eq!(resolve(r#"<fgColor indexed="2"/>"#), None);
        // A theme= / auto= node is not indexed → declined.
        assert_eq!(resolve(r#"<fgColor theme="3"/>"#), None);
        assert_eq!(resolve(r#"<bgColor auto="1"/>"#), None);
    }

    #[test]
    fn correct_indexed_colors_rewrites_only_indexed_entries() {
        use ironcalc_base::types::{Fill, Font};

        let dir = tempfile::tempdir().unwrap();
        let path = write_styles_only_xlsx(dir.path(), CRAFTED_INDEXED_STYLES);

        // A model whose style tables mirror how IronCalc imports the crafted styles.xml — every
        // `indexed=` colour resolved WRONGLY against IronCalc's hardcoded legacy palette.
        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        let solid = |fg: Option<&str>| Fill {
            pattern_type: "solid".into(),
            fg_color: fg.map(str::to_string),
            bg_color: None,
        };
        model.workbook.styles.fills = vec![
            Fill {
                pattern_type: "none".into(),
                fg_color: None,
                bg_color: None,
            },
            Fill {
                pattern_type: "gray125".into(),
                fg_color: None,
                bg_color: None,
            },
            solid(Some("#FFFFFF")), // fill 2: index 9 → IronCalc default white
            solid(Some("#FFFF00")), // fill 3: index 13 → IronCalc default yellow
            solid(Some("#000000")), // fill 4: index 99 → IronCalc out-of-range fallback
            solid(Some("#000000")), // fill 5: index 64 → IronCalc "auto"/fallback
            solid(Some("#AABBCC")), // fill 6: explicit rgb=FFAABBCC
        ];
        model.workbook.styles.fonts = vec![
            Font::default(),
            Font {
                color: Some("#FF00FF".into()), // font 1: index 14 → IronCalc default magenta
                ..Font::default()
            },
        ];

        correct_indexed_colors(&mut model, &path);

        // Fill 2 (index 9) → the file's #BDC0BF; its auto bgColor was never set and stays None.
        assert_eq!(
            model.workbook.styles.fills[2].fg_color.as_deref(),
            Some("#BDC0BF")
        );
        assert_eq!(model.workbook.styles.fills[2].bg_color, None);
        // Fill 3 (index 13) → the file's #FFD931 (gold), not IronCalc's yellow.
        assert_eq!(
            model.workbook.styles.fills[3].fg_color.as_deref(),
            Some("#FFD931")
        );
        // Fill 4 (index 99, out of range) → left as IronCalc parsed it.
        assert_eq!(
            model.workbook.styles.fills[4].fg_color.as_deref(),
            Some("#000000"),
            "an out-of-range index must not be remapped"
        );
        // Fill 5 (index 64, system auto) → left as IronCalc parsed it.
        assert_eq!(
            model.workbook.styles.fills[5].fg_color.as_deref(),
            Some("#000000"),
            "index 64 (auto) must not be remapped to an override slot"
        );
        // Fill 6 (explicit rgb=) → untouched.
        assert_eq!(
            model.workbook.styles.fills[6].fg_color.as_deref(),
            Some("#AABBCC"),
            "an explicit rgb= colour is not indexed and must be left alone"
        );
        // Font 1 (index 14) → the file's #FE634D (red-orange), not IronCalc's magenta.
        assert_eq!(
            model.workbook.styles.fonts[1].color.as_deref(),
            Some("#FE634D")
        );
    }

    #[test]
    fn correct_indexed_colors_rewrites_border_edges() {
        use ironcalc_base::types::{Border, BorderItem, BorderStyle};

        let dir = tempfile::tempdir().unwrap();
        let path = write_styles_only_xlsx(dir.path(), CRAFTED_INDEXED_STYLES);

        let thin = |c: &str| BorderItem {
            style: BorderStyle::Thin,
            color: Some(c.to_string()),
        };

        // Model borders mirror how IronCalc imports the crafted `<borders>` — each `indexed=` edge
        // resolved WRONGLY against IronCalc's hardcoded legacy palette. Border 1's `diagonal` edge is
        // absent from the model, so the in-range XML diagonal colour must NOT fabricate one.
        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.borders = vec![
            Border::default(), // border 0: the empty default
            Border {
                left: Some(thin("#FFFF00")),   // index 13 → IronCalc default yellow
                right: Some(thin("#AABBCC")),  // explicit rgb= (must be left alone)
                top: Some(thin("#000000")),    // index 64 (system auto → left alone)
                bottom: Some(thin("#000000")), // index 99 (out of range → left alone)
                diagonal: None,                // no model edge → must NOT be fabricated
                ..Border::default()
            },
        ];

        correct_indexed_colors(&mut model, &path);

        let b = &model.workbook.styles.borders[1];
        // left (index 13) → the file's #FFD931 gold, not IronCalc's yellow.
        assert_eq!(b.left.as_ref().unwrap().color.as_deref(), Some("#FFD931"));
        // right (explicit rgb=) is left alone.
        assert_eq!(b.right.as_ref().unwrap().color.as_deref(), Some("#AABBCC"));
        // top (index 64, system auto) must not be remapped to an override slot.
        assert_eq!(b.top.as_ref().unwrap().color.as_deref(), Some("#000000"));
        // bottom (index 99, out of range) must not be remapped.
        assert_eq!(b.bottom.as_ref().unwrap().color.as_deref(), Some("#000000"));
        // diagonal had no model edge → the in-range XML colour must not fabricate one.
        assert!(
            b.diagonal.is_none(),
            "a missing model edge is never fabricated"
        );
    }

    #[test]
    fn correct_indexed_colors_is_noop_without_override() {
        use ironcalc_base::types::{Fill, Font};
        // A standard-palette file (indexed fills, but NO `<colors><indexedColors>` override): the
        // pass must change nothing, so files using the standard palette never regress.
        let dir = tempfile::tempdir().unwrap();
        let path = write_styles_only_xlsx(dir.path(), CRAFTED_INDEXED_NO_OVERRIDE);

        let mut model = Model::new_empty("b", "en", "UTC", "en").unwrap();
        model.workbook.styles.fills = vec![
            Fill {
                pattern_type: "none".into(),
                fg_color: None,
                bg_color: None,
            },
            Fill {
                pattern_type: "solid".into(),
                fg_color: Some("#FFFF00".into()), // IronCalc's index-13 default
                bg_color: None,
            },
        ];
        model.workbook.styles.fonts = vec![Font {
            color: Some("#FF00FF".into()), // IronCalc's index-14 default
            ..Font::default()
        }];

        correct_indexed_colors(&mut model, &path);

        assert_eq!(
            model.workbook.styles.fills[1].fg_color.as_deref(),
            Some("#FFFF00"),
            "no override → indexed fill left exactly as IronCalc parsed it"
        );
        assert_eq!(
            model.workbook.styles.fonts[0].color.as_deref(),
            Some("#FF00FF"),
            "no override → indexed font colour left exactly as IronCalc parsed it"
        );
    }

    /// End-to-end colour regression over the committed Numbers fixture: opening it through the
    /// full public path (`WorkbookDocument::open`, which runs `apply_open_fixups`) and reading the
    /// resolved fill through the exact grid/cache accessor (`cache::render_style_from` over the
    /// resolved style) must yield the file's palette colours, not IronCalc's legacy defaults.
    #[test]
    fn numbers_fixture_indexed_fills_resolve_to_file_palette() {
        use crate::cache::render_style_from;
        use crate::document::WorkbookDocument;
        use freecell_core::CellRef;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/numbers_table.xlsx");
        let doc = WorkbookDocument::open(&path).expect("the Numbers fixture opens (via repair)");

        // The resolved fill the grid would paint for a 0-based cell.
        let fill = |row: u32, col: u32| -> Option<Rgb> {
            let style = doc
                .resolved_cell_style(0, CellRef::new(row, col))
                .expect("cell style resolves");
            render_style_from(&style).fill
        };

        // Header band B2 uses fill `indexed="9"` → the file's light grey #BDC0BF
        // (IronCalc's legacy default 9 is white).
        assert_eq!(
            fill(1, 1),
            Some(Rgb::from_hex(0xBDC0BF)),
            "B2 header band → file index 9 (#BDC0BF)"
        );
        // The ASDF/TOTAL label column A uses fill `indexed="12"` → the file's #DBDBDB
        // (IronCalc's legacy default 12 is bright blue #0000FF — the reported bug).
        assert_eq!(
            fill(2, 0),
            Some(Rgb::from_hex(0xDBDBDB)),
            "A3 label column → file index 12 (#DBDBDB)"
        );
        assert_eq!(
            fill(17, 0),
            Some(Rgb::from_hex(0xDBDBDB)),
            "A18 TOTAL label → file index 12 (#DBDBDB)"
        );
        // TOTAL B18 uses `indexed="13"` → gold #FFD931 (not IronCalc's yellow #FFFF00).
        assert_eq!(
            fill(17, 1),
            Some(Rgb::from_hex(0xFFD931)),
            "B18 → file index 13 (#FFD931 gold)"
        );
        // TOTAL C18 uses `indexed="14"` → red-orange #FE634D (not IronCalc's magenta #FF00FF).
        assert_eq!(
            fill(17, 2),
            Some(Rgb::from_hex(0xFE634D)),
            "C18 → file index 14 (#FE634D red-orange)"
        );
    }

    /// End-to-end border-colour regression over the committed FONTS.xlsx fixture (a Numbers export
    /// whose border edges reference `indexed="10"`/`indexed="11"`). Opening through the full public
    /// path (`WorkbookDocument::open` → `apply_open_fixups`) and reading the resolved border must
    /// yield the file's grey palette (10 → #A5A5A5, 11 → #3F3F3F), not IronCalc's legacy-palette
    /// red (#FF0000) / green (#00FF00). Cell map verified against `xl/styles.xml` + `sheet1.xml`.
    #[test]
    fn fonts_fixture_indexed_borders_resolve_to_file_palette() {
        use crate::document::WorkbookDocument;
        use freecell_core::CellRef;

        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/FONTS.xlsx");
        let doc = WorkbookDocument::open(&path).expect("FONTS.xlsx opens (via repair)");

        // C4 (0-based row 3, col 2): style 10 → borderId 7 → all four edges `indexed="10"` → the
        // file's grey #A5A5A5 (IronCalc's legacy default 10 is red #FF0000).
        let border = doc
            .resolved_cell_style(0, CellRef::new(3, 2))
            .expect("C4 style resolves")
            .border;
        for (name, edge) in [
            ("top", &border.top),
            ("right", &border.right),
            ("bottom", &border.bottom),
            ("left", &border.left),
        ] {
            let item = edge
                .as_ref()
                .unwrap_or_else(|| panic!("C4 {name} edge present"));
            assert_eq!(
                item.color.as_deref(),
                Some("#A5A5A5"),
                "C4 {name} border must be the file's index-10 grey #A5A5A5, not IronCalc's legacy red #FF0000",
            );
        }

        // B3 (0-based row 2, col 1): style 5 → borderId 3 → left edge `indexed="11"` → the file's
        // dark grey #3F3F3F (IronCalc's legacy default 11 is green #00FF00).
        let b3 = doc
            .resolved_cell_style(0, CellRef::new(2, 1))
            .expect("B3 style resolves")
            .border;
        assert_eq!(
            b3.left
                .as_ref()
                .expect("B3 left edge present")
                .color
                .as_deref(),
            Some("#3F3F3F"),
            "B3 left border must be the file's index-11 dark grey #3F3F3F, not IronCalc's legacy green #00FF00",
        );
    }
}
