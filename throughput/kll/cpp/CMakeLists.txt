cmake_minimum_required(VERSION 3.15)
project(kll_throughput_cpp)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -O3 -march=native -fomit-frame-pointer")

# Apache DataSketches C++ (for kll_sketch)
set(DATASKETCHES_CPP_ROOT "$ENV{DATASKETCHES_CPP_ROOT}" CACHE PATH "Path to apache/datasketches-cpp")
if(NOT DATASKETCHES_CPP_ROOT)
    set(DATASKETCHES_CPP_ROOT "$ENV{HOME}/datasketches-cpp")
endif()

if(NOT EXISTS "${DATASKETCHES_CPP_ROOT}")
    message(FATAL_ERROR "datasketches-cpp not found at: ${DATASKETCHES_CPP_ROOT}\nSet DATASKETCHES_CPP_ROOT or place the repo at ~/datasketches-cpp.")
endif()

# AWS Insert-Optimized-Data-Sketches (for final::KarninLangLiberty)
set(INSERT_OPT_ROOT "$ENV{INSERT_OPT_ROOT}" CACHE PATH "Path to Insert-Optimized-Data-Sketches")
if(NOT INSERT_OPT_ROOT)
    set(INSERT_OPT_ROOT "$ENV{HOME}/Insert-Optimized-Data-Sketches")
endif()

if(NOT EXISTS "${INSERT_OPT_ROOT}")
    message(FATAL_ERROR "Insert-Optimized-Data-Sketches not found at: ${INSERT_OPT_ROOT}\nSet INSERT_OPT_ROOT or place the repo at ~/Insert-Optimized-Data-Sketches.")
endif()

include_directories(
    SYSTEM
    ${DATASKETCHES_CPP_ROOT}/kll/include
    ${DATASKETCHES_CPP_ROOT}/common/include
    ${INSERT_OPT_ROOT}/src
    ${INSERT_OPT_ROOT}/include
    ${INSERT_OPT_ROOT}/include/pcg
)

add_executable(kll_throughput src/main.cpp)
add_executable(kll_throughput_query src/main_query.cpp)
