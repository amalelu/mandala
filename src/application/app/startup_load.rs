// SPDX-License-Identifier: MPL-2.0

//! What the shell does with the initial map load — the one answer
//! both targets use.
//!
//! Startup is the surface most likely to be a user's first contact
//! with the format, and it used to be the only one that swallowed a
//! loader error: the console `open` verb puts the message in the
//! overlay, `maptool` puts it on `stderr` and exits nonzero, and
//! startup logged it and rendered an empty canvas. On a desktop
//! launch — double-click, `.desktop` entry, file association — no
//! terminal is attached, so that log line does not exist for the
//! person who needs it.
//!
//! The answer is a *document*: a rejected load becomes the
//! [`baumhard::mindmap::placard`] map, which the shell then installs
//! exactly as it installs a real one. The consequences are the point
//! of doing it this way — the window opens, the canvas has content,
//! the camera fits it, input works, and the console (native) can
//! `open` something else without restarting. None of that needed a
//! new render path, and none of it needs a GPU to test.
//!
//! ## One call per target, taking only the source
//!
//! `native_startup_document` and `wasm_startup_document` are the
//! whole surface. Each takes the path or URL and returns the document
//! to install; neither hands its caller a `Result` to look at. That
//! is deliberate and it is the part worth defending: while the init
//! sites composed the load themselves, the loader's message passed
//! through their hands, and blanking it on the browser leg alone was
//! a one-line change that left every test green and put a placard
//! with an empty diagnosis on the canvas. There is now no `Result` at
//! either init site to intercept.
//!
//! What stays browser-only is `fetch_map_json` and the two-line
//! `wasm_startup_document` wrapper — `cargo test` runs the native
//! leg only (`TEST_CONVENTIONS.md` §T9), and `wasm-bindgen-test` is
//! not a tool this repo has (§T10), so neither can be reached at
//! runtime from here. The composition between them is therefore
//! lifted into [`browser_load`], which is target-independent,
//! compiled on native and tested there against the exact error
//! strings the fetch produces.
//!
//! The residue is pinned against the source, **in this file**:
//! `test_the_browser_entry_point_adds_nothing_of_its_own` holds
//! `wasm_startup_document`'s two statements by *equality*, and
//! `test_the_fetch_returns_a_real_diagnosis_that_never_names_the_url`
//! reads `fetch_map_json`'s messages out of its own body. Both were
//! once claimed for `test_both_targets_go_through_one_startup_call`,
//! which reads the two *init* files and never opens this one — so the
//! privacy of [`StartupSurface`] closed the init sites while the
//! module's own browser leg stayed open, and blanking the URL one
//! level in was still a one-line change under a green suite.

use baumhard::mindmap::placard;

use crate::application::document::MindMapDocument;

/// The decision: given the result of the initial load, what goes on
/// the canvas.
///
/// Every arm carries a document, because the shell always has
/// something to render — that is the fix. The variants exist so the
/// *difference* between them stays explicit and exhaustively
/// matched: a third outcome (a map that loaded with warnings worth
/// showing, say) is a compile error in [`resolve`] rather than a
/// silent fallthrough back to the empty window.
///
/// Private to this module, which the single-call entry points above
/// are what make possible: nothing outside can build one, take one
/// apart, or interpose on the way to [`adopt`].
enum StartupSurface {
    /// The map loaded. Render it.
    Loaded(MindMapDocument),
    /// The map did not load. Render `placard` — which carries the
    /// loader's own text — and report [`report_line`].
    Rejected {
        /// The one-node map standing in for the map that failed.
        placard: MindMapDocument,
        /// The path or URL that was asked for.
        source: String,
        /// The loader's message, verbatim. Held separately from the
        /// placard so the log line and the canvas cannot disagree
        /// about what went wrong.
        message: String,
    },
}

/// Native entry point: load `path` and return the document to
/// install, reporting on the way past if it did not load.
///
/// The whole of what `run_native_init` does about the initial load.
/// It takes a path and gets a document — see the module docs for why
/// no `Result` crosses that boundary.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_startup_document(path: &str) -> MindMapDocument {
    adopt(startup_surface(path, MindMapDocument::load(path)))
}

/// Browser entry point: fetch `url` over the page origin, parse it,
/// and return the document to install, reporting on the way past if
/// either half failed.
///
/// The peer of `native_startup_document`, with the same signature
/// shape and the same guarantee (CODE_CONVENTIONS §4). Everything it
/// does other than the fetch is [`browser_load`], which is compiled
/// and tested on native.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn wasm_startup_document(url: &str) -> MindMapDocument {
    let fetched = fetch_map_json(url).await;
    adopt(startup_surface(url, browser_load(url, fetched)))
}

/// Collapse the browser's two-stage load — fetch, then parse — into
/// the single `Result` the surface takes, tagging the document with
/// `url` so a later save-back knows where it came from.
///
/// Target-independent on purpose. The fetch is the caller's, so this
/// compiles on native and
/// `test_the_browser_leg_carries_every_fetch_message_through` runs it
/// against the real strings `fetch_map_json` emits, rather than
/// leaving the browser's only failure path to a source scan.
// Consumer is `run_wasm/`; the host build reaches this only from
// `#[cfg(test)]`, so arm the lint on wasm32 and silence it here
// rather than blanket-allowing on both targets — if the browser
// caller goes away, wasm32 says so.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn browser_load(url: &str, fetched: Result<String, String>) -> Result<MindMapDocument, String> {
    fetched.and_then(|json| MindMapDocument::from_json_str(&json, Some(url.to_string())))
}

/// Fetch `url` as text over the page origin. Browser-only:
/// `std::fs` does not exist there, and this is the counterpart to
/// native's `loader::load_from_file`.
///
/// Lives here rather than in `run_wasm` because it is one half of the
/// initial load, and the module that owns the initial load is the one
/// whose tests pin it.
///
/// **None of these messages names `url`, and how many there are is
/// not written down here.** It was — as "five", while the code
/// returned seven and a test checked six, all three of which looked
/// checked. The list is now read out of this body by
/// `tests::fetch_error_messages` and every pin over it runs from
/// there. The messages do not have to name the URL:
/// [`report_line`] puts the source in front of whatever comes back,
/// once, for both targets — so a 404 reads `startup: could not load
/// 'maps/x.mindmap.json': HTTP 404 Not Found` rather than the bare
/// `HTTP 404 Not Found` that told a reader with more than one map
/// nothing.
#[cfg(target_arch = "wasm32")]
async fn fetch_map_json(url: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no global window")?;
    let promise = window.fetch_with_str(url);
    let resp_value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("fetch failed: {:?}", e))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "fetch did not return a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} {}", resp.status(), resp.status_text()));
    }
    let text_promise = resp
        .text()
        .map_err(|e| format!("Response::text() failed: {:?}", e))?;
    wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("reading response body failed: {:?}", e))?
        .as_string()
        .ok_or_else(|| "response body was not a string".to_string())
}

/// Decide what the shell shows for `load`, the result of the initial
/// map load of `source` (the CLI path on native, the `?map=` URL in
/// the browser).
///
/// Pure: builds a document either way and reports nothing. The
/// reporting lives in [`adopt`], which is what makes this function
/// assertable line by line without a logger or a device.
fn startup_surface(source: &str, load: Result<MindMapDocument, String>) -> StartupSurface {
    match load {
        Ok(document) => StartupSurface::Loaded(document),
        Err(message) => StartupSurface::Rejected {
            placard: MindMapDocument::load_failure_placard(source, &message),
            source: source.to_string(),
            message,
        },
    }
}

