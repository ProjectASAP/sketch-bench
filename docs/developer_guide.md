# Developer Guidance

The target audience is who wants to add functionality to the benchmark.

## Pre-requirement

Rust toolchain is necessary.
Modification will require a re-compile of benchmark.
To get correct number, `cargo build -release` is necessary.

## Wire New Sketch

To add a new sketch, either from a new library or just a new algorithm / variance, will need to touch `wrappers`.

### Location

`wrappers` refers to this [folder](../sketch-bench/src/wrappers/) under `sketch-bench` crate.
Currently, the directory is grouped by algorithm name.
Add algorithm name as necessary.

For each operation of that sketch, make a closure for that operation.
Other existing wrappers can be used as examples.

### Registry

To register the wrapper (such that `cli` is aware of it), add an entry to [registry](../sketch-bench/src/registry.rs).

## Ground Truth

Accuracy comparison needs ground-truth to compare against.
All ground-truth are defined under [accuracy](../aqpbm-core/src/accuracy/).
Ground-truth is categorized by functionality.

### Adjustment of existing functionality

Some functionalities (like `quantile`) require a parameter.
At this moment, there is no place for user to specify the parameter.
Thus, the ground-truth logic is hard-coded.
For example, `quantile` ground truth means 101 quereis from `p0` `p1` to `p100`.
If user need to know `quantile` accuracy under different meaning (i.e., `p50` only), user need to re-write the ground-truth file and adjust the wrapper accordingly.

## TO-ADD: data generation ( as shared functionality )

## To be continued

Many detail missing at this moment

## developer guide

What a developer need.
interface or folder to find
such architechture as the first section in developer guide

after this common knowledge, based on functionality, user need to add new func1 func2 ec

related to architechture in the first section

need more detail about interface to add new functionality / component

share a list of task that a developer will want to do:
i.e.: add a new sketch
i.e.: add a new ground-truth
i.e.: add new data generation method

developer may want to know what output they will receive and how to interpret that output
output of benchmark

e.g.: if there is a python script to process the jsonl result to a plot (unnecessary) (lower priority)
e.g.: if the result really makes sense
e.g.: how to verify the output is reasonale (like which part I should look into to check if the output makes sense) (higher priority)
