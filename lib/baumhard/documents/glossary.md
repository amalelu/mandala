# Glossary

### Mutator

From a high level context a mutator can refer to a whole [MutatorTree](#mutatortree), which defines a mutator that can be applied to a
[GlyphTree](#glyphtree).
In certain contexts mutator can also refer to a single [GfxMutator](#gfxmutator) node, or a specific enum type called [Mutator](#mutator-enum-type). But the
latter uses should
hopefully be fairly obvious when used, however it's important to be aware that there are several different concepts that
may be called a mutator.

### Target

This sometimes means the target of a mutator, i.e. a [GlyphTree](#glyphtree). It can obviously also mean a lot of other things
depending on context.
But it is important to know that specifically in Baumhard, target often means "the target of a mutation".

### Node
In Baumhard, when we speak of a node we are usually speaking about a tree-node. This can either be a node of a 
[target](#Target) or a [mutator](#Mutator).

### GlyphTree

A GlyphTree is what is often called target, it is the target of the [MutatorTree](#mutatortree). The GlyphTree is a tree of basic glyph
structures called [GfxElement](#gfxelement)s.
One GfxElement represents one node in the tree. 

### GfxElement
A GfxElement is a node of a [GlyphTree](#glyphtree). It may be one of three types: 
[GlyphArea](#glypharea), [GlyphModel](#glyphmodel), or [Void](#void). 
The mutating counter-part to GfxElement is [GfxMutator](#gfxmutator)

### GlyphArea
The GlyphArea is the most fundamental type of [GfxElement](#gfxelement) node; Baumhard will not show any glyphs unless there is a 
GlyphArea to contain them. It contains a text string, regions that defines the styling, position, bounds, 
and other metadata used for processing. 

### GlyphModel
GlyphModel is an auxiliary type of [GfxElement](#gfxelement) that can only be used together with a [GlyphArea](#glypharea).
It defines a [GlyphMatrix](#glyphmatrix) model that can be mutated individually, but then inserted into a GlyphArea.
This is useful, for example, to represent and control several independent objects within a single GlyphArea.

### Void
Apart from its existing uses in programming languages and software engineering, in Baumhard Void typically refers to a special
kind of node type that exists for both [GfxElement](#gfxelement) and [GfxMutator](#gfxmutator).
The purpose of this node is to allow creating trees that match structurally without necessarily defining a proper node 
in every branch. It is never required, but is a useful tool for constructing more elegant trees.

### GlyphMatrix
GlyphMatrix is used by [GlyphModel](#glyphmodel) and simply contains the matrix that defines the model. 
It is a container of [GlyphLine](#glyphline) - that is, it contains zero or more GlyphLines.

### GlyphLine
GlyphLine is what makes up [GlyphMatrix](#glyphmatrix). It represents one line, or row in a matrix.
Like GlyphMatrix is a container of GlyphLines, GlyphLine is a container of [GlyphComponent](#glyphcomponent).

### GlyphComponent
Text, font, and color. A GlyphComponent defines one part of a [GlyphLine](#glyphline), which is built up by zero or
more GlyphComponent.

### MutatorTree
The mutating counter-part to a [GlyphTree](#glyphtree). Most generally, this is what we often mean by [mutator](#mutator)

### GfxMutator
A single node of a [MutatorTree](#mutatortree). It is technically a mutator on its own, but in order to mutate 
a tree you obviously need a tree, so for that reason this is not typically what we mean by [Mutator](#mutator)
unless it is in this specific context.

### Channel
In the context of [GlyphTree](#glyphtree) and [MutatorTree](#mutatortree), every node has a channel. 
More specifically [GfxElement](#gfxelement) and [GfxMutator](#gfxmutator) both have a channel that defaults to 0.
Mutation will only be performed between nodes whose channel, and hierarchy in the tree match. Channels are a means to
have finer control over what nodes will be affected by mutation. It can also refer to various other technical things depending
on the context.

### Event
A special kind of [Mutator](#Mutator) that does not alter its target state, but allows the target to react
to some event. It contains a [GlyphTreeEventInstance](#GlyphTreeEventInstance)

### GlyphTreeEventInstance
A specific instance of an [event](#Event), it contains a timestamp and a [GlyphTreeEvent](#GlyphTreeEvent)

### GlyphTreeEvent
An enum type where each variant is a kind of [event](#Event). For example KeyboardEvent.

### Flag
Flags are a way to store arbitrary state data on the [node](#Node) level

### GlyphTreeFlag
Implementation of [Flag](#Flag). 
