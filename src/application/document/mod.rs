// SPDX-License-Identifier: MPL-2.0

//! `MindMapDocument` — owns the data model (`MindMap`, selection,
//! undo stack, animation state, mutation registry, transient
//! previews) and hands intermediate representations to the
//! renderer. Behavior is sharded across sibling submodules; this
//! file carries only the struct definition, construction, and the
//! scene-build entry points.

use std::collections::HashMap;
use std::path::Path;

use log::info;

use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::loader;
use baumhard::mindmap::model::{MindMap, MAX_NODE_AXIS};
use baumhard::mindmap::placard;
use baumhard::mindmap::tree_builder::{
    self, effective_section_scale, FrameOverrides, MindMapTree, LINE_HEIGHT_FACTOR,
};

use crate::application::source_tier::SourceTier;

pub mod animations;
mod custom;
pub(in crate::application) mod defaults;
mod edges;
// Pointer hit-testing and drag/resize tree patching. Every caller
// is a native-gated mouse handler, so on wasm32 the module
// compiles unreferenced; the lint stays armed on the host.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod hit_test;
pub mod mutations;
pub mod mutations_loader;
mod nodes;
mod topology;
mod types;
mod undo;
mod undo_action;
mod zoom_bounds;

#[cfg(test)]
pub(crate) mod tests_common;
#[cfg(test)]
mod tests_delete;
#[cfg(test)]
mod tests_edges_chain;
#[cfg(test)]
mod tests_edges_style;
#[cfg(test)]
mod tests_hit_move;
#[cfg(test)]
mod tests_mutations;
#[cfg(test)]
mod tests_nodes;
#[cfg(test)]
mod tests_reparent;
#[cfg(test)]
mod tests_resize;
#[cfg(test)]
mod tests_selection;

