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

The larger legacy harnesses (`cpp/`, `rust/`, `accuracy/`, `throughput/`,
`run_all_benchmarks.sh`) are *not* under `legacy/` yet — they still have
relative-path coupling with `input/`, `sketch-bench/`, and external sibling
repos. They will move once their pipelines are absorbed into `sketchlib`
(see `docs/MERGE_PLAN.md`).
