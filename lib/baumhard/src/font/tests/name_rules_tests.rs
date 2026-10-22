// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::name_rules`] — the rules `build.rs`
//! follows when it turns `src/font/fonts/` into the `AppFont` enum.
//!
//! These exist because a build script is not compiled under
//! `cfg(test)`: the only way to test the scan's decisions is to keep
//! them in a module the library also compiles, which is exactly what
//! `name_rules` is. The heavy coverage sits on the two properties
//! the generated file's usefulness rests on — a **deterministic**
//! variant order (issue #18: two builds on one machine produced
//! completely different enum orders) and a **panic-free** name
//! decode (a UTF-16BE name table with one accented character used to
//! fail the build outright).
//!
//! The `do_*()` / `test_*()` split is the §B8 benchmark-reuse
//! pattern; `benches/test_bench.rs` imports the `do_*` bodies.

use crate::font::name_rules::{
    ascii_font_name, camel_case, capitalize_first, decode_name_record, extension_rank, fallback_sanitize,
    family_key, is_font_extension, is_usable_font_name, rotate_leading_digits, select_font_variants,
    variant_name, FontCandidate, NameEncoding, ANY_VARIANT, MAX_FALLBACK_LEN,
};

/// Encode `text` the way a Windows-platform `name` record does, so
/// the decode tests exercise real UTF-16BE bytes rather than a
/// hand-written byte array that could drift from the encoding.
fn utf16_be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// Build a candidate the way the build script does, with the same
/// path shape (`<family-dir>/<file>`), so ordering assertions read
/// like the real font tree.
fn candidate(relative_path: &str, full_name: Option<&str>) -> FontCandidate {
    let (stem, extension) = relative_path
        .rsplit_once('/')
        .map_or(relative_path, |(_, file)| file)
        .rsplit_once('.')
        .expect("test candidate paths carry an extension");
    FontCandidate::new(
        format!("/fonts/{relative_path}"),
        relative_path.to_string(),
        stem,
        extension.to_string(),
        full_name,
    )
}

#[test]
fn test_decode_name_record_utf16_be_ascii() {
    do_decode_name_record_utf16_be_ascii();
}

/// A pure-ASCII Windows name record is UTF-16BE: every other byte is
/// `0x00`. The old scan read those bytes as UTF-8 and got away with
/// it — NUL is valid UTF-8 and the downstream ASCII filter dropped
/// it — which is why the panic below went unnoticed for so long.
/// Decoding through the encoding tag gives the clean string with no
/// interior NULs.
pub fn do_decode_name_record_utf16_be_ascii() {
    let bytes = utf16_be("Norse Bold");
    assert_eq!(bytes[0], 0x00);
    assert_eq!(decode_name_record(&bytes, NameEncoding::Utf16Be), "Norse Bold");
}

#[test]
fn test_decode_name_record_utf16_be_non_ascii_does_not_panic() {
    do_decode_name_record_utf16_be_non_ascii_does_not_panic();
}

/// The regression this module exists for. `é` is `0x00 0xE9` in
/// UTF-16BE, which is **not** valid UTF-8, so
/// `str::from_utf8(..).expect("Not UTF-8")` turned any font with an
/// accented name into a hard build failure. Decoding as UTF-16BE
/// yields the real name; the ASCII reduction then drops the accent,
/// leaving a usable variant name instead of a dead build.
pub fn do_decode_name_record_utf16_be_non_ascii_does_not_panic() {
    let bytes = utf16_be("Café Noir");
    assert!(std::str::from_utf8(&bytes).is_err());
    assert_eq!(decode_name_record(&bytes, NameEncoding::Utf16Be), "Café Noir");
    assert_eq!(variant_name(Some("Café Noir"), "cafe-noir"), "CafNoir");
}

#[test]
fn test_decode_name_record_survives_malformed_input() {
    do_decode_name_record_survives_malformed_input();
}

