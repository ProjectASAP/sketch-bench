#pragma once

#include <cstdint>
#include <iomanip>
#include <ostream>
#include <string_view>

struct AccuracyRow {
  std::string_view implementation;
  std::string_view language;
  uint64_t seed;
  uint8_t lg_k;
  size_t registers;
  size_t total_items;
  size_t true_distinct;
  double estimate;
  double relative_error;
};

inline void write_csv_row(std::ostream& out, const AccuracyRow& row) {
  out << row.implementation << ',' << row.language << ',' << row.seed << ','
      << static_cast<unsigned>(row.lg_k) << ',' << row.registers << ',' << row.total_items << ','
      << row.true_distinct << ',' << std::fixed << std::setprecision(12) << row.estimate << ','
      << row.relative_error << '\n';
}
