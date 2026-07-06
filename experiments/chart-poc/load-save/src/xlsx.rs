//! Low-level `.xlsx`-as-OPC-zip helpers shared by [`crate::load`] and [`crate::save`]:
//! reading a named zip entry (mirrors `app/.../open_fixups.rs::read_zip_entry`), parsing an
//! OPC `_rels` part into an `Id → (Type, Target)` map, and resolving a relationship `Target`
//! (which may be `../`-relative) into an absolute part name inside the package.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

/// Reads one entry from an `.xlsx` on disk into a `String`. Mirrors
/// `open_fixups::read_zip_entry`, but surfaces a typed error (the load path wants to know
/// *why* a part is missing, unlike the best-effort fix-up pass).
pub fn read_entry(path: &Path, name: &str) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;
    read_entry_from(&mut archive, name)
}

/// Reads one entry from an already-open archive into a `String`.
pub fn read_entry_from<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("zip entry {name} not found"))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .with_context(|| format!("reading zip entry {name}"))?;
    Ok(buf)
}

/// Returns `true` if the archive contains an entry with this exact name.
pub fn has_entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> bool {
    archive.by_name(name).is_ok()
}

/// One parsed OPC relationship: its `Type` URI and `Target` (as written, possibly relative).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub rel_type: String,
    pub target: String,
}

/// A parsed `_rels` part — `Relationship/@Id → { Type, Target }`.
pub type Rels = BTreeMap<String, Relationship>;

/// Parses an OPC `_rels` XML document into an `Id → (Type, Target)` map. Relationships with a
/// `TargetMode="External"` are skipped (they point outside the package and are never chart
/// parts). Namespace/prefix-agnostic (matches by local tag/attribute name).
pub fn parse_rels(xml: &str) -> Result<Rels> {
    let doc = roxmltree::Document::parse(xml).context("parsing _rels XML")?;
    let mut map = Rels::new();
    for node in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
    {
        let Some(id) = attr(&node, "Id") else {
            continue;
        };
        let Some(target) = attr(&node, "Target") else {
            continue;
        };
        if attr(&node, "TargetMode") == Some("External") {
            continue;
        }
        let rel_type = attr(&node, "Type").unwrap_or("").to_string();
        map.insert(
            id.to_string(),
            Relationship {
                rel_type,
                target: target.to_string(),
            },
        );
    }
    Ok(map)
}

/// The `_rels` part name for a part — e.g. `xl/worksheets/sheet1.xml` →
/// `xl/worksheets/_rels/sheet1.xml.rels`.
pub fn rels_part_for(part: &str) -> String {
    match part.rfind('/') {
        Some(slash) => format!("{}/_rels/{}.rels", &part[..slash], &part[slash + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolves a relationship `Target` (relative to the directory of the part that *owns* the
/// `_rels`) into an absolute, normalized package part name. Handles `../` and `./` segments
/// and a leading `/` (already package-absolute). Example: owner `xl/drawings/drawing1.xml`,
/// target `../charts/chart1.xml` → `xl/charts/chart1.xml`.
pub fn resolve_target(owner_part: &str, target: &str) -> String {
    if let Some(stripped) = target.strip_prefix('/') {
        return normalize(stripped);
    }
    let base_dir = owner_part.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let joined = if base_dir.is_empty() {
        target.to_string()
    } else {
        format!("{base_dir}/{target}")
    };
    normalize(&joined)
}

/// Collapses `.`/`..` segments in a `/`-separated path (no leading slash in the result).
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// An attribute by **local name** (namespace-agnostic) — so `r:id` matches a query of `"id"`
/// and a plain `val` matches `"val"`. roxmltree stores the prefix expansion in the namespace,
/// so scanning by local name is the robust way to read OOXML attributes.
pub fn attr<'a>(node: &roxmltree::Node<'a, '_>, local: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_and_absolute_targets() {
        assert_eq!(
            resolve_target("xl/drawings/drawing1.xml", "../charts/chart1.xml"),
            "xl/charts/chart1.xml"
        );
        assert_eq!(
            resolve_target("xl/worksheets/sheet1.xml", "../drawings/drawing1.xml"),
            "xl/drawings/drawing1.xml"
        );
        assert_eq!(
            resolve_target("xl/workbook.xml", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        // A package-absolute target (leading slash) drops the slash.
        assert_eq!(
            resolve_target("xl/drawings/drawing1.xml", "/xl/charts/chart9.xml"),
            "xl/charts/chart9.xml"
        );
    }

    #[test]
    fn rels_part_naming() {
        assert_eq!(
            rels_part_for("xl/worksheets/sheet1.xml"),
            "xl/worksheets/_rels/sheet1.xml.rels"
        );
        assert_eq!(
            rels_part_for("xl/workbook.xml"),
            "xl/_rels/workbook.xml.rels"
        );
    }

    #[test]
    fn parses_rels_skipping_external() {
        let xml = r#"<?xml version="1.0"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://.../drawing" Target="../drawings/drawing1.xml"/>
          <Relationship Id="rId2" Type="http://.../hyperlink" Target="http://x" TargetMode="External"/>
        </Relationships>"#;
        let rels = parse_rels(xml).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels["rId1"].target, "../drawings/drawing1.xml");
        assert!(rels["rId1"].rel_type.ends_with("/drawing"));
    }
}
