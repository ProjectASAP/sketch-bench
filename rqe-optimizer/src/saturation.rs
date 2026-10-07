//! Sketch accuracy read from the saturation study's error-vs-N curves
//! (`scripts/study_saturation.py`, #130, #140) at the number of items a
//! deployment's answer covers. Design decisions: #156.
//!
//! A deployment answering an RAQE with lookback `T` covers
//! `n(T) = card(L)/card(G) · T / scrape` items per group. CMS, CountSketch,
//! HLL and DDSketch merge exactly, so merging `T/x` panes reads the curve at
//! `n(T)` like one unmerged sketch. KLL and top-k merge lossily (#131): a
//! merged answer also needs every pane saturated, `n(x) ≥ N_sat`. The
//! remaining merge penalty at saturation is #158.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use crate::autosketch::window_adapter;
use crate::{
    table_accuracy, AccuracyDirection, AtomicCostEntry, Deployment, LabelSet, MetricFacts, Raqe,
    WorkloadFacts,
};

/// The data parameters a saturation curve is keyed by, fitted from the
/// dataset per (metric, grouping) as the worst case over windows and groups.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DataShape {
    /// Zipf skew θ of the keys inserted into one group's sketch (CMS, top-k,
    /// HLL). Describes whichever insertion mode the engine uses: once per
    /// sample, or weighted by sample value.
    pub zipf_s: f64,
    /// `K`: distinct keys per group per window.
    pub distinct_keys: f64,
    /// Pareto tail index `a` of the values (KLL, DDSketch).
    pub tail_index: f64,
}

/// Sketches whose merged answer is less accurate than one sketch over the
/// same items (#131).
const LOSSY_MERGE: &[&str] = &["kll-percall", "cms-heap-topk-fastpath-vector2d"];

/// The two runs' output directories under `--saturation-dir`. A point in
/// both is taken whole from the later one.
const RUN_DIRS: [&str; 2] = ["out_grid_1e7_cost", "out_1e9"];

/// One (sketch, config, data shape) point of the study.
#[derive(Debug, Clone, PartialEq)]
struct GridPoint {
    params: BTreeMap<String, f64>,
    /// Zipf θ, or the Pareto tail index for quantile sketches.
    shape_param: f64,
    /// `K`; `None` for quantile sketches, whose rows leave it blank.
    distinct_keys: Option<f64>,
    error_metric: String,
    /// `None`: still changing at the largest measured N.
    n_sat: Option<f64>,
    /// `(n, seed_mean_error)`, ascending in `n`.
    curve: Vec<(f64, f64)>,
}

impl GridPoint {
    /// Q6 of #156: between checkpoints, the worse neighbour; below the first,
    /// unmeasured; past the last, the plateau only if the point saturated.
    fn error_at(&self, n: f64, direction: AccuracyDirection) -> Option<f64> {
        let (first_n, _) = *self.curve.first()?;
        let &(last_n, last_error) = self.curve.last()?;
        if n < first_n {
            return None;
        }
        if n >= last_n {
            return self.n_sat.map(|_| last_error);
        }
        let above = self
            .curve
            .partition_point(|&(checkpoint, _)| checkpoint < n);
        let (checkpoint, error) = self.curve[above];
        if checkpoint == n {
            Some(error)
        } else {
            Some(worse(error, self.curve[above - 1].1, direction))
        }
    }
}

fn worse(a: f64, b: f64, direction: AccuracyDirection) -> f64 {
    match direction {
        AccuracyDirection::LowerIsBetter => a.max(b),
        AccuracyDirection::HigherIsBetter => a.min(b),
    }
}

#[derive(Debug, Clone, Default)]
pub struct SaturationCurves {
    points_by_sketch: BTreeMap<String, Vec<GridPoint>>,
}

