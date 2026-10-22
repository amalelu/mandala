// SPDX-License-Identifier: MPL-2.0

//! The command registry: one table that is both what `describe`
//! publishes and what a request is dispatched against
//! (`format/ipc.md` §Command registry and `describe`).
//!
//! **Self-describing by construction.** A [`CommandSpec`] cannot be
//! written without a summary and a descriptor for every parameter and
//! result field, and the handler only ever sees parameters the
//! descriptors admitted. The discovery surface and the dispatch
//! surface are therefore one list. Parameter descriptors are built by
//! a `const fn` that refuses the envelope's reserved words, `id` and
//! `cmd`. Every descriptor lives in a `const`, so that refusal is a
//! compile error and never a wire ambiguity.
//!
//! **One table, extended one row at a time.** Each family is a
//! module under `commands/` exporting one [`Family`], and
//! `commands::FAMILIES` lists them. A new family adds its module and
//! one line there. Its PR touches no shared code, which is what lets
//! the family issues land in parallel (`work_plans/LLM_IPC.md` §D5).
//!
//! **The execution window.** [`execute`] validates a request against
//! its spec, then runs the handler inside a
//! [`baumhard::util::log::capture::capture`] window. Every `warn!` /
//! `error!` the command's execution emits on the way through
//! dispatch arms, document setters and Baumhard primitives therefore
//! reaches the reply's `warnings` array as well as the log, in the
//! same words (`format/ipc.md` §Warnings).

use serde_json::{json, Value};

use baumhard::util::log::capture::{capture, Captured, CAPTURE_LIMIT};

use super::commands::FAMILIES;
use super::envelope::{code, warning, IpcRequest, Reply, Warning};
use super::target::CommandTarget;
use super::Executed;

