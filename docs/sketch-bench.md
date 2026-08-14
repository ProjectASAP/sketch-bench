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

#### Two notes for the registry:

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

#### Wrapper shoud not be a trait

**Reasoning**:

A trait restricts the input of a function to be the same across different implementation.
However, it's natural that different sketch functions have different input.
It can be inferred that the function needs certain operations to process the common input to a format that the sketch can take.
The process of common input is an overhead that cannot be avoided.
In time-related benchmark, this is bad.
Thus, a wrapper should not be a trait.

## 1. Purpose

With all the wheels in other crates, the work of really comparing something is done in this crate.

Five things cannot be written without knowing an algorithm by name, and all five are this crate's:

- **The wrappers.** A **wrapper** is a newtype over one third-party sketch, implementing the framework's traits on that sketch's behalf.
  A wrapper claims exactly what the library beneath it provides: `merge` and `prepare` are stated where the library has them, and left unstated otherwise.
  An answer passes through as the library returns it, and its type follows the library's own.
  Where the library's documentation prescribes a treatment the wrapper applies it, and where it says nothing the value is reported unchanged.
- **The catalog.** The list itself, binding each `(algorithm, impl)` pair to the type implementing it.
- **The construction parameters.** `hll` takes an `lg_k` while `cms` takes a `rows` and a `cols`, and so on for every family.
- **The choice of comparator for each row.** Core ships the comparators, and one capability can carry several.
  A KLL states its error as a rank error and a DDSketch as a relative error.
- **The exact baselines.** The implementation that answers a capability exactly, and which sketch it stands beside.

Consider this crate to be something that wraps around existing functionalities and can be used by `aqpbm-cli`.

## 2. Inputs

- **The row selector, as two strings.** An algorithm name and an impl name.

- **The numeric width.** A **numeric width** is the item type an ordered quantile row is built at, `i64` or `f64`, chosen by the frontend.
  Every other row takes its item type off `Accumulator::Item`, so this is the one item-type choice a caller makes.
  An unsupported width is refused from the catalog before a single item is drawn.

- **One construction parameter point per family.**
  Omitting a configuration gives a parameterless point, which every row refuses by name.
  A row builds at exactly the point it is given or refuses it, so the values are never clamped to a library's range, rounded to a shape it prefers, or dropped because it has no knob for them.

- **The forwarded configuration.** `WorkloadSpec` and `BenchConfig` arrive from the frontend and reach core unread, with one exception.
  A comparator scoring a prefix of a ranking reads the row's own `k` out of the `ParamSet`.

- **The wrapped libraries.** Four, each present for a stated reason.
  `sketch_oxide` is a Rust sketch library covering cardinality, frequency, quantile, universal and elastic sketches.
  `datasketches` binds Apache DataSketches, the reference implementation.
  `asap_sketchlib` is the project's own library, supplying several structural variants per family: the variations it exposes are the trade-offs the benchmark exists to price, so each is its own algorithm.
  `polars` is a DataFrame engine.
  A polars row can serve as the exact baseline for a capability, and it is also timed and scored like every other row.

- **Build-time features.** Three cargo features, each forwarding to the `aqpbm-core` feature of the same name.

## 3. Outputs

- **Reports, one per square.** One report for each square the request selected, and a square nothing here measures is refused by name.

- **The catalog listing.** One line per row: algorithm, impl, and a description naming the concrete type wrapped, grouped by family.

```
hll-hip                   lib     asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}
cms-fastpath-fixedmatrix  lib     asap CMS, FixedMatrix (shape baked at compile time), FastPath
kll-cdf                   polars  polars exact: 101-point quantile grid
```

- **Answers about a row, before it runs.** Whether an algorithm exists, which family it belongs to, and whether a row is scored for accuracy.
  Whether a row takes multi-column input is known too, off the row's item type, but no caller has needed to ask: a grouped row consumes labelled records, and one handed a single-column spec refuses by name at construction.

