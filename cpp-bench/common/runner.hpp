#pragma once

// N-run benchmark loop + aggregation. Mirrors what the Rust
// `BenchRunner` does (sketch-bench/src/runner.rs): warmup, then N
// measured passes, then aggregate throughput + sampled latency
// into the v1 schema's RunStats / LatencySummary.
//
// Each `BinaryFixture<Sketch>` describes how to build a fresh
// sketch instance (`build`) and how to ingest one item
// (`insert(sketch, item)`); the runner takes care of timing,
// `benchmark::DoNotOptimize`, and aggregation.

#include <cstdint>
#include <functional>
#include <random>
#include <vector>

#include <benchmark/benchmark.h>

#include "record_v1.hpp"

namespace cpp_bench {

struct RunnerConfig {
    std::size_t warmup_items = 100'000;
    std::size_t measure_items = 1'000'000;   // capped at workload size
    std::size_t runs = 10;
    // Latency-sampling stride: time every Nth insert. 0 = no
    // latency sampling. 1000 gives ~1000 samples per 1M-item run,
    // which is enough for stable p99 / p999 estimates without
    // distorting throughput.
    std::size_t latency_sample_stride = 1000;
    std::uint64_t rng_seed = 0x5e7c4011ull;
};

// Result of one measured run: aggregated wall-time + sampled latencies.
struct RunResult {
    double throughput_items_per_sec = 0.0;
    std::vector<std::uint64_t> latency_samples_ns;   // empty if not sampled
};

// Per-run aggregator output, ready to be dropped into BenchSection.
struct AggregatedMetrics {
    RunStats throughput;
    LatencySummary latency;       // empty count means no samples taken
};

// Aggregate `runs` independent measurements into a v1 BenchSection's
// throughput + latency fields.
AggregatedMetrics aggregate(const std::vector<RunResult>& runs);

// Run the benchmark loop and return the raw per-run results
// (no aggregation). Callers that need the legacy long-format CSV
// reach in here to get one row per run.
template <typename Sketch, typename BuildFn, typename InsertFn>
std::vector<RunResult> run_throughput_latency_raw(
    const std::vector<std::int64_t>& items,
    const RunnerConfig& cfg,
    BuildFn  build,
    InsertFn insert)
{
    const std::size_t warmup_n  = std::min(cfg.warmup_items,  items.size());
    const std::size_t measure_n = std::min(cfg.measure_items, items.size());

    // One warmup pass to dirty caches / page in the workload.
    {
        Sketch s = build();
        for (std::size_t i = 0; i < warmup_n; ++i) {
            insert(s, items[i]);
        }
        benchmark::DoNotOptimize(s);
        benchmark::ClobberMemory();
    }

    std::vector<RunResult> results;
    results.reserve(cfg.runs);

    const std::size_t stride = cfg.latency_sample_stride;

    for (std::size_t r = 0; r < cfg.runs; ++r) {
        RunResult rr;
        if (stride > 0 && measure_n > 0) {
            rr.latency_samples_ns.reserve(measure_n / std::max<std::size_t>(stride, 1));
        }

        Sketch s = build();
        benchmark::DoNotOptimize(s);

        const auto t_start = std::chrono::steady_clock::now();
        auto t_last_sample = t_start;

        for (std::size_t i = 0; i < measure_n; ++i) {
            insert(s, items[i]);
            if (stride > 0 && ((i + 1) % stride == 0)) {
                const auto now = std::chrono::steady_clock::now();
                const auto dt_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                                       now - t_last_sample).count();
                // Per-item ns averaged over the last `stride` inserts.
                rr.latency_samples_ns.push_back(
                    static_cast<std::uint64_t>(dt_ns / static_cast<long long>(stride)));
                t_last_sample = now;
            }
        }
        benchmark::DoNotOptimize(s);
        benchmark::ClobberMemory();
        const auto t_end = std::chrono::steady_clock::now();

        const double elapsed_s = std::chrono::duration_cast<std::chrono::duration<double>>(
                                     t_end - t_start).count();
        if (elapsed_s > 0.0) {
            rr.throughput_items_per_sec = static_cast<double>(measure_n) / elapsed_s;
        }
        results.push_back(std::move(rr));
    }

    return results;
}

// Backwards-compatible aggregating wrapper.
template <typename Sketch, typename BuildFn, typename InsertFn>
AggregatedMetrics run_throughput_latency(
    const std::vector<std::int64_t>& items,
    const RunnerConfig& cfg,
    BuildFn  build,
    InsertFn insert)
{
    return aggregate(run_throughput_latency_raw<Sketch>(items, cfg, build, insert));
}

}  // namespace cpp_bench
