//! Legacy format converter: transforms a miMind-derived `.mindmap.json`
//! into the current format with structural IDs, named enums, hoisted
//! palettes, and channel support.

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
