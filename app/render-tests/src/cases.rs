//! The declarative case table — **one row per rendering feature or meaningful permutation**
//! (`components/render_test_harness.md §Case inventory`). Adding a rendering feature = adding
//! rows here (stated as a review requirement in `README.md`). `name` is snake_case and IS the
//! baseline filename, so a red CI line names the exact broken feature.

use freecell_chart_model::{
    Anchor, AnchorCell, Axis, BarDir, BarLayout, Category, Chart, ChartColor, ChartId,
    ChartInsertKind, ChartKind, ChartSpec, Color, Grouping, Legend, Marker, MarkerSymbol,
    ScatterStyle, Series, SizeRepresentation, SourceXml, ThemeSlot,
};
use freecell_core::{Align, BorderSpec, CellRef, Edge, LinePattern, Rgb, SelectionModel, VAlign};

use crate::scene::Scene;

/// A solid black border edge of the given px `weight` (the render-case builder's shorthand).
fn edge(weight: u8) -> Option<Edge> {
    Some(Edge::new(weight, Rgb::new(0, 0, 0)))
}

/// A black border edge of the given px `weight` and line `pattern` (dashed / double render cases).
fn edge_pat(weight: u8, pattern: LinePattern) -> Option<Edge> {
    Some(Edge::with_pattern(weight, Rgb::new(0, 0, 0), pattern))
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

/// A four-sided border, every edge `weight` px black drawn with `pattern`.
fn all_edges_pat(weight: u8, pattern: LinePattern) -> BorderSpec {
    BorderSpec {
        top: edge_pat(weight, pattern),
        right: edge_pat(weight, pattern),
        bottom: edge_pat(weight, pattern),
        left: edge_pat(weight, pattern),
    }
}

/// A demonstrative multi-line-ish word with an ascender + descenders, so bold / italic /
/// underline read clearly in a baseline.
const SAMPLE: &str = "Sample";

/// A medium sentence for the wrap auto-grow cases — long enough to wrap to a few lines at a
/// moderate column and more/fewer as the column narrows/widens (`functional_spec.md §3.2`).
const AUTO_GROW_TEXT: &str = "This note wraps across several lines when the column is narrow";

/// A very long note for the cap case — at a narrow column it needs far more than the ~10-line cap,
/// so the row clamps at `MAX_AUTO_ROW_HEIGHT_PX` and the overflow clips within the cell.
const AUTO_GROW_CAP_TEXT: &str = "This is a very long wrapped note that keeps going and going \
    well past ten lines so the row height is clamped at the cap and the remaining text is clipped \
    inside the cell rather than filling the entire screen with one enormous row of text";

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
    /// `Some(title)` prepends the macOS custom titlebar row over the grid (`architecture.md
    /// §7.1, §9`). The row is just a div, so it renders in the Linux harness too — the *native*
    /// macOS integration (transparent titlebar + traffic lights) is the on-device smoke.
    pub titlebar: Option<&'static str>,
    /// Charts installed on the grid's **ChartLayer** for this case (P8, `charts/architecture.md
    /// §4.2`) — the harness hands them to `GridView::set_sheet_charts` for the active sheet. Empty
    /// for every non-chart case (no ChartLayer painted → no baseline change).
    pub charts: Vec<ChartSpec>,
    /// A selected chart (P18) — its stable [`ChartId`], drawn with the selection outline + resize
    /// handles. `None` (the default) draws no selection chrome, so no existing baseline moves.
    pub selected_chart: Option<ChartId>,
    /// Opts this case into the wrap-driven row auto-grow measurement (`functional_spec.md §3`): the
    /// harness runs `GridView::autogrow_measure_now` before first paint so the captured frame shows
    /// the real grown row heights. `false` for every non-auto-grow case (no baseline change).
    pub auto_grow: bool,
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
            titlebar: None,
            charts: Vec::new(),
            selected_chart: None,
            auto_grow: false,
        }
    }

    /// Opts this case into the wrap-driven row auto-grow measurement (`functional_spec.md §3`).
    fn auto_grow(mut self) -> Self {
        self.auto_grow = true;
        self
    }

    fn selection(mut self, selection: SelectionModel) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Installs charts on the case's ChartLayer (P8).
    fn charts(mut self, charts: Vec<ChartSpec>) -> Self {
        self.charts = charts;
        self
    }

    /// Selects the chart with this [`ChartId`] (P18) — draws the selection outline + resize handles.
    fn selected_chart(mut self, id: ChartId) -> Self {
        self.selected_chart = Some(id);
        self
    }

    fn titlebar(mut self, title: &'static str) -> Self {
        self.titlebar = Some(title);
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

// ---- In-grid chart fixtures (P8, `charts/architecture.md §4.2`) ----------------------------
// A grid-chart case is an ordinary engine-backed grid scene plus `.charts(...)`, so the ChartLayer
// paints over real cells at the chart's anchor. The anchor + viewport are sized against the default
// 100 px column / 24 px row geometry so the chart lands over the data table below it.

/// Viewport for the in-grid chart scenes — wide/tall enough to show the anchored chart (which spans
/// ~500×312 content px) plus the surrounding header row + label column.
const CHART_GRID_VP: (u32, u32) = (760, 420);

/// The shared chart placement: a `twoCellAnchor` from B2 (col 1, row 1) to G15 (col 6, row 14), so
/// the chart floats **over** the data table's numbers (cols B–D) while the header row / label column
/// stay visible around it. Carries small EMU offsets so the intra-cell offset path is exercised.
fn chart_anchor() -> Anchor {
    Anchor::new(
        AnchorCell::with_offsets(1, 9_525, 1, 9_525),
        AnchorCell::with_offsets(6, 0, 14, 0),
    )
}

/// A small data table + header row/label column the chart floats over — the "spreadsheet with a
/// chart in it" look (`ui_design.md §1`).
fn chart_backing_scene() -> Scene {
    Scene::new()
        .input(0, 0, "Region")
        .input(0, 1, "Q1")
        .input(0, 2, "Q2")
        .input(0, 3, "Q3")
        .input(1, 0, "North")
        .input(2, 0, "South")
        .input(3, 0, "West")
        .input(1, 1, "32")
        .input(1, 2, "55")
        .input(1, 3, "78")
        .input(2, 1, "74")
        .input(2, 2, "48")
        .input(2, 3, "63")
        .input(3, 1, "50")
        .input(3, 2, "49")
        .input(3, 3, "61")
}

/// A three-region multi-series line chart (theme colors + markers), the picture the ChartLayer
/// paints for the Faithful / Degraded cases.
fn in_grid_line_chart(title: &str) -> Chart {
    let months = || {
        ["Jan", "Feb", "Mar", "Apr"]
            .into_iter()
            .map(|m| Category::Text(m.into()))
            .collect::<Vec<_>>()
    };
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        },
        series: vec![
            Series::category_value(Some("North"), months(), vec![32.0, 41.0, 55.0, 62.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent1)),
            Series::category_value(Some("South"), months(), vec![74.0, 60.0, 48.0, 52.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent2)),
            Series::category_value(Some("West"), months(), vec![50.0, 54.0, 49.0, 58.0])
                .with_color(ChartColor::theme(ThemeSlot::Accent3)),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** line-chart spec at [`chart_anchor`], whose retained source classifies to a chosen
/// [`Fidelity`](freecell_chart_model::Fidelity) (via `source_xml`) so a single fixture drives the
/// Faithful and Degraded cases.
fn in_grid_chart_spec(title: &str, source_xml: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_line_chart(title),
        SourceXml::new(source_xml),
        Vec::new(),
        chart_anchor(),
    )
}

/// The **near-empty authored** spec the action-bar insert flow produces (P17): a
/// [`ChartInsertKind::Line`] `near_empty_chart` — one placeholder series ("Series 1" over
/// categories 1..4), a generic "Chart" title, untitled axes, and a right legend — placed at the
/// shared [`chart_anchor`]. Unlike every other in-grid chart case (all `ChartSpec::loaded`), this
/// is an **authored** spec, so it exercises the authored → in-grid render path AND is the exact
/// picture the user sees the instant they insert a line chart. Authored ⇒
/// [`Fidelity::Faithful`](freecell_chart_model::Fidelity), so the real single-series line renders.
fn in_grid_authored_inserted_spec() -> ChartSpec {
    ChartSpec::authored(ChartInsertKind::Line.near_empty_chart(), chart_anchor())
}

/// A three-region clustered **column** chart (P22) over the backing table's quarters — the picture the
/// ChartLayer paints for the in-grid column case (`grid_chart_column`).
fn in_grid_column_chart(title: &str) -> Chart {
    let months = || {
        ["Jan", "Feb", "Mar", "Apr"]
            .into_iter()
            .map(|m| Category::Text(m.into()))
            .collect::<Vec<_>>()
    };
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        },
        series: vec![
            Series::category_value(Some("North"), months(), vec![32.0, 41.0, 55.0, 62.0])
                .with_color(Color::from_hex(0x4472C4)),
            Series::category_value(Some("South"), months(), vec![74.0, 60.0, 48.0, 52.0])
                .with_color(Color::from_hex(0xED7D31)),
            Series::category_value(Some("West"), months(), vec![50.0, 54.0, 49.0, 58.0])
                .with_color(Color::from_hex(0xFFC000)),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** clustered-column `ChartSpec` at [`chart_anchor`], with a `<c:barChart>` source so it
/// classifies Faithful — the in-grid proof of the ChartLayer → `bar_element` path (P22), the column
/// analogue of the loaded line case.
fn in_grid_column_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_column_chart(title),
        SourceXml::new("<c:barChart><c:barDir val=\"col\"/></c:barChart>"),
        Vec::new(),
        chart_anchor(),
    )
}

