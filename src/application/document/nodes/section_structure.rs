// SPDX-License-Identifier: MPL-2.0

//! Section structural mutators — `add_section`, `delete_section`,
//! `split_section`. These three change the *length* of a node's
//! `sections: Vec<MindSection>`, which shifts the indices of
//! subsequent sections; sibling section setters
//! (`set_section_offset`, `set_section_size`, `set_section_text`,
//! etc.) preserve indices and live in `mod.rs` / `section_text.rs`.
//!
//! All three push an `UndoAction::EditNodeStyle` undo entry — the
//! variant captures `before_sections: Vec<MindSection>` so undo
//! restores the entire structure (including text, runs, channels,
//! and trigger bindings on every section that was added / deleted /
//! split). Same envelope the index-preserving setters use
//! (`undo_envelope.rs`); no new undo variant required. All three
//! ask for [`NodeEditTail::GrowAndCleanup`], the only tail that
//! also repairs a selection or border preview left pointing at a
//! section index that no longer exists.
//!
//! Per `SECTIONS_BORDERS_RESIZE_PLAN.md` §4.5 (Batch 5).

use baumhard::mindmap::model::{validate, MindSection};

use super::super::MindMapDocument;
use super::NodeEditTail;

/// Error for a structural section mutation whose envelope came
/// back empty.
///
/// All three callers pre-validate the node id *and* the index, so
/// reaching this means the document moved underneath us rather
/// than that the user asked for something impossible. Naming only
/// one of the two possible causes — as all three messages
/// previously did, each saying "node not found" — sends a reader
/// hunting a missing node when a stale index is equally suspect.
/// This says both.
fn section_envelope_miss(op: &str, node_id: &str, idx: usize) -> String {
    format!("section {op}: node '{node_id}' or section[{idx}] no longer exists")
}