/// Three shapes a corrupt name table can take, none of which may
/// panic a build script: an odd byte count (a truncated UTF-16
/// record), an unpaired surrogate, and legacy high bytes. Each
/// degrades to something printable rather than unwinding.
pub fn do_decode_name_record_survives_malformed_input() {
    // Odd length: the trailing byte has no pair and is dropped.
    assert_eq!(
        decode_name_record(&[0x00, 0x41, 0x00], NameEncoding::Utf16Be),
        "A"
    );
    // Lone high surrogate: replaced, not rejected.
    assert_eq!(
        decode_name_record(&[0xD8, 0x00, 0x00, 0x41], NameEncoding::Utf16Be),
        "\u{FFFD}A"
    );
    // Macintosh Roman: ASCII passes through, the high half does not
    // survive the ASCII reduction anyway.
    assert_eq!(
        decode_name_record(&[b'M', b'a', b'c', 0x8E], NameEncoding::Legacy),
        "Mac\u{FFFD}"
    );
    assert_eq!(
        ascii_font_name(&decode_name_record(
            &[b'M', b'a', b'c', 0x8E],
            NameEncoding::Legacy
        )),
        "Mac"
    );
}

#[test]
fn test_ascii_font_name_reduces_to_identifier_material() {
    do_ascii_font_name_reduces_to_identifier_material();
}

/// Non-ASCII characters and punctuation go; whitespace runs collapse
/// to one space so [`camel_case`] sees clean word boundaries; the
/// result is trimmed at both ends. Internal capitalization is
/// preserved — `GalatiaSIL` and `NIGHTCROW` are real variant names
/// and a lowercasing pass would wreck them.
pub fn do_ascii_font_name_reduces_to_identifier_material() {
    assert_eq!(ascii_font_name("Alice In Wonderland"), "Alice In Wonderland");
    assert_eq!(ascii_font_name("  Toy\t\nTrain  "), "Toy Train");
    assert_eq!(ascii_font_name("R.C. Rocket (Demo)!"), "RC Rocket Demo");
    assert_eq!(ascii_font_name("Galatia SIL"), "Galatia SIL");
    assert_eq!(ascii_font_name("日本語"), "");
    assert_eq!(ascii_font_name("   "), "");
}

#[test]
fn test_camel_case_joins_and_rotates_digits() {
    do_camel_case_joins_and_rotates_digits();
}

/// Words are capitalized and joined; leading digits rotate to the
/// end because a Rust identifier cannot start with one. Both
/// rotations below are live in the tree today (`212 Keyboard`,
/// `2Peas Hearts Delight`), so a regression here renames real
/// variants.
///
/// See [`do_camel_case_capitalizes_after_rotating`] for the case
/// where rotation exposes a character that was not word-initial.
pub fn do_camel_case_joins_and_rotates_digits() {
    assert_eq!(
        camel_case("Alice In Wonderland").as_deref(),
        Some("AliceInWonderland")
    );
    assert_eq!(camel_case("212 Keyboard").as_deref(), Some("Keyboard212"));
    assert_eq!(
        camel_case("2Peas Hearts Delight").as_deref(),
        Some("PeasHeartsDelight2")
    );
    assert_eq!(
        camel_case("picto people Regular").as_deref(),
        Some("PictoPeopleRegular")
    );
}

#[test]
fn test_camel_case_capitalizes_after_rotating() {
    do_camel_case_capitalizes_after_rotating();
}

/// Rotation can promote a character that was in the middle of a
/// word, so the capitalization pass has to run *after* it. Every
/// name here has a leading digit run followed by a lowercase letter
/// — `8bit Wonder` joins to `8bitWonder`, and rotating alone leaves
/// `bitWonder8`, which rustc rejects with `variant should have an
/// upper camel case name`. Asserted as a property as well as by
/// example, because the failure is a lowercase first character and
/// nothing else about the name looks wrong.
pub fn do_camel_case_capitalizes_after_rotating() {
    assert_eq!(camel_case("8bit Wonder").as_deref(), Some("BitWonder8"));
    assert_eq!(camel_case("3d Blocks").as_deref(), Some("DBlocks3"));
    assert_eq!(camel_case("1st Place").as_deref(), Some("StPlace1"));
    assert_eq!(camel_case("2nd").as_deref(), Some("Nd2"));
    // The stem path rotates too, and owes the same guarantee.
    assert_eq!(fallback_sanitize("1abcd"), "Abcd1");
    assert_eq!(fallback_sanitize("8bit-wonder"), "BitWonder8");

    for name in ["8bit Wonder", "3d Blocks", "1st Place", "212 Keyboard"] {
        let variant = camel_case(name).expect("every sample yields a variant name");
        let first = variant.chars().next().expect("a non-empty variant name");
        assert!(
            first.is_ascii_uppercase(),
            "'{name}' derived '{variant}', which does not start with an uppercase letter"
        );
    }
}

