# Channels

Every node carries a `channel: usize` field (default 0). The channel is an
index that determines which mutators apply to the node when a mutation
tree walks the baumhard tree.

```json
"0": { "id": "0", "channel": 0, ... },
"1": { "id": "1", "channel": 1, ... },
"2": { "id": "2", "channel": 1, ... }
```

## What a channel is

A mutator tree walking the GfxElement tree matches children against target
children by channel. When a mutator on channel N reaches a tree level,
it applies to all children on channel N and skips the rest.

Channels are **not unique**. Multiple siblings can share a channel to form
a broadcast group — one mutator hits all of them. They are **not
hierarchical**. A parent and child can share a channel, or have different
channels; the walker only compares channels within the same sibling level.

## Why mirror this on MindNode?

The baumhard tree (the GfxElement layer that renders nodes) already has
a channel on every element. Until now, `tree_builder` hardcoded channel 0
for every mindmap-derived element, which meant every mutation broadcast
to every child — no way to target a subset.

Adding `channel` to `MindNode` surfaces the runtime concept in the format,
where authors can tag children as "emphasis" (channel 1) vs "normal"
(channel 0) and apply different animations to each group.

This is the foundation for:

- **Selective custom mutations** — `CustomMutation` with a `ChildrenOnChannel(n)`
  target scope (future work) applies only to children on a specific channel
- **Partial animations** — animate the channel-2 children out without
  touching the channel-0 children
- **In-place mutation updates** — the existing pattern used by portals,
  borders, and connections (where stable channels enable delta application
  instead of full tree rebuild) becomes available to node mutations

## Sorting constraint

The tree walker assumes strictly ascending channels within sibling groups.
If siblings share channels, the shared-channel nodes must be contiguous in
sibling order. The tree_builder handles this by sorting children by
(channel, id_sort_key) when constructing the baumhard tree.

From the format perspective: you can set channels however you like.
The runtime handles ordering.

## Backward compatibility

Every node in an existing map defaults to `channel: 0`. All current maps
behave identically before and after the channel field was added. Users
opt in by setting non-zero channels on specific nodes.