// Cross-platform: consumers (`scene_rebuild.rs`, `event_mouse_click.rs`,
// `run_wasm/`, `scene_host.rs`) compile on both targets. The
// plain `hit_test` (Option<String> shape) is reachable only via
// the native click handler today; the WASM click handler routes
// through `hit_test_target` (HitTarget enum). Gating the
// `hit_test` re-export to non-wasm silences the
// `#[warn(unused_imports)]` the WASM build would otherwise raise
// for an unused-on-wasm name.
#[cfg(not(target_arch = "wasm32"))]
pub use hit_test::hit_test;
pub use hit_test::{
    apply_inactive_node_dimming, apply_tree_highlights, hit_test_target, point_in_node_aabb, HitTarget,
};
// Cross-platform: a connection path is the last rung of the tap's
// hit chain (`dispatch::cross_dispatch::pointer`), which is one body
// on both targets. It was in the native-only group below for as long
// as native's click router was its only caller.
pub use hit_test::hit_test_edge;
// Native-only: consumed by drag handlers, the click router, and
// rect-select drain — none reachable on WASM today.
#[cfg(not(target_arch = "wasm32"))]
pub use hit_test::{
    apply_drag_delta, apply_drag_delta_and_collect_patches, apply_node_resize_to_tree,
    apply_section_drag_delta_and_collect_patches, apply_section_resize_to_tree, hit_test_node_resize_handle,
    hit_test_section_resize_handle, rect_select,
};
pub use nodes::{
    BorderConfigEdits, BorderEditOutcome, BorderPreview, BorderPreviewTarget, BorderSide, OptionEdit,
    SectionPayload,
};
pub use types::{
    AnimationInstance, EdgeLabelSel, EdgeRef, GraphemeRange, PortalLabelSel, SectionSel, SectionSpan,
    SelectionState, HIGHLIGHT_COLOR,
};
// `InteractionModeOverrides` lives in baumhard (next to the
// `SceneSelectionContext` it composes into). Re-exported here so
// callers across the application crate that already
// `use crate::application::document::*` for the doc API don't have
// to reach across into baumhard's tree_builder for the value type.
pub use baumhard::mindmap::model::MAX_SECTIONS_PER_NODE;
pub use baumhard::mindmap::tree_builder::InteractionModeOverrides;
// Native-only: consumed by `app/click.rs`'s reparent / connect mode
// rendering. WASM doesn't dispatch `EnterReparentMode` /
// `EnterConnectMode` (NativeOnly per `wasm_compatibility`).
#[cfg(not(target_arch = "wasm32"))]
pub use types::{REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR};
pub use undo_action::UndoAction;

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub selection: SelectionState,
    pub undo_stack: Vec<UndoAction>,
    /// Registry of all available custom mutations (app + user + map +
    /// inline, keyed by id). Later layers override earlier — see
    /// [`Self::build_mutation_registry_with_app_and_user`].
    pub mutation_registry: HashMap<String, CustomMutation>,
    /// Which source layer won the registry slot for each id. Populated
    /// alongside `mutation_registry` so `mutation help <id>` can
    /// report "source: app / user / map / inline" without re-walking
    /// the layers.
    pub mutation_sources: HashMap<String, SourceTier>,
    /// Per-mutation-id imperative handlers. When a handler is
    /// registered for a mutation's id, `apply_custom_mutation`
    /// delegates to it instead of the default flat-apply path — the
    /// seam size-aware / layout-generating / otherwise-Rust-computed
    /// mutations plug into. Handlers mutate the MindMap model
    /// directly; `target_scope` tells the undo path which nodes to
    /// snapshot before the handler runs.
    pub mutation_handlers: HashMap<String, mutations::DynamicMutationHandler>,
    /// Active toggle mutations, each a `(node_id, mutation_id)` pair,
    /// in **activation order**. An ordered `Vec` rather than a
    /// `HashSet` so `reapply_active_toggles` re-stamps them in the
    /// same sequence the user turned them on — non-commutative
    /// toggles on the same element (two font-size toggles, a `MoveTo`
    /// plus a nudge) would otherwise render a different final visual
    /// after a rebuild than they did on activation. Membership is
    /// checked with a linear scan; the active set is tiny (a user
    /// holds at most a handful of inspection toggles), so the `Vec`
    /// beats a hash set on both footprint and determinism.
    pub active_toggles: Vec<(String, String)>,
    /// Currently-running animations. Each instance carries the
    /// from/to snapshot of its target node and the timing
    /// envelope; [`Self::tick_animations`] interpolates and
    /// writes the blended state back to `mindmap.nodes` until
    /// `t = 1`. Empty when no animations are active — the event
    /// loop checks [`Self::has_active_animations`] to decide
    /// whether to keep ticking. See
    /// `lib/baumhard/src/mindmap/animation.rs` for the timing /
    /// easing / lerp primitives this uses.
    pub active_animations: Vec<AnimationInstance>,
    /// Transient label edit preview. When `Some((edge_key, buffer))`,
    /// scene-building substitutes `buffer` (plus a trailing caret) for
    /// the matching edge's `ConnectionLabelElement.text` — the inline
    /// label editor's live display. Cleared on commit or cancel.
    ///
    /// Lives on the document rather than on the app layer so all
    /// `frame_overrides` callers see the override without extra
    /// plumbing. The committed `MindEdge.label` in `self.mindmap` is
    /// never touched during editing; the preview is purely a
    /// scene-level substitution.
    pub label_edit_preview: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    /// Transient portal-text editor buffer. When `Some(...)`, the
    /// scene builder substitutes the buffer for the target
    /// endpoint's `PortalEndpointState.text` so text edits render
    /// live. Same discipline as `label_edit_preview`: the
    /// committed model in `self.mindmap` is never touched during
    /// editing; the preview is purely a scene-level substitution.
    /// Key shape is `(edge_key, endpoint_node_id, buffer)` —
    /// portal labels are per-endpoint, so the key needs both the
    /// owning edge and the endpoint side.
    pub portal_text_edit_preview: Option<(baumhard::mindmap::scene_cache::EdgeKey, String, String)>,
    /// Transient color-picker hover preview. When `Some(...)`, the
    /// scene builder substitutes the preview color for the edge
    /// under the wheel — overriding both the resolved `config.color`
    /// and any selection highlight on the previewed edge so the user
    /// sees the live HSV value on the element being edited. Fans out
    /// to the portal pass automatically for edges with
    /// `display_mode = "portal"`. Commit (`set_edge_color`) and
    /// cancel both clear the preview; neither the committed model
    /// nor the undo stack is touched during hover.
    pub color_picker_preview: Option<ColorPickerPreview>,
    /// Transient border-preview substitution. When `Some(...)`,
    /// the scene builder substitutes `edits` (folded into a clone
    /// of the committed slot) for the resolved border at the
    /// matching target — node border, section frame, or canvas
    /// default. Same discipline as the other `*_preview` fields:
    /// never serialized, never push undo, never flip `dirty`.
    /// Replaced atomically by a fresh `set_border_preview` call;
    /// cleared by `cancel_border_preview` /
    /// `commit_border_preview`; lazily ignored by the scene
    /// builder when the live selection no longer covers the
    /// preview's `selection_snapshot` (drift). Drives the
    /// `border preview …` / `section frame preview …` /
    /// `canvas border preview …` /
    /// `canvas section-frame [focused] preview …` console verbs.
    pub border_preview: Option<BorderPreview>,
    /// Transient rubber-band selection preview: the node ids a
    /// Shift+drag rectangle currently covers. When `Some(ids)`,
    /// `app::scene_rebuild::highlight_entries_for` — a plain code
    /// span because `scene_rebuild` is a private module and rustdoc
    /// cannot resolve a link into one — paints *those* nodes
    /// instead of `self.selection`'s: the
    /// gesture's release replaces the selection with the set
    /// outright, so the pre-drag selection's highlight stands down
    /// for the duration rather than sitting under a set that is
    /// about to overwrite it.
    ///
    /// Same discipline as the other `*_preview` fields: never
    /// serialized, never pushes undo, never flips `dirty`. Written
    /// only by `set_rect_select_preview` and cleared by
    /// `take_rect_select_preview` — private, and code spans rather
    /// than links, because both are native-gated and a link from
    /// this cross-platform doc into a stripped item breaks the
    /// wasm32 doc leg (CODE_CONVENTIONS §8). The field is private so
    /// that claim is the compiler's rather than a request;
    /// [`Self::rect_select_preview`] is the cross-platform read.
    ///
    /// It lives here rather than beside the (native-only)
    /// `DragState` for the reason the rest of this family does:
    /// *every* rebuild path has to see it. Before it existed the
    /// preview was stamped onto a private tree the rubber-band
    /// drain built for itself each frame, so an animation tick
    /// landing mid-gesture — its `rebuild_all` runs in the same
    /// frame, after the drain — wiped the preview until the next
    /// drain rebuilt it.
    ///
    /// **Derived, not owned.** Living here is what makes it visible
    /// to every rebuild, and *also* what would make it survive the
    /// gesture that authorized it — `DragState::SelectingRect` has
    /// more than one way out, and only one of them ever ran a
    /// `take`. So the authority stays on the drag state and this is
    /// its projection: `app::drain_frame::drain_rect_select` — a
    /// plain code span because `app` is a private module and rustdoc
    /// cannot resolve a link into one — re-derives the two from each
    /// other on every frame, so no exit has to remember.
    ///
    /// **Cross-platform field, native-only writers.** The read is
    /// shared because both targets' rebuild paths go through the one
    /// `highlight_entries_for` mapping; the gesture that fills it is
    /// native-only, because `DragState` and the per-frame drain are.
    /// On wasm32 this is therefore always `None` — see CLAUDE.md's
    /// "Dual-target status" registry.
    rect_select_preview: Option<Vec<String>>,
}

