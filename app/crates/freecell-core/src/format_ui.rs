//! `format_ui` — pure, engine-free helpers the action bar uses to drive its number-format
//! dropdown and decimals ± buttons (`components/action_bar.md §State derivation`).
//!
//! These operate purely on number-format **code strings** (the same codes FreeCell's dropdown
//! sends and the resident [`SheetCache`](crate::SheetCache) caches per cell), so they are unit
//! testable with no engine, no gpui, and no rendering. Display itself stays engine-owned — nothing
//! here formats a value; it only classifies a code and rewrites its decimal group.

/// The category a number-format code maps to for the dropdown's label + entry selection
/// (`components/action_bar.md`, `architecture.md §3.2`). Anything that is not one of FreeCell's own
/// dropdown codes displays as [`Category::Custom`] (loaded-file formats are shown, never edited).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    Number,
    Currency,
    Accounting,
    Percent,
    Date,
    Time,
    Scientific,
    Text,
    Custom,
}

impl Category {
    /// The human label shown on the dropdown button + the popover's section headers.
    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Number => "Number",
            Category::Currency => "Currency",
            Category::Accounting => "Accounting",
            Category::Percent => "Percent",
            Category::Date => "Date",
            Category::Time => "Time",
            Category::Scientific => "Scientific",
            Category::Text => "Text",
            Category::Custom => "Custom",
        }
    }
}

/// A single dropdown preset: the label shown in the popover paired with the exact format code it
/// sends (`architecture.md §6`). The code is the sole thing FreeCell's set-number-format command
/// carries; IronCalc renders it. `label` is a short human hint (a sample value or the pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumFmtPreset {
    pub label: &'static str,
    pub code: &'static str,
}

/// A labeled section of the number-format dropdown: a [`Category`] (its `label()` is the section
/// header + the collapsed dropdown-button label) and the presets it offers.
#[derive(Debug, Clone, Copy)]
pub struct NumFmtGroup {
    pub category: Category,
    pub presets: &'static [NumFmtPreset],
}

