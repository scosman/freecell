//! The declarative case table — **one row per rendering feature or meaningful permutation**
//! (`components/render_test_harness.md §Case inventory`). Adding a rendering feature = adding
//! rows here (stated as a review requirement in `README.md`). `name` is snake_case and IS the
//! baseline filename, so a red CI line names the exact broken feature.

use freecell_core::{Align, BorderSpec, CellRef, Edge, Rgb, SelectionModel};

use crate::scene::Scene;

/// A solid black border edge of the given px `weight` (the render-case builder's shorthand).
fn edge(weight: u8) -> Option<Edge> {
    Some(Edge::new(weight, Rgb::new(0, 0, 0)))
}

/// A four-sided border, every edge `weight` px black.
fn all_edges(weight: u8) -> BorderSpec {
    BorderSpec {
        top: edge(weight),
        right: edge(weight),
        bottom: edge(weight),
        left: edge(weight),
    }
}

/// A demonstrative multi-line-ish word with an ascender + descenders, so bold / italic /
/// underline read clearly in a baseline.
const SAMPLE: &str = "Sample";

/// One render case: a scene, the (tight) capture viewport, and any post-construction grid state
/// (selection / loading overlay / forced scrollbars / a scroll-into-view reveal) that the MVP
/// worker protocol can't express as an edit.
pub struct RenderCase {
    /// snake_case — IS the baseline PNG filename (`<name>.png`).
    pub name: &'static str,
    /// The engine-driven + cache-injected fixture.
    pub scene: Scene,
    /// Capture size in device px (small & tight — `components/render_test_harness.md`).
    pub viewport: (u32, u32),
    /// A non-default selection (drives the selection overlay / active-cell border).
    pub selection: Option<SelectionModel>,
    /// `Some(name)` renders the file-open loading overlay.
    pub loading: Option<&'static str>,
    /// Forces the overlay scrollbars visible (they otherwise fade).
    pub force_scrollbars: bool,
    /// A `(row, col)` scrolled fully into view before capture (deep-header / scroll cases).
    pub reveal: Option<(u32, u32)>,
    /// A live cell mirror `(row, col, raw text)` painted over the cell's published value while an
    /// edit is pending (`functional_spec.md §1.2`).
    pub mirror: Option<(u32, u32, &'static str)>,
    /// An open in-cell editor overlay `(row, col, text)` (`functional_spec.md §1.3`).
    pub in_cell: Option<(u32, u32, &'static str)>,
}

impl RenderCase {
    fn new(name: &'static str, scene: Scene, viewport: (u32, u32)) -> Self {
        Self {
            name,
            scene,
            viewport,
            selection: None,
            loading: None,
            force_scrollbars: false,
            reveal: None,
            mirror: None,
            in_cell: None,
        }
    }

    fn selection(mut self, selection: SelectionModel) -> Self {
        self.selection = Some(selection);
        self
    }

    fn mirror(mut self, row: u32, col: u32, text: &'static str) -> Self {
        self.mirror = Some((row, col, text));
        self
    }

    fn in_cell(mut self, row: u32, col: u32, text: &'static str) -> Self {
        self.in_cell = Some((row, col, text));
        self
    }

    fn loading(mut self, name: &'static str) -> Self {
        self.loading = Some(name);
        self
    }

    fn force_scrollbars(mut self) -> Self {
        self.force_scrollbars = true;
        self
    }

    fn reveal(mut self, row: u32, col: u32) -> Self {
        self.reveal = Some((row, col));
        self
    }
}

/// The tight viewport for single-feature cell/value/layout cases.
const CELL_VP: (u32, u32) = (480, 160);
/// A roomier viewport for whole-grid scenes (headers + selection + geometry).
const GRID_VP: (u32, u32) = (640, 320);

fn sel(anchor: (u32, u32), active: (u32, u32)) -> SelectionModel {
    SelectionModel {
        anchor: CellRef::new(anchor.0, anchor.1),
        active: CellRef::new(active.0, active.1),
    }
}

/// The whole initial suite (~45 cases). Rebuilt fresh on each call — the `render_scene` bin
/// looks a case up by name, so this is the single source of truth.
pub fn all() -> Vec<RenderCase> {
    let cell = |name, scene| RenderCase::new(name, scene, CELL_VP);
    // The tested cell sits at B2 so the default A1 active-cell outline never overlaps it.
    let at = |scene: Scene| scene.input(1, 1, SAMPLE);

    let mut cases = vec![
        // ---- Text attributes -----------------------------------------------------------
        cell("cell_plain", at(Scene::new())),
        cell("cell_bold", at(Scene::new()).bold(1, 1)),
        cell("cell_italic", at(Scene::new()).italic(1, 1)),
        cell("cell_underline", at(Scene::new()).underline(1, 1)),
        cell("cell_bold_italic", at(Scene::new()).bold(1, 1).italic(1, 1)),
        cell(
            "cell_bold_underline",
            at(Scene::new()).bold(1, 1).underline(1, 1),
        ),
        cell(
            "cell_italic_underline",
            at(Scene::new()).italic(1, 1).underline(1, 1),
        ),
        cell(
            "cell_bold_italic_underline",
            at(Scene::new()).bold(1, 1).italic(1, 1).underline(1, 1),
        ),
        // ---- Fill ----------------------------------------------------------------------
        cell("cell_fill_red", at(Scene::new()).fill(1, 1, 0xFF0000)),
        cell("cell_fill_yellow", at(Scene::new()).fill(1, 1, 0xFFEB3B)),
        cell(
            // A dark fill with an explicit light font colour for contrast.
            "cell_fill_dark_text_contrast",
            at(Scene::new())
                .fill(1, 1, 0x2E4053)
                .font_color(1, 1, 0xFFFFFF),
        ),
        cell(
            // A fill explicitly set then cleared ("No Fill") renders as the default white cell.
            "cell_fill_none_explicit",
            at(Scene::new()).fill(1, 1, 0xFF0000).fill_none(1, 1),
        ),
        cell(
            "cell_bold_fill_yellow",
            at(Scene::new()).bold(1, 1).fill(1, 1, 0xFFEB3B),
        ),
        cell(
            "cell_bold_italic_underline_fill_blue",
            at(Scene::new())
                .bold(1, 1)
                .italic(1, 1)
                .underline(1, 1)
                .fill(1, 1, 0x64B5F6),
        ),
        cell(
            // A 2×2 fill block: the fill paints over the interior gridlines (Excel look).
            "cell_fill_covers_gridlines",
            Scene::new().fill_range(
                freecell_core::CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)),
                0xFFEB3B,
            ),
        ),
        // ---- Values & engine-owned number formats --------------------------------------
        cell("cell_number_plain", Scene::new().input(1, 1, "42")),
        cell(
            "cell_number_thousands",
            Scene::new().input(1, 1, "1,234,567"),
        ),
        cell(
            "cell_number_currency",
            Scene::new().input(1, 1, "$1,234.50"),
        ),
        cell("cell_number_percent", Scene::new().input(1, 1, "50%")),
        cell(
            // NOTE (mvp-gaps Phase 1): despite the name, this renders in the DEFAULT colour,
            // not red. IronCalc infers `#,##0.00` from `-1,234.50` — a single colourless
            // section — so `resolve_text_color` short-circuits (`!num_fmt.contains('[')`) and
            // publishes no `text_color`. Phase 1 changes only its ALIGNMENT (Number → right).
            // The `[Red]` text-colour path (GAPS #2) has no render case — the Scene builder
            // can't set a custom `num_fmt` — so it is guarded by the engine integration test
            // `published_style_resolves_format_and_explicit_colors` (see DECISIONS §6).
            "cell_number_negative_red",
            Scene::new().input(1, 1, "-1,234.50"),
        ),
        cell("cell_date_default", Scene::new().input(1, 1, "2021-01-01")),
        cell("cell_boolean", Scene::new().input(1, 1, "TRUE")),
        cell("cell_text_plain", Scene::new().input(1, 1, "hello")),
        // ---- Formula errors (values, rendered in-cell) ---------------------------------
        cell("cell_error_div0", Scene::new().input(1, 1, "=1/0")),
        cell("cell_error_name", Scene::new().input(1, 1, "=NOTAREALNAME")),
        cell(
            // A two-cell ring: B2 → B3 → B2, so both resolve to #CIRC!.
            "cell_error_circ",
            Scene::new().input(1, 1, "=B3").input(2, 1, "=B2"),
        ),
        // ---- Layout / alignment / geometry ---------------------------------------------
        cell(
            "cell_align_left_text",
            Scene::new().input(1, 1, "Left").align(1, 1, Align::Left),
        ),
        cell(
            "cell_align_right_number",
            Scene::new().input(1, 1, "42").align(1, 1, Align::Right),
        ),
        cell(
            // A number defaults right (Phase-1 §1.3); an explicit Left alignment overrides
            // the type-default — the mirror of `cell_align_explicit_overrides_default`.
            "cell_number_align_left",
            Scene::new().input(1, 1, "42").align(1, 1, Align::Left),
        ),
        cell(
            "cell_align_center_explicit",
            Scene::new().input(1, 1, "Mid").align(1, 1, Align::Center),
        ),
        cell(
            // Text defaults left; an explicit Right alignment overrides that.
            "cell_align_explicit_overrides_default",
            Scene::new().input(1, 1, "Txt").align(1, 1, Align::Right),
        ),
        cell(
            "cell_text_clipped",
            Scene::new().input(1, 1, "clipped-very-long-text-abcdefghijklmnop"),
        ),
        cell(
            // A short word in a snug column: fills it without clipping.
            "cell_text_exact_fit",
            Scene::new().input(1, 1, "Exactly").col_width(1, 62.0),
        ),
        cell("cell_empty_styled", Scene::new().fill(1, 1, 0xFFEB3B)),
        cell(
            "cell_tall_row",
            Scene::new().input(1, 1, "Tall").row_height(1, 60.0),
        ),
        cell(
            "cell_wide_column",
            Scene::new().input(1, 1, "Wide column").col_width(1, 220.0),
        ),
        cell(
            "cell_narrow_column_clipped_number",
            Scene::new().input(1, 1, "123456789").col_width(1, 40.0),
        ),
        // ---- Whole-grid scenes ---------------------------------------------------------
        RenderCase::new("grid_empty_origin", Scene::new(), GRID_VP),
        RenderCase::new("grid_headers_scrolled_deep", Scene::new(), GRID_VP).reveal(500, 30),
        RenderCase::new(
            "grid_selection_single",
            Scene::new().input(2, 2, "C3").input(1, 1, "B2"),
            GRID_VP,
        )
        .selection(sel((2, 2), (2, 2))),
        RenderCase::new(
            "grid_selection_range",
            Scene::new().input(1, 1, "B2").input(3, 3, "D4"),
            GRID_VP,
        )
        .selection(sel((1, 1), (3, 3))),
        RenderCase::new(
            // A range extending well past the viewport → the overlay clips at the bottom/right.
            "grid_selection_range_spans_edge",
            Scene::new(),
            GRID_VP,
        )
        .selection(sel((1, 1), (40, 26))),
        RenderCase::new(
            // A shift-click / shift-extend outcome with the ACTIVE cell at the range's TOP-LEFT
            // corner (extension up-left): the "white anchor" sits at B2, exercising the overlay
            // sub-rectangles for an active cell the bottom-right cases don't cover (Phase 8).
            "grid_selection_shift_extended",
            Scene::new().input(1, 1, "B2").input(4, 4, "E5"),
            GRID_VP,
        )
        .selection(sel((4, 4), (1, 1))),
        RenderCase::new(
            // A click-drag outcome: a larger block dragged out from the anchor (B2→E6), active
            // at the bottom-right (Phase 8 drag-extend).
            "grid_selection_drag_extended",
            Scene::new().input(1, 1, "B2").input(5, 4, "E6"),
            GRID_VP,
        )
        .selection(sel((1, 1), (5, 4))),
        RenderCase::new(
            // A selection scrolled so its top-left is clipped ABOVE/LEFT of the viewport (the
            // complement of `grid_selection_range_spans_edge`): anchor off-screen, active cell
            // visible near the bottom-right — the drag-then-auto-scroll end state (Phase 8).
            "grid_selection_scrolled",
            Scene::new().input(2, 1, "top").input(24, 8, "deep"),
            GRID_VP,
        )
        .selection(sel((2, 1), (24, 8)))
        .reveal(24, 8),
        RenderCase::new(
            "grid_variable_geometry",
            Scene::new()
                .input(0, 0, "A1")
                .input(1, 1, "B2")
                .input(2, 2, "C3")
                .col_width(1, 180.0)
                .col_width(3, 60.0)
                .row_height(2, 52.0)
                .row_height(4, 40.0),
            GRID_VP,
        ),
        RenderCase::new(
            "grid_loading_overlay",
            Scene::new().input(0, 0, "A1"),
            GRID_VP,
        )
        .loading("Book.xlsx"),
        RenderCase::new(
            "grid_scrollbars_visible",
            Scene::new().input(0, 0, "A1").input(1, 1, "B2"),
            GRID_VP,
        )
        .force_scrollbars(),
        RenderCase::new("grid_mixed_content", mixed_content_scene(), (720, 400))
            .selection(sel((2, 1), (4, 3))),
        // ---- Editing feel (Phase 2): live mirror + in-cell editor overlay --------------
        RenderCase::new(
            // The active cell shows the raw text being typed (default style, left-aligned)
            // instead of its committed value (`functional_spec.md §1.2`).
            "cell_mirror_typing",
            Scene::new().input(1, 1, "42"),
            CELL_VP,
        )
        .selection(sel((1, 1), (1, 1)))
        .mirror(1, 1, "=1+2"),
        RenderCase::new(
            // The in-cell editor overlay open over B2 (2 px accent border, raw content;
            // `functional_spec.md §1.3`).
            "incell_editor_open",
            Scene::new().input(1, 1, "42"),
            CELL_VP,
        )
        .selection(sel((1, 1), (1, 1)))
        .in_cell(1, 1, "=SUM(A1:A3)"),
        // ---- Fonts (Phase 5): family + size + row auto-grow -----------------------------
        cell(
            // A serif family (visibly distinct from the default sans) rendered per-cell. NOTE:
            // this depends on a serif font being installed on the pinned runner — see DECISIONS.
            "font_family_serif",
            at(Scene::new()).font(1, 1, Some("DejaVu Serif"), None),
        ),
        cell(
            // 24pt text in a row grown to fit it (the worker's auto-grow, simulated by the injected
            // row height ≈ ceil(24*96/72*1.25)+4 IronCalc px → FreeCell px).
            "font_size_24_row_grown",
            at(Scene::new())
                .font(1, 1, None, Some(24.0))
                .row_height(1, 38.0),
        ),
        cell(
            // A family the runner does not have → gpui falls back to the default font (display-only;
            // the style is preserved). Guards that a missing family never blanks the cell.
            "font_missing_family_fallback",
            at(Scene::new()).font(1, 1, Some("NoSuchFontXYZ123"), None),
        ),
        // ---- Borders (Phase 6): edge paint, presets, shared-edge precedence -------------
        cell(
            // A single cell with all four thin (1px) black edges.
            "border_all_thin",
            at(Scene::new()).border(1, 1, all_edges(1)),
        ),
        cell(
            // A 2×2 block with a medium (2px) OUTER border only — each corner cell carries just its
            // two outward edges, so no interior edges draw.
            "border_outer_medium",
            at(Scene::new())
                .input(1, 2, "b")
                .input(2, 1, "c")
                .input(2, 2, "d")
                .border(
                    1,
                    1,
                    BorderSpec {
                        top: edge(2),
                        left: edge(2),
                        ..BorderSpec::NONE
                    },
                )
                .border(
                    1,
                    2,
                    BorderSpec {
                        top: edge(2),
                        right: edge(2),
                        ..BorderSpec::NONE
                    },
                )
                .border(
                    2,
                    1,
                    BorderSpec {
                        bottom: edge(2),
                        left: edge(2),
                        ..BorderSpec::NONE
                    },
                )
                .border(
                    2,
                    2,
                    BorderSpec {
                        bottom: edge(2),
                        right: edge(2),
                        ..BorderSpec::NONE
                    },
                ),
        ),
        cell(
            // Adjacent cells DISAGREE on the shared edge: B2's right is thin (1px), C2's left is
            // thick (3px). The heavier (thick) wins, and the edge is drawn once — by B2.
            "border_heavier_edge_wins",
            at(Scene::new())
                .input(1, 2, "X")
                .border(
                    1,
                    1,
                    BorderSpec {
                        right: edge(1),
                        ..BorderSpec::NONE
                    },
                )
                .border(
                    1,
                    2,
                    BorderSpec {
                        left: edge(3),
                        ..BorderSpec::NONE
                    },
                ),
        ),
        cell(
            // A border painted over a fill: the edges draw ON TOP of the yellow fill (Excel look).
            "border_over_fill",
            at(Scene::new())
                .fill(1, 1, 0xFFEB3B)
                .border(1, 1, all_edges(1)),
        ),
        cell(
            // Two adjacent all-thin cells: the shared vertical edge is drawn exactly ONCE (by the
            // left cell), so it looks identical to a single continuous line — no double-thick seam.
            "border_shared_edge_adjacent",
            at(Scene::new())
                .input(1, 2, "Y")
                .border(1, 1, all_edges(1))
                .border(1, 2, all_edges(1)),
        ),
        cell(
            // A cell whose border was cleared (NONE) renders as a plain cell — guards the clear path.
            "border_none_clear",
            at(Scene::new()).border(1, 1, BorderSpec::NONE),
        ),
    ];

