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
./target/release/approxbench sketchbench \
      --variant hll --library lib --config 'lg_k=14' \
      --dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 --dtype i64 \
      --runs 10 --warmup-runs 3 \
      --operations insert --metrics throughput
approxbench: hll/lib config={"lg_k":14} runs=10 warmup=3
{"schema_version":4,"sketch":"hll","algorithm":"hll","impl":"lib","language":"rust","sketch_config":{"algorithm":"hll","params":{"lg_k":14}},"workload":{"column_num":1,"column_label":["key"],"column_spec":[{"distribution":{"kind":"zipf","skewness":1.1,"population_size":100000,"seed":42},"special_rule":0,"data_type":"i64"}],"row_num":1000000},"mode":"bench","runs":10,"bench":{"metric":"throughput","operation":"insert","throughput_items_per_sec":{"mean":649049287.9350125,"stddev":16059873.037390046,"n":10,"samples":[651112458.1904414,650847598.8279536,651288868.1055374,648245685.1146585,659286177.6697134,655935130.6393204,657408168.2964911,662050848.1533417,649052253.8988569,605265690.4538101]},"wall_time_ms":{"mean":1.5416123999999998,"stddev":0.040304852639188135,"n":10,"samples":[1.535833,1.536458,1.535417,1.542625,1.516792,1.524541,1.521125,1.510458,1.540708,1.652167]},"memory_bytes":16384},"source":"cli","timestamp":"2026-08-19T21:06:41.288130Z"}
```

#### Explanaition

This example is testing HyperLogLog insertion throughput.
HyperLogLog instance comes from `asap_sketchlib`, with configuration `lg_k=14` (in short, register size is `2^14`).
Input data has zipf distribution.
Warmup the benchmark with 3 runs, and benchmark is run for 10 times.
Data is in field `throughput_items_per_sec`.

### Example: Hydra merge throughput

```sh
./target/release/approxbench sketchbench \
    --variant hydra-cms --library lib \
    --spec configs/datagen/hydra_columns.yaml --dtype i64\
    --config "rows=3 cols=1024 cell_rows=3 cell_cols=1024" \
    --operations insert,merge --metrics throughput,cpu,memory \
    --merge-shards 8 --runs 5 --warmup-runs 2 --flat
approxbench: hydra-cms/lib config={"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3} runs=5 warmup=2
{"schema_version":4,"sketch":"hydra-cms","impl":"lib","language":"rust","mode":"bench","runs":5,"source":"cli","sketch_config":{"algorithm":"hydra-cms","params":{"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3}},"workload":{"column_num":3,"column_label":["key1","key2","value"],"column_spec":[{"distribution":{"kind":"uniform","lower_bound":0.0,"upper_bound":200.0,"seed":1},"special_rule":0,"data_type":"string"},{"distribution":{"kind":"zipf","skewness":1.1,"population_size":50,"seed":2},"special_rule":0,"data_type":"string"},{"distribution":{"kind":"zipf","skewness":1.2,"population_size":1000,"seed":3},"special_rule":0,"data_type":"i64"}],"row_num":200000},"memory_bytes":38437088,"heap_bytes_net":null,"heap_bytes_peak":null,"insert_timestamp":"2026-08-20T04:10:29.932751Z","insert_throughput_items_per_sec":{"mean":3144342.5644713533,"stddev":86587.66744001806,"n":5,"samples":[3061671.618818454,3206185.7840062855,3216220.454673226,3199008.307424698,3038626.657434103]},"insert_latency_ns":null,"insert_cpu_time_ms":{"user_ms":{"mean":57.973800000000004,"stddev":1.0205582785906928,"n":5,"samples":[57.318,57.56,57.344,57.895,59.752]},"sys_ms":{"mean":5.2842,"stddev":0.7191360789169184,"n":5,"samples":[6.066,4.821,4.842,4.625,6.067]}},"insert_wall_time_ms":{"mean":63.6453168,"stddev":1.7710492566904223,"n":5,"samples":[65.323792,62.379417,62.184792,62.519375,65.819208]},"insert_rss_peak_kb":null,"insert_heap_allocated_kb":2276888,"query_timestamp":null,"query_throughput_items_per_sec":null,"query_latency_ns":null,"query_accuracy":null,"query_cpu_time_ms":null,"query_wall_time_ms":null,"query_rss_peak_kb":null,"query_heap_allocated_kb":null,"merge_timestamp":"2026-08-20T04:10:29.932972Z","merge_folds_per_sec":{"mean":249.32326079031935,"stddev":54.1908216627536,"n":5,"samples":[156.9734021127992,250.75113397722643,262.8807729415769,292.45568774088423,283.55530717911]},"merge_shards":8,"merge_supported":true,"merge_cpu_time_ms":{"user_ms":{"mean":13.682800000000002,"stddev":1.8371292006824134,"n":5,"samples":[16.892,13.565,12.683,12.65,12.624]},"sys_ms":{"mean":15.867,"stddev":6.73111517506572,"n":5,"samples":[27.69,14.351,13.945,11.286,12.063]}},"merge_wall_time_ms":{"mean":29.551900000000003,"stddev":8.554153538622835,"n":5,"samples":[44.593542,27.916125,26.628041,23.93525,24.686542]},"merge_rss_peak_kb":null,"merge_heap_allocated_kb":1204849,"prepare_timestamp":null,"prepare_cpu_time_ms":null,"prepare_wall_time_ms":null,"prepare_rss_peak_kb":null,"prepare_heap_allocated_kb":null}
```

#### Explanation

This example is testing Hydra-over-Count-Min insert throughput and merge cost.
Hydra instance comes from `asap_sketchlib`, with configuration `rows=3 cols=1024` (the outer sketch) and `cell_rows=3 cell_cols=1024` (the inner Count-Min sketch), which takes about 38.4 MB.
Input data is a 200k-row, 3-column table: two label columns (uniform, then zipf) and an i64 value column (zipf)
Warmup the benchmark with 2 runs, and benchmark is run for 5 times.
`merge` folds 8 shards into one.
Data is in field `insert_throughput_items_per_sec` and `merge_time_ms` / `merge_folds_per_sec` (merge/sec).

add one line description for each field existing
also pretty-print (JSONL) can be helpful

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
