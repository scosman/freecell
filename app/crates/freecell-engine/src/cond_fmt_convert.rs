//! `cond_fmt_convert` — the IronCalc ↔ engine-free conditional-formatting conversions.
//!
//! The single place a CF-related IronCalc type is mapped to/from FreeCell's engine-free
//! [`freecell_core::cond_fmt`] vocabulary, so no `ironcalc` type ever leaks past
//! [`WorkbookDocument`](crate::WorkbookDocument) (`architecture.md §4.3`,
//! `components/engine_cf.md §3`). Every function here is pure; the `WorkbookDocument` CF methods
//! (`document::cond_fmt`) call them around the `UserModel` CF API.
//!
//! Field/variant names are matched against the pinned fork (`scosman/ironcalc#freecell-fixes`):
//! `cf_types::{CfRule, CfRuleInput, ValueOperator, TextOperator, PeriodType, Cfvo,
//! ColorScaleThreshold}` and `types::{Color, Dxf, DxfFont, Fill}`.
//!
//! These conversions are reached from the `WorkbookDocument` CF methods, which P2 wires to the
//! worker `Command` dispatch + the published CF map — so the whole seam is now consumed in a release
//! build (no module-level dead-code allow needed).

use freecell_core::cond_fmt::{
    CfColorStop, CfFormat, CfPeriod, CfPreview, CfRuleSpec, CfRuleView, CfTextOp, CfThresholdKind,
    CfValueOp,
};
use freecell_core::Rgb;

use ironcalc_base::cf_types::{
    CfRule, CfRuleInput, Cfvo, ColorScaleThreshold, PeriodType, TextOperator, ValueOperator,
};
use ironcalc_base::types::{Color, Dxf, DxfFont, Fill};

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// An [`Rgb`] as an IronCalc `#RRGGBB` colour (the form the engine validates for styles set
/// through it).
pub(crate) fn rgb_to_color(rgb: Rgb) -> Color {
    Color::Rgb(format!("#{:06X}", rgb.to_hex()))
}

/// An IronCalc [`Color`] as an [`Rgb`], or `None` for a theme-indexed / absent colour (the CF
/// format editor only writes concrete `#RRGGBB` colours). Reuses the cache's `#RRGGBB`/`#AARRGGBB`
/// parser so it agrees with the resident style cache.
pub(crate) fn color_to_rgb(color: &Color) -> Option<Rgb> {
    match color {
        Color::Rgb(s) => crate::cache::parse_color(s),
        Color::Theme(..) | Color::None => None,
    }
}

// ---------------------------------------------------------------------------
// CfFormat ↔ Dxf
// ---------------------------------------------------------------------------

/// A [`CfFormat`] as a fresh IronCalc [`Dxf`] (the add path). Only the four modeled attributes are
/// written; `border`/`num_fmt`/`alignment` stay `None`, and the font is omitted entirely when no
/// font attribute is set — so a fill-only rule carries no font record (`functional_spec.md §4`).
pub(crate) fn cf_format_to_dxf(fmt: &CfFormat) -> Dxf {
    Dxf {
        font: dxf_font_from(fmt, None, None, None),
        fill: fmt.fill.map(|rgb| Fill {
            color: rgb_to_color(rgb),
        }),
        border: None,
        num_fmt: None,
        alignment: None,
    }
}

/// Folds a [`CfFormat`] onto an **existing** [`Dxf`] (the update path). The fill and the modeled
/// font attributes (`b`/`i`/`color`) are taken from the new format; the unmodeled font attributes
/// (`strike`/`u`/`sz`) and the whole `border`/`num_fmt`/`alignment` are **preserved**, so an edit
/// through the first-pass editor never drops a `Dxf` field it doesn't expose (`functional_spec.md
/// §4`, `components/engine_cf.md §3`). The font record is dropped only when nothing — neither a
/// modeled nor a preserved attribute — remains.
pub(crate) fn merge_cf_format_into_dxf(fmt: &CfFormat, mut existing: Dxf) -> Dxf {
    existing.fill = fmt.fill.map(|rgb| Fill {
        color: rgb_to_color(rgb),
    });
    let (strike, u, sz) = existing
        .font
        .as_ref()
        .map(|f| (f.strike, f.u, f.sz))
        .unwrap_or_default();
    existing.font = dxf_font_from(fmt, strike, u, sz);
    existing
}

