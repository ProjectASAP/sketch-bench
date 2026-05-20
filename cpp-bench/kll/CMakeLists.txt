add_executable(datasketches_kll datasketches_kll.cpp)
target_link_libraries(datasketches_kll PRIVATE cpp_bench_common)

# Upstream Apache DataSketches kll_sketch (has get_quantile). Headers live
# outside INSERT_OPT_ROOT — pull them in BEFORE cpp_bench_common's interface
# includes so anything that exists in both trees resolves to the upstream copy.
if(EXISTS "${DATASKETCHES_UPSTREAM_ROOT}/kll/include/kll_sketch.hpp")
    add_executable(datasketches_upstream_kll datasketches_upstream_kll.cpp)
    target_link_libraries(datasketches_upstream_kll PRIVATE cpp_bench_common)
    target_include_directories(datasketches_upstream_kll SYSTEM BEFORE PRIVATE
        ${DATASKETCHES_UPSTREAM_ROOT}/kll/include
        ${DATASKETCHES_UPSTREAM_ROOT}/common/include)
else()
    message(STATUS "[cpp-bench/kll] skipping datasketches_upstream_kll: "
        "${DATASKETCHES_UPSTREAM_ROOT}/kll/include/kll_sketch.hpp not found")
endif()

add_executable(final_kll final_kll.cpp)
target_link_libraries(final_kll PRIVATE cpp_bench_common)

# Optimization-evolution variants. Same v1 JSONL contract, same CLI; only the
# underlying header / namespace differs. All constructed with k=200 to match
# the legacy cpp/ Google-Benchmark numbers.
add_executable(naive_kll naive_kll.cpp)
target_link_libraries(naive_kll PRIVATE cpp_bench_common)

add_executable(cached_level_capacities_kll cached_level_capacities_kll.cpp)
target_link_libraries(cached_level_capacities_kll PRIVATE cpp_bench_common)

add_executable(no_min_max_kll no_min_max_kll.cpp)
target_link_libraries(no_min_max_kll PRIVATE cpp_bench_common)

add_executable(no_self_move_protection_kll no_self_move_protection_kll.cpp)
target_link_libraries(no_self_move_protection_kll PRIVATE cpp_bench_common)

add_executable(pcg_random_kll pcg_random_kll.cpp)
target_link_libraries(pcg_random_kll PRIVATE cpp_bench_common)
