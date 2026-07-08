#pragma once

// Cross-language accuracy contract for the v1 schema's `accuracy`
// payload. See docs/archive/SCHEMA_V1.md for the per-family JSON shape.
//
// Each helper returns a pre-serialised JSON object so it can be
// dropped straight into `BenchSection::accuracy_json`.

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace cpp_bench {

// ---------- Quantile family (KLL, t-digest, ...) ----------

struct QuantileAccuracy {
    std::string json;
};

// `estimate_q` should answer the sketch's estimated quantile for a
// given q in [0,1]. The exact baseline is built from `items` here.
QuantileAccuracy compute_quantile_accuracy(
    const std::vector<std::int64_t>& items,
    const std::vector<double>& queries,
    const std::function<std::int64_t(double q)>& estimate_q);

// ---------- Frequency family (CMS, CS) ----------

struct FrequencyAccuracy {
    std::string json;
};

// `estimate_freq` answers the sketch's estimated count for a key.
// Computes per-key absolute and relative error over `top_k` highest-
// frequency keys (chosen via the exact baseline of `items`).
FrequencyAccuracy compute_frequency_accuracy(
    const std::vector<std::int64_t>& items,
    std::size_t top_k,
    const std::function<std::uint64_t(std::int64_t key)>& estimate_freq);

// ---------- Cardinality family (HLL) ----------

struct CardinalityAccuracy {
    std::string json;
};

CardinalityAccuracy compute_cardinality_accuracy(
    const std::vector<std::int64_t>& items,
    double estimated_cardinality);

}  // namespace cpp_bench
