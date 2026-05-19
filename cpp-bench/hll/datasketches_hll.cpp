// HLL via Apache DataSketches (HLL_8 backing, lg_k = 12 by default).
// Mirrors cpp-bench/cs/datasketches_cs.cpp and supersedes the legacy
// cpp-bench/legacy/throughput/hll/ + accuracy/cardinality/ harnesses.
//
// Includes `hll.hpp` from the Insert-Optimized fork's apache layout
// (${INSERT_OPT_ROOT}/hll/include). The fork tracks apache/datasketches-cpp
// for non-customised families, so the public API (`hll_sketch::update`,
// `get_estimate`, `HLL_8`) is unchanged.

#include <chrono>
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

    std::vector<cpp_bench::RunResult> raw =
        cpp_bench::run_throughput_latency_raw<HllSketch>(wl.items, cfg, build, insert);
    cpp_bench::AggregatedMetrics m = cpp_bench::aggregate(raw);

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

    if (args.legacy_csv_path) {
        std::ofstream csv(*args.legacy_csv_path);
        if (!csv) { std::cerr << "cpp-bench: cannot write " << *args.legacy_csv_path << '\n'; return 2; }
        const std::size_t total_items = std::min(cfg.measure_items, wl.items.size());
        const std::size_t registers   = std::size_t{1} << lg_k;
        csv << "implementation,language,run,lg_k,registers,total_items,"
               "total_nanoseconds,throughput_items_per_sec\n";
        for (std::size_t i = 0; i < raw.size(); ++i) {
            const double tput = raw[i].throughput_items_per_sec;
            const long long total_ns = tput > 0.0
                ? static_cast<long long>(static_cast<double>(total_items) * 1e9 / tput)
                : 0LL;
            csv << "cpp_datasketches_hll,cpp," << (i + 1) << ','
                << static_cast<int>(lg_k) << ',' << registers << ','
                << total_items << ',' << total_ns << ',' << tput << '\n';
        }
    }

    // Query-throughput pass: HLL has a single nullary query (get_estimate).
    // Mirror the Rust CardinalityGT comparator (4096 repeats per run).
    if (args.query_csv_path) {
        std::ofstream qcsv(*args.query_csv_path);
        if (!qcsv) { std::cerr << "cpp-bench: cannot write " << *args.query_csv_path << '\n'; return 2; }
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());
        const std::size_t registers = std::size_t{1} << lg_k;

        HllSketch sketch(lg_k, datasketches::HLL_8);
        for (std::size_t i = 0; i < measure_n; ++i) sketch.update(wl.items[i]);

        const std::size_t repeats = 4096;
        qcsv << "implementation,language,run,lg_k,registers,total_items,"
                "total_queries,total_nanoseconds,throughput_queries_per_sec\n";
        {
            double sink = 0.0;
            for (std::size_t i = 0; i < repeats; ++i) sink += sketch.get_estimate();
            asm volatile("" : : "r"(sink) : "memory");
        }
        for (std::size_t run = 0; run < cfg.runs; ++run) {
            double sink = 0.0;
            auto t0 = std::chrono::steady_clock::now();
            for (std::size_t i = 0; i < repeats; ++i) sink += sketch.get_estimate();
            auto t1 = std::chrono::steady_clock::now();
            asm volatile("" : : "r"(sink) : "memory");
            const long long total_ns =
                std::chrono::duration_cast<std::chrono::nanoseconds>(t1 - t0).count();
            const double tput = total_ns > 0
                ? static_cast<double>(repeats) * 1e9 / static_cast<double>(total_ns)
                : 0.0;
            qcsv << "cpp_datasketches_hll,cpp," << (run + 1) << ','
                 << static_cast<int>(lg_k) << ',' << registers << ','
                 << measure_n << ',' << repeats << ',' << total_ns << ',' << tput << '\n';
        }
    }
    return 0;
}
