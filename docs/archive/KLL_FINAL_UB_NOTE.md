# KLL Final UB Note

This repo links against the external Insert-Optimized-Data-Sketches code. The
`final::KarninLangLiberty` implementation in that repo has undefined behavior
(UB) due to member initialization order.

## Root cause
- In `kll_final.hpp`, `max_capacity_` is initialized before
  `level_capacities`, but `compute_total_capacity()` reads
  `level_capacities`.
- C++ initializes members in declaration order, not the order in the
  constructor initializer list.
- This means `level_capacities` is read while uninitialized, which is UB.

## Why it sometimes crashes
- When uninitialized data happens to be zero/garbage, `max_capacity_` can be
  computed as 0.
- That causes `items_` to be created with a huge offset
  (`items_storage_ + (max_capacity_ - k)`), leading to a crash.

## Minimal repro idea (forces the crash)
Zeroing the object storage before construction makes the uninitialized
`level_capacities` read as zeros, which deterministically drives
`max_capacity_` to 0 and triggers the bad span offset.

```cpp
alignas(final::KarninLangLiberty<int64_t>) unsigned char storage[sizeof(final::KarninLangLiberty<int64_t>)];
std::memset(storage, 0, sizeof(storage));
auto* sketch = new (storage) final::KarninLangLiberty<int64_t>(200);
// Use sketch...
sketch->~KarninLangLiberty<int64_t>();
```

## Why the current setup appears to work
- We run all C++ benchmarks via Google Benchmark (same style as the upstream
  repo), which changes binary layout, initialization order, and memory
  patterns.
- Those changes can leave non-zero values in memory that make the UB "look"
  correct at runtime.
- This is not a real fix; it is a fragile, layout-dependent side effect.

## Current mitigation in this repo
- The C++ benchmarks mimic the external include order
  (see `cpp-bench/kll/final_kll.cpp`).
- This matches the upstream benchmark setup and avoids the crash in practice,
  but does not remove the UB.

## Real fix (external library)
- Reorder the member declarations in `kll_final.hpp` so
  `level_capacities` is declared before `max_capacity_`, or compute
  `max_capacity_` after `level_capacities` is initialized.
