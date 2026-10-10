//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::wrappers::BuildError;
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra};

pub(crate) fn labels(group: &[Option<String>]) -> Vec<Option<&str>> {
    group.iter().map(Option::as_deref).collect()
}

/// The label columns of a `;`-joined record stream, which every record must
/// share: a record of another width is a [`BuildError`] naming `variant` and
/// both widths. An empty label is a column of its own.
pub(crate) fn label_columns<V>(items: &[(String, V)], variant: &str) -> Result<usize, BuildError> {
    let width = |key: &str| key.split(';').count();
    let Some((first, _)) = items.first() else {
        return Ok(1);
    };
    let columns = width(first);
    match items.iter().find(|(key, _)| width(key) != columns) {
        Some((key, _)) => Err(BuildError(format!(
            "{variant}: record {key:?} has {} label(s), the stream's first {columns}",
            width(key)
        ))),
        None => Ok(columns),
    }
}

/// A grid over `label_columns` key columns, as asap_sketchlib 0.3 fixes
/// them at construction.
pub(crate) fn new_hydra(
    rows: usize,
    cols: usize,
    label_columns: usize,
    cell: HydraCounter,
    variant: &str,
) -> Result<Hydra, BuildError> {
    let schema = (0..label_columns).map(|i| format!("label{i}"));
    Hydra::with_schema(rows, cols, schema, cell).map_err(|e| BuildError(format!("{variant}: {e}")))
}

/// Records one `;`-joined record, one value per key column in place. The
/// build already refused a stream of mixed widths; this is the backstop.
pub(crate) fn update(h: &mut Hydra, key: &str, value: &DataInput, variant: &str) {
    update_counted(h, key, value, None, variant)
}

/// [`update`], the value counted `count` times in each cell it reaches
/// (`None` is once): the library's integer weight.
pub(crate) fn update_counted(
    h: &mut Hydra,
    key: &str,
    value: &DataInput,
    count: Option<i32>,
    variant: &str,
) {
    let parts: Vec<&str> = key.split(';').collect();
    if let Err(e) = h.update(&parts, value, count) {
        panic!(
            "{variant}: record {key:?} has {} label(s), the grid {}: {e}",
            parts.len(),
            h.schema().len()
        );
    }
}

/// Asks `query` of the subpopulation `group` constrains: a value at each
/// grouped column, `None` (unconstrained) at the rest and past its end, so any
/// non-empty subset of the key columns can be asked. A probe wider than the
/// grid is refused.
pub(crate) fn query(h: &Hydra, group: &[Option<&str>], query: &HydraQuery, variant: &str) -> f64 {
    let width = h.schema().len();
    if group.len() > width {
        panic!(
            "{variant}: probe {group:?} names {} label(s), the grid {width}",
            group.len()
        );
    }
    let key: Vec<Option<&str>> = (0..width)
        .map(|i| group.get(i).copied().flatten())
        .collect();
    h.query_key(&key, query)
        .unwrap_or_else(|e| panic!("{variant}: probe {group:?}: {e}"))
}

/// outer grid is the one shape they have in common.
pub(crate) fn check_grid(rows: usize, cols: usize, algorithm: &str) -> Result<(), BuildError> {
    for (name, v) in [("rows", rows), ("cols", cols)] {
        if v == 0 {
            return Err(BuildError(format!("{algorithm}: {name} must be > 0")));
        }
    }
    Ok(())
}

/// Bytes the grid itself costs, on top of the counters inside the cells: every
/// cell is an enum around a sketch struct, plus the prototype `Hydra` clones
/// from. Separate from the counter bytes, so every row states both components.
pub(crate) fn grid_overhead_bytes(rows: usize, cols: usize) -> usize {
    (rows * cols + 1) * std::mem::size_of::<HydraCounter>()
}

