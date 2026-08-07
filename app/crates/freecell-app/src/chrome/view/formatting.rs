//! The action row's formatting controls (`components/action_bar.md`, `ui_design.md §2`): the
//! bold/italic/underline/strikethrough/wrap and merge toggles, fill and text colour, alignment,
//! number format (including the basics-first drill-in), font family and size, and the borders
//! pen popover — with the popovers they render and the toggle-state accessors they read.
//!
//! One resident is not formatting: `commit_pending_edit` is an editing method that the
//! formatting and chart controls call before they act — `charts.rs` reaches it through
//! `pub(super)` — and it lives here only because it sat under the formatting banner (that
//! project's `findings.md` §2).
//! The five private free functions above the `impl` (border-icon drawing, `Hsla` conversion,
//! font-size labelling) *are* formatting; they are just free rather than methods.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

use gpui::Hsla;
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent};

use freecell_core::format_ui::{
    adjust_decimals_cell, displayed_decimals, font_size_display, is_more_only_num_fmt,
    num_fmt_category, toggle_thousands, Category, BASIC_FORMATS, NUM_FMT_GROUPS,
};
use freecell_core::{effective_range, region_at, regions_intersecting};
use freecell_engine::StylePath;

/// The fixed font-size dropdown list in points (`functional_spec.md §3.2`).
const FONT_SIZES: [f64; 12] = [8., 9., 10., 11., 12., 14., 16., 18., 20., 24., 28., 36.];
/// The top "clear the family override" entry in the font-family dropdown (`ui_design.md §2`).
pub(super) const SYSTEM_DEFAULT_FAMILY: &str = "Default (Inter)";

/// Converts a gpui `Hsla` to a 24-bit [`Rgb`] (the color picker's "Custom…" pick).
fn hsla_to_rgb(hsla: Hsla) -> Rgb {
    let rgba: Rgba = hsla.into();
    Rgb::new(
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
    )
}

/// Which of the target icon's six segments `(top, bottom, left, right, inner_h, inner_v)` a
/// `preset` paints **dark** (affected), the rest staying light-grey context (`ui_design.md §2.2`).
/// The mask mirrors IronCalc's per-`BorderType` edges: All = all six, Inner = the inner cross,
/// Outer = the perimeter, None = nothing, and each of Top/Bottom/Left/Right = its one outer edge.
/// Split out from [`border_target_icon`] so this affordance-defining table is unit-testable (the
/// render harness doesn't cover the chrome popover).
fn border_target_icon_mask(preset: BorderPreset) -> (bool, bool, bool, bool, bool, bool) {
    match preset {
        BorderPreset::All => (true, true, true, true, true, true),
        BorderPreset::Inner => (false, false, false, false, true, true),
        BorderPreset::Outer => (true, true, true, true, false, false),
        BorderPreset::None => (false, false, false, false, false, false),
        BorderPreset::Top => (true, false, false, false, false, false),
        BorderPreset::Bottom => (false, true, false, false, false, false),
        BorderPreset::Left => (false, false, true, false, false, false),
        BorderPreset::Right => (false, false, false, true, false, false),
    }
}

/// A borders **target icon** (`ui_design.md §2.2`): a ~22px 2×2 mini-grid drawn from `div`
/// rectangles. Every gridline is context light-grey (1px); the segments the `preset` affects are
/// solid dark (2px, heavier). The six segments are the four outer edges + the inner cross (mid-H,
/// mid-V); the per-preset dark mask ([`border_target_icon_mask`]) mirrors IronCalc's per-`BorderType`
/// edges. Grey segments paint first so a dark segment always wins at a crossing.
fn border_target_icon(preset: BorderPreset) -> gpui::AnyElement {
    let (top, bottom, left, right, inner_h, inner_v) = border_target_icon_mask(preset);
    let near = 1.0;
    let far = TARGET_ICON_PX - 1.0;
    let mid = TARGET_ICON_PX / 2.0;
    // A horizontal / vertical segment centered on `nominal`, spanning the inset box `[near, far]`
    // extended by its own thickness `t` at each end so it reaches the OUTER edge of the
    // perpendicular lines: corners meet flush (dark t=2 → full extent) with no gap or overhang.
    let hline = |nominal: f32, dark: bool| {
        let t = if dark { 2.0 } else { 1.0 };
        div()
            .absolute()
            .left(px(near - t / 2.0))
            .top(px(nominal - t / 2.0))
            .w(px(far - near + t))
            .h(px(t))
            .bg(rgb(if dark {
                TARGET_ICON_DARK
            } else {
                TARGET_ICON_GREY
            }))
    };
    let vline = |nominal: f32, dark: bool| {
        let t = if dark { 2.0 } else { 1.0 };
        div()
            .absolute()
            .top(px(near - t / 2.0))
            .left(px(nominal - t / 2.0))
            .h(px(far - near + t))
            .w(px(t))
            .bg(rgb(if dark {
                TARGET_ICON_DARK
            } else {
                TARGET_ICON_GREY
            }))
    };
    // Each segment as (is_horizontal, nominal, dark).
    let segments = [
        (true, near, top),
        (true, far, bottom),
        (true, mid, inner_h),
        (false, near, left),
        (false, far, right),
        (false, mid, inner_v),
    ];
    let mut icon = div()
        .relative()
        .flex_none()
        .w(px(TARGET_ICON_PX))
        .h(px(TARGET_ICON_PX));
    // Grey first, then dark on top (so a dark segment wins where it crosses a grey one).
    for &(is_h, nominal, _) in segments.iter().filter(|s| !s.2) {
        icon = icon.child(if is_h {
            hline(nominal, false)
        } else {
            vline(nominal, false)
        });
    }
    for &(is_h, nominal, _) in segments.iter().filter(|s| s.2) {
        icon = icon.child(if is_h {
            hline(nominal, true)
        } else {
            vline(nominal, true)
        });
    }
    icon.into_any_element()
}

/// A borders **line-style preview** (`ui_design.md §2.3`): a short horizontal sample of the real
/// line, vertically centered in a ~34px box. Solid weights are one dark bar (1/2/3px); dashed is a
/// row of short dark dashes; double is two 1px dark bars with a gap.
fn border_line_preview(line: BorderLine) -> gpui::AnyElement {
    const SAMPLE_W: f32 = 34.0;
    let box_ = || {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .w(px(SAMPLE_W))
            .h(px(12.0))
    };
    let bar = |weight: f32| {
        div()
            .w(px(SAMPLE_W))
            .h(px(weight))
            .bg(rgb(TARGET_ICON_DARK))
    };
    match line {
        BorderLine::ThinSolid => box_().child(bar(1.0)).into_any_element(),
        BorderLine::MediumSolid => box_().child(bar(2.0)).into_any_element(),
        BorderLine::ThickSolid => box_().child(bar(3.0)).into_any_element(),
        BorderLine::Dashed => {
            // A run of short dark dashes with gaps (5 dashes across the sample).
            let mut dashes = div().flex().items_center().gap(px(2.0)).h(px(2.0));
            for _ in 0..5 {
                dashes = dashes.child(div().w(px(4.0)).h(px(2.0)).bg(rgb(TARGET_ICON_DARK)));
            }
            box_().child(dashes).into_any_element()
        }
        BorderLine::Double => box_()
            .gap(px(1.0))
            .child(bar(1.0))
            .child(bar(1.0))
            .into_any_element(),
    }
}

/// Formats a font size in points for the size box, trimming a trailing `.0` (`13.0` → `"13"`,
/// `10.5` → `"10.5"`) — the same look as [`font_size_display`] for explicit sizes.
fn format_size_pt(pt: f64) -> String {
    format!("{pt}")
}

impl ChromeView {
    // ---- Action row: formatting -----------------------------------------------------------

