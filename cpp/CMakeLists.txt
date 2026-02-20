cmake_minimum_required(VERSION 3.15)
project(sketchbench_cpp)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Match external library's flags (append, don't replace)
set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -O3 -march=native -fomit-frame-pointer")

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

set(INSERT_OPT_ROOT "$ENV{INSERT_OPT_ROOT}" CACHE PATH "Path to Insert-Optimized-Data-Sketches")
if(NOT INSERT_OPT_ROOT)
    set(INSERT_OPT_ROOT "$ENV{HOME}/Insert-Optimized-Data-Sketches")
endif()

if(NOT EXISTS "${INSERT_OPT_ROOT}")
    message(FATAL_ERROR "Insert-Optimized-Data-Sketches not found at: ${INSERT_OPT_ROOT}\n"
                        "Set INSERT_OPT_ROOT or place the repo at ~/Insert-Optimized-Data-Sketches.")
endif()

# Match external library's include order: src first, then include
include_directories(
    SYSTEM
    ${INSERT_OPT_ROOT}/src
    ${INSERT_OPT_ROOT}/include
    ${INSERT_OPT_ROOT}/include/pcg
)
# Local common directory after external includes
include_directories(${CMAKE_CURRENT_SOURCE_DIR}/common)

option(SKETCHBENCH_ENABLE_GBENCH "Build Google Benchmark binaries" ON)
option(SKETCHBENCH_FETCH_GBENCH "Allow FetchContent to download Google Benchmark" OFF)
set(GOOGLE_BENCHMARK_ROOT "$ENV{GOOGLE_BENCHMARK_ROOT}" CACHE PATH
    "Path to Google Benchmark source tree or install prefix")

set(CS_SOURCES
    cs/cs_naive.cpp
    cs/cs_fastrange.cpp
    cs/cs_fixed_size.cpp
    cs/cs_final.cpp
    cs/cs_final_no_murmur_unroll.cpp
    cs/cs_datasketches.cpp
)

set(KLL_SOURCES
    kll/kll_naive.cpp
    kll/kll_datasketches.cpp
    kll/kll_no_self_move_protection.cpp
    kll/kll_pcg_random.cpp
    kll/kll_no_min_max.cpp
    kll/kll_cached_level_capacities.cpp
    kll/kll_final.cpp
)

foreach(source_file IN LISTS CS_SOURCES KLL_SOURCES)
    get_filename_component(bin_name "${source_file}" NAME_WE)
    add_executable("${bin_name}" "${source_file}")
endforeach()

if(SKETCHBENCH_ENABLE_GBENCH)
    set(BENCHMARK_TARGET "")
    set(BENCHMARK_ENABLE_GTEST_TESTS OFF CACHE BOOL "" FORCE)
    set(BENCHMARK_ENABLE_TESTING OFF CACHE BOOL "" FORCE)
    if(GOOGLE_BENCHMARK_ROOT)
        if(EXISTS "${GOOGLE_BENCHMARK_ROOT}/CMakeLists.txt")
            add_subdirectory("${GOOGLE_BENCHMARK_ROOT}"
                             "${CMAKE_BINARY_DIR}/_gbench" EXCLUDE_FROM_ALL)
        else()
            message(WARNING "GOOGLE_BENCHMARK_ROOT does not contain CMakeLists.txt: ${GOOGLE_BENCHMARK_ROOT}")
        endif()
    else()
        find_package(benchmark CONFIG QUIET)
    endif()

    if(TARGET benchmark::benchmark)
        set(BENCHMARK_TARGET benchmark::benchmark)
    elseif(TARGET benchmark)
        set(BENCHMARK_TARGET benchmark)
    endif()

    if((NOT BENCHMARK_TARGET) AND SKETCHBENCH_FETCH_GBENCH)
        include(FetchContent)
        set(FETCHCONTENT_QUIET ON)
        FetchContent_Declare(
            benchmark
            GIT_REPOSITORY "https://github.com/google/benchmark.git"
            GIT_TAG v1.9.1
        )
        FetchContent_MakeAvailable(benchmark)
    endif()

    if(NOT BENCHMARK_TARGET)
        if(TARGET benchmark::benchmark)
            set(BENCHMARK_TARGET benchmark::benchmark)
        elseif(TARGET benchmark)
            set(BENCHMARK_TARGET benchmark)
        endif()
    endif()

    if(NOT BENCHMARK_TARGET)
        message(FATAL_ERROR "Google Benchmark not found; set GOOGLE_BENCHMARK_ROOT or enable SKETCHBENCH_FETCH_GBENCH.")
    endif()

    foreach(source_file IN LISTS CS_SOURCES KLL_SOURCES)
        get_filename_component(bin_name "${source_file}" NAME_WE)
        target_link_libraries("${bin_name}" PRIVATE ${BENCHMARK_TARGET})
    endforeach()
endif()

message(STATUS "Build type: ${CMAKE_BUILD_TYPE}")
message(STATUS "CXX flags: ${CMAKE_CXX_FLAGS_RELEASE}")
message(STATUS "Insert-Optimized Repo: ${INSERT_OPT_ROOT}")
