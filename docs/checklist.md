# Checklist about Sketch Primitive Benchmark

... which is AQPBMV1

## Yes

- Multiple Distribution
    - key distribution: Uniform, Zipf
    - gap distribution (timestamp / monotonic only): Geometric, Exponential, Poisson
- Multiple input data constraint
    - finite element (`cardinality`)
- Data Generator with various output interface
    - keep in memory, save to file as `.bin`
    - `Sink<T>` is the extension point: CSV, ClickHouse, anything else is a
      new impl and touches no generator
- Data Generator is its own crate (`aqpbm-datagen`)
    - depends on nothing else in the workspace, knows nothing about sketches.
      named for the program, not for its first consumer
- Various sketch from different library connectet into the BM-framework
    - 41 rows, 8 families (hll kll cms countsketch elastic nitro dd univmon)
    - 3 libraries (sketch_oxide, datasketches, asap_sketchlib) + exact + null baseline
- Merge benchmark
    - split the stream into N shards, one sketch each, time only the fold
    - report the shard count actually folded, not the one requested
    - unsupported sketch will be reported as unsupported
- Data Type is an open axis, not a fixed list
    - generator: i64, u64, f64. adding one is a `DType` variant, a `GenValue`
      impl, and one arm in the dispatch `match` -- which has no `_`, so a
      missing arm does not compile. it used to be 14 scattered edits
    - variable-width types are not blocked by the design. `.bin` cannot hold
      them (no length field), but that constraint sits on that one sink:
      `BinSink::<String>` will not compile, memory and CSV are unaffected
    - benchmark: i64 everywhere, f64 on the ordered families (kll, dd) via
      `--dtype`. hash families stay i64-only on purpose -- f64 is not `Hash`
      in Rust and hashing its bits would retrace the i64 curve
    - u64 generates but nothing consumes it
    - a row that cannot take the dtype is skipped with a reason, and a sweep
      where everything skipped is an error, not an empty file
    - dtype changes the encoding, not which values are drawn, so it is a
      controlled variable rather than a second dataset

## Partial / Not what it looks like

- String / Bytes Data Type
    - not generated -- they are i64 decimal-formatted, so ~1-7 bytes over a
      10-char alphabet. Not a real string workload; hash cost and length
      distribution are the whole point and neither varies
    - no longer blocked by the generator's shape: a `GenValue` impl plus a
      dispatch arm is all it takes. what is missing is the value domain
      (alphabet, length distribution), which is a `Shape` question
- the generator owns the `.bin` format but can only write it
    - reading is `I64Workload::load`, up in `sketch-core`, so `aqpbm-datagen`
      cannot round-trip its own format or check it in its own tests
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
