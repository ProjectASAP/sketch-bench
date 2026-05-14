// Count-Min Sketch via Apache DataSketches. Supersedes the legacy
// cpp-bench/legacy/throughput/{cms,cms32k}/ + accuracy/frequency/cms/
// harnesses.
//
// Width (cols) is taken from `--k`, default 2048 to match the legacy
// throughput/cms harness. The 32k variant is reachable via `--k 32768`,
// so there is no separate cpp-bench/cms32k/ binary.
//
// Depth (rows) is fixed at 5 (apache's standard CMS preset); legacy used
// the same value across both width variants.

#include <cstddef>
#include <cstdint>
#include <fstream>
#include <iostream>

#include "common/accuracy.hpp"
#include "common/cli.hpp"
#include "common/record_v1.hpp"
#include "common/runner.hpp"
#include "common/workload.hpp"

#include "count_min.hpp"

using CmsSketch = datasketches::count_min_sketch<std::int64_t>;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);
    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;
    if (args.rng_seed)              cfg.rng_seed = *args.rng_seed;

    const std::uint8_t  cms_rows = 5;
    const std::uint32_t cms_cols = static_cast<std::uint32_t>(args.k.value_or(2048));
    const std::uint64_t cms_seed = args.rng_seed.value_or(cfg.rng_seed);

    auto build  = [&]() { return CmsSketch(cms_rows, cms_cols, cms_seed); };
    auto insert = [](CmsSketch& s, std::int64_t v) { s.update(v); };

    cpp_bench::AggregatedMetrics m =
        cpp_bench::run_throughput_latency<CmsSketch>(wl.items, cfg, build, insert);

    cpp_bench::Record rec;
    rec.sketch    = "cms";
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

        CmsSketch sketch(cms_rows, cms_cols, cms_seed);
        for (auto v : prefix) sketch.update(v);

        auto estimate_freq = [&](std::int64_t key) -> std::uint64_t {
            return static_cast<std::uint64_t>(sketch.get_estimate(key));
        };
        const std::size_t top_k = 100;
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
