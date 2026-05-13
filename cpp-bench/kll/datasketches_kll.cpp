// KLL via Apache DataSketches (Insert-Optimized fork), emitted as
// a v1 JSONL Record. Phase 1: throughput + latency only; accuracy
// is wired in by phase 2.

#include <cstddef>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <ostream>

#include "common/accuracy.hpp"
#include "common/cli.hpp"
#include "common/record_v1.hpp"
#include "common/runner.hpp"
#include "common/workload.hpp"

// Mirror cpp/kll/kll_datasketches.cpp's namespace dance so the
// insert-optimized fork's headers don't collide with anything else
// in this binary.
namespace insert_opt_datasketches_namespace {}
#define datasketches insert_opt_datasketches_namespace
#include "kll/kll_datasketches.hpp"
#undef datasketches
namespace insert_opt_datasketches = insert_opt_datasketches_namespace;

using KllSketch = insert_opt_datasketches::KarninLangLiberty<std::int64_t>;

int main(int argc, char** argv) {
    cpp_bench::CliArgs args = cpp_bench::parse_cli(argc, argv);

    cpp_bench::Workload wl = cpp_bench::load_i64_bin(args.workload_file);

    cpp_bench::RunnerConfig cfg;
    if (args.warmup_items)          cfg.warmup_items = *args.warmup_items;
    if (args.measure_items)         cfg.measure_items = *args.measure_items;
    if (args.runs)                  cfg.runs = *args.runs;
    if (args.latency_sample_stride) cfg.latency_sample_stride = *args.latency_sample_stride;
    if (args.rng_seed)              cfg.rng_seed = *args.rng_seed;

    const std::size_t k = args.k.value_or(200);

    auto build  = [&]() { return KllSketch(k); };
    auto insert = [](KllSketch& s, std::int64_t v) { s.Insert(v); };

    cpp_bench::AggregatedMetrics m =
        cpp_bench::run_throughput_latency<KllSketch>(wl.items, cfg, build, insert);

    cpp_bench::Record rec;
    rec.sketch     = "kll";
    rec.impl_name  = "datasketches";
    rec.language   = "cpp";
    rec.workload   = wl.desc;
    rec.runs       = cfg.runs;
    rec.bench.throughput_items_per_sec = m.throughput;
    if (m.latency.count > 0) rec.bench.latency_ns = m.latency;

    // ---- Accuracy ----
    //
    // One extra build+insert pass over items[0..measure_n], then query
    // each quantile in {0.5, 0.95, 0.99, 0.999}. The exact baseline
    // is built from the same prefix in compute_quantile_accuracy().
    if (args.with_accuracy) {
        const std::size_t measure_n = std::min(cfg.measure_items, wl.items.size());
        std::vector<std::int64_t> measured_prefix(
            wl.items.begin(),
            wl.items.begin() + static_cast<std::ptrdiff_t>(measure_n));

        KllSketch sketch(k);
        for (auto v : measured_prefix) sketch.Insert(v);

        // NOTE on method name: the insert-optimized fork's
        // KarninLangLiberty exposes its quantile query as
        // `GetQuantile(q)`. If the actual API in your checkout uses a
        // different spelling (e.g. `Quantile`, `get_quantile`), adjust
        // the call below — it's intentionally isolated to one line.
        const std::vector<double> queries = {0.5, 0.95, 0.99, 0.999};
        auto estimate_q = [&](double q) -> std::int64_t {
            return sketch.GetQuantile(q);
        };
        cpp_bench::QuantileAccuracy qa = cpp_bench::compute_quantile_accuracy(
            measured_prefix, queries, estimate_q);
        rec.bench.accuracy_json = qa.json;
    }

    if (args.report_path == "-" || args.report_path.empty()) {
        rec.emit(std::cout);
    } else {
        std::ofstream out(args.report_path, std::ios::app);
        if (!out) {
            std::cerr << "cpp-bench: cannot write " << args.report_path << '\n';
            return 2;
        }
        rec.emit(out);
    }
    return 0;
}
