use criterion::{criterion_group, criterion_main, Criterion};
use baumhard::core::tests::primitives_tests::*;
use baumhard::gfx_structs::tests::area_tests::*;
use baumhard::gfx_structs::tests::model_tests::*;
use baumhard::gfx_structs::tests::region_tests::*;
use baumhard::gfx_structs::tests::scene_tests::*;
use baumhard::gfx_structs::tests::tree_tests::*;
use baumhard::gfx_structs::tests::tree_walker_tests::*;
use baumhard::util::tests::arena_utils_tests::*;
use baumhard::util::tests::color_tests::*;
use baumhard::util::tests::geometry_tests::*;
use baumhard::util::tests::grapheme_chad_tests::*;
use baumhard::util::tests::primes_test::do_primes;

// We run all tests as benchmarks also, because the tests provides a good coverage of potential flows
fn criterion_benchmark(c: &mut Criterion) {
    // glyph_model //
    c.bench_function("matrix_place_in_1", |b| b.iter(|| matrix_place_in_1()));
    c.bench_function("matrix_place_in_2", |b| b.iter(|| matrix_place_in_2()));
    c.bench_function("matrix_place_in_3", |b| b.iter(|| matrix_place_in_3()));
    c.bench_function("matrix_add_assign_1", |b| b.iter(|| matrix_add_assign_1()));
    c.bench_function("matrix_add_assign_2", |b| b.iter(|| matrix_add_assign_2()));
    c.bench_function("line_add_assign_1", |b| b.iter(|| line_add_assign_1()));
    c.bench_function("line_add_assign_2", |b| b.iter(|| line_add_assign_2()));
    c.bench_function("line_add_assign_3", |b| b.iter(|| line_add_assign_3()));
    c.bench_function("line_add_assign_4", |b| b.iter(|| line_add_assign_4()));
    c.bench_function("component_of_index", |b| b.iter(|| component_of_index()));
    c.bench_function("index_of_component", |b| b.iter(|| index_of_component()));
    c.bench_function("expanding_insert_1", |b| b.iter(|| expanding_insert_1()));
    c.bench_function("expanding_insert_2", |b| b.iter(|| expanding_insert_2()));
    c.bench_function("expanding_insert_3", |b| b.iter(|| expanding_insert_3()));
    c.bench_function("expanding_insert_4", |b| b.iter(|| expanding_insert_4()));
    c.bench_function("expanding_insert_5", |b| b.iter(|| expanding_insert_5()));
    c.bench_function("expanding_insert_6", |b| b.iter(|| expanding_insert_6()));
    c.bench_function("expanding_insert_7", |b| b.iter(|| expanding_insert_7()));
    c.bench_function("overriding_insert_1", |b| b.iter(|| overriding_insert_1()));
    c.bench_function("overriding_insert_2", |b| b.iter(|| overriding_insert_2()));
    c.bench_function("overriding_insert_3", |b| b.iter(|| overriding_insert_3()));
    c.bench_function("overriding_insert_4", |b| b.iter(|| overriding_insert_4()));
    c.bench_function("overriding_insert_5", |b| b.iter(|| overriding_insert_5()));
    c.bench_function("overriding_insert_6", |b| b.iter(|| overriding_insert_6()));
    c.bench_function("overriding_insert_7", |b| b.iter(|| overriding_insert_7()));
    c.bench_function("overriding_insert_8", |b| b.iter(|| overriding_insert_8()));
    c.bench_function("overriding_insert_9", |b| b.iter(|| overriding_insert_9()));
    c.bench_function("overriding_insert_10", |b| b.iter(|| overriding_insert_10()));
    c.bench_function("overriding_insert_11", |b| b.iter(|| overriding_insert_11()));
    c.bench_function("overriding_insert_12", |b| b.iter(|| overriding_insert_12()));
    c.bench_function("overriding_insert_13", |b| b.iter(|| overriding_insert_13()));
    // glyph_area //
    c.bench_function("outline_default_is_none", |b| b.iter(|| do_outline_default_is_none()));
    c.bench_function("outline_assign_round_trip", |b| b.iter(|| do_outline_assign_round_trip()));
    c.bench_function("outline_subtract_clears", |b| b.iter(|| do_outline_subtract_clears()));
    c.bench_function("outline_changes_hash", |b| b.iter(|| do_outline_changes_hash()));
    c.bench_function("outline_field_add_picks_rhs", |b| b.iter(|| do_outline_field_add_picks_rhs()));
    // glyph_tree //
    c.bench_function("basics_solo_mutation", |b| b.iter(|| basics_solo_mutation()));
    c.bench_function("model_block_commands", |b| b.iter(|| model_block_commands()));
    c.bench_function("area_block_commands", |b| b.iter(|| area_block_commands()));
    c.bench_function("complex_tree_mutation", |b| b.iter(|| complex_tree_mutation()));
    c.bench_function("simple_tree_mutation", |b| b.iter(|| simple_tree_mutation()));
    c.bench_function("repeat_while_skip_while", |b| b.iter(|| repeat_while_skip_while()));
    c.bench_function("repeat_while_without_children_is_noop", |b| {
        b.iter(|| repeat_while_without_children_is_noop())
    });
    c.bench_function("event_propagation_complex", |b| b.iter(|| event_propagation_complex_symmetric()));
    c.bench_function("event_propagation_simple", |b| b.iter(|| event_propagation_simple()));
    // regions //
    c.bench_function("regions_sunny_day", |b| b.iter(|| do_region_params_new_sunny_day()));
    c.bench_function("region_indexer_initialise", |b| b.iter(|| do_region_indexer_initialize()));
    c.bench_function("region_indexer_insert", |b| b.iter(|| do_region_indexer_insert_delete()));
    c.bench_function("regions_rainy_day", |b| b.iter(|| do_regions_params_new_rainy_day()));
    c.bench_function("region_params_calculate_pixel_from_region", |b| b.iter(|| do_region_params_calculate_pixel_from_region()));
    c.bench_function("region_params_calculate_region_from_pixel", |b| b.iter(|| do_region_params_calculate_region_from_pixel()));
    c.bench_function("region_params_calculate_regions_intersected_by_rectangle", |b| b.iter(||
       do_region_params_calculate_regions_intersected_by_rectangle()));
    // grapheme_chad //
    c.bench_function("slice_to_newline", |b| b.iter(|| do_slice_to_newline()));
    c.bench_function("split_graphemes", |b| b.iter(|| do_split_graphemes()));
    c.bench_function("find_byte_index_of_grapheme", |b| b.iter(|| do_find_byte_index_of_grapheme()));
    c.bench_function("replace_graphemes_until_newline", |b| b.iter(|| do_replace_graphemes_until_newline()));
    c.bench_function("count_grapheme_clusters", |b| b.iter(|| do_count_grapheme_clusters()));
    c.bench_function("find_nth_line_byte_indices", |b| b.iter(|| do_find_nth_line_byte_indices()));
    c.bench_function("find_nth_line_grapheme_indices", |b| b.iter(|| do_find_nth_line_grapheme_indices()));
    c.bench_function("remove_prefix_unicode", |b| b.iter(|| do_remove_prefix_unicode()));
    c.bench_function("insert_new_lines", |b| b.iter(|| do_insert_new_lines()));
    c.bench_function("push_spaces", |b| b.iter(|| do_push_spaces()));
    c.bench_function("count_number_of_lines", |b| b.iter(|| do_count_number_of_lines()));
    c.bench_function("truncate_unicode", |b| b.iter(|| do_truncate_unicode()));
    c.bench_function("insert_str_at_grapheme", |b| b.iter(|| do_insert_str_at_grapheme()));
    c.bench_function("delete_grapheme_at", |b| b.iter(|| do_delete_grapheme_at()));
    c.bench_function("grapheme_display_width", |b| b.iter(|| do_grapheme_display_width()));
    c.bench_function("truncate_to_display_width", |b| b.iter(|| do_truncate_to_display_width()));
    // geometry //
    c.bench_function("90_deg_rotation", |b| b.iter(|| do_90_deg_rotation()));
    c.bench_function("180_deg_rotation", |b| b.iter(|| do_180_deg_rotation()));
    c.bench_function("non_origin_pivot_rotation", |b| b.iter(|| do_non_origin_pivot_rotation()));
    c.bench_function("0_deg_rotation", |b| b.iter(|| do_0_deg_rotation()));
    c.bench_function("pixel_functions", |b| b.iter(|| do_pixel_functions()));
    c.bench_function("almost_equal", |b| b.iter(|| do_almost_equal()));
    c.bench_function("almost_equal_vec2", |b| b.iter(|| do_almost_equal_vec2()));
    // color //
    c.bench_function("from_hex", |b| b.iter(|| do_from_hex()));
    c.bench_function("from_hex_lazy_static", |b| b.iter(|| do_from_hex_lazy_static()));
    c.bench_function("from_hex_garbage_falls_back_to_black", |b| {
        b.iter(|| do_from_hex_garbage_falls_back_to_black())
    });
    c.bench_function("rgba_hex_macros", |b| b.iter(|| do_rgba_hex_macros()));
    // primitives //
    c.bench_function("overlaps", |b| b.iter(|| do_overlaps()));
    c.bench_function("split_and_separate_1", |b| b.iter(|| do_split_and_separate_1()));
    c.bench_function("split_and_separate_2", |b| b.iter(|| do_split_and_separate_2()));
    c.bench_function("submit_region_drops_inverted_range", |b| {
        b.iter(|| do_submit_region_drops_inverted_range())
    });
    // scene + hit-test //
    c.bench_function("descendant_at_hits_single_area", |b| b.iter(|| do_descendant_at_hits_single_area()));
    c.bench_function("descendant_at_prefers_smallest", |b| b.iter(|| do_descendant_at_prefers_smallest()));
    c.bench_function("descendant_near_grants_slack", |b| b.iter(|| do_descendant_near_grants_slack()));
    c.bench_function("descendants_aabb", |b| b.iter(|| do_descendants_aabb_covers_all_areas()));
    c.bench_function("descendants_aabb_invalidated_by_mutator", |b| b.iter(|| do_descendants_aabb_cache_invalidated_by_mutator()));
    c.bench_function("scene_component_at", |b| b.iter(|| do_scene_insert_and_component_at()));
    c.bench_function("scene_layer_order_hit_priority", |b| b.iter(|| do_scene_layer_order_controls_hit_priority()));
    c.bench_function("scene_offset_hit_test", |b| b.iter(|| do_scene_offset_is_applied_to_hit_test()));
    // arena_utils //
    c.bench_function("arena_utils_clone", |b| b.iter(|| do_clone()));
    // primes //
    c.bench_function("primes", |b| b.iter(|| do_primes()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
