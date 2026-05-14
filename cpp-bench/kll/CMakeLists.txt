add_executable(datasketches_kll datasketches_kll.cpp)
target_link_libraries(datasketches_kll PRIVATE cpp_bench_common)

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
