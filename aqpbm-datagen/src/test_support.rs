use crate::column::ColumnSpec;
use crate::dist::{DataDistribution, NormalParameter, UniformParameter, ZipfParameter};
use crate::rule::RULE_NONE;
use crate::table::TableDescription;

pub(crate) fn zipf(population_size: u64, skewness: f64, seed: u64) -> DataDistribution {
    DataDistribution::Zipf(ZipfParameter {
        skewness,
        population_size,
        seed,
    })
}

pub(crate) fn uniform(lower_bound: f64, upper_bound: f64, seed: u64) -> DataDistribution {
    DataDistribution::Uniform(UniformParameter {
        lower_bound,
        upper_bound,
        seed,
    })
}

pub(crate) fn normal(mean: f64, standard_deviation: f64, seed: u64) -> DataDistribution {
    DataDistribution::Normal(NormalParameter {
        mean,
        standard_deviation,
        seed,
    })
}

pub(crate) fn column(distribution: DataDistribution, data_type: &str) -> ColumnSpec {
    ColumnSpec {
        distribution,
        shift: None,
        cardinality: None,
        special_rule: RULE_NONE,
        data_type: data_type.into(),
        string: None,
    }
}

pub(crate) fn one(spec: ColumnSpec, row_num: u64) -> TableDescription {
    TableDescription::single("c0", spec, row_num)
}
