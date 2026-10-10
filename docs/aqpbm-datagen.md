# `aqpbm-datagen` Design

`aqpbm-datagen` generates data based on the requirement it received.
Generated data can be a single column or a table, as requested.
`aqpbm-datagen` depends on no other crate in this workspace.

## Input

`aqpbm-datagen` takes argument about "description" of data shape.
The description includes:

- column number
- column label
- data range of one column
- column data distribution
  - different distribution takes different parameters
- special rule
  - increment only, integer only, etc.

The input requirement can be defined as this:

```rust
pub struct TableDescription {
    pub column_num: u32,
    pub column_label: Vec<String>,
    pub column_spec: Vec<ColumnSpec>,
    pub column_connected: Vec<Vec<String>>, // column label describes what columns are related
    pub row_num: u64,
}

pub struct ColumnSpec {
    pub distribution: DataDistribution,
    pub shift: Option<f64>, // if the data should not range from 0 or normal range of that distribution
    pub cardinality: Option<u64>, // if the data needs a range, the cardinality talks about it
    pub special_rule: u32, // bit mask about special_rule; 0 means no special rules
    pub data_type: String, // defines which case of enum ColumnData should be chosen from
    pub string: Option<StringOpts>, // if this column is about string data, description about how string looks like
    pub child_of: Option<usize>, // index of an earlier string column this one nests under; see child_of below
    pub fan_out: Option<u64>, // children per parent value; required with child_of
    pub scale_by: Option<usize>, // index of an earlier string column whose values scale this f64 column; see scale_by below
    pub scale_range: Option<[f64; 2]>, // [lo, hi] the per-label factors are log-uniform in; required with scale_by
}

enum DataDistribution {
    Zipf(ZipfParameter),
    Uniform(UniformParameter),
    Normal(NormalParameter),
    Pareto(ParetoParameter),
}

struct ZipfParameter {
    pub skewness: f64,
    pub population_size: u64,
    pub seed: u64,
}

struct UniformParameter {
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub seed: u64,
}

struct NormalParameter {
    pub mean: f64,
    pub standard_deviation: f64,
    pub seed: u64,
}

struct ParetoParameter {
    pub alpha: f64, // shape
    pub scale: f64, // minimum value
    pub seed: u64,
}
```

`special_rule` bit map contains the following at this moment:

- **0b0**: no rules
- **0b0001**: monotonically increase

### column_connected

Sometimes, some data are related across columns.
For example, source-ip and destination-ip are together to form a heavy flow.
Thus, the field `column_connected` is used to describe such relation.
`column_connected` use label to describe which columns are related to each other.
The requirement is: columns related to each other needs to have identical distribution and distribution parameter.
They will be generated once, and processed to meet the need.

### child_of

Labels are often hierarchical: a service lives in one region, an endpoint in one service.
A column with `child_of: <index>` and `fan_out: f` nests under the earlier column at that index:
its value is the parent row's value, a `.`, and a child index in `[0, f)`.
The child index is drawn from the column's own `distribution` (its 0-based rank in the domain), so the usual skew and seed options pick it,
independently of the parent: every parent value shares one child distribution and has at most `f` distinct children.

```yaml
- data_type: string
  child_of: 0        # nests under column 0, e.g. "abcd" -> "abcd.7"
  fan_out: 25
  distribution: {kind: zipf, skewness: 1.1, population_size: 25, seed: 2}
```

Each of the following is a description error:

- `child_of` without `fan_out`, or `fan_out` without `child_of`
- `child_of` naming the column itself, a later column, or no column (a parent is generated first)
- a parent that is not `data_type: string`, or a child that is not
- `fan_out` of 0, or one that disagrees with the distribution's domain size (an unbounded distribution has none)
- `cardinality` or a `string:` block on a child column (its distinct count is the parent's times `fan_out`, and it is not rendered from a rank)

`configs/datagen/hydra_hier.yaml` uses it for a region → service → endpoint hierarchy.

### scale_by

A value column drawn independently of the labels gives every group the same distribution.
A `data_type: f64` column with `scale_by: <index>` and `scale_range: [lo, hi]` multiplies each row's value
by a factor fixed per distinct value of the earlier string column at that index.
The factor is `lo·(hi/lo)^u`, log-uniform in `[lo, hi]`, with `u ∈ [0, 1)` a hash of the label value and the column's seed:
the same label always gets the same factor, whatever the row count, and a new seed gives new factors.
The draws themselves are unchanged, so a Pareto column keeps its tail index and each group's scale moves.

```yaml
- data_type: f64
  scale_by: 1        # each service's latencies × its own factor in [1, 10]
  scale_range: [1.0, 10.0]
  distribution: {kind: pareto, alpha: 2.0, scale: 1000.0, seed: 5}
```

Each of the following is a description error:

- `scale_by` without `scale_range`, or `scale_range` without `scale_by`
- `scale_by` naming the column itself, a later column, or no column, or a column that is not `data_type: string`
- a scaled column that is not `f64` (an integer column would round the scaled value)
- a `scale_range` without `0 < lo <= hi`

`configs/datagen/hydra_http_latency.yaml` scales latency by service.

## Output

The output is a struct containing data:

```rust
pub struct GeneratedTable {
    pub column_num: u32,
    pub column_title: Vec<String>,
    pub data: Vec<ColumnData>,
    pub row_num: u64,
}
```

and the `ColumnData` is defined as:

```rust
enum ColumnData {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    Unsigned64(Vec<u64>),
    String(Vec<String>),
}
```

This output can satisfies both single column generation (used by sketch insertion) and table generation (query processing).
Each column can be passed to downstream with transfer of ownership and peel out the data directly.

To make sure `column_num` and actual column count in `data`, helper function to check is required.

To make sure each column has the same length, helper function to check is required.

## What is promised

- **Determinism:** The same description always produces the same column.
- **Generation never runs inside a timed region:**
  - By default the whole column exists before timing starts.
- **Stays in Memory:** Generated data stays in memory and will be consumed through transfer of ownership

## Open questions

- **Reproducing a recorded trace with a synthetic description.** Matching its frequency distribution is within reach, and matching its arrival order and burstiness needs more detailed spec.
- The other option approximates a trace by its value distribution alone, at the cost of saying so wherever such a result is reported.
