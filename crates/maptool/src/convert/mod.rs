// SPDX-License-Identifier: MPL-2.0

//! One-way migration from the legacy miMind-derived `.mindmap.json`
//! format to the current one. Submodules perform one transform each
//! (IDs, enums, palettes, cleanup, portals, sections); pipeline order
//! is fixed in `convert_legacy`.
//!
//! Every verb runs through one scaffold — [`transform_map_file`] — so
//! the read / parse / serialize / write sequence exists once and all
//! three verbs land on disk the same way: through `write_atomic`'s
//! staging file plus a rename. That is what makes an in-place
//! conversion (`input == output`) safe. The original bytes stay
//! untouched until the rename swaps in the finished file, so a kill
//! mid-write leaves either the old file intact or the new one
//! complete — never the user's only copy truncated to a partial.

mod cleanup;
mod enums;
mod ids;
mod palettes;
mod portals;
mod sections;

pub use portals::convert_portals;
pub use sections::convert_sections;

use baumhard::mindmap::loader::write_atomic;
use serde_json::Value;
use std::path::Path;

/// Mutable handle to `root.nodes`. Returns `None` when missing or
/// wrong-typed — passes treat those as already-clean.
fn nodes_obj_mut(root: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    root.get_mut("nodes").and_then(|v| v.as_object_mut())
}

/// Mutable handle to `root.edges`.
fn edges_arr_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    root.get_mut("edges").and_then(|v| v.as_array_mut())
}

/// Mutable handle to legacy `root.portals` — a non-empty one is
/// rejected by the current-format loader, and only legacy passes
/// touch it.
fn portals_arr_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    root.get_mut("portals").and_then(|v| v.as_array_mut())
}

/// Read `input_path` as JSON, hand the parsed tree to `transform`,
/// and write the transformed tree to `output_path`.
///
/// The shared scaffolding behind every `convert` verb. `transform`
/// returns the one-line summary printed to stderr *after* the write
/// succeeds, so a conversion that fails on the way to disk never
/// reports a count.
///
/// The write goes through
/// [`write_atomic`] — a
/// sibling staging file plus a rename — which is what lets callers
/// pass the same path for input and output: the read has already
/// completed, and the original file is never opened for writing, so
/// an interrupted run cannot truncate it.
///
/// Cost: one full read plus one full write of the file, and one
/// `serde_json::Value` tree resident in memory between them.
fn transform_map_file<F>(input_path: &Path, output_path: &Path, transform: F) -> Result<(), String>
where
    F: FnOnce(&mut Value) -> Result<String, String>,
{
    let content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;

    let mut root: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    let summary = transform(&mut root)?;

    let json = serde_json::to_string_pretty(&root).map_err(|e| format!("failed to serialize: {e}"))?;
    write_atomic(output_path, &json)?;

    eprintln!("{summary}");
    Ok(())
}

/// Read a legacy `.mindmap.json`, convert it to the current format, and
/// write the result to `output_path`. Input and output may be the same
/// path; see [`transform_map_file`] for why that is safe.
pub fn convert_legacy(input_path: &Path, output_path: &Path) -> Result<(), String> {
    transform_map_file(input_path, output_path, |root| {
        // Order matters: IDs first so the rest can rewrite references;
        // the portal fold straight after, while the endpoint ids the
        // ids pass just rewrote are still where it expects them, so
        // nothing downstream has to know legacy portals existed;
        // enums before palettes so `theme_id` is gone before the
        // palette hoist; cleanup before the section fold so any legacy
        // `text` / `text_runs` survives the cleanup pass and gets
        // folded into a `sections[]` array at the end. Sections last
        // so an already-cleaned tree converges on the post-section
        // shape.
        let nodes = root
            .get("nodes")
            .and_then(|v| v.as_object())
            .ok_or_else(|| "missing or invalid \"nodes\" object".to_string())?;
        let id_map = ids::assign_dewey_ids(nodes);
        ids::rewrite_ids(root, &id_map);
        let folded_portals = portals::fold_portals_into_edges(root)?;
        enums::convert_enums(root);
        palettes::hoist_palettes(root);
        cleanup::cleanup_nodes(root);
        if let Some(nodes) = nodes_obj_mut(root) {
            for (_id, node) in nodes.iter_mut() {
                sections::migrate_one_node(node);
            }
        }

        let edge_count = root
            .get("edges")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        Ok(format!(
            "converted {} nodes, {} edges ({} folded in from legacy portals)",
            id_map.len(),
            edge_count,
            folded_portals
        ))
    })
}