/// A three-region standard **area** chart (P23) over the backing table's quarters — the picture the
/// ChartLayer paints for the in-grid area case (`grid_chart_area`). Authored tallest-first so the
/// overlapping bands read.
fn in_grid_area_chart(title: &str) -> Chart {
    let months = || {
        ["Jan", "Feb", "Mar", "Apr"]
            .into_iter()
            .map(|m| Category::Text(m.into()))
            .collect::<Vec<_>>()
    };
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Area {
            grouping: Grouping::Standard,
        },
        series: vec![
            Series::category_value(Some("North"), months(), vec![74.0, 60.0, 68.0, 82.0])
                .with_color(Color::from_hex(0x4472C4)),
            Series::category_value(Some("South"), months(), vec![50.0, 54.0, 49.0, 58.0])
                .with_color(Color::from_hex(0xED7D31)),
            Series::category_value(Some("West"), months(), vec![32.0, 41.0, 36.0, 45.0])
                .with_color(Color::from_hex(0xFFC000)),
        ],
        cat_axis: Axis::titled("Month"),
        val_axis: Axis::titled("Units"),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** standard-area `ChartSpec` at [`chart_anchor`], with a `<c:areaChart>` source so it
/// classifies Faithful — the in-grid proof of the ChartLayer → `area_element` path (P23), the area
/// analogue of the loaded column case.
fn in_grid_area_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_area_chart(title),
        SourceXml::new("<c:areaChart><c:grouping val=\"standard\"/></c:areaChart>"),
        Vec::new(),
        chart_anchor(),
    )
}

