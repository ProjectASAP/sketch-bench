## `legacy/`

Pre-`sketchlib` artifacts kept for reference and reproducibility. Nothing here
is on the active development path — new work goes through `sketch-core` /
`sketch-bench` / `sketch-cli` / `sketch-runtime`.

| path | what it is |
|---|---|
| `other_benchmark/bench_sketch_multi.c` | orphan single-file C benchmark, no build wiring |
| `insertion_optimized_sample_output/` | archived output from the Insert-Optimized C++ variants |
| `cumulative_result.txt`, `simplified_cumulative_result.txt` | aggregated text results from old runs |
| `design_conversion_conclusion.txt` | early design notes on "binary-isolated monolithic execution" |
| `dir_structures.md` | pre-merge directory snapshot |
| `plots/` | top-level PNGs produced by `legacy/scripts/plot_rust_lib_vs_oxide_throughput.py` |
| `scripts/` | the single legacy plot script that fed `legacy/plots/` |
| `throughput/scripts/` | per-family throughput plot scripts retired from `throughput/scripts/` |
| `throughput/plots/` | archived throughput plot PNGs from the old harness |
| `accuracy/scripts/` | per-statistic accuracy plot scripts retired from `accuracy/scripts/` |
| `accuracy/plots/` | archived accuracy plot PNGs |

The per-family **Rust** throughput / accuracy crates (`throughput/<family>/rust/`,
`accuracy/<statistic>/rust/`, plus `throughput/polars_*/`, `throughput/octo/rust/`,
`accuracy/{nitro,octo}/`) were removed in the Phase-4 migration — their
functionality is fully covered by `sketchlib bench` via:

- regular impls (`oxide` / `datasketches` / `lib-*`) in `sketch-cli/src/wrappers/`
- `polars` impl per family (exact baseline via DataFrame engine)
- `lib-fastpath-parallel` impl (octo equivalent, `--workers N`)

Per-call query CSVs (hll/kll/dd) come from `--raw-csv DIR --accuracy` after
the comparator gained per-call sample capture. See `scripts/run_throughput.sh`
and `scripts/run_accuracy.sh` for the new orchestrators.

Source histories survive in git — recover any removed file with
`git log --follow -- <path>` (the last commit before retirement preserves it
verbatim). The C++ side under `throughput/<family>/cpp/` and
`accuracy/<statistic>/cpp/` is a separate migration (MERGE_PLAN Phase 8)
and stays in its original location for now.
