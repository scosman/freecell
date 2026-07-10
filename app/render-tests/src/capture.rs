//! Pixel capture — the Rust port of the Phase-1 spike (`app/scripts/linux_render_spike.sh`),
//! per case (`components/render_test_harness.md §Mechanism`, capture variant **option 2**).
//!
//! Each case renders under its **own** `xvfb-run` display sized to the case viewport (+ a small
//! margin). This is load-bearing: gpui/lavapipe only *presents* a window's frame to the
//! framebuffer when the window nearly fills the screen — a small window on a large screen
//! captures blank (verified during Phase 7). So the harness sizes Xvfb per case. Inside that
//! display it: launches `render_scene` (the real grid in an X window), waits for Vulkan init +
//! paint, runs **`xrefresh`** to force the Expose that makes gpui present (the spike's
//! load-bearing trick — gpui only presents on an Expose and Xvfb has no compositor to emit one),
//! finds the grid window by its size, and captures it with ImageMagick `import -window <id>`.
//!
//! Because each case owns its display, the harness needs **no ambient `DISPLAY`** — only
//! `xvfb-run` + the lavapipe ICD, both discovered here.
//!
//! The mechanism is identical for the grid ([`render_all`], the real `GridView`) and for a
//! standalone chart widget ([`render_charts`]): both build a `launch_cmd` that opens a
//! viewport-sized window and self-quits, then thread it through the same `capture_window` core.
//! The two paths differ only in which `render_scene` sub-command they launch (`--case` vs
//! `--chart`) and which fixture table they iterate.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::cases::{self, RenderCase};
use crate::chart_scene;

/// Seconds to let `render_scene` init Vulkan, create the window, and render the first frame
/// before forcing presentation. Override with `RENDER_TESTS_SETTLE_S`.
fn settle_s() -> f64 {
    env_f64("RENDER_TESTS_SETTLE_S", 3.5)
}

/// Seconds to wait after `xrefresh` for gpui to present the exposed frame, before capturing.
/// Override with `RENDER_TESTS_PRESENT_S`.
fn present_s() -> f64 {
    env_f64("RENDER_TESTS_PRESENT_S", 1.3)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Extra px around the window so it is fully on-screen while still nearly filling the display
/// (both conditions are needed: fully on-screen for a clean by-id capture, nearly-filling so
/// gpui presents at all).
const SCREEN_MARGIN: u32 = 8;

/// Whether the tools the capture path needs (`xvfb-run` + a lavapipe ICD) are present. When they
/// aren't (e.g. a Mac dev box without the Linux stack), the suite skips with a clear note.
pub fn capture_available() -> bool {
    which("xvfb-run") && lavapipe_icd().is_some()
}

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The Mesa lavapipe (software Vulkan) ICD path, discovered the way the Phase-1 spike does.
fn lavapipe_icd() -> Option<PathBuf> {
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

/// Locate the `render_scene` bin as a sibling of the current executable (for the
/// `generate_baselines` bin, whose sibling is `target/<profile>/render_scene`). Tests pass the
/// path explicitly via `CARGO_BIN_EXE_render_scene`.
pub fn sibling_render_scene_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("current exe has no parent dir"))?;
    let bin = dir.join(if cfg!(windows) {
        "render_scene.exe"
    } else {
        "render_scene"
    });
    if !bin.exists() {
        bail!("render_scene bin not found next to {}", exe.display());
    }
    Ok(bin)
}

/// Render every case (or those whose name starts with `only`) into `out_dir` as `<name>.png`,
/// using `render_scene_bin`. Returns the rendered case names. Errors if the capture tooling is
/// unavailable or a capture comes back blank.
pub fn render_all(
    render_scene_bin: &Path,
    out_dir: &Path,
    only: Option<&str>,
) -> Result<Vec<String>> {
    let icd = lavapipe_icd()
        .ok_or_else(|| anyhow!("no lavapipe ICD found (install mesa-vulkan-drivers)"))?;
    if !which("xvfb-run") {
        bail!("xvfb-run not found (install xvfb); the render suite needs a virtual display");
    }
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    let mut rendered = Vec::new();
    for case in cases::all() {
        if let Some(prefix) = only {
            if !case.name.starts_with(prefix) {
                continue;
            }
        }
        render_one(render_scene_bin, &icd, &case, out_dir)
            .with_context(|| format!("rendering case {}", case.name))?;
        rendered.push(case.name.to_string());
    }
    Ok(rendered)
}

