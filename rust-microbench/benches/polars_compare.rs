use asap_sketchlib::{
    Count, CountMin, DDSketch, DataInput, ErtlMLE, FastPath, FixedMatrix, HyperLogLogP12, KLL,
};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use polars::prelude::*;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const DATA_PATH: &str = "../input/benchmark_data_10m_int64_zipf_s11_k500000.bin";
const NUM_PERCENTILES: usize = 101;
const KLL_K: i32 = 200;
const DDSKETCH_ALPHA: f64 = 0.01;

struct BenchData {
    ints: Vec<i64>,
    floats: Vec<f64>,
    distinct_keys: Vec<i64>,
    quantile_exprs: Vec<Expr>,
}

fn bench_data() -> &'static BenchData {
    static DATA: OnceLock<BenchData> = OnceLock::new();
    DATA.get_or_init(|| {
        let ints = load_dataset(Path::new(DATA_PATH)).unwrap_or_else(|err| {
            panic!(
                "failed to load {}: {err}. generate it with `cargo run --bin generate_zipf_data --release`",
                DATA_PATH
            )
        });
        let floats = ints.iter().map(|&value| value as f64).collect::<Vec<_>>();
        let mut distinct_keys = ints.clone();
        distinct_keys.sort_unstable();
        distinct_keys.dedup();

        BenchData {
            ints,
            floats,
            distinct_keys,
            quantile_exprs: build_quantile_exprs(),
        }
    })
}

fn load_dataset(path: &Path) -> Result<Vec<i64>, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if bytes.is_empty() || bytes.len() % std::mem::size_of::<i64>() != 0 {
        return Err(format!("bad dataset size: {}", path.display()));
    }

    let mut values = Vec::with_capacity(bytes.len() / std::mem::size_of::<i64>());
    for chunk in bytes.chunks_exact(std::mem::size_of::<i64>()) {
        values.push(i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]));
    }
    Ok(values)
}

fn build_quantile_exprs() -> Vec<Expr> {
    (0..NUM_PERCENTILES)
        .map(|percentile| {
            let rank = percentile as f64 / 100.0;
            col("v")
                .quantile(lit(rank), QuantileMethod::Linear)
                .alias(format!("p_{percentile}"))
        })
        .collect()
}

fn build_i64_df(values: &[i64]) -> DataFrame {
    DataFrame::new(vec![Column::new("v".into(), values)]).expect("build int64 dataframe")
}

fn build_f64_df(values: &[f64]) -> DataFrame {
    DataFrame::new(vec![Column::new("v".into(), values)]).expect("build float64 dataframe")
}

fn frequency_compare(c: &mut Criterion) {
    let data = bench_data();
    let values = &data.ints;
    let keys = &data.distinct_keys;
    let df = build_i64_df(values);
    let keys_df = build_i64_df(keys);

    let mut countmin_prefilled = CountMin::<FixedMatrix, FastPath>::default();
    let mut count_prefilled = Count::<FixedMatrix, FastPath>::default();
    for &value in values {
        let input = DataInput::I64(value);
        countmin_prefilled.insert(&input);
        count_prefilled.insert(&input);
    }

    let mut group = c.benchmark_group("polars_frequency_compare");

    group.bench_function("countmin_insert", |b| {
        b.iter_with_setup(CountMin::<FixedMatrix, FastPath>::default, |mut sketch| {
            for &value in values {
                sketch.insert(&DataInput::I64(value));
            }
            black_box(sketch);
        });
    });

    group.bench_function("count_insert", |b| {
        b.iter_with_setup(Count::<FixedMatrix, FastPath>::default, |mut sketch| {
            for &value in values {
                sketch.insert(&DataInput::I64(value));
            }
            black_box(sketch);
        });
    });

    group.bench_function("polars_exact_groupby_insert", |b| {
        b.iter(|| {
            let result = build_i64_df(values)
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
                .collect()
                .expect("polars frequency insert");
            black_box(result);
        });
    });

    group.bench_function("countmin_estimate_all_distinct_keys", |b| {
        b.iter(|| {
            let mut total = 0i64;
            for &key in keys {
                total += countmin_prefilled.estimate(&DataInput::I64(key)) as i64;
            }
            black_box(total);
        });
    });

    group.bench_function("count_estimate_all_distinct_keys", |b| {
        b.iter(|| {
            let mut total = 0i64;
            for &key in keys {
                total += count_prefilled.estimate(&DataInput::I64(key)) as i64;
            }
            black_box(total);
        });
    });

    group.bench_function("polars_lazy_join_query", |b| {
        b.iter(|| {
            let counts_lazy = df
                .clone()
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")]);
            let result = keys_df
                .clone()
                .lazy()
                .join(
                    counts_lazy,
                    [col("v")],
                    [col("v")],
                    JoinArgs::new(JoinType::Left),
                )
                .collect()
                .expect("polars lazy frequency query");
            black_box(result);
        });
    });

    group.bench_function("polars_eager_join_query", |b| {
        b.iter(|| {
            let counts_df = df
                .group_by(["v"])
                .expect("polars eager group_by")
                .select(["v"])
                .count()
                .expect("polars eager count");
            let result = keys_df
                .left_join(&counts_df, ["v"], ["v"])
                .expect("polars eager left_join");
            black_box(result);
        });
    });

    group.finish();
}

