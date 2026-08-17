# `aqpbm-core` Design

`aqpbm-core` is the crate for shared functionalities across crate.

Shared functionalities at this moment refers to the following:

- time one operation specified by `aqpbm-cli`
- compare output of one operation to ground-truth specified by `aqpbm-cli`
- functionalities to support above operations
  - for example, what kind of ground-truth can be compared to stays in `aqpbm-core`
  - for example, what kind of metrics can exist stays in `aqpbm-core`

When there are new demand, above functionalities will be extended.
This design doc tracks necessary functionalities as is.

## Input

`aqpbm-core` executes tasks based on request received by `aqpbm-cli`.
`aqpbm-cli` will call corresponding function in `aqpbm-core` to actually perform the benchmark.

## Output

Structured benchmark result will be returned to `aqpbm-cli`.

## Metrics

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

## Operations

Operation is a general group of operations.
There are four operations considered at this moment:

- `insert`: ingest data
- `prepare`: to better serve a query, `prepare` is necessary sometimes
- `query`: answer a request, get a result
- `merge`: if two objects can be combined into one, which is common for sketch

Not all metrics is applicable to all operations.
For example, `accuracy` is only meaningful against `query`.

## Capabilities

One important aspect of `aqpbm-core` is defined capabilities.

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
```

Define capabilities brings one advantage:
different targets can be compared to the same ground-truth without re-writing ground-truth repeatively.

### Ground Truth

As introduced previously, ground-truth is defined based on capabilities.
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

## Open Question

- Ground-truth made assumption about what is tested, and sketch instance needs to do the same thing
  - however, no static check for these two to match
  - static / compile-time check can be helpful