impl SaturationCurves {
    /// Reads `saturation.csv` (for `N_sat`) and `saturation_curve.csv` from
    /// each of [`RUN_DIRS`] under `dir`.
    pub fn load(dir: &Path) -> io::Result<Self> {
        type PointKey = (String, String, String, String, String);
        let key = |row: &CsvRow| -> PointKey {
            let field = |name: &str| row[name].clone();
            (
                field("sketch"),
                field("config"),
                field("dist"),
                field("param"),
                field("cardinality"),
            )
        };
        let mut points: BTreeMap<PointKey, (String, GridPoint)> = BTreeMap::new();
        for run in RUN_DIRS {
            let run_dir = dir.join(run);
            let mut run_points = BTreeMap::new();
            for row in read_csv(&run_dir.join("saturation.csv"))? {
                let point = GridPoint {
                    params: parse_config(&row["config"]),
                    shape_param: parse_number(&row["param"])?,
                    distinct_keys: match row["cardinality"].as_str() {
                        "" => None,
                        keys => Some(parse_number(keys)?),
                    },
                    error_metric: row["error_metric"].clone(),
                    n_sat: match row["n_sat"].as_str() {
                        "not_saturated" => None,
                        n => Some(parse_number(n)?),
                    },
                    curve: Vec::new(),
                };
                run_points.insert(key(&row), (row["sketch"].clone(), point));
            }
            for row in read_csv(&run_dir.join("saturation_curve.csv"))? {
                let Some((_, point)) = run_points.get_mut(&key(&row)) else {
                    return Err(invalid(format!(
                        "{run}: curve row for a point not in saturation.csv: {row:?}"
                    )));
                };
                point.curve.push((
                    parse_number(&row["n"])?,
                    parse_number(&row["seed_mean_error"])?,
                ));
            }
            points.extend(run_points);
        }
        let mut points_by_sketch: BTreeMap<String, Vec<GridPoint>> = BTreeMap::new();
        for (sketch, mut point) in points.into_values() {
            point.curve.sort_by(|a, b| a.0.total_cmp(&b.0));
            points_by_sketch.entry(sketch).or_default().push(point);
        }
        Ok(Self { points_by_sketch })
    }

    /// The accuracy `raqe` gets from `deployment`, or `None` when it can't be
    /// read: no fitted [`DataShape`], a config or shape outside the grid, the
    /// wrong metric, too few items, or lossy-merge panes below `N_sat`.
    /// Exact accumulators keep the cost table's value. `facts` must pass
    /// [`crate::validate_facts`].
    pub fn accuracy(
        &self,
        raqe: &Raqe,
        deployment: &Deployment,
        facts: &WorkloadFacts,
    ) -> Option<f64> {
        let sketch = deployment.config.sketch.as_str();
        if sketch.starts_with("exact-") {
            return table_accuracy(raqe, deployment);
        }
        let metric_facts = &facts[&deployment.metric];
        let grouping = &deployment.grouping_labels;
        let shape = metric_facts.data_shape.get(grouping)?;
        let points = self.bracketing_points(&deployment.config, shape)?;
        let covered = items_per_group(metric_facts, grouping, raqe.lookback_ms);
        let pane = items_per_group(metric_facts, grouping, deployment.window_ms);
        let needs_saturated_panes =
            raqe.lookback_ms > deployment.window_ms && LOSSY_MERGE.contains(&sketch);
        let mut worst = None;
        for point in points {
            if point.error_metric != raqe.accuracy_metric
                || needs_saturated_panes && !point.n_sat.is_some_and(|n_sat| pane >= n_sat)
            {
                return None;
            }
            let error = point.error_at(covered, raqe.accuracy_direction)?;
            worst = Some(worst.map_or(error, |w| worse(w, error, raqe.accuracy_direction)));
        }
        worst
    }