#[test]
fn test_camel_case_rejects_unusable_names() {
    do_camel_case_rejects_unusable_names();
}

/// Empty, whitespace-only, and digits-only names cannot become an
/// identifier. Returning `None` — rather than an empty string the
/// generator would happily emit — is what routes these fonts to the
/// file-name fallback instead of into source that does not compile.
pub fn do_camel_case_rejects_unusable_names() {
    assert_eq!(camel_case(""), None);
    assert_eq!(camel_case("   "), None);
    assert_eq!(camel_case("1234"), None);
    assert_eq!(camel_case("12 34"), None);
    assert!(!is_usable_font_name("日本語"));
    assert!(is_usable_font_name("Norse"));
}

#[test]
fn test_rotate_leading_digits() {
    do_rotate_leading_digits();
}

/// Rotation moves the whole leading digit run, keeps the digits
/// (they are part of the font's identity), and is a no-op when the
/// name already starts with a letter.
pub fn do_rotate_leading_digits() {
    assert_eq!(rotate_leading_digits("212Keyboard"), "Keyboard212");
    assert_eq!(rotate_leading_digits("Flowers3"), "Flowers3");
    assert_eq!(rotate_leading_digits(""), "");
    assert_eq!(rotate_leading_digits("2000"), "2000");
}

#[test]
fn test_capitalize_first_preserves_internal_case() {
    do_capitalize_first_preserves_internal_case();
}

/// Only the first character is touched. `SIL`, `RC`, and `NIGHTCROW`
/// keep their shouting; a title-casing implementation would silently
/// rename three variants.
pub fn do_capitalize_first_preserves_internal_case() {
    assert_eq!(capitalize_first("picto"), "Picto");
    assert_eq!(capitalize_first("SIL"), "SIL");
    assert_eq!(capitalize_first("NIGHTCROW"), "NIGHTCROW");
    assert_eq!(capitalize_first(""), "");
    assert_eq!(capitalize_first("2Peas"), "2Peas");
}

#[test]
fn test_fallback_sanitize_from_file_stem() {
    do_fallback_sanitize_from_file_stem();
}

/// The file-name path, reached when a font's name table is missing
/// or unreadable. It takes the *stem*: sanitizing the full file name
/// used to leave the extension behind as the variant name, so a font
/// called `1234.ttf` produced the identifier `ttf`.
pub fn do_fallback_sanitize_from_file_stem() {
    assert_eq!(fallback_sanitize("some-broken-face"), "SomeBrokenFace");
    // A long first token is the name; the host's cache-busting
    // suffix is dropped rather than glued on.
    assert_eq!(fallback_sanitize("ToyTrain-WYdO"), "ToyTrain");
    // Short tokens are noise — until they are all there is.
    assert_eq!(fallback_sanitize("x-y-face"), "Face");
    assert_eq!(fallback_sanitize("a-b-c"), "ABC");
    // `1234.ttf` reaches here as the stem `1234`, which yields no
    // legal identifier at all; the caller skips the file.
    assert_eq!(fallback_sanitize("1234"), "");
    assert_eq!(fallback_sanitize("2000-font"), "Font2000");
    assert_eq!(fallback_sanitize(""), "");
}

#[test]
fn test_fallback_sanitize_respects_length_ceiling() {
    do_fallback_sanitize_respects_length_ceiling();
}

/// Font hosts hand out very long stems. The join stops at
/// [`MAX_FALLBACK_LEN`] on a token boundary, and a single oversized
/// token is truncated to the same ceiling rather than dropped —
/// dropping it would leave the font unnamable.
pub fn do_fallback_sanitize_respects_length_ceiling() {
    let joined = fallback_sanitize("abcd-efgh-ijkl-mnop-qrst-uvwx");
    assert!(
        joined.len() <= MAX_FALLBACK_LEN,
        "'{joined}' exceeds the {MAX_FALLBACK_LEN}-character ceiling"
    );
    assert_eq!(joined, "AbcdEfghIjklMnopQrst");
    let single = fallback_sanitize("supercalifragilisticexpialidocious");
    assert_eq!(single.len(), MAX_FALLBACK_LEN);
    assert_eq!(single, "Supercalifragilistic");
}

