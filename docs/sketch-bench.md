# `sketch-bench` Design

`sketch-bench` is the domain bundle: the crate that ships rows and the catalog naming them.
It holds the wrapped sketch implementations, the per-algorithm construction parameters, and the catalog turning a `(algorithm, impl)` pair of strings into a concrete Rust type.
All of the sketch knowledge in the workspace lives here, which is what allows the framework underneath it to hold none.

> The repository is also called `sketch-bench`.
> This document describes the crate at `sketch-bench/`; the repository-level design doc is `docs/DESIGN.md`.

## 1. Purpose

The crate answers one question: given two strings, which sketch is that?

`approxbench bench --sketch hll --impl oxide` reaches the frontend holding `"hll"` and `"oxide"`, and the framework it dispatches into is generic over a Rust type.
Closing that gap requires a list: every `(algorithm, impl)` pair that exists, each bound to the type implementing it.

Four things cannot be written without knowing an algorithm by name, and all four are this crate's:

- **The wrappers.** A **wrapper** is a newtype over one third-party sketch, implementing the framework's traits on that sketch's behalf.
  Every entry in the catalog points at one, and a wrapper is where a foreign API is bent into the framework's shape.
  A wrapper claims exactly what the library beneath it provides: `merge` and `prepare` are stated where the library has them, and left unstated otherwise.
- **The catalog.** The list itself, binding each `(algorithm, impl)` pair to the type implementing it.
- **The construction parameters.** `hll` takes an `lg_k` while `cms` takes a `rows` and a `cols`, and no generic layer can say that.
- **The choice of comparator for each row.** A KLL states its error as a rank error and a DDSketch as a relative error.

The boundary is nominal knowledge, the same line `aqpbm-core` draws from its side.
Everything that spells an algorithm name or names a third-party sketch library is here, and everything that measures is in `aqpbm-core`.

Benchmarking a sketch of your own means depending on `aqpbm-core` and implementing its traits.
Nothing in this crate is on that path; this crate exists for the CLI.

The catalog is per bundle, so a second bundle sits beside this one on the same framework and ships a catalog of its own.

## 2. Inputs

- **The row selector, as two strings.** An algorithm name and an impl name, originating in the CLI's `--sketch` and `--impl` arguments.
  The catalog is the only thing that gives them meaning.

- **The numeric width.** A **numeric width** is the item type an ordered quantile row is built at, `i64` or `f64`, chosen by the frontend.
  Every other row takes its item type off `Accumulator::Item`, so this is the one item-type choice a caller makes.
  The ordered quantile rows are the declared exception, because the same algorithm is worth measuring over integers and over floats.
  The library's own conversion is part of the price, and an unsupported width is refused from the catalog before a single item is drawn.

- **One construction parameter point per algorithm.** Parsed from whitespace-separated `key=value` tokens, or taken as the algorithm's canonical point when the caller names none.

```
--config "rows=5 cols=2048"        # a Count-Min row: 5 hashed rows over 2048 counters each
--config "rows=5 cols=2048 k=100"  # a top-k row: the same counter array, plus 100 tracked keys
```

  Either way the result is a `ParamSet`: the algorithm name plus a JSON object that only that algorithm's own params struct reads.

- **The pass-through configuration.** `WorkloadSpec`, `BenchConfig` and `AccuracyCfg` arrive from the frontend and reach core unread, with one exception.
  A comparator scoring a prefix of a ranking reads the row's own `k` out of the `ParamSet`.
  The score is then taken at the `k` the sketch was built with.

- **The wrapped libraries.** Four, each present for a stated reason.
  `sketch_oxide` is a Rust sketch library covering cardinality, frequency, quantile, universal and elastic sketches.
  `datasketches` binds Apache DataSketches, the reference implementation whose guarantees the literature quotes.
  `asap_sketchlib` is the project's own library, supplying several impls per algorithm: the variations it exposes are the trade-offs the benchmark exists to price.
  `polars` is a DataFrame engine, and it backs the exact baselines.

- **Build-time features.** The latency histogram, jemalloc heap sampling and per-sketch allocation accounting are cargo features forwarding to core's.
  A linking binary therefore turns each of them on in one place.

## 3. Outputs

- **Reports, one per pass.** The reports a cell produced: the timed passes always, and the accuracy pass when scoring is on.
  The CLI serialises each into a JSONL `Record` through core.

- **The catalog listing.** One line per row: algorithm, impl, and a description naming the concrete type wrapped.

