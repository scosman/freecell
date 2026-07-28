//! The conditional-formatting sidebar's **Editor mode** (`components/cf_sidebar.md §3, §5–§8`,
//! P6): the rule editor's seeded inputs and inline dropdowns, the per-kind operands, the format
//! editor, the colour-scale stop editor, and validation + save.
//!
//! Opened from the rules list in [`super::cf_sidebar`], which is the same panel's other mode;
//! the two are separate files because together they exceed the 2,000-line production ceiling
//! (`architecture.md §7.3`). Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::cf_sidebar::{cf_color, CF_SWATCH_H, CF_SWATCH_W};
use super::*;

/// The tint behind an open CF-editor inline dropdown's option list (a faint grey wash so the
/// expanded options read as a group under their trigger).
const CF_MENU_BG: u32 = 0xF7F7F7;
/// A colour swatch's side in the CF format editor's fill/text palette.
const CF_SWATCH_SIDE: f32 = 18.0;
/// The "range required" validation message — shown inline under the Applies-to field (so it is
/// filtered out of the general message list). One source so the field hint + the filter agree.
const CF_RANGE_REQUIRED: &str = "Enter a range in Applies to.";

/// Which of the CF rule editor's inline dropdowns is expanded (`components/cf_sidebar.md §5`). Each
/// trigger toggles its own menu; only one is open at a time (a scrolling sidebar body, so the
/// options render inline below the trigger rather than as an anchored popover).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CfMenu {
    RuleType,
    ValueOp,
    TextOp,
    Period,
    /// The threshold-kind dropdown for color-scale stop `usize` (0-based).
    StopKind(usize),
}

/// The rule-type dropdown menu (`components/cf_sidebar.md §5`): the highlight families in Excel's
/// grouping order, then the **Color scale** group (P7). `(kind, debug tag, label)`.
const CF_KIND_MENU: [(CfEditorKind, &str, &str); 11] = [
    (CfEditorKind::CellValue, "cellvalue", "Cell value"),
    (CfEditorKind::Text, "text", "Text"),
    (CfEditorKind::Dates, "dates", "Date occurring"),
    (CfEditorKind::TopBottom, "topbottom", "Top / Bottom"),
    (CfEditorKind::Average, "average", "Above / Below average"),
    (CfEditorKind::Duplicate, "duplicate", "Duplicate / Unique"),
    (CfEditorKind::Blanks, "blanks", "Blank / No blanks"),
    (CfEditorKind::Errors, "errors", "Error / No errors"),
    (CfEditorKind::Formula, "formula", "Formula"),
    (CfEditorKind::ColorScale2, "colorscale2", "2-color scale"),
    (CfEditorKind::ColorScale3, "colorscale3", "3-color scale"),
];

/// The *Cell value* operator dropdown menu. `(op, debug tag, label)`.
const CF_VALUE_OP_MENU: [(CfValueOp, &str, &str); 8] = [
    (CfValueOp::Gt, "gt", "greater than"),
    (CfValueOp::Lt, "lt", "less than"),
    (CfValueOp::Ge, "ge", "greater than or equal to"),
    (CfValueOp::Le, "le", "less than or equal to"),
    (CfValueOp::Eq, "eq", "equal to"),
    (CfValueOp::Ne, "ne", "not equal to"),
    (CfValueOp::Between, "between", "between"),
    (CfValueOp::NotBetween, "notbetween", "not between"),
];

/// The *Text* operator dropdown menu. `(op, debug tag, label)`.
const CF_TEXT_OP_MENU: [(CfTextOp, &str, &str); 5] = [
    (CfTextOp::Contains, "contains", "contains"),
    (CfTextOp::NotContains, "notcontains", "does not contain"),
    (CfTextOp::BeginsWith, "beginswith", "begins with"),
    (CfTextOp::EndsWith, "endswith", "ends with"),
    (CfTextOp::Equals, "equals", "equal to"),
];

/// The *A date occurring* period dropdown menu (the parameterless periods). `(period, tag, label)`.
const CF_PERIOD_MENU: [(CfPeriod, &str, &str); 13] = [
    (CfPeriod::Today, "today", "Today"),
    (CfPeriod::Yesterday, "yesterday", "Yesterday"),
    (CfPeriod::Tomorrow, "tomorrow", "Tomorrow"),
    (CfPeriod::Last7Days, "last7days", "In the last 7 days"),
    (CfPeriod::LastWeek, "lastweek", "Last week"),
    (CfPeriod::ThisWeek, "thisweek", "This week"),
    (CfPeriod::NextWeek, "nextweek", "Next week"),
    (CfPeriod::LastMonth, "lastmonth", "Last month"),
    (CfPeriod::ThisMonth, "thismonth", "This month"),
    (CfPeriod::NextMonth, "nextmonth", "Next month"),
    (CfPeriod::LastYear, "lastyear", "Last year"),
    (CfPeriod::ThisYear, "thisyear", "This year"),
    (CfPeriod::NextYear, "nextyear", "Next year"),
];

/// The differential-format presets (`components/cf_sidebar.md §7`, the classic Excel highlight
/// looks). Each sets the whole [`CfFormat`] in one click. `(label, format)`.
const CF_PRESETS: [(&str, CfFormat); 5] = [
    (
        "Light red / dark red",
        CfFormat {
            fill: Some(Rgb::from_hex(0xFFC7CE)),
            text_color: Some(Rgb::from_hex(0x9C0006)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Yellow / dark yellow",
        CfFormat {
            fill: Some(Rgb::from_hex(0xFFEB9C)),
            text_color: Some(Rgb::from_hex(0x9C6500)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Green / dark green",
        CfFormat {
            fill: Some(Rgb::from_hex(0xC6EFCE)),
            text_color: Some(Rgb::from_hex(0x006100)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Red text",
        CfFormat {
            fill: None,
            text_color: Some(Rgb::from_hex(0x9C0006)),
            bold: false,
            italic: false,
        },
    ),
    (
        "Bold",
        CfFormat {
            fill: None,
            text_color: None,
            bold: true,
            italic: false,
        },
    ),
];

/// The default 2-color scale presets (`components/cf_sidebar.md §8`), `(label, colors)`; the first
/// colour is the `Min` endpoint, the last the `Max`.
const CF_SCALE_PRESETS_2: [(&str, [u32; 2]); 2] = [
    ("White – Blue", [0xFCFCFF, 0x5A8AC6]),
    ("Green – White", [0x63BE7B, 0xFCFCFF]),
];

/// The default 3-color scale presets, `(label, colors)`; `Min` / midpoint (`Percentile` 50) / `Max`.
const CF_SCALE_PRESETS_3: [(&str, [u32; 3]); 2] = [
    ("Green – Yellow – Red", [0x63BE7B, 0xFFEB84, 0xF8696B]),
    ("Red – Yellow – Green", [0xF8696B, 0xFFEB84, 0x63BE7B]),
];

/// The default colour of a 3-color scale's midpoint when a 2-color scale grows to 3 (Excel yellow).
const CF_SCALE_MID_DEFAULT: u32 = 0xFFEB84;

/// The threshold-kind options for the **first** (minimum) endpoint stop: `Min` (lowest value) plus
/// the value-carrying kinds. `(kind, debug tag, label)`.
const CF_STOP_KIND_MIN: [(CfThresholdKind, &str, &str); 4] = [
    (CfThresholdKind::Min, "min", "Lowest value"),
    (CfThresholdKind::Number, "number", "Number"),
    (CfThresholdKind::Percent, "percent", "Percent"),
    (CfThresholdKind::Percentile, "percentile", "Percentile"),
];

/// The threshold-kind options for the **last** (maximum) endpoint stop: `Max` (highest value) plus
/// the value-carrying kinds.
const CF_STOP_KIND_MAX: [(CfThresholdKind, &str, &str); 4] = [
    (CfThresholdKind::Max, "max", "Highest value"),
    (CfThresholdKind::Number, "number", "Number"),
    (CfThresholdKind::Percent, "percent", "Percent"),
    (CfThresholdKind::Percentile, "percentile", "Percentile"),
];

/// The threshold-kind options for a **midpoint** stop: only the value-carrying kinds (no Min/Max).
const CF_STOP_KIND_MID: [(CfThresholdKind, &str, &str); 3] = [
    (CfThresholdKind::Number, "number", "Number"),
    (CfThresholdKind::Percent, "percent", "Percent"),
    (CfThresholdKind::Percentile, "percentile", "Percentile"),
];

/// Static per-stop debug/element-id tags (`cf-stop0-…`) for the up-to-3 stop dropdowns.
const CF_STOP_TAGS: [&str; 3] = ["stop0", "stop1", "stop2"];

/// A color-scale stop's position in the scale, driving which threshold kinds it offers + its label.
#[derive(Clone, Copy)]
enum StopPos {
    First,
    Mid,
    Last,
}

/// The position of stop `i` in a scale of `len` stops (a 2-color scale has no middle).
fn cf_stop_pos(i: usize, len: usize) -> StopPos {
    if i == 0 {
        StopPos::First
    } else if i + 1 >= len {
        StopPos::Last
    } else {
        StopPos::Mid
    }
}

/// The muted label above a stop's controls.
fn cf_stop_label(pos: StopPos) -> &'static str {
    match pos {
        StopPos::First => "Minimum",
        StopPos::Mid => "Midpoint",
        StopPos::Last => "Maximum",
    }
}

/// Whether a threshold kind carries a numeric value (so its stop shows a value input + is validated).
fn cf_stop_needs_value(kind: CfThresholdKind) -> bool {
    matches!(
        kind,
        CfThresholdKind::Number | CfThresholdKind::Percent | CfThresholdKind::Percentile
    )
}

/// A threshold kind's dropdown-trigger label.
fn cf_threshold_kind_label(kind: CfThresholdKind) -> &'static str {
    match kind {
        CfThresholdKind::Min => "Lowest value",
        CfThresholdKind::Max => "Highest value",
        CfThresholdKind::Number => "Number",
        CfThresholdKind::Percent => "Percent",
        CfThresholdKind::Percentile => "Percentile",
    }
}

/// Build a color scale's default stops from a preset's colours: the first stop is `Min`, the last is
/// `Max`, and any middle stop is the 50th `Percentile` (`components/cf_sidebar.md §8`).
fn cf_stops_from_colors(colors: &[u32]) -> Vec<CfColorStop> {
    let last = colors.len().saturating_sub(1);
    colors
        .iter()
        .enumerate()
        .map(|(i, &hex)| {
            let kind = if i == 0 {
                CfThresholdKind::Min
            } else if i == last {
                CfThresholdKind::Max
            } else {
                CfThresholdKind::Percentile
            };
            let value = (kind == CfThresholdKind::Percentile).then_some(50.0);
            CfColorStop {
                kind,
                value,
                color: Rgb::from_hex(hex),
            }
        })
        .collect()
}

/// Format a stop's numeric value for its input (`50.0` → `"50"`, `12.5` → `"12.5"`).
fn cf_fmt_stop_value(value: f64) -> String {
    format!("{value}")
}

fn cf_kind_label(kind: CfEditorKind) -> &'static str {
    CF_KIND_MENU
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, _, label)| *label)
        .unwrap_or("Cell value")
}

fn cf_value_op_label(op: CfValueOp) -> &'static str {
    CF_VALUE_OP_MENU
        .iter()
        .find(|(o, _, _)| *o == op)
        .map(|(_, _, label)| *label)
        .unwrap_or("greater than")
}

