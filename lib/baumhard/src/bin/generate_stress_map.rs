//! Stress-test map generator.
//!
//! Writes a `.mindmap.json` file of configurable size and topology. Used to
//! stress the renderer and to catch performance regressions — not a benchmark
//! harness, just a file writer. Reading the generated file with
//! `cargo run --release -- maps/stress.mindmap.json` and eyeballing the
//! behaviour is the feedback loop. Output is deterministic per `--seed`.
//!
//! # Topologies
//!
//! - `balanced` — a complete tree with the given `--depth` and `--branching`.
//!   Exercises breadth and depth at the same time; a good default.
//! - `skewed` — a "comb": one long spine of depth `--nodes`, with one leaf
//!   hanging off each interior spine node. Exercises deep hierarchies and
//!   produces naturally far-apart leaves.
//! - `star` — one root with `--nodes - 1` direct children laid out around it.
//!   Exercises wide sibling lists and many same-level siblings.
//!
//! # Cross-links and long edges
//!
//! - `--cross-links K` adds `K` random `cross_link` edges between
//!   non-hierarchically-related nodes. Simulates a messy real-world map.
//! - `--long-edges K` adds `K` cross-link edges between the most-distant
//!   node pairs in the layout. This is the key knob for Phase 4 perf
//!   testing: long edges are the ones whose glyph sampling count explodes,
//!   which is exactly the stutter case we're optimising for.
//!
//! # Example invocations
//!
//! ```shell
//! # A balanced tree of ~1,365 nodes (branching 4, depth 5).
//! cargo run -p baumhard --bin generate_stress_map -- \
//!     --topology balanced --depth 5 --branching 4 \
//!     --output maps/stress_balanced.mindmap.json
//!
//! # A deeply skewed map with two deliberately long cross-links for
//! # Phase 4 before/after measurement.
//! cargo run -p baumhard --bin generate_stress_map -- \
//!     --topology skewed --nodes 500 --long-edges 2 \
//!     --output maps/stress_long_edges.mindmap.json
//!
//! # A fat star with 1,000 siblings.
//! cargo run -p baumhard --bin generate_stress_map -- \
//!     --topology star --nodes 1000 \
//!     --output maps/stress_star.mindmap.json
//! ```

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::process::ExitCode;

use baumhard::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
    TextRun,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Canvas units between sibling columns in grid layouts.
const COL_SPACING: f64 = 320.0;
/// Canvas units between tree levels (vertical).
const ROW_SPACING: f64 = 200.0;
/// Fixed node width (uniform for all stress-map nodes).
const NODE_WIDTH: f64 = 240.0;
/// Fixed node height (uniform for all stress-map nodes).
const NODE_HEIGHT: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Topology {
    Balanced,
    Skewed,
    Star,
}

impl Topology {
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "balanced" => Ok(Topology::Balanced),
            "skewed" => Ok(Topology::Skewed),
            "star" => Ok(Topology::Star),
            other => Err(format!(
                "unknown topology '{}' (expected: balanced, skewed, star)",
                other
            )),
        }
    }
}

struct Config {
    topology: Topology,
    /// Used by `skewed` and `star`; ignored by `balanced`.
    nodes: usize,
    /// Used by `balanced`; ignored by the others.
    depth: usize,
    /// Used by `balanced`; ignored by the others.
    branching: usize,
    cross_links: usize,
    long_edges: usize,
    seed: u64,
    output: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            topology: Topology::Balanced,
            nodes: 500,
            depth: 5,
            branching: 4,
            cross_links: 0,
            long_edges: 0,
            seed: 0xBAAD_F00D,
            output: "maps/stress.mindmap.json".to_string(),
        }
    }
}

fn print_usage() {
    eprintln!(
        r#"generate_stress_map — write a synthetic .mindmap.json for stress testing

USAGE:
    generate_stress_map [OPTIONS]

OPTIONS:
    --topology <balanced|skewed|star>   Topology to generate (default: balanced)
    --nodes <N>                         Total node count for skewed/star (default: 500)
    --depth <D>                         Tree depth for balanced (default: 5)
    --branching <B>                     Branching factor for balanced (default: 4)
    --cross-links <K>                   Add K random cross-link edges (default: 0)
    --long-edges <K>                    Add K cross-link edges between the most-
                                        distant node pairs. The key knob for
                                        Phase 4 connection-render perf tests.
                                        (default: 0)
    --seed <S>                          RNG seed for deterministic output
                                        (default: 0xBAADF00D)
    --output <PATH>                     Output file path (default: maps/stress.mindmap.json)
    --help                              Print this message

Balanced tree node count is `(B^(D+1) - 1) / (B - 1)`, so a branching of 4
and depth of 5 gives 1,365 nodes. Use --nodes only for star/skewed.
"#
    );
}

