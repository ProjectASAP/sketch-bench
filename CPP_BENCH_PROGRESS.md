# C++ track (`cpp-bench/`) build-out progress

Tracks the multi-phase rollout designed against `REORG_NOTES.md` §5 and
the design conversation on branch `chore/legacy-reorg`. Goal: a second
benchmark track (C++ + Google Benchmark + Apache DataSketches) that
emits the same v1 JSONL `Record` as `sketch-cli`, so the two stacks
become directly comparable through one report file.

## Decisions (locked in)

- **Position**: new `cpp-bench/` parallel to `sketch-cli`; old `cpp/`
  stays until parity is verified.
- **Workload**: shared `input/*.bin` pre-generated files take priority;
  same-RNG generation can be added later.
- **Metrics (phase 1)**: throughput + latency + accuracy. Memory and
  query-stage throughput are explicit non-goals for now.
- **Orchestrator**: Python script (`scripts/run_all.py`), independent
  of both tracks.
- **Schema contract**: a `Language` enum on the v1 `Record` is the
  only source-of-truth tag; everything else (`impl`, `accuracy.*`)
  must match byte-for-byte across the two tracks.

## Build / test environment caveat

The agent host running these edits does **not** have `cmake`, Google
Benchmark, or Apache DataSketches headers installed. C++ code is
written to the project's existing patterns (`cpp/CMakeLists.txt`,
`cpp/kll/kll_datasketches.cpp`) but **not compile-verified here**. Rust
schema changes are verified via `cargo test -p sketch-core`; Python is
syntax-checked via `python3 -m py_compile`.

Each phase commit notes what was and wasn't verified.

## Phases

- [x] **Phase 0 — schema lock** (`sketch-core`)
  - Add `Language { Rust, Cpp }` enum to `Record`.
  - Add `Source::CppBench` variant.
  - Keep `SCHEMA_VERSION = 2`; both additions have serde defaults so
    pre-existing JSONL still deserialises.
  - Write `docs/archive/SCHEMA_V1.md` describing the cross-language contract
    (especially the shape of the `accuracy` field).
- [x] **Phase 1 — cpp-bench skeleton + first binary**
  - `cpp-bench/CMakeLists.txt` (FetchContent fallback for Google
    Benchmark, reuses `INSERT_OPT_ROOT` headers like `cpp/` does).
  - `cpp-bench/common/`: `record_v1.{hpp,cpp}`, `workload.{hpp,cpp}`,
    `runner.{hpp,cpp}`, `cli.{hpp,cpp}`.
  - `cpp-bench/kll/datasketches_kll.cpp` — first end-to-end binary,
    emits v1 JSONL with throughput + latency.
- [x] **Phase 2 — accuracy parity**
  - `cpp-bench/common/accuracy.{hpp,cpp}`: ground-truth exact baseline
    + relative-error helpers, output keys match what the Rust
    `accuracy` payload uses.
  - Wire into `datasketches_kll.cpp`.
- [x] **Phase 3 — more sketches**
  - Migrate the headline cross-language comparison points:
    `kll_final` (insert-optimized), `cs_datasketches`, `cs_final`.
  - Insert-optimized micro-variants (e.g. `kll_no_min_max`) stay in
    `cpp/` for now — they're research artefacts, not parity targets.
- [x] **Phase 4 — orchestrator**
  - `scripts/run_all.py`: runs `sketchlib bench …` for the Rust track
    and the `cpp-bench` binaries, concatenates stdout into one JSONL.
  - **Family-name alignment**: Rust dispatch calls the family
    `countsketch`, so cpp-bench's CS binaries emit
    `Record.sketch = "countsketch"` too — even though they live in
    `cpp-bench/cs/` for parity with `cpp/cs/`. The orchestrator's
    `CPP_DIR_FOR_SKETCH` maps the family name to its on-disk dir.
  - Dry-run verified: `python3 scripts/run_all.py --dry-run …` prints
    the 4 Rust + 4 C++ commands with valid impl names from
    `sketch-cli/src/dispatch.rs`.
- [x] **Phase 5 — legacy cleanup**
  - `cpp-bench/legacy/{throughput,accuracy}/` (the pre-migration
    per-family C++ harnesses) has been deleted. The hll + cms variants
    are now `cpp-bench/{hll,cms}/datasketches_<family>.cpp` on
    `cpp-bench/common/`; `cms32k` is reachable via `--k 32768` on the
    cms binary. The standalone `cpp/` tree of Insert-Optimized variants
    is still in place and is *not* the same as the deleted legacy tree;
    moving it under a `legacy/` namespace remains a follow-up if/when
    parity is verified.

## Follow-ups (out of scope for this rollout)

- Memory metric on the C++ side — needs heap profiler + `/usr/bin/time -v` parser.
- Query-stage throughput — covered separately by `REORG_NOTES` gap §3-5.
- Same-RNG workload path (skip the `.bin` file).
- Per-(key, seed) error CSV stream — covered by `REORG_NOTES` gap §3-6.
