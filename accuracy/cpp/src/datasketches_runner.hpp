#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <vector>

#define private public
#include "cs/cs_datasketches.hpp"
#undef private

#include "baseline.hpp"
#include "output.hpp"
#include "seeds.hpp"

inline uint64_t estimate_value(const datasketches::CountMinSketch<int64_t>& sketch, int64_t value) {
  uint64_t min_estimate = std::numeric_limits<uint64_t>::max();
  uint64_t hash_seed_index = 0;
  for (const auto& hash_seed : sketch.hash_seeds) {
    const auto hash = MurmurHash3_x64_128(&value, sizeof(value), hash_seed);
    const uint64_t bucket = hash % sketch._num_buckets;
    const size_t index = static_cast<size_t>(hash_seed_index * sketch._num_buckets + bucket);
    min_estimate = std::min<uint64_t>(min_estimate, sketch._sketch_array[index]);
    hash_seed_index += 1;
  }
  return min_estimate;
}

inline std::vector<AccuracyRow> run_datasketches(const BaselineData& baseline) {
  constexpr size_t kRows = 3;
  constexpr size_t kCols[] = {2048, 4096, 8192};

  std::vector<AccuracyRow> results;
  results.reserve(kSeeds.size() * 3);

  for (const uint64_t seed : kSeeds) {
    for (const size_t cols : kCols) {
      datasketches::CountMinSketch<int64_t> sketch(
          static_cast<uint8_t>(kRows), static_cast<uint32_t>(cols), seed);
      for (const int64_t value : baseline.values) {
        sketch.Insert(value);
      }

      double total_relative_error = 0.0;
      double max_relative_error = 0.0;
      double total_absolute_error = 0.0;
      for (const auto& entry : baseline.frequencies) {
        const uint64_t estimate = estimate_value(sketch, entry.first);
        const double absolute_error =
            static_cast<double>(estimate > entry.second ? estimate - entry.second : entry.second - estimate);
        const double relative_error = absolute_error / static_cast<double>(entry.second);
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        max_relative_error = std::max(max_relative_error, relative_error);
      }

      const double distinct = static_cast<double>(baseline.frequencies.size());
      results.push_back(AccuracyRow{
          "cpp_datasketches_cms",
          "cpp",
          seed,
          kRows,
          cols,
          baseline.values.size(),
          baseline.frequencies.size(),
          total_relative_error / distinct,
          max_relative_error,
          total_absolute_error / distinct,
      });
    }
  }

  return results;
}
