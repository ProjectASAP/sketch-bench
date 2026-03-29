cmake_minimum_required(VERSION 3.15)
project(cms_accuracy_cpp)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -O3 -march=native -fomit-frame-pointer")

set(INSERT_OPT_ROOT "$ENV{INSERT_OPT_ROOT}" CACHE PATH "Path to Insert-Optimized-Data-Sketches")
if(NOT INSERT_OPT_ROOT)
    set(INSERT_OPT_ROOT "$ENV{HOME}/Insert-Optimized-Data-Sketches")
endif()

if(NOT EXISTS "${INSERT_OPT_ROOT}")
    message(FATAL_ERROR "Insert-Optimized-Data-Sketches not found at: ${INSERT_OPT_ROOT}\nSet INSERT_OPT_ROOT or place the repo at ~/Insert-Optimized-Data-Sketches.")
endif()

include_directories(
    SYSTEM
    ${INSERT_OPT_ROOT}/src
    ${INSERT_OPT_ROOT}/include
    ${INSERT_OPT_ROOT}/include/pcg
)
include_directories(${CMAKE_CURRENT_SOURCE_DIR}/src)

add_executable(cms_accuracy src/main.cpp)

message(STATUS "Build type: ${CMAKE_BUILD_TYPE}")
message(STATUS "Insert-Optimized Repo: ${INSERT_OPT_ROOT}")
