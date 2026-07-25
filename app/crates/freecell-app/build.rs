//! Build script for the `freecell-app` crate.
//!
//! **Windows only:** embed the FreeCell app icon into the `freecell` executable so the exe (and
//! its window) show the FreeCell icon instead of the generic Windows application icon.
//!
//! gpui's Windows backend loads the window-class / app icon straight out of the executable's
//! embedded resources via `LoadImageW(module, MAKEINTRESOURCE(1), IMAGE_ICON, …)` — i.e. the icon
//! bound to resource **ID 1**. With no such resource the call fails and gpui falls back to a null
//! `HICON`, so Windows shows its default generic icon (the reported bug). We therefore compile a
//! tiny `.rc` that binds `1 ICON` to `packaging/icons/icon.ico` (the same multi-size `.ico`
//! cargo-packager already ships to the Windows installer). This mirrors how Zed embeds its own icon
//! for the identical gpui `load_icon` code path.
//!
//! No manifest is embedded here: gpui already embeds the application manifest via its
//! `windows-manifest` feature, so this script only contributes the icon resource.
//!
//! No-op when the build **host** is macOS/Linux — those take their app icon from the `.app` bundle
//! / the `.desktop` + hicolor icons cargo-packager installs, so nothing is embedded into the
//! Mach-O/ELF here.
//!
//! **Native-build assumption (host == target).** The `#[cfg(target_os = "windows")]` guard below
//! keys off the build **host** — build scripts are compiled and run for the host — so it is true
//! only when building *on* Windows. This differs from the `Cargo.toml` build-dependency gate: a
//! `[target.'cfg(...)'.build-dependencies]` table is selected by the compilation **target** (Cargo
//! evaluates its `cfg` against `--target`), so a Linux → Windows *cross*-compile would still pull
//! `embed-resource` in — and it is then this host-based `#[cfg]` that skips the embed, not the gate.
//! FreeCell's release Windows build runs natively on `windows-latest` (host == target), so the icon
//! is embedded exactly when intended; a Linux/macOS → Windows cross-compile would silently omit it
//! (this guard is false on a non-Windows host) — a path that is not part of the packaging pipeline.
//! If cross-compiling to Windows ever becomes real, switch this `#[cfg]` guard to the target via the
//! `CARGO_CFG_TARGET_OS` build-script env var (the `Cargo.toml` gate is already target-based). This
//! mirrors Zed's own icon embedding, which makes the same native-build assumption.
fn main() {
    #[cfg(target_os = "windows")]
    embed_windows_app_icon();
}

#[cfg(target_os = "windows")]
fn embed_windows_app_icon() {
    use std::path::PathBuf;

    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");

    // gpui's `load_icon` looks up icon resource ID 1, so bind `1 ICON` to our `.ico`. Escape
    // backslashes for the `.rc` string literal (Windows paths).
    let icon_escaped = icon.to_string_lossy().replace('\\', "\\\\");
    let rc = format!("1 ICON \"{icon_escaped}\"\n");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let rc_path = out_dir.join("freecell_icon.rc");
    std::fs::write(&rc_path, rc).expect("write the FreeCell icon resource script");

    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .expect("embed the FreeCell app icon resource");
}
