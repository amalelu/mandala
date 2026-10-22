// SPDX-License-Identifier: MPL-2.0

//! The wire envelope as values: what a request decodes into, what a
//! reply or event encodes from, and the two byte caps that bound
//! both directions (`format/ipc.md` §Envelope, §Framing).
//!
//! Everything here is a pure function over plain values. The boundary
//! threads call [`decode_line`] and [`Frame::encode`]; neither needs a
//! socket, a window or app state, which is what lets the protocol's
//! hostile-input posture be tested byte for byte without either.
//!
//! **Errors are values, never Rust error types** (CODE_CONVENTIONS
//! §9): a frame that cannot become a request becomes the [`Reply`]
//! that says why, and the caller writes it like any other reply.

use serde::Serialize;
use serde_json::{Map, Value};

/// The one live protocol integer (`format/ipc.md` §Handshake and
/// versioning). Bumped, in the same commit as the document, by any
/// breaking change to framing, envelope or a shipped command.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// Inbound line cap: a longer line is consumed through its newline
/// and answered with a `parse_error`, and the stream stays usable.
pub(crate) const INBOUND_LINE_CAP: usize = 1 << 20;

/// Outbound frame cap: a reply that would serialize larger is
/// replaced by `reply_too_large`.
pub(crate) const OUTBOUND_FRAME_CAP: usize = 8 << 20;

/// Cap on a request `id`'s serialized length. An oversized id is not
/// echoed back — the reply carries `"id": null`.
const ID_CAP: usize = 256;

/// Error codes this build emits. **Codes are API** (`format/ipc.md`
/// §Errors): agents branch on them, so renaming one is a protocol
/// bump. A code joins this list in the commit whose command first
/// emits it, and `test_every_emitted_error_code_is_in_the_registry`
/// holds each against the documented registry table.
pub(crate) mod code {
    pub(crate) const PARSE_ERROR: &str = "parse_error";
    pub(crate) const UNKNOWN_CMD: &str = "unknown_cmd";
    pub(crate) const INVALID_PARAMS: &str = "invalid_params";
    pub(crate) const OVERLOADED: &str = "overloaded";
    pub(crate) const REPLY_TOO_LARGE: &str = "reply_too_large";
    pub(crate) const NO_DOCUMENT: &str = "no_document";
    pub(crate) const NOT_FOUND: &str = "not_found";
    pub(crate) const CONSOLE_ERROR: &str = "console_error";
    pub(crate) const INTERNAL: &str = "internal";

    /// Every code above, for the registry test.
    #[cfg(test)]
    pub(crate) const ALL: &[&str] = &[
        PARSE_ERROR,
        UNKNOWN_CMD,
        INVALID_PARAMS,
        OVERLOADED,
        REPLY_TOO_LARGE,
        NO_DOCUMENT,
        NOT_FOUND,
        CONSOLE_ERROR,
        INTERNAL,
    ];
}

/// A well-formed request, decoded on a reader thread and executed on
/// the main thread.
///
/// `generation` is the stamp of the controller connection the line
/// arrived on; the main thread executes a request only while that
/// generation is the live one, so nothing a departed controller sent
/// can act after it left (`work_plans/LLM_IPC.md` §D2).
#[derive(Debug)]
pub(crate) struct IpcRequest {
    pub(crate) generation: u64,
    /// Echoed verbatim on the reply. Already validated: a string or
    /// an integer, at most [`ID_CAP`] bytes serialized.
    pub(crate) id: Value,
    pub(crate) cmd: String,
    /// Everything beside `id` and `cmd` — params sit flat at the top
    /// level of the frame. Unknown ones are the command's to reject.
    pub(crate) params: Map<String, Value>,
}

/// One server → client reply, correlated to its request by `id`.
/// Exactly one of `result` / `error` is present, by construction:
/// the only ways to build one are [`Reply::ok`] and [`Reply::error`].
#[derive(Debug, Serialize)]
pub(crate) struct Reply {
    id: Value,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    /// Boxed so a `Reply` stays small enough to travel as the `Err`
    /// side of a handler's parse helpers (`Result<_, Reply>`) without
    /// every `Ok` paying for the error body.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Box<ErrorBody>>,
    /// The command's diagnostics (`format/ipc.md` §Warnings). Omitted
    /// when empty, on success and failure alike.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<Warning>,
}

