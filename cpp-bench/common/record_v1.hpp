#pragma once

// Minimal builder for a v1 JSONL `Record` (see docs/SCHEMA_V1.md and
// sketch-core/src/report.rs). Hand-rolled to avoid a JSON-library
// dependency; the output is verified round-trippable through
// `serde_json::from_str::<Record>` by sketch-core's tests.

#include <cstdint>
#include <optional>
#include <ostream>
#include <string>
#include <vector>

namespace cpp_bench {

struct RunStats {
    double mean   = 0.0;
    double stddev = 0.0;
    double ci95_lo = 0.0;
    double ci95_hi = 0.0;
    std::size_t n = 0;
};

struct LatencySummary {
    std::uint64_t p50  = 0;
    std::uint64_t p95  = 0;
    std::uint64_t p99  = 0;
    std::uint64_t p999 = 0;
    std::uint64_t max  = 0;
    std::uint64_t count = 0;
};

struct WorkloadDesc {
    std::string shape;                      // "uniform" | "zipf" | "file"
    std::size_t size = 0;
    std::optional<std::uint64_t> cardinality;
    std::optional<double>        zipf_s;
    std::optional<std::string>   source_path;
    std::optional<std::uint64_t> seed;
};

struct BenchSection {
    std::optional<RunStats>        throughput_items_per_sec;
    std::optional<LatencySummary>  latency_ns;
    // accuracy is a pre-serialised JSON object string (e.g.
    // R"({"queries":[0.5,0.95],"abs_rank_err":{...}})") so each
    // sketch family can shape it per docs/SCHEMA_V1.md.
    std::optional<std::string>     accuracy_json;
};

struct Record {
    std::uint32_t schema_version = 2;
    std::string   sketch;          // "kll", "cs", ...
    std::string   impl_name;       // "datasketches", "final", ...
    std::string   language = "cpp";
    WorkloadDesc  workload;
    std::string   mode = "bench";
    std::size_t   runs = 1;
    BenchSection  bench;
    std::string   source = "cpp-bench";
    std::string   timestamp_rfc3339;   // populated from current UTC clock by emit()

    // Serialise this record as one line of JSON (no trailing newline).
    std::string to_jsonl() const;

    // Convenience: stamp timestamp_rfc3339 with the current UTC clock
    // and write `to_jsonl() + "\n"` to `os`.
    void emit(std::ostream& os);
};

}  // namespace cpp_bench