/// Builds the [`DxfFont`] for a format, carrying forward any preserved `strike`/`u`/`sz`. Returns
/// `None` when neither a modeled attribute (`bold`/`italic`/`text_color`) nor a preserved one is
/// present, so a format with no font intent produces no font record.
fn dxf_font_from(
    fmt: &CfFormat,
    strike: Option<bool>,
    u: Option<bool>,
    sz: Option<i32>,
) -> Option<DxfFont> {
    let b = fmt.bold.then_some(true);
    let i = fmt.italic.then_some(true);
    let has_attr = b.is_some()
        || i.is_some()
        || fmt.text_color.is_some()
        || strike.is_some()
        || u.is_some()
        || sz.is_some();
    has_attr.then(|| DxfFont {
        strike,
        u,
        b,
        i,
        sz,
        color: fmt.text_color.map(rgb_to_color).unwrap_or(Color::None),
    })
}

/// Reads an IronCalc [`Dxf`] back into a [`CfFormat`] (seeds the editor / builds the preview) —
/// the modeled subset only; unmodeled attributes are ignored here (they still round-trip in the
/// engine via [`merge_cf_format_into_dxf`]).
pub(crate) fn dxf_to_cf_format(dxf: &Dxf) -> CfFormat {
    CfFormat {
        fill: dxf.fill.as_ref().and_then(|f| color_to_rgb(&f.color)),
        text_color: dxf.font.as_ref().and_then(|f| color_to_rgb(&f.color)),
        bold: dxf.font.as_ref().and_then(|f| f.b) == Some(true),
        italic: dxf.font.as_ref().and_then(|f| f.i) == Some(true),
    }
}

// ---------------------------------------------------------------------------
// Operator / period enums
// ---------------------------------------------------------------------------

fn value_op_to_ironcalc(op: CfValueOp) -> ValueOperator {
    match op {
        CfValueOp::Gt => ValueOperator::GreaterThan,
        CfValueOp::Lt => ValueOperator::LessThan,
        CfValueOp::Ge => ValueOperator::GreaterThanOrEqual,
        CfValueOp::Le => ValueOperator::LessThanOrEqual,
        CfValueOp::Eq => ValueOperator::Equal,
        CfValueOp::Ne => ValueOperator::NotEqual,
        CfValueOp::Between => ValueOperator::Between,
        CfValueOp::NotBetween => ValueOperator::NotBetween,
    }
}

fn value_op_from_ironcalc(op: &ValueOperator) -> CfValueOp {
    match op {
        ValueOperator::GreaterThan => CfValueOp::Gt,
        ValueOperator::LessThan => CfValueOp::Lt,
        ValueOperator::GreaterThanOrEqual => CfValueOp::Ge,
        ValueOperator::LessThanOrEqual => CfValueOp::Le,
        ValueOperator::Equal => CfValueOp::Eq,
        ValueOperator::NotEqual => CfValueOp::Ne,
        ValueOperator::Between => CfValueOp::Between,
        ValueOperator::NotBetween => CfValueOp::NotBetween,
    }
}

fn text_op_to_ironcalc(op: CfTextOp) -> TextOperator {
    match op {
        CfTextOp::Contains => TextOperator::Contains,
        CfTextOp::NotContains => TextOperator::DoesNotContain,
        CfTextOp::BeginsWith => TextOperator::BeginsWith,
        CfTextOp::EndsWith => TextOperator::EndsWith,
        CfTextOp::Equals => TextOperator::Equals,
    }
}

fn text_op_from_ironcalc(op: &TextOperator) -> CfTextOp {
    match op {
        TextOperator::Contains => CfTextOp::Contains,
        TextOperator::DoesNotContain => CfTextOp::NotContains,
        TextOperator::BeginsWith => CfTextOp::BeginsWith,
        TextOperator::EndsWith => CfTextOp::EndsWith,
        TextOperator::Equals => CfTextOp::Equals,
    }
}

fn period_to_ironcalc(period: CfPeriod) -> PeriodType {
    match period {
        CfPeriod::Today => PeriodType::Today,
        CfPeriod::Yesterday => PeriodType::Yesterday,
        CfPeriod::Tomorrow => PeriodType::Tomorrow,
        CfPeriod::Last7Days => PeriodType::Last7Days,
        CfPeriod::LastWeek => PeriodType::LastWeek,
        CfPeriod::ThisWeek => PeriodType::ThisWeek,
        CfPeriod::NextWeek => PeriodType::NextWeek,
        CfPeriod::LastMonth => PeriodType::LastMonth,
        CfPeriod::ThisMonth => PeriodType::ThisMonth,
        CfPeriod::NextMonth => PeriodType::NextMonth,
        CfPeriod::LastYear => PeriodType::LastYear,
        CfPeriod::ThisYear => PeriodType::ThisYear,
        CfPeriod::NextYear => PeriodType::NextYear,
    }
}