/// Render every chart scene (or those whose name starts with `only`) into `out_dir` as
/// `<name>.png`, using `render_scene_bin`. Returns the rendered scene names. The chart analogue
/// of [`render_all`] — same Xvfb capture core, launching the `render_scene --chart` sub-command
/// over a standalone chart widget instead of the grid.
pub fn render_charts(
    render_scene_bin: &Path,
    out_dir: &Path,
    only: Option<&str>,
) -> Result<Vec<String>> {
    let icd = lavapipe_icd()
        .ok_or_else(|| anyhow!("no lavapipe ICD found (install mesa-vulkan-drivers)"))?;
    if !which("xvfb-run") {
        bail!("xvfb-run not found (install xvfb); the render suite needs a virtual display");
    }
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    let mut rendered = Vec::new();
    for scene in chart_scene::all() {
        if let Some(prefix) = only {
            if !scene.name.starts_with(prefix) {
                continue;
            }
        }
        let out = out_dir.join(format!("{}.png", scene.name));
        let launch = format!(
            "{bin} --chart {name} --exit-after-ms {ms}",
            bin = shell_quote(path_str(render_scene_bin)?),
            name = scene.name,
            ms = default_exit_after_ms(),
        );
        capture_window(&launch, scene.viewport, &icd, &out, scene.name)
            .with_context(|| format!("rendering chart scene {}", scene.name))?;
        rendered.push(scene.name.to_string());
    }
    Ok(rendered)
}

/// Render + capture a single grid case into `out_dir/<name>.png` under its own Xvfb display.
fn render_one(
    render_scene_bin: &Path,
    icd: &Path,
    case: &RenderCase,
    out_dir: &Path,
) -> Result<()> {
    let out = out_dir.join(format!("{}.png", case.name));
    let launch = format!(
        "{bin} --case {name} --exit-after-ms {ms}",
        bin = shell_quote(path_str(render_scene_bin)?),
        name = case.name,
        ms = default_exit_after_ms(),
    );
    capture_window(&launch, case.viewport, icd, &out, case.name)
}

/// The exit-after-ms a launched renderer uses so it outlives the settle + present + capture
/// window (the script kills the child afterward regardless).
pub fn default_exit_after_ms() -> u64 {
    ((settle_s() + present_s()) * 1000.0) as u64 + 8000
}

