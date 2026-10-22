// SPDX-License-Identifier: MPL-2.0

//! The bootstrap pair, `describe` and `ping`, the only unnamespaced
//! commands (`format/ipc.md` §Command registry and `describe`).
//!
//! `describe` renders the registry itself. It walks
//! [`registry::commands`], the table dispatch matches against, so a
//! command cannot be callable without being described, or described
//! without being callable.

use serde_json::{json, Value};

use crate::application::ipc::envelope::{Reply, PROTOCOL_VERSION};
use crate::application::ipc::registry::{
    self, Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec, BOOTSTRAP_FAMILY,
};

pub(super) const FAMILY: Family = Family {
    name: BOOTSTRAP_FAMILY,
    commands: &[
        CommandSpec {
            name: "describe",
            summary: "Describe the command surface this build registers and the events it can send.",
            params: &[ParamSpec::new(
                "command",
                ParamType::String,
                false,
                "Describe just this command instead of the whole table. Does not narrow `events`.",
            )],
            result: &[
                ResultSpec::new(
                    "protocol",
                    ParamType::Integer,
                    "The protocol integer, as in `hello`.",
                ),
                ResultSpec::new("app", ParamType::String, "Always \"mandala\"."),
                ResultSpec::new(
                    "version",
                    ParamType::String,
                    "The Cargo package version of this build.",
                ),
                ResultSpec::new(
                    "commands",
                    ParamType::Array,
                    "One entry per command, in name order: name, family, summary, params, result.",
                ),
                ResultSpec::new(
                    "events",
                    ParamType::Array,
                    "The complete event catalog: name and doc per class.",
                ),
            ],
            run: describe,
        },
        CommandSpec {
            name: "ping",
            summary: "Liveness probe; also the cheapest way to read the app clock.",
            params: &[],
            result: &[ResultSpec::new(
                "now_ms",
                ParamType::Number,
                "The app's now_ms() clock, in milliseconds.",
            )],
            run: ping,
        },
    ],
};

/// Every event class this build can send, with what it means.
const EVENTS: &[(&str, &str)] = &[
    (
        "connection_rejected",
        "Sent to a refused connection instead of `hello` while another controller is attached; then closed.",
    ),
    (
        "hello",
        "The greeting on accept: protocol, app, version, pid, and the loaded map's path (null before the first load).",
    ),
    (
        "shutdown",
        "Best-effort notice, before the socket closes, that the app is exiting.",
    ),
];

fn ping(call: &mut Call<'_>) -> Reply {
    call.ok(json!({ "now_ms": crate::application::common::now_ms() }))
}

fn describe(call: &mut Call<'_>) -> Reply {
    let mut commands: Vec<(&Family, &CommandSpec)> = match call.str_param("command") {
        None => registry::commands().collect(),
        Some(name) => match registry::lookup(name) {
            Some(found) => vec![found],
            None => {
                return call.invalid_params(
                    "command",
                    format!("no command '{name}' to describe; omit `command` for the full table"),
                )
            }
        },
    };
    commands.sort_by_key(|(_, command)| command.name);
    let commands: Vec<Value> = commands
        .into_iter()
        .map(|(family, command)| command.describe(family))
        .collect();
    let events: Vec<Value> = EVENTS
        .iter()
        .map(|(name, doc)| json!({ "name": name, "doc": doc }))
        .collect();
    call.ok(json!({
        "protocol": PROTOCOL_VERSION,
        "app": "mandala",
        "version": env!("CARGO_PKG_VERSION"),
        "commands": commands,
        "events": events,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::registry::tests::run_detached;

    #[test]
    fn test_ping_reports_the_app_clock_and_changes_nothing() {
        let before = crate::application::common::now_ms();
        let (reply, repaint) = run_detached(r#"{"id":1,"cmd":"ping"}"#);
        assert!(!repaint, "ping changes no pixels");
        assert_eq!(reply["id"], json!(1));
        assert_eq!(reply["ok"], json!(true));
        let now = reply["result"]["now_ms"].as_f64().expect("now_ms is a number");
        assert!(now >= before, "the clock is monotonic: {now} < {before}");
    }

    /// `describe` completeness, on the wire: the whole table, every
    /// registered command exactly once, in name order, each carrying
    /// its family and its full descriptor set.
    #[test]
    fn test_describe_renders_every_registered_command_once_in_name_order() {
        let (reply, repaint) = run_detached(r#"{"id":3,"cmd":"describe"}"#);
        assert!(!repaint);
        let result = &reply["result"];
        assert_eq!(result["protocol"], json!(PROTOCOL_VERSION));
        assert_eq!(result["app"], json!("mandala"));
        assert_eq!(result["version"], json!(env!("CARGO_PKG_VERSION")));
        let described = result["commands"].as_array().expect("commands array");
        let names: Vec<&str> = described
            .iter()
            .map(|c| c["name"].as_str().expect("name"))
            .collect();
        let mut registered: Vec<&str> = registry::commands().map(|(_, c)| c.name).collect();
        registered.sort_unstable();
        assert_eq!(names, registered, "describe is the registry, sorted");
        for entry in described {
            let (family, command) =
                registry::lookup(entry["name"].as_str().expect("name")).expect("registered");
            assert_eq!(entry, &command.describe(family));
            assert_eq!(
                entry["params"].as_array().expect("params").len(),
                command.params.len()
            );
            assert_eq!(
                entry["result"].as_array().expect("result").len(),
                command.result.len()
            );
        }
        assert_eq!(result["events"].as_array().expect("events").len(), EVENTS.len());
    }

    #[test]
    fn test_describe_publishes_a_descriptor_exactly() {
        let (reply, _) = run_detached(r#"{"id":4,"cmd":"describe","command":"describe"}"#);
        let commands = reply["result"]["commands"].as_array().expect("commands");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["family"], json!("bootstrap"));
        assert_eq!(
            commands[0]["params"],
            json!([{
                "name": "command", "type": "string", "required": false,
                "doc": "Describe just this command instead of the whole table. Does not narrow `events`."
            }])
        );
    }

    #[test]
    fn test_describe_command_narrows_commands_but_not_events() {
        let (reply, _) = run_detached(r#"{"id":4,"cmd":"describe","command":"act.macro"}"#);
        let commands = reply["result"]["commands"].as_array().expect("commands");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["name"], json!("act.macro"));
        assert_eq!(commands[0]["family"], json!("act"));
        assert_eq!(
            reply["result"]["events"].as_array().expect("events").len(),
            EVENTS.len()
        );
    }

    #[test]
    fn test_describe_rejects_an_unknown_or_mistyped_command_filter() {
        for frame in [
            r#"{"id":5,"cmd":"describe","command":"nope"}"#,
            r#"{"id":5,"cmd":"describe","command":7}"#,
        ] {
            let (reply, _) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!("command"), "{frame}");
        }
    }

    /// Every event class this build can send is one the document
    /// reserves.
    #[test]
    fn test_every_described_event_is_a_reserved_class() {
        use baumhard::util::doc_fixtures::{documented_paragraph, names_token, repo_path};

        let reserved = documented_paragraph(
            &repo_path("format/ipc.md"),
            "## `events` — event stream + revision (IPC-14)",
            "Reserved event classes",
        );
        for (event, _) in EVENTS {
            assert!(
                names_token(&reserved, event),
                "event `{event}` is not a reserved class"
            );
        }
    }
}
