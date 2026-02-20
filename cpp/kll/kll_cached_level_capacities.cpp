#include <algorithm>
#include "data.hpp"
#include "kll/kll_cached_level_capacities.hpp"

#include "benchmark/benchmark.h"

static void BM_KllCachedLevelCapacities_Insert(benchmark::State& state) {
  const auto& data = GetData<int64_t>();
  constexpr size_t k = 200;
  for (auto _ : state) {
    state.PauseTiming();
    {
      cached_level_capacities::KarninLangLiberty<int64_t> sketch(k);
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

BENCHMARK(BM_KllCachedLevelCapacities_Insert)->Iterations(1);
BENCHMARK_MAIN();
