//! Legacy-format migration seam: rewrites miMind's numeric enum codes
//! as the named string values the current format expects.
//!
//! miMind encoded layout type, direction, node shape, edge line style,
//! and edge anchors as opaque integers (`0`, `1`, `2`, ...). The
//! current format uses human-readable strings (`"tree"`, `"rounded_
//! rectangle"`, `"dashed"`, ...) so a map can be read or diffed without
//! a decoder ring, and so `verify::enums` can check a node's shape
//! against a known-value list instead of a numeric range. This module
//! is the one-way translation that makes the named-enum invariant
//! reachable from legacy input — out-of-range integers fall back to the
//! safest default rather than failing the conversion.

use serde_json::Value;

pub fn convert_enums(root: &mut Value) {
    convert_node_enums(root);
    convert_edge_enums(root);
}

fn convert_node_enums(root: &mut Value) {
    let nodes = match root.get_mut("nodes").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };
    for node in nodes.values_mut() {
        convert_layout(node);
        convert_shape(node);
    }
}

fn convert_layout(node: &mut Value) {
    let layout = match node.get_mut("layout").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };

    if let Some(val) = layout.get("type").and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "map",
            1 => "tree",
            2 => "outline",
            _ => "map",
        };
        layout.insert("type".to_string(), Value::String(name.into()));
    }

    if let Some(val) = layout.get("direction").and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "auto",
            1 => "up",
            2 => "down",
            3 => "left",
            4 => "right",
            6 => "balanced",
            _ => "auto",
        };
        layout.insert("direction".to_string(), Value::String(name.into()));
    }
}

fn convert_shape(node: &mut Value) {
    let style = match node.get_mut("style").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };

    if let Some(val) = style.remove("shape_type") {
        let name = match val.as_i64() {
            Some(0) => "rectangle",
            Some(1) => "rounded_rectangle",
            Some(2) => "ellipse",
            Some(3) => "diamond",
            Some(4) => "parallelogram",
            Some(5) => "hexagon",
            _ => "rectangle",
        };
        style.insert("shape".to_string(), Value::String(name.into()));
    }
}

fn convert_edge_enums(root: &mut Value) {
    let edges = match root.get_mut("edges").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };
    for edge in edges.iter_mut() {
        let obj = match edge.as_object_mut() {
            Some(o) => o,
            None => continue,
        };

        if let Some(val) = obj.get("line_style").and_then(|v| v.as_i64()) {
            let name = match val {
                0 => "solid",
                1 => "dashed",
                _ => "solid",
            };
            obj.insert("line_style".to_string(), Value::String(name.into()));
        }

        convert_anchor(obj, "anchor_from");
        convert_anchor(obj, "anchor_to");
    }
}

fn convert_anchor(obj: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(val) = obj.get(field).and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "auto",
            1 => "top",
            2 => "right",
            3 => "bottom",
            4 => "left",
            _ => "auto",
        };
        obj.insert(field.to_string(), Value::String(name.into()));
    }
}