/// The JSON shape a parameter or result field takes. `describe`
/// renders it as one of the protocol's type words. A [`OneOf`] union
/// renders as its words joined with `|`.
///
/// [`OneOf`]: ParamType::OneOf
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamType {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
    /// Any one of the listed shapes. Used where the wire form is an
    /// existing serde representation with more than one shape, like
    /// an externally tagged `Action`: a string for a unit variant, an
    /// object for a payload variant.
    OneOf(&'static [ParamType]),
}

impl ParamType {
    /// The type word, as `describe` publishes it.
    pub(crate) fn word(self) -> String {
        match self {
            ParamType::String => "string".into(),
            ParamType::Integer => "integer".into(),
            ParamType::Number => "number".into(),
            ParamType::Boolean => "boolean".into(),
            ParamType::Object => "object".into(),
            ParamType::Array => "array".into(),
            ParamType::OneOf(shapes) => shapes.iter().map(|s| s.word()).collect::<Vec<_>>().join("|"),
        }
    }

    /// Whether `value` has this shape. `null` has none of them, so a
    /// parameter sent as `null` is a mistyped one, not an omitted one.
    pub(crate) fn admits(self, value: &Value) -> bool {
        match self {
            ParamType::String => value.is_string(),
            ParamType::Integer => value.is_i64() || value.is_u64(),
            ParamType::Number => value.is_number(),
            ParamType::Boolean => value.is_boolean(),
            ParamType::Object => value.is_object(),
            ParamType::Array => value.is_array(),
            ParamType::OneOf(shapes) => shapes.iter().any(|s| s.admits(value)),
        }
    }
}

/// One parameter, as `describe` publishes it and as [`execute`]
/// checks it. Build with [`ParamSpec::new`].
#[derive(Debug)]
pub(crate) struct ParamSpec {
    pub(crate) name: &'static str,
    pub(crate) ty: ParamType,
    pub(crate) required: bool,
    pub(crate) doc: &'static str,
}

/// The envelope keys a parameter may not be named
/// (`format/ipc.md` §Request, the reserved-word rule).
pub(crate) const RESERVED_PARAM_NAMES: [&str; 2] = ["id", "cmd"];

impl ParamSpec {
    /// Describe a parameter.
    ///
    /// **Panics on a reserved name.** Every call site is a `const`
    /// item, so the panic happens during compile-time evaluation and
    /// a command declaring an `id` or `cmd` parameter does not build.
    pub(crate) const fn new(name: &'static str, ty: ParamType, required: bool, doc: &'static str) -> Self {
        let mut i = 0;
        while i < RESERVED_PARAM_NAMES.len() {
            assert!(
                !const_str_eq(name, RESERVED_PARAM_NAMES[i]),
                "an IPC parameter may not be named `id` or `cmd`: params share the top level with \
                 the envelope (format/ipc.md, the reserved-word rule); pick a distinct name"
            );
            i += 1;
        }
        ParamSpec {
            name,
            ty,
            required,
            doc,
        }
    }
}

/// `str` equality usable in a `const fn`.
const fn const_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// One field of a command's `result` object, as `describe`
/// publishes it.
#[derive(Debug)]
pub(crate) struct ResultSpec {
    pub(crate) name: &'static str,
    pub(crate) ty: ParamType,
    pub(crate) doc: &'static str,
}

impl ResultSpec {
    pub(crate) const fn new(name: &'static str, ty: ParamType, doc: &'static str) -> Self {
        ResultSpec { name, ty, doc }
    }
}

/// A command handler. It runs only after [`execute`] has checked the
/// request against the command's [`ParamSpec`]s: no unknown
/// parameter, every required one present, every present one of its
/// declared type. Cross-parameter rules ("exactly one of") and value
/// rules (an enum word, a range) are the handler's to check.
///
/// Never panics. Every failure is an error [`Reply`] (§9).
pub(crate) type Handler = fn(&mut Call<'_>) -> Reply;

/// One registered command.
#[derive(Debug)]
pub(crate) struct CommandSpec {
    /// `family.verb`, or one of the two unnamespaced bootstrap names.
    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) params: &'static [ParamSpec],
    pub(crate) result: &'static [ResultSpec],
    pub(crate) run: Handler,
}

/// The name `describe` reports for the two unnamespaced commands.
pub(crate) const BOOTSTRAP_FAMILY: &str = "bootstrap";

/// A command family: one module under `commands/`, one row of the
/// family map in `format/ipc.md`.
#[derive(Debug)]
pub(crate) struct Family {
    /// The namespace every command in it carries before the dot, or
    /// [`BOOTSTRAP_FAMILY`] for the unnamespaced pair.
    pub(crate) name: &'static str,
    pub(crate) commands: &'static [CommandSpec],
}

/// Every registered command with the family it belongs to, in
/// registry order.
pub(crate) fn commands() -> impl Iterator<Item = (&'static Family, &'static CommandSpec)> {
    FAMILIES
        .iter()
        .flat_map(|family| family.commands.iter().map(move |command| (family, command)))
}

/// The registered command named `name`.
pub(crate) fn lookup(name: &str) -> Option<(&'static Family, &'static CommandSpec)> {
    commands().find(|(_, command)| command.name == name)
}

impl CommandSpec {
    /// The command's `describe` entry.
    pub(crate) fn describe(&self, family: &Family) -> Value {
        let params: Vec<Value> = self
            .params
            .iter()
            .map(|p| json!({ "name": p.name, "type": p.ty.word(), "required": p.required, "doc": p.doc }))
            .collect();
        let result: Vec<Value> = self
            .result
            .iter()
            .map(|f| json!({ "name": f.name, "type": f.ty.word(), "doc": f.doc }))
            .collect();
        json!({
            "name": self.name,
            "family": family.name,
            "summary": self.summary,
            "params": params,
            "result": result,
        })
    }
}

/// A handler's view of the request it is answering and the app it
/// may act on.
///
/// The app is reached through [`Call::drive`] (`&mut`) or
/// [`Call::observe`] (`&`), so a command that only reads cannot
/// mutate. Whether a command changed pixels is not inferred from the
/// borrow. The target answers it from each funnel's own outcome
/// ([`CommandTarget::changed_view`]), and the event loop requests a
/// redraw only then, as a winit handler does after a mutating input.
/// A refused action or a read-only console line costs no frame.
pub(crate) struct Call<'a> {
    request: &'a IpcRequest,
    /// `None` before the window, and with it the document, exists.
    target: Option<&'a mut dyn CommandTarget>,
}

