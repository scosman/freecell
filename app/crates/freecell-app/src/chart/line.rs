//! The FreeCell-owned **multi-series line** widget — Gate 1, the make-or-break example
//! (functional_spec §3, §7). Built over gpui-component's raw `Line` primitive rather than the
//! stock `LineChart` struct, because the struct is single-line only and, critically, each
//! `LineChart` normalizes its **own** y-domain, so overlaying several does not share a value
//! scale (`research/compare-line.md`).
//!
//! The pieces this proves FreeCell can build:
//! - **N lines over ONE shared value scale.** All series measure against a single
//!   [`NiceScale`] computed over the union of every series' values, so their heights are
//!   directly comparable (the exact thing overlaid `LineChart`s cannot do).
//! - **Straight or smooth segments** — Excel's straight ([`StrokeStyle::Linear`]) default, or the
//!   curved `Natural` line when `c:smooth` is set (P6).
//! - a **numeric value axis** with readable "nice" tick labels + gridlines (our
//!   [`NiceScale`]; the linear scale ships no tick generator), formatted through the axis'
//!   `numFmt` when it carries one (P6);
//! - a **category axis** (via [`ScalePoint`], which — unlike `ScaleBand` — honors its range
//!   start, so no gutter fix-up is needed);
//! - the **multi-series color cycle** (resolving explicit sRGB / theme / palette colors, P6),
//!   matched swatch-for-swatch by the legend the surrounding [`super::chrome`] frame draws;
//! - **per-point markers** (`c:marker` shapes, P6) painted at each data point.

use gpui::{
    point, px, size, Background, BorderStyle, Bounds, Hsla, IntoElement, PathBuilder, Pixels,
    Point, SharedString, TextAlign, Window,
};
use gpui_component::plot::{
    label::{measure_text_width, Text},
    origin_point,
    scale::{Scale, ScaleLinear, ScalePoint},
    shape::Line,
    AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, PlotLabel, StrokeStyle, AXIS_GAP,
};

use freecell_chart_model::{
    apply_number_format, downsample_for_paint, Chart, ChartKind, DataLabelPosition, DataLabels,
    Marker, MarkerSymbol, SeriesData, MAX_PAINT_VERTICES,
};

use super::chrome::chart_frame;
use super::style::{
    hsla, resolve_series_hsla, with_alpha, AXIS_STROKE, AXIS_TITLE_TEXT, GRID_STROKE, MUTED_TEXT,
};
use super::ticks::{format_tick, NiceScale};

/// Pixels reserved at the left of the plot for value-axis tick labels.
const VALUE_AXIS_GUTTER: f32 = 46.0;
/// Pixels reserved at the top so the highest point/dot isn't clipped.
const PLOT_TOP_GAP: f32 = 14.0;
/// Pixels reserved at the right so the last point/label isn't clipped.
const PLOT_RIGHT_GAP: f32 = 16.0;
/// Inset of the first/last category point from the plot edges, so end dots and their centered
/// labels have room to breathe instead of sitting on the axis line / frame edge.
const POINT_INSET: f32 = 10.0;
/// Roughly how many value-axis ticks to aim for.
const TARGET_TICKS: usize = 5;
/// Default series line weight in **points** when a series carries no `a:ln@w` — Excel's default
/// line-series weight (`a:ln w="28440"` ≈ 2.25pt), visibly heavier than a gridline (P13, obs. B).
const DEFAULT_LINE_WIDTH_PT: f32 = 2.25;
/// Points → device px for line weights at the chart render scale. Chosen so the Excel-default
/// 2.25pt reads as a clearly heavy ~2.7px stroke against the thin gridlines.
const PT_TO_PX: f32 = 1.2;
/// Clamp range for a resolved line width in px (a hostile `a:ln@w` can't produce a hairline or a slab).
const MIN_LINE_PX: f32 = 1.0;
const MAX_LINE_PX: f32 = 8.0;
/// Default marker diameter — also the round dot drawn when a series specifies no marker.
const DOT_SIZE: f32 = 6.0;
/// Stroke width for the open-shape markers (plus / x / dash).
const MARKER_STROKE_WIDTH: f32 = 1.6;
/// The `dot` marker's radius as a fraction of the full marker radius — Excel's `dot` is a small
/// filled dot, noticeably smaller than the `circle`/default marker.
const DOT_MARKER_SCALE: f32 = 0.55;
/// Data-label font size (matches the plot label primitive's default).
const LABEL_FONT_SIZE: f32 = 10.0;
/// Gap between a data label and its point (marker edge).
const LABEL_GAP: f32 = 4.0;
/// Legend-key swatch side length inside a data label.
const LABEL_SWATCH: f32 = 8.0;
/// Gap between the legend-key swatch and the label text.
const LABEL_SWATCH_GAP: f32 = 3.0;

