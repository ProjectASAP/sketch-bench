// Generate Zipf input as a raw int64 stream.
#include <algorithm>
#include <charconv>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <numeric>
#include <string>
#include <random>
#include <vector>

namespace {

bool ParseSizeT(const char* text, std::size_t& value) {
  const char* end = text + std::char_traits<char>::length(text);
  auto result = std::from_chars(text, end, value);
  return result.ec == std::errc() && result.ptr == end;
}

bool ParseUint64(const char* text, std::uint64_t& value) {
  const char* end = text + std::char_traits<char>::length(text);
  auto result = std::from_chars(text, end, value);
  return result.ec == std::errc() && result.ptr == end;
}

bool ParseDouble(const char* text, double& value) {
  try {
    std::size_t parsed = 0;
    value = std::stod(text, &parsed);
    return parsed == std::char_traits<char>::length(text);
  } catch (...) {
    return false;
  }
}

std::string FormatExponentTag(double exponent) {
  std::string tag = std::to_string(exponent);
  tag.erase(std::remove(tag.begin(), tag.end(), '.'), tag.end());
  while (!tag.empty() && tag.back() == '0') {
    tag.pop_back();
  }
  return tag.empty() ? "0" : tag;
}

}  // namespace

int main(int argc, char** argv) {
  std::size_t num_values = 10'000'000;
  std::size_t support_size = 100'000;
  double zipf_exponent = 1.1;
  std::uint64_t seed = 42;

  if (argc > 1 && !ParseSizeT(argv[1], num_values)) {
    std::cerr << "Invalid num_values: " << argv[1] << '\n';
    return 1;
  }
  if (argc > 2 && !ParseDouble(argv[2], zipf_exponent)) {
    std::cerr << "Invalid zipf_exponent: " << argv[2] << '\n';
    return 1;
  }
  if (argc > 3 && !ParseSizeT(argv[3], support_size)) {
    std::cerr << "Invalid support_size: " << argv[3] << '\n';
    return 1;
  }
  if (argc > 4 && !ParseUint64(argv[4], seed)) {
    std::cerr << "Invalid seed: " << argv[4] << '\n';
    return 1;
  }
  if (argc > 5) {
    std::cerr << "Usage: " << argv[0]
              << " [num_values] [zipf_exponent] [support_size] [seed]\n";
    return 1;
  }

  if (num_values == 0 || support_size == 0 || zipf_exponent <= 0.0) {
    std::cerr << "Parameters must satisfy: num_values > 0, support_size > 0, "
                 "zipf_exponent > 0\n";
    return 1;
  }

  const std::string filename = "benchmark_data_" + std::to_string(num_values / 1'000'000) +
                               "m_int64_zipf_s" + FormatExponentTag(zipf_exponent) + "_k" +
                               std::to_string(support_size) + ".bin";

  std::vector<double> cdf;
  cdf.reserve(support_size);

  double normalizer = 0.0;
  for (std::size_t rank = 1; rank <= support_size; ++rank) {
    normalizer += 1.0 / std::pow(static_cast<double>(rank), zipf_exponent);
  }

  double cumulative = 0.0;
  for (std::size_t rank = 1; rank <= support_size; ++rank) {
    cumulative += (1.0 / std::pow(static_cast<double>(rank), zipf_exponent)) / normalizer;
    cdf.push_back(cumulative);
  }
  cdf.back() = 1.0;

  std::mt19937_64 rng(seed);
  std::uniform_real_distribution<double> dist(
      0.0, std::nextafter(1.0, std::numeric_limits<double>::lowest()));

  std::vector<std::int64_t> data;
  data.reserve(num_values);
  for (std::size_t i = 0; i < num_values; ++i) {
    const double sample = dist(rng);
    const auto it = std::lower_bound(cdf.begin(), cdf.end(), sample);
    const std::size_t rank = static_cast<std::size_t>(std::distance(cdf.begin(), it)) + 1;
    data.push_back(static_cast<std::int64_t>(rank));
  }

  std::ofstream out(filename, std::ios::binary);
  if (!out) {
    std::cerr << "Failed to open file: " << filename << '\n';
    return 1;
  }

  out.write(reinterpret_cast<const char*>(data.data()),
            static_cast<std::streamsize>(data.size() * sizeof(std::int64_t)));
  if (!out) {
    std::cerr << "Failed to write file: " << filename << '\n';
    return 1;
  }

  std::cout << "Generated " << num_values << " Zipf-distributed int64 values\n";
  std::cout << "Zipf exponent: " << zipf_exponent << ", support size: " << support_size
            << ", seed: " << seed << '\n';
  std::cout << "File: " << filename << " ("
            << (num_values * sizeof(std::int64_t)) / (1024 * 1024) << " MB)\n";

  return 0;
}