/// The line a rejected load reports: the source that was asked for,
/// then the loader's own message — **with the source appearing
/// exactly once**.
///
/// *Why the source is added.* Exactly one of the loader's failure
/// modes names the file — `load_from_file`'s *read* error — while
/// `load_from_str`'s parse errors and every one of `fetch_map_json`'s
/// never do. Without this the log read `startup: Failed to parse
/// mindmap JSON: key must be a string at line 1 column 3`, which does
/// not say *which* map, and a line that cannot be pasted into a bug
/// report is not worth emitting.
///
/// *Why it is conditional.* The one failure mode that does name the
/// file is also the likeliest way anybody reaches this screen — a
/// mistyped path — and prepending unconditionally produced `startup:
/// could not load '/nope/x.mindmap.json': Failed to read file
/// /nope/x.mindmap.json: No such file or directory`. The predicate is
/// [`placard::message_names_source`], shared with the canvas so the
/// two surfaces cannot disagree about what "once" means.
///
/// Quoted because a path may contain spaces and what follows it is
/// prose.
fn report_line(source: &str, message: &str) -> String {
    if placard::message_names_source(source, message) {
        return message.to_string();
    }
    format!("could not load '{source}': {message}")
}

/// Take the document the canvas will render, reporting the loader's
/// message on the way past when there was one.
///
/// The single place either target turns a [`StartupSurface`] into a
/// document. The log line still fires — a terminal user and a bug
/// report both want it, and the placard is an addition to it, not a
/// replacement for it.
///
/// Three lines long on purpose: everything that *decides* lives in
/// [`resolve`], which is pure and pinned, so the only thing not
/// covered by a test here is the `log::error!` invocation itself.
fn adopt(surface: StartupSurface) -> MindMapDocument {
    let (document, report) = resolve(surface);
    if let Some(message) = report {
        log::error!("startup: {}", message);
    }
    document
}