/// Launch `launch_cmd` (a shell command that starts a viewport-sized render window in the
/// background and self-quits) under its own `xvfb-run` display sized to `viewport`, force
/// presentation with `xrefresh`, find the `WxH` window, capture it to `out`, and assert the
/// capture is non-blank. The reusable core of both the grid ([`render_all`]) and chart
/// ([`render_charts`]) paths. `launch_cmd` must NOT background itself (`&`) — the script does.
/// `label` names the fixture in error messages.
///
/// **Diagnosability (P15):** the happy path is unchanged, but a failure must reveal WHY. Two
/// error sinks that used to be `/dev/null` are captured and surfaced: `xvfb-run` gets an explicit
/// `-e <errfile>` (its default error file is `/dev/null`, so a **failure of Xvfb itself** to
/// start otherwise vanishes), and the capture script redirects the **render binary's** stdout+
/// stderr to a log it dumps on any failure (a Vulkan/lavapipe device-creation failure otherwise
/// vanishes). On a non-zero exit the bail includes the exit code, the failing-command context,
/// `xvfb-run`'s stdout + stderr, and the Xvfb error file — so a CI run names the real cause.
fn capture_window(
    launch_cmd: &str,
    viewport: (u32, u32),
    icd: &Path,
    out: &Path,
    label: &str,
) -> Result<()> {
    let (w, h) = viewport;
    let script = capture_script(launch_cmd, icd, viewport, out)?;
    let screen = format!("-screen 0 {}x{}x24", w + SCREEN_MARGIN, h + SCREEN_MARGIN);

    // Give `xvfb-run` an explicit error file instead of its `/dev/null` default, so an Xvfb
    // startup / xauth failure (which never reaches the wrapped script's stderr) is captured.
    let xvfb_err = unique_diag_path(label, "xvfb-err");
    let xvfb_err_arg = path_str(&xvfb_err)?.to_string();

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "-e",
            &xvfb_err_arg,
            "-s",
            &screen,
            "bash",
            "-c",
            &script,
        ])
        .output()
        .context("spawning xvfb-run")?;

    let xvfb_err_text = read_and_remove(&xvfb_err);

    if !output.status.success() {
        bail!(
            "{}",
            format_capture_failure(
                label,
                output.status.code(),
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
                &xvfb_err_text,
            )
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
            "capture for {label} is blank ({colors} unique colour(s)); xvfb stderr:\n{}\nxvfb error file:\n{}",
            String::from_utf8_lossy(&output.stderr).trim(),
            xvfb_err_text.trim(),
        );
    }
    Ok(())
}

/// Assemble the (diagnosable) capture-failure message from the pieces the harness collects: the
/// `xvfb-run` exit code, its stdout + stderr (the wrapped script's own diagnostic block flows
/// here), and the Xvfb error file. Factored out so the message contract is unit-testable without
/// a display. Empty sections are labelled `<empty>` so a blank one is an explicit signal (not a
/// gap that reads like the section is missing).
fn format_capture_failure(
    label: &str,
    exit_code: Option<i32>,
    xvfb_stdout: &str,
    xvfb_stderr: &str,
    xvfb_err_file: &str,
) -> String {
    let section = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            "<empty>".to_string()
        } else {
            t.to_string()
        }
    };
    format!(
        "capture failed for {label} (xvfb-run exit {exit_code:?}).\n\
         --- xvfb-run stderr (includes the capture script's diagnostic block: render-binary log, \
         xwininfo tree, which step failed) ---\n{stderr}\n\
         --- xvfb-run stdout ---\n{stdout}\n\
         --- Xvfb error file (-e; Xvfb/xauth startup errors, normally /dev/null) ---\n{errfile}",
        stderr = section(xvfb_stderr),
        stdout = section(xvfb_stdout),
        errfile = section(xvfb_err_file),
    )
}

/// A unique path in the temp dir for a per-capture diagnostic sink (`kind` disambiguates the
/// Xvfb error file from any future sinks). Includes the pid so parallel captures never collide.
fn unique_diag_path(label: &str, kind: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir().join(format!(
        "freecell_render_{kind}_{safe}_{}_{nanos}.log",
        std::process::id()
    ))
}

/// Read a diagnostic file's text (empty string if unreadable/absent) and remove it. Used for the
/// Xvfb `-e` error file, which we only need on a failure and never want to leave behind.
fn read_and_remove(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let _ = std::fs::remove_file(path);
    text
}

/// The per-case bash script run inside `xvfb-run`: launch the renderer, force presentation with
/// `xrefresh`, find the render window by its size, and capture it by id.
///
/// **Diagnosability (P15):** the render binary's stdout+stderr goes to a script-local log
/// (`APP_LOG`) instead of `/dev/null`, and any non-zero `rc` dumps a diagnostic block to stderr —
/// which step failed, the resolved `DISPLAY`, whether the render process is still alive, the
/// render binary's captured log (a Vulkan/lavapipe device failure lands here), `import`'s stderr,
/// and the `xwininfo` window tree (so a "no window" failure shows what DID present). The happy
/// path (rc=0) prints nothing extra and behaves exactly as before — the timing/capture is
/// unchanged so committed baselines never move.
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
APP_LOG=$(mktemp)
IMPORT_LOG=$(mktemp)
{launch_cmd} >"$APP_LOG" 2>&1 &
APP=$!
sleep {settle}
xrefresh >/dev/null 2>&1 || true
sleep {present}
WID=$(xwininfo -root -tree 2>/dev/null | grep "{w}x{h}+" | grep -oE '0x[0-9a-f]+' | head -1)
rc=0
step=""
if [ -z "$WID" ]; then
  step="find-window (no {w}x{h} window presented)"
  rc=3
