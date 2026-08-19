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

explanation of cli flag
not just adjusting the number
what different cli flag means and what can be done with that is more important

```sh
% ./target/release/approxbench sketchbench \
      --algorithm hll --impl lib --config 'lg_k=14' \
      --dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 \
      --runs 10 --warmup-runs 3 \
      --operations insert --metrics throughput
approxbench: hll/lib config={"lg_k":14} runs=10 warmup=3
{"schema_version":3,"sketch":"hll","family":"hll","impl":"lib","language":"rust","sketch_config":{"algorithm":"hll","params":{"lg_k":14}},"workload":{"shape":"zipf","size":1000000,"cardinality":100000,"zipf_s":1.1,"seed":42},"mode":"bench","runs":10,"bench":{"metric":"throughput","operation":"insert","throughput_items_per_sec":{"mean":694468886.4950684,"stddev":3084050.9269289766,"n":10},"throughput_samples":[695289414.2186685,697695024.9460857,692321325.7122601,695894224.0779402,699361832.3280008,689377519.6748344,695732999.9380797,694163404.6771342,690428928.972124,694424190.4055575],"wall_time_ms":{"mean":1.4399749,"stddev":0.006400518945106468,"n":10},"memory_bytes":16384},"source":"cli","timestamp":"2026-08-17T09:11:20.208786Z"}

```

#### Explanaition

This example is testing HyperLogLog insertion throughput.
HyperLogLog instance comes from `asap_sketchlib`, with configuration `lg_k=14` (in short, register size is `2^14`).
Input data has zipf distribution.
Warmup the benchmark with 3 runs, and benchmark is run for 10 times.
Data is in field `throughput_items_per_sec`.

### Example: Hydra merge throughput

```sh
% ./target/release/approxbench sketchbench \
    --algorithm hydra-cms --impl lib \
    --spec configs/datagen/hydra_columns.yaml \
    --config "rows=3 cols=1024 cell_rows=3 cell_cols=1024" \
    --operations insert,merge --metrics throughput,cpu,memory \
    --merge-shards 8 --runs 5 --warmup-runs 2 --flat
approxbench: hydra-cms/lib config={"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3} runs=5 warmup=2
{"schema_version":3,"sketch":"hydra-cms","impl":"lib","language":"rust","mode":"bench","runs":5,"source":"cli","sketch_config":{"algorithm":"hydra-cms","params":{"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3}},"workload":{"shape":"columns","size":200000,"seed":1,"spec":{"column_label":["key1","key2","value"],"column_num":3,"column_spec":[{"data_type":"string","distribution":{"kind":"uniform","lower_bound":0.0,"seed":1,"upper_bound":200.0},"special_rule":0},{"data_type":"string","distribution":{"kind":"zipf","population_size":50,"seed":2,"skewness":1.1},"special_rule":0},{"data_type":"i64","distribution":{"kind":"zipf","population_size":1000,"seed":3,"skewness":1.2},"special_rule":0}],"row_num":200000}},"memory_bytes":38437088,"heap_bytes_net":null,"heap_bytes_peak":null,"insert_timestamp":"2026-08-17T09:08:16.354935Z","insert_throughput_items_per_sec":{"mean":3146644.494778336,"stddev":661990.5995087331,"n":5},"insert_throughput_samples":[3548652.8107388476,2007851.5428301138,3160803.907486936,3379170.3714341833,3636743.841401601],"insert_build_throughput_items_per_sec":null,"insert_latency_ns":null,"insert_cpu_time_ms":{"user_ms":{"mean":65.00460000000001,"stddev":12.732758353946721,"n":5},"sys_ms":{"mean":0.9918,"stddev":1.0513268759049204,"n":5}},"insert_wall_time_ms":{"mean":66.6847582,"stddev":18.67578157952414,"n":5},"insert_rss_peak_kb":null,"insert_heap_allocated_kb":18221,"query_timestamp":null,"query_throughput_items_per_sec":null,"query_latency_ns":null,"query_accuracy":null,"query_cpu_time_ms":null,"query_wall_time_ms":null,"query_rss_peak_kb":null,"query_heap_allocated_kb":null,"merge_timestamp":"2026-08-17T09:08:16.354940Z","merge_time_ms":{"mean":12.1078084,"stddev":0.2835344216392432,"n":5},"merge_folds_per_sec":{"mean":578.3938365703082,"stddev":13.58991397643068,"n":5},"merge_shards":8,"merge_supported":true,"merge_cpu_time_ms":{"user_ms":{"mean":95.2826,"stddev":2.1805422032146056,"n":5},"sys_ms":{"mean":17.5492,"stddev":2.0397143672583176,"n":5}},"merge_wall_time_ms":{"mean":12.1078084,"stddev":0.2835344216392432,"n":5},"merge_rss_peak_kb":null,"merge_heap_allocated_kb":18268,"prepare_timestamp":null,"prepare_finalize_time_ms":null,"prepare_cpu_time_ms":null,"prepare_wall_time_ms":null,"prepare_rss_peak_kb":null,"prepare_heap_allocated_kb":null}
```

#### Explanation

This example is testing Hydra-over-Count-Min insert throughput and merge cost.
Hydra instance comes from `asap_sketchlib`, with configuration `rows=3 cols=1024` (the outer sketch) and `cell_rows=3 cell_cols=1024` (the inner Count-Min sketch), which takes about 38.4 MB.
Input data is a 200k-row, 3-column table: two label columns (uniform, then zipf) and an i64 value column (zipf)
Warmup the benchmark with 2 runs, and benchmark is run for 5 times.
`merge` folds 8 shards into one.
Data is in field `insert_throughput_items_per_sec` and `merge_time_ms` / `merge_folds_per_sec` (merge/sec).

## How to use

A detailed roadmap (under construction) for sketch instance benchmark can be found at [workflow](./docs/sketch-bench-workflow.md).

## Want to add more?

Check this (under construction) [developer_guida](./docs/developer_guide.md) about how to add a sketch instance to benchmark and how to adjust ground-truth calculation to meet demands.

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
