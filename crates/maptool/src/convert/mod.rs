//! One-way migration from the miMind-derived legacy `.mindmap.json`
//! format to the current one.
//!
//! The current format's loader has no runtime compatibility shim for
//! legacy files: `portals[]` is rejected outright by the loader and
//! every other legacy-shaped field (opaque integer IDs, enum codes,
//! inlined palettes, `index`) trips serde's own type mismatch on
//! parse. Either way, an unmigrated file does not load. This module
//! is how a user crosses the one-way door: each submodule performs
//! one orthogonal transform (IDs, enums, palettes, cleanup) and the
//! whole pipeline runs in a fixed order so later passes can assume
//! the earlier ones have already landed.

mod cleanup;
mod enums;
mod ids;
mod palettes;
mod portals;

pub use portals::convert_portals;

use serde_json::Value;
use std::path::Path;

/// Read a legacy `.mindmap.json`, convert it to the current format, and
/// write the result to `output_path`.
pub fn convert_legacy(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;

    let mut root: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    // 1. Assign Dewey-decimal IDs and rewrite all references.
    let nodes = root
        .get("nodes")
        .and_then(|v| v.as_object())
        .ok_or("missing or invalid \"nodes\" object")?;
    let id_map = ids::assign_dewey_ids(nodes);
    ids::rewrite_ids(&mut root, &id_map);

    // 2. Convert integer enums to named strings.
    enums::convert_enums(&mut root);

    // 3. Hoist color schemas into top-level palettes.
    palettes::hoist_palettes(&mut root);

    // 4. Drop index, add channel.
    cleanup::cleanup_nodes(&mut root);

    // Write output with sorted keys for deterministic output.
    let json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize: {e}"))?;

    std::fs::write(output_path, &json)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    eprintln!(
        "converted {} nodes, {} edges",
        id_map.len(),
        root.get("edges")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );

    Ok(())
}
