# Sketchlib-Rust Micro-benchmarks

Criterion benchmarks for individual sketch operations (insert, estimate, hashing, etc.) from [sketchlib-rust](https://github.com/ProjectASAP/sketchlib-rust).

## Running

```bash
# all benchmarks
cargo bench

# a single benchmark suite
cargo bench --bench countmin

# filter by function name
cargo bench -- countmin_default/insert_only
```

HTML reports are written to `target/criterion/`.

## Benchmarks

| File | What it measures |
|---|---|
| `countmin` | CountMin insert & estimate across storage/hash variants, plus a stack-array baseline |
| `count` | Count Sketch insert with FixedMatrix vs Vector2D storage |
| `nitro` | Nitro sampling at varying rates (1%–100%) on CountMin fast path |
| `nitro_batch` | Batch Nitro insert vs plain CountMin insert |
| `hashlayer` | HashSketchEnsemble (hash-once, fan-out) vs separate per-sketch inserts |
| `hash_detailed` | xxHash variant comparison: xxhash32, xxhash64, xxhash3_64, xxhash3_128 |
| `row_access` | Row-update strategies for Nitro: skip-nothing, skip-packet, skip-rows |
| `median_bench` | Median of 3/4/5 via sorting-network vs `sort_unstable` |
| `box_vec` | Memory layout comparison: `Vec<Vec>`, `Vec<Box<[T]>>`, flattened `Vec` |
