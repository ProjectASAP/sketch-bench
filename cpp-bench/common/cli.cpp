#include "cli.hpp"

#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <string>

namespace cpp_bench {

namespace {

[[noreturn]] void usage(const char* argv0, const char* msg = nullptr) {
    if (msg) {
        std::cerr << "cpp-bench: " << msg << "\n\n";
    }
    std::cerr <<
        "Usage: " << argv0 << " --workload-file PATH [options]\n"
        "\n"
        "Required:\n"
        "  --workload-file PATH    little-endian int64 .bin file\n"
        "\n"
        "Options:\n"
        "  --runs N                number of measured runs (default 10)\n"
        "  --warmup-items N        items inserted before measurement (default 100000)\n"
        "  --measure-items N       items inserted during measurement (default 1000000)\n"
        "  --latency-stride N      sample latency every Nth item; 0 disables (default 1000)\n"
        "  --seed S                RNG seed for any synthetic gen (default 0x5e7c4011)\n"
        "  --k K                   sketch construction param (KLL k, CS cols, ...)\n"
        "  --with-accuracy         compute accuracy against an exact baseline\n"
        "  --report PATH           write JSONL to PATH (default '-' = stdout)\n"
        "  --legacy-csv PATH       additionally write per-run rows in the\n"
        "                          legacy long-format CSV consumed by\n"
        "                          visualization/plots/*.py\n";
    std::exit(2);
}

std::size_t parse_size(const char* argv0, const std::string& v) {
    try {
        return static_cast<std::size_t>(std::stoull(v));
    } catch (...) {
        usage(argv0, ("not a non-negative integer: " + v).c_str());
    }
}

std::uint64_t parse_u64(const char* argv0, const std::string& v) {
    try {
        // stoull handles 0x prefix when base = 0.
        return static_cast<std::uint64_t>(std::stoull(v, nullptr, 0));
    } catch (...) {
        usage(argv0, ("not a u64: " + v).c_str());
    }
}

}  // namespace

CliArgs parse_cli(int argc, char** argv) {
    CliArgs out;
    const char* argv0 = argc > 0 ? argv[0] : "cpp-bench-binary";

    auto need = [&](int& i) -> std::string {
        if (i + 1 >= argc) usage(argv0, "missing value");
        return std::string(argv[++i]);
    };

    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        if      (a == "--workload-file") out.workload_file = need(i);
        else if (a == "--runs")          out.runs = parse_size(argv0, need(i));
        else if (a == "--warmup-items")  out.warmup_items = parse_size(argv0, need(i));
        else if (a == "--measure-items") out.measure_items = parse_size(argv0, need(i));
        else if (a == "--latency-stride")out.latency_sample_stride = parse_size(argv0, need(i));
        else if (a == "--seed")          out.rng_seed = parse_u64(argv0, need(i));
        else if (a == "--k")             out.k = parse_size(argv0, need(i));
        else if (a == "--with-accuracy") out.with_accuracy = true;
        else if (a == "--report")        out.report_path = need(i);
        else if (a == "--legacy-csv")    out.legacy_csv_path = need(i);
        else if (a == "-h" || a == "--help") usage(argv0);
        else                             usage(argv0, ("unknown arg: " + a).c_str());
    }
    if (out.workload_file.empty()) {
        usage(argv0, "--workload-file is required");
    }
    return out;
}

}  // namespace cpp_bench
