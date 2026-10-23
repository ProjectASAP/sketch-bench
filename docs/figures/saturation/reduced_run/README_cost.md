# How to read `cost_cpu_memory.png`

Serial measurements at N = 10⁷ (one process at a time, 3 runs + 1 warm-up, 16 merge shards, seed 1). Columns are sketches; rows are:

1. **insert CPU (ns / item)** — phase user+sys CPU / items inserted.
2. **merge CPU (µs / fold)** — one fold merges one shard into the accumulator (15 folds for 16 shards).
3. **query CPU (µs / query)** — per point query (frequency / top-k / cardinality) or per quantile probe.
4. **memory_bytes** — the sketch's own size formula for one instance.

x-axis is Zipf θ (one line per K) or Pareto α.

## What it shows

| sketch | insert | merge | query | memory | depends on θ / K / α? |
|---|---|---|---|---|---|
| CMS 3×1024 | 27 ns | ≈ 2 µs | ≈ 0.03 µs | 12 KB | **no** (flat; merge spikes of ~1 µs are timer noise at µs scale) |
| CountSketch 3×1024 | 31 ns | ≈ 2 µs | ≈ 0.06 µs | 12 KB | **no** |
| CMS-heap top-k 3×1024, heap 32 | **100 → 2,100 ns** | 70 – 145 µs | ≈ 0 – 2,300 µs | 12 KB | **yes, strongly**: insert cost grows ~20× with θ (more heap updates as heavy keys keep changing rank); query cost is large only at K = 10M with low θ (heap churn), ~0 otherwise |
| HLL lg_k=12 | 4 ns | 3 – 8 µs | 20 µs (K=1K) / 45 – 65 µs (K ≥ 100K) | 4 KB | insert **no**; query/merge depend on K (register fill), not θ |
| KLL k=200 | 32 ns | ≈ 7 µs | ≈ 33 µs | 6.4 KB | **no** (comparison-based) |
| DDSketch α=0.01 | 20 → 27 ns | 1 – 3 µs | 21 – 27 µs | **6.3 → 2.3 KB** | memory **yes**: heavier tail (small α) → wider value range → more buckets |

## Using it with the error figures

- Per-item insert cost is N-independent for every sketch, so ingest CPU for a deployment is `arrival rate × (x / y) × insert ns`, as in PR #129's cost model.
- Merge cost per fold is set by the instance size, not N; a query over lookback S merging m = S/x instances costs `(m − 1) × merge µs` plus one query.
- The two distribution-sensitive costs to plan for are **top-k insert at high θ** (use the *upper* θ bound for CPU sizing — the opposite end from accuracy) and **DDSketch memory at heavy tails** (use the *lower* α bound).
- Four top-k points (θ = 1.0 at all K, θ = 1.2 at K = 1K) were re-measured serially after an overlap with another job; KLL/DDSketch costs come from the 10-seed run.