```
hll          lib-hip                      asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate
cms          lib-fixedmatrix-fast-32k     asap CMS, FixedMatrix (5x32768), FastPath
kll          polars                       polars exact: 101-point quantile grid
```

  The description is the only new information in a **catalog entry**, one line of the catalog binding a row's type to how it runs.
  The listing answers "which rows exist", for a person reading a terminal and for a driver script expanding every impl of one algorithm.

- **Answers about a row, before it runs.** Whether an algorithm exists, and whether a row is scored for accuracy.
  The frontend asks both up front, so `--accuracy` against a row that answers no query is a message instead of a silent run.

- **Errors.** Five refusals, every one before measurement and every one naming the offending value.
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

// One parameter point for an algorithm, from the CLI's `--config` string.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet>;

// Resolve a row and run one cell against it: one report per pass.
pub fn run(algorithm: &str, impl_name: &str,
           cfg: &BenchConfig, spec: &WorkloadSpec, params: &ParamSet,
           acc: &AccuracyCfg, width: Numeric) -> Result<Vec<BenchReport>>;
```

`run` is the whole dispatch surface.
It resolves the pair of strings, refuses a width the row cannot serve, and runs the timed half plus the accuracy half when asked.
A row's set of acceptable parameter points is tabulated nowhere.
The answer is what the row's `init` does with the `ParamSet`, and a refusal names what was wrong.
Only a constructor taking the row's type builds an entry, so the listed name and the dispatched type are one fact.

Four shapes exhaust the ways rows differ behind that one signature.

```
scored     one type  + one comparator     timed passes, plus accuracy when asked
ordered    two types + one comparator     as scored, with the numeric width picking the type
plain      one type,  no comparator       timed passes only; the row answers no query
parallel   one type,  built with workers  timed passes only; construction takes a run knob
```

A `plain` row is a full row, and its missing accuracy line is a fact about what it claims.
An algorithm's impl set is uneven by construction, and the unevenness is the measurement.
The algorithm is the question and an impl is one library's answer, which is what makes a **panel** meaningful: the rows of one algorithm at one workload and one parameter point.

An **exact baseline** is a row computing the exact answer: it declares the same capability, takes its algorithm's parameters, and runs through identical machinery.
That makes it raceable on throughput, and a check on the comparator, since an exact answer correctly scored comes back at zero error.
Its work happens after the last item, so its insert clock times a buffer append and only its build clock is comparable to anything.

### 4.2 The parameter schema

An algorithm's parameters are a struct that denies unknown fields, plus one macro invocation binding it to an algorithm name and a canonical point.

```rust
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct CmsParams {
    pub rows: usize,   // one counter row per hash function
    pub cols: usize,   // counters per row
}
// the algorithm name this struct coins, then one point every impl in it can build at
sketch_params!(CmsParams, "cms", CmsParams { rows: 3, cols: 1024 });
```

`deny_unknown_fields` turns a misspelled key into an error naming it, because a run at parameters other than those requested is worse than a refusal.
The canonical point is what a frontend builds at when the caller names no configuration, and what a coverage test drives every row from.
The `ALGORITHM` const in the params struct, which every wrapper names, is the one place an algorithm's name is written.
Erasing the struct gives the `ParamSet` core carries and the record serialises, keys in a stable order: `{"algorithm": "cms", "params": {"cols": 2048, "rows": 5}}`.
Recovering the typed value fails for another algorithm's set, which stops a `topk` point being read as a `cms` one.

## 5. Open questions

- **Whether the parallel rows belong in this catalog.** They answer no query and they discard their per-worker state, so they share only the dispatch surface with every other row.
  Keeping them as rows means one dispatch surface; a subcommand of their own means a second one for a single shape.

- **Whether top-k is an algorithm or a capability.** As an algorithm it gets its own sweep vocabulary and its own panel, duplicating the frequency algorithms' `rows` and `cols`.
  As a capability on the frequency rows it would compare against them directly, at the cost of a `k` meaning nothing to a frequency row.

- **What the sampling and universal algorithms are scored under.** Rows declaring no capability are throughput-only, so an algorithm of them is a panel with one axis.
  Giving them comparators means naming a statistic, a moment estimate or a heavy-hitter set, whose definition is an `aqpbm-core` decision.

- **Whether an exact baseline shares the sketch's algorithm or holds its own.** Naming it inside the algorithm is what puts it on the same plot.
  It also hands it parameter knobs that mean nothing to it, and a sweep over those knobs redraws one baseline point once per configuration.

- **The crate's name.** `sketch-bench` names both this crate and the repository, and the repository is the larger of the two.
  A rename would make the bundle's role legible from the dependency graph alone, at the cost of touching every import and every citing document.
