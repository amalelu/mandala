// SPDX-License-Identifier: MPL-2.0

//! Build script for the compiled-in font table.
//!
//! Scans `src/font/fonts/` for `.ttf` / `.otf` files and emits
//! `$OUT_DIR/generated_fonts_data.rs`, which `src/font/fonts.rs`
//! pulls in with `include!`. The generated file defines the
//! `AppFont` enum (one variant per compiled-in font, plus the `Any`
//! sentinel) and the `FONT_DATA` table that binds each variant to
//! its `include_bytes!`-embedded payload.
//!
//! Two properties this script owes its callers:
//!
//! - **Determinism.** Two clean builds of an unchanged tree produce
//!   byte-identical output. Nothing here iterates a `HashMap`, and
//!   every ordering decision routes through
//!   [`name_rules::select_font_variants`], whose comparators are
//!   total. Without this the `AppFont` variant order — and so the
//!   discriminants and the generated diff — churned on every build.
//! - **Correct rebuild triggers.** Emitting even one
//!   `cargo:rerun-if-changed` line switches Cargo from "watch the
//!   whole package" to "watch exactly these paths", so the font
//!   *directories* are registered alongside the files. Watching only
//!   the files found on the previous run means a newly dropped font
//!   never triggers the regeneration that would notice it.
//!
//! Every naming rule lives in `src/font/name_rules.rs`, which this
//! script `include!`s and the library compiles as a normal module —
//! Cargo never builds a build script under `cfg(test)`, so that is
//! what makes the rules unit-testable.

// A build script is a crate root of its own, so `lib.rs`'s
// `#![warn(clippy::unwrap_used)]` never reaches a line written here —
// the same gap `src/bin/generate_stress_map.rs` documents, at the
// last root that still had it. The file is inside
// `unwrap_posture`'s scan, so a bare `unwrap()` here was already
// caught by the gate; what was missing is the second reader, and
// with it the editor-time feedback every other root gets. Proven
// rather than assumed: with an `o.unwrap()` planted below,
// `cargo clippy --workspace --all-targets` reported nothing against
// this file until the attribute landed.
//
// The `cfg_attr` twin below cannot fire here — cargo never builds a
// build script under `cfg(test)`, as the module docs above already
// say. It is carried anyway so every root spells the pair the same
// way, and a reader comparing roots is not left wondering which
// omission was deliberate.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::{env, fs};

use path_slash::PathExt;
use ttf_parser::Face;

mod name_rules {
    include!("src/font/name_rules.rs");
}

use name_rules::{
    decode_name_record, is_font_extension, select_font_variants, FontCandidate, NameEncoding, ANY_VARIANT,
};

/// Font root, relative to the crate manifest directory.
const FONT_DIR: &str = "src/font/fonts";
/// Name of the generated file inside `OUT_DIR`.
const FONTS_RS_FILE: &str = "generated_fonts_data.rs";
/// Sources whose edits must re-run this script but that the font
/// walk below never visits.
const OWN_SOURCES: [&str; 2] = ["build.rs", "src/font/name_rules.rs"];

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR not set by cargo");
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set by cargo");
    prepare_font_file(&out_dir, &manifest_dir)
}

/// Scan the font tree and write `$OUT_DIR/generated_fonts_data.rs`.
fn prepare_font_file(out_dir: &OsStr, manifest_dir: &OsStr) -> Result<(), Box<dyn Error>> {
    let manifest_path = Path::new(manifest_dir);
    let font_root = manifest_path.join(FONT_DIR);

    emit_rerun_directives(manifest_path, &font_root);

    let selection = select_font_variants(collect_candidates(&font_root)?);
    for warning in &selection.warnings {
        println!("cargo:warning={warning}");
    }

    let out_file = Path::new(out_dir).join(FONTS_RS_FILE);
    generate_font_file(&out_file, &selection.fonts)
}