- **Errors.** Every refusal names the offending value.
  - an unknown algorithm
  - an unknown impl within a known algorithm
  - a width a row cannot be built at
  - a parameter key the family does not have
  - a shape a fixed-size row cannot serve
  - a value the wrapped library would silently clamp, round or ignore

  The last one is the rule the others are instances of: a row builds at exactly the parameters it was given, or it refuses them and says what bound it has.
  That is how a row discharges core's guarantee that the config in a record is the config that ran.

## 4. Interfaces

### 4.1 The catalog

```rust
pub enum Numeric { I64, F64 }

// Which rows exist. `list` is for a person; the predicates are for a frontend
// checking its arguments before it does any work.
pub fn list() -> Vec<String>;
pub fn algorithm_exists(algorithm: &str) -> bool;
pub fn family_of(algorithm: &str) -> Option<&'static str>;                // None: unknown row
pub fn scores_accuracy(algorithm: &str, impl_name: &str) -> Option<bool>;  // None: unknown row

// One parameter point for an algorithm, from the CLI's `--config` string.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet>;

// Resolve a row and run one cell against it: one report per square.
// A named comparator must be one the row admits; omitted takes the row's default.
pub fn run(algorithm: &str, impl_name: &str,
           cfg: &BenchConfig, spec: &WorkloadSpec, params: &ParamSet,
           width: Numeric, comparator: Option<&str>) -> Result<Vec<BenchReport>>;
```

### 4.2 The two name axes

A row has two names, and each answers one question.

- **`impl` is the library**, one of `oxide`, `datasketches`, `lib`, `polars`.
  Grouping on it asks which library implements a thing best, and it can only answer that if nothing else is in the column.

- **`algorithm` is what is being implemented**, structural variant included.
  `hll` and `hll-hip` differ by estimator, `kll-percall` and `kll-cdf` by query strategy, `cms-fastpath-vector2d` and `cms-regularpath-vector2d` by hash strategy.
  Each pair gives different answers from the same state, so each is two questions and not two answers to one.

A **family** groups the variants back together: it is the set of algorithms sharing one parameter vocabulary, and it is what a cross-library comparison is taken over.
The family is derived, never written, since one params struct is one family.
It reaches the record as its own field, so a reader picks the axis instead of parsing names.

### 4.3 Sketch and baseline

An **exact baseline** is a row computing the exact answer: it declares the same capability and runs through identical machinery.
Some sketches answer more than one statistic, so a sketch is paired with a baseline once per capability, and this crate names the pairing.
This crate also provides the baseline implementations, built on ordinary data structures such as a hash map.
A baseline is timed like every other row, and the exact answer a comparator scores against is computed inside core.

### 4.4 The parameter schema

A family's parameters are a struct that denies unknown fields, plus one macro invocation binding it to a family name and a canonical point.
A misspelled key becomes an error naming it, because a run at parameters other than those requested is worse than a refusal.
The same rule covers a value the wrapped library would accept and quietly change: the wrapper checks the bound and refuses, or checks the built structure and refuses, so the parameters in the record are the parameters that ran.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmsParams {
    pub rows: usize,   // one counter row per hash function
    pub cols: usize,   // counters per row
}
// the family this struct coins, then one point every row in it can build at
sketch_params!(CmsParams, "cms", CmsParams { rows: 3, cols: 1024 });
```

The `FAMILY` const in the params struct is the one place a family's name is written, and a variant row writes its own algorithm name once, in its `BenchImpl`.

## 5. Open questions

- **What the sampling and universal algorithms are scored under.** No capability in core's list fits a moment estimate or a heavy-hitter set.
  Scoring them means adding a statistic to core, a wider change than adding a row here.

- **How wide the compiled-in matrix table should be.** A CMS row whose shape is baked at compile time runs only at the shapes some build instantiated.
  The table therefore decides what is measurable without recompiling.
  Widening it is nearly free in compile time but costs rlib size.
  The ceiling comes from an unoptimised build materialising the largest array on a test thread's stack, so it is a property of the build alone.

- **Whether the exact baselines should take a config at all.** They compute the exact answer, so no value changes what they return, and they parse the config only to refuse what their siblings refuse.
  Accepting anything would let a config that fails on every sketch row still produce a baseline number.
