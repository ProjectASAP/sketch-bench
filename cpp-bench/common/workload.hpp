#pragma once

// Workload loaders for the C++ track. Currently file-backed only;
// same .bin format as the Rust side's `FileI64` (little-endian int64
// stream), so both tracks can be pointed at the same input file.

#include <cstdint>
#include <string>
#include <vector>

#include "record_v1.hpp"

namespace cpp_bench {

struct Workload {
    std::vector<std::int64_t> items;
    WorkloadDesc desc;
};

// Read a little-endian int64 stream from `path`. Aborts on
// truncation / unreadable file. The returned `desc` has shape="file"
// and source_path set to `path`.
Workload load_i64_bin(const std::string& path);

}  // namespace cpp_bench
