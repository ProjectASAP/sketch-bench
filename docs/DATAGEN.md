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

Because the `.bin` stream is header-less it cannot describe itself — a
`u64`/`f64` file is byte-indistinguishable from an `i64` one. Passing
one to `bench --input` would reinterpret each 8-byte word as an `i64`
(the IEEE-754 pattern of `0.093` reads back as `4591388162153532928`)
and still emit a well-formed, plausible-looking report. The loader
therefore reads the sidecar and **refuses** any `.bin` whose declared
dtype is not `i64`. Files with no sidecar — every legacy
`input/benchmark_data_*.bin` — are still assumed `i64` and load
unchanged.

`cardinality` means the same thing for every dtype: the number of
distinct keys. `--dtype f64` emits whole numbers (`0.0`, `1.0`, …), not
a continuous range, so a fixed `(shape, size, seed)` yields the same
logical values under every dtype and only the physical encoding
changes. A genuinely continuous real-valued shape (for quantile
sketches) would be a new `Shape` variant, not a reinterpretation of
this flag.

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

For full generality (explicit weights, distributions the CLI flags
don't expose) pass a `.yaml`/`.yml` or `.json` spec; it overrides
`--shape` and its flags. Examples live in `configs/datagen/`.

A spec is a **structure** (`keys` / `categorical` / `monotonic`) plus a
**distribution** that structure draws over — the two axes are
orthogonal, so the same `dist` block is reused across structures:

```yaml
# keys: a large key space drawn zipf
shape: keys
cardinality: 1000000
dist: { kind: zipf, s: 1.1 }
dtype: i64
size: 1000000
seed: 42
```
```yaml
# categorical: a finite id domain with explicit weights
shape: categorical
categories: [0, 1, 2, 3, 4, 5, 6, 7]
dist: { kind: explicit, weights: [40, 25, 15, 8, 5, 3, 2, 2] }
size: 1000000
```
```yaml
# monotonic: cumulative non-negative gaps
shape: monotonic
start: 1700000000000
unit: millis
gap: { kind: exponential, lambda: 0.5 }
min_gap: 1
dtype: i64
size: 1000000
```

The CLI presets map onto these: `--shape uniform`/`zipf` →
`keys` with the matching `dist`; `--shape monotonic-timestamp` →
`monotonic`; `--shape skewed-categorical` → `categorical`.

## Extending

Distribution (*how* values spread) and structure (*what* they mean) are
separate axes:

- **New distribution:** add a `datagen::Distribution` variant plus the
  arm(s) in whichever realization it supports (`key_sampler` /
  `weights` / `gap_sampler`). Every structure picks it up for free.
- **New structure:** add a struct implementing
  `datagen::ColumnGenerator`, a variant to `datagen::shape::Shape`, and
  one arm to `Shape::build`. Nothing in the benchmark changes.
- **New physical type:** add a `datagen::DType` variant and a
  `datagen::Column` arm.

Every generator is a pure function of `(spec, seed, n)` — the same
inputs always yield byte-identical output.
