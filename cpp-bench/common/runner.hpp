#pragma once

// N-run benchmark loop + aggregation. Mirrors what the Rust
// `BenchRunner` does (sketch-bench/src/runner.rs): warmup, then N
// measured passes, then aggregate throughput + sampled latency
// into the v1 schema's RunStats / LatencySummary.
//
// Throughput and latency are measured in STRICTLY SEPARATE passes —
// throughput's hot loop has no sampling branch so it's not contaminated
// by the latency sampling cost (which is 40-60% on fast sketches).
//
// Each `BinaryFixture<Sketch>` describes how to build a fresh
// sketch instance (`build`) and how to ingest one item
// (`insert(sketch, item)`); the runner takes care of timing,
// `benchmark::DoNotOptimize`, and aggregation.

#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <functional>
#include <random>
#include <string>
#include <vector>

#include <benchmark/benchmark.h>

#include "record_v1.hpp"

namespace cpp_bench {

// Burn CPU on the current core so the cpufreq governor ramps to max turbo
// before timing starts. Mirrors `warmup_cpu_from_env` on the Rust side
// (sketch-bench/src/runner.rs). External shell warmups don't work
// reliably — the governor can drop frequency during the bench process's
// exec/startup window. Inserting into a sketch as warmup is also
// insufficient because the counter-array stores include memory stalls
// that the governor interprets as idle.
//
// Duration is read from BENCH_WARMUP_SECS (default 10s). Set to 0 to skip.
inline void warmup_cpu_from_env() {
    const char* env = std::getenv("BENCH_WARMUP_SECS");
    long secs = 10;
    if (env) {
        char* end = nullptr;
        long v = std::strtol(env, &end, 10);
        if (end != env && v >= 0) secs = v;
    }
    if (secs == 0) return;
    const auto deadline = std::chrono::steady_clock::now()
                        + std::chrono::seconds(secs);
    std::uint64_t x = 0xdeadbeefULL;
    while (std::chrono::steady_clock::now() < deadline) {
        for (int i = 0; i < 10000; ++i) {
            x = x * 6364136223846793005ULL + 1442695040888963407ULL;
        }
        benchmark::DoNotOptimize(x);
    }
}

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

// Run a single throughput pass: pure insert loop, ZERO instrumentation
// branches in the hot path. The latency-sampling branch + modulo in a
// fused throughput+latency loop costs 40-60% throughput on fast sketches,
// so the two metrics are measured in independent passes.
template <typename Sketch, typename BuildFn, typename InsertFn>
double measure_throughput_one_pass(
    const std::vector<std::int64_t>& items,
    std::size_t measure_n,
    BuildFn  build,
    InsertFn insert)
{
    Sketch s = build();
    benchmark::DoNotOptimize(s);

    const auto t_start = std::chrono::steady_clock::now();
    for (std::size_t i = 0; i < measure_n; ++i) {
        insert(s, items[i]);
    }
    benchmark::DoNotOptimize(s);
    benchmark::ClobberMemory();
    const auto t_end = std::chrono::steady_clock::now();

    const double elapsed_s = std::chrono::duration_cast<std::chrono::duration<double>>(
                                 t_end - t_start).count();
    return elapsed_s > 0.0 ? static_cast<double>(measure_n) / elapsed_s : 0.0;
}

// Run a single latency pass: samples per-item time every `stride` inserts.
// Returns the raw samples (one entry every `stride` items). Throughput is
// NOT measured here — use `measure_throughput_one_pass` for that.
template <typename Sketch, typename BuildFn, typename InsertFn>
std::vector<std::uint64_t> measure_latency_one_pass(
    const std::vector<std::int64_t>& items,
    std::size_t measure_n,
    std::size_t stride,
    BuildFn  build,
    InsertFn insert)
{
    std::vector<std::uint64_t> samples;
    if (stride == 0 || measure_n == 0) return samples;
    samples.reserve(measure_n / stride);

    Sketch s = build();
    benchmark::DoNotOptimize(s);

    auto t_last_sample = std::chrono::steady_clock::now();
    for (std::size_t i = 0; i < measure_n; ++i) {
        insert(s, items[i]);
        if ((i + 1) % stride == 0) {
            const auto now = std::chrono::steady_clock::now();
            const auto dt_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                                   now - t_last_sample).count();
            samples.push_back(
                static_cast<std::uint64_t>(dt_ns / static_cast<long long>(stride)));
            t_last_sample = now;
        }
    }
    benchmark::DoNotOptimize(s);
    benchmark::ClobberMemory();
    return samples;
}

// Run the benchmark and return per-run RunResult. Throughput and latency
// are measured in STRICTLY SEPARATE passes (throughput pass has a clean
// hot loop with no sampling branch). If `cfg.latency_sample_stride == 0`,
// only the throughput pass runs.
template <typename Sketch, typename BuildFn, typename InsertFn>
std::vector<RunResult> run_throughput_latency_raw(
    const std::vector<std::int64_t>& items,
    const RunnerConfig& cfg,
    BuildFn  build,
    InsertFn insert)
{
    const std::size_t warmup_n  = std::min(cfg.warmup_items,  items.size());
    const std::size_t measure_n = std::min(cfg.measure_items, items.size());

    // Pin CPU at turbo via pure-arithmetic burn. Without this the cpufreq
    // governor sees the brief I/O + setup at process start as idle and
    // throttles measurements to ~1/3 of turbo throughput.
    warmup_cpu_from_env();

    // One warmup pass to dirty caches / page in the workload.
    {
        Sketch s = build();
        for (std::size_t i = 0; i < warmup_n; ++i) {
            insert(s, items[i]);
        }
        benchmark::DoNotOptimize(s);
        benchmark::ClobberMemory();
    }

    std::vector<RunResult> results(cfg.runs);

    // Pass 1: throughput. Clean hot loop, no sampling.
    for (std::size_t r = 0; r < cfg.runs; ++r) {
        results[r].throughput_items_per_sec =
            measure_throughput_one_pass<Sketch>(items, measure_n, build, insert);
    }

    // Pass 2: latency. Only if stride > 0.
    const std::size_t stride = cfg.latency_sample_stride;
    if (stride > 0) {
        for (std::size_t r = 0; r < cfg.runs; ++r) {
            results[r].latency_samples_ns =
                measure_latency_one_pass<Sketch>(items, measure_n, stride, build, insert);
        }
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