/// Transient visual-only substitution of a color-pickerable element's
/// color. Read by `frame_overrides` and consumed by the
/// `EdgeColorPreview` and `PortalColorPreview` threaded params.
///
/// One variant handles every edge — including portal-mode edges —
/// because both routes key by the same `EdgeKey`. The scene pipeline
/// fans the preview out: the connection pass picks it up as
/// `EdgeColorPreview` when the edge renders as a line; the portal
/// pass picks it up as `PortalColorPreview` when the edge has
/// `display_mode = "portal"`.
#[derive(Debug, Clone)]
pub struct ColorPickerPreview {
    pub key: baumhard::mindmap::scene_cache::EdgeKey,
    pub color: String,
}

/// Announce a map that really did load.
///
/// Called by [`MindMapDocument::load`] and
/// [`MindMapDocument::from_json_str`] — the two constructors that
/// take a `.mindmap.json` and succeed — and deliberately *not* by
/// `finalize`, which the load-failure placard also goes through.
/// `info!`, so it is compiled out of release entirely (§9); a
/// failure is `error!` and reaches the user either way.
fn log_loaded(map: &MindMap) {
    info!(
        "document: loaded mindmap {:?} with {} node(s)",
        map.name,
        map.nodes.len()
    );
}

fn grow_node_sizes_to_fit_text(map: &mut MindMap) {
    for node in map.nodes.values_mut() {
        grow_one_node_to_fit_text(node);
    }
}

/// Per-node version of [`grow_node_sizes_to_fit_text`] — used by
/// the per-edit setters so a `font set <family>` on a single node
/// grows the box without re-walking the whole map. Same monotonic
/// "grow, never shrink" posture as the bulk pass: node sizes are
/// author intent, the loader and the per-edit setter just enforce
/// a floor.
///
/// Measures with the node's pinned font face when one is set
/// (`TextRun.font` resolves through `app_font_by_family`). Falls
/// back to cosmic-text's default when the run carries the empty
/// sentinel or names an unknown family. Without this, a node
/// pinned to a wide display face measures as if it were monospace
/// and the box undersizes by 30–60%, leaving text overflowing the
/// right edge after a `font set` or `font size=` edit.
///
/// Picks the *largest* `size_pt` across all runs rather than the
/// first — runs are usually homogeneous today (the inline editor
/// collapses to one), but a multi-size future shouldn't silently
/// fall back to the smallest measurement.
pub(super) fn grow_one_node_to_fit_text(node: &mut baumhard::mindmap::model::MindNode) {
    let (floor_w, floor_h) = compute_one_node_text_floor(node);
    if node.size.width < floor_w {
        node.size.width = floor_w;
    }
    if node.size.height < floor_h {
        node.size.height = floor_h;
    }
    clamp_node_size_to_ceiling(node);
}

/// How many lines of one section are actually laid out when
/// measuring its text floor.
///
/// The largest section in the canonical fixture is 33 lines, so this
/// is orders of magnitude above real authored content and never
/// binds on a map anyone wrote. It binds on a map written to be
/// expensive — see [`measured_prefix`].
pub(super) const MEASURED_LINE_BUDGET: usize = 512;

/// The prefix of `text` covering at most `max_lines` lines, together
/// with the total line count of the whole string.
///
/// The count is a byte scan; the prefix is what gets shaped. Cost:
/// O(text) with no allocation and no layout — which is the entire
/// point, since laying the same text out costs a cosmic-text
/// `BufferLine` per line.
pub(super) fn measured_prefix(text: &str, max_lines: usize) -> (&str, usize) {
    let total = text.lines().count();
    if total <= max_lines {
        return (text, total);
    }
    let mut end = text.len();
    let mut seen = 0usize;
    for (idx, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            seen += 1;
            if seen == max_lines {
                end = idx;
                break;
            }
        }
    }
    (&text[..end], total)
}

/// The `max_lines` widest lines of `text`, in their original order,
/// joined by `'\n'`.
///
/// With an unbounded measuring width nothing wraps, so a text
/// block's width is simply its widest line's width. Laying out the
/// *first* `max_lines` therefore measures the right width only when
/// the widest line happens to be among them — 512 short lines
/// followed by one long line measured as though the long one were
/// not there, and the node was sized to clip it. Selecting by width
/// instead of by position measures the lines that can actually
/// decide the answer.
///
/// **This sample is a candidate set, not an answer.** Ranking uses
/// `grapheme_display_width` — a column count — while cosmic-text
/// shapes real advances against a face that is not monospace, so
/// the proxy can and does rank backwards: at the default face
/// `"W".repeat(20)` shapes wider than `"i".repeat(30)` while losing
/// the ranking 20 columns to 30, and a tab counts as one column
/// while shaping to a tab stop. A sample picked this way can
/// therefore be *narrower* than the first `max_lines` lines.
///
/// That is why the caller measures this sample **and** the
/// positional prefix and takes the wider of the two, rather than
/// substituting one for the other — see `compute_one_node_text_floor`.
/// Neither sample is guaranteed to contain the true widest line,
/// because finding it means shaping every line and the budget
/// exists precisely to avoid that; taking the maximum is what makes
/// the result never worse than either alone.
///
/// Cost: O(lines) calls to `grapheme_display_width`, each linear in
/// its line — so O(text) overall, which dominates the O(lines ·
/// log max_lines) heap work. O(max_lines) memory for the heap,
/// which holds `(width, index)` pairs rather than the text, plus a
/// `Vec` of the chosen indices. The joined result is built by
/// repeated `push_str` into a `String` with no reserved capacity,
/// so it reallocates as it grows rather than allocating once — it
/// is bounded by the selected lines' total length, which is bounded
/// by the text. Two passes over `text`, no layout.
pub(super) fn widest_lines(text: &str, max_lines: usize) -> String {
    use baumhard::util::grapheme_chad::grapheme_display_width;
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    if max_lines == 0 {
        return String::new();
    }

    // `BinaryHeap` is a max-heap, so `Reverse` makes this a min-heap
    // on `(width, index)` and `pop()` evicts the current worst. The
    // index rides along to keep the eviction total and deterministic;
    // because it is compared after the width, a tie evicts the
    // *lowest* index, which means equal-width lines keep the ones
    // latest in the text. That is arbitrary but fixed — what matters
    // is that the same input always yields the same sample.
    let mut best: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::with_capacity(max_lines + 1);
    for (idx, line) in text.lines().enumerate() {
        best.push(Reverse((grapheme_display_width(line), idx)));
        if best.len() > max_lines {
            best.pop();
        }
    }

    let mut chosen: Vec<usize> = best.into_iter().map(|Reverse((_, idx))| idx).collect();
    chosen.sort_unstable();

    // Separator driven by a count rather than by `out.is_empty()`:
    // a selected *blank* line leaves `out` empty, so the emptiness
    // test would skip the newline before the next line and silently
    // splice two selected lines into one — losing a line from a
    // sample whose whole contract is `max_lines` of them.
    let mut out = String::new();
    let mut emitted = 0usize;
    let mut wanted = chosen.iter().peekable();
    for (idx, line) in text.lines().enumerate() {
        if wanted.peek() == Some(&&idx) {
            wanted.next();
            if emitted > 0 {
                out.push('\n');
            }
            out.push_str(line);
            emitted += 1;
        }
    }
    out
}

