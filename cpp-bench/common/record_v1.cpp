#include "record_v1.hpp"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <ctime>
#include <sstream>
#include <string>

namespace cpp_bench {

namespace {

// JSON string escaper. Covers the cases we actually produce
// (file paths, sketch / impl names) plus the JSON-required
// minimum: " \ control chars.
void write_string(std::ostringstream& o, const std::string& s) {
    o << '"';
    for (char c : s) {
        switch (c) {
            case '"':  o << "\\\""; break;
            case '\\': o << "\\\\"; break;
            case '\b': o << "\\b";  break;
            case '\f': o << "\\f";  break;
            case '\n': o << "\\n";  break;
            case '\r': o << "\\r";  break;
            case '\t': o << "\\t";  break;
            default:
                if (static_cast<unsigned char>(c) < 0x20) {
                    char buf[8];
                    std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                    o << buf;
                } else {
                    o << c;
                }
        }
    }
    o << '"';
}

// Write an f64 that survives a JSON round-trip back to itself.
// Uses %.17g which is enough for IEEE-754 doubles.
void write_double(std::ostringstream& o, double v) {
    if (std::isnan(v) || std::isinf(v)) {
        o << "null";
        return;
    }
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.17g", v);
    o << buf;
}

void write_run_stats(std::ostringstream& o, const RunStats& s) {
    // No "ci95": these runs share one process, so no honest confidence
    // interval can be computed from them. See RunStats::ci95 on the Rust
    // side — the field is optional and absent means "not computable here".
    o << "{\"mean\":";    write_double(o, s.mean);
    o << ",\"stddev\":";  write_double(o, s.stddev);
    o << ",\"n\":" << s.n << '}';
}

void write_latency(std::ostringstream& o, const LatencySummary& l) {
    o << "{\"p50\":" << l.p50
      << ",\"p95\":" << l.p95
      << ",\"p99\":" << l.p99
      << ",\"p999\":" << l.p999
      << ",\"max\":" << l.max
      << ",\"count\":" << l.count << '}';
}

void write_workload(std::ostringstream& o, const WorkloadDesc& w) {
    o << "{\"shape\":"; write_string(o, w.shape);
    o << ",\"size\":" << w.size;
    if (w.cardinality)  o << ",\"cardinality\":" << *w.cardinality;
    if (w.zipf_s)       { o << ",\"zipf_s\":"; write_double(o, *w.zipf_s); }
    if (w.source_path)  { o << ",\"source_path\":"; write_string(o, *w.source_path); }
    if (w.seed)         o << ",\"seed\":" << *w.seed;
    o << '}';
}

std::string utc_rfc3339_now() {
    using namespace std::chrono;
    const auto now = system_clock::now();
    const auto secs = time_point_cast<seconds>(now);
    const auto micros = duration_cast<microseconds>(now - secs).count();
    const std::time_t t = system_clock::to_time_t(secs);
    std::tm tm{};
    gmtime_r(&t, &tm);
    char buf[40];
    // 2026-05-13T07:14:22.123456Z
    std::snprintf(buf, sizeof(buf),
                  "%04d-%02d-%02dT%02d:%02d:%02d.%06ldZ",
                  tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
                  tm.tm_hour, tm.tm_min, tm.tm_sec,
                  static_cast<long>(micros));
    return buf;
}

}  // namespace

std::string Record::to_jsonl() const {
    std::ostringstream o;
    o << "{";
    o << "\"schema_version\":" << schema_version;
    o << ",\"sketch\":";    write_string(o, sketch);
    o << ",\"impl\":";      write_string(o, impl_name);
    o << ",\"language\":";  write_string(o, language);
    o << ",\"workload\":";  write_workload(o, workload);
    o << ",\"mode\":";      write_string(o, mode);
    o << ",\"runs\":" << runs;

    const bool has_bench =
        bench.throughput_items_per_sec.has_value() ||
        bench.latency_ns.has_value() ||
        bench.accuracy_json.has_value();
    if (has_bench) {
        o << ",\"bench\":{";
        bool first = true;
        auto sep = [&]() {
            if (!first) o << ',';
            first = false;
        };
        if (bench.throughput_items_per_sec) {
            sep();
            o << "\"throughput_items_per_sec\":";
            write_run_stats(o, *bench.throughput_items_per_sec);
        }
        if (bench.latency_ns) {
            sep();
            o << "\"latency_ns\":";
            write_latency(o, *bench.latency_ns);
        }
        if (bench.accuracy_json) {
            sep();
            o << "\"accuracy\":" << *bench.accuracy_json;
        }
        o << '}';
    }

    o << ",\"source\":";    write_string(o, source);
    o << ",\"timestamp\":"; write_string(o, timestamp_rfc3339);
    o << '}';
    return o.str();
}

void Record::emit(std::ostream& os) {
    if (timestamp_rfc3339.empty()) {
        timestamp_rfc3339 = utc_rfc3339_now();
    }
    os << to_jsonl() << '\n';
    os.flush();
}

}  // namespace cpp_bench
