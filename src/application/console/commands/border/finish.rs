// SPDX-License-Identifier: MPL-2.0

//! The closing move every border-family verb makes, written once.
//!
//! Five verbs stage a
//! [`crate::application::document::BorderConfigEdits`] and hand it
//! to a different setter — `border` to `set_node_border_config`,
//! `section frame` to `set_section_frame_border_config`, `canvas`
//! to one of two canvas-default setters, the `preview` family to
//! `set_border_preview`, and `preview commit` to whichever of those
//! the staged target names. What they do with the
//! [`crate::application::document::BorderEditOutcome`] afterwards
//! was five copies of the same thirty lines, and the copies had
//! drifted: the auto-promotion note named a different scope in
//! each (correctly), but the refusal wording, the
//! `preset=custom`-with-no-glyphs hint and the
//! single-line-vs-`Lines` collapse were byte-identical
//! reimplementations.
//!
//! Four is what an earlier pass counted, because the fifth —
//! `preview::commit_border_preview_verb` — prints only after a
//! *prior* line staged a preview, and every `EXEC_CORPUS` row runs
//! on a document of its own. It took the console oracle's
//! `EXEC_SEQ_CORPUS` — sequences run against one document, named
//! rather than linked because it is `#[cfg(test)]` and rustdoc
//! strips it — to make that copy visible: perturb the note here, and the
//! staging line moved while the commit line did not.
//!
//! [`BorderEdit`] is the four differences named as fields, and
//! [`BorderEdit::finish`] is the shared thirty lines. The one
//! genuinely structural difference — a staged preview has no "no
//! change" to report, because it is active either way — is carried
//! by `headline` being an `Option` rather than by a flag: `None`
//! *is* the no-change outcome, and a verb that always has a
//! headline can never reach that arm.

use super::execute::custom_preset_hint;
use crate::application::console::ExecResult;

/// One border-family verb's outcome, in the words that verb uses.
pub(crate) struct BorderEdit<'a> {
    /// The words every line from this verb leads with: `border`,
    /// `section frame`, `canvas border`, `border preview`, …
    pub(crate) label: &'a str,
    /// What the auto-promotion note calls the thing the glyph
    /// override hangs off — `node`, `section`, `canvas`, `target`.
    /// It reads as `the per-{scope} glyph override`.
    pub(crate) scope: &'static str,
    /// The success headline, or `None` when nothing changed.
    /// A staged preview passes `Some` unconditionally: it is
    /// active whether or not the edit moved anything, so it has no
    /// no-change arm to reach.
    pub(crate) headline: Option<String>,
    /// Glyphs the setter refused. Non-empty makes the whole call
    /// an error.
    pub(crate) rejected: Vec<String>,
    /// The preset the setter promoted to `custom`, if it did.
    pub(crate) auto_promoted: Option<String>,
    /// `preset=custom` with no glyph field alongside it — an edit
    /// that looks like a no-op unless the hint explains what
    /// `custom` is asking for.
    pub(crate) bare_custom: bool,
}

impl BorderEdit<'_> {
    /// Fold the outcome into the `ExecResult` the verb returns.
    pub(crate) fn finish(self) -> ExecResult {
        // A refused glyph is an error, not a "no change" and not an
        // active preview: the setter declined it because the loader
        // would reject the saved file, and reporting success here is
        // exactly how the user ends up with a map they cannot
        // reopen.
        if !self.rejected.is_empty() {
            return ExecResult::Err(format!("{}: {}", self.label, self.rejected.join("; ")));
        }
        let Some(headline) = self.headline else {
            // A `preset=custom`-only edit on a target that already
            // records `preset: custom` is a no-op at the data-model
            // level, but the user still benefits from the same
            // orientation message as the changed path. Emit it
            // instead of the bare "no change" line so the input does
            // not feel ignored.
            if self.bare_custom {
                return ExecResult::lines(vec![
                    format!("{}: preset=custom set; no glyph fields were given", self.label),
                    custom_preset_hint(self.label),
                ]);
            }
            return ExecResult::ok_msg(format!("{}: no change", self.label));
        };
        let mut lines: Vec<String> = vec![headline];
        // Surfaced exactly once per command invocation, not once per
        // affected target — the same edit applies to every one of
        // them, so the message would be redundant. The caller passes
        // the first promoted target's requested preset; every other
        // target received the same edit struct, so the value is
        // necessarily the same.
        if let Some(name) = self.auto_promoted {
            lines.push(format!(
                "note: preset='{}' auto-promoted to 'custom' \
                 (a side or corner glyph was set; non-custom presets \
                 ignore the per-{} glyph override)",
                name, self.scope
            ));
        }
        if self.bare_custom {
            lines.push(custom_preset_hint(self.label));
        }
        if lines.len() == 1 {
            ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
        } else {
            ExecResult::lines(lines)
        }
    }
}