fn cardinality_compare(c: &mut Criterion) {
    let data = bench_data();
    let values = &data.ints;
    let df = build_i64_df(values);
    let value_col = Column::new("v".into(), values);
    let value_series = value_col.as_materialized_series();

    let mut hll_prefilled = HyperLogLogP12::<ErtlMLE>::default();
    for &value in values {
        hll_prefilled.insert(&DataInput::I64(value));
    }

    let mut group = c.benchmark_group("polars_cardinality_compare");

    group.bench_function("hll_insert", |b| {
        b.iter_with_setup(HyperLogLogP12::<ErtlMLE>::default, |mut sketch| {
            for &value in values {
                sketch.insert(&DataInput::I64(value));
            }
            black_box(sketch);
        });
    });

    group.bench_function("polars_exact_n_unique_insert", |b| {
        b.iter(|| {
            let result = build_i64_df(values)
                .lazy()
                .select([col("v").n_unique().alias("exact_cardinality")])
                .collect()
                .expect("polars cardinality insert");
            black_box(result);
        });
    });

    group.bench_function("hll_estimate", |b| {
        b.iter(|| {
            black_box(hll_prefilled.estimate());
        });
    });

    group.bench_function("polars_lazy_n_unique_query", |b| {
        b.iter(|| {
            let result = df
                .clone()
                .lazy()
                .select([col("v").n_unique().alias("exact_cardinality")])
                .collect()
                .expect("polars lazy cardinality query");
            let estimate = result
                .column("exact_cardinality")
                .expect("exact_cardinality column")
                .as_materialized_series()
                .cast(&DataType::Float64)
                .expect("cast cardinality result")
                .f64()
                .expect("float view")
                .get(0)
                .unwrap_or(f64::NAN);
            black_box(estimate);
        });
    });

    group.bench_function("polars_eager_n_unique_query", |b| {
        b.iter(|| {
            let estimate = value_series.n_unique().expect("series n_unique");
            black_box(estimate);
        });
    });

    group.finish();
}

fn quantile_compare(c: &mut Criterion) {
    let data = bench_data();
    let floats = &data.floats;
    let df = build_f64_df(floats);
    let quantile_exprs = data.quantile_exprs.clone();
    let value_col = Column::new("v".into(), floats);
    let value_series = value_col
        .as_materialized_series()
        .cast(&DataType::Float64)
        .expect("cast quantile series");

    let mut kll_prefilled = KLL::<f64>::init_kll(KLL_K);
    let mut dd_prefilled = DDSketch::new(DDSKETCH_ALPHA);
    for &value in floats {
        kll_prefilled.update(&value);
        dd_prefilled.add(&value);
    }

    let mut group = c.benchmark_group("polars_quantile_compare");

    group.bench_function("kll_insert", |b| {
        b.iter_with_setup(
            || KLL::<f64>::init_kll(KLL_K),
            |mut sketch| {
                for &value in floats {
                    sketch.update(&value);
                }
                black_box(sketch);
            },
        );
    });

    group.bench_function("ddsketch_insert", |b| {
        b.iter_with_setup(
            || DDSketch::new(DDSKETCH_ALPHA),
            |mut sketch| {
                for &value in floats {
                    sketch.add(&value);
                }
                black_box(sketch);
            },
        );
    });

    group.bench_function("polars_exact_quantiles_insert", |b| {
        b.iter(|| {
            let result = build_f64_df(floats)
                .lazy()
                .select(quantile_exprs.clone())
                .collect()
                .expect("polars quantile insert");
            black_box(result);
        });
    });

    group.bench_function("kll_query_p0_to_p100", |b| {
        b.iter(|| {
            let mut acc = 0.0;
            for percentile in 0..NUM_PERCENTILES {
                acc += kll_prefilled.quantile(percentile as f64 / 100.0);
            }
            black_box(acc);
        });
    });

    group.bench_function("ddsketch_query_p0_to_p100", |b| {
        b.iter(|| {
            let mut acc = 0.0;
            for percentile in 0..NUM_PERCENTILES {
                acc += dd_prefilled
                    .get_value_at_quantile(percentile as f64 / 100.0)
                    .unwrap_or(f64::NAN);
            }
            black_box(acc);
        });
    });

    group.bench_function("polars_lazy_quantile_query", |b| {
        b.iter(|| {
            let result = df
                .clone()
                .lazy()
                .select(quantile_exprs.clone())
                .collect()
                .expect("polars lazy quantile query");
            black_box(result);
        });
    });

    group.bench_function("polars_eager_quantile_query", |b| {
        b.iter(|| {
            let estimates = (0..NUM_PERCENTILES)
                .map(|percentile| {
                    value_series
                        .f64()
                        .expect("float quantile view")
                        .quantile(percentile as f64 / 100.0, QuantileMethod::Linear)
                        .expect("series quantile")
                        .unwrap_or(f64::NAN)
                })
                .collect::<Vec<_>>();
            black_box(estimates);
        });
    });

    group.finish();
}

fn benchmark_config() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(8))
}

criterion_group!(
    name = polars_compare_benches;
    config = benchmark_config();
    targets = frequency_compare, cardinality_compare, quantile_compare
);
criterion_main!(polars_compare_benches);