#[cfg(test)]
mod tests {
    use crate::params::{HydraCmsParams, HydraCsParams, HydraHllParams, ParamSet};
    use crate::wrappers::hydra_cms::sketchlib::merge_query_hydra_cms;
    use crate::wrappers::hydra_cs::sketchlib::merge_query_hydra_cs;
    use crate::wrappers::hydra_hll::sketchlib::merge_query_hydra_hll;
    use crate::wrappers::{interleave, QueryPass};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    /// Records over two labels, deterministic: `r0..r5` with weights halving
    /// from 32, `s0..s4` uniform, and a value skewed towards 0 over about
    /// 2000 distinct values.
    fn records(n: usize) -> Vec<(String, i64)> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            state >> 11
        };
        (0..n)
            .map(|_| {
                let pick = (next() % 63) as u32;
                let region = (0..6).find(|&r| pick < 64 - (64 >> (r + 1))).unwrap_or(5);
                let service = next() % 5;
                let u = (next() % 10_000) as f64 / 10_000.0;
                (format!("r{region};s{service}"), (u * u * 2000.0) as i64)
            })
            .collect()
    }

    /// Every subpopulation the grid answers for: each first label alone and
    /// each pair, as the query names them (leading columns).
    fn groups(items: &[(String, i64)]) -> Vec<Vec<Option<String>>> {
        let mut out: Vec<Vec<Option<String>>> = Vec::new();
        for (key, _) in items {
            let parts: Vec<Option<String>> = key.split(';').map(|p| Some(p.to_string())).collect();
            for g in [parts[..1].to_vec(), parts] {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        out
    }

    fn in_group(key: &str, group: &[Option<String>]) -> bool {
        key.split(';')
            .zip(group)
            .all(|(a, b)| b.as_deref().is_none_or(|b| a == b))
    }

    /// Every (group, value) pair that occurs, the frequency probes.
    fn pairs(items: &[(String, i64)]) -> Vec<(Vec<Option<String>>, i64)> {
        let mut seen = HashSet::new();
        for g in groups(items) {
            for (key, v) in items {
                if in_group(key, &g) {
                    seen.insert((g.clone(), *v));
                }
            }
        }
        let mut out: Vec<_> = seen.into_iter().collect();
        out.sort();
        out
    }

    fn answers(mut passes: Vec<QueryPass<f64>>) -> Vec<f64> {
        (passes.pop().expect("one pass"))().0
    }

    // ---------- merge exactness (hydra doc §2.3) ----------

    const SHARDS: [usize; 2] = [4, 7];

    /// Asks `run` of the stream folded from every shard count, under both
    /// splits, and of the single stream: every answer must be equal, because
    /// these cells merge exactly and every cell clones one seeded template.
    fn merged_equals_single(
        items: Vec<(String, i64)>,
        run: impl Fn(Rc<Vec<(String, i64)>>, usize) -> Vec<f64>,
    ) {
        let single = run(Rc::new(items.clone()), 1);
        for k in SHARDS {
            for (split, stream) in [
                ("contiguous", items.clone()),
                ("interleaved", interleave(&items, k)),
            ] {
                assert_eq!(
                    run(Rc::new(stream), k),
                    single,
                    "{k} {split} shards answer differently from one stream"
                );
            }
        }
    }

    /// A small grid, so groups collide and the merge has contamination to
    /// carry too.
    const SMALL_W: usize = 16;

    #[test]
    fn hydra_cms_merged_from_shards_answers_as_one_stream() {
        let items = records(3_000);
        let probes = Rc::new(pairs(&items));
        let params = ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: SMALL_W,
            cell_rows: 3,
            cell_cols: 64,
        });
        merged_equals_single(items, |stream, k| {
            answers(merge_query_hydra_cms(&params, stream, probes.clone(), k, 1).unwrap())
        });
    }

    #[test]
    fn hydra_cs_merged_from_shards_answers_as_one_stream() {
        let items = records(3_000);
        let probes = Rc::new(pairs(&items));
        let params = ParamSet::of(&HydraCsParams {
            rows: 3,
            cols: SMALL_W,
            cell_rows: 3,
            cell_cols: 64,
        });
        merged_equals_single(items, |stream, k| {
            answers(merge_query_hydra_cs(&params, stream, probes.clone(), k, 1).unwrap())
        });
    }

    #[test]
    fn hydra_hll_merged_from_shards_answers_as_one_stream() {
        let items = records(3_000);
        let probes = Rc::new(groups(&items));
        let params = ParamSet::of(&HydraHllParams {
            rows: 3,
            cols: SMALL_W,
        });
        merged_equals_single(items, |stream, k| {
            answers(merge_query_hydra_hll(&params, stream, probes.clone(), k, 1).unwrap())
        });
    }

    // ---------- the §2.2 bounds, on a fixed stream ----------
    //
    // Every scored group must land inside its bound in a good row. `BETA`, the
    // Markov factor on the colliders' mass, is generous: the bounds are
    // sanity checks on the grid, not a measurement of its confidence.

    const N: usize = 20_000;
    const BETA: f64 = 8.0;
    /// Subsets every two-label record is inserted under.
    const FAN_OUT: f64 = 3.0;
    const CELL_COLS: usize = 512;

    /// Each value's fanned-out frequency `F_v`: its count over all subkeys.
    fn fanned(items: &[(String, i64)]) -> HashMap<i64, f64> {
        let mut f = HashMap::new();
        for (_, v) in items {
            *f.entry(*v).or_insert(0.0) += FAN_OUT;
        }
        f
    }

    /// Group `g`'s value frequencies `f_q(v)`.
    fn frequencies(items: &[(String, i64)], g: &[Option<String>]) -> HashMap<i64, f64> {
        let mut f = HashMap::new();
        for (_, v) in items.iter().filter(|(key, _)| in_group(key, g)) {
            *f.entry(*v).or_insert(0.0) += 1.0;
        }
        f
    }

    #[test]
    fn hydra_cms_never_under_and_within_its_bound() {
        let items = records(N);
        let probes = pairs(&items);
        let params = ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: SMALL_W,
            cell_rows: 3,
            cell_cols: CELL_COLS,
        });
        let got = answers(
            merge_query_hydra_cms(
                &params,
                Rc::new(items.clone()),
                Rc::new(probes.clone()),
                1,
                1,
            )
            .unwrap(),
        );
        let (fv, m, w) = (fanned(&items), FAN_OUT * N as f64, SMALL_W as f64);
        let eps = std::f64::consts::E / CELL_COLS as f64;
        let mut by_group = HashMap::new();
        for ((g, v), est) in probes.iter().zip(got) {
            let f = by_group
                .entry(g.clone())
                .or_insert_with(|| frequencies(&items, g));
            let (truth, n_q) = (f[v], f.values().sum::<f64>());
            assert!(est >= truth, "{g:?} {v}: {est} under {truth}");
            let bound = truth + eps * n_q + BETA * (fv[v] + eps * m) / w;
            assert!(est <= bound, "{g:?} {v}: {est} over its bound {bound}");
        }
    }

    #[test]
    fn hydra_cs_within_its_bound() {
        let items = records(N);
        let probes = pairs(&items);
        let params = ParamSet::of(&HydraCsParams {
            rows: 3,
            cols: SMALL_W,
            cell_rows: 3,
            cell_cols: CELL_COLS,
        });
        let got = answers(
            merge_query_hydra_cs(
                &params,
                Rc::new(items.clone()),
                Rc::new(probes.clone()),
                1,
                1,
            )
            .unwrap(),
        );
        let (fv, w) = (fanned(&items), SMALL_W as f64);
        let f2: f64 = fv.values().map(|f| f * f).sum();
        let eps = 1.0 / (CELL_COLS as f64).sqrt();
        // The colliders' L2 under the one Markov event `F2_C <= BETA * F2 / W`.
        let l2_c = (BETA * f2 / w).sqrt();
        let mut by_group = HashMap::new();
        for ((g, v), est) in probes.iter().zip(got) {
            let f = by_group
                .entry(g.clone())
                .or_insert_with(|| frequencies(&items, g));
            let truth = f[v];
            let l2_q = f.values().map(|x| x * x).sum::<f64>().sqrt();
            let (low, high) = (
                truth - eps * (l2_q + l2_c),
                truth + eps * l2_q + BETA * fv[v] / w + eps * l2_c,
            );
            assert!(
                (low..=high).contains(&est),
                "{g:?} {v}: {est} outside [{low}, {high}]"
            );
        }
    }

    #[test]
    fn hydra_hll_within_its_bound() {
        let items = records(N);
        let probes = groups(&items);
        let params = ParamSet::of(&HydraHllParams {
            rows: 3,
            cols: SMALL_W,
        });
        let got = answers(
            merge_query_hydra_hll(
                &params,
                Rc::new(items.clone()),
                Rc::new(probes.clone()),
                1,
                1,
            )
            .unwrap(),
        );
        let distinct = |g: &[Option<String>]| frequencies(&items, g).len() as f64;
        // `ΣD`: every subkey's distinct count, the first label, the second and
        // the pair.
        let mut by_subkey: HashMap<String, HashSet<i64>> = HashMap::new();
        for (key, v) in &items {
            let (region, service) = key.split_once(';').unwrap();
            for subkey in [format!("{region};"), format!(";{service}"), key.clone()] {
                by_subkey.entry(subkey).or_default().insert(*v);
            }
        }
        let sum_d: f64 = by_subkey.values().map(|s| s.len() as f64).sum();
        // Three standard errors of a 2^14-register HLL.
        let eps = 3.0 * 1.04 / ((1u64 << 14) as f64).sqrt();
        for (g, est) in probes.iter().zip(got) {
            let d_q = distinct(g);
            let (low, high) = (
                (1.0 - eps) * d_q,
                (1.0 + eps) * (d_q + BETA * sum_d / SMALL_W as f64),
            );
            assert!(
                (low..=high).contains(&est),
                "{g:?}: {est} outside [{low}, {high}]"
            );
        }
    }
}
