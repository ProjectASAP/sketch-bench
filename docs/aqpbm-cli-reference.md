# `approxbench` CLI reference

The intended command surface, hand-authored to match [`aqpbm-cli.md`](aqpbm-cli.md).
The code is changed to match this file, and `scripts/dump_cli_reference.sh` diffs the built binary against it.
`aqpbm-cli.md` describes the shape of each option group; this file is the full surface.

Options are grouped under the headings §4.2 of the design doc names, so `--help` and the design doc read the same way.

## `approxbench`

```
Approximate query processing benchmark suite

Usage: approxbench <COMMAND>

Commands:
  sketchbench    Measure one cell of the sketch bundle
  sketchprofile  Profile one cell of the sketch bundle
  workload       Generate or inspect synthetic `.bin` workloads
  help           Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

One subcommand per bundle and measuring kind, plus the tools that name no bundle.
A second bundle adds `<bundle>bench` and `<bundle>profile`, and changes nothing else here.
`workload` is the one tool that stays fixed as bundles come and go.

## `approxbench sketchbench`

```
Measure one cell of the sketch bundle

Usage: approxbench sketchbench [OPTIONS] --algorithm <ALGORITHM> --impl <IMPL_NAME>
       approxbench sketchbench --list-impls

Identity:
      --algorithm <ALGORITHM>
          Accumulator algorithm (hll, kll, cms, countsketch, dd, topk, elastic, nitro, univmon,
          hydra-cms, hydra-hll). The Hydra cell type is on this axis because it decides which
          statistic the grid answers. `--list-impls` prints every (algorithm, impl) pair

      --impl <IMPL_NAME>
          Implementation within the algorithm, exactly one (`oxide`). `--list-impls` shows the
          choices. One invocation measures one (impl, config) cell; to race several, invoke once
          per impl

      --list-impls
          Print every (algorithm, impl) pair this bundle offers, then exit. Selects no cell and
          writes no record, so it ignores every other option

Construction:
      --config <CONFIG>
          Construction config for this cell: `'k1=v1 k2=v2'`, one value per key. Omitted gives a
          parameterless point, which tunable impls reject by name. A comma list is an error: one
          invocation is one cell, never a grid

      --workers <WORKERS>
          Worker threads for the parallel-insert impls (`lib-fastpath-parallel`). Other impls
          ignore it; `1` is single-threaded

          [default: 1]

Workload:
      --input <INPUT>
          Load the workload from a file instead of generating it. Format comes from the
          extension: `.bin` (little-endian i64), `.pcap` (IPv4 src addr), `.csv` (column 0 below
          a header). Wins over `--spec` and over the inline options

      --spec <SPEC>
          Generate in-process from a `datagen` spec file (examples in `configs/datagen/`),
          unlocking every generator shape without a disk round-trip. Wins over the inline
          options; `--input` wins over it.

          A file holding a *list* of specs is a multi-column stream: the last column is the
          value, the ones before it are labels. The rows ingesting labelled records (the
          `hydra-*` algorithms) need one; every other row refuses it.

      --workload <WORKLOAD>
          Inline shape: "uniform" or "zipf". Ignored when `--input` or `--spec` is set

          [default: uniform]

      --size <SIZE>
          Number of items in the workload

          [default: 1000000]

      --cardinality <CARDINALITY>
          Cardinality (uniform: max key; zipf: key-space size)

          [default: 100000]

      --zipf-s <ZIPF_S>
          Zipf `s` exponent (only used when `--workload zipf`)

          [default: 1.1]

      --dtype <DTYPE>
          Numeric width for the ordered algorithms (`kll`, `dd`): `i64` or `f64`. The one
          item-type choice left, since every other row's is fixed by its wrapper, and `f64`
          elsewhere is refused by name. Encoding only

          [default: i64]

      --alphabet <ALPHABET>
          Alphabet for generated string keys, for the rows whose wrappers take text. Character
          order is the digit order of the positional encoding, so a rank always renders the same
          key. Overridden by `--spec`'s own `string:` block

          [default: abcdefghijklmnopqrstuvwxyz0123456789]

      --key-len <KEY_LEN>...
          Inclusive length bounds for generated string keys; equal values give a fixed length.
          Key length dominates the cost of a hashing insert path, so it is the knob worth
          sweeping

          [default: 8 24]

      --seed <SEED>
          Seed for reproducibility

          [default: 42]

Repetition:
      --runs <RUNS>
          Measured runs inside one process, summarised as mean / stddev / `throughput_samples`.
          They share a process, so they support no confidence interval. See `--repeats`

          [default: 10]

      --warmup-runs <WARMUP_RUNS>
          Runs discarded before measurement begins. A cold allocator and a cold cache measure
          the wrong thing

          [default: 3]

      --repeats <REPEATS>
          Re-execute the whole invocation in this many separate processes and report the 95% CI
          over their means. The only setting that makes `ci95` appear. At 1 no interval is
          claimed. Costs R times the wall clock

          [default: 1]

Measurement content:
      --metrics <METRICS>
          Comma-separated: throughput,latency,cpu,memory,accuracy,merge. Each of throughput,
          latency, accuracy and merge is one pass over the workload; cpu and memory attach to
          whichever passes run. Default: all except merge

      --accuracy
          Compute ground-truth accuracy per run, one comparator per algorithm. Rows declaring no
          query capability are not scored: they still run, timed only, after a stderr note

      --accuracy-probes <ACCURACY_PROBES>
          Cap on distinct keys probed by the frequency comparator; `0` probes every one. Ignored
          by the cardinality / quantile / top-k comparators

          [default: 100000]

      --merge-shards <MERGE_SHARDS>
          Split the stream into this many shards, time folding them into one, and compare
          against the whole stream; `1` skips the pass. Linear sketches merge exactly, so a gap
          is a defect; for KLL it is the result

          [default: 1]

