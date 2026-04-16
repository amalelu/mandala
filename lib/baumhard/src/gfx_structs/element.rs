use crate::core::primitives::{ColorFontRegionField, Flag, Flaggable, Range};
use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::model::{GlyphModel, GlyphModelField};
use crate::gfx_structs::mutator::GlyphTreeEventInstance;
use crate::gfx_structs::tree::{BranchChannel, EventSubscriber, TreeEventConsumer, TreeNode};
use crate::util::color::FloatRgba;
use crate::util::geometry::clockwise_rotation_around_pivot;
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};

/// Identifies a single field-level change to a [`GfxElement`].
///
/// Used by the mutator pipeline to describe which part of an element is
/// being mutated. Each variant wraps the field enum of the corresponding
/// inner type (`GlyphAreaField`, `GlyphModelField`) or targets
/// element-level metadata (channel, id, flag). Cost: O(1) construction,
/// no allocation beyond what the inner field enum carries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GfxElementField {
    /// A field change targeting the inner [`GlyphArea`].
    GlyphArea(GlyphAreaField),
    /// A field change targeting the inner [`GlyphModel`].
    GlyphModel(GlyphModelField),
    /// A field change targeting a specific [`ColorFontRegion`](crate::core::primitives::ColorFontRegion)
    /// within the given [`Range`].
    Region(Range, ColorFontRegionField),
    /// Reassign the element's branch channel index.
    Channel(usize),
    /// Reassign the element's unique id.
    Id(usize),
    /// Toggle or set a [`Flag`] on the element.
    Flag(Flag),
}

/// Discriminant tag for [`GfxElement`] variants.
///
/// Returned by [`GfxElement::get_type`]. Useful for branching on the
/// variant without destructuring the full enum. O(1), no allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GfxElementType {
    /// The element wraps a [`GlyphArea`].
    GlyphArea,
    /// The element wraps a [`GlyphModel`].
    GlyphModel,
    /// The element is a placeholder void node.
    Void,
}

/// A node element in the Baumhard glyph tree.
///
/// Every node in a [`Tree<GfxElement, GfxMutator>`](crate::gfx_structs::tree::Tree)
/// stores one `GfxElement`. The three variants cover the two renderable
/// glyph types ([`GlyphArea`] for text regions, [`GlyphModel`] for
/// composed glyph shapes) and a lightweight [`Void`](GfxElement::Void)
/// placeholder used to pad tree structure without rendering cost.
///
/// All variants carry a `channel` (branch routing), a `unique_id`
/// (application-assigned identity), a set of [`Flag`]s, and a list of
/// [`EventSubscriber`] callbacks.
///
/// Cost: construction allocates a `Box` for the inner glyph type
/// (except `Void`). Field access is O(1) with a single match.
pub enum GfxElement {
    /// A text region rendered through cosmic-text layout.
    ///
    /// Wraps a heap-allocated [`GlyphArea`] that owns the text content,
    /// scale, position, colour-font regions, and hit-box. This is the
    /// primary renderable element for mindmap nodes, labels, and borders.
    GlyphArea {
        glyph_area: Box<GlyphArea>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
        /// Cached AABB covering this node and all its descendants.
        /// Computed lazily by [`Tree::ensure_subtree_aabbs`] and
        /// invalidated when a mutation touches the tree. Not
        /// serialised — it is a runtime cache, not persistent state.
        subtree_aabb: Option<(Vec2, Vec2)>,
    },
    /// A composed glyph shape built from lines and components.
    ///
    /// Wraps a heap-allocated [`GlyphModel`]. Typically a child of a
    /// [`GlyphArea`] node — the model provides the visual geometry while
    /// the parent area supplies the text-layout context.
    GlyphModel {
        glyph_model: Box<GlyphModel>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
        /// See [`GfxElement::GlyphArea::subtree_aabb`].
        subtree_aabb: Option<(Vec2, Vec2)>,
    },
    /// A no-op placeholder node that produces no rendering output.
    ///
    /// Used to pad tree structure so that a mutator tree can target
    /// specific positions without requiring a renderable element at
    /// every slot. Carries only metadata (channel, id, flags,
    /// subscribers) and no glyph payload.
    Void {
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
        flags: FxHashSet<Flag>,
        /// See [`GfxElement::GlyphArea::subtree_aabb`].
        subtree_aabb: Option<(Vec2, Vec2)>,
    },
}

