#pragma once

#include <algorithm>
#include <array>
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

inline std::vector<AccuracyRow> run_datasketches_summary(const BaselineData& baseline) {
  constexpr size_t kRows = 5;
  constexpr size_t kCols[] = {2048, 4096, 8192, 16384, 32768, 65536, 131072};
  const auto hitters = heavy_hitters(baseline);

  std::vector<AccuracyRow> results;
  results.reserve(kSeeds.size() * 7);

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
      for (const auto& entry : hitters) {
        const uint64_t estimate = estimate_value(sketch, entry.first);
        const double absolute_error = static_cast<double>(
            estimate > entry.second ? estimate - entry.second : entry.second - estimate);
        const double relative_error = absolute_error / static_cast<double>(entry.second);
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        max_relative_error = std::max(max_relative_error, relative_error);
      }

      const double distinct = static_cast<double>(hitters.size());
      results.push_back(AccuracyRow{
          "cpp_datasketches_cms",
          "cpp",
          seed,
          kRows,
          cols,
          baseline.values.size(),
          hitters.size(),
          total_relative_error / distinct,
          max_relative_error,
          total_absolute_error / distinct,
      });
    }
  }

  return results;
}

inline std::vector<KeyMedianErrorRow> run_datasketches_key_errors(const BaselineData& baseline) {
  constexpr size_t kRows = 5;
  constexpr size_t kCols[] = {2048, 4096, 8192, 16384, 32768, 65536, 131072};
  const auto hitters = heavy_hitters(baseline);

  std::vector<KeyMedianErrorRow> results;
  results.reserve(hitters.size() * 7);

  for (const size_t cols : kCols) {
    std::vector<datasketches::CountMinSketch<int64_t>> sketches;
    sketches.reserve(kSeeds.size());
    for (const uint64_t seed : kSeeds) {
      datasketches::CountMinSketch<int64_t> sketch(
          static_cast<uint8_t>(kRows), static_cast<uint32_t>(cols), seed);
      for (const int64_t value : baseline.values) {
        sketch.Insert(value);
      }
      sketches.push_back(std::move(sketch));
    }

    for (const auto& entry : hitters) {
      std::array<uint64_t, 10> estimates{};
      for (size_t i = 0; i < sketches.size(); ++i) {
        estimates[i] = estimate_value(sketches[i], entry.first);
      }
      std::sort(estimates.begin(), estimates.end());
      const uint64_t median_estimate = estimates[(estimates.size() / 2) - 1];
      const double median_relative_error =
          static_cast<double>(median_estimate > entry.second ? median_estimate - entry.second
                                                             : entry.second - median_estimate) /
          static_cast<double>(entry.second);

      results.push_back(KeyMedianErrorRow{
          "cpp_datasketches_cms",
          "cpp",
          kRows,
          cols,
          entry.first,
          entry.second,
          median_estimate,
          median_relative_error,
      });
    }
  }

  return results;
}
