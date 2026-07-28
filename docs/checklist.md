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
    - 41 rows, 8 algorithms (hll kll cms countsketch elastic nitro dd univmon)
    - 3 libraries (sketch_oxide, datasketches, asap_sketchlib) + exact + null baseline
- Merge benchmark
    - split the stream into N shards, one sketch each, time only the fold
    - report the shard count actually folded, not the one requested
    - unsupported sketch will be reported as unsupported
- Data Type is an open axis, not a fixed list
    - generator: i64, u64, f64, string. adding one is a `DType` variant, a
      `GenValue` impl, and one arm in the dispatch `match` -- which has no
      `_`, so a missing arm does not compile. it used to be 14 scattered edits
    - benchmark: i64 everywhere; f64 on the ordered algorithms (kll, dd);
      string on the text rows (elastic, nitro, univmon). all via `--dtype`
    - hash algorithms stay i64-only for f64 on purpose -- f64 is not `Hash` in
      Rust and hashing its bits would retrace the i64 curve. quantile rows
      refuse strings: a lexicographic quantile is a different question
    - u64 generates but nothing consumes it
    - a row that cannot take the dtype is skipped with a reason, and a sweep
      where everything skipped is an error, not an empty file
    - for i64/f64 the drawn values are identical and only the encoding
      differs, so dtype is a controlled variable. string is the exception:
      the rank is rendered rather than cast, so the bytes are genuinely new
- Real string workloads
    - generated, not `i64::to_string()`: configurable alphabet, length varies
      per key within `[min_len, max_len]`
    - rendering is injective over `cardinality` by construction (the leading
      characters positionally encode the rank), so a run asking for N distinct
      keys gets N. without that every per-key error is divided by the wrong
      denominator
    - a key renders the same way every time it is drawn, so a repeated key is
      a repeated key
    - the old decimal-formatted path is kept under `--dtype i64`, so the two
      are directly comparable rather than one replacing the other

## Partial / Not what it looks like

- strings have no file format
    - `.bin` is a bare sequence of equal-width values with nowhere to record a
      length, so `workload generate --dtype string` refuses rather than
      writing something unreadable. strings only exist in-process until there
      is a CSV sink
- Bytes are derived from strings, not drawn
    - the `Vec<u8>` rows take the bytes of whichever string workload is in
      play. fine for now, but it means there is no byte-string value domain
      of its own (e.g. arbitrary non-UTF8 keys)
- the generator owns the `.bin` format but can only write it
    - reading is `I64Workload::load`, up in `sketch-core`, so `aqpbm-datagen`
      cannot round-trip its own format or check it in its own tests
- HLL(our wrapper for datasketches) merge is broken

## No / In-Progress

- a continuous value domain for the quantile algorithms
    - every shape draws from a finite `cardinality`, so kll/dd are always
      scored on tied data whatever the dtype
- range constraint on the key space (currently always `[0, cardinality)`)
- manual code review
- Potentially more code refactor / removal
    - including plotting scripts
    - including removal of server
- match C++ BM to Rust BM
