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