#[test]
fn test_variant_name_prefers_name_table_then_stem() {
    do_variant_name_prefers_name_table_then_stem();
}

/// The full derivation order: the font's own name wins, the stem is
/// the fallback, and an unusable name-table entry does not block the
/// fallback — the old scan's `expect("Not UTF-8")` made that
/// fallback unreachable for exactly the fonts that needed it.
pub fn do_variant_name_prefers_name_table_then_stem() {
    assert_eq!(variant_name(Some("Toy Train"), "ToyTrain-WYdO"), "ToyTrain");
    assert_eq!(variant_name(None, "ToyTrain-WYdO"), "ToyTrain");
    assert_eq!(variant_name(Some("   "), "ToyTrain-WYdO"), "ToyTrain");
    assert_eq!(variant_name(Some("日本語"), "ToyTrain-WYdO"), "ToyTrain");
    assert_eq!(variant_name(None, "1234"), "");
}

#[test]
fn test_font_extension_matching_is_case_insensitive() {
    do_font_extension_matching_is_case_insensitive();
}

/// `Foo.TTF` is a font. The old scan compared the extension
/// byte-for-byte against `"ttf"` / `"otf"`, so an upper-case
/// extension — what several font hosts ship — was silently skipped.
/// Rank orders the containers for collision resolution: TTF first.
pub fn do_font_extension_matching_is_case_insensitive() {
    assert_eq!(is_font_extension("ttf").as_deref(), Some("ttf"));
    assert_eq!(is_font_extension("TTF").as_deref(), Some("ttf"));
    assert_eq!(is_font_extension("Otf").as_deref(), Some("otf"));
    assert_eq!(is_font_extension("woff2"), None);
    assert!(extension_rank("ttf") < extension_rank("otf"));
    assert!(extension_rank("otf") < extension_rank("woff2"));
}

#[test]
fn test_family_key_groups_container_pairs() {
    do_family_key_groups_container_pairs();
}

/// Font hosts ship one face as both `.ttf` and `.otf` with different
/// random suffixes. The stem prefix up to the first `-`, lowercased,
/// is what brings the pair together — and what keeps `Norse` and
/// `NorseBold` apart, since the style is part of the stem prefix
/// there rather than after the dash.
///
/// The last pair is the key's blind spot, and the reason
/// [`select_font_variants`] never collapses on it alone: two styles
/// written as `Family-Style` share a key while being two different
/// faces.
pub fn do_family_key_groups_container_pairs() {
    assert_eq!(family_key("AppleTea-jELql"), "appletea");
    assert_eq!(family_key("AppleTea-z8R1a"), "appletea");
    assert_eq!(family_key("Treeroot-OVyJo"), "treeroot");
    assert_eq!(family_key("TreeRoot-ZVgOl"), "treeroot");
    assert_ne!(family_key("Norse-KaWl"), family_key("NorseBold-2Kge"));
    assert_eq!(family_key("Novem"), "novem");
    assert_eq!(
        family_key("NotoSerifTibetan-Bold"),
        family_key("NotoSerifTibetan-Regular"),
        "the key cannot tell two styles apart; the selection pass must not rely on it alone"
    );
}

/// The five-file sample the selection tests share: two containers of
/// one family, two faces of another, and a lone face. Returned in an
/// order no filesystem would produce, so a test that passes only
/// because the input happened to be sorted fails here.
fn scrambled_sample() -> Vec<FontCandidate> {
    vec![
        candidate("toy-train-font/ToyTrain-WYdO.ttf", Some("Toy Train")),
        candidate("norse-font/NorseBold-2Kge.otf", Some("Norse Bold")),
        candidate("a-apple-tea-font/AppleTea-jELql.otf", Some("Apple Tea")),
        candidate("norse-font/Norse-KaWl.otf", Some("Norse")),
        candidate("a-apple-tea-font/AppleTea-z8R1a.ttf", Some("Apple Tea")),
    ]
}

