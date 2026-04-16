//! Remove or normalize fields that don't belong in the new format:
//! drop the `index` field from nodes, add default `channel: 0`.

use serde_json::Value;

pub fn cleanup_nodes(root: &mut Value) {
    let nodes = match root.get_mut("nodes").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };
    for node in nodes.values_mut() {
        let obj = match node.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        obj.remove("index");
        obj.entry("channel").or_insert(Value::Number(0.into()));
    }
}
