//! `cond_fmt` — the engine-free conditional-formatting vocabulary.
//!
//! Plain `serde` data shared by the worker protocol (the `Command` CF variants + the published
//! rule list) and the sidebar UI. It carries **no** gpui or ironcalc type (the
//! `dependency_rule.rs` guard enforces that) — the IronCalc↔core mapping lives entirely in
//! `freecell-engine::cond_fmt_convert`, so the only colour type here is the crate's own [`Rgb`].
//!
//! Scope is the **first pass** (`functional_spec.md §3`): the *highlight* families (cell value,
//! text, dates, top/bottom, average, duplicate/unique, blanks/errors, formula) whose overlay is a
//! differential cell format, plus *color scales*. Data bars, icon sets, and ratings are deferred
//! families — they surface in the read model ([`CfRuleView`]) as a non-editable [`CfPreview::Badge`]
//! but have no authoring variant here (`architecture.md §2.1`, `components/engine_cf.md §1`).

use serde::{Deserialize, Serialize};

use crate::color::Rgb;

/// The editable differential-format subset a highlight rule applies — the four attributes the
/// first-pass format editor exposes (`functional_spec.md §4`). Everything else IronCalc's `Dxf`
/// models (underline, strikethrough, border, number format, alignment) is left untouched on the
/// engine side and round-trips through an edit (`components/engine_cf.md §3` merge). `None`
/// fill/`text_color` means "inherit the cell's base value"; `bold`/`italic` `false` likewise do not
/// override the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CfFormat {
    pub fill: Option<Rgb>,
    pub text_color: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
}

/// The comparison operator of a *Cell value* (`CfRuleSpec::CellIs`) rule. `Between`/`NotBetween`
/// use both operands; the rest use only the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfValueOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    Between,
    NotBetween,
}

/// The operator of a *Text* (`CfRuleSpec::Text`) rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfTextOp {
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
    Equals,
}

/// A parameterless date period for an *A date occurring* (`CfRuleSpec::TimePeriod`) rule
/// (`functional_spec.md §2.3`). The explicit date-range periods (`Between`/`NotBetween`) — and
/// IronCalc's `Next7Days` — are deferred variants (`functional_spec.md §9`), so they are absent
/// here and only ever surface on read-back as a [`CfPreview::Badge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfPeriod {
    Today,
    Yesterday,
    Tomorrow,
    Last7Days,
    LastWeek,
    ThisWeek,
    NextWeek,
    LastMonth,
    ThisMonth,
    NextMonth,
    LastYear,
    ThisYear,
    NextYear,
}

/// How a color-scale stop's position is expressed (`functional_spec.md §2.3`). `Min`/`Max` are the
/// endpoints (no value); `Number`/`Percent`/`Percentile` carry a `value`. `Formula` thresholds are
/// deferred (`functional_spec.md §9`), so they have no kind here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfThresholdKind {
    Min,
    Max,
    Number,
    Percent,
    Percentile,
}

/// One stop of a color scale: its threshold kind, an optional numeric value (`None` for
/// `Min`/`Max`), and its colour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CfColorStop {
    pub kind: CfThresholdKind,
    pub value: Option<f64>,
    pub color: Rgb,
}

