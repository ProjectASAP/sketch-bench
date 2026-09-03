# `aqpbm-cli` Design

`aqpbm-cli` is the frontend.
It builds the `approxbench` binary.
This is where user interact with the benchmark.

Different crates are orchestrated by the CLI (this crate).

## Input

User specify benchmark requirement by providing command line arguments.
For supported flag, user can check the full list by:

```sh
approxbench --help
```

or to check `sketchbench` related helper message:

```sh
approxbench sketchbench --help
```

If user want to know what sketches are supported, user can do:

```sh
approxbench sketchbench --list-impls
```

### Binding

For CLI to recognize what sketches are registered in `sketch-bench`, CLI maintains a `binding()` about which sketch can be selected.
User input (a raw string) is translated to a function pointer that points to the actual `wrapper` struct.

## Output

User can specify the benchmark result are printed on screen or to a JSONL file.
User specify this by the `--report` flag.
The output file will be a JSONL.
Each line in the JSONL stands for one execution.
Example is:

```sh
approxbench sketchbench \
    --variant cms-fastpath-vector2d --library lib \
    --config 'rows=5 cols=2048' \
    --dataset uniform --size 1000000 --cardinality 100000 \
    --dtype i64 \
    --operations insert,query --metrics throughput \
    --report results.jsonl
```

## Interaction with aqpbm-datagen

User define how input data looks like through command line.
CLI will process that spec and call corresponding functions in `aqpbm-datagen`, and receives the data.

### External workloads

External traces can be selected with `--workload-spec` and `--data-root`:

```sh
approxbench sketchbench \
    --workload-spec configs/workloads/google/task-usage-cpu.yaml \
    --data-root ../../benchmarks/metrics_observability/data \
    --variant hll --library oxide --config 'lg_k=12' --dtype f64 \
    --operations insert --metrics throughput
```

The first adapter slice supports BOOM Arrow, Google Cluster `task_usage`, and
Alibaba microservice metrics. Each invocation loads one half-open window before
measurement; the sweep helper in `scripts/run_external_sweep.py` runs complete
tumbling windows. Rows crossing an interval boundary are excluded, and empty
windows are skipped. Missing selected values, malformed rows, NaN, and infinite
values fail loudly. Keyed external input is tracked separately in issue #122.

## Interaction with sketch-bench

User specify the `operation`, `metrics` and sketch to run in benchmark.
CLI will check against `registry` in `sketch-bench` crate.
Once CLI receives the wrapper of benchmark components, CLI will pass a package containing the wrapper and data received previously to `aqpbm-core` for execution.

## Interaction with `aqpbm-core`

Once CLI receives the data and wrapper based on the specification received by command line, CLI will pass a package containing the wrapper and data to `aqpbm-core` for execution.
`aqpbm-core` will return the (benchmark) execution result back to CLI.
CLI will display the result accordingly.