#[test]
fn test_select_font_variants_sorts_by_variant_name() {
    do_select_font_variants_sorts_by_variant_name();
}

/// The generated enum lists variants in ascending name order. This
/// is the contract issue #18 was opened over: the scan used to
/// accumulate into a `HashMap` and emit `into_iter()` order, so two
/// builds on one machine produced completely different variant
/// orders — and so different discriminants, a different `FONT_DATA`
/// layout, and a generated diff that churned for no reason.
pub fn do_select_font_variants_sorts_by_variant_name() {
    let selection = select_font_variants(scrambled_sample());
    let names: Vec<&str> = selection.fonts.iter().map(|font| font.variant.as_str()).collect();
    assert_eq!(names, ["AppleTea", "Norse", "NorseBold", "ToyTrain"]);
}

#[test]
fn test_select_font_variants_is_order_independent() {
    do_select_font_variants_is_order_independent();
}

/// Determinism proper: the result depends on the input *set*, not on
/// the order a directory walk happens to yield. Sorting the output
/// alone would not be enough — the *which file wins* decisions have
/// to be order-independent too, which is why every permutation below
/// must agree on the embedded paths, not just on the names.
pub fn do_select_font_variants_is_order_independent() {
    let baseline = select_font_variants(scrambled_sample());
    let sample = scrambled_sample();
    for rotation in 0..sample.len() {
        let mut permuted = sample.clone();
        permuted.rotate_left(rotation);
        assert_eq!(
            select_font_variants(permuted.clone()),
            baseline,
            "rotation by {rotation} changed the selection"
        );
        permuted.reverse();
        assert_eq!(
            select_font_variants(permuted),
            baseline,
            "reversed rotation by {rotation} changed the selection"
        );
    }
    assert!(baseline.warnings.is_empty());
}

#[test]
fn test_select_font_variants_prefers_ttf_container() {
    do_select_font_variants_prefers_ttf_container();
}

/// `AppleTea` ships as both `.otf` and `.ttf` under one family key
/// *and* derives the same variant name from both, which is what
/// makes the pair redundant. One face, so one variant — and the TTF
/// is the one embedded, matching the preference the scan has always
/// had. Silent by design: this is the expected shape of the font
/// tree, not a problem worth a build warning.
pub fn do_select_font_variants_prefers_ttf_container() {
    let selection = select_font_variants(scrambled_sample());
    let apple_tea = selection
        .fonts
        .iter()
        .find(|font| font.variant == "AppleTea")
        .expect("AppleTea survives family dedup");
    assert_eq!(apple_tea.relative_path, "a-apple-tea-font/AppleTea-z8R1a.ttf");
    assert!(selection.warnings.is_empty());
}

#[test]
fn test_select_font_variants_breaks_ties_by_path() {
    do_select_font_variants_breaks_ties_by_path();
}

/// Two *distinct* files that derive the same name are both kept —
/// they are two faces, not one face in two containers — and the
/// lexicographically smallest relative path wins the bare name.
/// Relative, because the absolute path embeds a checkout directory
/// and would make the choice machine-dependent.
///
/// The rename is the point. An earlier revision keyed the collapse on
/// `(family_key, variant)` and silently discarded `Regal-vmJE`, the
/// same data loss as the `Family-Style` case reached through another
/// door. Only a genuine container pair may make a file disappear.
pub fn do_select_font_variants_breaks_ties_by_path() {
    let selection = select_font_variants(vec![
        candidate("regal-font/Regal-vmJE.ttf", Some("Regal")),
        candidate("regal-font/Regal-aaaa.ttf", Some("Regal")),
    ]);
    assert_eq!(selection.fonts.len(), 2, "two distinct faces must both survive");
    assert_eq!(selection.fonts[0].variant, "Regal");
    assert_eq!(selection.fonts[0].relative_path, "regal-font/Regal-aaaa.ttf");
    assert_eq!(selection.fonts[1].variant, "Regal2");
    assert_eq!(selection.fonts[1].relative_path, "regal-font/Regal-vmJE.ttf");
    assert_eq!(selection.warnings.len(), 1, "the rename is warned, not silent");
}