/// Pure floor-compute extracted from [`grow_one_node_to_fit_text`]
/// so the explicit-shrink path
/// [`MindMapDocument::fit_node_to_content`] can read the floor
/// without triggering the max-wins-grow side effect. Each
/// section contributes the larger of its measured text and its
/// pinned `size + offset` — pin survives when text fits;
/// overflow grows the parent so nothing visually clips.
pub(super) fn compute_one_node_text_floor(node: &baumhard::mindmap::model::MindNode) -> (f64, f64) {
    use baumhard::font::fonts::{
        acquire_font_system_write, app_font_by_family, measure_text_block_unbounded,
    };

    // §B5 lock-scope discipline: each section's measurement
    // acquires + drops the `FONT_SYSTEM` write guard
    // independently to keep parallel cargo-test workers from
    // thrashing the lock.
    let mut floor_w: f64 = 0.0;
    let mut floor_h: f64 = 0.0;
    for section in &node.sections {
        // Non-finite offsets contribute nothing — the verifier
        // flags them, and a NaN propagating into floor_w / floor_h
        // would corrupt every downstream `node.size` reader.
        if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
            continue;
        }
        // The scale and the line height the tree builder will lay
        // this section out at, asked of the builder rather than
        // re-derived: a measurement that disagreed with the render
        // would grow the node to the wrong height and clip the very
        // text it was grown for.
        let scale = effective_section_scale(section);
        let line_height = scale * LINE_HEIGHT_FACTOR;
        let pad_x = scale * 1.5;
        let pad_y = scale * 0.5;

        let measure_font = section
            .text_runs
            .iter()
            .max_by(|a, b| a.size_pt.total_cmp(&b.size_pt))
            .and_then(|r| {
                if r.font.is_empty() {
                    None
                } else {
                    app_font_by_family(&r.font)
                }
            });

        // Shaping is linear in line count and this runs at load, on
        // text that came out of an untrusted file. A section
        // carrying millions of newlines would shape millions of
        // cosmic-text lines here — each one an owned `String`, an
        // `AttrsList`, and two layout caches — before a single frame
        // is drawn. So only a bounded prefix is laid out.
        //
        // The height loses nothing by it: `TextBlockSize::height` is
        // `line_count * line_height`, and with an unbounded measuring
        // width no line wraps, so counting newlines is the same
        // number for a byte scan instead of a layout pass.
        //
        // The width would lose something, and past the budget it
        // takes **two** samples to lose as little as possible.
        //
        // With no wrapping the block's width is its widest line, so
        // the first `MEASURED_LINE_BUDGET` lines answer the wrong
        // question: 512 short lines followed by one long line sized
        // the node as though the long one did not exist and clipped
        // it. Picking the widest lines instead — [`widest_lines`] —
        // answers the right question with the wrong ruler: its
        // ranking is a column count, and cosmic-text shapes real
        // advances against a proportional face, so the proxy can
        // rank backwards and produce a sample *narrower* than the
        // prefix it replaced.
        //
        // So both are measured and the wider wins. That costs one
        // extra bounded shaping pass on sections past the budget,
        // and it is the only version of this that is never worse
        // than the alternative it replaced. Neither sample is
        // guaranteed to hold the true widest line — that needs a
        // shaping pass per line, which is the cost the budget exists
        // to refuse — so this is a floor that improves, not a proof.
        //
        // Worth stating plainly because an earlier comment here got
        // it wrong: a node past the budget is *not* saved by the
        // `MAX_NODE_AXIS` clamp. At the default 14 pt, 513 lines is
        // 8,618 pt against a 1,000,000 ceiling, and the clamp only
        // starts covering above roughly 1,624 pt. Nor is this only
        // the load path — `grow_one_node_to_fit_text` is the
        // per-edit setter too.
        let (measured, total_lines) = measured_prefix(&section.text, MEASURED_LINE_BUDGET);
        let truncated = measured.len() < section.text.len();
        let mut block = {
            let mut fs = acquire_font_system_write("compute_one_node_text_floor");
            measure_text_block_unbounded(&mut fs, measured, scale, line_height, measure_font)
        };
        if truncated {
            // The second sample: the widest lines by column proxy.
            // Only its *width* is taken — its lines are not
            // contiguous, so its height means nothing.
            let widest = widest_lines(&section.text, MEASURED_LINE_BUDGET);
            let wide_block = {
                let mut fs = acquire_font_system_write("compute_one_node_text_floor");
                measure_text_block_unbounded(&mut fs, &widest, scale, line_height, measure_font)
            };
            block.width = block.width.max(wide_block.width);

            // Under budget the shaper's own line count is
            // authoritative and a byte scan must not second-guess
            // it: `str::lines()` and cosmic-text disagree by exactly
            // one on text ending in a newline, and that would
            // shorten every such node by a line. Past it, neither
            // shaped block spans the whole text, so the counted
            // total is the only correct source.
            block.height = total_lines as f32 * line_height;
        }

        // Section dimension contribution: text needs `block + pad`
        // at minimum, but a `Some`-size section also pins a user-
        // set floor (the author wrote "this section is at least
        // this big"). Take the max so user intent survives when
        // text fits, and overflow still grows the parent so
        // nothing visually clips.
        let mut section_w = (block.width + pad_x) as f64;
        let mut section_h = (block.height + pad_y) as f64;
        if let Some(s) = section.size.as_ref() {
            if s.width.is_finite() && s.width > section_w {
                section_w = s.width;
            }
            if s.height.is_finite() && s.height > section_h {
                section_h = s.height;
            }
        }
        // Pass the offset through unmodified — the prior `.max(0)`
        // clamp silently treated leftward / upward overflow as zero,
        // hiding the actual visible-text width.
        let need_w = section_w + section.offset.x;
        let need_h = section_h + section.offset.y;
        if need_w > floor_w {
            floor_w = need_w;
        }
        if need_h > floor_h {
            floor_h = need_h;
        }
    }
    (floor_w, floor_h)
}