/// One entry of a reply's `warnings` array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Warning {
    /// Stable snake_case identifier; API, like an error code
    /// ([`warning`] lists every one this build emits).
    pub(crate) code: String,
    /// What happened, in the words the log site used.
    pub(crate) message: String,
}

/// Warning codes this build emits. **Codes are API**, held to
/// `format/ipc.md`'s warning registry table by
/// `test_every_emitted_warning_code_is_in_the_registry`.
///
/// A coded log site attaches its code as the record's `code`
/// key-value, so the code is chosen once, where the degrade happens.
/// The site's constant lives beside it, in the module that owns the
/// degrade, and is listed in the test-only `ALL` below. Uncoded
/// records are classified by level as
/// [`LOG_WARN`](warning::LOG_WARN) /
/// [`LOG_ERROR`](warning::LOG_ERROR).
pub(crate) mod warning {
    /// A `warn!` the command's execution emitted with no code of its
    /// own; `message` is the log line.
    pub(crate) const LOG_WARN: &str = "log_warn";
    /// An `error!` the command's execution emitted with no code of
    /// its own; `message` is the log line.
    pub(crate) const LOG_ERROR: &str = "log_error";
    /// The command emitted more diagnostics than one reply carries;
    /// `message` says how many were left out.
    pub(crate) const WARNINGS_TRUNCATED: &str = "warnings_truncated";

