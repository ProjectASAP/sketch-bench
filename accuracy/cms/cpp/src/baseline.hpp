#pragma once

#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <unordered_map>
#include <vector>

constexpr uint64_t kHeavyHitterMinTrueCount = 100;

struct BaselineData {
  std::vector<int64_t> values;
  std::unordered_map<int64_t, uint64_t> frequencies;
};

inline BaselineData load_pcap_baseline(const std::string& path);
inline BaselineData load_csv_baseline(const std::string& path);

inline std::vector<std::pair<int64_t, uint64_t>> heavy_hitters(const BaselineData& baseline) {
  std::vector<std::pair<int64_t, uint64_t>> result;
  result.reserve(baseline.frequencies.size());
  for (const auto& entry : baseline.frequencies) {
    if (entry.second >= kHeavyHitterMinTrueCount) {
      result.emplace_back(entry.first, entry.second);
    }
  }
  if (result.empty()) {
    throw std::runtime_error("Baseline contains no heavy hitters with true_count >= 100");
  }
  return result;
}

inline BaselineData load_baseline(const std::string& path) {
  if (path.size() >= 5 && path.substr(path.size() - 5) == ".pcap") {
    return load_pcap_baseline(path);
  }
  if (path.size() >= 4 && path.substr(path.size() - 4) == ".csv") {
    return load_csv_baseline(path);
  }

  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    throw std::runtime_error("Failed to open dataset: " + path);
  }

  const std::streamsize size = input.tellg();
  if (size <= 0) {
    throw std::runtime_error("Dataset is empty: " + path);
  }
  if ((size % static_cast<std::streamsize>(sizeof(int64_t))) != 0) {
    throw std::runtime_error("Dataset size is not divisible by 8 bytes: " + path);
  }

  input.seekg(0, std::ios::beg);
  BaselineData baseline;
  baseline.values.resize(static_cast<size_t>(size / sizeof(int64_t)));
  if (!input.read(reinterpret_cast<char*>(baseline.values.data()), size)) {
    throw std::runtime_error("Failed to read dataset: " + path);
  }

  for (const int64_t value : baseline.values) {
    baseline.frequencies[value] += 1;
  }
  if (baseline.frequencies.empty()) {
    throw std::runtime_error("Baseline contains zero distinct keys: " + path);
  }
  return baseline;
}

enum class PcapEndianness { kLittle, kBig };

inline uint32_t read_u32(const unsigned char* bytes, PcapEndianness endianness) {
  if (endianness == PcapEndianness::kLittle) {
    return static_cast<uint32_t>(bytes[0]) | (static_cast<uint32_t>(bytes[1]) << 8) |
           (static_cast<uint32_t>(bytes[2]) << 16) | (static_cast<uint32_t>(bytes[3]) << 24);
  }
  return (static_cast<uint32_t>(bytes[0]) << 24) | (static_cast<uint32_t>(bytes[1]) << 16) |
         (static_cast<uint32_t>(bytes[2]) << 8) | static_cast<uint32_t>(bytes[3]);
}

inline PcapEndianness detect_pcap_endianness(const unsigned char* magic) {
  if ((magic[0] == 0xd4 && magic[1] == 0xc3 && magic[2] == 0xb2 && magic[3] == 0xa1) ||
      (magic[0] == 0x4d && magic[1] == 0x3c && magic[2] == 0xb2 && magic[3] == 0xa1)) {
    return PcapEndianness::kLittle;
  }
  if ((magic[0] == 0xa1 && magic[1] == 0xb2 && magic[2] == 0xc3 && magic[3] == 0xd4) ||
      (magic[0] == 0xa1 && magic[1] == 0xb2 && magic[2] == 0x3c && magic[3] == 0x4d)) {
    return PcapEndianness::kBig;
  }
  throw std::runtime_error("Unsupported pcap magic");
}

inline bool extract_ipv4_source_raw(const unsigned char* packet, size_t size, int64_t* out_value) {
  if (size < 20) {
    return false;
  }
  if ((packet[0] >> 4) != 4) {
    return false;
  }

  const uint32_t src_ip = (static_cast<uint32_t>(packet[12]) << 24) |
                          (static_cast<uint32_t>(packet[13]) << 16) |
                          (static_cast<uint32_t>(packet[14]) << 8) |
                          static_cast<uint32_t>(packet[15]);
  *out_value = static_cast<int64_t>(src_ip);
  return true;
}

inline bool extract_ipv4_source(const std::vector<unsigned char>& packet, uint32_t linktype,
                                int64_t* out_value) {
  if (linktype == 1) {
    if (packet.size() < 34) {
      return false;
    }
    if (packet[12] != 0x08 || packet[13] != 0x00) {
      return false;
    }
    return extract_ipv4_source_raw(packet.data() + 14, packet.size() - 14, out_value);
  }
  if (linktype == 101) {
    return extract_ipv4_source_raw(packet.data(), packet.size(), out_value);
  }
  return false;
}

inline BaselineData load_pcap_baseline(const std::string& path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    throw std::runtime_error("Failed to open dataset: " + path);
  }

  unsigned char global_header[24];
  if (!input.read(reinterpret_cast<char*>(global_header), sizeof(global_header))) {
    throw std::runtime_error("Failed to read pcap global header: " + path);
  }
  const PcapEndianness endianness = detect_pcap_endianness(global_header);
  const uint32_t linktype = read_u32(global_header + 20, endianness);

  BaselineData baseline;
  while (true) {
    unsigned char packet_header[16];
    if (!input.read(reinterpret_cast<char*>(packet_header), sizeof(packet_header))) {
      break;
    }

    const uint32_t incl_len = read_u32(packet_header + 8, endianness);
    std::vector<unsigned char> packet(incl_len);
    if (!input.read(reinterpret_cast<char*>(packet.data()), static_cast<std::streamsize>(incl_len))) {
      throw std::runtime_error("Truncated packet data in pcap: " + path);
    }

    int64_t value = 0;
    if (extract_ipv4_source(packet, linktype, &value)) {
      baseline.values.push_back(value);
      baseline.frequencies[value] += 1;
    }
  }

  if (baseline.values.empty()) {
    throw std::runtime_error("PCAP contains zero IPv4 packets: " + path);
  }
  return baseline;
}

inline BaselineData load_csv_baseline(const std::string& path) {
  std::ifstream input(path);
  if (!input) {
    throw std::runtime_error("Failed to open CSV dataset: " + path);
  }

  BaselineData baseline;
  std::string line;

  // Skip header
  if (!std::getline(input, line)) {
    throw std::runtime_error("CSV file is empty: " + path);
  }

  while (std::getline(input, line)) {
    if (line.empty()) {
      continue;
    }
    const auto comma_pos = line.find(',');
    std::string field = (comma_pos != std::string::npos) ? line.substr(0, comma_pos) : line;
    // trim whitespace
    while (!field.empty() && field.front() == ' ') field.erase(field.begin());
    while (!field.empty() && field.back() == ' ') field.pop_back();
    if (field.empty()) {
      continue;
    }
    char* end_ptr = nullptr;
    const int64_t value = std::strtoll(field.c_str(), &end_ptr, 10);
    if (end_ptr == field.c_str()) {
      throw std::runtime_error("Failed to parse first CSV field as integer: " + field);
    }
    baseline.values.push_back(value);
    baseline.frequencies[value] += 1;
  }

  if (baseline.values.empty()) {
    throw std::runtime_error("CSV contains no data rows: " + path);
  }
  return baseline;
}