    /// The oracle for [`crate::autosketch::plan`]: AutoSketch keeps one
    /// unmerged sketch per query window.
    pub fn autosketch_accuracy(
        &self,
        raqe: &Raqe,
        config: &AtomicCostEntry,
        facts: &WorkloadFacts,
    ) -> Option<f64> {
        let (window_ms, slide_ms) = window_adapter(raqe);
        let deployment = Deployment {
            capability: raqe.capability,
            metric: raqe.metric.clone(),
            spatial_filter: raqe.spatial_filter.clone(),
            grouping_labels: raqe.grouping_labels.clone(),
            config: config.clone(),
            window_ms,
            slide_ms,
            key_tracker: None,
        };
        self.accuracy(raqe, &deployment, facts)
    }

    /// Every grid point of `config` bracketing `shape` (Q8 of #156), or
    /// `None` when `config` isn't in the grid, `shape` lies outside it, or a
    /// bracketing point is missing.
    fn bracketing_points(
        &self,
        config: &AtomicCostEntry,
        shape: &DataShape,
    ) -> Option<Vec<&GridPoint>> {
        let params = config_params(config)?;
        let points: Vec<&GridPoint> = self
            .points_by_sketch
            .get(&config.sketch)?
            .iter()
            .filter(|point| point.params == params)
            .collect();
        let quantile = points.first()?.distinct_keys.is_none();
        let shape_params = if quantile {
            bracket(points.iter().map(|p| p.shape_param), shape.tail_index)?
        } else {
            bracket(points.iter().map(|p| p.shape_param), shape.zipf_s)?
        };
        let keys = if quantile {
            vec![None]
        } else {
            let keys = points.iter().filter_map(|p| p.distinct_keys);
            bracket(keys, shape.distinct_keys)?
                .into_iter()
                .map(Some)
                .collect()
        };
        let bracketing: Vec<&GridPoint> = points
            .into_iter()
            .filter(|p| shape_params.contains(&p.shape_param) && keys.contains(&p.distinct_keys))
            .collect();
        (bracketing.len() == shape_params.len() * keys.len()).then_some(bracketing)
    }
}

/// `card(L)/card(G) · window / scrape`: one group's items in a window.
fn items_per_group(facts: &MetricFacts, grouping: &LabelSet, window_ms: u64) -> f64 {
    let series = facts.cardinality[&facts.labels] as f64;
    let groups = facts.cardinality[grouping] as f64;
    series / groups * window_ms as f64 / facts.scrape_interval_ms as f64
}

/// The grid values equal to `target`, else the two either side of it;
/// `None` outside the grid.
fn bracket(values: impl Iterator<Item = f64>, target: f64) -> Option<Vec<f64>> {
    let mut values: Vec<f64> = values.collect();
    values.sort_by(f64::total_cmp);
    values.dedup();
    if values.contains(&target) {
        return Some(vec![target]);
    }
    let above = values.partition_point(|&v| v < target);
    (above > 0 && above < values.len()).then(|| vec![values[above - 1], values[above]])
}

/// `{"params": {"rows": 3, "cols": 1024}}` → `{rows: 3, cols: 1024}`.
fn config_params(config: &AtomicCostEntry) -> Option<BTreeMap<String, f64>> {
    config.sketch_config["params"]
        .as_object()?
        .iter()
        .map(|(name, value)| Some((name.clone(), value.as_f64()?)))
        .collect()
}

/// `"rows=3 cols=1024"` → `{rows: 3, cols: 1024}`; the grid's config strings.
fn parse_config(config: &str) -> BTreeMap<String, f64> {
    config
        .split_whitespace()
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            Some((name.to_string(), value.parse().ok()?))
        })
        .collect()
}