/// A single-series **pie** (P24) over four market-share slices — the picture the ChartLayer paints
/// for the in-grid pie case (`grid_chart_pie`). A pie is single-series; its slices are the
/// categories, colored by the varied palette.
fn in_grid_pie_chart(title: &str) -> Chart {
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Pie {
            doughnut_hole: None,
            first_slice_ang: 0,
            vary_colors: true,
        },
        series: vec![Series::category_value(
            Some("Share"),
            ["North", "South", "East", "West"]
                .into_iter()
                .map(|c| Category::Text(c.into()))
                .collect(),
            vec![40.0, 25.0, 20.0, 15.0],
        )],
        cat_axis: Axis::untitled(),
        val_axis: Axis::untitled(),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** pie `ChartSpec` at [`chart_anchor`], with a `<c:pieChart>` source so it classifies
/// Faithful — the in-grid proof of the ChartLayer → `pie_element` path (P24), the pie analogue of the
/// loaded column/area cases.
fn in_grid_pie_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_pie_chart(title),
        SourceXml::new("<c:pieChart><c:varyColors val=\"1\"/></c:pieChart>"),
        Vec::new(),
        chart_anchor(),
    )
}

/// A two-series **marker scatter** (P25) over two numeric axes — the picture the ChartLayer paints
/// for the in-grid scatter case (`grid_chart_scatter`). Each series carries its xy pairs, a distinct
/// color, and a distinct marker symbol.
fn in_grid_scatter_chart(title: &str) -> Chart {
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Scatter {
            style: ScatterStyle::Marker,
        },
        series: vec![
            Series::xy(
                Some("Group A"),
                vec![1.0, 2.5, 3.5, 5.0, 6.0],
                vec![12.0, 24.0, 19.0, 33.0, 41.0],
            )
            .with_color(Color::from_hex(0x4472C4))
            .with_marker(Marker::new(MarkerSymbol::Circle)),
            Series::xy(
                Some("Group B"),
                vec![1.5, 3.0, 4.5, 6.0, 7.0],
                vec![40.0, 32.0, 51.0, 45.0, 62.0],
            )
            .with_color(Color::from_hex(0xED7D31))
            .with_marker(Marker::new(MarkerSymbol::Diamond)),
        ],
        cat_axis: Axis::titled("X"),
        val_axis: Axis::titled("Y"),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** marker-scatter `ChartSpec` at [`chart_anchor`], with a `<c:scatterChart>` source so it
/// classifies Faithful — the in-grid proof of the ChartLayer → `scatter_element` path (P25), the
/// scatter analogue of the loaded column/area/pie cases.
fn in_grid_scatter_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_scatter_chart(title),
        SourceXml::new("<c:scatterChart><c:scatterStyle val=\"marker\"/></c:scatterChart>"),
        Vec::new(),
        chart_anchor(),
    )
}

