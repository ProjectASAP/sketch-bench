# Repo reorganization notes

Captures the architecture walkthrough and the legacy-reorg work done on
branch `chore/legacy-reorg`. Standalone — not part of the design docs in
`docs/`.

> **Historical snapshot.** Paths in §2 / §6 / §7 reflect the mid-reorg
> state. The cleanup completed since: `throughput/`, `accuracy/`, and
> the original `legacy/` top-level trees have been deleted; the per-family
> C++ harnesses they contained have been ported onto `cpp-bench/common/`
> (now live under `cpp-bench/{hll,cms,cs,kll}/`); the plot scripts that
> orchestrated the old layout now sit at `visualization/plots/`. The
> per-impl `run_*_{cpp,rust,polars,all}.sh` shell scaffolding has been
> retired in favour of top-level `scripts/run_throughput.sh` /
> `scripts/run_accuracy.sh` (Rust via `sketchlib bench`) and
> `cmake --build cpp-bench/build` (C++). Use `git log -- <path>` to
> recover any pre-deletion content.

## 1. The three workspace crates

| crate | role | kind |
|---|---|---|
| `sketch-core` | shared contract: `Sketch` trait, `Probe`, `Workload`, v1 JSONL `Record` schema, errors | library — depends on nothing project-internal |
| `sketch-bench` | macro-metric measurement: throughput / latency / CPU / memory / accuracy, runner, aggregation, exact baselines | library — depends on `sketch-core` only |
| `sketch-cli` | the `sketchlib` binary: clap subcommands (`bench`, `list-impls`), dispatch table, wrappers around `sketch_oxide` / `datasketches` / `asap_sketchlib`, sweep parsing, allocator selection | binary — depends on `sketch-bench` |
| `sketch-runtime` | embeddable always-on `Sampler` + gRPC `Exporter` that streams the same v1 schema to ASAPController | library — depends on `sketch-core` and `sketch-bench` |

Dependency direction is strictly `sketch-cli` → `sketch-bench` → `sketch-core`,
with `sketch-runtime` as a parallel consumer of `sketch-core` (+ `sketch-bench`).

### Why split `sketch-bench` from `sketch-cli`

- **Reusability**: `sketch-bench` is also consumable by `sketch-profile`
  (future), `sketch-runtime`, unit tests, and the Criterion micro-benches in
  `rust-microbench/` (formerly `benchmark/`).
- **Dependency isolation**: `sketch-bench` does not pull in any concrete sketch
  implementation. All third-party sketch crates (`sketch_oxide`,
  `datasketches`, `asap_sketchlib`) are declared only in `sketch-cli`'s
  `Cargo.toml`.
- **Testability**: the measurement library is unit-testable on its own; the
  CLI only needs to test arg parsing and dispatch.

### Minimal call chain

```
user: sketchlib bench --sketch hll --impl oxide --workload zipf --size 1M --runs 10
  sketch-cli/main.rs         clap parse → BenchArgs
  sketch-cli/dispatch.rs     "hll/oxide" → ImplEntry (factory + default grid)
  sketch-cli/sweep.rs        parse config grid
  sketch-cli/wrappers/hll    construct sketch_oxide::Hll, wrap as Sketch
    ↓ BenchConfig + workload + Sketch fed into:
  sketch-bench/runner.rs     BenchRunner::run() — warmup, measured loops
  sketch-bench/metrics/*     throughput / latency / memory / accuracy
  sketch-bench/aggregation   N runs → mean / quantiles / CI → Report
    ↑ Report
  sketch-cli/main.rs         Report → JSONL line → stdout or --report file
```

## 2. Overlap between new stack and `accuracy/` + `throughput/`

The README admits the legacy harness still runs in parallel during migration.
Concrete overlap:

| legacy (per-sketch shell harness) | new (per-metric library + CLI) |
|---|---|
| `throughput/` | `sketch-bench/src/metrics/throughput.rs` |
| `accuracy/`   | `sketch-bench/src/accuracy/` |
| `cpp/` + `run_*.sh` | `sketchlib bench` (`sketch-cli/`) |

