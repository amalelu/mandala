// SPDX-License-Identifier: MPL-2.0

//! Logger initialization — single entry point for both targets.
//!
//! The `log` crate's macros (`log::info!` / `warn!` / ...) are
//! the universal Rust idiom: every alternative (`tracing`,
//! `defmt`, structured collectors) implements `log::Log` or
//! provides the same names. Wrapping the macros themselves
//! gains nothing portable, so callsites continue to use
//! `log::warn!(...)` etc. directly.
//!
//! What WAS scattered across `src/main.rs` was the per-target
//! init — `env_logger::init()` on native, `console_log::init_with_level`
//! on WASM, plus the panic hook wiring on WASM. This module
//! collapses both onto one [`crate::util::log::init`] call.

/// Initialize the global logger for whichever target this binary
/// is built for. Native uses `env_logger` (reads `RUST_LOG`);
/// WASM uses `console_log` at `Info` level and installs the
/// `console_error_panic_hook` so a panic surfaces a JS-side
/// stack trace.
///
/// The native default filter is `warn`, not `env_logger`'s own
/// `error` default. CODE_CONVENTIONS §9 designates `warn!` as the
/// channel a degraded frame reports on, and the `log` crate is
/// built with `release_max_level_warn` so those calls survive into
/// release; defaulting the runtime filter to `error` would delete
/// them again one layer down. `RUST_LOG` still overrides, so
/// `RUST_LOG=debug` in a debug build behaves as before.
///
/// WASM's `console_log` filter stays at `Info` — looser than
/// native's `warn`, and harmless: the release cap compiles
/// `info!` out before the runtime filter ever sees it, so both
/// targets ship the same warn/error channel (§4).
///
/// Idempotent in the sense that calling twice is a programming
/// error — both backends will panic on a second init. Should
/// fire once at program start.
pub fn init() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(NATIVE_DEFAULT_FILTER),
        );
        // `RUST_LOG=` — set but empty — is a shell accident, not a
        // request for silence. `default_filter_or` applies only when
        // the variable is *absent*, so an empty value would slide past
        // the `warn` default onto env_logger's own `error` and delete
        // the §9 channel again one layer down. Force the default back.
        if !rust_log_is_effective(std::env::var("RUST_LOG").ok().as_deref()) {
            builder.parse_filters(NATIVE_DEFAULT_FILTER);
        }
        builder.init();
    }

    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("failed to init logger");
    }
}

/// The native runtime filter applied when `RUST_LOG` carries nothing
/// usable. Matches the `release_max_level_warn` compile-time cap, so
/// the runtime filter never becomes the narrower of the two.
#[cfg(not(target_arch = "wasm32"))]
const NATIVE_DEFAULT_FILTER: &str = "warn";

