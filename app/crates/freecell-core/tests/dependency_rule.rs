//! Guard for the strict crate dependency rule (`architecture.md §1`): `freecell-core`
//! and `freecell-engine` are the **headless, GPU-free foundation** and must never depend
//! on GPUI; `freecell-core` must additionally stay free of IronCalc. That rule is the
//! whole point of the split (core/engine build and test anywhere with no GPU/display), so
//! it is enforced here rather than left to the hand-written crate graph.
//!
//! This scans the two manifests' dependency tables directly (zero extra deps, instant):
//! if anyone adds `gpui*`/`ironcalc*` to `freecell-core`, or `gpui*` to `freecell-engine`,
//! this test fails. Dev-dependencies are intentionally ignored — a headless test-only
//! tool never ships in the crate and does not violate the runtime split.

use std::path::PathBuf;

/// Returns the dependency-key names declared under the runtime dependency tables of a
/// `Cargo.toml`. It recognizes both the inline-table form under a runtime dependency
/// section — `[dependencies]`, `[build-dependencies]`, `[target.'…'.dependencies]`,
/// `[target.'…'.build-dependencies]` — AND the dotted sub-table header form
/// (`[dependencies.NAME]`, `[target.'…'.dependencies.NAME]`, …), collecting NAME from the
/// header itself. Skips `[dev-dependencies]` (and its sub-tables), comments, and unrelated
/// tables.
fn runtime_dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            let header = line.trim_start_matches('[').trim_end_matches(']');
            // A runtime dependency section ends in `.dependencies` /
            // `.build-dependencies` (target-scoped) or is exactly `dependencies` /
            // `build-dependencies`.
            let is_section = |h: &str| {
                h == "dependencies"
                    || h == "build-dependencies"
                    || h.ends_with(".dependencies")
                    || h.ends_with(".build-dependencies")
            };
            if is_section(header) {
                in_deps = true;
            } else if let Some((section, name)) = header.rsplit_once('.') {
                // Dotted sub-table form, e.g. `dependencies.gpui` or
                // `target.'cfg(unix)'.dependencies.ironcalc`: the dependency name is the
                // final segment, so collect it directly from the header.
                if is_section(section) {
                    in_deps = false; // its body is that one crate's fields, not new keys
                    let name = name.trim().trim_matches('"');
                    if !name.is_empty() {
                        names.push(name.to_string());
                    }
                    continue;
                }
                in_deps = false;
            } else {
                in_deps = false;
            }
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Key is the text before `=` (e.g. `gpui = { … }`) or `.` (e.g. `gpui.workspace
        // = true`), with any surrounding quotes stripped.
        if let Some(key) = line.split(['=', '.']).next() {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() {
                names.push(key.to_string());
            }
        }
    }
    names
}

fn manifest_of(sibling_crate: &str) -> String {
    // CARGO_MANIFEST_DIR = .../app/crates/freecell-core → up to crates/, into the sibling.
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        sibling_crate,
        "Cargo.toml",
    ]
    .iter()
    .collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn assert_none_start_with(crate_name: &str, deps: &[String], forbidden_prefixes: &[&str]) {
    for dep in deps {
        for prefix in forbidden_prefixes {
            assert!(
                !dep.starts_with(prefix),
                "dependency rule violated: `{crate_name}` must not depend on `{dep}` \
                 (forbidden prefix `{prefix}`). See architecture.md §1 — core/engine are \
                 the headless, GPU-free foundation.",
            );
        }
    }
}

#[test]
fn core_has_no_gpui_or_ironcalc_dependency() {
    let deps = runtime_dependency_names(&manifest_of("freecell-core"));
    assert_none_start_with("freecell-core", &deps, &["gpui", "ironcalc"]);
}

#[test]
fn engine_has_no_gpui_dependency() {
    let deps = runtime_dependency_names(&manifest_of("freecell-engine"));
    assert_none_start_with("freecell-engine", &deps, &["gpui"]);
}

#[test]
fn guard_detects_a_forbidden_dependency() {
    // Negative control: the scanner + assertion actually trip on a violation, so a green
    // run above means "no gpui/ironcalc", not "the check silently passed nothing".
    let synthetic = "[dependencies]\ngpui.workspace = true\nserde = \"1\"\n";
    let deps = runtime_dependency_names(synthetic);
    assert!(deps.contains(&"gpui".to_string()));
    assert!(deps.contains(&"serde".to_string()));
    let caught = std::panic::catch_unwind(|| {
        assert_none_start_with("synthetic", &deps, &["gpui"]);
    });
    assert!(caught.is_err(), "guard should reject a gpui dependency");
}

#[test]
fn guard_catches_dotted_subtable_and_target_forms() {
    // Sub-table header forms must NOT slip past: the dependency name lives in the header
    // itself (`[dependencies.gpui]`), and target-scoped forms nest it deeper. A parser
    // that only matched exact `[dependencies]` headers would silently miss these.
    let manifest = "\
[package]
name = \"x\"

[dependencies.gpui]
version = \"1\"
features = [\"blade\"]

[target.'cfg(unix)'.dependencies.ironcalc]
version = \"=0.7.1\"

[build-dependencies.serde]
version = \"1\"
";
    let deps = runtime_dependency_names(manifest);
    assert!(
        deps.contains(&"gpui".to_string()),
        "missed [dependencies.gpui]"
    );
    assert!(
        deps.contains(&"ironcalc".to_string()),
        "missed target-scoped dotted sub-table"
    );
    assert!(
        deps.contains(&"serde".to_string()),
        "missed [build-dependencies.serde]"
    );
    // The crate-field lines under a sub-table (version/features) are NOT dep names.
    assert!(!deps.iter().any(|d| d == "version" || d == "features"));
    let caught = std::panic::catch_unwind(|| {
        assert_none_start_with("synthetic", &deps, &["gpui", "ironcalc"]);
    });
    assert!(
        caught.is_err(),
        "guard must reject dotted sub-table gpui/ironcalc deps"
    );
}

#[test]
fn guard_ignores_dev_dependency_subtables() {
    // Dev-dependencies (inline or sub-table, plain or target-scoped) are exempt from the
    // runtime split — a headless test-only tool never ships in the crate.
    let manifest = "\
[dev-dependencies]
gpui = \"1\"

[dev-dependencies.ironcalc]
version = \"1\"

[target.'cfg(unix)'.dev-dependencies.gpui_platform]
version = \"1\"
";
    let deps = runtime_dependency_names(manifest);
    assert!(
        deps.is_empty(),
        "dev-dependency sections must be ignored, got {deps:?}"
    );
}
