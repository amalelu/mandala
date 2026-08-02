// SPDX-License-Identifier: MPL-2.0

//! Single-line editor tests: selection routing, `clean` buffer
//! seeding, the pure key router, and a differential oracle over the
//! whole lifecycle. No winit event loop and no wgpu device — the
//! router is a pure function and the lifecycle core is
//! renderer-free.

use super::*;
use crate::application::app::dispatch::apply_label_edit_action_to_buffer;
use crate::application::document::{EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState};
use crate::application::keybinds::Action;
use crate::application::platform::input::{Key, SmolStr};

fn ch(s: &str) -> Key {
    Key::Character(SmolStr::new(s))
}

// ── Selection → single-line editor routing ──────────────────
//
// One resolver behind `Action::EditSelection*`,
// `Action::LabelEditOnSelection` and the type-to-edit path;
// these pin the mapping the three used to each re-derive.

#[test]
fn test_resolve_single_line_target_routes_edge_label_to_the_label_editor() {
    let er = EdgeRef::new("a", "b", "cross_link");
    let sel = SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone()));
    assert_eq!(
        resolve_single_line_target(&sel),
        Some(SingleLineEditTarget::EdgeLabel { edge_ref: er })
    );
}

/// `PortalLabel` and `PortalText` are two selections on the same
/// endpoint (its glyph and its caption) and both edit the
/// caption, so both route to the portal-caption target.
#[test]
fn test_resolve_single_line_target_routes_both_portal_selections_to_the_portal_editor() {
    let er = EdgeRef::new("a", "b", "portal");
    let expected = Some(SingleLineEditTarget::PortalText {
        edge_ref: er.clone(),
        endpoint_node_id: "a".to_string(),
    });
    let label_sel = PortalLabelSel {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey::from(&er),
        endpoint_node_id: "a".to_string(),
    };
    assert_eq!(
        resolve_single_line_target(&SelectionState::PortalLabel(label_sel.clone())),
        expected
    );
    assert_eq!(
        resolve_single_line_target(&SelectionState::PortalText(label_sel)),
        expected
    );
}

/// Node-scoped and empty selections belong to the node text
/// editor (or to nothing) — the single-line editor declines.
/// All seven non-editor variants, so the list plus the three
/// editor cases above covers every `SelectionState` arm.
#[test]
fn test_resolve_single_line_target_declines_non_edge_selections() {
    for sel in [
        SelectionState::None,
        SelectionState::Single("a".into()),
        SelectionState::Multi(vec!["a".into(), "b".into()]),
        SelectionState::Section(SectionSel::new("a", 0)),
        SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 1)]),
        SelectionState::SectionRange {
            sel: SectionSel::new("a", 0),
            range: (0, 2),
        },
        SelectionState::Edge(EdgeRef::new("a", "b", "cross_link")),
    ] {
        assert_eq!(
            resolve_single_line_target(&sel),
            None,
            "{:?} must not open a single-line editor",
            sel
        );
    }
}

// ── Press-hit identity (the double-click re-open guard) ─────
//
// A double-click on the element already under edit must NOT
// re-open the editor: re-opening re-seeds the buffer from the
// committed model value and silently destroys the in-progress
// edit. The guard in `event_mouse_click` consumes press-time
// hits and asks the open target whether one of them is it, so
// this predicate is the whole guard.

fn ek(from: &str, to: &str) -> baumhard::mindmap::scene_cache::EdgeKey {
    baumhard::mindmap::scene_cache::EdgeKey::new(from, to, "cross_link")
}

#[test]
fn test_matches_press_hit_edge_label_reads_only_the_edge_label_hit() {
    let target = SingleLineEditTarget::EdgeLabel {
        edge_ref: EdgeRef::new("a", "b", "cross_link"),
    };
    assert!(target.matches_press_hit(Some(&ek("a", "b")), None));
    assert!(!target.matches_press_hit(Some(&ek("a", "c")), None));
    assert!(!target.matches_press_hit(None, None));
    // A portal-caption hit on the same edge is a different
    // element; the edge-label editor must not claim it.
    assert!(!target.matches_press_hit(None, Some(&(ek("a", "b"), "a".to_string()))));
}

