# `aqpbm-cli` Design

`aqpbm-cli` is the frontend.
It builds the `approxbench` binary.
That binary is the only program in the workspace that reads argv, writes a report file, or spawns a process.

## 1. Purpose

Three stages, in order.

- **Resolve** turns an **invocation**, one execution of the binary with its argv and its environment, into one cell.
  A **cell** is one algorithm, one implementation of it, one parameter point and one workload, and it is the unit that gets measured.
  One invocation measures exactly one cell, and expanding a panel of rows is the driver script's loop.

- **Dispatch** hands the cell to a **bundle**, a crate that holds wrapped implementations and a catalog resolving an `(algorithm, impl)` pair to one of them.
  `sketch-bench` is the bundle for sketches.
  Two bundles linked into one binary must not claim the same algorithm name, so a pair always resolves in exactly one catalog.

- **Render** writes the records that come back to the destination the invocation named.

A fourth responsibility sits outside that pipeline: the repeat driver, which re-executes a whole invocation and merges what the executions produced.

The frontend holds no sketch knowledge and measures nothing itself.
It validates algorithms, impls, parameter keys and generator shapes against sets read from the bundle or from `aqpbm-datagen`.
Its own work is what only a binary can do: install a global allocator, own the process, and choose what reaches stdout.

## 2. Inputs

- **Argv.** A subcommand and its options, the only input always present, and the source of every path the other inputs arrive by.

- **A generator spec file.** A path the frontend hands to `aqpbm-datagen`, which parses it into a generator spec and draws from it.
  The frontend reads no field out of it and no option overrides one, because that would edit the user's file from the command line.

- **Input data files.** A path forwarded to the bundle as a file workload spec, since what such a file holds and which item type it must serve are `aqpbm-core`'s questions.

- **The environment.** Two variables, one setting the CPU ramp that brings the clock to a steady state, one marking a repeat child.
  Both are named in §4.2, because a variable an operator sets is command surface.

- **The bundle's catalog.** Which `(algorithm, impl)` rows exist, whether a row can be scored, and whether a parameter point builds that algorithm.
  Every one of those is asked before dispatch, so a request the catalog rejects fails before the cell is built.

- **The build.** Two cargo features decide what the process can measure about its own memory, and they live here because only the final binary installs a `#[global_allocator]`.

## 3. Outputs

- **The record stream.** One JSONL line per pass, exactly as `aqpbm-core` serialises it, appended to the named report path or written to stdout when none is given.
  The frontend measures nothing, so the only values it writes into a record are provenance and the fold across repeat processes.
  A report file accumulates the cells of many invocations, since the frontend appends and never truncates.

- **Terminal output.** Stdout carries machine-readable output only: records and the `list-impls` table.
  Everything a human reads while waiting goes to stderr: which cell is starting, which repeat is running, which request has no comparator.
  The split is load-bearing, because a repeat parent parses a child's stdout as records and drivers pipe stdout into a report file.
  A repeat child writes to stdout whatever the invocation asked for, so only the parent honours the report path.

- **Exit status.** Zero when the cell was measured and every output was written, and one non-zero code for every failure, with the stderr message naming the cause.
  An unresolvable request, a catalog rejection, an unwritable output, and a repeat child exiting non-zero all fail the invocation.
  Asking for accuracy against a row with no comparator is the one downgrade: the row runs timed, with a note on stderr.

## 4. Interfaces

### 4.1 The command surface

- **`bench`** measures the cell an invocation describes, handing it to the bundle's one dispatch entry point.
- **`profile`** is the peer of `bench` for microarchitectural measurement, over the same cell-selection options and into the same record stream.
- **`list-impls`** enumerates the rows the linked bundles expose, so a driver expands "every impl of this algorithm" without a stale copy.

### 4.2 The invocation surface

One invocation is one cell, so every option below selects a single value.
`--help` owns the defaults and the option-by-option reference; what follows is what each group decides.

- **Identity.** `--sketch` names the algorithm and `--impl` names one implementation of it, and the pair must resolve in exactly one linked catalog.

- **Construction.** `--config` carries one parameter point, `key=value` with one value per key, and a list of points is refused because a list would be a sweep.

- **Workload.** `--input` names a data file, `--spec` names a generator spec file, and `--workload` with its shape options describes a stream inline.
  They rank `--input`, then `--spec`, then the shape options, and whichever wins answers the question alone.
  The shape options build the same generator spec a spec file would, so all three paths converge on one generator.

- **Run shape.** `--runs`, `--warmup-runs`, `--repeats` and `--metrics` decide how many measurements are taken and which passes they populate.
  Only `--repeats` above one makes an interval appear, since a per-process mean is the only sample an interval may be taken over.

- **Output.** `--report` names the file the record stream is appended to, and stdout takes the stream when the option is absent.

Two environment variables complete the surface.

- `BENCH_WARMUP_SECS` sets the CPU ramp, and the frontend supplies ten seconds only when it is unset.
- `APPROXBENCH_REPEAT_CHILD` is reserved: the repeat driver sets it on a child, and its presence is what stops that child recursing.

### 4.3 A worked invocation

```sh
approxbench bench \
    --sketch cms --impl oxide \              # the row: the count-min algorithm, as the sketch_oxide crate implements it
    --config 'rows=5 cols=32768' \           # the construction point: a 5 x 32768 counter matrix
    --workload zipf --zipf-s 1.1 \           # generated keys, Zipf-skewed with exponent 1.1, a typical traffic skew
    --size 1000000 --cardinality 100000 \    # a million items drawn from a hundred thousand distinct keys
    --runs 10 --warmup-runs 3                # ten measured runs, after three discarded ones
```

No report path is given, so records land on stdout while progress lands on stderr, and redirecting stdout yields clean JSONL.
