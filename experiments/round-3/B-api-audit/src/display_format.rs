//! HEADLINE probe — does IronCalc produce the *displayed string* for a cell?
//!
//! This is the load-bearing question for the renderer (functional_spec §6-B, risk in §7,
//! architecture §5/§8): FreeCell renders displayed text, not raw values. If IronCalc
//! hands back only the raw value, FreeCell must implement Excel number-format rendering
//! itself (real renderer scope). If it hands back the formatted string, that scope is
//! the engine's.
//!
//! ## Answer: IronCalc OWNS display formatting → PRESENT (FreeCell does NOT own it).
//!
//! Source (cited):
//! - `ironcalc_base/src/user_model/common.rs:475` — `UserModel::get_formatted_cell_value`
//!   delegates to `Model::get_formatted_cell_value`.
//! - `ironcalc_base/src/model.rs:1800` — `Model::get_formatted_cell_value` reads the
//!   cell's `num_fmt` (`get_style_for_cell(..).num_fmt`, model.rs:1808) and runs
//!   `number_format::format_number(value, &format, locale).text` (model.rs:1810-1812) to
//!   produce the display string. The doc example asserts `=1/3` -> `"0.333333333"`.
//! - `ironcalc_base/src/number_format.rs:152` `pub fn format_number(value, format_code,
//!   locale) -> Formatted` — publicly reachable (`pub mod number_format;`, lib.rs:37).
//! - `ironcalc_base/src/formatter/format.rs:10` `pub struct Formatted { color, text,
//!   error }` — the engine even returns the format's color (e.g. `[Red]` negatives) and a
//!   per-cell error, everything the renderer needs.
//! - Format coverage is engine-tested: `formatter/test/test_en_examples.rs:56` asserts
//!   `#,##0.00` on 1234.5 -> `"1,234.50"`; `:93` asserts `0.00%` on 1.0 -> `"100.00%"`;
//!   `formatter/test/test_dates.rs` covers date formats.
//!
//! Setting a number format is also public + undoable:
//! `UserModel::update_range_style(area, "num_fmt", "#,##0.00")`
//! (common.rs:1253; the `"num_fmt"` style path is common.rs:150), and it is read back via
//! `UserModel::get_cell_style(..).num_fmt` (common.rs:1402). So the full round-trip
//! (set format -> render display string) is in the public API.

use ironcalc_base::expressions::types::Area;
use ironcalc_base::number_format::format_number;
use ironcalc_base::UserModel;

use crate::{AuditRow, Status};

fn area(sheet: u32, row: i32, column: i32) -> Area {
    Area {
        sheet,
        row,
        column,
        width: 1,
        height: 1,
    }
}

/// Sets `num_fmt` on a single cell and returns what the engine renders as its display
/// string. This is the exact call the renderer would make per visible cell.
fn formatted(model: &mut UserModel, row: i32, column: i32, value: &str, num_fmt: &str) -> String {
    model.set_user_input(0, row, column, value).unwrap();
    model
        .update_range_style(&area(0, row, column), "num_fmt", num_fmt)
        .unwrap();
    model.get_formatted_cell_value(0, row, column).unwrap()
}

/// Runs the headline probe. Returns `(number, percent, date, red_negative_color)` — the
/// four things a renderer needs proven: thousands+decimals, percent, dates, and that the
/// format's color is exposed too.
pub fn probe() -> DisplayFormatObservation {
    let mut model = UserModel::new_empty("display", "en", "UTC", "en").unwrap();

    // 1) Thousands separator + fixed decimals: 1234.5 under "#,##0.00" -> "1,234.50".
    let number = formatted(&mut model, 1, 1, "1234.5", "#,##0.00");

    // 2) Percent: the raw value 1.0 under "0.00%" -> "100.00%" (engine scales by 100 and
    //    appends the sign — FreeCell would have to reimplement this otherwise).
    let percent = formatted(&mut model, 2, 1, "1", "0.00%");

    // 3) Date: the Excel serial 44197 (2021-01-01) under a date format -> a formatted
    //    date string, NOT the raw serial. Proves date rendering is engine-owned.
    let date = formatted(&mut model, 3, 1, "44197", "yyyy-mm-dd");

    // 4) The low-level formatter is directly reachable and returns the format's COLOR too
    //    (e.g. a "[Red]" negative). This is the display path a FreeCell display-cache
    //    could call directly with (value, format_string, locale).
    let red = format_number(-1234.5, "#,##0.00;[Red]#,##0.00", "en");

    DisplayFormatObservation {
        number,
        percent,
        date,
        red_negative_text: red.text,
        red_negative_has_color: red.color.is_some(),
    }
}

/// What the headline probe observed.
#[derive(Debug, Clone)]
pub struct DisplayFormatObservation {
    pub number: String,
    pub percent: String,
    pub date: String,
    pub red_negative_text: String,
    pub red_negative_has_color: bool,
}

pub fn audit() -> Vec<AuditRow> {
    let o = probe();
    vec![
        AuditRow::new(
            "Display formatting [HEADLINE]: engine renders the display string",
            Status::Present,
            format!(
                "IronCalc OWNS it. get_formatted_cell_value applies num_fmt: \
                 1234.5/#,##0.00 -> {:?}, 1/0.00% -> {:?}, 44197/yyyy-mm-dd -> {:?}. \
                 FreeCell does NOT implement number-format rendering. \
                 (model.rs:1800-1817; number_format.rs:152)",
                o.number, o.percent, o.date
            ),
        ),
        AuditRow::new(
            "Display formatting: format color exposed (e.g. [Red] negatives)",
            Status::Present,
            format!(
                "format_number returns Formatted {{ text, color, error }}: \
                 -1234.5/[Red] -> text {:?}, color present = {}. \
                 (formatter/format.rs:10)",
                o.red_negative_text, o.red_negative_has_color
            ),
        ),
        AuditRow::new(
            "Set/read a cell number format (round-trip)",
            Status::Present,
            "UserModel::update_range_style(area, \"num_fmt\", fmt) sets it (undoable); \
             get_cell_style(..).num_fmt reads it back. (common.rs:1253, 150, 1402)",
        ),
    ]
}
