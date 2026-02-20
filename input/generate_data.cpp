// Generate benchmark data file: 1M int64 values with seed 42
#include <cstdint>
#include <fstream>
#include <iostream>
#include <random>
#include <vector>

int main() {
  constexpr size_t num_values = 1'000'000;
  constexpr uint64_t seed = 42;
  constexpr const char* filename = "benchmark_data_1m_int64.bin";

  std::mt19937_64 rng(seed);
  std::uniform_int_distribution<int64_t> dist(std::numeric_limits<int64_t>::min(),
                                              std::numeric_limits<int64_t>::max());

  std::vector<int64_t> data;
  data.reserve(num_values);

  for (size_t i = 0; i < num_values; ++i) {
    data.push_back(dist(rng));
  }

  std::ofstream out(filename, std::ios::binary);
  if (!out) {
    std::cerr << "Failed to open file: " << filename << std::endl;
    return 1;
  }

  out.write(reinterpret_cast<const char*>(data.data()),
            data.size() * sizeof(int64_t));
  out.close();

  std::cout << "Generated " << num_values << " random int64 values" << std::endl;
  std::cout << "File: " << filename << " (" << (num_values * sizeof(int64_t)) / (1024 * 1024) << " MB)" << std::endl;

  return 0;
}