Output:
      --report <REPORT>
          Path to append JSONL records to. `-` or omitted sends them to stdout

      --raw-csv <RAW_CSV>
          Output directory for long-format CSVs, one row per measured run, named
          `<algorithm>_throughput[_query]_results_rust.csv`. Coexists with `--report`; for plot
          scripts that consume that CSV shape

      --flat
          Write one line for the whole cell instead of one line per pass. Every metric field
          carries the name of the pass that produced it, and the cell's identity is written once.
          The shape a leaderboard consumes

      --pass-prefix <PASS_PREFIX>
          Rename a pass on the wire under `--flat`, as `pass=prefix`, repeatable. Only for
          holding a consumer's existing field names steady while it migrates

Options:
  -h, --help
          Print help (see a summary with '-h')
```

## `approxbench sketchprofile`

```
Profile one cell of the sketch bundle

Usage: approxbench sketchprofile [OPTIONS] --algorithm <ALGORITHM> --impl <IMPL_NAME>
       approxbench sketchprofile --list-impls

Identity, Construction and Workload:
      Identical to `sketchbench`. The two subcommands select a cell the same way, and only the
      instruments differ

Instrument:
      --profiler <PROFILER>
          perf-stat | perf-record | cachegrind | heaptrack | vtune. perf-stat fills the record's
          counter fields; the others write an artifact and record its path

          [default: perf-stat]

      --counters <COUNTERS>
          Comma-separated hardware counters for `--profiler perf-stat`, for example
          `cycles,instructions,cache-misses,branch-misses`. Ignored by every other profiler

          [default: cycles,instructions,cache-misses,branch-misses]

      --profile-out <PROFILE_OUT>
          Directory the artifact-producing profilers write into. Required by every profiler
          except perf-stat

Repetition:
      --runs <RUNS>
          Measured runs inside one process. Sampling profilers need enough work to collect a
          usable number of samples

          [default: 1]

      --warmup-runs <WARMUP_RUNS>
          Runs discarded before the profiler attaches

          [default: 1]

Output:
      --report <REPORT>
          Path to append JSONL records to. `-` or omitted sends them to stdout

Options:
  -h, --help
          Print help (see a summary with '-h')
```

`sketchprofile` writes the same record schema as `sketchbench`, with the profile section
populated instead of the bench section.
It takes no `--metrics`, since which passes to run is a question only measurement asks.

## `approxbench workload`

```
Generate or inspect synthetic `.bin` workloads

Usage: approxbench workload <COMMAND>

Commands:
  generate  Generate a synthetic `.bin` workload (+ `.meta.json` sidecar)
  describe  Print the provenance/stats of a generated `.bin` workload
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

Generation and measurement share no options, which is why generation is its own subcommand.
It calls `aqpbm-datagen` as a library, so there is no second binary.

## `approxbench workload generate`

```
Generate a synthetic `.bin` workload (+ `.meta.json` sidecar)

Usage: approxbench workload generate [OPTIONS] --out <OUT>

Options:
      --shape <SHAPE>              Distribution: uniform | zipf | monotonic-timestamp | skewed-categorical. Ignored when `--spec` is set [default: uniform]
      --size <SIZE>                Number of values to generate [default: 1000000]
      --seed <SEED>                Seed for reproducibility [default: 42]
      --out <OUT>                  Output `.bin` path. Parent directories are created if missing
      --dtype <DTYPE>              Physical output type: i64 | u64 | f64. `sketchbench --input` reads i64 only; f64 is benchmarkable through `sketchbench --dtype f64`, which generates in-process. u64 has no consumer in this repo [default: i64]
      --cardinality <CARDINALITY>  Uniform: max key (exclusive). Zipf: key-space size [default: 100000]
      --zipf-s <ZIPF_S>            Zipf `s` exponent (only used when `--shape zipf`) [default: 1.1]
      --start <START>              Monotonic-timestamp: starting value (first emitted value) [default: 0]
      --unit <UNIT>                Monotonic-timestamp: unit label: nanos | millis | secs [default: nanos]
      --gap <GAP>                  Monotonic-timestamp: inter-arrival gap as `kind:param` (const:1000 | geometric:0.01 | exp:0.5 | poisson:5). Required for `--shape monotonic-timestamp`
      --min-gap <MIN_GAP>          Monotonic-timestamp: minimum gap (1 gives strictly increasing, 0 allows duplicates) [default: 1]
      --categories <CATEGORIES>    Skewed-categorical: size of the id domain (ids `0..n`). Required for `--shape skewed-categorical` (use `--spec` for explicit ids)
      --weights <WEIGHTS>          Skewed-categorical: weight scheme: `uniform` | `zipf:s` [default: zipf:1.1]
      --spec <SPEC>                Read the full spec from a `.yaml`/`.yml`/`.json` file. Overrides `--shape` and its per-shape flags (`--size`/`--seed` still apply only when NOT set here; the spec file is authoritative)
      --no-meta                    Skip writing the `.meta.json` sidecar
  -h, --help                       Print help
```

## `approxbench workload describe`

```
Print the provenance/stats of a generated `.bin` workload

Usage: approxbench workload describe <PATH>

Arguments:
  <PATH>  Path to a generated `.bin` file. Reads `<path>.meta.json` if present, else falls back to loading the raw i64 stream

Options:
  -h, --help  Print help
```

## Environment

- `BENCH_WARMUP_SECS` sets the CPU ramp, and the frontend supplies ten seconds only when it is unset.
- `APPROXBENCH_REPEAT_CHILD` is reserved: the repeat driver sets it on a child, and its presence stops that child recursing.
