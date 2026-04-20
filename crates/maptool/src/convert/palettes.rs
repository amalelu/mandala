//! Hoist per-node color_schema groups into a top-level `palettes` map.
//!
//! miMind stored the full group list on every node that carried a
//! palette, so a single palette shared across dozens of nodes was
//! duplicated dozens of times in the file. The current format names
//! each palette once at the map level and references it by key from
//! each node — same visual result, diffs that survive palette tweaks.
//! Each node's color_schema is rewritten to the reference form
//! (palette key + level + flags). `theme_id` is dropped; `variant` is
//! folded into the key name so two schemas that differ only in variant
//! get distinct palette entries instead of clobbering each other.

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

        let palette_name = palette_key(schema);
        if !palettes.contains_key(&palette_name) {
            palettes.insert(palette_name, json!({ "groups": groups }));
        }
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
    let palette_obj: serde_json::Map<String, Value> = palettes
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(obj) = root.as_object_mut() {
        obj.insert("palettes".to_string(), Value::Object(palette_obj));
    }
}

/// Simplify each node's color_schema: keep palette (as key), level,
/// starts_at_root, connections_colored. Drop groups, theme_id, variant.
fn simplify_node_schemas(root: &mut Value) {
    let nodes = match root.get_mut("nodes").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };

    for node in nodes.values_mut() {
        let schema = match node.get("color_schema") {
            Some(Value::Object(obj)) => obj.clone(),
            _ => continue,
        };

        let key = palette_key(&schema);
        let level = schema.get("level").cloned().unwrap_or(json!(0));
        let starts_at_root = schema.get("starts_at_root").cloned().unwrap_or(json!(true));
        let connections_colored = schema
            .get("connections_colored")
            .cloned()
            .unwrap_or(json!(true));

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
