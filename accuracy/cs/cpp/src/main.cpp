#include <exception>
#include <iostream>
#include <string>

#include "baseline.hpp"
#include "datasketches_runner.hpp"
#include "output.hpp"

namespace {

struct Args {
  std::string data_path = "../data/benchmark_data_10m_int64_zipf_s11_k100000.bin";
  std::string mode;
};

Args parse_args(int argc, char** argv) {
  Args args;
  for (int i = 1; i < argc; ++i) {
    const std::string_view arg(argv[i]);
    if (arg == "--data") {
      if (i + 1 >= argc) {
        throw std::runtime_error("--data requires a path");
      }
      args.data_path = argv[++i];
    } else if (arg == "--mode") {
      if (i + 1 >= argc) {
        throw std::runtime_error("--mode requires a value");
      }
      args.mode = argv[++i];
    } else if (arg == "--help" || arg == "-h") {
      std::cout << "Usage: cms_accuracy [--data PATH] --mode summary|key-errors\n";
      std::exit(0);
    } else {
      throw std::runtime_error("Unknown argument: " + std::string(arg));
    }
  }
  if (args.mode != "summary" && args.mode != "key-errors") {
    throw std::runtime_error("--mode must be one of: summary, key-errors");
  }
  return args;
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const Args args = parse_args(argc, argv);
    const BaselineData baseline = load_baseline(args.data_path);
    if (args.mode == "summary") {
      const auto rows = run_datasketches_summary(baseline);
      for (const auto& row : rows) {
        write_csv_row(std::cout, row);
      }
    } else {
      const auto rows = run_datasketches_key_errors(baseline);
      for (const auto& row : rows) {
        write_key_error_csv_row(std::cout, row);
      }
    }
    return 0;
  } catch (const std::exception& ex) {
    std::cerr << ex.what() << '\n';
    return 1;
  }
}