impl<'a> Call<'a> {
    /// The value of parameter `name`, already checked against its
    /// declared type when present.
    pub(crate) fn param(&self, name: &str) -> Option<&'a Value> {
        self.request.params.get(name)
    }

    /// A string parameter. A declared-`string` parameter that is
    /// present always yields `Some`.
    pub(crate) fn str_param(&self, name: &str) -> Option<&'a str> {
        self.param(name).and_then(Value::as_str)
    }

    /// The app, for a command that acts on it.
    pub(crate) fn drive(&mut self) -> Option<&mut dyn CommandTarget> {
        // Re-wrapping shortens the trait object's lifetime to the
        // borrow of `self`; `&mut` is invariant, so it cannot be
        // handed out at the field's own lifetime.
        self.target
            .as_deref_mut()
            .map(|target| target as &mut dyn CommandTarget)
    }

    /// The app, read-only.
    pub(crate) fn observe(&self) -> Option<&dyn CommandTarget> {
        self.target.as_deref()
    }

    pub(crate) fn ok(&self, result: Value) -> Reply {
        Reply::ok(self.request.id.clone(), result)
    }

    pub(crate) fn error(&self, code: &'static str, message: impl Into<String>, data: Option<Value>) -> Reply {
        Reply::error(self.request.id.clone(), code, message, data)
    }

    pub(crate) fn invalid_params(&self, param: &str, message: impl Into<String>) -> Reply {
        Reply::invalid_params(self.request.id.clone(), param, message)
    }

    /// `no_document`, for a command that needs the app before the
    /// app has a document, which on native means before its window
    /// exists.
    pub(crate) fn no_document(&self) -> Reply {
        self.error(
            code::NO_DOCUMENT,
            "no document is loaded yet: the app is still starting; retry in a moment",
            None,
        )
    }
}

/// Execute one request. `target` is the app, or `None` while it is
/// still starting.
///
/// Never panics: an unknown command, a stray, missing or mistyped
/// parameter, and a command that fails are all error replies, and
/// the app keeps running (§9).
pub(crate) fn execute(request: &IpcRequest, target: Option<&mut dyn CommandTarget>) -> Executed {
    let Some((_, spec)) = lookup(&request.cmd) else {
        return Executed {
            reply: Reply::error(
                request.id.clone(),
                code::UNKNOWN_CMD,
                unknown_command_message(&request.cmd),
                None,
            ),
            repaint: false,
        };
    };
    if let Some(reply) = check_params(request, spec) {
        return Executed {
            reply,
            repaint: false,
        };
    }
    // Provenance is a logging concern, not a privilege one
    // (format/ipc.md §Trust model): say where a dispatch came from.
    log::debug!("ipc: executing `{}` (id {})", request.cmd, request.id);
    let mut call = Call {
        request,
        // Shortens the trait object's lifetime to the request's, which
        // is all a handler needs. `&mut` is invariant, so the field
        // cannot take the caller's lifetime as is.
        target: target.map(|target| target as &mut dyn CommandTarget),
    };
    let (reply, captured) = capture(|| (spec.run)(&mut call));
    Executed {
        reply: reply.with_warnings(warnings_from(captured)),
        repaint: call.observe().is_some_and(|target| target.changed_view()),
    }
}

