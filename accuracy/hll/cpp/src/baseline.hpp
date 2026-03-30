#pragma once

#include <cstdint>
#include <fstream>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <vector>

struct BaselineData {
  std::vector<int64_t> values;
  size_t distinct_items;
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

  std::unordered_set<int64_t> distinct(values.begin(), values.end());
  return BaselineData{std::move(values), distinct.size()};
}
