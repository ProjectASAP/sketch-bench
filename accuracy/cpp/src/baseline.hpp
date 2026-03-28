#pragma once

#include <cstdint>
#include <fstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <unordered_map>
#include <vector>

constexpr uint64_t kHeavyHitterMinTrueCount = 100;

struct BaselineData {
  std::vector<int64_t> values;
  std::unordered_map<int64_t, uint64_t> frequencies;
};

inline std::vector<std::pair<int64_t, uint64_t>> heavy_hitters(const BaselineData& baseline) {
  std::vector<std::pair<int64_t, uint64_t>> result;
  result.reserve(baseline.frequencies.size());
  for (const auto& entry : baseline.frequencies) {
    if (entry.second >= kHeavyHitterMinTrueCount) {
      result.emplace_back(entry.first, entry.second);
    }
  }
  if (result.empty()) {
    throw std::runtime_error("Baseline contains no heavy hitters with true_count >= 100");
  }
  return result;
}

inline BaselineData load_baseline(const std::string& path) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    throw std::runtime_error("Failed to open dataset: " + path);
  }

  const std::streamsize size = input.tellg();
  if (size <= 0) {
    throw std::runtime_error("Dataset is empty: " + path);
  }
  if ((size % static_cast<std::streamsize>(sizeof(int64_t))) != 0) {
    throw std::runtime_error("Dataset size is not divisible by 8 bytes: " + path);
  }

  input.seekg(0, std::ios::beg);
  BaselineData baseline;
  baseline.values.resize(static_cast<size_t>(size / sizeof(int64_t)));
  if (!input.read(reinterpret_cast<char*>(baseline.values.data()), size)) {
    throw std::runtime_error("Failed to read dataset: " + path);
  }

  for (const int64_t value : baseline.values) {
    baseline.frequencies[value] += 1;
  }
  if (baseline.frequencies.empty()) {
    throw std::runtime_error("Baseline contains zero distinct keys: " + path);
  }
  return baseline;
}
