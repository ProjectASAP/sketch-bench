//! `asap_sketchlib::CMSHeap` wrappers — a Count-Min sketch paired with a
//! top-k heap. A separate algorithm from `cms`: it takes a different knob set
//! (`heap`, the heap's capacity, and `topk_k`, the answered `k`, default
//! `sketchlib::TOPK_K`)
//! and answers two different questions (per-key frequency, and top-k), so it
//! gets two registry rows per backend rather than one.

pub mod sketchlib;

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use crate::params::{CmsHeapParams, ParamSet};
    use std::collections::HashMap;
    use std::rc::Rc;

    /// `deny_unknown_fields` turns a stray `top_k` in `--config` into a named
    /// error: the answered `k`'s knob is `topk_k`.
    #[test]
    fn top_k_is_not_a_config_field() {
        let bad = ParamSet {
            variant: "cms-heap-fastpath-vector2d".into(),
            params: serde_json::json!({"rows": 3, "cols": 256, "top_k": 64}),
        };
        let err = build_cms_heap_lib_vector2d_fast(&bad)
            .err()
            .expect("an unknown `top_k` field must be refused, not silently ignored");
        assert!(err.to_string().contains("top_k"), "{err}");
    }

    /// The one piece of logic unique to this wrapper: reading `HHItem`s off the
    /// heap and translating each back to `(i64, u64)`. A generous shape against
    /// a handful of low-cardinality keys makes a collision astronomically
    /// unlikely, so an exact match pins the translation, not CMS's approximation.
    #[test]
    fn topk_query_reports_exact_counts_for_a_small_stream() {
        let params = ParamSet::of(&CmsHeapParams {
            rows: 5,
            cols: 4096,
            heap: None,
            topk_k: None,
        });
        let items = Rc::new(vec![1i64, 1, 1, 2, 2, 3]);
        let probes = Rc::new(vec![()]);
        let mut passes = query_cms_heap_lib_vector2d_fast_topk(&params, items, probes, 1)
            .expect("canonical dimensions build");
        let (answers, _footprint) = passes.remove(0)();
        let ranked: HashMap<i64, u64> = answers[0].iter().copied().collect();
        assert_eq!(ranked, HashMap::from([(1, 3), (2, 2), (3, 1)]));
    }

    /// `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
    /// zero-row matrix builds happily and then answers every query out of an
    /// empty fold — the same failure mode `cms`'s Vector2D rows guard against.
    #[test]
    fn degenerate_shapes_are_refused_by_name() {
        for (rows, cols) in [(0usize, 256usize), (3, 0), (0, 0)] {
            let p = ParamSet::of(&CmsHeapParams {
                rows,
                cols,
                heap: None,
                topk_k: None,
            });
            assert!(
                build_cms_heap_lib_vector2d_fast(&p).is_err(),
                "fastpath {rows}x{cols}"
            );
            assert!(
                build_cms_heap_lib_vector2d_regular(&p).is_err(),
                "regularpath {rows}x{cols}"
            );
        }
    }

    /// Footprint is `rows * cols` counters at the real width plus a full heap
    /// of `heap` residents: items, digests and the position index (#146).
    #[test]
    fn footprint_is_counters_plus_heap() {
        let p = ParamSet::of(&CmsHeapParams {
            rows: 5,
            cols: 2048,
            heap: Some(128),
            topk_k: None,
        });
        let heap = crate::wrappers::hh_heap_footprint(128);
        let fast = build_cms_heap_lib_vector2d_fast(&p).expect("5x2048 is a valid shape");
        assert_eq!(
            memory_cms_heap_lib_vector2d_fast(&fast),
            5 * 2048 * std::mem::size_of::<i32>() + heap
        );
        let regular = build_cms_heap_lib_vector2d_regular(&p).expect("5x2048 is a valid shape");
        assert_eq!(
            memory_cms_heap_lib_vector2d_regular(&regular),
            5 * 2048 * std::mem::size_of::<i32>() + heap
        );
    }

    /// A resident costs more than its `HHItem`: its digest and its share of
    /// the position index too.
    #[test]
    fn the_footprint_grows_by_more_than_an_item_per_resident() {
        let at = |heap| {
            let p = ParamSet::of(&CmsHeapParams {
                rows: 3,
                cols: 256,
                heap: Some(heap),
                topk_k: None,
            });
            memory_cms_heap_lib_vector2d_fast(&build_cms_heap_lib_vector2d_fast(&p).unwrap())
        };
        let item = std::mem::size_of::<asap_sketchlib::input::HHItem>();
        for (small, large) in [(32, 128), (512, 2048)] {
            assert!(
                at(large) - at(small) > (large - small) * item,
                "{small} -> {large}"
            );
        }
    }

    /// A heap smaller than the `k` it answers can't answer, and is refused.
    #[test]
    fn a_heap_below_k_is_refused() {
        let p = ParamSet::of(&CmsHeapParams {
            rows: 3,
            cols: 256,
            heap: Some(TOPK_K - 1),
            topk_k: None,
        });
        let err = build_cms_heap_lib_vector2d_fast(&p).err().expect("refused");
        assert!(err.to_string().contains("heap="), "{err}");
    }

    /// A heap larger than `k` still answers its heaviest `k` (`TOPK_K` by
    /// default).
    #[test]
    fn a_larger_heap_answers_its_heaviest_k() {
        let params = ParamSet::of(&CmsHeapParams {
            rows: 5,
            cols: 4096,
            heap: Some(4 * TOPK_K),
            topk_k: None,
        });
        // Key i appears i times, for 2k keys: the heaviest k are k+1..=2k.
        let keys = 2 * TOPK_K as i64;
        let stream: Vec<i64> = (1..=keys)
            .flat_map(|i| std::iter::repeat_n(i, i as usize))
            .collect();
        let mut passes =
            query_cms_heap_lib_vector2d_fast_topk(&params, Rc::new(stream), Rc::new(vec![()]), 1)
                .expect("builds");
        let (answers, _) = passes.remove(0)();
        let mut answered: Vec<i64> = answers[0].iter().map(|&(k, _)| k).collect();
        answered.sort_unstable();
        assert_eq!(answered, (TOPK_K as i64 + 1..=keys).collect::<Vec<_>>());
    }

    /// `topk_k` sets the answered `k`: a heap of 4·10 answers its heaviest 10,
    /// and the heap defaults to `k`.
    #[test]
    fn topk_k_sets_the_answered_k() {
        let answer = |heap, k: usize| {
            let params = ParamSet::of(&CmsHeapParams {
                rows: 5,
                cols: 4096,
                heap,
                topk_k: Some(k),
            });
            let keys = 4 * k as i64;
            let stream: Vec<i64> = (1..=keys)
                .flat_map(|i| std::iter::repeat_n(i, i as usize))
                .collect();
            let mut passes = query_cms_heap_lib_vector2d_fast_topk(
                &params,
                Rc::new(stream),
                Rc::new(vec![()]),
                1,
            )
            .expect("builds");
            let (answers, _) = passes.remove(0)();
            let mut answered: Vec<i64> = answers[0].iter().map(|&(key, _)| key).collect();
            answered.sort_unstable();
            (answered, answered_k(&params))
        };
        let (answered, k) = answer(Some(40), 10);
        assert_eq!(k, 10);
        assert_eq!(answered, (31..=40).collect::<Vec<_>>());
        let (answered, _) = answer(None, 100);
        assert_eq!(answered.len(), 100);
        // A heap below topk_k is refused.
        let p = ParamSet::of(&CmsHeapParams {
            rows: 3,
            cols: 256,
            heap: Some(50),
            topk_k: Some(100),
        });
        assert!(build_cms_heap_lib_vector2d_fast(&p).is_err());
    }
}
