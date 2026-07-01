//! FreeCell Round-3 Investigation C — the macOS/human-run render->PNG harness.
//!
//! Renders a minimal FreeCell grid **offscreen** (no visible window) via GPUI's headless
//! capture surface and writes a PNG, then perceptual-diffs it against a baseline using the
//! parent `ci_rendering` crate. This closes the C GATE end-to-end on a Mac:
//!
//! ```text
//!   render baseline -> baseline.png
//!   re-render       -> rerender.png     ;  diff(baseline, rerender)  MUST PASS
//!   render changed  -> changed.png      ;  diff(baseline, changed)   MUST FAIL
//! ```
//!
//! # Why macOS-only (the DISCOVERY)
//!
//! At the pinned Zed rev (`1d217ee39d381ac101b7cf49d3d22451ac1093fe`) GPUI's offscreen
//! capture is real but Metal-backed:
//! - `PlatformHeadlessRenderer::render_scene_to_image(&Scene, Size<DevicePixels>) ->
//!   RgbaImage` (`crates/gpui/src/platform.rs`),
//! - surfaced via `VisualTestAppContext::capture_screenshot(window) -> RgbaImage` which
//!   calls `Window::render_to_image()` (`crates/gpui/src/app/visual_test_context.rs`),
//! - fed by `gpui_platform::current_headless_renderer()`
//!   (`crates/gpui_platform/src/gpui_platform.rs`), which returns
//!   `Some(MetalHeadlessRenderer)` **only** under `#[cfg(target_os = "macos")]` and `None`
//!   otherwise.
//!
//! So there is NO windowless GPU capture on Linux in this rev: the CI mechanism is a
//! **macOS runner** rendering offscreen (no display needed). This program mirrors Zed's own
//! `crates/zed/src/visual_test_runner.rs` (which uses the identical
//! `VisualTestAppContext` + `capture_screenshot` path), so it tracks a real, maintained
//! reference — not a speculative API.
//!
//! Run ONLY on macOS (see ../scripts/render_and_diff.sh). It does not build in the headless
//! Linux container — that is expected and is itself a Phase-C finding.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use ci_rendering::{diff_png_files, DiffOptions};
use gpui::{
    div, px, rgb, size, Bounds, Context, FontWeight, IntoElement, ParentElement, Pixels,
    Point, Render, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_platform::current_platform;

/// One rendered cell of the tiny fixed scene. Kept minimal on purpose — a snapshot needs to
/// be *deterministic and meaningful*, not a full Excel-max grid (that is the raw-gpui PoC's
/// perf job; this is the *capture* job).
#[derive(Clone)]
struct Cell {
    text: String,
    /// `0xRRGGBB` fill.
    fill: u32,
    bold: bool,
}

/// The scene variant to render.
#[derive(Clone, Copy, PartialEq)]
enum Scene {
    /// The normal grid — used for `baseline.png` and `rerender.png` (must diff-PASS).
    Baseline,
    /// A deliberately-changed grid (one cell recolored + relabeled) — used for
    /// `changed.png`, which MUST diff-FAIL against the baseline, proving the diff has real
    /// discriminating power (not a rubber stamp).
    Changed,
}

/// A minimal absolute-positioned grid view (mirrors the raw-gpui PoC's `grid.rs` render
/// style: white bg, grey gridlines, per-cell fill + bold, header strip). Fixed 5x4 layout.
struct Grid {
    cells: Vec<Vec<Cell>>,
}

const ROWS: usize = 5;
const COLS: usize = 4;
const CELL_W: f32 = 90.0;
const CELL_H: f32 = 28.0;
const HEADER_H: f32 = 24.0;
const HEADER_W: f32 = 40.0;

impl Grid {
    fn new(scene: Scene) -> Self {
        let mut cells = Vec::with_capacity(ROWS);
        for r in 0..ROWS {
            let mut row = Vec::with_capacity(COLS);
            for c in 0..COLS {
                // A deterministic mix: some numbers, one highlighted cell, one bold header
                // row, so the snapshot has varied content (text, fill, weight).
                let highlighted = r == 2 && c == 1;
                let fill = if highlighted { 0xFFF9C4 } else { 0xFFFFFF };
                let text = format!("{}{}", (b'A' + c as u8) as char, r + 1);
                row.push(Cell {
                    text,
                    fill,
                    bold: r == 0,
                });
            }
            cells.push(row);
        }

        if scene == Scene::Changed {
            // The single deliberate change: recolor + relabel one interior cell. Small
            // enough to be a realistic "regression", large enough to exceed the diff's
            // per-channel tolerance on a clear fraction of pixels.
            cells[3][2] = Cell {
                text: "XX".to_string(),
                fill: 0x2A5ADC,
                bold: true,
            };
        }

        Self { cells }
    }
}

impl Render for Grid {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut children: Vec<gpui::AnyElement> = Vec::new();

        // Column headers (A..) across the top.
        for c in 0..COLS {
            let left = HEADER_W + c as f32 * CELL_W;
            children.push(
                header_cell(
                    ((b'A' + c as u8) as char).to_string(),
                    px(CELL_W),
                    px(HEADER_H),
                    px(left),
                    px(0.0),
                )
                .into_any_element(),
            );
        }
        // Row headers (1..) down the left gutter.
        for r in 0..ROWS {
            let top = HEADER_H + r as f32 * CELL_H;
            children.push(
                header_cell(
                    (r + 1).to_string(),
                    px(HEADER_W),
                    px(CELL_H),
                    px(0.0),
                    px(top),
                )
                .into_any_element(),
            );
        }
        // Data cells.
        for (r, row) in self.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let left = HEADER_W + c as f32 * CELL_W;
                let top = HEADER_H + r as f32 * CELL_H;
                children.push(data_cell(cell, px(left), px(top)).into_any_element());
            }
        }

        div()
            .relative()
            .size_full()
            .bg(rgb(0xFFFFFF))
            .children(children)
    }
}

