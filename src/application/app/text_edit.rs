//! Inline node text editor (Session 7A): open / close / handle key /
//! apply preview-to-tree. The text editor is a multi-line in-place
//! buffer whose cursor + content live on `TextEditState::Open` and
//! whose preview is stamped into the live Baumhard tree via
//! `apply_text_edit_to_tree` so the user sees their typing on every
//! keystroke without touching the model. Commit on Esc folds the
//! buffer into `MindNode.text` via `MindMapDocument::set_node_text`.

use winit::keyboard::Key;

use baumhard::util::grapheme_chad;

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::{
    cursor_to_line_end, cursor_to_line_start, delete_at_cursor, delete_before_cursor,
    insert_at_cursor, insert_caret, move_cursor_down_line, move_cursor_up_line, rebuild_all,
    TextEditState,
};

// =====================================================================
// Session 7A: inline node text editor
// =====================================================================

/// Open the text editor on the given node. Seeds the buffer (empty if
/// `from_creation`, else the node's current text), and pushes the
/// initial caret through the Baumhard mutation pipeline so the live
/// tree shows the cursor on the next frame.
///
/// No snapshot of the node's pre-edit text is stored on
/// `TextEditState`: the model is untouched during typing, so the
/// model itself *is* the pre-edit state. `set_node_text` takes its
/// own "before" snapshot at commit time, and cancel just rebuilds
/// the tree from the unchanged model.
pub(in crate::application::app) fn open_text_edit(
    node_id: &str,
    from_creation: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    _app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let current_text = match doc.mindmap.nodes.get(node_id) {
        Some(n) => n.text.clone(),
        None => return,
    };
    let buffer = if from_creation { String::new() } else { current_text };
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    // Seed `buffer_regions` from the tree's current `area.regions`,
    // which the tree builder populated from the node's `text_runs`.
    // The tree is the source of truth for regions during an edit
    // session; the model is frozen until commit. `from_creation`
    // nodes have no prior regions, so we start from empty.
    let buffer_regions = if from_creation {
        baumhard::core::primitives::ColorFontRegions::new_empty()
    } else {
        read_node_regions(mindmap_tree.as_ref(), node_id).unwrap_or_default()
    };
    // Snapshot the tree's pre-edit text + regions so cancel can
    // apply them back as a delta instead of triggering `rebuild_all`.
    // `from_creation` nodes were just inserted — the tree's current
    // `area.text` is `node.text` (empty) and regions are empty.
    let original_text = read_node_text(mindmap_tree.as_ref(), node_id).unwrap_or_default();
    let original_regions = buffer_regions.clone();
    *text_edit_state = TextEditState::Open {
        node_id: node_id.to_string(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        buffer_regions: buffer_regions.clone(),
        original_text,
        original_regions,
    };
    // Push the initial (caret-only for creation, or "existing text +
    // caret at end" for edit) through the Baumhard mutation pipeline.
    apply_text_edit_to_tree(
        node_id,
        &buffer,
        &buffer_regions,
        cursor_grapheme_pos,
        mindmap_tree,
        renderer,
    );
}

/// Read a node's `GlyphArea::regions` off the live tree. Returns
/// `None` when the tree or the node isn't present, or when the
/// target element isn't a `GlyphArea` (it's a `GlyphModel` for
/// multi-line node containers). The text-edit path uses this to
/// seed `TextEditState::Open::buffer_regions` at open time so
/// per-run color and `AppFont` pins survive the edit lifecycle.
pub(in crate::application::app) fn read_node_regions(
    mindmap_tree: Option<&baumhard::mindmap::tree_builder::MindMapTree>,
    node_id: &str,
) -> Option<baumhard::core::primitives::ColorFontRegions> {
    let tree = mindmap_tree?;
    let nid = *tree.node_map.get(node_id)?;
    let element = tree.tree.arena.get(nid)?.get();
    element.glyph_area().map(|a| a.regions.clone())
}

/// Read a node's `GlyphArea::text` off the live tree. Pairs with
/// [`read_node_regions`] — together they capture the pre-edit
/// snapshot the cancel path restores via `DeltaGlyphArea`.
pub(in crate::application::app) fn read_node_text(
    mindmap_tree: Option<&baumhard::mindmap::tree_builder::MindMapTree>,
    node_id: &str,
) -> Option<String> {
    let tree = mindmap_tree?;
    let nid = *tree.node_map.get(node_id)?;
    let element = tree.tree.arena.get(nid)?.get();
    element.glyph_area().map(|a| a.text.clone())
}

/// Apply a snapshot of `(text, regions)` back to the live tree's
/// `GlyphArea` for `node_id`, via a `DeltaGlyphArea` with `Assign`
/// semantics. Used by the text-editor cancel path to revert the
/// tree to its pre-edit state without going through the full
/// `rebuild_all` (which rebuilds every node from the model and
/// re-walks the scene). Returns early on the usual "node
/// disappeared" edge cases.
pub(in crate::application::app) fn revert_node_text_on_tree(
    node_id: &str,
    text: String,
    regions: baumhard::core::primitives::ColorFontRegions,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use baumhard::core::primitives::{Applicable, ApplyOperation};
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};

    let tree = match mindmap_tree.as_mut() {
        Some(t) => t,
        None => return,
    };
    let indextree_node_id = match tree.node_map.get(node_id) {
        Some(id) => *id,
        None => return,
    };
    let element = match tree.tree.arena.get_mut(indextree_node_id) {
        Some(n) => n.get_mut(),
        None => return,
    };
    let area = match element.glyph_area_mut() {
        Some(a) => a,
        None => return,
    };

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(text),
        GlyphAreaField::ColorFontRegions(regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(area);
    renderer.rebuild_buffers_from_tree(&tree.tree);
}

/// Session 7A: commit or cancel the open text editor.
///
/// - **Commit**: writes the final buffer back to the model via
///   `set_node_text` (no-op on unchanged text, handles its own undo
///   push), then `rebuild_all` to pull the tree back to the freshly
///   mutated model.
/// - **Cancel**: applies the `(original_text, original_regions)`
///   snapshot captured at open time as a `DeltaGlyphArea` to the
///   edited node. The model is untouched during editing, so the rest
///   of the tree + scene are already in sync — no `rebuild_all` is
///   needed. This skips the `doc.build_tree()` walk and the full
///   `rebuild_scene_only` (connections, borders, portals, labels,
///   edge handles), which matters on maps with many nodes.
pub(in crate::application::app) fn close_text_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let snapshot = match std::mem::replace(text_edit_state, TextEditState::Closed) {
        TextEditState::Open {
            node_id,
            buffer,
            original_text,
            original_regions,
            ..
        } => (node_id, buffer, original_text, original_regions),
        TextEditState::Closed => return,
    };
    let (node_id, buffer, original_text, original_regions) = snapshot;
    if commit {
        doc.set_node_text(&node_id, buffer);
        // Commit changed the model — pull the tree back to it.
        rebuild_all(doc, mindmap_tree, app_scene, renderer);
    } else {
        // Cancel: model is untouched, so we only need to revert the
        // edited node's transient caret-bearing text/regions to the
        // pre-edit snapshot. Scene elements (borders, connections,
        // etc.) were never mutated during the edit session.
        revert_node_text_on_tree(
            &node_id,
            original_text,
            original_regions,
            mindmap_tree,
            renderer,
        );
    }
}

