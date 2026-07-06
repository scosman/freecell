//! Headless pixel capture — adapted from `app/render-tests/src/capture.rs` (the repo's
//! proven Linux path), copied here because experiments stay independent of `/app`.
//!
//! Each scene renders under its **own** `xvfb-run` display sized to the scene viewport (+ a
//! small margin). This is load-bearing: gpui/lavapipe only *presents* a window's frame to the
//! framebuffer when the window nearly fills the screen — a small window on a large screen
//! captures blank. Inside that display the harness launches `render_scene`, waits for Vulkan
//! init + paint, runs **`xrefresh`** to force the Expose that makes gpui present (Xvfb has no
//! compositor to emit one), finds the chart window by its size, and captures it with
//! ImageMagick `import -window <id>`.
//!
//! It also writes `manifest.json` (one `{name, description, expectation}` entry per image)
//! for the agent-review step (functional_spec §6).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

use crate::scenes::{self, Scene};

/// Seconds to let `render_scene` init Vulkan, create the window, and render the first frame
/// before forcing presentation. Override with `CHART_POC_SETTLE_S`.
fn settle_s() -> f64 {
    env_f64("CHART_POC_SETTLE_S", 3.5)
}

/// Seconds to wait after `xrefresh` for gpui to present the exposed frame, before capturing.
/// Override with `CHART_POC_PRESENT_S`.
fn present_s() -> f64 {
    env_f64("CHART_POC_PRESENT_S", 1.3)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Extra px around the window so it is fully on-screen while still nearly filling the display
/// (both are needed: fully on-screen for a clean by-id capture, nearly-filling so gpui
/// presents at all).
const SCREEN_MARGIN: u32 = 8;

/// One `manifest.json` entry — the review harness feeds each PNG + its `expectation` to the
/// reviewer agent (functional_spec §6).
#[derive(Serialize)]
pub struct ManifestEntry {
    pub name: String,
    pub png: String,
    pub description: String,
    pub expectation: String,
}

/// Whether the tools the capture path needs (`xvfb-run` + a lavapipe ICD + ImageMagick
/// `import` + `xwininfo` + `xrefresh`) are present.
pub fn capture_available() -> bool {
    which("xvfb-run")
        && which("import")
        && which("xwininfo")
        && which("xrefresh")
        && lavapipe_icd().is_some()
}

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The Mesa lavapipe (software Vulkan) ICD path.
pub fn lavapipe_icd() -> Option<PathBuf> {
    let dir = Path::new("/usr/share/vulkan/icd.d");
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("lvp_icd") && name.ends_with(".json") {
            return Some(entry.path());
        }
    }
    None
}

/// Locate a bin as a sibling of the current executable (they are built into the same
/// `target/<profile>/` dir). Reused by the `load-save` render proof to find `render_loaded`.
pub fn sibling_bin(name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("current exe has no parent dir"))?;
    let bin = dir.join(name);
    if !bin.exists() {
        bail!("{name} bin not found next to {}", exe.display());
    }
    Ok(bin)
}

/// Locate the `render_scene` bin as a sibling of the current executable.
pub fn sibling_render_scene_bin() -> Result<PathBuf> {
    sibling_bin("render_scene")
}

/// Render every scene (or those whose name starts with `only`) into `out_dir` as
/// `<name>.png`, write `out_dir/manifest.json`, and return the rendered scene names. Errors
/// if the capture tooling is unavailable or a capture comes back blank.
pub fn render_all(
    render_scene_bin: &Path,
    out_dir: &Path,
    only: Option<&str>,
) -> Result<Vec<String>> {
    let icd = lavapipe_icd()
        .ok_or_else(|| anyhow!("no lavapipe ICD found (install mesa-vulkan-drivers)"))?;
    for tool in ["xvfb-run", "xrefresh", "xwininfo", "import"] {
        if !which(tool) {
            bail!("{tool} not found; the capture path needs xvfb + x11-utils + imagemagick");
        }
    }
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    let mut rendered = Vec::new();
    let mut manifest = Vec::new();
    for scene in scenes::all() {
        if let Some(prefix) = only {
            if !scene.name.starts_with(prefix) {
                continue;
            }
        }
        render_one(render_scene_bin, &icd, &scene, out_dir)
            .with_context(|| format!("rendering scene {}", scene.name))?;
        manifest.push(ManifestEntry {
            name: scene.name.to_string(),
            png: format!("{}.png", scene.name),
            description: scene.description.to_string(),
            expectation: scene.expectation.to_string(),
        });
        rendered.push(scene.name.to_string());
    }

    let manifest_path = out_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(&manifest).context("serializing manifest")?;
    std::fs::write(&manifest_path, json)
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    Ok(rendered)
}

