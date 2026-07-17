//! Conditional-formatting methods on [`WorkbookDocument`] (`architecture.md §4.1`,
//! `components/engine_cf.md §4`).
//!
//! A child module of `document` so it can reach the private `model` field, keeping the CF surface
//! isolated from the large I/O file. Every method takes and returns only engine-free
//! `freecell-core` types — the IronCalc mapping is done by [`crate::cond_fmt_convert`], so no
//! `ironcalc` type leaks past this crate. Mutations delegate to the identically-purposed
//! `UserModel` CF API (each records its own undoable diff); reads convert through the same seam.
//!
//! P2 wired the mutators + list/gate reads to the worker `Command` dispatch + the published CF map;
//! P3 wires [`WorkbookDocument::extended_render_style`] into the value-dependent render cache
//! (`cache::build_sheet_cache` / `refresh_cell` on a CF sheet + the worker's value-change rebuild).

use freecell_core::{CellRef, CfRuleSpec, CfRuleView, RenderStyle};
use ironcalc_base::types::Theme;

use super::{to_engine_coords, WorkbookDocument};
use crate::cache::render_style_from;
use crate::cond_fmt_convert::{
    cf_format_to_dxf, cf_rule_spec_to_input, cf_rule_to_view, merge_cf_format_into_dxf,
};

impl WorkbookDocument {
    /// Adds a CF rule over `range` (an A1 range / multi-area). The rule's [`CfFormat`] is converted
    /// to a fresh `Dxf` (a color scale has none → a default, ignored `Dxf`), then the whole spec is
    /// mapped to a `CfRuleInput`. Returns the engine's `Err` (bad range/formula/operand) verbatim so
    /// the sidebar can show it; nothing is partially applied.
    pub(crate) fn add_cond_fmt(
        &mut self,
        sheet_idx: u32,
        range: &str,
        spec: &CfRuleSpec,
    ) -> Result<(), String> {
        let dxf = spec.format().map(cf_format_to_dxf).unwrap_or_default();
        let input = cf_rule_spec_to_input(spec, dxf);
        self.user_model_mut()
            .add_conditional_formatting(sheet_idx, range, input)
    }

    /// Replaces the rule at storage `index` with `spec` over `new_range`. For a highlight rule the
    /// new [`CfFormat`] is **merged** onto the rule's existing `Dxf` (fetched first) so unmodeled
    /// differential attributes (underline/strike/border/num-fmt/alignment) survive the edit
    /// (`functional_spec.md §4`); a color scale carries no format, so it skips the merge.
    pub(crate) fn update_cond_fmt(
        &mut self,
        sheet_idx: u32,
        index: u32,
        new_range: &str,
        spec: &CfRuleSpec,
    ) -> Result<(), String> {
        let dxf = match spec.format() {
            Some(fmt) => {
                let existing = self
                    .user_model()
                    .get_dxf_for_conditional_formatting(sheet_idx, index)?
                    .unwrap_or_default();
                merge_cf_format_into_dxf(fmt, existing)
            }
            None => ironcalc_base::types::Dxf::default(),
        };
        let input = cf_rule_spec_to_input(spec, dxf);
        self.user_model_mut()
            .update_conditional_formatting(sheet_idx, index, new_range, input)
    }

    /// Deletes the CF rule at storage `index`.
    pub(crate) fn delete_cond_fmt(&mut self, sheet_idx: u32, index: u32) -> Result<(), String> {
        self.user_model_mut()
            .delete_conditional_formatting(sheet_idx, index)
    }

    /// Raises the priority of the rule at storage `index` (swaps with the next-higher rule; no-op
    /// at the top).
    pub(crate) fn raise_cond_fmt(&mut self, sheet_idx: u32, index: u32) -> Result<(), String> {
        self.user_model_mut()
            .raise_conditional_formatting_priority(sheet_idx, index)
    }

    /// Lowers the priority of the rule at storage `index` (swaps with the next-lower rule; no-op at
    /// the bottom).
    pub(crate) fn lower_cond_fmt(&mut self, sheet_idx: u32, index: u32) -> Result<(), String> {
        self.user_model_mut()
            .lower_conditional_formatting_priority(sheet_idx, index)
    }

