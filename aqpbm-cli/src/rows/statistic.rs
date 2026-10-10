use super::*;
use sketch_bench::wrappers::cms_heap::sketchlib as chl;

// ---------- the statistic a row answers ----------

/// What an operation a row does not have would be: the registry declares the
/// operations each row supports and the frontend checks a request against it,
/// so reaching one here is a table disagreeing with itself.
pub(super) const SUPPORTED: &str = "the registry declares the operations this row supports";

/// How many primed closures one measurement needs: its measured runs plus the
/// warm-ups thrown away before them. The run count follows the metric, which
/// [`runs_for`] is the one statement of.
pub(super) fn passes(req: &Requirement, metric: Metric) -> usize {
    req.warmup_runs + runs_for(metric, req.runs)
}

/// The shard count a fold runs at, floored where a fold stops being one.
pub(super) fn shards(req: &Requirement) -> usize {
    req.merge_shards.max(MIN_MERGE_SHARDS)
}

/// A row answering **frequency**: how often a key occurs in the stream.
#[allow(clippy::too_many_arguments)]
pub(super) fn frequency_row<T: CountedValue>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, T, u64>,
    merge: Option<Folds<T, T, u64>>,
    prepare: Option<PrepareBody<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        FrequencyGT::<T>::over_column(value_column(description)),
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **cardinality**: how many distinct keys the stream carried.
/// The probe is `()` — there is one question, asked repeatedly.
#[allow(clippy::too_many_arguments)]
pub(super) fn cardinality_row<T: ColumnItem>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, (), f64>,
    merge: Option<Folds<T, (), f64>>,
    prepare: Option<PrepareBody<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        CardinalityGT {
            column: value_column(description),
        },
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **top-k**: the heaviest `k` keys, ranked. The probe is
/// `()` — one question, the whole list, asked repeatedly for timing — and `k` is
/// the row's `topk_k` (`chl::answered_k`, default `chl::TOPK_K`), whatever its
/// heap's capacity (`heap=`): a larger heap answers its heaviest `k`.
#[allow(clippy::too_many_arguments)]
pub(super) fn topk_row<T: CountedValue>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, (), chl::TopkAnswer<T>>,
    merge: Option<Folds<T, (), chl::TopkAnswer<T>>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        TopkGT::<T>::over_column(chl::answered_k(&req.params), value_column(description)),
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        None,
    )
}

/// The label columns a grouped row asks and scores: `--group-columns`, which
/// must name at least one column, and only columns before the value column,
/// since those are the grid's key columns.
pub(super) fn group_columns(
    req: &Requirement,
    description: &TableDescription,
) -> Result<Vec<usize>, RunError> {
    let labels = value_column(description);
    if req.group_columns.is_empty() {
        return Err(RunError::Sketch(
            "--group-columns names no column; a group is taken over at least one".into(),
        ));
    }
    if let Some(&column) = req.group_columns.iter().find(|&&c| c >= labels) {
        return Err(RunError::Sketch(format!(
            "--group-columns {column}: the label columns are 0..{labels}, the ones \
             before the value column"
        )));
    }
    Ok(req.group_columns.clone())
}

/// A row answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn subpop_frequency_row<V: CountedValue + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), (GroupKey, V), f64>,
    merge: Option<Folds<(String, V), (GroupKey, V), f64>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopFrequencyGT::<V>::over_columns(
            group_columns(req, description)?,
            value_column(description),
        ),
        peel_labeled::<V>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation cardinality**: how many distinct values a
/// group holds. The statistic a Count-Min cell structurally cannot reach.
#[allow(clippy::too_many_arguments)]
pub(super) fn subpop_cardinality_row<V: ColumnItem + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), GroupKey, f64>,
    merge: Option<Folds<(String, V), GroupKey, f64>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopCardinalityGT {
            group_columns: group_columns(req, description)?,
            value_column: value_column(description),
        },
        peel_labeled::<V>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation quantile**: the ordered statistic inside a