/// A two-series **area-encoded bubble** (P26) over two numeric axes — the picture the ChartLayer
/// paints for the in-grid bubble case (`grid_chart_bubble`). Each series carries its `(x, y, size)`
/// points and a distinct color.
fn in_grid_bubble_chart(title: &str) -> Chart {
    Chart {
        title: Some(title.into()),
        kind: ChartKind::Bubble {
            size_representation: SizeRepresentation::Area,
        },
        series: vec![
            Series::bubble(
                Some("North"),
                vec![2.0, 4.0, 5.5, 7.0],
                vec![18.0, 34.0, 26.0, 45.0],
                vec![10.0, 40.0, 22.0, 60.0],
            )
            .with_color(Color::from_hex(0x4472C4)),
            Series::bubble(
                Some("South"),
                vec![3.0, 5.0, 6.5, 8.0],
                vec![52.0, 44.0, 60.0, 55.0],
                vec![48.0, 18.0, 34.0, 25.0],
            )
            .with_color(Color::from_hex(0xED7D31)),
        ],
        cat_axis: Axis::titled("X"),
        val_axis: Axis::titled("Y"),
        legend: Some(Legend::default()),
    }
}

/// A **loaded** bubble `ChartSpec` at [`chart_anchor`], with a `<c:bubbleChart>` source so it
/// classifies Faithful — the in-grid proof of the ChartLayer → `bubble_element` path (P26), the
/// bubble analogue of the loaded scatter case.
fn in_grid_bubble_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_bubble_chart(title),
        SourceXml::new("<c:bubbleChart><c:sizeRepresents val=\"area\"/></c:bubbleChart>"),
        Vec::new(),
        chart_anchor(),
    )
}