    /// The sheet's CF rules as engine-free read models, ordered by priority descending (the
    /// engine's list order). Each row carries its stable storage `index` — the handle the reorder /
    /// edit / delete mutators take — its human summary, preview, and (for authorable rules) a
    /// reconstructed `spec`. A dxf-carrying rule's format is fetched via
    /// `get_dxf_for_conditional_formatting`; color scales / deferred families have no dxf.
    pub(crate) fn cond_fmt_rules(&self, sheet_idx: u32) -> Result<Vec<CfRuleView>, String> {
        let list = self
            .user_model()
            .get_conditional_formatting_list(sheet_idx)?;
        let mut views = Vec::with_capacity(list.len());
        for entry in list {
            let index = entry.index as u32;
            let dxf = self
                .user_model()
                .get_dxf_for_conditional_formatting(sheet_idx, index)?;
            views.push(cf_rule_to_view(
                index,
                entry.range,
                entry.priority,
                &entry.cf_rule,
                dxf,
            ));
        }
        Ok(views)
    }

    /// Whether the sheet carries any CF rule — the cheap gate that keeps every added CF cost
    /// (extended-style reads, value-publish→style-refresh) off non-CF workbooks (`architecture.md
    /// §4.4`). A read failure degrades to `false` (no CF), never a panic.
    pub(crate) fn has_cond_fmt(&self, sheet_idx: u32) -> bool {
        self.worksheet(sheet_idx)
            .map(|ws| !ws.conditional_formatting.is_empty())
            .unwrap_or(false)
    }

