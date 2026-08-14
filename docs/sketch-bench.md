# `sketch-bench` Design

`sketch-bench` provides a group of wrappers over sketch instance being tested.

This crate has two major functionalities:

- a `registry` that let user (through `aqpbm-cli`) knows what sketches are included, and what benchmarks are related to that sketch
- a thin wrapper that ships the sketch instance to the benchmark runtime

## Major Components

### Registry

The `registry` is basically a list registere sketch.

Consider "CountMin Sketch" as an example.
There are multiple library that implements "CountMin Sketch".
For each of those implementations, there can be slightly difference in terms of API and configuration.
Also, it's possible that some implementation provides some "distinct" functionalities.
In short, to use the same sketch from different library, user needs a specific way to use it (instead a common way to use all implementations).

The above exmple introduce the need of a `registry`.
`registry` contains the information of where the sketch is from:

- the algorithm name
- which library it is from
- what instance the sketch is
  - in `asap_sketchlib`, "CountMin Sketch" can depends on different data structure; it's easier to register them differently
- what "capability" this sketch is supposed to have
  - take "CountMin Sketch" as an example, common usage includes "frequency estimation" and "heavy hitters"
- what operations and metrics can be supported

#### Two notes for the registry

- benchmark should add minimal overhead to benchmark targets, thus compiled-time fixed setup is more favorable then runtime choice
  - it's okay to register many targets that are never tested
  - run-time dynamic dispatch (`dyn` Trait) should be avoided
- for simplicity, if a sketch instance has more than one capability to compare against, the sketch will be registered multiple time

### Wrapper

Wrapper is the major component of `sketch-bench` crate.
Wrapper serves the functionalities about how a sketch can be used.

For example, sketch query function may take different arguments from different implementations, even for the same algorithm.
This is the wrapper's job to provide how a sketch is used.

#### Wrapper can be a closure

To achieve the functionality that a wrapper can pass a sketch around, a closure can be a good choice.
Inside the closure, how sketch is used is accomodated.
`aqpbm-cli` can pass this closure around to `aqpbm-core` alongside the data received from `aqpbm-datagen` for benchmark result.

#### Wrapper shoud not be a trait

**Reasoning**:

A trait restricts the input of a function to be the same across different implementation.
However, it's natural that different sketch functions have different input.
It can be inferred that the function needs certain operations to process the common input to a format that the sketch can take.
The process of common input is an overhead that cannot be avoided.
In time-related benchmark, this is bad.
Thus, a wrapper should not be a trait.

## Input

Requirement that `aqpbm-cli` received from command line.
The requirement is passed as a string contains the following:

- algorithm name
- implementation (the library that contains this implementation)
- configuration and parameters
- metrics to benchmark
  - not all metrics are supported by all sketch
- operations to benchmark
  - for example, `prepare_for_query` as an operation exists for KLL but missing for most sketches

The requirement is passed as `enum` in `aqpbm-core`.
The `registry` will register different `enum` instance for sketch instance to describe available metrics, operations and capabilities.

## Output

An `Result` that contains a `closure option` or error message.
Error message will states what is failed.

## Open questions

- **What the sampling and universal algorithms are scored under.** No capability in core's list fits a moment estimate or a heavy-hitter set.
  Scoring them means adding a statistic to core, a wider change than adding a row here.

- **How wide the compiled-in matrix table should be.** A CMS row whose shape is baked at compile time runs only at the shapes some build instantiated.
  The table therefore decides what is measurable without recompiling.
  Widening it is nearly free in compile time but costs rlib size.
  The ceiling comes from an unoptimised build materialising the largest array on a test thread's stack, so it is a property of the build alone.

- **Whether the exact baselines should take a config at all.** They compute the exact answer, so no value changes what they return, and they parse the config only to refuse what their siblings refuse.
  Accepting anything would let a config that fails on every sketch row still produce a baseline number.
