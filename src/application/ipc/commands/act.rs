// SPDX-License-Identifier: MPL-2.0

//! The `act` family: the dispatch bridge (`format/ipc.md` §`act`).
//!
//! CODE_CONVENTIONS §2's "reach for the existing seam" made
//! wire-visible. Each command is one existing funnel reached through
//! [`CommandTarget`](crate::application::ipc::CommandTarget):
//! `act.action` is `dispatch_action`, `act.console` is the console's
//! parse → execute → side-effect drain, and `act.macro` is the macro
//! dispatcher's gated step loop. There is no parallel control path,
//! so an IPC-driven change takes the same arms, leaves the same undo
//! entries and passes the same privilege gates as the keystroke,
//! console line or bound macro it stands in for.
//!
//! Every handler validates the whole request before it touches the
//! app. A malformed request is refused without side effects, and the
//! refusal does not depend on whether the app has finished starting.

use serde_json::{json, Value};

use crate::application::console::{ExecResult, LineOutcome};
use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::space::{Point, Space};
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;

/// An `Action`'s serde form: a string for a unit variant, a
/// `{"Variant": payload}` object for a payload variant.
const ACTION_SHAPE: ParamType = ParamType::OneOf(&[ParamType::String, ParamType::Object]);

pub(super) const FAMILY: Family = Family {
    name: "act",
    commands: &[
        CommandSpec {
            name: "act.action",
            summary: "Dispatch one Action through dispatch_action, exactly as a key binding fires it.",
            params: &[
                ParamSpec::new(
                    "action",
                    ACTION_SHAPE,
                    true,
                    "The Action, in the externally tagged form macro Action steps use: a string for a \
                     unit variant (\"Undo\"), a {\"Variant\": payload} object for a payload variant.",
                ),
                ParamSpec::new(
                    "x",
                    ParamType::Number,
                    false,
                    "Pointer x, in `space`. With `y`, the cursor stands there for this dispatch only and \
                     the dispatch carries what the click hit test finds under it, as a mouse press does.",
                ),
                ParamSpec::new(
                    "y",
                    ParamType::Number,
                    false,
                    "Pointer y, in `space`. Sent with `x`.",
                ),
                ParamSpec::new(
                    "space",
                    ParamType::String,
                    false,
                    "Coordinate space of `x`/`y`: \"canvas\" (default) or \"surface\".",
                ),
            ],
            result: &[ResultSpec::new(
                "outcome",
                ParamType::String,
                "\"handled\" or \"unhandled\": the funnel's DispatchOutcome. Unhandled means the \
                 Action did not apply in the current context.",
            )],
            run: action,
        },
        CommandSpec {
            name: "act.console",
            summary: "Run one console line through the console's own parse, execute and effect drain, \
                      without opening the console.",
            params: &[ParamSpec::new(
                "line",
                ParamType::String,
                true,
                "The line, as typed at the console prompt.",
            )],
            result: &[ResultSpec::new(
                "output",
                ParamType::Array,
                "The verb's output lines, as strings; empty for a blank line or a silent verb.",
            )],
            run: console,
        },
        CommandSpec {
            name: "act.macro",
            summary: "Run a registered macro under its own tier, or an anonymous step list at the User tier.",
            params: &[
                ParamSpec::new(
                    "macro_id",
                    ParamType::String,
                    false,
                    "Run the registered macro with this id. Exactly one of macro_id / steps.",
                ),
                ParamSpec::new(
                    "steps",
                    ParamType::Array,
                    false,
                    "Run these steps, in the on-disk step form ({\"kind\": \"Action\" | \
                     \"CustomMutation\" | \"ConsoleLine\", …}). Exactly one of macro_id / steps.",
                ),
            ],
            result: &[ResultSpec::new(
                "ran",
                ParamType::Boolean,
                "Whether at least one step executed. A privilege-gate abort is a \
                 macro_gate_rejected warning.",
            )],
            run: run_macro,
        },
    ],
};

