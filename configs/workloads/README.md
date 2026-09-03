# External workload specs

These specs select one bounded workload from the external trace trees used by
the metrics-observability benchmark. They contain logical paths only; pass the
root of the data tree with `--data-root`.

For example:

```sh
approxbench sketchbench \
  --workload-spec configs/workloads/google/task-usage-cpu.yaml \
  --data-root ../../benchmarks/metrics_observability/data \
  --variant cms --library oxide --dtype f64 \
  --metrics throughput --operations insert
```

The loader materializes one selected file and one half-open time window before
measurement. Point records use `[start, end)` membership; interval records are
included only when both endpoints are inside the selected window. Malformed
rows, missing selected fields, NaN, and infinities fail the invocation.

For complete one-minute tumbling windows, use the standard-library sweep
driver. It invokes one benchmark process per window and appends JSONL records:

\`\`\`sh
python3 scripts/run_external_sweep.py \
  --workload-spec configs/workloads/google/task-usage-cpu.yaml \
  --data-root ../../benchmarks/metrics_observability/data \
  --start 1970-01-16T04:52:15Z --end 1970-01-16T05:52:15Z \
  -- --variant hll --library oxide --config lg_k=12 --dtype f64 \
     --metrics throughput --operations insert
\`\`\`

Keyed external workloads are intentionally separate from this first slice and
are tracked in issue #122.
