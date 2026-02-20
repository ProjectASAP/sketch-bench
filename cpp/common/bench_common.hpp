#pragma once

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <random>
#include <string>
#include <vector>

#if defined(_MSC_VER)
#include <intrin.h>
#endif

struct BenchmarkConfig {
    std::string implementation_name;
    std::string data_path;
    size_t warmup_items;
    size_t measure_items;
};

struct BenchmarkResult {
    std::string implementation_name;
    long long total_nanoseconds;
};

constexpr size_t DEFAULT_WARMUP_ITEMS = 100000;
constexpr size_t DEFAULT_MEASURE_ITEMS = 1000000;

inline std::string default_data_path() {
    const char* root = std::getenv("SKETCH_BENCH_ROOT");
    if (root && *root) {
        return std::string(root) + "/input/benchmark_data_1m_int64.bin";
    }
    return "input/benchmark_data_1m_int64.bin";
}

inline void black_box(const void* value) {
#if defined(_MSC_VER)
    (void)value;
    _ReadWriteBarrier();
#else
    asm volatile("" : : "g"(value) : "memory");
#endif
}

inline std::vector<int64_t> load_i64_dataset(const std::string& path) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file) {
        std::cerr << "Error: Cannot open file " << path << std::endl;
        std::exit(1);
    }

    std::streamsize size = file.tellg();
    file.seekg(0, std::ios::beg);

    size_t num_elements = static_cast<size_t>(size) / sizeof(int64_t);
    std::vector<int64_t> data(num_elements);

    if (!file.read(reinterpret_cast<char*>(data.data()), size)) {
        std::cerr << "Error: Failed to read file " << path << std::endl;
        std::exit(1);
    }

    return data;
}

template <typename BuildFn, typename InsertFn>
BenchmarkResult run_benchmark_i64(const BenchmarkConfig& config, BuildFn build, InsertFn insert) {
    auto data = load_i64_dataset(config.data_path);
    black_box(data.data());

    size_t warmup_count = std::min(config.warmup_items, data.size());
    size_t measure_count = std::min(config.measure_items, data.size());

    {
        auto warmup_sketch = build();
        for (size_t i = 0; i < warmup_count; ++i) {
            insert(warmup_sketch, data[i]);
        }
        black_box(&warmup_sketch);
    }

    auto sketch = build();
    auto start = std::chrono::steady_clock::now();
    for (size_t i = 0; i < measure_count; ++i) {
        insert(sketch, data[i]);
    }
    auto end = std::chrono::steady_clock::now();
    black_box(&sketch);

    auto elapsed = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();

    return BenchmarkResult{config.implementation_name, elapsed};
}

inline void print_result(const BenchmarkResult& result) {
    std::cout << "{\"implementation_name\":\"" << result.implementation_name
              << "\",\"total_nanoseconds\":" << result.total_nanoseconds << "}" << std::endl;
}
