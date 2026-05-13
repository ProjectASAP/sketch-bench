// CountSketch via the insert-optimized "final" variant.

#include <cstddef>
#include <cstdint>
#include <fstream>
#include <iostream>

#include "common/accuracy.hpp"
#include "common/cli.hpp"
#include "common/record_v1.hpp"
#include "common/runner.hpp"
#include "common/workload.hpp"

#include "cs/cs_final.hpp"

using CsFinal = final::CountSketch<std::int64_t>;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);
    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;

    auto build  = []()  { return CsFinal{}; };
    auto insert = [](CsFinal& s, std::int64_t v) { s.Insert(v); };

    cpp_bench::AggregatedMetrics m =
        cpp_bench::run_throughput_latency<CsFinal>(wl.items, cfg, build, insert);

    cpp_bench::Record rec;
    rec.sketch    = "cs";
    rec.impl_name = "final";
    rec.workload  = wl.desc;
    rec.runs      = cfg.runs;
    rec.bench.throughput_items_per_sec = m.throughput;
    if (m.latency.count > 0) rec.bench.latency_ns = m.latency;

    if (args.with_accuracy) {
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());
        std::vector<std::int64_t> prefix(
            wl.items.begin(),
            wl.items.begin() + static_cast<std::ptrdiff_t>(measure_n));

        CsFinal sketch;
        for (auto v : prefix) sketch.Insert(v);

        auto estimate_freq = [&](std::int64_t key) -> std::uint64_t {
            return static_cast<std::uint64_t>(sketch.Estimate(key));
        };
        const std::size_t top_k = args.k.value_or(100);
        rec.bench.accuracy_json =
            cpp_bench::compute_frequency_accuracy(prefix, top_k, estimate_freq).json;
    }

    if (args.report_path == "-" || args.report_path.empty()) {
        rec.emit(std::cout);
    } else {
        std::ofstream out(args.report_path, std::ios::app);
        if (!out) { std::cerr << "cpp-bench: cannot write " << args.report_path << '\n'; return 2; }
        rec.emit(out);
    }
    return 0;
}