#[test]
fn test_select_font_variants_keeps_same_container_faces_of_one_family() {
    do_select_font_variants_keeps_same_container_faces_of_one_family();
}

/// Two faces of one family whose name tables are unreadable both fall
/// back to the same stem-derived identifier. They are still two
/// files, so both must survive under renamed variants.
///
/// This is the residual half of the `Family-Style.ttf` data loss:
/// narrowing the collapse key from `family_key` to
/// `(family_key, variant)` fixed the readable-name-table case but
/// left this one, where adding `Family-Bold.ttf` silently repointed
/// the existing `Family` variant at a different face — no build
/// break, no warning, just a different glyph set behind the same
/// name. Requiring distinct containers closes it.
pub fn do_select_font_variants_keeps_same_container_faces_of_one_family() {
    let selection = select_font_variants(vec![
        candidate("family-font/Family-Bold.ttf", None),
        candidate("family-font/Family-Regular.ttf", None),
    ]);
    assert_eq!(
        selection.fonts.len(),
        2,
        "two same-container faces must both survive"
    );
    assert_eq!(selection.fonts[0].variant, "Family");
    assert_eq!(selection.fonts[0].relative_path, "family-font/Family-Bold.ttf");
    assert_eq!(selection.fonts[1].variant, "Family2");
    assert_eq!(selection.fonts[1].relative_path, "family-font/Family-Regular.ttf");
}

#[test]
fn test_select_font_variants_collapses_one_face_in_two_containers() {
    do_select_font_variants_collapses_one_face_in_two_containers();
}

/// The case the collapse exists for, pinned so the stem key does not
/// over-correct into keeping both: one face shipped as `.ttf` and
/// `.otf` is one file per container, so exactly one is embedded — the
/// `.ttf`, by `FontCandidate::preference` — and nothing is warned,
/// because a redundant container is not a collision.
pub fn do_select_font_variants_collapses_one_face_in_two_containers() {
    let selection = select_font_variants(vec![
        candidate("regal-font/Regal.otf", Some("Regal")),
        candidate("regal-font/Regal.ttf", Some("Regal")),
    ]);
    assert_eq!(
        selection.fonts.len(),
        1,
        "one face in two containers is one variant"
    );
    assert_eq!(selection.fonts[0].relative_path, "regal-font/Regal.ttf");
    assert!(
        selection.warnings.is_empty(),
        "container dedup is not a collision"
    );
}

#[test]
fn test_select_font_variants_keeps_distinct_styles_of_one_family() {
    do_select_font_variants_keeps_distinct_styles_of_one_family();
}

/// The `Family-Style.ttf` shape — what Google Fonts ships and what
/// four fonts in this tree already use — puts two *different faces*
/// under one [`family_key`]. Collapsing on the key alone made
/// dropping the second one look like ordinary container dedup: add
/// `NotoSerifTibetan-Bold.ttf` beside the existing Regular and the
/// `NotoSerifTibetanRegular` variant vanished, taking twelve call
/// sites' worth of compilation with it, and the only build
/// diagnostic named an unrelated font.
///
/// Both styles must survive with their own variants, and silently:
/// two faces that derive two names are not a collision, so there is
/// nothing to warn about. The container pair in the same input still
/// collapses, which is the distinction the pass turns on.
pub fn do_select_font_variants_keeps_distinct_styles_of_one_family() {
    let selection = select_font_variants(vec![
        candidate(
            "noto-font/NotoSerifTibetan-Bold.ttf",
            Some("Noto Serif Tibetan Bold"),
        ),
        candidate(
            "noto-font/NotoSerifTibetan-Regular.ttf",
            Some("Noto Serif Tibetan Regular"),
        ),
        candidate("a-apple-tea-font/AppleTea-jELql.otf", Some("Apple Tea")),
        candidate("a-apple-tea-font/AppleTea-z8R1a.ttf", Some("Apple Tea")),
    ]);
    let names: Vec<&str> = selection.fonts.iter().map(|font| font.variant.as_str()).collect();
    assert_eq!(
        names,
        ["AppleTea", "NotoSerifTibetanBold", "NotoSerifTibetanRegular"],
        "two styles of one family are two faces; only the container pair is redundant"
    );
    assert!(
        selection.warnings.is_empty(),
        "distinct names are not a collision: {:?}",
        selection.warnings
    );
}

