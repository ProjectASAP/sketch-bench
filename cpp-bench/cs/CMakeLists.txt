add_executable(datasketches_cs datasketches_cs.cpp)
target_link_libraries(datasketches_cs PRIVATE cpp_bench_common)

add_executable(final_cs final_cs.cpp)
target_link_libraries(final_cs PRIVATE cpp_bench_common)

# Optimization-evolution variants. Same v1 JSONL contract, same CLI; only the
# underlying header / namespace differs. Comparable run-to-run with final_cs.
add_executable(naive_cs naive_cs.cpp)
target_link_libraries(naive_cs PRIVATE cpp_bench_common)

add_executable(fastrange_cs fastrange_cs.cpp)
target_link_libraries(fastrange_cs PRIVATE cpp_bench_common)

add_executable(fixed_size_cs fixed_size_cs.cpp)
target_link_libraries(fixed_size_cs PRIVATE cpp_bench_common)

add_executable(final_no_murmur_unroll_cs final_no_murmur_unroll_cs.cpp)
target_link_libraries(final_no_murmur_unroll_cs PRIVATE cpp_bench_common)
