#include <algorithm>
#include "data.hpp"

#include "benchmark/benchmark.h"

namespace insert_opt_datasketches_namespace {}
#define datasketches insert_opt_datasketches_namespace
#include "kll/kll_datasketches.hpp"
#undef datasketches
namespace insert_opt_datasketches = insert_opt_datasketches_namespace;

static void BM_KllDatasketches_Insert(benchmark::State& state) {
  const auto& data = GetData<int64_t>();
  constexpr size_t k = 200;
  for (auto _ : state) {
    state.PauseTiming();
    {
      insert_opt_datasketches::KarninLangLiberty<int64_t> sketch(k);
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

BENCHMARK(BM_KllDatasketches_Insert)->Iterations(1);
BENCHMARK_MAIN();