else
  import -window "$WID" {out} 2>"$IMPORT_LOG" || rc=$?
  [ "$rc" -ne 0 ] && step="import -window $WID"
fi
if [ "$rc" -ne 0 ]; then
  {{
    echo "capture step failed: $step (rc=$rc) on DISPLAY=${{DISPLAY:-<unset>}} screen {w}x{h}"
    if kill -0 "$APP" 2>/dev/null; then echo "[render process $APP still alive]"; else echo "[render process $APP already exited]"; fi
    echo "--- render binary log (stdout+stderr) ---"
    cat "$APP_LOG" 2>/dev/null || echo "<no render log>"
    echo "--- import stderr ---"
    cat "$IMPORT_LOG" 2>/dev/null || echo "<none>"
    echo "--- xwininfo -root -tree ---"
    xwininfo -root -tree 2>&1 | head -n 40 || echo "<xwininfo failed>"
  }} >&2
fi
kill -KILL $APP >/dev/null 2>&1 || true
rm -f "$APP_LOG" "$IMPORT_LOG"
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

/// The count of distinct RGBA colours in a PNG, capped at 3 — a cheap non-blank check. The
/// caller only needs to know whether there are at least two distinct colours (a blank present is
/// uniform), so counting stops early once a third colour proves the frame is non-blank.
fn unique_colors(path: &Path) -> Result<usize> {
    let img = image::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .to_rgba8();
    let mut seen = HashSet::new();
    for px in img.pixels() {
        seen.insert(px.0);
        if seen.len() > 2 {
            break; // >= 2 distinct colours already proves the capture is non-blank
        }
    }
    Ok(seen.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The diagnosable failure message (P15) surfaces the exit code and every diagnostic section
    /// so a CI run reveals the real cause of a capture failure — not the old blank
    /// `capture failed (exit Some(1)):`. Pure string assembly, so it needs no display.
    #[test]
    fn failure_message_surfaces_exit_code_and_every_section() {
        let msg = format_capture_failure(
            "border_all_thin",
            Some(1),
            "",
            "capture step failed: find-window (rc=3)\n--- render binary log (stdout+stderr) ---\nVulkan device creation failed",
            "Fatal server error:\nCannot establish any listening sockets",
        );
        // Names the case + the real exit code (the old message stopped here with empty stderr).
        assert!(msg.contains("border_all_thin"), "message: {msg}");
        assert!(msg.contains("exit Some(1)"), "message: {msg}");
        // Every source of truth is present and labelled.
        assert!(msg.contains("xvfb-run stderr"), "message: {msg}");
        assert!(msg.contains("xvfb-run stdout"), "message: {msg}");
        assert!(msg.contains("Xvfb error file"), "message: {msg}");
        // The swallowed subprocess detail now reaches the log.
        assert!(
            msg.contains("Vulkan device creation failed"),
            "render-binary log must surface: {msg}"
        );
        assert!(
            msg.contains("Cannot establish any listening sockets"),
            "Xvfb error file must surface: {msg}"
        );
    }

    /// An empty section is labelled `<empty>` (an explicit signal) rather than a blank that reads
    /// like the section was omitted — the exact ambiguity the old one-line message had.
    #[test]
    fn empty_sections_are_labelled_not_blank() {
        let msg = format_capture_failure("cell_plain", Some(1), "", "", "");
        assert!(
            msg.contains("<empty>"),
            "empty sections must be explicit: {msg}"
        );
        assert!(msg.contains("exit Some(1)"), "message: {msg}");
    }
}
