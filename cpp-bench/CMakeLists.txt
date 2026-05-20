cmake_minimum_required(VERSION 3.15)
project(cpp_bench LANGUAGES CXX)

# C++ track of sketch-bench. Mirrors the Rust `sketch-cli` track:
# every binary emits a single v1 JSONL `Record` to stdout. See
# `docs/SCHEMA_V1.md` and `CPP_BENCH_PROGRESS.md`.

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

# Match cpp/ flags so the two trees are comparable.
set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -O3 -march=native -fomit-frame-pointer")

# ---------- Apache DataSketches headers (via Insert-Optimized fork) ----------
#
# Same convention as cpp/: $INSERT_OPT_ROOT points at the Insert-Optimized
# datasketches checkout, defaulting to ~/Insert-Optimized-Data-Sketches.

set(INSERT_OPT_ROOT "$ENV{INSERT_OPT_ROOT}" CACHE PATH "Path to Insert-Optimized-Data-Sketches")
if(NOT INSERT_OPT_ROOT)
    set(INSERT_OPT_ROOT "$ENV{HOME}/Insert-Optimized-Data-Sketches")
endif()

if(NOT EXISTS "${INSERT_OPT_ROOT}")
    message(FATAL_ERROR
        "Insert-Optimized-Data-Sketches not found at: ${INSERT_OPT_ROOT}\n"
        "Set INSERT_OPT_ROOT or place the repo at ~/Insert-Optimized-Data-Sketches.")
endif()

# ---------- Upstream Apache DataSketches (for families the Insert-Opt fork
# does not customize, e.g. kll_sketch which exposes get_quantile). Only
# binaries that opt in get these include paths.

set(DATASKETCHES_UPSTREAM_ROOT "$ENV{DATASKETCHES_UPSTREAM_ROOT}" CACHE PATH
    "Path to upstream apache/datasketches-cpp checkout")
if(NOT DATASKETCHES_UPSTREAM_ROOT)
    set(DATASKETCHES_UPSTREAM_ROOT "$ENV{HOME}/datasketches-cpp")
endif()

# ---------- Google Benchmark ----------
#
# Same pattern as cpp/CMakeLists.txt: prefer an installed copy or a
# user-supplied source tree, fall back to FetchContent only when asked.

option(CPP_BENCH_FETCH_GBENCH "Allow FetchContent to download Google Benchmark" OFF)
set(GOOGLE_BENCHMARK_ROOT "$ENV{GOOGLE_BENCHMARK_ROOT}" CACHE PATH
    "Path to Google Benchmark source tree or install prefix")

set(BENCHMARK_ENABLE_GTEST_TESTS OFF CACHE BOOL "" FORCE)
set(BENCHMARK_ENABLE_TESTING OFF CACHE BOOL "" FORCE)

if(GOOGLE_BENCHMARK_ROOT AND EXISTS "${GOOGLE_BENCHMARK_ROOT}/CMakeLists.txt")
    add_subdirectory("${GOOGLE_BENCHMARK_ROOT}" "${CMAKE_BINARY_DIR}/_gbench" EXCLUDE_FROM_ALL)
else()
    find_package(benchmark CONFIG QUIET)
endif()

if(NOT TARGET benchmark::benchmark AND NOT TARGET benchmark)
    if(CPP_BENCH_FETCH_GBENCH)
        include(FetchContent)
        set(FETCHCONTENT_QUIET ON)
        FetchContent_Declare(
            benchmark
            GIT_REPOSITORY "https://github.com/google/benchmark.git"
            GIT_TAG v1.9.1
        )
        FetchContent_MakeAvailable(benchmark)
    else()
        message(FATAL_ERROR
            "Google Benchmark not found. Either install it, set "
            "GOOGLE_BENCHMARK_ROOT, or pass -DCPP_BENCH_FETCH_GBENCH=ON.")
    endif()
endif()

if(TARGET benchmark::benchmark)
    set(CPP_BENCH_GBENCH benchmark::benchmark)
else()
    set(CPP_BENCH_GBENCH benchmark)
endif()

# ---------- subdirs ----------
add_subdirectory(common)
add_subdirectory(kll)
add_subdirectory(cs)
add_subdirectory(hll)
add_subdirectory(cms)

message(STATUS "[cpp-bench] build type:  ${CMAKE_BUILD_TYPE}")
message(STATUS "[cpp-bench] CXX flags:   ${CMAKE_CXX_FLAGS_RELEASE}")
message(STATUS "[cpp-bench] insert-opt:  ${INSERT_OPT_ROOT}")
message(STATUS "[cpp-bench] gbench tgt:  ${CPP_BENCH_GBENCH}")
