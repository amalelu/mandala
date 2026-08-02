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

/// Simplify each node's color_schema: keep palette (as key), level,
/// starts_at_root, connections_colored. Drop groups, theme_id, variant.
fn simplify_node_schemas(root: &mut Value) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };

    for node in nodes.values_mut() {
        let schema = match node.get("color_schema") {
            Some(Value::Object(obj)) => obj.clone(),
            _ => continue,
        };

        let key = palette_key(&schema);
        let level = schema.get("level").cloned().unwrap_or(json!(0));
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
