//! Non-dominated filtering for `(peak memory, TCO CPU, per-RQE latency)`.

use crate::objectives::Objectives;

fn dominates(a: &[f64], b: &[f64]) -> bool {
    let mut strict = false;
    for (left, right) in a.iter().zip(b) {
        if left > right {
            return false;
        }
        if left < right {
            strict = true;
        }
    }
    strict
}

pub fn pareto_front(objs: &[Objectives]) -> Vec<usize> {
    let vectors: Vec<Vec<f64>> = objs
        .iter()
        .map(|o| {
            let mut vector = vec![o.peak_query_memory_bytes, o.tco_cpu_secs_per_sec];
            vector.extend(&o.query_latency_secs);
            vector
        })
        .collect();
    (0..objs.len())
        .filter(|&i| !(0..objs.len()).any(|j| j != i && dominates(&vectors[j], &vectors[i])))
        .collect()
}