/// Grow every framed node's size to also accommodate its border's
/// static parts plus, when feasible, one full fill iteration on
/// each side. Mirrors [`grow_node_sizes_to_fit_text`]'s posture:
/// only grows, never shrinks — node sizes are author intent, the
/// loader and the per-edit setter just enforce a floor.
///
/// Composes monotonically with the text floor (max wins) when
/// both run on the same map.
pub(super) fn grow_node_sizes_to_fit_borders(map: &mut MindMap) {
    let canvas_default = map.canvas.default_border.clone();
    for node in map.nodes.values_mut() {
        grow_one_node_to_fit_border(node, canvas_default.as_ref());
    }
}

/// Per-node version of [`grow_node_sizes_to_fit_borders`] — used
/// by the per-edit setters so a `border preset=heavy` on a small
/// node grows the box without re-walking the whole map. The
/// canvas default is passed in so callers can hold a single
/// borrow once and re-use it.
pub(super) fn grow_one_node_to_fit_border(
    node: &mut baumhard::mindmap::model::MindNode,
    canvas_default: Option<&baumhard::mindmap::model::GlyphBorderConfig>,
) {
    use baumhard::mindmap::border::{resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC};
    if !node.style.show_frame {
        return;
    }
    // **This function is deliberately color-blind.** It reads
    // `font_size_pt`, `corner_clusters()` and `side_patterns` and
    // nothing else, and `resolve_border_style`'s own contract says
    // the last two arguments feed `BorderStyle::color` alone — the
    // size and preset cascades are independent of both. So the
    // theme tier is `None` and the floor is `""`: threading a
    // resolved frame color in here would be an `FxHashMap` and two
    // `String`s per node on the load pass, plus one `String` per
    // committed edit, to compute a field this function never
    // looks at.
    let style = resolve_border_style(node.style.border.as_ref(), canvas_default, None, "");
    let approx_char_width = style.font_size_pt * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let corners = style.corner_clusters();

    // Soft target: include one full fill iteration on each side.
    // Hard floor: cover the static parts only.
    let need_top = style.side_patterns.top.minimum_with_one_fill() + corners.top_horizontal();
    let need_bottom = style.side_patterns.bottom.minimum_with_one_fill() + corners.bottom_horizontal();
    let need_left = style.side_patterns.left.minimum_with_one_fill();
    let need_right = style.side_patterns.right.minimum_with_one_fill();

    let need_horizontal_clusters = need_top.max(need_bottom);
    let need_vertical_clusters = need_left.max(need_right);

    let need_w = need_horizontal_clusters as f32 * approx_char_width;
    let need_h = need_vertical_clusters as f32 * style.font_size_pt;

    let size = node.size_vec2();
    if size.x < need_w {
        node.size.width = need_w as f64;
    }
    if size.y < need_h {
        node.size.height = need_h as f64;
    }
    clamp_node_size_to_ceiling(node);
}

/// Clamp `node.size` to the shared `MAX_NODE_AXIS` ceiling. The
/// explicit setters (`set_node_size`, `set_node_aabb`) already reject
/// sizes above this bound; the grow-to-fit floor functions must also
/// honor it so the editor cannot produce a saved map that `maptool
/// verify` would reject.
fn clamp_node_size_to_ceiling(node: &mut baumhard::mindmap::model::MindNode) {
    if node.size.width > MAX_NODE_AXIS {
        node.size.width = MAX_NODE_AXIS;
    }
    if node.size.height > MAX_NODE_AXIS {
        node.size.height = MAX_NODE_AXIS;
    }
}

impl MindMapDocument {
    /// Wrap a `MindMap` in a fresh document shell (selection cleared,
    /// undo stack empty, mutation registry rebuilt from the map's
    /// declared mutations). Shared by `load`, `from_json_str`,
    /// `new_blank`, and the test fixture loader so the transient-
    /// state defaults stay in one place.
    ///
    /// Does **not** run [`Self::finalize`] (grow-to-fit passes) —
    /// callers must either use [`Self::load`] / [`Self::from_json_str`]
    /// (which call finalize first), or pass a map whose node sizes
    /// already accommodate its text and borders (`new_blank` —
    /// trivially; the testament fixture — by authored construction).
    pub(crate) fn from_mindmap(mindmap: MindMap, file_path: Option<String>) -> Self {
        let mut doc = MindMapDocument {
            mindmap,
            file_path,
            dirty: false,
            selection: SelectionState::None,
            undo_stack: Vec::new(),
            mutation_registry: HashMap::new(),
            mutation_sources: HashMap::new(),
            mutation_handlers: HashMap::new(),
            active_toggles: Vec::new(),
            label_edit_preview: None,
            portal_text_edit_preview: None,
            color_picker_preview: None,
            border_preview: None,
            rect_select_preview: None,
            active_animations: Vec::new(),
        };
        doc.build_mutation_registry();
        doc
    }

