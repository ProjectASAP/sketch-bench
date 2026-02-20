#include <cstdint>
#include <vector>
#include <iostream>

#if defined(_MSC_VER)
#include <intrin.h>
#endif

inline void black_box(const void* value) {
#if defined(_MSC_VER)
    (void)value;
    _ReadWriteBarrier();
#else
    asm volatile("" : : "g"(value) : "memory");
#endif
}

struct SimpleSketch {
    int64_t sum;

    SimpleSketch() : sum(0) {}

    __attribute__((always_inline))
    inline void insert(int64_t value) {
        sum += value;
    }
};

// Version 1: Indexed loop - current C++ implementation
__attribute__((noinline))
int64_t bench_indexed_loop(const std::vector<int64_t>& data, size_t count) {
    SimpleSketch sketch;
    for (size_t i = 0; i < count; ++i) {
        sketch.insert(data[i]);
    }
    black_box(&sketch);
    return sketch.sum;
}

// Version 2: Pointer iteration
__attribute__((noinline))
int64_t bench_pointer_iteration(const std::vector<int64_t>& data, size_t count) {
    SimpleSketch sketch;
    const int64_t* ptr = data.data();
    const int64_t* end = ptr + count;
    for (; ptr != end; ++ptr) {
        sketch.insert(*ptr);
    }
    black_box(&sketch);
    return sketch.sum;
}

// Version 3: Range-based for loop
__attribute__((noinline))
int64_t bench_range_loop(const std::vector<int64_t>& data, size_t count) {
    SimpleSketch sketch;
    for (const auto& value : data) {
        sketch.insert(value);
    }
    black_box(&sketch);
    return sketch.sum;
}

int main() {
    std::vector<int64_t> data;
    data.reserve(1000000);
    for (int64_t i = 0; i < 1000000; ++i) {
        data.push_back(i);
    }

    size_t count = 1000000;

    int64_t r1 = bench_indexed_loop(data, count);
    int64_t r2 = bench_pointer_iteration(data, count);
    int64_t r3 = bench_range_loop(data, count);

    std::cout << "Results: " << r1 << " " << r2 << " " << r3 << std::endl;
    return 0;
}