fn action(call: &mut Call<'_>) -> Reply {
    let action = match parse_action(call) {
        Ok(action) => action,
        Err(reply) => return reply,
    };
    let pointer = match parse_pointer(call) {
        Ok(pointer) => pointer,
        Err(reply) => return reply,
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    let handled = app.dispatch_action(action, pointer);
    call.ok(json!({ "outcome": if handled { "handled" } else { "unhandled" } }))
}

/// The `action` parameter as an [`Action`]. Deserialization is serde's,
/// the same as for a macro step's `action`, so an unknown variant or a
/// mistyped payload is refused naming what serde expected.
fn parse_action(call: &Call<'_>) -> Result<Action, Reply> {
    let raw = call
        .param("action")
        .ok_or_else(|| call.invalid_params("action", "missing `action`"))?;
    serde_json::from_value(raw.clone()).map_err(|e| {
        call.invalid_params(
            "action",
            format!(
                "not an Action: {e}. Unit variants are strings (\"ZoomIn\"); payload variants are \
                 {{\"Variant\": payload}} objects"
            ),
        )
    })
}

/// The optional synthetic pointer: `x` and `y` together, in `space`.
fn parse_pointer(call: &Call<'_>) -> Result<Option<Point>, Reply> {
    let coordinate = |name: &str| call.param(name).and_then(Value::as_f64);
    let space = call.str_param("space");
    let (x, y) = match (coordinate("x"), coordinate("y")) {
        (Some(x), Some(y)) => (x, y),
        (None, None) if space.is_some() => {
            return Err(call.invalid_params(
                "space",
                "`space` qualifies a pointer position; send it with `x` and `y`",
            ))
        }
        (None, None) => return Ok(None),
        (Some(_), None) => return Err(call.invalid_params("y", "`x` and `y` are sent together")),
        (None, Some(_)) => return Err(call.invalid_params("x", "`x` and `y` are sent together")),
    };
    let space = match space {
        None => Space::Canvas,
        Some(word) => Space::parse(word)
            .ok_or_else(|| call.invalid_params("space", format!("`space` must be {}", Space::WORDS)))?,
    };
    // Positions are f32 everywhere downstream; a coordinate past
    // f32's range would arrive as infinity and poison the camera math.
    let (x, y) = (x as f32, y as f32);
    for (name, value) in [("x", x), ("y", y)] {
        if !value.is_finite() {
            return Err(call.invalid_params(name, format!("`{name}` is out of range for a position")));
        }
    }
    Ok(Some(Point { x, y, space }))
}

fn console(call: &mut Call<'_>) -> Reply {
    let Some(line) = call.str_param("line") else {
        return call.invalid_params("line", "missing `line`");
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    let Some(outcome) = app.run_console_line(line) else {
        return call.no_document();
    };
    match console_output(outcome) {
        Ok(output) => call.ok(json!({ "output": output })),
        Err(message) => call.error(code::CONSOLE_ERROR, message, None),
    }
}

/// A console line's outcome as `act.console` reports it: the output
/// lines, or the console's own failure text. A blank line succeeds
/// with no output, as it does at the prompt.
fn console_output(outcome: LineOutcome) -> Result<Vec<String>, String> {
    if let Some(failure) = outcome.failure() {
        return Err(failure);
    }
    Ok(match outcome {
        LineOutcome::Ran(ExecResult::Ok(text)) => text.lines().map(str::to_owned).collect(),
        LineOutcome::Ran(ExecResult::Lines(lines)) => lines.into_iter().map(|line| line.text).collect(),
        LineOutcome::Blank | LineOutcome::Unknown(_) | LineOutcome::Ran(ExecResult::Err(_)) => Vec::new(),
    })
}

fn run_macro(call: &mut Call<'_>) -> Reply {
    enum Run {
        Registered(String),
        Steps(Vec<MacroStep>),
    }
    let run = match (call.str_param("macro_id"), call.param("steps")) {
        (Some(_), Some(_)) => {
            return call.invalid_params("steps", "send exactly one of `macro_id` / `steps`, not both")
        }
        (None, None) => return call.invalid_params("macro_id", "send one of `macro_id` / `steps`"),
        (Some(id), None) => Run::Registered(id.to_owned()),
        (None, Some(steps)) => match serde_json::from_value::<Vec<MacroStep>>(steps.clone()) {
            Ok(steps) => Run::Steps(steps),
            Err(e) => {
                return call.invalid_params(
                    "steps",
                    format!(
                        "not a step list: {e}. Each step is {{\"kind\": \"Action\", \"action\": …}}, \
                         {{\"kind\": \"CustomMutation\", \"id\": …}} or {{\"kind\": \"ConsoleLine\", \"line\": …}} \
                         (format/macros.md)"
                    ),
                )
            }
        },
    };
    let ran = match run {
        Run::Registered(id) => {
            match call.observe() {
                None => return call.no_document(),
                Some(app) if !app.has_macro(&id) => {
                    return call.error(
                        code::NOT_FOUND,
                        format!("no macro '{id}' is registered in any tier"),
                        None,
                    )
                }
                Some(_) => {}
            }
            let Some(app) = call.drive() else {
                return call.no_document();
            };
            app.run_macro(&id)
        }
        Run::Steps(steps) => {
            let Some(app) = call.drive() else {
                return call.no_document();
            };
            app.run_macro_steps(&steps)
        }
    };
    call.ok(json!({ "ran": ran }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::{parse, Args, ParseResult};
    use crate::application::console::ConsoleEffects;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::ipc::registry::tests::run_detached;

    /// Every refusal below happens before the app is touched, so it
    /// is the same reply whether or not the app has started; detached
    /// is the strictest place to check that.
    fn refused(frame: &str) -> (String, String) {
        let (reply, repaint) = run_detached(frame);
        assert!(!repaint, "{frame}: a refused request drives nothing");
        assert_eq!(reply["ok"], json!(false), "{frame}");
        (
            reply["error"]["code"].as_str().expect("code").to_owned(),
            reply["error"]["data"]["param"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    }

    /// A malformed `action` payload is a structured `invalid_params`,
    /// never a panic (issue #63's acceptance criterion).
    #[test]
    fn test_a_malformed_action_is_invalid_params_naming_action() {
        for frame in [
            r#"{"id":1,"cmd":"act.action","action":"NoSuchAction"}"#,
            r#"{"id":1,"cmd":"act.action","action":"zoom_in"}"#,
            r#"{"id":1,"cmd":"act.action","action":{"OpenDocument":7}}"#,
            r#"{"id":1,"cmd":"act.action","action":{"ZoomIn":null,"Undo":null}}"#,
            r#"{"id":1,"cmd":"act.action","action":{}}"#,
        ] {
            assert_eq!(
                refused(frame),
                ("invalid_params".into(), "action".into()),
                "{frame}"
            );
        }
    }

    /// Both serde shapes an `Action` has are accepted, unit and payload:
    /// past parsing, a detached run reaches the app check.
    #[test]
    fn test_both_action_shapes_parse() {
        for frame in [
            r#"{"id":1,"cmd":"act.action","action":"ZoomIn"}"#,
            r#"{"id":1,"cmd":"act.action","action":{"OpenDocument":"/tmp/m.mindmap.json"}}"#,
            r#"{"id":1,"cmd":"act.action","action":"DoubleClickActivate","x":1,"y":2.5,"space":"surface"}"#,
        ] {
            let (reply, _) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
        }
    }

    #[test]
    fn test_a_half_or_mis_spaced_pointer_is_refused_naming_the_offender() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"act.action","action":"ZoomIn","x":1}"#, "y"),
            (r#"{"id":1,"cmd":"act.action","action":"ZoomIn","y":1}"#, "x"),
            (
                r#"{"id":1,"cmd":"act.action","action":"ZoomIn","space":"canvas"}"#,
                "space",
            ),
            (
                r#"{"id":1,"cmd":"act.action","action":"ZoomIn","x":1,"y":1,"space":"world"}"#,
                "space",
            ),
            (
                r#"{"id":1,"cmd":"act.action","action":"ZoomIn","x":1e300,"y":1}"#,
                "x",
            ),
            (
                r#"{"id":1,"cmd":"act.action","action":"ZoomIn","x":"1","y":1}"#,
                "x",
            ),
        ] {
            assert_eq!(refused(frame), ("invalid_params".into(), param.into()), "{frame}");
        }
    }

    #[test]
    fn test_act_macro_takes_exactly_one_of_macro_id_and_steps() {
        assert_eq!(
            refused(r#"{"id":1,"cmd":"act.macro"}"#),
            ("invalid_params".into(), "macro_id".into())
        );
        assert_eq!(
            refused(r#"{"id":1,"cmd":"act.macro","macro_id":"m","steps":[]}"#),
            ("invalid_params".into(), "steps".into())
        );
        assert_eq!(
            refused(r#"{"id":1,"cmd":"act.macro","id_":"m"}"#),
            ("invalid_params".into(), "id_".into()),
            "a stray parameter is named"
        );
    }

    #[test]
    fn test_a_malformed_step_list_is_invalid_params_naming_steps() {
        for frame in [
            r#"{"id":1,"cmd":"act.macro","steps":[{"kind":"Teleport"}]}"#,
            r#"{"id":1,"cmd":"act.macro","steps":[{"kind":"Action","action":"NoSuchAction"}]}"#,
            r#"{"id":1,"cmd":"act.macro","steps":[{"kind":"ConsoleLine"}]}"#,
            r#"{"id":1,"cmd":"act.macro","steps":["Undo"]}"#,
        ] {
            assert_eq!(
                refused(frame),
                ("invalid_params".into(), "steps".into()),
                "{frame}"
            );
        }
        let (reply, _) = run_detached(
            r#"{"id":1,"cmd":"act.macro","steps":[{"kind":"Action","action":"Undo"},{"kind":"ConsoleLine","line":"help"},{"kind":"CustomMutation","id":"m","target":{"node_id":"1.2"}}]}"#,
        );
        assert_eq!(
            reply["error"]["code"],
            json!("no_document"),
            "a well-formed list of every step kind parses"
        );
    }

    /// Run `line` through the console's parse and execute against the
    /// testament fixture, the half of `execute_console_line` that
    /// needs no renderer, and hand back the outcome it would return.
    fn console_outcome(line: &str) -> LineOutcome {
        let mut doc = load_test_doc();
        match parse(line) {
            ParseResult::Empty => LineOutcome::Blank,
            ParseResult::Unknown(head) => LineOutcome::Unknown(head),
            ParseResult::Ok { cmd, args } => {
                let mut effects = ConsoleEffects::new(&mut doc);
                LineOutcome::Ran((cmd.execute)(&Args::new(&args), &mut effects))
            }
        }
    }

    /// `act.console`'s result mapping, against real verbs on the
    /// fixture document: output lines for a verb that prints, the
    /// console's own message for an unknown verb and for a verb that
    /// fails, and an empty success for a blank line.
    #[test]
    fn test_console_output_maps_each_outcome_the_way_the_console_shows_it() {
        let help = console_output(console_outcome("help")).expect("help succeeds");
        assert!(help.len() > 1, "help prints a table: {help:?}");
        assert!(
            help.iter().all(|line| !line.contains('\n')),
            "one element per line"
        );

        assert_eq!(
            console_output(console_outcome("frobnicate now")),
            Err("unknown command: frobnicate".to_string())
        );

        let failed = console_output(console_outcome("border show"));
        assert!(
            failed.as_ref().is_err_and(|m| !m.is_empty()),
            "`border show` with nothing selected fails with the verb's message: {failed:?}"
        );

        assert_eq!(console_output(LineOutcome::Blank), Ok(Vec::new()));
        assert!(console_outcome("help").applied());
        for not_applied in ["frobnicate now", "border show", "   "] {
            assert!(
                !console_outcome(not_applied).applied(),
                "`{not_applied}` did not apply, so a file Action or macro step running it is not Handled"
            );
        }
        assert_eq!(
            console_output(LineOutcome::Ran(ExecResult::Ok("a\nb".into()))),
            Ok(vec!["a".to_string(), "b".to_string()]),
            "a multi-line Ok splits into lines"
        );
        assert_eq!(
            console_output(LineOutcome::Ran(ExecResult::Ok(String::new()))),
            Ok(Vec::new()),
            "a silent success has no output lines"
        );
    }
}
