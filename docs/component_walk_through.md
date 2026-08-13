# Component Walk Through

This is the document for expected components and expected functionality of each component.

## Overview

The repo is structured as a cargo workspace with following crates:

```rust
members = [
    "aqpbm-datagen",
    "aqpbm-core",
    "sketch-bench",
    "sketch-runtime",
    "aqpbm-cli",
]
```

Notice: the `sketch-runtime` is a placeholder, which doesn't contain anything essential or meaningful or executable or understandable at this moment.

### Idea to keep in mind

The benchmark framework should add minimal overhead as well as not optimize away what is being benchmarked.

### Graphic View of structure

```mermaid
%%{init: {'theme': 'default', 'themeVariables': { 'fontSize': '24px' }}}%%
flowchart TB
commandline_input(["`**Command from user**
- what input data needs to be synthesized
- what instance (like, a sketch implementation) to test
- what metric to include in output`"])

commandline_input -->|"Request from user about what benchmark to execute"| cli

subgraph Benchmark Components [Main Benchmark Components]
cli["`**aqpbm-cli**
- command line parameter parsing
- calling corresponding data generation function
- pass generated data, benchmark instance, metrics to test, ground truth to aqpbm-core
- return benchmark result to user`"]

sketch-bench["`**sketch-bench**
- sketch instance wrapper
- registry about what sketch from which library is included and can be compared to what ground truth`"]

aqpbm-core["`**aqpbm-core**
Core functionalities shared by different benchmarks for different targets
- timing of one operation
- ground truth of one capability
- runner for each instance to be benchmarked
- calculation of result to be report to user`"]

aqpbm-datagen["`**aqpbm-datagen**
- generate data in memory following the requests recieved by cli
- generated data will be provided in-memory, where cli decides who will have the data`"]

cli -->|"data generation requirement"| aqpbm-datagen
aqpbm-datagen -->|"generated data"| cli

cli -->|"check if user request is doable"| sketch-bench
cli -->|"generated data, wrapper of sketch instance, metric requirement"| aqpbm-core
aqpbm-core -->|"benchmark result"| cli

class aqp-bench,runtime future

end

output(["`**Output**
- STDOUT or JSON/JSONL file on disk`"])

cli --> output

classDef future stroke-dasharray: 5 5
```

## aqpbm-core

Check [aqpbm-core](./aqpbm-core.md) for detail.

## aqpbm-datagen

Check [aqpbm-datagen](./aqpbm-datagen.md) for detail.

## aqpbm-cli

Check [aqpbm-cli](./aqpbm-cli.md) for detail.

A reference of expected cli parameters is in [aqpbm-cli-reference.md](./aqpbm-cli-reference.md).

## sketch-bench

Check [sketch-bench](./sketch-bench.md) for detail.

## sketch-runtime

No real implementation at this moment.
The directory exist, but no meaningful contents.

## aqp-bench (placeholder)

This part is postponed to V2, even name may be changed to "xxx-bench" (where xxx means something else).
It is listed here to indicate structure: this is a component parallel to sketch-bench and how it will connect to `aqpbm-cli` and backboned by `aqpbm-core`.

## Other language

This part is not started yet.
It is reasonable to have benchmark in other language.
Yet, not started.