/// Maps an IronCalc [`PeriodType`] back to a first-pass [`CfPeriod`], or `None` for a deferred
/// date-range variant (`Between`/`NotBetween`) or `Next7Days` (`functional_spec.md §9`).
fn period_from_ironcalc(period: &PeriodType) -> Option<CfPeriod> {
    Some(match period {
        PeriodType::Today => CfPeriod::Today,
        PeriodType::Yesterday => CfPeriod::Yesterday,
        PeriodType::Tomorrow => CfPeriod::Tomorrow,
        PeriodType::Last7Days => CfPeriod::Last7Days,
        PeriodType::LastWeek => CfPeriod::LastWeek,
        PeriodType::ThisWeek => CfPeriod::ThisWeek,
        PeriodType::NextWeek => CfPeriod::NextWeek,
        PeriodType::LastMonth => CfPeriod::LastMonth,
        PeriodType::ThisMonth => CfPeriod::ThisMonth,
        PeriodType::NextMonth => CfPeriod::NextMonth,
        PeriodType::LastYear => CfPeriod::LastYear,
        PeriodType::ThisYear => CfPeriod::ThisYear,
        PeriodType::NextYear => CfPeriod::NextYear,
        PeriodType::Between | PeriodType::NotBetween | PeriodType::Next7Days => return None,
    })
}

// ---------------------------------------------------------------------------
// Color-scale thresholds
// ---------------------------------------------------------------------------

fn color_stop_to_threshold(stop: &CfColorStop) -> ColorScaleThreshold {
    let cfvo = match stop.kind {
        CfThresholdKind::Min => Cfvo::Min,
        CfThresholdKind::Max => Cfvo::Max,
        CfThresholdKind::Number => Cfvo::Number(stop.value.unwrap_or(0.0)),
        CfThresholdKind::Percent => Cfvo::Percent(stop.value.unwrap_or(0.0)),
        CfThresholdKind::Percentile => Cfvo::Percentile(stop.value.unwrap_or(0.0)),
    };
    ColorScaleThreshold {
        cfvo,
        color: rgb_to_color(stop.color),
    }
}

/// Reads an IronCalc [`Cfvo`] into a first-pass (kind, value), or `None` for a deferred
/// `Formula` threshold (`functional_spec.md §9`).
fn cfvo_to_kind_value(cfvo: &Cfvo) -> Option<(CfThresholdKind, Option<f64>)> {
    Some(match cfvo {
        Cfvo::Min => (CfThresholdKind::Min, None),
        Cfvo::Max => (CfThresholdKind::Max, None),
        Cfvo::Number(v) => (CfThresholdKind::Number, Some(*v)),
        Cfvo::Percent(v) => (CfThresholdKind::Percent, Some(*v)),
        Cfvo::Percentile(v) => (CfThresholdKind::Percentile, Some(*v)),
        Cfvo::Formula(_) => return None,
    })
}

