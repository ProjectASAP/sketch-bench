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

- **One construction parameter point per algorithm.**
  Omitting a configuration gives a parameterless point, which an impl that needs parameters refuses by name.

- **The pass-through configuration.** `WorkloadSpec`, `BenchConfig` and `AccuracyCfg` arrive from the frontend and reach core unread, with one exception.
  A comparator scoring a prefix of a ranking reads the row's own `k` out of the `ParamSet`.

- **The wrapped libraries.** Four, each present for a stated reason.
  `sketch_oxide` is a Rust sketch library covering cardinality, frequency, quantile, universal and elastic sketches.
  `datasketches` binds Apache DataSketches, the reference implementation.
  `asap_sketchlib` is the project's own library, supplying several impls per algorithm: the variations it exposes are the trade-offs the benchmark exists to price.
  `polars` is a DataFrame engine, and it backs the exact baselines.

- **Build-time features.** Three cargo features, each forwarding to the `aqpbm-core` feature of the same name.

## 3. Outputs

- **Reports, one per pass.** The reports a cell produced: the timed passes always, and the accuracy pass when scoring is on.

- **The catalog listing.** One line per row: algorithm, impl, and a description naming the concrete type wrapped.

```
hll          lib-hip                      asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate
cms          lib-fixedmatrix-fast-32k     asap CMS, FixedMatrix (5x32768), FastPath
kll          polars                       polars exact: 101-point quantile grid
```

- **Answers about a row, before it runs.** Whether an algorithm exists, whether a row is scored for accuracy, and whether it takes multi-column input.
  A grouped row consumes labelled records, several columns of label plus a value, and a frontend that guessed would fail at construction.

- **Errors.** Five refusals, every one naming the offending value.
  - an unknown algorithm
  - an unknown impl within a known algorithm
  - a width a row cannot be built at
  - a parameter key the algorithm does not have
  - a shape a fixed-size row cannot serve

## 4. Interfaces

### 4.1 The catalog

```rust
pub enum Numeric { I64, F64 }

// Which rows exist. `list` is for a person; the predicates are for a frontend
// checking its arguments before it does any work.
pub fn list() -> Vec<String>;
pub fn algorithm_exists(algorithm: &str) -> bool;
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

Four shapes exhaust the ways rows differ behind that one signature.

```
scored     one type  + one comparator     timed passes, plus accuracy when asked
ordered    two types + one comparator     as scored, with the numeric width picking the type
plain      one type,  no comparator       timed passes only; the row answers no query
parallel   one type,  built with workers  timed passes only; construction takes a run knob
```

An **exact baseline** is a row computing the exact answer: it declares the same capability, takes its algorithm's parameters, and runs through identical machinery.

### 4.2 The parameter schema

An algorithm's parameters are a struct that denies unknown fields, plus one macro invocation binding it to an algorithm name and a canonical point.
A misspelled key becomes an error naming it, because a run at parameters other than those requested is worse than a refusal.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmsParams {
    pub rows: usize,   // one counter row per hash function
    pub cols: usize,   // counters per row
}
// the algorithm name this struct coins, then one point every impl in it can build at
sketch_params!(CmsParams, "cms", CmsParams { rows: 3, cols: 1024 });
```

The `ALGORITHM` const in the params struct, which every wrapper names, is the one place an algorithm's name is written.

## 5. Open questions

- **Whether the parallel rows belong in this catalog.** They answer no query and they discard their per-worker state, so they share only the dispatch surface with every other row.
  Keeping them as rows means one dispatch surface; a subcommand of their own means a second one for a single shape.

- **Whether top-k is an algorithm or a capability.** As an algorithm it gets its own panel, duplicating the frequency algorithms' `rows` and `cols`.
  As a capability on the frequency rows it would compare against them directly, at the cost of a `k` meaning nothing to a frequency row.

- **What the sampling and universal algorithms are scored under.** Giving them comparators means naming a statistic, a moment estimate or a heavy-hitter set, whose definition is an `aqpbm-core` decision.
