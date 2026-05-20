// KLL via upstream Apache DataSketches (kll_sketch<int64_t>).
//
// The companion `datasketches_kll` binary uses the Insert-Optimized fork's
// `KarninLangLiberty`, which lacks a quantile query API — so for the query
// side of the throughput v2 charts we need an upstream build with
// `kll_sketch::get_quantile`.
//
// Headers come from $DATASKETCHES_UPSTREAM_ROOT (default
// ~/datasketches-cpp), wired up in cpp-bench/kll/CMakeLists.txt.

#include <array>
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

#include "kll_sketch.hpp"

using KllSketch = datasketches::kll_sketch<std::int64_t>;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);
    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;
    if (args.rng_seed)              cfg.rng_seed = *args.rng_seed;

    const std::uint16_t k = static_cast<std::uint16_t>(args.k.value_or(200));

    auto build  = [&]() { return KllSketch(k); };
    auto insert = [](KllSketch& s, std::int64_t v) { s.update(v); };

    std::vector<cpp_bench::RunResult> raw =
        cpp_bench::run_throughput_latency_raw<KllSketch>(wl.items, cfg, build, insert);
    cpp_bench::AggregatedMetrics m = cpp_bench::aggregate(raw);

    cpp_bench::Record rec;
    rec.sketch    = "kll";
    rec.impl_name = "datasketches_upstream";
    rec.language  = "cpp";
    rec.workload  = wl.desc;
    rec.runs      = cfg.runs;
    rec.bench.throughput_items_per_sec = m.throughput;
    if (m.latency.count > 0) rec.bench.latency_ns = m.latency;

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
        csv << "implementation,language,run,k,total_items,"
               "total_nanoseconds,throughput_items_per_sec\n";
        for (std::size_t i = 0; i < raw.size(); ++i) {
            const double tput = raw[i].throughput_items_per_sec;
            const long long total_ns = tput > 0.0
                ? static_cast<long long>(static_cast<double>(total_items) * 1e9 / tput)
                : 0LL;
            csv << "cpp_datasketches_upstream_kll,cpp," << (i + 1) << ',' << k << ','
                << total_items << ',' << total_ns << ',' << tput << '\n';
        }
    }

    // Query-throughput pass: cycle through {p50, p95, p99, p999}, mirroring
    // the Rust comparator's query mix. 1024 repeats per run keeps the loop
    // short enough that the steady_clock noise floor dominates only the
    // smallest measurements.
    if (args.query_csv_path) {
        std::ofstream qcsv(*args.query_csv_path);
        if (!qcsv) { std::cerr << "cpp-bench: cannot write " << *args.query_csv_path << '\n'; return 2; }
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());

        KllSketch sketch(k);
        for (std::size_t i = 0; i < measure_n; ++i) sketch.update(wl.items[i]);

        const std::array<double, 4> qs = {0.5, 0.95, 0.99, 0.999};
        const std::size_t repeats = 1024;
        qcsv << "implementation,language,run,k,total_items,"
                "total_queries,total_nanoseconds,throughput_queries_per_sec\n";
        {
            std::int64_t sink = 0;
            for (std::size_t i = 0; i < repeats; ++i) sink += sketch.get_quantile(qs[i & 3]);
            asm volatile("" : : "r"(sink) : "memory");
        }
        for (std::size_t run = 0; run < cfg.runs; ++run) {
            std::int64_t sink = 0;
            auto t0 = std::chrono::steady_clock::now();
            for (std::size_t i = 0; i < repeats; ++i) sink += sketch.get_quantile(qs[i & 3]);
            auto t1 = std::chrono::steady_clock::now();
            asm volatile("" : : "r"(sink) : "memory");
            const long long total_ns =
                std::chrono::duration_cast<std::chrono::nanoseconds>(t1 - t0).count();
            const double tput = total_ns > 0
                ? static_cast<double>(repeats) * 1e9 / static_cast<double>(total_ns)
                : 0.0;
            qcsv << "cpp_datasketches_upstream_kll,cpp," << (run + 1) << ',' << k << ','
                 << measure_n << ',' << repeats << ',' << total_ns << ',' << tput << '\n';
        }
    }

    // Per-call query CSV: matches the Rust comparator schema (one row per
    // (run, repeat, percentile)). Wraps each call in its own steady_clock
    // pair so timer overhead (~25-40ns vDSO) is included — directly
    // comparable to the Rust per-call number.
    if (args.query_percall_csv_path) {
        std::ofstream pcsv(*args.query_percall_csv_path);
        if (!pcsv) { std::cerr << "cpp-bench: cannot write " << *args.query_percall_csv_path << '\n'; return 2; }
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());

        KllSketch sketch(k);
        for (std::size_t i = 0; i < measure_n; ++i) sketch.update(wl.items[i]);

        const std::size_t repeats_per_run = 10;
        const std::size_t percentile_grid = 101;
        pcsv << "implementation,language,run,k,total_items,"
                "repeat,percentile,call_index,nanoseconds,estimate\n";
        { std::int64_t s = sketch.get_quantile(0.5); asm volatile("" : : "r"(s) : "memory"); }
        for (std::size_t run = 0; run < cfg.runs; ++run) {
            std::size_t call_index = 0;
            for (std::size_t repeat = 1; repeat <= repeats_per_run; ++repeat) {
                for (std::size_t p = 0; p < percentile_grid; ++p) {
                    const double rank = static_cast<double>(p) / 100.0;
                    ++call_index;
                    auto t0 = std::chrono::steady_clock::now();
                    std::int64_t est = sketch.get_quantile(rank);
                    auto t1 = std::chrono::steady_clock::now();
                    asm volatile("" : : "r"(est) : "memory");
                    const long long ns =
                        std::chrono::duration_cast<std::chrono::nanoseconds>(t1 - t0).count();
                    pcsv << "cpp_datasketches_upstream_kll,cpp," << (run + 1) << ',' << k << ','
                         << measure_n << ',' << repeat << ',' << rank << ','
                         << call_index << ',' << ns << ',' << est << '\n';
                }
            }
        }
    }
    return 0;
}