/// One line: its per-category values, its resolved color (the series' `a:ln`/fill/theme/palette
/// color with any `a:ln` alpha applied, P13), its resolved stroke **width** in px (from `a:ln@w`,
/// else the Excel-like default, P13), its marker (`c:marker`, P6), its data labels (`c:dLbls`, P12),
/// and its name (for the series-name label part).
#[derive(Clone)]
struct LineSeries {
    values: Vec<f64>,
    color: Hsla,
    width_px: f32,
    marker: Option<Marker>,
    data_labels: Option<DataLabels>,
    name: Option<String>,
}

/// Resolve a series' `a:ln@w` (in points) to a clamped device-px stroke width, falling back to the
/// Excel-like default weight when the series carries no explicit width (P13).
fn line_width_px(width_pt: Option<f32>) -> f32 {
    (width_pt.unwrap_or(DEFAULT_LINE_WIDTH_PT) * PT_TO_PX).clamp(MIN_LINE_PX, MAX_LINE_PX)
}

/// A multi-series line plot over the raw `Line` primitive with ONE shared value scale
/// (the `scale` field) covering every series.
#[derive(IntoPlot)]
pub struct LinePlot {
    categories: Vec<SharedString>,
    series: Vec<LineSeries>,
    /// The single value domain shared by every series (the union of all their values, nice-d, then
    /// clamped to any explicit `c:scaling` min/max, P13).
    scale: NiceScale,
    /// Whether to draw curved (`c:smooth`) rather than straight segments.
    smooth: bool,
    /// The value-axis `c:numFmt` format code, applied to tick labels when present.
    value_format: Option<String>,
    /// Value axis reversed (`c:scaling/c:orientation val="maxMin"`) — max at the bottom (P13).
    value_reversed: bool,
    /// Category axis reversed — categories run right→left (P13).
    category_reversed: bool,
    /// Draw the value-axis major gridlines (`c:majorGridlines`; the line renderer's gridlines are
    /// horizontal, off the value axis). `false` suppresses them (P13).
    major_gridlines: bool,
}

impl LinePlot {
    /// Build the plot from a [`ChartKind::Line`] chart. Categories come from the first series;
    /// every series contributes to the shared value domain. Returns `None` for a non-line
    /// chart or one with no category/value data.
    pub fn multi_series(chart: &Chart) -> Option<Self> {
        let ChartKind::Line { smooth, .. } = chart.kind else {
            return None;
        };

        let mut categories: Option<Vec<SharedString>> = None;
        let mut series = Vec::new();
        for (i, s) in chart.series.iter().enumerate() {
            let SeriesData::CategoryValue {
                categories: cats,
                values,
            } = &s.data
            else {
                continue;
            };
            if categories.is_none() {
                categories = Some(cats.iter().map(|c| c.label().into()).collect());
            }
            // The line color prefers the `a:ln` stroke color, then the series fill/theme color, then
            // the palette cycle; any `a:ln` alpha is applied to it. The width comes from `a:ln@w`.
            let stroke = s.stroke;
            let base = resolve_series_hsla(stroke.and_then(|st| st.color).or(s.color), i);
            let color = match stroke.and_then(|st| st.alpha) {
                Some(alpha) => with_alpha(base, alpha),
                None => base,
            };
            series.push(LineSeries {
                values: values.clone(),
                color,
                width_px: line_width_px(stroke.and_then(|st| st.width_pt)),
                marker: s.marker,
                data_labels: s.data_labels.clone(),
                name: s.name.clone(),
            });
        }

        let categories = categories?;
        if series.is_empty() {
            return None;
        }

        // The SHARED value domain: nice-d over the union of EVERY series' values (so all lines are
        // drawn against one scale and their heights compare directly), then clamped to any explicit
        // `c:scaling` min/max the value axis carries (P13).
        let scale = NiceScale::spanning(
            series.iter().flat_map(|s| s.values.iter().copied()),
            TARGET_TICKS,
        )
        .bounded(chart.val_axis.min, chart.val_axis.max, TARGET_TICKS);

        Some(Self {
            categories,
            series,
            scale,
            smooth,
            value_format: chart.val_axis.number_format.clone(),
            value_reversed: chart.val_axis.reversed,
            category_reversed: chart.cat_axis.reversed,
            major_gridlines: chart.val_axis.major_gridlines,
        })
    }