impl MindMapDocument {
    /// Insert a new section into `node_id.sections` at `at` (default
    /// end). Validates the new section's AABB against the parent
    /// node's size. Pushes one `EditNodeStyle` undo entry.
    ///
    /// Returns the index the section was inserted at, or an error
    /// when the node doesn't exist or the AABB is invalid.
    ///
    /// `at` is clamped to `[0, sections.len()]`; inserting past the
    /// end is the same as appending. Authoring-time conveniences
    /// (`at = None` to append, `at = Some(0)` to prepend) cover the
    /// two common patterns; `Some(i)` for `i > sections.len()` is
    /// silently clamped rather than rejected so a console caller
    /// passing an off-by-one doesn't fail mid-multi-step macro.
    pub fn add_section(
        &mut self,
        node_id: &str,
        at: Option<usize>,
        section: MindSection,
    ) -> Result<usize, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("section add: node '{}' not found", node_id)),
        };
        let len = node.sections.len();
        if len >= crate::application::document::MAX_SECTIONS_PER_NODE {
            return Err(format!(
                "section add: node '{}' already has {} sections (cap = {})",
                node_id,
                len,
                crate::application::document::MAX_SECTIONS_PER_NODE
            ));
        }
        let insert_at = at.unwrap_or(len).min(len);
        validate::section_candidate_aabb(node.size, insert_at, section.offset, section.size)?;

        self.mutate_node_with_style_undo(node_id, NodeEditTail::GrowAndCleanup, move |node| {
            node.sections.insert(insert_at, section);
            Some(insert_at)
        })
        .ok_or_else(|| section_envelope_miss("add", node_id, insert_at))
    }

    /// Remove the section at `idx` from `node_id.sections`. Returns
    /// the removed section so callers can stash it (e.g. for a
    /// console-side "deleted; undo to restore" message). Errors
    /// when the node doesn't exist, `idx` is out of range, or the
    /// node has only one section (the model invariant: every
    /// renderable node has at least one section).
    ///
    /// Pushes one `EditNodeStyle` undo entry — the
    /// `before_sections` snapshot fully restores the deleted
    /// section on undo (including text, runs, channels, trigger
    /// bindings, frame border).
    pub fn delete_section(&mut self, node_id: &str, idx: usize) -> Result<MindSection, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("section delete: node '{}' not found", node_id)),
        };
        if idx >= node.sections.len() {
            return Err(format!(
                "section delete: section[{}] not found on node '{}' (has {} section(s))",
                idx,
                node_id,
                node.sections.len()
            ));
        }
        if node.sections.len() == 1 {
            return Err(format!(
                "section delete: cannot delete the only section on node '{}' \
                 (every renderable node has at least one section); use \
                 `section text \"\"` to clear its content, or press Delete \
                 (the `delete_selection` keybind) to remove the whole node",
                node_id
            ));
        }

        // `idx` is bounds-checked above, so the `get` can only
        // fail if the node changed underneath us; expressing it
        // as a `?` rather than calling `Vec::remove` directly
        // keeps the closure panic-free on its own terms (§9).
        self.mutate_node_with_style_undo(node_id, NodeEditTail::GrowAndCleanup, |node| {
            node.sections.get(idx)?;
            Some(node.sections.remove(idx))
        })
        .ok_or_else(|| section_envelope_miss("delete", node_id, idx))
    }

    /// Split the section at `idx` into two adjacent sections.
    /// `at_grapheme` is the grapheme boundary in the section's
    /// `text` where the split lands; the prefix stays at `idx`,
    /// the suffix becomes a new section at `idx + 1`. `None`
    /// defaults to the end of the existing text — equivalent to
    /// "clone this section with empty text and insert it after".
    ///
    /// **TextRun handling**: `text_runs` are split grapheme-
    /// correctly at the boundary using
    /// [`baumhard::mindmap::model::text_run_ops::slice`], so
    /// per-grapheme styling survives on both halves. A run
    /// straddling the split is partitioned: the prefix's
    /// portion stays on the prefix; the suffix's portion lands
    /// on the new section with its `start`/`end` shifted into
    /// the new section's coordinate space. `slice` is the
    /// canonical entry point for range-aware run extraction —
    /// pre-fix this used a byte-vs-grapheme comparison
    /// (`r.end <= split_byte` against grapheme-indexed `end`)
    /// which silently corrupted runs on any non-ASCII text and
    /// dropped suffix runs entirely.
    ///
    /// `frame_border`, `channel`, and `trigger_bindings` are
    /// **cloned** onto the new section — splitting visually
    /// creates two slices of the same authoring intent, and per-
    /// section overrides (frame style, click bindings, mutation
    /// channel) typically apply to both halves. `offset` and
    /// `size` are also cloned — the split preserves the original
    /// AABB on both halves so the user sees a same-shape split.
    /// Authors who want asymmetric overrides edit the new
    /// section after.
    ///
    /// Pushes one `EditNodeStyle` undo entry. Returns the new
    /// section's index (`idx + 1`). Errors on missing node,
    /// out-of-range `idx`, or `at_grapheme` past the section's
    /// text length.
    pub fn split_section(
        &mut self,
        node_id: &str,
        idx: usize,
        at_grapheme: Option<usize>,
    ) -> Result<usize, String> {
        use baumhard::mindmap::model::text_run_ops;
        use baumhard::util::grapheme_chad::{count_grapheme_clusters, find_byte_index_of_grapheme};

        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("section split: node '{}' not found", node_id)),
        };
        let Some(section) = node.sections.get(idx) else {
            return Err(format!(
                "section split: section[{}] not found on node '{}' (has {} section(s))",
                idx,
                node_id,
                node.sections.len()
            ));
        };

        let original_text = &section.text;
        // Resolve `at_grapheme` against the section's text. We need
        // both the byte offset (for slicing the text) and the
        // grapheme index (for partitioning the runs — `TextRun.start`
        // / `.end` are grapheme-cluster indices per
        // `format/text-runs.md`). Walking once gives us both.
        let total_graphemes = count_grapheme_clusters(original_text);
        let split_grapheme = match at_grapheme {
            Some(g) if g > total_graphemes => {
                return Err(format!(
                    "section split: at={} > section's {} graphemes \
                     (pass at in [0, {}] or use `section show` to see the count)",
                    g, total_graphemes, total_graphemes
                ));
            }
            Some(g) => g,
            None => total_graphemes,
        };
        let split_byte = if split_grapheme == total_graphemes {
            original_text.len()
        } else {
            // The g-th grapheme boundary is the byte offset of
            // the start of the g-th grapheme cluster.
            find_byte_index_of_grapheme(original_text, split_grapheme).unwrap_or(original_text.len())
        };

        let prefix_text = original_text[..split_byte].to_string();
        let suffix_text = original_text[split_byte..].to_string();

        // Partition the runs grapheme-correctly via `slice`.
        // Prefix gets the runs in `[0, split_grapheme)` (clipped
        // to the slice bounds); suffix gets `[split_grapheme,
        // total_graphemes)` shifted into the new section's
        // coordinate space (`-split_grapheme`).
        let prefix_runs = text_run_ops::slice(&section.text_runs, 0, split_grapheme);
        let suffix_runs_at_original_coords =
            text_run_ops::slice(&section.text_runs, split_grapheme, total_graphemes);
        let suffix_runs: Vec<_> = suffix_runs_at_original_coords
            .into_iter()
            .map(|mut r| {
                r.start -= split_grapheme;
                r.end -= split_grapheme;
                r
            })
            .collect();

        // Build the new (suffix) section. Clone the per-section
        // metadata that semantically applies to both halves
        // (offset, size, channel, trigger_bindings, frame_border).
        let mut new_section = section.clone();
        new_section.text = suffix_text;
        new_section.text_runs = suffix_runs;

        let new_idx = idx + 1;
        self.mutate_node_with_style_undo(node_id, NodeEditTail::GrowAndCleanup, move |node| {
            let target = node.sections.get_mut(idx)?;
            target.text = prefix_text;
            target.text_runs = prefix_runs;
            node.sections.insert(new_idx, new_section);
            Some(new_idx)
        })
        .ok_or_else(|| section_envelope_miss("split", node_id, idx))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests_common::{first_testament_node_id, load_test_doc};
    use super::super::super::{SectionSel, SelectionState};
    use baumhard::mindmap::model::{MindSection, Position};

    fn empty_section() -> MindSection {
        MindSection {
            text: String::new(),
            text_runs: Vec::new(),
            offset: Position::default(),
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
            frame_border: None,
        }
    }

    #[test]
    fn add_section_appends_when_at_is_none() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let mut s = empty_section();
        s.text = "appended".into();
        let idx = doc.add_section(&id, None, s).expect("add ok");
        assert_eq!(idx, original_len);
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            original_len + 1
        );
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[idx].text, "appended");
    }

    #[test]
    fn add_section_inserts_at_index_when_at_is_some() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        let mut s = empty_section();
        s.text = "prepended".into();
        let idx = doc.add_section(&id, Some(0), s).expect("add ok");
        assert_eq!(idx, 0);
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[0].text, "prepended");
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1].text,
            original,
            "previous section[0] is now at idx 1"
        );
    }

    #[test]
    fn add_section_clamps_at_past_end() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let mut s = empty_section();
        s.text = "clamped".into();
        let idx = doc.add_section(&id, Some(9999), s).expect("add ok");
        assert_eq!(idx, original_len, "at past len clamps to len (append)");
    }

    #[test]
    fn add_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.undo_stack.clear();
        doc.dirty = false;
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let _ = doc.add_section(&id, None, empty_section()).expect("add ok");
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        // Undo restores the original section count.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections.len(), original_len);
    }

    #[test]
    fn add_section_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let err = doc.add_section("nonexistent", None, empty_section()).unwrap_err();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn add_section_rejects_out_of_bounds_aabb() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let mut s = empty_section();
        // Negative offset triggers the section-AABB validator.
        s.offset = Position { x: -10.0, y: 0.0 };
        let err = doc.add_section(&id, None, s).unwrap_err();
        assert!(err.contains("negative"), "got: {}", err);
    }

    #[test]
    fn delete_section_removes_at_index_returns_section() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Ensure at least 2 sections.
        doc.add_section(&id, None, empty_section()).unwrap();
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let removed_text = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        let removed = doc.delete_section(&id, 0).expect("delete ok");
        assert_eq!(removed.text, removed_text);
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections.len(), len_before - 1);
    }

    #[test]
    fn delete_section_rejects_last_remaining() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Force down to one section.
        while doc.mindmap.nodes.get(&id).unwrap().sections.len() > 1 {
            doc.delete_section(&id, 0).unwrap();
        }
        let err = doc.delete_section(&id, 0).unwrap_err();
        assert!(err.contains("only section"), "got: {}", err);
    }

    #[test]
    fn delete_section_rejects_out_of_range() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let err = doc.delete_section(&id, 9999).unwrap_err();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn delete_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Set up: 2 sections so delete is allowed.
        doc.add_section(&id, None, empty_section()).unwrap();
        let len_after_add = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.undo_stack.clear();
        doc.dirty = false;
        doc.delete_section(&id, 0).unwrap();
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_after_add,
            "undo restores the deleted section"
        );
    }

    #[test]
    fn split_section_splits_text_at_grapheme() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Set up: replace section[0]'s text with a known string.
        doc.set_section_text(&id, 0, "abcdef".to_string());
        let new_idx = doc.split_section(&id, 0, Some(3)).expect("split ok");
        assert_eq!(new_idx, 1);
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "abc");
        assert_eq!(sections[1].text, "def");
    }

    #[test]
    fn split_section_at_end_creates_empty_suffix() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abc".to_string());
        let new_idx = doc.split_section(&id, 0, None).expect("split ok");
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "abc");
        assert_eq!(sections[new_idx].text, "");
    }

    #[test]
    fn split_section_handles_unicode_graphemes() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Family emoji (multi-codepoint grapheme cluster) + ascii.
        doc.set_section_text(&id, 0, "👨‍👩‍👧 hi".to_string());
        // Split between the family emoji and the space (idx 1).
        let _new_idx = doc.split_section(&id, 0, Some(1)).expect("split ok");
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "👨\u{200d}👩\u{200d}👧");
        assert_eq!(sections[1].text, " hi");
    }

    #[test]
    fn split_section_rejects_grapheme_past_end() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abc".to_string());
        let err = doc.split_section(&id, 0, Some(99)).unwrap_err();
        assert!(err.contains("grapheme"), "got: {}", err);
    }

    #[test]
    fn split_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abcdef".to_string());
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.undo_stack.clear();
        doc.dirty = false;
        doc.split_section(&id, 0, Some(3)).unwrap();
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections.len(), len_before + 1);
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_before,
            "undo restores the pre-split section count"
        );
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[0].text,
            "abcdef",
            "undo restores the original text"
        );
    }

    /// Pinning the byte/grapheme bug fix: a section with a
    /// multi-byte text and a `TextRun` that lands wholly inside
    /// the prefix range must survive the split with its run
    /// intact (not silently dropped or mistruncated).
    /// Pre-fix the retain predicate compared grapheme-indexed
    /// `r.end` against a byte-offset `split_byte`, which silently
    /// dropped runs on any non-ASCII text.
    #[test]
    fn split_section_preserves_prefix_run_on_multibyte_text() {
        use baumhard::mindmap::model::TextRun;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // 5 multi-byte graphemes (Greek lowercase): each is 2 bytes.
        doc.set_section_text(&id, 0, "αβγδε".to_string());
        // Style the first two graphemes (αβ) as a single run.
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[0].text_runs = vec![TextRun {
                start: 0,
                end: 2,
                bold: true,
                italic: false,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#ff0000".into(),
                hyperlink: None,
            }];
        }
        // Split at grapheme 3 → prefix αβγ, suffix δε. The bold
        // run [0..2) sits wholly inside the prefix and must
        // survive.
        let _new = doc.split_section(&id, 0, Some(3)).unwrap();
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "αβγ");
        assert_eq!(sections[0].text_runs.len(), 1);
        let run = &sections[0].text_runs[0];
        assert_eq!((run.start, run.end), (0, 2), "prefix run must survive intact");
        assert!(run.bold, "prefix run's bold must survive");
        assert_eq!(run.color, "#ff0000");
    }

    /// Suffix runs survive the split with their indices shifted
    /// into the new section's coordinate space. Pre-fix all
    /// suffix runs were dropped.
    #[test]
    fn split_section_preserves_suffix_run_with_shifted_indices() {
        use baumhard::mindmap::model::TextRun;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abcdef".to_string());
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[0].text_runs = vec![TextRun {
                start: 4,
                end: 6,
                bold: false,
                italic: true,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#00ff00".into(),
                hyperlink: None,
            }];
        }
        // Split at grapheme 3 → prefix abc, suffix def. The italic
        // run [4..6) sits wholly inside the suffix; it should land
        // on the new section at [1..3) (shifted by -3).
        let new_idx = doc.split_section(&id, 0, Some(3)).unwrap();
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[new_idx].text, "def");
        assert_eq!(sections[new_idx].text_runs.len(), 1, "suffix run must survive");
        let run = &sections[new_idx].text_runs[0];
        assert_eq!(
            (run.start, run.end),
            (1, 3),
            "suffix run indices must shift into new-section coords"
        );
        assert!(run.italic);
        assert_eq!(run.color, "#00ff00");
    }

    /// Pin the `node.size` undo restoration. Pre-fix the floor
    /// pass after `add_section` could grow `node.size` to
    /// accommodate the inserted section's measured-text floor;
    /// `EditNodeStyle` only restored `style` + `sections`, leaving
    /// the node visibly inflated after undo. Now restored
    /// alongside.
    /// Hostile-mindmap defense: `add_section` rejects when the
    /// node is already at the section-count cap. Pre-fix the
    /// cap was unenforced and a Map-tier macro firing AddSection
    /// in a loop could OOM the host. Tests against a synthetic
    /// 1024-section node.
    #[test]
    fn add_section_rejects_at_cap() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Reach the cap quickly — the cap is 1024.
        if let Some(node) = doc.mindmap.nodes.get_mut(&id) {
            for _ in 0..(crate::application::document::MAX_SECTIONS_PER_NODE - node.sections.len()) {
                node.sections.push(empty_section());
            }
        }
        let result = doc.add_section(&id, None, empty_section());
        match result {
            Err(msg) => {
                assert!(msg.contains("cap = 1024"), "error should name the cap: {}", msg);
            }
            Ok(idx) => panic!("expected cap-rejection error, got Ok({})", idx),
        }
    }

    #[test]
    fn add_section_undo_restores_node_size_when_floor_pass_grew_it() {
        use baumhard::mindmap::model::{MindSection, Position};
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let before_size = doc.mindmap.nodes.get(&id).unwrap().size;
        let new_section = MindSection {
            text: "this is a long section that may grow the node".repeat(3),
            text_runs: Vec::new(),
            offset: Position::default(),
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
            frame_border: None,
        };
        doc.add_section(&id, None, new_section).unwrap();
        assert!(doc.undo());
        let after_undo_size = doc.mindmap.nodes.get(&id).unwrap().size;
        assert_eq!(
            (after_undo_size.width, after_undo_size.height),
            (before_size.width, before_size.height),
            "undo must restore node.size to pre-mutation value"
        );
    }

    /// `delete_section` rewrites `doc.selection` via
    /// `cleanup_after_structural_mutation` — `Section(idx=2)` →
    /// `Single(node)` after `delete_section(2)`. Pre-fix
    /// `UndoAction::EditNodeStyle` snapshotted only the model
    /// sections, leaving the user with `Single(node)` after
    /// undo even though the data was restored. Post-fix
    /// `before_selection` is captured + restored mirroring the
    /// AABB-restore work, so undo gets the user back to where
    /// they were.
    #[test]
    fn delete_section_undo_restores_selection() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Add a second section so we can delete one without
        // tripping the "last section" guard.
        doc.add_section(&id, None, empty_section()).unwrap();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let before_selection = doc.selection.clone();
        doc.delete_section(&id, 1).unwrap();
        assert!(matches!(doc.selection, SelectionState::Single(_)));
        assert!(doc.undo());
        // Undo restores both the deleted section AND the
        // user's pre-mutation selection. Pre-fix the model came
        // back but the user was stranded at `Single(node)`.
        assert_eq!(doc.selection, before_selection);
    }

    /// Parallel pin for `set_section_text`: a long-text replace
    /// triggers `grow_one_node_to_fit_text` which inflates
    /// `node.size`. Pre-fix, `UndoAction::EditNodeText` only
    /// restored `node.sections`, leaving the inflated AABB in
    /// place after undo. Post-fix, `before_position` and
    /// `before_size` are captured + restored mirroring
    /// `EditNodeStyle`.
    #[test]
    fn set_section_text_undo_restores_node_size_when_floor_pass_grew_it() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let before_size = doc.mindmap.nodes.get(&id).unwrap().size;
        let before_position = doc.mindmap.nodes.get(&id).unwrap().position;
        // Replace section[0]'s text with content long enough to
        // force the floor pass to grow the node.
        let long = "extremely long replacement text that will force the floor pass to grow the parent node beyond its original AABB by a noticeable amount".repeat(2);
        assert!(doc.set_section_text(&id, 0, long));
        let after_set_size = doc.mindmap.nodes.get(&id).unwrap().size;
        // Sanity: the floor pass actually grew the node.
        assert!(
            after_set_size.width > before_size.width || after_set_size.height > before_size.height,
            "fixture-validity check: long text must trigger floor-pass growth (before: {:?}, after: {:?})",
            before_size,
            after_set_size
        );
        assert!(doc.undo());
        let after_undo_size = doc.mindmap.nodes.get(&id).unwrap().size;
        let after_undo_position = doc.mindmap.nodes.get(&id).unwrap().position;
        assert_eq!(
            (after_undo_size.width, after_undo_size.height),
            (before_size.width, before_size.height),
            "undo must restore node.size to pre-mutation value"
        );
        assert_eq!(
            (after_undo_position.x, after_undo_position.y),
            (before_position.x, before_position.y),
            "undo must restore node.position to pre-mutation value"
        );
    }

    /// A run straddling the split — partitioned: the prefix gets
    /// the in-prefix portion clamped, the suffix gets the
    /// in-suffix portion shifted.
    #[test]
    fn split_section_partitions_straddling_run() {
        use baumhard::mindmap::model::TextRun;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abcdef".to_string());
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[0].text_runs = vec![TextRun {
                start: 1,
                end: 5,
                bold: true,
                italic: false,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#0000ff".into(),
                hyperlink: None,
            }];
        }
        // Split at grapheme 3 → prefix abc, suffix def. The bold
        // run [1..5) straddles: prefix side gets [1..3), suffix
        // side gets [3..5) shifted to [0..2).
        let new_idx = doc.split_section(&id, 0, Some(3)).unwrap();
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;

        let prefix_runs = &sections[0].text_runs;
        assert_eq!(prefix_runs.len(), 1, "prefix gets in-range portion");
        assert_eq!((prefix_runs[0].start, prefix_runs[0].end), (1, 3));
        assert!(prefix_runs[0].bold);

        let suffix_runs = &sections[new_idx].text_runs;
        assert_eq!(suffix_runs.len(), 1, "suffix gets shifted portion");
        assert_eq!((suffix_runs[0].start, suffix_runs[0].end), (0, 2));
        assert!(suffix_runs[0].bold);
        assert_eq!(suffix_runs[0].color, "#0000ff");
    }

    /// `delete_section` clamps a `Section` selection that
    /// pointed past the new section count back to a
    /// `Single(node)` selection. Pre-fix the selection stayed
    /// at the deleted idx, so subsequent verbs operated on the
    /// shifted-in section[K+1] thinking it was section[K]
    /// (silent misapplication).
    #[test]
    fn delete_section_clamps_section_selection_to_valid_idx() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Two sections so delete is allowed; selection on the last.
        doc.add_section(&id, None, empty_section()).unwrap();
        let last_idx = doc.mindmap.nodes.get(&id).unwrap().sections.len() - 1;
        doc.selection =
            crate::application::document::SelectionState::Section(crate::application::document::SectionSel {
                node_id: id.clone(),
                section_idx: last_idx,
            });
        doc.delete_section(&id, last_idx).unwrap();
        // Selection demotes to Single — the section is gone.
        assert!(
            matches!(&doc.selection, crate::application::document::SelectionState::Single(s) if s == &id),
            "selection must demote to Single after deleted section: {:?}",
            doc.selection
        );
    }

    /// `add_section` cancels an active border preview targeting
    /// this node's sections — the preview's idx reference is
    /// potentially stale after the structural change.
    #[test]
    fn add_section_cancels_active_section_border_preview() {
        use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection =
            crate::application::document::SelectionState::Section(crate::application::document::SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            });

        // Stage a section-targeted border preview.
        let mut edits = BorderConfigEdits::default();
        edits.preset = OptionEdit::Set("heavy".into());
        let _ = doc.set_border_preview(BorderPreviewTarget::Sections(vec![(id.clone(), 0)]), edits);
        assert!(doc.border_preview.is_some());

        // Add a section — preview must cancel because the idx
        // reference is potentially stale after the structural
        // change.
        doc.add_section(&id, Some(0), empty_section()).unwrap();
        assert!(
            doc.border_preview.is_none(),
            "structural mutation must cancel section-targeted preview"
        );
    }
}
