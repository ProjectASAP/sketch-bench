# `sketch-bench` Design

`sketch-bench` is the domain bundle: the crate that ships rows and the catalog naming them.

## 1. Purpose

The crate answers one question: given two strings, which sketch is that?

Four things cannot be written without knowing an algorithm by name, and all four are this crate's:

- **The wrappers.** A **wrapper** is a newtype over one third-party sketch, implementing the framework's traits on that sketch's behalf.
  A wrapper claims exactly what the library beneath it provides: `merge` and `prepare` are stated where the library has them, and left unstated otherwise.
- **The catalog.** The list itself, binding each `(algorithm, impl)` pair to the type implementing it.
- **The construction parameters.** `hll` takes an `lg_k` while `cms` takes a `rows` and a `cols`, and no generic layer can say that.
- **The choice of comparator for each row.** A KLL states its error as a rank error and a DDSketch as a relative error.

Everything that spells an algorithm name or names a third-party sketch library is here.
Benchmarking a sketch of your own does not go through this crate, which exists for the CLI.

## 2. Inputs

- **The row selector, as two strings.** An algorithm name and an impl name.

- **The numeric width.** A **numeric width** is the item type an ordered quantile row is built at, `i64` or `f64`, chosen by the frontend.
  Every other row takes its item type off `Accumulator::Item`, so this is the one item-type choice a caller makes.
  An unsupported width is refused from the catalog before a single item is drawn.

- **One construction parameter point per family.**
  Omitting a configuration gives a parameterless point, which every row refuses by name.
  A row builds at exactly the point it is given or refuses it, so the values are never clamped to a library's range, rounded to a shape it prefers, or dropped because it has no knob for them.

- **The pass-through configuration.** `WorkloadSpec`, `BenchConfig` and `AccuracyCfg` arrive from the frontend and reach core unread, with one exception.
  A comparator scoring a prefix of a ranking reads the row's own `k` out of the `ParamSet`.

- **The wrapped libraries.** Four, each present for a stated reason.
  `sketch_oxide` is a Rust sketch library covering cardinality, frequency, quantile, universal and elastic sketches.
  `datasketches` binds Apache DataSketches, the reference implementation.
  `asap_sketchlib` is the project's own library, supplying several structural variants per family: the variations it exposes are the trade-offs the benchmark exists to price, so each is its own algorithm.
  `polars` is a DataFrame engine, and it backs the exact baselines.

- **Build-time features.** Three cargo features, each forwarding to the `aqpbm-core` feature of the same name.

## 3. Outputs

- **Reports, one per pass.** The reports a cell produced: the timed passes always, and the accuracy pass when scoring is on.

- **The catalog listing.** One line per row: algorithm, impl, and a description naming the concrete type wrapped, grouped by family.

```
hll-hip                       lib     asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}
cms-fastpath-fixedmatrix  lib     asap CMS, FixedMatrix (5x32768), FastPath
kll-cdf                       polars  polars exact: 101-point quantile grid
```

- **Answers about a row, before it runs.** Whether an algorithm exists, whether a row is scored for accuracy, and whether it takes multi-column input.
  A grouped row consumes labelled records, several columns of label plus a value, and a frontend that guessed would fail at construction.

- **Errors.** Every refusal names the offending value.
  - an unknown algorithm
  - an unknown impl within a known algorithm
  - a width a row cannot be built at
  - a parameter key the family does not have
  - a shape a fixed-size row cannot serve
  - a value the wrapped library would silently clamp, round or ignore

  The last one is the rule the others are instances of: a row builds at exactly the parameters it was given, or it refuses them and says what bound it has.
  Building at anything else would put a config in the record that is not the config that ran, and every plot keys on that field.

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
pub fn takes_columns(algorithm: &str, impl_name: &str) -> Option<bool>;    // None: unknown row

// One parameter point for an algorithm, from the CLI's `--config` string.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet>;

// Resolve a row and run one cell against it: one report per pass.
pub fn run(algorithm: &str, impl_name: &str,
           cfg: &BenchConfig, spec: &WorkloadSpec, params: &ParamSet,
           acc: &AccuracyCfg, width: Numeric) -> Result<Vec<BenchReport>>;
```

`run` is the whole dispatch surface.
A row's set of acceptable parameter points is tabulated nowhere, and the answer is what the row's `init` does with the `ParamSet`.
Only a constructor taking the row's type builds an entry, so the listed name and the dispatched type are one fact.

Five shapes exhaust the ways rows differ behind that one signature.

```
scored     one type   + one comparator     timed passes, plus accuracy when asked
ordered    two types  + one comparator     as scored, with the numeric width picking the type
lib-hll    three types + one comparator    as scored, with `lg_k` picking the type
plain      one type,  no comparator        timed passes only; the row answers no query
parallel   one type,  built with workers   timed passes only; construction takes a run knob
```

The two multi-type shapes exist for the same reason: a library that puts a construction parameter in a *type* forces the choice to be made where a type can still be named, which is the dispatch and not the wrapper.
Resolving it inside the wrapper would mean an enum, and a branch per insert on rows that exist to price that insert.

An **exact baseline** is a row computing the exact answer: it declares the same capability, takes its family's parameters, and runs through identical machinery.
It sits on the family's base algorithm, so every structural variant of that family is scored against one exact answer.

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

### 4.3 The parameter schema

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

- **Whether the parallel rows belong in this catalog.** They answer no query and they discard their per-worker state, so they share only the dispatch surface with every other row.
  Keeping them as rows means one dispatch surface; a subcommand of their own means a second one for a single shape.

- **Whether top-k is an algorithm or a capability.** As an algorithm it gets its own panel, duplicating the frequency algorithms' `rows` and `cols`.
  As a capability on the frequency rows it would compare against them directly, at the cost of a `k` meaning nothing to a frequency row.

- **What the sampling and universal algorithms are scored under.** Giving them comparators means naming a statistic, a moment estimate or a heavy-hitter set, whose definition is an `aqpbm-core` decision.

- **Whether a fixed-shape row should be a row at all.** Three Count-Min variants differ only in a baked matrix size, which is a value their family already has a knob for.
  As rows they price what a compile-time shape buys, and their config is a single point that must be typed exactly.
  As one row taking `rows` and `cols` they would sweep like their peers, at the cost of losing the compile-time specialisation that is the thing being measured.

- **Whether the exact baselines should take a config at all.** They compute the exact answer, so no value changes what they return, and they parse the config only to refuse what their siblings refuse.
  Accepting anything would let a config that fails on every sketch row still produce a baseline number.