/// Reconstructs the [`CfColorStop`]s of a color scale, or `None` if the scale is non-authorable
/// this pass — either a deferred `Formula` cfvo, or a **non-RGB stop colour** (`Color::Theme`/
/// `Color::None`, e.g. from an imported file). Returning `None` for a theme colour is load-bearing:
/// coercing it to a concrete `Rgb` here would let a later edit+save overwrite the file's original
/// theme colours, so such a scale is surfaced read-only (Badge) instead (`functional_spec.md §9`).
fn color_scale_stops(thresholds: &[ColorScaleThreshold]) -> Option<Vec<CfColorStop>> {
    thresholds
        .iter()
        .map(|t| {
            let (kind, value) = cfvo_to_kind_value(&t.cfvo)?;
            let color = color_to_rgb(&t.color)?;
            Some(CfColorStop { kind, value, color })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// CfRuleSpec → CfRuleInput
// ---------------------------------------------------------------------------

/// A [`CfRuleSpec`] (plus its already-converted [`Dxf`]) as an IronCalc [`CfRuleInput`] — 1:1 by
/// variant (`architecture.md §4.3`). `ColorScale` ignores `dxf`; the boolean sub-variants fan out
/// (`Top{bottom}`→`Top10`/`Bottom10`, `Average{below}`→`Above`/`BelowAverage`,
/// `DuplicateValues{unique}`→`Unique`/`DuplicateValues`, `Blanks{no_blanks}`→`NotBlanks`/`Blanks`,
/// `Errors{no_errors}`→`NoErrors`/`Errors`).
pub(crate) fn cf_rule_spec_to_input(spec: &CfRuleSpec, dxf: Dxf) -> CfRuleInput {
    match spec {
        CfRuleSpec::CellIs {
            op,
            operand,
            operand2,
            stop_if_true,
            ..
        } => CfRuleInput::CellIs {
            operator: value_op_to_ironcalc(*op),
            formula: operand.clone(),
            formula2: operand2.clone(),
            format: dxf,
            stop_if_true: *stop_if_true,
        },
        CfRuleSpec::Text {
            op,
            value,
            stop_if_true,
            ..
        } => CfRuleInput::Text {
            operator: text_op_to_ironcalc(*op),
            value: value.clone(),
            format: dxf,
            stop_if_true: *stop_if_true,
        },
        CfRuleSpec::TimePeriod {
            period,
            stop_if_true,
            ..
        } => CfRuleInput::TimePeriod {
            time_period: period_to_ironcalc(*period),
            date1: None,
            date2: None,
            format: dxf,
            stop_if_true: *stop_if_true,
        },
        CfRuleSpec::Top {
            rank,
            percent,
            bottom,
            stop_if_true,
            ..
        } => {
            if *bottom {
                CfRuleInput::Bottom10 {
                    rank: *rank,
                    percent: *percent,
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            } else {
                CfRuleInput::Top10 {
                    rank: *rank,
                    percent: *percent,
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            }
        }
        CfRuleSpec::Average {
            below,
            stop_if_true,
            ..
        } => {
            if *below {
                CfRuleInput::BelowAverage {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            } else {
                CfRuleInput::AboveAverage {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            }
        }
        CfRuleSpec::DuplicateValues {
            unique,
            stop_if_true,
            ..
        } => {
            if *unique {
                CfRuleInput::UniqueValues {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            } else {
                CfRuleInput::DuplicateValues {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            }
        }
        CfRuleSpec::Blanks {
            no_blanks,
            stop_if_true,
            ..
        } => {
            if *no_blanks {
                CfRuleInput::NotBlanks {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            } else {
                CfRuleInput::Blanks {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            }
        }
        CfRuleSpec::Errors {
            no_errors,
            stop_if_true,
            ..
        } => {
            if *no_errors {
                CfRuleInput::NoErrors {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            } else {
                CfRuleInput::Errors {
                    format: dxf,
                    stop_if_true: *stop_if_true,
                }
            }
        }
        CfRuleSpec::Formula {
            formula,
            stop_if_true,
            ..
        } => CfRuleInput::Formula {
            formula: formula.clone(),
            format: dxf,
            stop_if_true: *stop_if_true,
        },
        CfRuleSpec::ColorScale { stops } => CfRuleInput::ColorScale {
            thresholds: stops.iter().map(color_stop_to_threshold).collect(),
        },
    }
}

// ---------------------------------------------------------------------------
// CfRule → CfRuleView
// ---------------------------------------------------------------------------

/// Maps a stored IronCalc [`CfRule`] (and its fetched [`Dxf`], `None` for non-dxf variants) into
/// the list read model (`components/engine_cf.md §3`): authorable rules get `editable:true`, a
/// human `summary`, a highlight/gradient `preview`, and a reconstructed `spec`; deferred families
/// (`DataBar`/`IconSet`/`IconRating`) and deferred variants (a `TimePeriod`
/// `Between`/`NotBetween`/`Next7Days`, a `ColorScale` with a `Formula` threshold or a non-RGB
/// theme stop colour) get `editable:false`, a [`CfPreview::Badge`], and no `spec`.
pub(crate) fn cf_rule_to_view(
    index: u32,
    range: String,
    priority: u32,
    rule: &CfRule,
    dxf: Option<Dxf>,
) -> CfRuleView {
    match rule {
        CfRule::CellIs {
            operator,
            formula,
            formula2,
            stop_if_true,
            ..
        } => {
            let op = value_op_from_ironcalc(operator);
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                cell_is_summary(op, formula, formula2.as_deref()),
                format,
                CfRuleSpec::CellIs {
                    op,
                    operand: formula.clone(),
                    operand2: formula2.clone(),
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::Text {
            operator,
            value,
            stop_if_true,
            ..
        } => {
            let op = text_op_from_ironcalc(operator);
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                format!("Text {} \"{value}\"", text_op_label(op)),
                format,
                CfRuleSpec::Text {
                    op,
                    value: value.clone(),
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::TimePeriod {
            time_period,
            stop_if_true,
            ..
        } => match period_from_ironcalc(time_period) {
            Some(period) => {
                let format = dxf_to_cf_format(&dxf.unwrap_or_default());
                highlight_view(
                    index,
                    range,
                    priority,
                    format!("Date: {}", period_label(period)),
                    format,
                    CfRuleSpec::TimePeriod {
                        period,
                        format,
                        stop_if_true: *stop_if_true,
                    },
                )
            }
            None => badge_view(index, range, priority, "Date range".to_string()),
        },
        CfRule::Top10 {
            rank,
            percent,
            stop_if_true,
            ..
        } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                format!("Top {rank}{}", if *percent { "%" } else { "" }),
                format,
                CfRuleSpec::Top {
                    rank: *rank,
                    percent: *percent,
                    bottom: false,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::Bottom10 {
            rank,
            percent,
            stop_if_true,
            ..
        } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                format!("Bottom {rank}{}", if *percent { "%" } else { "" }),
                format,
                CfRuleSpec::Top {
                    rank: *rank,
                    percent: *percent,
                    bottom: true,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::AboveAverage { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Above average".to_string(),
                format,
                CfRuleSpec::Average {
                    below: false,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::BelowAverage { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Below average".to_string(),
                format,
                CfRuleSpec::Average {
                    below: true,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::DuplicateValues { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Duplicate values".to_string(),
                format,
                CfRuleSpec::DuplicateValues {
                    unique: false,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::UniqueValues { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Unique values".to_string(),
                format,
                CfRuleSpec::DuplicateValues {
                    unique: true,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::Blanks { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Blank".to_string(),
                format,
                CfRuleSpec::Blanks {
                    no_blanks: false,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::NotBlanks { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "No blanks".to_string(),
                format,
                CfRuleSpec::Blanks {
                    no_blanks: true,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::Errors { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "Error".to_string(),
                format,
                CfRuleSpec::Errors {
                    no_errors: false,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::NoErrors { stop_if_true, .. } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                "No errors".to_string(),
                format,
                CfRuleSpec::Errors {
                    no_errors: true,
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::Formula {
            formula,
            stop_if_true,
            ..
        } => {
            let format = dxf_to_cf_format(&dxf.unwrap_or_default());
            highlight_view(
                index,
                range,
                priority,
                format!("Formula: {formula}"),
                format,
                CfRuleSpec::Formula {
                    formula: formula.clone(),
                    format,
                    stop_if_true: *stop_if_true,
                },
            )
        }
        CfRule::ColorScale { thresholds } => match color_scale_stops(thresholds) {
            Some(stops) => {
                let colors = stops.iter().map(|s| s.color).collect();
                let summary = format!("{}-color scale", stops.len());
                CfRuleView {
                    index,
                    range,
                    priority,
                    editable: true,
                    summary,
                    preview: CfPreview::ColorScale { colors },
                    spec: Some(CfRuleSpec::ColorScale { stops }),
                }
            }
            // A `Formula` threshold or a non-RGB (theme) stop colour makes the scale
            // non-authorable this pass — surfaced read-only so an edit can't overwrite it.
            None => badge_view(
                index,
                range,
                priority,
                format!("{}-color scale", thresholds.len()),
            ),
        },
        CfRule::DataBar { .. } => badge_view(index, range, priority, "Data bar".to_string()),
        CfRule::IconSet { .. } => badge_view(index, range, priority, "Icon set".to_string()),
        CfRule::IconRating { .. } => badge_view(index, range, priority, "Rating".to_string()),
    }
}

/// An authorable highlight row: `Highlight` preview from the format, `Some(spec)`, editable.
fn highlight_view(
    index: u32,
    range: String,
    priority: u32,
    summary: String,
    format: CfFormat,
    spec: CfRuleSpec,
) -> CfRuleView {
    CfRuleView {
        index,
        range,
        priority,
        editable: true,
        summary,
        preview: CfPreview::Highlight {
            fill: format.fill,
            text_color: format.text_color,
        },
        spec: Some(spec),
    }
}

/// A non-editable, delete-only row for a deferred family/variant: a `Badge` preview and no `spec`.
fn badge_view(index: u32, range: String, priority: u32, label: String) -> CfRuleView {
    CfRuleView {
        index,
        range,
        priority,
        editable: false,
        summary: label.clone(),
        preview: CfPreview::Badge(label),
        spec: None,
    }
}

fn cell_is_summary(op: CfValueOp, operand: &str, operand2: Option<&str>) -> String {
    match op {
        CfValueOp::Between => {
            format!(
                "Cell value between {operand} and {}",
                operand2.unwrap_or("")
            )
        }
        CfValueOp::NotBetween => {
            format!(
                "Cell value not between {operand} and {}",
                operand2.unwrap_or("")
            )
        }
        _ => format!("Cell value {} {operand}", value_op_symbol(op)),
    }
}

fn value_op_symbol(op: CfValueOp) -> &'static str {
    match op {
        CfValueOp::Gt => ">",
        CfValueOp::Lt => "<",
        CfValueOp::Ge => "≥",
        CfValueOp::Le => "≤",
        CfValueOp::Eq => "=",
        CfValueOp::Ne => "≠",
        // Between/NotBetween are handled by `cell_is_summary` directly.
        CfValueOp::Between => "between",
        CfValueOp::NotBetween => "not between",
    }
}

fn text_op_label(op: CfTextOp) -> &'static str {
    match op {
        CfTextOp::Contains => "contains",
        CfTextOp::NotContains => "does not contain",
        CfTextOp::BeginsWith => "begins with",
        CfTextOp::EndsWith => "ends with",
        CfTextOp::Equals => "equals",
    }
}

fn period_label(period: CfPeriod) -> &'static str {
    match period {
        CfPeriod::Today => "today",
        CfPeriod::Yesterday => "yesterday",
        CfPeriod::Tomorrow => "tomorrow",
        CfPeriod::Last7Days => "in the last 7 days",
        CfPeriod::LastWeek => "last week",
        CfPeriod::ThisWeek => "this week",
        CfPeriod::NextWeek => "next week",
        CfPeriod::LastMonth => "last month",
        CfPeriod::ThisMonth => "this month",
        CfPeriod::NextMonth => "next month",
        CfPeriod::LastYear => "last year",
        CfPeriod::ThisYear => "this year",
        CfPeriod::NextYear => "next year",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::cf_types::CfRule;
    use ironcalc_base::types::Border;

    fn red() -> Rgb {
        Rgb::from_hex(0xFF0000)
    }
    fn blue() -> Rgb {
        Rgb::from_hex(0x0000FF)
    }

    #[test]
    fn rgb_color_round_trip() {
        for hex in [0x000000u32, 0xFFFFFF, 0x4472C4, 0xED7D31] {
            let rgb = Rgb::from_hex(hex);
            assert_eq!(color_to_rgb(&rgb_to_color(rgb)), Some(rgb));
        }
        // Theme / None resolve to no concrete colour.
        assert_eq!(color_to_rgb(&Color::Theme(4, 0.0)), None);
        assert_eq!(color_to_rgb(&Color::None), None);
        // #AARRGGBB is tolerated (alpha dropped).
        assert_eq!(
            color_to_rgb(&Color::Rgb("#FFFF0000".to_string())),
            Some(red())
        );
    }

    #[test]
    fn cf_format_to_dxf_round_trip() {
        let fmt = CfFormat {
            fill: Some(red()),
            text_color: Some(blue()),
            bold: true,
            italic: false,
        };
        let dxf = cf_format_to_dxf(&fmt);
        assert_eq!(dxf_to_cf_format(&dxf), fmt);
        // Fill produces a Fill record; the font carries bold but not italic.
        assert!(dxf.fill.is_some());
        let font = dxf.font.unwrap();
        assert_eq!(font.b, Some(true));
        assert_eq!(font.i, None);
    }

    #[test]
    fn empty_format_has_no_font_or_fill() {
        let dxf = cf_format_to_dxf(&CfFormat::default());
        assert!(dxf.font.is_none(), "no font attrs → no font record");
        assert!(dxf.fill.is_none());
    }

    #[test]
    fn fill_only_format_has_no_font() {
        let dxf = cf_format_to_dxf(&CfFormat {
            fill: Some(red()),
            ..Default::default()
        });
        assert!(dxf.fill.is_some());
        assert!(dxf.font.is_none());
    }

    #[test]
    fn merge_preserves_unmodeled_dxf_fields() {
        // An existing dxf carrying attributes the first-pass editor never touches.
        let existing = Dxf {
            font: Some(DxfFont {
                strike: Some(true),
                u: Some(true),
                b: Some(true),
                i: None,
                sz: Some(14),
                color: rgb_to_color(blue()),
            }),
            fill: None,
            border: Some(Border::default()),
            num_fmt: None,
            alignment: None,
        };
        let new_fmt = CfFormat {
            fill: Some(red()),
            text_color: None,
            bold: false,
            italic: true,
        };
        let merged = merge_cf_format_into_dxf(&new_fmt, existing);
        // Fill + b/i/color come from the new format.
        assert_eq!(
            merged.fill.as_ref().map(|f| &f.color),
            Some(&rgb_to_color(red()))
        );
        let font = merged.font.as_ref().unwrap();
        assert_eq!(font.b, None, "bold cleared by the new format");
        assert_eq!(font.i, Some(true), "italic set by the new format");
        assert!(matches!(font.color, Color::None), "text colour cleared");
        // Unmodeled font attrs + border survive.
        assert_eq!(font.strike, Some(true));
        assert_eq!(font.u, Some(true));
        assert_eq!(font.sz, Some(14));
        assert!(merged.border.is_some());
    }

    #[test]
    fn merge_drops_font_when_nothing_remains() {
        let existing = Dxf {
            font: Some(DxfFont {
                b: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = merge_cf_format_into_dxf(&CfFormat::default(), existing);
        assert!(
            merged.font.is_none(),
            "no modeled or preserved font attr → font dropped"
        );
    }

    fn fmt() -> CfFormat {
        CfFormat {
            fill: Some(red()),
            ..Default::default()
        }
    }
    fn dxf() -> Dxf {
        cf_format_to_dxf(&fmt())
    }

    #[test]
    fn spec_to_input_cell_is() {
        let spec = CfRuleSpec::CellIs {
            op: CfValueOp::Between,
            operand: "10".into(),
            operand2: Some("20".into()),
            format: fmt(),
            stop_if_true: true,
        };
        match cf_rule_spec_to_input(&spec, dxf()) {
            CfRuleInput::CellIs {
                operator,
                formula,
                formula2,
                stop_if_true,
                ..
            } => {
                assert_eq!(operator, ValueOperator::Between);
                assert_eq!(formula, "10");
                assert_eq!(formula2, Some("20".to_string()));
                assert!(stop_if_true);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn spec_to_input_boolean_subvariants() {
        let top = CfRuleSpec::Top {
            rank: 5,
            percent: true,
            bottom: true,
            format: fmt(),
            stop_if_true: false,
        };
        assert!(matches!(
            cf_rule_spec_to_input(&top, dxf()),
            CfRuleInput::Bottom10 {
                rank: 5,
                percent: true,
                ..
            }
        ));

        let avg = CfRuleSpec::Average {
            below: true,
            format: fmt(),
            stop_if_true: false,
        };
        assert!(matches!(
            cf_rule_spec_to_input(&avg, dxf()),
            CfRuleInput::BelowAverage { .. }
        ));

        let dup = CfRuleSpec::DuplicateValues {
            unique: true,
            format: fmt(),
            stop_if_true: false,
        };
        assert!(matches!(
            cf_rule_spec_to_input(&dup, dxf()),
            CfRuleInput::UniqueValues { .. }
        ));

        let blanks = CfRuleSpec::Blanks {
            no_blanks: true,
            format: fmt(),
            stop_if_true: false,
        };
        assert!(matches!(
            cf_rule_spec_to_input(&blanks, dxf()),
            CfRuleInput::NotBlanks { .. }
        ));

        let errors = CfRuleSpec::Errors {
            no_errors: true,
            format: fmt(),
            stop_if_true: false,
        };
        assert!(matches!(
            cf_rule_spec_to_input(&errors, dxf()),
            CfRuleInput::NoErrors { .. }
        ));
    }

    #[test]
    fn spec_to_input_time_period_dates_none() {
        let spec = CfRuleSpec::TimePeriod {
            period: CfPeriod::Last7Days,
            format: fmt(),
            stop_if_true: false,
        };
        match cf_rule_spec_to_input(&spec, dxf()) {
            CfRuleInput::TimePeriod {
                time_period,
                date1,
                date2,
                ..
            } => {
                assert_eq!(time_period, PeriodType::Last7Days);
                assert!(date1.is_none() && date2.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn spec_to_input_color_scale_thresholds() {
        let spec = CfRuleSpec::ColorScale {
            stops: vec![
                CfColorStop {
                    kind: CfThresholdKind::Min,
                    value: None,
                    color: red(),
                },
                CfColorStop {
                    kind: CfThresholdKind::Percentile,
                    value: Some(50.0),
                    color: blue(),
                },
            ],
        };
        match cf_rule_spec_to_input(&spec, Dxf::default()) {
            CfRuleInput::ColorScale { thresholds } => {
                assert_eq!(thresholds.len(), 2);
                assert_eq!(thresholds[0].cfvo, Cfvo::Min);
                assert_eq!(thresholds[1].cfvo, Cfvo::Percentile(50.0));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rule_to_view_cell_is_authorable() {
        let rule = CfRule::CellIs {
            operator: ValueOperator::GreaterThan,
            formula: "100".into(),
            formula2: None,
            dxf_id: 0,
            stop_if_true: false,
        };
        let view = cf_rule_to_view(0, "B2:B20".into(), 1, &rule, Some(dxf()));
        assert!(view.editable);
        assert_eq!(view.summary, "Cell value > 100");
        assert!(matches!(view.preview, CfPreview::Highlight { .. }));
        assert!(matches!(view.spec, Some(CfRuleSpec::CellIs { .. })));
    }

    #[test]
    fn rule_to_view_color_scale_authorable() {
        let rule = CfRule::ColorScale {
            thresholds: vec![
                ColorScaleThreshold {
                    cfvo: Cfvo::Min,
                    color: rgb_to_color(red()),
                },
                ColorScaleThreshold {
                    cfvo: Cfvo::Max,
                    color: rgb_to_color(blue()),
                },
            ],
        };
        let view = cf_rule_to_view(0, "A1:A9".into(), 1, &rule, None);
        assert!(view.editable);
        assert_eq!(view.summary, "2-color scale");
        assert!(matches!(view.preview, CfPreview::ColorScale { .. }));
        assert!(matches!(view.spec, Some(CfRuleSpec::ColorScale { .. })));
    }

    #[test]
    fn rule_to_view_deferred_family_is_badge() {
        for rule in [
            CfRule::DataBar {
                min: None,
                max: None,
                positive_color: Color::None,
                negative_color: Color::None,
                is_gradient: false,
                show_value: false,
            },
            CfRule::IconSet {
                thresholds: vec![],
                show_value: false,
            },
            CfRule::IconRating {
                icon: ironcalc_base::cf_types::Icon::Star,
                color: Color::None,
                thresholds: vec![],
                show_value: false,
            },
        ] {
            let view = cf_rule_to_view(0, "A1:A9".into(), 1, &rule, None);
            assert!(!view.editable);
            assert!(view.spec.is_none());
            assert!(matches!(view.preview, CfPreview::Badge(_)));
        }
    }

    #[test]
    fn rule_to_view_deferred_variant_is_badge() {
        // A date-range TimePeriod is a deferred variant.
        let time = CfRule::TimePeriod {
            time_period: PeriodType::NotBetween,
            date1: Some("2024-01-01".into()),
            date2: Some("2024-12-31".into()),
            dxf_id: 0,
            stop_if_true: false,
        };
        let view = cf_rule_to_view(0, "A1:A9".into(), 1, &time, Some(dxf()));
        assert!(!view.editable);
        assert!(view.spec.is_none());
        assert!(matches!(view.preview, CfPreview::Badge(_)));

        // A colour scale with a Formula threshold is a deferred variant.
        let scale = CfRule::ColorScale {
            thresholds: vec![
                ColorScaleThreshold {
                    cfvo: Cfvo::Formula("A1".into()),
                    color: rgb_to_color(red()),
                },
                ColorScaleThreshold {
                    cfvo: Cfvo::Max,
                    color: rgb_to_color(blue()),
                },
            ],
        };
        let view = cf_rule_to_view(1, "A1:A9".into(), 2, &scale, None);
        assert!(!view.editable);
        assert!(view.spec.is_none());
        assert!(matches!(view.preview, CfPreview::Badge(_)));
    }

    #[test]
    fn rule_to_view_theme_colored_scale_is_badge() {
        // A colour scale with a non-RGB (theme) stop colour must NOT round-trip to an editable
        // spec: reconstructing it would coerce the theme colour to a concrete RGB and an
        // edit+save would overwrite the file's original colour. It is surfaced read-only instead.
        let scale = CfRule::ColorScale {
            thresholds: vec![
                ColorScaleThreshold {
                    cfvo: Cfvo::Min,
                    color: Color::Theme(4, 0.0),
                },
                ColorScaleThreshold {
                    cfvo: Cfvo::Max,
                    color: rgb_to_color(red()),
                },
            ],
        };
        let view = cf_rule_to_view(0, "A1:A9".into(), 1, &scale, None);
        assert!(!view.editable);
        assert!(view.spec.is_none());
        assert!(matches!(view.preview, CfPreview::Badge(_)));
    }
}
