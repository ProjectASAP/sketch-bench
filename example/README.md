# Examples

Twelve requests, each run twice: once writing one record per square, once
writing the whole cell as one row.

This directory exists to be read, not merged. Everything under it is generated
by [`regenerate.sh`](regenerate.sh), so a change in the record shape shows up
here as a diff instead of as prose that quietly stopped being true.

```
./example/regenerate.sh
```

## What a square is

A measurement is named by two things: the **operation** it is taken over, and
the **metric** it reads. `--operations` and `--metrics` name a set of each, and
the run measures their cross product. Both are required.

|             | throughput | latency | accuracy |
|-------------|------------|---------|----------|
| **insert**  | items/s | per-update clock | empty |
| **query**   | items/s | per-question clock | comparator score |
| **merge**   | folds/s | time per fold | empty |
| **prepare** | empty | time to build | empty |

`cpu` and `memory` are readings that attach to whichever squares run, so they
form no square of their own. An empty square is refused by name, so
`--operations prepare --metrics throughput` is an error and not a blank result.

## Layout

```
CMS/  CS/  HLL/  KLL/
└── insert-and-query/   two operations, both metrics, cpu and memory along for the ride
    query-accuracy/     one operation, scored against ground truth
    merge/              one operation, read as a time and as a rate
        ├── command.sh     the two commands, differing only by `--flat`
        ├── records.jsonl  one record per square
        └── flat.json      the same squares as one row
```

Every leaf runs `--impl lib`, which is `asap_sketchlib`. The library exposes no
bare `cms` or `countsketch` row: its counting sketches are structural variants,
so the storage path is part of what the row is, and the algorithm named under
`CMS/` is `cms-fastpath-vector2d`.

All four use one workload, 200000 items drawn from 20000 keys at Zipf 1.1, so
numbers in different directories are taken over the same stream.

## What the two output shapes are for

`records.jsonl` names each square in its own record, under `operation` and
`metric`. `flat.json` is the shape a leaderboard reads: one slot per operation,
and a metric is a field inside a slot, so `latency_ns` becomes
`insert_latency_ns` and `query_latency_ns`.

Keying the row on either name alone would put two squares in the same place.
`CMS/insert-and-query` is the case that shows it: both operations are measured
for both metrics, and all four numbers survive.

## Three things to know before comparing numbers

**The two files in a leaf are two runs.** There is no way to emit both shapes
from one invocation, so `command.sh` runs the request twice. Timings therefore
differ between `records.jsonl` and `flat.json` by ordinary run-to-run noise.
Compare the shape, not the digits.

**Two squares of one operation are two measurements.** `merge/` runs its fold
once for the latency square and again for the throughput square, each over its
own five runs, because a square owns its own population. So `merge_time_ms` and
`merge_folds_per_sec` do not reconcile exactly. For KLL, whose fold takes 49
microseconds and varies by 9% run to run, they differ by about 9%.

**A query is not one thing.** `query_latency_ns` counts what the comparator
asked, and the comparators ask differently: HLL puts one question 4096 times,
KLL sweeps 101 quantiles, and the frequency comparators ask once per distinct
key. Reading `HLL/insert-and-query/flat.json`, insert runs at 265M items/s
while query runs at 6K/s, because one `estimate_distinct` scans all 16384
registers. Those two rates are both real and are not comparable across
directories.

## What is deterministic here

Accuracy is a function of the data and the parameters, so CMS and HLL score
identically in both runs of a leaf, to the last digit.

KLL does not. Its compaction is randomised and `asap_sketchlib` does not seed
it from the run, so `mean_rank_err` moved between 0.0016 and 0.0024 across
three runs over identical data. The four KLL accuracy fields are the only
numbers here that change when nothing else did.
