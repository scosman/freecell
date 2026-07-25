//! `freecell-app` library surface.
//!
//! The GPUI application is a binary (`src/main.rs`), but the custom [`grid`] component is
//! also exposed as a library so `render-tests` (Phase 7) and the perf harness (Phase 12)
//! can render the **real** grid over hand-built fixtures (`architecture.md §1` crate
//! dependency rule: `render-tests → freecell-app (grid)`). Nothing engine-specific lives
//! here; the grid reads only `freecell-core` read models (`components/grid.md`).

pub mod chart;
pub mod chrome;
pub mod grid;
pub mod shell;

/// Guards the cargo-packager `file-associations` block in this crate's `Cargo.toml`
/// (`specs/projects/xlsx-file-association/`). That block is packaging config, not runtime
/// code, so nothing else exercises it — these tests keep the two entries (`.xlsx`, `.csv`)
/// present and well-formed and, notably, pin `role = "editor"`: cargo-packager's
/// `CFBundleTypeRole` enum is camelCase + `deny_unknown_fields`, so a stray `"Editor"` would
/// fail to deserialize and silently break packaging.
#[cfg(test)]
mod packaging_metadata {
    use toml::Value;

    fn file_associations() -> Vec<Value> {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read freecell-app Cargo.toml");
        let manifest: Value = manifest.parse().expect("parse freecell-app Cargo.toml");
        manifest["package"]["metadata"]["packager"]["file-associations"]
            .as_array()
            .expect("[[package.metadata.packager.file-associations]] is an array-of-tables")
            .clone()
    }

    fn entry_for_extension<'a>(entries: &'a [Value], extension: &str) -> &'a Value {
        entries
            .iter()
            .find(|entry| {
                entry["extensions"]
                    .as_array()
                    .is_some_and(|exts| exts.iter().any(|ext| ext.as_str() == Some(extension)))
            })
            .unwrap_or_else(|| panic!("no file-association entry registers .{extension}"))
    }

    #[test]
    fn declares_exactly_xlsx_and_csv() {
        let entries = file_associations();
        assert_eq!(
            entries.len(),
            2,
            "expected exactly two file-association entries (xlsx, csv)"
        );
        entry_for_extension(&entries, "xlsx");
        entry_for_extension(&entries, "csv");
    }

    #[test]
    fn xlsx_entry_is_well_formed() {
        let entries = file_associations();
        let xlsx = entry_for_extension(&entries, "xlsx");
        assert_eq!(
            xlsx["mime-type"].as_str(),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "xlsx MimeType must be the OpenXML spreadsheet type"
        );
        assert_eq!(xlsx["name"].as_str(), Some("Excel Workbook"));
        assert_eq!(xlsx["description"].as_str(), Some("Excel Workbook"));
    }

    #[test]
    fn csv_entry_is_well_formed() {
        let entries = file_associations();
        let csv = entry_for_extension(&entries, "csv");
        assert_eq!(csv["mime-type"].as_str(), Some("text/csv"));
        assert_eq!(csv["name"].as_str(), Some("CSV Document"));
        assert_eq!(csv["description"].as_str(), Some("CSV Document"));
    }

    #[test]
    fn roles_use_cargo_packager_lowercase_enum() {
        // cargo-packager's `BundleTypeRole` is `#[serde(rename_all = "camelCase",
        // deny_unknown_fields)]`, so the Editor role's only valid spelling is lowercase
        // "editor"; "Editor" fails to deserialize and breaks packaging.
        for entry in file_associations() {
            if let Some(role) = entry.get("role") {
                assert_eq!(
                    role.as_str(),
                    Some("editor"),
                    "file-association role must be lowercase \"editor\" for cargo-packager"
                );
            }
        }
    }
}
