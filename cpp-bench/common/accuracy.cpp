#include "accuracy.hpp"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <sstream>
#include <unordered_map>
#include <vector>

namespace cpp_bench {

namespace {

void write_double(std::ostringstream& o, double v) {
    if (std::isnan(v) || std::isinf(v)) {
        o << "null";
        return;
    }
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.17g", v);
    o << buf;
}

// Index in the sorted-ascending vector where `v` would belong
// (lower bound). For an int64 `v`, that's the count of elements
// strictly less than `v`, i.e. exactly the true rank.
std::size_t true_rank(const std::vector<std::int64_t>& sorted, std::int64_t v) {
    auto it = std::lower_bound(sorted.begin(), sorted.end(), v);
    return static_cast<std::size_t>(it - sorted.begin());
}

}  // namespace

QuantileAccuracy compute_quantile_accuracy(
    const std::vector<std::int64_t>& items,
    const std::vector<double>& queries,
    const std::function<std::int64_t(double q)>& estimate_q)
{
    QuantileAccuracy out;
    if (items.empty() || queries.empty()) {
        out.json = R"({"queries":[],"abs_rank_err":{"mean":0,"max":0},"rel_rank_err":{"mean":0,"max":0}})";
        return out;
    }

    std::vector<std::int64_t> sorted(items);
    std::sort(sorted.begin(), sorted.end());
    const double N = static_cast<double>(sorted.size());

    double abs_sum = 0.0, abs_max = 0.0;
    double rel_sum = 0.0, rel_max = 0.0;
    std::size_t rel_n = 0;

    for (double q : queries) {
        const std::int64_t est_v = estimate_q(q);
        const std::size_t est_rank = true_rank(sorted, est_v);
        const double true_rank_frac = q;
        const double est_rank_frac  = static_cast<double>(est_rank) / N;
        const double abs_err = std::fabs(est_rank_frac - true_rank_frac);
        abs_sum += abs_err;
        if (abs_err > abs_max) abs_max = abs_err;

        if (true_rank_frac > 0.0) {
            const double true_rank_abs = true_rank_frac * N;
            const double est_rank_abs  = static_cast<double>(est_rank);
            const double rel_err = std::fabs(est_rank_abs - true_rank_abs) / true_rank_abs;
            rel_sum += rel_err;
            if (rel_err > rel_max) rel_max = rel_err;
            ++rel_n;
        }
    }
    const double abs_mean = abs_sum / static_cast<double>(queries.size());
    const double rel_mean = rel_n > 0 ? rel_sum / static_cast<double>(rel_n) : 0.0;

    std::ostringstream o;
    o << "{\"queries\":[";
    for (std::size_t i = 0; i < queries.size(); ++i) {
        if (i) o << ',';
        write_double(o, queries[i]);
    }
    o << "],\"abs_rank_err\":{\"mean\":";
    write_double(o, abs_mean);
    o << ",\"max\":";
    write_double(o, abs_max);
    o << "},\"rel_rank_err\":{\"mean\":";
    write_double(o, rel_mean);
    o << ",\"max\":";
    write_double(o, rel_max);
    o << "}}";
    out.json = o.str();
    return out;
}

FrequencyAccuracy compute_frequency_accuracy(
    const std::vector<std::int64_t>& items,
    std::size_t top_k,
    const std::function<std::uint64_t(std::int64_t key)>& estimate_freq)
{
    FrequencyAccuracy out;

    std::unordered_map<std::int64_t, std::uint64_t> truth;
    truth.reserve(items.size());
    for (auto v : items) ++truth[v];

    std::vector<std::pair<std::int64_t, std::uint64_t>> ranked(truth.begin(), truth.end());
    if (ranked.empty()) {
        out.json = R"({"top_k":0,"abs_freq_err":{"mean":0,"p99":0},"rel_freq_err":{"mean":0,"p99":0}})";
        return out;
    }
    const std::size_t k = std::min(top_k, ranked.size());
    std::partial_sort(
        ranked.begin(), ranked.begin() + static_cast<std::ptrdiff_t>(k), ranked.end(),
        [](const auto& a, const auto& b) { return a.second > b.second; });

    std::vector<double> abs_errs;
    std::vector<double> rel_errs;
    abs_errs.reserve(k);
    rel_errs.reserve(k);
    for (std::size_t i = 0; i < k; ++i) {
        const auto [key, true_c] = ranked[i];
        const std::uint64_t est_c = estimate_freq(key);
        const double abs_e = static_cast<double>(
            est_c > true_c ? est_c - true_c : true_c - est_c);
        abs_errs.push_back(abs_e);
        if (true_c > 0) {
            rel_errs.push_back(abs_e / static_cast<double>(true_c));
        }
    }

    auto stats = [](std::vector<double>& v) {
        if (v.empty()) return std::pair<double, double>{0.0, 0.0};
        double sum = 0.0;
        for (double x : v) sum += x;
        const double mean = sum / static_cast<double>(v.size());
        std::sort(v.begin(), v.end());
        const std::size_t idx = static_cast<std::size_t>(
            0.99 * static_cast<double>(v.size() - 1) + 0.5);
        return std::pair<double, double>{mean, v[idx]};
    };
    auto [abs_mean, abs_p99] = stats(abs_errs);
    auto [rel_mean, rel_p99] = stats(rel_errs);

    std::ostringstream o;
    o << "{\"top_k\":" << k
      << ",\"abs_freq_err\":{\"mean\":";
    write_double(o, abs_mean);
    o << ",\"p99\":";
    write_double(o, abs_p99);
    o << "},\"rel_freq_err\":{\"mean\":";
    write_double(o, rel_mean);
    o << ",\"p99\":";
    write_double(o, rel_p99);
    o << "}}";
    out.json = o.str();
    return out;
}

CardinalityAccuracy compute_cardinality_accuracy(
    const std::vector<std::int64_t>& items,
    double estimated_cardinality)
{
    CardinalityAccuracy out;

    std::unordered_map<std::int64_t, char> uniq;
    uniq.reserve(items.size());
    for (auto v : items) uniq.emplace(v, 1);
    const double truth = static_cast<double>(uniq.size());

    double rel = 0.0;
    if (truth > 0.0) {
        rel = std::fabs(estimated_cardinality - truth) / truth;
    }

    std::ostringstream o;
    o << "{\"rel_card_err\":";
    write_double(o, rel);
    o << '}';
    out.json = o.str();
    return out;
}

}  // namespace cpp_bench