/// The grouped dropdown inventory in menu order (`functional_spec.md §6` D6.1). Each code is the
/// exact string sent to the set-number-format command; `General` sends the engine's `"general"`
/// (which clears the format). This is the single source of truth for both the popover's contents
/// and [`num_fmt_category`]'s reverse map, so the highlighted preset always matches an active
/// cell's code.
pub const NUM_FMT_GROUPS: &[NumFmtGroup] = &[
    NumFmtGroup {
        category: Category::General,
        presets: &[NumFmtPreset {
            label: "General",
            code: "general",
        }],
    },
    NumFmtGroup {
        category: Category::Number,
        presets: &[
            NumFmtPreset {
                label: "1234.56",
                code: "0.00",
            },
            NumFmtPreset {
                label: "1,234.56",
                code: "#,##0.00",
            },
            NumFmtPreset {
                label: "1,235",
                code: "#,##0",
            },
            NumFmtPreset {
                label: "-1,234.56",
                code: "#,##0.00;[Red]-#,##0.00",
            },
        ],
    },
    NumFmtGroup {
        category: Category::Currency,
        presets: &[
            NumFmtPreset {
                label: "$1,234.56",
                code: "$#,##0.00",
            },
            NumFmtPreset {
                label: "€1,234.56",
                code: "€#,##0.00",
            },
            // `£` / `¥` must use IronCalc's bracketed locale-currency form `[$sym]…`: its format
            // lexer only accepts the *bare* symbols `$` and `€`, so a bare `£#,##0.00` / `¥#,##0.00`
            // fails to parse and every such cell renders `#VALUE!`. The `[`/`]` don't trip the
            // decimals / thousands adjustability gate, so both toggles still work.
            NumFmtPreset {
                label: "£1,234.56",
                code: "[$£]#,##0.00",
            },
            NumFmtPreset {
                label: "¥1,234.56",
                code: "[$¥]#,##0.00",
            },
        ],
    },
    NumFmtGroup {
        category: Category::Accounting,
        presets: &[NumFmtPreset {
            label: "$(1,234.56)",
            code: "$#,##0.00;($#,##0.00)",
        }],
    },
    NumFmtGroup {
        category: Category::Date,
        presets: &[
            NumFmtPreset {
                label: "m/d/yyyy",
                code: "m/d/yyyy",
            },
            NumFmtPreset {
                label: "yyyy-mm-dd",
                code: "yyyy-mm-dd",
            },
            NumFmtPreset {
                label: "d-mmm-yyyy",
                code: "d-mmm-yyyy",
            },
            NumFmtPreset {
                label: "mmm d, yyyy",
                code: "mmm d, yyyy",
            },
            NumFmtPreset {
                label: "m/d/yy",
                code: "m/d/yy",
            },
        ],
    },
    NumFmtGroup {
        category: Category::Time,
        presets: &[
            NumFmtPreset {
                label: "1:30 PM",
                code: "h:mm AM/PM",
            },
            NumFmtPreset {
                label: "1:30:55 PM",
                code: "h:mm:ss AM/PM",
            },
            NumFmtPreset {
                label: "13:30",
                code: "h:mm",
            },
            NumFmtPreset {
                label: "[h]:mm:ss (elapsed)",
                code: "[h]:mm:ss",
            },
        ],
    },
    NumFmtGroup {
        category: Category::Percent,
        presets: &[
            NumFmtPreset {
                label: "12.34%",
                code: "0.00%",
            },
            NumFmtPreset {
                label: "12%",
                code: "0%",
            },
        ],
    },
    NumFmtGroup {
        category: Category::Scientific,
        presets: &[NumFmtPreset {
            label: "1.23E+04",
            code: "0.00E+00",
        }],
    },
    // Fraction (`# ?/?`) is intentionally absent: IronCalc's `?/?` fraction formatting is
    // effectively unimplemented (`1.5` → "  /2"), and this batch is FreeCell-side / no-fork. It
    // is deferred to a fork feature — see `PROJECTS.md` / `projects/fraction-number-format.md`.
    NumFmtGroup {
        category: Category::Text,
        presets: &[NumFmtPreset {
            label: "Text",
            code: "@",
        }],
    },
];

/// The **basic** dropdown set — the original seven presets that shipped *before* Phase 6's
/// grouped breadth (`functional_spec.md §10.1`, D10.1). These are the common formats the popover
/// shows flat, first, without scrolling; the full [`NUM_FMT_GROUPS`] inventory lives behind the
/// trailing "More ▸" row. Labeled by category name, exactly as the pre-Phase-6 menu displayed them.
///
/// Every basic code is also a member of [`NUM_FMT_GROUPS`], so [`num_fmt_category`] (the single
/// reverse-map source of truth) stays consistent across both dropdown levels — a basic preset and
/// its `NUM_FMT_GROUPS` twin share the same code and reverse-map to the same category.
pub const BASIC_FORMATS: &[NumFmtPreset] = &[
    NumFmtPreset {
        label: "General",
        code: "general",
    },
    NumFmtPreset {
        label: "Number",
        code: "#,##0.00",
    },
    NumFmtPreset {
        label: "Currency",
        code: "$#,##0.00",
    },
    NumFmtPreset {
        label: "Percent",
        code: "0.00%",
    },
    NumFmtPreset {
        label: "Date",
        code: "m/d/yyyy",
    },
    NumFmtPreset {
        label: "Time",
        code: "h:mm AM/PM",
    },
    NumFmtPreset {
        label: "Text",
        code: "@",
    },
];

/// Reverse-maps a number-format `code` to its [`Category`] by exact match against the codes in
/// [`NUM_FMT_GROUPS`] (case-insensitive only for `"general"`, which the engine stores lowercase).
/// Any other code — including richer file-authored formats — is [`Category::Custom`].
pub fn num_fmt_category(code: &str) -> Category {
    if code.eq_ignore_ascii_case("general") {
        return Category::General;
    }
    NUM_FMT_GROUPS
        .iter()
        .find(|g| g.presets.iter().any(|p| p.code == code))
        .map(|g| g.category)
        .unwrap_or(Category::Custom)
}

