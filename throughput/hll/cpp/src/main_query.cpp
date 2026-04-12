#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

#include "hll.hpp"

namespace {

constexpr uint8_t kLgK = 12;
constexpr size_t kRegisters = size_t{1} << kLgK;
constexpr size_t kRuns = 10;
constexpr size_t kCallsPerRun = 10;
constexpr const char* kCsvHeader =
    "implementation,language,run,lg_k,registers,total_items,call_index,nanoseconds,estimate";

inline void black_box(const void* value) {
#if defined(__GNUC__) || defined(__clang__)
  asm volatile("" : : "g"(value) : "memory");
#else
  (void)value;
#endif
}

struct Row {
  size_t run;
  size_t total_items;
  size_t call_index;
  long long nanoseconds;
  double estimate;
};

struct Args {
  std::string data_path = "../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin";
  std::string output_path = "../output/hll_throughput_query_results_cpp.csv";
};

Args parse_args(int argc, char** argv) {
  Args args;
  for (int i = 1; i < argc; ++i) {
    const std::string arg(argv[i]);
    if (arg == "--data") {
      if (i + 1 >= argc) throw std::runtime_error("--data requires a path");
      args.data_path = argv[++i];
    } else if (arg == "--output") {
      if (i + 1 >= argc) throw std::runtime_error("--output requires a path");
      args.output_path = argv[++i];
    } else if (arg == "--help" || arg == "-h") {
      std::cout << "Usage: hll_throughput_query [--data PATH] [--output PATH]\n";
      std::exit(0);
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }
  return args;
}

std::vector<int64_t> load_dataset(const std::string& path) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) throw std::runtime_error("Failed to open dataset: " + path);
  const std::streamsize size = input.tellg();
  if (size <= 0) throw std::runtime_error("Dataset is empty: " + path);
  if ((size % static_cast<std::streamsize>(sizeof(int64_t))) != 0) {
    throw std::runtime_error("Dataset size is not divisible by 8 bytes: " + path);
  }
  input.seekg(0, std::ios::beg);
  std::vector<int64_t> data(static_cast<size_t>(size / sizeof(int64_t)));
  if (!input.read(reinterpret_cast<char*>(data.data()), size)) {
    throw std::runtime_error("Failed to read dataset: " + path);
  }
  black_box(data.data());
  return data;
}

std::vector<Row> run_datasketches(const std::vector<int64_t>& data) {
  std::vector<Row> rows;
  rows.reserve(kRuns * kCallsPerRun);
  for (size_t run = 1; run <= kRuns; ++run) {
    datasketches::hll_sketch sketch(kLgK);
    for (const int64_t value : data) {
      sketch.update(value);
    }
    for (size_t c = 1; c <= kCallsPerRun; ++c) {
      const auto start = std::chrono::steady_clock::now();
      const double estimate = sketch.get_estimate();
      black_box(&estimate);
      const auto end = std::chrono::steady_clock::now();
      const auto elapsed =
          std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();
      rows.push_back(Row{run, data.size(), c, elapsed, estimate});
    }
  }
  return rows;
}

void write_rows(const std::string& path, const std::vector<Row>& rows) {
  const auto slash = path.find_last_of("/\\");
  if (slash != std::string::npos) {
    const std::string directory = path.substr(0, slash);
    if (!directory.empty()) {
      std::string command = "mkdir -p \"" + directory + "\"";
      if (std::system(command.c_str()) != 0) {
        throw std::runtime_error("Failed to create output directory: " + directory);
      }
    }
  }
  std::ofstream out(path);
  if (!out) throw std::runtime_error("Failed to open output: " + path);
  out << kCsvHeader << '\n';
  for (const auto& row : rows) {
    out << "cpp_datasketches_hll,cpp," << row.run << ',' << static_cast<int>(kLgK) << ','
        << kRegisters << ',' << row.total_items << ',' << row.call_index << ','
        << row.nanoseconds << ',' << row.estimate << '\n';
  }
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const Args args = parse_args(argc, argv);
    const auto data = load_dataset(args.data_path);
    write_rows(args.output_path, run_datasketches(data));
    return 0;
  } catch (const std::exception& ex) {
    std::cerr << ex.what() << '\n';
    return 1;
  }
}
