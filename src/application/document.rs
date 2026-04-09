use std::path::Path;
use log::{error, info};
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
}

impl MindMapDocument {
    /// Load a MindMap from a file path and create a Document.
    pub fn load(path: &str) -> Result<Self, String> {
        match loader::load_from_file(Path::new(path)) {
            Ok(map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
                Ok(MindMapDocument {
                    mindmap: map,
                    file_path: Some(path.to_string()),
                    dirty: false,
                })
            }
            Err(e) => {
                let msg = format!("Failed to load mindmap '{}': {}", path, e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    /// Build a Baumhard mutation tree from the MindMap hierarchy.
    /// Each MindNode becomes a GlyphArea in the tree, preserving parent-child structure.
    pub fn build_tree(&self) -> MindMapTree {
        tree_builder::build_mindmap_tree(&self.mindmap)
    }

    /// Build a RenderScene from the current MindMap state.
    /// Used for connections and borders (flat pipeline).
    pub fn build_scene(&self) -> RenderScene {
        scene_builder::build_scene(&self.mindmap)
    }
}
