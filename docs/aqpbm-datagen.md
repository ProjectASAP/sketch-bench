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
}

enum DataDistribution {
    Zipf(ZipfParameter),
    Uniform(UniformParameter),
    Normal(NormalParameter),
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
  - In stream mode the caller sets a chunk size, and generation alternates with consumption.
- **Stays in Memory:** Generated data stays in memory and will be consumed through transfer of ownership

## Open questions

- **Reproducing a recorded trace with a synthetic description.** Matching its frequency distribution is within reach, and matching its arrival order and burstiness needs more detailed spec.
- The other option approximates a trace by its value distribution alone, at the cost of saying so wherever such a result is reported.
