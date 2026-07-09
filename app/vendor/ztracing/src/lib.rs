//! Permissively-licensed (MIT OR Apache-2.0) no-op stub for zed's GPL-3.0-or-later
//! `ztracing` crate.
//!
//! FreeCell never sets the `ZTRACING` env var, so upstream `ztracing` already compiles to a
//! no-op shim: an empty `Span`, token-eating span/event macros, a pass-through `instrument`,
//! and a `init()` that does nothing. This crate reproduces that exact no-op public API under
//! a permissive license. It is substituted for the upstream crate via `[patch]` in
//! `app/Cargo.toml`, which — because this stub does not depend on `zlog` — also drops the GPL
//! `zlog` crate out of the dependency graph entirely. See `app/vendor/README.md`.
//!
//! The only surface FreeCell's tree actually exercises is `instrument` (used as
//! `#[instrument(skip_all)]` in zed's `sum_tree`); the rest is kept for API parity with the
//! upstream crate.

pub use tracing::{field, Level};

pub use ztracing_macro::instrument;

/// Expands its call site to an empty [`Span`], discarding all arguments — the no-op form of
/// the upstream span/event macros.
#[macro_export]
macro_rules! __consume_all_tokens {
    ($($t:tt)*) => {
        $crate::Span
    };
}

pub use __consume_all_tokens as debug_span;
pub use __consume_all_tokens as error_span;
pub use __consume_all_tokens as event;
pub use __consume_all_tokens as info_span;
pub use __consume_all_tokens as span;
pub use __consume_all_tokens as trace_span;
pub use __consume_all_tokens as warn_span;

/// No-op stand-in for `tracing::Span`.
pub struct Span;

impl Span {
    pub fn current() -> Self {
        Self
    }

    pub fn enter(&self) {}

    pub fn record<T, S>(&self, _t: T, _s: S) {}
}

/// No-op stand-in for upstream `ztracing::init()`.
pub fn init() {}