The monolithic `rust/` tree (one binary per sketch, JSONL → `rust/output/`)
has been retired: cms / cs / hll / kll / nitro are now covered by
`throughput/<family>/rust/`, and accuracy/* covers the ground-truth side.
**Caveat:** `throughput/` does *not* yet have subtrees for **univmon** or
**elastic** — for those two families, `sketchlib bench --sketch
{univmon,elastic}` is currently the only path.

Both produce comparable throughput / accuracy numbers, but the legacy tree
still does several things the new stack cannot.

## 3. Gaps the new stack does *not* yet cover

What `accuracy/` + `throughput/` still offer that `sketch-cli → sketch-bench →
sketch-core` cannot match today:

1. **C++ implementations / cross-language comparison** — every legacy sketch
   has a `cpp/` subtree (CMake + Apache DataSketches C++). New stack is Rust
   only.
2. **HLL estimator variants** — `ErtlMLE`, `HIP`, multiple `lg_k` precisions
   in `accuracy/cardinality/rust/`. The `sketch-cli` `hll/lib` exposes only
   one ASAP HLL variant.
3. **OctoSketch** — `accuracy/octo/` and `throughput/octo/` sweep over worker
   counts and emit per-family (CMS / CS / HLL) CSVs. No `octo` family in
   `sketch-cli` dispatch.
4. **Polars exact baselines** — `throughput/polars_freq/`,
   `polars_cardinality/`, `polars_quantile/` are vectorised columnar exact
   baselines, not per-item inserts. `sketch-bench/src/baselines/` only has
   the per-item `HashSet` / `HashMap` / sorted-`Vec` baselines.
5. **Query-stage throughput** — every legacy sketch has both
   `plot_*_throughput.py` and `plot_*_throughput_query.py`. `BenchConfig`
   has a `query_count` field but the CLI hardcodes it to `None`.
6. **Per-seed / per-key error CSVs** — the accuracy runner emits a separate
   `*_key_seed_errors_rust.csv` with one row per (key, seed) for box-plot
   variance analysis. The current `Record` schema only has aggregated mean /
   p99 rel-err.
7. **PNG output** — `visualization/plots/{throughput,accuracy}/plot_*.py`
   produce publication PNGs from the long-format CSVs emitted by
   `sketchlib bench --raw-csv DIR`. The browser-only side (`visualization/`)
   stops at JSONL.
8. **CPU pinning / process-per-trial** — the previous shell layer
   (`run_throughput_with_cpu.py`, retired) implemented `taskset` pinning,
   `scaling_governor=performance`, Turbo off, and per-trial fork.
   `sketchlib` runs all trials inside one long-lived process; the
   equivalent pinning hooks have not yet been ported.
9. **Parametric naming convention** — `throughput/cms32k/`, `cs32k/` carry
   the parameter in the directory name; new stack expresses this via
   `--config 'rows=5 cols=32768'  # every key is required; see docs/BENCH_SWEEP.md` but the plot scripts/CSV conventions haven't
   migrated.

## 4. Where each gap should land

| gap | layer |
|---|---|
| query-stage fields, per-seed/per-key detail stream | `sketch-core` (schema first — everyone downstream depends on it) |
| CPU pinning / governor lock, process-per-trial isolation | `sketch-bench` (runner / new `env.rs`) |
| `BatchSketch` sub-trait for vectorised baselines | `sketch-bench` |
| OctoSketch family, ASAP HLL estimator variants, Polars baselines | `sketch-cli` (dispatch + wrappers) |
| PNG output (`plot_*.py`) | **not** in these three crates — belongs in `scripts/`, fed by the unified v1 JSONL |
| C++ comparison | **independent** — see §5 |

Recommended ordering: core schema first, then bench-level runner changes,
then CLI wrappers, then plot scripts. Schema before wrappers, otherwise each
wrapper has to be rewritten after the schema lands.

## 5. C++: keep it independent

Decision: the C++ comparison stays out of the `sketchlib` process entirely.

- C++ already has its own backbone (Google Benchmark, plus the helpers in
  `cpp/common/`) and its own ecosystem (Apache DataSketches C++).
- The current `cpp/` tree already follows the "binary-isolated monolithic
  execution" model from `legacy/design_conversion_conclusion.txt` — one
  binary per sketch, independent build (CMake), orchestrated externally.
- Forcing FFI into `sketchlib` would introduce ABI / build pain without
  improving fairness.

The only contract between C++ and the new stack is **the v1 JSONL schema**:
each C++ binary emits `sketch-core::report::Record` lines (with
`language: "cpp"`, e.g. `implementation: "cpp_datasketches_hll"`) to stdout.
A thin orchestrator (shell or Python) concatenates the C++ JSONL and the
`sketchlib bench …` JSONL into a single report file that the plot scripts
and `visualization/` consume without distinguishing source.

Work required:
- In `cpp/common/`: a small helper that formats Google Benchmark results
  into v1 JSONL.
- In `sketch-core/src/report.rs`: ensure `language` / `implementation`
  fields are sufficient (likely already are).
- No changes to `sketch-cli`.

## 6. Reorg done on `chore/legacy-reorg` (option B)

Three options were considered:

- **A** — docs-only labels, no file moves.
- **B** — create `legacy/`, move the self-contained legacy artifacts in,
  leave anything with relative-path coupling (`cpp/`, `accuracy/`,
  `throughput/`, `run_all_benchmarks.sh`) in place.
- **C** — also move the coupled trees, fixing all `path = "..."` Cargo
  references and shell `cd cpp/rust` calls.

Chose **B**. Reason: it cuts the root from 23 to 19 top-level entries and
makes the new stack visually obvious, without touching any relative-path
coupling.

Moved into `legacy/`:

```
legacy/
├── README.md                                  (new — explains the dir)
├── other_benchmark/                           orphan C file
├── insertion_optimized_sample_output/         archived sample output
├── plots/                                     old PNGs from the single legacy plot script
├── scripts/                                   the single legacy plot script
├── cumulative_result.txt                      aggregated text results
├── simplified_cumulative_result.txt           aggregated text results
├── design_conversion_conclusion.txt           early design notes
└── dir_structures.md                          pre-merge directory snapshot
```

All done with `git mv` so history is preserved. Verified no operational
references (`*.sh`, `*.toml`, `*.py`, `*.rs`) point at any moved path; only
two doc references exist in `docs/MERGE_PLAN.md` (a TODO bullet) and they
remain valid since the TODO is about deletion, not relocation.

## 7. Still in original location (not moved)

These were *not* moved because they have live relative-path coupling and
would need a coordinated path-fix commit:

- `cpp/` — referenced by `run_all_benchmarks.sh` and contains its own
  `output/` consumed by visualization.
- `accuracy/`, `throughput/` — their per-sketch crates use
  `path = "../../../sketch-bench"` and `path = "../../../../asap_sketchlib"`
  in `Cargo.toml`; their shell scripts use `ACCURACY_DIR/../input`.
- `run_all_benchmarks.sh` — top-level orchestrator for `cpp/` (the
  legacy monolithic `rust/` tree it used to invoke has been removed).

Promoting these to `legacy/` (the option-C path) is tracked implicitly by
`docs/MERGE_PLAN.md`'s migration phases. Once each legacy harness is
absorbed by `sketchlib bench` or by the C++ JSONL adapter (§5), the
corresponding tree can be moved or deleted.
