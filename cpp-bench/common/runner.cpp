#include "runner.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace cpp_bench {

namespace {

RunStats compute_run_stats(const std::vector<double>& samples) {
    RunStats s;
    s.n = samples.size();
    if (s.n == 0) {
        return s;
    }
    double sum = 0.0;
    for (double v : samples) sum += v;
    s.mean = sum / static_cast<double>(s.n);

    if (s.n > 1) {
        double sq = 0.0;
        for (double v : samples) {
            const double d = v - s.mean;
            sq += d * d;
        }
        // Sample stddev (Bessel).
        s.stddev = std::sqrt(sq / static_cast<double>(s.n - 1));
        // No confidence interval is computed here. These samples are
        // iterations inside one process — one allocator arena, one
        // address-space layout, one governor ramp — so they are not
        // independent draws of this implementation's throughput, and
        // mean +- 1.96*stddev/sqrt(n) over them yields an interval far
        // tighter than the command's own run-to-run reproducibility.
    } else {
        s.stddev = 0.0;
    }
    return s;
}

std::uint64_t percentile(std::vector<std::uint64_t>& sorted, double q) {
    if (sorted.empty()) return 0;
    // Nearest-rank percentile.
    double idx_d = q * static_cast<double>(sorted.size() - 1);
    std::size_t idx = static_cast<std::size_t>(idx_d + 0.5);
    if (idx >= sorted.size()) idx = sorted.size() - 1;
    return sorted[idx];
}

LatencySummary compute_latency(std::vector<std::uint64_t> all_samples) {
    LatencySummary l;
    if (all_samples.empty()) {
        return l;
    }
    std::sort(all_samples.begin(), all_samples.end());
    l.count = static_cast<std::uint64_t>(all_samples.size());
    l.p50  = percentile(all_samples, 0.50);
    l.p95  = percentile(all_samples, 0.95);
    l.p99  = percentile(all_samples, 0.99);
    l.p999 = percentile(all_samples, 0.999);
    l.max  = all_samples.back();
    return l;
}

}  // namespace

AggregatedMetrics aggregate(const std::vector<RunResult>& runs) {
    AggregatedMetrics m;

    std::vector<double> tput;
    tput.reserve(runs.size());
    std::size_t total_samples = 0;
    for (const auto& r : runs) {
        tput.push_back(r.throughput_items_per_sec);
        total_samples += r.latency_samples_ns.size();
    }
    m.throughput = compute_run_stats(tput);

    if (total_samples > 0) {
        std::vector<std::uint64_t> merged;
        merged.reserve(total_samples);
        for (const auto& r : runs) {
            merged.insert(merged.end(),
                          r.latency_samples_ns.begin(),
                          r.latency_samples_ns.end());
        }
        m.latency = compute_latency(std::move(merged));
    }
    return m;
}

}  // namespace cpp_bench
