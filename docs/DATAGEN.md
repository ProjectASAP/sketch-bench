# Synthetic data generation (`sketchlib workload`)

`sketch_core::datagen` produces benchmark workloads as raw
little-endian `.bin` files plus a self-describing `.meta.json` sidecar.
The generator is deliberately decoupled from the benchmark: it writes
files, and `sketchlib bench --input <path>` reads them through the
existing `FileI64` loader — so adding a distribution never touches the
benchmark dispatch machinery.

## Quick start

```bash
# Zipfian i64 keys, 1M rows, into input/
sketchlib workload generate --shape zipf --cardinality 100000 --zipf-s 1.1 \
    --size 1000000 --out input/zipf_1m.bin

# Inspect provenance + stats
sketchlib workload describe input/zipf_1m.bin

# Feed it to any i64 sketch
sketchlib bench --sketch cms --impl datasketches --input input/zipf_1m.bin
```

## Output format

- **`.bin`** — a header-less, raw little-endian stream of the chosen
  dtype. For `i64` this is byte-compatible with the legacy
  `input/benchmark_data_*.bin` files and both the Rust and C++ readers.
- **`foo.bin.meta.json`** — provenance: schema version, generator
  version, dtype, count, seed, the full shape parameters (round-trips
  exactly), and a `min/max/first/last` summary. The benchmark ignores
  it; `describe` reads it. Skip it with `--no-meta`.

Only `i64` files are consumable by `bench` today. `u64`/`f64` are
written for other tooling; the logical dtype lives in the sidecar.

## Shapes

| shape | flags | notes |
|-------|-------|-------|
| `uniform` | `--cardinality` `--dtype` | uniform over `[0, cardinality)` |
| `zipf` | `--cardinality` `--zipf-s` `--dtype` | ranks `[1, cardinality]` |
| `monotonic-timestamp` | `--start` `--unit` `--gap` `--min-gap` `--dtype` | cumulative gaps; i64/u64 |
| `skewed-categorical` | `--categories <n>` `--weights` | draws from ids `0..n` (i64) |

`--gap` is `kind:param`: `const:1000`, `geometric:0.01`, `exp:0.5`,
`poisson:5`. `--min-gap 1` (default) → strictly increasing; `0` allows
duplicate timestamps. `--weights` is `uniform` or `zipf:s`; explicit
per-id weights require a spec file. `--unit` (`nanos|millis|secs`) is a
metadata label only — it does not rescale values.

```bash
# Event timestamps: Poisson-process arrivals in milliseconds
sketchlib workload generate --shape monotonic-timestamp \
    --start 1700000000000 --unit millis --gap exp:0.5 --size 1000000 \
    --out input/events_ts.bin

# ~40 OS-type ids, heavy skew toward the common ones
sketchlib workload generate --shape skewed-categorical \
    --categories 40 --weights zipf:1.2 --size 1000000 \
    --out input/os_types.bin
```

## Spec files (`--spec`)

For full generality (explicit weights, complex gap trees) pass a
`.yaml`/`.yml` or `.json` spec; it overrides `--shape` and its flags.
The shape's tagged fields sit alongside `size`/`seed`. Examples live in
`configs/datagen/`.

```bash
sketchlib workload generate --spec configs/datagen/os_types.yaml \
    --out input/os_types.bin
```

## Extending

- **New distribution:** add a struct implementing
  `datagen::ColumnGenerator`, a variant to `datagen::shape::Shape`, and
  one arm to `Shape::build`. Nothing in the benchmark changes.
- **New physical type:** add a `datagen::DType` variant and a
  `datagen::Column` arm.

Every generator is a pure function of `(spec, seed, n)` — the same
inputs always yield byte-identical output.
