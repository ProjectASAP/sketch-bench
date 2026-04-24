# `sketchlib bench-sweep` — design proposal

> Status: **design, not implemented**. This doc is the review artifact for a proposed CLI addition. Once approved, Phase-4 TODO gets updated and implementation lands in a follow-up PR.

Extends the offline benchmarking path with a **single-shot sweep** over a sketch family's configuration space — e.g. "run CMS 10× per config across `{width × depth}`, on a Zipf workload, and write one JSONL record per config."

Relation to existing `sketchlib bench`:
- `bench` stays as the single-point command (one impl, one config, one workload → one record).
- `bench-sweep` (new) is the many-configs command. It re-uses `BenchRunner`; the sweep happens outside the runner, in the CLI layer.

---

## 1. Why

Today the repo's 21 Rust wrappers hard-code their construction params from `sketch-cli/src/params.rs` (`CMS_ROWS=5`, `CMS_COLS=2048`, `HLL_PRECISION=14`, …). To compare a sketch across its parameter space you have to either:

- edit `params.rs`, rebuild, re-run — per config, by hand; or
- write shell loops around `sketchlib bench` with `--impl` and manually-generated `--report` paths, and there's still no way to vary the hard-coded params.

`bench-sweep` removes both — one command, one JSONL output, reproducible.

Non-goals:
- Not multi-threaded — `BenchConfig::threads` stays as-is (documented "planned, not wired").
- Not a full experiment DSL — that's the eventual YAML loader tracked separately. Start with a flat CLI; YAML can wrap it later.
- No distributed execution. Sequential within one process.

---

## 2. Command surface

```
sketchlib bench-sweep \
    --sketch cms \
    [--impl lib-vector2d-fast | all | oxide,datasketches] \
    --workload zipf --size 1000000 --zipf-s 1.1 \
    --runs 10 --warmup-runs 3 \
    [--preset small|medium|large] \
    [--config 'width=512,1024,2048 depth=3,5,7'] \
    --metrics throughput,latency,memory,accuracy \
    --report out.jsonl
```

### 2.1 Required
- `--sketch <family>` — one of `hll|kll|cms|countsketch|elastic|nitro|univmon`.
- A workload selector — one of `--workload {uniform,zipf}` (synthetic) or `--input <path>` (file-backed; see §5).

### 2.2 Impl selection (mutually exclusive, default = all)
- `--impl <name>` — sweep the config space for one impl only.
- `--impl <a>,<b>,<c>` — comma-list of impls within the family.
- `--impl all` or omit — every registered impl for the family.

### 2.3 Config selection (mutually exclusive, default = `--preset medium`)
- `--preset {small,medium,large}` — a built-in grid per family (§3).
- `--config 'k1=v1,v2 k2=v3,v4'` — explicit Cartesian product. Keys are family-specific (§3); unknown keys error out.
- `--config-file path.json` — JSON array of ParamSet objects, for full control (skips Cartesian expansion).

### 2.4 Runs & metrics
- `--runs N` / `--warmup-runs M` — passed through to `BenchRunner` once per `(impl, config)` pair.
- `--metrics ...` — same syntax as `bench`, applied uniformly.
- `--report path.jsonl` — one JSONL record per `(impl, config)` pair, appended. Default stdout.

### 2.5 Sweep ergonomics
- `--dry-run` — print the expansion (every `(impl, config)` pair) and exit without running. Lets you sanity-check the grid size before committing to a long run.
- `--fail-fast` — stop at the first failing `(impl, config)` pair (default: keep going, record the failure in the JSONL stream).
- `--progress {auto,plain,none}` — per-config progress line to stderr (default `auto` → plain if tty, none otherwise).

---

## 3. Parameter space per family

Each sketch family declares a `ParamSet` enum (serialisable to JSON) plus three presets. Impls within a family share the same `ParamSet` — the wrapper's job is to translate it into whatever its crate needs.

