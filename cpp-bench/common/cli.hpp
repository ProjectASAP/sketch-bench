#pragma once

// Tiny argparse for cpp-bench binaries. Every binary takes the same
// shape of CLI: it knows its own sketch+impl statically, the user
// supplies workload + runs + report destination.

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

namespace cpp_bench {

struct CliArgs {
    // Required.
    std::string workload_file;

    // Optional, default to RunnerConfig defaults if absent.
    std::optional<std::size_t> warmup_items;
    std::optional<std::size_t> measure_items;
    std::optional<std::size_t> runs;
    std::optional<std::size_t> latency_sample_stride;
    std::optional<std::uint64_t> rng_seed;

    // Optional sketch-family construction param. Interpretation is
    // per-binary (KLL uses it as k; CS as cols; CMS as rows etc.).
    std::optional<std::size_t> k;

    // "-" or empty means stdout.
    std::string report_path = "-";

    // Whether the binary should also compute accuracy and include
    // it in the emitted record. Off by default because it costs
    // extra memory (a full exact baseline).
    bool with_accuracy = false;
};

// Parse argv. Aborts (exit 2) with a usage message on bad input.
CliArgs parse_cli(int argc, char** argv);

}  // namespace cpp_bench
