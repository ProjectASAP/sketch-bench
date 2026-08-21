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
    merge: Option<Folds<T>>,
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
    merge: Option<Folds<T>>,
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
/// `()` — one question, the whole list, asked once — and `k` is
/// `chl::CMS_HEAP_TOP_K`, the same compile-time constant the sketch was built
/// with, so the heap's capacity and the truth it's graded against can never
/// silently disagree (see #95's design-decision comment on the registry entry).
#[allow(clippy::too_many_arguments)]
pub(super) fn topk_row<T: CountedValue>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, (), chl::TopkAnswer<T>>,
    merge: Option<Folds<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        TopkGT::<T>::over_column(chl::CMS_HEAP_TOP_K, value_column(description)),
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        None,
    )
}

/// A row answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments)]
pub(super) fn subpop_frequency_row<V: CountedValue + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), (Vec<String>, V), f64>,
    merge: Option<Folds<(String, V)>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopFrequencyGT::<V>::over_columns(vec![SCORED_LABEL_COLUMN], value_column(description)),
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
    query: QueryBody<(String, V), Vec<String>, f64>,
    merge: Option<Folds<(String, V)>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopCardinalityGT {
            group_columns: vec![SCORED_LABEL_COLUMN],
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
    query: QueryBody<(String, V), Vec<String>, f64>,
    merge: Option<Folds<(String, V)>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth<Probe = Vec<String>, Answer = f64> + 'static,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn subpop_quantile_row<V: ColumnItem + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, V)>,
    insert_step: InsertStepBody<(String, V)>,
    query: QueryBody<(String, V), (Vec<String>, f64), f64>,
    merge: Option<Folds<(String, V)>>,
    prepare: Option<PrepareBody<(String, V)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopRankErrorGT {
            group_columns: vec![SCORED_LABEL_COLUMN],
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
pub(super) fn keyed_row<G>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    insert: InsertBody<(u64, i64)>,
    insert_step: InsertStepBody<(u64, i64)>,
    query: QueryBody<(u64, i64), (), f64>,
    merge: Option<Folds<(u64, i64)>>,
    prepare: Option<PrepareBody<(u64, i64)>>,
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
        peel_keyed,
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
    merge: Option<Folds<T>>,
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
    merge: Option<Folds<I>>,
    prepare: Option<PrepareBody<I>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
{
    let (probes, score) = questions(ground_truth, &table)?;
    let items = materialise(description, table)?;
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
            (Operation::Merge, Metric::Latency) => stepped(
                merge.expect(SUPPORTED).1(&req.params, items.clone(), shards(req), n)
                    .map_err(cannot_build)?,
            ),
            (Operation::Merge, _) => timed(
                merge.expect(SUPPORTED).0(&req.params, items.clone(), shards(req), n)
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