/// group, scored in rank error.
#[allow(clippy::too_many_arguments)]
pub(super) fn subpop_vector_row<V: ColumnItem + 'static, G>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), GroupKey, f64>,
    merge: Option<Folds<(String, V), GroupKey, f64>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth<Probe = GroupKey, Answer = f64> + 'static,
    G::Truth: 'static,
{
    scored_row(
        req,
        description,
        table,
        want,
        ground_truth,
        peel_labeled::<V>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn subpop_quantile_row<V: ColumnItem + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), (GroupKey, f64), f64>,
    merge: Option<Folds<(String, V), (GroupKey, f64), f64>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopRankErrorGT {
            group_columns: group_columns(req, description)?,
            value_column: value_column(description),
        },
        peel_labeled::<V>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation CDF**: the share of a group at or below a
/// value, scored in absolute CDF error.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn subpop_cdf_row<V: ColumnItem + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), (GroupKey, f64), f64>,
    merge: Option<Folds<(String, V), (GroupKey, f64), f64>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopCdfErrorGT {
            group_columns: group_columns(req, description)?,
            value_column: value_column(description),
        },
        peel_labeled::<V>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn keyed_row<K: ColumnItem, G>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    insert: InsertBody<(K, i64)>,
    insert_step: InsertStepBody<(K, i64)>,
    query: QueryBody<(K, i64), (), f64>,
    merge: Option<Folds<(K, i64), (), f64>>,
    prepare: Option<PrepareBody<(K, i64)>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth<Probe = (), Answer = f64> + 'static,
    G::Truth: 'static,
{
    scored_row(
        req,
        description,
        table,
        want,
        ground_truth,
        peel_keyed::<K>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **quantile**, scored in rank error: the value at a fraction
/// of the sorted stream. The one statistic whose rows build at either width.
#[allow(clippy::too_many_arguments)]
pub(super) fn quantile_row<T: ColumnItem>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, f64, f64>,
    merge: Option<Folds<T, f64, f64>>,
    prepare: Option<PrepareBody<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        RankErrorGT {
            column: value_column(description),
        },
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **quantile**, scored by relative value error. DDSketch's
/// paper guarantees this metric, unlike rank error.
#[allow(clippy::too_many_arguments)]
pub(super) fn relative_value_quantile_row<T: ColumnItem>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, f64, f64>,
    merge: Option<Folds<T, f64, f64>>,
    prepare: Option<PrepareBody<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        RelativeValueErrorGT {
            column: value_column(description),
        },
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn scored_row<G, I>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    materialise: Materialise<I>,
    insert: InsertBody<I>,
    insert_step: InsertStepBody<I>,
    query: QueryBody<I, G::Probe, G::Answer>,
    merge: Option<Folds<I, G::Probe, G::Answer>>,
    prepare: Option<PrepareBody<I>>,
) -> Result<Measurements, RunError>
where
    I: Clone,
    G: GroundTruth + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
{
    let (probes, score, per_group) = questions(ground_truth, &table)?;
    let score = match &req.per_group_out {
        Some(path) => per_group_written(score, per_group, path)?,
        None => score,
    };
    let items = materialise(description, table)?;
    // What the folds cut into shards: the stream itself, or reordered so the
    // same cut deals it round-robin.
    let merged = match req.merge_split {
        MergeSplit::Contiguous => items.clone(),
        MergeSplit::Interleaved => Rc::new(interleave(&items, shards(req))),
    };
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let n = passes(req, metric);
        let body = match (operation, metric) {
            (Operation::Insert, Metric::Latency) => {
                stepped(insert_step(&req.params, items.clone(), n).map_err(cannot_build)?)
            }
            (Operation::Insert, _) => timed(
                insert(&req.params, items.clone(), n).map_err(cannot_build)?,
                items.len() as u64,
            ),
            (Operation::Query, _) => answered(
                query(&req.params, items.clone(), probes.clone(), n).map_err(cannot_build)?,
                score.clone(),
                metric,
            ),
            // Scored exactly as the query is: same probes, same comparator, and
            // the truth is still the whole stream, split across the shards.
            (Operation::Merge, Metric::Accuracy) => answered(
                merge.expect(SUPPORTED).2(
                    &req.params,
                    merged.clone(),
                    probes.clone(),
                    shards(req),
                    n,
                )
                .map_err(cannot_build)?,
                score.clone(),
                metric,
            ),
            (Operation::Merge, Metric::Latency) => stepped(
                merge.expect(SUPPORTED).1(&req.params, merged.clone(), shards(req), n)
                    .map_err(cannot_build)?,
            ),
            (Operation::Merge, _) => timed(
                merge.expect(SUPPORTED).0(&req.params, merged.clone(), shards(req), n)
                    .map_err(cannot_build)?,
                (shards(req) - 1) as u64,
            ),
            (Operation::Prepare, _) => timed(
                prepare.expect(SUPPORTED)(&req.params, items.clone(), n).map_err(cannot_build)?,
                items.len() as u64,
            ),
        };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
}

/// The scorer, also writing every group to `path` as `group_key,n_q,error`
/// whenever it scores; `error` is empty for a group that cannot be scored. Every pass answers the same, so
/// each rewrites the same rows. The file is created here, so a path that
/// cannot be written fails before anything is measured.
fn per_group_written<A: 'static>(
    score: Score<A>,
    per_group: PerGroup<A>,
    path: &std::path::Path,
) -> Result<Score<A>, RunError> {
    let fail =
        |e: std::io::Error| RunError::Sketch(format!("--per-group-out {}: {e}", path.display()));
    std::fs::File::create(path).map_err(fail)?;
    let path = path.to_path_buf();
    Ok(Rc::new(move |answers: &[A]| {
        let mut csv = String::from("group_key,n_q,error\n");
        for g in per_group(answers) {
            // A label may hold a comma or a quote; CSV quotes the field then.
            let key = if g.group.contains([',', '"', '\n']) {
                format!("\"{}\"", g.group.replace('"', "\"\""))
            } else {
                g.group
            };
            let error = g.error.map_or(String::new(), |e| e.to_string());
            csv.push_str(&format!("{key},{},{error}\n", g.n_q));
        }
        std::fs::write(&path, csv)
            .unwrap_or_else(|e| panic!("--per-group-out {}: {e}", path.display()));
        score(answers)
    }))
}

/// A row that answers **nothing**: measured but not scored. The parallel-insert
/// rows, whose worker sketches are dropped rather than asked, and whose ingest
/// is one call over the whole stream rather than a loop over it.
pub(super) fn timed_row<T: ColumnItem>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: ParallelInsertBody<T>,
) -> Result<Measurements, RunError> {
    let items = peel::<T>(description, table)?;
    let workers = req.workers.max(1);
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let body =
            match operation {
                Operation::Insert => timed(
                    insert(&req.params, workers, items.clone(), passes(req, metric))
                        .map_err(cannot_build)?,
                    items.len() as u64,
                ),
                _ => return Err(RunError::Sketch(
                    "answers no statistic and folds nothing, so insert is all it is measured over"
                        .to_string(),
                )),
            };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
}