/// Whether `code` is one of the [`BASIC_FORMATS`] presets — the codes the basics-first dropdown
/// shows flat (`functional_spec.md §10.1`). `general` matches case-insensitively (the engine echoes
/// either case); every other code matches exactly.
pub fn is_basic_num_fmt(code: &str) -> bool {
    if code.eq_ignore_ascii_case("general") {
        return true;
    }
    BASIC_FORMATS.iter().any(|p| p.code == code)
}

/// Whether `code` is a **recognized** preset that lives only behind "More ▸" (a member of
/// [`NUM_FMT_GROUPS`] but not [`BASIC_FORMATS`]) — used to mark the "More ▸" row active and to open
/// the dropdown directly onto the matched group when the active cell's format isn't a basic one
/// (`architecture.md §10`, D10.1). A [`Category::Custom`] code (no preset match at all) is neither
/// basic nor more-only, so this returns `false` for it (nothing to highlight / open onto).
pub fn is_more_only_num_fmt(code: &str) -> bool {
    num_fmt_category(code) != Category::Custom && !is_basic_num_fmt(code)
}

/// Adds (`delta > 0`) or removes (`delta < 0`) one decimal place from a number-format `code`,
/// returning the rewritten code, or `None` when the change is a no-op
/// (`components/action_bar.md §Command emission`).
///
/// It rewrites the **last** `0(\.0+)?` group in the code: `+1` appends a `0` (creating `.0` when the
/// group has no decimals yet), `-1` drops one trailing `0` (dropping the `.` when the group empties).
/// Decimals never go below zero. Codes with no such group — General, Text, and the canonical Date /
/// Time codes (none carry a `0` placeholder) — return `None`, so the button renders disabled and
/// no-ops. `delta` is expected to be `±1`; other magnitudes clamp at zero decimals.
///
/// **Only single-section formats with no exponent and no quoted/escaped literal are adjusted** — the
/// last-group heuristic can't safely edit multi-section (`;`), scientific (`E`/`e`), or
/// quoted/escaped (`"…"`, `\`) codes without corrupting them (it would target the wrong `0`, diverge
/// the sections, or mangle a literal), and `functional_spec.md §3.4` only guarantees adjustment for
/// the dropdown-native numeric formats. Such codes return `None` (buttons disable), so a file-authored
/// custom format is never one-click-broken.
pub fn adjust_decimals(code: &str, delta: i8) -> Option<String> {
    if !is_decimals_adjustable(code) {
        return None;
    }
    let (start, end, decimals) = last_zero_group(code)?;
    let new_decimals = (decimals as i32 + delta as i32).max(0) as usize;
    if new_decimals == decimals {
        return None; // e.g. decrease at zero decimals: nothing to remove
    }
    let mut group = String::with_capacity(new_decimals + 2);
    group.push('0');
    if new_decimals > 0 {
        group.push('.');
        for _ in 0..new_decimals {
            group.push('0');
        }
    }
    let mut out = String::with_capacity(code.len() + 2);
    out.push_str(&code[..start]);
    out.push_str(&group);
    out.push_str(&code[end..]);
    Some(out)
}