#[test]
fn test_matches_press_hit_portal_text_requires_the_same_endpoint() {
    let target = SingleLineEditTarget::PortalText {
        edge_ref: EdgeRef::new("a", "b", "cross_link"),
        endpoint_node_id: "a".to_string(),
    };
    assert!(target.matches_press_hit(None, Some(&(ek("a", "b"), "a".to_string()))));
    // The *other* endpoint of the same portal edge is a
    // different element: clicking it commits this side and
    // routes as a fresh selection, so the guard must let the
    // double-click through.
    assert!(!target.matches_press_hit(None, Some(&(ek("a", "b"), "b".to_string()))));
    assert!(!target.matches_press_hit(None, Some(&(ek("a", "c"), "a".to_string()))));
    assert!(!target.matches_press_hit(None, None));
    // An edge-label hit belongs to the other role.
    assert!(!target.matches_press_hit(Some(&ek("a", "b")), None));
}

// ── `clean` buffer seeding ──────────────────────────────────
//
// `Action::EditSelectionClean`'s empty-buffer contract. Before
// the fix the dispatch arm computed the flag into `let _clean`
// and threw it away, so the contract held on nodes and silently
// didn't on edge labels / portal endpoints.

#[test]
fn test_seed_edit_buffer_clean_opens_empty_with_the_cursor_at_zero() {
    assert_eq!(seed_edit_buffer(true, Some("existing label")), (String::new(), 0));
}

#[test]
fn test_seed_edit_buffer_non_clean_seeds_existing_text_cursor_at_end() {
    assert_eq!(seed_edit_buffer(false, Some("café")), ("café".to_string(), 4));
}

/// The cursor is counted in grapheme clusters, so an existing
/// label ending in a ZWJ emoji sequence puts the caret after
/// the whole cluster rather than mid-sequence.
#[test]
fn test_seed_edit_buffer_counts_the_cursor_in_grapheme_clusters() {
    let family = "a\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(seed_edit_buffer(false, Some(family)), (family.to_string(), 2));
}

/// An unlabeled edge opens on an empty buffer either way — the
/// `clean` path differs only when there is text to suppress.
#[test]
fn test_seed_edit_buffer_handles_a_missing_original() {
    assert_eq!(seed_edit_buffer(false, None), (String::new(), 0));
    assert_eq!(seed_edit_buffer(true, None), (String::new(), 0));
}

// Structural-key behavior (Backspace / Delete / Arrow*/Home/End)
// moved from the key router to
// `dispatch::apply_label_edit_action_to_buffer` in Phase 5. Tests
// migrated alongside.

#[test]
fn test_label_edit_backspace_deletes_grapheme_before_cursor() {
    let mut buf = String::from("café");
    let mut cursor = 4;
    let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteBack, &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "caf");
    assert_eq!(cursor, 3);
}

#[test]
fn test_label_edit_backspace_at_zero_is_noop() {
    let mut buf = String::from("abc");
    let mut cursor = 0;
    let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteBack, &mut buf, &mut cursor);
    assert!(!changed);
    assert_eq!(buf, "abc");
    assert_eq!(cursor, 0);
}

#[test]
fn test_label_edit_delete_at_end_is_noop() {
    let mut buf = String::from("abc");
    let mut cursor = 3;
    let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteForward, &mut buf, &mut cursor);
    assert!(!changed);
    assert_eq!(buf, "abc");
    assert_eq!(cursor, 3);
}

#[test]
fn test_label_edit_delete_removes_grapheme_at_cursor() {
    let mut buf = String::from("abc");
    let mut cursor = 1;
    let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteForward, &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "ac");
    assert_eq!(cursor, 1);
}