| family | params | preset: small | preset: medium | preset: large |
|---|---|---|---|---|
| `hll` | `lg_k ∈ [4,18]` | `{10,12}` | `{10,12,14,16}` | `{10,11,12,13,14,15,16,17,18}` |
| `kll` | `k ∈ [8,4096]` | `{100,200}` | `{100,200,400,800}` | `{50,100,200,400,800,1600,3200}` |
| `cms` | `rows ∈ [1,16], cols` (pow2) | `rows∈{3,5}, cols∈{1024,2048}` | `rows∈{3,5,7}, cols∈{512,1024,2048,4096}` | `rows∈{3,4,5,6,7}, cols∈{256,512,1024,2048,4096,8192,16384}` |
| `countsketch` | same as `cms` | same | same | same |
| `elastic` | `buckets ∈ [64,16384], depth ∈ [2,8]` | `buckets∈{512,1024}, depth∈{3}` | `buckets∈{512,1024,2048,4096}, depth∈{2,3,4}` | `buckets∈{256,512,1024,2048,4096,8192}, depth∈{2,3,4,5}` |
| `nitro` | `rate ∈ (0,1]` | `{0.01,0.05}` | `{0.01,0.02,0.05,0.10}` | `{0.005,0.01,0.02,0.05,0.10,0.25,0.5}` |
| `univmon` | `layers ∈ [4,16], max_stream ∈ pow2` | `layers∈{6,8}, max_stream∈{256}` | `layers∈{4,6,8,10}, max_stream∈{128,256,512}` | `layers∈{4,6,8,10,12,14}, max_stream∈{64,128,256,512,1024}` |

Numbers are **proposals, not final** — the review should land on preset sizes the paper actually cares about. Medium default targets "about 20 configs in ~2 min for size=1M on one impl."

### 3.1 Cartesian expansion
`--config 'rows=3,5,7 cols=1024,2048'` → 6 configs (3 × 2). Order of keys is stable; iteration is row-major so the JSONL stream is deterministic and diffable.

### 3.2 Impls that don't honor a knob
Some wrappers bake dimensions at compile time (`lib-fixedmatrix-custom-fast` is `5×65538`). Two options:

1. **Skip** — impl entry declares a `supported_params: BitSet`; sweep silently drops configs that touch unsupported knobs.
2. **Warn** — same, but emit a stderr warning per drop.

Proposal: **skip + one-line summary** ("skipped 9 `(cms, lib-fixedmatrix-custom-fast)` configs — fixed shape"). Keeps the JSONL clean, tells the user it happened.

---

## 4. Wrapper refactor (scope of the eventual impl PR)

Today: `CmsOxide::new()` → `(CMS_EPSILON, CMS_DELTA)` from `params.rs`.

Proposed: `CmsOxide::new(&CmsParams { rows, cols })` — factory takes the family's `ParamSet`. Dispatch table entry gains a `params: ParamSet` field plumbed from CLI.

Impact: all 21 wrappers change signature (mechanical — one commit). `params.rs` becomes the *default* for `bench` (single-shot) and each preset's smallest bucket.

Legacy `bench` command: add a `--config 'k=v'` flag that constructs a single `ParamSet`; when omitted, the default from `params.rs` applies — so today's invocations keep working.

---

## 5. Input data sources

Today the CLI supports `--workload uniform` and `--workload zipf`. `sketch-core::workload::FileI64` exists but isn't exposed. Proposal:

| flag | shape | notes |
|---|---|---|
| `--workload uniform` | synthetic | existing; `--size`, `--cardinality`, `--seed` |
| `--workload zipf` | synthetic | existing; `--zipf-s` |
| `--input path.bin` | binary i64 LE | already in `FileI64::load`; expose it |
| `--input path.txt --input-format newline-i64` | one int per line | plain text |
| `--input path.csv --input-format csv-i64 [--csv-column N] [--csv-has-header]` | single-column CSV | column by index; header skipping |
| `--input path.csv --input-format csv-string --csv-column N` | string column | for Elastic / UnivMon wrappers |

`--workload` and `--input` are mutually exclusive. `--size` applies to synthetic only; file-backed uses the file's row count, with optional `--limit N` to truncate.

CSV parsing: keep it minimal — use the `csv` crate, single-column extraction, no type coercion beyond `i64::from_str` / `String`. No multi-column, no computed columns. That's out of scope — if users need that, they preprocess to a single-column file.

---

## 6. Output JSONL

Same v1 schema as `bench`, one record per `(impl, config)` pair. **One new optional field** on `sketch_core::report::Record`:

```json
{
  "schema_version": 1,
  "sketch": "cms",
  "impl": "oxide",
  "sketch_config": { "rows": 5, "cols": 2048 },   // NEW — family-specific ParamSet
  "workload": { "shape": "zipf", "s": 1.1, "size": 1000000 },
  "mode": "bench",
  "runs": 10,
  "bench": { ... },
  "sweep_id": "2026-04-24T12:00:00Z-abcd1234",    // NEW — groups records from one sweep
  ...
}
```