    /// Load a MindMap from a file path. Native-only — WASM builds
    /// must use `from_json_str` since the browser has no filesystem.
    ///
    /// **The error is returned, not reported.** Reporting belongs to
    /// whichever surface the caller is: the console `open` verb
    /// prints it in the overlay, and the startup path renders it on
    /// the canvas via [`Self::load_failure_placard`]. A constructor
    /// that both logged and returned the failure made every caller
    /// choose between reporting it twice and suppressing it, and the
    /// startup path did the former.
    pub fn load(path: &str) -> Result<Self, String> {
        loader::load_from_file(Path::new(path)).map(|map| {
            log_loaded(&map);
            Self::finalize(map, Some(path.to_string()))
        })
    }

    /// Construct a Document from an in-memory JSON string. `file_path`
    /// is the origin tag stored for save-back; pass the URL/path the
    /// JSON came from, or `None` for ad-hoc JSON.
    ///
    /// Reports nothing, for the reason spelled out on [`Self::load`].
    pub fn from_json_str(json: &str, file_path: Option<String>) -> Result<Self, String> {
        loader::load_from_str(json).map(|map| {
            log_loaded(&map);
            Self::finalize(map, file_path)
        })
    }

    /// The document a rejected load is rendered as: a one-node map
    /// carrying `message` — the loader's own text — over the
    /// `source` that was asked for. See
    /// [`baumhard::mindmap::placard`] for why the failure surface is
    /// a map rather than an overlay.
    ///
    /// **Bound to no file path, on purpose.** `save` and `Ctrl+S`
    /// both refuse a document with no `file_path`, so a reflexive
    /// save on the placard cannot overwrite the file that failed to
    /// parse — which, being unparseable, is likely the only copy of
    /// whatever the author was hand-editing.
    pub fn load_failure_placard(source: &str, message: &str) -> Self {
        Self::finalize(placard::load_failure(source, message), None)
    }

    /// Grow undersized node boxes to fit their text and their
    /// border's static parts before the model is handed to the
    /// tree/scene builders. Both passes only grow, so the order
    /// composes — text-driven floor first, then border-driven —
    /// and the larger of the two wins per node.
    ///
    /// **Reports nothing.** The "loaded mindmap …" line belongs to
    /// the two constructors that really did load one, not here:
    /// [`Self::load_failure_placard`] also finalizes, and a placard
    /// is the map that stands in for a load that did not happen.
    /// Announcing `document: loaded mindmap "load-failure"` beside
    /// `startup: could not load …` is a log that contradicts itself
    /// in two consecutive lines.
    fn finalize(mut map: MindMap, file_path: Option<String>) -> Self {
        grow_node_sizes_to_fit_text(&mut map);
        grow_node_sizes_to_fit_borders(&mut map);
        Self::from_mindmap(map, file_path)
    }

    /// Construct an empty document, optionally bound to a target file
    /// path. Used by the `new` console command. `dirty` starts `false`
    /// — the in-memory map matches its (possibly absent) on-disk state
    /// at construction time. When `file_path` is `Some`, the caller is
    /// expected to write the blank map to disk so the binding is real;
    /// otherwise the document is "untitled" and `save` will require a
    /// path argument.
    pub fn new_blank(file_path: Option<String>) -> Self {
        let name = file_path
            .as_deref()
            .and_then(|p| {
                Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.trim_end_matches(".mindmap").to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "untitled".to_string());
        Self::from_mindmap(MindMap::new_blank(name), file_path)
    }

    /// Construct a doc carrying a single orphan node at the
    /// given canvas position. Test- and small-scenario fixture
    /// for "smallest interactive doc" — replaces field-by-field
    /// `MindMapDocument { ... }` literal construction at
    /// downstream test sites so the field list lives in one
    /// place. Test-gated: the interactive orphan-creation path is
    /// `Action::CreateOrphanNode`, which adds a node to the open
    /// document rather than constructing one around it.
    #[cfg(test)]
    pub fn with_orphan(id: &str, pos: glam::Vec2) -> Self {
        let mut doc = Self::from_mindmap(MindMap::new_blank("t"), None);
        let node = super::document::defaults::default_orphan_node(id, pos);
        doc.mindmap.nodes.insert(id.to_string(), node);
        doc
    }

    /// Build a Baumhard mutation tree from the MindMap hierarchy.
    /// Each MindNode becomes a GlyphArea in the tree, preserving parent-child structure.
    ///
    /// This is a **pure** projection of the model — it carries no
    /// transient visual overlays (selection highlights, active
    /// toggles). Those are render-layer decorations applied by
    /// `rebuild_all` to the tree it hands the renderer, deliberately
    /// *not* here: `build_tree` is also the projection the
    /// `Persistent` custom-mutation apply path syncs back to the
    /// model, and an overlay baked in here would be written into the
    /// saved model (a nudge toggle would become a permanent move).
    /// Keeping `build_tree` overlay-free is what makes
    /// `sync_node_from_tree` safe.
    pub fn build_tree(&self) -> MindMapTree {
        tree_builder::build_mindmap_tree(&self.mindmap)
    }

    /// The node ids a live rubber-band rectangle currently covers,
    /// or `None` when no gesture is live.
    ///
    /// The cross-platform read of [`Self::rect_select_preview`]:
    /// every node-tree rebuild on both targets goes through
    /// `app::scene_rebuild::highlight_entries_for`, which asks this.
    /// The two writers are native-gated (the gesture that fills it
    /// needs `DragState` and the per-frame drain, neither of which
    /// the browser has), so on wasm32 this is always `None`.
    ///
    /// `Some(&[])` and `None` are different answers and the
    /// distinction is load-bearing: an empty set means "a gesture is
    /// live and covers nothing", which paints no highlight at all,
    /// while `None` means "no gesture", which paints the selection.
    pub fn rect_select_preview(&self) -> Option<&[String]> {
        self.rect_select_preview.as_deref()
    }

    /// Install `ids` as the live rubber-band preview and report
    /// whether that **changed** anything.
    ///
    /// `false` means the drag moved without crossing a node
    /// boundary, so the canvas already shows the right set and the
    /// caller owes it no repaint at all. That is the whole of what
    /// makes a rubber-band frame cheap: the hit-test is a geometry
    /// read over the tree that is already installed, and the tree
    /// rebuild behind it happens per *change of set*, not per drain.
    ///
    /// Order is part of the identity on purpose — `rect_select`
    /// walks `MindMapTree::node_ids`, which is arena order and
    /// therefore stable for a given tree, so two frames covering the
    /// same nodes produce the same `Vec` and compare equal.
    ///
    /// Native-only, with the read left cross-platform: the browser
    /// has no rubber-band select to install one from. See CLAUDE.md's
    /// "Dual-target status" registry.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_rect_select_preview(&mut self, ids: Vec<String>) -> bool {
        if self.rect_select_preview.as_deref() == Some(ids.as_slice()) {
            return false;
        }
        self.rect_select_preview = Some(ids);
        true
    }

