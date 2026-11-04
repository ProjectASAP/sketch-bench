use anyhow::{bail, Result};
use aqpbm_core::metrics::MetricsMask;
use aqpbm_core::TrialMode;
use aqpbm_datagen::{splitmix64, GeneratedTable, KeyOffset, TableDescription};
use sketch_bench::params::{seeded_rows, takes_seed, ParamSet};

use crate::cli::SketchbenchArgs;

const KEY_OFFSET_STEP: u64 = 1 << 40;

pub fn mix(a: u64, b: u64) -> u64 {
    splitmix64(a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

pub fn derived_seed(seed: u64, trial: u64) -> u64 {
    if trial == 0 {
        seed
    } else {
        mix(seed, trial)
    }
}

pub fn requested(args: &SketchbenchArgs) -> bool {
    args.trials != 1 || args.trial_start.is_some() || args.trial_mode.is_some()
}

pub fn key_columns(comparator: Option<&str>, description: &TableDescription) -> Vec<usize> {
    let value = description.column_spec.len().saturating_sub(1);
    match comparator.unwrap_or("none") {
        "frequency" | "cardinality" | "topk" | "heavy-hitter" | "membership" => vec![value],
        name if name.starts_with("keyed-") => vec![0],
        "subpop-rank-error" | "subpop-cdf-error" | "subpop-sum" => (0..value).collect(),
        name if name.starts_with("subpop-") => (0..=value).collect(),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn check(
    args: &SketchbenchArgs,
    metrics: MetricsMask,
    mode: TrialMode,
    variant: &str,
    library: &str,
    comparator: Option<&str>,
    key_columns: &[usize],
    description: Option<&TableDescription>,
) -> Result<()> {
    if args.trials == 0 {
        bail!("--trials must be >= 1");
    }
    if !requested(args) {
        return Ok(());
    }
    if args.repeat_experiment > 1 {
        bail!(
            "--trials is the accuracy axis and --repeat-experiment the throughput one; \
             ask for one of them"
        );
    }
    if metrics != MetricsMask::ACCURACY {
        bail!(
            "--trials varies what accuracy is scored over, so --metrics has to be accuracy alone"
        );
    }
    let Some(description) = description else {
        bail!("--trials: an external workload has no seeds or keys a trial could draw again");
    };
    let comparator = comparator.unwrap_or("none");
    match mode {
        TrialMode::Relabel if key_columns.is_empty() => bail!(
            "--trial-mode relabel moves keys, and {variant}/{library} is scored by \
             {comparator}, which scores values; use fresh"
        ),
        TrialMode::Reseed if !takes_seed(variant) => bail!(
            "--trial-mode reseed sets the row's seed field, and {variant}/{library} has none; \
             the rows with one are {}",
            seeded_rows().join(", ")
        ),
        _ => {}
    }
    if mode != TrialMode::Reseed {
        if let Some(&column) = key_columns
            .iter()
            .find(|&&c| description.column_spec[c].key_offset.is_some())
        {
            bail!(
                "--trials: column '{}' already carries a key_offset, and a trial sets its own",
                description.column_label[column]
            );
        }
    }
    Ok(())
}

pub fn reseeded(description: &TableDescription, trial: u64) -> TableDescription {
    let mut description = description.clone();
    for column in &mut description.column_spec {
        let seed = derived_seed(column.distribution.seed(), trial);
        column.distribution.set_seed(seed);
    }
    description
}

fn key_offset(data_type: &str, trial: u64) -> Result<Option<KeyOffset>> {
    if trial == 0 {
        return Ok(None);
    }
    if data_type == "string" {
        return Ok(Some(KeyOffset::Prefix(format!("{trial}:"))));
    }
    match trial.checked_mul(KEY_OFFSET_STEP) {
        Some(offset) => Ok(Some(KeyOffset::Add(offset))),
        None => bail!("trial {trial}: its key offset, {trial}*2^40, does not fit a u64"),
    }
}

pub fn relabel(
    description: &mut TableDescription,
    table: &mut GeneratedTable,
    key_columns: &[usize],
    trial: u64,
) -> Result<()> {
    for &column in key_columns {
        let spec = &mut description.column_spec[column];
        if let Some(offset) = key_offset(&spec.data_type, trial)? {
            offset.apply(&mut table.data[column])?;
            spec.key_offset = Some(offset);
        }
    }
    Ok(())
}

pub fn reseeded_params(params: &ParamSet, trial: u64) -> ParamSet {
    params.with_seed(derived_seed(params.seed().unwrap_or(0), trial))
}

pub fn seeds(description: &TableDescription, burst_seed: u64, params: &ParamSet) -> String {
    let columns: Vec<String> = description
        .column_spec
        .iter()
        .map(|column| column.distribution.seed().to_string())
        .collect();
    let mut out = format!(
        "column seeds [{}], burst seed {burst_seed}",
        columns.join(", ")
    );
    if let Some(seed) = params.seed() {
        out.push_str(&format!(", sketch seed {seed}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_datagen::{ColumnData, ColumnSpec, DataDistribution, ZipfParameter, RULE_NONE};

    fn column(data_type: &str, seed: u64) -> ColumnSpec {
        ColumnSpec {
            distribution: DataDistribution::Zipf(ZipfParameter {
                skewness: 1.1,
                population_size: 100,
                seed,
            }),
            shift: None,
            cardinality: None,
            special_rule: RULE_NONE,
            data_type: data_type.into(),
            string: None,
            child_of: None,
            fan_out: None,
            scale_by: None,
            scale_range: None,
            key_offset: None,
        }
    }

    #[test]
    fn trial_zero_keeps_every_seed_and_later_trials_mix_them() {
        let description = TableDescription::single("key", column("i64", 42), 100);
        assert_eq!(reseeded(&description, 0), description);
        let seed = reseeded(&description, 3).column_spec[0].distribution.seed();
        assert_eq!(seed, mix(42, 3));
        assert_ne!(seed, 42);
    }

    #[test]
    fn relabel_matches_generating_with_the_offset_written_out() {
        for data_type in ["i64", "u64", "f64", "string"] {
            let base = TableDescription::single("key", column(data_type, 7), 500);
            let mut description = base.clone();
            let mut table = base.generate().unwrap();
            relabel(&mut description, &mut table, &[0], 5).unwrap();
            assert!(description.column_spec[0].key_offset.is_some());
            assert_eq!(description.generate().unwrap(), table, "{data_type}");
            assert_ne!(base.generate().unwrap(), table, "{data_type}");
        }
    }

    #[test]
    fn an_f64_key_past_two_to_the_fifty_three_is_refused() {
        let mut column = ColumnData::Float64(vec![1.0]);
        let err = KeyOffset::Add(1 << 53).apply(&mut column).unwrap_err();
        assert!(err.to_string().contains("2^53"), "{err}");
    }

    #[test]
    fn value_scored_comparators_have_no_key_columns() {
        let description = TableDescription::single("key", column("i64", 1), 10);
        assert_eq!(key_columns(Some("frequency"), &description), vec![0]);
        assert!(key_columns(Some("rank-error"), &description).is_empty());
        assert!(key_columns(Some("relative-value-error"), &description).is_empty());
    }
}
