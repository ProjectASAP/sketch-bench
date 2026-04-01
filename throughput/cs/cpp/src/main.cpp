#include <chrono>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

#include "cs/cs_final.hpp"

namespace {

constexpr size_t kRows = 5;
constexpr size_t kCols = 2048;
constexpr size_t kRuns = 10;
constexpr const char* kCsvHeader =
    "implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec";

inline void black_box(const void* value) {
#if defined(__GNUC__) || defined(__clang__)
  asm volatile("" : : "g"(value) : "memory");
#else
  (void)value;
#endif
}

struct Row {
  size_t run_index;
  size_t total_items;
  long long total_nanoseconds;
  double throughput_items_per_sec;
};

struct Args {
  std::string data_path = "../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin";
  std::string output_path = "../output/cs_throughput_results_cpp.csv";
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
      std::cout << "Usage: cs_throughput [--data PATH] [--output PATH]\n";
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
  input.seekg(0, std::ios::beg);
  std::vector<int64_t> data(static_cast<size_t>(size / sizeof(int64_t)));
  if (!input.read(reinterpret_cast<char*>(data.data()), size)) {
    throw std::runtime_error("Failed to read dataset: " + path);
  }
  black_box(data.data());
  return data;
}

std::vector<Row> run_insert_optimized(const std::vector<int64_t>& data) {
  std::vector<Row> rows;
  rows.reserve(kRuns);
  for (size_t run_index = 1; run_index <= kRuns; ++run_index) {
    final::CountSketch<int64_t> sketch;
    const auto start = std::chrono::steady_clock::now();
    for (const int64_t value : data) {
      sketch.Insert(value);
    }
    black_box(&sketch);
    const auto end = std::chrono::steady_clock::now();
    const auto elapsed =
        std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();
    rows.push_back(Row{
        run_index,
        data.size(),
        elapsed,
        static_cast<double>(data.size()) * 1'000'000'000.0 / static_cast<double>(elapsed),
    });
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
    out << "cpp_insert_optimized_cs,cpp," << row.run_index << ',' << kRows << ',' << kCols << ','
        << row.total_items << ',' << row.total_nanoseconds << ',' << row.throughput_items_per_sec
        << '\n';
  }
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const Args args = parse_args(argc, argv);
    const auto data = load_dataset(args.data_path);
    write_rows(args.output_path, run_insert_optimized(data));
    return 0;
  } catch (const std::exception& ex) {
    std::cerr << ex.what() << '\n';
    return 1;
  }
}
