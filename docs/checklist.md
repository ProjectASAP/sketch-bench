# Checklist about Sketch Primitive Benchmark

... which is AQPBMV1

## Yes

- Multiple Distribution
    - key distribution: Uniform, Zipf
    - gap distribution (timestamp / monotonic only): Geometric, Exponential, Poisson
- Multiple input data constraint
    - finite element (`cardinality`)
- Data Generator with various output interface
    - keep in memory, save to file
- Various sketch from different library connectet into the BM-framework
    - 41 rows, 8 families (hll kll cms countsketch elastic nitro dd univmon)
    - 3 libraries (sketch_oxide, datasketches, asap_sketchlib) + exact + null baseline
- Merge benchmark
    - split the stream into N shards, one sketch each, time only the fold
    - report the shard count actually folded, not the one requested
    - unsupported sketch will be reported as unsupported
- Numeric Data Type: i64 and f64, via `--dtype`
    - ordered families (kll, dd) take both; hash families are i64-only on
      purpose, since f64 is not `Hash` in Rust and hashing its bits would
      retrace the i64 curve
    - a row that cannot take the dtype is skipped with a reason, and a sweep
      where everything skipped is an error, not an empty file
    - the values are the same either way, only the encoding differs, so this
      is an encoding axis and not a second dataset

## Partial / Not what it looks like

- String / Bytes Data Type
    - not generated -- they are i64 decimal-formatted, so ~1-7 bytes over a
      10-char alphabet. Not a real string workload; hash cost and length
      distribution are the whole point and neither varies
- HLL(our wrapper for datasketches) merge is broken

## No / In-Progress

- real string generator (length distribution + alphabet)
- a continuous value domain for the quantile families
    - every shape draws from a finite `cardinality`, so kll/dd are always
      scored on tied data whatever the dtype
- range constraint on the key space (currently always `[0, cardinality)`)
- manual code review
- Potentially more code refactor / removal
    - including plotting scripts
    - including removal of server
- match C++ BM to Rust BM
