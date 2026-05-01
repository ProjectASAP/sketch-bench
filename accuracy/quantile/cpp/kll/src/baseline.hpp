#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <stdexcept>
#include <string>
#include <vector>

struct BaselineData {
  std::vector<int64_t> values;
  std::vector<int64_t> sorted_values;

  /// Ground truth quantile at percentile p (0..100).
  double ground_truth_quantile(int p) const {
    const size_t n = sorted_values.size();
    const size_t index =
        static_cast<size_t>(std::round(static_cast<double>(p) / 100.0 *
                                       static_cast<double>(n - 1)));
    return static_cast<double>(sorted_values[index]);
  }
};

inline BaselineData load_baseline(const std::string& path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    throw std::runtime_error("failed to open dataset: " + path);
  }

  input.seekg(0, std::ios::end);
  const std::streamoff size = input.tellg();
  input.seekg(0, std::ios::beg);

  if (size <= 0) {
    throw std::runtime_error("dataset is empty: " + path);
  }
  if (size % static_cast<std::streamoff>(sizeof(int64_t)) != 0) {
    throw std::runtime_error("dataset size is not divisible by 8 bytes: " + path);
  }

  std::vector<int64_t> values(static_cast<size_t>(size / sizeof(int64_t)));
  input.read(reinterpret_cast<char*>(values.data()), size);
  if (!input) {
    throw std::runtime_error("failed to read dataset: " + path);
  }

  std::vector<int64_t> sorted_values(values);
  std::sort(sorted_values.begin(), sorted_values.end());

  return BaselineData{std::move(values), std::move(sorted_values)};
}
