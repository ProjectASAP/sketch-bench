#include <algorithm>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <numeric>
#include <random>
#include <vector>

int main() {
  constexpr std::size_t kNumValues = 10'000'000;
  constexpr std::size_t kSupportSize = 100'000;
  constexpr double kZipfExponent = 1.1;
  constexpr std::uint64_t kSeed = 42;
  constexpr const char* kFilename = "benchmark_data_10m_int64_zipf_s11_k100000.bin";

  std::vector<double> cdf;
  cdf.reserve(kSupportSize);

  double normalizer = 0.0;
  for (std::size_t rank = 1; rank <= kSupportSize; ++rank) {
    normalizer += 1.0 / std::pow(static_cast<double>(rank), kZipfExponent);
  }

  double cumulative = 0.0;
  for (std::size_t rank = 1; rank <= kSupportSize; ++rank) {
    cumulative += (1.0 / std::pow(static_cast<double>(rank), kZipfExponent)) / normalizer;
    cdf.push_back(cumulative);
  }
  cdf.back() = 1.0;

  std::mt19937_64 rng(kSeed);
  std::uniform_real_distribution<double> dist(
      0.0, std::nextafter(1.0, std::numeric_limits<double>::lowest()));

  std::vector<std::int64_t> data;
  data.reserve(kNumValues);
  for (std::size_t i = 0; i < kNumValues; ++i) {
    const double sample = dist(rng);
    const auto it = std::lower_bound(cdf.begin(), cdf.end(), sample);
    const std::size_t rank = static_cast<std::size_t>(std::distance(cdf.begin(), it)) + 1;
    data.push_back(static_cast<std::int64_t>(rank));
  }

  std::ofstream out(kFilename, std::ios::binary);
  if (!out) {
    std::cerr << "Failed to open file: " << kFilename << '\n';
    return 1;
  }

  out.write(reinterpret_cast<const char*>(data.data()),
            static_cast<std::streamsize>(data.size() * sizeof(std::int64_t)));
  if (!out) {
    std::cerr << "Failed to write file: " << kFilename << '\n';
    return 1;
  }

  std::cout << "Generated " << kNumValues << " Zipf-distributed int64 values\n";
  std::cout << "Zipf exponent: " << kZipfExponent << ", support size: " << kSupportSize
            << ", seed: " << kSeed << '\n';
  std::cout << "File: " << kFilename << " ("
            << (kNumValues * sizeof(std::int64_t)) / (1024 * 1024) << " MB)\n";

  return 0;
}
