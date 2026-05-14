add_library(cpp_bench_common STATIC
    record_v1.cpp
    workload.cpp
    runner.cpp
    cli.cpp
    accuracy.cpp
)

target_include_directories(cpp_bench_common PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/..
)

# Public deps surface to every binary that links us.
target_link_libraries(cpp_bench_common PUBLIC ${CPP_BENCH_GBENCH})

# Insert-Optimized headers are SYSTEM so their warnings don't pollute us.
target_include_directories(cpp_bench_common SYSTEM PUBLIC
    ${INSERT_OPT_ROOT}/src
    ${INSERT_OPT_ROOT}/include
    ${INSERT_OPT_ROOT}/include/pcg
)
