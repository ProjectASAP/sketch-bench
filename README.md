# Approximate Benchmark

A benchmark to tell how approximation is helping.
Current status: a benchmark over sketch instance.

## Quickstart

### Compile

At root directory of this project, just run:

```sh
% cargo build --release
```

### Example: HyperLogLog insertion throughput

```sh
./target/release/approxbench sketchbench \
      --variant hll --library lib --config 'lg_k=14' \
      --dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 --dtype i64 \
      --runs 10 --warmup-runs 3 \
      --operations insert --metrics throughput --pretty-print
approxbench: hll/lib config={"lg_k":14} runs=10 warmup=3
{
  "schema_version": 4,
  "sketch": "hll",
  "algorithm": "hll",
  "impl": "lib",
  "language": "rust",
  "sketch_config": {
    "algorithm": "hll",
    "params": {
      "lg_k": 14
    }
  },
  "workload": {
    "column_num": 1,
    "column_label": [
      "key"
    ],
    "column_spec": [
      {
        "distribution": {
          "kind": "zipf",
          "skewness": 1.1,
          "population_size": 100000,
          "seed": 42
        },
        "special_rule": 0,
        "data_type": "i64"
      }
    ],
    "row_num": 1000000
  },
  "mode": "bench",
  "runs": 10,
  "bench": {
    "metric": "throughput",
    "operation": "insert",
    "throughput_items_per_sec": {
      "mean": 649590618.9830792,
      "stddev": 14589920.644625586,
      "n": 10,
      "samples": [
        653007917.7210023,
        654718062.0345364,
        639829242.3717958,
        655039711.7825269,
        661066114.5442479,
        657912916.0147846,
        653097051.5280511,
        657714298.1168982,
        652227225.5297389,
        611293650.1872087
      ]
    },
    "wall_time_ms": {
      "mean": 1.5401624,
      "stddev": 0.03618442327300518,
      "n": 10,
      "samples": [
        1.531375,
        1.527375,
        1.562917,
        1.526625,
        1.512708,
        1.519958,
        1.531166,
        1.520417,
        1.533208,
        1.635875
      ]
    },
    "memory_bytes": 16384
  },
  "source": "cli",
  "timestamp": "2026-08-25T23:19:34.546016Z"
}
```

#### CLI Flag Explanation

In the example above, there are many CLI flags being used.
Here is a explanation about what they are.

**`--variant`:** `variant` defines which instance to test against.
In this case, it is a regular HyperLogLog.

**`--library`:** `library` defines which library the variant is from.
`lib` stands for `asap_sketchlib`.

**`--config`:** `config` takes user configuration for each sketch instance.
Different variant takes configuration differently.
In this case, `hll` takes a `lg_k` which describes the size of register list.

**`--dataset`:** A benchmark needs to take different data as input.
`dataset` is the command line argument that takes description of input data.
`zipf` means the input data is under zipf distribution.

**`--size`:** `size` means how large the dataset is.

**`--zipf-s`:** Some distribution requires parameter, like `zipf-s`.
`zipf-s` means skewness, a parameter for zipf distribution.
`--dataset pareto` instead takes `--pareto-alpha` (shape) and `--pareto-scale`
(minimum value, default 1.0); it is unbounded, so it needs `--dtype f64` (or
`i64`, which floors each draw) and ignores `--cardinality`.

**`--cardinality`:** Another parameter required by zipf distribution.
`cardinality` means the size of key-space of this synthesized zipf dataset.

**`--dtype`:** `dtype` means what data the generated will be.
Possible `dtype` are `i64`, `u64`, `f64` and `string`.
Specially, some `dtype` may not be supported by specific sketch (i.e., quantile related sketch will reject `string` data).
Those command line argument will be rejected.

**`--runs`:** To get accurate benchmark result, repeating benchmarks will be necessary in many cases.
`runs` is how to specify the number to repeat the benchmark.

**`--warmup-runs`:** Some benchmark result, like throughput, will be different at a cold start or at middle.
To minimize the difference caused by under-utilized hardware, `warmup-runs` will be helpful and even necessary.

**`--operations`:** There are four operations defined: `insert`, `query`, `merge` and `prepare`.
User can specify which operation they are interested in for benchmark.

**`--metrics`:** To specify which metric to test, user can specify things like `throughput` or `accuracy`.
Not all combinations of `metrics` and `operations` are meaningful.
For exmample, `accuracy` of `insert` is meaningless, thus will be rejected.

**`--pretty-print`:** just a simple command to print the `json` in multiple lines for readability.

#### Output Explanaition

