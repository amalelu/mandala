// SPDX-License-Identifier: MPL-2.0

//! The protocol's two named coordinate spaces (`format/ipc.md`
//! §Coordinate spaces). Every command that takes or returns geometry
//! names one, and a command that accepts either takes a `space`
//! parameter spelled with [`Space::parse`]'s words.

/// Where a coordinate pair lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    /// Physical pixels on the rendered surface, origin top-left,
    /// y-down: the space of screenshots and synthetic mouse input.
    Surface,
    /// The document's world space, the space of `MindNode` offsets,
    /// as the camera sees it.
    Canvas,
}

impl Space {
    /// The wire words, for a parameter's `describe` doc and its
    /// error messages.
    pub(crate) const WORDS: &'static str = "\"canvas\" or \"surface\"";

    /// Parse a `space` parameter's value.
    pub(crate) fn parse(word: &str) -> Option<Space> {
        match word {
            "surface" => Some(Space::Surface),
            "canvas" => Some(Space::Canvas),
            _ => None,
        }
    }
}

/// A position in a named space: where a synthetic pointer stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) space: Space,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_parses_exactly_the_documented_words() {
        assert_eq!(Space::parse("canvas"), Some(Space::Canvas));
        assert_eq!(Space::parse("surface"), Some(Space::Surface));
        for rejected in ["Canvas", "screen", "world", ""] {
            assert_eq!(Space::parse(rejected), None, "{rejected:?}");
        }
    }

    /// The document names both words in its coordinate-space section;
    /// a third space added here without the document fails.
    #[test]
    fn test_space_words_match_the_documented_spaces() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("format/ipc.md"), "### Coordinate spaces");
        for word in ["surface", "canvas"] {
            assert!(
                section.contains(&format!("**`{word}`**")),
                "format/ipc.md §Coordinate spaces must define `{word}`"
            );
            assert!(Space::WORDS.contains(&format!("\"{word}\"")));
        }
    }
}
