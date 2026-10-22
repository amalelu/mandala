// SPDX-License-Identifier: MPL-2.0

use baumhard::core::tests::primitives_tests::*;
use baumhard::font::tests::attrs_tests::*;
use baumhard::font::tests::color_tests::*;
use baumhard::font::tests::fonts_tests::*;
use baumhard::font::tests::hex_tests::*;
use baumhard::font::tests::metric_cache_tests::*;
use baumhard::font::tests::metrics_tests::*;
use baumhard::font::tests::name_rules_tests::*;
use baumhard::gfx_structs::tests::area_tests::*;
use baumhard::gfx_structs::tests::bvh_descent_tests::*;
use baumhard::gfx_structs::tests::camera_tests::*;
use baumhard::gfx_structs::tests::delta_tests::*;
use baumhard::gfx_structs::tests::element_tests::*;
use baumhard::gfx_structs::tests::map_children_tests::*;
use baumhard::gfx_structs::tests::model_tests::*;
use baumhard::gfx_structs::tests::mutator_tests::*;
use baumhard::gfx_structs::tests::predicate_tests::*;
use baumhard::gfx_structs::tests::region_indexer_tests::*;
use baumhard::gfx_structs::tests::region_params_tests::*;
use baumhard::gfx_structs::tests::region_rect_tests::*;
use baumhard::gfx_structs::tests::scene_tests::*;
use baumhard::gfx_structs::tests::shape_tests::*;
use baumhard::gfx_structs::tests::spatial_descend_tests::*;
use baumhard::gfx_structs::tests::subtree_aabb_tests::*;
use baumhard::gfx_structs::tests::tree_tests::*;
use baumhard::gfx_structs::tests::tree_walker_tests::*;
use baumhard::gfx_structs::tests::zoom_visibility_tests::*;
use baumhard::util::tests::arena_utils_tests::*;
use baumhard::util::tests::color_tests::*;
use baumhard::util::tests::geometry_tests::*;
use baumhard::util::tests::grapheme_chad_tests::*;
use baumhard::util::tests::ordered_vec2_tests::*;
use baumhard::util::tests::primes_test::{do_is_prime_above_the_sieve_ceiling, do_primes};
use baumhard::util::tests::rust_source_tests::*;
use criterion::{criterion_group, criterion_main, Criterion};

use std::collections::HashMap;
use std::path::PathBuf;

use baumhard::mindmap::loader;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::{self, SceneSelectionContext};

/// Run every per-role projection pass for one frame, in the order
/// the application's `CanvasFrame::update_all` does — one shared
/// fold-hidden set, one clip-AABB pass, then the seven role passes.
///
/// The benchmarks below measure "the cost of a canvas rebuild", and
/// this is what a canvas rebuild *is* after the dual-pipeline
/// consolidation: there is no single entry point in the library any
/// more, because each role's data pass feeds its own canvas tree.
/// Keeping the sequence here rather than in the library is
/// deliberate — the library must not grow an orchestrator whose
/// only consumer is a benchmark.
#[allow(clippy::too_many_arguments)]
fn project_all_roles(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selection: SceneSelectionContext<'_>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
) {
    use baumhard::mindmap::scene_cache::EdgeKey;
    use baumhard::mindmap::tree_builder::BorderChromeOverrides;

    cache.ensure_zoom(camera_zoom);
    let hidden = map.fold_hidden_set();
    let node_aabbs = tree_builder::node_clip_aabbs(map, offsets, None, &hidden);
    // One memo across both connection passes, as `CanvasFrame` owns
    // one across its `update_*` methods — measuring them with a memo
    // each would measure a shape the app does not run.
    let mut paths = tree_builder::EdgePathCache::new(map, offsets);
    let _ = tree_builder::build_connection_elements(
        map,
        offsets,
        &node_aabbs,
        selection.edge,
        None,
        cache,
        &mut paths,
        camera_zoom,
        &hidden,
    );
    let highlight_key: Option<EdgeKey> = selection
        .edge_label
        .clone()
        .or_else(|| selection.edge.map(|(f, t, ty)| EdgeKey::new(f, t, ty)));
    let _ = tree_builder::build_label_elements(
        map,
        selection.label_edit,
        None,
        highlight_key.as_ref(),
        &mut paths,
        camera_zoom,
        &hidden,
    );
    let _ = tree_builder::build_section_frames(
        map,
        offsets,
        selection.node_edit_for,
        selection.focused_section,
        None,
        &hidden,
    );
    let _ =
        tree_builder::build_selected_node_handles(map, offsets, selection.selected_node_for_resize, &hidden);
    let _ = tree_builder::build_selected_section_handles(map, offsets, selection.selected_section, &hidden);
    let _ = tree_builder::border_node_data(
        map,
        offsets,
        BorderChromeOverrides {
            preview: None,
            node_edit_for: selection.node_edit_for,
        },
        &hidden,
    );
    let _ = tree_builder::portal_pair_data(
        map,
        offsets,
        selection.edge,
        selection.portal_label,
        None,
        None,
        camera_zoom,
        &hidden,
    );
}

/// Load the testament fixture for the drag-drain benchmark. Panics
/// if the fixture is missing — this is benchmark code, not a test,
/// and a missing fixture means the benchmark binary can't do its job.
fn load_testament_map() -> MindMap {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("maps/testament.mindmap.json");
    loader::load_from_file(&path).expect("testament map should load for bench")
}

/// One drain of the translate path: re-enter the role passes
/// with a fresh offset carrying the same delta for every dragged
/// node. The cache is already warm from the previous drain, so
/// every internal edge of the "subtree" falls into the translate
/// path.
fn do_subtree_drag_translate_path(
    map: &MindMap,
    cache: &mut SceneConnectionCache,
    dragged_ids: &[String],
    dx: f32,
    dy: f32,
    zoom: f32,
) {
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::with_capacity(dragged_ids.len());
    for id in dragged_ids {
        offsets.insert(id.clone(), (dx, dy));
    }
    project_all_roles(map, &offsets, SceneSelectionContext::default(), cache, zoom);
}

/// Baseline: simulate the pre-translate-path behavior by clearing
/// the cache before every drain. Every edge falls into the slow
/// path (`build_connection_path` + `sample_path`). The ratio
/// between this and `do_subtree_drag_translate_path` is the
/// headline number the translate path buys.
fn do_subtree_drag_slow_path(
    map: &MindMap,
    cache: &mut SceneConnectionCache,
    dragged_ids: &[String],
    dx: f32,
    dy: f32,
    zoom: f32,
) {
    cache.clear();
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::with_capacity(dragged_ids.len());
    for id in dragged_ids {
        offsets.insert(id.clone(), (dx, dy));
    }
    project_all_roles(map, &offsets, SceneSelectionContext::default(), cache, zoom);
}