fn cf_text_op_label(op: CfTextOp) -> &'static str {
    CF_TEXT_OP_MENU
        .iter()
        .find(|(o, _, _)| *o == op)
        .map(|(_, _, label)| *label)
        .unwrap_or("contains")
}

fn cf_period_label(period: CfPeriod) -> &'static str {
    CF_PERIOD_MENU
        .iter()
        .find(|(p, _, _)| *p == period)
        .map(|(_, _, label)| *label)
        .unwrap_or("Today")
}

/// A small red inline error/hint line (`ui_design.md §2.2`).
fn cf_inline_error(text: impl Into<SharedString>) -> gpui::AnyElement {
    div()
        .text_size(px(10.5))
        .text_color(rgb(DANGER))
        .child(text.into())
        .into_any_element()
}

/// A two-option segmented toggle (e.g. Top/Bottom, Above/Below) — two small selectable buttons.
/// `left`/`right` are `(debug tag, label, selected)`; the click handlers are supplied by the caller.
fn cf_segmented(
    options: [(&'static str, &'static str, bool); 2],
    on_left: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_right: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let [(ltag, llabel, lsel), (rtag, rlabel, rsel)] = options;
    div()
        .flex()
        .gap_1()
        .child(
            Button::new(ltag)
                .label(llabel)
                .ghost()
                .small()
                .selected(lsel)
                .debug_selector(move || ltag.to_string())
                .on_click(on_left),
        )
        .child(
            Button::new(rtag)
                .label(rlabel)
                .ghost()
                .small()
                .selected(rsel)
                .debug_selector(move || rtag.to_string())
                .on_click(on_right),
        )
}

/// Client-side validation of the editor form (`components/cf_sidebar.md §6`): the messages that
/// block Save (empty = valid). Range non-empty; Cell-value operand(s) present (a second for
/// Between/NotBetween); Text value present; Formula present; Top rank ≥ 1.
fn cf_validate(
    editor: &CfEditorState,
    range: &str,
    operand1: &str,
    operand2: &str,
    formula: &str,
) -> Vec<String> {
    let mut errs = Vec::new();
    if range.trim().is_empty() {
        errs.push(CF_RANGE_REQUIRED.to_string());
    }
    match editor.kind {
        CfEditorKind::CellValue => {
            if operand1.trim().is_empty() {
                errs.push("Enter a value.".to_string());
            }
            if matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween)
                && operand2.trim().is_empty()
            {
                errs.push("Enter a second value.".to_string());
            }
        }
        CfEditorKind::Text => {
            if operand1.trim().is_empty() {
                errs.push("Enter text to match.".to_string());
            }
        }
        CfEditorKind::Formula => {
            if formula.trim().is_empty() {
                errs.push("Enter a formula.".to_string());
            }
        }
        CfEditorKind::TopBottom => {
            if !matches!(operand1.trim().parse::<u32>(), Ok(n) if n >= 1) {
                errs.push("Enter a rank of 1 or more.".to_string());
            }
        }
        CfEditorKind::ColorScale2 | CfEditorKind::ColorScale3 => {
            // Min/Max endpoints need no value; every Number/Percent/Percentile stop needs a
            // parseable one (the on-edit sync leaves an empty/unparseable input as `None`).
            if editor
                .scale
                .iter()
                .any(|s| cf_stop_needs_value(s.kind) && s.value.is_none())
            {
                errs.push(
                    "Enter a value for each Number, Percent, or Percentile stop.".to_string(),
                );
            }
        }
        CfEditorKind::Dates
        | CfEditorKind::Average
        | CfEditorKind::Duplicate
        | CfEditorKind::Blanks
        | CfEditorKind::Errors => {}
    }
    errs
}

/// Assemble the [`CfRuleSpec`] the editor commits (add/update). Assumes [`cf_validate`] passed, so
/// the operands are present + a Top rank parses (falls back to the seed rank otherwise).
fn cf_build_spec(
    editor: &CfEditorState,
    operand1: &str,
    operand2: &str,
    formula: &str,
) -> CfRuleSpec {
    let format = editor.format;
    let stop_if_true = editor.stop_if_true;
    match editor.kind {
        CfEditorKind::CellValue => {
            let operand2 = matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween)
                .then(|| operand2.trim().to_string());
            CfRuleSpec::CellIs {
                op: editor.value_op,
                operand: operand1.trim().to_string(),
                operand2,
                format,
                stop_if_true,
            }
        }
        CfEditorKind::Text => CfRuleSpec::Text {
            op: editor.text_op,
            value: operand1.trim().to_string(),
            format,
            stop_if_true,
        },
        CfEditorKind::Dates => CfRuleSpec::TimePeriod {
            period: editor.period,
            format,
            stop_if_true,
        },
        CfEditorKind::TopBottom => CfRuleSpec::Top {
            rank: operand1.trim().parse().unwrap_or(editor.top_rank),
            percent: editor.top_percent,
            bottom: editor.top_bottom,
            format,
            stop_if_true,
        },
        CfEditorKind::Average => CfRuleSpec::Average {
            below: editor.average_below,
            format,
            stop_if_true,
        },
        CfEditorKind::Duplicate => CfRuleSpec::DuplicateValues {
            unique: editor.duplicate_unique,
            format,
            stop_if_true,
        },
        CfEditorKind::Blanks => CfRuleSpec::Blanks {
            no_blanks: editor.blanks_no,
            format,
            stop_if_true,
        },
        CfEditorKind::Errors => CfRuleSpec::Errors {
            no_errors: editor.errors_no,
            format,
            stop_if_true,
        },
        CfEditorKind::Formula => CfRuleSpec::Formula {
            formula: formula.trim().to_string(),
            format,
            stop_if_true,
        },
        // Color scales carry no format/stop-if-true — the spec is just the (synced) stops.
        CfEditorKind::ColorScale2 | CfEditorKind::ColorScale3 => CfRuleSpec::ColorScale {
            stops: editor.scale.clone(),
        },
    }
}

