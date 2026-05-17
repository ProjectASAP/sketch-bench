# Profile-Guided Optimization for `sketchlib`

When `sketchlib` is built normally, its 28 MB monolithic binary runs the
asap_sketchlib `Count::<FixedMatrix, FastPath>::insert` hot loop **~75 %
slower** than the same source code compiled into a 437 KB standalone
binary. The difference is entirely LLVM's specialization decision on
that one monomorphization: in the big link unit it picks a generic
"dynamic rows" version (per-row `cmp/jb` bailout, no BMI2 `rorx`, base
pointer reloaded from stack every row); in the small link unit it picks
the fully-unrolled BMI2 path. The decision is made by LLVM's cost
model, which uses module size as a heuristic input.

Profile-Guided Optimization (PGO) replaces that heuristic with measured
ground truth, and on this codebase recovers the full ~75 % gap. This
doc records the exact recipe and the measured impact.

## What PGO is

Two-stage build, both stages use rustc's built-in PGO (a thin wrapper
over LLVM's `IRPGO`). No external tools beyond `llvm-profdata`, which
ships with the `llvm-tools-preview` rustup component.

1. **Instrumented build** (`-Cprofile-generate=<dir>`): rustc inserts a
   counter on every basic block and links in `libprofiler_builtins`.
   When the program exits it dumps `<dir>/default_*.profraw`.
2. **Profile run**: execute the instrumented binary against a workload
   that exercises the hot paths you care about. The counters end up in
   the profraw file.
3. **Merge** (`llvm-profdata merge`): combine one or more profraw files
   into a single `.profdata` in LLVM's standard PGO format.
4. **Optimised build** (`-Cprofile-use=<.profdata>`): rebuild from
   scratch. LLVM now knows which functions are hot, which branches are
   taken, and which loops have small constant trip counts, and feeds
   that into its inliner / unroller / block layout / function-splitting
   passes.

PGO does not change source, does not change algorithms, and does not
require code annotations. It just lets the cost model stop guessing.

## Prerequisites

```bash
rustup component add llvm-tools-preview
```

`llvm-profdata` lives at:

```
~/.rustup/toolchains/<toolchain>/lib/rustlib/<host-triple>/bin/llvm-profdata
```

## Recipe

The workspace uses `target-cpu=native` via
[`.cargo/config.toml`](../.cargo/config.toml). Setting `RUSTFLAGS` in the
environment **replaces** (does not merge with) that config, so every
PGO command below explicitly passes `-C target-cpu=native`. Forgetting
this silently drops BMI2 codegen from `Count::insert` and makes the
whole comparison meaningless.

### Stage 1 — instrumented build

```bash
PGO_DIR="$(pwd)/target/pgo-profiles"
rm -rf "$PGO_DIR" && mkdir -p "$PGO_DIR"

RUSTFLAGS="-C target-cpu=native -Cprofile-generate=$PGO_DIR" \
  cargo build --release -p sketch-cli \
    --target x86_64-unknown-linux-gnu
```

The explicit `--target` is needed: rustc only emits instrumented code
for the explicit target, not the host fallback path. The instrumented
`sketchlib` lands at
`target/x86_64-unknown-linux-gnu/release/sketchlib` (~57 MB —
instrumentation overhead).

### Stage 2 — profile run

Run any representative throughput workload. The convenience subcommand
`sketchlib diag-cs` runs the CountSketch `FixedMatrix + FastPath` hot
loop directly; it's the minimum sufficient workload to specialise the
single function that matters most:

```bash
BENCH_WARMUP_SECS=10 taskset -c 2 \
  ./target/x86_64-unknown-linux-gnu/release/sketchlib diag-cs \
  >/dev/null
```

For a profile that also covers KLL / HLL / CMS, run the full
throughput sweep instead:

```bash
PIN_CORE=2 scripts/run_throughput_fast.sh >/dev/null
```

Either run produces one `default_<hash>_<pid>.profraw` per process exit
inside `$PGO_DIR`.

### Stage 3 — merge

```bash
PROFDATA=~/.rustup/toolchains/$(rustc -vV | awk '/host/{print $2}')/lib/rustlib/$(rustc -vV | awk '/host/{print $2}')/bin/llvm-profdata
"$PROFDATA" merge -o target/pgo-merged.profdata "$PGO_DIR"
```

### Stage 4 — optimised build

```bash
RUSTFLAGS="-C target-cpu=native -Cprofile-use=$(pwd)/target/pgo-merged.profdata" \
  cargo build --release -p sketch-cli \
    --target x86_64-unknown-linux-gnu

cp target/x86_64-unknown-linux-gnu/release/sketchlib \
   target/release/sketchlib
```