#[test]
fn test_select_font_variants_renames_name_collisions() {
    do_select_font_variants_renames_name_collisions();
}

/// Two genuinely different fonts can derive the same variant name.
/// They are renamed, not dropped: dropping one would break the
/// "drop a font file in and the variant appears" promise, and
/// emitting both under one name would not compile. The preferred
/// file keeps the bare name and the collision is reported.
pub fn do_select_font_variants_renames_name_collisions() {
    let selection = select_font_variants(vec![
        candidate("second-font/Second-0002.ttf", Some("Twin Face")),
        candidate("first-font/First-0001.ttf", Some("Twin Face")),
        candidate("third-font/Third-0003.ttf", Some("Twin Face")),
    ]);
    let names: Vec<&str> = selection.fonts.iter().map(|font| font.variant.as_str()).collect();
    assert_eq!(names, ["TwinFace", "TwinFace2", "TwinFace3"]);
    assert_eq!(
        selection.fonts[0].relative_path, "first-font/First-0001.ttf",
        "the lexicographically smallest path keeps the bare name"
    );
    assert_eq!(selection.warnings.len(), 2);
    assert!(
        selection.warnings[0].contains("second-font/Second-0002.ttf")
            && selection.warnings[0].contains("TwinFace2"),
        "collision warning names the file and its new variant: {}",
        selection.warnings[0]
    );
}

#[test]
fn test_select_font_variants_reserves_the_any_sentinel() {
    do_select_font_variants_reserves_the_any_sentinel();
}

/// A font whose name derives to `Any` would collide with the
/// no-preference sentinel and emit an enum with two `Any` variants.
/// The sentinel is reserved before any font is assigned, so the font
/// is renamed instead.
pub fn do_select_font_variants_reserves_the_any_sentinel() {
    let selection = select_font_variants(vec![candidate("any-font/Any-1234.ttf", Some("Any"))]);
    assert_eq!(selection.fonts.len(), 1);
    assert_eq!(selection.fonts[0].variant, format!("{ANY_VARIANT}2"));
    assert_eq!(selection.warnings.len(), 1);
    assert!(
        selection.warnings[0].contains("is reserved"),
        "the warning should say why the name was unavailable: {}",
        selection.warnings[0]
    );
}

#[test]
fn test_select_font_variants_reserves_the_self_keyword() {
    do_select_font_variants_reserves_the_self_keyword();
}

/// `Self` is the one Rust keyword this derivation can produce:
/// every other keyword is lowercase and [`capitalize_first`]
/// uppercases the first character, so `self` — from a name table or
/// from a file stem — arrives as `Self`. `enum AppFont { Any, Self
/// }` does not parse, and `Self` is not expressible as a raw
/// identifier either, so the name has to be refused somewhere.
///
/// It is refused here rather than in [`camel_case`], so the font is
/// renamed and still compiled in; skipping the file would lose it
/// over a name clash it did not choose. Every route to the name is
/// covered below, because the two derivation paths reject
/// separately.
pub fn do_select_font_variants_reserves_the_self_keyword() {
    for (path, full_name) in [
        ("self-font/Self-1234.ttf", Some("Self")),
        ("self-font/self-1234.ttf", Some("self")),
        ("self-font/self.ttf", None),
        ("self-font/Self.ttf", None),
    ] {
        let selection = select_font_variants(vec![candidate(path, full_name)]);
        assert_eq!(selection.fonts.len(), 1, "'{path}' should still be compiled in");
        assert_eq!(
            selection.fonts[0].variant, "Self2",
            "'{path}' must not emit the Rust keyword 'Self' as a variant"
        );
        assert_eq!(selection.warnings.len(), 1);
        assert!(selection.warnings[0].contains("is reserved"));
    }
}

#[test]
fn test_no_variant_is_a_rust_keyword() {
    do_no_variant_is_a_rust_keyword();
}