/// Seed a [`CfEditorState`] + the operand/formula input texts `(operand1, operand2, formula)` from
/// an authorable rule's `spec` (edit mode). A `ColorScale` seeds `state.scale` (its stop value
/// inputs are seeded separately by [`open_cf_editor`](ChromeView::open_cf_editor)); returns `None`
/// only if a future non-authorable spec variant is added.
fn cf_state_from_spec(
    index: u32,
    spec: &CfRuleSpec,
) -> Option<(CfEditorState, String, String, String)> {
    let mut state = CfEditorState::new(Some(index));
    let (op1, op2, formula) = match spec {
        CfRuleSpec::CellIs {
            op,
            operand,
            operand2,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::CellValue;
            state.value_op = *op;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (
                operand.clone(),
                operand2.clone().unwrap_or_default(),
                String::new(),
            )
        }
        CfRuleSpec::Text {
            op,
            value,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Text;
            state.text_op = *op;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (value.clone(), String::new(), String::new())
        }
        CfRuleSpec::TimePeriod {
            period,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Dates;
            state.period = *period;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Top {
            rank,
            percent,
            bottom,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::TopBottom;
            state.top_rank = *rank;
            state.top_percent = *percent;
            state.top_bottom = *bottom;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (rank.to_string(), String::new(), String::new())
        }
        CfRuleSpec::Average {
            below,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Average;
            state.average_below = *below;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::DuplicateValues {
            unique,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Duplicate;
            state.duplicate_unique = *unique;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Blanks {
            no_blanks,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Blanks;
            state.blanks_no = *no_blanks;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Errors {
            no_errors,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Errors;
            state.errors_no = *no_errors;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), String::new())
        }
        CfRuleSpec::Formula {
            formula,
            format,
            stop_if_true,
        } => {
            state.kind = CfEditorKind::Formula;
            state.format = *format;
            state.stop_if_true = *stop_if_true;
            (String::new(), String::new(), formula.clone())
        }
        CfRuleSpec::ColorScale { stops } => {
            // An editable (concrete-RGB) scale — the arity follows the stop count (2 ⇒ ColorScale2,
            // 3+ ⇒ ColorScale3). The stops (kind / value / color) carry the whole editor scale.
            state.kind = if stops.len() >= 3 {
                CfEditorKind::ColorScale3
            } else {
                CfEditorKind::ColorScale2
            };
            state.scale = stops.clone();
            (String::new(), String::new(), String::new())
        }
    };
    Some((state, op1, op2, formula))
}

impl ChromeView {
    /// Whether the CF rule **editor** is open (Editor mode) — introspection for the window's engine
    /// error routing + tests.
    pub fn cf_editor_open(&self) -> bool {
        self.cond_fmt.as_ref().is_some_and(|p| p.editor.is_some())
    }

    /// Open the rule editor (List → Editor mode) for `edit_index` (`Some` = edit an existing rule,
    /// `None` = add). Add mode seeds a fresh [`CfEditorState`] + the Applies-to range from the
    /// current selection; edit mode seeds the state, the text inputs, and (for a color scale) the
    /// stop inputs from the row's `spec` — `Some` for any authorable rule (highlight **or** a
    /// concrete-RGB color scale). A row with no `spec` (a deferred-family / theme-colored Badge,
    /// whose edit control is disabled) is a no-op. `components/cf_sidebar.md §4`.
    pub(super) fn open_cf_editor(
        &mut self,
        edit_index: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.cond_fmt.as_ref() else {
            return;
        };
        // Assemble the editor state + the operand/formula seeds without holding a borrow of `panel`.
        let (state, range, op1, op2, formula) = match edit_index {
            None => (
                CfEditorState::new(None),
                self.selection.to_a1(),
                String::new(),
                String::new(),
                String::new(),
            ),
            Some(index) => {
                let Some(spec) = panel
                    .rows
                    .iter()
                    .find(|r| r.index == index)
                    .and_then(|r| r.spec.clone())
                else {
                    return; // no such row / a deferred-family (Badge) row: not editable.
                };
                let range = panel
                    .rows
                    .iter()
                    .find(|r| r.index == index)
                    .map(|r| r.range.clone())
                    .unwrap_or_default();
                let Some((state, op1, op2, formula)) = cf_state_from_spec(index, &spec) else {
                    return; // only a hypothetical future non-authorable spec variant lands here.
                };
                (state, range, op1, op2, formula)
            }
        };
        self.seed_cf_inputs(range, op1, op2, formula, window, cx);
        self.seed_cf_stop_inputs(&state.scale, window, cx);
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.editor = Some(state);
            cx.notify();
        }
    }

    /// Cancel the editor (Editor → List mode) — the back-chevron / Cancel button
    /// (`components/cf_sidebar.md §4`).
    pub fn cancel_cf_editor(&mut self, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            if panel.editor.take().is_some() {
                cx.notify();
            }
        }
    }

    /// Seed the four editor text inputs (`set_value` suppresses the widgets' change events, so
    /// seeding never trips the live re-render/validation subscription spuriously).
    fn seed_cf_inputs(
        &self,
        range: String,
        operand1: String,
        operand2: String,
        formula: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cf_range_input
            .clone()
            .update(cx, |i, cx| i.set_value(range, window, cx));
        self.cf_operand1_input
            .clone()
            .update(cx, |i, cx| i.set_value(operand1, window, cx));
        self.cf_operand2_input
            .clone()
            .update(cx, |i, cx| i.set_value(operand2, window, cx));
        self.cf_formula_input
            .clone()
            .update(cx, |i, cx| i.set_value(formula, window, cx));
    }

    /// Re-render the chrome so the editor's Save `disabled` state + inline validation track the
    /// field text live as the user types (the inputs are read at render, not committed per
    /// keystroke). A no-op while the editor is closed.
    pub(super) fn on_cf_input_event(
        &mut self,
        _input: &Entity<InputState>,
        _event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cf_editor_open() {
            // Keep the color-scale stops' values current with the stop inputs so live validation +
            // the built spec read `editor.scale` directly (a no-op for a highlight-rule editor).
            self.sync_cf_scale_values(cx);
            cx.notify();
        }
    }

    /// Read the color-scale stop value inputs into `editor.scale` (the authoritative store): each
    /// Number/Percent/Percentile stop takes its input's parsed `f64` (or `None` when empty /
    /// unparseable). A no-op unless a color-scale editor is open (`components/cf_sidebar.md §8`).
    fn sync_cf_scale_values(&mut self, cx: &mut Context<Self>) {
        let values: Vec<Option<f64>> = self
            .cf_stop_value_inputs
            .iter()
            .map(|input| {
                let text = input.read(cx).value();
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    trimmed.parse::<f64>().ok()
                }
            })
            .collect();
        let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) else {
            return;
        };
        if !matches!(
            editor.kind,
            CfEditorKind::ColorScale2 | CfEditorKind::ColorScale3
        ) {
            return;
        }
        // Cap to the value-input count: only the up-to-3 authored stops have an input to read from,
        // so a stop beyond index 2 (a pathological loaded scale) keeps its value untouched.
        for (i, stop) in editor.scale.iter_mut().enumerate().take(values.len()) {
            if cf_stop_needs_value(stop.kind) {
                stop.value = values[i];
            }
        }
    }

    /// Seed the 3 stop value inputs from `scale` (`set_value` suppresses the change event, so this
    /// never triggers a sync-back). A stop shows its value string only for Number/Percent/Percentile;
    /// Min/Max (and a blank Number) seed empty.
    fn seed_cf_stop_inputs(
        &self,
        scale: &[CfColorStop],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (i, input) in self.cf_stop_value_inputs.iter().enumerate() {
            let text = scale
                .get(i)
                .filter(|s| cf_stop_needs_value(s.kind))
                .and_then(|s| s.value)
                .map(cf_fmt_stop_value)
                .unwrap_or_default();
            input
                .clone()
                .update(cx, |inp, cx| inp.set_value(text, window, cx));
        }
    }

    /// The current text of the four editor inputs `(range, operand1, operand2, formula)` — read at
    /// render (for validation) and at Save (to assemble the spec).
    fn cf_input_texts(&self, cx: &Context<Self>) -> (String, String, String, String) {
        (
            self.cf_range_input.read(cx).value().to_string(),
            self.cf_operand1_input.read(cx).value().to_string(),
            self.cf_operand2_input.read(cx).value().to_string(),
            self.cf_formula_input.read(cx).value().to_string(),
        )
    }

    /// Mutate the open editor's state through `f`, clearing any stale engine error and re-rendering.
    /// A no-op while the editor is closed.
    fn with_cf_editor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut CfEditorState)) {
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            f(editor);
            editor.errors.clear();
            cx.notify();
        }
    }

    /// Toggle the editor's inline dropdown `menu` (open it, or close it if already open).
    fn toggle_cf_menu(&mut self, menu: CfMenu, cx: &mut Context<Self>) {
        self.cf_menu_open = (self.cf_menu_open != Some(menu)).then_some(menu);
        cx.notify();
    }

    /// Select the rule family/variant from the rule-type dropdown, closing the menu. Reseeds the
    /// operand inputs so a switch starts each kind fresh (the Top/Bottom rank seeds to its default);
    /// a color-scale kind seeds a default scale (2 or 3 stops) + its stop value inputs.
    fn select_cf_kind(&mut self, kind: CfEditorKind, window: &mut Window, cx: &mut Context<Self>) {
        let rank = self
            .cond_fmt
            .as_ref()
            .and_then(|p| p.editor.as_ref())
            .map(|e| e.top_rank)
            .unwrap_or(10);
        let operand1 = if kind == CfEditorKind::TopBottom {
            rank.to_string()
        } else {
            String::new()
        };
        self.seed_cf_inputs(
            self.cf_range_input.read(cx).value().to_string(),
            operand1,
            String::new(),
            String::new(),
            window,
            cx,
        );
        // A color-scale kind carries stops (a default preset of the right arity); a highlight kind
        // clears any prior scale + stop inputs.
        let scale = match kind {
            CfEditorKind::ColorScale2 => cf_stops_from_colors(&CF_SCALE_PRESETS_2[0].1),
            CfEditorKind::ColorScale3 => cf_stops_from_colors(&CF_SCALE_PRESETS_3[0].1),
            _ => Vec::new(),
        };
        self.seed_cf_stop_inputs(&scale, window, cx);
        self.cf_menu_open = None;
        self.with_cf_editor(cx, move |e| {
            e.kind = kind;
            e.scale = scale;
        });
    }

    /// The color-scale 2-vs-3 segmented control: switch arity, resizing `scale` while **preserving
    /// the endpoints** (going to 3 inserts a 50th-percentile midpoint; going to 2 drops the middle).
    fn set_cf_scale_arity(&mut self, three: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.cond_fmt.as_ref().and_then(|p| p.editor.as_ref()) else {
            return;
        };
        let mut scale = editor.scale.clone();
        if scale.len() < 2 {
            // Degenerate (shouldn't happen for an active scale) — fall back to the default preset.
            scale = cf_stops_from_colors(if three {
                &CF_SCALE_PRESETS_3[0].1
            } else {
                &CF_SCALE_PRESETS_2[0].1
            });
        } else if three && scale.len() == 2 {
            scale.insert(
                1,
                CfColorStop {
                    kind: CfThresholdKind::Percentile,
                    value: Some(50.0),
                    color: Rgb::from_hex(CF_SCALE_MID_DEFAULT),
                },
            );
        } else if !three && scale.len() >= 3 {
            let last = scale[scale.len() - 1];
            scale.truncate(1);
            scale.push(last);
        }
        self.seed_cf_stop_inputs(&scale, window, cx);
        self.cf_menu_open = None;
        self.with_cf_editor(cx, move |e| {
            e.kind = if three {
                CfEditorKind::ColorScale3
            } else {
                CfEditorKind::ColorScale2
            };
            e.scale = scale;
        });
    }

    /// Apply a color-scale preset's colours (a preset chip): rebuild `scale` from the preset — the
    /// first stop `Min`, the last `Max`, a 3-colour midpoint `Percentile` 50 — reseeding the inputs.
    fn apply_cf_scale_preset(
        &mut self,
        colors: &[u32],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scale = cf_stops_from_colors(colors);
        self.seed_cf_stop_inputs(&scale, window, cx);
        self.with_cf_editor(cx, move |e| e.scale = scale);
    }

    /// Set a color-scale stop's threshold kind (its dropdown): `Min`/`Max`/`Number` seed no value
    /// (blank — a Number then needs the user's entry), `Percent`/`Percentile` seed 50. Reseeds the
    /// stop's value input to match.
    fn set_cf_stop_kind(
        &mut self,
        i: usize,
        kind: CfThresholdKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = match kind {
            CfThresholdKind::Percent | CfThresholdKind::Percentile => Some(50.0),
            CfThresholdKind::Min | CfThresholdKind::Max | CfThresholdKind::Number => None,
        };
        self.cf_menu_open = None;
        self.with_cf_editor(cx, move |e| {
            if let Some(stop) = e.scale.get_mut(i) {
                stop.kind = kind;
                stop.value = value;
            }
        });
        let text = value.map(cf_fmt_stop_value).unwrap_or_default();
        if let Some(input) = self.cf_stop_value_inputs.get(i) {
            input
                .clone()
                .update(cx, |inp, cx| inp.set_value(text, window, cx));
        }
    }

    /// Set a color-scale stop's colour (a stop swatch click).
    fn set_cf_stop_color(&mut self, i: usize, color: Rgb, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, move |e| {
            if let Some(stop) = e.scale.get_mut(i) {
                stop.color = color;
            }
        });
    }

    /// Select the *Cell value* operator, closing the menu.
    fn select_cf_value_op(&mut self, op: CfValueOp, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.value_op = op);
    }

    /// Select the *Text* operator, closing the menu.
    fn select_cf_text_op(&mut self, op: CfTextOp, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.text_op = op);
    }

    /// Select the *A date occurring* period, closing the menu.
    fn select_cf_period(&mut self, period: CfPeriod, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        self.with_cf_editor(cx, |e| e.period = period);
    }

    /// Apply an entire format preset (`CF_PRESETS`) to the editor's differential format.
    fn set_cf_format(&mut self, format: CfFormat, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format = format);
    }

    /// Set (or clear, `None` = "No fill") the highlight fill colour.
    fn set_cf_fill(&mut self, fill: Option<Rgb>, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.fill = fill);
    }

    /// Set (or clear, `None` = "Automatic") the highlight text colour.
    fn set_cf_text_color(&mut self, color: Option<Rgb>, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.text_color = color);
    }

    /// Toggle the highlight's bold attribute.
    fn toggle_cf_bold(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.bold = !e.format.bold);
    }

    /// Toggle the highlight's italic attribute.
    fn toggle_cf_italic(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.format.italic = !e.format.italic);
    }

    /// Set the *Stop if true* flag (the checkbox).
    fn set_cf_stop_if_true(&mut self, on: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.stop_if_true = on);
    }

    /// Set Top/Bottom to pick the bottom N (`true`) or the top N (`false`).
    fn set_cf_top_bottom(&mut self, bottom: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.top_bottom = bottom);
    }

    /// Toggle Top/Bottom counting by percent of range rather than item count.
    fn toggle_cf_top_percent(&mut self, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.top_percent = !e.top_percent);
    }

    /// Set Above/Below-average to pick below (`true`) or above (`false`) the average.
    fn set_cf_average_below(&mut self, below: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.average_below = below);
    }

    /// Set Duplicate/Unique to target unique values (`true`) or duplicates (`false`).
    fn set_cf_duplicate_unique(&mut self, unique: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.duplicate_unique = unique);
    }

    /// Set Blanks to target non-blank cells (`true`) or blanks (`false`).
    fn set_cf_blanks_no(&mut self, no_blanks: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.blanks_no = no_blanks);
    }

    /// Set Errors to target non-error cells (`true`) or errors (`false`).
    fn set_cf_errors_no(&mut self, no_errors: bool, cx: &mut Context<Self>) {
        self.with_cf_editor(cx, |e| e.errors_no = no_errors);
    }

    /// Surface an engine `Err` inline and keep the editor **open** (`functional_spec.md §8`): push
    /// the message into the editor's errors + clear the pending-save flag. Called by the window when
    /// an `EditRejectedReason::Engine` arrives while the CF editor is open. A no-op if the editor
    /// closed in the meantime.
    pub fn show_cf_editor_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            editor.pending_save = false;
            editor.errors.push(message);
            cx.notify();
        }
    }

    /// Validate + commit the editor (the Save button). Client-side validation blocks a bad form
    /// (`components/cf_sidebar.md §6`); a valid form assembles a [`CfRuleSpec`] and sends
    /// `AddCondFmt` (add) / `UpdateCondFmt` (edit). The editor stays open with `pending_save` set —
    /// a success `CondFmtUpdated` returns it to List mode ([`refresh_cond_fmt`](Self::refresh_cond_fmt)),
    /// an engine `Err` keeps it open ([`show_cf_editor_error`](Self::show_cf_editor_error)).
    pub fn save_cf_editor(&mut self, cx: &mut Context<Self>) {
        if self.degraded {
            return;
        }
        // Fold any just-typed stop value into `editor.scale` before validating/building (defensive:
        // the on-edit sync normally keeps it current).
        self.sync_cf_scale_values(cx);
        let (range, op1, op2, formula) = self.cf_input_texts(cx);
        let Some(panel) = self.cond_fmt.as_ref() else {
            return;
        };
        let Some(editor) = panel.editor.as_ref() else {
            return;
        };
        // Don't re-send while a prior Save is still awaiting its outcome (the button is disabled
        // then; this guards a direct call).
        if editor.pending_save {
            return;
        }
        // Never send a Save the live validation would block (Save is disabled anyway; this guards a
        // direct call). The inline hints already explain what's missing.
        if !cf_validate(editor, &range, &op1, &op2, &formula).is_empty() {
            return;
        }
        let sheet = panel.sheet;
        let edit_index = editor.edit_index;
        let spec = cf_build_spec(editor, &op1, &op2, &formula);
        let range = range.trim().to_string();
        match edit_index {
            None => self.client.send(Command::AddCondFmt { sheet, range, spec }),
            Some(index) => self.client.send(Command::UpdateCondFmt {
                sheet,
                index,
                range,
                spec,
            }),
        }
        self.cf_menu_open = None;
        if let Some(editor) = self.cond_fmt.as_mut().and_then(|p| p.editor.as_mut()) {
            editor.errors.clear();
            editor.pending_save = true;
        }
        cx.notify();
    }

    /// The CF sidebar's **Editor mode** body (`components/cf_sidebar.md §5`, `ui_design.md §2.2`):
    /// a back-row, the Applies-to range, the rule-type dropdown, the per-kind operands, the format
    /// editor, the Stop-if-true checkbox, the inline errors, and Save/Cancel.
    pub(super) fn render_cf_editor(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (range, op1, op2, formula) = self.cf_input_texts(cx);
        let problems = cf_validate(editor, &range, &op1, &op2, &formula);
        let range_problem = range.trim().is_empty();
        let save_disabled = self.degraded || editor.pending_save || !problems.is_empty();

        // Back-row: a chevron-left + the mode title, whole row returns to List mode.
        let title = if editor.edit_index.is_some() {
            "Edit rule"
        } else {
            "New rule"
        };
        let back = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("cf-editor-back")
                    .icon(Icon::empty().path("icons/chevron-left.svg"))
                    .ghost()
                    .small()
                    .debug_selector(|| "cf-editor-back".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.cancel_cf_editor(cx);
                    })),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child(title),
            );

        // Applies-to: the A1 range input + an inline error when empty.
        let mut applies = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(Input::new(&self.cf_range_input).small().w_full());
        if range_problem {
            applies = applies.child(cf_inline_error(CF_RANGE_REQUIRED));
        }

        // Inline validation hints (operand problems, excluding the range one already shown) +
        // engine errors, red above Save.
        let mut messages = div().flex().flex_col().gap_1();
        for p in problems.iter().filter(|p| *p != CF_RANGE_REQUIRED) {
            messages = messages.child(cf_inline_error(p.clone()));
        }
        for e in &editor.errors {
            messages = messages.child(cf_inline_error(e.clone()));
        }

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cf-editor-save")
                    .label("Save")
                    .primary()
                    .small()
                    .disabled(save_disabled)
                    .debug_selector(|| "cf-editor-save".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.save_cf_editor(cx);
                    })),
            )
            .child(
                Button::new("cf-editor-cancel")
                    .label("Cancel")
                    .ghost()
                    .small()
                    .debug_selector(|| "cf-editor-cancel".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.cancel_cf_editor(cx);
                    })),
            );

        let is_scale = matches!(
            editor.kind,
            CfEditorKind::ColorScale2 | CfEditorKind::ColorScale3
        );

        let mut form = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(back)
            .child(section("Applies to", applies))
            .child(section(
                "Rule type",
                self.render_cf_type_dropdown(editor, cx),
            ));
        if is_scale {
            // A ColorScale carries no `CfFormat` / stop-if-true — the operand + format + checkbox
            // controls are hidden; the color-scale editor takes their place.
            form = form.child(section(
                "Color scale",
                self.render_cf_scale_editor(editor, cx),
            ));
        } else {
            form = form
                .child(self.render_cf_operands(editor, cx))
                .child(section("Format", self.render_cf_format_editor(editor, cx)))
                .child(
                    Checkbox::new("cf-stop-if-true")
                        .label("Stop if true")
                        .checked(editor.stop_if_true)
                        .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                            this.set_cf_stop_if_true(*checked, cx);
                        })),
                );
        }
        form.child(messages).child(actions).into_any_element()
    }

    /// A generic inline dropdown for the editor: a full-width trigger showing `current`, and — while
    /// this `menu` is the open one — the `options` listed directly below it (`components/cf_sidebar.md
    /// §5`). `tag` scopes the `cf-{tag}-trigger` / `cf-{tag}-{opt}` debug selectors + element ids.
    /// `on_select(this, opt_index, cx)` applies the chosen option.
    fn cf_dropdown(
        &self,
        menu: CfMenu,
        tag: &'static str,
        current: &str,
        options: &[(&'static str, String, bool)],
        on_select: impl Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + Copy + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.cf_menu_open == Some(menu);
        let trigger = Button::new(SharedString::from(format!("cf-{tag}-trigger")))
            .label(current.to_string())
            .small()
            .w_full()
            .debug_selector(move || format!("cf-{tag}-trigger"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.toggle_cf_menu(menu, cx);
            }));

        let mut col = div().flex().flex_col().gap_1().child(trigger);
        if open {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .p_1()
                .rounded_md()
                .bg(rgb(CF_MENU_BG))
                .border_1()
                .border_color(rgb(HAIRLINE));
            for (i, (opt_tag, label, selected)) in options.iter().enumerate() {
                let opt_tag = *opt_tag;
                list = list.child(
                    Button::new(SharedString::from(format!("cf-{tag}-{opt_tag}")))
                        .label(label.clone())
                        .ghost()
                        .small()
                        .w_full()
                        .selected(*selected)
                        .debug_selector(move || format!("cf-{tag}-{opt_tag}"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            on_select(this, i, window, cx);
                        })),
                );
            }
            col = col.child(list);
        }
        col.into_any_element()
    }

    /// The rule-type dropdown (the highlight families + the color-scale group, per `CF_KIND_MENU`).
    fn render_cf_type_dropdown(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let current = cf_kind_label(editor.kind);
        let options: Vec<(&'static str, String, bool)> = CF_KIND_MENU
            .iter()
            .map(|(kind, tag, label)| (*tag, label.to_string(), *kind == editor.kind))
            .collect();
        self.cf_dropdown(
            CfMenu::RuleType,
            "type",
            current,
            &options,
            |this, i, window, cx| this.select_cf_kind(CF_KIND_MENU[i].0, window, cx),
            cx,
        )
    }

    /// The per-kind operand controls (`ui_design.md §2.2` step 3).
    fn render_cf_operands(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let body = match editor.kind {
            CfEditorKind::CellValue => {
                let op_current = cf_value_op_label(editor.value_op);
                let options: Vec<(&'static str, String, bool)> = CF_VALUE_OP_MENU
                    .iter()
                    .map(|(op, tag, label)| (*tag, label.to_string(), *op == editor.value_op))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::ValueOp,
                    "valueop",
                    op_current,
                    &options,
                    |this, i, _window, cx| this.select_cf_value_op(CF_VALUE_OP_MENU[i].0, cx),
                    cx,
                );
                let two = matches!(editor.value_op, CfValueOp::Between | CfValueOp::NotBetween);
                let mut inputs = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Input::new(&self.cf_operand1_input).small().w_full());
                if two {
                    inputs = inputs
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(MUTED_TEXT))
                                .child("and"),
                        )
                        .child(Input::new(&self.cf_operand2_input).small().w_full());
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(dropdown)
                    .child(inputs)
            }
            CfEditorKind::Text => {
                let op_current = cf_text_op_label(editor.text_op);
                let options: Vec<(&'static str, String, bool)> = CF_TEXT_OP_MENU
                    .iter()
                    .map(|(op, tag, label)| (*tag, label.to_string(), *op == editor.text_op))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::TextOp,
                    "textop",
                    op_current,
                    &options,
                    |this, i, _window, cx| this.select_cf_text_op(CF_TEXT_OP_MENU[i].0, cx),
                    cx,
                );
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(dropdown)
                    .child(Input::new(&self.cf_operand1_input).small().w_full())
            }
            CfEditorKind::Dates => {
                let current = cf_period_label(editor.period);
                let options: Vec<(&'static str, String, bool)> = CF_PERIOD_MENU
                    .iter()
                    .map(|(p, tag, label)| (*tag, label.to_string(), *p == editor.period))
                    .collect();
                let dropdown = self.cf_dropdown(
                    CfMenu::Period,
                    "period",
                    current,
                    &options,
                    |this, i, _window, cx| this.select_cf_period(CF_PERIOD_MENU[i].0, cx),
                    cx,
                );
                div().flex().flex_col().gap_1().child(dropdown)
            }
            CfEditorKind::TopBottom => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(cf_segmented(
                    [
                        ("cf-top-top", "Top", !editor.top_bottom),
                        ("cf-top-bottom", "Bottom", editor.top_bottom),
                    ],
                    cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_top_bottom(false, cx)),
                    cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_top_bottom(true, cx)),
                ))
                .child(Input::new(&self.cf_operand1_input).small().w_full())
                .child(
                    Button::new("cf-top-percent")
                        .label("% of range")
                        .ghost()
                        .small()
                        .selected(editor.top_percent)
                        .debug_selector(|| "cf-top-percent".to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.toggle_cf_top_percent(cx);
                        })),
                ),
            CfEditorKind::Average => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-avg-above", "Above average", !editor.average_below),
                    ("cf-avg-below", "Below average", editor.average_below),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_average_below(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_average_below(true, cx)),
            )),
            CfEditorKind::Duplicate => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-dup-duplicate", "Duplicate", !editor.duplicate_unique),
                    ("cf-dup-unique", "Unique", editor.duplicate_unique),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_duplicate_unique(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_duplicate_unique(true, cx)),
            )),
            CfEditorKind::Blanks => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-blank-blank", "Blank", !editor.blanks_no),
                    ("cf-blank-noblank", "No blanks", editor.blanks_no),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_blanks_no(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_blanks_no(true, cx)),
            )),
            CfEditorKind::Errors => div().flex().flex_col().gap_1().child(cf_segmented(
                [
                    ("cf-err-error", "Error", !editor.errors_no),
                    ("cf-err-noerror", "No errors", editor.errors_no),
                ],
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_errors_no(false, cx)),
                cx.listener(|this, _: &ClickEvent, _w, cx| this.set_cf_errors_no(true, cx)),
            )),
            CfEditorKind::Formula => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(Input::new(&self.cf_formula_input).small().w_full()),
            // Color scales render `render_cf_scale_editor` instead — `render_cf_editor` never calls
            // this for them; the arm only keeps the match exhaustive.
            CfEditorKind::ColorScale2 | CfEditorKind::ColorScale3 => div(),
        };
        section("Condition", body).into_any_element()
    }

    /// The differential-format editor (`components/cf_sidebar.md §7`): preset chips, fill + text
    /// swatch palettes, Bold/Italic toggles, and a live preview cell.
    fn render_cf_format_editor(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // Preset chips — each paints its own look and sets the whole format in one click.
        let mut presets = div().flex().flex_wrap().gap(px(4.0));
        for (i, (label, fmt)) in CF_PRESETS.iter().enumerate() {
            let fmt = *fmt;
            let bg = fmt.fill.map(cf_color).unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
            let fg = fmt.text_color.map(cf_color).unwrap_or_else(|| rgb(TEXT));
            presets = presets.child(
                div()
                    .id(SharedString::from(format!("cf-preset-{i}")))
                    .debug_selector(move || format!("cf-preset-{i}"))
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .bg(bg)
                    .text_size(px(10.5))
                    .text_color(fg)
                    .when(fmt.bold, |d| d.font_weight(gpui::FontWeight::BOLD))
                    .when(fmt.italic, |d| d.italic())
                    .cursor_pointer()
                    .child(SharedString::from(*label))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.set_cf_format(fmt, cx);
                        }),
                    ),
            );
        }

        let fill_row = self.render_cf_color_row(
            "fill",
            "Fill",
            editor.format.fill,
            "No fill",
            |this, color, cx| this.set_cf_fill(color, cx),
            cx,
        );
        let text_row = self.render_cf_color_row(
            "text",
            "Text",
            editor.format.text_color,
            "Automatic",
            |this, color, cx| this.set_cf_text_color(color, cx),
            cx,
        );

        let bi = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("cf-bold")
                    .icon(Icon::empty().path("icons/bold.svg"))
                    .tooltip("Bold")
                    .ghost()
                    .small()
                    .selected(editor.format.bold)
                    .debug_selector(|| "cf-bold".to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.toggle_cf_bold(cx))),
            )
            .child(
                Button::new("cf-italic")
                    .icon(Icon::empty().path("icons/italic.svg"))
                    .tooltip("Italic")
                    .ghost()
                    .small()
                    .selected(editor.format.italic)
                    .debug_selector(|| "cf-italic".to_string())
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _w, cx| this.toggle_cf_italic(cx)),
                    ),
            );

        // Live preview: a sample cell painted with the working format.
        let preview_bg = editor
            .format
            .fill
            .map(cf_color)
            .unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
        let preview_fg = editor
            .format
            .text_color
            .map(cf_color)
            .unwrap_or_else(|| rgb(TEXT));
        let preview = div()
            .debug_selector(|| "cf-format-preview".to_string())
            .px(px(8.0))
            .py(px(4.0))
            .rounded_sm()
            .border_1()
            .border_color(rgb(HAIRLINE))
            .bg(preview_bg)
            .text_color(preview_fg)
            .text_size(px(12.0))
            .when(editor.format.bold, |d| {
                d.font_weight(gpui::FontWeight::BOLD)
            })
            .when(editor.format.italic, |d| d.italic())
            .child("123 Abc");

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(presets)
            .child(fill_row)
            .child(text_row)
            .child(bi)
            .child(preview)
            .into_any_element()
    }

    /// A labelled compact colour palette (fill or text): the `FILL_PALETTE` swatches (the current
    /// one ringed) + a "none" entry (`No fill` / `Automatic`). `on_pick(this, Option<Rgb>, cx)`
    /// applies the choice.
    fn render_cf_color_row(
        &self,
        tag: &'static str,
        label: &'static str,
        current: Option<Rgb>,
        none_label: &'static str,
        on_pick: impl Fn(&mut Self, Option<Rgb>, &mut Context<Self>) + Copy + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut swatches = div().flex().flex_wrap().items_center().gap(px(3.0));
        for sw in FILL_PALETTE {
            let selected = current == Some(sw.rgb);
            let color = sw.rgb;
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("cf-{tag}-{}", sw.name)))
                    .debug_selector(move || format!("cf-{tag}-{}", sw.name))
                    .w(px(CF_SWATCH_SIDE))
                    .h(px(CF_SWATCH_SIDE))
                    .rounded_sm()
                    .bg(rgb(sw.rgb.to_hex()))
                    .border_1()
                    .border_color(rgb(if selected {
                        SWATCH_SELECTED_RING
                    } else {
                        HAIRLINE
                    }))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            on_pick(this, Some(color), cx);
                        }),
                    ),
            );
        }
        swatches = swatches.child(
            Button::new(SharedString::from(format!("cf-{tag}-none")))
                .label(none_label)
                .ghost()
                .small()
                .selected(current.is_none())
                .debug_selector(move || format!("cf-{tag}-none"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    on_pick(this, None, cx);
                })),
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(MUTED_TEXT))
                    .child(label),
            )
            .child(swatches)
            .into_any_element()
    }

    /// The color-scale editor (`components/cf_sidebar.md §8`): a 2-vs-3 arity control, per-arity
    /// default presets, one stop row per stop (threshold-kind dropdown, an optional value input, and
    /// a colour swatch grid), and a horizontal gradient preview across the stop colours.
    fn render_cf_scale_editor(
        &self,
        editor: &CfEditorState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let three = editor.kind == CfEditorKind::ColorScale3;

        // 2-vs-3 arity segmented control (switching resizes `scale`, preserving the endpoints).
        let arity = cf_segmented(
            [
                ("cf-scale-2color", "2 color", !three),
                ("cf-scale-3color", "3 color", three),
            ],
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.set_cf_scale_arity(false, window, cx)
            }),
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.set_cf_scale_arity(true, window, cx)
            }),
        );

        // Preset chips of the current arity — each fills `scale` with its colours in one click.
        let presets: Vec<(&'static str, &'static [u32])> = if three {
            CF_SCALE_PRESETS_3
                .iter()
                .map(|(label, colors)| (*label, &colors[..]))
                .collect()
        } else {
            CF_SCALE_PRESETS_2
                .iter()
                .map(|(label, colors)| (*label, &colors[..]))
                .collect()
        };
        let mut preset_row = div().flex().flex_wrap().gap(px(6.0));
        for (i, (label, colors)) in presets.into_iter().enumerate() {
            let mut gradient = div()
                .w(px(CF_SWATCH_W * 2.0))
                .h(px(CF_SWATCH_H))
                .rounded_sm()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .flex();
            for hex in colors {
                gradient =
                    gradient.child(div().flex_1().h_full().bg(cf_color(Rgb::from_hex(*hex))));
            }
            preset_row = preset_row.child(
                div()
                    .id(SharedString::from(format!("cf-scale-preset-{i}")))
                    .debug_selector(move || format!("cf-scale-preset-{i}"))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.0))
                    .cursor_pointer()
                    .child(gradient)
                    .child(
                        div()
                            .text_size(px(9.0))
                            .text_color(rgb(MUTED_TEXT))
                            .child(SharedString::from(label)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.apply_cf_scale_preset(colors, window, cx);
                        }),
                    ),
            );
        }

        // One row per stop: position label, threshold-kind dropdown, an optional value input, and a
        // colour swatch grid.
        // Author only 2 or 3 stops; a (pathological) loaded scale with more is clamped to the 3
        // available stop inputs/tags for rendering — its extra stops still round-trip via the spec.
        let len = editor.scale.len();
        let mut stops = div().flex().flex_col().gap_2();
        for (i, stop) in editor.scale.iter().enumerate().take(CF_STOP_TAGS.len()) {
            let pos = cf_stop_pos(i, len);
            let kinds: &'static [(CfThresholdKind, &'static str, &'static str)] = match pos {
                StopPos::First => &CF_STOP_KIND_MIN,
                StopPos::Last => &CF_STOP_KIND_MAX,
                StopPos::Mid => &CF_STOP_KIND_MID,
            };
            let options: Vec<(&'static str, String, bool)> = kinds
                .iter()
                .map(|(kind, tag, label)| (*tag, label.to_string(), *kind == stop.kind))
                .collect();
            let dropdown = self.cf_dropdown(
                CfMenu::StopKind(i),
                CF_STOP_TAGS[i],
                cf_threshold_kind_label(stop.kind),
                &options,
                move |this, opt, window, cx| this.set_cf_stop_kind(i, kinds[opt].0, window, cx),
                cx,
            );

            let mut row = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgb(MUTED_TEXT))
                        .child(cf_stop_label(pos)),
                )
                .child(dropdown);
            if cf_stop_needs_value(stop.kind) && i < self.cf_stop_value_inputs.len() {
                row = row.child(Input::new(&self.cf_stop_value_inputs[i]).small().w_full());
            }
            row = row.child(self.render_cf_stop_color(i, stop.color, cx));
            stops = stops.child(row);
        }

        // Horizontal gradient preview: equal bands across the stop colours (a stepped gradient).
        let mut preview = div()
            .debug_selector(|| "cf-scale-preview".to_string())
            .w_full()
            .h(px(18.0))
            .rounded_sm()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(HAIRLINE))
            .flex();
        for stop in &editor.scale {
            preview = preview.child(div().flex_1().h_full().bg(cf_color(stop.color)));
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(arity)
            .child(preset_row)
            .child(stops)
            .child(preview)
            .into_any_element()
    }

    /// A color-scale stop's colour picker: the `FILL_PALETTE` swatch grid (the P6 fill/text palette,
    /// minus the "none" entry — a stop always has a colour), the current one ringed.
    fn render_cf_stop_color(
        &self,
        i: usize,
        current: Rgb,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // Share the stop's `stop{i}` tag with its threshold dropdown (`cf-stop0-…`).
        let tag = CF_STOP_TAGS[i.min(CF_STOP_TAGS.len() - 1)];
        let mut swatches = div().flex().flex_wrap().items_center().gap(px(3.0));
        for sw in FILL_PALETTE {
            let selected = current == sw.rgb;
            let color = sw.rgb;
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("cf-{tag}-{}", sw.name)))
                    .debug_selector(move || format!("cf-{tag}-{}", sw.name))
                    .w(px(CF_SWATCH_SIDE))
                    .h(px(CF_SWATCH_SIDE))
                    .rounded_sm()
                    .bg(rgb(sw.rgb.to_hex()))
                    .border_1()
                    .border_color(rgb(if selected {
                        SWATCH_SELECTED_RING
                    } else {
                        HAIRLINE
                    }))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.set_cf_stop_color(i, color, cx);
                        }),
                    ),
            );
        }
        swatches.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::cf_sidebar::cf_row_controls;
    use super::*;
    use crate::chrome::view::test_support::*;
    use gpui::TestAppContext;

    // ---- CF rule editor (P6) --------------------------------------------------------------

    /// A B2:B20 selection (anchor B2 → active B20).
    fn selection_b2_b20() -> SelectionModel {
        SelectionModel {
            anchor: cell(1, 1),
            active: cell(19, 1),
        }
    }

    /// A published, editable highlight row carrying `spec` (so edit mode can seed from it). The
    /// preview mirrors the spec's format; `cf_view` (spec: None) is the deferred-family case.
    fn cf_spec_view(index: u32, range: &str, summary: &str, spec: CfRuleSpec) -> CfRuleView {
        let preview = match spec.format() {
            Some(f) => CfPreview::Highlight {
                fill: f.fill,
                text_color: f.text_color,
            },
            None => CfPreview::ColorScale { colors: vec![] },
        };
        CfRuleView {
            index,
            range: range.to_string(),
            priority: 0,
            editable: true,
            summary: summary.to_string(),
            preview,
            spec: Some(spec),
        }
    }

    /// The open editor's text-input values `(range, operand1)`.
    fn cf_input_values(h: &Harness, cx: &mut TestAppContext) -> (String, String) {
        upd(h, cx, |c, _w, cx| {
            (
                c.cf_range_input.read(cx).value().to_string(),
                c.cf_operand1_input.read(cx).value().to_string(),
            )
        })
    }

    #[gpui::test]
    fn cf_add_editor_seeds_range_from_selection(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.selection = selection_b2_b20();
            c.open_cf_editor(None, window, cx);
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "opening the add editor enters Editor mode"
        );
        assert_eq!(
            cf_input_values(&h, cx).0,
            "B2:B20",
            "the Applies-to range seeds from the current selection"
        );
    }

    #[gpui::test]
    fn cf_add_cell_value_rule_saves_add_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.selection = selection_b2_b20();
            c.open_cf_editor(None, window, cx);
            // Default kind = Cell value, op = greater than. Enter the operand + pick a preset.
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("100", window, cx));
            c.set_cf_format(CF_PRESETS[0].1, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        let expected = CfRuleSpec::CellIs {
            op: CfValueOp::Gt,
            operand: "100".to_string(),
            operand2: None,
            format: CF_PRESETS[0].1,
            stop_if_true: false,
        };
        match h.client.take_commands().as_slice() {
            [Command::AddCondFmt { sheet, range, spec }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(range, "B2:B20");
                assert_eq!(*spec, expected);
            }
            other => panic!("expected one AddCondFmt, got {other:?}"),
        }
    }

    #[gpui::test]
    fn cf_save_success_returns_to_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("5", window, cx));
            c.save_cf_editor(cx);
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "the editor stays open pending the save's CondFmtUpdated"
        );
        // The worker accepted the rule + republished → CondFmtUpdated → refresh returns to List mode.
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "a successful save returns to List mode"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "the sidebar itself stays open"
        );
    }

    #[gpui::test]
    fn cf_edit_row_seeds_form_and_saves_update(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let spec = CfRuleSpec::CellIs {
            op: CfValueOp::Ge,
            operand: "50".to_string(),
            operand2: None,
            format: CF_PRESETS[2].1,
            stop_if_true: false,
        };
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_spec_view(4, "C1:C9", "Cell value ≥ 50", spec.clone())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(Some(4), window, cx);
        });
        // The form seeds from the row's spec: range, operand, and the editor state.
        let (range, operand1) = cf_input_values(&h, cx);
        assert_eq!(range, "C1:C9", "the range seeds from the edited row");
        assert_eq!(operand1, "50", "the operand seeds from the spec");
        upd(&h, cx, |c, _w, _cx| {
            let e = c
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .expect("editor open");
            assert_eq!(e.edit_index, Some(4));
            assert_eq!(e.kind, CfEditorKind::CellValue);
            assert_eq!(e.value_op, CfValueOp::Ge);
            assert_eq!(e.format, CF_PRESETS[2].1);
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        match h.client.take_commands().as_slice() {
            [Command::UpdateCondFmt {
                sheet,
                index,
                range,
                spec: got,
            }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(*index, 4);
                assert_eq!(range, "C1:C9");
                assert_eq!(*got, spec);
            }
            other => panic!("expected one UpdateCondFmt, got {other:?}"),
        }
    }

    #[gpui::test]
    fn cf_empty_operand_blocks_save(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx); // Cell value, operand left empty
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        assert!(
            h.client.take_commands().is_empty(),
            "an empty Cell-value operand blocks Save (no command sent)"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "the editor stays open when Save is blocked"
        );
    }

    #[gpui::test]
    fn cf_rule_type_dropdown_selects_kind(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // Open the rule-type dropdown, then pick "Text".
        let trigger = vcx
            .debug_bounds("cf-type-trigger")
            .expect("the rule-type trigger is painted");
        vcx.simulate_click(trigger.center(), Modifiers::default());
        let text_opt = vcx
            .debug_bounds("cf-type-text")
            .expect("the Text option is painted while the dropdown is open");
        vcx.simulate_click(text_opt.center(), Modifiers::default());
        let kind = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .kind
        });
        assert_eq!(kind, CfEditorKind::Text, "the dropdown drives the kind");
    }

    #[gpui::test]
    fn cf_preset_sets_format(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let preset = vcx
            .debug_bounds("cf-preset-0")
            .expect("the first format preset chip is painted");
        vcx.simulate_click(preset.center(), Modifiers::default());
        let format = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .format
        });
        assert_eq!(
            format, CF_PRESETS[0].1,
            "clicking a preset chip sets the whole differential format"
        );
    }

    #[gpui::test]
    fn cf_preview_reflects_format(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            // A preset with a fill; the live preview cell must paint.
            c.set_cf_format(CF_PRESETS[0].1, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-format-preview").is_some(),
            "the live preview cell renders for the current format"
        );
        let fill = vcx.update(|_w, app| {
            h.chrome
                .read(app)
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .format
                .fill
        });
        assert_eq!(
            fill,
            Some(Rgb::from_hex(0xFFC7CE)),
            "the preview is driven by the editor's working fill"
        );
    }

    #[gpui::test]
    fn cf_cancel_returns_to_list(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.cf_editor_open()));
        upd(&h, cx, |c, _w, cx| c.cancel_cf_editor(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "Cancel leaves Editor mode"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "Cancel returns to List mode, sidebar still open"
        );
    }

    #[gpui::test]
    fn cf_engine_error_keeps_editor_open(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.cf_operand1_input
                .clone()
                .update(cx, |i, cx| i.set_value("5", window, cx));
            c.save_cf_editor(cx);
        });
        // The worker refuses the rule (e.g. a bad range) → the window routes it inline.
        upd(&h, cx, |c, _w, cx| {
            c.show_cf_editor_error("Invalid range".to_string(), cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cf_editor_open()),
            "an engine error keeps the editor open"
        );
        let errors = upd(&h, cx, |c, _w, _cx| {
            c.cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .errors
                .clone()
        });
        assert_eq!(errors, vec!["Invalid range".to_string()]);
    }

    #[test]
    fn cf_color_scale_row_edit_enabled() {
        // P7 re-enables ✎ for an editable (concrete-RGB) color-scale row — it opens the color-scale
        // editor. A theme-colored scale is a Badge with `editable == false` and stays non-editable.
        let scale = CfRuleView {
            index: 3,
            range: "A1:A9".to_string(),
            priority: 0,
            editable: true,
            summary: "3-color scale".to_string(),
            preview: CfPreview::ColorScale {
                colors: vec![Rgb::from_hex(0x63BE7B), Rgb::from_hex(0xF8696B)],
            },
            spec: Some(CfRuleSpec::ColorScale { stops: vec![] }),
        };
        let controls = cf_row_controls(&scale, false, false);
        assert!(
            controls.edit,
            "an editable color-scale row can now be edited"
        );
        assert!(controls.delete, "and it stays deletable");

        let badge = cf_view(
            4,
            "B1:B9",
            "3-color scale",
            false, // a theme-colored scale P1 could not resolve to concrete RGB
            CfPreview::Badge("Color scale".to_string()),
        );
        assert!(
            !cf_row_controls(&badge, false, false).edit,
            "a non-editable (theme-colored) scale Badge stays non-editable"
        );
    }

    /// The open editor's working color-scale stops.
    fn cf_scale_stops(h: &Harness, cx: &mut TestAppContext) -> Vec<CfColorStop> {
        upd(h, cx, |c, _w, _cx| {
            c.cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .expect("editor open")
                .scale
                .clone()
        })
    }

    #[gpui::test]
    fn cf_add_color_scale_saves_add_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.selection = selection_b2_b20();
            c.open_cf_editor(None, window, cx);
            // Switch to a 3-color scale (seeds the default green-yellow-red preset stops).
            c.select_cf_kind(CfEditorKind::ColorScale3, window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        match h.client.take_commands().as_slice() {
            [Command::AddCondFmt { sheet, range, spec }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(range, "B2:B20");
                match spec {
                    CfRuleSpec::ColorScale { stops } => {
                        assert_eq!(stops.len(), 3, "a 3-color scale saves 3 stops");
                        assert_eq!(stops[0].kind, CfThresholdKind::Min, "first stop is Min");
                        assert_eq!(
                            stops[1].kind,
                            CfThresholdKind::Percentile,
                            "midpoint defaults to a percentile"
                        );
                        assert_eq!(
                            stops[1].value,
                            Some(50.0),
                            "the midpoint is the 50th percentile"
                        );
                        assert_eq!(stops[2].kind, CfThresholdKind::Max, "last stop is Max");
                    }
                    other => panic!("expected a ColorScale spec, got {other:?}"),
                }
            }
            other => panic!("expected one AddCondFmt, got {other:?}"),
        }
    }

    #[gpui::test]
    fn cf_scale_preset_sets_stops(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.select_cf_kind(CfEditorKind::ColorScale3, window, cx);
            // Apply the red-yellow-green preset (the second 3-color preset).
            c.apply_cf_scale_preset(&CF_SCALE_PRESETS_3[1].1, window, cx);
        });
        let stops = cf_scale_stops(&h, cx);
        let colors: Vec<u32> = stops.iter().map(|s| s.color.to_hex()).collect();
        assert_eq!(
            colors,
            CF_SCALE_PRESETS_3[1].1.to_vec(),
            "a preset fills the stop colours"
        );
    }

    #[gpui::test]
    fn cf_scale_arity_toggle_resizes(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.select_cf_kind(CfEditorKind::ColorScale2, window, cx);
        });
        assert_eq!(
            cf_scale_stops(&h, cx).len(),
            2,
            "a 2-color scale has 2 stops"
        );
        let ends: Vec<Rgb> = cf_scale_stops(&h, cx).iter().map(|s| s.color).collect();

        upd(&h, cx, |c, window, cx| {
            c.set_cf_scale_arity(true, window, cx)
        });
        let three = cf_scale_stops(&h, cx);
        assert_eq!(three.len(), 3, "toggling to 3 color resizes to 3 stops");
        assert_eq!(three[0].color, ends[0], "the min endpoint is preserved");
        assert_eq!(three[2].color, ends[1], "the max endpoint is preserved");
        assert_eq!(
            three[1].kind,
            CfThresholdKind::Percentile,
            "the inserted midpoint is a percentile"
        );

        upd(&h, cx, |c, window, cx| {
            c.set_cf_scale_arity(false, window, cx)
        });
        assert_eq!(
            cf_scale_stops(&h, cx).len(),
            2,
            "toggling back to 2 color drops the midpoint"
        );
    }

    #[gpui::test]
    fn cf_scale_editor_renders(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(None, window, cx);
            c.select_cf_kind(CfEditorKind::ColorScale3, window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // The scale editor paints: the arity control, a preset chip, all 3 stop dropdowns + swatch
        // grids, and the gradient preview. (Also proves the render path can't panic.)
        for sel in [
            "cf-scale-3color",
            "cf-scale-preset-0",
            "cf-stop0-trigger",
            "cf-stop1-trigger",
            "cf-stop2-trigger",
            "cf-stop0-Accent 1",
            "cf-scale-preview",
        ] {
            assert!(
                vcx.debug_bounds(sel).is_some(),
                "{sel} must paint in the color-scale editor"
            );
        }
        // The format editor + stop-if-true are hidden for a color scale.
        assert!(
            vcx.debug_bounds("cf-format-preview").is_none(),
            "the highlight format editor is hidden for a color scale"
        );
    }

    #[gpui::test]
    fn cf_edit_color_scale_seeds_and_saves_update(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
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
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_spec_view(7, "C1:C9", "3-color scale", spec.clone())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.open_cf_editor(Some(7), window, cx);
        });
        // The editor seeds the scale kind + all 3 stops from the row's spec.
        upd(&h, cx, |c, _w, _cx| {
            let e = c
                .cond_fmt
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .expect("editor open");
            assert_eq!(e.edit_index, Some(7));
            assert_eq!(e.kind, CfEditorKind::ColorScale3);
            assert_eq!(e.scale.len(), 3, "an editable 3-stop scale seeds 3 stops");
        });
        h.client.take_commands();
        upd(&h, cx, |c, _w, cx| c.save_cf_editor(cx));
        match h.client.take_commands().as_slice() {
            [Command::UpdateCondFmt {
                sheet,
                index,
                range,
                spec: got,
            }] => {
                assert_eq!(*sheet, SheetId(0));
                assert_eq!(*index, 7);
                assert_eq!(range, "C1:C9");
                assert_eq!(*got, spec, "editing a scale sends the same stops back");
            }
            other => panic!("expected one UpdateCondFmt, got {other:?}"),
        }
    }

    #[test]
    fn cf_scale_number_stop_without_value_blocks_save() {
        // A Number/Percent/Percentile stop with no value must block Save; Min/Max need none.
        let editor = cf_state(|s| {
            s.kind = CfEditorKind::ColorScale2;
            s.scale = vec![
                CfColorStop {
                    kind: CfThresholdKind::Number,
                    value: None,
                    color: Rgb::from_hex(0x63BE7B),
                },
                CfColorStop {
                    kind: CfThresholdKind::Max,
                    value: None,
                    color: Rgb::from_hex(0xF8696B),
                },
            ];
        });
        assert!(
            !cf_validate(&editor, "A1:A9", "", "", "").is_empty(),
            "a Number stop with no value blocks Save"
        );

        let ok = cf_state(|s| {
            s.kind = CfEditorKind::ColorScale2;
            s.scale = vec![
                CfColorStop {
                    kind: CfThresholdKind::Number,
                    value: Some(10.0),
                    color: Rgb::from_hex(0x63BE7B),
                },
                CfColorStop {
                    kind: CfThresholdKind::Max,
                    value: None,
                    color: Rgb::from_hex(0xF8696B),
                },
            ];
        });
        assert!(
            cf_validate(&ok, "A1:A9", "", "", "").is_empty(),
            "a filled Number stop + a Max endpoint validates"
        );
    }

    /// A minimal editor state (fresh add-defaults, `edit_index == Some(1)`) with only the
    /// kind-relevant fields set — the exact shape [`cf_state_from_spec`] reconstructs, so a
    /// build→seed round-trip can assert full equality.
    fn cf_state(mutate: impl FnOnce(&mut CfEditorState)) -> CfEditorState {
        let mut s = CfEditorState::new(Some(1));
        mutate(&mut s);
        s
    }

    /// The exhaustive mapping + round-trip guard: for **every** highlight `CfEditorKind` and its
    /// sub-toggles, `cf_build_spec` must produce the expected `CfRuleSpec` variant+fields, and
    /// `cf_state_from_spec(build(state))` must reproduce the state + operand strings exactly. This
    /// red-flags any future field transposition (e.g. `blanks_no`↔`errors_no`, or a swapped
    /// `bottom`/`percent`) that the per-kind mappings would otherwise hide. Pure — no gpui context.
    #[test]
    fn cf_build_spec_and_state_round_trip_cover_every_highlight_kind() {
        use CfEditorKind::*;
        // A non-default format proves the fill/text/bold/italic + stop_if_true seed round-trips
        // across non-CellValue kinds (used by Text + Formula below).
        let fancy = CfFormat {
            fill: Some(Rgb::from_hex(0xC6EFCE)),
            text_color: Some(Rgb::from_hex(0x006100)),
            bold: true,
            italic: true,
        };
        let plain = CfFormat::default();

        // (label, state, operand1, operand2, formula, expected spec)
        let cases: Vec<(&str, CfEditorState, &str, &str, &str, CfRuleSpec)> = vec![
            (
                "cell value gt",
                cf_state(|s| s.value_op = CfValueOp::Gt),
                "100",
                "",
                "",
                CfRuleSpec::CellIs {
                    op: CfValueOp::Gt,
                    operand: "100".to_string(),
                    operand2: None,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "cell value between",
                cf_state(|s| {
                    s.value_op = CfValueOp::Between;
                    s.stop_if_true = true;
                }),
                "10",
                "20",
                "",
                CfRuleSpec::CellIs {
                    op: CfValueOp::Between,
                    operand: "10".to_string(),
                    operand2: Some("20".to_string()),
                    format: plain,
                    stop_if_true: true,
                },
            ),
            (
                "text contains (fancy format)",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::Contains;
                    s.format = fancy;
                    s.stop_if_true = true;
                }),
                "foo",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::Contains,
                    value: "foo".to_string(),
                    format: fancy,
                    stop_if_true: true,
                },
            ),
            (
                "text not-contains",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::NotContains;
                }),
                "bar",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::NotContains,
                    value: "bar".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text begins-with",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::BeginsWith;
                }),
                "pre",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::BeginsWith,
                    value: "pre".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text ends-with",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::EndsWith;
                }),
                "post",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::EndsWith,
                    value: "post".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "text equals",
                cf_state(|s| {
                    s.kind = Text;
                    s.text_op = CfTextOp::Equals;
                }),
                "exact",
                "",
                "",
                CfRuleSpec::Text {
                    op: CfTextOp::Equals,
                    value: "exact".to_string(),
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "dates today",
                cf_state(|s| {
                    s.kind = Dates;
                    s.period = CfPeriod::Today;
                }),
                "",
                "",
                "",
                CfRuleSpec::TimePeriod {
                    period: CfPeriod::Today,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "dates last-month",
                cf_state(|s| {
                    s.kind = Dates;
                    s.period = CfPeriod::LastMonth;
                }),
                "",
                "",
                "",
                CfRuleSpec::TimePeriod {
                    period: CfPeriod::LastMonth,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "top N",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 25;
                    s.top_percent = false;
                    s.top_bottom = false;
                }),
                "25",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 25,
                    percent: false,
                    bottom: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "top percent",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 10;
                    s.top_percent = true;
                    s.top_bottom = false;
                }),
                "10",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 10,
                    percent: true,
                    bottom: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "bottom N",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 5;
                    s.top_percent = false;
                    s.top_bottom = true;
                }),
                "5",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 5,
                    percent: false,
                    bottom: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "bottom percent",
                cf_state(|s| {
                    s.kind = TopBottom;
                    s.top_rank = 15;
                    s.top_percent = true;
                    s.top_bottom = true;
                }),
                "15",
                "",
                "",
                CfRuleSpec::Top {
                    rank: 15,
                    percent: true,
                    bottom: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "above average",
                cf_state(|s| {
                    s.kind = Average;
                    s.average_below = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Average {
                    below: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "below average",
                cf_state(|s| {
                    s.kind = Average;
                    s.average_below = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Average {
                    below: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "duplicate values",
                cf_state(|s| {
                    s.kind = Duplicate;
                    s.duplicate_unique = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::DuplicateValues {
                    unique: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "unique values",
                cf_state(|s| {
                    s.kind = Duplicate;
                    s.duplicate_unique = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::DuplicateValues {
                    unique: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "blank",
                cf_state(|s| {
                    s.kind = Blanks;
                    s.blanks_no = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Blanks {
                    no_blanks: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "no blanks",
                cf_state(|s| {
                    s.kind = Blanks;
                    s.blanks_no = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Blanks {
                    no_blanks: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "error",
                cf_state(|s| {
                    s.kind = Errors;
                    s.errors_no = false;
                }),
                "",
                "",
                "",
                CfRuleSpec::Errors {
                    no_errors: false,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "no errors",
                cf_state(|s| {
                    s.kind = Errors;
                    s.errors_no = true;
                }),
                "",
                "",
                "",
                CfRuleSpec::Errors {
                    no_errors: true,
                    format: plain,
                    stop_if_true: false,
                },
            ),
            (
                "formula (fancy format)",
                cf_state(|s| {
                    s.kind = Formula;
                    s.format = fancy;
                    s.stop_if_true = true;
                }),
                "",
                "",
                "=A1>0",
                CfRuleSpec::Formula {
                    formula: "=A1>0".to_string(),
                    format: fancy,
                    stop_if_true: true,
                },
            ),
        ];

        // Sanity: every highlight kind is represented at least once.
        let covered: Vec<CfEditorKind> = cases.iter().map(|(_, s, ..)| s.kind).collect();
        for kind in [
            CellValue, Text, Dates, TopBottom, Average, Duplicate, Blanks, Errors, Formula,
        ] {
            assert!(
                covered.contains(&kind),
                "the table must exercise every highlight kind (missing {kind:?})"
            );
        }

        for (label, state, op1, op2, formula, expected) in cases {
            // Forward: the editor state maps to the expected engine spec.
            let built = cf_build_spec(&state, op1, op2, formula);
            assert_eq!(built, expected, "cf_build_spec mismatch for `{label}`");
            // Round-trip: seeding the editor back from that spec reproduces the state exactly.
            let (state2, r1, r2, rf) =
                cf_state_from_spec(1, &built).expect("highlight specs are authorable");
            assert_eq!(state2, state, "state round-trip mismatch for `{label}`");
            assert_eq!(
                (r1.as_str(), r2.as_str(), rf.as_str()),
                (op1, op2, formula),
                "operand round-trip mismatch for `{label}`"
            );
        }
    }

    /// The color-scale sibling of the highlight round-trip guard: for a 2-stop and a 3-stop scale,
    /// `cf_build_spec` must produce the expected `ColorScale` spec, and `cf_state_from_spec(build)`
    /// must reproduce the editor state (kind + scale) exactly. Pure — no gpui context.
    #[test]
    fn cf_build_spec_and_state_round_trip_color_scale() {
        let two = cf_stops_from_colors(&CF_SCALE_PRESETS_2[0].1);
        let three_stops = vec![
            CfColorStop {
                kind: CfThresholdKind::Number,
                value: Some(0.0),
                color: Rgb::from_hex(0x63BE7B),
            },
            CfColorStop {
                kind: CfThresholdKind::Percent,
                value: Some(50.0),
                color: Rgb::from_hex(0xFFEB84),
            },
            CfColorStop {
                kind: CfThresholdKind::Max,
                value: None,
                color: Rgb::from_hex(0xF8696B),
            },
        ];

        let cases: Vec<(&str, CfEditorState, CfRuleSpec)> = vec![
            (
                "2-color (Min/Max default preset)",
                cf_state(|s| {
                    s.kind = CfEditorKind::ColorScale2;
                    s.scale = two.clone();
                }),
                CfRuleSpec::ColorScale { stops: two.clone() },
            ),
            (
                "3-color (Number/Percent/Max mix)",
                cf_state(|s| {
                    s.kind = CfEditorKind::ColorScale3;
                    s.scale = three_stops.clone();
                }),
                CfRuleSpec::ColorScale {
                    stops: three_stops.clone(),
                },
            ),
        ];

        for (label, state, expected) in cases {
            let built = cf_build_spec(&state, "", "", "");
            assert_eq!(built, expected, "cf_build_spec mismatch for `{label}`");
            let (state2, r1, r2, rf) =
                cf_state_from_spec(1, &built).expect("color scales are authorable");
            assert_eq!(state2, state, "state round-trip mismatch for `{label}`");
            assert_eq!(
                (r1.as_str(), r2.as_str(), rf.as_str()),
                ("", "", ""),
                "a color scale seeds no operand/formula text for `{label}`"
            );
        }
    }
}
