// HLL via Apache DataSketches (HLL_8 backing, lg_k = 12 by default).
// Mirrors cpp-bench/cs/datasketches_cs.cpp and supersedes the legacy
// cpp-bench/legacy/throughput/hll/ + accuracy/cardinality/ harnesses.
//
// Includes `hll.hpp` from the Insert-Optimized fork's apache layout
// (${INSERT_OPT_ROOT}/hll/include). The fork tracks apache/datasketches-cpp
// for non-customised families, so the public API (`hll_sketch::update`,
// `get_estimate`, `HLL_8`) is unchanged.

#include <cstddef>
#include <cstdint>
#include <fstream>
#include <iostream>

#include "common/accuracy.hpp"
#include "common/cli.hpp"
#include "common/record_v1.hpp"
#include "common/runner.hpp"
#include "common/workload.hpp"

#include "hll.hpp"

using HllSketch = datasketches::hll_sketch;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);
    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;
    if (args.rng_seed)              cfg.rng_seed = *args.rng_seed;

    // lg_k: matches legacy throughput/hll/ default of 12. Override via `--k`.
    const std::uint8_t lg_k = static_cast<std::uint8_t>(args.k.value_or(12));

    auto build  = [&]() { return HllSketch(lg_k, datasketches::HLL_8); };
    auto insert = [](HllSketch& s, std::int64_t v) { s.update(v); };

    cpp_bench::AggregatedMetrics m =
        cpp_bench::run_throughput_latency<HllSketch>(wl.items, cfg, build, insert);

    cpp_bench::Record rec;
    rec.sketch    = "hll";
    rec.impl_name = "datasketches";
    rec.workload  = wl.desc;
    rec.runs      = cfg.runs;
    rec.bench.throughput_items_per_sec = m.throughput;
    if (m.latency.count > 0) rec.bench.latency_ns = m.latency;

    if (args.with_accuracy) {
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());
        std::vector<std::int64_t> prefix(
            wl.items.begin(),
            wl.items.begin() + static_cast<std::ptrdiff_t>(measure_n));

        HllSketch sketch(lg_k, datasketches::HLL_8);
        for (auto v : prefix) sketch.update(v);

        const double est = sketch.get_estimate();
        rec.bench.accuracy_json =
            cpp_bench::compute_cardinality_accuracy(prefix, est).json;
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
