#include <algorithm>
#include "data.hpp"
#include "cs/cs_final_no_murmur_unroll.hpp"

#include "benchmark/benchmark.h"

static void BM_CsFinalNoMurmurUnroll_Insert(benchmark::State& state) {
  const auto& data = GetData<int64_t>();
  for (auto _ : state) {
    state.PauseTiming();
    {
      final_no_murmur_unroll::CountSketch<int64_t> sketch;
      state.ResumeTiming();
      for (const auto& value : data) {
        sketch.Insert(value);
      }
      benchmark::DoNotOptimize(sketch);
      benchmark::ClobberMemory();
      state.PauseTiming();
    }
    state.ResumeTiming();
  }

  const int64_t num_items =
      static_cast<int64_t>(state.iterations()) * static_cast<int64_t>(data.size());
  state.SetItemsProcessed(num_items);
}

BENCHMARK(BM_CsFinalNoMurmurUnroll_Insert)->Iterations(1);
BENCHMARK_MAIN();