/// Toggles the thousands (`,`) grouping on a number-format `code`, returning the rewritten code, or
/// `None` when grouping can't be safely toggled (`functional_spec.md §6`, D6.2). It mirrors the
/// decimals-rewrite safety gate: only single-section codes with an integer `0` digit placeholder and
/// no exponent / quoted / escaped literal are touched, so General, Text, the canonical Date/Time
/// codes, Scientific, and multi-section customs (including the red-negative Number preset) all
/// return `None` and the button disables.
///
/// FreeCell's numeric presets carry grouping as the canonical `#,##0` integer placeholder, so
/// removal strips that back to a bare `0` (`$#,##0.00` → `$0.00`) and addition inserts `#,##` before
/// the first integer `0` (`0.00%` → `#,##0.00%`). The dropdown-native numeric codes round-trip
/// exactly through repeated toggles.
pub fn toggle_thousands(code: &str) -> Option<String> {
    if !is_decimals_adjustable(code) || !code.contains('0') {
        return None;
    }
    if code.contains("#,##0") {
        return Some(code.replacen("#,##0", "0", 1));
    }
    // Add grouping: prefix the integer digit placeholder (the first `0`, which precedes any decimal
    // point) with `#,##`. `find('0')` is `Some` here (the gate required a `0`).
    let idx = code.find('0')?;
    let mut out = String::with_capacity(code.len() + 3);
    out.push_str(&code[..idx]);
    out.push_str("#,##0");
    out.push_str(&code[idx + 1..]);
    Some(out)
}

/// Whether the last-`0`-group decimals heuristic can safely rewrite `code`. It bails on the shapes it
/// can't edit correctly: multi-section codes (a top-level-or-nested `;` — Excel adjusts *all*
/// sections, we only see the last group), scientific notation (`E`/`e` — the group is the mantissa,
/// not the exponent `0`s), and any quoted (`"`) or backslash-escaped (`\`) literal (its `0`s aren't
/// placeholders). The dropdown-native numeric formats (`#,##0.00`, `$#,##0.00`, `0.00%`, thousands)
/// carry none of these, so the common path is unaffected.
fn is_decimals_adjustable(code: &str) -> bool {
    !code.contains(';')
        && !code.contains('"')
        && !code.contains('\\')
        && !code.contains('E')
        && !code.contains('e')
}

/// Locates the **last** `0(\.0+)?` group in `code`: a `0` byte optionally followed by `.` and one or
/// more `0` bytes. Returns `(start, end, decimals)` — the byte span of the whole group and the count
/// of `0`s after the `.` (`0` when the group is a bare `0`). `None` when the code has no `0`.
///
/// Number-format codes are ASCII in every case FreeCell produces or reads through its dropdown, so
/// byte scanning is safe; a stray multibyte char in a hostile file's code simply won't match `0`.
fn last_zero_group(code: &str) -> Option<(usize, usize, usize)> {
    let bytes = code.as_bytes();
    let mut best: Option<(usize, usize, usize)> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'0' {
            let start = i;
            let mut end = i + 1;
            let mut decimals = 0;
            if end < bytes.len() && bytes[end] == b'.' {
                let mut k = end + 1;
                while k < bytes.len() && bytes[k] == b'0' {
                    k += 1;
                }
                let count = k - (end + 1);
                if count > 0 {
                    end = k;
                    decimals = count;
                }
            }
            best = Some((start, end, decimals));
            i = end;
        } else {
            i += 1;
        }
    }
    best
}

/// Whether `code` is the engine's General format (or the empty/absent code) — the default
/// numeric display that carries no `0` placeholder, so [`adjust_decimals`] treats it as a no-op.
fn is_general(code: &str) -> bool {
    let t = code.trim();
    t.is_empty() || t.eq_ignore_ascii_case("general")
}

/// The number-format code for a plain number with `n` decimal places: `0` → `"0"`, `2` → `"0.00"`.
fn decimals_code(n: u8) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut s = String::with_capacity(2 + n as usize);
    s.push_str("0.");
    for _ in 0..n {
        s.push('0');
    }
    s
}

