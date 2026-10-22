# Animation Roadmap

**Status:** Roadmap, not contract. Animations are not currently
tested end-to-end, and large parts of the infrastructure are
deliberately dormant. Treat this document as the agreed-upon
direction, not a feature you can rely on today.

This doc exists because the border / section-frame work in this
batch is a step toward a wider goal: borders (and other Mandala
visuals) becoming a creative toolkit that authors can animate to
tell stories. Today's customization machinery (any glyph, any
[`SidePattern`](./border-patterns.md), any color, any palette,
any font) is the static half. This file enumerates what would
need to land for the dynamic half.

## What works today

- **`MindNode.position` lerp.** A `CustomMutation` carrying
  `timing: { duration_ms, delay_ms, easing }` interpolates a
  node's position from its pre-mutation state to its post-mutation
  state via `Vec2::lerp` in
  `src/application/document/animations.rs::tick_animations`.
  Drives the basic "node slides into place" effect.
- **Easing curves.** `mindmap/animation.rs::Easing::{Linear,
  EaseIn, EaseOut, EaseInOut}` — used by the position lerp
  above.
- **`AnimationTiming` envelope on `CustomMutation`.** Authors
  set `duration_ms` / `delay_ms` / `easing` on a per-mutation
  basis; see [`./mutations.md`](./mutations.md) `timing` field.
- **Position-only snapshot interpolation.** When a triggered
  mutation has `timing.duration_ms > 0`, the runtime captures a
  `from_node` snapshot, derives the `to_node` by applying the
  mutation's effects, and **lerps `MindNode.position` between them
  per frame** (`from.pos_vec2().lerp(to.pos_vec2(), t)` in
  `tick_animations`). Every other field on `MindNode` snaps to the
  post-mutation state at completion — there is no general
  field-by-field tween. Completion routes through the normal
  `apply_custom_mutation` path so the model state lands cleanly.

## What's dormant (defined but unused)

These exist in the source but are not wired into the live
runtime. Listed so future work has a clear starting point and so
authors don't accidentally write maps that depend on them.

- **`lerp_color` and `lerp_vec2`** in
  `lib/baumhard/src/mindmap/animation.rs` — color and vector
  blend helpers. Defined; not called by `tick_animations`.
- **`Followup::{Reverse, Chain, Loop}`** —
  `mindmap/animation.rs::Followup` describes how an animation
  continues after its primary duration ends. The variant
  derives compile cleanly, and `Followup::Loop` would be the
  natural seat for a perpetual border animation. **But** the
  field is `#[serde(skip)]` and the dispatcher in
  `tick_animations` only handles the no-followup case. The
  test suite explicitly pins
  `test_an_authored_followup_reaches_no_field` so a future
  deserializer change is forced through review.
- **`apply_position_mutations_to_node`** in
  `src/application/document/animations.rs` covers `NudgeLeft /
  Right / Up / Down` only. Every other `GlyphAreaCommand` is a
  no-op on the snapshot today. So a `CustomMutation` that
  changes (say) glyph text would compile and dispatch but
  wouldn't animate — it would jump to the post-state.

A fourth entry used to sit here: `core/animation.rs`'s generic
`Timeline` / `TimelineEvent` skeleton, offered as the long-form
scheduling primitive. It is gone (#41). It was not a partial
implementation of the item below — it was an API that could not
have worked: its `AnimationDef` boxed *values* rather than
mutators, its `update` took its instance by value and returned
nothing, and its `Mutable` bound had no implementers, so no
scheduler could have driven it without a rewrite. The long-form
scheduling need it stood for is real and is item 3 below;
whoever lands it starts from `AnimationTiming` + `Followup`,
which already speak the model this crate animates.

## What's missing for animated borders specifically

A user who wants to author "marquee text 'HELLO WORLD' scrolling
around a node's border" needs:

1. **A trigger that fires on a clock, not a click.**
   `CustomMutation::Trigger` covers `OnClick`, `OnHover`,
   `OnKey(_)`, `OnLink(_)` only. An animated border needs
   `OnTimer { interval_ms }` or `OnLoad` (start once at map
   load, run forever).
2. **A per-tick mutation surface for border `GlyphArea` leaves.**
   Today `tree_builder/border.rs` rebuilds the eight border runs
   from `node.style.border` on every scene rebuild. A per-tick
   animation either (a) dirties the model's `border` field and
   triggers `rebuild_all` every tick — expensive and wasteful —
   or (b) applies `GlyphAreaCommand`s directly to the registered
   `Borders` canvas tree leaves keyed by `(node parent_channel,
   run channel)`, bypassing model-rebuild. The §B2 in-place
   mutator path (`build_border_mutator_tree_from_nodes` →
   `apply_canvas_mutator`) is the natural seam — it already
   re-stamps every `GlyphArea` field cheaply — but is currently
   only called from scene-rebuild, not from a tick driver.
3. **`Followup::Loop` deserialization + dispatch.** A border
   animation is by nature perpetual; without `Loop` wired, every
   animation completes after one cycle.
4. **`apply_position_mutations_to_node` extension.** Either
   generalize the snapshot logic to cover every
   `GlyphAreaCommand` (so any mutation animates), or introduce a
   parallel "animation mutator" type that schedules
   `GlyphAreaCommand`s on a tick rather than blending two
   snapshots.
5. **(Optionally) Declarative animated-border model fields.** A
   shape like `border.animation: { kind: "marquee", text:
   "HELLO WORLD", speed_glyphs_per_sec: 4.0, direction: Right
   }` lets authors describe the effect without hand-rolling a
   `CustomMutation`. Whether to go declarative or keep
   everything in `CustomMutation` is an open design call.

## What's missing more broadly

For the "borders that tell a story" direction:

- **Author-visible animation surface.** Console verbs like
  `border animate marquee text="HELLO" speed=4` would let
  authors prototype animations without hand-editing JSON.
- **Test coverage.** `lerp_color` / `lerp_vec2` /
  `Followup::*` — the dormant primitives have no integration
  tests because they have no consumer.
- **Performance budget.** A per-tick border re-shape touches
  cosmic-text's shaping pipeline. We'd need a benchmark before
  shipping animations on a map with hundreds of borders.
- **Section frames.** Once node borders animate, the same
  surface applies to `MindSection.frame_border` and
  `Canvas.default_section_frame_border` (per the
  Section-frames-as-creative-toolkit work landed alongside this
  doc) — the resolver is shared.

## Non-commitment

None of the above is scheduled. This doc exists so that:

1. Authors who try `Followup::Loop` and find it doesn't work
   know it's not a bug — it's deliberately dormant.
2. Future contributors picking up animation work don't have to
   re-derive the gap analysis.
3. The static border / section-frame work landing today doesn't
   foreclose any of these directions — every cascade level
   (`MindSection.frame_border`, `Canvas.default_*`,
   `MindNode.style.border`) carries a `GlyphBorderConfig` that
   can grow an `animation` field without breaking existing
   consumers.

When animation work resumes, start by enabling
`Followup::Loop` deserialization, adding an `OnLoad` /
`OnTimer` trigger, and wiring a tick driver against the §B2
in-place mutator path so border content can change per frame
without per-tick `rebuild_all` cost.
