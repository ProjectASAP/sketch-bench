# `aqpbm-planeval` design

`aqpbm-planeval` is still being implemented, and this doc will keep changing.

The crate `aqpbm-planeval` is trying to answer one question:
if a query is approximated, does the application still make the same decisions.

This is phase 2 of aqpbm.
The query is planned and executed by `ASAPPlanner`.
The use cases are described in `approx-bench-use-cases.md`.

This design doc of `aqpbm-planeval` contains the following contents:

- input
- how a query is run
- output
- fidelity
- how to run

## Input

Input is a spec file in `aqpbm-planeval/workloads/`.
There is one spec for each use case.
A spec has two parts: the data and the queries.

### Data

Data is generated in memory from the spec.
Each series has a baseline distribution.
A phase changes the distribution for a period of time, for example an attack or a load surge.

### Queries

Each query is a PromQL.
It is evaluated every interval over a sliding window.
A query can also carry a threshold that the application uses.

### Arm

An arm is one setting of the planner.
The exact arm plans with no accuracy target.
An approx arm plans with an accuracy target, for example `--max-error 0.01`.
An arm can also say that raw samples are not kept.

## How a Query is Run

`ASAPPlanner` plans the query for the arm.
`ASAPPlanner`'s executor runs the plan at every evaluation time.
Both run in the same process as `aqpbm-planeval`.
A plan can keep partial results as materialized views, refreshed as data arrives.

## Output

`planeval` writes one JSON file per arm.
It has the answer of every query at every evaluation time.
It also has the plan: the chosen sketches, their sizes and the predicted memory.
For each sketch, `asap_sketchlib` gives its theoretical error bound and memory.

## Fidelity

Fidelity compares the approx arm with the exact arm.
It is measured on the application, not only on the query result.
For example, does the alert fire at the same evaluations.
Each application has its own script in `scripts/`.
The relative error of each query result is also recorded.

## How to Run

```bash
cargo build --release -p aqpbm-planeval
python3 scripts/run_use_case.py 4
tools/grafana/run.sh
```

`run_use_case.py` runs the exact arm and every approx arm of one use case.
The results go to `target/grafana-data`.
Grafana shows one dashboard per application and one overview.
