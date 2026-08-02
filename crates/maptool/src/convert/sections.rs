// SPDX-License-Identifier: MPL-2.0

//! Section-shape migration — moves a legacy node's `text` +
//! `text_runs` into a single-element `sections[]` array on the
//! same node. Idempotent: a node already carrying `sections` is
//! left alone, so running the converter twice is safe.
//!
//! Operates at the `serde_json::Value` level so legacy maps that
//! the typed `load_from_str` would now reject (per CODE_CONVENTIONS
//! §10 "no dual shapes") still flow through cleanly. Read, transform,
//! and write are the shared `super::transform_map_file` scaffold —
//! the same one `convert_portals` and `convert_legacy` use — so this
//! verb also lands on disk through the atomic staging-file + rename.

use serde_json::{Map, Value};
use std::path::Path;

use super::nodes_obj_mut;

/// Read a legacy `.mindmap.json`, fold each node's `text` /
/// `text_runs` pair into a default single section under
/// `sections[0]`, and write the result to `output_path`.
///
/// Output `sections[0]` shape: `{ "text": <node.text>, "text_runs":
/// <node.text_runs> }` — `offset`, `size`, and `channel` are
/// omitted (they default to `(0, 0)`, "fill the parent", and `0`
/// respectively, all `skip_serializing_if`-guarded on the typed
/// side).
///
/// Idempotency: a node that already has `sections` (and no
/// `text` / `text_runs`) is left untouched — no double-wrapping.
/// A node with both `sections` *and* legacy `text` is treated as
/// a partial migration: legacy fields are dropped and the
/// existing sections remain authoritative.
///
/// Input and output may be the same path — the write is atomic, so an
/// interrupted in-place migration cannot truncate the user's only copy.
pub fn convert_sections(input_path: &Path, output_path: &Path) -> Result<(), String> {
    super::transform_map_file(input_path, output_path, |root| {
        let mut migrated = 0usize;
        if let Some(nodes) = nodes_obj_mut(root) {
            for (_id, node) in nodes.iter_mut() {
                if migrate_one_node(node) {
                    migrated += 1;
                }
            }
        }
        Ok(format!("converted {migrated} nodes into section-bearing shape"))
    })
}

/// Migrate one node's JSON object. Returns `true` when the node
/// shape changed — used by the caller for the summary log.
///
/// Order of operations matters: the legacy `text` / `text_runs`
/// values are *moved* (not copied) into the new section so a
/// round-trip through the typed loader matches the on-disk file
/// byte-for-byte (no orphaned legacy keys remain).
pub(super) fn migrate_one_node(node: &mut Value) -> bool {
    let Some(obj) = node.as_object_mut() else {
        return false;
    };

    let already_has_sections = obj
        .get("sections")
        .and_then(|s| s.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let legacy_text = obj.remove("text");
    let legacy_runs = obj.remove("text_runs");

    if already_has_sections {
        // Partial-migration case: keep the existing sections,
        // drop any stray legacy fields. Fired silently — the
        // summary line counts only fresh migrations.
        return legacy_text.is_some() || legacy_runs.is_some();
    }

    // Build the default section. `MindSection.text` carries
    // `#[serde(default)]` so a missing key parses as empty, but
    // we synthesize the key explicitly here to keep the on-disk
    // shape predictable for downstream tooling (grep / show /
    // hand-inspection). Pre-fix this comment claimed the default
    // was implicit without `#[serde(default)]` — wrong; the
    // attribute is now present and load-bearing for hand-edited
    // partial-migration files.
    let mut section = Map::new();
    let text = legacy_text.unwrap_or_else(|| Value::String(String::new()));
    section.insert("text".to_string(), text);
    if let Some(runs) = legacy_runs {
        // Skip serializing an empty runs array — matches the
        // typed `MindSection`'s `skip_serializing_if =
        // "Vec::is_empty"` so converted maps stay byte-stable
        // against unconverted-but-otherwise-identical sibling
        // sections.
        let is_empty = runs.as_array().map(|a| a.is_empty()).unwrap_or(true);
        if !is_empty {
            section.insert("text_runs".to_string(), runs);
        }
    }

    obj.insert("sections".to_string(), Value::Array(vec![Value::Object(section)]));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_migrate_one_node_lifts_text_and_runs() {
        let mut node = json!({
            "id": "0",
            "text": "Hello",
            "text_runs": [{ "start": 0, "end": 5, "color": "#fff" }],
            "size": { "width": 100, "height": 40 },
        });
        assert!(migrate_one_node(&mut node));

        let obj = node.as_object().unwrap();
        assert!(obj.get("text").is_none(), "legacy text removed");
        assert!(obj.get("text_runs").is_none(), "legacy text_runs removed");
        let sections = obj.get("sections").unwrap().as_array().unwrap();
        assert_eq!(sections.len(), 1);
        let s0 = sections[0].as_object().unwrap();
        assert_eq!(s0.get("text").unwrap().as_str(), Some("Hello"));
        let runs = s0.get("text_runs").unwrap().as_array().unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_migrate_one_node_skips_empty_runs() {
        let mut node = json!({
            "id": "0",
            "text": "Hi",
            "text_runs": [],
        });
        migrate_one_node(&mut node);
        let s0 = node.get("sections").unwrap().as_array().unwrap()[0]
            .as_object()
            .unwrap();
        assert!(s0.get("text_runs").is_none(), "empty runs not serialized");
    }

    #[test]
    fn test_migrate_one_node_idempotent_for_already_migrated() {
        let mut node = json!({
            "id": "0",
            "sections": [{"text": "already here"}],
        });
        let changed = migrate_one_node(&mut node);
        assert!(!changed, "no legacy fields, no migration");
        let sections = node.get("sections").unwrap().as_array().unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].get("text").unwrap().as_str(), Some("already here"));
    }

    #[test]
    fn test_migrate_one_node_drops_legacy_when_sections_present() {
        let mut node = json!({
            "id": "0",
            "text": "stale",
            "sections": [{"text": "fresh"}],
        });
        let changed = migrate_one_node(&mut node);
        assert!(changed);
        assert!(node.get("text").is_none());
        let sections = node.get("sections").unwrap().as_array().unwrap();
        assert_eq!(sections[0].get("text").unwrap().as_str(), Some("fresh"));
    }

    #[test]
    fn test_migrate_one_node_text_defaults_to_empty() {
        let mut node = json!({"id": "0"});
        migrate_one_node(&mut node);
        let s0 = node.get("sections").unwrap().as_array().unwrap()[0]
            .as_object()
            .unwrap();
        assert_eq!(s0.get("text").unwrap().as_str(), Some(""));
    }
}
