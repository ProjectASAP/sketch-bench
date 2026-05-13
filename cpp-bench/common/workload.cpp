#include "workload.hpp"

#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <string>

namespace cpp_bench {

Workload load_i64_bin(const std::string& path) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file) {
        std::cerr << "cpp-bench: cannot open workload file: " << path << '\n';
        std::exit(2);
    }
    const std::streamsize bytes = file.tellg();
    if (bytes < 0 || (bytes % static_cast<std::streamsize>(sizeof(std::int64_t))) != 0) {
        std::cerr << "cpp-bench: " << path
                  << ": size " << bytes << " not a multiple of 8\n";
        std::exit(2);
    }
    file.seekg(0, std::ios::beg);

    Workload w;
    const std::size_t n = static_cast<std::size_t>(bytes) / sizeof(std::int64_t);
    w.items.resize(n);
    if (!file.read(reinterpret_cast<char*>(w.items.data()), bytes)) {
        std::cerr << "cpp-bench: failed to read " << path << '\n';
        std::exit(2);
    }

    w.desc.shape = "file";
    w.desc.size = n;
    w.desc.source_path = path;
    return w;
}

}  // namespace cpp_bench
