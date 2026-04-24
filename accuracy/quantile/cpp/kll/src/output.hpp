#pragma once

#include <cstdint>
#include <iomanip>
#include <ostream>
#include <string_view>

struct AccuracyRow {
  std::string_view implementation;
  std::string_view language;
  int k;
  int percentile;
  size_t total_items;
  double true_quantile;
  double estimate;
  double relative_error;
};

inline void write_csv_row(std::ostream& out, const AccuracyRow& row) {
  out << row.implementation << ',' << row.language << ',' << row.k << ','
      << row.percentile << ',' << row.total_items << ',' << std::fixed
      << std::setprecision(12) << row.true_quantile << ',' << row.estimate
      << ',' << row.relative_error << '\n';
}