/// The engine-free definition of a CF rule, used to add or update one. Each highlight variant
/// carries its [`CfFormat`]; [`ColorScale`](Self::ColorScale) carries its stops instead
/// (`architecture.md §2.1`, `components/engine_cf.md §1`). Variant → `ironcalc CfRuleInput` mapping
/// is in `freecell-engine::cond_fmt_convert`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CfRuleSpec {
    /// Format cells whose value compares to one (or two, for `Between`/`NotBetween`) operands.
    CellIs {
        op: CfValueOp,
        operand: String,
        operand2: Option<String>,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Format cells whose text contains / begins / ends / equals a string.
    Text {
        op: CfTextOp,
        value: String,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Format cells whose date falls in a parameterless period.
    TimePeriod {
        period: CfPeriod,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Top N / Bottom N (by count or `percent`) of the range.
    Top {
        rank: u32,
        percent: bool,
        bottom: bool,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Above / below the range average.
    Average {
        below: bool,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Duplicate (or `unique`) values in the range.
    DuplicateValues {
        unique: bool,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Blank (or `no_blanks`) cells.
    Blanks {
        no_blanks: bool,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// Error (or `no_errors`) cells.
    Errors {
        no_errors: bool,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// A custom formula returning TRUE/FALSE for the range's top-left cell.
    Formula {
        formula: String,
        format: CfFormat,
        stop_if_true: bool,
    },
    /// A 2- or 3-stop color scale (an interpolated fill; no [`CfFormat`]).
    ColorScale { stops: Vec<CfColorStop> },
}

impl CfRuleSpec {
    /// The rule's [`CfFormat`], or `None` for [`ColorScale`](Self::ColorScale) (which has none).
    /// Lets the add/update path fetch the differential format uniformly.
    pub fn format(&self) -> Option<&CfFormat> {
        match self {
            CfRuleSpec::CellIs { format, .. }
            | CfRuleSpec::Text { format, .. }
            | CfRuleSpec::TimePeriod { format, .. }
            | CfRuleSpec::Top { format, .. }
            | CfRuleSpec::Average { format, .. }
            | CfRuleSpec::DuplicateValues { format, .. }
            | CfRuleSpec::Blanks { format, .. }
            | CfRuleSpec::Errors { format, .. }
            | CfRuleSpec::Formula { format, .. } => Some(format),
            CfRuleSpec::ColorScale { .. } => None,
        }
    }

    /// Mutable access to the rule's [`CfFormat`] (the editor edits it in place); `None` for
    /// [`ColorScale`](Self::ColorScale).
    pub fn format_mut(&mut self) -> Option<&mut CfFormat> {
        match self {
            CfRuleSpec::CellIs { format, .. }
            | CfRuleSpec::Text { format, .. }
            | CfRuleSpec::TimePeriod { format, .. }
            | CfRuleSpec::Top { format, .. }
            | CfRuleSpec::Average { format, .. }
            | CfRuleSpec::DuplicateValues { format, .. }
            | CfRuleSpec::Blanks { format, .. }
            | CfRuleSpec::Errors { format, .. }
            | CfRuleSpec::Formula { format, .. } => Some(format),
            CfRuleSpec::ColorScale { .. } => None,
        }
    }
}

/// The list-row preview of a rule's effect (`architecture.md §2.1`): a solid swatch for a highlight
/// rule, an ordered gradient for a color scale, or a text badge for a deferred family/variant the
/// first pass cannot author.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CfPreview {
    Highlight {
        fill: Option<Rgb>,
        text_color: Option<Rgb>,
    },
    ColorScale {
        colors: Vec<Rgb>,
    },
    Badge(String),
}

/// The read model for one CF rule row (`architecture.md §2.1`). `index` is the rule's stable
/// storage index (the handle the index-based mutators take — *not* its display position); the list
/// is ordered by `priority` descending. `spec` is `Some` for a first-pass-authorable rule (it seeds
/// the editor) and `None` (with `editable == false`) for a deferred family/variant, which the list
/// still shows and can delete but cannot edit (`functional_spec.md §9`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfRuleView {
    pub index: u32,
    pub range: String,
    pub priority: u32,
    pub editable: bool,
    pub summary: String,
    pub preview: CfPreview,
    pub spec: Option<CfRuleSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_format() -> CfFormat {
        CfFormat {
            fill: Some(Rgb::from_hex(0xFFC7CE)),
            text_color: Some(Rgb::from_hex(0x9C0006)),
            bold: true,
            italic: false,
        }
    }

    #[test]
    fn cell_is_spec_serde_round_trips() {
        let spec = CfRuleSpec::CellIs {
            op: CfValueOp::Between,
            operand: "10".to_string(),
            operand2: Some("20".to_string()),
            format: sample_format(),
            stop_if_true: true,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: CfRuleSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn color_scale_spec_serde_round_trips() {
        let spec = CfRuleSpec::ColorScale {
            stops: vec![
                CfColorStop {
                    kind: CfThresholdKind::Min,
                    value: None,
                    color: Rgb::from_hex(0x63BE7B),
                },
                CfColorStop {
                    kind: CfThresholdKind::Percentile,
                    value: Some(50.0),
                    color: Rgb::from_hex(0xFFEB84),
                },
                CfColorStop {
                    kind: CfThresholdKind::Max,
                    value: None,
                    color: Rgb::from_hex(0xF8696B),
                },
            ],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: CfRuleSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn view_serde_round_trips() {
        let view = CfRuleView {
            index: 2,
            range: "B2:B20".to_string(),
            priority: 3,
            editable: true,
            summary: "Cell value > 100".to_string(),
            preview: CfPreview::Highlight {
                fill: Some(Rgb::from_hex(0xFFC7CE)),
                text_color: None,
            },
            spec: Some(CfRuleSpec::CellIs {
                op: CfValueOp::Gt,
                operand: "100".to_string(),
                operand2: None,
                format: sample_format(),
                stop_if_true: false,
            }),
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: CfRuleView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
    }

    #[test]
    fn format_accessor_reflects_variant() {
        let highlight = CfRuleSpec::Text {
            op: CfTextOp::Contains,
            value: "foo".to_string(),
            format: sample_format(),
            stop_if_true: false,
        };
        assert_eq!(highlight.format(), Some(&sample_format()));

        let scale = CfRuleSpec::ColorScale { stops: vec![] };
        assert_eq!(scale.format(), None);
    }

    #[test]
    fn format_mut_edits_in_place() {
        let mut spec = CfRuleSpec::Formula {
            formula: "A1>5".to_string(),
            format: CfFormat::default(),
            stop_if_true: false,
        };
        spec.format_mut().unwrap().bold = true;
        assert!(spec.format().unwrap().bold);

        let mut scale = CfRuleSpec::ColorScale { stops: vec![] };
        assert!(scale.format_mut().is_none());
    }
}