impl GfxElement {
    /// Create a [`GlyphArea`] element with `unique_id` defaulting to `0`.
    ///
    /// Convenience wrapper around [`new_area_non_indexed_with_id`](Self::new_area_non_indexed_with_id).
    /// Allocates a `Box<GlyphArea>`. The element is *not* inserted into a
    /// region index — callers that need indexing must register it
    /// separately.
    pub fn new_area_non_indexed(section: GlyphArea, channel: usize) -> GfxElement {
        Self::new_area_non_indexed_with_id(section, channel, 0)
    }
    /// Create a [`GlyphArea`] element with an explicit `unique_id`.
    ///
    /// * `section` — the [`GlyphArea`] payload (text, scale, position, bounds).
    /// * `channel` — branch-routing index for the tree walker.
    /// * `unique_id` — application-assigned identity for this element.
    ///
    /// Cost: one heap allocation (`Box<GlyphArea>`). Flags and
    /// subscribers start empty.
    pub fn new_area_non_indexed_with_id(
        section: GlyphArea,
        channel: usize,
        unique_id: usize,
    ) -> GfxElement {
        GfxElement::GlyphArea {
            glyph_area: Box::new(section),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
            subtree_aabb: None,
        }
    }

    /// Create a [`Void`](GfxElement::Void) placeholder with `unique_id`
    /// defaulting to `0`.
    ///
    /// No heap allocation — `Void` carries only metadata.
    pub fn new_void(channel: usize) -> GfxElement {
        Self::new_void_with_id(channel, 0)
    }

