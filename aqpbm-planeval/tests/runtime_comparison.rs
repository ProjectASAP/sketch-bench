use std::path::Path;
use std::rc::Rc;

use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::run::DataFusionRunConfig;
use aqpbm_planeval::plan::{plan_sql, Plan};
use aqpbm_planeval::record::AnswerCheck;
use aqpbm_planeval::run::{RowsFrom, RunConfig};
use aqpbm_planeval::runtimes::{
    quantities, readout_differences, readout_pairs, records_per_runtime, RuntimeRecords,
};
use aqpbm_planeval::sql::catalog_from_spec;
use asap_types::types::AccuracyTarget;

const TABLE: &str = "metrics";
const SPEC: &str = "../configs/datagen/planeval_sql_metrics.yaml";
const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);
const SEED: u64 = 0;
const ROWS: u64 = 20_000;

const QUANTILE_SQL: &str = "SELECT approx_percentile_cont(latency, 0.99) FROM metrics";
const GROUPED_SUM_SQL: &str = "SELECT service, SUM(bytes) FROM metrics GROUP BY service";
const GROUPED_QUANTILE_SQL: &str =
    "SELECT service, approx_percentile_cont(latency, 0.99) FROM metrics GROUP BY service";

fn description() -> TableDescription {
    let mut description = TableDescription::from_path(Path::new(SPEC)).expect("the spec parses");
    description.row_num = ROWS;
    description
}

fn plan_of(sql: &str, description: &TableDescription) -> Plan {
    let catalog = catalog_from_spec(TABLE, description).expect("the spec is a catalog");
    plan_sql(sql, &catalog, ACCURACY).expect("the SQL plans")
}

fn both_runtimes(sql: &str) -> RuntimeRecords {
    let description = description();
    let rows: Rc<GeneratedTable> = Rc::new(description.generate().expect("the spec generates"));
    let plan = plan_of(sql, &description);
    let config = RunConfig::new(RowsFrom::Generated(Rc::clone(&rows)), SEED, true);
    let engine = DataFusionRunConfig {
        table: TABLE.to_string(),
        description,
        rows,
        split: true,
    };
    records_per_runtime(sql, &plan, &config, &engine).expect("both runtimes run the same plan")
}

#[test]
fn both_runtimes_read_the_same_answer_out_of_the_quantile_plan() {
    let records = both_runtimes(QUANTILE_SQL);

    assert_eq!(records.interp.runtime, "interp");
    assert_eq!(records.datafusion.runtime, "datafusion");
    assert_eq!(
        records.interp.plan.plan_id, records.datafusion.plan.plan_id,
        "the comparison is only a comparison if both runtimes ran the same document"
    );
    assert_eq!(records.interp.rows_scanned, records.datafusion.rows_scanned);

    let pairs = readout_pairs(&records);
    assert_eq!(pairs.len(), 1, "one SummaryEstimate, one readout");
    assert_eq!(
        (pairs[0].interp.len(), pairs[0].datafusion.len()),
        (1, 1),
        "the readout is named the same way on both runtimes and neither repeats it: {pairs:?}"
    );

    let differences = readout_differences(&records);
    assert!(
        differences.is_empty(),
        "same plan, same rows, same seed, so the approximate and the exact answer have to \
         match bit for bit: {differences:?}"
    );
}

