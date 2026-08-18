//! Count-Min wrappers — five types, each answering a `&i64` point lookup with a
//! `u64` estimate in its own `ask_*` rather than through a shared trait. All
//! take `(rows, cols)` and honour it, by four different routes.

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod datasketches;
pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// Invert `(rows, cols)` into the error bounds `sketch_oxide`'s constructor
/// takes, against its own `ceil(2/ε).next_power_of_two()`. The half-step keeps
/// the quotient off the boundary one ulp would tip, doubling the table.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = 2.0 / (cols as f64 - 0.5);
    let delta = (-(rows as f64 - 0.5)).exp();
    (epsilon, delta)
}

#[cfg(test)]
mod tests {
    use super::datasketches::*;
    use super::oxide::*;
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;
    use crate::wrappers::cs::oxide::{build_cs_oxide, memory_cs_oxide};

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
        let sketch = build_cms_oxide(&shape(), 1).expect("5x2048 is a valid oxide shape");
        assert_eq!(memory_cms_oxide(&sketch), 5 * 2048 * 8);
    }

    /// A `cols` the crate cannot resolve exactly is refused, naming the table it
    /// would have built: the error bound the API takes cannot express an
    /// arbitrary column count, and a silently-rounded one misplaces every plot.
    #[test]
    fn oxide_cms_refuses_a_cols_it_cannot_resolve_exactly() {
        let Err(err) = build_cms_oxide(
            &ParamSet::of(&CmsParams {
                rows: 5,
                cols: 3000,
            }),
            1,
        ) else {
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
        let Err(err) = build_cs_oxide(
            &ParamSet::of(&crate::params::CountSketchParams {
                rows: 2,
                cols: 2048,
            }),
            1,
        ) else {
            panic!("rows=2 resolves to a 3-row table, so it must be refused");
        };
        assert!(err.to_string().contains("2048"), "{err}");
    }

    /// One `--config`, one counter budget, across the two oxide rows — the
    /// property the ε inversions exist to hold. Without it a CMS-vs-CountSketch
    /// comparison at one config compares two different budgets.
    #[test]
    fn the_two_oxide_rows_resolve_one_config_to_one_shape() {
        let cms = build_cms_oxide(&shape(), 1).expect("5x2048 is a valid oxide shape");
        let cs = build_cs_oxide(
            &ParamSet::of(&crate::params::CountSketchParams {
                rows: 5,
                cols: 2048,
            }),
            1,
        )
        .expect("5x2048 is a valid oxide shape");

        // 8 bytes per counter on both sides: `table: Vec<u64>` and `Vec<i64>`.
        // Sizing either as 32-bit once halved a reported footprint and made one
        // algorithm look twice as space-efficient at identical measured accuracy.
        assert_eq!(memory_cms_oxide(&cms), 5 * 2048 * 8);
        assert_eq!(memory_cs_oxide(&cs), 5 * 2048 * 8);
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
                let cms = build_cms_oxide(&ParamSet::of(&CmsParams { rows, cols }), 1)
                    .unwrap_or_else(|e| panic!("cms {rows}x{cols}: {e}"));
                assert_eq!(memory_cms_oxide(&cms), rows * cols * 8, "cms {rows}x{cols}");
                let cs = build_cs_oxide(
                    &ParamSet::of(&crate::params::CountSketchParams { rows, cols }),
                    1,
                )
                .unwrap_or_else(|e| panic!("countsketch {rows}x{cols}: {e}"));
                assert_eq!(
                    memory_cs_oxide(&cs),
                    rows * cols * 8,
                    "countsketch {rows}x{cols}"
                );
            }
        }
    }

    /// The datasketches API takes `(u8, u32)` and asserts inside C++, so every
    /// bound has to be checked on this side. Each value below reproduced a
    /// distinct failure before the guards existed.
    #[test]
    fn datasketches_refuses_what_its_api_cannot_take() {
        let build = |rows: usize, cols: usize| {
            build_cms_datasketches(&ParamSet::of(&CmsParams { rows, cols }), 1)
        };
        for (rows, cols, why) in [
            (
                0usize,
                1024usize,
                "aborted inside C++: num_hashes must be at least 1",
            ),
            (256, 1024, "`as u8` made it 0, then the same abort"),
            (
                257,
                1024,
                "`as u8` made it 1: a one-row sketch labelled 257",
            ),
            (5, 2, "aborted: num_buckets must be at least 3"),
            (5, 4_294_967_296, "`as u32` made it 0, then abort"),
            (
                5,
                4_294_968_320,
                "`as u32` made it 1024: reported 160 GiB, allocated 682 KiB",
            ),
            (
                3,
                1 << 30,
                "aborted: the table-entry cap is a product bound",
            ),
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

    #[test]
    fn vector2d_rows_refuse_a_degenerate_shape() {
        for (rows, cols) in [(0usize, 1024usize), (5, 0), (0, 0)] {
            let p = ParamSet::of(&CmsParams { rows, cols });
            assert!(
                build_cms_lib_vector2d_fast(&p, 1).is_err(),
                "fastpath {rows}x{cols}"
            );
            assert!(
                build_cms_lib_vector2d_regular(&p, 1).is_err(),
                "regularpath {rows}x{cols}"
            );
            let q = ParamSet::of(&crate::params::CountSketchParams { rows, cols });
            assert!(
                crate::wrappers::cs::sketchlib::build_cs_lib_vector2d_fast(&q, 1).is_err(),
                "cs fastpath {rows}x{cols}"
            );
            assert!(
                crate::wrappers::cs::sketchlib::build_cs_lib_vector2d_regular(&q, 1).is_err(),
                "cs regularpath {rows}x{cols}"
            );
        }
        assert!(
            build_cms_lib_vector2d_fast(&ParamSet::of(&CmsParams { rows: 1, cols: 1 }), 1).is_ok()
        );
    }

    /// Every row in the family agrees on a degenerate shape.
    #[test]
    fn the_frequency_rows_agree_on_a_degenerate_shape() {
        let p = ParamSet::of(&CmsParams {
            rows: 0,
            cols: 1024,
        });
        assert!(build_cms_oxide(&p, 1).is_err(), "oxide");
        assert!(build_cms_datasketches(&p, 1).is_err(), "datasketches");
        assert!(
            build_cms_lib_vector2d_fast(&p, 1).is_err(),
            "vector2d fastpath"
        );
        assert!(
            build_cms_lib_vector2d_regular(&p, 1).is_err(),
            "vector2d regularpath"
        );
        // The fixed-shape and parallel rows already refused it, via require_shape.
        assert!(
            build_cms_lib_fixedmatrix::<crate::wrappers::fixed_matrix::M5x2048>(&p, 1).is_err()
        );
    }
}