/// An **Unsupported** spec: a `surfaceChart` source (no faithful 2-D rendering) so the ChartLayer
/// draws the placeholder box; its `chart` carries the title the placeholder shows.
fn in_grid_unsupported_spec(title: &str) -> ChartSpec {
    ChartSpec::loaded(
        in_grid_line_chart(title),
        SourceXml::new("<c:surfaceChart/>"),
        Vec::new(),
        chart_anchor(),
    )
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
        cell("cell_strikethrough", at(Scene::new()).strikethrough(1, 1)),
        cell(
            // Strikethrough combines with underline (a cell can carry both — `functional_spec.md §1.1`).
            "cell_strikethrough_underline",
            at(Scene::new()).strikethrough(1, 1).underline(1, 1),
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
            // Wrap on: the text flows onto multiple lines constrained to the (narrow) column and
            // clips to the row height — no overflow into neighbours, no auto-grow (GAPS F1).
            "cell_wrap_multiline_clipped",
            Scene::new()
                .input(1, 1, "wrap this long text onto several lines")
                .col_width(1, 72.0)
                .row_height(1, 48.0)
                .wrap(1, 1),
        ),
        // Vertical alignment: a tall row so top / middle / bottom placement is visible. Unset
        // renders bottom (the grid default under decision C), covered by every other tall-row
        // case above.
        cell(
            "cell_valign_top",
            Scene::new()
                .input(1, 1, "Top")
                .row_height(1, 60.0)
                .v_align(1, 1, VAlign::Top),
        ),
        cell(
            "cell_valign_middle",
            Scene::new()
                .input(1, 1, "Mid")
                .row_height(1, 60.0)
                .v_align(1, 1, VAlign::Center),
        ),
        cell(
            "cell_valign_bottom",
            Scene::new()
                .input(1, 1, "Bot")
                .row_height(1, 60.0)
                .v_align(1, 1, VAlign::Bottom),
        ),
        cell(
            // Wrap + vertical alignment: the wrapped multi-line block is positioned as a unit at
            // the bottom of the row (`functional_spec.md §1.3` — "positioned as a unit").
            "cell_wrap_valign_bottom",
            Scene::new()
                .input(1, 1, "wrapped block aligned bottom")
                .col_width(1, 80.0)
                .row_height(1, 64.0)
                .wrap(1, 1)
                .v_align(1, 1, VAlign::Bottom),
        ),
        cell(
            "cell_wide_column",
            Scene::new().input(1, 1, "Wide column").col_width(1, 220.0),
        ),
        cell(
            "cell_narrow_column_clipped_number",
            Scene::new().input(1, 1, "123456789").col_width(1, 40.0),
        ),
        // ---- Auto-grow rows (Phase 7, `functional_spec.md §3`) ---------------------------
        // Each opt-in case runs the REAL render-thread wrap measurement (`autogrow_measure_now`),
        // so the captured row height is what the product computes — not a hand-injected number.
        // A wrap-on cell with no manual row height grows to fit all its wrapped lines. The viewport
        // is tall enough to show the whole grown row (so a clipped last line would be a real defect).
        RenderCase::new(
            "autogrow_wrap_grows",
            Scene::new()
                .input(1, 1, AUTO_GROW_TEXT)
                .col_width(1, 96.0)
                .wrap(1, 1),
            (480, 220),
        )
        .auto_grow(),
        // Narrowing the column produces more wrapped lines → the row grows taller still.
        RenderCase::new(
            "autogrow_narrow_col_more_lines",
            Scene::new()
                .input(1, 1, AUTO_GROW_TEXT)
                .col_width(1, 64.0)
                .wrap(1, 1),
            (480, 280),
        )
        .auto_grow(),
        // Widening the column produces fewer lines → the row shrinks back toward the default.
        cell(
            "autogrow_wide_col_fewer_lines",
            Scene::new()
                .input(1, 1, AUTO_GROW_TEXT)
                .col_width(1, 220.0)
                .wrap(1, 1),
        )
        .auto_grow(),
        // A MANUAL row (its height was set — here an injected custom height, the file-loaded /
        // user-resized case) is NOT auto-grown: it stays put and clips the overflowing wrapped
        // text, even though auto-grow runs (`functional_spec.md §3.3`).
        cell(
            "autogrow_manual_row_unchanged",
            Scene::new()
                .input(1, 1, AUTO_GROW_TEXT)
                .col_width(1, 96.0)
                .row_height(1, 30.0)
                .wrap(1, 1),
        )
        .auto_grow(),
        // A pathologically long wrapped cell is capped at `MAX_AUTO_ROW_HEIGHT_PX` (~10 lines);
        // content beyond the cap clips within the cell (a bigger viewport shows the whole capped row).
        RenderCase::new(
            "autogrow_cap_clip",
            Scene::new()
                .input(1, 1, AUTO_GROW_CAP_TEXT)
                .col_width(1, 70.0)
                .wrap(1, 1),
            (440, 300),
        )
        .auto_grow(),
        // ---- Text spill / overflow (Phase 3, `functional_spec.md §2`) -------------------
        // Long general/left text spills RIGHT over the empty neighbours (B2 → C2, D2 …), reading
        // as one continuous run crossing the gridlines (the must-have case).
        cell(
            "spill_right_over_empties",
            Scene::new().input(1, 1, "This long label spills to the right"),
        ),
        // Right-aligned long text spills LEFT: the origin (D2) is anchored at its right edge and the
        // text flows back over the empty C2/B2/A2.
        cell(
            "spill_left_right_aligned",
            Scene::new()
                .input(1, 3, "spilling to the left here")
                .align(1, 3, Align::Right),
        ),
        // Center-aligned long text spills BOTH ways, centred over the empty run; blockers at A2/E2
        // bound each side independently (the text spans B2–D2, centred over C2).
        cell(
            "spill_center_both",
            Scene::new()
                .input(1, 0, "L")
                .input(1, 4, "R")
                .input(1, 2, "centered spill across")
                .align(1, 2, Align::Center),
        ),
        // Spill STOPS at the first cell with content: B2's long text spills over the empty C2 but
        // clips at D2 ("STOP"). Content — not fill/border — is what stops a spill.
        cell(
            "spill_stop_at_nonempty",
            Scene::new()
                .input(1, 1, "long text stops at the next value")
                .input(1, 3, "STOP"),
        ),
        // A fill-only (content-less) neighbour does NOT stop the spill: B2's text spills over the
        // yellow-filled C2 (the fill still paints; the text sits on top).
        cell(
            "spill_over_fill_only_neighbor",
            Scene::new()
                .input(1, 1, "spills over a filled empty cell")
                .fill(1, 2, 0xFFEB3B),
        ),
        // Wrap-on cells never spill (mutually exclusive with §3): the long text wraps within the
        // column and clips to the row height — no overflow into C2.
        cell(
            "spill_wrap_on_no_spill",
            Scene::new()
                .input(1, 1, "wrapped long text does not spill it wraps instead")
                .col_width(1, 80.0)
                .row_height(1, 48.0)
                .wrap(1, 1),
        ),
        // Numbers never spill — a too-wide number clips as today (the `#####` indicator is out of
        // scope), leaving the empty neighbour blank.
        cell(
            "spill_number_no_spill",
            Scene::new().input(1, 1, "123456789").col_width(1, 50.0),
        ),
        // Spill is bounded by the covered region: publishing only cols 0..3 makes cols ≥3 uncovered,
        // so B2's long text spills over the covered-empty C2 but STOPS at the coverage edge — it
        // never treats the uncovered (blank-rendered) cells to the right as reliably empty (§2.5).
        cell(
            "spill_stop_at_coverage_edge",
            Scene::new()
                .input(1, 1, "long text bounded by the coverage edge")
                .publish(0..80, 0..3),
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
        // ---- In-grid charts (P8): the ChartLayer painted over cells at the anchor rect --------
        // A Faithful line chart floating over the data table (`charts/functional_spec.md §1`).
        RenderCase::new("grid_chart_line", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_chart_spec("Regional Sales", "<c:lineChart/>")]),
        // A Faithful clustered-COLUMN chart floating over the same backing table (P22) — the in-grid
        // proof of the ChartLayer → `bar_element` path. Its own baseline, so no existing
        // `grid_chart_*` baseline moves.
        RenderCase::new("grid_chart_column", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_column_spec("Regional Sales")]),
        // A Faithful standard-AREA chart floating over the same backing table (P23) — the in-grid
        // proof of the ChartLayer → `area_element` path. Its own baseline, so no existing
        // `grid_chart_*` baseline moves.
        RenderCase::new("grid_chart_area", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_area_spec("Regional Sales")]),
        // A Faithful PIE chart floating over the same backing table (P24) — the in-grid proof of the
        // ChartLayer → `pie_element` path. Its own baseline, so no existing `grid_chart_*` baseline
        // moves.
        RenderCase::new("grid_chart_pie", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_pie_spec("Regional Share")]),
        // A Faithful marker-SCATTER chart floating over the same backing table (P25) — the in-grid
        // proof of the ChartLayer → `scatter_element` path. Its own baseline, so no existing
        // `grid_chart_*` baseline moves.
        RenderCase::new("grid_chart_scatter", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_scatter_spec("Measurements")]),
        // In-grid bubble (P26): a loaded area-encoded bubble over the backing table — the in-grid
        // proof of the ChartLayer → `bubble_element` path. Its own baseline, so no existing
        // `grid_chart_*` baseline moves.
        RenderCase::new("grid_chart_bubble", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![in_grid_bubble_spec("Segments")]),
        // A Degraded chart still renders as a line, plus the corner "⚠ May not display as intended"
        // badge (`ui_design.md §2.2`) — here from a 3-D group (`line3DChart`) rendered as its 2-D
        // line. (A shown `c:dLbls` on a line is Faithful as of P12 — it renders — so the badge case
        // uses the 3-D→2-D reduction, which keeps the same rendered line + badge, hence the baseline
        // is unchanged.)
        RenderCase::new(
            "grid_chart_degraded_badge",
            chart_backing_scene(),
            CHART_GRID_VP,
        )
        .charts(vec![in_grid_chart_spec(
            "Regional Sales",
            "<c:line3DChart/>",
        )]),
        // An Unsupported group draws the placeholder box (title + "Unsupported chart type"),
        // occupying the chart's space (`ui_design.md §2.3`).
        RenderCase::new(
            "grid_chart_unsupported_placeholder",
            chart_backing_scene(),
            CHART_GRID_VP,
        )
        .charts(vec![in_grid_unsupported_spec("Surface Data")]),
        // The same Faithful chart scrolled deep so its top-left is clipped at the content edge —
        // proves the anchor tracks scroll and the layer clips to the viewport (`architecture.md §5`).
        RenderCase::new(
            "grid_chart_scrolled_clipped",
            chart_backing_scene(),
            CHART_GRID_VP,
        )
        .charts(vec![in_grid_chart_spec("Regional Sales", "<c:lineChart/>")])
        .reveal(22, 8),
        // A SELECTED chart (P18): the same Faithful line chart with the selection outline + eight
        // resize handles drawn over its rect (`ui_design.md §3.2`). New ChartLayer chrome — its own
        // baseline, so no existing `grid_chart_*` baseline moves.
        RenderCase::new("grid_chart_selected", chart_backing_scene(), CHART_GRID_VP)
            .charts(vec![
                in_grid_chart_spec("Regional Sales", "<c:lineChart/>").with_id(ChartId(1))
            ])
            .selected_chart(ChartId(1)),
        // The near-empty AUTHORED chart the insert flow produces (P17/P21): a single placeholder
        // series titled "Chart" over the backing table — the exact picture shown the instant a line
        // chart is inserted. The only in-grid chart case built from an **authored** spec (all others
        // are loaded), so it is the pixel proof of the authored → in-grid render path + the insert
        // visual (`charts/functional_spec.md §6.A`, `ui_design.md §3.1`). Its own baseline, so no
        // existing `grid_chart_*` baseline moves.
        RenderCase::new(
            "grid_chart_authored_inserted",
            chart_backing_scene(),
            CHART_GRID_VP,
        )
        .charts(vec![in_grid_authored_inserted_spec()]),
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
            // `functional_spec.md §1.3`). The editor grows to fit its own text (here just wide enough
            // for `=SUM(A1:A3)`), painted above the cells (`DECISIONS_TO_REVIEW.md`).
            "incell_editor_open",
            Scene::new().input(1, 1, "42"),
            CELL_VP,
        )
        .selection(sel((1, 1), (1, 1)))
        .in_cell(1, 1, "=SUM(A1:A3)"),
        RenderCase::new(
            // A wrap-off in-cell editor GROWS RIGHTWARD over its neighbours to fit a long string in a
            // narrow column, painted above the cells and clamped at the content viewport's right edge
            // (`DECISIONS_TO_REVIEW.md`).
            "incell_editor_grow_right",
            Scene::new().input(1, 1, "x").col_width(1, 56.0),
            CELL_VP,
        )
        .selection(sel((1, 1), (1, 1)))
        .in_cell(
            1,
            1,
            "A really long label that grows the editor rightward over its neighbours",
        ),
        RenderCase::new(
            // A wrap-ON in-cell editor GROWS DOWNWARD (taller) instead of rightward, previewing the
            // committed multi-line footprint; its hosted single-line input stays a first-line control
            // at the top (`DECISIONS_TO_REVIEW.md`).
            "incell_editor_grow_wrap",
            Scene::new().input(1, 1, "x").col_width(1, 96.0).wrap(1, 1),
            (480, 220),
        )
        .selection(sel((1, 1), (1, 1)))
        .in_cell(1, 1, "wrap this text onto several visual lines while editing in place"),
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
        // Auto-grow regression (`functional_spec.md §3.1`): the pre-existing font-size auto-grow is
        // RETAINED — a 24 pt row is grown to fit (injected height, like `font_size_24_row_grown`),
        // and the new wrap measurement pass (opt-in here) leaves this non-wrap row untouched.
        cell(
            "autogrow_large_font_grows",
            at(Scene::new())
                .font(1, 1, None, Some(24.0))
                .row_height(1, 38.0),
        )
        .auto_grow(),
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
        // ---- Border line patterns (Phase 2): dashed + double edge paint -------------------
        cell(
            // All four edges dashed (medium, 2px) — exercises the dashed run on both a vertical
            // (left/right) and a horizontal (top/bottom) edge.
            "border_dashed_all",
            at(Scene::new()).border(1, 1, all_edges_pat(2, LinePattern::Dashed)),
        ),
        cell(
            // All four edges double (3px, two thin parallel strips) on both axes.
            "border_double_all",
            at(Scene::new()).border(1, 1, all_edges_pat(3, LinePattern::Double)),
        ),
        cell(
            // Pen model (Phase 3): "select Outer with a dashed + red pen" — a 2×2 block whose four
            // corner cells carry only their two OUTER edges as dashed red (2px); the interior edges
            // stay bare. This is the resolved `BorderSpec` a `SetBorders { preset: Outer, line:
            // Dashed, color: red }` produces, so it covers a pen-applied border (dashed + a
            // non-default color, on the perimeter only) end-to-end at the render layer — chrome
            // (the popover itself) has no pixel coverage in this harness.
            "border_pen_outer_dashed_red",
            {
                // A dashed red (2px) edge — the pen the worked example paints (`functional_spec.md
                // §2.1`). Each corner cell of the 2×2 block gets only its two perimeter edges.
                let dr = || {
                    Some(Edge::with_pattern(
                        2,
                        Rgb::new(0xFF, 0, 0),
                        LinePattern::Dashed,
                    ))
                };
                at(Scene::new())
                    .border(
                        1,
                        1,
                        BorderSpec {
                            top: dr(),
                            left: dr(),
                            ..BorderSpec::NONE
                        },
                    )
                    .border(
                        1,
                        2,
                        BorderSpec {
                            top: dr(),
                            right: dr(),
                            ..BorderSpec::NONE
                        },
                    )
                    .border(
                        2,
                        1,
                        BorderSpec {
                            bottom: dr(),
                            left: dr(),
                            ..BorderSpec::NONE
                        },
                    )
                    .border(
                        2,
                        2,
                        BorderSpec {
                            bottom: dr(),
                            right: dr(),
                            ..BorderSpec::NONE
                        },
                    )
            },
        ),
        cell(
            // One cell mixing all three patterns + weights so solid, dashed, and double read
            // side by side (and confirm the solid path is unchanged next to the new patterns):
            // solid-thin top, dashed-medium right, double bottom, solid-thick left.
            "border_pattern_mixed",
            at(Scene::new()).border(
                1,
                1,
                BorderSpec {
                    top: edge(1),
                    right: edge_pat(2, LinePattern::Dashed),
                    bottom: edge_pat(3, LinePattern::Double),
                    left: edge(3),
                },
            ),
        ),
        // ---- Structure (Phase 7): resized geometry + header selection -------------------
        cell(
            // A number in a column narrowed to 20 px — the value clips (resize geometry honored
            // end-to-end through the cache, `components/grid_structure.md §5.1`).
            "col_resized_narrow_clips_text",
            Scene::new().input(1, 1, "1234567").col_width(1, 20.0),
        ),
        cell(
            // A row grown to 48 px — the tall row's geometry reflows the grid below it.
            "row_resized_tall",
            at(Scene::new()).row_height(1, 48.0),
        ),
        RenderCase::new(
            // A full-column header selection (`functional_spec.md §5.2`): the whole column is tinted
            // and its header selected. The overlay is viewport-clamped (the range spans all rows).
            "header_full_column_selected",
            Scene::new().input(0, 1, "B1").input(2, 1, "B3"),
            GRID_VP,
        )
        .selection(sel((0, 1), (freecell_core::limits::MAX_ROWS - 1, 1))),
        RenderCase::new(
            // A full-row header selection: the whole row is tinted, its header selected.
            "header_full_row_selected",
            Scene::new().input(2, 0, "A3").input(2, 2, "C3"),
            GRID_VP,
        )
        .selection(sel((2, 0), (2, freecell_core::limits::MAX_COLS - 1))),
        // ---- Chrome / formatting (Phase 8) ---------------------------------------------
        cell(
            // An explicit RED font colour on a cell (`architecture.md §1.2` precedence: an
            // explicit `font.color` wins). The §9 `format_red_negative` companion — a colour
            // produced by a `[Red]` number format — has NO render case (the Scene builder can't
            // set a custom `num_fmt`; it is guarded by the engine test
            // `published_style_resolves_format_and_explicit_colors`, see DECISIONS §8).
            "text_color_red",
            at(Scene::new()).font_color(1, 1, 0xFF0000),
        ),
        RenderCase::new(
            // The macOS custom titlebar row (§7.1) over a short grid. It is just a div, so it
            // renders in the Linux harness (this case); the *native* macOS integration
            // (transparent titlebar, repositioned traffic lights, drag/zoom/fullscreen) is the
            // on-device smoke gate, not pixel-baselined here.
            "titlebar_row",
            Scene::new().input(0, 0, "A1"),
            (480, 120),
        )
        .titlebar("Budget.xlsx — Edited"),
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
