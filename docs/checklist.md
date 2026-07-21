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

## Partial / Not what it looks like

- Multiple Data Type
    - generator emits i64 / u64 / f64 columns
    - but `bench` consumes i64 only; f64 spec is rejected, not converted
    - String / Bytes are not generated -- they are i64 decimal-formatted,
      so ~1-7 bytes over a 10-char alphabet. Not a real string workload;
      hash cost and length distribution are the whole point and neither varies
    - KLL / DDSketch are fed integers
- HLL(our wrapper for datasketches) merge is broken

## No / In-Progress

- f64 into BM
- real string generator (length distribution + alphabet)
- range constraint on the key space (currently always `[0, cardinality)`)
- manual code review
- Potentially more code refactor / removal
    - including plotting scripts
    - including removal of server
- match C++ BM to Rust BM
