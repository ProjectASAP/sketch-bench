# `aqpbm-cli` Design

`aqpbm-cli` is the frontend.
It builds the `approxbench` binary.
That binary is the only program in the workspace that reads argv, writes a report file, or spawns a process.

If user want to use the benchmark somehow or use the built-in benchmark, this is where they should go.
<!-- 
## 1. Purpose

Three stages, in order.

- **Resolve** turns an **invocation**, one execution of the binary with its argv and its environment, into one cell.
  A **cell** is one algorithm, one implementation of it, one parameter point and one workload.
  A cell is the unit that gets measured, and one invocation measures exactly one.
  Expanding a **panel**, the rows of one algorithm at one workload and one parameter point, is the driver script's loop.

- **Dispatch** hands the cell to the **bundle** the subcommand names.
  A bundle is a crate holding wrapped implementations, and a catalog resolving an `(algorithm, impl)` pair to one of them.
  `sketch-bench` is the bundle for sketches.
  Each bundle owns its own subcommand, so two bundles may both offer an algorithm named `cms`.

- **Render** writes the records that come back to the destination the invocation named.
  It writes one line per square, or one line for the whole cell when the invocation asks for the flattened shape.
  §4.3 gives the rule that turns a cell's squares into one row.

**Repeat runs and aggregation** sits outside those three stages and measures nothing.
It re-executes one invocation as several child processes, then aggregates what those processes measured.
§4.2 gives the difference of repeats (process) vs runs (loop).

The frontend knows each bundle's name and its dispatch entry, and nothing about what a bundle measures.
It validates algorithms, impls, parameter keys and generator shapes against sets the bundle and `aqpbm-datagen` expose.
Its own work is what only a binary can do: install a global allocator, own the process, and choose what reaches stdout.

## 2. Inputs

- **Argv.** A subcommand and its options, the only input always present.
  Every path the other inputs arrive by comes from here.

- **A generator spec file.** A path the frontend hands to `aqpbm-datagen`, which parses it and draws from it.
  The frontend reads no field out of it, and no option overrides one.
  An option that overrode a field would edit the user's file from the command line.

- **Input data files.** A path forwarded to the bundle as a file workload spec.
  What such a file holds, and which item type it must serve, are `aqpbm-core`'s questions.

- **The environment.** Two variables, one setting the CPU ramp that brings the clock to a steady state, one marking a repeat child.
  Both are named in §4.2, because a variable an operator sets is command surface.

- **The bundle's catalog.** Which rows exist, whether a row can be scored, whether it takes multi-column input, and whether a parameter point builds.
  Every one of those is asked before dispatch, so a request the catalog rejects fails before the cell is built.

- **The build.** Cargo features decide what the process can measure about its own memory.
  They live here because only the final binary installs a `#[global_allocator]`.

## 3. Outputs

- **The record stream.** One JSONL line per square, exactly as `aqpbm-core` serialises it.
  A **square** is one operation read for one metric, and the invocation names which squares it wants.
  The stream is appended to the named report path, or written to stdout when no path is named.
  The frontend measures nothing, so the only values it writes into a record are provenance and the aggregate across repeat processes.
  A report file accumulates the cells of many invocations, since the frontend appends and never truncates.

- **The flattened stream.** What the record stream becomes when the invocation asks for it: one JSONL line per cell.
  A leaderboard consumes this shape, so its schema is a contract with a reader outside this workspace.
  `aqpbm-core` owns that schema, beside the per-square one, since both are serialised report shapes.
  The frontend chooses the shape and where it lands, and invents no field.

- **Terminal output.** Stdout carries machine-readable output only: records, and the table a listing flag prints.
  Everything a human reads while waiting goes to stderr.
  That covers which cell is starting, which repeat is running, and which request has no comparator.
  The split is load-bearing, because a repeat parent parses a child's stdout as records, and drivers pipe stdout into a report file.
  A repeat child writes to stdout whatever the invocation asked for, so only the parent honours the report path.

- **Exit status.** Zero when the cell was measured and every output was written.
  Every failure gets one non-zero code, with the stderr message naming the cause.
  An unresolvable request, a catalog rejection, an unwritable output, and a repeat child exiting non-zero all fail the invocation.
  Asking for accuracy against a row with no comparator is the one downgrade: the row runs timed, with a note on stderr.

## 4. Interfaces

### 4.1 The command surface