#[test]
fn test_label_edit_arrow_left_right_walks_graphemes() {
    let mut buf = String::from("café");
    let mut cursor = 4;
    assert!(apply_label_edit_action_to_buffer(
        Action::LabelEditCursorLeft,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 3);
    assert!(apply_label_edit_action_to_buffer(
        Action::LabelEditCursorLeft,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 2);
    assert!(apply_label_edit_action_to_buffer(
        Action::LabelEditCursorRight,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 3);
}

#[test]
fn test_label_edit_arrow_left_at_zero_is_noop() {
    let mut buf = String::from("abc");
    let mut cursor = 0;
    assert!(!apply_label_edit_action_to_buffer(
        Action::LabelEditCursorLeft,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 0);
}

#[test]
fn test_label_edit_home_end_jump_to_ends() {
    let mut buf = String::from("café");
    let mut cursor = 2;
    assert!(apply_label_edit_action_to_buffer(
        Action::LabelEditCursorHome,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 0);
    assert!(!apply_label_edit_action_to_buffer(
        Action::LabelEditCursorHome,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 0);
    assert!(apply_label_edit_action_to_buffer(
        Action::LabelEditCursorEnd,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 4);
    assert!(!apply_label_edit_action_to_buffer(
        Action::LabelEditCursorEnd,
        &mut buf,
        &mut cursor
    ));
    assert_eq!(cursor, 4);
}

#[test]
fn test_route_single_line_printable_inserts_and_advances() {
    let mut buf = String::from("ab");
    let mut cursor = 1;
    let changed = route_single_line_key(&ch("X"), &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "aXb");
    assert_eq!(cursor, 2);
}

/// IME / dead-key sequences can arrive as multi-char strings.
/// The cursor advances by visible grapheme clusters, not by
/// codepoints.
#[test]
fn test_route_single_line_multichar_typed_payload() {
    let mut buf = String::new();
    let mut cursor = 0;
    let changed = route_single_line_key(&ch("né"), &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "né");
    assert_eq!(cursor, 2);
}

#[test]
fn test_route_single_line_ime_payload_advances_by_grapheme_delta() {
    let mut buf = String::new();
    let mut cursor = 0;
    let jamo = "\u{1112}\u{1161}\u{11AB}";
    let changed = route_single_line_key(&ch(jamo), &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, jamo);
    assert_eq!(cursor, 1);
}

#[test]
fn test_route_single_line_combining_mark_merge_does_not_overadvance_cursor() {
    let mut buf = String::from("e");
    let mut cursor = 1;
    let changed = route_single_line_key(&ch("\u{0301}"), &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "e\u{0301}");
    assert_eq!(cursor, 1);
}

/// Control characters in a typed payload are filtered out.
/// Pins the regression where an IME sequence like `"a\t"`
/// would otherwise insert a literal tab.
#[test]
fn test_route_single_line_typed_control_chars_are_skipped() {
    let mut buf = String::new();
    let mut cursor = 0;
    let changed = route_single_line_key(&ch("a\tb"), &mut buf, &mut cursor);
    assert!(changed);
    assert_eq!(buf, "ab");
    assert_eq!(cursor, 2);
}

/// After Phase 5 the structural keys (Backspace / Delete /
/// arrows / Home / End) flow through
/// `dispatch::apply_label_edit_action_to_buffer`, not the
/// router. The router now ignores all `Key::Named` events; only
/// `Key::Character` printable payloads are inserted. This pins
/// the new contract — a future regression that re-introduces
/// `Key::Named(Backspace) → delete` would leak the structural
/// behavior past the action layer and let users no longer
/// disable the key by clearing the binding.
#[test]
fn test_route_named_key_is_noop_post_phase_5() {
    use crate::application::platform::input::NamedKey;
    let mut buf = String::from("abc");
    let mut cursor = 3;
    let changed = route_single_line_key(&Key::Named(NamedKey::Backspace), &mut buf, &mut cursor);
    assert!(!changed);
    assert_eq!(buf, "abc");
    assert_eq!(cursor, 3);

    let changed = route_single_line_key(&Key::Named(NamedKey::Delete), &mut buf, &mut cursor);
    assert!(!changed);
    assert_eq!(buf, "abc");
    assert_eq!(cursor, 3);
}

/// Unprintable winit `Key` variants (dead keys, identifier-only,
/// modifiers reaching the dispatcher) must not insert anything
/// and must not panic.
#[test]
fn test_route_unhandled_key_is_noop() {
    use crate::application::platform::input::NamedKey;
    let mut buf = String::from("abc");
    let mut cursor = 1;
    let changed = route_single_line_key(&Key::Named(NamedKey::Shift), &mut buf, &mut cursor);
    assert!(!changed);
    assert_eq!(buf, "abc");
    assert_eq!(cursor, 1);
}

// ── Differential oracle over the editor lifecycle ───────────
//
// Every scenario below drives the renderer-free editor core over
// a scripted input sequence and renders one line per step
// describing everything the step could observably change: the
// refresh decree, the editor's buffer + grapheme cursor, the
// target's committed model value, the staged preview string, and
// the undo-stack depth.
//
// The expected traces are the oracle. They were authored against
// the two *separate* `LabelEditState` / `PortalTextEditState`
// implementations that preceded `SingleLineEditor`, and they are
// carried across the unification byte for byte — `git log -p`
// on this file shows the driver changing and not one expected
// line moving.
mod oracle {
    use super::super::editor::{EditInput, EditRefresh};
    use super::super::*;
    use crate::application::document::{EdgeRef, MindMapDocument};
    use crate::application::keybinds::Action;
    use crate::application::platform::input::{Key, SmolStr};

    const FROM_ID: &str = "node-a";
    const TO_ID: &str = "node-b";
    const EDGE_TYPE: &str = "cross_link";
    /// Pre-edit text on both targets, so an edge-label trace and
    /// a portal-caption trace over the same script are directly
    /// comparable.
    const SEED_TEXT: &str = "hi";

    /// Which single-line target a scenario drives.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TargetKind {
        EdgeLabel,
        PortalText,
    }

    /// One scripted step. `DeleteEdge` / `FlipToLine` are
    /// environment mutations that model something happening
    /// *underneath* an open editor (an undo popping the edge, a
    /// console command flipping `display_mode`).
    #[derive(Debug, Clone)]
    enum Step {
        Open { clean: bool },
        Type(&'static str),
        Act(Action),
        DeleteEdge,
        FlipToLine,
        Close { commit: bool },
    }

    fn target(kind: TargetKind) -> SingleLineEditTarget {
        let edge_ref = EdgeRef::new(FROM_ID, TO_ID, EDGE_TYPE);
        match kind {
            TargetKind::EdgeLabel => SingleLineEditTarget::EdgeLabel { edge_ref },
            TargetKind::PortalText => SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id: FROM_ID.to_string(),
            },
        }
    }

    fn key(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    /// Two nodes plus one edge carrying [`SEED_TEXT`] on the
    /// field the `kind`'s target owns.
    fn fixture_doc(kind: TargetKind) -> MindMapDocument {
        use crate::application::document::defaults::{default_cross_link_edge, default_orphan_node};
        use glam::Vec2;

        let mut edge = default_cross_link_edge(FROM_ID, TO_ID);
        match kind {
            TargetKind::EdgeLabel => edge.label = Some(SEED_TEXT.to_string()),
            TargetKind::PortalText => {
                edge.display_mode = Some(baumhard::mindmap::model::DISPLAY_MODE_PORTAL.to_string());
                edge.portal_from = Some(baumhard::mindmap::model::PortalEndpointState {
                    text: Some(SEED_TEXT.to_string()),
                    ..Default::default()
                });
            }
        }
        let json = serde_json::json!({
            "version": "1.0",
            "name": "oracle",
            "canvas": {"background_color": "#000000"},
            "nodes": {
                FROM_ID: default_orphan_node(FROM_ID, Vec2::new(0.0, 0.0)),
                TO_ID: default_orphan_node(TO_ID, Vec2::new(400.0, 0.0)),
            },
            "edges": [edge],
        })
        .to_string();
        MindMapDocument::from_json_str(&json, None).expect("oracle fixture JSON must parse")
    }

    /// The target's committed model value.
    fn model_value(kind: TargetKind, doc: &MindMapDocument) -> Option<String> {
        let edge = doc.mindmap.edges.first()?;
        match kind {
            TargetKind::EdgeLabel => edge.label.clone(),
            TargetKind::PortalText => edge.portal_from.as_ref().and_then(|s| s.text.clone()),
        }
    }

    /// The preview string staged for the renderer, if any.
    fn preview(kind: TargetKind, doc: &MindMapDocument) -> Option<String> {
        match kind {
            TargetKind::EdgeLabel => doc.label_edit_preview.as_ref().map(|(_, s)| s.clone()),
            TargetKind::PortalText => doc.portal_text_edit_preview.as_ref().map(|(_, _, s)| s.clone()),
        }
    }

    fn render(
        refresh: EditRefresh,
        editor: Option<(&str, usize)>,
        value: Option<String>,
        prev: Option<String>,
        undo: usize,
    ) -> String {
        let editor = match editor {
            Some((buf, cur)) => format!("open[{}]@{}", buf, cur),
            None => "closed".to_string(),
        };
        // Rendered by hand rather than through `{:?}` on the
        // `Option<String>`s: `Debug` escapes ZWJ / combining
        // marks, which would make a grapheme trace unreadable
        // in exactly the case it exists to pin.
        let opt = |v: Option<String>| match v {
            Some(s) => format!("\"{}\"", s),
            None => "-".to_string(),
        };
        format!(
            "{:?} {} value={} preview={} undo={}",
            refresh,
            editor,
            opt(value),
            opt(prev),
            undo
        )
    }

    /// Drive `script` against `kind`'s target, one trace line per
    /// step.
    fn run(kind: TargetKind, script: &[Step]) -> Vec<String> {
        let mut doc = fixture_doc(kind);
        let mut editor = SingleLineEditor::Closed;
        let mut trace = Vec::new();
        for step in script {
            let refresh = match step {
                Step::DeleteEdge => {
                    doc.mindmap.edges.clear();
                    EditRefresh::None
                }
                Step::FlipToLine => {
                    for e in doc.mindmap.edges.iter_mut() {
                        e.display_mode = None;
                    }
                    EditRefresh::None
                }
                Step::Open { clean } => editor.open_for_test(target(kind), *clean, &mut doc),
                Step::Type(s) => {
                    let k = key(s);
                    editor.handle_input_for_test(EditInput::Key(&k), &mut doc)
                }
                Step::Act(a) => editor.handle_input_for_test(EditInput::Action(a.clone()), &mut doc),
                Step::Close { commit } => editor.close_for_test(*commit, &mut doc),
            };
            trace.push(render(
                refresh,
                editor.buffer_and_cursor(),
                model_value(kind, &doc),
                preview(kind, &doc),
                doc.undo_stack.len(),
            ));
        }
        trace
    }

    /// Assert a script produces `expected` on **both** targets.
    /// Every scenario the two are supposed to agree on goes
    /// through here, so the pair stays pinned as one editor.
    fn assert_both(script: &[Step], expected: &[&str]) {
        for kind in [TargetKind::EdgeLabel, TargetKind::PortalText] {
            assert_eq!(
                run(kind, script),
                expected,
                "{:?} diverged from the shared single-line editor trace",
                kind
            );
        }
    }

    #[test]
    fn test_oracle_open_type_commit() {
        assert_both(
            &[
                Step::Open { clean: false },
                Step::Type("!"),
                Step::Close { commit: true },
            ],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"All closed value="hi!" preview=- undo=1"#,
            ],
        );
    }

    #[test]
    fn test_oracle_open_type_cancel_restores_and_pushes_no_undo() {
        assert_both(
            &[
                Step::Open { clean: false },
                Step::Type("!"),
                Step::Close { commit: false },
            ],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"All closed value="hi" preview=- undo=0"#,
            ],
        );
    }

    /// `Action::EditSelectionClean`'s empty-buffer contract all
    /// the way through commit.
    #[test]
    fn test_oracle_clean_open_replaces_the_value() {
        assert_both(
            &[
                Step::Open { clean: true },
                Step::Type("x"),
                Step::Close { commit: true },
            ],
            &[
                r#"Preview open[]@0 value="hi" preview="|" undo=0"#,
                r#"Preview open[x]@1 value="hi" preview="x|" undo=0"#,
                r#"All closed value="x" preview=- undo=1"#,
            ],
        );
    }

    /// Cursor / delete primitives arriving as resolved Actions
    /// move the same buffer the literal-key path does.
    #[test]
    fn test_oracle_cursor_primitives() {
        assert_both(
            &[
                Step::Open { clean: false },
                Step::Act(Action::LabelEditCursorLeft),
                Step::Act(Action::LabelEditDeleteBack),
                Step::Close { commit: true },
            ],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi]@1 value="hi" preview="h|i" undo=0"#,
                r#"Preview open[i]@0 value="hi" preview="|i" undo=0"#,
                r#"All closed value="i" preview=- undo=1"#,
            ],
        );
    }

    /// An unchanged commit must not leave a dead undo entry.
    #[test]
    fn test_oracle_unchanged_commit_pushes_no_undo() {
        assert_both(
            &[Step::Open { clean: false }, Step::Close { commit: true }],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"All closed value="hi" preview=- undo=0"#,
            ],
        );
    }

    /// Committing an empty buffer clears the model value (both
    /// setters normalize `Some("")` to `None`).
    #[test]
    fn test_oracle_empty_commit_clears_the_value() {
        assert_both(
            &[Step::Open { clean: true }, Step::Close { commit: true }],
            &[
                r#"Preview open[]@0 value="hi" preview="|" undo=0"#,
                r#"All closed value=- preview=- undo=1"#,
            ],
        );
    }

    /// A ZWJ family sequence is one grapheme: the cursor
    /// advances by 1, not by the codepoint count.
    #[test]
    fn test_oracle_zwj_cluster_advances_one_grapheme() {
        assert_both(
            &[
                Step::Open { clean: false },
                Step::Type("\u{1F469}\u{200D}\u{1F467}"),
                Step::Close { commit: true },
            ],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                "Preview open[hi\u{1F469}\u{200D}\u{1F467}]@3 value=\"hi\" \
                 preview=\"hi\u{1F469}\u{200D}\u{1F467}|\" undo=0",
                "All closed value=\"hi\u{1F469}\u{200D}\u{1F467}\" preview=- undo=1",
            ],
        );
    }

    /// The edge is deleted underneath an open editor and the
    /// user then commits. Both targets keep the buffer, both
    /// find nothing to write, and neither pushes an undo entry.
    #[test]
    fn test_oracle_edge_deleted_underneath_then_commit() {
        assert_both(
            &[
                Step::Open { clean: false },
                Step::Type("!"),
                Step::DeleteEdge,
                Step::Close { commit: true },
            ],
            &[
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"None open[hi!]@3 value=- preview="hi!|" undo=0"#,
                r#"All closed value=- preview=- undo=0"#,
            ],
        );
    }

    /// **The one intentional behavioral asymmetry.** A keystroke
    /// arriving after the edge vanished trips the portal
    /// caption's `still_editable` guard — the editor closes
    /// without committing, so the following commit is a no-op on
    /// a closed editor. The edge-label target has no such guard
    /// and keeps typing into a buffer whose target is gone; its
    /// commit then no-ops inside `set_edge_label`.
    ///
    /// Unifying the two editors had to preserve *both* columns.
    /// Making `still_editable` unconditional in either direction
    /// collapses them, which is exactly what this pins.
    #[test]
    fn test_oracle_keystroke_after_edge_deleted_diverges_by_design() {
        let script = [
            Step::Open { clean: false },
            Step::Type("!"),
            Step::DeleteEdge,
            Step::Type("?"),
            Step::Close { commit: true },
        ];
        let shared_prefix = [
            r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
            r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
            r#"None open[hi!]@3 value=- preview="hi!|" undo=0"#,
        ];

        let mut edge_label = shared_prefix.to_vec();
        edge_label.push(r#"Preview open[hi!?]@4 value=- preview="hi!?|" undo=0"#);
        edge_label.push(r#"All closed value=- preview=- undo=0"#);
        assert_eq!(run(TargetKind::EdgeLabel, &script), edge_label);

        let mut portal = shared_prefix.to_vec();
        portal.push(r#"All closed value=- preview=- undo=0"#);
        portal.push(r#"None closed value=- preview=- undo=0"#);
        assert_eq!(run(TargetKind::PortalText, &script), portal);
    }

    /// Second half of the portal `still_editable` guard: the edge
    /// survives but leaves portal mode, so the caption the editor
    /// is bound to stops existing as a rendered element. The
    /// guard closes the editor without committing, leaving the
    /// pre-edit value intact.
    #[test]
    fn test_oracle_portal_edge_flipped_to_line_mode_closes_without_commit() {
        assert_eq!(
            run(
                TargetKind::PortalText,
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::FlipToLine,
                    Step::Type("?"),
                    Step::Close { commit: true },
                ],
            ),
            vec![
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"None open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"All closed value="hi" preview=- undo=0"#,
                r#"None closed value="hi" preview=- undo=0"#,
            ]
        );
    }

    /// Opening on a target that already vanished leaves the
    /// editor closed and stages no preview.
    #[test]
    fn test_oracle_open_on_missing_target_is_a_noop() {
        assert_both(
            &[Step::DeleteEdge, Step::Open { clean: false }, Step::Type("x")],
            &[
                "None closed value=- preview=- undo=0",
                "None closed value=- preview=- undo=0",
                "None closed value=- preview=- undo=0",
            ],
        );
    }

    /// **A recorded behavior change, not a side effect.** A
    /// `LabelEdit*` cursor / delete action is `Step::Act`, and the
    /// funnel arm in `dispatch/native.rs` now routes it through the
    /// same `handle_input_core` a keystroke takes. That means it
    /// meets `still_editable` first, where on `main` the arm was a
    /// bare buffer mutation with no validity guard on any path.
    ///
    /// So an `Action` reaching an invalidated portal caption — from
    /// a macro, the console or IPC, the three callers that can send
    /// one without a keystroke — closes the editor and **discards
    /// the buffer uncommitted**, exactly as a keystroke does. On
    /// `main` it moved the caret in a buffer that survived until
    /// Enter or a click outside.
    ///
    /// It is the more consistent behavior — one entry point, one
    /// guard — and it is pinned here so it stays a decision.
    #[test]
    fn test_oracle_funnel_action_on_an_invalidated_portal_caption_discards_the_buffer() {
        assert_eq!(
            run(
                TargetKind::PortalText,
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::FlipToLine,
                    Step::Act(Action::LabelEditCursorLeft),
                    Step::Close { commit: true },
                ],
            ),
            vec![
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"None open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                // The caret move never happens: the guard fires
                // first, the editor closes, `hi!` is gone and the
                // `All` decree is a full rebuild.
                r#"All closed value="hi" preview=- undo=0"#,
                r#"None closed value="hi" preview=- undo=0"#,
            ]
        );
    }

    /// The other column of the same asymmetry, on the funnel path:
    /// the edge-label target has no `still_editable` guard, so the
    /// same action on a deleted edge moves the caret in a live
    /// buffer and the editor stays open.
    ///
    /// Together with the test above this is the funnel counterpart
    /// of `test_oracle_keystroke_after_edge_deleted_diverges_by_design`
    /// — the two entry points now agree per target, which is the
    /// whole point of the unification, and the targets still
    /// disagree with each other, which is deliberate.
    #[test]
    fn test_oracle_funnel_action_on_a_deleted_edge_label_keeps_the_buffer() {
        assert_eq!(
            run(
                TargetKind::EdgeLabel,
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::DeleteEdge,
                    Step::Act(Action::LabelEditCursorLeft),
                    Step::Close { commit: true },
                ],
            ),
            vec![
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"None open[hi!]@3 value=- preview="hi!|" undo=0"#,
                r#"Preview open[hi!]@2 value=- preview="hi|!" undo=0"#,
                r#"All closed value=- preview=- undo=0"#,
            ]
        );
    }
}
