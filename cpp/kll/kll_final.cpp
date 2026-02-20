// Match external benchmark's exact include order
#include <algorithm>
#include "data.hpp"
#include "kll/kll_cached_level_capacities.hpp"
#include "kll/kll_final.hpp"

#include "benchmark/benchmark.h"

static void BM_KllFinal_Insert(benchmark::State& state) {
  const auto& data = GetData<int64_t>();
  for (auto _ : state) {
    state.PauseTiming();
    {
      final::KarninLangLiberty<int64_t> sketch;
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

BENCHMARK(BM_KllFinal_Insert)->Iterations(1);
BENCHMARK_MAIN();
