use anyhow::Result;
use asap_sketchlib::analytics::{
    DdParams, HllParams, KllParams, KmvParams, MatrixHeapParams, MatrixParams, StatisticKind,
    UnivMonParams,
};
use asap_sketchlib::{
    Classic, CountMinF64, CountMinSketchWithHeap, CountSketch, CountSketchWithHeap, DdSketch,
    ErrorMetric, HyperLogLog, Kll, Kmv, NormScale, SketchAnalytics, UnivMon, Vector2D,
};
use asap_types::ir::schema::family_capability::StatisticKind as PlannedStatistic;
use asap_types::ir::schema::sketch::{SketchKind, SketchParams, SketchStatistic};
use serde_json::{json, Value};

fn metric_name(metric: ErrorMetric) -> &'static str {
    match metric {
        ErrorMetric::Absolute {
            scale: Some(NormScale::StreamL1),
        } => "absolute_l1",
        ErrorMetric::Absolute {
            scale: Some(NormScale::StreamL2),
        } => "absolute_l2",
        ErrorMetric::Absolute { scale: None } => "absolute",
        ErrorMetric::Relative => "relative",
        ErrorMetric::Rank => "rank",
    }
}

struct Readout {
    statistic: StatisticKind,
    merged_error: Option<String>,
}

fn priced<S: SketchAnalytics>(
    params: &S::Params,
    readouts: &[Readout],
    delta: Option<f64>,
) -> Value {
    let data = S::Data::default();
    let readouts: Vec<Value> = readouts
        .iter()
        .map(|readout| {
            let bound = S::theoretical_error(readout.statistic, params, &data, delta);
            json!({
                "statistic": readout.statistic,
                "merged_error": readout.merged_error,
                "error": bound.map(|b| json!({
                    "metric": metric_name(b.metric),
                    "epsilon": b.epsilon,
                    "delta": b.delta,
                    "contract": b.contract,
                })),
                "query_ops": S::theoretical_query_ops(readout.statistic, params, &data),
            })
        })
        .collect();
    json!({
        "memory_bytes": S::theoretical_memory_bytes(params, &data),
        "update_ops": S::theoretical_update_ops(params, &data),
        "merge_ops": S::theoretical_merge_ops(params, &data),
        "readouts": readouts,
    })
}

pub fn statistics(node: &Value, out: &mut Vec<PlannedStatistic>) {
    match node {
        Value::Object(map) => {
            if let Some(query) = map.get("SummaryEstimate").and_then(|e| e.get("query")) {
                if let Ok(statistic) = serde_json::from_value::<SketchStatistic>(query.clone()) {
                    if !out.contains(&statistic.kind()) {
                        out.push(statistic.kind());
                    }
                }
            }
            map.values().for_each(|value| statistics(value, out));
        }
        Value::Array(items) => items.iter().for_each(|item| statistics(item, out)),
        _ => {}
    }
}

pub fn analytics(
    sketch: &Value,
    planned_statistics: &[PlannedStatistic],
    delta: Option<f64>,
) -> Result<Value> {
    let kind: SketchKind = serde_json::from_value(sketch.clone())?;
    let capability = kind.algorithm().capability();
    let mut readouts = Vec::new();
    for planned in planned_statistics {
        if capability.answers(*planned) {
            readouts.push(Readout {
                statistic: serde_json::from_value(serde_json::to_value(planned)?)?,
                merged_error: capability.merged_error(*planned).map(|e| format!("{e:?}")),
            });
        }
    }
    let priced = match *kind.params() {
        SketchParams::Kll { k, .. } => priced::<Kll>(
            &KllParams {
                k: k as usize,
                ..Default::default()
            },
            &readouts,
            delta,
        ),
        SketchParams::DDSketch { alpha } => {
            priced::<DdSketch>(&DdParams { alpha }, &readouts, delta)
        }
        SketchParams::Hll { precision, .. } => priced::<HyperLogLog<Classic>>(
            &HllParams {
                precision: precision as usize,
            },
            &readouts,
            delta,
        ),
        SketchParams::Kmv { k, .. } => {
            priced::<Kmv>(&KmvParams { k: k as usize }, &readouts, delta)
        }
        SketchParams::Cms { width, depth } => priced::<CountMinF64>(
            &MatrixParams {
                rows: depth as usize,
                cols: width as usize,
            },
            &readouts,
            delta,
        ),
        SketchParams::CountSketch { width, depth } => priced::<CountSketch<Vector2D<f64>>>(
            &MatrixParams {
                rows: depth as usize,
                cols: width as usize,
            },
            &readouts,
            delta,
        ),
        SketchParams::CmsWithHeap {
            width,
            depth,
            heap_size,
        } => priced::<CountMinSketchWithHeap<Vector2D<f64>>>(
            &MatrixHeapParams {
                rows: depth as usize,
                cols: width as usize,
                heap_size: heap_size as usize,
            },
            &readouts,
            delta,
        ),
        SketchParams::CountSketchWithHeap {
            width,
            depth,
            heap_size,
        } => priced::<CountSketchWithHeap<Vector2D<f64>>>(
            &MatrixHeapParams {
                rows: depth as usize,
                cols: width as usize,
                heap_size: heap_size as usize,
            },
            &readouts,
            delta,
        ),
        SketchParams::UnivMon {
            heap_size,
            sketch_rows,
            sketch_cols,
            layers,
            ..
        } => priced::<UnivMon>(
            &UnivMonParams {
                heap_size: heap_size as usize,
                rows: sketch_rows as usize,
                cols: sketch_cols as usize,
                layers: layers as usize,
            },
            &readouts,
            delta,
        ),
    };
    Ok(priced)
}