/// The `invalid_params` reply for the first way `request` departs
/// from `spec`'s descriptors, or `None` when it conforms. Checked in
/// a fixed order (unknown, then missing, then mistyped) and, within
/// each, in key order, so the answer is deterministic.
fn check_params(request: &IpcRequest, spec: &CommandSpec) -> Option<Reply> {
    let invalid =
        |param: &str, message: String| Some(Reply::invalid_params(request.id.clone(), param, message));
    if let Some(stray) = request
        .params
        .keys()
        .find(|key| !spec.params.iter().any(|p| p.name == key.as_str()))
    {
        return invalid(
            stray,
            format!(
                "`{}` takes no parameter `{stray}`; send {{\"cmd\": \"describe\", \"command\": \"{}\"}} for the ones it accepts",
                spec.name, spec.name
            ),
        );
    }
    for param in spec.params {
        match request.params.get(param.name) {
            None if param.required => {
                return invalid(
                    param.name,
                    format!("`{}` requires `{}`: {}", spec.name, param.name, param.doc),
                )
            }
            Some(value) if !param.ty.admits(value) => {
                return invalid(
                    param.name,
                    format!(
                        "`{}` must be of type {}, got {value}",
                        param.name,
                        param.ty.word()
                    ),
                )
            }
            _ => {}
        }
    }
    None
}

