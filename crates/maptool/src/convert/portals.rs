//! Legacy `portals` → portal-mode edges migration. Reads a pre-refactor
//! `.mindmap.json` that still has a top-level `portals` array and
//! rewrites each entry as an edge with `display_mode = "portal"`.
//! The loader rejects files that still carry `portals`, so this is
//! the one-way door users with unmigrated maps walk through.
//!
//! Round-trip guarantee: every `PortalPair` field survives the
//! migration —
//! - `endpoint_a` / `endpoint_b` → `from_id` / `to_id`
//! - `glyph` → `glyph_connection.body`
//! - `color` → `edge.color` (the marker reads this when no
//!   `glyph_connection.color` override is set)
//! - `font` / `font_size_pt` → `glyph_connection.{font, font_size_pt}`
//!
//! The portal `label` is dropped — post-refactor portals identify
//! themselves by `(from_id, to_id, edge_type)` like any other edge,
//! so the auto-assigned column-letter label has no role. Users who
//! relied on the label for visual identification were reading the
//! marker glyph, which carries over unchanged.

use serde_json::{json, Value};
use std::path::Path;

/// Read `input_path`, convert any `portals[]` entries into
/// portal-mode edges appended to `edges[]`, and write the result to
/// `output_path`. In-place migrations (input == output) are fine:
/// the read completes before the write begins, and the write uses
/// a temp-file + rename so a kill mid-write leaves the original
/// intact rather than truncated.
pub fn convert_portals(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;

    let mut root: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    let portals = match root.as_object_mut() {
        Some(obj) => obj.remove("portals"),
        None => None,
    };
    let portals_array = match portals {
        Some(Value::Array(a)) => a,
        Some(_) | None => {
            // No portals field, or an unexpected shape: pass through.
            let json = serde_json::to_string_pretty(&root)
                .map_err(|e| format!("failed to serialize: {e}"))?;
            write_atomic(output_path, &json)?;
            return Ok(());
        }
    };

    let converted = portals_array.len();
    let edges = root
        .as_object_mut()
        .ok_or("map root must be a JSON object")?
        .entry("edges")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("map `edges` field is not an array")?;

    for portal in portals_array {
        let obj = match portal {
            Value::Object(o) => o,
            _ => continue,
        };
        let endpoint_a = obj
            .get("endpoint_a")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let endpoint_b = obj
            .get("endpoint_b")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // An empty `glyph` string would render as a zero-width marker
        // that's impossible to click. Treat `""` the same as missing
        // and fall back to the default marker glyph — a legacy file
        // carrying an empty glyph is almost certainly a bug in
        // whatever tool wrote it.
        let glyph = obj
            .get("glyph")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("\u{25C8}") // ◈ default
            .to_string();
        let color = obj
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("#aa88cc")
            .to_string();
        let font_size_pt = obj
            .get("font_size_pt")
            .and_then(|v| v.as_f64())
            .unwrap_or(16.0);
        let font = obj.get("font").cloned().unwrap_or(Value::Null);

        let mut glyph_connection = serde_json::Map::new();
        glyph_connection.insert("body".into(), Value::String(glyph));
        glyph_connection.insert(
            "font_size_pt".into(),
            Value::Number(
                serde_json::Number::from_f64(font_size_pt)
                    .unwrap_or_else(|| serde_json::Number::from(16)),
            ),
        );
        if !font.is_null() {
            glyph_connection.insert("font".into(), font);
        }

        let edge = json!({
            "from_id": endpoint_a,
            "to_id": endpoint_b,
            "type": "cross_link",
            "color": color,
            "width": 3,
            "line_style": "solid",
            "visible": true,
            "label": Value::Null,
            "anchor_from": "auto",
            "anchor_to": "auto",
            "control_points": Value::Array(Vec::new()),
            "glyph_connection": Value::Object(glyph_connection),
            "display_mode": "portal",
        });
        edges.push(edge);
    }

    let json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize: {e}"))?;
    write_atomic(output_path, &json)?;

    eprintln!("converted {} portal(s) to portal-mode edges", converted);
    Ok(())
}

/// Write `contents` to `path` atomically via a sibling temp file +
/// rename. Mirrors the helper in `main.rs` but returns `String`
/// errors to match this module's `Result<(), String>` API. Rename is
/// atomic on POSIX within the same filesystem, so a kill mid-write
/// leaves the original file intact instead of truncated — the
/// property `convert --portals` needs to support safe in-place
/// migration (input == output).
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid path: {}", path.display()))?
        .to_string_lossy();
    let tmp_path = dir.join(format!(
        ".{}.maptool.{}.tmp",
        file_name,
        std::process::id()
    ));
    std::fs::write(&tmp_path, contents)
        .map_err(|e| format!("failed to write {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn no_portals_field_is_noop() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t","canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},"nodes":{{}},"edges":[]}}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        assert!(out.get("portals").is_none());
    }

    #[test]
    fn empty_glyph_falls_back_to_default_marker() {
        // A legacy portal with `glyph: ""` would migrate to an edge
        // with an empty `glyph_connection.body`, rendering as a
        // zero-width marker that's impossible to interact with.
        // The converter substitutes the default marker instead.
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"","color":"#ff0","font_size_pt":16.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value =
            serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        let body = &out.get("edges").unwrap().as_array().unwrap()[0]["glyph_connection"]["body"];
        assert_eq!(body.as_str().unwrap(), "\u{25C8}");
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file_on_success() {
        // The atomic writer stages a `.<name>.maptool.<pid>.tmp`
        // file and renames it; after success the dir should only
        // contain the final output.
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t","canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},"nodes":{{}},"edges":[]}}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let dir = dst.path().parent().unwrap();
        let file_name = dst.path().file_name().unwrap().to_string_lossy().to_string();
        let pid = std::process::id();
        let leftover = dir.join(format!(".{file_name}.maptool.{pid}.tmp"));
        assert!(
            !leftover.exists(),
            "atomic writer left a temp file behind: {}",
            leftover.display()
        );
    }

    #[test]
    fn legacy_portal_becomes_portal_mode_edge() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"⬢","color":"#ff00aa","font_size_pt":20.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();

        // portals field must be gone.
        assert!(out.get("portals").is_none());
        // edges must have one entry, display_mode=portal, body=⬢.
        let edges = out.get("edges").unwrap().as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["display_mode"], "portal");
        assert_eq!(edges[0]["from_id"], "0");
        assert_eq!(edges[0]["to_id"], "1");
        assert_eq!(edges[0]["color"], "#ff00aa");
        assert_eq!(edges[0]["glyph_connection"]["body"], "⬢");
        assert_eq!(edges[0]["glyph_connection"]["font_size_pt"], 20.0);
    }
}