fn parse_number(text: &str) -> io::Result<f64> {
    text.parse()
        .map_err(|_| invalid(format!("not a number: {text:?}")))
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

type CsvRow = BTreeMap<String, String>;

// ponytail: plain comma split; the study's CSVs quote nothing (configs hold
// spaces, never commas). Use the csv crate if that changes.
fn read_csv(path: &Path) -> io::Result<Vec<CsvRow>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    Ok(lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            header
                .iter()
                .zip(line.split(','))
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{facts, raqe, METRIC};
    use crate::Capability;

    const TOPK: &str = "cms-heap-topk-fastpath-vector2d";

    /// Top-k rows=3 cols=1024 at θ ∈ {1.0, 1.2}, K ∈ {1e3, 1e5}. Precision
    /// falls with N; θ = 1.0 is worse, and K = 1e5 never saturates.
    fn curves() -> SaturationCurves {
        let point = |theta: f64, keys: f64, offset: f64, n_sat: Option<f64>| GridPoint {
            params: parse_config("rows=3 cols=1024"),
            shape_param: theta,
            distinct_keys: Some(keys),
            error_metric: "precision_at_k".into(),
            n_sat,
            curve: vec![
                (1e3, 1.0 - offset),
                (1e4, 0.95 - offset),
                (1e5, 0.9 - offset),
            ],
        };
        SaturationCurves {
            points_by_sketch: BTreeMap::from([(
                TOPK.to_string(),
                vec![
                    point(1.0, 1e3, 0.05, Some(1e4)),
                    point(1.2, 1e3, 0.0, Some(1e4)),
                    point(1.0, 1e5, 0.1, None),
                    point(1.2, 1e5, 0.08, None),
                ],
            )]),
        }
    }

    /// One group of `series` series scraped every second, shaped `shape`.
    fn workload(series: u64, shape: DataShape) -> WorkloadFacts {
        let mut facts = facts(1, series);
        let metric = facts.get_mut(METRIC).unwrap();
        metric.data_shape.insert(LabelSet::new(), shape);
        facts
    }

    fn shape(zipf_s: f64, distinct_keys: f64) -> DataShape {
        DataShape {
            zipf_s,
            distinct_keys,
            tail_index: 2.0,
        }
    }

    fn topk_raqe(lookback_ms: u64) -> Raqe {
        Raqe {
            capability: Capability::TopKByValue,
            accuracy_metric: "precision_at_k".into(),
            accuracy_direction: AccuracyDirection::HigherIsBetter,
            ..raqe(lookback_ms, lookback_ms)
        }
    }

    fn deployment(sketch: &str, window_ms: u64) -> Deployment {
        Deployment {
            capability: Capability::TopKByValue,
            metric: METRIC.into(),
            spatial_filter: String::new(),
            grouping_labels: LabelSet::new(),
            config: AtomicCostEntry {
                sketch: sketch.into(),
                sketch_config: serde_json::json!({"params": {"rows": 3, "cols": 1024}}),
                mem_bytes_per_instance: 1.0,
                insert_cpu_secs: 1.0,
                merge_cpu_secs: 1.0,
                query_cpu_secs: 1.0,
                query_accuracy: BTreeMap::from([("precision_at_k".into(), 1.0)]),
                merge_accuracy: BTreeMap::new(),
                measured_at: None,
            },
            window_ms,
            slide_ms: window_ms,
            key_tracker: None,
        }
    }

    #[test]
    fn reads_the_worse_neighbour_between_checkpoints_and_the_plateau_past_the_last() {
        let point = &curves().points_by_sketch[TOPK][1];
        let higher = AccuracyDirection::HigherIsBetter;
        assert_eq!(point.error_at(1e4, higher), Some(0.95));
        assert_eq!(point.error_at(3e4, higher), Some(0.9));
        assert_eq!(point.error_at(5e2, higher), None);
        assert_eq!(point.error_at(1e9, higher), Some(0.9));
        let unsaturated = &curves().points_by_sketch[TOPK][2];
        assert_eq!(unsaturated.error_at(1e9, higher), None);
    }

    #[test]
    fn takes_the_worst_bracketing_grid_point() {
        // 10 series × 1000 s = 1e4 items; θ = 1.1 brackets 1.0 and 1.2.
        let facts = workload(10, shape(1.1, 1e3));
        let r = topk_raqe(1_000_000);
        let value = curves().accuracy(&r, &deployment(TOPK, 1_000_000), &facts);
        assert_eq!(
            value,
            Some(0.95 - 0.05),
            "θ = 1.0 at 1e4 is worse than θ = 1.2"
        );
    }

    #[test]
    fn a_shape_outside_the_grid_or_without_a_fit_has_no_accuracy() {
        let r = topk_raqe(1_000_000);
        let d = deployment(TOPK, 1_000_000);
        assert_eq!(
            curves().accuracy(&r, &d, &workload(10, shape(0.5, 1e3))),
            None
        );
        assert_eq!(
            curves().accuracy(&r, &d, &workload(10, shape(1.0, 1e7))),
            None
        );
        assert_eq!(curves().accuracy(&r, &d, &facts(1, 10)), None);
    }

    #[test]
    fn a_metric_the_curve_did_not_measure_has_no_accuracy() {
        let facts = workload(10, shape(1.2, 1e3));
        let r = Raqe {
            accuracy_metric: "recall_at_k".into(),
            ..topk_raqe(1_000_000)
        };
        assert_eq!(
            curves().accuracy(&r, &deployment(TOPK, 1_000_000), &facts),
            None
        );
    }

    #[test]
    fn lossy_merges_need_saturated_panes() {
        // 10 items/s: 100 s panes hold 1e3 < N_sat = 1e4; 1000 s panes reach it.
        let facts = workload(10, shape(1.2, 1e3));
        let r = topk_raqe(10_000_000);
        assert_eq!(
            curves().accuracy(&r, &deployment(TOPK, 100_000), &facts),
            None
        );
        assert_eq!(
            curves().accuracy(&r, &deployment(TOPK, 1_000_000), &facts),
            Some(0.9)
        );
        // The same small panes are fine for a sketch that merges exactly.
        let mut exact_merge = curves();
        let points = exact_merge.points_by_sketch.remove(TOPK).unwrap();
        exact_merge.points_by_sketch.insert("cms".into(), points);
        assert_eq!(
            exact_merge.accuracy(&r, &deployment("cms", 100_000), &facts),
            Some(0.9)
        );
    }

    #[test]
    fn exact_accumulators_keep_the_cost_table_value() {
        let r = topk_raqe(1_000_000);
        let value = curves().accuracy(&r, &deployment("exact-sum", 1_000_000), &facts(1, 10));
        assert_eq!(value, Some(1.0));
    }

    #[test]
    fn load_takes_a_point_in_both_runs_from_the_1e9_run() {
        let dir = std::env::temp_dir().join(format!("rqe-saturation-{}", std::process::id()));
        let header = "family,sketch,config,dist,param,cardinality";
        for (run, n_sat, last) in [
            (RUN_DIRS[0], "not_saturated", "1e7"),
            (RUN_DIRS[1], "1e8", "1e9"),
        ] {
            std::fs::create_dir_all(dir.join(run)).unwrap();
            let point = "quantile,kll-percall,k=200,pareto,2.0,";
            std::fs::write(
                dir.join(run).join("saturation.csv"),
                format!(
                    "{header},n_sat,final_error,error_metric\n{point},{n_sat},0.01,mean_rank_err\n"
                ),
            )
            .unwrap();
            std::fs::write(
                dir.join(run).join("saturation_curve.csv"),
                format!("{header},n,seed_mean_error,seed_se\n{point},1000,0.05,0\n{point},{last},0.01,0\n"),
            )
            .unwrap();
        }
        let loaded = SaturationCurves::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let points = &loaded.points_by_sketch["kll-percall"];
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].n_sat, Some(1e8));
        assert_eq!(points[0].curve, vec![(1e3, 0.05), (1e9, 0.01)]);
        assert_eq!(points[0].distinct_keys, None);
        assert_eq!(points[0].params, BTreeMap::from([("k".to_string(), 200.0)]));
    }
}
