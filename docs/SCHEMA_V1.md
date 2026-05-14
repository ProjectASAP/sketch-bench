# v1 JSONL `Record` cross-language contract

`sketch-core::report::Record` (`sketch-core/src/report.rs`) is the
single record shape shared by every track that produces benchmark
output: the Rust `sketch-cli` (`sketchlib bench …`), the C++ track
under `cpp-bench/`, and any future embedded sampler. Everyone serialises
*exactly* the same fields with *exactly* the same names.

This file pins down the contract for the cross-language case so a
non-Rust emitter (currently just C++) can produce bytes that
`serde_json::from_str::<Record>` will deserialise without surprises.

## Top-level fields

| field | required | notes |
|---|---|---|
| `schema_version` | yes | integer; currently **2**. |
| `sketch` | yes | family name. Lowercase. Examples: `"hll"`, `"cms"`, `"cs"`, `"kll"`. |
| `impl` | yes | implementation key. Rust uses `"oxide"` / `"lib"` / etc.; C++ uses `"datasketches"` / `"final"` / `"naive"` / etc. |
| `language` | recommended | `"rust"` or `"cpp"`. Optional — missing is interpreted as `"rust"` for back-compat with pre-`language` records. |
| `workload` | yes | `WorkloadDesc` (see below). |
| `mode` | yes | `"bench" \| "profile" \| "runtime"`. C++ track always emits `"bench"`. |
| `runs` | yes | number of measured runs aggregated into this record. |
| `bench` | optional | `BenchSection` — measured numbers. Required for `mode = "bench"`. |
| `profile` | optional | `ProfileSection` — micro-architectural data. Not used by C++ track yet. |
| `sketch_config` | optional | free-form JSON of construction params. |
| `source` | yes | `"cli" \| "asap-fusion" \| "data-collector" \| "asap-query" \| "cpp-bench"`. The C++ track always emits `"cpp-bench"`. |
| `timestamp` | yes | RFC3339 / ISO-8601 with a `Z` suffix or numeric offset. |

`WorkloadDesc` (snake_case keys):

```jsonc
{
  "shape": "file",          // "uniform" | "zipf" | "file"
  "size": 1000000,
  "cardinality": null,      // optional u64
  "zipf_s": null,           // optional f64
  "source_path": "input/benchmark_data_1m_int64.bin",  // optional string
  "seed": null              // optional u64
}
```

Omit any optional key by leaving it out (Rust's
`skip_serializing_if = "Option::is_none"`). Emitting `null` works too
because every optional field deserialises from `null` to `None`.

## `BenchSection` shape

```jsonc
{
  "throughput_items_per_sec": { "mean": 4.2e7, "stddev": 1.1e6, "ci95": [4.15e7, 4.25e7], "n": 10 },
  "latency_ns":               { "p50": 17, "p95": 41, "p99": 60, "p999": 95, "max": 312, "count": 1000000 },
  "accuracy":                 { /* family-specific, see below */ }
}
```

`RunStats` (mean/stddev/ci95/n): both ends of `ci95` are absolute
values (not deltas). `n` matches the top-level `runs`.

## `accuracy` payload — cross-language convention

`accuracy` is `serde_json::Value` on the Rust side, so the schema does
not enforce structure. To keep plot scripts simple, both tracks
**SHOULD** use the same keys per family:

### Quantile family (KLL, t-digest, etc.)

```jsonc
{
  "queries":         [0.5, 0.95, 0.99, 0.999],
  "abs_rank_err":    { "mean": 0.0021, "max": 0.0084 },
  "rel_rank_err":    { "mean": 0.0043, "max": 0.0190 }
}
```

`abs_rank_err` is `|est_rank/N - true_rank/N|`. `rel_rank_err` is
`|est_rank - true_rank| / true_rank` (skip the `q=0` corner).

### Frequency family (CMS, CS)

```jsonc
{
  "top_k":           100,
  "abs_freq_err":    { "mean": 12.4, "p99": 84.0 },
  "rel_freq_err":    { "mean": 0.0008, "p99": 0.005 }
}
```

### Cardinality family (HLL)

```jsonc
{
  "rel_card_err": 0.012   // |est - true| / true
}
```

Tracks may add extra keys; readers MUST ignore unknown keys.

## Numeric formatting rules

- Use plain JSON numbers, not strings. `2e7` and `20000000` are both fine.
- Avoid `NaN` / `Inf` — emit `null` instead (or simply omit the field).
- Latency percentiles are unsigned integers in nanoseconds.

## Round-trip test (Rust-side enforcement)

Any new emitter SHOULD have a Rust integration test that reads a
representative sample of its output and deserialises it via
`serde_json::from_str::<Record>`. See
`sketch-core/src/report.rs::tests::cpp_record_roundtrips` for the
shape such a test takes.