/// Belt and braces over the real generated enum: no compiled-in
/// font is named with a Rust keyword. The build would fail to parse
/// if one were, but the failure would point at generated source in
/// `OUT_DIR` with no hint of which font caused it, so the property
/// is asserted where it can be read.
///
/// The whole strict-and-reserved list is checked, not just `Self`.
/// Only `Self` is reachable while [`capitalize_first`] uppercases
/// the first character of every derived name — the rest are here so
/// that relaxing that (a font name arriving pre-lowercased, say)
/// fails as a named assertion rather than as a parse error in
/// generated source.
pub fn do_no_variant_is_a_rust_keyword() {
    const KEYWORDS: [&str; 51] = [
        "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for",
        "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "Self",
        "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
        "async", "await", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
        "typeof", "unsized", "virtual", "yield",
    ];
    for (app_font, _) in crate::font::fonts::FONT_DATA.iter() {
        let name = format!("{app_font:?}");
        assert!(
            !KEYWORDS.contains(&name.as_str()),
            "AppFont::{name} is a Rust keyword"
        );
    }
}

#[test]
fn test_select_font_variants_skips_unnamable_files() {
    do_select_font_variants_skips_unnamable_files();
}

/// A file that yields no legal identifier from either its name table
/// or its stem is skipped with a warning rather than emitted as an
/// empty variant — which would fail the crate build, not the scan.
/// The rest of the tree still generates.
pub fn do_select_font_variants_skips_unnamable_files() {
    let selection = select_font_variants(vec![
        candidate("odd-font/1234.ttf", None),
        candidate("regal-font/Regal-vmJE.ttf", Some("Regal")),
    ]);
    assert_eq!(selection.fonts.len(), 1);
    assert_eq!(selection.fonts[0].variant, "Regal");
    assert_eq!(selection.warnings.len(), 1);
    assert!(selection.warnings[0].contains("odd-font/1234.ttf"));
}

#[test]
fn test_font_candidate_display_name_falls_back_to_stem() {
    do_font_candidate_display_name_falls_back_to_stem();
}

/// The display name feeds the generated variant's doc comment. It
/// carries the font's own spacing (`Alice In Wonderland`) rather
/// than the squashed identifier, and degrades to the file stem when
/// the name table gave nothing.
pub fn do_font_candidate_display_name_falls_back_to_stem() {
    let named = candidate(
        "alice-in-wonderland-font/AliceInWonderland-1GzL0.ttf",
        Some("Alice In Wonderland"),
    );
    assert_eq!(named.display_name, "Alice In Wonderland");
    assert_eq!(named.variant, "AliceInWonderland");

    let unnamed = candidate("odd-font/SomeFace-abcd.ttf", None);
    assert_eq!(unnamed.display_name, "SomeFace-abcd");
    assert_eq!(unnamed.variant, "SomeFace");
}

#[test]
fn test_generated_app_font_table_is_ordered_and_unique() {
    do_generated_app_font_table_is_ordered_and_unique();
}

/// End-to-end check against the artifact `build.rs` actually emitted
/// for this compilation: `FONT_DATA` is in ascending variant order,
/// carries no duplicate variant and no duplicate payload, and leaves
/// the sentinel at discriminant zero.
///
/// The per-function tests above pin the rules; this one pins the
/// generated file, so a future change that bypasses
/// [`select_font_variants`] cannot quietly restore the
/// nondeterministic ordering. `Debug` is the variant name, which is
/// also what serde writes — the property being asserted is exactly
/// the one the on-disk format depends on.
pub fn do_generated_app_font_table_is_ordered_and_unique() {
    let names: Vec<String> = crate::font::fonts::FONT_DATA
        .iter()
        .map(|(app_font, _)| format!("{app_font:?}"))
        .collect();
    assert!(!names.is_empty(), "no fonts were compiled in");

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "FONT_DATA is not in ascending variant-name order");

    let unique: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "duplicate AppFont variant");
    assert!(
        !names.iter().any(|name| name == ANY_VARIANT),
        "the {ANY_VARIANT} sentinel must not carry font bytes"
    );

    assert_eq!(
        crate::font::fonts::AppFont::Any as usize,
        0,
        "the sentinel must stay at discriminant zero so adding a font cannot renumber it"
    );
}
