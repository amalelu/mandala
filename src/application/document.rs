use std::path::Path;
use log::{error, info};
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};

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

    /// Build a RenderScene from the current MindMap state.
    pub fn build_scene(&self) -> RenderScene {
        scene_builder::build_scene(&self.mindmap)
    }
}
