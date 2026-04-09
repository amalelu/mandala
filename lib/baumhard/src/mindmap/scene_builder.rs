use crate::mindmap::border::BorderStyle;
use crate::mindmap::model::{MindMap, TextRun};

/// Intermediate representation between MindMap data and GPU rendering.
/// Produced by `build_scene()`, consumed by Renderer to create cosmic-text buffers.
pub struct RenderScene {
    pub text_elements: Vec<TextElement>,
    pub border_elements: Vec<BorderElement>,
    pub connection_elements: Vec<ConnectionElement>,
    pub portal_elements: Vec<PortalElement>,
    pub background_color: String,
}

/// A visible text node to be rendered.
pub struct TextElement {
    pub node_id: String,
    pub text: String,
    pub text_runs: Vec<TextRun>,
    pub position: (f32, f32),
    pub size: (f32, f32),
}

/// A border to be rendered around a node.
pub struct BorderElement {
    pub node_id: String,
    pub border_style: BorderStyle,
    pub node_position: (f32, f32),
    pub node_size: (f32, f32),
}

/// Placeholder for future connection rendering (M2).
pub struct ConnectionElement {}

/// Placeholder for future portal rendering (M3).
pub struct PortalElement {}

/// Builds a RenderScene from a MindMap, determining which nodes and borders
/// are visible (accounting for fold state) and extracting their layout data.
pub fn build_scene(map: &MindMap) -> RenderScene {
    let mut text_elements = Vec::new();
    let mut border_elements = Vec::new();

    for node in map.nodes.values() {
        if map.is_hidden_by_fold(node) {
            continue;
        }

        // Text element (skip empty text nodes)
        if !node.text.is_empty() {
            text_elements.push(TextElement {
                node_id: node.id.clone(),
                text: node.text.clone(),
                text_runs: node.text_runs.clone(),
                position: (node.position.x as f32, node.position.y as f32),
                size: (node.size.width as f32, node.size.height as f32),
            });
        }

        // Border element
        if node.style.show_frame {
            let border_style = BorderStyle::default_with_color(&node.style.frame_color);
            border_elements.push(BorderElement {
                node_id: node.id.clone(),
                border_style,
                node_position: (node.position.x as f32, node.position.y as f32),
                node_size: (node.size.width as f32, node.size.height as f32),
            });
        }
    }

    RenderScene {
        text_elements,
        border_elements,
        connection_elements: Vec::new(),
        portal_elements: Vec::new(),
        background_color: map.canvas.background_color.clone(),
    }
}