```json
"sketch": "hll",
"algorithm": "hll",
"impl": "lib",
"language": "rust",
"sketch_config": {
    "algorithm": "hll",
    "params": {
      "lg_k": 14
    }
}
```

Above part describes the sketch instance.
It is read as "a regular hll variant of hll algorithm from asap_sketchlib, with 2^14 registers".

```json
"workload": {
    "column_num": 1,
    "column_label": [
      "key"
    ],
    "column_spec": [
      {
        "distribution": {
          "kind": "zipf",
          "skewness": 1.1,
          "population_size": 100000,
          "seed": 42
        },
        "special_rule": 0,
        "data_type": "i64"
      }
    ],
    "row_num": 1000000
},
```

Above is the description of synthetic data for input.
This is following the configuration specified by user.

```json
"mode": "bench",
"runs": 10,
"bench": {
    "metric": "throughput",
    "operation": "insert",
    "throughput_items_per_sec": {
      "mean": 649590618.9830792,
      "stddev": 14589920.644625586,
      "n": 10,
      "samples": [
        653007917.7210023,
        654718062.0345364,
        639829242.3717958,
        655039711.7825269,
        661066114.5442479,
        657912916.0147846,
        653097051.5280511,
        657714298.1168982,
        652227225.5297389,
        611293650.1872087
      ]
    },
    "wall_time_ms": {
      "mean": 1.5401624,
      "stddev": 0.03618442327300518,
      "n": 10,
      "samples": [
        1.531375,
        1.527375,
        1.562917,
        1.526625,
        1.512708,
        1.519958,
        1.531166,
        1.520417,
        1.533208,
        1.635875
      ]
    },
    "memory_bytes": 16384
},
```

Above is the actual throughput data.
It describes the `metric` being tested as well as `operation` being tested.
`n` means how many runs are performed.
In this case, 10 runs are performed.
Thus there can be a `mean` and `stddev` of all those 10 runs data.
Raw data are recorded under `samples`.
`wall_time_ms` is the time of each run.
`memory_bytes` is the memory of the `hll` being used in this experiment.

### Example: Hydra merge throughput

```sh
./target/release/approxbench sketchbench \
    --variant hydra-cms --library lib \
    --spec configs/datagen/hydra_columns.yaml --dtype i64\
    --config "rows=3 cols=1024 cell_rows=3 cell_cols=1024" \
    --operations insert,merge --metrics throughput,cpu,memory \
    --merge-shards 8 --runs 5 --warmup-runs 2 --flat --pretty-print
approxbench: hydra-cms/lib config={"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3} runs=5 warmup=2
{
  "schema_version": 4,
  "sketch": "hydra-cms",
  "impl": "lib",
  "language": "rust",
  "mode": "bench",
  "runs": 5,
  "source": "cli",
  "sketch_config": {
    "algorithm": "hydra-cms",
    "params": {
      "cell_cols": 1024,
      "cell_rows": 3,
      "cols": 1024,
      "rows": 3
    }
  },
  "workload": {
    "column_num": 3,
    "column_label": [
      "key1",
      "key2",
      "value"
    ],
    "column_spec": [
      {
        "distribution": {
          "kind": "uniform",
          "lower_bound": 0.0,
          "upper_bound": 200.0,
          "seed": 1
        },
        "special_rule": 0,
        "data_type": "string"
      },
      {
        "distribution": {
          "kind": "zipf",
          "skewness": 1.1,
          "population_size": 50,
          "seed": 2
        },
        "special_rule": 0,
        "data_type": "string"
      },
      {
        "distribution": {
          "kind": "zipf",
          "skewness": 1.2,
          "population_size": 1000,
          "seed": 3
        },
        "special_rule": 0,
        "data_type": "i64"
      }
    ],
    "row_num": 200000
  },
  "memory_bytes": 38437088,
  "heap_bytes_net": null,
  "heap_bytes_peak": null,
  "insert_timestamp": "2026-08-25T23:19:46.170228Z",
  "insert_throughput_items_per_sec": {
    "mean": 3362367.724670238,
    "stddev": 89256.64220060871,
    "n": 5,
    "samples": [
      3465463.7857041755,
      3369347.9567077872,
      3376888.15535374,
      3381360.372674605,
      3218778.3529108823
    ]
  },
  "insert_latency_ns": null,
  "insert_cpu_time_ms": {
    "user_ms": {
      "mean": 56.474399999999996,
      "stddev": 1.2323211026351892,
      "n": 5,
      "samples": [
        55.688,
        56.549,
        55.61,
        55.947,
        58.578
      ]
    },
    "sys_ms": {
      "mean": 2.9716,
      "stddev": 0.5908500655834776,
      "n": 5,
      "samples": [
        2.027,
        2.811,
        3.263,
        3.202,
        3.555
      ]
    }
  },
  "insert_wall_time_ms": {
    "mean": 59.5160582,
    "stddev": 1.6092466137157788,
    "n": 5,
    "samples": [
      57.712333,
      59.358666,
      59.226125,
      59.147792,
      62.135375
    ]
  },
  "insert_rss_peak_kb": null,
  "insert_heap_allocated_kb": 2270156,
  "query_timestamp": null,
  "query_throughput_items_per_sec": null,
  "query_latency_ns": null,
  "query_accuracy": null,
  "query_cpu_time_ms": null,
  "query_wall_time_ms": null,
  "query_rss_peak_kb": null,
  "query_heap_allocated_kb": null,
  "merge_timestamp": "2026-08-25T23:19:46.170231Z",
  "merge_folds_per_sec": {
    "mean": 244.1933171842133,
    "stddev": 129.27731177919472,
    "n": 5,
    "samples": [
      81.74828374441567,
      198.36724487506888,
      211.26866622801256,
      300.797655222118,
      428.78473585145133
    ]
  },
  "merge_shards": 8,
  "merge_supported": true,
  "merge_cpu_time_ms": {
    "user_ms": {
      "mean": 13.839599999999999,
      "stddev": 0.9072333216984481,
      "n": 5,
      "samples": [
        14.453,
        13.201,
        13.126,
        15.138,
        13.28
      ]
    },
    "sys_ms": {
      "mean": 20.4894,
      "stddev": 21.064262491243316,
      "n": 5,
      "samples": [
        56.377,
        18.185,
        17.27,
        7.57,
        3.045
      ]
    }
  },
  "merge_wall_time_ms": {
    "mean": 38.7293252,
    "stddev": 27.311176532236207,
    "n": 5,
    "samples": [
      85.628709,
      35.288084,
      33.133167,
      23.271458,
      16.325208
    ]
  },
  "merge_rss_peak_kb": null,
  "merge_heap_allocated_kb": 1201747,
  "prepare_timestamp": null,
  "prepare_cpu_time_ms": null,
  "prepare_wall_time_ms": null,
  "prepare_rss_peak_kb": null,
  "prepare_heap_allocated_kb": null
}
```

