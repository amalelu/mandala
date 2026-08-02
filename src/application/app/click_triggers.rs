// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `OnClick`-trigger fan-out, called by the
//! native click handler and the WASM mouse-released handler. The
//! per-section / per-node `find_triggered_mutations_at` lookup
//! and the animated-vs-instant routing are identical on both
//! platforms; only the `PlatformContext` and the `now` source
//! differ, both injected.

use baumhard::mindmap::custom_mutation::{PlatformContext, Trigger};

use crate::application::document::MindMapDocument;

/// Fire any `OnClick` triggers bound to `(node_id, hit_section)`
/// on the given `platform`. Animated triggers (duration > 0) get
/// a fresh instance via `start_animation_at(&cm, id, hit_section,
/// now_ms)`; instant triggers apply via `apply_custom_mutation`.
/// Document-actions on the trigger apply unconditionally
/// afterwards. Clears the scene-connection cache when any
/// instant mutation lands so the next rebuild re-samples.
pub(in crate::application::app) fn fire_onclick_triggers(
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    hit_node_id: &str,
    hit_section: Option<usize>,
    platform: PlatformContext,
    now_ms: u64,
) {
    let triggered = doc.find_triggered_mutations_at(hit_node_id, hit_section, &Trigger::OnClick, &platform);
    for cm in triggered {
        if cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
            // Second copy of the animated-vs-instant routing —
            // `dispatch::apply_keybind_custom_mutation` is the other,
            // and it is the one CLAUDE.md's "Dual-target status"
            // registry names for the keystroke tier. Named pre-existing
            // duplication: the two differ in the no-tree case (that
            // one returns `false` and skips `apply_document_actions`;
            // this loop applies them regardless) and in the target
            // shape (`start_animation_at` is section-aware), so
            // collapsing them decides behavior rather than moving it.
            //
            // Both stall identically on the browser: `start_animation*`
            // only queues the envelope and `drain_animation_tick` is
            // native-only.
            doc.start_animation_at(&cm, hit_node_id, hit_section, now_ms);
        } else if let Some(tree) = mindmap_tree.as_mut() {
            doc.apply_custom_mutation(&cm, hit_node_id, Some(tree));
            scene_cache.clear();
        }
        doc.apply_document_actions(&cm);
    }
}
