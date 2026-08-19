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

## To be continued

Many detail missing at this moment
