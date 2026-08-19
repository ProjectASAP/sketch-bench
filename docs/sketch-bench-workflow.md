# Sketch Benchmark Workflow

This is a roadmap about how `sketch-bench` is executed.

In this document, the following is the example usage:

```sh
% ./target/release/approxbench sketchbench \
      --algorithm hll --impl lib --config 'lg_k=14' \
      --dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 \
      --runs 10 --warmup-runs 3 \
      --operations insert --metrics throughput
```

Assuming compilation is finished with:

```sh
cargo build --release
```

## CLI

### sketchbench

User starts from specifying `sketchbench`.
Command group refers to benchmark against wrappers defined in `sketch-bench`.

### Sketch Instance

**rename underway** be careful, will change after rename finishes

The following flag is the sketch instance selection:

```sh
--algorithm hll --impl lib --config 'lg_k=14'
```

`--algorithm` means the algorithm to be tested is `hll`, shorthand of HyperLogLog.
`--impl` means which library it is from.
`lib` means `asap_sketchlib`.
`--config` is the configuration of sketch.
The configuration is defined by the library providing the implementation.

### Input Data

Input data is important for benchmark.
User can specify the benchmark through command line argument:

```sh
--dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000
```

The example input data is interpreted as "1M data under zipf distribution, with skewness of 1.1 and 100000 distinct items".

More detail about data generation can be found at [aqpbm-datagen](./aqpbm-datagen.md).

### Benchmark Specification

**rename underway** will update later

The following two lines are about benchmark experiment specification:

```sh
--runs 10 --warmup-runs 3 \
--operations insert --metrics throughput
```

This is specifying that "the experiment will be executed 10 times, with 3 warm-up execution; this experiment is intended to test insertion throughput".

## Sketch-Bench

The wrappers are located in [sketch-bench](../sketch-bench/).

Each benchmark is a wrapper of sketch instance from different library.
Possible operation are wrapped under closure to pass across the benchmark.

...
I'm lost when I reach this...
Need to think about what I'm writing...
will continue...
