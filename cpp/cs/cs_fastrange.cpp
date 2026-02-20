#include <algorithm>
#include "data.hpp"
#include "cs/cs_fastrange.hpp"

#include "benchmark/benchmark.h"

static void BM_CsFastrange_Insert(benchmark::State& state) {
  const auto& data = GetData<int64_t>();
  for (auto _ : state) {
    state.PauseTiming();
    {
      fastrange::CountSketch<int64_t> sketch;
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

BENCHMARK(BM_CsFastrange_Insert)->Iterations(1);
BENCHMARK_MAIN();
