#pragma once

#include <cmath>
#include <cstdint>
#include <iomanip>
#include <ostream>
#include <string_view>

struct AccuracyRow {
  std::string_view implementation;
  std::string_view language;
  uint64_t seed;
  size_t rows;
  size_t cols;
  size_t total_items;
  size_t distinct_items;
  double avg_relative_error;
  double max_relative_error;
  double mean_absolute_error;
};

inline void write_csv_row(std::ostream& out, const AccuracyRow& row) {
  out << row.implementation << ',' << row.language << ',' << row.seed << ',' << row.rows << ','
      << row.cols << ',' << row.total_items << ',' << row.distinct_items << ','
      << std::fixed << std::setprecision(12) << row.avg_relative_error << ','
      << row.max_relative_error << ',' << row.mean_absolute_error << '\n';
}

struct KeyMedianErrorRow {
  std::string_view implementation;
  std::string_view language;
  size_t rows;
  size_t cols;
  int64_t key;
  uint64_t true_count;
  uint64_t median_estimate;
  double median_relative_error;
};

inline void write_key_error_csv_row(std::ostream& out, const KeyMedianErrorRow& row) {
  out << row.implementation << ',' << row.language << ',' << row.rows << ',' << row.cols << ','
      << row.key << ',' << row.true_count << ',' << row.median_estimate << ','
      << std::fixed << std::setprecision(12) << row.median_relative_error << '\n';
}

struct KeySeedErrorRow {
  std::string_view implementation;
  std::string_view language;
  uint64_t seed;
  size_t rows;
  size_t cols;
  int64_t key;
  uint64_t true_count;
  uint64_t estimate;
  double relative_error;
};

inline void write_key_seed_error_csv_row(std::ostream& out, const KeySeedErrorRow& row) {
  out << row.implementation << ',' << row.language << ',' << row.seed << ',' << row.rows << ','
      << row.cols << ',' << row.key << ',' << row.true_count << ',' << row.estimate << ','
      << std::fixed << std::setprecision(12) << row.relative_error << '\n';
}