/// Whether `RUST_LOG`'s raw value carries directives worth honoring.
/// `None` (unset) and `Some("")` / `Some("   ")` both mean "no", which
/// is the point: env_logger distinguishes them and we must not.
#[cfg(not(target_arch = "wasm32"))]
fn rust_log_is_effective(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// The `log` feature the workspace's single `log` declaration — in
/// the root `[workspace.dependencies]` table — must carry. Changing
/// this constant without changing that declaration fails
/// [`tests::test_the_workspace_keeps_warn_and_error_in_release`].
#[cfg(all(test, not(target_arch = "wasm32")))]
const REQUIRED_LOG_LEVEL_FEATURE: &str = "release_max_level_warn";

/// The CODE_CONVENTIONS heading whose section owns the release
/// log-level policy. Named once so the doc test pins the *section*
/// rather than scanning the whole file.
#[cfg(all(test, not(target_arch = "wasm32")))]
const POLICY_HEADING: &str = "## §9 Error handling";

/// `log` versions in which the
/// `compile_error!("multiple release_max_level_* features set")` guard
/// the root manifest's comment leans on was *read*, not assumed —
/// `log-0.4.29/src/lib.rs:376-394`.
///
/// `[workspace.dependencies]` requests `"0.4.29"`, which is a caret
/// requirement, so `cargo update` can move the resolved version
/// underneath that text without touching the manifest. Pinning the
/// *lockfile* is therefore the only place the claim can be held.
#[cfg(all(test, not(target_arch = "wasm32")))]
const VERIFIED_LOG_VERSIONS: &[&str] = &["0.4.29"];

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{rust_log_is_effective, POLICY_HEADING, REQUIRED_LOG_LEVEL_FEATURE, VERIFIED_LOG_VERSIONS};
    use crate::util::doc_fixtures::{repo_path, section_text};

    /// Every line in the workspace that declares a `log` dependency,
    /// paired with the manifest it came from.
    ///
    /// Sourced through [`crate::util::manifests::member_manifests`]
    /// rather than a hard-coded pair of paths, so a future member
    /// that declares its own `log` is checked rather than missed —
    /// which is precisely how the level could drift back apart after
    /// `[workspace.dependencies]` made the shared declaration
    /// singular.
    fn log_dependency_lines() -> Vec<(String, String)> {
        let mut found = Vec::new();
        for manifest in crate::util::manifests::member_manifests() {
            let text = std::fs::read_to_string(repo_path(&manifest))
                .unwrap_or_else(|e| panic!("{manifest} must be readable: {e}"));
            found.extend(
                text.lines()
                    .filter(|line| line.trim_start().starts_with("log = "))
                    .map(|line| (manifest.clone(), line.to_string())),
            );
        }
        found
    }

    /// The whole failure mode this issue was about: a `log` max-level
    /// feature that quietly deletes CODE_CONVENTIONS §9's failure
    /// channel from the binaries users run.
    ///
    /// The workspace used to declare the feature twice — once in the
    /// app crate, once in baumhard. Those could not silently
    /// *disagree*, because cargo unions `log`'s features across the
    /// workspace and `log` itself `compile_error!`s on "multiple
    /// release_max_level_* features set", so a mixed workspace failed
    /// to build. `[workspace.dependencies]` removed the second
    /// declaration outright, so there is nothing left to mix.
    ///
    /// What still has no compiler behind it is the surviving
    /// declaration naming the *wrong* level: a drift back to `off`, a
    /// swap to `release_max_level_error`, or the feature dropped
    /// (which uncaps release to `Trace` and re-admits the chatty
    /// walker instrumentation). That is what this pins — over every
    /// `log = ` line in the workspace, so re-splitting the
    /// declaration cannot slip a second, wrong one past it either.
    #[test]
    fn test_the_workspace_keeps_warn_and_error_in_release() {
        // Every level cap that would drop `warn!` below the §9
        // contract, plus the two looser ones — naming them all means
        // a swap to any other variant fails here rather than sliding
        // through on a substring match.
        const REJECTED: &[&str] = &[
            "release_max_level_off",
            "release_max_level_error",
            "release_max_level_info",
            "release_max_level_debug",
            "release_max_level_trace",
        ];

        let declarations = log_dependency_lines();
        assert_eq!(
            declarations.len(),
            1,
            "the workspace must declare `log` exactly once, in the root \
             `[workspace.dependencies]` table, with every member writing \
             `log.workspace = true`; found {declarations:#?}"
        );

        for (manifest, line) in declarations {
            assert!(
                line.contains(REQUIRED_LOG_LEVEL_FEATURE),
                "{manifest} must declare `{REQUIRED_LOG_LEVEL_FEATURE}` on the `log` \
                 dependency so CODE_CONVENTIONS §9's warn!/error! channel survives \
                 release builds; found: {line}"
            );
            for rejected in REJECTED {
                assert!(
                    !line.contains(rejected),
                    "{manifest} declares `{rejected}`, which contradicts \
                     `{REQUIRED_LOG_LEVEL_FEATURE}`; found: {line}"
                );
            }
        }
    }

    /// The version of `log` the lockfile actually resolves to.
    fn locked_log_version() -> String {
        let lock = std::fs::read_to_string(repo_path("Cargo.lock"))
            .unwrap_or_else(|e| panic!("Cargo.lock must be readable: {e}"));
        lock.split("[[package]]")
            .find(|block| block.lines().any(|l| l.trim() == r#"name = "log""#))
            .and_then(|block| {
                block
                    .lines()
                    .find_map(|l| l.trim().strip_prefix(r#"version = ""#))
                    .and_then(|v| v.strip_suffix('"'))
                    .map(str::to_string)
            })
            .expect("Cargo.lock must resolve a `log` package with a version")
    }

    /// The manifest comments justify the two-manifest lockstep rule by
    /// pointing at a `compile_error!` inside `log` — a claim about a
    /// *dependency's source*, which nothing in this workspace would
    /// otherwise notice going stale.
    ///
    /// Both manifests request `"0.4.29"`, a caret requirement, so
    /// `cargo update` alone can move the resolved crate to a version
    /// whose guard nobody has read while the manifests, the comments
    /// and every other test stay green. This fails instead, and the
    /// fix is deliberately manual: open the new `log`'s `src/lib.rs`,
    /// confirm the `compile_error!("multiple release_max_level_*
    /// features set")` is still there, then add the version here.
    ///
    /// Deleting this test to make a bump pass is the one wrong answer
    /// — the comments would then be citing a file no one has opened.
    #[test]
    fn test_locked_log_version_has_a_verified_mixed_feature_guard() {
        let version = locked_log_version();
        assert!(
            VERIFIED_LOG_VERSIONS.contains(&version.as_str()),
            "Cargo.lock resolves `log` {version}, which is not in the verified set \
             {VERIFIED_LOG_VERSIONS:?}. `Cargo.toml` and `lib/baumhard/Cargo.toml` \
             both cite log's `compile_error!(\"multiple release_max_level_* features \
             set\")` as the reason the two manifests cannot silently disagree. \
             Re-read `log-{version}/src/lib.rs`, confirm the guard is still there, \
             and add {version:?} to `VERIFIED_LOG_VERSIONS` — do not delete this test."
        );
    }

    /// The end-to-end pin: not what the manifests *say*, but what the
    /// unified feature set actually resolves `log::STATIC_MAX_LEVEL`
    /// to in the profile this test was compiled for.
    ///
    /// `STATIC_MAX_LEVEL` selects on `cfg!(debug_assertions)`, so the
    /// release half of the policy is only observable under
    /// `cargo test --release`; `./test.sh` runs the dev profile and
    /// exercises the `Trace` half. Both halves are asserted rather
    /// than only the interesting one, because a `max_level_*` feature
    /// arriving from some future dependency would cap debug builds
    /// too and silently delete the instrumentation this repo debugs
    /// with.
    #[test]
    fn test_static_max_level_resolves_to_the_documented_boundary() {
        let expected = if cfg!(debug_assertions) {
            log::LevelFilter::Trace
        } else {
            log::LevelFilter::Warn
        };
        assert_eq!(
            log::STATIC_MAX_LEVEL,
            expected,
            "the unioned `log` feature set resolves the compile-time cap to {:?}, not \
             {expected:?}; debug builds must stay uncapped and release builds must stop \
             at `{REQUIRED_LOG_LEVEL_FEATURE}` so CODE_CONVENTIONS §9's warn/error \
             channel survives",
            log::STATIC_MAX_LEVEL
        );
    }

    /// The conventions document is the policy's only prose home, and
    /// §9 is the section that owns it. Scoped to that section on
    /// purpose: a whole-file `contains()` stays green when the
    /// paragraph is moved into another section or §9 is renamed out
    /// from under it, which is precisely the silent drift
    /// [`crate::util::doc_fixtures`] exists to refuse.
    ///
    /// Asserts the *boundary*, not just the feature name — §9 could
    /// otherwise state the opposite of what ships and still pass.
    #[test]
    fn test_code_conventions_section_9_documents_the_release_log_boundary() {
        let path = repo_path("CODE_CONVENTIONS.md");
        let section = section_text(&path, POLICY_HEADING);

        assert!(
            section.contains(REQUIRED_LOG_LEVEL_FEATURE),
            "CODE_CONVENTIONS.md {POLICY_HEADING} must name \
             `{REQUIRED_LOG_LEVEL_FEATURE}` so the manifests and the stated \
             policy cannot drift apart"
        );

        // The levels that survive release must be named as surviving,
        // and the ones that do not must be named as not surviving.
        // Checking both halves is what makes this a boundary pin:
        // `release_max_level_warn` alone does not say where the line
        // falls to a reader deciding which macro to reach for.
        for surviving in ["`warn!`", "`error!`"] {
            assert!(
                section.contains(surviving),
                "CODE_CONVENTIONS.md {POLICY_HEADING} must name {surviving} as a \
                 level that survives release builds"
            );
        }
        for compiled_out in ["`info!`", "`debug!`", "`trace!`"] {
            assert!(
                section.contains(compiled_out),
                "CODE_CONVENTIONS.md {POLICY_HEADING} must name {compiled_out} as a \
                 level that is compiled out of release builds"
            );
        }

        // Direction, not just vocabulary. Compared with runs of
        // whitespace collapsed so a re-wrap of the paragraph does not
        // fail the test, but an inverted claim does.
        let flat = section.split_whitespace().collect::<Vec<_>>().join(" ");
        let boundary = "`warn!` and `error!` survive into release; `info!`, `debug!` and `trace!` do not.";
        assert!(
            flat.contains(boundary),
            "CODE_CONVENTIONS.md {POLICY_HEADING} must state the boundary in the \
             direction the manifests actually implement it — expected the sentence \
             {boundary:?}. A §9 that named the same five macros while claiming the \
             opposite would otherwise pass."
        );
    }

    /// `RUST_LOG=` set-but-empty must behave as unset. env_logger's
    /// `default_filter_or` fires only on an *absent* variable, so
    /// without this distinction an empty value lands on env_logger's
    /// own `error` default and deletes §9's warn channel at runtime.
    #[test]
    fn test_empty_rust_log_is_treated_as_unset() {
        assert!(rust_log_is_effective(Some("debug")));
        assert!(rust_log_is_effective(Some("mandala=trace,warn")));
        assert!(
            !rust_log_is_effective(None),
            "unset must fall back to the default"
        );
        assert!(!rust_log_is_effective(Some("")), "`RUST_LOG=` must fall back too");
        assert!(
            !rust_log_is_effective(Some("   ")),
            "whitespace-only is not a directive"
        );
    }
}
