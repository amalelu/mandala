// SPDX-License-Identifier: MPL-2.0

//! End-to-end resize-mode lifecycle test (`SECTIONS_BORDERS_RESIZE_PLAN.md`
//! §7.2 scenario 4): drive the console verbs that flip
//! `InteractionMode` and assert that the scene-builder gate
//! responds — handles emit when Resize is active, vanish when
//! Default is active.
//!
//! The actual `Action::ExitMode` / `Action::EnterResizeMode`
//! dispatch arms aren't invoked here (they sit behind
//! `RebuildContext`, which needs a `Renderer` we can't construct
//! per `TEST_CONVENTIONS §T8`). Instead we mirror the
//! `apply_enter_resize_mode` resolver path via the console verb's
//! side-effect, manually flip `interaction_mode` (matching what
//! the dispatcher's `SetInteractionMode` consumer does), and
//! re-build the scene to assert handle visibility.
//!
//! This is the integration-shape version of the scene-rebuild
//! tests in `document/tests_resize.rs` — those pin the gate; this
//! pins the verb→side-effect→rebuild loop.

use crate::application::app::InteractionMode;
use crate::application::console::tests::fixtures::{first_node_id, load_test_doc};
use crate::application::console::{Args, ConsoleEffects, ConsoleSideEffect, ExecResult};
use crate::application::document::{InteractionModeOverrides, SelectionState};

/// Resolve and run a `mode` console line, returning the side
/// effect (the dispatcher consumes this in the production path).
fn run_mode_line(line: &str, doc: &mut crate::application::document::MindMapDocument) -> Option<ConsoleSideEffect> {
    let cmd = &crate::application::console::commands::mode::COMMAND;
    let parsed = crate::application::console::parser::parse(line);
    let args = match parsed {
        crate::application::console::parser::ParseResult::Ok { cmd: _, args } => args,
        _ => panic!("expected Ok parse for {:?}", line),
    };
    let mut eff = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut eff);
    assert!(matches!(result, ExecResult::Ok { .. }), "verb errored: {:?}", line);
    eff.side_effect.take()
}

#[test]
fn test_resize_mode_lifecycle_default_to_resize_to_default() {
    let mut doc = load_test_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());

    let mut mode = InteractionMode::Default;

    // 1. Default mode — no handles.
    let scene = crate::application::document::tests_common::project_roles(&doc, 1.0, mode.resize_handle_overrides());
    assert!(
        scene.node_resize_handles.is_empty(),
        "Default mode + Single selection must emit no handles before mode resize"
    );

    // 2. `mode resize` — verb produces a SetInteractionMode side
    //    effect carrying the resolved Resize { Node(id) }. The
    //    dispatcher's pre-rebuild handler writes this through to
    //    `ctx.interaction_mode`; mirror that.
    let side = run_mode_line("mode resize", &mut doc).expect("mode resize must produce a side effect");
    let new_mode = match side {
        ConsoleSideEffect::SetInteractionMode(m) => m,
        other => panic!("expected SetInteractionMode, got {:?}", other),
    };
    assert!(matches!(new_mode, InteractionMode::Resize { .. }));
    mode = new_mode;

    // 3. After the flip — 8 handles emit.
    let scene = crate::application::document::tests_common::project_roles(&doc, 1.0, mode.resize_handle_overrides());
    assert_eq!(
        scene.node_resize_handles.len(),
        8,
        "Resize {{ Node(id) }} mode must emit 8 handles for the targeted node"
    );

    // 4. `mode default` — same side-effect mechanism back to Default.
    let side = run_mode_line("mode default", &mut doc).expect("mode default must produce a side effect");
    let new_mode = match side {
        ConsoleSideEffect::SetInteractionMode(m) => m,
        other => panic!("expected SetInteractionMode, got {:?}", other),
    };
    assert_eq!(new_mode, InteractionMode::Default);
    mode = new_mode;

    // 5. Back to Default — handles disappear.
    let scene = crate::application::document::tests_common::project_roles(&doc, 1.0, mode.resize_handle_overrides());
    assert!(
        scene.node_resize_handles.is_empty(),
        "Returning to Default must clear node handles"
    );
    // And the override-resolution helper agrees.
    assert_eq!(mode.resize_handle_overrides().node, None);
    assert!(matches!(
        mode.resize_handle_overrides(),
        InteractionModeOverrides {
            node: None,
            section: None,
            node_edit_for: None,
            focused_section: None,
        }
    ));
}