/// The document to render, and the line to report if there is one.
///
/// The exhaustive match: a new [`StartupSurface`] variant is a
/// compile error here, not a silent fallthrough back to the empty
/// window #107 is about. Pure, so both halves of the answer — which
/// document, and what gets said about it — are assertable without a
/// logger or a device.
fn resolve(surface: StartupSurface) -> (MindMapDocument, Option<String>) {
    match surface {
        StartupSurface::Loaded(document) => (document, None),
        StartupSurface::Rejected {
            placard,
            source,
            message,
        } => (placard, Some(report_line(&source, &message))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::placard;
    use baumhard::util::rust_source::{self, braced_block_after, production_code, string_literals};

    /// This module's own path, for the pins that read it.
    const THIS_FILE: &str = "src/application/app/startup_load.rs";

    /// The two init files. Each must reach the canvas through one
    /// call into this module and nothing else.
    const INIT_FILES: &[(&str, &str)] = &[
        (
            "src/application/app/run_native_init.rs",
            "startup_load::native_startup_document(&options.mindmap_path)",
        ),
        (
            "src/application/app/run_wasm/mod.rs",
            "startup_load::wasm_startup_document(&mindmap_path).await",
        ),
    ];

    /// The failing shape a hand-author actually meets: valid JSON,
    /// but a node with no sections. Routed through the real loader
    /// rather than a fabricated `Err`, so the message under test is
    /// the message the loader emits.
    const ZERO_SECTION_MAP: &str = r##"{
        "version": "1.0",
        "name": "broken",
        "canvas": {"background_color": "#000", "default_border": null,
                   "default_connection": null, "theme_variables": {},
                   "theme_variants": {}},
        "nodes": {"0": {
            "id": "0", "parent_id": null,
            "position": {"x": 0, "y": 0},
            "size": {"width": 100, "height": 50},
            "sections": [],
            "style": {"background_color":"#000","frame_color":"#000",
                      "text_color":"#fff","shape":"rectangle",
                      "corner_radius_percent":0,"frame_thickness":0,
                      "show_frame":false,"show_shadow":false},
            "layout": {"type":"map","direction":"auto","spacing":0},
            "folded": false, "notes": "",
            "color_schema": null
        }},
        "edges": []
    }"##;

    /// The load result for a map the loader rejects, produced by the
    /// loader itself.
    fn rejected() -> (String, Result<MindMapDocument, String>) {
        let result = MindMapDocument::from_json_str(ZERO_SECTION_MAP, None);
        let message = result
            .as_ref()
            .err()
            .cloned()
            .expect("the zero-section fixture must be rejected");
        (message, result)
    }

    /// The exact body `wasm_startup_document` is allowed to have,
    /// whitespace-flattened.
    ///
    /// Equality, not `contains`. This is the browser's *whole* entry
    /// point, and the reason the module is shaped the way it is: the
    /// privacy of [`StartupSurface`] stops an init site interposing,
    /// but nothing stopped these two lines from doing it themselves.
    /// `fetch_map_json(url).await.map_err(|_| String::new())` and
    /// `startup_surface("", …)` each left 13/13 green and
    /// `cargo check --target wasm32-unknown-unknown` clean while the
    /// browser got a blank diagnosis and no URL. Two statements is
    /// small enough that "these two and nothing else" is a statement
    /// a test can make.
    const WASM_ENTRY_BODY: &str = "{ let fetched = fetch_map_json(url).await; \
                                   adopt(startup_surface(url, browser_load(url, fetched))) }";

    /// `code` with every run of whitespace collapsed to one space.
    fn flatten(code: &str) -> String {
        code.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The body of `item` in this module's production code, or a
    /// panic naming it — a pin whose subject has moved has found
    /// something, and must not pass quietly.
    fn body_of(item: &str) -> String {
        let code = production_code(THIS_FILE);
        let body = braced_block_after(&code, item)
            .unwrap_or_else(|| panic!("{THIS_FILE} must still declare `{item}`"));
        flatten(body)
    }

    /// The messages `fetch_map_json` can return, **read out of its
    /// body** rather than restated here.
    ///
    /// Every string literal in that function is one of its `Err`
    /// values; it has no other literals. Deriving them is the point:
    /// the list was documented as five and tested as six while the
    /// code produced seven, and all three numbers looked checked. A
    /// list a test writes down is a list that goes stale the next
    /// time the function grows an arm.
    ///
    /// Each comes back with its format placeholders filled by a
    /// stand-in, so `"HTTP {} {}"` reads as a message rather than as
    /// a template — what a reader of the log would see.
    fn fetch_error_messages() -> Vec<String> {
        let body = body_of("async fn fetch_map_json(");
        let literals = string_literals(&body);
        assert!(
            !literals.is_empty(),
            "no literals found in `fetch_map_json` — its messages are gone, or the reader is"
        );
        literals
            .into_iter()
            .map(|literal| {
                assert!(
                    !literal.trim().is_empty(),
                    "`fetch_map_json` returns a blank diagnosis — the browser then reports \
                     `startup: could not load '<url>': ` and the placard says nothing"
                );
                let mut filled = String::with_capacity(literal.len());
                let mut rest = literal;
                while let Some(open) = rest.find('{') {
                    let close = rest[open..]
                        .find('}')
                        .unwrap_or_else(|| panic!("unclosed format placeholder in {literal:?}"));
                    filled.push_str(&rest[..open]);
                    filled.push_str("JsValue(TypeError)");
                    rest = &rest[open + close + 1..];
                }
                filled.push_str(rest);
                filled
            })
            .collect()
    }

    /// **The browser's entry point hands the URL and the loader's
    /// words straight through.**
    ///
    /// The module docs used to claim this residue was pinned by
    /// `test_both_targets_go_through_one_startup_call`. It was not —
    /// that test reads the two *init* files and never opens this one,
    /// so the privacy argument covered the init sites and nothing
    /// covered the module's own wasm-gated body. It is the one place
    /// `cargo test` cannot reach at runtime (`TEST_CONVENTIONS.md`
    /// §T9), and `wasm-bindgen-test` is not an option here (§T10), so
    /// it is read from the source like [`adopt`]'s log line.
    ///
    /// Both halves are pinned by equality against
    /// [`WASM_ENTRY_BODY`]: what the two statements *are*, which
    /// catches an interposed `.map_err`, a blanked `url`, or a third
    /// statement appearing between them.
    #[test]
    fn test_the_browser_entry_point_adds_nothing_of_its_own() {
        assert_eq!(
            body_of("async fn wasm_startup_document("),
            flatten(WASM_ENTRY_BODY),
            "`wasm_startup_document` is the browser's whole startup path and `cargo test` \
             never runs it. Anything in it other than the fetch and the one call into \
             `adopt` is a place the browser can report a failure differently from native \
             (CODE_CONVENTIONS §4) under a green suite"
        );
    }

    /// **Every message the fetch can return is a real diagnosis, and
    /// none of them names the URL.**
    ///
    /// The second half is why [`report_line`] exists, and it is
    /// checked rather than asserted in prose: if a fetch message ever
    /// did name the URL, the log line would say it twice.
    ///
    /// `fetch_map_json` is browser-only, so this reads its body —
    /// [`fetch_error_messages`] derives the list — and then the two
    /// tests below run that derived list through the code that
    /// carries it.
    #[test]
    fn test_the_fetch_returns_a_real_diagnosis_that_never_names_the_url() {
        let body = body_of("async fn fetch_map_json(");
        let messages = fetch_error_messages();
        assert!(
            messages.len() >= 7,
            "`fetch_map_json` now returns {} distinct messages, fewer than the seven it had \
             — an arm has stopped saying what went wrong. Seen: {messages:#?}",
            messages.len()
        );
        for message in &messages {
            assert!(
                !message.contains("url"),
                "{message:?} names the URL, and `report_line` puts the URL in front of it — \
                 the log line would then carry it twice"
            );
        }
        // The URL reaches the network and nothing else. A second
        // mention would be the message that names it.
        assert_eq!(
            body.matches("url").count(),
            1,
            "`fetch_map_json` mentions `url` more than once; the only use it has for it is \
             `fetch_with_str(url)`. Body seen: {body}"
        );
        assert!(
            body.contains("fetch_with_str(url)"),
            "`fetch_map_json` no longer fetches the URL it was given: {body}"
        );
    }

    /// A load that succeeded is installed untouched — no placard, no
    /// rewriting of the document, and the file binding survives so
    /// `Ctrl+S` still knows where to write.
    #[test]
    fn test_successful_load_is_adopted_unchanged() {
        let path = crate::application::document::tests_common::test_map_path();
        let doc = MindMapDocument::load(&path.to_string_lossy()).expect("the testament fixture loads");
        let name = doc.mindmap.name.clone();
        let nodes = doc.mindmap.nodes.len();

        let surface = startup_surface("maps/testament.mindmap.json", Ok(doc));
        assert!(matches!(surface, StartupSurface::Loaded(_)));
        let (adopted, report) = resolve(surface);
        assert!(report.is_none(), "a successful load reports nothing: {report:?}");
        assert_eq!(adopted.mindmap.name, name);
        assert_eq!(adopted.mindmap.nodes.len(), nodes);
        assert!(adopted.file_path.is_some(), "a real load stays bound to its file");
    }

    /// A rejected load reports the loader's message verbatim, under
    /// §9's `"<area>: message"` prefix idiom once [`adopt`] stamps
    /// it. The canvas is the surface #107 is about, but the log line
    /// is what a terminal user and a bug report still get, so it is
    /// pinned to the same words rather than left to drift.
    #[test]
    fn test_rejected_load_reports_the_loader_message_verbatim() {
        let (message, result) = rejected();
        let (_, report) = resolve(startup_surface("broken.mindmap.json", result));
        let report = report.expect("a rejected load must report");
        assert!(
            report.contains(&message),
            "the reported line must carry the loader's message, not a paraphrase.\n\
             wanted to find: {message}\ngot: {report}"
        );
    }

    /// **The reported line names the map.** The loader names the file
    /// in exactly one of its failure modes — the *read* error — and
    /// in none of the others: a parse failure says `Failed to parse
    /// mindmap JSON: …` and the browser's fetch says `HTTP 404 Not
    /// Found`, neither of which tells a reader with more than one map
    /// open which one died. The placard carries the source on the
    /// canvas; this pins the copy that goes to `stderr` and the
    /// browser console, which is where a bug report is pasted from.
    ///
    /// Runtime, not a source scan: [`report_line`] is pure and
    /// compiled on both targets, so the guarantee is executed rather
    /// than read.
    #[test]
    fn test_the_reported_line_names_the_map_that_failed() {
        // The parse failure is the case that regressed: its message
        // is produced by `load_from_str`, which never sees a path.
        let (message, result) = rejected();
        assert!(
            !message.contains("precious"),
            "fixture assumption broken — the loader message must not already name the source: {message}"
        );
        let (_, report) = resolve(startup_surface("/home/user/precious.mindmap.json", result));
        let report = report.expect("a rejected load must report");
        assert!(
            report.contains("/home/user/precious.mindmap.json"),
            "the reported line must name the map that failed: {report}"
        );
        assert!(
            report.contains(&message),
            "…without dropping the loader's own words: {report}"
        );

        // Every shape the browser's fetch can produce is a bare
        // diagnosis with no URL in it. Same guarantee, same line —
        // and the list is read out of `fetch_map_json` rather than
        // copied, so an arm added there is covered here without
        // anyone remembering to add it.
        for bare in fetch_error_messages() {
            let line = report_line("maps/testament.mindmap.json", &bare);
            assert!(
                line.contains("maps/testament.mindmap.json") && line.contains(&bare),
                "{bare:?} must reach the log with its source attached, got {line:?}"
            );
        }
    }

    /// **The source appears once, in both failure shapes and on both
    /// surfaces.**
    ///
    /// The loader names the file in exactly one of its failure modes,
    /// and that one — `load_from_file`'s read error — is the mode a
    /// mistyped path produces, which is the likeliest way anybody
    /// meets this screen at all. Prepending unconditionally therefore
    /// duplicated the path on the commonest line the fix emits:
    ///
    /// ```text
    /// startup: could not load '/nope/absent.mindmap.json': Failed to
    /// read file /nope/absent.mindmap.json: No such file or directory
    /// ```
    ///
    /// Both shapes are checked, in both directions, because a fix
    /// that stopped naming the source in the *parse* case would be
    /// the regression this whole module exists to prevent — and so is
    /// the canvas, which joins the same two strings its own way
    /// through the same predicate.
    #[test]
    fn test_the_source_is_named_once_whichever_way_the_load_failed() {
        // Shape 1: the read failure, produced by the real loader
        // against a path that is not there. Its message already names
        // the file.
        let missing = "/nonexistent/definitely-not-here.mindmap.json";
        let read_error = MindMapDocument::load(missing)
            .err()
            .expect("a path that is not there must not load");
        assert!(
            read_error.contains(missing),
            "fixture assumption broken — `load_from_file`'s read error used to name the \
             file, and this test is about what happens when it does: {read_error}"
        );

        // Shape 2: the parse failure, produced by the real loader
        // against a body that arrived. Its message names nothing.
        let (parse_error, _) = rejected();
        assert!(
            !parse_error.contains(missing),
            "fixture assumption broken — the parse message must not name a source: {parse_error}"
        );

        for (shape, message) in [("read", &read_error), ("parse", &parse_error)] {
            let line = report_line(missing, message);
            assert_eq!(
                line.matches(missing).count(),
                1,
                "the {shape} failure names {missing} {} time(s) in one log line. \
                 `report_line` adds the source because most loader messages do not \
                 carry it — but adding it to one that does reads as a stutter on the \
                 line a bug report is pasted from. Line: {line}",
                line.matches(missing).count()
            );
            assert!(
                line.contains(message.as_str()),
                "the {shape} failure lost the loader's own words: {line}"
            );

            let canvas = placard::load_failure_text(missing, message);
            assert_eq!(
                canvas.replace('\n', " ").matches(missing).count(),
                1,
                "the {shape} failure names {missing} more than once on the canvas:\n{canvas}"
            );
            assert!(
                canvas.contains(placard::PLACARD_HEADLINE) && canvas.contains(placard::PLACARD_FOOTER),
                "the {shape} placard lost its frame:\n{canvas}"
            );
        }
    }

    /// **A source too short to be a path of its own still reaches
    /// both surfaces.**
    ///
    /// The pin above uses a long absolute path, and so did every
    /// other one here — which is how the "once" rule shipped with a
    /// hole in it. `mandala map` on a malformed file called `map`
    /// draws the loader's `Failed to parse mindmap JSON: …`, and the
    /// old predicate found `map` inside the word **mindmap**. Both
    /// surfaces concluded the message had already named the file and
    /// dropped it, so neither said which file failed: the empty
    /// screen #107 is about, reached by a shorter route than #107's.
    ///
    /// Short sources are not exotic — a map in the working directory
    /// is typed as its bare name. The predicate lives in
    /// `baumhard::mindmap::placard`, which pins the decision itself
    /// across a dozen spellings; this pins the two surfaces that ask
    /// it, because a predicate can be right while a caller drops its
    /// answer on the floor.
    #[test]
    fn test_a_short_source_reaches_both_surfaces() {
        let short = "map";
        let message = MindMapDocument::from_json_str("map", None)
            .err()
            .expect("a bare word is not a mindmap and must not load");
        assert!(
            message.contains(short),
            "fixture assumption broken — this test is about a message that contains the \
             source as a fragment rather than as a name: {message}"
        );

        let line = report_line(short, &message);
        assert!(
            line.contains(&format!("'{short}'")) && line.contains(&message),
            "the log line for a short source must name it and keep the loader's words, \
             got {line:?}"
        );

        let canvas = placard::load_failure_text(short, &message);
        assert!(
            canvas.lines().any(|line| line == short),
            "the placard for a short source dropped it, leaving nothing on the canvas \
             that says which file failed:\n{canvas}"
        );
    }

    /// **The browser leg keeps the loader's words.** Blanking the
    /// message on that leg alone used to be a one-line edit at the
    /// init site that no test could see, because `cargo test` runs
    /// the native leg only (TEST_CONVENTIONS §T9).
    ///
    /// [`browser_load`] is the whole of that leg apart from the
    /// `await`, and it is target-independent precisely so this can be
    /// *executed* rather than scanned: every string the fetch can
    /// return — the list [`fetch_error_messages`] reads out of
    /// `fetch_map_json` — and a parse failure of a body that did
    /// arrive, must come out the far end intact.
    #[test]
    fn test_the_browser_leg_carries_every_fetch_message_through() {
        const URL: &str = "maps/from-the-query-string.mindmap.json";
        for bare in fetch_error_messages() {
            let (_, report) = resolve(startup_surface(URL, browser_load(URL, Err(bare.clone()))));
            let report = report.expect("a failed fetch must report");
            assert!(
                report.contains(&bare) && report.contains(URL),
                "the browser leg lost {bare:?} or its URL: {report}"
            );
        }

        // A body that arrived and did not parse: the loader's own
        // message, not the fetch's.
        let (parse_message, _) = rejected();
        let (_, report) = resolve(startup_surface(
            URL,
            browser_load(URL, Ok(ZERO_SECTION_MAP.to_string())),
        ));
        let report = report.expect("a rejected body must report");
        assert!(
            report.contains(&parse_message) && report.contains(URL),
            "the browser leg lost the parse message or its URL: {report}"
        );

        // A body that *did* parse keeps its origin, so the browser's
        // save-back knows where the document came from.
        let good = std::fs::read_to_string(crate::application::document::tests_common::test_map_path())
            .expect("the testament fixture is readable");
        let doc = browser_load(URL, Ok(good)).expect("the testament fixture parses");
        assert_eq!(doc.file_path.as_deref(), Some(URL));
    }

    /// **The fix, stated as a test.** A rejected load does not leave
    /// the shell with nothing to render: it gets a document, and
    /// that document carries the loader's own words.
    #[test]
    fn test_rejected_load_puts_the_loader_message_on_the_canvas() {
        let (message, result) = rejected();
        let doc = adopt(startup_surface("broken.mindmap.json", result));

        let text = doc
            .mindmap
            .nodes
            .values()
            .map(|n| {
                n.sections
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.trim().is_empty(), "the placard must not be blank");

        // The loader's message, not a paraphrase of it. Compared
        // word-by-word because the placard wraps to a column.
        let on_canvas = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let expected = message.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            on_canvas.contains(&expected),
            "the placard does not carry the loader message.\nwanted: {expected}\ngot: {text}"
        );
        assert!(
            on_canvas.contains("broken.mindmap.json"),
            "the placard must name the map that failed: {text}"
        );
    }

    /// **The native entry point, end to end.** Everything above tests
    /// a piece; this runs the function `run_native_init` actually
    /// calls, on a path that does not exist, and checks the shell
    /// gets something to render.
    #[test]
    fn test_native_startup_document_returns_a_placard_for_a_map_that_is_not_there() {
        let missing = "/nonexistent/definitely-not-here.mindmap.json";
        let doc = native_startup_document(missing);
        assert_eq!(doc.mindmap.name, placard::PLACARD_MAP_NAME);
        assert!(doc.file_path.is_none(), "a placard must never be savable");
        let text = &doc.mindmap.nodes[placard::PLACARD_NODE_ID].sections[0].text;
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(missing), "the placard must name the map: {text}");
    }

    /// The placard is bound to no file, so the save paths — which
    /// both refuse a document without one — cannot write it over the
    /// file that failed to parse. That file is unparseable, which
    /// means it is very likely the only copy of whatever the author
    /// was hand-editing.
    #[test]
    fn test_placard_is_not_bound_to_the_file_that_failed() {
        let (_, result) = rejected();
        let doc = adopt(startup_surface("/home/user/precious.mindmap.json", result));
        assert!(doc.file_path.is_none(), "a placard must never be savable");
        assert!(!doc.dirty, "a placard is not unsaved work");
        assert_eq!(doc.mindmap.name, placard::PLACARD_MAP_NAME);
    }

    /// The placard is a live document, not a still image: the shell
    /// installs it through the same path as a real map, so the tree
    /// projection has to succeed on it. A placard that could not
    /// build a tree would render the same empty canvas #107 is
    /// about.
    #[test]
    fn test_placard_projects_into_a_renderable_tree() {
        let (_, result) = rejected();
        let doc = adopt(startup_surface("broken.mindmap.json", result));
        let tree = doc.build_tree();
        assert_eq!(tree.node_count(), 1, "the placard must project its one node");
        assert_eq!(
            tree.section_count_for(placard::PLACARD_NODE_ID),
            1,
            "the placard's text must project as a renderable section"
        );
    }

    /// **A placard is not announced as a map that loaded.** Under
    /// `RUST_LOG=info` the shell used to print
    /// `document: loaded mindmap "load-failure" with 1 node(s)`
    /// immediately beside `startup: could not load …`, because
    /// `MindMapDocument::finalize` carried the announcement and the
    /// placard goes through `finalize` like any other map. A log
    /// that contradicts itself in two consecutive lines is worse
    /// than a quiet one.
    ///
    /// The same shape as the pin above, for the same reason — an
    /// `info!` has no sink the suite installs — and the same two
    /// scopings: [`production_code`] strips the comments that quote
    /// these names, and [`braced_block_after`] asks the question of
    /// one function body at a time. Both directions are checked, so
    /// deleting the announcement outright fails here too.
    #[test]
    fn test_the_placard_is_not_announced_as_a_loaded_map() {
        const DOCUMENT: &str = "src/application/document/mod.rs";
        let code = production_code(DOCUMENT);
        // A file whose test modules are all external: if the reader
        // ever truncates at one of those declarations it hands back
        // nothing, and every assertion below passes vacuously.
        assert!(
            code.contains("fn finalize("),
            "{DOCUMENT} did not reach this test — the reader truncated it"
        );

        for (item, announces) in [
            ("pub fn load(", true),
            ("pub fn from_json_str(", true),
            ("pub fn load_failure_placard(", false),
            ("fn finalize(", false),
        ] {
            let body = braced_block_after(&code, item)
                .unwrap_or_else(|| panic!("{DOCUMENT} must still declare `{item}`"));
            assert_eq!(
                body.contains("log_loaded("),
                announces,
                "`{item}` {} announce the load. The line belongs to the constructors that \
                 really loaded a map — not to `finalize`, which the load-failure placard also \
                 goes through. Body seen: {body}",
                if announces { "must" } else { "must not" }
            );
        }
    }

    /// **[`adopt`] still logs.** The placard is an addition to the
    /// stderr line, not a replacement for it — a terminal user and a
    /// bug report both want the line, and removing it would make
    /// this change a net loss for anyone who *does* have a terminal.
    ///
    /// Deleting that one statement is the only mutation of this
    /// module a runtime assertion cannot see: `log::error!` writes
    /// through a process-global logger the suite does not install,
    /// and standing one up would be a second logging sink for the
    /// tests to keep in step (TEST_CONVENTIONS §T10).
    ///
    /// Scoped twice, by [`production_code`] and
    /// [`braced_block_after`]. Comments are gone, so gutting `adopt`
    /// and leaving `// this used to be log::error!(…)` fails — that
    /// is not a contrived mutation, it is the ordinary shape of a
    /// deletion. Test modules are gone — including the ones written
    /// under a run of attributes, which is the shape that let a shim
    /// supply a needle for a while — so the needle spelled out in
    /// this very body cannot satisfy the scan. And the search is the
    /// body of `adopt` alone, so moving the statement out of the
    /// function it belongs to fails too. Whitespace-flattened so a
    /// re-wrap does not fail it; the prefix is asserted with it,
    /// because §9 requires `"<area>: message"` and a bare message is
    /// what the old `run_native_init` arm emitted.
    #[test]
    fn test_adopt_still_reports_the_message_to_the_log() {
        let code = production_code(THIS_FILE);
        let body = braced_block_after(&code, "fn adopt(")
            .unwrap_or_else(|| panic!("{THIS_FILE} must still declare `fn adopt(`"));
        let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(r#"log::error!("startup: {}", message);"#),
            "`adopt` must still write the report to the log under the \
             `\"<area>: message\"` prefix — the placard is an addition to that line, \
             not a replacement for it. Body seen: {flat}"
        );
    }

    /// **Both targets reach the canvas through one call, and hand it
    /// nothing but the source.**
    ///
    /// CODE_CONVENTIONS §4 makes the two targets peers, and a
    /// startup-error surface that exists on one of them is not a fix.
    /// Runtime tests cannot see this: `cargo test` runs the native
    /// leg only (TEST_CONVENTIONS §T9), so the browser arm would be
    /// free to drift under a green suite.
    ///
    /// What is *not* asserted here, because the compiler asserts it:
    /// `StartupSurface`, `startup_surface` and `adopt` are private to
    /// this module, so no init site can build a surface, take one
    /// apart, or interpose between the loader and the log. That used
    /// to be a `!source.contains("StartupSurface::")` and it is now a
    /// visibility.
    ///
    /// What is left to check is the *argument*, and it is checked
    /// exactly: each init file must contain its entry call spelled
    /// with the source and nothing else. `.map_err(…)` on the way in
    /// — the mutation that blanked the browser's diagnosis while
    /// every test stayed green — changes that text and fails here.
    /// The scan reads [`production_code`], so neither a comment nor
    /// an appended test module can supply the needle.
    #[test]
    fn test_both_targets_go_through_one_startup_call() {
        for (relative, call) in INIT_FILES {
            let code = production_code(relative);
            let flat = code.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                flat.contains(call),
                "{relative} must install its document with exactly `{call}` — anything \
                 between the loader and `startup_load` is a place the two targets can \
                 report the same failure differently"
            );
        }

        // The corollary: neither init file still loads for itself.
        // A second, unrouted load is how the old arm would come back.
        let native = production_code(INIT_FILES[0].0);
        assert!(
            !native.contains("MindMapDocument::load("),
            "run_native_init must not load the map itself; `native_startup_document` does"
        );
        let wasm = production_code(INIT_FILES[1].0);
        for own_work in ["from_json_str(", "fetch_map_json("] {
            assert!(
                !wasm.contains(own_work),
                "run_wasm must not do the load itself ({own_work}); \
                 `wasm_startup_document` does"
            );
        }
    }

    /// The files startup runs through — §9's phase list, resolved,
    /// with the phrase each one answers.
    ///
    /// This is what makes the roster's *third* direction possible.
    /// §9 ⊆ [`STARTUP_SITES`] catches a stale name; roster ⊆ code
    /// catches a rename; neither catches a fallible startup site that
    /// is simply never named, which is how `request_adapter` and
    /// `request_device` came to `expect` on every launch of both
    /// targets, unlisted, for as long as this list did not exist.
    ///
    /// ## The residual, and why it is a list rather than a derivation
    ///
    /// §9's phase list is prose, and resolving it to files is done
    /// here by hand. A startup file nobody adds is out of reach of
    /// every test below — but a panicking call added to one that *is*
    /// here is not, and neither is a roster entry pointing at a file
    /// that is not on the list.
    ///
    /// **That residual is not hypothetical, and saying it was cost
    /// two live files.** Round 3 disclosed the missing file as
    /// something nobody had written yet. Two already existed: the
    /// browser's `Application::new` in `src/application/app/mod.rs`,
    /// which creates the event loop and the window with two
    /// `.expect(`s that the roster attributed to `run_native.rs`; and
    /// `fonts.rs`'s family-index build, which §9's phase list names
    /// as `fonts::init` and which nothing here resolved to a file.
    /// Turning the browser's `create_window` `.expect(` into a bare
    /// `unwrap()` left 17/17 green. Both are on the list now.
    ///
    /// **A third was found the same way, and it was this round's own
    /// edit.** `loader.rs` is where `MindMapDocument::load` ends up,
    /// so it is as much of the initial map load as this file is — and
    /// the cycle reporter in it was rewritten *for* §9, its
    /// `position(…).unwrap_or(0)` chosen over an `unwrap()` because
    /// the load path may not panic. Restoring that `unwrap()` left
    /// the sweep green, because the file it lives in was not on this
    /// list. Which is the shape to watch for: the residual is not
    /// only "a file nobody thought of", it is "a file somebody
    /// deliberately made panic-free without adding it here", and the
    /// second is the more likely of the two.
    ///
    /// **Can it be derived instead?** Not by anything available here,
    /// and the reasons are worth writing down so the question is not
    /// re-opened by guesswork:
    ///
    /// - The set wanted is "files reachable from `main` before the
    ///   first frame". That is a call graph across two crates and two
    ///   `cfg` worlds. [`production_code`] is a text scanner; it
    ///   cannot resolve a name to a definition, so it cannot walk one.
    /// - Dropping the startup restriction and sweeping the workspace
    ///   for bare `unwrap()` does not work either, and not for a small
    ///   reason: baumhard declares its test trees as plain `pub mod
    ///   tests;` on purpose (`lib/baumhard/CONVENTIONS.md` §B8, so
    ///   criterion can reuse the bodies), so they carry no `cfg(test)`
    ///   for [`above_test_modules`] to cut at. A sweep of every `.rs`
    ///   file counts **1680** bare `unwrap()`s in 66 files, essentially
    ///   all of them in tests that are entitled to them. Deriving the
    ///   startup set would first require deriving the test set, which
    ///   §B8 has deliberately made underivable.
    ///
    /// So it is a list, maintained by inspection, and its weakness is
    /// stated rather than engineered away. What holds it: the three
    /// directions below, and the fact that a file's *absence* is the
    /// only thing they cannot see.
    const STARTUP_FILES: &[(&str, &str)] = &[
        ("src/main.rs", "the CLI parse"),
        (
            "src/application/app/mod.rs",
            "`Application::new` — the browser's event loop and window",
        ),
        (
            "src/application/app/run_native.rs",
            "the winit event loop and its window",
        ),
        (
            "src/application/app/run_native_init.rs",
            "`fonts::init` and the native bootstrap",
        ),
        (
            "src/application/app/run_wasm/mod.rs",
            "the page attachment, the `?map=` read and the browser bootstrap",
        ),
        (
            "src/application/ipc/transport/mod.rs",
            "`IpcServer::start` — the `--ipc` listener, which reports every failure as a \
             message for its caller's `expect` and must therefore panic nowhere",
        ),
        (
            "src/application/ipc/transport/socket.rs",
            "the `--ipc` socket's bind sequence, inside the same promise",
        ),
        ("src/application/renderer/mod.rs", "the renderer bootstrap"),
        (
            "src/application/renderer/pipeline.rs",
            "the adapter and the device that bootstrap asks wgpu for",
        ),
        (
            "lib/baumhard/src/font/fonts.rs",
            "what `fonts::init` reaches — the font-system lock and the family index",
        ),
        (
            "src/application/app/startup_load.rs",
            "the initial map load — §9's carve-out, which must therefore panic nowhere",
        ),
        (
            "lib/baumhard/src/mindmap/loader.rs",
            "what the initial map load reaches — the parse and the validators behind \
             `MindMapDocument::load`, inside the same carve-out",
        ),
    ];

    /// Where §9's startup roster must resolve to, and whether every
    /// call of it in that file carries an `.expect(` — which is a
    /// claim about the code, and so is checked against the code.
    ///
    /// The `reason` column is for the `false` rows, because "no
    /// `expect` here" has three different causes and a failure
    /// message that conflated them would send a reader the wrong way.
    ///
    /// The list exists because §9's used to name `Renderer::new`,
    /// which the bootstrap split replaced, and `fonts::init`, which
    /// returns `()` and has no failure to report. Both survived
    /// review because nothing checked them. It grew its second half
    /// because nothing checked the other direction either: every row
    /// below `create_surface` was a live `.expect(` on the startup
    /// path that §9 did not mention.
    const STARTUP_SITES: &[(&str, &str, bool, &str)] = &[
        // Native builds its loop with a user-event type — the IPC wake —
        // which winit only offers through the builder.
        (
            "EventLoop::with_user_event",
            "src/application/app/run_native.rs",
            true,
            "",
        ),
        ("create_window", "src/application/app/run_native.rs", true, ""),
        ("IpcServer::start", "src/application/app/run_native.rs", true, ""),
        // The browser creates both in `Application::new` rather than
        // in `run_wasm`, so the same two names need a row per file —
        // an entry keyed only by name reads as covered while the
        // browser's copy is not.
        ("EventLoop::new", "src/application/app/mod.rs", true, ""),
        ("create_window", "src/application/app/mod.rs", true, ""),
        ("run_app", "src/application/app/run_native.rs", true, ""),
        ("create_surface", "src/application/renderer/mod.rs", true, ""),
        (
            "request_adapter",
            "src/application/renderer/pipeline.rs",
            true,
            "",
        ),
        ("request_device", "src/application/renderer/pipeline.rs", true, ""),
        ("web_sys::window", "src/application/app/run_wasm/mod.rs", true, ""),
        ("canvas", "src/application/app/run_wasm/mod.rs", true, ""),
        ("document", "src/application/app/run_wasm/mod.rs", true, ""),
        ("body", "src/application/app/run_wasm/mod.rs", true, ""),
        ("append_child", "src/application/app/run_wasm/mod.rs", true, ""),
        ("inner_width", "src/application/app/run_wasm/mod.rs", true, ""),
        ("inner_height", "src/application/app/run_wasm/mod.rs", true, ""),
        (
            "Renderer::bootstrap_native",
            "src/application/app/run_native_init.rs",
            false,
            "the failure is one level in, at `create_surface`",
        ),
        (
            "Renderer::bootstrap_wasm",
            "src/application/app/run_wasm/mod.rs",
            false,
            "the failure is one level in, at `create_surface`",
        ),
        (
            "fonts::init",
            "src/application/app/run_native_init.rs",
            false,
            "it returns `()` and has nothing to report",
        ),
        (
            "strip_prefix",
            "src/application/app/run_wasm/mod.rs",
            false,
            "the query-string extractor is infallible by construction",
        ),
        (
            "Application::new",
            "src/main.rs",
            false,
            "the failures are one level in, at the browser's `EventLoop::new` and \
             `create_window`; the native arm of it cannot fail at all",
        ),
        (
            "acquire_font_system_read",
            "lib/baumhard/src/font/fonts.rs",
            false,
            "it returns a guard, not a `Result`; its failure is the helper's own \
             `panic!`, which is the next row",
        ),
        // The one roster name that is a macro rather than a call.
        // `acquire_font_system_write_with_timeout` is where the
        // decision lives, but the statement that ends the process is
        // the `panic!` itself, and the sweep matches on the statement
        // — so the site is spelled the way the code spells it.
        ("panic!", "lib/baumhard/src/font/fonts.rs", true, ""),
        (
            "web_sys::window",
            "src/application/app/startup_load.rs",
            false,
            "the map fetch's second ask degrades into the placard's message rather \
             than aborting — §9's carve-out bullet, and the reason this name needs a \
             row per file",
        ),
    ];

    /// The three spellings of "this statement can end the process",
    /// as they appear in source. `.unwrap_or…` is deliberately not
    /// one of them: it is the opposite of a panic.
    ///
    /// Index panics — `formats[0]`, `rgba[0..3]` — are not spellings
    /// and are not here. They end the process just as thoroughly;
    /// recognizing them means telling a slice index from a tuple
    /// field from a generic argument in text, which this reader does
    /// not attempt. Named so the gap is a decision rather than an
    /// oversight.
    const PANICKING: &[&str] = &[".expect(", ".unwrap()", "panic!("];

    /// **Every** spelling in [`PANICKING`] that `statement` carries,
    /// in [`PANICKING`] order.
    ///
    /// All of them, not the first: `PANICKING.iter().find(…)` returns
    /// index 0, `.expect(`, for a statement carrying both an
    /// `.expect(` and a bare `unwrap()`, so the `unwrap()` was never
    /// looked at and
    ///
    /// ```text
    /// required_limits: …using_resolution(Some(adapter.limits()).unwrap()),
    /// ```
    ///
    /// left the suite green. That is the same defect as the `;`-split
    /// this module already fixed, one level in: the chunk was right,
    /// and the choice *within* the chunk was not. A statement is a
    /// unit of grouping, never a unit of "one panic".
    fn panicking_spellings_in(statement: &str) -> Vec<&'static str> {
        PANICKING
            .iter()
            .copied()
            .filter(|spelling| statement.contains(spelling))
            .collect()
    }

    /// Backticked words in the `expect` bullet that are Rust
    /// *vocabulary* rather than startup sites — they are what the
    /// rule is about, or the type it is stated in, not entries in
    /// its list.
    const NOT_A_SITE: &[&str] = &["expect", "unwrap", "Err", "Result"];

    /// The bold lead of §9's `expect` bullet, verbatim.
    const EXPECT_LEAD: &str = "**Startup paths use `expect(\"<reason>\")` with a human-readable message.**";

    /// The bold lead of §9's carve-out bullet, verbatim. This is the
    /// sentence that states the direction of the decision, and it is
    /// pinned by equality rather than by `contains` for that reason.
    const CARVE_OUT_LEAD: &str =
        "**The initial map load is the one startup path that does not `expect`, and the reason names the boundary.**";

    /// Split a Markdown section into its top-level `- ` bullets, each
    /// whitespace-flattened with its continuation lines folded in.
    fn bullets(section: &str) -> Vec<String> {
        let mut raw: Vec<String> = Vec::new();
        for line in section.lines() {
            if let Some(rest) = line.strip_prefix("- ") {
                raw.push(rest.to_string());
            } else if let Some(last) = raw.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        }
        raw.iter()
            .map(|b| b.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// The one bullet of `section` whose flattened text starts with
    /// `lead`. Panics naming both, because a caller that cannot find
    /// its bullet has found the drift it was looking for.
    fn bullet_led_by<'a>(bullets: &'a [String], lead: &str) -> &'a str {
        bullets
            .iter()
            .find(|b| b.starts_with(lead))
            .map(String::as_str)
            .unwrap_or_else(|| {
                panic!(
                    "CODE_CONVENTIONS.md §9 no longer opens a bullet with:\n  {lead}\nbullet leads seen:\n{}",
                    bullets
                        .iter()
                        .map(|b| format!("  {}", b.split(".**").next().unwrap_or(b)))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
    }

    /// Every backticked span in `text`.
    fn backticked(text: &str) -> Vec<&str> {
        text.split('`').skip(1).step_by(2).collect()
    }

    /// Whether `token` is a bare Rust identifier or `::` path — the
    /// shape a symbol reference takes. `expect("<reason>")` and
    /// `?map=` are not.
    fn is_path_shaped(token: &str) -> bool {
        !token.is_empty()
            && token.split("::").all(|seg| {
                !seg.is_empty()
                    && seg.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
    }

    /// Whether `statement` *calls* `name`: the name followed by `(`,
    /// with whatever precedes it not an identifier character.
    ///
    /// The boundary is the whole point. A bare
    /// `statement.contains("document(")` also matches
    /// `wasm_startup_document(`, and a bare `create_surface(` also
    /// matches `create_surface_config(` — which is infallible, and
    /// would answer "does every call of this carry an `.expect(`"
    /// with the wrong answer.
    ///
    /// Whitespace around a `.` is closed up first, so a roster name
    /// may be a receiver plus a method — `FONT_SYSTEM.read` — and
    /// still match after rustfmt has broken the chain over three
    /// lines. Rust does not care where the newline falls and neither
    /// should the roster: the alternative is either a bare `read`,
    /// which says nothing about *which* lock, or production code
    /// squeezed onto one line to suit a test.
    fn statement_calls(statement: &str, name: &str) -> bool {
        let statement = statement.replace(" .", ".").replace(". ", ".");
        let needle = format!("{name}(");
        let mut from = 0usize;
        while let Some(offset) = statement[from..].find(&needle) {
            let at = from + offset;
            let preceded_by_identifier = statement[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !preceded_by_identifier {
                return true;
            }
            from = at + 1;
        }
        false
    }

    /// [`rust_source::statements`], each piece whitespace-flattened
    /// so a re-wrap cannot change the answer.
    ///
    /// The split used to be `code.split(';')` here. That merges every
    /// expression-bodied `fn` in a file with its neighbors — such a
    /// body ends in `}`, not `;` — and `renderer/pipeline.rs` has two
    /// of them side by side, so turning `request_device`'s `.expect(`
    /// into a bare `unwrap()` left the chunk carrying
    /// `request_adapter`'s `.expect(` and every pin below stayed
    /// green.
    fn statements(code: &str) -> Vec<String> {
        rust_source::statements(code).into_iter().map(flatten).collect()
    }

    /// **§9's `expect` roster names things that exist, and only
    /// things that exist.**
    ///
    /// The bullet used to enumerate `Renderer::new` — replaced by
    /// `bootstrap_native` / `bootstrap_wasm` — and `fonts::init`,
    /// which returns `()` and so has nothing to `expect` on. Neither
    /// was caught, because a convention that lists code is only as
    /// accurate as the last person who checked, and nobody had.
    ///
    /// Three checks, in the order they fail:
    ///
    /// 1. every path-shaped name the bullet backticks is in
    ///    [`STARTUP_SITES`] — so adding a stale one fails here rather
    ///    than surviving a decade;
    /// 2. every roster entry is *called* in the file the roster names
    ///    — so a rename fails;
    /// 3. every call of an entry the roster marks as `expect`ing sits
    ///    in a statement that also carries an `.expect(` — so listing
    ///    something that cannot fail fails, and so does one arm of a
    ///    two-arm site quietly dropping its reason.
    ///
    /// All three read [`production_code`], so a name that survives
    /// only in a comment does not count as surviving.
    ///
    /// **All three run §9 → roster → code, and that is only half the
    /// job.** A fallible startup site that is simply never named is
    /// invisible to every one of them;
    /// [`test_no_startup_file_panics_outside_the_roster`] is the
    /// other half.
    #[test]
    fn test_code_conventions_section_9_expect_roster_names_real_sites() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("CODE_CONVENTIONS.md"), "## §9 Error handling");
        let all = bullets(&section);
        let bullet = bullet_led_by(&all, EXPECT_LEAD);

        for token in backticked(bullet).into_iter().filter(|t| is_path_shaped(t)) {
            if NOT_A_SITE.contains(&token) {
                continue;
            }
            assert!(
                STARTUP_SITES.iter().any(|(name, _, _, _)| *name == token),
                "CODE_CONVENTIONS.md §9 names `{token}` as a startup site, but this test's \
                 roster does not know it. Add it here with the file it lives in and whether \
                 it can fail — or take it out of §9, which is what `Renderer::new` needed."
            );
        }

        for (name, relative, expects, reason) in STARTUP_SITES {
            assert!(
                bullet.contains(&format!("`{name}`")),
                "CODE_CONVENTIONS.md §9's `expect` bullet must still name `{name}`"
            );
            let code = production_code(relative);
            // The paren makes this a *call* rather than a mention:
            // without it `create_surface` also matches
            // `create_surface_config`, which is infallible and would
            // answer the next question wrongly.
            let calls: Vec<String> = statements(&code)
                .into_iter()
                .filter(|s| statement_calls(s, name))
                .collect();
            assert!(
                !calls.is_empty(),
                "§9 names `{name}` but {relative} never calls it — the convention is \
                 pointing at something that moved"
            );
            // *Every* call, not merely one of them. `create_surface`
            // is reached from both `bootstrap_native` and
            // `bootstrap_wasm`; an `any` here would let one of the
            // two stop saying why it died while §9 goes on claiming
            // both do.
            // "Says why it died", in either spelling that can. §9's
            // demand is a human-readable message, and
            // `panic!("<reason>")` carries one exactly as
            // `expect("<reason>")` does — `fonts.rs` raises its own
            // from lock helpers that hand back a guard, not a
            // `Result`, so there is nothing to `expect` on. A bare `unwrap()` carries
            // neither and still fails, here and in
            // [`test_no_startup_file_panics_outside_the_roster`].
            let all_expect = calls
                .iter()
                .all(|s| s.contains(".expect(") || s.contains("panic!("));
            assert_eq!(
                all_expect,
                *expects,
                "§9 classes `{name}` in {relative} as {}, and the code says otherwise: a \
                 site that `expect`s is one whose *every* call says why it died, with an \
                 `.expect(\"<reason>\")` or a `panic!(\"<reason>\")`.{} Calls seen: {calls:#?}",
                if *expects {
                    "a site that says why it died"
                } else {
                    "a site that does not `expect`"
                },
                if reason.is_empty() {
                    String::new()
                } else {
                    format!(" The roster's reason for the `false`: {reason}.")
                }
            );
        }
    }

    /// **§9 records the decision, in the direction the code
    /// implements it.**
    ///
    /// #107's acceptance asked for exactly this: the convention
    /// prescribed `expect` for the initial `loader::load_from_file`
    /// and the code did something else, so one of the two had to
    /// move. §9 moved — but a paragraph is only a decision for as
    /// long as it survives the next edit of the file.
    ///
    /// **What "directional" can and cannot mean here.** No test reads
    /// prose. An earlier version of this one asserted a handful of
    /// `contains` on flattened §9 text and called itself directional;
    /// it was not — a §9 rewritten to say the rule was *withdrawn*
    /// kept every needle and stayed green. Three things replace that:
    ///
    /// - the bullet's **bold lead** is pinned by equality, not
    ///   `contains`. The lead is where the rule is stated, and a
    ///   negation cannot be wrapped around a string that has to match
    ///   exactly;
    /// - the `expect` bullet must **not** name the initial load. "The
    ///   initial load now `expect`s" has to be written down
    ///   somewhere, and that list is where;
    /// - the claim itself is checked **against the code**: the two
    ///   init sites must carry no `expect`/`unwrap` on the startup
    ///   document, and the two modules §9 names must exist. A §9 that
    ///   says the opposite of the code is then a §9 that disagrees
    ///   with a green suite.
    ///
    /// The residual, stated plainly: a bullet that keeps its lead
    /// verbatim and contradicts it in its own body is out of reach of
    /// any text test, and is a self-refuting document rather than a
    /// drift. Verified to include its near variants — [`bullets`]
    /// folds a nested `  - ` sub-bullet into its parent, so
    /// contradicting the lead from a sub-bullet is the same case; and
    /// [`bullet_led_by`] takes the *first* bullet with the lead, so a
    /// second bullet opening with the same words is out of reach the
    /// same way. What is **not** residual, and is worth saying
    /// because it is the near miss: contradicting the rule by
    /// *listing the initial load* among the sites that `expect` fails
    /// below, and so does implementing the contradiction, because
    /// [`test_no_startup_file_panics_outside_the_roster`] reads
    /// `startup_load.rs` itself and no panicking call in it is
    /// roster-named.
    #[test]
    fn test_code_conventions_section_9_records_the_startup_load_decision() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("CODE_CONVENTIONS.md"), "## §9 Error handling");
        let all = bullets(&section);
        let carve_out = bullet_led_by(&all, CARVE_OUT_LEAD);
        let expect_bullet = bullet_led_by(&all, EXPECT_LEAD);

        // The `expect` list is where "the initial load now `expect`s"
        // would have to be written. It must not be.
        for excluded in [
            "load_from_file",
            "from_json_str",
            "initial map load",
            "native_startup_document",
            "wasm_startup_document",
        ] {
            assert!(
                !expect_bullet.contains(excluded),
                "CODE_CONVENTIONS.md §9's `expect` bullet names {excluded:?} — the initial map \
                 load is carved *out* of that list, and putting it back is the reversal this \
                 test exists to catch"
            );
        }

        // The carve-out has to say where it is implemented, or a
        // reader cannot get from the rule to the code.
        for named in [
            "app::startup_load",
            "baumhard::mindmap::placard",
            "keep the shell alive",
        ] {
            assert!(
                carve_out.contains(named),
                "CODE_CONVENTIONS.md §9's carve-out must name {named:?}"
            );
        }

        // …and what it names has to be there.
        for module in [
            "src/application/app/startup_load.rs",
            "lib/baumhard/src/mindmap/placard.rs",
        ] {
            assert!(
                repo_path(module).is_file(),
                "§9 points at {module}, which does not exist"
            );
        }

        // The claim, checked against the code: no init site turns a
        // rejected map into a panic. This is the half that a §9
        // rewritten to say the opposite cannot make true.
        for (relative, call) in INIT_FILES {
            let code = production_code(relative);
            let statement = statements(&code)
                .into_iter()
                .find(|s| s.contains("startup_document("))
                .unwrap_or_else(|| panic!("{relative} no longer calls a startup entry point"));
            assert!(
                statement.contains(call),
                "{relative}'s startup statement is {statement:?}, not {call:?}"
            );
            for panicking in [".expect(", ".unwrap(", "panic!("] {
                assert!(
                    !statement.contains(panicking),
                    "{relative} turns the initial load into a panic with {panicking} — §9 says \
                     it does not, and §9 is the decision this module implements: {statement}"
                );
            }
        }
    }

    /// **The third direction: no file on the startup path panics
    /// without §9 knowing.**
    ///
    /// The two checks above run §9 → roster → code. Both are
    /// one-directional, and the gap between them was invisible by
    /// construction: `request_device` and `request_adapter` in
    /// `renderer/pipeline.rs` each carry an `.expect(`, each is
    /// reached from `Renderer::new` and so from both bootstraps, and
    /// neither §9 nor the roster had ever heard of them. Turning one
    /// into a bare `unwrap()` — an explicit §9 violation — left all
    /// thirteen of this module's tests green.
    ///
    /// So: every statement in a [`STARTUP_FILES`] file that can end
    /// the process must call something the roster names as a site
    /// that `expect`s. Two consequences worth naming:
    ///
    /// - a bare `unwrap()` anywhere on the startup path fails here —
    ///   *anywhere*, including beside an `.expect(` in the same
    ///   statement, which is what [`panicking_spellings_in`] is for
    ///   and what this sentence used to overstate;
    /// - `startup_load.rs` is on the list and must therefore panic
    ///   *nowhere*, which is §9's carve-out checked from the code
    ///   side rather than restated.
    ///
    /// Reads [`production_code`], so a `.expect(` in a test module or
    /// quoted in a doc comment is not a startup panic.
    #[test]
    fn test_no_startup_file_panics_outside_the_roster() {
        // The two lists have to agree about which files exist, or a
        // roster entry could name a file this sweep never opens.
        for (name, relative, _, _) in STARTUP_SITES {
            assert!(
                STARTUP_FILES.iter().any(|(file, _)| file == relative),
                "the roster puts `{name}` in {relative}, which is not on the startup-file \
                 list — either the file belongs there, or the entry is pointing at code \
                 that startup does not run"
            );
        }

        let mut seen = 0usize;
        for (relative, phase) in STARTUP_FILES {
            let code = production_code(relative);
            for statement in statements(&code) {
                let hits = panicking_spellings_in(&statement);
                if hits.is_empty() {
                    continue;
                }
                seen += 1;
                assert!(
                    !hits.contains(&".unwrap()"),
                    "{relative} ({phase}) ends the process with a bare `unwrap()`. \
                     CODE_CONVENTIONS §9: startup panics carry `expect(\"<reason>\")`, \
                     because the message is the only thing the person holding the \
                     binary gets. Statement: {statement}"
                );
                let named = STARTUP_SITES.iter().any(|(name, file, expects, _)| {
                    file == relative && *expects && statement_calls(&statement, name)
                });
                assert!(
                    named,
                    "{relative} ({phase}) can end the process here, and neither \
                     CODE_CONVENTIONS §9 nor this roster names the call that does it. \
                     Add it to `STARTUP_SITES` and to §9's `expect` bullet — a startup \
                     site nobody has written down is one nobody has decided about. \
                     Statement: {statement}"
                );
            }
        }
        // Non-vacuity: `production_code` handing back an empty string
        // would satisfy every assertion above.
        assert!(
            seen >= 12,
            "only {seen} panicking statements were found across {} startup files — the \
             reader is not reaching them",
            STARTUP_FILES.len()
        );
    }

    /// **A statement carrying two spellings carries both of them.**
    ///
    /// The sweep above reads whatever
    /// [`panicking_spellings_in`] hands back, and for as long as that
    /// was `PANICKING.iter().find(…)` it handed back index 0 —
    /// `.expect(` — for a statement holding an `.expect(` and a bare
    /// `unwrap()` alike. No statement in the startup files carries
    /// two today, so the sweep cannot notice: it would go green on a
    /// `find` and on a `filter` both, which is exactly the shape that
    /// let the defect ship. The rule is therefore pinned here,
    /// against statements written for the purpose, rather than left
    /// to a tree that happens not to contain one.
    ///
    /// The first row is the mutation that survived: real code from
    /// `renderer/pipeline.rs` with a bare `unwrap()` folded into the
    /// argument of a call whose statement ends in an `.expect(`.
    #[test]
    fn test_every_panicking_spelling_in_a_statement_is_read() {
        let cases: &[(&str, &[&str])] = &[
            (
                "required_limits: wgpu::Limits::downlevel_defaults() \
                 .using_resolution(Some(adapter.limits()).unwrap()), }) .await \
                 .expect(\"Failed to create device\")",
                &[".expect(", ".unwrap()"],
            ),
            ("let w = EventLoop::new().expect(\"no loop\")", &[".expect("]),
            ("let w = EventLoop::new().unwrap()", &[".unwrap()"]),
            ("panic!(\"poisoned\")", &["panic!("]),
            // `PANICKING` order, not statement order — the caller
            // asks "is `.unwrap()` among these", never "which came
            // first".
            (
                "if x { panic!(\"a\") } else { y.unwrap() }",
                &[".unwrap()", "panic!("],
            ),
            // `unwrap_or_default` is the opposite of a panic and must
            // not read as one, which is why `.unwrap()` carries its
            // parentheses.
            ("let n = maybe.unwrap_or_default()", &[]),
            ("let n = maybe.unwrap_or_else(|| 0)", &[]),
            ("let n = plain_call(a, b)", &[]),
        ];
        for (statement, expected) in cases {
            assert_eq!(
                panicking_spellings_in(statement),
                *expected,
                "panicking_spellings_in({statement:?})"
            );
        }
    }
}