One subcommand per bundle and measuring kind, plus the tools that name no bundle.

- **`sketchbench`** measures one cell of the sketch bundle and writes its record stream.

- **`sketchprofile`** measures the same cell with hardware counters and sampling profilers.
  It shares the cell-selection options and adds instrument options of its own.

- **`workload`** generates and inspects synthetic `.bin` files through `aqpbm-datagen`.

Enumerating what a bundle offers is a flag on that bundle's subcommand, and it prints and exits.
It selects no cell and writes no record, so it is not a peer of measurement.
Choosing the flattened shape is a flag too, since it decides how one cell's records are written.

A second bundle adds one subcommand per measuring kind, and changes nothing else on this surface.
`workload` is the one tool that stays fixed as bundles come and go.

### 4.2 The invocation surface

One invocation is one cell, so every option selects a single value.
`--help` owns every default and the option-by-option reference.
What follows is what each group decides.

- **Identity.** Which algorithm, and which implementation of it.
  The pair must resolve in the catalog of the bundle the subcommand named.

- **Construction.** One parameter point, written as `key=value` with one value per key.
  A list of points is refused, because a list would be a sweep and a sweep is many cells.

- **Workload.** A data file, a generator spec file, or a stream described inline.
  They rank in that order, and whichever wins answers the question alone.
  The inline options build the same generator spec a spec file would, so all three paths converge on one generator.

- **Repetition.** How many runs loop inside one process, how many are discarded ahead of them, and how many repeat processes run.
  Runs are discarded first because a cold allocator and a cold cache measure the wrong thing.

- **Measurement content.** Which squares run, and therefore which lines the invocation writes.
  Two options name them, one for the operations and one for the metrics, and the squares are the cross product.

- **Output.** The file the record stream is appended to, and stdout when no file is named.
  One option here picks the flattened shape over the per-square one, since both describe the same cell.

Two environment variables complete the surface.

- `BENCH_WARMUP_SECS` sets the CPU ramp, and the frontend supplies ten seconds only when it is unset.
- `APPROXBENCH_REPEAT_CHILD` marks a child process, and a repeat parent is the only thing that sets it.
  A process that finds it set measures once and spawns nothing.

### 4.3 The flattening rule

One cell's records become one row, and the row carries the cell's identity once: algorithm, impl, language, construction point, workload and mode.

The row holds one slot per operation, and a metric is a field inside a slot.
A square is named by both, so a slot keyed on either name alone would hold two squares at once.

Each record joins the slot its operation names.
Two squares of one operation share a slot and combine field by field, since each fills the fields its own metric produced.
A second value for a field one metric owns — `throughput_items_per_sec`, `latency_ns`, `accuracy`, `merge_folds_per_sec`, `merge_shards`, `merge_supported` — is refused by field name, since nothing legitimately produces that field twice for one operation.
`cpu_time_ms`, `wall_time_ms`, `rss_peak_kb` and `heap_allocated_kb` ride along with every square of an operation regardless of which metric drove it, so a field already holding one of those keeps it: the two squares carry a reading each and neither is the other's correction.

Every metric field carries the name of the operation it was measured over.
A metric only one operation ever produces keeps that prefix too, so a reader never has to know which squares ran.
Identity fields are written once and unprefixed, as are the readings a sketch has one of however many squares measured it.

A record whose operation has no slot fails the run.

### 4.4 A worked invocation

```sh
approxbench sketchbench \
    --algorithm cms --impl oxide \           # the row: count-min, as the sketch_oxide crate implements it
    --config 'rows=5 cols=32768' \           # the construction point: a 5 x 32768 counter matrix
    --workload zipf --zipf-s 1.1 \           # generated keys, Zipf-skewed with exponent 1.1, a typical traffic skew
    --size 1000000 --cardinality 100000 \    # a million items drawn from a hundred thousand distinct keys
    --runs 10 --warmup-runs 3 \              # ten measured runs, after three discarded ones
    --operations insert,query \              # what the metric is measured over
    --metrics throughput                     # crossed with the operations: two squares, so two records
```

No report path is given, so records land on stdout while progress lands on stderr.
Redirecting stdout yields clean JSONL.

`scripts/example_config_override.py` walks the same surface as a runnable tour: what each family's knobs are, that they reach the structure, that an odd point is honoured exactly, and what a row says when it cannot build at the point it was given. -->
