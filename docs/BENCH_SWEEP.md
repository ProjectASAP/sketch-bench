# `approxbench bench` — config-sweep extension

> Status: **landed**. See the "Changed for the user" section of the PR for the final surface.

Extends the existing `approxbench bench` command so it can sweep a sketch algorithm's configuration space in one invocation — instead of hard-coding params in `sketch-cli/src/params.rs`, rebuilding, and looping by hand.

Scope is deliberately small:
- **No new subcommand.** Sweep is the default behaviour of `bench`.
- **No new input formats.** Synthetic `uniform` / `zipf` only. CSV / file / binary inputs tracked separately as a follow-up.
- **No presets, no YAML, no manifest sidecar.** One default grid per algorithm; one `--config` flag to override.

---

## 1. Why

Today `approxbench bench --sketch hll --impl oxide` bakes `HLL_PRECISION=14` (from `params.rs`) into the wrapper. To compare HLL across `lg_k ∈ {10, 12, 14, 16}` you must edit, rebuild, and re-run — per value, by hand. Same for every other algorithm.

---

## 2. Command surface

The existing flags stay. Two new things:

```
approxbench bench \
    --sketch hll \                                # required
    [--impl oxide | oxide,datasketches | all]     # NEW — optional, default = all
    --workload zipf --size 1000000 --zipf-s 1.1 \
    --runs 10 --warmup-runs 3 \
    [--config 'lg_k=10,12,14,16']                 # NEW — optional, override default grid
    [--metrics throughput,latency,memory,accuracy]
    [--report out.jsonl]
```

### 2.1 Sweep semantics

Given `--sketch X`:
- `--impl` omitted or `all` → every registered impl for algorithm `X`.
- `--impl a,b` → only those impls.
- `--impl a` → only that impl (same as today).

Given those impls:
- `--config` omitted → use the **default grid** for algorithm `X` (§3).
- `--config 'k=v1,v2 ...'` → Cartesian product of the listed values. Keys must belong to algorithm `X`'s param set (unknown key → error).
- `--config 'k=v'` (single value per key) → single config — reproduces today's behaviour exactly.

Output is one JSONL record per `(impl, config)` pair, appended to `--report`.

### 2.2 Dropped from the earlier proposal

- `--preset small|medium|large` → one default grid per algorithm is enough for v1. If the default is too big / small, the user passes `--config`.
- `--dry-run` / `--progress` / `--fail-fast` → nice to have, add later if needed. v1 fails loudly on the first error.
- `--config-file` → YAML is tracked as a separate TODO item; not needed for the core use case.
- Sweep manifest sidecar → over-engineering for v1. The JSONL records already contain the full config.

### 2.3 Backward compatibility

Today's invocation `bench --sketch hll --impl oxide ...` currently emits **1 record** with `lg_k=14`. With this change it would emit **N records** covering the default grid.

That's a soft break for anyone scripting against the single-record assumption. Two ways to land it:

- **(A) Break — simpler.** Old behaviour is reproducible with `--config 'lg_k=14'`. One-line migration.
- **(B) No break.** Require a new `--sweep` flag; without it, fall back to `params.rs` defaults as today.

Proposal: **(A)** — matches "make sweep the default, easy path." The existing single-shot behaviour is an edge case after this change (ad-hoc one-off runs), and `--config 'k=v'` is one token longer than today.

---

## 3. Default grids (per algorithm)

Straw-man — tune these to match the paper's plots. One grid per algorithm; used when `--config` isn't given. Medium-sized on purpose: ~5–15 configs so a full sweep fits in a couple minutes at `size=1M`.

| algorithm | param(s) | default grid | count |
|---|---|---|---|
| `hll` | `lg_k` | `{10, 12, 14, 16}` | 4 |
| `kll` | `k` | `{100, 200, 400, 800}` | 4 |
| `cms` | `rows × cols` | `rows ∈ {3,5,7}, cols ∈ {1024, 2048, 4096}` | 9 |
| `countsketch` | `rows × cols` | same as `cms` | 9 |
| `elastic` | `buckets × depth` | `buckets ∈ {512, 1024, 2048}, depth ∈ {2, 3, 4}` | 9 |
| `nitro` | `rate` | `{0.01, 0.02, 0.05, 0.10}` | 4 |
| `univmon` | `layers × max_stream` | `layers ∈ {6, 8, 10}, max_stream ∈ {128, 256, 512}` | 9 |

### 3.1 Impls that don't honour a knob

Some wrappers bake dimensions at compile time (e.g. `lib-fixedmatrix-custom-fast` is hard-coded `5 × 65538`). For those, sweep runs the single supported config and prints one stderr line explaining the skip. The JSONL stream stays clean.

---

## 4. Wrapper refactor (scope of the impl PR)

Every wrapper's ctor changes from zero-arg to algorithm-specific params:

```rust
// before
CmsOxide::new()
// after
CmsOxide::new(&CmsParams { rows: 5, cols: 2048 })
```

`ParamSet` lives in `sketch-core::config` — `{algorithm, params}`, with the params type per algorithm implementing `SketchParams`, `serde` round-tripping. `params.rs` becomes a `fn default_params_for(algorithm) -> ParamSet` helper used by the single-config path.

Impact: all 21 wrappers get a signature change. Mechanical, one commit.

---

## 5. Output JSONL

Same v1 schema. One new optional field:

```json
{
  "schema_version": 1,
  "sketch": "hll",
  "impl": "oxide",
  "sketch_config": { "lg_k": 14 },              // NEW — the ParamSet used
  "workload": {"shape": "zipf", "s": 1.1, "size": 1000000},
  "mode": "bench",
  "runs": 10,
  "bench": { ... }
}
```

`sketch_config` is optional on read (backward compat with any pre-existing records). The visualization layer gets one extra grouping key: `(sketch, impl, sketch_config)`. Drop-in for the existing viewer.

---

## 6. Implementation phases

1. **`sketch-core::config::ParamSet`** — typed per-algorithm enum + serde + unit tests.
2. **Wrapper refactor** — 21 wrappers switch to `new(&ParamSet)`; `params.rs` becomes a defaults helper. `bench` keeps working with today's CLI (falls back to default single config).
3. **`--config` parser + default grids** — Cartesian expansion, unknown-key error, unit-tested.
4. **Sweep loop in `bench`** — iterate `(impl × config)`, emit one record per pair.

Estimated ~600 LOC total. No viz changes required in-scope; the new `sketch_config` field is optional.

---

## 7. Open questions

Only two worth confirming before coding:

1. **Backward compat (§2.3): (A) break or (B) add `--sweep` flag?** Proposal: (A). "Simple CLI" is the user's north star; today's single-shot is trivially reachable with `--config 'k=v'`.

2. **Default grid sizes (§3).** The table above is a straw-man. Which configs does the paper actually want plotted? Answer pins the defaults; picking now avoids churn later.

Everything else defers to the impl PR.


## `--config` requires every key

`--config` no longer fills in an axis you leave out. `--config 'rows=5'` for
cms is an error naming the missing field, rather than silently pairing your
`rows` with a built-in `cols` default:

```
Error: --config: bad parameter: cms params: missing field `cols`
```

Omitting `--config` entirely still sweeps the algorithm's default grid. The
change is deliberate: a partially-specified grid produced a config the
operator never wrote, under a report that looked fully specified.