    /// Toggles a character style over the selection; commits any pending edit first (the same
    /// rule as clicking another cell). A cap-rejected pending edit blocks the toggle.
    pub fn toggle_style(&mut self, attr: StyleAttr, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the format, keeping the field editing
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr,
        });
    }

    /// Merges or unmerges the selection — the action-row toggle + Edit-menu ⌃⌘M action
    /// (merged-cell-ui `architecture.md §8`, `functional_spec.md F2`). Commits any pending edit
    /// first (the same click-away rule as the other action-row controls). The decision mirrors
    /// Excel's Merge & Center toggle:
    /// - the selection's effective range contains any merged region → **unmerge** every
    ///   intersecting region (one engine call each);
    /// - otherwise a multi-cell range → **merge** it — sent unconfirmed, so the worker answers with
    ///   [`WorkerEvent::MergeNeedsConfirm`] when the merge would discard data (the window's confirm
    ///   dialog then re-sends `confirmed: true`);
    /// - a lone 1×1 not in any merge → no-op (the button is disabled for this case anyway).
    pub fn toggle_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // No mutating control may dispatch while degraded/read-only (`functional_spec.md §6`) — a
        // backstop to the disabled button, covering the menu/⌃⌘M path.
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the toggle, keeping the field editing
        }
        let merges = self.active_sheet_merges();
        let range = effective_range(&merges, self.selection);
        let hit = regions_intersecting(&merges, range);
        if !hit.is_empty() {
            // Unmerge every region the selection touches (each removed region is one engine call).
            for region in hit {
                self.client.send(Command::UnmergeCells {
                    sheet: self.active_sheet,
                    anchor: region.start,
                });
            }
        } else if range.start != range.end {
            // A merge-free multi-cell rectangle → merge it (unconfirmed; the worker gates data loss).
            self.client.send(Command::MergeCells {
                sheet: self.active_sheet,
                area: range,
                confirmed: false,
            });
        }
    }

    /// Applies a fill colour (`Some`) or clears it (`None`) over the selection; commits any
    /// pending edit first, and closes the fill popover.
    pub fn apply_fill(&mut self, fill: Option<Rgb>, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_open = false;
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr: StyleAttr::Fill(fill),
        });
        cx.notify();
    }

    /// Commits a pending data-row edit if any. Returns whether the field is now committable
    /// (`false` = a cap-rejected edit is still open).
    pub(super) fn commit_pending_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.data_row.mode() == FieldMode::Editing {
            let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
            self.apply_data_effects(effects, window, cx);
            self.note_commit(true);
            if self.data_row.mode() != FieldMode::Editing {
                self.edit.close();
            }
            self.refresh_edit_grid_state(window, cx);
        }
        self.data_row.mode() != FieldMode::Editing
    }

    pub(super) fn toggle_fill_popover(&mut self, cx: &mut Context<Self>) {
        self.fill_open = !self.fill_open;
        cx.notify();
    }

    pub(super) fn on_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_fill(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Sends one `SetStylePath` over the selection after committing any pending edit (the same
    /// rule as clicking another cell). Fire-and-forget: a cap-rejected pending edit blocks it, and
    /// the worker logs any engine rejection (the UI only ever sends valid paths/values).
    fn apply_style_path(
        &mut self,
        path: StylePath,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // No mutating control may dispatch while degraded/read-only (`functional_spec.md §6`) — a
        // backstop to the disabled buttons, covering a swatch/entry clicked in a popover that was
        // open at the instant of degradation (also closed by `set_degraded`).
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStylePath {
            sheet: self.active_sheet,
            range: self.selection.range(),
            path,
            value,
        });
        cx.notify();
    }

    /// Applies a text colour (`Some`) or clears it to Automatic (`None`, value `""`), closing the
    /// text-color popover.
    pub fn apply_text_color(
        &mut self,
        color: Option<Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text_color_open = false;
        let value = match color {
            Some(rgb) => format!("#{:06X}", rgb.to_hex()),
            None => String::new(),
        };
        self.apply_style_path(StylePath::FontColor, value, window, cx);
    }

    /// Applies a horizontal alignment; re-pressing the active one clears the explicit alignment
    /// back to the type default (value `"general"` — clears horizontal only, never wrap/vertical).
    pub fn apply_alignment(&mut self, align: Align, window: &mut Window, cx: &mut Context<Self>) {
        let value = if self.align_active(align) {
            "general".to_string()
        } else {
            match align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
            }
            .to_string()
        };
        self.apply_style_path(StylePath::AlignHorizontal, value, window, cx);
    }

    /// Applies a vertical alignment (top/center/bottom) over the selection — a plain radio-style
    /// set (`functional_spec.md §1.3`, `architecture.md §2`). Unlike horizontal align there is no
    /// re-press-to-clear: IronCalc's vertical default is `bottom` and the grid's default placement
    /// is also bottom (decision C — Excel-faithful), so there is no independent "unset" value to
    /// clear back to; the group is purely one-of-N (top / center / bottom).
    pub fn apply_valign(&mut self, valign: VAlign, window: &mut Window, cx: &mut Context<Self>) {
        let value = match valign {
            VAlign::Top => "top",
            VAlign::Center => "center",
            VAlign::Bottom => "bottom",
        }
        .to_string();
        self.apply_style_path(StylePath::AlignVertical, value, window, cx);
    }

    /// Applies a number-format code over the selection, closing the number-format dropdown.
    pub fn apply_num_fmt(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.num_fmt_open = false;
        self.num_fmt_more_open = false;
        self.apply_style_path(StylePath::NumFmt, code.to_string(), window, cx);
    }

    /// Adjusts the active cell's number of decimal places by `delta` (`+1` / `-1`). Computed
    /// UI-side from the cached format string and the active cell's kind/display: a real numeric
    /// format is rewritten directly, and a *numeric* General cell (`200000`) is switched to a
    /// `0.0…` format (BUG 3); a no-op (`adjust_decimals_cell` → `None`) does nothing (the buttons
    /// also render disabled in that case).
    pub fn bump_decimals(&mut self, delta: i8, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.active_num_fmt.clone();
        let (numeric, displayed) = self.active_numeric_decimals();
        if let Some(new_code) = current
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
        {
            self.apply_num_fmt(&new_code, window, cx);
        }
    }

    /// Whether the thousands-separator toggle is enabled: not degraded, and the active cell's
    /// format can be safely re-grouped (`toggle_thousands` — a single-section numeric code with an
    /// integer `0` placeholder). General/Text/Date/Time/Scientific/multi-section customs disable it.
    pub fn toggle_thousands_enabled(&self) -> bool {
        if self.degraded {
            return false;
        }
        self.active_num_fmt
            .as_deref()
            .and_then(toggle_thousands)
            .is_some()
    }

    /// Whether the active cell's format currently carries a thousands separator (the toggle renders
    /// pressed). Only true when the toggle is also actionable, so a disabled button never shows as
    /// selected.
    pub fn thousands_active(&self) -> bool {
        self.toggle_thousands_enabled()
            && self
                .active_num_fmt
                .as_deref()
                .is_some_and(|c| c.contains("#,##0"))
    }

    /// Toggles the thousands separator on the active cell's number format, applying the rewritten
    /// code over the selection (one undo step). A no-op when the format can't be toggled (the
    /// button also renders disabled in that case).
    pub fn toggle_thousands_separator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(new_code) = self.active_num_fmt.as_deref().and_then(toggle_thousands) {
            self.apply_num_fmt(&new_code, window, cx);
        }
    }

    pub(super) fn toggle_text_color_popover(&mut self, cx: &mut Context<Self>) {
        self.text_color_open = !self.text_color_open;
        cx.notify();
    }

    pub(super) fn toggle_num_fmt_popover(&mut self, cx: &mut Context<Self>) {
        self.num_fmt_open = !self.num_fmt_open;
        // Basics-first on open — except when the active cell's format lives only under "More ▸":
        // then land directly on the grouped view so the current format is visible/highlighted
        // (`architecture.md §10`, "open onto the matched group"). Always reset when closing.
        self.num_fmt_more_open =
            self.num_fmt_open && is_more_only_num_fmt(&self.num_fmt_active_code());
        cx.notify();
    }

    /// The active cell's number-format code, normalized so `general` (which the engine may echo as
    /// `"General"`) compares lowercase against the preset codes. `None` → the default `"general"`.
    fn num_fmt_active_code(&self) -> String {
        let c = self.active_num_fmt.as_deref().unwrap_or("general");
        if c.eq_ignore_ascii_case("general") {
            "general".to_string()
        } else {
            c.to_string()
        }
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    /// Sends one `SetFont` over the selection after committing any pending edit (fire-and-forget,
    /// degraded-guarded — the same rule as the `SetStylePath` controls).
    fn apply_set_font(
        &mut self,
        family: Option<String>,
        size_pt: Option<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetFont {
            sheet: self.active_sheet,
            range: self.selection.range(),
            family,
            size_pt,
        });
        cx.notify();
    }

    /// Applies a font family over the selection, closing the family dropdown. "Default (Inter)"
    /// clears the override (sent as `Some("")`); any other name sets it.
    pub fn apply_font_family(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.font_family_open = false;
        let family = if name == SYSTEM_DEFAULT_FAMILY {
            String::new()
        } else {
            name.to_string()
        };
        self.apply_set_font(Some(family), None, window, cx);
    }

    /// Applies a font size (points) over the selection, closing the size dropdown.
    pub fn apply_font_size(&mut self, pt: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.font_size_open = false;
        self.apply_set_font(None, Some(pt), window, cx);
    }

    pub(super) fn toggle_font_family_popover(&mut self, cx: &mut Context<Self>) {
        self.font_family_open = !self.font_family_open;
        cx.notify();
    }

    pub(super) fn toggle_font_size_popover(&mut self, cx: &mut Context<Self>) {
        self.font_size_open = !self.font_size_open;
        cx.notify();
    }

    // ---- Action row: SetBorders (pen popover) ---------------------------------------------

    /// Paints the current pen (`border_line` + `border_color`) onto `preset`'s edges over the
    /// selection. Degraded-guards + commits any pending edit first, the same rule as the other
    /// action-row controls (`components/action_bar.md`); returns whether it dispatched. Shared by
    /// [`select_border_target`](Self::select_border_target) and the pen-tweak repaints. For
    /// [`BorderPreset::None`] the engine clears the selection's borders (line/color unused).
    /// Fire-and-forget: the worker logs any engine rejection.
    fn send_border_paint(
        &mut self,
        preset: BorderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.degraded {
            return false;
        }
        if !self.commit_pending_edit(window, cx) {
            return false;
        }
        self.client.send(Command::SetBorders {
            sheet: self.active_sheet,
            range: self.selection.range(),
            preset,
            line: self.border_line,
            color: Some(self.border_color),
        });
        true
    }

    /// Selects a border **target** and paints the current pen onto just its edges — the pen model
    /// (`functional_spec.md §2.1`, `ui_design.md §2.4`). The popover **stays open** (unlike the old
    /// apply-and-close preset path): only click-away / Esc closes it. `None` clears all borders in
    /// the selection and leaves **no** target selected (there is nothing left to keep styling).
    pub fn select_border_target(
        &mut self,
        preset: BorderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.send_border_paint(preset, window, cx) {
            return;
        }
        // `None` is an action, not a paintable target — it deselects; every other preset becomes
        // the selected target so subsequent pen tweaks repaint it.
        self.border_target = (preset != BorderPreset::None).then_some(preset);
        cx.notify();
    }

    /// Sets the pen's **line style**. If a target is selected, repaints that target with the new
    /// pen; with no target, updates the pen only (MVP — no sheet change until a target is picked;
    /// P2 restyle-all is deferred, GAPS F2). The pen carries across target switches.
    pub fn set_border_line(
        &mut self,
        line: BorderLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.border_line = line;
        if let Some(preset) = self.border_target {
            self.send_border_paint(preset, window, cx);
        }
        cx.notify();
    }

    /// Sets the pen's **color** (symmetric to [`set_border_line`](Self::set_border_line)):
    /// repaints the selected target, or updates the pen only when no target is selected.
    pub fn set_border_color(&mut self, color: Rgb, window: &mut Window, cx: &mut Context<Self>) {
        self.border_color = color;
        if let Some(preset) = self.border_target {
            self.send_border_paint(preset, window, cx);
        }
        cx.notify();
    }

    /// Toggles the borders popover. **Opening resets the transient pen state** — no target
    /// selected, pen back to thin solid black — even if the selection already has borders (border
    /// state is never derived from existing cell borders; `functional_spec.md §2.1`).
    pub(super) fn toggle_borders_popover(&mut self, cx: &mut Context<Self>) {
        self.borders_open = !self.borders_open;
        if self.borders_open {
            self.border_target = None;
            self.border_line = BorderLine::ThinSolid;
            // The pen color is our source of truth; resetting it re-rings the black swatch. We
            // deliberately do NOT reach into the stock `border_color_picker`'s internal display
            // state, so its "Custom…" preview can still show the previous custom color until the
            // user picks again — cosmetic, and identical to the fill/text-color pickers by precedent.
            self.border_color = Rgb::new(0, 0, 0);
        }
        cx.notify();
    }

    /// Whether the borders popover is open (test/render introspection).
    pub fn borders_open(&self) -> bool {
        self.borders_open
    }

    /// The pen's selected target, if any (test introspection).
    #[cfg(test)]
    pub fn border_target(&self) -> Option<BorderPreset> {
        self.border_target
    }

    /// The pen's current line style (test introspection).
    #[cfg(test)]
    pub fn border_line(&self) -> BorderLine {
        self.border_line
    }

    /// The pen's current color (test introspection).
    #[cfg(test)]
    pub fn border_color(&self) -> Rgb {
        self.border_color
    }

    /// The font-family dropdown's active label: the active cell's family, or "Default (Inter)" for a
    /// default-font (or multi-cell) selection (`components/action_bar.md`).
    pub fn font_family_label(&self) -> &str {
        match self.active_font_family.as_deref() {
            Some(name) if !name.is_empty() => name,
            _ => SYSTEM_DEFAULT_FAMILY,
        }
    }

    /// The font-size dropdown's active label. An explicit size (`font_size_q != 0`) shows `q/4` pt;
    /// a **default** cell shows the workbook's real default size (13pt for a new workbook, the file's
    /// default otherwise) — never a hardcoded value that would mismatch the cell. Re-picking that
    /// shown default from the list is a visual no-op (the engine maps size == the workbook default
    /// back to the sentinel), so no surprising size jump. `13` is the fallback before a cache loads
    /// (IronCalc's default; `DECISIONS_TO_REVIEW` records the residual pt↔px seam).
    pub fn font_size_label(&self) -> String {
        let q = self.active_style.map(|s| s.font_size_q).unwrap_or(0);
        if q != 0 {
            font_size_display(q)
        } else {
            format_size_pt(self.default_font_size_pt.unwrap_or(13.0))
        }
    }

    pub(super) fn on_text_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_text_color(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    /// The borders "Custom…" picker changed → set the pen color (repaints the selected target, if
    /// any). Mirrors [`on_color_picker_event`](Self::on_color_picker_event).
    pub(super) fn on_border_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.set_border_color(hsla_to_rgb(*hsla), window, cx);
        }
    }

    /// Whether the bold toggle is pressed (active cell is bold).
    pub fn bold_active(&self) -> bool {
        self.active_style.map(|s| s.bold).unwrap_or(false)
    }

    /// Whether the italic toggle is pressed.
    pub fn italic_active(&self) -> bool {
        self.active_style.map(|s| s.italic).unwrap_or(false)
    }

    /// Whether the underline toggle is pressed.
    pub fn underline_active(&self) -> bool {
        self.active_style.map(|s| s.underline).unwrap_or(false)
    }

    /// Whether the strikethrough toggle is pressed.
    pub fn strikethrough_active(&self) -> bool {
        self.active_style.map(|s| s.strikethrough).unwrap_or(false)
    }

    /// Whether the wrap-text toggle is pressed.
    pub fn wrap_active(&self) -> bool {
        self.active_style.map(|s| s.wrap).unwrap_or(false)
    }

    /// The active sheet's merged regions (0-based), read live from the resident cache
    /// (merged-cell-ui `architecture.md §8`). Cheap (merge counts are tiny) and always current,
    /// so the Merge toggle reflects the live merge state without a cached-at-selection snapshot.
    fn active_sheet_merges(&self) -> Vec<CellRange> {
        self.client.sheet_merges(self.active_sheet)
    }

    /// Whether the Merge/Unmerge toggle reads as **pressed/active** — the selection's effective
    /// range contains a merged region, so a click unmerges (`ui_design.md §1`, mirroring
    /// [`bold_active`](Self::bold_active)). Also drives the tooltip swap.
    pub fn merge_active(&self) -> bool {
        let merges = self.active_sheet_merges();
        let range = effective_range(&merges, self.selection);
        !regions_intersecting(&merges, range).is_empty()
    }

    /// Whether the Merge/Unmerge toggle is **disabled**: degraded/read-only, or the selection is a
    /// lone 1×1 cell not in any merge (nothing to toggle) (`ui_design.md §1`, `architecture.md §8`).
    pub fn merge_disabled(&self) -> bool {
        if self.degraded {
            return true;
        }
        let merges = self.active_sheet_merges();
        self.selection.is_single() && region_at(&merges, self.selection.active).is_none()
    }

    /// Whether an alignment button is pressed — the **explicit** alignment only (a number aligned
    /// right by type default shows no pressed button, matching Excel; `components/action_bar.md`).
    pub fn align_active(&self, align: Align) -> bool {
        self.active_style.and_then(|s| s.h_align) == Some(align)
    }

    /// Whether a vertical-alignment button is pressed — the active cell's resolved vertical
    /// alignment (`functional_spec.md §1.3`). Under decision C the resolver reports a defaulted
    /// bottom as `Some(Bottom)`, so a cell whose vertical is merely defaulted (e.g. only horizontal
    /// set, or loaded from `.xlsx`) lights **Align bottom**; a truly-clean cell (no alignment
    /// record at all) lights nothing but still renders bottom. Accepted Excel-ish behavior.
    pub fn valign_active(&self, valign: VAlign) -> bool {
        self.active_style.and_then(|s| s.v_align) == Some(valign)
    }

    /// The active cell's number-format [`Category`] (General on a multi-cell selection / no cache).
    pub fn num_fmt_category(&self) -> Category {
        num_fmt_category(self.active_num_fmt.as_deref().unwrap_or("general"))
    }

    /// The number-format dropdown's button label (the active cell's category name).
    pub fn num_fmt_category_label(&self) -> &'static str {
        self.num_fmt_category().label()
    }

    /// Whether the "increase decimals" button is enabled (not degraded, single cell, and the
    /// active format has an adjustable decimal group).
    pub fn increase_decimals_enabled(&self) -> bool {
        self.decimals_enabled(1)
    }

    /// Whether the "decrease decimals" button is enabled.
    pub fn decrease_decimals_enabled(&self) -> bool {
        self.decimals_enabled(-1)
    }

    fn decimals_enabled(&self, delta: i8) -> bool {
        if self.degraded {
            return false;
        }
        let (numeric, displayed) = self.active_numeric_decimals();
        self.active_num_fmt
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
            .is_some()
    }

    /// Whether the active cell is a *number* (not text/date/bool/error/empty) and, if so, how many
    /// decimals its value currently displays — the inputs the decimals ± need to enable/adjust a
    /// General-formatted number (BUG 3). Both come from the cached publication of the active cell.
    fn active_numeric_decimals(&self) -> (bool, Option<u8>) {
        match &self.active_published {
            Some((CellKind::Number, display)) => (true, displayed_decimals(display)),
            _ => (false, None),
        }
    }

    /// Whether the text-color popover is open.
    pub fn text_color_open(&self) -> bool {
        self.text_color_open
    }

    /// Whether the number-format dropdown is open.
    pub fn num_fmt_open(&self) -> bool {
        self.num_fmt_open
    }

    /// Whether the fill popover is open.
    pub fn fill_open(&self) -> bool {
        self.fill_open
    }

    pub(super) fn render_fill_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // 5×2 swatch grid.
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("swatch-{}", swatch.name).into(),
                        ))
                        .debug_selector(|| format!("fill-swatch-{}", swatch.name))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_fill(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.fill_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Fill.idx()]))
                    // Occlude the card so a mouse-down on it can't reach the backdrop's dismiss
                    // listener painted behind it (BUG A/B): the card's `BlockMouse` hitbox drops
                    // the backdrop out of the hit-test under the pointer, so `is_hovered` is false
                    // there and the backdrop's `on_mouse_down` never fires. Without this, clicking
                    // an item dismissed the popover on mouse-DOWN, tearing it down before the item's
                    // `on_click` (mouse-UP) could apply. Items inside paint above the card, so their
                    // own clicks are unaffected; a click OUTSIDE the card still dismisses.
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("no-fill")
                            .label("No fill")
                            .debug_selector(|| "fill-no-fill".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_fill(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The text-color popover: the same palette as Fill, with **Automatic** (clear) in place of
    /// "No fill" (`components/action_bar.md`, `ui_design.md §2`).
    pub(super) fn render_text_color_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("text-swatch-{}", swatch.name).into(),
                        ))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_text_color(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.text_color_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::TextColor.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("text-automatic")
                            .label("Automatic")
                            .debug_selector(|| "text-automatic".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_text_color(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.text_color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The number-format dropdown (`functional_spec.md §10.1`, D10.1). Basics-first: the default
    /// view is the seven [`BASIC_FORMATS`] flat (no scroll) plus a trailing "More ▸" row; drilling
    /// in ([`num_fmt_more_open`](Self::num_fmt_more_open)) swaps to the full grouped
    /// [`NUM_FMT_GROUPS`] inventory with a "◂ Back" row. The preset matching the active cell's exact
    /// format code is highlighted at whichever level it lives; when the active format is a More-only
    /// preset the "More ▸" row is marked active (and the dropdown opens straight onto the grouped
    /// view — see [`toggle_num_fmt_popover`](Self::toggle_num_fmt_popover)).
    ///
    /// **Drill-in over flyout (D10.1):** the popover is a single fixed-anchor occluded card over one
    /// full-screen backdrop; a flyout would need a second card anchored to the dynamically-positioned
    /// "More ▸" row (offset + card width unknown without measurement), so drill-in — reusing the same
    /// card/backdrop/occlude/dismiss machinery — is the clean fit.
    pub(super) fn render_num_fmt_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Highlight the preset whose code exactly matches the active cell's, normalizing `general`
        // case (the engine may echo "General"). Presets can share a category but never a code, so an
        // exact match selects at most one preset (at whichever level it lives).
        let active_code = self.num_fmt_active_code();
        let body = if self.num_fmt_more_open {
            self.num_fmt_more_menu(&active_code, cx)
        } else {
            self.num_fmt_basic_menu(&active_code, cx)
        };

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.num_fmt_open = false;
                        this.num_fmt_more_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("numfmt-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::NumFmt.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .debug_selector(|| "numfmt-card".into())
                    .flex()
                    .flex_col()
                    .p_1()
                    // The grouped "More" inventory is tall — cap the height and scroll it (like the
                    // font-family popover). The basic list is short and never scrolls.
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(body),
            )
            .into_any_element()
    }

    /// The basics-first view: the seven [`BASIC_FORMATS`] flat, then a trailing "More ▸" row that
    /// drills into the full grouped inventory. `active_code` is the normalized active-cell format.
    fn num_fmt_basic_menu(&self, active_code: &str, cx: &mut Context<Self>) -> gpui::Div {
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for preset in BASIC_FORMATS {
            let code = preset.code.to_string();
            let selector = preset.code;
            menu = menu.child(
                Button::new(gpui::ElementId::Name(
                    format!("numfmt-{}", preset.code).into(),
                ))
                .label(preset.label)
                .debug_selector(move || format!("numfmt-{selector}"))
                .ghost()
                .small()
                .selected(preset.code == active_code)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_num_fmt(&code, window, cx);
                })),
            );
        }
        // "More ▸" reveals the full grouped inventory (drill-in). Marked active when the active
        // cell's format lives only under it, so the current format stays discoverable.
        menu.child(
            Button::new("numfmt-more")
                .label("More ▸")
                .debug_selector(|| "numfmt-more".into())
                .ghost()
                .small()
                .selected(is_more_only_num_fmt(active_code))
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.num_fmt_more_open = true;
                    cx.notify();
                })),
        )
    }

    /// The drilled-in "More" view: a "◂ Back" row that restores the basics, then the full grouped
    /// [`NUM_FMT_GROUPS`] inventory (section headers for multi-preset groups, each preset highlighted
    /// by exact code). This is the verbatim Phase-6 grouped render, relocated behind "More ▸".
    fn num_fmt_more_menu(&self, active_code: &str, cx: &mut Context<Self>) -> gpui::Div {
        let mut menu = div().flex().flex_col().gap(px(1.0)).child(
            Button::new("numfmt-back")
                .label("◂ Back")
                .debug_selector(|| "numfmt-back".into())
                .ghost()
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.num_fmt_more_open = false;
                    cx.notify();
                })),
        );
        for group in NUM_FMT_GROUPS {
            // A multi-preset group gets a muted section header; single-preset groups (General,
            // Text, …) read as plain top-level items, so no redundant header.
            if group.presets.len() > 1 {
                menu = menu.child(
                    div()
                        .px_1()
                        .pt(px(3.0))
                        .pb(px(1.0))
                        .text_xs()
                        .text_color(rgb(MUTED_TEXT))
                        .child(group.category.label()),
                );
            }
            for preset in group.presets {
                let code = preset.code.to_string();
                let selector = preset.code;
                menu = menu.child(
                    Button::new(gpui::ElementId::Name(
                        format!("numfmt-{}", preset.code).into(),
                    ))
                    .label(preset.label)
                    .debug_selector(move || format!("numfmt-{selector}"))
                    .ghost()
                    .small()
                    .selected(preset.code == active_code)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.apply_num_fmt(&code, window, cx);
                        },
                    )),
                );
            }
        }
        menu
    }

    /// The font-family dropdown: a scrolling menu of the installed families (fetched once at build),
    /// "Default (Inter)" first, the active cell's family highlighted (`components/action_bar.md`).
    pub(super) fn render_font_family_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_family_label().to_string();
        let names = Rc::clone(&self.font_names);
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for (i, name) in names.iter().enumerate() {
            let pick = name.to_string();
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger(
                    "font-family".into(),
                    i as u64,
                ))
                .label(name.clone())
                .debug_selector(move || format!("font-family-{i}"))
                .ghost()
                .small()
                .selected(name.as_ref() == active)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_font_family(&pick, window, cx);
                })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_family_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-family-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontFamily.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    // The installed-font list is long — cap the height and scroll it.
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The font-size dropdown: the fixed point list, the active cell's size highlighted.
    pub(super) fn render_font_size_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_size_label();
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for pt in FONT_SIZES {
            let label = format!("{pt}");
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger("font-size".into(), pt as u64))
                    .label(label.clone())
                    .debug_selector(move || format!("font-size-{pt}"))
                    .ghost()
                    .small()
                    .selected(label == active)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.apply_font_size(pt, window, cx);
                    })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_size_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-size-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontSize.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The borders **pen** popover (`ui_design.md §2`): three stacked regions — "Borders"
    /// target icons, a "Line" style gallery, and a "Color" swatch grid + custom picker. A target
    /// click paints the pen onto just those edges and keeps the popover open; only click-away / Esc
    /// closes it. The current target/pen is shown `.selected`.
    pub(super) fn render_borders_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Region A — the eight "Borders" target icons (icon-only, so each carries a tooltip).
        let target_btn = |id: &'static str,
                          name: &'static str,
                          preset: BorderPreset,
                          this: &Self,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .debug_selector(move || id.to_string())
                .ghost()
                .small()
                .w(px(40.0))
                .h(px(34.0))
                .tooltip(name)
                .selected(this.border_target == Some(preset))
                .child(border_target_icon(preset))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.select_border_target(preset, window, cx);
                }))
        };
        let row1 = div()
            .flex()
            .gap_1()
            .child(target_btn("border-all", "All", BorderPreset::All, self, cx))
            .child(target_btn(
                "border-inner",
                "Inner",
                BorderPreset::Inner,
                self,
                cx,
            ))
            .child(target_btn(
                "border-outer",
                "Outer",
                BorderPreset::Outer,
                self,
                cx,
            ))
            .child(target_btn(
                "border-none",
                "None",
                BorderPreset::None,
                self,
                cx,
            ));
        let row2 = div()
            .flex()
            .gap_1()
            .child(target_btn("border-top", "Top", BorderPreset::Top, self, cx))
            .child(target_btn(
                "border-bottom",
                "Bottom",
                BorderPreset::Bottom,
                self,
                cx,
            ))
            .child(target_btn(
                "border-left",
                "Left",
                BorderPreset::Left,
                self,
                cx,
            ))
            .child(target_btn(
                "border-right",
                "Right",
                BorderPreset::Right,
                self,
                cx,
            ));

        // Region B — the line-style gallery (each button previews the real line).
        let line_btn = |id: &'static str,
                        name: &'static str,
                        line: BorderLine,
                        this: &Self,
                        cx: &mut Context<Self>| {
            Button::new(id)
                .debug_selector(move || id.to_string())
                .ghost()
                .small()
                .h(px(28.0))
                .tooltip(name)
                .selected(this.border_line == line)
                .child(border_line_preview(line))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_border_line(line, window, cx);
                }))
        };
        let gallery = div()
            .flex()
            .gap_1()
            .child(line_btn(
                "border-line-thin",
                "Thin",
                BorderLine::ThinSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-medium",
                "Medium",
                BorderLine::MediumSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-thick",
                "Thick",
                BorderLine::ThickSolid,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-dashed",
                "Dashed",
                BorderLine::Dashed,
                self,
                cx,
            ))
            .child(line_btn(
                "border-line-double",
                "Double",
                BorderLine::Double,
                self,
                cx,
            ));

        // Region C — the color swatches (verbatim reuse of the fill popover's `FILL_PALETTE` grid;
        // the current pen color's swatch is ringed) + the inline "Custom…" picker.
        let mut swatches = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                let selected = color == self.border_color;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("border-swatch-{}", swatch.name).into(),
                        ))
                        .debug_selector(|| format!("border-swatch-{}", swatch.name))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        // Ring the pen's current swatch (a 2px accent border) so the selected color
                        // reads over any swatch fill; others keep the hairline outline.
                        .border_2()
                        .border_color(rgb(if selected {
                            SWATCH_SELECTED_RING
                        } else {
                            HAIRLINE
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.set_border_color(color, window, cx);
                            }),
                        ),
                );
            }
            swatches = swatches.child(r);
        }
        let color_region = div().flex().flex_col().gap_1().child(swatches).child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(MUTED_TEXT))
                        .child("Custom…"),
                )
                .child(ColorPicker::new(&self.border_color_picker).small()),
        );

        let section_label = |text: &'static str| {
            div()
                .text_size(px(11.0))
                .text_color(rgb(MUTED_TEXT))
                .child(text)
        };
        let divider = || div().h(px(1.0)).bg(rgb(HAIRLINE));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.borders_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Borders.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(section_label("Borders"))
                    .child(row1)
                    .child(row2)
                    .child(divider())
                    .child(section_label("Line"))
                    .child(gallery)
                    .child(section_label("Color"))
                    .child(color_region),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use gpui::TestAppContext;

    // ---- Action row: toggles + fill --------------------------------------------------------

    #[gpui::test]
    fn toggle_bold_sends_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Bold, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Bold,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn toggles_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                bold: true,
                italic: false,
                underline: true,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.bold_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.italic_active()));
        assert!(upd(&h, cx, |c, _w, _cx| c.underline_active()));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_toggles_send_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Strikethrough, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Strikethrough,
                ..
            }]
        ));
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::WrapText, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::WrapText,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                strikethrough: true,
                wrap: false,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.strikethrough_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.wrap_active()));
    }

    // ---- Action row: Merge / Unmerge toggle ------------------------------------------------

    /// B2:C3 (0-based rows 1–2, cols 1–2) — the merge fixture the toggle tests reuse.
    fn b2_c3() -> CellRange {
        CellRange::new(cell(1, 1), cell(2, 2))
    }

    /// Sets the active sheet's merges + drives a selection change, so `merge_active`/`toggle_merge`
    /// read a known merge state against a known selection.
    fn with_merges_and_selection(
        h: &Harness,
        cx: &mut TestAppContext,
        merges: Vec<CellRange>,
        selection: SelectionModel,
    ) {
        h.client.set_merges(SheetId(0), merges);
        upd(h, cx, |c, window, cx| {
            c.on_selection_changed(selection, window, cx)
        });
        h.client.take_commands(); // drop the selection-change fetch/commands
    }

    #[gpui::test]
    fn merge_toggle_inactive_on_mergeable_multicell(cx: &mut TestAppContext) {
        // A multi-cell selection with no merge inside it → inactive (a click would merge), enabled.
        let h = one_sheet(cx);
        with_merges_and_selection(
            &h,
            cx,
            vec![],
            SelectionModel {
                anchor: cell(0, 0),
                active: cell(1, 1),
            },
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.merge_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.merge_disabled()));
    }

    #[gpui::test]
    fn merge_toggle_active_when_selection_contains_a_merge(cx: &mut TestAppContext) {
        // A selection whose effective range hits a region → pressed/active (a click would unmerge).
        let h = one_sheet(cx);
        with_merges_and_selection(&h, cx, vec![b2_c3()], SelectionModel::single(cell(1, 1)));
        assert!(upd(&h, cx, |c, _w, _cx| c.merge_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.merge_disabled()));
    }

    #[gpui::test]
    fn merge_toggle_disabled_on_lone_single_cell(cx: &mut TestAppContext) {
        // A lone 1×1 not in any merge → nothing to toggle → disabled (and inactive).
        let h = one_sheet(cx);
        with_merges_and_selection(&h, cx, vec![b2_c3()], SelectionModel::single(cell(5, 5)));
        assert!(upd(&h, cx, |c, _w, _cx| c.merge_disabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.merge_active()));
    }

    #[gpui::test]
    fn merge_toggle_disabled_when_degraded(cx: &mut TestAppContext) {
        // Degraded/read-only disables the toggle even over a mergeable multi-cell selection.
        let h = one_sheet(cx);
        with_merges_and_selection(
            &h,
            cx,
            vec![],
            SelectionModel {
                anchor: cell(0, 0),
                active: cell(1, 1),
            },
        );
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.merge_disabled()));
    }

    #[gpui::test]
    fn toggle_merge_merges_a_plain_multicell_selection(cx: &mut TestAppContext) {
        // No interior merge → a click sends one unconfirmed MergeCells over the effective range.
        let h = one_sheet(cx);
        with_merges_and_selection(
            &h,
            cx,
            vec![],
            SelectionModel {
                anchor: cell(0, 0),
                active: cell(1, 1),
            },
        );
        upd(&h, cx, |c, window, cx| c.toggle_merge(window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::MergeCells { area, confirmed: false, .. }]
                    if *area == CellRange::new(cell(0, 0), cell(1, 1))
            ),
            "a merge-free multi-cell selection merges its effective range unconfirmed, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn toggle_merge_unmerges_when_selection_contains_regions(cx: &mut TestAppContext) {
        // A selection spanning two regions → a click unmerges BOTH (one UnmergeCells each, at the
        // region anchors), never a merge.
        let h = one_sheet(cx);
        let r1 = b2_c3(); // anchor (1,1)
        let r2 = CellRange::new(cell(4, 4), cell(5, 5)); // anchor (4,4)
        with_merges_and_selection(
            &h,
            cx,
            vec![r1, r2],
            SelectionModel {
                anchor: cell(1, 1),
                active: cell(5, 5),
            },
        );
        upd(&h, cx, |c, window, cx| c.toggle_merge(window, cx));
        let cmds = h.client.take_commands();
        let anchors: Vec<CellRef> = cmds
            .iter()
            .filter_map(|c| match c {
                Command::UnmergeCells { anchor, .. } => Some(*anchor),
                _ => None,
            })
            .collect();
        assert_eq!(
            anchors,
            vec![cell(1, 1), cell(4, 4)],
            "unmerge both regions"
        );
        assert!(
            !cmds.iter().any(|c| matches!(c, Command::MergeCells { .. })),
            "a selection containing merges must never issue a MergeCells, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn toggle_merge_noop_on_lone_single_cell(cx: &mut TestAppContext) {
        // A lone 1×1 not in any merge → the toggle sends nothing (the button is disabled anyway).
        let h = one_sheet(cx);
        with_merges_and_selection(&h, cx, vec![b2_c3()], SelectionModel::single(cell(5, 5)));
        upd(&h, cx, |c, window, cx| c.toggle_merge(window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a lone single cell not in a merge is a no-op"
        );
    }

    #[gpui::test]
    fn fill_swatch_and_no_fill(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        let accent = FILL_PALETTE[4].rgb; // Accent 1
        upd(&h, cx, |c, window, cx| {
            c.apply_fill(Some(accent), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if *rgb == accent
        ));
        upd(&h, cx, |c, window, cx| c.apply_fill(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Fill(None),
                ..
            }]
        ));
    }

    #[gpui::test]
    fn formatting_commits_pending_edit_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Italic, window, cx)
        });
        let cmds = h.client.take_commands();
        // Commit first, then the style.
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { input, .. }) if input == "=A1"),
            "pending edit committed first, got {cmds:?}"
        );
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::SetStyleAttr {
                attr: StyleAttr::Italic,
                ..
            }
        )));
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Select `cell` as a single-cell selection and drain the resulting fetch command.
    fn select_single(h: &Harness, cx: &mut TestAppContext, r: u32, c: u32) {
        upd(h, cx, |chrome, window, cx| {
            chrome.on_selection_changed(SelectionModel::single(cell(r, c)), window, cx)
        });
        h.client.take_commands();
    }

    #[gpui::test]
    fn alignment_toggle_emits_clear_on_repress(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly right-aligned.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                h_align: Some(Align::Right),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.align_active(Align::Right)));

        // Re-pressing the pressed alignment clears horizontal only (value "general").
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Right, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "general"
            ),
            "re-press clears with general, got {cmds:?}"
        );

        // Pressing a different (unpressed) alignment sets it directly.
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Left, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "left"
        ));
    }

    #[gpui::test]
    fn vertical_alignment_sets_and_reflects(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly top-aligned → the Top button reads pressed, others not.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Top),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Top)));
        assert!(!upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Bottom)));

        // Pressing a vertical-align button is a plain set (no re-press-to-clear).
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Bottom, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "bottom"
        ));

        // Re-pressing the already-active alignment re-applies it (no clear value).
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Center),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Center, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "center"
        ));
    }

    #[gpui::test]
    fn text_color_automatic_and_swatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        // Automatic clears the color (empty value).
        upd(&h, cx, |c, window, cx| c.apply_text_color(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()
            ),
            "Automatic clears color, got {cmds:?}"
        );

        // A swatch sends its #RRGGBB hex.
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value == "#FF0000"
        ));
    }

    #[gpui::test]
    fn num_fmt_pick_emits_code_and_category_reflects_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00%");
        select_single(&h, cx, 1, 1);
        // The dropdown label reflects the active cell's format category.
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Percent"
        );

        upd(&h, cx, |c, window, cx| {
            c.apply_num_fmt("$#,##0.00", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"
        ));
    }

    #[gpui::test]
    fn num_fmt_category_label_reflects_new_categories(cx: &mut TestAppContext) {
        // The grouped model added Scientific / Accounting categories; the dropdown button label must
        // reverse-map an active cell's code to the new category names.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00E+00");
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Scientific"
        );

        h.client
            .set_num_fmt(SheetId(0), cell(1, 1), "$#,##0.00;($#,##0.00)");
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Accounting"
        );
    }

    #[gpui::test]
    fn num_fmt_preset_pick_emits_grouped_code(cx: &mut TestAppContext) {
        // Picking a preset from a multi-preset group routes that exact code to the set-format command.
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_num_fmt("yyyy-mm-dd", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "yyyy-mm-dd"
            ),
            "a grouped Date preset routes its exact code, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn thousands_toggle_adds_and_removes_grouping(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // An ungrouped numeric format: toggle enabled, not active → adds grouping.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.00"
            ),
            "toggling on adds grouping, got {cmds:?}"
        );

        // A grouped format: enabled + active → removes grouping.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.00"
            ),
            "toggling off removes grouping, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn thousands_toggle_disabled_for_date_and_degraded(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // A date format has no integer digit placeholder → disabled + no-op.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "m/d/yyyy");
        select_single(&h, cx, 1, 1);
        assert!(!upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.thousands_active()));
        upd(&h, cx, |c, window, cx| {
            c.toggle_thousands_separator(window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a non-toggleable format sends nothing"
        );

        // A toggleable format disables once degraded.
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.toggle_thousands_enabled()));
    }

    #[gpui::test]
    fn decimals_buttons_emit_adjusted_code(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.000"
            ),
            "increase decimals rewrites the code, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_for_date_format(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "m/d/yyyy");
        select_single(&h, cx, 1, 1);
        // A date format has no adjustable decimal group → both buttons disabled + no-op.
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a no-op decimals adjust sends nothing"
        );
    }

    #[gpui::test]
    fn dropdown_anchors_capture_button_positions_left_to_right(cx: &mut TestAppContext) {
        // BUG 2c: each dropdown popover anchors under its real (content-sized) trigger button, not
        // a hardcoded x. After a paint, the `canvas` probes capture each button's laid-out left
        // edge; they must land in left-to-right action-row order and be strictly increasing.
        let h = one_sheet(cx);
        // Force the window to paint so the canvas probes capture each button's laid-out x.
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }

        let xs = upd(&h, cx, |c, _w, _cx| {
            [
                c.anchor_x_of(Anchor::FontFamily),
                c.anchor_x_of(Anchor::FontSize),
                c.anchor_x_of(Anchor::TextColor),
                c.anchor_x_of(Anchor::Fill),
                c.anchor_x_of(Anchor::Borders),
                c.anchor_x_of(Anchor::NumFmt),
            ]
        });
        assert!(
            xs[0] >= 0.0 && xs.windows(2).all(|w| w[1] > w[0]),
            "trigger anchors must be captured in strictly increasing left-to-right order, got {xs:?}"
        );
    }

    #[gpui::test]
    fn decimals_enabled_on_general_numeric_cell(cx: &mut TestAppContext) {
        // BUG 3: a plain number like `200000` is stored with the General format. The ± must still
        // be adjustable — increase applies `0.0`; decrease is a no-op at zero decimals (disabled).
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Number, "200000");
        select_single(&h, cx, 1, 1);

        assert!(
            upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
            "increase must be enabled on a General-formatted number"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            "decrease is a no-op on a General integer (0 decimals)"
        );

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.0"
            ),
            "increase on a General number applies a 0.0 format, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_on_general_text_cell(cx: &mut TestAppContext) {
        // A text cell under General is not numeric → the ± stay disabled and no-op.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Text, "hello");
        select_single(&h, cx, 1, 1);

        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a text General cell must not emit a number-format change"
        );
    }

    #[gpui::test]
    fn decimals_gating_for_custom_formats_matches_spec(cx: &mut TestAppContext) {
        // BUG C audit: for a cell with an explicit *custom* number format, ± must be enabled iff the
        // format is safely adjustable — single-section, no exponent (`E`/`e`), no quoted/escaped
        // literal (`functional_spec.md §3.4`, the deliberate Phase-4 gate). This locks the exact
        // enable/disable set so it can be reconciled against what the owner observed.
        let h = one_sheet(cx);
        fn gate(h: &Harness, cx: &mut TestAppContext, code: &str) -> (bool, bool) {
            h.client.set_num_fmt(SheetId(0), cell(1, 1), code);
            select_single(h, cx, 1, 1);
            (
                upd(h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
                upd(h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            )
        }
        // Safe single-section customs ARE enabled: increase always, decrease when ≥1 decimal.
        assert_eq!(gate(&h, cx, "0.00"), (true, true), "0.00");
        assert_eq!(gate(&h, cx, "#,##0.00"), (true, true), "#,##0.00");
        assert_eq!(gate(&h, cx, "0.00%"), (true, true), "0.00%");
        // `#,##0` has zero decimals → increase enabled, decrease a correct no-op (Excel: can't go
        // below 0). This is NOT a bug: the format IS adjustable, there is just nothing to remove.
        assert_eq!(gate(&h, cx, "#,##0"), (true, false), "#,##0");
        // Only exponent / quoted / multi-section customs are (correctly) disabled both ways.
        assert_eq!(gate(&h, cx, "0.00E+00"), (false, false), "0.00E+00");
        assert_eq!(gate(&h, cx, "0.0\"x\""), (false, false), "0.0\"x\"");
        assert_eq!(
            gate(&h, cx, "0.00;[Red]0.00"),
            (false, false),
            "0.00;[Red]0.00"
        );
    }

    #[gpui::test]
    fn controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        // Enabled before degradation.
        assert!(!upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));

        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        // The decimals gate folds in the degraded flag (the other controls disable via
        // `.disabled(self.is_degraded())` in the render path).
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
    }

    #[gpui::test]
    fn degraded_closes_popovers_and_blocks_dispatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Open the text-color popover, then degrade.
        upd(&h, cx, |c, _w, cx| c.toggle_text_color_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a swatch click can no longer dispatch a command.
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetStylePath dispatches while degraded"
        );
    }

    // ---- BUG A/B: popover item clicks APPLY (real mouse dispatch, not direct `apply_*`) -----
    //
    // These drive real mouse events through the rendered popover with a `VisualTestContext` over a
    // full-height backdrop (`tall_sheet` mounts a tall body stub so the backdrop — `size_full` of
    // the chrome — actually spans the dropdown items) — the path the part-1 anchor test and the
    // `apply_*` unit tests never exercised. Pre-fix, EVERY mouse-down inside the card reached the
    // backdrop: the menu `Button`s insert a plain (Normal) hitbox and only `prevent_default()` on
    // down (never `.occlude()`/`stop_propagation`), and the backdrop's `on_mouse_down` is not gated
    // on `default_prevented`, so a down directly on an item — as well as on the p_1/p_2 padding and
    // the gaps between rows — fired the backdrop's dismiss, tearing the popover down before the
    // item's `on_click` (mouse-UP) could dispatch. Wrapping the card in `.occlude()` inserts a
    // BlockMouse hitbox that breaks the hit-test before the backdrop for ALL in-card presses, so no
    // in-popover press can dismiss it. The mouse-DOWN is the discriminating signal (a full
    // `simulate_click` would not catch the regression: it sends down+up with no intervening repaint,
    // so the doomed button's `on_click` still fires); each per-item test below asserts the down
    // keeps the popover open — and fails without the card `.occlude()`.

    /// Opens a popover via `open`, paints, presses mouse **down** on the item registered under
    /// debug-selector `item`, asserts `open_flag` still holds (the down did not reach the backdrop
    /// dismiss — the BUG A/B guard), then releases and returns the dispatched commands.
    fn press_popover_button(
        h: &Harness,
        cx: &mut TestAppContext,
        open: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>),
        item: &'static str,
        open_flag: impl Fn(&ChromeView) -> bool,
    ) -> Vec<Command> {
        upd(h, cx, |c, w, cx| open(c, w, cx));
        h.client.take_commands(); // drop anything incidental to opening; isolate the click
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds(item)
            .unwrap_or_else(|| panic!("popover item {item:?} was not painted"))
            .center();
        let mods = gpui::Modifiers::default();
        vcx.simulate_mouse_down(center, MouseButton::Left, mods);
        let alive = vcx.update(|_w, cx| open_flag(h.chrome.read(cx)));
        assert!(
            alive,
            "popover item {item:?}: a mouse-DOWN must not dismiss the popover"
        );
        vcx.simulate_mouse_up(center, MouseButton::Left, mods);
        h.client.take_commands()
    }

    #[gpui::test]
    fn card_padding_click_keeps_popover_open(cx: &mut TestAppContext) {
        // Covers the card region a press can land on that ISN'T an item — the p_1 padding ring and
        // the gaps between rows. Like the buttons, pre-fix this reached the backdrop's dismiss
        // listener and closed the popover; the card `.occlude()` shields it too. Verified to fail
        // without the fix.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // The card's top-left padding corner (inside the p_1 border, above the first menu button).
        let pad = gpui::point(card.origin.x + px(1.0), card.origin.y + px(1.0));
        vcx.simulate_mouse_down(pad, MouseButton::Left, gpui::Modifiers::default());
        assert!(
            vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "a press on the card's padding must not dismiss the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "a press on the card padding dispatches no command"
        );
    }

    #[gpui::test]
    fn numfmt_currency_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Grouped presets are keyed by their exact code (`numfmt-<code>`); the `$1,234.56` Currency
        // preset sends `$#,##0.00`.
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_num_fmt_popover(cx),
            "numfmt-$#,##0.00",
            |c| c.num_fmt_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"),
            "clicking the Currency preset must dispatch its num-fmt, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying"
        );
    }

    // ---- Phase 10.1: number-format dropdown basics-first + "More ▸" drill-in ----------------

    #[gpui::test]
    fn num_fmt_basic_menu_paints_seven_and_more_row(cx: &mut TestAppContext) {
        // Basics-first: opening the dropdown paints the seven basic presets flat + a "More ▸" row,
        // and NONE of the More-only grouped inventory (`0.00` — the "1234.56" Number preset) nor a
        // "◂ Back" row. This is the regression fix — the common formats are visible without scroll.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("numfmt-card").is_some(),
            "the number-format card must be painted when open"
        );
        // The seven basic presets (`debug_bounds` needs a `'static` selector, so enumerate them).
        for sel in [
            "numfmt-general",
            "numfmt-#,##0.00",
            "numfmt-$#,##0.00",
            "numfmt-0.00%",
            "numfmt-m/d/yyyy",
            "numfmt-h:mm AM/PM",
            "numfmt-@",
        ] {
            assert!(
                vcx.debug_bounds(sel).is_some(),
                "basic preset {sel} must be painted in the basics-first view"
            );
        }
        assert!(
            vcx.debug_bounds("numfmt-more").is_some(),
            "the 'More ▸' row must be painted"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_none(),
            "a More-only preset (0.00) must NOT be painted in the basic view"
        );
        assert!(
            vcx.debug_bounds("numfmt-back").is_none(),
            "the '◂ Back' row must NOT be painted in the basic view"
        );
    }

    #[gpui::test]
    fn num_fmt_more_drilldown_and_back(cx: &mut TestAppContext) {
        // Clicking "More ▸" drills into the full grouped inventory (a More-only preset + the
        // "◂ Back" row now paint); clicking "◂ Back" restores the basics-first view.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let more = vcx
            .debug_bounds("numfmt-more")
            .expect("the 'More ▸' row was painted")
            .center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_more_open),
            "clicking 'More ▸' must enter the drill-in view"
        );
        assert!(
            vcx.debug_bounds("numfmt-back").is_some(),
            "the '◂ Back' row must be painted in the More view"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_some(),
            "a More-only preset (0.00) must be painted in the More view"
        );

        let back = vcx
            .debug_bounds("numfmt-back")
            .expect("the '◂ Back' row was painted")
            .center();
        vcx.simulate_click(back, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            !vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_more_open),
            "clicking '◂ Back' must restore the basics-first view"
        );
        assert!(
            vcx.debug_bounds("numfmt-more").is_some(),
            "the 'More ▸' row must be painted again after Back"
        );
        assert!(
            vcx.debug_bounds("numfmt-0.00").is_none(),
            "the More-only preset must be gone after Back"
        );
    }

    #[gpui::test]
    fn num_fmt_basic_pick_applies_and_closes(cx: &mut TestAppContext) {
        // Selecting a basic preset from the basics-first view routes its exact code and closes the
        // popover (and resets the drill-in state).
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_num_fmt_popover(cx),
            "numfmt-#,##0.00",
            |c| c.num_fmt_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.00"),
            "the basic Number preset must dispatch #,##0.00, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "the drill-in state must reset after applying"
        );
    }

    #[gpui::test]
    fn num_fmt_more_pick_applies_and_closes(cx: &mut TestAppContext) {
        // Drilling into "More ▸" and selecting a More-only preset routes its exact code and closes.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let more = vcx
            .debug_bounds("numfmt-more")
            .expect("the 'More ▸' row was painted")
            .center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        vcx.run_until_parked();
        h.client.take_commands(); // isolate the preset click
        let preset = vcx
            .debug_bounds("numfmt-0.00")
            .expect("the More-only preset (0.00) was painted")
            .center();
        vcx.simulate_click(preset, gpui::Modifiers::default());
        vcx.run_until_parked();
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.00"),
            "the More-only preset must dispatch 0.00, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying from the More view"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "the drill-in state must reset after applying"
        );
    }

    #[gpui::test]
    fn num_fmt_opens_onto_more_for_more_only_active(cx: &mut TestAppContext) {
        // Discoverability (D10.1): when the active cell's format lives only under "More ▸", opening
        // the dropdown lands directly on the grouped view so the current format is visible; a basic
        // active format opens basics-first.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00E+00"); // Scientific — More-only
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "a More-only active format must open onto the grouped view"
        );
        // Close, then a basic active format opens basics-first.
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "$#,##0.00"); // Currency — basic
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must be open"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_more_open),
            "a basic active format must open basics-first"
        );
    }

    #[gpui::test]
    fn text_color_automatic_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_text_color_popover(cx),
            "text-automatic",
            |c| c.text_color_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()),
            "Automatic must clear the font colour (empty value), got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open));
    }

    #[gpui::test]
    fn fill_no_fill_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_fill_popover(cx),
            "fill-no-fill",
            |c| c.fill_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr {
                    attr: StyleAttr::Fill(None),
                    ..
                }]
            ),
            "No fill must clear the fill, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn fill_swatch_click_applies_and_closes(cx: &mut TestAppContext) {
        // A swatch applies on `on_mouse_down` (the backdrop also dismissed on that same down pre-fix,
        // but the swatch's own listener still ran, so the command went out either way). This is
        // positive coverage that the card `.occlude()` doesn't break a swatch's own down-to-apply. A
        // single down suffices to dispatch its command.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_fill_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds("fill-swatch-Background 1")
            .expect("the first fill swatch was painted")
            .center();
        vcx.simulate_mouse_down(center, MouseButton::Left, gpui::Modifiers::default());
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if rgb.to_hex() == 0xFFFFFF
            ),
            "the first swatch must apply its colour, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn font_family_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Item 0 is always "Default (Inter)" → clears the family override (sent as `Some("")`).
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_family_popover(cx),
            "font-family-0",
            |c| c.font_family_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()),
            "Default (Inter) must clear the font family, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_family_open));
    }

    #[gpui::test]
    fn font_size_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_size_popover(cx),
            "font-size-14",
            |c| c.font_size_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 14.0).abs() < 1e-6),
            "clicking 14 must set the font size to 14 pt, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_size_open));
    }

    #[gpui::test]
    fn border_target_icon_click_paints_and_stays_open(cx: &mut TestAppContext) {
        // Pen model (`functional_spec.md §2.1`): a real click on the "All" target icon paints the
        // pen onto those edges AND — unlike the old apply-and-close preset path — leaves the
        // popover open with the target selected. `press_popover_button` already asserts the
        // mouse-DOWN doesn't dismiss; here we additionally assert it is still open after mouse-UP.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_borders_popover(cx),
            "border-all",
            |c| c.borders_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetBorders {
                    preset: BorderPreset::All,
                    line: BorderLine::ThinSolid,
                    color: Some(rgb),
                    ..
                }] if rgb.to_hex() == 0x000000
            ),
            "clicking All must paint the default thin-solid-black pen onto All, got {cmds:?}"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open),
            "the popover must STAY OPEN after a target click (pen model)"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            Some(BorderPreset::All),
            "the clicked target must become selected"
        );
    }

    #[gpui::test]
    fn popover_backdrop_outside_click_dismisses_without_dispatch(cx: &mut TestAppContext) {
        // The occluded card must still let a click OUTSIDE it hit the backdrop → dismiss (and never
        // dispatch a command).
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // A point on the backdrop but clear of the card: same top strip as the card (so it is within
        // the backdrop, which only spans the chrome height when no grid body is hosted) but far to
        // its left (the number-format trigger anchors the card on the right).
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );
        vcx.simulate_click(outside, gpui::Modifiers::default());

        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "a click outside the card dismisses the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "dismissing via the backdrop dispatches no command"
        );
    }

    #[gpui::test]
    fn popover_outside_click_removes_card_on_next_render_without_hover(cx: &mut TestAppContext) {
        // BUG B: the backdrop's dismiss closure must `cx.notify()` so the view repaints on the
        // very next frame. Without the notify the open-flag flips false but the view is never
        // marked dirty, so the popover card stays painted until some *unrelated* later event (a
        // hover/mouse-move) happens to repaint it — exactly the reported "won't close until the
        // mouse moves" symptom.
        //
        // The element-level discriminator: `debug_bounds` reads `window.rendered_frame`, which
        // only changes on an actual draw, and `simulate_event` ends in `run_until_parked`, which
        // redraws a window ONLY if something marked it dirty. So a single outside mouse-DOWN that
        // clears the flag but does not notify leaves the *previous* frame — card still present —
        // standing, with no intervening mouse-move. This asserts the card is GONE on that next
        // frame. Reverting the `cx.notify()` in `render_num_fmt_popover`'s backdrop closure makes
        // this fail (card still painted on the next render). Verified fail-without / pass-with.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted while open");
        // A point on the backdrop but clear of the card (top strip, far left of the right-anchored
        // card) — same geometry the sibling outside-click test uses.
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );

        // A single mouse-DOWN on the backdrop, and crucially NO following mouse-move / hover.
        vcx.simulate_mouse_down(outside, MouseButton::Left, gpui::Modifiers::default());

        // The flag flipped false...
        assert!(
            !vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "the outside press must clear the open flag"
        );
        // ...AND the dismiss notified, so the view repainted on the very next frame and the card
        // element is gone — no intervening hover needed. This is the assertion that fails without
        // the `cx.notify()`.
        assert!(
            vcx.debug_bounds("numfmt-card").is_none(),
            "the popover card must be gone on the very next render (the dismiss must cx.notify)"
        );
    }

    // ---- Action row: SetBorders (pen popover) ---------------------------------------------

    /// The pen dispatched by one `SetBorders`, asserting it is the single command and returning its
    /// `(preset, line, color)` for the test to check. Also asserts the range is the whole selection.
    fn one_border_cmd(cmds: &[Command]) -> (BorderPreset, BorderLine, Option<Rgb>) {
        match cmds {
            [Command::SetBorders {
                preset,
                line,
                color,
                range,
                ..
            }] => {
                assert_eq!(
                    *range,
                    freecell_core::CellRange::single(cell(1, 1)),
                    "the paint must cover the selection"
                );
                (*preset, *line, *color)
            }
            other => panic!("expected exactly one SetBorders, got {other:?}"),
        }
    }

    #[gpui::test]
    fn borders_popover_toggles(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
    }

    #[gpui::test]
    fn select_border_target_paints_and_stays_open(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        let (preset, line, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Outer);
        assert_eq!(line, BorderLine::ThinSolid, "the default pen line");
        assert_eq!(
            color.map(|c| c.to_hex()),
            Some(0x000000),
            "the default pen color (explicit black)"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open()),
            "a target click keeps the popover open (pen model)"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            Some(BorderPreset::Outer)
        );
    }

    #[gpui::test]
    fn set_border_line_with_target_repaints(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        // Changing the line with a target selected repaints that target with the new pen.
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::Dashed, window, cx)
        });
        let (preset, line, _) = one_border_cmd(&h.client.take_commands());
        assert_eq!((preset, line), (BorderPreset::Outer, BorderLine::Dashed));
    }

    #[gpui::test]
    fn set_border_color_with_target_repaints(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| c.set_border_color(red, window, cx));
        let (preset, _, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Outer);
        assert_eq!(color, Some(red), "the target repaints in the new pen color");
    }

    #[gpui::test]
    fn pen_carries_across_target_switch(cx: &mut TestAppContext) {
        // Set a non-default pen on Outer, then switch to Top — the carried-over pen paints Top.
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::Dashed, window, cx);
            c.set_border_color(red, window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Top, window, cx)
        });
        let (preset, line, color) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::Top);
        assert_eq!(
            line,
            BorderLine::Dashed,
            "pen line carries across the switch"
        );
        assert_eq!(color, Some(red), "pen color carries across the switch");
    }

    #[gpui::test]
    fn set_border_line_without_target_updates_pen_only(cx: &mut TestAppContext) {
        // No target selected: changing the line updates the pen but changes nothing on the sheet
        // (MVP; P2 restyle-all is deferred — GAPS F2).
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.set_border_line(BorderLine::ThickSolid, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "changing the line with no target selected must not touch the sheet"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_line()),
            BorderLine::ThickSolid,
            "the pen still updates (the next target click paints with it)"
        );
    }

    #[gpui::test]
    fn set_border_color_without_target_updates_pen_only(cx: &mut TestAppContext) {
        // Symmetric to the line path: with no target selected, changing the color updates the pen
        // only — no sheet change (MVP; P2 restyle-all is deferred — GAPS F2).
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        h.client.take_commands();
        let red = Rgb::from_hex(0xFF0000);
        upd(&h, cx, |c, window, cx| c.set_border_color(red, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "changing the color with no target selected must not touch the sheet"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_color()),
            red,
            "the pen still updates (the next target click paints with it)"
        );
    }

    #[gpui::test]
    fn border_none_clears_and_deselects(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        // Select a real target first so we can see None clear it.
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::None, window, cx)
        });
        let (preset, _, _) = one_border_cmd(&h.client.take_commands());
        assert_eq!(preset, BorderPreset::None, "None dispatches a clear");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "None leaves no target selected"
        );
        assert!(
            upd(&h, cx, |c, _w, _cx| c.borders_open()),
            "None clears but does not close the popover (only click-away/Esc closes)"
        );
    }

    #[test]
    fn border_target_icon_mask_matches_border_type_edges() {
        // The 2×2 icon's per-preset dark-edge table is the one piece of new UI logic with no render
        // coverage (the harness doesn't render the chrome popover), so pin it here: a future
        // Top/Bottom (or inner/outer) swap fails loudly. Tuple = (top, bottom, left, right,
        // inner_h, inner_v). Mirrors `functional_spec.md §2.2` / IronCalc's per-`BorderType` edges.
        use BorderPreset::*;
        assert_eq!(
            border_target_icon_mask(All),
            (true, true, true, true, true, true),
            "All darkens every outer edge + the inner cross"
        );
        assert_eq!(
            border_target_icon_mask(Inner),
            (false, false, false, false, true, true),
            "Inner darkens only the inner cross"
        );
        assert_eq!(
            border_target_icon_mask(Outer),
            (true, true, true, true, false, false),
            "Outer darkens only the perimeter"
        );
        assert_eq!(
            border_target_icon_mask(BorderPreset::None),
            (false, false, false, false, false, false),
            "None darkens nothing (all grey)"
        );
        assert_eq!(
            border_target_icon_mask(Top),
            (true, false, false, false, false, false),
            "Top darkens only the top outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Bottom),
            (false, true, false, false, false, false),
            "Bottom darkens only the bottom outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Left),
            (false, false, true, false, false, false),
            "Left darkens only the left outer edge"
        );
        assert_eq!(
            border_target_icon_mask(Right),
            (false, false, false, true, false, false),
            "Right darkens only the right outer edge"
        );
    }

    #[gpui::test]
    fn borders_reopen_resets_target_and_pen(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        // Dirty the transient state: a target + a non-default pen.
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx);
            c.set_border_line(BorderLine::Double, window, cx);
            c.set_border_color(Rgb::from_hex(0xFF0000), window, cx);
        });
        // Close, then reopen.
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "reopen resets the target"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_line()),
            BorderLine::ThinSolid,
            "reopen resets the pen line to the default"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_color().to_hex()),
            0x000000,
            "reopen resets the pen color to black"
        );
    }

    #[gpui::test]
    fn borders_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a target click can no longer dispatch.
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, window, cx| {
            c.select_border_target(BorderPreset::Outer, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetBorders dispatches while degraded"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.border_target()),
            None,
            "a degraded target click leaves no target selected"
        );
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    #[gpui::test]
    fn font_dropdowns_reflect_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_font_family(SheetId(0), cell(1, 1), "Arial");
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                font_size_q: 48, // 12pt
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.font_family_label().to_string()),
            "Arial"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "12");
    }

    #[gpui::test]
    fn font_size_box_shows_workbook_default_for_default_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // A default cell (no explicit font_size_q) shows the WORKBOOK default (13pt for a new
        // workbook) — not a hardcoded "11" that would mismatch the cell (CR Moderate).
        h.client.set_default_font_size_pt(13.0);
        select_single(&h, cx, 1, 1);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "13");

        // An opened file whose default is 10pt shows "10" for its default cells (and re-picking 10
        // is a no-op in the engine, so no size jump).
        h.client.set_default_font_size_pt(10.0);
        select_single(&h, cx, 2, 2);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "10");
    }

    #[gpui::test]
    fn font_family_pick_and_system_default(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Times New Roman", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetFont { family: Some(f), size_pt: None, .. }] if f == "Times New Roman"
            ),
            "family pick emits SetFont, got {cmds:?}"
        );

        // "Default (Inter)" clears the override (family = Some("")).
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family(SYSTEM_DEFAULT_FAMILY, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()
        ));
    }

    #[gpui::test]
    fn font_size_pick_emits_setfont(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| c.apply_font_size(18.0, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 18.0).abs() < 1e-9
        ));
    }

    #[gpui::test]
    fn font_controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // A pick made while degraded dispatches nothing.
        upd(&h, cx, |c, window, cx| c.apply_font_size(24.0, window, cx));
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Arial", window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetFont dispatches while degraded"
        );
    }
}
