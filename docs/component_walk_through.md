# Component Walk Through

This is the document for expected components and expected functionality of each component.

**Target audience:** users who happen to be interested in the general workflow of the benchmark
**Potential Developers:** documentation is on the way

## Overview

The repo is structured as a cargo workspace with following crates:

```rust
members = [
    "aqpbm-datagen",
    "aqpbm-core",
    "sketch-bench",
    "aqpbm-cli",
]
```

Notice: the `sketch-runtime` is a placeholder, which doesn't contain anything essential or meaningful or executable or understandable at this moment.

### Idea to keep in mind

The benchmark framework should add minimal overhead as well as not optimize away what is being benchmarked.

### Graphic View of structure

```mermaid
flowchart TB
commandline_input(["`**Command from user**`"]):::io

output(["`**Output**
STDOUT
JSON/JSONL file on disk`"]):::io

cli ---->|"8: result"| output

commandline_input --->|"1: benchmark setup"| cli

subgraph  Main [Benchmark Framework]

spacer["Benchmark Framework"]:::invisible

style Main fill:#e6f2ff,stroke:#333,stroke-width:2px,font-size:15px,font-weight:bold

cli["`**aqpbm-cli**
- cli parameter parsing
- function argument passing
- benchmark result report`"]

sketch-bench["`**sketch-bench**
- sketch registry
- registered sketch wrapper`"]

aqpbm-datagen["`**aqpbm-datagen**
- in-memory data genertion`"]

aqpbm-core["`**aqpbm-core**
shared functionalities
- timing
- ground truth comparator
- benchmark runner
- result report`"]

sketch-bench --->|"3: sketch wrapper"| cli
cli --->|"2: feasibility check"| sketch-bench

cli --->|"4: data generation requirement"| aqpbm-datagen
aqpbm-datagen --->|"5:generated data"| cli

cli --->|"6: necessary components"| aqpbm-core
aqpbm-core --->|"7: benchmark result"| cli

class aqp-bench,runtime future

spacer~~~cli

end

classDef future stroke-dasharray: 5 5
classDef invisible fill:none,stroke:none,color:transparent
classDef io fill:#feeba8
```
