#pragma once

#include <array>
#include <cmath>
#include <cstdint>
#include <vector>

#include "baseline.hpp"
#include "kll_sketch.hpp"
#include "output.hpp"

inline constexpr std::array<int, 5> kKList = {50, 100, 200, 400, 800};
inline constexpr int kNumPercentiles = 101;

inline double compute_relative_error(double true_q, double estimate) {
  if (std::abs(true_q) < 1e-15) {
    return (std::abs(estimate) < 1e-15) ? 0.0 : std::abs(estimate);
  }
  return std::abs(estimate - true_q) / std::abs(true_q);
}

inline std::vector<AccuracyRow> run_datasketches_summary(
    const BaselineData& baseline) {
  std::vector<AccuracyRow> rows;
  rows.reserve(kKList.size() * kNumPercentiles);

  for (const int k : kKList) {
    datasketches::kll_sketch<int64_t> sketch(k);
    for (const int64_t value : baseline.values) {
      sketch.update(value);
    }
    for (int p = 0; p < kNumPercentiles; ++p) {
      const double rank = static_cast<double>(p) / 100.0;
      const double true_q = baseline.ground_truth_quantile(p);
      const double estimate = static_cast<double>(sketch.get_quantile(rank));
      rows.push_back(AccuracyRow{
          "cpp_datasketches_kll",
          "cpp",
          k,
          p,
          baseline.values.size(),
          true_q,
          estimate,
          compute_relative_error(true_q, estimate),
      });
    }
  }

  return rows;
}
