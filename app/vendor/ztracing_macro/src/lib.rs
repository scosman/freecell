//! Permissively-licensed (MIT OR Apache-2.0) no-op stub for zed's GPL-3.0-or-later
//! `ztracing_macro` crate.
//!
//! Upstream `ztracing_macro::instrument` is itself a pass-through attribute (it returns the
//! annotated item unchanged) in every FreeCell build, since we never set the `ZTRACING` env
//! var. This crate reproduces that trivial behavior under a permissive license so that
//! `[patch]` (in `app/Cargo.toml`) can substitute it and keep the GPL crate out of the
//! compiled/linked binary. See `app/vendor/README.md`.

/// No-op stand-in for `ztracing_macro::instrument`: expands to the annotated item unchanged.
#[proc_macro_attribute]
pub fn instrument(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    item
}