fn criterion_benchmark(c: &mut Criterion) {
    // glyph_model //
    c.bench_function("matrix_place_in_1", |b| b.iter(do_matrix_place_in_1));
    c.bench_function("matrix_place_in_2", |b| b.iter(do_matrix_place_in_2));
    c.bench_function("matrix_place_in_3", |b| b.iter(do_matrix_place_in_3));
    c.bench_function("matrix_place_in_multiline_component", |b| {
        b.iter(do_matrix_place_in_multiline_component)
    });
    c.bench_function("matrix_place_in_fusing_component", |b| {
        b.iter(do_matrix_place_in_fusing_component)
    });
    c.bench_function("matrix_add_assign_1", |b| b.iter(do_matrix_add_assign_1));
    c.bench_function("matrix_add_assign_2", |b| b.iter(do_matrix_add_assign_2));
    c.bench_function("line_add_assign_1", |b| b.iter(do_line_add_assign_1));
    c.bench_function("line_add_assign_2", |b| b.iter(do_line_add_assign_2));
    c.bench_function("line_add_assign_3", |b| b.iter(do_line_add_assign_3));
    c.bench_function("line_add_assign_4", |b| b.iter(do_line_add_assign_4));
    c.bench_function("line_ignore_initial_space_multibyte_indent", |b| {
        b.iter(do_line_ignore_initial_space_multibyte_indent)
    });
    c.bench_function("line_ignore_initial_space_crlf_indent", |b| {
        b.iter(do_line_ignore_initial_space_crlf_indent)
    });
    c.bench_function("line_ignore_initial_space_zwj_content", |b| {
        b.iter(do_line_ignore_initial_space_zwj_content)
    });
    c.bench_function("line_ignore_initial_space_sub_assign_rhs_longer_than_lhs", |b| {
        b.iter(do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs)
    });
    c.bench_function("line_ignore_initial_space_mul_assign_rhs_longer_than_lhs", |b| {
        b.iter(do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs)
    });
    c.bench_function(
        "line_ignore_initial_space_sub_assign_uses_lhs_color_when_present",
        |b| b.iter(do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present),
    );
    c.bench_function("line_ignore_initial_space_surplus_rhs_runs_append", |b| {
        b.iter(do_line_ignore_initial_space_surplus_rhs_runs_append)
    });
    c.bench_function(
        "line_ignore_initial_space_all_whitespace_rhs_paints_nothing",
        |b| b.iter(do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing),
    );
    c.bench_function("line_runs_paint_by_column_with_the_flag_off", |b| {
        b.iter(do_line_runs_paint_by_column_with_the_flag_off)
    });
    c.bench_function("line_surplus_runs_paint_at_their_own_rhs_offsets", |b| {
        b.iter(do_line_surplus_runs_paint_at_their_own_rhs_offsets)
    });
    c.bench_function("line_surplus_runs_overwrite_a_longer_lhs_in_place", |b| {
        b.iter(do_line_surplus_runs_overwrite_a_longer_lhs_in_place)
    });
    c.bench_function("overriding_insert_is_partition_independent", |b| {
        b.iter(do_overriding_insert_is_partition_independent)
    });
    c.bench_function("component_of_index", |b| b.iter(do_component_of_index));
    c.bench_function("index_of_component", |b| b.iter(do_index_of_component));
    c.bench_function("expanding_insert_1", |b| b.iter(do_expanding_insert_1));
    c.bench_function("expanding_insert_2", |b| b.iter(do_expanding_insert_2));
    c.bench_function("expanding_insert_3", |b| b.iter(do_expanding_insert_3));
    c.bench_function("expanding_insert_4", |b| b.iter(do_expanding_insert_4));
    c.bench_function("expanding_insert_5", |b| b.iter(do_expanding_insert_5));
    c.bench_function("expanding_insert_6", |b| b.iter(do_expanding_insert_6));
    c.bench_function("expanding_insert_7", |b| b.iter(do_expanding_insert_7));
    c.bench_function("overriding_insert_1", |b| b.iter(do_overriding_insert_1));
    c.bench_function("overriding_insert_2", |b| b.iter(do_overriding_insert_2));
    c.bench_function("overriding_insert_3", |b| b.iter(do_overriding_insert_3));
    c.bench_function("overriding_insert_4", |b| b.iter(do_overriding_insert_4));
    c.bench_function("overriding_insert_5", |b| b.iter(do_overriding_insert_5));
    c.bench_function("overriding_insert_6", |b| b.iter(do_overriding_insert_6));
    c.bench_function("overriding_insert_7", |b| b.iter(do_overriding_insert_7));
    c.bench_function("overriding_insert_8", |b| b.iter(do_overriding_insert_8));
    c.bench_function("overriding_insert_9", |b| b.iter(do_overriding_insert_9));
    c.bench_function("overriding_insert_10", |b| b.iter(do_overriding_insert_10));
    c.bench_function("overriding_insert_11", |b| b.iter(do_overriding_insert_11));
    c.bench_function("overriding_insert_12", |b| b.iter(do_overriding_insert_12));
    c.bench_function("overriding_insert_13", |b| b.iter(do_overriding_insert_13));
    c.bench_function("delta_glyph_line_assign_applies_through_apply_to", |b| {
        b.iter(do_delta_glyph_line_assign_applies_through_apply_to)
    });
    c.bench_function("delta_glyph_lines_assign_applies_through_apply_to", |b| {
        b.iter(do_delta_glyph_lines_assign_applies_through_apply_to)
    });
    c.bench_function("delta_glyph_line_delete_clears_line", |b| {
        b.iter(do_delta_glyph_line_delete_clears_line)
    });
    c.bench_function("model_layer_subtract_saturates_at_zero", |b| {
        b.iter(do_model_layer_subtract_saturates_at_zero)
    });
    c.bench_function("delta_glyph_matrix_noop_and_delete_ignore_payload", |b| {
        b.iter(do_delta_glyph_matrix_noop_and_delete_ignore_payload)
    });
    c.bench_function("delta_glyph_matrix_applies_repeatedly", |b| {
        b.iter(do_delta_glyph_matrix_applies_repeatedly)
    });
    c.bench_function("matrix_add_assign_absorbs_taller_rhs", |b| {
        b.iter(do_matrix_add_assign_absorbs_taller_rhs)
    });
    c.bench_function("matrix_destructive_assigns_drop_surplus_rhs_rows", |b| {
        b.iter(do_matrix_destructive_assigns_drop_surplus_rhs_rows)
    });
    c.bench_function("component_add_assign_appends_text", |b| {
        b.iter(do_component_add_assign_appends_text)
    });
    c.bench_function("model_rotate_moves_position_around_pivot", |b| {
        b.iter(do_model_rotate_moves_position_around_pivot)
    });
    // The three `model_tests.rs` bodies the section above left out:
    // the scalar multiply that has no `*_2` sibling, and the two
    // destructive-op pins that hold `Delete`/`Subtract` to *not*
    // growing a matrix to reach a line that is not there.
    c.bench_function("matrix_mul_assign_1", |b| b.iter(do_matrix_mul_assign_1));
    c.bench_function("delta_glyph_line_destructive_ops_do_not_grow_missing_line", |b| {
        b.iter(do_delta_glyph_line_destructive_ops_do_not_grow_missing_line)
    });
    c.bench_function(
        "delta_glyph_lines_destructive_ops_do_not_grow_missing_lines",
        |b| b.iter(do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines),
    );
    // shared delta plumbing //
    c.bench_function("area_field_tags_cover_every_field", |b| {
        b.iter(do_area_field_tags_cover_every_field)
    });
    c.bench_function("model_field_tags_cover_every_field", |b| {
        b.iter(do_model_field_tags_cover_every_field)
    });
    c.bench_function("same_type_ignores_payload", |b| {
        b.iter(do_same_type_ignores_payload)
    });
    c.bench_function("pipeline_tags_are_derived", |b| {
        b.iter(do_pipeline_tags_are_derived)
    });
    c.bench_function("delta_new_collapses_duplicates_and_defaults_to_noop", |b| {
        b.iter(do_delta_new_collapses_duplicates_and_defaults_to_noop)
    });
    c.bench_function("delta_add_merges_both_sides", |b| {
        b.iter(do_delta_add_merges_both_sides)
    });
    c.bench_function("operation_key_matches_control_variant", |b| {
        b.iter(do_operation_key_matches_control_variant)
    });
    c.bench_function("apply_ref_matches_apply", |b| b.iter(do_apply_ref_matches_apply));
    c.bench_function("apply_reaches_every_area_field", |b| {
        b.iter(do_apply_reaches_every_area_field)
    });
    c.bench_function("apply_reaches_every_model_field", |b| {
        b.iter(do_apply_reaches_every_model_field)
    });
    // glyph_area //
    c.bench_function("outline_assign_round_trip", |b| {
        b.iter(do_outline_assign_round_trip)
    });
    c.bench_function("outline_subtract_clears", |b| b.iter(do_outline_subtract_clears));
    c.bench_function("outline_changes_hash", |b| b.iter(do_outline_changes_hash));
    c.bench_function("outline_field_add_picks_rhs", |b| {
        b.iter(do_outline_field_add_picks_rhs)
    });
    c.bench_function("shape_default_is_rectangle", |b| {
        b.iter(do_shape_default_is_rectangle)
    });
    c.bench_function("shape_assign_round_trip", |b| b.iter(do_shape_assign_round_trip));
    c.bench_function("shape_subtract_resets_to_rectangle", |b| {
        b.iter(do_shape_subtract_resets_to_rectangle)
    });
    c.bench_function("shape_changes_hash", |b| b.iter(do_shape_changes_hash));
    c.bench_function("shape_field_add_picks_rhs", |b| {
        b.iter(do_shape_field_add_picks_rhs)
    });
    c.bench_function(
        "change_region_range_missing_region_warns_and_leaves_area_intact",
        |b| b.iter(do_change_region_range_missing_region_warns_and_leaves_area_intact),
    );
    c.bench_function("delta_text_delete_clears_text", |b| {
        b.iter(do_delta_text_delete_clears_text)
    });
    c.bench_function("delta_regions_delete_clears_regions", |b| {
        b.iter(do_delta_regions_delete_clears_regions)
    });
    c.bench_function("area_rotate_moves_position_around_pivot", |b| {
        b.iter(do_area_rotate_moves_position_around_pivot)
    });
    c.bench_function("area_rotate_about_self_is_identity", |b| {
        b.iter(do_area_rotate_about_self_is_identity)
    });
    c.bench_function("area_rotate_matches_siblings", |b| {
        b.iter(do_area_rotate_matches_siblings)
    });
    c.bench_function("area_rotate_command_applies", |b| {
        b.iter(do_area_rotate_command_applies)
    });
    c.bench_function("area_rotate_command_json_wire_shape", |b| {
        b.iter(do_area_rotate_command_json_wire_shape)
    });
    // The canonical eight-direction outline stamp — the offset
    // table every bordered area expands through.
    c.bench_function("outline_offsets_canonical_8_stamp", |b| {
        b.iter(do_outline_offsets_canonical_8_stamp)
    });
    // zoom_visibility //
    c.bench_function("zoom_visibility_unbounded_contains_full_camera_range", |b| {
        b.iter(do_unbounded_contains_full_camera_range)
    });
    c.bench_function("zoom_visibility_inclusive_band_on_every_authored_shape", |b| {
        b.iter(do_inclusive_band_on_every_authored_shape)
    });
    c.bench_function("zoom_visibility_inverted_band_never_contains", |b| {
        b.iter(do_inverted_band_never_contains)
    });
    c.bench_function("zoom_visibility_nan_zoom_never_contains", |b| {
        b.iter(do_nan_zoom_never_contains)
    });
    c.bench_function("zoom_visibility_try_new_enforces_invariants", |b| {
        b.iter(do_try_new_enforces_invariants)
    });
    c.bench_function("zoom_visibility_assign_round_trip", |b| {
        b.iter(do_zoom_visibility_assign_round_trip)
    });
    c.bench_function("zoom_visibility_subtract_resets_to_unbounded", |b| {
        b.iter(do_zoom_visibility_subtract_resets_to_unbounded)
    });
    c.bench_function("zoom_visibility_field_add_picks_rhs", |b| {
        b.iter(do_zoom_visibility_field_add_picks_rhs)
    });
    c.bench_function("zoom_visibility_changes_hash", |b| {
        b.iter(do_zoom_visibility_changes_hash)
    });
    c.bench_function("zoom_visibility_default_is_skipped_in_json", |b| {
        b.iter(do_zoom_visibility_default_is_skipped_in_json)
    });
    // shape math (point-in-shape / shape-vs-AABB) //
    c.bench_function("shape_from_style_string_known_names", |b| {
        b.iter(do_shape_from_style_string_known_names)
    });
    c.bench_function(
        "shape_from_style_string_empty_and_unknown_fall_back_to_rectangle",
        |b| b.iter(do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle),
    );
    // Written as `b.iter(do_*)` rather than `b.iter(|| do_*())`: the
    // closure form around a bare zero-argument call is
    // `clippy::redundant_closure`, and the sweep that cleared those
    // left none of them in this file. The `b.iter(|| …)` entries
    // further down are not the other spelling of this — each passes
    // an argument or builds per-iteration state, which a function
    // reference cannot express.
    c.bench_function("shape_every_known_spelling_is_non_warning", |b| {
        b.iter(do_shape_every_known_spelling_is_non_warning)
    });
    c.bench_function("shape_classification_partitions_by_warning", |b| {
        b.iter(do_shape_classification_partitions_by_warning)
    });
    c.bench_function("shape_variant_spellings_are_all_known", |b| {
        b.iter(do_shape_variant_spellings_are_all_known)
    });
    c.bench_function("shape_known_shapes_are_lowercase", |b| {
        b.iter(do_shape_known_shapes_are_lowercase)
    });
    c.bench_function("shape_unrecognized_spelling_still_warns", |b| {
        b.iter(do_shape_unrecognized_spelling_still_warns)
    });
    c.bench_function("shape_reporting_predicates_partition", |b| {
        b.iter(do_shape_reporting_predicates_partition)
    });
    c.bench_function("shape_report_routes_every_classification", |b| {
        b.iter(do_shape_report_routes_every_classification)
    });
    c.bench_function("shape_report_levels_are_warn_and_trace", |b| {
        b.iter(do_shape_report_levels_are_warn_and_trace)
    });
    c.bench_function("shape_classify_case_and_alias", |b| {
        b.iter(do_shape_classify_case_and_alias)
    });
    // Reads `format/enums.md` from disk on every iteration — same
    // caveat as the testament entry below: this is a file-read
    // number, kept for the §B8 one-bench-per-`do_*()` contract.
    c.bench_function("shape_format_doc_publishes_exactly_known_shapes", |b| {
        b.iter(do_shape_format_doc_publishes_exactly_known_shapes)
    });
    // Pure string work over an inline fixture, unlike its neighbor
    // above: this one controls where `shape_section_in` stops, so it
    // touches no file.
    c.bench_function("shape_section_stops_at_the_next_heading_of_any_level", |b| {
        b.iter(do_shape_section_stops_at_the_next_heading_of_any_level)
    });
    // Loads the demo map from disk on every iteration — a file-read
    // benchmark, not a classifier one. Kept in the harness for the
    // §B8 one-bench-per-`do_*()` contract; read its number as I/O.
    c.bench_function("shape_testament_map_has_no_unknown_shapes", |b| {
        b.iter(do_shape_testament_map_has_no_unknown_shapes)
    });
    c.bench_function("shape_rectangle_contains_local", |b| {
        b.iter(do_shape_rectangle_contains_local)
    });
    c.bench_function("shape_ellipse_contains_center_and_rim", |b| {
        b.iter(do_shape_ellipse_contains_center_and_rim)
    });
    c.bench_function("shape_ellipse_rejects_aabb_corners", |b| {
        b.iter(do_shape_ellipse_rejects_aabb_corners)
    });
    c.bench_function("shape_ellipse_handles_stretched_conic", |b| {
        b.iter(do_shape_ellipse_handles_stretched_conic)
    });
    c.bench_function("shape_degenerate_bounds_never_hit", |b| {
        b.iter(do_shape_degenerate_bounds_never_hit)
    });
    c.bench_function("shape_ellipse_intersects_aabb_fully_inside", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_fully_inside)
    });
    c.bench_function("shape_ellipse_intersects_aabb_corner_only", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_corner_only)
    });
    c.bench_function("shape_ellipse_intersects_aabb_straddling_rim", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_straddling_rim)
    });
    c.bench_function("shape_ellipse_intersects_aabb_fully_outside", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_fully_outside)
    });
    c.bench_function("shape_shader_ids_are_stable", |b| {
        b.iter(do_shape_shader_ids_are_stable)
    });
    // glyph_tree //
    c.bench_function("basics_solo_mutation", |b| b.iter(do_basics_solo_mutation));
    c.bench_function("model_block_commands", |b| b.iter(do_model_block_commands));
    c.bench_function("area_block_commands", |b| b.iter(do_area_block_commands));
    c.bench_function("complex_tree_mutation", |b| b.iter(do_complex_tree_mutation));
    c.bench_function("simple_tree_mutation", |b| b.iter(do_simple_tree_mutation));
    c.bench_function("repeat_while_skip_while", |b| b.iter(do_repeat_while_skip_while));
    c.bench_function("repeat_while_without_children_is_noop", |b| {
        b.iter(do_repeat_while_without_children_is_noop)
    });
    c.bench_function("event_propagation_complex_symmetric", |b| {
        b.iter(do_event_propagation_complex_symmetric)
    });
    c.bench_function("event_propagation_simple", |b| {
        b.iter(do_event_propagation_simple)
    });
    c.bench_function("mutator_macro_applies_all_mutations_in_order", |b| {
        b.iter(do_mutator_macro_applies_all_mutations_in_order)
    });
    c.bench_function("mutator_macro_empty_is_noop", |b| {
        b.iter(do_mutator_macro_empty_is_noop)
    });
    c.bench_function("mutator_void_is_noop_when_applied_directly", |b| {
        b.iter(do_mutator_void_is_noop_when_applied_directly)
    });
    c.bench_function("mutator_void_preserves_channel_alignment_in_tree_walk", |b| {
        b.iter(do_mutator_void_preserves_channel_alignment_in_tree_walk)
    });
    // `walk_tree` proper: channel alignment, RepeatWhile
    // continuation, and the deep-chain / wide-fan-out shapes. §B7
    // puts everything inside `walk_tree_from` under the hot-path
    // rules, so these are the bodies whose numbers those rules are
    // argued from.
    c.bench_function("macro_applies_all_mutations_in_order", |b| {
        b.iter(do_macro_applies_all_mutations_in_order)
    });
    c.bench_function("macro_with_empty_mutations_is_noop", |b| {
        b.iter(do_macro_with_empty_mutations_is_noop)
    });
    c.bench_function("mutation_none_is_noop", |b| b.iter(do_mutation_none_is_noop));
    c.bench_function("single_mutator_channel_filter_in_align_child_walks", |b| {
        b.iter(do_single_mutator_channel_filter_in_align_child_walks)
    });
    c.bench_function("direct_walk_at_mismatched_channels_is_noop", |b| {
        b.iter(do_direct_walk_at_mismatched_channels_is_noop)
    });
    c.bench_function("deep_chain_walk_reaches_every_node", |b| {
        b.iter(do_deep_chain_walk_reaches_every_node)
    });
    c.bench_function("wide_fan_out_applies_to_all_matching_siblings", |b| {
        b.iter(do_wide_fan_out_applies_to_all_matching_siblings)
    });
    c.bench_function("applying_same_delta_twice_accumulates", |b| {
        b.iter(do_applying_same_delta_twice_accumulates)
    });
    c.bench_function("mutation_is_deterministic_across_tree_clones", |b| {
        b.iter(do_mutation_is_deterministic_across_tree_clones)
    });
    c.bench_function("clone_preserves_unique_id_and_channel", |b| {
        b.iter(do_clone_preserves_unique_id_and_channel)
    });
    c.bench_function("repeat_while_aligns_non_ascending_target_channels", |b| {
        b.iter(do_repeat_while_aligns_non_ascending_target_channels)
    });
    c.bench_function(
        "repeat_while_merge_advance_does_not_drop_mutator_without_target",
        |b| b.iter(do_repeat_while_merge_advance_does_not_drop_mutator_without_target),
    );
    c.bench_function(
        "default_terminator_resumes_over_non_ascending_after_mutators",
        |b| b.iter(do_default_terminator_resumes_over_non_ascending_after_mutators),
    );
    // `Mutation` and `Instruction` at the single-node scale —
    // AreaDelta / AreaCommand / None, both `Instruction` variants,
    // `MutatorTree::apply_to`, and the `writes_the_same` predicate
    // that decides whether a delta is worth walking for at all.
    c.bench_function("mutation_area_delta_applies_field", |b| {
        b.iter(do_mutation_area_delta_applies_field)
    });
    c.bench_function("mutation_area_command_nudge_right", |b| {
        b.iter(do_mutation_area_command_nudge_right)
    });
    c.bench_function("mutation_noop_leaves_tree_unchanged", |b| {
        b.iter(do_mutation_noop_leaves_tree_unchanged)
    });
    c.bench_function("writes_the_same_is_reflexive_where_partial_eq_is_not", |b| {
        b.iter(do_writes_the_same_is_reflexive_where_partial_eq_is_not)
    });
    c.bench_function("writes_the_same_separates_values_that_write_differently", |b| {
        b.iter(do_writes_the_same_separates_values_that_write_differently)
    });
    c.bench_function("instruction_repeat_while_always_true", |b| {
        b.iter(do_instruction_repeat_while_always_true)
    });
    c.bench_function("instruction_rotate_while", |b| {
        b.iter(do_instruction_rotate_while)
    });
    c.bench_function("mutator_tree_applies_to_target", |b| {
        b.iter(do_mutator_tree_applies_to_target)
    });
    // regions //
    c.bench_function("region_params_new_sunny_day", |b| {
        b.iter(do_region_params_new_sunny_day)
    });
    c.bench_function("region_indexer_initialize", |b| {
        b.iter(do_region_indexer_initialize)
    });
    c.bench_function("region_indexer_insert_and_remove", |b| {
        b.iter(do_region_indexer_insert_and_remove)
    });
    c.bench_function("region_params_non_divisor_target", |b| {
        b.iter(do_region_params_non_divisor_target)
    });
    c.bench_function("region_params_pixel_to_region", |b| {
        b.iter(do_region_params_pixel_to_region)
    });
    c.bench_function("region_params_region_to_pixel", |b| {
        b.iter(do_region_params_region_to_pixel)
    });
    c.bench_function("region_rect_exhaustive_4x4_grid", |b| {
        b.iter(do_rect_exhaustive_4x4_grid)
    });
    // `RegionParams` pixel/region math — the rest of
    // `region_params_tests.rs`, whose four longest-standing entries
    // are above. Every one is arithmetic over a resolution and a
    // factor, which is what runs per pointer event once the index is
    // wired (§B6).
    c.bench_function("region_params_resolution_1x1", |b| {
        b.iter(do_region_params_resolution_1x1)
    });
    c.bench_function("region_params_factor_one", |b| {
        b.iter(do_region_params_factor_one)
    });
    c.bench_function("region_params_factor_equals_dimension", |b| {
        b.iter(do_region_params_factor_equals_dimension)
    });
    c.bench_function("region_params_pixel_region_roundtrip", |b| {
        b.iter(do_region_params_pixel_region_roundtrip)
    });
    c.bench_function("region_params_roundtrip_asymmetric", |b| {
        b.iter(do_region_params_roundtrip_asymmetric)
    });
    c.bench_function("region_params_boundary_pixels", |b| {
        b.iter(do_region_params_boundary_pixels)
    });
    c.bench_function("region_params_asymmetric", |b| {
        b.iter(do_region_params_asymmetric)
    });
    c.bench_function("region_params_very_wide", |b| b.iter(do_region_params_very_wide));
    c.bench_function("region_params_exhaustive_4x4", |b| {
        b.iter(do_region_params_exhaustive_4x4)
    });
    c.bench_function("region_params_exhaustive_12x12", |b| {
        b.iter(do_region_params_exhaustive_12x12)
    });
    c.bench_function("region_params_adapt_changes_resolution", |b| {
        b.iter(do_region_params_adapt_changes_resolution)
    });
    c.bench_function("region_params_adapt_chained", |b| {
        b.iter(do_region_params_adapt_chained)
    });
    // `RegionIndexer` insert / remove / query — the forward and
    // reverse index that makes selection highlighting O(log n)
    // instead of O(n) (§B6).
    c.bench_function("region_indexer_initialize_with", |b| {
        b.iter(do_region_indexer_initialize_with)
    });
    c.bench_function("region_indexer_initialize_zero", |b| {
        b.iter(do_region_indexer_initialize_zero)
    });
    c.bench_function("region_indexer_reinitialize_clears_forward_index", |b| {
        b.iter(do_region_indexer_reinitialize_clears_forward_index)
    });
    c.bench_function("region_indexer_multiple_elements_in_one_region", |b| {
        b.iter(do_region_indexer_multiple_elements_in_one_region)
    });
    c.bench_function("region_indexer_one_element_in_multiple_regions", |b| {
        b.iter(do_region_indexer_one_element_in_multiple_regions)
    });
    c.bench_function("region_indexer_empty_region_query", |b| {
        b.iter(do_region_indexer_empty_region_query)
    });
    c.bench_function("region_indexer_boundary_regions", |b| {
        b.iter(do_region_indexer_boundary_regions)
    });
    c.bench_function("region_indexer_duplicate_insert_is_idempotent", |b| {
        b.iter(do_region_indexer_duplicate_insert_is_idempotent)
    });
    c.bench_function("region_indexer_remove_nonexistent_is_noop", |b| {
        b.iter(do_region_indexer_remove_nonexistent_is_noop)
    });
    c.bench_function("region_indexer_no_reverse_index", |b| {
        b.iter(do_region_indexer_no_reverse_index)
    });
    c.bench_function("region_indexer_reverse_index_unknown_element", |b| {
        b.iter(do_region_indexer_reverse_index_unknown_element)
    });
    c.bench_function("region_indexer_reinitialize_stale_reverse_index", |b| {
        b.iter(do_region_indexer_reinitialize_stale_reverse_index)
    });
    c.bench_function("region_indexer_default", |b| b.iter(do_region_indexer_default));
    c.bench_function("region_indexer_element_id_zero", |b| {
        b.iter(do_region_indexer_element_id_zero)
    });
    c.bench_function("region_indexer_element_id_max", |b| {
        b.iter(do_region_indexer_element_id_max)
    });
    c.bench_function("region_indexer_clone_is_independent", |b| {
        b.iter(do_region_indexer_clone_is_independent)
    });
    c.bench_function("region_indexer_initialize_with_zero_axis", |b| {
        b.iter(do_region_indexer_initialize_with_zero_axis)
    });
    c.bench_function("region_indexer_remove_wrong_region_no_damage", |b| {
        b.iter(do_region_indexer_remove_wrong_region_no_damage)
    });
    c.bench_function("region_indexer_single_region", |b| {
        b.iter(do_region_indexer_single_region)
    });
    c.bench_function("region_indexer_insert_at_scale", |b| {
        b.iter(do_region_indexer_insert_at_scale)
    });
    // `calculate_regions_intersected_by_rectangle` across the grid
    // shapes: origin blocks, single cells, strips, offsets, the
    // out-of-bounds error paths, and the 12x12 exhaustive sweep whose
    // 4x4 sibling is above.
    c.bench_function("rect_origin_4x4_block", |b| b.iter(do_rect_origin_4x4_block));
    c.bench_function("rect_origin_4x5_block", |b| b.iter(do_rect_origin_4x5_block));
    c.bench_function("rect_origin_5x4_block", |b| b.iter(do_rect_origin_5x4_block));
    c.bench_function("rect_origin_3x2_block", |b| b.iter(do_rect_origin_3x2_block));
    c.bench_function("rect_single_cell", |b| b.iter(do_rect_single_cell));
    c.bench_function("rect_single_pixel_each_corner", |b| {
        b.iter(do_rect_single_pixel_each_corner)
    });
    c.bench_function("rect_single_pixel_center", |b| {
        b.iter(do_rect_single_pixel_center)
    });
    c.bench_function("rect_full_width_single_row", |b| {
        b.iter(do_rect_full_width_single_row)
    });
    c.bench_function("rect_full_height_single_column", |b| {
        b.iter(do_rect_full_height_single_column)
    });
    c.bench_function("rect_full_grid", |b| b.iter(do_rect_full_grid));
    c.bench_function("rect_thin_vertical_strip", |b| {
        b.iter(do_rect_thin_vertical_strip)
    });
    c.bench_function("rect_thin_horizontal_strip", |b| {
        b.iter(do_rect_thin_horizontal_strip)
    });
    c.bench_function("rect_offset_single_row", |b| b.iter(do_rect_offset_single_row));
    c.bench_function("rect_offset_multi_row", |b| b.iter(do_rect_offset_multi_row));
    c.bench_function("rect_offset_last_column_multi_row", |b| {
        b.iter(do_rect_offset_last_column_multi_row)
    });
    c.bench_function("rect_center_3x3", |b| b.iter(do_rect_center_3x3));
    c.bench_function("rect_bottom_right_2x2", |b| b.iter(do_rect_bottom_right_2x2));
    c.bench_function("rect_asymmetric_grid", |b| b.iter(do_rect_asymmetric_grid));
    c.bench_function("rect_start_after_end", |b| b.iter(do_rect_start_after_end));
    c.bench_function("rect_start_out_of_bounds", |b| {
        b.iter(do_rect_start_out_of_bounds)
    });
    c.bench_function("rect_end_out_of_bounds", |b| b.iter(do_rect_end_out_of_bounds));
    c.bench_function("rect_exhaustive_12x12_grid", |b| {
        b.iter(do_rect_exhaustive_12x12_grid)
    });
    // grapheme_chad //
    c.bench_function("slice_to_newline", |b| b.iter(do_slice_to_newline));
    c.bench_function("split_graphemes", |b| b.iter(do_split_graphemes));
    c.bench_function("find_byte_index_of_grapheme", |b| {
        b.iter(do_find_byte_index_of_grapheme)
    });
    c.bench_function("byte_indices_of_graphemes", |b| {
        b.iter(do_byte_indices_of_graphemes)
    });
    c.bench_function("apply_operation_metric_clamp", |b| {
        b.iter(do_apply_operation_clamps_the_shaper_metrics)
    });
    c.bench_function("font_metric_setters_clamp", |b| {
        b.iter(do_font_metric_setters_clamp_to_the_shaper_domain)
    });
    c.bench_function("replace_graphemes_until_newline", |b| {
        b.iter(do_replace_graphemes_until_newline)
    });
    c.bench_function("replace_substring_matches_the_byte_splice_reference", |b| {
        b.iter(do_replace_substring_matches_the_byte_splice_reference)
    });
    c.bench_function("replace_substring_refuses_a_mid_character_range", |b| {
        b.iter(do_replace_substring_refuses_a_mid_character_range)
    });
    c.bench_function(
        "split_off_graphemes_matches_the_collect_and_concat_reference",
        |b| b.iter(do_split_off_graphemes_matches_the_collect_and_concat_reference),
    );
    // Reads one module's source per iteration, the same shape as
    // `the_metric_cache_probes_without_owning_the_cluster` below:
    // after the first iteration what moves is the parse and the brace
    // walk, not the disk.
    c.bench_function("the_text_edit_primitives_carry_no_whole_buffer_copy", |b| {
        b.iter(do_the_text_edit_primitives_carry_no_whole_buffer_copy)
    });
    c.bench_function("count_grapheme_clusters", |b| b.iter(do_count_grapheme_clusters));
    c.bench_function("first_non_whitespace_grapheme", |b| {
        b.iter(do_first_non_whitespace_grapheme)
    });
    c.bench_function("find_nth_line_grapheme_indices", |b| {
        b.iter(do_find_nth_line_grapheme_indices)
    });
    c.bench_function("line_model_is_coherent", |b| b.iter(do_line_model_is_coherent));
    c.bench_function("remove_prefix_unicode", |b| b.iter(do_remove_prefix_unicode));
    c.bench_function("insert_new_lines", |b| b.iter(do_insert_new_lines));
    c.bench_function("push_spaces", |b| b.iter(do_push_spaces));
    c.bench_function("count_number_of_lines", |b| b.iter(do_count_number_of_lines));
    c.bench_function("truncate_unicode", |b| b.iter(do_truncate_unicode));
    c.bench_function("insert_str_at_grapheme", |b| b.iter(do_insert_str_at_grapheme));
    c.bench_function("insert_str_at_grapheme_counted", |b| {
        b.iter(do_insert_str_at_grapheme_counted)
    });
    c.bench_function("delete_grapheme_at", |b| b.iter(do_delete_grapheme_at));
    c.bench_function("grapheme_display_width", |b| b.iter(do_grapheme_display_width));
    c.bench_function("truncate_to_display_width", |b| {
        b.iter(do_truncate_to_display_width)
    });
    c.bench_function("truncate_to_byte_budget", |b| b.iter(do_truncate_to_byte_budget));
    c.bench_function("wrap_to_display_width", |b| b.iter(do_wrap_to_display_width));
    c.bench_function("word_left", |b| b.iter(do_word_left));
    c.bench_function("word_right", |b| b.iter(do_word_right));
    c.bench_function("prev_word_boundary_ws", |b| b.iter(do_prev_word_boundary_ws));
    c.bench_function("token_start_ws", |b| b.iter(do_token_start_ws));
    c.bench_function("take_graphemes", |b| b.iter(do_take_graphemes));
    c.bench_function("line_bounds_at", |b| b.iter(do_line_bounds_at));
    c.bench_function("insert_spaces", |b| b.iter(do_insert_spaces));
    c.bench_function("split_graphemes_owned", |b| b.iter(do_split_graphemes_owned));
    c.bench_function("join_graphemes", |b| b.iter(do_join_graphemes));
    // rust_source // — every `do_*()` the module exports, which is
    // the whole rule this file follows: an entry per body, and a
    // body that should not be measured is a plain `#[test]` rather
    // than a `do_*()` with an exemption. The tree-wide sweep is the
    // one that took that opt-out; `production_code` did not, and
    // reads one file per iteration like the two `shape_*` entries
    // above.
    c.bench_function("strip_comments", |b| {
        b.iter(do_strip_comments_removes_only_comments)
    });
    c.bench_function("strip_comments_preserves_line_count", |b| {
        b.iter(do_strip_comments_preserves_line_count)
    });
    c.bench_function("strip_comments_survives_unterminated_input", |b| {
        b.iter(do_strip_comments_survives_unterminated_input)
    });
    c.bench_function("above_test_modules", |b| {
        b.iter(do_above_test_modules_cuts_at_the_module_only)
    });
    c.bench_function("braced_block_after", |b| {
        b.iter(do_braced_block_after_matches_one_item)
    });
    c.bench_function("string_literals", |b| {
        b.iter(do_string_literals_returns_every_literal)
    });
    c.bench_function("blank_string_literals_keeps_code_and_offsets", |b| {
        b.iter(do_blank_string_literals_keeps_code_and_offsets)
    });
    c.bench_function("comment_text_keeps_comments_and_drops_code", |b| {
        b.iter(do_comment_text_keeps_comments_and_drops_code)
    });
    c.bench_function("statements", |b| b.iter(do_statements_split_at_the_right_places));
    c.bench_function("production_code_returns_code_without_prose", |b| {
        b.iter(do_production_code_returns_code_without_prose)
    });
    // geometry //
    c.bench_function("90_deg_rotation", |b| b.iter(do_90_deg_rotation));
    c.bench_function("180_deg_rotation", |b| b.iter(do_180_deg_rotation));
    c.bench_function("non_origin_pivot_rotation", |b| {
        b.iter(do_non_origin_pivot_rotation)
    });
    c.bench_function("0_deg_rotation", |b| b.iter(do_0_deg_rotation));
    c.bench_function("pixel_functions", |b| b.iter(do_pixel_functions));
    c.bench_function("aabb_contains_includes_every_boundary", |b| {
        b.iter(do_aabb_contains_includes_every_boundary)
    });
    c.bench_function("aabb_contains_rejects_on_each_axis_independently", |b| {
        b.iter(do_aabb_contains_rejects_on_each_axis_independently)
    });
    c.bench_function("almost_equal", |b| b.iter(do_almost_equal));
    c.bench_function("almost_equal_f64_is_tighter_than_its_f32_sibling", |b| {
        b.iter(do_almost_equal_f64_is_tighter_than_its_f32_sibling)
    });
    c.bench_function("almost_equal_vec2", |b| b.iter(do_almost_equal_vec2));
    c.bench_function("is_positive_finite", |b| b.iter(do_is_positive_finite));
    c.bench_function("is_non_negative_finite_f64", |b| {
        b.iter(do_is_non_negative_finite_f64)
    });
    c.bench_function("option_almost_equal", |b| b.iter(do_option_almost_equal));
    // font / metrics //
    c.bench_function("monospace_advance_scales_linearly", |b| {
        b.iter(do_monospace_advance_scales_linearly)
    });
    // font / metric cache //
    c.bench_function("glyph_advance_cache_hit_matches_miss", |b| {
        b.iter(do_glyph_advance_cache_hit_matches_miss)
    });
    c.bench_function("glyph_advance_distinct_per_grapheme", |b| {
        b.iter(do_glyph_advance_distinct_per_grapheme)
    });
    c.bench_function("cluster_width_sums_per_grapheme", |b| {
        b.iter(do_cluster_width_sums_per_grapheme)
    });
    c.bench_function("glyph_advance_scales_with_size", |b| {
        b.iter(do_glyph_advance_scales_with_size)
    });
    c.bench_function("glyph_ink_distinct_per_size", |b| {
        b.iter(do_glyph_ink_distinct_per_size)
    });
    c.bench_function("glyph_ink_distinct_per_grapheme", |b| {
        b.iter(do_glyph_ink_distinct_per_grapheme)
    });
    c.bench_function("glyph_advance_with_shapes_cold_key_under_held_guard", |b| {
        b.iter(do_glyph_advance_with_shapes_cold_key_under_held_guard)
    });
    c.bench_function("glyph_ink_with_cold_key_under_held_guard", |b| {
        b.iter(do_glyph_ink_with_cold_key_under_held_guard)
    });
    c.bench_function("glyph_advance_distinct_per_face", |b| {
        b.iter(do_glyph_advance_distinct_per_face)
    });
    // Reads one module's source per iteration, the same shape as
    // `every_hex_entry_point_is_downstream_of_the_one_parser` above:
    // after the first iteration what moves is the parse and the
    // brace walk, not the disk.
    c.bench_function("the_metric_cache_probes_without_owning_the_cluster", |b| {
        b.iter(do_the_metric_cache_probes_without_owning_the_cluster)
    });
    // font / name rules //
    c.bench_function("decode_name_record_utf16_be_ascii", |b| {
        b.iter(do_decode_name_record_utf16_be_ascii)
    });
    c.bench_function("decode_name_record_utf16_be_non_ascii_does_not_panic", |b| {
        b.iter(do_decode_name_record_utf16_be_non_ascii_does_not_panic)
    });
    c.bench_function("decode_name_record_survives_malformed_input", |b| {
        b.iter(do_decode_name_record_survives_malformed_input)
    });
    c.bench_function("ascii_font_name_reduces_to_identifier_material", |b| {
        b.iter(do_ascii_font_name_reduces_to_identifier_material)
    });
    c.bench_function("camel_case_joins_and_rotates_digits", |b| {
        b.iter(do_camel_case_joins_and_rotates_digits)
    });
    c.bench_function("camel_case_capitalizes_after_rotating", |b| {
        b.iter(do_camel_case_capitalizes_after_rotating)
    });
    c.bench_function("camel_case_rejects_unusable_names", |b| {
        b.iter(do_camel_case_rejects_unusable_names)
    });
    c.bench_function("rotate_leading_digits", |b| b.iter(do_rotate_leading_digits));
    c.bench_function("capitalize_first_preserves_internal_case", |b| {
        b.iter(do_capitalize_first_preserves_internal_case)
    });
    c.bench_function("fallback_sanitize_from_file_stem", |b| {
        b.iter(do_fallback_sanitize_from_file_stem)
    });
    c.bench_function("fallback_sanitize_respects_length_ceiling", |b| {
        b.iter(do_fallback_sanitize_respects_length_ceiling)
    });
    c.bench_function("variant_name_prefers_name_table_then_stem", |b| {
        b.iter(do_variant_name_prefers_name_table_then_stem)
    });
    c.bench_function("font_extension_matching_is_case_insensitive", |b| {
        b.iter(do_font_extension_matching_is_case_insensitive)
    });
    c.bench_function("family_key_groups_container_pairs", |b| {
        b.iter(do_family_key_groups_container_pairs)
    });
    c.bench_function("font_candidate_display_name_falls_back_to_stem", |b| {
        b.iter(do_font_candidate_display_name_falls_back_to_stem)
    });
    c.bench_function("select_font_variants_sorts_by_variant_name", |b| {
        b.iter(do_select_font_variants_sorts_by_variant_name)
    });
    c.bench_function("select_font_variants_is_order_independent", |b| {
        b.iter(do_select_font_variants_is_order_independent)
    });
    c.bench_function("select_font_variants_prefers_ttf_container", |b| {
        b.iter(do_select_font_variants_prefers_ttf_container)
    });
    c.bench_function("select_font_variants_breaks_ties_by_path", |b| {
        b.iter(do_select_font_variants_breaks_ties_by_path)
    });
    c.bench_function("select_font_variants_keeps_distinct_styles_of_one_family", |b| {
        b.iter(do_select_font_variants_keeps_distinct_styles_of_one_family)
    });
    c.bench_function(
        "select_font_variants_keeps_same_container_faces_of_one_family",
        |b| b.iter(do_select_font_variants_keeps_same_container_faces_of_one_family),
    );
    c.bench_function("select_font_variants_collapses_one_face_in_two_containers", |b| {
        b.iter(do_select_font_variants_collapses_one_face_in_two_containers)
    });
    c.bench_function("select_font_variants_renames_name_collisions", |b| {
        b.iter(do_select_font_variants_renames_name_collisions)
    });
    c.bench_function("select_font_variants_reserves_the_any_sentinel", |b| {
        b.iter(do_select_font_variants_reserves_the_any_sentinel)
    });
    c.bench_function("select_font_variants_reserves_the_self_keyword", |b| {
        b.iter(do_select_font_variants_reserves_the_self_keyword)
    });
    c.bench_function("no_variant_is_a_rust_keyword", |b| {
        b.iter(do_no_variant_is_a_rust_keyword)
    });
    c.bench_function("select_font_variants_skips_unnamable_files", |b| {
        b.iter(do_select_font_variants_skips_unnamable_files)
    });
    c.bench_function("generated_app_font_table_is_ordered_and_unique", |b| {
        b.iter(do_generated_app_font_table_is_ordered_and_unique)
    });
    // color //
    c.bench_function("from_hex", |b| b.iter(do_from_hex));
    c.bench_function("from_hex_lazy_static", |b| b.iter(do_from_hex_lazy_static));
    c.bench_function("from_hex_garbage_falls_back_to_black", |b| {
        b.iter(do_from_hex_garbage_falls_back_to_black)
    });
    c.bench_function("hex_to_color_parses_bytes_at_compile_time", |b| {
        b.iter(do_hex_to_color_parses_bytes_at_compile_time)
    });
    // Reads the two hex-bearing modules' source per iteration, the
    // same shape as `production_code_returns_code_without_prose`
    // above: after the first iteration what moves is the two parses
    // and the closure walk, not the disk.
    c.bench_function("every_hex_entry_point_is_downstream_of_the_one_parser", |b| {
        b.iter(do_every_hex_entry_point_is_downstream_of_the_one_parser)
    });
    c.bench_function("color_with_alpha_replaces_only_the_alpha_channel", |b| {
        b.iter(do_color_with_alpha_replaces_only_the_alpha_channel)
    });
    c.bench_function("hex_to_rgba_three_digit", |b| b.iter(do_hex_to_rgba_three_digit));
    c.bench_function("hex_to_rgba_four_digit", |b| b.iter(do_hex_to_rgba_four_digit));
    c.bench_function("hex_to_rgba_six_digit", |b| b.iter(do_hex_to_rgba_six_digit));
    c.bench_function("hex_to_rgba_eight_digit", |b| b.iter(do_hex_to_rgba_eight_digit));
    c.bench_function("hex_to_rgba_rejects_invalid_length", |b| {
        b.iter(do_hex_to_rgba_rejects_invalid_length)
    });
    c.bench_function("hex_to_rgba_rejects_non_hex_char", |b| {
        b.iter(do_hex_to_rgba_rejects_non_hex_char)
    });
    c.bench_function("cosmic_color_from_color_carries_every_channel", |b| {
        b.iter(do_cosmic_color_from_color_carries_every_channel)
    });
    c.bench_function("cosmic_color_from_color_agrees_with_the_float_bridge", |b| {
        b.iter(do_cosmic_color_from_color_agrees_with_the_float_bridge)
    });
    c.bench_function("hex_to_cosmic_color_round_trip", |b| {
        b.iter(do_hex_to_cosmic_color_round_trip)
    });
    c.bench_function("resolve_var_hit", |b| b.iter(do_resolve_var_hit));
    c.bench_function("resolve_var_miss_returns_raw", |b| {
        b.iter(do_resolve_var_miss_returns_raw)
    });
    c.bench_function("resolve_var_plain_hex_passes_through", |b| {
        b.iter(do_resolve_var_plain_hex_passes_through)
    });
    c.bench_function("resolve_var_malformed_passes_through", |b| {
        b.iter(do_resolve_var_malformed_passes_through)
    });
    c.bench_function("resolve_var_tolerates_whitespace_inside", |b| {
        b.iter(do_resolve_var_tolerates_whitespace_inside)
    });
    c.bench_function("resolve_var_single_level_no_recursion", |b| {
        b.iter(do_resolve_var_single_level_no_recursion)
    });
    c.bench_function("hex_to_rgba_safe_good_input", |b| {
        b.iter(do_hex_to_rgba_safe_good_input)
    });
    c.bench_function("hex_to_rgba_safe_garbage_returns_fallback", |b| {
        b.iter(do_hex_to_rgba_safe_garbage_returns_fallback)
    });
    c.bench_function("hex_with_alpha_scaled_halves_opaque_input", |b| {
        b.iter(do_hex_with_alpha_scaled_halves_opaque_input)
    });
    c.bench_function("hex_with_alpha_scaled_factor_one_round_trips", |b| {
        b.iter(do_hex_with_alpha_scaled_factor_one_round_trips)
    });
    c.bench_function("hex_with_alpha_scaled_factor_zero_zeros_alpha", |b| {
        b.iter(do_hex_with_alpha_scaled_factor_zero_zeros_alpha)
    });
    c.bench_function(
        "hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha",
        |b| b.iter(do_hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha),
    );
    c.bench_function("hex_with_alpha_scaled_preserves_rgb_on_8_char_input", |b| {
        b.iter(do_hex_with_alpha_scaled_preserves_rgb_on_8_char_input)
    });
    c.bench_function("hex_with_alpha_scaled_parse_failure_passes_through", |b| {
        b.iter(do_hex_with_alpha_scaled_parse_failure_passes_through)
    });
    c.bench_function("hex_with_alpha_scaled_composes", |b| {
        b.iter(do_hex_with_alpha_scaled_composes)
    });
    c.bench_function("hex_to_rgba_safe_with_alpha", |b| {
        b.iter(do_hex_to_rgba_safe_with_alpha)
    });
    c.bench_function("hex_to_rgba_safe_no_panic_on_malformed_batch", |b| {
        b.iter(do_hex_to_rgba_safe_no_panic_on_malformed_batch)
    });
    c.bench_function("hex_to_rgba_safe_short_hex_expands_each_nibble", |b| {
        b.iter(do_hex_to_rgba_safe_short_hex_expands_each_nibble)
    });
    c.bench_function("hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases", |b| {
        b.iter(do_hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases)
    });
    c.bench_function("resolve_var_large_theme_map_zero_copy_passthrough", |b| {
        b.iter(do_resolve_var_large_theme_map_zero_copy_passthrough)
    });
    c.bench_function("resolve_var_passthrough_on_unknown_is_verbatim", |b| {
        b.iter(do_resolve_var_passthrough_on_unknown_is_verbatim)
    });
    c.bench_function("hsv_to_rgb_primaries", |b| b.iter(do_hsv_to_rgb_primaries));
    c.bench_function("hsv_to_rgb_grayscale_ignores_hue", |b| {
        b.iter(do_hsv_to_rgb_grayscale_ignores_hue)
    });
    c.bench_function("hsv_to_rgb_wraps_hue", |b| b.iter(do_hsv_to_rgb_wraps_hue));
    c.bench_function("rgb_to_hsv_primaries", |b| b.iter(do_rgb_to_hsv_primaries));
    c.bench_function("hsv_hex_roundtrip_named_colors", |b| {
        b.iter(do_hsv_hex_roundtrip_named_colors)
    });
    c.bench_function("hex_to_hsv_safe_rejects_garbage", |b| {
        b.iter(do_hex_to_hsv_safe_rejects_garbage)
    });
    c.bench_function("hsv_to_hex_emits_six_char_format", |b| {
        b.iter(do_hsv_to_hex_emits_six_char_format)
    });
    c.bench_function("rgba_to_hex_drops_alpha_only_when_saturated_opaque", |b| {
        b.iter(do_rgba_to_hex_drops_alpha_only_when_saturated_opaque)
    });
    c.bench_function("color_add_wraps_per_channel_modulo_256", |b| {
        b.iter(do_color_add_wraps_per_channel_modulo_256)
    });
    c.bench_function("color_sub_wraps_underflow_modulo_256", |b| {
        b.iter(do_color_sub_wraps_underflow_modulo_256)
    });
    c.bench_function("color_mul_wraps_overflow_modulo_256", |b| {
        b.iter(do_color_mul_wraps_overflow_modulo_256)
    });
    c.bench_function("color_div_per_channel", |b| b.iter(do_color_div_per_channel));
    c.bench_function("color_to_float_does_not_collapse_mid_range_channels", |b| {
        b.iter(do_color_to_float_does_not_collapse_mid_range_channels)
    });
    c.bench_function("color_new_f32_to_float_round_trips_within_one_byte", |b| {
        b.iter(do_color_new_f32_to_float_round_trips_within_one_byte)
    });
    c.bench_function("is_valid_hex_color_matches_hex_parser", |b| {
        b.iter(do_is_valid_hex_color_matches_hex_parser)
    });
    c.bench_function("parse_var_name_tolerates_inner_whitespace", |b| {
        b.iter(do_parse_var_name_tolerates_inner_whitespace)
    });
    c.bench_function("parse_var_name_rejects_malformed_refs", |b| {
        b.iter(do_parse_var_name_rejects_malformed_refs)
    });
    // primitives //
    c.bench_function("overlaps", |b| b.iter(do_overlaps));
    c.bench_function("split_and_separate_1", |b| b.iter(do_split_and_separate_1));
    c.bench_function("split_and_separate_2", |b| b.iter(do_split_and_separate_2));
    c.bench_function("split_and_separate_truth_table", |b| {
        b.iter(do_split_and_separate_truth_table)
    });
    c.bench_function("split_and_separate_preserves_payload_on_both_halves", |b| {
        b.iter(do_split_and_separate_preserves_payload_on_both_halves)
    });
    c.bench_function("split_and_separate_overflow_drops_the_whole_call", |b| {
        b.iter(do_split_and_separate_overflow_drops_the_whole_call)
    });
    c.bench_function("split_and_separate_precondition_violations_propagate", |b| {
        b.iter(do_split_and_separate_precondition_violations_propagate)
    });
    c.bench_function("range_checked_push_right", |b| {
        b.iter(do_range_checked_push_right)
    });
    c.bench_function("shift_regions_after_overflow_drops_the_whole_call", |b| {
        b.iter(do_shift_regions_after_overflow_drops_the_whole_call)
    });
    c.bench_function("insert_regions_at_overflow_drops_the_whole_call", |b| {
        b.iter(do_insert_regions_at_overflow_drops_the_whole_call)
    });
    c.bench_function("insertion_primitives_differ_only_at_the_three_seams", |b| {
        b.iter(do_insertion_primitives_differ_only_at_the_three_seams)
    });
    c.bench_function("submit_region_drops_inverted_range", |b| {
        b.iter(do_submit_region_drops_inverted_range)
    });
    c.bench_function("single_span_empty_is_empty", |b| {
        b.iter(do_single_span_empty_is_empty)
    });
    c.bench_function("single_span_non_empty_covers_range", |b| {
        b.iter(do_single_span_non_empty_covers_range)
    });
    c.bench_function("single_span_none_color_none_font", |b| {
        b.iter(do_single_span_none_color_none_font)
    });
    c.bench_function("region_shift_and_shrink_disagree_at_the_seam", |b| {
        b.iter(do_region_shift_and_shrink_disagree_at_the_seam)
    });
    c.bench_function("shrink_regions_after_fully_right_shifts_left", |b| {
        b.iter(do_shrink_regions_after_fully_right_shifts_left)
    });
    c.bench_function("shrink_regions_after_spanning_region_absorbs", |b| {
        b.iter(do_shrink_regions_after_spanning_region_absorbs)
    });
    c.bench_function("shrink_regions_after_fully_inside_collapses", |b| {
        b.iter(do_shrink_regions_after_fully_inside_collapses)
    });
    c.bench_function("shrink_regions_after_left_partial_clamps", |b| {
        b.iter(do_shrink_regions_after_left_partial_clamps)
    });
    c.bench_function("shrink_regions_after_right_partial_clamps", |b| {
        b.iter(do_shrink_regions_after_right_partial_clamps)
    });
    c.bench_function("shrink_regions_after_zero_magnitude_is_noop", |b| {
        b.iter(do_shrink_regions_after_zero_magnitude_is_noop)
    });
    c.bench_function("insert_regions_at_straddling_region_absorbs", |b| {
        b.iter(do_insert_regions_at_straddling_region_absorbs)
    });
    c.bench_function("insert_regions_at_left_adjacent_region_absorbs", |b| {
        b.iter(do_insert_regions_at_left_adjacent_region_absorbs)
    });
    c.bench_function("insert_regions_at_shifts_right_regions", |b| {
        b.iter(do_insert_regions_at_shifts_right_regions)
    });
    c.bench_function("insert_regions_at_zero_position_shifts_all", |b| {
        b.iter(do_insert_regions_at_zero_position_shifts_all)
    });
    c.bench_function("insert_regions_at_empty_returns_false", |b| {
        b.iter(do_insert_regions_at_empty_returns_false)
    });
    c.bench_function("same_content_separates_a_recolor_from_equality", |b| {
        b.iter(do_same_content_separates_a_recolor_from_equality)
    });
    c.bench_function("same_content_compares_whole_tables", |b| {
        b.iter(do_same_content_compares_whole_tables)
    });
    // font / ink-bounds //
    c.bench_function("measure_glyph_ink_bounds_latin_has_positive_advance", |b| {
        b.iter(do_measure_glyph_ink_bounds_latin_has_positive_advance)
    });
    c.bench_function("measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing", |b| {
        b.iter(do_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing)
    });
    c.bench_function("measure_glyph_ink_bounds_empty_string_is_zero", |b| {
        b.iter(do_measure_glyph_ink_bounds_empty_string_is_zero)
    });
    c.bench_function("measure_glyph_ink_bounds_x_offset_from_advance_center", |b| {
        b.iter(do_measure_glyph_ink_bounds_x_offset_from_advance_center)
    });
    c.bench_function("measure_glyph_ink_bounds_reports_baseline_line_y", |b| {
        b.iter(do_measure_glyph_ink_bounds_reports_baseline_line_y)
    });
    c.bench_function("measure_glyph_ink_bounds_y_offset_from_box_center", |b| {
        b.iter(do_measure_glyph_ink_bounds_y_offset_from_box_center)
    });
    c.bench_function("measure_text_block_unbounded_empty_is_zero", |b| {
        b.iter(do_measure_text_block_unbounded_empty_is_zero)
    });
    c.bench_function("measure_text_block_unbounded_single_line_nonzero", |b| {
        b.iter(do_measure_text_block_unbounded_single_line_nonzero)
    });
    c.bench_function(
        "measure_text_block_unbounded_multiline_width_is_widest_line",
        |b| b.iter(do_measure_text_block_unbounded_multiline_width_is_widest_line),
    );
    c.bench_function("measure_text_block_unbounded_width_scales_with_font_size", |b| {
        b.iter(do_measure_text_block_unbounded_width_scales_with_font_size)
    });
    // font / region-attrs bridges //
    c.bench_function("attrs_list_from_empty_regions_yields_no_spans", |b| {
        b.iter(do_attrs_list_from_empty_regions_yields_no_spans)
    });
    c.bench_function("attrs_list_from_single_color_region_emits_one_span", |b| {
        b.iter(do_attrs_list_from_single_color_region_emits_one_span)
    });
    c.bench_function("attrs_list_from_two_regions_emits_two_spans", |b| {
        b.iter(do_attrs_list_from_two_regions_emits_two_spans)
    });
    c.bench_function("attrs_list_pins_family_name_when_region_carries_app_font", |b| {
        b.iter(do_attrs_list_pins_family_name_when_region_carries_app_font)
    });
    c.bench_function(
        "attrs_list_falls_back_to_monospace_when_region_has_no_font",
        |b| b.iter(do_attrs_list_falls_back_to_monospace_when_region_has_no_font),
    );
    c.bench_function(
        "rich_text_spans_empty_regions_yield_single_whole_text_span",
        |b| b.iter(do_rich_text_spans_empty_regions_yield_single_whole_text_span),
    );
    c.bench_function("rich_text_spans_two_regions_slice_text_per_range", |b| {
        b.iter(do_rich_text_spans_two_regions_slice_text_per_range)
    });
    c.bench_function("rich_text_spans_drop_zero_width_regions", |b| {
        b.iter(do_rich_text_spans_drop_zero_width_regions)
    });
    c.bench_function("rich_text_spans_color_override_recolors_every_span", |b| {
        b.iter(do_rich_text_spans_color_override_recolors_every_span)
    });
    c.bench_function(
        "rich_text_spans_color_override_applies_to_uncolored_region",
        |b| b.iter(do_rich_text_spans_color_override_applies_to_uncolored_region),
    );
    c.bench_function("rich_text_spans_color_override_drops_zero_width_regions", |b| {
        b.iter(do_rich_text_spans_color_override_drops_zero_width_regions)
    });
    c.bench_function("rich_text_spans_pin_family_name_when_region_has_app_font", |b| {
        b.iter(do_rich_text_spans_pin_family_name_when_region_has_app_font)
    });
    c.bench_function("rich_text_spans_no_family_pin_when_region_has_no_font", |b| {
        b.iter(do_rich_text_spans_no_family_pin_when_region_has_no_font)
    });
    c.bench_function("rich_text_spans_clamps_out_of_range_region_end", |b| {
        b.iter(do_rich_text_spans_clamps_out_of_range_region_end)
    });
    c.bench_function("rich_text_spans_clamps_fully_out_of_range_region", |b| {
        b.iter(do_rich_text_spans_clamps_fully_out_of_range_region)
    });
    c.bench_function("rich_text_spans_empty_text_with_region_yields_no_spans", |b| {
        b.iter(do_rich_text_spans_empty_text_with_region_yields_no_spans)
    });
    c.bench_function("rich_text_spans_into_refills_rather_than_appends", |b| {
        b.iter(do_rich_text_spans_into_refills_rather_than_appends)
    });
    c.bench_function("rich_text_spans_into_keeps_the_no_region_whole_text_span", |b| {
        b.iter(do_rich_text_spans_into_keeps_the_no_region_whole_text_span)
    });
    // The grapheme-boundary slicing paths on both bridges: a region
    // range that lands mid-cluster is the input that turns a byte
    // slice into a panic, and the ZWJ cases are the widest clusters
    // the app actually renders.
    c.bench_function("attrs_list_slice_at_zwj_grapheme_boundary", |b| {
        b.iter(do_attrs_list_slice_at_zwj_grapheme_boundary)
    });
    c.bench_function("rich_text_spans_slice_at_grapheme_boundary", |b| {
        b.iter(do_rich_text_spans_slice_at_grapheme_boundary)
    });
    c.bench_function("rich_text_spans_slice_at_zwj_grapheme_boundary", |b| {
        b.iter(do_rich_text_spans_slice_at_zwj_grapheme_boundary)
    });
    // font family enumeration / lookup //
    c.bench_function("list_loaded_families_is_nonempty_sorted_unique", |b| {
        b.iter(do_list_loaded_families_is_nonempty_sorted_unique)
    });
    c.bench_function("app_font_by_family_round_trips", |b| {
        b.iter(do_app_font_by_family_round_trips)
    });
    c.bench_function("app_font_by_family_unknown_returns_none", |b| {
        b.iter(do_app_font_by_family_unknown_returns_none)
    });
    c.bench_function("loaded_families_iter_matches_owned_list", |b| {
        b.iter(do_loaded_families_iter_matches_owned_list)
    });
    c.bench_function("family_name_of_round_trips", |b| {
        b.iter(do_family_name_of_round_trips)
    });
    // scene + hit-test //
    c.bench_function("descendant_at_hits_single_area", |b| {
        b.iter(do_descendant_at_hits_single_area)
    });
    c.bench_function("descendant_at_prefers_smallest", |b| {
        b.iter(do_descendant_at_prefers_smallest)
    });
    c.bench_function("descendant_near_grants_slack", |b| {
        b.iter(do_descendant_near_grants_slack)
    });
    c.bench_function("descendants_aabb", |b| {
        b.iter(do_descendants_aabb_covers_all_areas)
    });
    c.bench_function("descendants_aabb_invalidated_by_mutator", |b| {
        b.iter(do_descendants_aabb_cache_invalidated_by_mutator)
    });
    c.bench_function("scene_component_at", |b| b.iter(do_scene_insert_and_component_at));
    c.bench_function("scene_layer_order_hit_priority", |b| {
        b.iter(do_scene_layer_order_controls_hit_priority)
    });
    c.bench_function("scene_offset_hit_test", |b| {
        b.iter(do_scene_offset_is_applied_to_hit_test)
    });
    // Passed by path rather than wrapped in `|| f()` — the
    // surrounding file predates `clippy::redundant_closure` being
    // clean and carries 165 instances of the wrapped form; new
    // entries do not add to that count.
    c.bench_function("scene_component_in", |b| {
        b.iter(do_scene_component_in_scopes_the_hit_to_one_tree)
    });
    c.bench_function("scene_component_in_offset_visibility", |b| {
        b.iter(do_scene_component_in_honors_offset_and_visibility)
    });
    c.bench_function("scene_component_in_overlap_smallest_area", |b| {
        b.iter(do_scene_component_in_resolves_overlap_by_smallest_area)
    });
    // `Scene` bookkeeping around the hit tests above: the
    // empty-tree miss, visibility skipping, removal, ownership
    // hand-back, and the insertion-stable layer order the hit
    // priority rests on.
    c.bench_function("descendant_at_returns_none_on_empty_tree", |b| {
        b.iter(do_descendant_at_returns_none_on_empty_tree)
    });
    c.bench_function("scene_invisible_trees_are_skipped", |b| {
        b.iter(do_scene_invisible_trees_are_skipped)
    });
    c.bench_function("scene_remove_drops_entry", |b| {
        b.iter(do_scene_remove_drops_entry)
    });
    c.bench_function("scene_entry_into_tree_returns_ownership", |b| {
        b.iter(do_scene_entry_into_tree_returns_ownership)
    });
    c.bench_function("scene_ids_in_layer_order_is_stable_by_insertion", |b| {
        b.iter(do_scene_ids_in_layer_order_is_stable_by_insertion)
    });
    // subtree AABBs (ensure/invalidate) //
    c.bench_function("subtree_aabb_leaf_equals_own_bounds", |b| {
        b.iter(do_subtree_aabb_leaf_equals_own_bounds)
    });
    c.bench_function("subtree_aabb_parent_encloses_children", |b| {
        b.iter(do_subtree_aabb_parent_encloses_children)
    });
    c.bench_function("subtree_aabb_root_encloses_entire_tree", |b| {
        b.iter(do_subtree_aabb_root_encloses_entire_tree)
    });
    c.bench_function("subtree_aabb_invalidated_by_mutation", |b| {
        b.iter(do_subtree_aabb_invalidated_by_mutation)
    });
    c.bench_function("subtree_aabb_void_tree_is_none", |b| {
        b.iter(do_subtree_aabb_void_tree_is_none)
    });
    c.bench_function("subtree_aabb_ensure_is_idempotent", |b| {
        b.iter(do_subtree_aabb_ensure_is_idempotent)
    });
    c.bench_function("subtree_aabb_deep_chain_propagates_to_root", |b| {
        b.iter(do_subtree_aabb_deep_chain_propagates_to_root)
    });
    c.bench_function("subtree_aabb_wide_tree_root_covers_all", |b| {
        b.iter(do_subtree_aabb_wide_tree_root_covers_all)
    });
    c.bench_function("subtree_aabb_single_area_node", |b| {
        b.iter(do_subtree_aabb_single_area_node)
    });
    c.bench_function("subtree_aabb_zero_bounds_area_ignored", |b| {
        b.iter(do_subtree_aabb_zero_bounds_area_ignored)
    });
    c.bench_function("subtree_aabb_negative_position", |b| {
        b.iter(do_subtree_aabb_negative_position)
    });
    c.bench_function("subtree_aabb_merge_none_base_with_child", |b| {
        b.iter(do_subtree_aabb_merge_none_base_with_child)
    });
    c.bench_function("subtree_aabb_merge_two_disjoint_children", |b| {
        b.iter(do_subtree_aabb_merge_two_disjoint_children)
    });
    c.bench_function("subtree_aabb_all_children_zero_bounds", |b| {
        b.iter(do_subtree_aabb_all_children_zero_bounds)
    });
    // BVH-accelerated descendant_at / descendant_near //
    c.bench_function("descendant_at_finds_leaf_via_bvh", |b| {
        b.iter(do_descendant_at_finds_leaf_via_bvh)
    });
    c.bench_function("descendant_at_prunes_disjoint_subtree", |b| {
        b.iter(do_descendant_at_prunes_disjoint_subtree)
    });
    c.bench_function("descendant_at_returns_none_on_miss", |b| {
        b.iter(do_descendant_at_returns_none_on_miss)
    });
    c.bench_function("descendant_at_smallest_area_wins", |b| {
        b.iter(do_descendant_at_smallest_area_wins)
    });
    c.bench_function("descendant_at_deep_chain_finds_leaf", |b| {
        b.iter(do_descendant_at_deep_chain_finds_leaf)
    });
    c.bench_function("descendant_at_deep_chain_miss", |b| {
        b.iter(do_descendant_at_deep_chain_miss)
    });
    c.bench_function("descendant_at_wide_tree_finds_correct_child", |b| {
        b.iter(do_descendant_at_wide_tree_finds_correct_child)
    });
    c.bench_function("descendant_at_wide_tree_gap_is_miss", |b| {
        b.iter(do_descendant_at_wide_tree_gap_is_miss)
    });
    c.bench_function("descendant_at_overlapping_siblings", |b| {
        b.iter(do_descendant_at_overlapping_siblings)
    });
    c.bench_function("descendant_near_negative_coords", |b| {
        b.iter(do_descendant_near_negative_coords)
    });
    c.bench_function("bvh_descend_skips_glyph_model_nodes", |b| {
        b.iter(do_bvh_descend_skips_glyph_model_nodes)
    });
    c.bench_function("bvh_descend_point_on_exact_boundary", |b| {
        b.iter(do_bvh_descend_point_on_exact_boundary)
    });
    c.bench_function("bvh_far_edge_hit_survives_float_rounding", |b| {
        b.iter(do_bvh_far_edge_hit_survives_float_rounding)
    });
    c.bench_function("bvh_point_in_subtree_aabb_but_outside_own_area", |b| {
        b.iter(do_bvh_point_in_subtree_aabb_but_outside_own_area)
    });
    c.bench_function("descendant_near_slack_expands_hit_region", |b| {
        b.iter(do_descendant_near_slack_expands_hit_region)
    });
    c.bench_function("descendant_near_slack_smallest_area_still_wins", |b| {
        b.iter(do_descendant_near_slack_smallest_area_still_wins)
    });
    // SpatialDescend event delivery //
    c.bench_function("spatial_descend_delivers_event_to_leaf", |b| {
        b.iter(do_spatial_descend_delivers_event_to_leaf)
    });
    c.bench_function("spatial_descend_miss_is_noop", |b| {
        b.iter(do_spatial_descend_miss_is_noop)
    });
    c.bench_function("spatial_descend_finds_innermost_node", |b| {
        b.iter(do_spatial_descend_finds_innermost_node)
    });
    c.bench_function("spatial_descend_deep_chain_delivers_to_leaf", |b| {
        b.iter(do_spatial_descend_deep_chain_delivers_to_leaf)
    });
    c.bench_function("spatial_descend_wide_tree_hits_correct_child", |b| {
        b.iter(do_spatial_descend_wide_tree_hits_correct_child)
    });
    c.bench_function("spatial_descend_no_mutation_is_noop", |b| {
        b.iter(do_spatial_descend_no_mutation_is_noop)
    });
    c.bench_function("spatial_descend_ignores_channel_mismatch", |b| {
        b.iter(do_spatial_descend_ignores_channel_mismatch)
    });
    c.bench_function("apply_to_redirties_subtree_aabbs_after_spatial_descend", |b| {
        b.iter(do_apply_to_redirties_subtree_aabbs_after_spatial_descend)
    });
    c.bench_function("walker_redirties_subtree_aabbs_between_spatial_descends", |b| {
        b.iter(do_walker_redirties_subtree_aabbs_between_spatial_descends)
    });
    c.bench_function("mouse_event_data_round_trips_constructor_inputs", |b| {
        b.iter(do_mouse_event_data_round_trips_constructor_inputs)
    });
    c.bench_function("glyph_tree_event_mouse_carries_data", |b| {
        b.iter(do_glyph_tree_event_mouse_carries_data)
    });
    // MapChildren zip walker //
    c.bench_function("one_to_one_zip_applies_each_child_by_position", |b| {
        b.iter(do_one_to_one_zip_applies_each_child_by_position)
    });
    c.bench_function("zip_shorter_mutator_than_target", |b| {
        b.iter(do_zip_shorter_mutator_than_target)
    });
    c.bench_function("zip_shorter_target_than_mutator", |b| {
        b.iter(do_zip_shorter_target_than_mutator)
    });
    c.bench_function("zip_empty_mutator_children_is_noop", |b| {
        b.iter(do_zip_empty_mutator_children_is_noop)
    });
    c.bench_function("zip_empty_target_children_is_noop", |b| {
        b.iter(do_zip_empty_target_children_is_noop)
    });
    c.bench_function("zip_empty_both_sides_is_noop", |b| {
        b.iter(do_zip_empty_both_sides_is_noop)
    });
    c.bench_function("nested_map_children_descends_recursively", |b| {
        b.iter(do_nested_map_children_descends_recursively)
    });
    c.bench_function("instruction_carrying_mutation_applies_to_current_target", |b| {
        b.iter(do_instruction_carrying_mutation_applies_to_current_target)
    });
    c.bench_function("instruction_mutation_skipped_on_channel_mismatch", |b| {
        b.iter(do_instruction_mutation_skipped_on_channel_mismatch)
    });
    c.bench_function("compose_repeat_inside_map_children", |b| {
        b.iter(do_compose_repeat_inside_map_children)
    });
    c.bench_function("map_children_ignores_sibling_channels_when_unequal_counts", |b| {
        b.iter(do_map_children_ignores_sibling_channels_when_unequal_counts)
    });
    // camera viewport math //
    c.bench_function("canvas_to_screen_round_trips", |b| {
        b.iter(do_canvas_to_screen_round_trips)
    });
    c.bench_function("screen_to_canvas_identity_at_default", |b| {
        b.iter(do_screen_to_canvas_identity_at_default)
    });
    c.bench_function("zoom_at_preserves_point_under_cursor", |b| {
        b.iter(do_zoom_at_preserves_point_under_cursor)
    });
    c.bench_function("pan_shifts_viewport", |b| b.iter(do_pan_shifts_viewport));
    c.bench_function("fit_to_bounds_with_single_element", |b| {
        b.iter(do_fit_to_bounds_with_single_element)
    });
    c.bench_function("fit_to_bounds_empty", |b| b.iter(do_fit_to_bounds_empty));
    c.bench_function("camera_mutation_pan", |b| b.iter(do_camera_mutation_pan));
    c.bench_function("camera_mutation_zoom", |b| b.iter(do_camera_mutation_zoom));
    c.bench_function("camera_mutation_set_zoom_clamps_to_bounds", |b| {
        b.iter(do_camera_mutation_set_zoom_clamps_to_bounds)
    });
    c.bench_function("camera_mutation_fit_to_bounds_matches_imperative", |b| {
        b.iter(do_camera_mutation_fit_to_bounds_matches_imperative)
    });
    c.bench_function("camera_mutation_set_position_assigns_directly", |b| {
        b.iter(do_camera_mutation_set_position_assigns_directly)
    });
    // predicates + comparators //
    c.bench_function("comparator_equal_f32", |b| b.iter(do_comparator_equal_f32));
    c.bench_function("comparator_not_equal_f32", |b| {
        b.iter(do_comparator_not_equal_f32)
    });
    c.bench_function("comparator_less_than_f32", |b| {
        b.iter(do_comparator_less_than_f32)
    });
    c.bench_function("comparator_greater_than_f32", |b| {
        b.iter(do_comparator_greater_than_f32)
    });
    c.bench_function("comparator_greater_equal_f32", |b| {
        b.iter(do_comparator_greater_equal_f32)
    });
    c.bench_function("comparator_less_equal_f32", |b| {
        b.iter(do_comparator_less_equal_f32)
    });
    c.bench_function("predicate_always_true_matches_anything", |b| {
        b.iter(do_predicate_always_true_matches_anything)
    });
    c.bench_function("predicate_matches_field_value", |b| {
        b.iter(do_predicate_matches_field_value)
    });
    c.bench_function("predicate_text_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_text_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_region_this_with_equals_degrades_to_false", |b| {
        b.iter(do_predicate_region_this_with_equals_degrades_to_false)
    });
    c.bench_function("predicate_region_font_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_region_font_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_region_color_with_less_than_degrades_to_false", |b| {
        b.iter(do_predicate_region_color_with_less_than_degrades_to_false)
    });
    c.bench_function("predicate_glyph_lines_with_equals_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_lines_with_equals_degrades_to_false)
    });
    c.bench_function(
        "predicate_glyph_matrix_with_greater_than_degrades_to_false",
        |b| b.iter(do_predicate_glyph_matrix_with_greater_than_degrades_to_false),
    );
    c.bench_function("predicate_glyph_matrix_with_less_than_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_matrix_with_less_than_degrades_to_false)
    });
    c.bench_function("predicate_glyph_area_field_on_void_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_area_field_on_void_degrades_to_false)
    });
    c.bench_function("predicate_flag_equals_matches_set_flag", |b| {
        b.iter(do_predicate_flag_equals_matches_set_flag)
    });
    c.bench_function("predicate_flag_equals_negated_matches_clear_flag", |b| {
        b.iter(do_predicate_flag_equals_negated_matches_clear_flag)
    });
    c.bench_function("predicate_flag_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_flag_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_truth_table", |b| b.iter(do_predicate_truth_table));
    // GfxElement constructors + accessors //
    c.bench_function("new_area_constructs_glyph_area_variant", |b| {
        b.iter(do_new_area_constructs_glyph_area_variant)
    });
    c.bench_function("new_void_constructs_void_variant", |b| {
        b.iter(do_new_void_constructs_void_variant)
    });
    c.bench_function("flags_accessor_round_trips", |b| {
        b.iter(do_flags_accessor_round_trips)
    });
    c.bench_function("subtree_aabb_set_and_read", |b| {
        b.iter(do_subtree_aabb_set_and_read)
    });
    c.bench_function("subtree_aabb_clone_resets_cache", |b| {
        b.iter(do_subtree_aabb_clone_resets_cache)
    });
    c.bench_function("event_subscribers_add_and_check", |b| {
        b.iter(do_event_subscribers_add_and_check)
    });
    c.bench_function("event_subscribers_observe_dispatched_event", |b| {
        b.iter(do_event_subscribers_observe_dispatched_event)
    });
    c.bench_function("event_subscriber_can_capture_rc_refcell_state", |b| {
        b.iter(do_event_subscriber_can_capture_rc_refcell_state)
    });
    c.bench_function(
        "event_subscriber_can_mutate_subscriber_list_during_delivery",
        |b| b.iter(do_event_subscriber_can_mutate_subscriber_list_during_delivery),
    );
    // ordered_vec2 //
    c.bench_function("ordered_vec2_round_trips_through_hashmap", |b| {
        b.iter(do_ordered_vec2_round_trips_through_hashmap)
    });
    c.bench_function("ordered_vec2_distinguishes_close_floats_in_hashset", |b| {
        b.iter(do_ordered_vec2_distinguishes_close_floats_in_hashset)
    });
    // arena_utils //
    c.bench_function("arena_utils_clone", |b| b.iter(do_clone));
    // primes //
    c.bench_function("primes", |b| b.iter(do_primes));
    c.bench_function("is_prime_above_the_sieve_ceiling", |b| {
        b.iter(do_is_prime_above_the_sieve_ceiling)
    });

    // subtree-drag drain at zoom 1 and 30. Caches are warmed outside
    // `iter()` so the first-frame cold miss doesn't dominate the sample.
    let bench_map = load_testament_map();
    let dragged_ids: Vec<String> = bench_map.nodes.keys().cloned().collect();
    let mut translate_cache_1 = SceneConnectionCache::new();
    do_subtree_drag_translate_path(&bench_map, &mut translate_cache_1, &dragged_ids, 0.0, 0.0, 1.0);
    let mut slow_cache_1 = SceneConnectionCache::new();
    c.bench_function("subtree_drag_translate_path_zoom_1", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_translate_path(&bench_map, &mut translate_cache_1, &dragged_ids, dx, dy, 1.0);
        })
    });
    c.bench_function("subtree_drag_slow_path_zoom_1", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_slow_path(&bench_map, &mut slow_cache_1, &dragged_ids, dx, dy, 1.0);
        })
    });
    let mut translate_cache_30 = SceneConnectionCache::new();
    do_subtree_drag_translate_path(&bench_map, &mut translate_cache_30, &dragged_ids, 0.0, 0.0, 30.0);
    let mut slow_cache_30 = SceneConnectionCache::new();
    c.bench_function("subtree_drag_translate_path_zoom_30", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_translate_path(&bench_map, &mut translate_cache_30, &dragged_ids, dx, dy, 30.0);
        })
    });
    c.bench_function("subtree_drag_slow_path_zoom_30", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_slow_path(&bench_map, &mut slow_cache_30, &dragged_ids, dx, dy, 30.0);
        })
    });
}

