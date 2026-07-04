//! Captures the **build** toolchain version into `FREECELL_RUSTC_VERSION` so the perf
//! harness can stamp its report/JSON with the exact `rustc` that produced the measured
//! binary (`CLAUDE.md`: environment-stamped numbers). Uses cargo's `RUSTC` so it reflects
//! the pinned toolchain, not whatever happens to be on PATH at run time.

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=FREECELL_RUSTC_VERSION={version}");
    // Only the compiler version matters here; don't re-run on unrelated source changes.
    println!("cargo:rerun-if-changed=build.rs");
}