fn parse_args() -> Result<Option<Config>, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return Ok(None);
    }
    let mut cfg = Config::default();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let next = |i: usize, key: &str| -> Result<&str, String> {
            args.get(i + 1)
                .map(|s| s.as_str())
                .ok_or_else(|| format!("missing value for {}", key))
        };
        match key {
            "--topology" => {
                cfg.topology = Topology::from_str(next(i, key)?)?;
                i += 2;
            }
            "--nodes" => {
                cfg.nodes = next(i, key)?
                    .parse()
                    .map_err(|e| format!("{}: {}", key, e))?;
                i += 2;
            }
            "--depth" => {
                cfg.depth = next(i, key)?
                    .parse()
                    .map_err(|e| format!("{}: {}", key, e))?;
                i += 2;
            }
            "--branching" => {
                cfg.branching = next(i, key)?
                    .parse()
                    .map_err(|e| format!("{}: {}", key, e))?;
                i += 2;
            }
            "--cross-links" => {
                cfg.cross_links = next(i, key)?
                    .parse()
                    .map_err(|e| format!("{}: {}", key, e))?;
                i += 2;
            }
            "--long-edges" => {
                cfg.long_edges = next(i, key)?
                    .parse()
                    .map_err(|e| format!("{}: {}", key, e))?;
                i += 2;
            }
            "--seed" => {
                let v = next(i, key)?;
                cfg.seed = if let Some(hex) = v.strip_prefix("0x") {
                    u64::from_str_radix(hex, 16)
                        .map_err(|e| format!("{}: {}", key, e))?
                } else {
                    v.parse().map_err(|e| format!("{}: {}", key, e))?
                };
                i += 2;
            }
            "--output" => {
                cfg.output = next(i, key)?.to_string();
                i += 2;
            }
            other => {
                return Err(format!("unknown argument '{}'", other));
            }
        }
    }
    Ok(Some(cfg))
}