/// Inline a minimal `MindNode` constructor for the bench file —
/// `baumhard::mindmap::test_helpers::synthetic_node_full` is
/// `pub(crate)` so external benches can't reach it. Mirrors the
/// shape that helper produces (no border, simple style).
fn bench_node(
    id: &str,
    x: f64,
    sections: Vec<baumhard::mindmap::model::MindSection>,
) -> baumhard::mindmap::model::MindNode {
    use baumhard::mindmap::model::{MindNode, NodeLayout, NodeStyle, Position, Size};
    MindNode {
        id: id.to_string(),
        parent_id: None,
        position: Position { x, y: 0.0 },
        size: Size {
            width: 80.0,
            height: 40.0,
        },
        sections,
        style: NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape: "rectangle".into(),
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame: false,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: "map".into(),
            direction: "auto".into(),
            spacing: 0.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
        trigger_bindings: vec![],
        inline_mutations: vec![],
        inline_macros: Vec::new(),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

fn synthetic_single_section_map(node_count: usize) -> MindMap {
    use baumhard::mindmap::model::MindSection;
    let mut map = MindMap::new_blank("bench-single");
    for i in 0..node_count {
        let section = MindSection::new_default(format!("node {}", i), Vec::new());
        let node = bench_node(&format!("n{}", i), (i as f64) * 5.0, vec![section]);
        map.nodes.insert(node.id.clone(), node);
    }
    map
}

fn synthetic_multi_section_map(node_count: usize, sections_per_node: usize) -> MindMap {
    use baumhard::mindmap::model::MindSection;
    let mut map = MindMap::new_blank("bench-multi");
    for i in 0..node_count {
        let sections: Vec<MindSection> = (0..sections_per_node)
            .map(|s_idx| MindSection::new_default(format!("section {} of {}", s_idx, i), Vec::new()))
            .collect();
        let node = bench_node(&format!("m{}", i), (i as f64) * 5.0, sections);
        map.nodes.insert(node.id.clone(), node);
    }
    map
}

/// Same shape as [`synthetic_multi_section_map`] but every section
/// carries `runs_per_section` sized text runs, so a fold over them
/// has something to fold.
fn synthetic_multi_section_map_with_runs(
    node_count: usize,
    sections_per_node: usize,
    runs_per_section: usize,
) -> MindMap {
    use baumhard::mindmap::model::{MindSection, TextRun};
    let mut map = MindMap::new_blank("bench-multi-runs");
    for i in 0..node_count {
        let sections: Vec<MindSection> = (0..sections_per_node)
            .map(|s_idx| {
                let text = format!("section {} of {}", s_idx, i);
                let runs = (0..runs_per_section)
                    .map(|r| TextRun {
                        start: r,
                        end: r + 1,
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::new(),
                        size_pt: 10.0 + r as f32,
                        color: String::new(),
                        hyperlink: None,
                    })
                    .collect();
                MindSection::new_default(text, runs)
            })
            .collect();
        let node = bench_node(&format!("r{}", i), (i as f64) * 5.0, sections);
        map.nodes.insert(node.id.clone(), node);
    }
    map
}

fn do_build_mindmap_tree(map: &MindMap) {
    use baumhard::mindmap::tree_builder::build_mindmap_tree;
    let _ = build_mindmap_tree(map);
}

fn section_tree_build_benchmark(c: &mut Criterion) {
    // 243-node single-section map — the canonical "every node has
    // one default section" shape (every legacy / migrated map).
    let single_section = synthetic_single_section_map(243);
    c.bench_function("section_tree_build_243_single_section", |b| {
        b.iter(|| do_build_mindmap_tree(&single_section));
    });

    // 50-node × 5-section multi-section map — the heavy authoring
    // shape that the post-section refactor newly enables. The
    // ratio between this and the single-section benchmark is the
    // headline number for "how much does multi-section authoring
    // cost the tree builder?".
    let multi_section = synthetic_multi_section_map(50, 5);
    c.bench_function("section_tree_build_50_multi_section", |b| {
        b.iter(|| do_build_mindmap_tree(&multi_section));
    });

    // `effective_section_scale` runs once per section per tree
    // build and once more per section in the app's auto-size
    // measurement, so the row that matters is a whole map's worth
    // of sections rather than one call. The 5-run sections are the
    // fold's real shape — a run-less section short-circuits on the
    // empty iterator and would measure the fallback instead.
    let with_runs = synthetic_multi_section_map_with_runs(50, 5, 5);
    c.bench_function("effective_section_scale_over_250_sections", |b| {
        b.iter(|| {
            let mut total = 0.0f32;
            for node in with_runs.nodes.values() {
                for section in &node.sections {
                    total += baumhard::mindmap::tree_builder::effective_section_scale(section);
                }
            }
            total
        });
    });
}

/// One scene-rebuild pass at zoom 1 with the given resize-handle
/// override configuration. Compares the cost of the three regimes:
///
/// - **Default mode** — `selected_section: None`, `selected_node_for_resize: None`.
///   Pre-Batch-2 of `SECTIONS_BORDERS_RESIZE_PLAN.md`, this would have
///   been any selection that wasn't `Single`/`Section`. Today it's the
///   only thing emitting zero handles.
/// - **Resize { Node }** — `selected_node_for_resize = Some(id)`.
///   Triggers `build_selected_node_handles` (8 handle elements).
/// - **Resize { Section }** — `selected_section = Some((id, idx))`.
///   Triggers `build_selected_section_handles` (also 8, when sized).
fn do_scene_rebuild_with_handle_overrides(
    map: &MindMap,
    cache: &mut SceneConnectionCache,
    selected_node_for_resize: Option<&str>,
    selected_section: Option<(&str, usize)>,
) {
    project_all_roles(
        map,
        &HashMap::new(),
        SceneSelectionContext {
            selected_section,
            selected_node_for_resize,
            ..SceneSelectionContext::default()
        },
        cache,
        1.0,
    );
}

fn resize_mode_rebuild_benchmark(c: &mut Criterion) {
    // Plan §7.4: `bench_scene_rebuild_with_resize_mode_active`.
    // Three regimes pin the cost of handle emission against the
    // (pre-Batch-2-equivalent) no-handles baseline.
    let bench_map = load_testament_map();
    let any_node_id: String = bench_map
        .nodes
        .keys()
        .next()
        .cloned()
        .expect("testament map must have nodes");

    let mut default_cache = SceneConnectionCache::new();
    do_scene_rebuild_with_handle_overrides(&bench_map, &mut default_cache, None, None);
    c.bench_function("scene_rebuild_default_mode_no_handles", |b| {
        b.iter(|| {
            do_scene_rebuild_with_handle_overrides(&bench_map, &mut default_cache, None, None);
        })
    });

    let mut node_cache = SceneConnectionCache::new();
    do_scene_rebuild_with_handle_overrides(&bench_map, &mut node_cache, Some(any_node_id.as_str()), None);
    c.bench_function("scene_rebuild_resize_mode_node_target", |b| {
        b.iter(|| {
            do_scene_rebuild_with_handle_overrides(
                &bench_map,
                &mut node_cache,
                Some(any_node_id.as_str()),
                None,
            );
        })
    });

    // Section handles are only emitted for `Some`-sized sections.
    // The testament fixture uses single-section nodes whose sole
    // section has size = None (fill-parent), so the section pass
    // returns zero handles regardless. Bench is still useful: it
    // pins the cost of *checking* the section override path at
    // zero output.
    let mut section_cache = SceneConnectionCache::new();
    do_scene_rebuild_with_handle_overrides(
        &bench_map,
        &mut section_cache,
        None,
        Some((any_node_id.as_str(), 0)),
    );
    c.bench_function("scene_rebuild_resize_mode_section_target_fill_parent", |b| {
        b.iter(|| {
            do_scene_rebuild_with_handle_overrides(
                &bench_map,
                &mut section_cache,
                None,
                Some((any_node_id.as_str(), 0)),
            );
        })
    });
}

/// Plan §7.4: `bench_scene_rebuild_with_node_edit_mode_active`.
/// Pins the cost of the section-frame pass on a NodeEdit-active
/// rebuild. Uses the synthetic 50-node × 5-section fixture so
/// the section-frame chrome has real work to do (the testament
/// map's nodes are mostly single-section).
fn node_edit_mode_rebuild_benchmark(c: &mut Criterion) {
    let bench_map = synthetic_multi_section_map(50, 5);
    let any_node_id: String = bench_map
        .nodes
        .keys()
        .next()
        .expect("synthetic map has at least one node")
        .clone();
    let mut cache = SceneConnectionCache::new();
    // Warm cache once so the bench measures steady-state.
    project_all_roles(
        &bench_map,
        &HashMap::new(),
        SceneSelectionContext {
            node_edit_for: Some(any_node_id.as_str()),
            ..SceneSelectionContext::default()
        },
        &mut cache,
        1.0,
    );
    c.bench_function("scene_rebuild_node_edit_mode_active", |b| {
        b.iter(|| {
            project_all_roles(
                &bench_map,
                &HashMap::new(),
                SceneSelectionContext {
                    node_edit_for: Some(any_node_id.as_str()),
                    ..SceneSelectionContext::default()
                },
                &mut cache,
                1.0,
            );
        })
    });
}

/// Plan §7.4: `bench_fast_resize_anchor_inference`. The pure
/// quadrant-math helper; sub-microsecond per call. Pin against
/// the eight quadrant cases.
fn fast_resize_anchor_inference_benchmark(c: &mut Criterion) {
    use baumhard::mindmap::tree_builder::infer_resize_anchor;
    use glam::Vec2;
    let aabb_pos = Vec2::new(0.0, 0.0);
    let aabb_size = Vec2::new(200.0, 100.0);
    let cases = [
        Vec2::new(10.0, 10.0),
        Vec2::new(190.0, 10.0),
        Vec2::new(10.0, 90.0),
        Vec2::new(190.0, 90.0),
        Vec2::new(100.0, 10.0),
        Vec2::new(10.0, 50.0),
        Vec2::new(190.0, 50.0),
        Vec2::new(100.0, 90.0),
    ];
    c.bench_function("fast_resize_anchor_inference", |b| {
        b.iter(|| {
            for cursor in &cases {
                let _ = infer_resize_anchor(*cursor, aabb_pos, aabb_size);
            }
        })
    });
}

/// Plan §7.4: `bench_section_frame_emission`. Isolates the
/// section-frame scene-builder pass against a 50×5 synthetic
/// map; pins the chrome-emission cost separate from the rest of
/// the scene rebuild.
fn section_frame_emission_benchmark(c: &mut Criterion) {
    use baumhard::mindmap::tree_builder::build_section_frames;
    let bench_map = synthetic_multi_section_map(50, 5);
    let any_node_id: String = bench_map
        .nodes
        .keys()
        .next()
        .expect("synthetic map has at least one node")
        .clone();
    let offsets: HashMap<String, (f32, f32)> = HashMap::new();
    let hidden_set = bench_map.fold_hidden_set();
    c.bench_function("section_frame_emission_50x5_with_node_edit_active", |b| {
        b.iter(|| {
            let _ = build_section_frames(
                &bench_map,
                &offsets,
                Some(any_node_id.as_str()),
                None,
                None,
                &hidden_set,
            );
        })
    });
}

/// Inline a minimal `MindEdge` constructor for the bench file, for
/// the same reason [`bench_node`] exists — the crate's
/// `synthetic_*_edge` helpers are `pub(crate)`.
fn bench_edge(from: &str, to: &str, portal: bool, label: Option<&str>) -> baumhard::mindmap::model::MindEdge {
    use baumhard::mindmap::model::{GlyphConnectionConfig, MindEdge, DISPLAY_MODE_PORTAL};
    MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: "cross_link".into(),
        color: "#ff0000".into(),
        width: 3,
        line_style: "solid".into(),
        visible: true,
        label: label.map(str::to_string),
        label_config: None,
        anchor_from: "auto".into(),
        anchor_to: "auto".into(),
        control_points: vec![],
        glyph_connection: Some(GlyphConnectionConfig {
            body: "\u{25C8}".into(),
            font_size_pt: 16.0,
            ..GlyphConnectionConfig::default()
        }),
        display_mode: portal.then(|| DISPLAY_MODE_PORTAL.to_string()),
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// P1-34: the two canvas-role hit indexes. `resolve` runs on every
/// pointer event — §B1 puts hit tests under the same rules as
/// `walk_tree_from` — and its whole claim is that naming a hit is
/// O(1): two `parent()` hops plus a slice index for portals, one
/// arena lookup plus a slice index for labels. A change that turned
/// either into a scan over the index would show as a slope against
/// the fixture size, so both are benched against a deliberately wide
/// fixture with the hit taken on the *last* channel — the worst case
/// for a hypothetical scan and indistinguishable from the first for
/// the index lookup that is actually there.
fn hit_index_resolve_benchmark(c: &mut Criterion) {
    use baumhard::mindmap::tree_builder::{
        build_connection_label_tree, build_label_elements, build_portal_tree_from_pairs, portal_pair_data,
    };

    const PAIRS: usize = 60;
    let offsets: HashMap<String, (f32, f32)> = HashMap::new();

    // One hub joined to `PAIRS` partners by portal-mode edges, so
    // the index carries `PAIRS` entries.
    let mut portal_map = synthetic_single_section_map(PAIRS + 1);
    for i in 0..PAIRS {
        portal_map
            .edges
            .push(bench_edge("n0", &format!("n{}", i + 1), true, None));
    }
    let hidden = portal_map.fold_hidden_set();
    let pairs = portal_pair_data(&portal_map, &offsets, None, None, None, None, 1.0, &hidden);
    assert_eq!(pairs.len(), PAIRS, "portal bench fixture did not build");
    let portal = build_portal_tree_from_pairs(&pairs);
    // Deepest channel path: last pair, second endpoint, text slot.
    let last_leaf = {
        let t = &portal.tree;
        let pair = t.root.children(&t.arena).nth(PAIRS - 1).expect("last pair");
        let endpoint = pair.children(&t.arena).nth(1).expect("to-endpoint void");
        endpoint.children(&t.arena).nth(1).expect("text leaf")
    };
    c.bench_function("portal_hit_index_resolve", |b| {
        b.iter(|| portal.hit_index.resolve(&portal.tree, last_leaf))
    });

    // Same shape for labels: `PAIRS` line-mode edges each carrying a
    // label, hit on the last channel.
    let mut label_map = synthetic_single_section_map(PAIRS + 1);
    for i in 0..PAIRS {
        label_map
            .edges
            .push(bench_edge("n0", &format!("n{}", i + 1), false, Some("label")));
    }
    let label_hidden = label_map.fold_hidden_set();
    let mut label_paths = tree_builder::EdgePathCache::new(&label_map, &offsets);
    let elements = build_label_elements(&label_map, None, None, None, &mut label_paths, 1.0, &label_hidden);
    assert_eq!(elements.len(), PAIRS, "label bench fixture did not build");
    let labels = build_connection_label_tree(&elements);
    let last_label = labels
        .tree
        .root
        .children(&labels.tree.arena)
        .nth(PAIRS - 1)
        .expect("last label leaf");
    c.bench_function("connection_label_hit_index_resolve", |b| {
        b.iter(|| labels.hit_index.resolve(&labels.tree, last_label))
    });
}

/// `dewey_cmp` sorting a node-id list, which is the shape every
/// call site uses it in — `maptool grep` / `apply` ordering their
/// hits, and `maptool verify` ordering its violations. Measured as
/// a sort rather than a single compare because a single compare is
/// below what criterion can resolve, and because the sort is what
/// the callers pay for.
///
/// No `do_*()` body to reuse: `mindmap::model`'s tests are an
/// inline `#[cfg(test)] mod tests;`, not a `pub mod tests;` tree,
/// so nothing there is reachable from this file (§B8).
fn dewey_cmp_benchmark(c: &mut Criterion) {
    use baumhard::mindmap::model::dewey_cmp;
    // A realistic three-level tree, shuffled deterministically by
    // construction order rather than by an RNG so the row measures
    // the comparator and not a seed.
    let ids: Vec<String> = (0..8)
        .flat_map(|a| (0..8).flat_map(move |b| (0..8).map(move |c| format!("{}.{}.{}", a, c, b))))
        .collect();
    c.bench_function("dewey_cmp_sorts_a_three_level_id_list", |b| {
        b.iter(|| {
            let mut scratch: Vec<&str> = ids.iter().map(String::as_str).collect();
            scratch.sort_by(|x, y| dewey_cmp(x, y));
            scratch.len()
        })
    });
}

/// The edge hit-test's path geometry, one path per row rather than
/// through a whole projection pass.
///
/// Four rows. Three because the hit-test caller composes three
/// answers: `path_bounds` is the control-polygon box,
/// `distance_to_path` is the sampled walk that box protects, and
/// `distance_to_path_within` is the pair as
/// `document::hit_test::hit_test_edge` calls it — driven here at a
/// point the box rejects, which is the case the composition exists
/// for.
///
/// The fourth is `sample_path_into`, which is the connection pass's
/// side of the same geometry rather than the hit test's. It is
/// driven with a buffer that is cleared but not freed between
/// iterations, because that is the shape the pass runs: the scene
/// cache hands back the buffer this edge filled last frame.
fn connection_geometry_benchmark(c: &mut Criterion) {
    use baumhard::mindmap::connection::{
        distance_to_path, distance_to_path_within, path_bounds, sample_path_into, ConnectionPath,
        MAX_PATH_SAMPLES,
    };
    use glam::Vec2;

    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(2_500.0, 1_000.0),
        control2: Vec2::new(7_500.0, -1_000.0),
        end: Vec2::new(10_000.0, 0.0),
    };
    let near = Vec2::new(5_000.0, 20.0);
    let far = Vec2::new(60_000.0, 60_000.0);

    c.bench_function("connection_path_bounds", |b| b.iter(|| path_bounds(&path)));
    c.bench_function("connection_distance_to_path", |b| {
        b.iter(|| distance_to_path(near, &path))
    });
    c.bench_function("connection_distance_to_path_within_rejects", |b| {
        b.iter(|| distance_to_path_within(far, &path, 12.0))
    });
    let mut samples: Vec<Vec2> = Vec::new();
    c.bench_function("connection_sample_path_into", |b| {
        b.iter(|| {
            samples.clear();
            sample_path_into(&path, 12.0, MAX_PATH_SAMPLES, &mut samples);
            samples.len()
        })
    });
}

criterion_group!(
    benches,
    criterion_benchmark,
    dewey_cmp_benchmark,
    section_tree_build_benchmark,
    resize_mode_rebuild_benchmark,
    node_edit_mode_rebuild_benchmark,
    fast_resize_anchor_inference_benchmark,
    section_frame_emission_benchmark,
    hit_index_resolve_benchmark,
    connection_geometry_benchmark,
);
criterion_main!(benches);