    /// End the rubber-band gesture, handing back the set it was
    /// previewing. `None` when no preview was live.
    ///
    /// Native-only, for the same reason as
    /// `set_rect_select_preview` above.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_rect_select_preview(&mut self) -> Option<Vec<String>> {
        self.rect_select_preview.take()
    }

    /// Assemble every frame-local override the per-role tree
    /// builders read: the selected edge (highlight — routed to
    /// either the connection or the portal pass depending on the
    /// edge's `display_mode`), the two inline editors' uncommitted
    /// buffers, the color-picker hover (fanned out to both the
    /// line-edge and portal channels so a portal-mode edge under
    /// the wheel picks it up), and the staged border-preview edits.
    ///
    /// Assembled once per rebuild and shared by every pass, so no
    /// two canvas roles can disagree about what the user is
    /// pointing at. Everything borrows from `&self`, so the result
    /// lives as long as the document reference.
    ///
    /// `resize_overrides` carries the mode-derived inputs the
    /// document itself doesn't know about — which node / section
    /// grows resize handles, which node is the active `NodeEdit`
    /// target, which section is focused. The application layer
    /// translates `InteractionMode` into that bundle; the document
    /// stays mode-agnostic.
    pub fn frame_overrides<'a>(
        &'a self,
        resize_overrides: InteractionModeOverrides<'a>,
    ) -> FrameOverrides<'a> {
        let edge = self
            .selection
            .selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        // Edge-label sub-selection: when the user clicked just
        // the label (not the whole edge), only the label text
        // tints cyan. The label pass upgrades a whole-edge
        // selection to also paint the label, so we don't need to
        // fill `edge_label` in for `Edge` selections here. The
        // `EdgeLabelSel` stores an `EdgeRef`, so we build an
        // owned `EdgeKey` per call — three small string clones,
        // negligible next to the per-frame projection.
        let edge_label = match &self.selection {
            crate::application::document::SelectionState::EdgeLabel(s) => {
                Some(baumhard::mindmap::scene_cache::EdgeKey::from(&s.edge_ref))
            }
            _ => None,
        };
        let selection = tree_builder::SceneSelectionContext {
            edge,
            edge_label,
            portal_label: self.selection.selected_portal_label_scene_ref(),
            label_edit: self.label_edit_preview.as_ref().map(|(k, s)| (k, s.as_str())),
            // Resize-handle emission, NodeEdit dimming, and
            // section-frame focus are all driven by
            // `InteractionMode`, not by selection — the application
            // layer translates the active mode into
            // `InteractionModeOverrides` and threads it through
            // here. Fill-parent sections emit zero handles inside
            // the builder regardless of the override value (no own
            // AABB to stretch).
            selected_section: resize_overrides.section,
            selected_node_for_resize: resize_overrides.node,
            node_edit_for: resize_overrides.node_edit_for,
            focused_section: resize_overrides.focused_section,
        };
        let (edge_color, portal_color) = match &self.color_picker_preview {
            Some(ColorPickerPreview { key, color }) => (
                Some(tree_builder::EdgeColorPreview {
                    edge_key: key,
                    color: color.as_str(),
                }),
                Some(tree_builder::PortalColorPreview {
                    edge_key: key,
                    color: color.as_str(),
                }),
            ),
            None => (None, None),
        };
        // Border preview: build a borrowed view from the owned
        // `self.border_preview`. `None` when no preview is active
        // OR when the preview's target is no longer covered by the
        // live selection (defer-clear posture — the slot itself
        // empties at the next `set_*` / `cancel_*` / `commit_*`
        // call; here at projection time, an orphan-by-drift preview
        // just stops applying).
        let border = if self.border_preview_covers_live_selection() {
            self.border_preview.as_ref().map(build_border_preview_scene_view)
        } else {
            None
        };
        FrameOverrides {
            selection,
            edge_color,
            portal_color,
            border,
        }
    }
}