#### CLI Flag Explanation (Alternative)

**`--spec`**: Alternatively, the input data configuration can be provided with a `yaml` file.
`--spec configs/datagen/hydra_columns.yaml` is how user can define the synthetic table for insertion.
The file can be found [under configs directory](./configs/datagen/hydra_columns.yaml).

**`--flat`**: flatten record such that output data of multiple experiments is flatten into one json file.

#### General Explanation

Most of the input and output is shared between this hydra example and the previous hll example.

Thus, here is the read of the hydra experiment:

This example is testing Hydra-over-Count-Min insert throughput and merge cost.
Hydra instance comes from `asap_sketchlib`, with configuration `rows=3 cols=1024` (the outer sketch) and `cell_rows=3 cell_cols=1024` (the inner Count-Min sketch), which takes about 38.4 MB.
Input data is a 200k-row, 3-column table: two label columns (uniform, then zipf) and an i64 value column (zipf)
Warmup the benchmark with 2 runs, and benchmark is run for 5 times.
`merge` folds 8 shards into one.
Data is in field `insert_throughput_items_per_sec` and `merge_time_ms` / `merge_folds_per_sec` (merge/sec).

## How to use

A detailed roadmap (under construction) for sketch instance benchmark can be found at [workflow](./docs/sketch-bench-workflow.md).

## Want to add more?

Check this (under construction) [developer_guide](./docs/developer_guide.md) about how to add a sketch instance to benchmark and how to adjust ground-truth calculation to meet demands.

## Reference

Some crate-oriented design thought is as followed, if anyone is interested:

<!-- separate by functionality / logic, not by crate
docs mimicing code structure is not necessary

dev docs: help developer to achieve new things, like extend/contribute, what do they need; specific to the code; code detail, can be added after code is there;

desgin doc: how something is achieved someway; why something is done this way; design principle;

some concepts can go to one location

separate by crates may not be necessary at this moments -->

[Core crate](./docs/aqpbm-core.md)

[CLI](./docs/aqpbm-cli.md)

[Data Generation](./docs/aqpbm-datagen.md)

[Sketch benchmark wrapper](./docs/sketch-bench.md)