/// The count of fractional digits a **plain decimal** number displays, for the General-format
/// decimals ± entry point: `"200000"` → `Some(0)`, `"200000.5"` → `Some(1)`, `"-1,234.50"` →
/// `Some(2)`. Returns `None` for any text that isn't a plain decimal — scientific notation
/// (`"1E+20"`), error strings, booleans — so the ± stays disabled on a General cell whose value
/// can't be cleanly re-expressed as `0.0…` (`components/action_bar.md §Command emission`).
///
/// A leading sign is allowed and thousands separators (`,`) are tolerated (General never emits
/// one, but a robust scan shouldn't reject one); anything else makes it not-a-plain-number.
pub fn displayed_decimals(text: &str) -> Option<u8> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let mut seen_dot = false;
    let mut frac: u8 = 0;
    let mut any_digit = false;
    for &b in body.as_bytes() {
        match b {
            b'0'..=b'9' => {
                any_digit = true;
                if seen_dot {
                    frac = frac.checked_add(1)?;
                }
            }
            b'.' if !seen_dot => seen_dot = true,
            b',' if !seen_dot => {} // thousands separator — ignore
            _ => return None,       // scientific `E`, currency symbol, letters → not a plain number
        }
    }
    any_digit.then_some(frac)
}

/// The decimals ±1 result for a specific **cell**, layered on [`adjust_decimals`]: a format that
/// already carries an editable decimal group (Number/Currency/Percent/thousands, or a bare `0`)
/// is rewritten exactly as before, independent of the cell's kind. The **new** case this adds is
/// a *numeric* cell shown under **General** — Excel enables ± on a plain number by switching it to
/// a `0.0…` format (`200000` → increase → `"0.0"`; a General value already showing N decimals
/// starts from `0.{N}`). `numeric` gates this to real number cells (text / date / bool / error /
/// empty → `false` → stays disabled), and `displayed` (the cell's shown decimal count, `None` for
/// a non-plain display like scientific) gates the entry point. The unsafe custom formats
/// [`adjust_decimals`] already refuses (multi-section / exponent / quoted) are **not** General, so
/// they stay gated off here too.
pub fn adjust_decimals_cell(
    code: &str,
    delta: i8,
    numeric: bool,
    displayed: Option<u8>,
) -> Option<String> {
    if let Some(next) = adjust_decimals(code, delta) {
        return Some(next);
    }
    if numeric && is_general(code) {
        let displayed = displayed?;
        let next = match delta.cmp(&0) {
            std::cmp::Ordering::Greater => displayed.checked_add(1)?,
            std::cmp::Ordering::Less => displayed.checked_sub(1)?, // 0 decimals → None (disabled)
            std::cmp::Ordering::Equal => return None,
        };
        return Some(decimals_code(next));
    }
    None
}

