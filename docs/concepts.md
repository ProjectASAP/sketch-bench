# Concepts

Concepts or terminology used across this project is aggregated here.
Ideally, this document can help reduce potential confusion in other docs.

## Terminologies

This section defines some terminologies adopted across this projects.

### Metrics

Metrics is defined in `aqpbm-core` to describe what can be tested.

Major metrics are following:

- `throughput`: how many item can be processed by an operation in a certain amount of time (per sec)
- `latency`: how long one operation can take
- `accuracy`: how accurate one operation is, comparing to ground-truth

Some other metrics that are helpful includes:

- memory usage
  - especially heap usage tracking
- cpu time
  - at this moment, the same with latency

### Operations

Operation is a general group of operations.
There are four operations considered at this moment:

- `insert`: ingest data
- `prepare`: to better serve a query, `prepare` is necessary sometimes
- `query`: answer a request, get a result
- `merge`: if two objects can be combined into one, which is common for sketch

Not all metrics is applicable to all operations.
For example, `accuracy` is only meaningful against `query`.

### Capabilities

Capability is defined as the general, high-level target of an operation.
Current capabilites includes:

```text
cardinality         how many distinct items are there
frequency           how many times did this key occur
quantile            which value sits at this fraction
top-k               which k keys are heaviest, and how heavy
heavy-hitter        heavy items and how heavy
subpop-cardinality  cardinality, within the records carrying a set of labels
subpop-frequency    frequency, within the records carrying a set of labels
subpop-quantile     quantile, within the records carrying a set of labels
keyed-cardinality   how many distinct items for a specific key
keyed-entropy       entropy of values for a specific key
keyed-l1norm        L1-norm of values for a specific key
keyed-l2norm        L2-norm of values for a specific key
```

Capabilities are defined in `aqpbm-core`.

### Ground Truth

Ground-truth is defined based on capabilities.
Ground-truth will take necessary arguments.
For example, `k` value for `top-k`.
For example, input data that a benchmark target will receive.

However, some capabilities is hard to be defined by parameter.
For example, heavy-hitter is a capability doesn't take specific argument.
It's hard to describe how heavy is the bound for heavy hitter.
Thus, this is something adjustable.
For accuracy comparison, user provide query closure for heavy hitter capabilities, that is supposed to match.
It's user's job to ensure these operations are doing the same job.
If the operation doesn't match, the accuracy will be inaccurate.

Ground-truth is calculated by exact data structure.
For example, `cardinality` is calculated based on hash-set.
For example, `frequency` and `heavy-hitter` are both supported by a hash-map.

### SketchId

`SketchId` is a descriptive entry of one sketch instance.
The struct definition is defined as follow:

```rust
pub struct SketchId {
    pub family: &'static str,
    pub algorithm: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
    pub capability: Capability,
    pub comparator: Option<&'static str>,
    pub operations: OperationMask,
    pub metrics: MetricsMask,
}
```

**rename soon:** `family` -> `algorithm` , which is the general name of sketch algorithm; example including: `cms` `hll`

- whether a full name is essential (i.e., `CountMin Sketch` vs `cms`) is undecided yet

**rename soon:** `algorithm` -> `variance`, which is the variance of one algorihtm

- example: HyperLogLog vs HyperLogLog-MLE
  - HyperLogLog: the classic algorithm
  - HyperLogLog-MLE: adopted in data-fusion and Redis, identical to classical HyperLogLog except a special query algorithm to achieve cardinality estimation
  - reason as a `variance`: both claim themselves to be "HyperLogLog"
- another example: CountMin vs CountMin-FixedMatrix-FastPath
  - FixedMatrix: the matrix is bounded in a `Box<[T]>` such that the size is pre-determined at compile time; thus there is no row/col configuration possible
  - FastPath: hash reuse optimization such that hash elment once and insert to multiple rows (instead of independent hash at each row, in a regular CountMin sketch algorithm)
  - reason as a `variance`: the same algorithm with some optimization techniques

**rename soon:** `impl_name` -> `library`, where this implementation is from (i.e., `asap_sketchlib`, `sketch_oxide`, `datasketches`)

`description`: one-line description about what this instance is

`capability`: what this sketch is used for, with [capability](./concepts.md#capabilities) defined above

`comparator`: what this sketch should be compared against

- sometimes a sketch can have multiple capabilities, so this sketch can be registered multiple times, with different comparators

`operations`: what operation this sketch provides

`metrics`: what metrics can this sketch be benchmarked against
