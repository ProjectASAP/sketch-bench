# `sketch-bench` Design

`sketch-bench` is the domain bundle: the crate that ships rows and the catalog naming them.

## 1. Purpose

With all the wheels in other crates, the work of really comparing something is done in this crate.

Four things cannot be written without knowing an algorithm by name, and all four are this crate's:

- **The wrappers.** A **wrapper** is a newtype over one third-party sketch, implementing the framework's traits on that sketch's behalf.
  A wrapper claims exactly what the library beneath it provides: `merge` and `prepare` are stated where the library has them, and left unstated otherwise.
  An answer passes through as the library returns it, and its type follows the library's own.
  Where the library's documentation prescribes a treatment the wrapper applies it, and where it says nothing the value is reported unchanged.
- **The catalog.** The list itself, binding each `(algorithm, impl)` pair to the type implementing it.
- **The construction parameters.** `hll` takes an `lg_k` while `cms` takes a `rows` and a `cols`, and so on for every family.
- **The choice of comparator for each row.** Core ships the comparators, and one capability can carry several.
  A KLL states its error as a rank error and a DDSketch as a relative error.

Consider this crate to be something that wraps around existing functionalities and can be used by `aqpbm-cli`.

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

// Resolve a row and run one cell against it: one report per pass.
pub fn run(algorithm: &str, impl_name: &str,
           cfg: &BenchConfig, spec: &WorkloadSpec, params: &ParamSet,
           acc: &AccuracyCfg, width: Numeric) -> Result<Vec<BenchReport>>;
```

`run` is the whole dispatch surface.
A row's set of acceptable parameter points is tabulated nowhere, and the answer is what the row's `init` does with the `ParamSet`.
Only a constructor taking the row's type builds an entry, so the listed name and the dispatched type are one fact.

Six shapes exhaust the ways rows differ behind that one signature.

```
scored        one type    + one comparator     timed passes, plus accuracy when asked
ordered       two types   + one comparator     as scored, with the numeric width picking the type
lib-hll       three types + one comparator     as scored, with `lg_k` picking the type
fixed-matrix  a table of types + one comparator as scored, with `(rows, cols)` picking the type
plain         one type,  no comparator         timed passes only; the row answers no query
parallel      one type,  built with workers    timed passes only; construction takes a run knob
```

The three multi-type shapes exist for one reason: a library that puts a construction parameter in a *type* forces the choice to be made where a type can still be named, which is the dispatch and not the wrapper.
Resolving it inside the wrapper would mean an enum, and a branch per insert on rows that exist to price that insert.
They differ only in how many types there are and where the set is written: two spelled in the signature, three in a const, and a table generated beside the types it admits.

An **exact baseline** is a row computing the exact answer: it declares the same capability, takes its family's parameters, and runs through identical machinery.
It sits on one algorithm of its family, and every other algorithm in that family is read against it.
Which one is a judgement per family: usually the family's base algorithm.
`kll` has no base algorithm, since every KLL row states a query path, so its baseline sits on `kll-cdf`, the path it takes.
A baseline is timed like every other row, and the exact answer a comparator scores against is computed inside core.

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

- **Whether top-k is its own algorithm or a capability the frequency rows declare.** As an algorithm it gets its own panel, duplicating the frequency algorithms' `rows` and `cols`.
  As a capability on the frequency rows it would compare against them directly, at the cost of a `k` meaning nothing to a frequency row.

- **What the sampling and universal algorithms are scored under.** No capability in core's list fits a moment estimate or a heavy-hitter set.
  Scoring them means adding a statistic to core, a wider change than adding a row here.

- **How wide the compiled-in shape table should be.** A fixed-matrix row sweeps only the shapes some build instantiated, so the table decides what is measurable without recompiling.
  Widening it is nearly free in compile time and costs rlib size; the current ceiling is set by an unoptimised build materialising the largest array on a test thread's stack, not by anything about the measurement.

- **Whether the exact baselines should take a config at all.** They compute the exact answer, so no value changes what they return, and they parse the config only to refuse what their siblings refuse.
  Accepting anything would let a config that fails on every sketch row still produce a baseline number.
