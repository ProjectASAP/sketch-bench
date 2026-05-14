# CMS uses apache/datasketches headers shipped by the Insert-Optimized
# fork. Apache's CMS lives under count/include/count_min.hpp.

add_executable(datasketches_cms datasketches_cms.cpp)
target_link_libraries(datasketches_cms PRIVATE cpp_bench_common)
target_include_directories(datasketches_cms SYSTEM PRIVATE
    ${INSERT_OPT_ROOT}/count/include
    ${INSERT_OPT_ROOT}/common/include
)
