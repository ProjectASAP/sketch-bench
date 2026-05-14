# HLL uses apache/datasketches headers shipped by the Insert-Optimized
# fork (apache layout: <family>/include/<family>.hpp). The cpp_bench_common
# target already adds ${INSERT_OPT_ROOT}/{src,include} as SYSTEM includes;
# we layer the per-family include dirs on top for hll.hpp + its common deps.

add_executable(datasketches_hll datasketches_hll.cpp)
target_link_libraries(datasketches_hll PRIVATE cpp_bench_common)
target_include_directories(datasketches_hll SYSTEM PRIVATE
    ${INSERT_OPT_ROOT}/hll/include
    ${INSERT_OPT_ROOT}/common/include
)