/// Session 7A: push the current (`buffer`, `cursor`) state into the
/// live Baumhard tree via a `Mutation::AreaDelta { text: Assign }`
/// targeting the edited node's GlyphArea. This is the "utilize
/// Baumhard" path — the buffer is transient UI state on the app
/// layer, but every visual frame goes through the existing
/// `Mutation::apply_to_area` vocabulary. The renderer's text buffers
/// are rebuilt from the mutated tree so the next frame reflects the
/// keystroke.
pub(in crate::application::app) fn apply_text_edit_to_tree(
    node_id: &str,
    buffer: &str,
    buffer_regions: &baumhard::core::primitives::ColorFontRegions,
    cursor_grapheme_pos: usize,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::core::primitives::{
        Applicable, ApplyOperation, ColorFontRegion, Range,
    };

    let tree = match mindmap_tree.as_mut() {
        Some(t) => t,
        None => return,
    };
    let indextree_node_id = match tree.node_map.get(node_id) {
        Some(id) => *id,
        None => return,
    };
    // Grab a mutable handle to the target node's GlyphArea.
    let element = tree.tree.arena.get_mut(indextree_node_id);
    let element = match element {
        Some(n) => n.get_mut(),
        None => return,
    };
    let area = match element.glyph_area_mut() {
        Some(a) => a,
        None => return,
    };

    // Compose display-text regions from the canonical buffer regions
    // via Baumhard's `insert_regions_at` primitive: the caret glyph
    // is a one-char structural insertion at `cursor_grapheme_pos`
    // that the surrounding run should absorb (so the caret inherits
    // its color and — importantly — its `AppFont` pin, keeping
    // per-script glyphs rendering correctly). If no region absorbs
    // the caret (empty buffer, cursor at an uncovered position), we
    // `set_or_insert` a blank region for it so it still renders.
    let display_text = insert_caret(buffer, cursor_grapheme_pos);
    let mut display_regions = buffer_regions.clone();
    let absorbed = display_regions.insert_regions_at(cursor_grapheme_pos, 1);
    if !absorbed {
        display_regions.set_or_insert(&ColorFontRegion::new(
            Range::new(cursor_grapheme_pos, cursor_grapheme_pos + 1),
            None,
            None,
        ));
    }

    // Construct the Baumhard delta: Text + ColorFontRegions + Assign.
    // The Assign operation replaces both fields wholesale — see
    // `GlyphArea::apply_operation` at area.rs:261 for regions and
    // area.rs:273 for text.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(display_text),
        GlyphAreaField::ColorFontRegions(display_regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(area);

    // Re-shape the node buffers off the mutated tree. This is the
    // existing tree-render path, reused.
    renderer.rebuild_buffers_from_tree(&tree.tree);
}

/// Session 7A: route a keystroke to the open node text editor. All
/// keys are stolen from normal keybind dispatch — Tab and Enter
/// produce literal characters, Esc cancels, arrows/Home/End navigate,
/// Backspace/Delete delete, and printable chars are inserted at the
/// cursor. Every successful mutation is pushed through
/// `apply_text_edit_to_tree` so the tree and renderer stay in sync.
pub(in crate::application::app) fn handle_text_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    text_edit_state: &mut TextEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    if name == Some("escape") {
        close_text_edit(false, doc, text_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }

    let (node_id, buffer, cursor, regions) = match text_edit_state {
        TextEditState::Open {
            node_id,
            buffer,
            cursor_grapheme_pos,
            buffer_regions,
            ..
        } => (node_id, buffer, cursor_grapheme_pos, buffer_regions),
        TextEditState::Closed => return,
    };

    let mut changed = false;
    match name {
        Some("backspace") => {
            if *cursor > 0 {
                // Delete grapheme at `cursor - 1`. `shrink_regions_after`
                // rewrites ranges so per-run color / `AppFont` pins
                // survive the deletion — a single-char cut never
                // collapses a straddling run, it just shrinks its end.
                regions.shrink_regions_after(*cursor - 1, 1);
                *cursor = delete_before_cursor(buffer, *cursor);
                changed = true;
            }
        }
        Some("delete") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                regions.shrink_regions_after(*cursor, 1);
                *cursor = delete_at_cursor(buffer, *cursor);
                changed = true;
            }
        }
        Some("arrowleft") => {
            if *cursor > 0 {
                *cursor -= 1;
                changed = true;
            }
        }
        Some("arrowright") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
                changed = true;
            }
        }
        Some("arrowup") => {
            let new_cursor = move_cursor_up_line(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("arrowdown") => {
            let new_cursor = move_cursor_down_line(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("home") => {
            let new_cursor = cursor_to_line_start(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("end") => {
            let new_cursor = cursor_to_line_end(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("enter") => {
            regions.insert_regions_at(*cursor, 1);
            *cursor = insert_at_cursor(buffer, *cursor, '\n');
            changed = true;
        }
        Some("tab") => {
            regions.insert_regions_at(*cursor, 1);
            *cursor = insert_at_cursor(buffer, *cursor, '\t');
            changed = true;
        }
        _ => {
            // Printable character: accept each non-control char. Mirrors
            // `handle_label_edit_key` at app.rs ~line 1929.
            if let Key::Character(c) = logical_key {
                for ch in c.as_str().chars() {
                    if !ch.is_control() {
                        regions.insert_regions_at(*cursor, 1);
                        *cursor = insert_at_cursor(buffer, *cursor, ch);
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        // Text editing only mutates the live tree during typing; the
        // model is untouched until commit (click-outside) or rolled
        // back on cancel (Esc). We clone node_id + buffer to release
        // the mutable borrow on `text_edit_state` before calling
        // `apply_text_edit_to_tree`, which wants its own mutable
        // borrow on `mindmap_tree`.
        let node_id_owned = node_id.clone();
        let buffer_owned = buffer.clone();
        let regions_owned = regions.clone();
        let cursor_snapshot = *cursor;
        apply_text_edit_to_tree(
            &node_id_owned,
            &buffer_owned,
            &regions_owned,
            cursor_snapshot,
            mindmap_tree,
            renderer,
        );
    }
}

