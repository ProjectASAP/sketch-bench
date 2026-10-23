//! Synthetic data generation — see `docs/aqpbm-datagen.md`. In-memory only: no
//! files, no workspace deps. Four separable axes: distribution ([`dist`]), rule
//! ([`rule`]), rendering ([`value`]), relation ([`table`]).

pub mod burst;
pub mod column;
pub mod dist;
pub mod error;
pub mod rule;
pub mod table;
pub mod value;

mod splitmix64;
#[cfg(test)]
mod test_support;

pub use burst::{inject_interval_bursts, BurstSpec};
pub use column::{ColumnSpec, DATA_TYPES};
pub use dist::{
    DataDistribution, Domain, NormalParameter, ParetoParameter, Sampler, UniformParameter,
    ZipfParameter,
};
pub use error::DataGenError;
pub use rule::{RULE_MASK_ALL, RULE_MONOTONIC_INCREASE, RULE_NONE};
pub use table::{GeneratedTable, TableDescription};
pub use value::{ColumnData, ColumnItem, StrCfg, StringOpts};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{column, normal, one, uniform, zipf};

    fn pareto(alpha: f64, scale: f64, seed: u64) -> DataDistribution {
        DataDistribution::Pareto(ParetoParameter { alpha, scale, seed })
    }

    /// The MLE of the shape, `n / sum(ln(x / scale))`, recovers `alpha` from a
    /// large fixed-seed sample.
    #[test]
    fn pareto_mle_recovers_alpha() {
        let (alpha, scale) = (1.5, 2.0);
        let t = one(column(pareto(alpha, scale, 13), "f64"), 200_000)
            .generate()
            .unwrap();
        let values = t.into_column(0).unwrap().into_f64().unwrap();
        assert!(values.iter().all(|&x| x >= scale));
        let log_sum: f64 = values.iter().map(|x| (x / scale).ln()).sum();
        let alpha_hat = values.len() as f64 / log_sum;
        assert!(
            (alpha_hat - alpha).abs() / alpha < 0.05,
            "alpha_hat {alpha_hat} not within 5% of {alpha}"
        );
    }

    /// An i64 column is the f64 draws floored: every draw is >= scale > 0, so
    /// the renderer's `as` truncation is a floor. The i64-only exact quantile
    /// baseline scores a Pareto stream through this.
    #[test]
    fn pareto_i64_floors_the_f64_draws() {
        let draw = |data_type| {
            one(column(pareto(1.1, 1000.0, 7), data_type), 50_000)
                .generate()
                .unwrap()
                .into_column(0)
                .unwrap()
        };
        let floats = draw("f64").into_f64().unwrap();
        let ints = draw("i64").into_i64().unwrap();
        assert!(ints.iter().all(|&v| v >= 1000));
        assert!(ints
            .iter()
            .zip(&floats)
            .all(|(&i, f)| i == f.floor() as i64));
    }

    // ---------- child_of ----------

    fn child(parent: usize, fan_out: u64, distribution: DataDistribution) -> ColumnSpec {
        ColumnSpec {
            child_of: Some(parent),
            fan_out: Some(fan_out),
            ..column(distribution, "string")
        }
    }

    /// region (4) → service (fan-out 25, Zipf) → endpoint (fan-out 5), then a value.
    fn hier_table() -> TableDescription {
        TableDescription {
            column_num: 4,
            column_label: vec![
                "region".into(),
                "service".into(),
                "endpoint".into(),
                "value".into(),
            ],
            column_spec: vec![
                column(uniform(0.0, 4.0, 1), "string"),
                child(0, 25, zipf(25, 1.2, 2)),
                child(1, 5, uniform(0.0, 5.0, 3)),
                column(zipf(1000, 1.1, 4), "i64"),
            ],
            column_connected: Vec::new(),
            row_num: 20_000,
        }
    }

    /// Each child splits into its parent, a `.`, and an index below `fan_out`;
    /// returns the distinct children seen under each parent.
    fn children_by_parent(
        parents: &[String],
        children: &[String],
        fan_out: u64,
    ) -> std::collections::HashMap<String, std::collections::HashSet<String>> {
        let mut by_parent: std::collections::HashMap<_, std::collections::HashSet<_>> =
            Default::default();
        for (parent, child) in parents.iter().zip(children) {
            let (head, index) = child.rsplit_once('.').expect("a child has a `.`");
            assert_eq!(head, parent, "'{child}' does not start with its parent");
            assert!(index.parse::<u64>().unwrap() < fan_out, "'{child}'");
            by_parent
                .entry(parent.clone())
                .or_default()
                .insert(child.clone());
        }
        by_parent
    }

    #[test]
    fn a_child_table_is_deterministic() {
        let d = hier_table();
        assert_eq!(d.generate().unwrap(), d.generate().unwrap());
    }

    #[test]
    fn every_child_extends_its_parent_within_its_fan_out() {
        let t = hier_table().generate().unwrap();
        let region = t.data[0].as_string().unwrap();
        let service = t.data[1].as_string().unwrap();
        let endpoint = t.data[2].as_string().unwrap();

        let services = children_by_parent(region, service, 25);
        assert_eq!(services.len(), 4);
        // 20k draws over 25 Zipf(1.2) ranks reach every service under every region.
        assert!(services.values().all(|s| s.len() == 25), "{services:?}");

        let endpoints = children_by_parent(service, endpoint, 5);
        assert_eq!(endpoints.len(), 100);
        assert!(endpoints.values().all(|e| e.len() <= 5));
    }

    /// The child index is the column's own draw, ranked: the distribution is
    /// what picks it, and the parent plays no part.
    #[test]
    fn the_child_index_follows_the_columns_distribution() {
        let d = hier_table();
        let draws = d.column_spec[1].draw(d.row_num as usize).unwrap();
        let t = d.generate().unwrap();
        let service = t.data[1].as_string().unwrap();
        let mut counts = [0u64; 25];
        for (draw, s) in draws.iter().zip(service) {
            let index: u64 = s.rsplit_once('.').unwrap().1.parse().unwrap();
            assert_eq!(index, (draw - 1.0) as u64);
            counts[index as usize] += 1;
        }
        // Zipf(1.2) over 25 ranks gives rank 1 about 30% of the mass.
        let head = counts[0] as f64 / d.row_num as f64;
        assert!((0.25..0.35).contains(&head), "{head}");
        assert!(counts[0] > counts[1] && counts[1] > counts[24]);
    }

    #[test]
    fn hydra_hier_yaml_generates_its_hierarchy() {
        let d =
            TableDescription::from_path(std::path::Path::new("../configs/datagen/hydra_hier.yaml"))
                .unwrap();
        let t = d.generate().unwrap();
        let col = |i: usize| t.data[i].as_string().unwrap();
        assert_eq!(children_by_parent(col(0), col(1), 25).len(), 4);
        assert_eq!(children_by_parent(col(1), col(2), 25).len(), 100);
    }

    /// The schema-width cuts keep the first labels and the value column.
    #[test]
    fn hydra_hier_cuts_keep_their_leading_labels() {
        for (file, labels) in [("hydra_hier_d2.yaml", 2), ("hydra_hier_d3.yaml", 3)] {
            let path = format!("../configs/datagen/{file}");
            let d = TableDescription::from_path(std::path::Path::new(&path)).unwrap();
            assert_eq!(d.column_num, labels + 1, "{file}");
            let t = d.generate().unwrap();
            let col = |i: usize| t.data[i].as_string().unwrap();
            assert_eq!(children_by_parent(col(0), col(1), 25).len(), 4, "{file}");
        }
    }

    #[test]
    fn bad_child_columns_are_refused_by_name() {
        type Edit = fn(&mut TableDescription);
        let cases: Vec<(&str, Edit)> = vec![
            ("earlier column", |d| d.column_spec[1].child_of = Some(1)),
            ("earlier column", |d| d.column_spec[1].child_of = Some(9)),
            ("not a string column", |d| {
                d.column_spec[0].data_type = "i64".into()
            }),
            ("needs fan_out", |d| d.column_spec[1].fan_out = None),
            ("only to a child_of", |d| d.column_spec[1].child_of = None),
            ("fan_out must be > 0", |d| {
                d.column_spec[1].fan_out = Some(0)
            }),
            ("fan_out 24 disagrees", |d| {
                d.column_spec[1].fan_out = Some(24)
            }),
            ("bounded domain", |d| {
                d.column_spec[1].distribution = normal(0.0, 1.0, 2)
            }),
            ("has to be `string`", |d| {
                d.column_spec[1].data_type = "u64".into()
            }),
            ("whole numbers", |d| {
                d.column_spec[2].distribution = uniform(0.0, 5.9, 3)
            }),
            ("cardinality", |d| d.column_spec[1].cardinality = Some(25)),
            ("`string:` block", |d| {
                d.column_spec[1].string = Some(StringOpts::default())
            }),
        ];
        for (want, edit) in cases {
            let mut d = hier_table();
            edit(&mut d);
            let err = d.generate().unwrap_err().to_string();
            assert!(err.contains(want), "wanted '{want}' in: {err}");
        }
    }

    // ---------- scale_by ----------

    /// service (25 labels) and a latency scaled by it, next to the same
    /// latency unscaled.
    fn scaled_table() -> TableDescription {
        TableDescription {
            column_num: 3,
            column_label: vec!["service".into(), "plain".into(), "scaled".into()],
            column_spec: vec![
                column(zipf(25, 1.1, 1), "string"),
                column(pareto(2.0, 1000.0, 5), "f64"),
                ColumnSpec {
                    scale_by: Some(0),
                    scale_range: Some([1.0, 10.0]),
                    ..column(pareto(2.0, 1000.0, 5), "f64")
                },
            ],
            column_connected: Vec::new(),
            row_num: 20_000,
        }
    }

    /// Every row is its unscaled draw times one factor per label, the factors
    /// stay in the range and spread across it.
    #[test]
    fn each_label_scales_its_rows_by_one_factor_in_the_range() {
        let d = scaled_table();
        let t = d.generate().unwrap();
        assert_eq!(t, d.generate().unwrap());
        let service = t.data[0].as_string().unwrap();
        let plain = t.data[1].as_f64().unwrap();
        let scaled = t.data[2].as_f64().unwrap();
        let mut factors: std::collections::HashMap<&str, f64> = Default::default();
        for ((s, p), v) in service.iter().zip(plain).zip(scaled) {
            let f = *factors
                .entry(s)
                .or_insert_with(|| d.column_spec[2].scale_factor(s));
            assert_eq!(*v, p * f, "'{s}'");
        }
        assert_eq!(factors.len(), 25);
        let (min, max) = factors
            .values()
            .fold((f64::MAX, f64::MIN), |(a, b), &f| (a.min(f), b.max(f)));
        assert!(min >= 1.0 && max <= 10.0, "{min}..{max}");
        // 25 log-uniform factors over a decade span most of it.
        assert!(min < 2.0 && max > 5.0, "{min}..{max}");
    }

    /// The factor depends on the label and the seed only.
    #[test]
    fn the_scale_factor_is_fixed_by_label_and_seed() {
        let spec = scaled_table().column_spec[2].clone();
        assert_eq!(spec.scale_factor("abcd"), spec.scale_factor("abcd"));
        assert_ne!(spec.scale_factor("abcd"), spec.scale_factor("abce"));
        let reseeded = ColumnSpec {
            distribution: pareto(2.0, 1000.0, 6),
            ..spec.clone()
        };
        assert_ne!(spec.scale_factor("abcd"), reseeded.scale_factor("abcd"));
        let flat = ColumnSpec {
            scale_range: Some([3.0, 3.0]),
            ..spec
        };
        assert_eq!(flat.scale_factor("abcd"), 3.0);
    }

    #[test]
    fn bad_scaled_columns_are_refused_by_name() {
        type Edit = fn(&mut TableDescription);
        let cases: Vec<(&str, Edit)> = vec![
            ("earlier column", |d| d.column_spec[2].scale_by = Some(2)),
            ("earlier column", |d| d.column_spec[2].scale_by = Some(9)),
            ("'plain', which is not a string", |d| {
                d.column_spec[2].scale_by = Some(1)
            }),
            ("needs scale_range", |d| d.column_spec[2].scale_range = None),
            ("only to a scale_by", |d| d.column_spec[2].scale_by = None),
            ("has to be `f64`, not 'i64'", |d| {
                d.column_spec[2].data_type = "i64".into()
            }),
            ("has to be `f64`, not 'u64'", |d| {
                d.column_spec[2].data_type = "u64".into()
            }),
            ("0 < lo <= hi", |d| {
                d.column_spec[2].scale_range = Some([0.0, 10.0])
            }),
            ("0 < lo <= hi", |d| {
                d.column_spec[2].scale_range = Some([5.0, 2.0])
            }),
        ];
        for (want, edit) in cases {
            let mut d = scaled_table();
            edit(&mut d);
            let err = d.generate().unwrap_err().to_string();
            assert!(err.contains(want), "wanted '{want}' in: {err}");
        }
    }

    /// The eval's latency spec scales by service and leaves the labels alone.
    #[test]
    fn hydra_http_latency_scales_by_service() {
        let path = std::path::Path::new("../configs/datagen/hydra_http_latency.yaml");
        let d = TableDescription::from_path(path).unwrap();
        assert_eq!(d.column_label[1], "service");
        assert_eq!(d.column_spec[4].scale_by, Some(1));
        let mut plain = d.clone();
        plain.column_spec[4].scale_by = None;
        plain.column_spec[4].scale_range = None;
        plain.row_num = 1000;
        let mut scaled = d;
        scaled.row_num = 1000;
        let (a, b) = (plain.generate().unwrap(), scaled.generate().unwrap());
        assert_eq!(a.data[..4], b.data[..4]);
        assert_ne!(a.data[4], b.data[4]);
    }
}
