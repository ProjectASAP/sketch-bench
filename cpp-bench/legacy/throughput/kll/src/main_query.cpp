#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

#include "kll_sketch.hpp"

namespace {

constexpr int kK = 200;
constexpr size_t kRuns = 10;
constexpr size_t kRepeatsPerRun = 10;
constexpr size_t kNumPercentiles = 101;  // p0..p100 inclusive
constexpr const char* kCsvHeader =
    "implementation,language,run,k,total_items,repeat,percentile,call_index,nanoseconds,estimate";

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
  size_t repeat;
  size_t percentile;
  size_t call_index;
  long long nanoseconds;
  double estimate;
};

struct Args {
  std::string data_path = "../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin";
  std::string output_path = "../output/kll_throughput_query_results_cpp.csv";
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
      std::cout << "Usage: kll_throughput_query [--data PATH] [--output PATH]\n";
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
  rows.reserve(kRuns * kRepeatsPerRun * kNumPercentiles);
  for (size_t run = 1; run <= kRuns; ++run) {
    datasketches::kll_sketch<int64_t> sketch(kK);
    for (const int64_t value : data) {
      sketch.update(value);
    }
    black_box(&sketch);
    size_t call_index = 0;
    for (size_t repeat = 1; repeat <= kRepeatsPerRun; ++repeat) {
      for (size_t p = 0; p < kNumPercentiles; ++p) {
        ++call_index;
        const double rank = static_cast<double>(p) / 100.0;
        const auto start = std::chrono::steady_clock::now();
        const int64_t q = sketch.get_quantile(rank);
        black_box(&q);
        const auto end = std::chrono::steady_clock::now();
        const auto elapsed =
            std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();
        rows.push_back(Row{run, data.size(), repeat, p, call_index, elapsed,
                           static_cast<double>(q)});
      }
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
    out << "cpp_datasketches_kll,cpp," << row.run << ',' << kK << ','
        << row.total_items << ',' << row.repeat << ',' << row.percentile << ','
        << row.call_index << ',' << row.nanoseconds << ',' << row.estimate << '\n';
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