    /// Create a [`Void`](GfxElement::Void) placeholder with an explicit
    /// `unique_id`.
    ///
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: no heap allocation. Flags and subscribers start empty.
    pub fn new_void_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::Void {
            channel,
            unique_id,
            event_subscribers: vec![],
            flags: Default::default(),
            subtree_aabb: None,
        }
    }
    /// Create a [`GlyphModel`] element. Delegates to
    /// [`new_model_non_indexed_with_id`](Self::new_model_non_indexed_with_id).
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`).
    pub fn new_model_non_indexed(model: GlyphModel, channel: usize, unique_id: usize) -> Self {
        Self::new_model_non_indexed_with_id(model, channel, unique_id)
    }

    /// Create a [`GlyphModel`] element with an explicit `unique_id`.
    ///
    /// * `model` — the [`GlyphModel`] payload (lines, components, position).
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`). Flags and
    /// subscribers start empty.
    pub fn new_model_non_indexed_with_id(
        model: GlyphModel,
        channel: usize,
        unique_id: usize,
    ) -> Self {
        GfxElement::GlyphModel {
            glyph_model: Box::new(model),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
            subtree_aabb: None,
        }
    }

    /// Create a [`GlyphModel`] element with a default (empty) model.
    /// Delegates to [`new_model_blank_with_id`](Self::new_model_blank_with_id).
    ///
    /// Cost: one heap allocation for the empty `Box<GlyphModel>`.
    pub fn new_model_blank(channel: usize, unique_id: usize) -> GfxElement {
        Self::new_model_blank_with_id(channel, unique_id)
    }

    /// Create a [`GlyphModel`] element wrapping a default-constructed
    /// [`GlyphModel::new()`].
    ///
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`). Flags and
    /// subscribers start empty.
    pub fn new_model_blank_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::GlyphModel {
            glyph_model: Box::new(GlyphModel::new()),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
            subtree_aabb: None,
        }
    }

    /// Return a mutable reference to this element's event-subscriber list.
    ///
    /// Works across all variants. O(1) — single match, no allocation.
    pub fn subscribers_mut(&mut self) -> &mut Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers,
        }
    }

    /// Return a shared reference to this element's event-subscriber list.
    ///
    /// Works across all variants. O(1) — single match, no allocation.
    pub fn subscribers_as_ref(&self) -> &Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
        }
    }

    /// Overwrite the element's `unique_id`.
    ///
    /// * `id` — the new identity value.
    ///
    /// Works across all variants. O(1), no allocation.
    pub fn set_unique_id(&mut self, id: usize) {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id = id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id = id,
            GfxElement::Void { unique_id, .. } => *unique_id = id,
        }
    }

    /// Return the discriminant tag for this element.
    ///
    /// O(1), no allocation. Useful for branching without destructuring.
    pub fn get_type(&self) -> GfxElementType {
        match self {
            GfxElement::GlyphArea { .. } => GfxElementType::GlyphArea,
            GfxElement::Void { .. } => GfxElementType::Void,
            GfxElement::GlyphModel { .. } => GfxElementType::GlyphModel,
        }
    }

    /// Return a mutable reference to the inner [`GlyphArea`], or `None`
    /// if this element is not a `GlyphArea` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_area_mut(&mut self) -> Option<&mut GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => Some(section.as_mut()),
            GfxElement::Void { .. } => None,
            GfxElement::GlyphModel { .. } => None,
        }
    }

    /// Return a shared reference to the inner [`GlyphArea`], or `None`
    /// if this element is not a `GlyphArea` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_area(&self) -> Option<&GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => Some(section),
            GfxElement::Void { .. } => None,
            GfxElement::GlyphModel { .. } => None,
        }
    }

    /// Return a shared reference to the inner [`GlyphModel`], or `None`
    /// if this element is not a `GlyphModel` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_model(&self) -> Option<&GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    /// Return a mutable reference to the inner [`GlyphModel`], or `None`
    /// if this element is not a `GlyphModel` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_model_mut(&mut self) -> Option<&mut GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    /// Return the application-assigned identity for this element.
    ///
    /// Works across all variants. O(1), no allocation.
    pub fn unique_id(&self) -> usize {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id,
            GfxElement::Void { unique_id, .. } => *unique_id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id,
        }
    }

    /// Return the world-space position of this element.
    ///
    /// * `GlyphArea` / `GlyphModel` — returns the stored position as a
    ///   `Vec2`.
    /// * `Void` — returns `Vec2::NAN` (void nodes have no spatial
    ///   meaning).
    ///
    /// O(1), no allocation.
    pub fn position(&self) -> Vec2 {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => section.position.to_vec2(),
            GfxElement::Void { .. } => Vec2::NAN,
            GfxElement::GlyphModel { glyph_model, .. } => glyph_model.position.to_vec2(),
        }
    }

    /// Return the colour stored in the colour-font region at `range`,
    /// if one exists.
    ///
    /// * `GlyphArea` — looks up the region and returns its colour.
    /// * `GlyphModel` / `Void` — returns `None` (no colour-font
    ///   regions on these variants).
    ///
    /// O(1) region lookup (hash map).
    pub fn color_at_region(&self, range: Range) -> Option<FloatRgba> {
        match self {
            GfxElement::GlyphArea { glyph_area, .. } => glyph_area
                .regions
                .get(range)
                .and_then(|region| region.color),
            GfxElement::GlyphModel { .. } => None,
            GfxElement::Void { .. } => None,
        }
    }

    /// Set the world-space position of this element.
    ///
    /// * `GlyphArea` / `GlyphModel` — overwrites the stored position.
    /// * `Void` — no-op (void nodes have no spatial meaning).
    ///
    /// O(1), no allocation.
    pub fn set_position(&mut self, position: Vec2) {
        match self {
            GfxElement::GlyphArea {
                ref mut glyph_area, ..
            } => {
                glyph_area.position = OrderedVec2::from_vec2(position);
            }
            GfxElement::Void { .. } => {
                // Nothing needs doing
            }
            GfxElement::GlyphModel {
                ref mut glyph_model,
                ..
            } => {
                glyph_model.position = OrderedVec2::from_vec2(position);
            }
        }
    }

    /// Rotate this element's position clockwise around `pivot` by
    /// `degrees`.
    ///
    /// Void elements (whose position is `NAN`) are skipped. Delegates to
    /// [`clockwise_rotation_around_pivot`]. O(1), no allocation.
    pub fn rotate(&mut self, pivot: Vec2, degrees: f32) {
        let position = self.position();
        if !position.is_nan() {
            self.set_position(clockwise_rotation_around_pivot(position, pivot, degrees));
        }
    }

    /// Return the cached subtree AABB covering this node and all its
    /// descendants, or `None` if the cache has not been computed or has
    /// been invalidated.
    ///
    /// The tuple is `(top_left, bottom_right)` in tree-local coordinates.
    /// O(1), no allocation.
    pub fn subtree_aabb(&self) -> Option<(Vec2, Vec2)> {
        match self {
            GfxElement::GlyphArea { subtree_aabb, .. }
            | GfxElement::GlyphModel { subtree_aabb, .. }
            | GfxElement::Void { subtree_aabb, .. } => *subtree_aabb,
        }
    }

    /// Write the cached subtree AABB for this node.
    ///
    /// Called by [`Tree::compute_subtree_aabbs`] during the bottom-up
    /// pass. Application code should not call this directly — use the
    /// tree-level API instead.
    ///
    /// O(1), no allocation.
    pub fn set_subtree_aabb(&mut self, aabb: Option<(Vec2, Vec2)>) {
        match self {
            GfxElement::GlyphArea { subtree_aabb, .. }
            | GfxElement::GlyphModel { subtree_aabb, .. }
            | GfxElement::Void { subtree_aabb, .. } => *subtree_aabb = aabb,
        }
    }

    /// Clear the cached subtree AABB, forcing recomputation on next
    /// access. O(1), no allocation.
    pub fn invalidate_subtree_aabb(&mut self) {
        self.set_subtree_aabb(None);
    }
}

impl TreeNode for GfxElement {
    fn void() -> Self {
        Self::new_void_with_id(0, 0)
    }
}

impl Flaggable for GfxElement {
    fn flag_is_set(&self, flag: Flag) -> bool {
        match self {
            GfxElement::GlyphArea { flags, .. } => flags.contains(&flag),
            GfxElement::GlyphModel { flags, .. } => flags.contains(&flag),
            GfxElement::Void { .. } => false,
        }
    }

    fn set_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::Void { .. } => {}
        }
    }

    fn clear_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::Void { .. } => {}
        }
    }
}

impl Debug for GfxElement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GfxElement::GlyphArea { channel, unique_id, glyph_area, .. } => {
                write!(f, "GlyphArea(id={}, ch={}, text={:?})", unique_id, channel, glyph_area.text)
            }
            GfxElement::GlyphModel { channel, unique_id, .. } => {
                write!(f, "GlyphModel(id={}, ch={})", unique_id, channel)
            }
            GfxElement::Void { channel, unique_id, .. } => {
                write!(f, "Void(id={}, ch={})", unique_id, channel)
            }
        }
    }
}

impl PartialEq for GfxElement {
    fn eq(&self, other: &Self) -> bool {
        if self.get_type() == other.get_type() {
            return match self.get_type() {
                GfxElementType::GlyphArea => {
                    *(self.glyph_area().unwrap()) == *(other.glyph_area().unwrap())
                }
                GfxElementType::GlyphModel => {
                    *self.glyph_model().unwrap() == *other.glyph_model().unwrap()
                }
                GfxElementType::Void => true,
            };
        }
        false
    }
}

impl Clone for GfxElement {
    fn clone(&self) -> Self {
        match self.get_type() {
            GfxElementType::GlyphArea => {
                let mut output = GfxElement::new_area_non_indexed_with_id(
                    self.glyph_area().unwrap().clone(),
                    self.channel(),
                    self.unique_id(),
                );

                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
            GfxElementType::GlyphModel => {
                let mut output = GfxElement::new_model_non_indexed_with_id(
                    self.glyph_model().unwrap().clone(),
                    self.channel(),
                    self.unique_id(),
                );

                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
            GfxElementType::Void => {
                let mut output = GfxElement::new_void_with_id(self.channel(), self.unique_id());
                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
        }
    }
}

impl TreeEventConsumer for GfxElement {
    fn accept_event(&mut self, event: &GlyphTreeEventInstance) {
        let subscribers = self.subscribers_as_ref().clone();
        for sub in subscribers {
            sub.lock()
                .expect("Failed to acquire lock for EventSubscriber")(
                self, event.clone()
            );
        }
    }
}

impl BranchChannel for GfxElement {
    fn channel(&self) -> usize {
        match self {
            GfxElement::GlyphArea { channel, .. } => *channel,
            GfxElement::Void { channel, .. } => *channel,
            GfxElement::GlyphModel { channel, .. } => *channel,
        }
    }
}

impl Default for GfxElement {
    fn default() -> Self {
        Self::void()
    }
}
