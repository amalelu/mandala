// SPDX-License-Identifier: MPL-2.0

//! Hoist per-node color_schema groups into a top-level `palettes`
//! map; rewrite each node to reference its palette by key. `theme_id`
//! is dropped; `variant != 2` is folded into the key (`coral-v3`).

use serde_json::{json, Value};
use std::collections::HashMap;

pub fn hoist_palettes(root: &mut Value) {
    let palettes = collect_palettes(root);
    inject_palettes(root, &palettes);
    simplify_node_schemas(root);
}

/// Scan all nodes for level-0 color_schema entries with non-empty groups.
/// Build a palette map keyed by palette name. When variant differs, fold
/// it into the name (e.g. "coral", "coral-alt").
fn collect_palettes(root: &Value) -> HashMap<String, Value> {
    let mut palettes: HashMap<String, Value> = HashMap::new();

    let nodes = match root.get("nodes").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => return palettes,
    };

    for node in nodes.values() {
        let schema = match node.get("color_schema") {
            Some(Value::Object(obj)) => obj,
            _ => continue,
        };

        let level = schema.get("level").and_then(|v| v.as_i64()).unwrap_or(-1);
        if level != 0 {
            continue;
        }

        let groups = match schema.get("groups").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr.clone(),
            _ => continue,
        };

        // First-writer-wins on a name collision, as
        // `format/migration.md` §"Known limitations" documents. One
        // hash lookup via `entry` rather than the `contains_key` +
        // `insert` pair this used to do (clippy::map_entry).
        let palette_name = palette_key(schema);
        palettes
            .entry(palette_name)
            .or_insert_with(|| json!({ "groups": groups }));
    }

    palettes
}

/// Derive a palette key from a color_schema object. Uses the palette
/// name, with variant folded in for non-standard variants.
fn palette_key(schema: &serde_json::Map<String, Value>) -> String {
    let base = schema
        .get("palette")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let variant = schema.get("variant").and_then(|v| v.as_i64()).unwrap_or(2);
    if variant == 2 {
        base.to_string()
    } else {
        format!("{}-v{}", base, variant)
    }
}

fn inject_palettes(root: &mut Value, palettes: &HashMap<String, Value>) {
    if palettes.is_empty() {
        return;
    }
    let palette_obj: serde_json::Map<String, Value> =
        palettes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(obj) = root.as_object_mut() {
        obj.insert("palettes".to_string(), Value::Object(palette_obj));
    }
}

/// Read a legacy `level` as the non-negative depth the current model
/// requires (`ColorSchema.level: usize`).
///
/// The legacy field was a signed integer and the model's was too, so
/// nothing upstream ever had to mean it. A negative depth is not a
/// depth — it is corruption or a hand edit — and emitting one now
/// produces a file `maptool convert` itself cannot load back. Floor it
/// at 0, which is the schema root, and say so: the conversion still
/// succeeds and the author learns which node to look at. A missing or
/// non-integer field defaults to 0 silently, as it always has.
fn nonnegative_level(raw: Option<&Value>, node_id: &str) -> u64 {
    match raw.and_then(Value::as_i64) {
        Some(level) if level < 0 => {
            eprintln!(
                "convert: node {node_id} carries a negative color_schema.level ({level}); \
                 flooring to 0 — depth below the schema root has no meaning"
            );
            0
        }
        Some(level) => level as u64,
        None => 0,
    }
}

/// Simplify each node's color_schema: keep palette (as key), level,
/// starts_at_root, connections_colored. Drop groups, theme_id, variant.
fn simplify_node_schemas(root: &mut Value) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };

    for (node_id, node) in nodes.iter_mut() {
        let schema = match node.get("color_schema") {
            Some(Value::Object(obj)) => obj.clone(),
            _ => continue,
        };

        let key = palette_key(&schema);
        let level = json!(nonnegative_level(schema.get("level"), node_id));
        let starts_at_root = schema.get("starts_at_root").cloned().unwrap_or(json!(true));
        let connections_colored = schema.get("connections_colored").cloned().unwrap_or(json!(true));

        let simplified = json!({
            "palette": key,
            "level": level,
            "starts_at_root": starts_at_root,
            "connections_colored": connections_colored
        });

        if let Some(obj) = node.as_object_mut() {
            obj.insert("color_schema".to_string(), simplified);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::hoist_palettes;
    use serde_json::{json, Value};

    fn legacy_map_with_level(level: Value) -> Value {
        json!({
            "version": "1.0",
            "name": "t",
            "canvas": {"background_color": "#000"},
            "nodes": {
                "0": {
                    "color_schema": {
                        "palette": "coral",
                        "variant": 2,
                        "level": level,
                        "starts_at_root": true,
                        "connections_colored": true,
                        "groups": [
                            {"background": "#a9decb", "frame": "#30b082", "text": "#000000", "title": "#000000"}
                        ]
                    }
                }
            },
            "edges": []
        })
    }

    fn converted_level(level: Value) -> i64 {
        let mut root = legacy_map_with_level(level);
        hoist_palettes(&mut root);
        root["nodes"]["0"]["color_schema"]["level"]
            .as_i64()
            .expect("level must survive as an integer")
    }

    #[test]
    fn test_hoist_palettes_keeps_a_nonnegative_level() {
        assert_eq!(converted_level(json!(0)), 0);
        assert_eq!(converted_level(json!(4)), 4);
    }

    /// `ColorSchema.level` is a `usize` in the model, so a converted
    /// file carrying a negative depth would not load back — and a
    /// converter whose output its own loader refuses is worse than
    /// one that says what it changed.
    #[test]
    fn test_hoist_palettes_floors_a_negative_level_to_the_schema_root() {
        assert_eq!(converted_level(json!(-1)), 0);
        assert_eq!(converted_level(json!(-9000)), 0);
    }

    #[test]
    fn test_hoist_palettes_defaults_a_missing_level_to_zero() {
        let mut root = legacy_map_with_level(json!(3));
        root["nodes"]["0"]["color_schema"]
            .as_object_mut()
            .unwrap()
            .remove("level");
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["level"], json!(0));
    }
}
