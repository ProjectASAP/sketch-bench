//! Count-Min wrappers — five types. Each answers a `&i64` point lookup with a
//! `u64` count estimate, which is the shape `FrequencyGT` scores; each says so
//! in its own `SketchOps` at the bottom of this file rather than by
//! implementing a shared trait.
//!
//! All of them take `(rows, cols)` and honour it, by four different routes.
//! `oxide` inverts the error bounds its API takes and checks the table it got
//! back. `datasketches` range-checks the `(u8, u32)` its API narrows to.
//! `CmsLibVector2dFast` / `CmsLibVector2dRegular` size at run time.
//! `CmsLibFixedmatrix<M>` is generic over a storage type that bakes the shape
//! in, so the shape selects a monomorphisation from the table in
//! `wrappers::fixed_matrix` and the registry dispatches on it.

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod datasketches;
pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// error bounds and derives the dimensions back out of them.
///
/// The inversion has to land on the library's own arithmetic, which is
/// `width = ceil(2/ε).next_power_of_two()` and `depth = ceil(ln(1/δ))`. Solving
/// for `ceil(2/ε) = cols` gives `ε = 2/cols`, and the half-step below keeps the
/// quotient off the integer boundary where one float ulp would tip the `ceil`
/// to `cols + 1` and the power-of-two rounding would then double the table.
/// That is exactly the bug the CountSketch side of this pair had.
///
/// This is a claim about `sketch_oxide` 0.1.6, so nothing rests on it being
/// right: [`require_resolved_shape`] checks the built sketch and refuses if the
/// library resolved the request to anything else.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = 2.0 / (cols as f64 - 0.5);
    let delta = (-(rows as f64 - 0.5)).exp();
    (epsilon, delta)
}



#[cfg(test)]
mod tests {
    use aqpbm_core::init::InitSketch;
    use aqpbm_core::memory_footprint::MemoryFootprint;
    use aqpbm_core::config::ParamSet;
    use super::datasketches::*;
    use super::oxide::*;
    use super::sketchlib::*;
    use crate::wrappers::cs::oxide::CsOxide;
    use super::*;

    fn shape() -> ParamSet {
        ParamSet::of(&CmsParams {
            rows: 5,
            cols: 2048,
        })
    }

    /// `sketch_oxide` stores its counters as `Vec<u64>`. Sizing them at 4 bytes
    /// halved the reported footprint, which is the number the accuracy-vs-memory
    /// plots divide by.
    #[test]
    fn oxide_sizes_counters_at_the_real_width() {
        let sketch = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        assert_eq!(sketch.memory_bytes(), 5 * 2048 * 8);
    }

    /// A `cols` the crate cannot resolve exactly is refused, naming the table it
    /// would have built.
    ///
    /// This used to build: `cols = 3000` and `cols = 4096` both allocated 4096
    /// columns, scored identical error, and were recorded as two different
    /// configs. That put one measurement at two x-positions 27% apart on every
    /// accuracy-vs-memory plot. A refusal is the only honest answer, since the
    /// error bound the API takes cannot express 3000 columns.
    #[test]
    fn oxide_cms_refuses_a_cols_it_cannot_resolve_exactly() {
        let Err(err) = CmsOxide::init(&ParamSet::of(&CmsParams {
            rows: 5,
            cols: 3000,
        })) else {
            panic!("cols=3000 resolves to a 4096-wide table, so it must be refused");
        };
        let err = err.to_string();
        assert!(err.contains("3000") && err.contains("4096"), "{err}");
    }