/// Register every path whose change must re-run this script.
///
/// Cargo's default is to watch the whole package; the first
/// `cargo:rerun-if-changed` line replaces that default with an
/// explicit allow-list. The previous version of this script listed
/// only the font files it had found, which meant a font *added*
/// after the last run was invisible: nothing in the watch set had
/// changed, the script never re-ran, and the promised variant never
/// appeared. Registering the directories themselves closes that —
/// Cargo stats a watched directory recursively, so creating,
/// renaming, or deleting a file under it is a change.
///
/// The font files stay in the list as well: a directory's recursive
/// stat covers content edits too, but naming the files keeps the
/// dependency explicit and survives any future narrowing of Cargo's
/// directory handling. `build.rs` and `name_rules.rs` are added
/// because the allow-list must cover this script's own sources.
///
/// Output is sorted so the emitted directive block is as stable as
/// the generated source.
fn emit_rerun_directives(manifest_path: &Path, font_root: &Path) {
    let mut watched: BTreeSet<String> = BTreeSet::new();
    for source in OWN_SOURCES {
        watched.insert(manifest_path.join(source).to_slash_lossy().into_owned());
    }
    watched.insert(font_root.to_slash_lossy().into_owned());

    for entry in walkdir::WalkDir::new(font_root).follow_links(true) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                println!("cargo:warning=font directory walk failed: {error}");
                continue;
            }
        };
        let path = entry.path();
        let is_font_file = path
            .extension()
            .and_then(|extension| is_font_extension(&extension.to_string_lossy()))
            .is_some();
        if entry.file_type().is_dir() || is_font_file {
            watched.insert(path.to_slash_lossy().into_owned());
        }
    }

    for path in watched {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// Walk the font tree and turn every `.ttf` / `.otf` file into a
/// [`FontCandidate`]. Ordering of the returned vector is whatever
/// the filesystem hands back; [`select_font_variants`] is
/// responsible for making the result deterministic, and is the only
/// consumer.
fn collect_candidates(font_root: &Path) -> Result<Vec<FontCandidate>, Box<dyn Error>> {
    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(font_root).follow_links(true) {
        let entry = match entry {
            Ok(entry) => entry,
            // Already reported by `emit_rerun_directives`, which
            // walks the same tree; skipping quietly here avoids
            // doubling every warning.
            Err(_) => continue,
        };
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let Some(extension) = path
            .extension()
            .and_then(|extension| is_font_extension(&extension.to_string_lossy()))
        else {
            continue;
        };
        let Some(file_stem) = path.file_stem() else {
            continue;
        };
        let relative_path = path
            .strip_prefix(font_root)
            .map(|relative| relative.to_slash_lossy().into_owned())
            .unwrap_or_else(|_| path.to_slash_lossy().into_owned());

        let full_name = read_full_name(path)?;
        candidates.push(FontCandidate::new(
            path.to_slash_lossy().into_owned(),
            relative_path,
            &file_stem.to_string_lossy(),
            extension,
            full_name.as_deref(),
        ));
    }
    Ok(candidates)
}

/// Read a font's `FULL_NAME` record, decoded through
/// [`decode_name_record`].
///
/// Returns the first `FULL_NAME` record that survives
/// [`name_rules::is_usable_font_name`], or `None` when the file has
/// none — in which case the caller falls back to the file stem.
///
/// Two failure modes are deliberately *not* build errors. An
/// unparseable container and an undecodable name table both warn and
/// degrade to the file-stem path, because a build script that
/// panics on a font file leaves no recovery short of deleting the
/// file. In particular the previous
/// `str::from_utf8(name.name).expect("Not UTF-8")` turned any
/// non-ASCII Windows-platform name — UTF-16BE, so a single accented
/// character makes the bytes invalid UTF-8 — into a hard build
/// failure, and made the fallback path unreachable for exactly the
/// fonts that needed it. An I/O error *is* propagated: a font file
/// we can see but cannot read is a broken checkout, not a font
/// problem.
fn read_full_name(path: &Path) -> Result<Option<String>, Box<dyn Error>> {
    let data = fs::read(path)?;
    let Ok(face) = Face::parse(&data, 0) else {
        println!(
            "cargo:warning=font '{}' could not be parsed as an OpenType face; naming it from \
             its file name",
            path.display()
        );
        return Ok(None);
    };

    for name in face.names() {
        if name.name_id != ttf_parser::name_id::FULL_NAME {
            continue;
        }
        let encoding = if name.is_unicode() {
            NameEncoding::Utf16Be
        } else {
            NameEncoding::Legacy
        };
        let decoded = decode_name_record(name.name, encoding);
        if name_rules::is_usable_font_name(&decoded) {
            return Ok(Some(decoded));
        }
    }
    Ok(None)
}

/// Escape a path for embedding in a Rust string literal. Font
/// directories are ordinary names today, but a `\` or `"` anywhere
/// in the absolute path would otherwise emit source that does not
/// parse.
fn escape_string_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Make a value safe to interpolate into a generated `///` line:
/// no line breaks (which would silently truncate the comment and
/// then be parsed as code) and no backticks (which would unbalance
/// the inline-code spans the doc uses).
fn escape_doc_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '`' => '\'',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect()
}

