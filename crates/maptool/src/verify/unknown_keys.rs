// SPDX-License-Identifier: MPL-2.0

//! Keys the loader kept without understanding them.
//!
//! The loader's job is to not lose them; saying they are *wrong* is
//! this tool's job. `"min_zoom_to_rendr": 2.0` loads, warns once into
//! a log the author may never see, and is written back forever — so
//! without a check here a typo has no moment at which anybody finds
//! out. That was the strongest argument for reject-at-load and it is
//! answered here rather than by refusing the document.
//!
//! **Error, not warning.** A key nothing acts on is a key that does
//! nothing the author asked for, and `verify` exists to say so with a
//! nonzero exit. The map still loads and still renders; only the
//! verdict is severe.

use super::Violation;
use baumhard::mindmap::model::MindMap;

/// One violation per key the load did not recognize, stamped with the
/// part of the document it sits in.
///
/// Cost: O(preserved keys) — nothing at all for a map this build
/// authored, which carries none.
pub fn check(map: &MindMap) -> Vec<Violation> {
    map.unknown_keys
        .iter()
        .map(|entry| {
            Violation::at(
                "unknown_keys",
                entry.location(),
                format!(
                    "`{}` is not a key this build knows; it is preserved across saves but \
                     nothing reads it. Check the spelling, or the map was written by a \
                     newer version.",
                    entry.path_within_location()
                ),
            )
        })
        .collect()
}

/// One violation per construct the load could not read at all.
///
/// **The other half of the same bargain, and the severe half.** The
/// loader now skips a custom mutation or a trigger binding built on a
/// variant this build does not know, so a map from a newer version
/// opens instead of showing nothing. That is right for *loading* and
/// would be wrong for *validating*: `{"mutator": {"Glwo": …}}` is a
/// typo, and the moment somebody finds out about it has to exist.
/// This is that moment — a nonzero exit naming the variant serde
/// refused, on a file that nonetheless opens and renders.
///
/// Cost: O(skipped constructs) — nothing for a map this build
/// understood.
pub fn check_skipped_constructs(map: &MindMap) -> Vec<Violation> {
    map.skipped_constructs
        .iter()
        .map(|entry| {
            Violation::at(
                "unknown_variant",
                entry.location(),
                format!(
                    "this build cannot read this construct, so the load skipped it and \
                     nothing it describes runs: {}. It is preserved across saves. \
                     Check the spelling, or the map was written by a newer version.",
                    entry.reason()
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map with nothing unrecognized in it reports nothing — the
    /// negative control for the test below.
    #[test]
    fn test_a_map_with_no_preserved_keys_reports_nothing() {
        assert!(check(&MindMap::new_blank("t")).is_empty());
    }

    /// The key a load kept is the key `verify` complains about, named
    /// where the author has to go to fix it.
    #[test]
    fn test_a_preserved_key_is_reported_where_it_sits() {
        let json = r##"{
            "version": "1.0", "name": "typo",
            "canvas": {"background_color": "#000"},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [{"text": "n"}],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "min_zoom_to_rendr": 2.0
            }},
            "edges": []
        }"##;
        let map = baumhard::mindmap::loader::load_from_str(json).expect("the map must load");
        let found = check(&map);
        assert_eq!(found.len(), 1, "one preserved key, one violation: {found:?}");
        assert_eq!(found[0].location, "node \"0\"");
        assert!(
            found[0].message.contains("min_zoom_to_rendr"),
            "must name the key: {}",
            found[0].message
        );
    }

    /// **Loading and validating are different questions, and this is
    /// the answer to the second one.** The loader now opens a map
    /// whose custom mutation names a variant it has never heard of,
    /// which is right for a map from a newer build and wrong for a
    /// typo. `verify` is where the typo is still caught: nonzero, at
    /// the construct, naming the variant serde refused.
    #[test]
    fn test_a_skipped_construct_is_reported_with_the_variant_that_caused_it() {
        let json = r##"{
            "version": "1.0", "name": "future",
            "canvas": {"background_color": "#000"},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [{"text": "n"}],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": ""
            }},
            "edges": [],
            "custom_mutations": [
                {"id": "g", "name": "g", "target_scope": "SelfOnly",
                 "mutator": {"Glwo": {"channel": 0}}}
            ]
        }"##;
        let map =
            baumhard::mindmap::loader::load_from_str(json).expect("the map must open around the construct");
        assert!(
            check(&map).is_empty(),
            "an unreadable construct is not an unrecognized key; the two are reported \
             separately or a reader cannot tell a typo'd variant from a typo'd field"
        );
        let found = check_skipped_constructs(&map);
        assert_eq!(found.len(), 1, "one skipped construct, one violation: {found:?}");
        assert_eq!(found[0].location, "custom_mutations[0]");
        assert!(
            found[0].message.contains("Glwo"),
            "must name the variant the author has to fix: {}",
            found[0].message
        );

        // Through the verb the user actually runs, not just the check
        // in isolation — a check nothing calls reports nothing, and
        // `maptool verify` exiting 0 on a typo'd variant is the whole
        // failure this exists to prevent.
        let reported = crate::verify::verify(&map);
        assert!(
            reported
                .iter()
                .any(|v| v.category == "unknown_variant" && v.message.contains("Glwo")),
            "`verify` itself must carry the violation: {reported:?}"
        );
    }

    /// The negative control for the one above: a map this build reads
    /// completely reports nothing, so the check cannot be passing on
    /// an always-true condition.
    #[test]
    fn test_a_fully_readable_map_reports_no_skipped_construct() {
        assert!(check_skipped_constructs(&MindMap::new_blank("t")).is_empty());
    }

    /// **The door `verify` actually walks through.** Every test above
    /// hands `check` a map built by `loader::load_from_str` — the
    /// editor's strict door. `maptool verify` does not use it: it
    /// loads through `loader::parse_file_for_inspection`, precisely
    /// so it can still open the files the editor refuses.
    ///
    /// Those are two different parses, and only one of them was
    /// capturing. When the inspection path was a plain
    /// `serde_json::from_str`, an unrecognized key was dropped on the
    /// floor before `check` ever saw the map: `verify` reported
    /// **zero** violations and exited **0** on the map below, while
    /// every test above stayed green. A check is only as reachable as
    /// the load that feeds it, so this one goes end to end — real
    /// file, real `parse_file_for_inspection`, real `verify`.
    #[test]
    fn test_verify_reports_an_unknown_key_through_the_inspection_door() {
        let dir = baumhard::util::test_temp::TempDir::new("verify-inspection-door");
        let path = dir.join("probe.mindmap.json");
        let mut map = MindMap::new_blank("t");
        map.nodes
            .insert("0".into(), super::super::test_helpers::node("0", None));
        let mut raw = serde_json::to_value(&map).expect("serialize the map");
        raw["nodes"]["0"]["a_key_from_a_newer_build"] = serde_json::json!(42);
        std::fs::write(&path, serde_json::to_string_pretty(&raw).expect("render"))
            .expect("write the probe map");

        let loaded = baumhard::mindmap::loader::parse_file_for_inspection(&path)
            .expect("an unknown key must not stop the inspection load");
        let reported = crate::verify::verify(&loaded);
        assert!(
            reported
                .iter()
                .any(|v| { v.category == "unknown_keys" && v.message.contains("a_key_from_a_newer_build") }),
            "`verify` must report the key through its own load path: {reported:?}"
        );
        // The blank map is otherwise clean, so nothing else can be
        // supplying the nonzero exit this check is responsible for.
        let _ = &map;
    }
}