/// The `unknown_cmd` message: what does exist near the name that
/// does not. A name in a registered family lists that family's
/// commands; any other name lists the families.
fn unknown_command_message(name: &str) -> String {
    // The bootstrap family is a label, not a namespace: `describe` and
    // `ping` carry no prefix, so `bootstrap.ping` names nothing in it.
    let in_family = name.split_once('.').and_then(|(prefix, _)| {
        FAMILIES
            .iter()
            .find(|f| f.name == prefix && f.name != BOOTSTRAP_FAMILY)
    });
    let nearby = match in_family {
        Some(family) => format!(
            "family `{}` has {}",
            family.name,
            family
                .commands
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => format!(
            "the families are {}",
            FAMILIES
                .iter()
                .map(|f| match f.name {
                    BOOTSTRAP_FAMILY => f.commands.iter().map(|c| c.name).collect::<Vec<_>>().join(", "),
                    name => format!("{name}.*"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!("no command '{name}' — {nearby}; send `describe` for the full table")
}

/// The reply's `warnings` array for what the execution window saw.
fn warnings_from(captured: Captured) -> Vec<Warning> {
    let mut warnings: Vec<Warning> = captured
        .records
        .into_iter()
        .map(|record| Warning {
            code: record.code.unwrap_or_else(|| {
                match record.level {
                    log::Level::Error => warning::LOG_ERROR,
                    _ => warning::LOG_WARN,
                }
                .to_string()
            }),
            message: record.message,
        })
        .collect();
    if captured.dropped > 0 {
        warnings.push(Warning {
            code: warning::WARNINGS_TRUNCATED.into(),
            message: format!(
                "{} further diagnostics were left out: one reply carries at most {CAPTURE_LIMIT}",
                captured.dropped
            ),
        });
    }
    warnings
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::application::ipc::envelope::{decode_line, Decoded, Frame, OUTBOUND_FRAME_CAP};

    /// Run `frame` through the real decoder and [`execute`] with no
    /// app attached, and return the reply as it would appear on the
    /// wire plus the repaint flag.
    pub(crate) fn run_detached(frame: &str) -> (Value, bool) {
        let Decoded::Request(request) = decode_line(frame.as_bytes(), 0, 1) else {
            panic!("{frame} must decode to a request");
        };
        let executed = execute(&request, None);
        let bytes = Frame::Reply(executed.reply)
            .encode(OUTBOUND_FRAME_CAP)
            .expect("encodes");
        (serde_json::from_slice(&bytes).expect("JSON"), executed.repaint)
    }

    #[test]
    fn test_every_command_name_is_unique_and_carries_its_family_namespace() {
        let mut seen = std::collections::HashSet::new();
        for (family, command) in commands() {
            assert!(
                seen.insert(command.name),
                "`{}` is registered twice",
                command.name
            );
            if family.name == BOOTSTRAP_FAMILY {
                assert!(
                    !command.name.contains('.'),
                    "bootstrap commands are unnamespaced: {}",
                    command.name
                );
            } else {
                let (prefix, verb) = command
                    .name
                    .split_once('.')
                    .unwrap_or_else(|| panic!("`{}` must be `family.verb`", command.name));
                assert_eq!(
                    prefix, family.name,
                    "`{}` sits in family `{}`",
                    command.name, family.name
                );
                let snake = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_');
                assert!(
                    snake(prefix) && snake(verb),
                    "`{}` must be lowercase snake_case",
                    command.name
                );
            }
        }
    }

    /// `describe` completeness, at the source: every command documents
    /// itself — summary, and a doc for every parameter and result
    /// field — and every parameter name is unique within its command.
    #[test]
    fn test_every_command_documents_itself_completely() {
        for (_, command) in commands() {
            assert!(
                !command.summary.trim().is_empty(),
                "{} has no summary",
                command.name
            );
            let mut names = std::collections::HashSet::new();
            for param in command.params {
                assert!(
                    names.insert(param.name),
                    "{} declares `{}` twice",
                    command.name,
                    param.name
                );
                assert!(
                    !param.doc.trim().is_empty(),
                    "{}.{} has no doc",
                    command.name,
                    param.name
                );
            }
            for field in command.result {
                assert!(
                    !field.doc.trim().is_empty(),
                    "{} result `{}` has no doc",
                    command.name,
                    field.name
                );
            }
        }
    }

    #[test]
    fn test_type_words_are_the_protocol_vocabulary() {
        const TYPE_WORDS: &[&str] = &["string", "integer", "number", "boolean", "object", "array"];
        for (_, command) in commands() {
            let types = command.params.iter().map(|p| (p.name, p.ty));
            for (name, ty) in types.chain(command.result.iter().map(|r| (r.name, r.ty))) {
                for word in ty.word().split('|') {
                    assert!(TYPE_WORDS.contains(&word), "{}.{name}: {word}", command.name);
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "may not be named `id` or `cmd`")]
    fn test_a_parameter_named_id_is_refused() {
        // Outside a `const` the same refusal is a panic; inside one,
        // where every registry descriptor lives, it is a compile error.
        let _ = ParamSpec::new("id", ParamType::String, false, "collides with the envelope");
    }

    #[test]
    #[should_panic(expected = "may not be named `id` or `cmd`")]
    fn test_a_parameter_named_cmd_is_refused() {
        let _ = ParamSpec::new("cmd", ParamType::String, false, "collides with the envelope");
    }

    #[test]
    fn test_param_types_admit_exactly_their_shape() {
        let any = [
            json!("s"),
            json!(1),
            json!(-1),
            json!(1.5),
            json!(true),
            json!({}),
            json!([]),
            Value::Null,
        ];
        let admitted = |ty: ParamType| any.iter().filter(|v| ty.admits(v)).cloned().collect::<Vec<_>>();
        assert_eq!(admitted(ParamType::String), [json!("s")]);
        assert_eq!(admitted(ParamType::Integer), [json!(1), json!(-1)]);
        assert_eq!(admitted(ParamType::Number), [json!(1), json!(-1), json!(1.5)]);
        assert_eq!(admitted(ParamType::Boolean), [json!(true)]);
        assert_eq!(admitted(ParamType::Object), [json!({})]);
        assert_eq!(admitted(ParamType::Array), [json!([])]);
        const STRING_OR_OBJECT: ParamType = ParamType::OneOf(&[ParamType::String, ParamType::Object]);
        assert_eq!(admitted(STRING_OR_OBJECT), [json!("s"), json!({})]);
        assert_eq!(STRING_OR_OBJECT.word(), "string|object");
    }

    #[test]
    fn test_unknown_command_names_its_family_or_the_families() {
        let (reply, repaint) = run_detached(r#"{"id":2,"cmd":"act.actoin"}"#);
        assert!(!repaint);
        assert_eq!(reply["id"], json!(2));
        assert_eq!(reply["error"]["code"], json!("unknown_cmd"));
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("act.action") && message.contains("describe"),
            "{message}"
        );

        let (reply, _) = run_detached(r#"{"id":3,"cmd":"clock.stepp"}"#);
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("act.*") && message.contains("ping"),
            "an unregistered family lists the families: {message}"
        );

        let (reply, _) = run_detached(r#"{"id":4,"cmd":"bootstrap.ping"}"#);
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(
            !message.contains("family `bootstrap`") && message.contains("act.*"),
            "`bootstrap` is not a namespace, so it is answered with the families: {message}"
        );
    }

    #[test]
    fn test_execute_rejects_stray_missing_and_mistyped_params_naming_the_offender() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"ping","verbose":true}"#, "verbose"),
            (r#"{"id":1,"cmd":"act.console"}"#, "line"),
            (r#"{"id":1,"cmd":"act.console","line":3}"#, "line"),
            (r#"{"id":1,"cmd":"act.console","line":null}"#, "line"),
            (r#"{"id":1,"cmd":"act.action","action":7}"#, "action"),
        ] {
            let (reply, repaint) = run_detached(frame);
            assert!(!repaint, "{frame}: a refused request changes nothing");
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}");
        }
    }

    #[test]
    fn test_a_command_that_needs_the_app_answers_no_document_before_it_exists() {
        for frame in [
            r#"{"id":1,"cmd":"act.action","action":"ZoomIn"}"#,
            r#"{"id":1,"cmd":"act.console","line":"help"}"#,
            r#"{"id":1,"cmd":"act.macro","macro_id":"m"}"#,
        ] {
            let (reply, repaint) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(!repaint, "{frame}: nothing was driven");
        }
    }

    #[test]
    fn test_warnings_carry_the_record_code_or_classify_by_level() {
        use baumhard::util::log::capture::CapturedRecord;

        let warnings = warnings_from(Captured {
            records: vec![
                CapturedRecord {
                    level: log::Level::Warn,
                    code: None,
                    message: "area: a".into(),
                },
                CapturedRecord {
                    level: log::Level::Error,
                    code: None,
                    message: "area: b".into(),
                },
                CapturedRecord {
                    level: log::Level::Warn,
                    code: Some(crate::application::macros::GATE_REJECTED_CODE.into()),
                    message: "macros: c".into(),
                },
            ],
            dropped: 3,
        });
        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "log_warn",
                "log_error",
                "macro_gate_rejected",
                "warnings_truncated"
            ]
        );
        assert_eq!(warnings[0].message, "area: a", "the log line's own words");
        assert!(
            warnings[3].message.starts_with("3 further"),
            "{}",
            warnings[3].message
        );
    }

    /// The execution window is live: a `warn!` emitted while a handler
    /// runs reaches the reply. `describe` with a bad filter emits none,
    /// so this drives the window directly with the real `capture` the
    /// executor uses.
    #[test]
    fn test_a_warning_logged_inside_the_window_reaches_the_reply() {
        baumhard::util::test_logger::install();
        let (reply, captured) = capture(|| {
            log::warn!("ipc-test: degraded-6c2e");
            Reply::ok(json!(1), json!({}))
        });
        let bytes = Frame::Reply(reply.with_warnings(warnings_from(captured)))
            .encode(OUTBOUND_FRAME_CAP)
            .expect("encodes");
        let wire: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(
            wire["warnings"],
            json!([{ "code": "log_warn", "message": "ipc-test: degraded-6c2e" }])
        );
    }
}