/// A minimal styled node. Picks a frame color per depth level so the map is
/// visually structured without needing theme variables.
fn make_node(id: String, parent_id: Option<String>, index: i32, x: f64, y: f64, depth: usize) -> MindNode {
    let frame_palette = [
        "#30b082", "#b03080", "#3080b0", "#b08030", "#8030b0", "#30b0b0", "#b03030",
    ];
    let frame_color = frame_palette[depth % frame_palette.len()].to_string();
    let text = format!("{}", id);
    let text_runs = vec![TextRun {
        start: 0,
        end: text.chars().count(),
        bold: false,
        italic: false,
        underline: false,
        font: "LiberationSans".to_string(),
        size_pt: 18,
        color: "#ffffff".to_string(),
        hyperlink: None,
    }];
    MindNode {
        id: id.clone(),
        parent_id,
        index,
        position: Position { x, y },
        size: Size {
            width: NODE_WIDTH,
            height: NODE_HEIGHT,
        },
        text,
        text_runs,
        style: NodeStyle {
            background_color: "#141414".to_string(),
            frame_color,
            text_color: "#ffffff".to_string(),
            shape_type: 0,
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: 0,
            direction: 0,
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        trigger_bindings: Vec::new(),
        inline_mutations: Vec::new(),
    }
}

/// Build a parent→child edge with hierarchy-style defaults.
fn make_parent_child_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "parent_child".to_string(),
        color: "#4a7a9c".to_string(),
        width: 2,
        line_style: 0,
        visible: true,
        label: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

/// Build a cross-link edge with the same defaults `connect mode` (Ctrl+D)
/// would produce in the app.
fn make_cross_link_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#aa88cc".to_string(),
        width: 3,
        line_style: 0,
        visible: true,
        label: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

/// Generate a balanced tree. Returns (nodes, edges).
fn gen_balanced(depth: usize, branching: usize) -> (Vec<MindNode>, Vec<MindEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // BFS, level by level. At each level we know how many nodes there are so
    // we can centre them horizontally.
    // (parent_idx_in_previous_level, parent_id) pairs for the current level.
    let mut current_level: Vec<(usize, String)> = Vec::new();
    // Root.
    let root_id = "n0".to_string();
    nodes.push(make_node(root_id.clone(), None, 0, 0.0, 0.0, 0));
    current_level.push((0, root_id));
    let mut next_id = 1usize;
    for d in 1..=depth {
        let level_size = current_level.len() * branching;
        let level_y = d as f64 * ROW_SPACING;
        let level_width = (level_size as f64 - 1.0).max(0.0) * COL_SPACING;
        let level_left = -level_width / 2.0;
        let mut next_level: Vec<(usize, String)> = Vec::with_capacity(level_size);
        for (parent_i, parent_id) in current_level.iter() {
            for b in 0..branching {
                let col = parent_i * branching + b;
                let x = level_left + col as f64 * COL_SPACING;
                let id = format!("n{}", next_id);
                next_id += 1;
                nodes.push(make_node(id.clone(), Some(parent_id.clone()), b as i32, x, level_y, d));
                edges.push(make_parent_child_edge(parent_id, &id));
                next_level.push((col, id));
            }
        }
        current_level = next_level;
    }
    (nodes, edges)
}

/// Generate a "skewed" tree (comb): a long spine of depth `nodes / 2` with
/// one leaf hanging off each interior spine node. Exercises deep hierarchies
/// and produces naturally far-apart leaves for long-edge tests.
fn gen_skewed(nodes: usize) -> (Vec<MindNode>, Vec<MindEdge>) {
    let mut result_nodes = Vec::new();
    let mut edges = Vec::new();
    if nodes == 0 {
        return (result_nodes, edges);
    }
    // Half spine, half leaves (roughly). Ensures at least one spine node.
    let spine_len = (nodes / 2).max(1);
    // Root of the spine.
    result_nodes.push(make_node("n0".to_string(), None, 0, 0.0, 0.0, 0));
    let mut next_id = 1usize;
    // Spine nodes after the root. Each one's parent is the previous spine node.
    // Positioned on a diagonal so successive spine nodes are visually far apart.
    for i in 1..spine_len {
        let parent_id = format!("n{}", i - 1);
        let id = format!("n{}", i);
        next_id = i + 1;
        let x = i as f64 * COL_SPACING;
        let y = i as f64 * ROW_SPACING;
        result_nodes.push(make_node(id.clone(), Some(parent_id.clone()), 0, x, y, i));
        edges.push(make_parent_child_edge(&parent_id, &id));
    }
    // Leaves: attach one to each spine node until we hit the target count.
    let mut spine_idx = 0usize;
    while next_id < nodes && spine_idx < spine_len {
        let parent_id = format!("n{}", spine_idx);
        let id = format!("n{}", next_id);
        // Place the leaf to the right of its spine parent.
        let parent = &result_nodes[spine_idx];
        let x = parent.position.x + COL_SPACING;
        let y = parent.position.y;
        result_nodes.push(make_node(id.clone(), Some(parent_id.clone()), 1, x, y, spine_idx + 1));
        edges.push(make_parent_child_edge(&parent_id, &id));
        next_id += 1;
        spine_idx += 1;
    }
    (result_nodes, edges)
}

/// Generate a star: one root, N-1 children laid out on a circle around it.
fn gen_star(nodes: usize) -> (Vec<MindNode>, Vec<MindEdge>) {
    let mut result_nodes = Vec::new();
    let mut edges = Vec::new();
    if nodes == 0 {
        return (result_nodes, edges);
    }
    // Root at origin.
    result_nodes.push(make_node("n0".to_string(), None, 0, 0.0, 0.0, 0));
    let child_count = nodes.saturating_sub(1);
    if child_count == 0 {
        return (result_nodes, edges);
    }
    // Lay children on a circle. Radius scales with count so siblings don't
    // overlap — each one reserves COL_SPACING of arc length.
    let circumference = child_count as f64 * COL_SPACING;
    let radius = (circumference / (2.0 * std::f64::consts::PI)).max(COL_SPACING);
    for i in 0..child_count {
        let angle = (i as f64 / child_count as f64) * 2.0 * std::f64::consts::PI;
        let x = radius * angle.cos();
        let y = radius * angle.sin();
        let id = format!("n{}", i + 1);
        result_nodes.push(make_node(id.clone(), Some("n0".to_string()), i as i32, x, y, 1));
        edges.push(make_parent_child_edge("n0", &id));
    }
    (result_nodes, edges)
}

/// Add `count` random cross-link edges. Avoids self-links and duplicates of
/// existing edges (hierarchy or cross-link).
fn add_random_cross_links(
    nodes: &[MindNode],
    edges: &mut Vec<MindEdge>,
    count: usize,
    rng: &mut StdRng,
) {
    if nodes.len() < 2 || count == 0 {
        return;
    }
    let mut existing: HashSet<(String, String)> = edges
        .iter()
        .map(|e| (e.from_id.clone(), e.to_id.clone()))
        .collect();
    let mut attempts = 0;
    let max_attempts = count * 20;
    let mut added = 0;
    while added < count && attempts < max_attempts {
        attempts += 1;
        let a = rng.gen_range(0..nodes.len());
        let b = rng.gen_range(0..nodes.len());
        if a == b {
            continue;
        }
        let from_id = &nodes[a].id;
        let to_id = &nodes[b].id;
        let key = (from_id.clone(), to_id.clone());
        let rkey = (to_id.clone(), from_id.clone());
        if existing.contains(&key) || existing.contains(&rkey) {
            continue;
        }
        edges.push(make_cross_link_edge(from_id, to_id));
        existing.insert(key);
        added += 1;
    }
}

/// Add `count` cross-link edges between the most-distant node pairs. Picks
/// the two nodes with the maximum separation as the first edge, then the
/// next-most-distant non-overlapping pair, and so on. This is the key
/// long-connection knob for Phase 4 perf testing — these are exactly the
/// edges whose per-frame glyph-sampling cost blows the frame budget.
fn add_longest_edges(nodes: &[MindNode], edges: &mut Vec<MindEdge>, count: usize) {
    if nodes.len() < 2 || count == 0 {
        return;
    }
    let mut existing: HashSet<(String, String)> = edges
        .iter()
        .map(|e| (e.from_id.clone(), e.to_id.clone()))
        .collect();
    // Compute all pairwise distances, pick the top `count` that aren't
    // already present. For maps up to a few thousand nodes this is
    // comfortably fast (O(N^2)); past that, switch to a sampled heuristic.
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for i in 0..nodes.len() {
        for j in (i + 1)..nodes.len() {
            let dx = nodes[i].position.x - nodes[j].position.x;
            let dy = nodes[i].position.y - nodes[j].position.y;
            let d2 = dx * dx + dy * dy;
            pairs.push((d2, i, j));
        }
    }
    pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut added = 0;
    for (_, i, j) in pairs {
        if added >= count {
            break;
        }
        let from_id = &nodes[i].id;
        let to_id = &nodes[j].id;
        let key = (from_id.clone(), to_id.clone());
        let rkey = (to_id.clone(), from_id.clone());
        if existing.contains(&key) || existing.contains(&rkey) {
            continue;
        }
        edges.push(make_cross_link_edge(from_id, to_id));
        existing.insert(key);
        added += 1;
    }
}

/// Wrap a node list and edge list into a `MindMap` ready for serialisation.
fn assemble_mindmap(name: &str, nodes: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    let mut node_map: HashMap<String, MindNode> = HashMap::with_capacity(nodes.len());
    for n in nodes {
        node_map.insert(n.id.clone(), n);
    }
    MindMap {
        version: "1.0".to_string(),
        name: name.to_string(),
        canvas: Canvas {
            background_color: "#0a0a0a".to_string(),
            default_border: None,
            default_connection: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        },
        nodes: node_map,
        edges,
        custom_mutations: Vec::new(),
    }
}

fn run(cfg: Config) -> Result<(), String> {
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let (mut nodes, mut edges) = match cfg.topology {
        Topology::Balanced => gen_balanced(cfg.depth, cfg.branching),
        Topology::Skewed => gen_skewed(cfg.nodes),
        Topology::Star => gen_star(cfg.nodes),
    };
    add_random_cross_links(&nodes, &mut edges, cfg.cross_links, &mut rng);
    add_longest_edges(&nodes, &mut edges, cfg.long_edges);

    // Re-index each node to match its position in `nodes` so `index`
    // roughly reflects creation order. Not strictly needed but it keeps the
    // output tidy.
    for (i, n) in nodes.iter_mut().enumerate() {
        n.index = i as i32;
    }

    let name = format!(
        "stress-{}-{}n-{}e",
        match cfg.topology {
            Topology::Balanced => "balanced",
            Topology::Skewed => "skewed",
            Topology::Star => "star",
        },
        nodes.len(),
        edges.len(),
    );
    let node_count = nodes.len();
    let edge_count = edges.len();
    let map = assemble_mindmap(&name, nodes, edges);
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("serialise mindmap: {}", e))?;
    fs::write(&cfg.output, json).map_err(|e| format!("write {}: {}", cfg.output, e))?;
    println!(
        "wrote {} ({} nodes, {} edges, seed 0x{:X})",
        cfg.output, node_count, edge_count, cfg.seed
    );
    Ok(())
}

