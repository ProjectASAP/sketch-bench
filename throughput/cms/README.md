# CMS Throughput

This variant mirrors the `accuracy/cms` structure, but measures insertion throughput instead of error.

Compared implementations:

- Rust `sketchlib-rust`
- Rust `datasketches`
- C++ Apache DataSketches `count_min_sketch`

Shared assets:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `throughput/scripts/`
- plots: `throughput/plots/cms/`

Fixed benchmark configuration:

- rows: `5`
- cols: `2048`
- runs: `10` seeded runs (`1..10`)
- metric: insertion-only throughput

## Run

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp throughput/scripts/run_throughput_all.sh cms
```

To capture CPU usage while the Rust and C++ throughput stages run:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp python3 throughput/scripts/run_throughput_with_cpu.py --variant cms
```

## Outputs

- merged CSV: `throughput/cms/output/cms_throughput_results.csv`
- plot: `throughput/plots/cms/cms_throughput_insertion.png`
- CPU samples: `throughput/cms/output/cms_throughput_cpu_rust.csv`
- CPU samples: `throughput/cms/output/cms_throughput_cpu_cpp.csv`
- CPU summary: `throughput/cms/output/cms_throughput_cpu_summary.csv`
