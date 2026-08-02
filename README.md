# Mandala

Mandala is a Rust mindmap application built on
[wgpu](https://wgpu.rs/) and
[cosmic-text](https://github.com/pop-os/cosmic-text), using the
**Baumhard** glyph-animation library under
[`lib/baumhard/`](lib/baumhard/). It runs on both native desktop and
as a WebAssembly build. `.mindmap.json` files load and render as
interactive canvases where every visual element — text, borders,
connection paths — is laid out as positioned font glyphs.

## What..?

A few years ago I decided to start developing a "next-gen ascii art" game engine (Baumhard, under lib/baumhard). 
I would then use that engine to develop a game, but I eventually scrapped the idea. 
Then in 2026 I was studying the Old Testament for fun, and I started mapping out the family tree in my favorite 
mind-mapping app. Eventually the tree became too big for the app, and I couldn't find any good open source replacement.

So I figured that maybe I can use Claude Code to build a mind-mapping tool based on Baumhard, and so here we are.
My time and resources are quite limited these days so things do take time, but I have a crystal clear vision and I will
certainly make it happen.

My plan is to use the foundation I laid out in Baumhard to create a fully **animatable mind-map**. 

Very much still learning how to optimally work with Claude Code. I'm working on exposing an LLM-friendly
IPC interface so that I can give Claude a better feedback loop.

## Animatable mind-map

I find visual graphs to be one of the most powerful ways that you can present information. Flow-charts, relational trees, 
file systems, decision-trees, it's everywhere. Typical mind-mapping apps are very static, I want to take it one step further
and create a tool that allows users to embed scripts and animations within the map. Then, although you can obviously use it just like a normal mind-mapping tool, you have the option to create something more interactable. For example embedding different color themes, different layouts, sequential mutation of the map, and so forth. 

So the codebase right now contains a lot of unconnected dots that will start to make sense as development continues. 


## Quickstart

```sh
./test.sh                              # full test suite + wasm32 type-check
./build.sh                             # release build (native + wasm)
./run.sh maps/testament.mindmap.json   # native + trunk serve in parallel
```

For one-off iteration:

```sh
cargo run -- maps/testament.mindmap.json   # native only
trunk serve                                 # WASM only (loads via ?map=…)
```

## Where to read next

| Document                                        | What it covers                                                            |
| ----------------------------------------------- | ------------------------------------------------------------------------- |
| [`CLAUDE.md`](CLAUDE.md)                        | Special instructions for Claude                                           |
| [`CONCEPTS.md`](CONCEPTS.md)                    | Conceptual building-blocks (`GlyphArea`, `MutatorTree`, `Channel`, ...)   |
| [`CODE_CONVENTIONS.md`](CODE_CONVENTIONS.md)    | Workspace-wide coding conventions and philosophy (mandatory)              |
| [`lib/baumhard/CONVENTIONS.md`](lib/baumhard/CONVENTIONS.md) | Crate-local rules for Baumhard (mutation-not-rebuild, arena, ...)        |
| [`TEST_CONVENTIONS.md`](TEST_CONVENTIONS.md)    | Testing philosophy + the `do_*()` benchmark-reuse pattern                 |
| [`format/`](format/)                            | The `.mindmap.json` format spec; start with `format/schema.md`            |

## Repository layout

- [`src/application/`](src/application/) — the app shell (event loop,
  document state, rendering pipeline, input handling).
- [`lib/baumhard/`](lib/baumhard/) — data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- [`crates/maptool/`](crates/maptool/) — CLI for working with
  `.mindmap.json` files: `show`, `grep`, `apply`, `export`,
  `convert --legacy`, `verify`.
- [`lib/mandala_derive/`](lib/mandala_derive/) — proc-macro support.

## License

MPL-2.0 — see per-file SPDX identifiers.