    /// The cell's fully-resolved **effective** style — base style with any winning CF overlay
    /// (highlight dxf or color-scale fill) folded in — as an engine-free [`RenderStyle`]
    /// (`architecture.md §4.1`, `§4.4`). This is the value-dependent read the CF-sheet cache path
    /// uses instead of the static base style; the engine returns the base style unchanged when no
    /// rule matches, so it is always correct. The `.icon`/`.data_bar`/`.rating` decorations are
    /// dropped this pass (their families are deferred). A read failure degrades to the default
    /// style (logged), never a panic.
    ///
    /// Consumed by the value-dependent render cache (`cache::build_sheet_cache` / `refresh_cell` on a
    /// CF sheet) and the worker's value-change invalidation.
    pub(crate) fn extended_render_style(
        &self,
        sheet_idx: u32,
        cell: CellRef,
        theme: &Theme,
    ) -> RenderStyle {
        let (row, col) = to_engine_coords(cell);
        match self
            .user_model()
            .get_extended_cell_style(sheet_idx, row, col)
        {
            Ok(extended) => render_style_from(&extended.style, theme),
            Err(err) => {
                tracing::warn!(
                    sheet = sheet_idx,
                    row = cell.row,
                    col = cell.col,
                    error = %err,
                    "extended cell style read failed; falling back to the default style",
                );
                RenderStyle::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use freecell_core::cond_fmt::{
        CfColorStop, CfFormat, CfPeriod, CfPreview, CfRuleSpec, CfRuleView, CfTextOp,
        CfThresholdKind, CfValueOp,
    };
    use freecell_core::{CellRef, RenderStyle, Rgb};

    use crate::document::WorkbookDocument;

    const RED: Rgb = Rgb::new(255, 0, 0);
    const BLUE: Rgb = Rgb::new(0, 0, 255);
    const GREEN: Rgb = Rgb::new(0, 255, 0);

    fn fill_format(color: Rgb) -> CfFormat {
        CfFormat {
            fill: Some(color),
            ..Default::default()
        }
    }

    fn cell_is(op: CfValueOp, operand: &str, format: CfFormat) -> CfRuleSpec {
        CfRuleSpec::CellIs {
            op,
            operand: operand.to_string(),
            operand2: None,
            format,
            stop_if_true: false,
        }
    }

    /// Reads the effective fill for a cell (fresh theme borrow so the read never conflicts with a
    /// later `&mut` edit).
    fn fill_at(doc: &WorkbookDocument, row: u32, col: u32) -> Option<Rgb> {
        render_at(doc, row, col).fill
    }

    /// Reads a cell's full effective [`RenderStyle`] (base + winning CF overlay).
    fn render_at(doc: &WorkbookDocument, row: u32, col: u32) -> RenderStyle {
        doc.extended_render_style(0, CellRef::new(row, col), doc.workbook_theme())
    }

    /// Finds the single rule covering `range` in a rules list (ranges are unique per test).
    fn rule_by_range<'a>(rules: &'a [CfRuleView], range: &str) -> &'a CfRuleView {
        rules
            .iter()
            .find(|r| r.range == range)
            .unwrap_or_else(|| panic!("no rule over {range} in {rules:?}"))
    }

    #[test]
    fn add_then_list_reflects_rule() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let spec = cell_is(CfValueOp::Gt, "100", fill_format(RED));
        doc.add_cond_fmt(0, "B2:B20", &spec).unwrap();

        let rules = doc.cond_fmt_rules(0).unwrap();
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.index, 0);
        assert_eq!(rule.range, "B2:B20");
        assert_eq!(rule.priority, 1);
        assert!(rule.editable);
        assert_eq!(rule.summary, "Cell value > 100");
        match &rule.preview {
            CfPreview::Highlight { fill, .. } => assert_eq!(*fill, Some(RED)),
            other => panic!("expected a Highlight preview, got {other:?}"),
        }
        assert_eq!(
            rule.spec.as_ref(),
            Some(&spec),
            "spec round-trips through the engine"
        );
    }

    #[test]
    fn authorable_variants_round_trip_through_engine() {
        // Each spec added to a fresh workbook (index 0, priority 1) and read back for equality.
        // These variants carry no engine-translated formula string, so full equality is exact.
        let specs = vec![
            cell_is(CfValueOp::Ge, "50", fill_format(RED)),
            CfRuleSpec::Text {
                op: CfTextOp::Contains,
                value: "foo".to_string(),
                format: fill_format(BLUE),
                stop_if_true: true,
            },
            CfRuleSpec::TimePeriod {
                period: CfPeriod::Last7Days,
                format: fill_format(GREEN),
                stop_if_true: false,
            },
            CfRuleSpec::Top {
                rank: 10,
                percent: true,
                bottom: true,
                format: fill_format(RED),
                stop_if_true: false,
            },
            CfRuleSpec::Average {
                below: true,
                format: fill_format(RED),
                stop_if_true: false,
            },
            CfRuleSpec::DuplicateValues {
                unique: true,
                format: fill_format(RED),
                stop_if_true: false,
            },
            CfRuleSpec::Blanks {
                no_blanks: true,
                format: fill_format(RED),
                stop_if_true: false,
            },
            CfRuleSpec::Errors {
                no_errors: true,
                format: fill_format(RED),
                stop_if_true: false,
            },
            CfRuleSpec::ColorScale {
                stops: vec![
                    CfColorStop {
                        kind: CfThresholdKind::Min,
                        value: None,
                        color: GREEN,
                    },
                    CfColorStop {
                        kind: CfThresholdKind::Max,
                        value: None,
                        color: RED,
                    },
                ],
            },
        ];

        for spec in specs {
            let mut doc = WorkbookDocument::new_empty().unwrap();
            doc.add_cond_fmt(0, "A1:A20", &spec).unwrap();
            let rules = doc.cond_fmt_rules(0).unwrap();
            assert_eq!(
                rules[0].spec.as_ref(),
                Some(&spec),
                "round-trip failed for {spec:?}"
            );
        }
    }

    #[test]
    fn formula_rule_round_trips_variant_and_format() {
        // The Formula string is translated to/from the engine's internal form; assert the variant,
        // format, and stop_if_true survive (the exact echo is the engine's concern, not ours).
        let mut doc = WorkbookDocument::new_empty().unwrap();
        let spec = CfRuleSpec::Formula {
            formula: "A1>5".to_string(),
            format: fill_format(BLUE),
            stop_if_true: true,
        };
        doc.add_cond_fmt(0, "A1:A20", &spec).unwrap();
        let rules = doc.cond_fmt_rules(0).unwrap();
        match rules[0].spec.as_ref() {
            Some(CfRuleSpec::Formula {
                formula,
                format,
                stop_if_true,
            }) => {
                assert!(!formula.is_empty());
                assert_eq!(*format, fill_format(BLUE));
                assert!(*stop_if_true);
            }
            other => panic!("expected a Formula spec, got {other:?}"),
        }
    }

    #[test]
    fn update_changes_format_and_range() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.add_cond_fmt(
            0,
            "A1:A10",
            &cell_is(CfValueOp::Gt, "100", fill_format(RED)),
        )
        .unwrap();

        let updated = CfRuleSpec::CellIs {
            op: CfValueOp::Ge,
            operand: "50".to_string(),
            operand2: None,
            format: CfFormat {
                fill: Some(BLUE),
                bold: true,
                ..Default::default()
            },
            stop_if_true: false,
        };
        doc.update_cond_fmt(0, 0, "B1:B10", &updated).unwrap();

        let rules = doc.cond_fmt_rules(0).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].range, "B1:B10");
        assert_eq!(rules[0].summary, "Cell value ≥ 50");
        assert_eq!(rules[0].spec.as_ref(), Some(&updated));
    }

    #[test]
    fn delete_removes_rule() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.add_cond_fmt(0, "A1:A10", &cell_is(CfValueOp::Gt, "1", fill_format(RED)))
            .unwrap();
        assert_eq!(doc.cond_fmt_rules(0).unwrap().len(), 1);
        doc.delete_cond_fmt(0, 0).unwrap();
        assert!(doc.cond_fmt_rules(0).unwrap().is_empty());
    }

    #[test]
    fn raise_lower_reorders_priority() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Rule A (storage index 0, priority 1) then rule B (index 1, priority 2).
        doc.add_cond_fmt(0, "A1:A1", &cell_is(CfValueOp::Gt, "1", fill_format(RED)))
            .unwrap();
        doc.add_cond_fmt(0, "B1:B1", &cell_is(CfValueOp::Gt, "1", fill_format(BLUE)))
            .unwrap();

        // Priority-desc: B first.
        assert_eq!(doc.cond_fmt_rules(0).unwrap()[0].range, "B1:B1");

        // Raise A (storage index 0) above B.
        doc.raise_cond_fmt(0, 0).unwrap();
        assert_eq!(doc.cond_fmt_rules(0).unwrap()[0].range, "A1:A1");

        // Lower A back down.
        doc.lower_cond_fmt(0, 0).unwrap();
        assert_eq!(doc.cond_fmt_rules(0).unwrap()[0].range, "B1:B1");
    }

    #[test]
    fn has_cond_fmt_gate() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        assert!(!doc.has_cond_fmt(0), "empty sheet has no CF");
        doc.add_cond_fmt(0, "A1:A10", &cell_is(CfValueOp::Gt, "1", fill_format(RED)))
            .unwrap();
        assert!(doc.has_cond_fmt(0), "sheet has CF after add");
    }

    #[test]
    fn extended_style_reflects_rule() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "150").unwrap(); // A1 > 100
        doc.set_cell_input(0, CellRef::new(4, 0), "50").unwrap(); // A5 <= 100
        doc.add_cond_fmt(
            0,
            "A1:A10",
            &cell_is(CfValueOp::Gt, "100", fill_format(RED)),
        )
        .unwrap();

        assert_eq!(
            fill_at(&doc, 0, 0),
            Some(RED),
            "matching cell gets the fill"
        );
        assert_eq!(
            fill_at(&doc, 4, 0),
            None,
            "non-matching cell keeps the base style"
        );
    }

    #[test]
    fn extended_style_flips_on_value_change() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "150").unwrap();
        doc.add_cond_fmt(
            0,
            "A1:A10",
            &cell_is(CfValueOp::Gt, "100", fill_format(RED)),
        )
        .unwrap();
        assert_eq!(fill_at(&doc, 0, 0), Some(RED));

        // Editing the source value re-evaluates CF — no CF command issued.
        doc.set_cell_input(0, CellRef::new(0, 0), "50").unwrap();
        assert_eq!(
            fill_at(&doc, 0, 0),
            None,
            "value dropped below threshold → fill gone"
        );

        doc.set_cell_input(0, CellRef::new(0, 0), "150").unwrap();
        assert_eq!(
            fill_at(&doc, 0, 0),
            Some(RED),
            "value back above threshold → fill back"
        );
    }

    #[test]
    fn color_scale_interpolates() {
        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "0").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "50").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 0), "100").unwrap();
        doc.add_cond_fmt(
            0,
            "A1:A3",
            &CfRuleSpec::ColorScale {
                stops: vec![
                    CfColorStop {
                        kind: CfThresholdKind::Min,
                        value: None,
                        color: GREEN,
                    },
                    CfColorStop {
                        kind: CfThresholdKind::Max,
                        value: None,
                        color: RED,
                    },
                ],
            },
        )
        .unwrap();

        let mid = fill_at(&doc, 1, 0);
        assert!(mid.is_some(), "the middle cell gets an interpolated fill");
        assert_ne!(mid, Some(GREEN), "midpoint is not the min endpoint");
        assert_ne!(mid, Some(RED), "midpoint is not the max endpoint");
        // The endpoints still receive a fill from the scale.
        assert!(fill_at(&doc, 0, 0).is_some());
        assert!(fill_at(&doc, 2, 0).is_some());
    }

    /// CF is part of the IronCalc worksheet model, so it must save/load through the **native**
    /// `.xlsx` writer/reader with **no** special FreeCell save handling (unlike charts). This
    /// drives the real FreeCell save path (`WorkbookDocument::save` — the exact call the worker's
    /// chart-less `save_workbook` branch makes, funnelling through `save_xlsx_to_writer`) then
    /// reopens into a fresh document via `open`, and asserts both the **rules** (range / kind /
    /// priority / the highlight rule's format) and the **effective render style** survive the
    /// round-trip (`components/engine_cf.md §7`, `functional_spec.md §7`).
    #[test]
    fn cond_fmt_round_trips_through_xlsx_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cf.xlsx");

        let mut doc = WorkbookDocument::new_empty().unwrap();
        // Highlight domain over A1:A10 — one cell above the threshold, one below.
        doc.set_cell_input(0, CellRef::new(0, 0), "150").unwrap(); // A1 > 100
        doc.set_cell_input(0, CellRef::new(4, 0), "50").unwrap(); // A5 <= 100
                                                                  // Color-scale domain over C1:C3.
        doc.set_cell_input(0, CellRef::new(0, 2), "0").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 2), "50").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 2), "100").unwrap();

        // A highlight rule carrying a full format (fill + text colour + bold) — every modeled
        // format attribute, so a lossy writer/reader would show up as a changed spec.
        let highlight = CfRuleSpec::CellIs {
            op: CfValueOp::Gt,
            operand: "100".to_string(),
            operand2: None,
            format: CfFormat {
                fill: Some(RED),
                text_color: Some(BLUE),
                bold: true,
                italic: false,
            },
            stop_if_true: false,
        };
        doc.add_cond_fmt(0, "A1:A10", &highlight).unwrap();

        let scale = CfRuleSpec::ColorScale {
            stops: vec![
                CfColorStop {
                    kind: CfThresholdKind::Min,
                    value: None,
                    color: GREEN,
                },
                CfColorStop {
                    kind: CfThresholdKind::Max,
                    value: None,
                    color: RED,
                },
            ],
        };
        doc.add_cond_fmt(0, "C1:C3", &scale).unwrap();

        // Sanity before saving: both rules present and the highlight already paints.
        let before = doc.cond_fmt_rules(0).unwrap();
        assert_eq!(before.len(), 2);
        assert_eq!(fill_at(&doc, 0, 0), Some(RED));
        // range → priority, to prove priority is preserved across the round-trip.
        let priorities_before: std::collections::BTreeMap<String, u32> = before
            .iter()
            .map(|r| (r.range.clone(), r.priority))
            .collect();

        // Save via the real FreeCell save API, then reopen into a *fresh* document.
        doc.save(&path).expect("save should succeed");
        let reopened = WorkbookDocument::open(&path).expect("reopen should succeed");

        // The rules survive: same count, same range → priority mapping.
        let after = reopened.cond_fmt_rules(0).unwrap();
        assert_eq!(after.len(), 2, "both rules survive the round-trip");
        let priorities_after: std::collections::BTreeMap<String, u32> = after
            .iter()
            .map(|r| (r.range.clone(), r.priority))
            .collect();
        assert_eq!(
            priorities_after, priorities_before,
            "each rule keeps its range and priority"
        );

        // The highlight rule round-trips to the same editable spec — this equality proves the
        // operator AND the full format (fill / text colour / bold) survived the writer/reader.
        let highlight_after = rule_by_range(&after, "A1:A10");
        assert!(highlight_after.editable);
        assert_eq!(highlight_after.spec.as_ref(), Some(&highlight));

        // The color scale round-trips to the same editable spec (concrete-RGB stops).
        let scale_after = rule_by_range(&after, "C1:C3");
        assert!(scale_after.editable);
        assert_eq!(scale_after.spec.as_ref(), Some(&scale));

        // The effective render style still reflects the rules on the reopened document (the CF
        // cache is repopulated by the loader — no explicit evaluate needed).
        let a1 = render_at(&reopened, 0, 0);
        assert_eq!(a1.fill, Some(RED), ">100 cell keeps its fill after reopen");
        assert_eq!(a1.font_color, Some(BLUE), "text colour survives");
        assert!(a1.bold, "bold survives");
        assert_eq!(
            fill_at(&reopened, 4, 0),
            None,
            "the below-threshold cell has no fill"
        );
        assert!(
            fill_at(&reopened, 1, 2).is_some(),
            "the color-scale midpoint still gets an interpolated fill after reopen"
        );
    }

    /// A **loaded** deferred-family rule (data bar / icon set / rating) must degrade gracefully:
    /// it lists as a non-editable `Badge` (delete-only, `spec: None`) and its cell render is not
    /// corrupted — `extended_render_style` drops the in-cell decoration and returns the base style
    /// (`functional_spec.md §9`, `components/engine_cf.md §6`). There is no `CfRuleSpec` variant
    /// for these families, so the rule is constructed straight through the engine's `UserModel`
    /// (the same shape a rule loaded from an xlsx authored elsewhere takes).
    #[test]
    fn loaded_deferred_family_rule_is_badge_and_renders_base_style() {
        use ironcalc_base::cf_types::CfRuleInput;
        use ironcalc_base::types::Color;

        let mut doc = WorkbookDocument::new_empty().unwrap();
        doc.set_cell_input(0, CellRef::new(0, 0), "10").unwrap();
        doc.set_cell_input(0, CellRef::new(1, 0), "50").unwrap();
        doc.set_cell_input(0, CellRef::new(2, 0), "100").unwrap();

        // A data bar — a deferred family with no first-pass `CfRuleSpec`. Automatic (None) min/max
        // so it always evaluates; it produces an `ExtendedStyle.data_bar` the render read drops.
        doc.user_model_mut()
            .add_conditional_formatting(
                0,
                "A1:A3",
                CfRuleInput::DataBar {
                    min: None,
                    max: None,
                    positive_color: Color::Rgb("#638EC6".to_string()),
                    negative_color: Color::Rgb("#FF0000".to_string()),
                    is_gradient: true,
                    show_value: true,
                },
            )
            .unwrap();

        // It lists as a non-editable, spec-less Badge (delete-only), and the gate still sees CF.
        assert!(doc.has_cond_fmt(0));
        let rules = doc.cond_fmt_rules(0).unwrap();
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.range, "A1:A3");
        assert!(
            !rule.editable,
            "a deferred family is not authorable this pass"
        );
        assert!(rule.spec.is_none(), "no spec for a deferred family");
        assert!(
            matches!(rule.preview, CfPreview::Badge(_)),
            "surfaced as a read-only Badge, got {:?}",
            rule.preview
        );

        // The cell render is not corrupted: the data-bar decoration is dropped and the read
        // returns the cell's base style (a plain cell → the default style), never garbage/panic.
        assert_eq!(
            render_at(&doc, 1, 0),
            RenderStyle::default(),
            "a data-bar cell renders its base style (decoration dropped)"
        );
    }
}