/// Build a borrowed scene-side `BorderPreview<'a>` from the owned
/// document-side `BorderPreview`. The scene-side view is `Copy +
/// 'a`; it holds `&'a str` borrows pointing at the owned
/// `BorderConfigEdits` fields, so the resulting view lives as
/// long as the document reference the caller already has.
///
/// `force_show_frame` fires when the preview touches **any**
/// field, or carries the whole-slot `clear` flag — see
/// [`view_implies_visible`]. Not just the preset / glyph / pattern
/// axes: a preview must be visible even when the committed
/// `style.show_frame == false`, and that is as true of
/// `border preview color=red` as of `border preview preset=heavy`.
/// Narrowing the predicate to the shape-changing fields would make
/// the color, padding, font, and palette previews render nothing on
/// a frameless node — the exact "the verb is broken" failure the
/// flag exists to prevent. `border preview clear` pops a frame for
/// the same reason: showing what the cascade falls back to *is*
/// the preview.
///
/// Commit writes `style.show_frame = true` through the normal
/// setter when the user wants the visibility flip persisted (today
/// via `border on`); the force flag never reaches the model.
fn build_border_preview_scene_view<'a>(bp: &'a BorderPreview) -> tree_builder::BorderPreview<'a> {
    let target = match &bp.target {
        BorderPreviewTarget::Nodes(ids) => tree_builder::BorderPreviewTargetRef::Nodes(ids.as_slice()),
        BorderPreviewTarget::Sections(ts) => tree_builder::BorderPreviewTargetRef::Sections(ts.as_slice()),
        BorderPreviewTarget::CanvasDefault => tree_builder::BorderPreviewTargetRef::CanvasDefault,
        BorderPreviewTarget::CanvasSectionFrame => tree_builder::BorderPreviewTargetRef::CanvasSectionFrame,
        BorderPreviewTarget::CanvasSectionFrameFocused => {
            tree_builder::BorderPreviewTargetRef::CanvasSectionFrameFocused
        }
    };
    let edits = build_border_config_edits_view(&bp.edits);
    let force_show_frame = view_implies_visible(&edits);
    tree_builder::BorderPreview {
        target,
        edits,
        force_show_frame,
    }
}

/// Convert an owned `BorderConfigEdits` (from the application
/// crate) into a borrowed scene-side `BorderConfigEditsView<'a>`
/// the scene builder consumes. Per-field tri-state: `Keep` →
/// `EditView::Keep`, `Clear` → `EditView::Clear`, `Set(v)` →
/// `EditView::Set(&v)`. Pre-fix this projection collapsed both
/// `Keep` and `Clear` to a single "no edit" sentinel, dropping
/// the `Clear` axis entirely and breaking the parity contract
/// with `apply_glyph_border_edits_to_slot` (Risk #1 in the plan).
/// Test-only re-export of [`build_border_config_edits_view`].
/// Used by the parity test in `tests_nodes.rs` that exercises
/// `apply_view_to_slot` (baumhard) vs `apply_glyph_border_edits_to_slot`
/// (application) against identical edits across every per-field
/// axis. Keep `pub(crate)` — production callers go through
/// `frame_overrides`.
#[cfg(test)]
pub(crate) fn build_border_config_edits_view_for_test(
    edits: &BorderConfigEdits,
) -> tree_builder::BorderConfigEditsView<'_> {
    build_border_config_edits_view(edits)
}

/// Test-only proxy for the private `nodes::border::apply_glyph_border_edits_to_slot`
/// — keeps the module-level visibility narrow while still letting
/// the parity test in `tests_nodes.rs` exercise the helper directly.
#[cfg(test)]
pub(crate) fn nodes_border_apply_glyph_border_edits_to_slot_for_test(
    slot: &mut Option<baumhard::mindmap::model::GlyphBorderConfig>,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    nodes::apply_glyph_border_edits_to_slot(slot, edits, outcome)
}

fn build_border_config_edits_view(edits: &BorderConfigEdits) -> tree_builder::BorderConfigEditsView<'_> {
    use crate::application::document::OptionEdit;
    use tree_builder::EditView;
    fn opt_str(e: &OptionEdit<String>) -> EditView<&str> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(s) => EditView::Set(s.as_str()),
        }
    }
    fn opt_f32(e: &OptionEdit<f32>) -> EditView<f32> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(v) => EditView::Set(*v),
        }
    }
    fn opt_field(e: &OptionEdit<baumhard::mindmap::border::PaletteField>) -> EditView<&str> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(v) => EditView::Set(v.as_str()),
        }
    }
    tree_builder::BorderConfigEditsView {
        preset: opt_str(&edits.preset),
        font: opt_str(&edits.font),
        font_size_pt: opt_f32(&edits.font_size_pt),
        color: opt_str(&edits.color),
        padding: opt_f32(&edits.padding),
        color_palette: opt_str(&edits.color_palette),
        color_palette_field: opt_field(&edits.color_palette_field),
        side_top: opt_str(&edits.side_top),
        side_bottom: opt_str(&edits.side_bottom),
        side_left: opt_str(&edits.side_left),
        side_right: opt_str(&edits.side_right),
        corner_top_left: opt_str(&edits.corner_top_left),
        corner_top_right: opt_str(&edits.corner_top_right),
        corner_bottom_left: opt_str(&edits.corner_bottom_left),
        corner_bottom_right: opt_str(&edits.corner_bottom_right),
        clear: edits.clear,
    }
}

/// `true` iff `view`'s edits include at least one field that
/// implies the resolved border should be visible — any field
/// edit (`Set` or `Clear`) or the entire-slot `clear` flag.
/// Force-show then ignores a committed `style.show_frame == false`
/// for the duration of the preview so the user sees their staged
/// edits even on a frameless node.
///
/// Delegates to [`tree_builder::BorderConfigEditsView::touches_any_field`]
/// so the predicate stays in lockstep with the slot-allocation
/// gate inside `apply_view_to_slot` (the previous parallel
/// implementation drifted by one field — `clear` was excluded
/// from this side and included on the other).
fn view_implies_visible(view: &tree_builder::BorderConfigEditsView<'_>) -> bool {
    view.touches_any_field() || view.clear
}