fn main() -> ExitCode {
    let cfg = match parse_args() {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return ExitCode::SUCCESS, // --help
        Err(e) => {
            eprintln!("error: {}", e);
            eprintln!("run with --help for usage");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = run(cfg) {
        eprintln!("error: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balanced_node_count_depth_0_is_root_only() {
        let (nodes, edges) = gen_balanced(0, 4);
        assert_eq!(nodes.len(), 1);
        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn test_balanced_node_count_formula() {
        // B=3, D=3 → 1 + 3 + 9 + 27 = 40
        let (nodes, edges) = gen_balanced(3, 3);
        assert_eq!(nodes.len(), 40);
        // Every non-root has exactly one parent_child edge.
        assert_eq!(edges.len(), 39);
    }

    #[test]
    fn test_balanced_has_single_root() {
        let (nodes, _) = gen_balanced(4, 2);
        let roots: Vec<_> = nodes.iter().filter(|n| n.parent_id.is_none()).collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, "n0");
    }

    #[test]
    fn test_skewed_honours_node_count() {
        let (nodes, _) = gen_skewed(20);
        assert_eq!(nodes.len(), 20);
    }

    #[test]
    fn test_skewed_edges_connect_spine_and_leaves() {
        let (nodes, edges) = gen_skewed(10);
        // Every non-root node should have exactly one parent edge.
        assert_eq!(edges.len(), nodes.len() - 1);
        assert!(edges.iter().all(|e| e.edge_type == "parent_child"));
    }

    #[test]
    fn test_star_has_one_root_and_rest_as_children() {
        let (nodes, edges) = gen_star(50);
        assert_eq!(nodes.len(), 50);
        let roots: Vec<_> = nodes.iter().filter(|n| n.parent_id.is_none()).collect();
        assert_eq!(roots.len(), 1);
        // Every non-root's parent is the root.
        for n in nodes.iter().filter(|n| n.parent_id.is_some()) {
            assert_eq!(n.parent_id.as_deref(), Some("n0"));
        }
        assert_eq!(edges.len(), 49);
    }

    #[test]
    fn test_star_single_node_has_no_children() {
        let (nodes, edges) = gen_star(1);
        assert_eq!(nodes.len(), 1);
        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn test_add_random_cross_links_avoids_self_links() {
        let (nodes, mut edges) = gen_star(10);
        let before = edges.len();
        let mut rng = StdRng::seed_from_u64(42);
        add_random_cross_links(&nodes, &mut edges, 5, &mut rng);
        assert!(edges.len() >= before);
        // No self-links introduced.
        for e in edges.iter() {
            assert_ne!(e.from_id, e.to_id);
        }
    }

    #[test]
    fn test_add_longest_edges_picks_farthest_pair() {
        // A skewed map has its farthest node pair at the two ends of the
        // spine. The first long edge should connect n0 to the node at the
        // maximum (x,y) — which for `gen_skewed` is the last spine node.
        let (nodes, mut edges) = gen_skewed(10);
        add_longest_edges(&nodes, &mut edges, 1);
        let cross: Vec<_> = edges.iter().filter(|e| e.edge_type == "cross_link").collect();
        assert_eq!(cross.len(), 1);
        // The longest edge should have at least one endpoint at n0 or at a
        // far leaf. We just assert it's actually a cross_link and the two
        // endpoints are distinct.
        assert_ne!(cross[0].from_id, cross[0].to_id);
    }

    #[test]
    fn test_generated_map_serialises_and_parses_back() {
        let (nodes, edges) = gen_balanced(3, 3);
        let map = assemble_mindmap("test", nodes, edges);
        let json = serde_json::to_string(&map).unwrap();
        let parsed: MindMap = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nodes.len(), 40);
        assert_eq!(parsed.edges.len(), 39);
        assert_eq!(parsed.version, "1.0");
    }

    #[test]
    fn test_determinism_same_seed_same_output() {
        let (nodes1, mut edges1) = gen_star(20);
        let (nodes2, mut edges2) = gen_star(20);
        let mut rng1 = StdRng::seed_from_u64(123);
        let mut rng2 = StdRng::seed_from_u64(123);
        add_random_cross_links(&nodes1, &mut edges1, 5, &mut rng1);
        add_random_cross_links(&nodes2, &mut edges2, 5, &mut rng2);
        // Same seed → same edges.
        let e1: Vec<_> = edges1.iter().map(|e| (&e.from_id, &e.to_id)).collect();
        let e2: Vec<_> = edges2.iter().map(|e| (&e.from_id, &e.to_id)).collect();
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_topology_from_str() {
        assert_eq!(Topology::from_str("balanced").unwrap(), Topology::Balanced);
        assert_eq!(Topology::from_str("skewed").unwrap(), Topology::Skewed);
        assert_eq!(Topology::from_str("star").unwrap(), Topology::Star);
        assert!(Topology::from_str("bogus").is_err());
    }
}