    /// The stroke style for the current `smooth` flag: curved (`Natural`) when smooth, else the
    /// straight `Linear` segments Excel draws by default.
    fn stroke_style(&self) -> StrokeStyle {
        if self.smooth {
            StrokeStyle::Natural
        } else {
            StrokeStyle::Linear
        }
    }

    /// A value-axis tick label, formatted through the axis `numFmt` when it carries one.
    fn tick_label(&self, tick: f64) -> String {
        match &self.value_format {
            Some(code) => apply_number_format(code, tick),
            None => format_tick(tick),
        }
    }

    /// The shared value domain (exposed for tests: it must cover every series' values).
    #[cfg(test)]
    fn shared_scale(&self) -> NiceScale {
        self.scale
    }
}

/// Paint one series' marker at `center` (absolute window coordinates). The default (`marker ==
/// None`) is the P5 round dot — a filled circle at [`DOT_SIZE`] with a white edge — so a series
/// that specifies no marker looks exactly as it did before P6. Filled shapes
/// (circle/square/diamond/triangle/star/dot/auto) are painted as a filled path or quad; the open
/// shapes (plus/x/dash) as a stroked path in the series color; `none` paints nothing.
fn paint_marker(window: &mut Window, center: Point<Pixels>, marker: Option<Marker>, color: Hsla) {
    // A series with no `c:marker` defaults to the round dot (the P5 default), so absence resolves to
    // `Circle`; an explicit `none` paints nothing (handled in the match, no early return).
    let symbol = marker.map(|m| m.symbol).unwrap_or(MarkerSymbol::Circle);
    let diameter = marker.and_then(|m| m.size).unwrap_or(DOT_SIZE);
    let r = px(diameter / 2.0);
    let edge = hsla(0xFFFFFF);
    let cx = center.x;
    let cy = center.y;

    // A white-edged filled disc of `radius`, centered — the circle/dot marker primitive.
    let disc = |window: &mut Window, radius: Pixels, border: Pixels| {
        let top_left = point(cx - radius, cy - radius);
        window.paint_quad(gpui::quad(
            gpui::bounds(top_left, size(radius * 2.0, radius * 2.0)),
            radius,
            color,
            border,
            edge,
            BorderStyle::default(),
        ));
    };

    // A closed filled polygon through `pts` (absolute coordinates), edged in white.
    let filled_polygon = |window: &mut Window, pts: &[Point<Pixels>]| {
        let mut b = PathBuilder::fill();
        b.move_to(pts[0]);
        for p in &pts[1..] {
            b.line_to(*p);
        }
        b.close();
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    // A stroked segment set (each pair is a move+line) in the series color.
    let stroked = |window: &mut Window, segments: &[(Point<Pixels>, Point<Pixels>)]| {
        let mut b = PathBuilder::stroke(px(MARKER_STROKE_WIDTH));
        for (from, to) in segments {
            b.move_to(*from);
            b.line_to(*to);
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };

    match symbol {
        // Reachable: an explicit `<c:symbol val="none"/>` (the default-marker case resolves to
        // `Circle` above, so this is only hit for an authored/parsed `none`).
        MarkerSymbol::None => {}
        // Circle / auto: a white-edged filled disc at the full radius (the P5 default dot).
        MarkerSymbol::Circle | MarkerSymbol::Auto => disc(window, r, px(1.0)),
        // Dot: a smaller, unbordered filled dot (Excel's `dot` is noticeably smaller).
        MarkerSymbol::Dot => disc(window, r * DOT_MARKER_SCALE, px(0.0)),
        MarkerSymbol::Square => {
            let top_left = point(cx - r, cy - r);
            window.paint_quad(gpui::quad(
                gpui::bounds(top_left, size(r * 2.0, r * 2.0)),
                px(0.0),
                color,
                px(1.0),
                edge,
                BorderStyle::default(),
            ));
        }
        MarkerSymbol::Diamond => filled_polygon(
            window,
            &[
                point(cx, cy - r),
                point(cx + r, cy),
                point(cx, cy + r),
                point(cx - r, cy),
            ],
        ),
        MarkerSymbol::Triangle => filled_polygon(
            window,
            &[
                point(cx, cy - r),
                point(cx + r, cy + r),
                point(cx - r, cy + r),
            ],
        ),
        MarkerSymbol::Star => filled_polygon(window, &star_points(cx, cy, r)),
        MarkerSymbol::Plus => stroked(
            window,
            &[
                (point(cx, cy - r), point(cx, cy + r)),
                (point(cx - r, cy), point(cx + r, cy)),
            ],
        ),
        MarkerSymbol::X => stroked(
            window,
            &[
                (point(cx - r, cy - r), point(cx + r, cy + r)),
                (point(cx - r, cy + r), point(cx + r, cy - r)),
            ],
        ),
        MarkerSymbol::Dash => stroked(window, &[(point(cx - r, cy), point(cx + r, cy))]),
    }
}

/// The ten vertices (alternating outer/inner radius) of a five-pointed star centered at
/// `(cx, cy)`, outer radius `r`.
fn star_points(cx: Pixels, cy: Pixels, r: Pixels) -> Vec<Point<Pixels>> {
    let outer = r.as_f32();
    let inner = outer * 0.4;
    (0..10)
        .map(|i| {
            let radius = if i % 2 == 0 { outer } else { inner };
            // Start at the top point (−90°) and step 36° per vertex.
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            point(cx + px(radius * angle.cos()), cy + px(radius * angle.sin()))
        })
        .collect()
}

impl Plot for LinePlot {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();

        let plot_left = VALUE_AXIS_GUTTER;
        let plot_right = (w - PLOT_RIGHT_GAP).max(plot_left + 1.0);
        let plot_top = PLOT_TOP_GAP;
        let plot_bottom = (h - AXIS_GAP).max(plot_top + 1.0);

        // Category axis: evenly spaced points, first/last inset from the plot edges. `ScalePoint`
        // (unlike `ScaleBand`) honors its range start, so we can hand it the true pixel range.
        let point_scale = ScalePoint::new(
            self.categories.clone(),
            vec![plot_left + POINT_INSET, plot_right - POINT_INSET],
        );
        // Precompute each category's x pixel once and share it across every series. A reversed
        // category axis mirrors each x about the center of the point range, so categories run
        // right→left (labels + line follow the same mirrored xs, so they stay in sync) (P13).
        let (inset_left, inset_right) = (plot_left + POINT_INSET, plot_right - POINT_INSET);
        let xs: Vec<f32> = self
            .categories
            .iter()
            .map(|c| {
                let x = point_scale.tick(c).unwrap_or(plot_left);
                if self.category_reversed {
                    inset_left + inset_right - x
                } else {
                    x
                }
            })
            .collect();

        // Value axis: the ONE shared nice domain -> pixel range. Normally inverted (min at the
        // bottom); a reversed value axis (`c:orientation maxMin`) puts max at the bottom (P13).
        let value_range = if self.value_reversed {
            vec![plot_top, plot_bottom]
        } else {
            vec![plot_bottom, plot_top]
        };
        let value_scale = ScaleLinear::new(vec![self.scale.min, self.scale.max], value_range);
        let ticks = self.scale.ticks();

        // Gridlines at each nice tick (horizontal, for the value axis) — only when the value axis
        // carries `c:majorGridlines` (P13; Excel's default line chart does).
        if self.major_gridlines {
            let grid_ys: Vec<f32> = ticks.iter().filter_map(|t| value_scale.tick(t)).collect();
            Grid::new()
                .stroke(hsla(GRID_STROKE))
                .dash_array(&[px(4.), px(2.)])
                .y(grid_ys)
                .paint(&bounds, window);
        }

        // Axes + labels: value labels left of the value axis (formatted through the axis numFmt
        // when present), category labels below the baseline.
        let value_labels = ticks.iter().filter_map(|t| {
            value_scale.tick(t).map(|y| {
                AxisText::new(self.tick_label(*t), px(y), hsla(MUTED_TEXT)).align(TextAlign::Right)
            })
        });
        let cat_labels = self.categories.iter().enumerate().map(|(i, c)| {
            AxisText::new(c.clone(), px(xs[i]), hsla(MUTED_TEXT)).align(TextAlign::Center)
        });
        PlotAxis::new()
            .x(px(plot_bottom))
            .x_label(cat_labels)
            .y(px(plot_left))
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(hsla(AXIS_STROKE))
            .paint(&bounds, window, cx);

        // One `Line` per series, all sharing `xs` (category positions) and `value_scale` (the
        // shared value domain). Straight or smooth segments per `c:smooth`, then the series' `c:marker`
        // painted at each point. Per series (line then its markers) so ordering matches the primitive.
        let stroke_style = self.stroke_style();
        for s in &self.series {
            let n = s.values.len().min(xs.len());
            // Large-series down-sample FOR PAINT ONLY (architecture §5 challenge 5): decimate a huge
            // series to <= MAX_PAINT_VERTICES shape-preserving vertices so the per-frame stroke +
            // marker cost is bounded. The retained model/source is untouched (save keeps every
            // point). A series within budget returns identity, so ordinary charts (every committed
            // scene is <= ~12 points) paint byte-identically — no baseline moves.
            let keep = downsample_for_paint(&s.values[..n], MAX_PAINT_VERTICES);

            let xs_for_line = xs.clone();
            let values = s.values.clone();
            let scale_for_line = value_scale.clone();
            let stroke: Background = s.color.into();
            let keep_x = keep.clone();
            let keep_y = keep.clone();

            Line::new()
                .data((0..keep.len()).collect::<Vec<usize>>())
                .x(move |j: &usize| Some(xs_for_line[keep_x[*j]]))
                // Drop a non-finite value (NaN/Inf) rather than emit a bad point: the primitive
                // omits a `None` point, connecting its finite neighbors with a straight segment
                // (never panics). So an interior non-numeric cell bridges across, and a
                // leading/trailing run leaves a visible break — "render what's valid, blank the
                // rest" (functional_spec §7). A true per-cell break is a future-fidelity item.
                .y(move |j: &usize| {
                    let v = values[keep_y[*j]];
                    v.is_finite().then(|| scale_for_line.tick(&v)).flatten()
                })
                .stroke(stroke)
                .stroke_width(px(s.width_px))
                .stroke_style(stroke_style)
                .paint(&bounds, window);

            // Markers at each kept finite point (in absolute coordinates, like the primitive's dots).
            // Iterating `keep` matches the vertices the line drew (identity keep == every point).
            for &i in &keep {
                let (x, v) = (xs[i], s.values[i]);
                if let Some(y) = v.is_finite().then(|| value_scale.tick(&v)).flatten() {
                    let center = origin_point(px(x), px(y), bounds.origin);
                    paint_marker(window, center, s.marker, s.color);
                }
            }
        }

        // Data labels (P12), drawn last so they sit above the lines/markers.
        self.paint_data_labels(&bounds, &xs, &value_scale, window, cx);
    }
}

impl LinePlot {
    /// Paint each series' data labels (`c:dLbls`, P12) at their points. For every series whose
    /// labels are shown, at every finite point: compose the label text (value / percent / category
    /// / series name — the value + percent formatted through the numFmt applier), place it per the
    /// `c:dLblPos` position (default **above** for a line), and — when the legend key is shown —
    /// draw a small series-color swatch to its left. Text is painted via the plot's [`PlotLabel`]
    /// primitive; swatches via quads (both share the plot's coordinate origin).
    fn paint_data_labels(
        &self,
        bounds: &Bounds<Pixels>,
        xs: &[f32],
        value_scale: &ScaleLinear<f64>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let color = hsla(AXIS_TITLE_TEXT);
        let mut texts: Vec<Text> = Vec::new();

        for s in &self.series {
            let Some(dl) = &s.data_labels else { continue };
            if !dl.is_shown() {
                continue;
            }
            // Percent (`c:showPercent`) is a point's share of the series total (finite points only).
            let finite_total: f64 = s.values.iter().copied().filter(|v| v.is_finite()).sum();
            let position = dl.position.unwrap_or(DataLabelPosition::Above);

            for (i, &v) in s.values.iter().enumerate() {
                if i >= xs.len() || !v.is_finite() {
                    continue;
                }
                let Some(y) = value_scale.tick(&v) else {
                    continue;
                };
                let category = self.categories.get(i).map(SharedString::as_ref);
                let percent = (finite_total != 0.0).then_some(v / finite_total);
                let text = dl.label_text(s.name.as_deref(), category, v, percent);
                if text.is_empty() && !dl.show_legend_key {
                    continue;
                }

                let text: SharedString = text.into();
                let text_width = if text.is_empty() {
                    0.0
                } else {
                    measure_text_width(&text, px(LABEL_FONT_SIZE), window)
                };
                let swatch = if dl.show_legend_key {
                    LABEL_SWATCH
                } else {
                    0.0
                };
                let inner_gap = if swatch > 0.0 && !text.is_empty() {
                    LABEL_SWATCH_GAP
                } else {
                    0.0
                };
                let block_width = swatch + inner_gap + text_width;

                // Anchor the (swatch + text) block relative to the point, per `dLblPos`. The block's
                // top-left is `(block_left, top)`, in plot-relative coordinates.
                let half_marker = DOT_SIZE / 2.0;
                let (block_left, top) = match position {
                    DataLabelPosition::Above => (
                        xs[i] - block_width / 2.0,
                        y - half_marker - LABEL_GAP - LABEL_FONT_SIZE,
                    ),
                    DataLabelPosition::Below => {
                        (xs[i] - block_width / 2.0, y + half_marker + LABEL_GAP)
                    }
                    DataLabelPosition::Center => {
                        (xs[i] - block_width / 2.0, y - LABEL_FONT_SIZE / 2.0)
                    }
                    DataLabelPosition::Right => {
                        (xs[i] + half_marker + LABEL_GAP, y - LABEL_FONT_SIZE / 2.0)
                    }
                    DataLabelPosition::Left => (
                        xs[i] - half_marker - LABEL_GAP - block_width,
                        y - LABEL_FONT_SIZE / 2.0,
                    ),
                };

                // Clamp the block into the plot bounds so a label near an edge — an `Above` label on
                // a near-max point, or a wide composed label at the last category — stays visible
                // instead of overflowing with nothing clipping it (latent for real P14 data). A label
                // already inside the bounds is unchanged; only an overflowing one is nudged in.
                let plot_w = bounds.size.width.as_f32();
                let plot_h = bounds.size.height.as_f32();
                let block_left = block_left.clamp(0.0, (plot_w - block_width).max(0.0));
                let top = top.clamp(0.0, (plot_h - LABEL_FONT_SIZE).max(0.0));

                if dl.show_legend_key {
                    // A filled series-color swatch at the block's left, vertically centered on the text.
                    let swatch_top = top + (LABEL_FONT_SIZE - LABEL_SWATCH) / 2.0;
                    let top_left = origin_point(px(block_left), px(swatch_top), bounds.origin);
                    window.paint_quad(gpui::quad(
                        gpui::bounds(top_left, size(px(LABEL_SWATCH), px(LABEL_SWATCH))),
                        px(1.0),
                        s.color,
                        px(0.0),
                        hsla(0xFFFFFF),
                        BorderStyle::default(),
                    ));
                }

                if !text.is_empty() {
                    let text_left = block_left + swatch + inner_gap;
                    texts.push(
                        Text::new(text, point(px(text_left), px(top)), color)
                            .font_size(px(LABEL_FONT_SIZE))
                            .align(TextAlign::Left),
                    );
                }
            }
        }

        if !texts.is_empty() {
            PlotLabel::new(texts).paint(bounds, window, cx);
        }
    }
}

/// Build the full multi-series line chart element (title, axis titles, plot, legend). Returns
/// `None` for a chart this widget can't render.
pub fn line_element(chart: &Chart) -> Option<gpui::AnyElement> {
    let plot = LinePlot::multi_series(chart)?;
    Some(chart_frame(chart, plot.into_any_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_chart_model::{Axis, Category, Color, Grouping, Legend, LineStroke, Series};

    fn q_categories() -> Vec<Category> {
        vec![
            Category::Text("Q1".into()),
            Category::Text("Q2".into()),
            Category::Text("Q3".into()),
        ]
    }

    fn three_series_line() -> Chart {
        Chart {
            title: Some("Sales".into()),
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![
                Series::category_value(Some("North"), q_categories(), vec![32.0, 55.0, 91.0]),
                Series::category_value(Some("South"), q_categories(), vec![74.0, 48.0, 63.0]),
                Series::category_value(Some("West"), q_categories(), vec![50.0, 49.0, 66.0]),
            ],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("Units"),
            legend: Some(Legend::default()),
        }
    }

    #[test]
    fn shared_scale_covers_all_series() {
        let chart = three_series_line();
        let plot = LinePlot::multi_series(&chart).expect("line plot");
        let scale = plot.shared_scale();
        // The single shared domain must contain EVERY value of EVERY series — the core
        // "one shared value scale across all series" property Gate 1 hinges on.
        for s in &chart.series {
            if let SeriesData::CategoryValue { values, .. } = &s.data {
                for &v in values {
                    assert!(
                        scale.min <= v && v <= scale.max,
                        "value {v} outside shared domain [{}, {}]",
                        scale.min,
                        scale.max
                    );
                }
            }
        }
        // Zoomed to the data (not forced to zero) — the min value is 32.
        assert!(
            scale.min > 0.0,
            "line axis should not force zero: {}",
            scale.min
        );
    }

    #[test]
    fn multi_series_reads_all_series_and_categories() {
        let chart = three_series_line();
        let plot = LinePlot::multi_series(&chart).expect("line plot");
        assert_eq!(plot.series.len(), 3, "all three series must be kept");
        assert_eq!(plot.categories.len(), 3, "all categories must be kept");
        // Each series carries its own resolved color (distinct palette entries).
        assert_ne!(plot.series[0].color, plot.series[1].color);
        assert_ne!(plot.series[1].color, plot.series[2].color);
    }

    #[test]
    fn single_series_line_builds() {
        let mut chart = three_series_line();
        chart.series.truncate(1);
        let plot = LinePlot::multi_series(&chart).expect("single-series line plot");
        assert_eq!(plot.series.len(), 1, "the one series must be kept");
        assert_eq!(plot.categories.len(), 3);
    }

    #[test]
    fn non_finite_values_do_not_break_the_scale() {
        // A series with a NaN and an +Inf among finite values still builds a plot; the shared
        // domain ignores the non-finite entries and stays finite, covering the finite values
        // (the paint path then blanks the non-finite points — no panic, no bad scale).
        let mut chart = three_series_line();
        chart.series.push(Series::category_value(
            Some("Broken"),
            q_categories(),
            vec![f64::NAN, 40.0, f64::INFINITY],
        ));
        let plot = LinePlot::multi_series(&chart).expect("plot despite non-finite values");
        let scale = plot.shared_scale();
        assert!(
            scale.min.is_finite() && scale.max.is_finite() && scale.max > scale.min,
            "shared scale must stay finite: [{}, {}]",
            scale.min,
            scale.max
        );
        // Every FINITE value across all series still fits the domain.
        for s in &chart.series {
            if let SeriesData::CategoryValue { values, .. } = &s.data {
                for &v in values.iter().filter(|v| v.is_finite()) {
                    assert!(
                        scale.min <= v && v <= scale.max,
                        "finite value {v} outside domain [{}, {}]",
                        scale.min,
                        scale.max
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_non_line_and_empty() {
        // A bar chart is not a line chart.
        let mut bar = three_series_line();
        bar.kind = ChartKind::Bar {
            dir: freecell_chart_model::BarDir::Col,
            grouping: Grouping::Clustered,
        };
        assert!(LinePlot::multi_series(&bar).is_none());

        // A line chart with no series has nothing to draw.
        let mut empty = three_series_line();
        empty.series.clear();
        assert!(LinePlot::multi_series(&empty).is_none());
    }

    #[test]
    fn smooth_flag_selects_stroke_style() {
        // A non-smooth line draws straight (`Linear`) segments; a smooth one curves (`Natural`).
        let straight = LinePlot::multi_series(&three_series_line()).expect("plot");
        assert!(matches!(straight.stroke_style(), StrokeStyle::Linear));
        assert!(!straight.smooth);

        let mut curved = three_series_line();
        curved.kind = ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: true,
        };
        let curved = LinePlot::multi_series(&curved).expect("plot");
        assert!(curved.smooth);
        assert!(matches!(curved.stroke_style(), StrokeStyle::Natural));
    }

    #[test]
    fn series_marker_is_carried_into_the_plot() {
        let mut chart = three_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_marker(Marker::new(MarkerSymbol::Square));
        let plot = LinePlot::multi_series(&chart).expect("plot");
        assert_eq!(
            plot.series[0].marker,
            Some(Marker::new(MarkerSymbol::Square))
        );
        // A series with no marker leaves it None (the renderer draws its default dot).
        assert_eq!(plot.series[1].marker, None);
    }

    #[test]
    fn series_data_labels_and_name_carry_into_the_plot() {
        use freecell_chart_model::DataLabels;
        let mut chart = three_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_data_labels(DataLabels::new().value().with_number_format("$#,##0"));
        let plot = LinePlot::multi_series(&chart).expect("plot");
        // The labelled series carries its dLbls + name (for the series-name part) into LineSeries.
        let dl = plot.series[0].data_labels.as_ref().expect("labels carried");
        assert!(dl.show_value);
        assert_eq!(dl.number_format.as_deref(), Some("$#,##0"));
        assert_eq!(plot.series[0].name.as_deref(), Some("North"));
        // A series with no labels stays None (nothing drawn).
        assert!(plot.series[1].data_labels.is_none());
    }

    #[test]
    fn theme_color_series_resolves_to_office_accent() {
        use freecell_chart_model::{ChartColor, ThemePalette, ThemeSlot};
        let mut chart = three_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_color(ChartColor::theme(ThemeSlot::Accent1));
        let plot = LinePlot::multi_series(&chart).expect("plot");
        let expected = super::super::style::model_hsla(ThemePalette::office_default().accent1);
        assert_eq!(
            plot.series[0].color, expected,
            "a schemeClr=accent1 series must resolve to the Office accent1 color"
        );
    }

    #[test]
    fn explicit_bounds_override_the_shared_scale() {
        // A value axis with explicit min/max clamps the shared domain to exactly those bounds
        // (P13), overriding the auto nice-scale that would otherwise zoom to the data.
        let mut chart = three_series_line();
        chart.val_axis = Axis::titled("Units").with_bounds(Some(0.0), Some(200.0));
        let plot = LinePlot::multi_series(&chart).expect("plot");
        assert_eq!(plot.scale.min, 0.0);
        assert_eq!(plot.scale.max, 200.0);

        // With no explicit bounds the scale auto-ranges to the data (min > 0 here, data floor 32).
        let auto = LinePlot::multi_series(&three_series_line()).expect("plot");
        assert!(auto.scale.min > 0.0, "auto scale zooms to the data");
    }

    #[test]
    fn axis_orientation_and_gridline_flags_carry_from_the_model() {
        // Defaults: not reversed, major gridlines on (Excel value-axis default).
        let plot = LinePlot::multi_series(&three_series_line()).expect("plot");
        assert!(!plot.value_reversed && !plot.category_reversed);
        assert!(plot.major_gridlines);

        let mut chart = three_series_line();
        chart.val_axis = Axis::titled("Units").reversed().without_major_gridlines();
        chart.cat_axis = Axis::titled("Quarter").reversed();
        let plot = LinePlot::multi_series(&chart).expect("plot");
        assert!(plot.value_reversed, "valAx maxMin → value axis reversed");
        assert!(
            plot.category_reversed,
            "catAx maxMin → category axis reversed"
        );
        assert!(
            !plot.major_gridlines,
            "no majorGridlines → gridlines suppressed"
        );
    }

    #[test]
    fn stroke_width_honors_a_ln_and_defaults_heavier() {
        // A series with no `a:ln` uses the Excel-like default weight (2.25pt → ~2.7px).
        let plain = LinePlot::multi_series(&three_series_line()).expect("plot");
        let expected_default = DEFAULT_LINE_WIDTH_PT * PT_TO_PX;
        assert!((plain.series[0].width_px - expected_default).abs() < 1e-3);
        assert!(
            plain.series[0].width_px > 2.0,
            "default line is heavier than the old 2.0px hairline"
        );

        // A series carrying `a:ln w="28440"` (~2.24pt) resolves to ~2.24pt × PT_TO_PX px.
        let mut chart = three_series_line();
        chart.series[0] = chart.series[0]
            .clone()
            .with_stroke(LineStroke::new().with_width_emu(28_440));
        let styled = LinePlot::multi_series(&chart).expect("plot");
        let expected = 2.24 * PT_TO_PX;
        assert!(
            (styled.series[0].width_px - expected).abs() < 0.05,
            "w=28440 should map to ~{expected}px, got {}",
            styled.series[0].width_px
        );
    }

    #[test]
    fn stroke_color_and_alpha_apply_to_the_line() {
        let mut chart = three_series_line();
        chart.series[0] = chart.series[0].clone().with_stroke(
            LineStroke::new()
                .with_color(Color::from_hex(0x123456))
                .with_alpha(0.4),
        );
        let plot = LinePlot::multi_series(&chart).expect("plot");
        // The stroke color wins over the palette, and the alpha rides on the resolved Hsla.
        let expected = with_alpha(
            super::super::style::model_hsla(Color::from_hex(0x123456)),
            0.4,
        );
        assert_eq!(plot.series[0].color, expected);
        assert!((plot.series[0].color.a - 0.4).abs() < 1e-6);

        // A series with no stroke stays fully opaque on its palette color.
        assert!((plot.series[1].color.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn value_axis_numfmt_formats_tick_labels() {
        // With a percent numFmt the ticks read as percentages; without one they are plain numbers.
        let mut chart = three_series_line();
        chart.val_axis = Axis::titled("Share").with_number_format("0%");
        let plot = LinePlot::multi_series(&chart).expect("plot");
        assert_eq!(plot.tick_label(0.25), "25%");

        let plain = LinePlot::multi_series(&three_series_line()).expect("plot");
        assert_eq!(plain.tick_label(40.0), format_tick(40.0));
    }
}