The copy step puts the PGO binary where `scripts/run_throughput_fast.sh`
and the rest of the repo expect it. Without `--target`, the
`-Cprofile-use` build is skipped for host-only artefacts and you'd get
a non-PGO binary.

## Measured impact

Hardware: CloudLab node, single core (`taskset -c 2`), governor on
`schedutil` (the busy-loop warmup ramps it to turbo). Input:
`input/benchmark_data_1m_int64.bin`, 1 M `i64`. 10 measured trials
after 2 discarded warmup trials, 10 s `BENCH_WARMUP_SECS`.

### CountSketch `FixedMatrix + FastPath` 5 × 2048 (`diag-cs`)

| build                                | binary  | total `rorx` | hot loop                          | time / 1M | items / s |
| ------------------------------------ | ------- | ------------ | --------------------------------- | --------- | --------- |
| baseline (thin LTO)                  | 28 MB   | 4 191        | generic, `cmp $1/$3/$5,%rsi` bail | ~14.0 ms  | ~71 M     |
| inline / unroll threshold bump       | 41 MB   | 6 333        | same bailout                      | ~14.0 ms  | ~71 M     |
| **PGO**                              | **24 MB** | **2 665**  | **`rorx $0x20,...$0xf,...$0x28`, unrolled** | **~7.78 ms** | **~129 M** |
| `throughput-bench` reference (small) | 386 KB  | —            | same unrolled `rorx`              | ~8.00 ms  | ~125 M    |
| `sketch-cli-throughput` (small)      | 437 KB  | —            | same unrolled `rorx`              | ~8.03 ms  | ~125 M    |

The PGO binary runs the same monolithic `sketchlib` we already ship,
but is now **slightly faster than the standalone reference binary** on
the same hot loop, and ~45 % faster than the non-PGO build. The binary
is also ~14 % smaller than the non-PGO build because cold code paths
get outlined.

### Full `sketchlib bench --metrics throughput` sweep (PGO build)

`scripts/run_throughput_fast.sh`, 10 runs + 2 warmup runs, PIN_CORE=2.

| sketch        | impl                    | config             | mean items / s | stddev   |
| ------------- | ----------------------- | ------------------ | -------------- | -------- |
| KLL           | `lib`                   | k = 200            | 45.7 M         | 0.66 M   |
| CMS           | `lib-fixedmatrix-fast`  | rows = 5, cols = 2048 | 179.2 M     | 0.29 M   |
| CountSketch   | `lib-fixedmatrix-fast`  | rows = 5, cols = 2048 | 119.3 M     | 4.3 M    |
| HLL           | `lib`                   | lg_k = 14          | 316.2 M        | 2.2 M    |

For comparison, the pre-PGO commit measured the `sketchlib bench`
CountSketch path at ~72 M items / s. PGO recovers ~65 % more throughput
on the same source tree.

## What didn't work

Documented here so we don't retry these blind in the future.

- **Switching `[profile.release]` between `lto = "fat"` and `lto = "thin"`** — no
  effect on `Count::insert` specialization in either direction.
- **`-C llvm-args=-inline-threshold=1000 -inlinehint-threshold=1000 -unroll-threshold=500`** —
  binary grew 28 MB → 41 MB and global `rorx` count went 4 191 → 6 333,
  but the specific `Count::<FixedMatrix, FastPath>::insert` monomorphization
  still got the dynamic-rows fallback. The specialization choice is
  not gated by inline / unroll cost-model thresholds; it is gated by
  whatever module-size heuristic PGO bypasses.
- **`#[inline(never)]` on `run_diag_cs`** — isolated the symbol cleanly
  for objdump, did not change runtime.
- **`#[inline(always)]` on all wrapper `update` impls** — necessary for
  cross-crate inlining but does not reach the underlying
  `Count::insert` specialization decision.

## Operational notes

- **PGO is not in CI yet.** The default `cargo build --release` is the
  non-PGO 28 MB build; treat the PGO recipe above as a manual release
  step until / unless we automate it.
- **The profile is workload-specific.** A profile collected from
  `diag-cs` (CountSketch only) is enough to specialise that single hot
  function, but for the full four-family throughput sweep you want a
  profile run that exercises all four. The script
  `scripts/run_throughput_fast.sh` is a self-contained workload for
  that.
- **Profile staleness.** If you upgrade `asap_sketchlib`, change
  workspace layout, or bump rustc, the profile may no longer match the
  current IR. Regenerate it — a stale profile silently degrades to
  non-PGO quality without warning.
- **`--target` is required** for the PGO build steps. Without it,
  Cargo builds for the host triple via the "no explicit target" code
  path, which on stable rustc does not pick up `-Cprofile-use`
  consistently.
