//! `asap_sketchlib::CMSHeap` wrappers — a Count-Min sketch paired with a
//! fixed-capacity top-k heap. A separate algorithm from `cms`: it takes a
//! different knob set (no `top_k` in `--config`, see `sketchlib::CMS_HEAP_TOP_K`)
//! and answers two different questions (per-key frequency, and top-k), so it
//! gets two registry rows per backend rather than one.

pub mod sketchlib;

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use crate::params::{CmsHeapParams, ParamSet};
    use std::collections::HashMap;
    use std::rc::Rc;

    /// `deny_unknown_fields` is what turns a stray `top_k` in `--config` into a
    /// named error rather than silently ignoring it or building at the wrong
    /// capacity — the guarantee the whole compile-time-`k` design rests on.
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
            let p = ParamSet::of(&CmsHeapParams { rows, cols });
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

    /// Footprint is `rows * cols` counters at the real width — the heap itself
    /// isn't counted, matching how the plain CMS rows don't count their own
    /// bookkeeping fields either.
    #[test]
    fn footprint_is_rows_times_cols_times_counter_width() {
        let p = ParamSet::of(&CmsHeapParams {
            rows: 5,
            cols: 2048,
        });
        let fast = build_cms_heap_lib_vector2d_fast(&p).expect("5x2048 is a valid shape");
        assert_eq!(
            memory_cms_heap_lib_vector2d_fast(&fast),
            5 * 2048 * std::mem::size_of::<i32>()
        );
        let regular = build_cms_heap_lib_vector2d_regular(&p).expect("5x2048 is a valid shape");
        assert_eq!(
            memory_cms_heap_lib_vector2d_regular(&regular),
            5 * 2048 * std::mem::size_of::<i32>()
        );
    }
}