/// Render + capture a single scene into `out_dir/<name>.png` under its own Xvfb display.
fn render_one(render_scene_bin: &Path, icd: &Path, scene: &Scene, out_dir: &Path) -> Result<()> {
    let out = out_dir.join(format!("{}.png", scene.name));
    let bin = shell_quote(path_str(render_scene_bin)?);
    let exit_after_ms = default_exit_after_ms();
    let launch = format!(
        "{bin} --scene {} --exit-after-ms {exit_after_ms}",
        scene.name
    );
    capture_window(&launch, scene.viewport, icd, &out)
        .with_context(|| format!("capturing scene {}", scene.name))
}

/// The exit-after-ms a launched renderer should use so it outlives the settle + present +
/// capture window.
pub fn default_exit_after_ms() -> u64 {
    ((settle_s() + present_s()) * 1000.0) as u64 + 8000
}

/// Launch `launch_cmd` (a shell command that starts a viewport-sized render window in the
/// background and self-quits), force presentation with `xrefresh`, find the `WxH` window, and
/// capture it to `out` — then assert the capture is non-blank. This is the reusable core of the
/// proven `app/render-tests` path; both the scene harness and the `load-save` render proof call
/// it. `launch_cmd` should NOT background itself (`&`) — this wrapper does.
pub fn capture_window(
    launch_cmd: &str,
    viewport: (u32, u32),
    icd: &Path,
    out: &Path,
) -> Result<()> {
    let (w, h) = viewport;
    let script = capture_script(launch_cmd, icd, viewport, out)?;
    let screen = format!("-screen 0 {}x{}x24", w + SCREEN_MARGIN, h + SCREEN_MARGIN);

    let output = Command::new("xvfb-run")
        .args(["-a", "-s", &screen, "bash", "-c", &script])
        .output()
        .context("spawning xvfb-run")?;
    if !output.status.success() {
        bail!(
            "capture failed (exit {:?}):\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    // Guard against a silent blank (the window failed to present): a failed present yields a
    // single uniform colour, so a non-blank capture must have at least two distinct colours.
    let colors = unique_colors(out).with_context(|| {
        format!(
            "reading captured {} (xvfb stderr: {})",
            out.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    if colors <= 1 {
        bail!(
            "capture {} is blank ({colors} unique colour(s)); xvfb stderr:\n{}",
            out.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// The bash script run inside `xvfb-run`: launch the renderer, force presentation with
/// `xrefresh`, find the window by its size, and capture it by id.
fn capture_script(
    launch_cmd: &str,
    icd: &Path,
    viewport: (u32, u32),
    out: &Path,
) -> Result<String> {
    let (w, h) = viewport;
    let icd = shell_quote(path_str(icd)?);
    let out = shell_quote(path_str(out)?);
    let settle = settle_s();
    let present = present_s();

    Ok(format!(
        r#"set -u
export VK_ICD_FILENAMES={icd}
export LIBGL_ALWAYS_SOFTWARE=1
export ZED_ALLOW_EMULATED_GPU=1
{launch_cmd} >/dev/null 2>&1 &
APP=$!
sleep {settle}
xrefresh >/dev/null 2>&1 || true
sleep {present}
WID=$(xwininfo -root -tree 2>/dev/null | grep "{w}x{h}+" | grep -oE '0x[0-9a-f]+' | head -1)
rc=0
if [ -z "$WID" ]; then
  echo "no {w}x{h} render window found" >&2
  rc=3
else
  import -window "$WID" {out} || rc=$?
fi
kill -KILL $APP >/dev/null 2>&1 || true
exit $rc
"#,
    ))
}

/// Single-quote a string for safe bash interpolation (paths may contain spaces).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}

/// The count of distinct RGBA colours in a PNG, capped at 3 — a cheap non-blank check.
fn unique_colors(path: &Path) -> Result<usize> {
    let img = image::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .to_rgba8();
    let mut seen = HashSet::new();
    for px in img.pixels() {
        seen.insert(px.0);
        if seen.len() > 2 {
            break;
        }
    }
    Ok(seen.len())
}
