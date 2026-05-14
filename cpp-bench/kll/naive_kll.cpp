// KLL via the insert-optimized "naive" variant.

#include <cstddef>
#include <cstdint>
#include <fstream>
#include <iostream>

#include "common/accuracy.hpp"
#include "common/cli.hpp"
#include "common/record_v1.hpp"
#include "common/runner.hpp"
#include "common/workload.hpp"

#include "kll/kll_naive.hpp"

using KllImpl = naive::KarninLangLiberty<std::int64_t>;

// Matches the `k` used by the legacy cpp/ Google-Benchmark variants so the
// optimization-evolution numbers stay directly comparable.
static constexpr std::size_t kKllK = 200;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);
    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;

    auto build  = []()  { return KllImpl{kKllK}; };
    auto insert = [](KllImpl& s, std::int64_t v) { s.Insert(v); };

    std::vector<cpp_bench::RunResult> raw =
        cpp_bench::run_throughput_latency_raw<KllImpl>(wl.items, cfg, build, insert);
    cpp_bench::AggregatedMetrics m = cpp_bench::aggregate(raw);

    cpp_bench::Record rec;
    rec.sketch    = "kll";
    rec.impl_name = "naive";
    rec.workload  = wl.desc;
    rec.runs      = cfg.runs;
    rec.bench.throughput_items_per_sec = m.throughput;
    if (m.latency.count > 0) rec.bench.latency_ns = m.latency;

#if 0  // accuracy block disabled: Insert-Opt lacks query API
    if (args.with_accuracy) {
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());
        std::vector<std::int64_t> prefix(
            wl.items.begin(),
            wl.items.begin() + static_cast<std::ptrdiff_t>(measure_n));

        KllImpl sketch{kKllK};
        for (auto v : prefix) sketch.Insert(v);

        const std::vector<double> queries = {0.5, 0.95, 0.99, 0.999};
        auto estimate_q = [&](double q) -> std::int64_t {
            return sketch.GetQuantile(q);
        };
        rec.bench.accuracy_json =
            cpp_bench::compute_quantile_accuracy(prefix, queries, estimate_q).json;
    }
#endif

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
            csv << "cpp_naive_kll,cpp," << (i + 1) << ',' << kKllK << ','
                << total_items << ',' << total_ns << ',' << tput << '\n';
        }
    }
    return 0;
}