    /// Every code this build emits, for the registry test: the three
    /// above, and each coded log site's constant.
    #[cfg(test)]
    pub(crate) const ALL: &[&str] = &[
        LOG_WARN,
        LOG_ERROR,
        WARNINGS_TRUNCATED,
        // A macro step was refused by its tier's privilege gate and
        // the rest of the macro aborted.
        crate::application::macros::GATE_REJECTED_CODE,
        // A console line run by an Action or a macro step failed.
        crate::application::console::LINE_FAILED_CODE,
        // SaveDocument had no bound path, or its write failed.
        crate::application::console::SAVE_UNBOUND_CODE,
        crate::application::console::SAVE_FAILED_CODE,
    ];
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl Reply {
    pub(crate) fn ok(id: Value, result: Value) -> Self {
        Reply {
            id,
            ok: true,
            result: Some(result),
            error: None,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn error(
        id: Value,
        code: &'static str,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Reply {
            id,
            ok: false,
            result: None,
            error: Some(Box::new(ErrorBody {
                code,
                message: message.into(),
                data,
            })),
            warnings: Vec::new(),
        }
    }

    /// The same reply, carrying `warnings`.
    pub(crate) fn with_warnings(mut self, warnings: Vec<Warning>) -> Self {
        self.warnings = warnings;
        self
    }

    /// `invalid_params` naming the offending parameter in
    /// `data.param`, as the error registry requires.
    pub(crate) fn invalid_params(id: Value, param: &str, message: impl Into<String>) -> Self {
        Reply::error(
            id,
            code::INVALID_PARAMS,
            message,
            Some(serde_json::json!({ "param": param })),
        )
    }

    /// `parse_error` for inbound line `line`. The id is always
    /// `null` — there was no parseable id to echo — so `data.line` is
    /// how a pipelining client finds which of its lines failed.
    pub(crate) fn parse_error(line: u64, message: impl Into<String>) -> Self {
        Reply::error(
            Value::Null,
            code::PARSE_ERROR,
            message,
            Some(serde_json::json!({ "line": line })),
        )
    }

    /// The `parse_error` for a line that blew [`INBOUND_LINE_CAP`].
    pub(crate) fn oversized_line(line: u64) -> Self {
        Reply::parse_error(
            line,
            format!(
                "line exceeds the {} MiB inbound cap and was discarded; route large payloads \
                 through files (format/ipc.md \"Large payloads\")",
                INBOUND_LINE_CAP >> 20
            ),
        )
    }

    /// `overloaded`: the request queue already holds its cap of
    /// unexecuted requests. The request is not queued; the client
    /// resends once it has drained replies.
    pub(crate) fn overloaded(id: Value, cap: usize) -> Self {
        Reply::error(
            id,
            code::OVERLOADED,
            format!("request queue full ({cap} pending); resend after draining replies"),
            None,
        )
    }
}

/// One unsolicited server → client event. The class names a client
/// may see are the catalog `describe` publishes.
#[derive(Debug, Serialize)]
pub(crate) struct Event {
    event: &'static str,
    data: Value,
}

impl Event {
    /// The greeting written on accept, before anything is read.
    /// `map` is the main thread's last-published document path —
    /// `None` before the first load.
    pub(crate) fn hello(map: Option<&str>) -> Self {
        Event {
            event: "hello",
            data: serde_json::json!({
                "protocol": PROTOCOL_VERSION,
                "app": "mandala",
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
                "map": map,
            }),
        }
    }

    /// Best-effort notice that the app is exiting, so a client can
    /// tell "app exited" from "connection died".
    pub(crate) fn shutdown() -> Self {
        Event {
            event: "shutdown",
            data: serde_json::json!({}),
        }
    }

    /// The single frame a refused connection receives instead of any
    /// greeting: a controller is already attached.
    pub(crate) fn connection_rejected_busy() -> Self {
        Event {
            event: "connection_rejected",
            data: serde_json::json!({ "reason": "busy" }),
        }
    }
}

/// A frame bound for the wire. The outbound thread serializes it, so
/// the cap check and the `reply_too_large` substitution run off the
/// interactive path.
#[derive(Debug)]
pub(crate) enum Frame {
    Reply(Reply),
    Event(Event),
}

impl Frame {
    /// Append the frame to `out` as one newline-terminated NDJSON
    /// line, held to `cap` bytes (the newline excluded). Appending
    /// rather than returning is what lets the outbound thread gather
    /// every queued frame into a single write.
    ///
    /// Serialization stops the moment it passes the cap, so a
    /// pathological reply never costs more than `cap` bytes of buffer.
    /// It is replaced by `reply_too_large` carrying the same `id`, so
    /// the client's correlation still resolves, and its full size —
    /// `data.bytes` — is measured by a second pass into a counting sink
    /// that allocates nothing. An event over the cap has no requester
    /// to tell; nothing is appended and `false` returned, which the
    /// outbound thread logs. Serialization itself cannot otherwise fail
    /// for these types (string keys, no maps keyed by non-strings), but
    /// should it, a reply degrades to `internal` rather than vanishing.
    ///
    /// Cost: one serialization into `out`; an oversized reply adds one
    /// non-allocating counting pass and the small substitute.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>, cap: usize) -> bool {
        let start = out.len();
        let mut capped = CappedSink {
            out: &mut *out,
            limit: start + cap,
            overflowed: false,
        };
        let serialized = self.serialize(&mut capped);
        let overflowed = capped.overflowed;
        if let Err(e) = serialized {
            out.truncate(start);
            let Frame::Reply(reply) = self else { return false };
            let substitute = if overflowed {
                let mut counter = CountingSink(0);
                // Only the count matters; the sink cannot fail.
                let _ = self.serialize(&mut counter);
                Reply::error(
                    reply.id.clone(),
                    code::REPLY_TOO_LARGE,
                    format!("reply of {} bytes exceeds the {cap} byte outbound cap", counter.0),
                    Some(serde_json::json!({ "bytes": counter.0 })),
                )
            } else {
                Reply::error(
                    reply.id.clone(),
                    code::INTERNAL,
                    format!("reply could not be serialized: {e}"),
                    None,
                )
            };
            if serde_json::to_writer(&mut *out, &substitute).is_err() {
                out.truncate(start);
                return false;
            }
        }
        out.push(b'\n');
        true
    }

    /// [`Frame::encode_into`] a fresh buffer — for a frame written on
    /// its own. `None` when the frame was dropped.
    pub(crate) fn encode(&self, cap: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_into(&mut out, cap).then_some(out)
    }

    fn serialize(&self, sink: &mut impl std::io::Write) -> serde_json::Result<()> {
        match self {
            Frame::Reply(reply) => serde_json::to_writer(sink, reply),
            Frame::Event(event) => serde_json::to_writer(sink, event),
        }
    }
}

/// A writer into a `Vec` that refuses to grow it past `limit`, so an
/// oversized frame is abandoned at the cap instead of being built in
/// full first.
struct CappedSink<'a> {
    out: &'a mut Vec<u8>,
    limit: usize,
    overflowed: bool,
}