/// Write the generated `AppFont` enum and `FONT_DATA` table.
///
/// `fonts` arrives sorted and collision-free from
/// [`select_font_variants`]; this function adds no ordering of its
/// own, so the emitted bytes are a pure function of that slice.
fn generate_font_file(out_file: &Path, fonts: &[FontCandidate]) -> Result<(), Box<dyn Error>> {
    let mut file = File::create(out_file)?;

    writeln!(file, "// @generated by lib/baumhard/build.rs — do not edit.")?;
    writeln!(file, "//")?;
    writeln!(file, "// One variant per font file under `{FONT_DIR}`, plus the")?;
    writeln!(
        file,
        "// `{ANY_VARIANT}` sentinel. The sentinel comes first and the rest are"
    )?;
    writeln!(
        file,
        "// sorted by variant name, so two clean builds of an unchanged"
    )?;
    writeln!(file, "// font tree emit byte-identical source.")?;
    writeln!(file)?;

    writeln!(file, "/// Every font compiled into the binary, plus the")?;
    writeln!(
        file,
        "/// [`AppFont::{ANY_VARIANT}`] sentinel for text that expresses no"
    )?;
    writeln!(file, "/// preference.")?;
    writeln!(file, "///")?;
    writeln!(
        file,
        "/// Generated by `build.rs` from the files under `{FONT_DIR}`;"
    )?;
    writeln!(
        file,
        "/// drop a `.ttf` or `.otf` in there and the variant appears on the"
    )?;
    writeln!(
        file,
        "/// next build. Variant names come from each font's `FULL_NAME`"
    )?;
    writeln!(
        file,
        "/// record, or from its file name when that record is missing or"
    )?;
    writeln!(
        file,
        "/// unreadable; the rules live in `src/font/name_rules.rs`."
    )?;
    writeln!(file, "///")?;
    writeln!(
        file,
        "/// Ordering is fixed — sentinel first, then variants sorted by"
    )?;
    writeln!(
        file,
        "/// name — so the generated source does not churn between builds."
    )?;
    writeln!(
        file,
        "/// Adding or removing a font still shifts the discriminants of the"
    )?;
    writeln!(
        file,
        "/// variants after it, which is safe because nothing persists them:"
    )?;
    writeln!(
        file,
        "/// the derived `Serialize` / `Deserialize` impls key on the variant"
    )?;
    writeln!(
        file,
        "/// *name*, and `.mindmap.json` stores fonts as family-name strings"
    )?;
    writeln!(file, "/// resolved through `app_font_by_family`.")?;
    writeln!(file, "///")?;
    writeln!(
        file,
        "/// Costs: `Copy`, one machine word. Cheap to pass by value."
    )?;
    writeln!(
        file,
        "#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]"
    )?;
    writeln!(file, "pub enum AppFont {{")?;
    writeln!(
        file,
        "    /// No font preference — the face is left to cosmic-text's"
    )?;
    writeln!(
        file,
        "    /// fallback chain. The default pin for text that carries no"
    )?;
    writeln!(file, "    /// explicit family.")?;
    writeln!(file, "    {ANY_VARIANT},")?;
    for font in fonts {
        writeln!(
            file,
            "    /// `{}` — `{}`.",
            escape_doc_text(&font.display_name),
            escape_doc_text(&font.relative_path)
        )?;
        writeln!(file, "    {},", font.variant)?;
    }
    writeln!(file, "}}")?;
    writeln!(file)?;

    writeln!(
        file,
        "/// Every compiled-in font paired with its embedded bytes, in the"
    )?;
    writeln!(
        file,
        "/// same order as the [`AppFont`] variants. The [`AppFont::{ANY_VARIANT}`]"
    )?;
    writeln!(file, "/// sentinel has no payload and is absent. Read once by")?;
    writeln!(
        file,
        "/// `load_font_sources` at startup; the byte slices live in the"
    )?;
    writeln!(file, "/// binary's read-only data, so nothing is copied.")?;
    writeln!(
        file,
        "pub(crate) static FONT_DATA: [(AppFont, &[u8]); {}] = [",
        fonts.len()
    )?;
    for font in fonts {
        writeln!(
            file,
            "    (AppFont::{}, include_bytes!(\"{}\")),",
            font.variant,
            escape_string_literal(&font.absolute_path)
        )?;
    }
    writeln!(file, "];")?;
    Ok(())
}