/// The size-dropdown display for a quarter-point font size (`components/action_bar.md`): `0` → the
/// engine default `"11"`; otherwise `q/4` with a trailing `.0` trimmed (e.g. `48` → `"12"`,
/// `46` → `"11.5"`).
pub fn font_size_display(q: u16) -> String {
    if q == 0 {
        return "11".to_string();
    }
    format!("{}", q as f32 / 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_exact_matches_across_groups() {
        assert_eq!(num_fmt_category("general"), Category::General);
        assert_eq!(num_fmt_category("General"), Category::General); // engine may echo either case
        assert_eq!(num_fmt_category("0.00"), Category::Number);
        assert_eq!(num_fmt_category("#,##0.00"), Category::Number);
        assert_eq!(num_fmt_category("#,##0"), Category::Number);
        assert_eq!(num_fmt_category("$#,##0.00"), Category::Currency);
        assert_eq!(num_fmt_category("€#,##0.00"), Category::Currency);
        // `£`/`¥` use the engine's bracketed locale-currency form.
        assert_eq!(num_fmt_category("[$£]#,##0.00"), Category::Currency);
        assert_eq!(num_fmt_category("[$¥]#,##0.00"), Category::Currency);
        assert_eq!(
            num_fmt_category("$#,##0.00;($#,##0.00)"),
            Category::Accounting
        );
        assert_eq!(num_fmt_category("0.00%"), Category::Percent);
        assert_eq!(num_fmt_category("0%"), Category::Percent);
        assert_eq!(num_fmt_category("m/d/yyyy"), Category::Date);
        assert_eq!(num_fmt_category("yyyy-mm-dd"), Category::Date); // now a Date preset
        assert_eq!(num_fmt_category("h:mm AM/PM"), Category::Time);
        assert_eq!(num_fmt_category("[h]:mm:ss"), Category::Time);
        assert_eq!(num_fmt_category("0.00E+00"), Category::Scientific);
        assert_eq!(num_fmt_category("@"), Category::Text);
    }

    #[test]
    fn category_custom_fallback() {
        // A richer file-authored format that is not one of our preset codes → Custom.
        assert_eq!(
            num_fmt_category("$#,##0.00;[Red]$#,##0.00"),
            Category::Custom
        );
        assert_eq!(num_fmt_category("0.000"), Category::Custom); // 3 decimals is not a preset
        assert_eq!(num_fmt_category("mm/dd/yy"), Category::Custom);
    }

    #[test]
    fn basic_formats_are_the_original_seven() {
        // The pre-Phase-6 `DROPDOWN_FORMATS` set, recovered verbatim (commit 382f075^): exactly
        // these seven codes in this order (`functional_spec.md §10.1`).
        let codes: Vec<&str> = BASIC_FORMATS.iter().map(|p| p.code).collect();
        assert_eq!(
            codes,
            [
                "general",
                "#,##0.00",
                "$#,##0.00",
                "0.00%",
                "m/d/yyyy",
                "h:mm AM/PM",
                "@",
            ]
        );
    }

    #[test]
    fn every_basic_code_reverse_maps_through_num_fmt_category() {
        // The basic set is a curated subset of `NUM_FMT_GROUPS`, so each basic preset must
        // reverse-map to a real (non-Custom) category — the highlight is consistent across levels.
        for preset in BASIC_FORMATS {
            assert_ne!(
                num_fmt_category(preset.code),
                Category::Custom,
                "basic preset {:?} ({}) must reverse-map to a real category",
                preset.label,
                preset.code
            );
            assert!(
                is_basic_num_fmt(preset.code),
                "basic preset {} must be classified basic",
                preset.code
            );
            assert!(
                !is_more_only_num_fmt(preset.code),
                "basic preset {} must not be classified more-only",
                preset.code
            );
        }
    }

    #[test]
    fn is_basic_vs_more_only_partitions_recognized_presets() {
        // Basic active codes → basic, not more-only (General matches either case).
        assert!(is_basic_num_fmt("general"));
        assert!(is_basic_num_fmt("General"));
        assert!(is_basic_num_fmt("$#,##0.00"));
        assert!(!is_more_only_num_fmt("$#,##0.00"));
        // Recognized presets that live only under "More ▸" → more-only, not basic.
        assert!(!is_basic_num_fmt("0.00E+00"));
        assert!(is_more_only_num_fmt("0.00E+00")); // Scientific
        assert!(is_more_only_num_fmt("yyyy-mm-dd")); // a non-basic Date preset
        assert!(is_more_only_num_fmt("$#,##0.00;($#,##0.00)")); // Accounting
                                                                // A Custom code (matches no preset) is neither basic nor more-only.
        assert!(!is_basic_num_fmt("0.000"));
        assert!(!is_more_only_num_fmt("0.000"));
    }

    #[test]
    fn every_preset_code_reverse_maps_to_its_group_category() {
        // Structural invariant: the popover highlights by exact code, so each preset must reverse-map
        // through `num_fmt_category` to the category of the group it lives in (no cross-group collision).
        for group in NUM_FMT_GROUPS {
            for preset in group.presets {
                assert_eq!(
                    num_fmt_category(preset.code),
                    group.category,
                    "preset {:?} ({}) must map to {:?}",
                    preset.label,
                    preset.code,
                    group.category
                );
            }
        }
    }

    #[test]
    fn toggle_thousands_adds_and_removes() {
        // Round-trips for every dropdown-native numeric shape.
        assert_eq!(toggle_thousands("0.00").as_deref(), Some("#,##0.00"));
        assert_eq!(toggle_thousands("#,##0.00").as_deref(), Some("0.00"));
        assert_eq!(toggle_thousands("0").as_deref(), Some("#,##0"));
        assert_eq!(toggle_thousands("#,##0").as_deref(), Some("0"));
        assert_eq!(toggle_thousands("$0.00").as_deref(), Some("$#,##0.00"));
        assert_eq!(toggle_thousands("$#,##0.00").as_deref(), Some("$0.00"));
        assert_eq!(toggle_thousands("0.00%").as_deref(), Some("#,##0.00%"));
        assert_eq!(toggle_thousands("#,##0.00%").as_deref(), Some("0.00%"));
        // A multibyte currency symbol keeps its prefix intact (byte-safe rewrite).
        assert_eq!(toggle_thousands("€0.00").as_deref(), Some("€#,##0.00"));
        assert_eq!(toggle_thousands("€#,##0.00").as_deref(), Some("€0.00"));
        // The bracketed locale-currency form (`£`/`¥`) toggles too — the `[`/`]` don't block it.
        assert_eq!(
            toggle_thousands("[$£]#,##0.00").as_deref(),
            Some("[$£]0.00")
        );
        assert_eq!(
            toggle_thousands("[$£]0.00").as_deref(),
            Some("[$£]#,##0.00")
        );
    }

    #[test]
    fn toggle_thousands_gated_off() {
        // No integer digit placeholder / unsafe shapes → the toggle disables (returns None).
        assert_eq!(toggle_thousands("general"), None);
        assert_eq!(toggle_thousands("@"), None);
        assert_eq!(toggle_thousands("m/d/yyyy"), None);
        assert_eq!(toggle_thousands("h:mm AM/PM"), None);
        assert_eq!(toggle_thousands("[h]:mm:ss"), None);
        assert_eq!(toggle_thousands("0.00E+00"), None); // exponent
                                                        // Multi-section (the red-negative Number preset): can't be safely edited section-by-section.
        assert_eq!(toggle_thousands("#,##0.00;[Red]-#,##0.00"), None);
    }

    #[test]
    fn adjust_decimals_adds_and_removes() {
        assert_eq!(adjust_decimals("#,##0.00", 1).as_deref(), Some("#,##0.000"));
        assert_eq!(adjust_decimals("0.0", -1).as_deref(), Some("0"));
        assert_eq!(adjust_decimals("0", 1).as_deref(), Some("0.0"));
        // A whole-number format with no decimals grows a decimal group.
        assert_eq!(adjust_decimals("#,##0", 1).as_deref(), Some("#,##0.0"));
    }

    #[test]
    fn adjust_decimals_noop_on_general_text_date_time() {
        // None of these canonical codes carry a `0` placeholder → no adjustable group.
        assert_eq!(adjust_decimals("general", 1), None);
        assert_eq!(adjust_decimals("@", 1), None);
        assert_eq!(adjust_decimals("m/d/yyyy", 1), None);
        assert_eq!(adjust_decimals("h:mm AM/PM", -1), None);
        // Decrease at zero decimals is a no-op (min zero).
        assert_eq!(adjust_decimals("0", -1), None);
    }

    #[test]
    fn adjust_decimals_gated_off_for_unsafe_custom_formats() {
        // Scientific: the last `0` group is the exponent, not the mantissa → don't touch it.
        assert_eq!(adjust_decimals("0.00E+00", 1), None);
        assert_eq!(adjust_decimals("0.00e+00", -1), None);
        // Quoted literal: its `0` is text, not a placeholder.
        assert_eq!(adjust_decimals("0.0\"0km\"", 1), None);
        // Backslash-escaped literal likewise.
        assert_eq!(adjust_decimals("0.0\\0", 1), None);
        // Multi-section: Excel adjusts every section; the heuristic only sees the last → refuse.
        assert_eq!(adjust_decimals("#,##0.00;(#,##0.00)", 1), None);
        assert_eq!(adjust_decimals("0.00;[Red]0.00", -1), None);
        // The clean single-section dropdown-native formats are still adjustable.
        assert_eq!(adjust_decimals("#,##0.00", 1).as_deref(), Some("#,##0.000"));
    }

    #[test]
    fn adjust_decimals_currency_keeps_prefix_and_percent_keeps_suffix() {
        assert_eq!(
            adjust_decimals("$#,##0.00", -1).as_deref(),
            Some("$#,##0.0")
        );
        assert_eq!(
            adjust_decimals("$#,##0.00", 1).as_deref(),
            Some("$#,##0.000")
        );
        assert_eq!(adjust_decimals("0.00%", 1).as_deref(), Some("0.000%"));
        assert_eq!(adjust_decimals("0.00%", -1).as_deref(), Some("0.0%"));
    }

    #[test]
    fn displayed_decimals_counts_fraction_of_plain_numbers_only() {
        assert_eq!(displayed_decimals("200000"), Some(0));
        assert_eq!(displayed_decimals("200000.5"), Some(1));
        assert_eq!(displayed_decimals("-1,234.50"), Some(2));
        assert_eq!(displayed_decimals("0"), Some(0));
        // Not plain decimals → the ± must stay disabled.
        assert_eq!(displayed_decimals(""), None);
        assert_eq!(displayed_decimals("1E+20"), None); // scientific
        assert_eq!(displayed_decimals("#DIV/0!"), None); // error text
        assert_eq!(displayed_decimals("hello"), None);
        assert_eq!(displayed_decimals("TRUE"), None);
    }

    #[test]
    fn adjust_decimals_cell_enables_general_numeric() {
        // `200000` is a General-formatted number (GAPS bug): increase enables → `0.0`; decrease is
        // a no-op at zero decimals (disabled).
        assert_eq!(
            adjust_decimals_cell("general", 1, true, Some(0)).as_deref(),
            Some("0.0")
        );
        assert_eq!(adjust_decimals_cell("general", -1, true, Some(0)), None);
        // A General number already showing decimals starts from its displayed count.
        assert_eq!(
            adjust_decimals_cell("general", 1, true, Some(2)).as_deref(),
            Some("0.000")
        );
        assert_eq!(
            adjust_decimals_cell("general", -1, true, Some(2)).as_deref(),
            Some("0.0")
        );
        assert_eq!(
            adjust_decimals_cell("general", -1, true, Some(1)).as_deref(),
            Some("0")
        );
    }

    #[test]
    fn adjust_decimals_cell_disabled_for_nonnumeric_and_nonplain() {
        // A text cell under General is never adjustable, either direction.
        assert_eq!(adjust_decimals_cell("general", 1, false, Some(0)), None);
        assert_eq!(adjust_decimals_cell("general", -1, false, Some(0)), None);
        // A numeric General cell whose display isn't a plain decimal (scientific) → disabled.
        assert_eq!(adjust_decimals_cell("general", 1, true, None), None);
    }

    #[test]
    fn adjust_decimals_cell_preserves_existing_format_gating() {
        // Real numeric formats behave exactly as `adjust_decimals`, kind-independent.
        assert_eq!(
            adjust_decimals_cell("#,##0.00", 1, true, None).as_deref(),
            Some("#,##0.000")
        );
        assert_eq!(
            adjust_decimals_cell("0.00%", -1, false, None).as_deref(),
            Some("0.0%")
        );
        // The unsafe custom formats stay gated off even for a numeric cell (they aren't General).
        assert_eq!(adjust_decimals_cell("0.00E+00", 1, true, Some(2)), None);
        assert_eq!(
            adjust_decimals_cell("#,##0.00;(#,##0.00)", 1, true, Some(2)),
            None
        );
    }

    #[test]
    fn font_size_display_default_and_halves() {
        assert_eq!(font_size_display(0), "11"); // engine default
        assert_eq!(font_size_display(44), "11"); // 11.0 pt trims .0
        assert_eq!(font_size_display(48), "12");
        assert_eq!(font_size_display(144), "36");
        assert_eq!(font_size_display(46), "11.5"); // a half point keeps its fraction
    }
}