`sketch_config` is optional (absent in today's single-shot `bench` records, backward-compatible since the schema allows unknown-to-missing on read). `sweep_id` is a random tag per sweep invocation so post-processing can group records belonging to the same sweep.

Visualization impact: one additional grouping dimension (`sketch_config`). The existing viewer already groups by `(sketch, impl)` — we extend the key to `(sketch, impl, sketch_config)`. Drop-in for the chart code; no dual-read needed.

---

## 7. Progress / failure / reproducibility

- **Progress line** (stderr): `[12/45] cms/oxide rows=5 cols=2048 — runs=10 warmup=3 … 4.1s` per config. `--progress none` suppresses.
- **Per-config failure**: caught, logged to stderr, emitted as a JSONL record with `"error": "<msg>"` on the record (instead of `bench`). `--fail-fast` changes behaviour to exit on first error.
- **Reproducibility**: single `--seed S` seeds the workload; `BenchConfig::seed` is derived per-config as `S ⊕ hash(impl, ParamSet)` so every (impl, config) in a sweep is independently reproducible without the user doing bookkeeping.
- **Manifest**: at the top of a sweep run, emit a single `{"sweep_id": "...", "expansion": [...]}` JSONL record listing every pair about to run. Lets the consumer know the planned surface before any measurement lands.

---

## 8. Interaction with existing `sketchlib bench`

No breaking changes. Mapping:

| `bench` flag | `bench-sweep` equivalent |
|---|---|
| `--impl foo` (required) | `--impl foo` (optional; default `all`) |
| no config flags | `--preset medium` (default) or `--config …` |
| single `--report` | single `--report` (multi-record) |

`bench` stays as the "minimum sketch, smallest surface" entry point — kept for ad-hoc single runs and for CI that wants one-record-per-invocation. `bench-sweep` is the workhorse for producing a result set.

---

## 9. Implementation phases (after design approval)

Order optimised so each phase is independently mergeable and the repo keeps building.

1. **`sketch-core::config::ParamSet`** — typed per-family enums + serde. No wrapper changes yet; lands alongside unit tests.
2. **Wrapper refactor** — all 21 wrappers switch to `new(&ParamSet)`; `params.rs` becomes the default-builder. `bench` keeps working (no CLI change).
3. **Preset tables** — per-family small/medium/large grids, plus `--config` parser. Unit-tested Cartesian expansion.
4. **`bench-sweep` subcommand** — CLI + sweep loop + progress + manifest. End-to-end test: HLL medium preset against a 10k Zipf.
5. **File-backed workload flags** — `--input`, CSV/newline readers. Independent, can land in parallel with 1–4.
6. **Viz update** — add `sketch_config` grouping. Smallest slice.

Estimated LOC: ~800 across steps 1–4, ~200 for step 5, ~100 for step 6.

---

## 10. Open questions (for this review)

1. **Command name**: `bench-sweep` vs. fold into `bench` with `--sweep`? Proposal: separate subcommand — clearer semantics, neither command's help text gets crowded, scripts can grep for exactly one.
2. **Default when `--preset` and `--config` are both omitted**: use `medium` preset, or require one to be explicit? Proposal: default to `medium` — the whole point is zero-friction sweeps.
3. **Impl default**: `all` or require explicit `--impl`? Proposal: `all` for a family — typical use case is cross-impl comparison, and it matches the 21-impl matrix the repo already ships.
4. **`sketch_config` schema in JSONL**: typed-per-family object (as sketched above) vs. flat `{ "key": "value", ... }` string map? Proposal: typed — keeps the viewer / downstream analytics unambiguous; cost is one enum per family, already needed for the wrappers anyway.
5. **CSV scope**: single-column only, or at least one "select by header name" convenience? Proposal: index-only for v1, header-by-name deferred.
6. **Preset sizing**: the tables in §3 are straw-men. Which preset "medium" does the paper actually want to plot? This is the most important knob in the doc.
7. **Manifest record**: JSONL-with-type-tag (mixed `{"kind":"manifest",...}` / `{"kind":"result",...}` records) vs. a sidecar `sweep.manifest.json`? Proposal: sidecar — keeps the result stream homogeneous, viewers stay simple.

Answers here pin the impl-PR scope.
