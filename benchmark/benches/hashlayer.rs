use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rand::{Rng, SeedableRng, rngs::StdRng};
use sketchlib_rust::{
    Count, CountMin, DataFusion, DefaultXxHasher, FastPath, HyperLogLog, RegularPath, SketchInput,
    Vector2D, sketch_framework::hashlayer::HashSketchEnsemble,
};

const SAMPLE_COUNT: usize = 10_000;
const RNG_SEED: u64 = 0x5eed_c0de_1234_5678;

fn build_keys() -> Vec<SketchInput<'static>> {
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    (0..SAMPLE_COUNT)
        .map(|_| SketchInput::U64(rng.random::<u64>()))
        .collect()
}

fn bench_separate_insert_three_sketches(c: &mut Criterion) {
    let keys = build_keys();

    c.bench_function("separate_insert_three_sketches", |b| {
        b.iter_with_setup(
            || {
                (
                    CountMin::<Vector2D<i32>, FastPath>::default(),
                    Count::<Vector2D<i32>, RegularPath>::default(),
                    HyperLogLog::<DataFusion>::default(),
                )
            },
            |(mut cm, mut count, mut hll)| {
                for key in &keys {
                    cm.insert(key);
                    count.insert(key);
                    hll.insert(key);
                }
                black_box((cm, count, hll));
            },
        );
    });
}

fn bench_hashlayer_insert_all(c: &mut Criterion) {
    let keys = build_keys();

    c.bench_function("hashlayer_insert_all", |b| {
        b.iter_with_setup(
            || {
                HashSketchEnsemble::new(vec![
                    CountMin::<Vector2D<i32>, FastPath>::default().into(),
                    Count::<Vector2D<i32>, FastPath>::default().into(),
                    HyperLogLog::<DataFusion>::default().into(),
                ])
                .expect("compatible sketches")
            },
            |mut layer: HashSketchEnsemble<DefaultXxHasher>| {
                for key in &keys {
                    layer.insert(key);
                }
                black_box(layer);
            },
        );
    });
}

criterion_group!(
    benches,
    bench_separate_insert_three_sketches,
    bench_hashlayer_insert_all,
);

criterion_main!(benches);
