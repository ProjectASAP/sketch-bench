#pragma once

#include <cmath>
#include <cstdint>
#include <vector>

#include "baseline.hpp"
#include "hll.hpp"
#include "output.hpp"
#include "seeds.hpp"

inline constexpr uint8_t kLgK = 14;
inline constexpr size_t kRegisters = size_t{1} << kLgK;

inline uint64_t splitmix64(uint64_t x) {
  x += 0x9E3779B97F4A7C15ULL;
  x = (x ^ (x >> 30)) * 0xBF58476D1CE4E5B9ULL;
  x = (x ^ (x >> 27)) * 0x94D049BB133111EBULL;
  return x ^ (x >> 31);
}

inline uint64_t seeded_key(int64_t value, uint64_t seed) {
  return splitmix64(static_cast<uint64_t>(value) ^ (seed * 0x9E3779B97F4A7C15ULL));
}

inline std::vector<AccuracyRow> run_datasketches_summary(const BaselineData& baseline) {
  std::vector<AccuracyRow> rows;
  rows.reserve(kSeeds.size());

  for (const uint64_t seed : kSeeds) {
    datasketches::hll_sketch sketch(kLgK, datasketches::HLL_8);
    for (const int64_t value : baseline.values) {
      sketch.update(seeded_key(value, seed));
    }
    const double estimate = sketch.get_estimate();
    const double relative_error =
        std::abs(estimate - static_cast<double>(baseline.distinct_items)) /
        static_cast<double>(baseline.distinct_items);
    rows.push_back(AccuracyRow{
        "cpp_datasketches_hll",
        "cpp",
        seed,
        kLgK,
        kRegisters,
        baseline.values.size(),
        baseline.distinct_items,
        estimate,
        relative_error,
    });
  }

  return rows;
}