#[test]
fn a_plan_with_no_estimate_is_scored_on_datafusion_and_unscored_on_the_interpreter() {
    let records = both_runtimes(GROUPED_SUM_SQL);

    assert!(
        records.interp.readouts.is_empty(),
        "the interpreter emits a readout only for a SummaryEstimate, and an ExactAggregate \
         plan has none"
    );
    assert_eq!(
        records.interp.advantage().accuracy,
        None,
        "so the interpreter reports no accuracy at all for this plan"
    );
    assert_eq!(
        records.interp.answer_check,
        AnswerCheck::NoReadoutCompared,
        "its exact arm ran and its timings are real, and the record has to say on its face \
         that nothing was scored against them"
    );

    assert_eq!(
        records.datafusion.readouts.len(),
        8,
        "DataFusion scores every value column against its own arm-A column, so the eight \
         groups of an exact sum are compared: {:?}",
        records.datafusion.readouts
    );
    assert_eq!(
        records.datafusion.advantage().accuracy,
        Some(0.0),
        "an exact accumulator has to reproduce the exact answer"
    );
    assert_eq!(
        records.datafusion.answer_check,
        AnswerCheck::ScoredAgainstExact
    );

    assert!(
        readout_pairs(&records)
            .iter()
            .all(|pair| pair.interp.is_empty()),
        "the pairing is one-sided, so this plan has no cross-runtime accuracy comparison \
         even though one arm scores it"
    );
    assert!(
        readout_differences(&records).is_empty(),
        "a column only DataFusion scores is not a disagreement between the runtimes: {:?}",
        readout_differences(&records)
    );
}

#[test]
fn the_memory_numbers_the_two_runtimes_report_are_not_the_same_measurement() {
    let records = both_runtimes(QUANTILE_SQL);
    let held = quantities(&records);

    let state = held
        .iter()
        .find(|quantity| quantity.name == "state bytes")
        .expect("both runtimes report a state size");
    assert!(
        !state.same_measurement,
        "an in-process footprint and a serialized Arrow column are not the same quantity"
    );
    assert!(state.interp.is_some() && state.datafusion.is_some());

    let pool = held
        .iter()
        .find(|quantity| {
            quantity.arm == "post-ASAP" && quantity.name == "maintenance pool peak bytes"
        })
        .expect("the pool row is always present");
    assert_eq!(
        pool.interp, None,
        "the interpreter has no memory pool to read a peak from"
    );
    assert_eq!(
        pool.datafusion,
        Some("0 B".to_string()),
        "DataFusion 43 never charges an ungrouped accumulator that does not grow"
    );

    let times: Vec<_> = held
        .iter()
        .filter(|quantity| quantity.same_measurement)
        .collect();
    assert_eq!(
        times
            .iter()
            .map(|quantity| (quantity.arm, quantity.name))
            .collect::<Vec<_>>(),
        vec![("post-ASAP", "aggregate time"), ("pre-ASAP", "query time"),],
        "the post-ASAP query time is a bare sketch probe on interp and a whole read query on \
         DataFusion, so it is not one of them"
    );
    for quantity in times {
        assert!(
            quantity.interp.is_some() && quantity.datafusion.is_some(),
            "a comparable pair has both sides: {quantity:?}"
        );
    }
}

#[test]
fn every_group_of_a_utf8_keyed_quantile_agrees_on_both_runtimes() {
    let records = both_runtimes(GROUPED_QUANTILE_SQL);

    let pairs = readout_pairs(&records);
    assert_eq!(pairs.len(), 8, "the spec draws eight service names");
    for pair in &pairs {
        assert_eq!(
            (pair.interp.len(), pair.datafusion.len()),
            (1, 1),
            "each group is named once per runtime: {pair:?}"
        );
    }
    let differences = readout_differences(&records);
    assert!(
        differences.is_empty(),
        "a per-group KLL readout has to agree bit for bit across the runtimes: {differences:?}"
    );
}

#[test]
fn the_grouped_sum_charges_a_pool_peak_on_datafusion_and_none_on_the_interpreter() {
    let records = both_runtimes(GROUPED_SUM_SQL);

    assert_eq!(
        records.interp.approximate.maintenance_peak_reserved_bytes,
        None
    );
    assert!(
        records
            .datafusion
            .approximate
            .maintenance_peak_reserved_bytes
            .expect("DataFusion reports a pool peak")
            > 0,
        "a grouped aggregate resizes its reservation as the hash table grows"
    );
}