impl std::io::Write for CappedSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.out.len() + buf.len() > self.limit {
            self.overflowed = true;
            return Err(std::io::Error::other("frame exceeds the outbound cap"));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A writer that keeps nothing and counts what it is given.
struct CountingSink(usize);

impl std::io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What one inbound line decodes into.
#[derive(Debug)]
pub(crate) enum Decoded {
    /// A well-formed request, ready for the main thread.
    Request(IpcRequest),
    /// A line that cannot become a request, and the reply that says
    /// why. Never needs app state, so the reader writes it directly.
    Rejected(Reply),
    /// An empty (or whitespace-only) line, which the protocol ignores.
    Blank,
}

/// Decode inbound line number `line` (0-based, counting every line
/// the connection has sent, blank ones included) arriving on
/// controller `generation`.
///
/// Rejections follow the error registry: bytes that are not a JSON
/// object are `parse_error` with `data.line`; an object whose `id` is
/// missing, mistyped or oversized is `invalid_params` with
/// `"id": null`, because there is no id fit to echo; an object with a
/// good `id` but no string `cmd` is `invalid_params` echoing that id.
///
/// Cost: one `serde_json` parse of the line; the object's own map
/// becomes the params map without copying its values.
pub(crate) fn decode_line(line: &[u8], line_index: u64, generation: u64) -> Decoded {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Decoded::Blank;
    }
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(e) => return Decoded::Rejected(Reply::parse_error(line_index, format!("not valid JSON: {e}"))),
    };
    let Value::Object(mut params) = value else {
        return Decoded::Rejected(Reply::parse_error(
            line_index,
            "a request frame must be a JSON object: {\"id\": …, \"cmd\": …, <params>}",
        ));
    };

    let id = match params.remove("id") {
        None => {
            return Decoded::Rejected(Reply::invalid_params(
                Value::Null,
                "id",
                "missing `id`: every request carries a client-chosen string or integer id",
            ))
        }
        Some(id) if !(id.is_string() || id.is_i64() || id.is_u64()) => {
            return Decoded::Rejected(Reply::invalid_params(
                Value::Null,
                "id",
                "`id` must be a string or an integer",
            ))
        }
        Some(id) => id,
    };
    // `to_string` on a `Value` cannot fail; the length is what is
    // capped, so a giant id string is refused without being echoed.
    let id_bytes = id.to_string().len();
    if id_bytes > ID_CAP {
        return Decoded::Rejected(Reply::invalid_params(
            Value::Null,
            "id",
            format!("`id` serializes to {id_bytes} bytes; the cap is {ID_CAP}"),
        ));
    }

    let cmd = match params.remove("cmd") {
        Some(Value::String(cmd)) => cmd,
        Some(_) => return Decoded::Rejected(Reply::invalid_params(id, "cmd", "`cmd` must be a string")),
        None => {
            return Decoded::Rejected(Reply::invalid_params(
                id,
                "cmd",
                "missing `cmd`: name the command to run, e.g. {\"id\": 1, \"cmd\": \"ping\"}",
            ))
        }
    };

    Decoded::Request(IpcRequest {
        generation,
        id,
        cmd,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The JSON a frame puts on the wire, minus its newline.
    fn wire(frame: Frame) -> Value {
        let bytes = frame
            .encode(OUTBOUND_FRAME_CAP)
            .expect("frame encodes under the cap");
        assert_eq!(bytes.last(), Some(&b'\n'), "every frame is newline-terminated");
        assert!(
            !bytes[..bytes.len() - 1].contains(&b'\n'),
            "a frame must not contain a raw newline"
        );
        serde_json::from_slice(&bytes).expect("the encoder emits JSON")
    }

    fn rejected(line: &[u8]) -> Value {
        match decode_line(line, 7, 1) {
            Decoded::Rejected(reply) => wire(Frame::Reply(reply)),
            other => panic!(
                "expected a rejection for {:?}, got {other:?}",
                String::from_utf8_lossy(line)
            ),
        }
    }

    #[test]
    fn test_decode_line_splits_envelope_from_flat_params() {
        let Decoded::Request(request) = decode_line(br#"{"id":7,"cmd":"clock.step","ms":100}"#, 0, 3) else {
            panic!("a well-formed frame must decode to a request");
        };
        assert_eq!(request.generation, 3);
        assert_eq!(request.id, json!(7));
        assert_eq!(request.cmd, "clock.step");
        assert_eq!(Value::Object(request.params), json!({ "ms": 100 }));
    }

    #[test]
    fn test_decode_line_accepts_string_ids_and_tolerates_crlf() {
        let Decoded::Request(request) = decode_line(b"{\"id\":\"a-1\",\"cmd\":\"ping\"}\r", 0, 1) else {
            panic!("a CRLF-terminated frame is still a frame");
        };
        assert_eq!(request.id, json!("a-1"));
        assert!(request.params.is_empty());
    }

    #[test]
    fn test_decode_line_ignores_blank_lines() {
        for blank in [&b""[..], b"   ", b"\r", b"\t \r"] {
            assert!(matches!(decode_line(blank, 0, 1), Decoded::Blank), "{blank:?}");
        }
    }

    #[test]
    fn test_decode_line_answers_garbage_with_a_located_parse_error() {
        for garbage in [
            &b"not json"[..],
            b"{\"id\":1,",
            b"\xff\xfe\x00",
            b"[1,2,3]",
            b"42",
            b"\"ping\"",
        ] {
            let reply = rejected(garbage);
            assert_eq!(reply["id"], Value::Null, "{garbage:?}");
            assert_eq!(reply["ok"], json!(false));
            assert_eq!(reply["error"]["code"], json!("parse_error"));
            assert_eq!(
                reply["error"]["data"]["line"],
                json!(7),
                "data.line locates the line"
            );
            assert!(reply.get("result").is_none(), "an error reply carries no result");
        }
    }

    #[test]
    fn test_decode_line_rejects_a_bad_id_without_echoing_it() {
        let oversized = format!(r#"{{"id":"{}","cmd":"ping"}}"#, "x".repeat(ID_CAP));
        for line in [
            r#"{"cmd":"ping"}"#,
            r#"{"id":1.5,"cmd":"ping"}"#,
            r#"{"id":null,"cmd":"ping"}"#,
            r#"{"id":[1],"cmd":"ping"}"#,
            r#"{"id":true,"cmd":"ping"}"#,
            oversized.as_str(),
        ] {
            let reply = rejected(line.as_bytes());
            assert_eq!(reply["id"], Value::Null, "{line}");
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{line}");
            assert_eq!(reply["error"]["data"]["param"], json!("id"), "{line}");
        }
    }

    #[test]
    fn test_decode_line_id_cap_is_inclusive() {
        // A string id serializes with its two quotes.
        let at_cap = format!(r#"{{"id":"{}","cmd":"ping"}}"#, "x".repeat(ID_CAP - 2));
        assert!(matches!(
            decode_line(at_cap.as_bytes(), 0, 1),
            Decoded::Request(_)
        ));
    }

    #[test]
    fn test_decode_line_rejects_a_missing_or_mistyped_cmd_echoing_the_id() {
        for line in [
            r#"{"id":"k"}"#,
            r#"{"id":"k","cmd":3}"#,
            r#"{"id":"k","cmd":null}"#,
        ] {
            let reply = rejected(line.as_bytes());
            assert_eq!(reply["id"], json!("k"), "{line}");
            assert_eq!(reply["error"]["code"], json!("invalid_params"));
            assert_eq!(reply["error"]["data"]["param"], json!("cmd"));
        }
    }

    #[test]
    fn test_reply_shapes_match_the_envelope() {
        assert_eq!(
            wire(Frame::Reply(Reply::ok(json!(7), json!({ "now_ms": 1.5 })))),
            json!({ "id": 7, "ok": true, "result": { "now_ms": 1.5 } })
        );
        assert_eq!(
            wire(Frame::Reply(Reply::error(
                json!("x"),
                code::UNKNOWN_CMD,
                "nope",
                None
            ))),
            json!({ "id": "x", "ok": false, "error": { "code": "unknown_cmd", "message": "nope" } })
        );
    }

    #[test]
    fn test_event_shapes_match_the_envelope() {
        let hello = wire(Frame::Event(Event::hello(Some("maps/a.mindmap.json"))));
        assert_eq!(hello["event"], json!("hello"));
        assert_eq!(hello["data"]["protocol"], json!(PROTOCOL_VERSION));
        assert_eq!(hello["data"]["app"], json!("mandala"));
        assert_eq!(hello["data"]["version"], json!(env!("CARGO_PKG_VERSION")));
        assert_eq!(hello["data"]["pid"], json!(std::process::id()));
        assert_eq!(hello["data"]["map"], json!("maps/a.mindmap.json"));
        assert_eq!(
            wire(Frame::Event(Event::hello(None)))["data"]["map"],
            Value::Null,
            "`map` is null before the first load"
        );
        assert_eq!(
            wire(Frame::Event(Event::shutdown())),
            json!({ "event": "shutdown", "data": {} })
        );
        assert_eq!(
            wire(Frame::Event(Event::connection_rejected_busy())),
            json!({ "event": "connection_rejected", "data": { "reason": "busy" } })
        );
    }

    #[test]
    fn test_an_oversized_reply_becomes_reply_too_large_with_its_id() {
        let big = Frame::Reply(Reply::ok(json!(9), json!({ "blob": "y".repeat(200) })));
        let bytes = big.encode(64).expect("a substituted reply still encodes");
        let reply: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(reply["id"], json!(9), "correlation survives the substitution");
        assert_eq!(reply["error"]["code"], json!("reply_too_large"));
        let full = big.encode(OUTBOUND_FRAME_CAP).expect("fits the real cap").len() - 1;
        assert_eq!(
            reply["error"]["data"]["bytes"],
            json!(full),
            "the counted size is the reply's real size"
        );

        let fits = Frame::Reply(Reply::ok(json!(9), json!({})));
        let exact = fits.encode(OUTBOUND_FRAME_CAP).expect("fits").len() - 1;
        assert!(
            fits.encode(exact).is_some_and(|b| b.len() == exact + 1),
            "the cap is inclusive"
        );
    }

    #[test]
    fn test_encode_into_appends_and_leaves_earlier_frames_intact() {
        let mut batch = Vec::new();
        assert!(Frame::Reply(Reply::ok(json!(1), json!({}))).encode_into(&mut batch, 64));
        let first = batch.clone();
        assert!(
            !Frame::Event(Event::hello(Some(&"m".repeat(500)))).encode_into(&mut batch, 64),
            "an oversized event is dropped"
        );
        assert_eq!(batch, first, "a dropped frame leaves no partial bytes behind");
        let big = Frame::Reply(Reply::ok(json!(2), json!({ "blob": "y".repeat(200) })));
        assert!(
            big.encode_into(&mut batch, 64),
            "an oversized reply is substituted"
        );
        assert!(batch.starts_with(&first));
        let lines: Vec<Value> = batch
            .split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("each line is JSON"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["error"]["code"], json!("reply_too_large"));
    }

    #[test]
    fn test_an_oversized_event_is_dropped_rather_than_truncated() {
        assert!(Frame::Event(Event::hello(Some(&"m".repeat(500))))
            .encode(64)
            .is_none());
    }

    #[test]
    fn test_oversized_line_reply_is_a_located_parse_error() {
        let reply = wire(Frame::Reply(Reply::oversized_line(12)));
        assert_eq!(reply["id"], Value::Null);
        assert_eq!(reply["error"]["code"], json!("parse_error"));
        assert_eq!(reply["error"]["data"]["line"], json!(12));
    }

    /// Warning codes are API on the same terms as error codes, and
    /// have their own registry table.
    #[test]
    fn test_every_emitted_warning_code_is_in_the_registry() {
        use baumhard::util::doc_fixtures::{documented_paragraph, repo_path};

        let table = documented_paragraph(
            &repo_path("format/ipc.md"),
            "### Warnings — per-command diagnostics",
            "| warning code | meaning |",
        );
        for emitted in warning::ALL {
            assert!(
                table.contains(&format!("| `{emitted}` |")),
                "warning code `{emitted}` is emitted but has no row in format/ipc.md's warning table"
            );
        }
    }

    #[test]
    fn test_warnings_are_omitted_when_empty_and_carried_when_present() {
        assert!(
            wire(Frame::Reply(Reply::ok(json!(1), json!({}))))
                .get("warnings")
                .is_none(),
            "an empty warnings array is omitted"
        );
        let warned = Reply::error(json!(2), code::INTERNAL, "boom", None).with_warnings(vec![Warning {
            code: warning::LOG_WARN.into(),
            message: "area: degraded".into(),
        }]);
        assert_eq!(
            wire(Frame::Reply(warned))["warnings"],
            json!([{ "code": "log_warn", "message": "area: degraded" }]),
            "a failed command's diagnostics still reach the client"
        );
    }

    /// **Codes are API, and the registry table is where the API is
    /// written down.** Every code this build can emit must appear as
    /// a row of `format/ipc.md`'s error-code table, so an agent
    /// pinning against the document never meets an undocumented code.
    #[test]
    fn test_every_emitted_error_code_is_in_the_registry() {
        use baumhard::util::doc_fixtures::{documented_paragraph, repo_path};

        let table = documented_paragraph(&repo_path("format/ipc.md"), "### Errors", "| code | meaning |");
        for emitted in code::ALL {
            assert!(
                table.contains(&format!("| `{emitted}` |")),
                "error code `{emitted}` is emitted but has no row in format/ipc.md's registry table"
            );
        }
    }
}