fn header_cell(
    text: String,
    w: Pixels,
    h: Pixels,
    left: Pixels,
    top: Pixels,
) -> impl IntoElement {
    div()
        .absolute()
        .left(left)
        .top(top)
        .w(w)
        .h(h)
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(0xF2F2F2))
        .border_1()
        .border_color(rgb(0xD0D0D0))
        .text_color(rgb(0x555555))
        .text_xs()
        .child(text)
}

fn data_cell(cell: &Cell, left: Pixels, top: Pixels) -> impl IntoElement {
    let mut el = div()
        .absolute()
        .left(left)
        .top(top)
        .w(px(CELL_W))
        .h(px(CELL_H))
        .px_1()
        .flex()
        .items_center()
        .bg(rgb(cell.fill))
        .border_1()
        .border_color(rgb(0xD0D0D0))
        .text_color(rgb(0x1A1A1A))
        .text_sm()
        .child(cell.text.clone());
    if cell.bold {
        el = el.font_weight(FontWeight::BOLD);
    }
    el
}

/// Renders `scene` offscreen and returns the captured image, saved to `out`.
///
/// Mirrors Zed's `visual_test_runner.rs`: build a `VisualTestAppContext` on the headless
/// platform with a real (Metal) headless renderer, open a hidden window, park, refresh,
/// park, then `capture_screenshot`. No display is required.
#[cfg(target_os = "macos")]
fn render_scene_to_png(scene: Scene, out: &Path) -> Result<()> {
    use gpui::VisualTestAppContext;
    use std::sync::Arc;

    // Headless platform. `VisualTestAppContext` wraps it in a `VisualTestPlatform` and pulls
    // the (Metal) headless renderer from the platform itself — the exact path Zed's own
    // `visual_test_runner.rs` uses (`current_platform(false)` there). Offscreen capture comes
    // from `WindowOptions.show = false` below, not from this flag, so we mirror the reference.
    let platform = current_platform(false);
    let mut cx = VisualTestAppContext::with_asset_source(platform, Arc::new(()));

    let bounds = Bounds {
        origin: Point::default(),
        size: size(
            px(HEADER_W + COLS as f32 * CELL_W),
            px(HEADER_H + ROWS as f32 * CELL_H),
        ),
    };

    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: false,
                ..Default::default()
            },
            |_window, cx| cx.new(|_| Grid::new(scene)),
        )
    })?;

    // open -> park -> refresh -> park -> capture (Zed's own sequence).
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _cx| window.refresh())?;
    cx.run_until_parked();

    let image = cx.capture_screenshot(window.into())?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    image
        .save(out)
        .with_context(|| format!("saving PNG to {}", out.display()))?;
    println!("wrote {} ({}x{})", out.display(), image.width(), image.height());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn render_scene_to_png(_scene: Scene, _out: &Path) -> Result<()> {
    // GPUI's headless renderer is macOS/Metal-only in the pinned rev; there is no windowless
    // GPU capture path on other platforms. This mirrors `current_headless_renderer()`
    // returning `None` off-macOS. See ../findings.md.
    bail!(
        "render_grid can only capture on macOS: GPUI's headless renderer \
         (current_headless_renderer) returns None off-macOS in this Zed rev"
    );
}

fn results_dir() -> PathBuf {
    // ../results relative to this package (i.e. C-ci-rendering/results).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("results"))
        .unwrap_or_else(|| PathBuf::from("results"))
}

fn print_usage() {
    eprintln!(
        "usage:\n  \
         render_grid --scene <baseline|changed> [--out <path.png>]\n  \
         render_grid --diff <a.png> <b.png>   # exit 0 = PASS, 1 = FAIL (perceptual)\n\n\
         The macOS run closes the C GATE via ../scripts/render_and_diff.sh:\n  \
         baseline vs re-render MUST PASS; baseline vs changed MUST FAIL."
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--scene") => {
            let scene = match args.get(2).map(String::as_str) {
                Some("baseline") => Scene::Baseline,
                Some("changed") => Scene::Changed,
                _ => {
                    print_usage();
                    bail!("--scene requires 'baseline' or 'changed'");
                }
            };
            // --out override, else results/<scene>.png.
            let out = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let name = if scene == Scene::Baseline {
                        "baseline.png"
                    } else {
                        "changed.png"
                    };
                    results_dir().join(name)
                });
            render_scene_to_png(scene, &out)
        }
        Some("--diff") => {
            let (a, b) = match (args.get(2), args.get(3)) {
                (Some(a), Some(b)) => (PathBuf::from(a), PathBuf::from(b)),
                _ => {
                    print_usage();
                    bail!("--diff requires two PNG paths");
                }
            };
            let report = diff_png_files(&a, &b, &DiffOptions::default())?;
            println!("{}", report.summary());
            if report.passed {
                Ok(())
            } else {
                std::process::exit(1);
            }
        }
        _ => {
            print_usage();
            Ok(())
        }
    }
}