/// Digest of a whole map file, for tests that assert byte stability.
///
/// A converted map is two screens of JSON; putting one in an
/// assertion means a failure prints both copies and buries the single
/// key that moved. Comparing digests keeps `assert_eq!` and the exact
/// byte-for-byte property (TEST_CONVENTIONS §T5 — improve the values
/// being compared, not the macro) while the failure output stays two
/// numbers and the message that explains them. Pair it with a parsed
/// `serde_json::Value` comparison when the test also wants to name
/// *which* key drifted.
#[cfg(test)]
pub(crate) fn content_digest(contents: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::util::test_temp::TempDir;
    use std::path::PathBuf;

    /// One entry per `convert` verb, so an atomicity claim is checked
    /// against all of them rather than whichever one happened to be
    /// written first.
    type ConvertVerb = fn(&Path, &Path) -> Result<(), String>;
    const VERBS: &[(&str, ConvertVerb)] = &[
        ("--legacy", convert_legacy),
        ("--portals", convert_portals),
        ("--sections", convert_sections),
    ];

    /// A genuine legacy map: opaque numeric ids, integer enums,
    /// node-level `text`, a top-level `portals[]`. Every verb finds
    /// something to change in it, which is what makes it usable as
    /// the input for all three.
    fn legacy_fixture_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/legacy_with_portals.mindmap.json");
        p
    }

    use super::content_digest as digest;

    /// The atomicity contract, tested rather than asserted: a
    /// converter must never write *through* the file already at the
    /// output path. It stages the new content elsewhere and renames
    /// it into place, so the old bytes survive untouched right up to
    /// the instant the swap happens — which is what makes a kill
    /// mid-write leave either the old file or the new one, never a
    /// truncated partial.
    ///
    /// A hard link is the witness. It names the *original* inode; a
    /// `fs::write` would truncate and rewrite that inode in place and
    /// the witness would see the new bytes, while a rename leaves the
    /// old inode alone and the witness still reads the old file.
    /// Deterministic, no kill required.
    #[test]
    fn test_convert_verbs_never_write_through_the_existing_file() {
        for (name, verb) in VERBS {
            let dir = TempDir::new("convert-atomic");
            let target = dir.join("map.mindmap.json");
            std::fs::copy(legacy_fixture_path(), &target).unwrap();
            let before = std::fs::read_to_string(&target).unwrap();

            let witness = dir.join("witness.mindmap.json");
            std::fs::hard_link(&target, &witness).unwrap();

            // In-place: the case where getting this wrong destroys
            // the user's only copy.
            verb(&target, &target).unwrap_or_else(|e| panic!("{name}: {e}"));

            let after = std::fs::read_to_string(&target).unwrap();
            assert_ne!(
                digest(&after),
                digest(&before),
                "{name}: fixture must actually change, or the test proves nothing"
            );
            let witness_content = std::fs::read_to_string(&witness).unwrap();
            assert_eq!(
                digest(&witness_content),
                digest(&before),
                "{name}: wrote through the existing file instead of renaming a staging \
                 file over it — an interrupted in-place run would truncate the original"
            );
        }
    }

    /// The staging file is a `write_atomic` implementation detail and
    /// must not outlive a successful conversion. Mirrors baumhard's
    /// `loader::tests::test_save_to_file_leaves_no_tmp_file_on_success`
    /// at the maptool seam, where a verb that quietly went back to
    /// `fs::write` plus a hand-rolled temp file would show up.
    #[test]
    fn test_convert_verbs_leave_no_staging_file() {
        for (name, verb) in VERBS {
            let dir = TempDir::new("convert-staging");
            let input = dir.join("in.mindmap.json");
            std::fs::copy(legacy_fixture_path(), &input).unwrap();
            let output = dir.join("out.mindmap.json");

            verb(&input, &output).unwrap_or_else(|e| panic!("{name}: {e}"));

            let leftovers: Vec<String> = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|entry| entry.ends_with(".tmp"))
                .collect();
            assert!(
                leftovers.is_empty(),
                "{name}: staging file(s) left behind: {leftovers:?}"
            );
        }
    }

    /// Every verb reports its input path in the read error rather
    /// than a bare OS message, so a mistyped path is diagnosable.
    #[test]
    fn test_convert_verbs_name_the_missing_input() {
        for (name, verb) in VERBS {
            let dir = TempDir::new("convert-missing-input");
            let missing = dir.join("nope.mindmap.json");
            let Err(err) = verb(&missing, &dir.join("out.mindmap.json")) else {
                panic!("{name}: a missing input must be an error");
            };
            assert!(
                err.contains("nope.mindmap.json"),
                "{name}: error must name the input path, got: {err}"
            );
        }
    }

    /// A transform that fails must leave the output path untouched —
    /// the summary is printed after the write, and the write never
    /// happens. `convert_legacy` is the only verb with a failing
    /// transform (a root without a usable `nodes` object).
    #[test]
    fn test_failed_legacy_transform_writes_nothing() {
        let dir = TempDir::new("convert-failed-transform");
        let input = dir.join("in.mindmap.json");
        std::fs::write(&input, r#"{"version":"1.0","name":"t"}"#).unwrap();
        let output = dir.join("out.mindmap.json");

        let err = convert_legacy(&input, &output).expect_err("missing nodes must fail");
        assert!(err.contains("nodes"), "got: {err}");
        assert!(
            !output.exists(),
            "a failed conversion must not leave an output file behind"
        );
    }
}