    /// The same for CountSketch's depth floor: the crate takes a median across
    /// rows and refuses to do it over fewer than 3, so `rows = 2` cannot be
    /// honoured and is refused instead of quietly building 3.
    #[test]
    fn oxide_countsketch_refuses_a_depth_below_its_floor() {
        let Err(err) = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 2,
            cols: 2048,
        })) else {
            panic!("rows=2 resolves to a 3-row table, so it must be refused");
        };
        assert!(err.to_string().contains("2048"), "{err}");
    }

    /// One `--config`, one counter budget, across the two oxide rows.
    ///
    /// This is the property the ε inversions exist to hold. It did not hold
    /// before: CountSketch asked through `ε = sqrt(3/cols)`, `3/ε²` did not
    /// round-trip in `f64`, `ceil` took it to `cols + 1` and the power-of-two
    /// rounding doubled it, so `cols = 2048` built 2048 columns of Count-Min and
    /// 4096 of CountSketch. A CMS-vs-CountSketch comparison at one config was
    /// comparing two budgets.
    #[test]
    fn the_two_oxide_rows_resolve_one_config_to_one_shape() {
        let cms = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 5,
            cols: 2048,
        }))
        .expect("5x2048 is a valid oxide shape");

        // 8 bytes per counter on both sides: `table: Vec<u64>` and `Vec<i64>`.
        // Sizing either as 32-bit once halved a reported footprint and made one
        // algorithm look twice as space-efficient at identical measured accuracy.
        assert_eq!(cms.memory_bytes(), 5 * 2048 * 8);
        assert_eq!(cs.memory_bytes(), 5 * 2048 * 8);
    }

    /// Every power-of-two width in the range a sweep would walk resolves
    /// exactly, on both rows. The inversion is arithmetic on floats, so the
    /// property worth pinning is that it holds across the range and not just at
    /// the one shape the tests above happen to use.
    #[test]
    fn every_power_of_two_shape_round_trips_on_both_oxide_rows() {
        for lg in 3..=16u32 {
            let cols = 1usize << lg;
            for rows in 3..=8usize {
                let cms = CmsOxide::init(&ParamSet::of(&CmsParams { rows, cols }))
                    .unwrap_or_else(|e| panic!("cms {rows}x{cols}: {e}"));
                assert_eq!(cms.memory_bytes(), rows * cols * 8, "cms {rows}x{cols}");
                let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
                    rows,
                    cols,
                }))
                .unwrap_or_else(|e| panic!("countsketch {rows}x{cols}: {e}"));
                assert_eq!(cs.memory_bytes(), rows * cols * 8, "countsketch {rows}x{cols}");
            }
        }
    }

    /// The datasketches API takes `(u8, u32)` and asserts inside C++, so every
    /// bound has to be checked on this side. Each value below reproduced a
    /// distinct failure before the guards existed.
    #[test]
    fn datasketches_refuses_what_its_api_cannot_take() {
        let build = |rows: usize, cols: usize| CmsDatasketches::init(&ParamSet::of(&CmsParams { rows, cols }));
        for (rows, cols, why) in [
            (0usize, 1024usize, "aborted inside C++: num_hashes must be at least 1"),
            (256, 1024, "`as u8` made it 0, then the same abort"),
            (257, 1024, "`as u8` made it 1: a one-row sketch labelled 257"),
            (5, 2, "aborted: num_buckets must be at least 3"),
            (5, 4_294_967_296, "`as u32` made it 0, then abort"),
            (5, 4_294_968_320, "`as u32` made it 1024: reported 160 GiB, allocated 682 KiB"),
            (3, 1 << 30, "aborted: the table-entry cap is a product bound"),
        ] {
            let err = build(rows, cols)
                .err()
                .unwrap_or_else(|| panic!("{rows}x{cols} must be refused ({why})"));
            let err = err.to_string();
            assert!(
                err.contains(&rows.to_string()) || err.contains(&cols.to_string()),
                "the refusal should name the value: {err}"
            );
        }
        // The whole legal domain still builds, including both ends.
        for (rows, cols) in [(1usize, 3usize), (5, 2048), (255, 4096)] {
            assert!(build(rows, cols).is_ok(), "{rows}x{cols} is legal");
        }
    }

    /// A `Vector2D` row used to call the library straight through. `cols = 0`
    /// aborted in `ilog2`, and `rows = 0` was worse: it built, ingested, and
    /// wrote a *scored* record whose error was `i32::MAX` — a garbage number
    /// that survived into the output.
    #[test]
    fn vector2d_rows_refuse_a_degenerate_shape() {
        for (rows, cols) in [(0usize, 1024usize), (5, 0), (0, 0)] {
            let p = ParamSet::of(&CmsParams { rows, cols });
            assert!(CmsLibVector2dFast::init(&p).is_err(), "fastpath {rows}x{cols}");
            assert!(CmsLibVector2dRegular::init(&p).is_err(), "regularpath {rows}x{cols}");
            let q = ParamSet::of(&crate::params::CountSketchParams { rows, cols });
            assert!(
                crate::wrappers::cs::sketchlib::CsLibVector2dFast::init(&q).is_err(),
                "cs fastpath {rows}x{cols}"
            );
            assert!(
                crate::wrappers::cs::sketchlib::CsLibVector2dRegular::init(&q).is_err(),
                "cs regularpath {rows}x{cols}"
            );
        }
        assert!(CmsLibVector2dFast::init(&ParamSet::of(&CmsParams { rows: 1, cols: 1 })).is_ok());
    }

    /// Every row in the family agrees on a degenerate shape. This is the
    /// property the whole guard pass exists to restore: one config used to give
    /// four different answers across seven rows — refused, aborted, silently
    /// accepted, and accepted-with-a-scored-record.
    #[test]
    fn the_frequency_rows_agree_on_a_degenerate_shape() {
        let p = ParamSet::of(&CmsParams { rows: 0, cols: 1024 });
        assert!(CmsOxide::init(&p).is_err(), "oxide");
        assert!(CmsDatasketches::init(&p).is_err(), "datasketches");
        assert!(CmsLibVector2dFast::init(&p).is_err(), "vector2d fastpath");
        assert!(CmsLibVector2dRegular::init(&p).is_err(), "vector2d regularpath");
        // The fixed-shape and parallel rows already refused it, via require_shape.
        assert!(CmsLibFixedmatrix::<crate::wrappers::fixed_matrix::M5x2048>::init(&p).is_err());
    }
}

