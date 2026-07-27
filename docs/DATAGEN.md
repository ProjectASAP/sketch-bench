# aqpbm-datagen: current and future

`aqpbm-datagen` generates synthetic benchmark workloads.
`aqpbm-datagen` does the following things:

- draw distinct keys from a key space under uniform or zipf distribution
- draw category ids from a small finite domain under uniform, zipf, or explicit per-id weights
- build a monotonically non-decreasing series by summing non-negative gaps where each gap can be constant, geometric, exponential, or poisson

## Output

Generated values go to one of two destinations: an in-memory `Vec<T>`, or a raw little-endian `.bin` file with a `.meta.json` sidecar (`--no-meta` skips it).
Output data types are `i64`, `u64`, `f64`, and `string`.
In-memory generation can pass data of `i64`, `f64`, and `string` to benchmark.

### Current limitations

Only `i64`/`u64`/`f64` can go to a file.
Read from `.bin` only supports `i64` currently.

## Spec files (`--spec`)

User can use a spec file to describe what data they want.
Example can be found as follows:

```yaml
shape: categorical
categories: [0, 1, 2, 3, 4, 5, 6, 7]
dist: { kind: explicit, weights: [40, 25, 15, 8, 5, 3, 2, 2] }
size: 1000000
seed: 42
```

## Future

- **Multi-column generation, with correlation between columns.**
Each generator produces one column today.
Independent-column is one case.
Correlated-column is a different case.
One column of source addresses and another column of destination addresses can be related to each other.
This represents a busy source talking to a small set of destinations, not to a uniform sample of them.
The accuracy question is whether correlation moves sketch error away from what independent columns predict.
The throughput question is whether correlation affects insertion and query performance.

- **Synthesize a stand-in for a real capture.**
Reading in `.pcap`, taking each IPv4 source address as a key, is one option for input data.
Synthesizing that data is another option.
The goal is synthetic data faithful enough to take `.pcap`'s place.
How faithful that has to be is still an open question.
