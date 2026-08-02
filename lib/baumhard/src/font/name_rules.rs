// SPDX-License-Identifier: MPL-2.0

// The module-level concept header for this file lives as a `///`
// doc on `pub mod name_rules;` in `src/font/mod.rs` rather than as
// a `//!` header here: `build.rs` pulls this file in with
// `include!`, and an inner doc comment cannot appear in a macro
// expansion. Rustdoc renders the two forms identically.

/// File extensions the font scan accepts, lowercase. Compared
/// case-insensitively by [`is_font_extension`] so `Foo.TTF` is not
/// silently skipped on a case-preserving filesystem.
///
/// Order is meaningful: it is the preference order used to break a
/// collision between two files that describe the same font, so
/// `TreeRoot-x.ttf` wins over `Treeroot-y.otf`.
pub const FONT_EXTENSIONS: [&str; 2] = ["ttf", "otf"];

/// Shortest token [`fallback_sanitize`] keeps when a font's `name`
/// table yields nothing usable and the file stem has to carry the
/// variant name. Tokens shorter than this are noise (`v2`, `a`, the
/// random suffix a font host appends) — unless dropping them would
/// leave no name at all.
pub const MIN_TOKEN_LEN: usize = 4;

/// Length at which [`fallback_sanitize`] treats a stem's first
/// token as the whole name and ignores what follows.
///
/// Font hosts ship `ToyTrain-WYdO.ttf`: a real name followed by a
/// cache-busting suffix. A first token this long is the name, and
/// gluing the suffix onto it would only produce `ToyTrainWYdO`.
/// Below the threshold the token is too generic to stand alone and
/// the remaining tokens are joined onto it.
pub const SELF_SUFFICIENT_TOKEN_LEN: usize = 5;

/// Longest variant name [`fallback_sanitize`] will build out of a
/// file stem. Font hosts hand out stems like
/// `some-extremely-long-display-face-name-a1b2c3`; without a
/// ceiling the generated enum grows unreadable variants.
pub const MAX_FALLBACK_LEN: usize = 20;

/// Name of the "no preference" sentinel variant. Reserved: a font
/// whose derived name collides with it is renamed by
/// [`select_font_variants`] rather than allowed to produce a
/// duplicate variant.
pub const ANY_VARIANT: &str = "Any";

/// Variant names [`select_font_variants`] will not hand to a font,
/// renaming the font with a numeric suffix instead.
///
/// - [`ANY_VARIANT`] is the sentinel the generated enum defines
///   itself; a second `Any` would not compile.
/// - `Self` is a Rust keyword, and the *only* one reachable here:
///   every other keyword is lowercase, and [`capitalize_first`]
///   uppercases the first character of every derived name. It is
///   also one of the few keywords with no raw-identifier escape —
///   `r#Self` is rejected — so a font named "self" can only be
///   renamed or dropped, and renaming keeps the font.
pub const RESERVED_VARIANTS: [&str; 2] = [ANY_VARIANT, "Self"];

/// Text encoding of a raw OpenType `name`-table record.
///
/// The `name` table stores one string per (platform, encoding,
/// language) triple, and the encoding is *not* implied by the
/// bytes: a Windows-platform record is UTF-16BE, a
/// Macintosh-platform record is a legacy single-byte codepage.
/// Decoding a UTF-16BE record as UTF-8 happens to work for pure
/// ASCII names — every other byte is `0x00`, which is valid UTF-8
/// and filtered out downstream — and fails the moment a name
/// contains a single accented character. [`decode_name_record`]
/// takes the encoding explicitly so that accident cannot be
/// mistaken for correctness.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NameEncoding {
    /// UTF-16 big-endian: Unicode platform, or Windows platform
    /// with the Symbol / Unicode-BMP encoding IDs. This is what
    /// `ttf_parser::name::Name::is_unicode` reports.
    Utf16Be,
    /// Anything else — in practice Macintosh Roman. Bytes below
    /// `0x80` are ASCII; the high half is a legacy codepage whose
    /// characters [`ascii_font_name`] discards anyway, so they are
    /// decoded to the replacement character rather than through a
    /// codepage table that would only feed the filter.
    Legacy,
}