    // A stable order is nice for the changed/unchanged summary; keep table order.
    cases.sort_by_key(|c| c.name);
    cases
}

/// The busy "canary" scene: a realistic mix of headers, values, number formats, fills,
/// character styles, alignment, and variable geometry — the case that catches "everything
/// subtly moved" (`components/render_test_harness.md`).
fn mixed_content_scene() -> Scene {
    Scene::new()
        .input(0, 0, "Item")
        .input(0, 1, "Qty")
        .input(0, 2, "Price")
        .input(0, 3, "Total")
        .bold(0, 0)
        .bold(0, 1)
        .bold(0, 2)
        .bold(0, 3)
        .fill_range(
            freecell_core::CellRange::new(CellRef::new(0, 0), CellRef::new(0, 3)),
            0xE0E0E0,
        )
        .input(1, 0, "Widget")
        .input(1, 1, "3")
        .input(1, 2, "$4.50")
        .input(1, 3, "=B2*C2")
        .input(2, 0, "Gadget")
        .input(2, 1, "12")
        .input(2, 2, "$1.25")
        .input(2, 3, "=B3*C3")
        .input(3, 0, "Gizmo")
        .input(3, 1, "7")
        .input(3, 2, "$9.99")
        .input(3, 3, "=B4*C4")
        .input(5, 0, "Discount")
        .input(5, 2, "10%")
        .italic(5, 0)
        .input(6, 0, "Note")
        .input(6, 1, "clipped-long-note-text")
        .align(1, 1, Align::Right)
        .align(2, 1, Align::Right)
        .align(3, 1, Align::Right)
        .col_width(0, 120.0)
        .row_height(0, 30.0)
}
