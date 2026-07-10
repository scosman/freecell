//! `render-tests` — the cell-render snapshot suite (`components/render_test_harness.md`).
//!
//! Renders the **real** [`GridView`](freecell_app::grid::GridView) over scenes produced by the
//! **real** engine ([`freecell_engine::DocumentClient`]), captures PNGs on Linux under Xvfb +
//! Mesa lavapipe (the Phase-1 spike's proven capture path), and perceptually diffs them against
//! committed baselines (the diff ported from round-3 C `ci_rendering`).
//!
//! Module split by dependency edge:
//! - [`diff`] is GPUI-free (pure `image`), so the perceptual-diff logic is unit-tested
//!   everywhere (it needs no display).
//! - [`scene`] / [`cases`] describe + realize grid fixtures through the engine; [`chart_scene`]
//!   describes chart fixtures (static [`freecell_chart_model::Chart`] data — no engine).
//! - [`render`] (gpui) opens the window (the real grid, or a standalone chart widget);
//!   [`capture`] drives `xrefresh` + `import` around it — the same headless mechanism for both.

pub mod capture;
pub mod cases;
pub mod chart_scene;
pub mod diff;
pub mod perf;
pub mod render;
pub mod scene;

pub use capture::{capture_available, render_all, render_charts, sibling_render_scene_bin};
pub use diff::{diff_image, diff_images, diff_png_files, DiffOptions, DiffReport};

#[cfg(test)]
mod tests {
    use super::scene::{engine_display, Scene};

    /// Guards the number-format-inference assumption the scene builder relies on: the engine
    /// yields the specced display strings for the currency / percent / thousands / date /
    /// boolean / error inputs, without any FreeCell-side format logic (round-3 B). If an
    /// IronCalc bump changes inference, this flips and the number-format baselines get a
    /// conscious update.
    #[test]
    fn scene_number_formats_infer() {
        let scene = Scene::new()
            .input(0, 0, "1,234,567")
            .input(0, 1, "$1,234.50")
            .input(0, 2, "50%")
            .input(0, 3, "2021-01-01")
            .input(1, 0, "TRUE")
            .input(1, 1, "=1/0");
        let display: std::collections::HashMap<(u32, u32), String> = engine_display(&scene)
            .expect("engine display")
            .into_iter()
            .collect();
        assert_eq!(display.get(&(0, 0)).map(String::as_str), Some("1,234,567"));
        assert_eq!(display.get(&(0, 1)).map(String::as_str), Some("$1,234.50"));
        assert_eq!(display.get(&(0, 2)).map(String::as_str), Some("50%"));
        assert_eq!(display.get(&(0, 3)).map(String::as_str), Some("2021-01-01"));
        assert_eq!(display.get(&(1, 0)).map(String::as_str), Some("TRUE"));
        assert_eq!(display.get(&(1, 1)).map(String::as_str), Some("#DIV/0!"));
    }
}