/// Decode one raw `name`-table record into a Rust `String`.
///
/// Never fails and never panics: malformed UTF-16 (unpaired
/// surrogates, an odd byte count) and legacy high bytes both decode
/// to `U+FFFD`. That matters because this runs inside a build
/// script — a panic here is a build failure with no recovery path,
/// and the whole point of the fallback chain in [`variant_name`] is
/// that an undecodable name degrades to the file stem instead of
/// stopping the build.
///
/// Costs: one `String` allocation, one pass over `bytes`.
pub fn decode_name_record(bytes: &[u8], encoding: NameEncoding) -> String {
    match encoding {
        NameEncoding::Utf16Be => {
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
            char::decode_utf16(units)
                .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        NameEncoding::Legacy => bytes
            .iter()
            .map(|&byte| {
                if byte.is_ascii() {
                    byte as char
                } else {
                    char::REPLACEMENT_CHARACTER
                }
            })
            .collect(),
    }
}

/// Reduce a decoded font name to the ASCII alphanumerics and single
/// spaces a Rust identifier can be built from.
///
/// Rust identifiers may contain non-ASCII XID characters, but an
/// `AppFont::Café` variant is a trap for every downstream consumer
/// (source encoding, `grep`, generated bindings), so the enum stays
/// ASCII. Whitespace runs collapse to one space and the result is
/// trimmed, leaving [`camel_case`] a clean word sequence to
/// capitalize.
///
/// Costs: one `String` allocation, one pass over `decoded`.
pub fn ascii_font_name(decoded: &str) -> String {
    let mut out = String::with_capacity(decoded.len());
    let mut pending_space = false;
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else if ch.is_ascii_alphanumeric() {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Return the extension's lowercase form when it names a font
/// container we compile in, `None` otherwise. Case-insensitive, so
/// `Foo.TTF` and `Foo.ttf` are the same kind of file.
///
/// Costs: one `String` allocation for the lowercased extension,
/// then a linear scan of [`FONT_EXTENSIONS`] (two entries).
pub fn is_font_extension(extension: &str) -> Option<String> {
    let lowered = extension.to_ascii_lowercase();
    FONT_EXTENSIONS.contains(&lowered.as_str()).then_some(lowered)
}

/// Preference rank of a font container, lower wins. Used to break a
/// collision between two files describing the same font: the TTF is
/// preferred over the OTF, matching the historical behavior of the
/// scan. An unknown extension ranks last.
///
/// Costs: no allocation; a linear scan of [`FONT_EXTENSIONS`] (two
/// entries). `extension` is expected already lowercased by
/// [`is_font_extension`].
pub fn extension_rank(extension: &str) -> usize {
    FONT_EXTENSIONS
        .iter()
        .position(|candidate| *candidate == extension)
        .unwrap_or(FONT_EXTENSIONS.len())
}

/// Family grouping key for a font file: the lowercased file-stem
/// prefix up to the first `-`.
///
/// Font hosts ship the same face as both a TTF and an OTF with the
/// same stem prefix and different random suffixes
/// (`AppleTea-jELql.otf` / `AppleTea-z8R1a.ttf`). Both describe one
/// font, and this key is what brings the pair together.
///
/// **It is a coarse key, not a statement that two files are the
/// same face.** `NotoSerifTibetan-Bold.ttf` and
/// `NotoSerifTibetan-Regular.ttf` — the `Family-Style.ttf` shape
/// Google Fonts ships, and four fonts in this tree already use —
/// share a key and are two different faces. So this key alone must
/// never decide that a file is redundant; [`select_font_variants`]
/// pairs it with the derived variant name and only collapses files
/// that agree on *both*. Grouping on the key alone silently deleted
/// a live variant the moment a second style landed beside it.
///
/// Costs: one `String` allocation for the lowercased prefix.
pub fn family_key(file_stem: &str) -> String {
    file_stem
        .split_once('-')
        .map_or(file_stem, |(prefix, _)| prefix)
        .to_ascii_lowercase()
}

/// Uppercase the first character of `word`, leaving the rest as-is.
///
/// Deliberately *not* a lowercasing pass over the tail: font names
/// carry meaningful internal capitalization (`GalatiaSIL`,
/// `RCRocket`, `NIGHTCROW`) that a naive title-case would destroy.
///
/// Costs: one `String` allocation, one pass over `word`.
pub fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Move a run of leading ASCII digits to the end of `name`, so
/// `212Keyboard` becomes `Keyboard212`.
///
/// A Rust identifier cannot start with a digit, and several fonts
/// in the tree do (`212 Keyboard`, `2Peas Hearts Delight`).
/// Rotating rather than dropping keeps the digits — they are part
/// of the font's identity — while producing a legal variant name.
///
/// Rotation exposes a character that was in the *middle* of the
/// name, so callers capitalize *after* rotating, never before:
/// `8bit Wonder` rotates to `bitWonder8`, and only a following
/// [`capitalize_first`] makes it the `BitWonder8` that CamelCase
/// requires.
///
/// Costs: one `String` allocation, one pass over `name`.
pub fn rotate_leading_digits(name: &str) -> String {
    let split = name.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(name.len());
    if split == 0 {
        return name.to_string();
    }
    format!("{}{}", &name[split..], &name[..split])
}

/// Turn an [`ascii_font_name`] result into a Rust enum-variant
/// identifier, or `None` when the name carries nothing an
/// identifier can be built from (empty, whitespace only, digits
/// only).
///
/// Words are joined without a separator and each word's first
/// character is uppercased; leading digits are rotated to the end
/// by [`rotate_leading_digits`]. The input is expected to be the
/// output of [`ascii_font_name`] — ASCII alphanumerics separated by
/// single spaces — so that the "which characters survive" decision
/// lives in exactly one place.
///
/// The final [`capitalize_first`] runs *after* the rotation, not
/// before: rotating `8bit Wonder` promotes the `b` of `bit` — a
/// character that was never word-initial — to the front, and
/// without the second pass the result is `bitWonder8`, which rustc
/// flags as `variant should have an upper camel case name`.
///
/// Costs: three `String` allocations, one pass per word plus two
/// passes over the joined name.
pub fn camel_case(ascii_name: &str) -> Option<String> {
    let joined: String = ascii_name.split_whitespace().map(capitalize_first).collect();
    let rotated = capitalize_first(&rotate_leading_digits(&joined));
    if rotated.is_empty() || rotated.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(rotated)
}

/// Derive a variant name from a font file's stem, for files whose
/// `name` table is missing, undecodable, or carries nothing usable.
///
/// The stem is split on whitespace, `-`, and `_`, and each token is
/// reduced to ASCII alphanumerics. Tokens shorter than
/// [`MIN_TOKEN_LEN`] are dropped as noise — version markers, the
/// random suffix font hosts append — unless dropping them would
/// leave nothing. Then:
///
/// - a first token of at least [`SELF_SUFFICIENT_TOKEN_LEN`]
///   characters *is* the name (`ToyTrain-WYdO` → `ToyTrain`, not
///   `ToyTrainWYdO`);
/// - otherwise the tokens are capitalized and joined while the
///   result fits [`MAX_FALLBACK_LEN`].
///
/// Either way the result is capped at [`MAX_FALLBACK_LEN`], has its
/// leading digits rotated by [`rotate_leading_digits`], and is
/// capitalized once more afterwards — for the reason spelled out on
/// [`camel_case`], so both derivation paths produce the same shape
/// of identifier.
///
/// Takes the *stem*, not the file name: sanitizing `1234.ttf` would
/// otherwise yield the variant name `ttf`.
///
/// Returns an empty string when the stem yields no legal
/// identifier — the caller (the build script) warns and skips the
/// file rather than emitting source that will not compile.
///
/// Costs: one `Vec` of per-token `String`s plus a few `String`
/// allocations for the joined result. Runs once per font file.
pub fn fallback_sanitize(file_stem: &str) -> String {
    let tokens: Vec<String> = file_stem
        .split(|ch: char| ch.is_whitespace() || ch == '-' || ch == '_')
        .map(|token| {
            token
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .collect();

    let substantial: Vec<&String> = tokens
        .iter()
        .filter(|token| token.len() >= MIN_TOKEN_LEN)
        .collect();
    let kept: Vec<&String> = if substantial.is_empty() {
        tokens.iter().collect()
    } else {
        substantial
    };

    let mut out = String::new();
    match kept.first() {
        None => {}
        Some(first) if first.len() >= SELF_SUFFICIENT_TOKEN_LEN => {
            out = capitalize_first(first);
            out.truncate(MAX_FALLBACK_LEN);
        }
        Some(_) => {
            for token in kept {
                let word = capitalize_first(token);
                if out.len() + word.len() > MAX_FALLBACK_LEN {
                    break;
                }
                out.push_str(&word);
            }
        }
    }

    let rotated = capitalize_first(&rotate_leading_digits(&out));
    if rotated.chars().all(|ch| ch.is_ascii_digit()) {
        // All-digit stems ("2000.ttf") cannot become an identifier.
        return String::new();
    }
    rotated
}

/// Whether a decoded `name`-table record carries enough to build a
/// variant name from.
///
/// A font's name table holds one record per (platform, encoding,
/// language) triple, and not all of them survive the ASCII
/// reduction — a Japanese-language `FULL_NAME` record reduces to
/// nothing. The build script walks the records in table order and
/// takes the first this accepts, so a font that carries a usable
/// name anywhere in its table is named from it rather than from its
/// file name.
///
/// Costs: a full [`ascii_font_name`] + [`camel_case`] derivation,
/// whose result is discarded. Runs once per `FULL_NAME` record, so
/// a handful of times per font file.
pub fn is_usable_font_name(decoded: &str) -> bool {
    camel_case(&ascii_font_name(decoded)).is_some()
}

/// The full derivation: prefer the font's own `name`-table entry,
/// fall back to the file stem.
///
/// `font_full_name` is the already-decoded `FULL_NAME` record (see
/// [`decode_name_record`]) or `None` when the file has no readable
/// one. An empty return means neither source yielded a legal
/// identifier.
///
/// The returned name is not yet guaranteed unique or available —
/// [`select_font_variants`] owns collisions and the
/// [`RESERVED_VARIANTS`] check.
///
/// Costs: one [`ascii_font_name`] + [`camel_case`] pass, or one
/// [`fallback_sanitize`] pass. Runs once per font file.
pub fn variant_name(font_full_name: Option<&str>, file_stem: &str) -> String {
    font_full_name
        .map(ascii_font_name)
        .and_then(|ascii| camel_case(&ascii))
        .unwrap_or_else(|| fallback_sanitize(file_stem))
}

/// One font file discovered by the build script's directory walk,
/// with every naming decision already applied.
///
/// Constructed by [`FontCandidate::new`]; consumed by
/// [`select_font_variants`], which resolves collisions and fixes the
/// emission order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontCandidate {
    /// Rust enum-variant identifier for this font. Assigned by
    /// [`variant_name`] and possibly given a numeric suffix by
    /// [`select_font_variants`] when two files derive the same name.
    pub variant: String,
    /// Human-readable family name for the generated doc comment —
    /// the font's own `FULL_NAME` when it had a readable one, the
    /// file stem otherwise.
    pub display_name: String,
    /// Absolute, forward-slash path handed to `include_bytes!`.
    pub absolute_path: String,
    /// Path relative to the font root, for the generated doc
    /// comment. Also the tiebreaker for equal-preference
    /// collisions, so it must be stable across machines — which the
    /// absolute path is not.
    pub relative_path: String,
    /// Lowercased file-stem prefix; see [`family_key`].
    pub family_key: String,
    /// Lowercased container extension; see [`is_font_extension`].
    pub extension: String,
}

impl FontCandidate {
    /// Build a candidate from the pieces a directory walk yields.
    ///
    /// `font_full_name` is the decoded `FULL_NAME` record or `None`.
    /// `relative_path` is relative to the font root and uses forward
    /// slashes on every platform. The derived variant name may be
    /// empty when neither the name table nor the stem yields a legal
    /// identifier; [`select_font_variants`] skips those with a
    /// warning.
    ///
    /// Costs: takes ownership of the two paths, and allocates the
    /// derived variant name, display name, and family key. Runs
    /// once per font file during the build script's walk.
    pub fn new(
        absolute_path: String,
        relative_path: String,
        file_stem: &str,
        extension: String,
        font_full_name: Option<&str>,
    ) -> Self {
        let ascii_name = font_full_name.map(ascii_font_name).unwrap_or_default();
        let display_name = if ascii_name.is_empty() {
            file_stem.to_string()
        } else {
            ascii_name.clone()
        };
        FontCandidate {
            variant: variant_name(font_full_name, file_stem),
            display_name,
            absolute_path,
            relative_path,
            family_key: family_key(file_stem),
            extension,
        }
    }

    /// Collision-preference key: TTF before OTF, then the
    /// lexicographically smallest relative path.
    ///
    /// Total over any set of candidates with **distinct relative
    /// paths**, which is what makes selection independent of
    /// directory-walk order. Two candidates sharing a relative path
    /// compare equal and their relative order would be decided by
    /// the caller's sort — not reachable from a filesystem walk,
    /// which yields each path once, but the guarantee is scoped to
    /// what the key actually distinguishes rather than to "any set
    /// of distinct files".
    fn preference(&self) -> (usize, &str) {
        (extension_rank(&self.extension), self.relative_path.as_str())
    }
}

/// Outcome of [`select_font_variants`]: the fonts that become
/// `AppFont` variants, plus the human-readable notes the build
/// script re-emits as `cargo:warning` lines.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FontSelection {
    /// Surviving fonts, sorted by variant name. This is the emission
    /// order of the generated enum.
    pub fonts: Vec<FontCandidate>,
    /// Notes about files that were renamed or skipped. Sorted with
    /// the decisions that produced them, so the set of warnings is
    /// as reproducible as the generated source.
    pub warnings: Vec<String>,
}

/// Resolve the discovered font files into the exact variant list the
/// generated enum will carry.
///
/// Three passes, each with an explicit comparator so the result
/// depends on the input *set* and never on its order:
///
/// 1. **Unnamable files are dropped** with a warning — a file whose
///    name table and stem both yield nothing would otherwise emit an
///    empty variant and fail the crate build.
/// 2. **Redundant containers are collapsed.** Font hosts ship one
///    face as both `.ttf` and `.otf`, and `FontCandidate::preference`
///    picks which of the pair is embedded. This is the only pass
///    that discards a font, so it fires on exactly one shape: a
///    group of files agreeing on [`family_key`] **and** derived
///    variant name in which **every file sits in a different
///    container**. Silent, because one face published twice is the
///    expected shape of the tree rather than a problem.
///
///    The container test is what makes the rule safe, and it cannot
///    be replaced by comparing stems: hosts publish the pair with
///    per-container stems (`AppleTea-jELql.otf` beside
///    `AppleTea-z8R1a.ttf`), so the stems of one face routinely
///    differ. Two earlier keys were both wrong, in the same
///    direction — each discarded a file that pass 3 could have kept:
///
///    - [`family_key`] alone put `Family-Regular.ttf` and
///      `Family-Bold.ttf` in one group, so adding a style could
///      silently delete an `AppFont` variant that code elsewhere
///      referenced — breaking the build with a diagnostic naming an
///      unrelated font.
///    - `(family_key, variant)` still dropped one of two **distinct**
///      faces whenever both derived the same name *and* shipped in
///      the same container: two files whose name tables are
///      unreadable fall back to the same stem-derived identifier, and
///      the second vanished with no warning, silently repointing an
///      existing variant at a different face. Requiring distinct
///      containers closes that: two `.ttf`s are never redundant.
/// 3. **Distinct fonts that derive the same name are renamed**, not
///    dropped: the preferred one keeps the bare name and the rest
///    take the first free `Name2`, `Name3`, … suffix, each with a
///    warning. Dropping them would break the documented "drop a
///    font file in and the variant appears" workflow. The
///    [`RESERVED_VARIANTS`] names are taken up front, so a font
///    that derives `Any` or `Self` is renamed rather than emitting
///    an enum that does not compile.
///
///    Suffixes are assigned in variant order and skip names already
///    spoken for, so a suffixed name can displace a font that
///    legitimately derives it: two fonts deriving `Regal` plus one
///    deriving `Regal 2` emit `Regal`, `Regal2`, `Regal22`, with
///    the genuine `Regal 2` pushed to `Regal22` if it sorts last.
///    Deterministic and warned about, which is what matters here;
///    picking a "better" claimant would mean ranking a derived name
///    against a generated one, and there is no principled order.
///
/// Costs: O(n log n) in the number of font files, a handful of
/// `String` clones per file. Runs once per build-script execution.
pub fn select_font_variants(candidates: Vec<FontCandidate>) -> FontSelection {
    let mut selection = FontSelection::default();

    let mut named: Vec<FontCandidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.variant.is_empty() {
            selection.warnings.push(format!(
                "font '{}' yields no legal variant name from either its name table or its \
                 file name; skipping it",
                candidate.relative_path
            ));
        } else {
            named.push(candidate);
        }
    }
    // Sorting the skip warnings separately keeps them ordered even
    // though the walk that produced them is not.
    selection.warnings.sort();

    // Group on (family, derived name), then collapse a group only
    // when every file in it sits in a *different* container. That is
    // the one shape this pass exists for — the same face shipped as
    // `.ttf` and `.otf`, which font hosts publish with per-container
    // stems (`AppleTea-jELql.otf` beside `AppleTea-z8R1a.ttf`), so a
    // stem cannot identify a face and the extension has to.
    //
    // Two files sharing a container are two faces, however their
    // names collide: `Family-Regular.ttf` and `Family-Bold.ttf` with
    // unreadable name tables both derive `Family`. Collapsing those
    // silently repoints an existing variant at a different face. They
    // fall through to the rename pass below instead.
    let mut by_face: std::collections::BTreeMap<(String, String), Vec<FontCandidate>> =
        std::collections::BTreeMap::new();
    for candidate in named {
        let key = (candidate.family_key.clone(), candidate.variant.clone());
        by_face.entry(key).or_default().push(candidate);
    }

    let mut survivors: Vec<FontCandidate> = Vec::new();
    for (_, mut group) in by_face {
        let containers: std::collections::BTreeSet<&str> =
            group.iter().map(|font| font.extension.as_str()).collect();
        if containers.len() == group.len() {
            // One file per container: redundant copies of one face.
            group.sort_by(|left, right| {
                left.preference()
                    .cmp(&right.preference())
                    .then_with(|| left.relative_path.cmp(&right.relative_path))
            });
            survivors.push(group.into_iter().next().expect("group is never empty"));
        } else {
            survivors.extend(group);
        }
    }
    survivors.sort_by(|left, right| {
        left.variant
            .cmp(&right.variant)
            .then_with(|| left.preference().cmp(&right.preference()))
    });

    let mut taken: std::collections::BTreeSet<String> =
        RESERVED_VARIANTS.iter().map(|name| name.to_string()).collect();
    for candidate in survivors.iter_mut() {
        if taken.insert(candidate.variant.clone()) {
            continue;
        }
        let base = candidate.variant.clone();
        let mut ordinal = 2usize;
        let renamed = loop {
            let attempt = format!("{base}{ordinal}");
            if taken.insert(attempt.clone()) {
                break attempt;
            }
            ordinal += 1;
        };
        let reason = if RESERVED_VARIANTS.contains(&base.as_str()) {
            "is reserved"
        } else {
            "is already taken"
        };
        selection.warnings.push(format!(
            "font '{}' derives the variant name '{}', which {}; emitting it as '{}' instead",
            candidate.relative_path, base, reason, renamed
        ));
        candidate.variant = renamed;
    }

    survivors.sort_by(|left, right| left.variant.cmp(&right.variant));
    selection.fonts = survivors;
    selection
}
