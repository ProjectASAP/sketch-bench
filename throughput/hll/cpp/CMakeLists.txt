cmake_minimum_required(VERSION 3.15)
project(hll_throughput_cpp)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -O3 -march=native -fomit-frame-pointer")

set(DATASKETCHES_CPP_ROOT "$ENV{DATASKETCHES_CPP_ROOT}" CACHE PATH "Path to apache/datasketches-cpp")
if(NOT DATASKETCHES_CPP_ROOT)
    set(DATASKETCHES_CPP_ROOT "$ENV{HOME}/datasketches-cpp")
endif()

if(NOT EXISTS "${DATASKETCHES_CPP_ROOT}")
    message(FATAL_ERROR "datasketches-cpp not found at: ${DATASKETCHES_CPP_ROOT}\nSet DATASKETCHES_CPP_ROOT or place the repo at ~/datasketches-cpp.")
endif()

include_directories(
    SYSTEM
    ${DATASKETCHES_CPP_ROOT}/hll/include
    ${DATASKETCHES_CPP_ROOT}/common/include
)

add_executable(hll_throughput src/main.cpp)
