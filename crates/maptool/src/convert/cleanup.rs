//! Field-shape cleanup that runs after IDs, enums, and palettes have
//! been rewritten.
//!
//! miMind tracked sibling order with an explicit `index` integer; the
//! current format encodes that ordering in the Dewey-decimal ID itself
//! (`0.3` is the fourth child of `0`), so once IDs have been assigned
//! `index` is redundant and is removed. `channel` is a new field the
//! legacy format didn't carry — every node defaults to the base channel
//! (`0`) so the output validates against the post-migration invariants.

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
