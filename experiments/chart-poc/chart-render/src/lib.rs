//! `chart-render` — FreeCell chart widgets over gpui-component's plot primitives, example
//! scenes, and the headless capture + agent-review harness (Experiments 2/3, §3-§6).
//!
//! gpui-free logic (the nice-tick generator, the color palette, the scene data) lives in
//! its own modules so it is unit-tested without a GPU; the gpui rendering + capture live in
//! [`bar`], [`render`], and [`capture`].

pub mod palette;
pub mod scenes;
pub mod ticks;

pub mod bar;
pub mod capture;
pub mod render;
